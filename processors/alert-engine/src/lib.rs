#![doc = "Deterministic, revision-aware alert evaluation."]

use std::fmt::Write as _;

mod mempool;

pub use api_contract::Completeness;
pub use mempool::{
    AlertTransition, DegradedPolicy, MempoolAlertError, MempoolAlertEvaluation,
    MempoolAlertEvaluator, QuorumFeeBandSnapshot, QuorumVbytesAboveDefinition, SnapshotCause,
};
use serde::Serialize;
use thiserror::Error;

/// A chain-native Bitcoin reorganization event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BitcoinReorgEvent {
    network: String,
    disconnected_tip_hash: String,
    common_ancestor_height: u32,
    reorg_depth: u32,
    revision: u64,
    source_ids: Vec<String>,
    completeness: Completeness,
}

impl BitcoinReorgEvent {
    /// Creates a validated reorganization event.
    ///
    /// # Errors
    ///
    /// Rejects blank identity fields, zero depth/revision, and missing
    /// source-qualified evidence.
    #[allow(clippy::too_many_arguments)]
    pub fn new<I, S>(
        network: impl Into<String>,
        disconnected_tip_hash: impl Into<String>,
        common_ancestor_height: u32,
        reorg_depth: u32,
        revision: u64,
        source_ids: I,
        completeness: Completeness,
    ) -> Result<Self, RuleError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let network = network.into();
        let disconnected_tip_hash = disconnected_tip_hash.into();
        if network.trim().is_empty() || disconnected_tip_hash.trim().is_empty() {
            return Err(RuleError::EmptySubject);
        }
        if revision == 0 {
            return Err(RuleError::ZeroRevision);
        }
        if reorg_depth == 0 {
            return Err(RuleError::ZeroDepth);
        }
        let mut source_ids = source_ids.into_iter().map(Into::into).collect::<Vec<_>>();
        if source_ids.is_empty()
            || source_ids
                .iter()
                .any(|source| source.trim().is_empty() || !source.is_ascii())
        {
            return Err(RuleError::MissingSources);
        }
        source_ids.sort_unstable();
        source_ids.dedup();
        Ok(Self {
            network,
            disconnected_tip_hash,
            common_ancestor_height,
            reorg_depth,
            revision,
            source_ids,
            completeness,
        })
    }
}

/// Deterministic threshold rule for Bitcoin canonical-chain reorganizations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BitcoinReorgRule {
    rule_id: String,
    minimum_depth: u32,
}

impl BitcoinReorgRule {
    /// Constructs an enabled rule with a positive depth threshold.
    ///
    /// # Errors
    ///
    /// Rejects blank rule identity and zero depth.
    pub fn new(rule_id: impl Into<String>, minimum_depth: u32) -> Result<Self, RuleError> {
        let rule_id = rule_id.into();
        if rule_id.trim().is_empty() {
            return Err(RuleError::EmptyRuleId);
        }
        if minimum_depth == 0 {
            return Err(RuleError::ZeroDepth);
        }
        Ok(Self {
            rule_id,
            minimum_depth,
        })
    }

    /// Evaluates one revision without side effects.
    #[must_use]
    pub fn evaluate(&self, event: &BitcoinReorgEvent) -> AlertDecision {
        if event.completeness == Completeness::KnownIncomplete {
            return AlertDecision::SuppressedIncomplete;
        }
        if event.reorg_depth < self.minimum_depth {
            return AlertDecision::BelowThreshold;
        }
        let logical_key = format!(
            "{}\0{}\0{}\0{}",
            self.rule_id, event.network, event.disconnected_tip_hash, event.revision
        );
        AlertDecision::Fire(BitcoinReorgAlert {
            alert_id: encode_hex(blake3::hash(logical_key.as_bytes()).as_bytes()),
            kind: "bitcoin_reorg",
            rule_id: self.rule_id.clone(),
            network: event.network.clone(),
            disconnected_tip_hash: event.disconnected_tip_hash.clone(),
            common_ancestor_height: event.common_ancestor_height,
            reorg_depth: event.reorg_depth,
            revision: event.revision,
            source_ids: event.source_ids.clone(),
            completeness: event.completeness,
        })
    }
}

/// Pure alert evaluation outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AlertDecision {
    /// A fully evidenced event met the configured threshold.
    Fire(BitcoinReorgAlert),
    /// Event depth did not meet the rule threshold.
    BelowThreshold,
    /// Known-incomplete evidence prevented a delivery.
    SuppressedIncomplete,
}

/// Stable payload emitted once per rule/subject revision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BitcoinReorgAlert {
    /// Deterministic idempotency identity.
    pub alert_id: String,
    /// Stable alert kind.
    pub kind: &'static str,
    /// Rule identity.
    pub rule_id: String,
    /// Bitcoin network.
    pub network: String,
    /// Tip removed from the canonical chain.
    pub disconnected_tip_hash: String,
    /// Last common canonical height.
    pub common_ancestor_height: u32,
    /// Count of disconnected blocks.
    pub reorg_depth: u32,
    /// Triggering canonicality revision.
    pub revision: u64,
    /// Exact evidence sources.
    pub source_ids: Vec<String>,
    /// Coverage state at evaluation time.
    pub completeness: Completeness,
}

/// Alert rule or event boundary failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RuleError {
    /// Rule identity was blank.
    #[error("alert rule ID must not be empty")]
    EmptyRuleId,
    /// Event network or subject identity was blank.
    #[error("alert subject identity must not be empty")]
    EmptySubject,
    /// Revision zero is reserved.
    #[error("alert event revision must be positive")]
    ZeroRevision,
    /// Reorg depth and thresholds must be positive.
    #[error("reorg depth must be positive")]
    ZeroDepth,
    /// No valid source evidence was supplied.
    #[error("alert event requires source-qualified evidence")]
    MissingSources,
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut hex, byte| {
            write!(hex, "{byte:02x}").expect("writing into a String cannot fail");
            hex
        })
}
