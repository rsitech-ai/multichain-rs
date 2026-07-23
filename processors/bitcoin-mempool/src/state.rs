use std::collections::{BTreeSet, HashMap};

use bitcoin_domain::Txid;
use sha2::{Digest as _, Sha256};

use crate::{MembershipCause, MembershipRevision, MembershipState, MempoolError, RemovalCause};

/// Current observer health used by aggregate policy evaluation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObserverHealth {
    /// Live and sequence-aligned.
    Healthy,
    /// Live but inside a known incomplete interval.
    Gapped,
    /// Not currently connected.
    Offline,
}

#[derive(Clone, Debug)]
struct ActiveEpoch {
    epoch_id: [u8; 32],
    first_seen_at_unix_ns: Option<i64>,
}

/// One observer's independent mempool projection and immutable epoch history.
#[derive(Clone, Debug)]
pub struct ObserverMempool {
    source_id: String,
    snapshot_sequence: Option<u64>,
    last_snapshot_members: Option<BTreeSet<Txid>>,
    members: HashMap<Txid, ActiveEpoch>,
    epoch_counts: HashMap<Txid, u64>,
    revisions: Vec<MembershipRevision>,
    health: ObserverHealth,
    clock_offset_ns: i64,
}

impl ObserverMempool {
    /// Creates isolated state for one observer.
    ///
    /// # Errors
    ///
    /// Rejects a blank source identity.
    pub fn new(source_id: impl Into<String>) -> Result<Self, MempoolError> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() {
            return Err(MempoolError::EmptySourceId);
        }
        Ok(Self {
            source_id,
            snapshot_sequence: None,
            last_snapshot_members: None,
            members: HashMap::new(),
            epoch_counts: HashMap::new(),
            revisions: Vec::new(),
            health: ObserverHealth::Offline,
            clock_offset_ns: 0,
        })
    }

    /// Applies a directly observed mempool addition.
    pub fn observe_add(
        &mut self,
        txid: Txid,
        observed_at_unix_ns: i64,
    ) -> Option<MembershipRevision> {
        self.add(
            txid,
            MembershipCause::Observed,
            Some(observed_at_unix_ns),
            observed_at_unix_ns,
        )
    }

    /// Applies reacceptance caused by a disconnected block.
    pub fn reaccept_after_disconnect(
        &mut self,
        txid: Txid,
        observed_at_unix_ns: i64,
    ) -> Option<MembershipRevision> {
        self.add(
            txid,
            MembershipCause::Disconnected,
            Some(observed_at_unix_ns),
            observed_at_unix_ns,
        )
    }

    /// Applies a directly observed mempool removal.
    pub fn observe_remove(
        &mut self,
        txid: Txid,
        observed_at_unix_ns: i64,
        cause: RemovalCause,
    ) -> Option<MembershipRevision> {
        self.remove(
            txid,
            cause.into(),
            Some(observed_at_unix_ns),
            observed_at_unix_ns,
        )
    }

    /// Converges current state to one atomic RPC snapshot.
    ///
    /// Reconciled revisions intentionally omit the unknown original source
    /// arrival or removal time.
    ///
    /// # Errors
    ///
    /// Rejects sequence regression or reuse of one sequence for different
    /// membership.
    pub fn apply_snapshot(
        &mut self,
        sequence: u64,
        members: &[Txid],
        recorded_at_unix_ns: i64,
    ) -> Result<Vec<MembershipRevision>, MempoolError> {
        let snapshot: BTreeSet<_> = members.iter().copied().collect();
        if let Some(current) = self.snapshot_sequence {
            if sequence < current {
                return Err(MempoolError::SnapshotSequenceRegression {
                    current,
                    attempted: sequence,
                });
            }
            if sequence == current {
                return if self.last_snapshot_members.as_ref() == Some(&snapshot) {
                    Ok(Vec::new())
                } else {
                    Err(MempoolError::SnapshotSequenceConflict { sequence })
                };
            }
        }

        let current: BTreeSet<_> = self.members.keys().copied().collect();
        let removals: Vec<_> = current.difference(&snapshot).copied().collect();
        let additions: Vec<_> = snapshot.difference(&current).copied().collect();
        let mut revisions = Vec::with_capacity(removals.len() + additions.len());
        for txid in removals {
            if let Some(revision) = self.remove(
                txid,
                MembershipCause::ReconciledSnapshot,
                None,
                recorded_at_unix_ns,
            ) {
                revisions.push(revision);
            }
        }
        for txid in additions {
            if let Some(revision) = self.add(
                txid,
                MembershipCause::ReconciledSnapshot,
                None,
                recorded_at_unix_ns,
            ) {
                revisions.push(revision);
            }
        }
        self.snapshot_sequence = Some(sequence);
        self.last_snapshot_members = Some(snapshot);
        Ok(revisions)
    }

    /// Updates operational health without altering membership history.
    pub const fn set_health(&mut self, health: ObserverHealth, clock_offset_ns: i64) {
        self.health = health;
        self.clock_offset_ns = clock_offset_ns;
    }

    /// Returns the stable source identity.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Returns current operational health.
    #[must_use]
    pub const fn health(&self) -> ObserverHealth {
        self.health
    }

    /// Returns measured source-host clock offset.
    #[must_use]
    pub const fn clock_offset_ns(&self) -> i64 {
        self.clock_offset_ns
    }

    /// Returns whether the transaction is currently present at this observer.
    #[must_use]
    pub fn contains(&self, txid: &Txid) -> bool {
        self.members.contains_key(txid)
    }

    /// Returns trusted first-seen candidate for the active epoch, if observed.
    #[must_use]
    pub fn first_seen_at_unix_ns(&self, txid: &Txid) -> Option<i64> {
        self.members
            .get(txid)
            .and_then(|epoch| epoch.first_seen_at_unix_ns)
    }

    /// Returns immutable membership revisions in replay order.
    #[must_use]
    pub fn revisions(&self) -> &[MembershipRevision] {
        &self.revisions
    }

    fn add(
        &mut self,
        txid: Txid,
        cause: MembershipCause,
        source_observed_at_unix_ns: Option<i64>,
        recorded_at_unix_ns: i64,
    ) -> Option<MembershipRevision> {
        if self.members.contains_key(&txid) {
            return None;
        }
        let epoch_number = self
            .epoch_counts
            .entry(txid)
            .and_modify(|count| *count = count.saturating_add(1))
            .or_insert(1);
        let epoch_id = epoch_id(&self.source_id, txid, *epoch_number);
        self.members.insert(
            txid,
            ActiveEpoch {
                epoch_id,
                first_seen_at_unix_ns: source_observed_at_unix_ns,
            },
        );
        Some(self.record(MembershipRevision::new(
            txid,
            epoch_id,
            1,
            MembershipState::Present,
            cause,
            source_observed_at_unix_ns,
            recorded_at_unix_ns,
        )))
    }

    fn remove(
        &mut self,
        txid: Txid,
        cause: MembershipCause,
        source_observed_at_unix_ns: Option<i64>,
        recorded_at_unix_ns: i64,
    ) -> Option<MembershipRevision> {
        let epoch = self.members.remove(&txid)?;
        Some(self.record(MembershipRevision::new(
            txid,
            epoch.epoch_id,
            2,
            MembershipState::Absent,
            cause,
            source_observed_at_unix_ns,
            recorded_at_unix_ns,
        )))
    }

    fn record(&mut self, revision: MembershipRevision) -> MembershipRevision {
        self.revisions.push(revision.clone());
        revision
    }
}

fn epoch_id(source_id: &str, txid: Txid, epoch_number: u64) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"multichain.bitcoin.mempool.epoch.v1");
    digest.update(
        u64::try_from(source_id.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    digest.update(source_id.as_bytes());
    digest.update(txid.as_bytes());
    digest.update(epoch_number.to_le_bytes());
    digest.finalize().into()
}
