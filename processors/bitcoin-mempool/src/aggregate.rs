use bitcoin_domain::Txid;

use crate::{ObserverHealth, ObserverMempool};

/// Aggregate view selection policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MempoolViewPolicy {
    /// One named healthy observer.
    Source(String),
    /// Present at any healthy observer.
    Union,
    /// Present at every healthy observer.
    Intersection,
    /// Present at at least `required` healthy observers.
    Quorum {
        /// Minimum healthy present sources.
        required: u16,
    },
}

/// Coverage quality of an aggregate membership decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoverageQuality {
    /// Every configured observer is healthy and participating.
    Complete,
    /// At least one healthy observer exists but coverage or clock trust is reduced.
    Degraded,
    /// No healthy observer can support a membership decision.
    Unavailable,
}

/// Health-aware aggregate membership evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AggregateMembership {
    /// Transaction identity.
    pub txid: Txid,
    /// Configured observers currently eligible for policy evaluation.
    pub healthy_source_count: u16,
    /// Eligible observers where the transaction is present.
    pub present_source_count: u16,
    /// Sorted eligible source identities where the transaction is present.
    pub present_sources: Vec<String>,
    /// Whether the selected policy is satisfied.
    pub policy_satisfied: bool,
    /// Minimum clock-trusted source observation for the active epochs.
    pub platform_first_seen_at_unix_ns: Option<i64>,
    /// Whether at least one relevant source timestamp exceeded the clock bound.
    pub clock_untrusted: bool,
    /// Coverage quality for this decision.
    pub quality: CoverageQuality,
}

/// Evaluates one transaction across configured observer-local states.
#[must_use]
pub fn aggregate_membership(
    txid: Txid,
    observers: &[&ObserverMempool],
    policy: &MempoolViewPolicy,
    maximum_clock_offset_ns: u64,
) -> AggregateMembership {
    let healthy: Vec<_> = observers
        .iter()
        .copied()
        .filter(|observer| observer.health() == ObserverHealth::Healthy)
        .collect();
    let mut present_sources: Vec<_> = healthy
        .iter()
        .filter(|observer| observer.contains(&txid))
        .map(|observer| observer.source_id().to_owned())
        .collect();
    present_sources.sort_unstable();
    let healthy_source_count = bounded_count(healthy.len());
    let present_source_count = bounded_count(present_sources.len());

    let mut clock_untrusted = false;
    let platform_first_seen_at_unix_ns = observers
        .iter()
        .filter_map(|observer| {
            let first_seen = observer.first_seen_at_unix_ns(&txid)?;
            if observer.clock_offset_ns().unsigned_abs() > maximum_clock_offset_ns {
                clock_untrusted = true;
                None
            } else {
                Some(first_seen)
            }
        })
        .min();
    let quality = if healthy.is_empty() {
        CoverageQuality::Unavailable
    } else if healthy.len() != observers.len() || clock_untrusted {
        CoverageQuality::Degraded
    } else {
        CoverageQuality::Complete
    };
    let policy_satisfied = if quality == CoverageQuality::Unavailable {
        false
    } else {
        match policy {
            MempoolViewPolicy::Source(source_id) => healthy
                .iter()
                .any(|observer| observer.source_id() == source_id && observer.contains(&txid)),
            MempoolViewPolicy::Union => present_source_count > 0,
            MempoolViewPolicy::Intersection => {
                present_source_count == healthy_source_count && healthy_source_count > 0
            }
            MempoolViewPolicy::Quorum { required } => {
                *required > 0 && present_source_count >= *required
            }
        }
    };

    AggregateMembership {
        txid,
        healthy_source_count,
        present_source_count,
        present_sources,
        policy_satisfied,
        platform_first_seen_at_unix_ns,
        clock_untrusted,
        quality,
    }
}

fn bounded_count(count: usize) -> u16 {
    u16::try_from(count).unwrap_or(u16::MAX)
}
