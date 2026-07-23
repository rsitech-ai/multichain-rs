use bitcoin_domain::BlockHash;

/// Durable association between source progress and canonical state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StateCheckpoint {
    consumer_offset: u64,
    canonical_tip: Option<BlockHash>,
    revision: u64,
    state_hash: [u8; 32],
}

impl StateCheckpoint {
    pub(crate) const fn new(
        consumer_offset: u64,
        canonical_tip: Option<BlockHash>,
        revision: u64,
        state_hash: [u8; 32],
    ) -> Self {
        Self {
            consumer_offset,
            canonical_tip,
            revision,
            state_hash,
        }
    }

    /// Returns the fully materialized broker offset.
    #[must_use]
    pub const fn consumer_offset(self) -> u64 {
        self.consumer_offset
    }

    /// Returns the canonical tip bound to the checkpoint.
    #[must_use]
    pub const fn canonical_tip(self) -> Option<BlockHash> {
        self.canonical_tip
    }

    /// Returns the last emitted canonical revision.
    #[must_use]
    pub const fn revision(self) -> u64 {
        self.revision
    }

    /// Returns the canonical UTXO state digest.
    #[must_use]
    pub const fn state_hash(self) -> [u8; 32] {
        self.state_hash
    }
}
