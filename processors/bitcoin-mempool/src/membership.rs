use bitcoin_domain::Txid;

/// Source-local membership state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MembershipState {
    /// Transaction is currently present at this observer.
    Present,
    /// Transaction is currently absent at this observer.
    Absent,
}

/// Why a membership revision exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MembershipCause {
    /// Direct add or remove observation.
    Observed,
    /// Removal was directly associated with block inclusion.
    Mined,
    /// Removal was directly associated with replacement.
    Replaced,
    /// Reacceptance followed a disconnected block.
    Disconnected,
    /// Atomic RPC snapshot reconciled current state without inventing time.
    ReconciledSnapshot,
}

/// Directly observed removal classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemovalCause {
    /// Generic observed removal without stronger evidence.
    Observed,
    /// Removal associated with canonical block inclusion.
    Mined,
    /// Removal associated with a replacing transaction.
    Replaced,
}

/// Immutable source-local membership revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MembershipRevision {
    txid: Txid,
    epoch_id: [u8; 32],
    epoch_revision: u32,
    state: MembershipState,
    cause: MembershipCause,
    source_observed_at_unix_ns: Option<i64>,
    recorded_at_unix_ns: i64,
}

impl MembershipRevision {
    pub(crate) const fn new(
        txid: Txid,
        epoch_id: [u8; 32],
        epoch_revision: u32,
        state: MembershipState,
        cause: MembershipCause,
        source_observed_at_unix_ns: Option<i64>,
        recorded_at_unix_ns: i64,
    ) -> Self {
        Self {
            txid,
            epoch_id,
            epoch_revision,
            state,
            cause,
            source_observed_at_unix_ns,
            recorded_at_unix_ns,
        }
    }

    /// Returns the transaction identity.
    #[must_use]
    pub const fn txid(&self) -> Txid {
        self.txid
    }

    /// Returns the deterministic membership-epoch identity.
    #[must_use]
    pub const fn epoch_id(&self) -> [u8; 32] {
        self.epoch_id
    }

    /// Returns the monotonic revision within this epoch.
    #[must_use]
    pub const fn epoch_revision(&self) -> u32 {
        self.epoch_revision
    }

    /// Returns present or absent.
    #[must_use]
    pub const fn state(&self) -> MembershipState {
        self.state
    }

    /// Returns the evidence classification.
    #[must_use]
    pub const fn cause(&self) -> MembershipCause {
        self.cause
    }

    /// Returns the source event time only when directly observed.
    #[must_use]
    pub const fn source_observed_at_unix_ns(&self) -> Option<i64> {
        self.source_observed_at_unix_ns
    }

    /// Returns when the platform recorded this revision.
    #[must_use]
    pub const fn recorded_at_unix_ns(&self) -> i64 {
        self.recorded_at_unix_ns
    }
}

impl From<RemovalCause> for MembershipCause {
    fn from(value: RemovalCause) -> Self {
        match value {
            RemovalCause::Observed => Self::Observed,
            RemovalCause::Mined => Self::Mined,
            RemovalCause::Replaced => Self::Replaced,
        }
    }
}
