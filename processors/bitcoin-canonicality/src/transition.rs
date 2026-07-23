use bitcoin_domain::BlockHash;

/// Ordered canonical-chain correction emitted by the state machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockTransition {
    /// A block became canonical.
    Connected {
        /// Block hash.
        hash: BlockHash,
        /// Canonical height.
        height: u32,
        /// Monotonic state revision.
        revision: u64,
    },
    /// A formerly canonical block became non-canonical.
    Disconnected {
        /// Block hash.
        hash: BlockHash,
        /// Former canonical height.
        height: u32,
        /// Monotonic state revision.
        revision: u64,
    },
}

impl BlockTransition {
    /// Constructs a connected transition.
    #[must_use]
    pub const fn connected(hash: BlockHash, height: u32, revision: u64) -> Self {
        Self::Connected {
            hash,
            height,
            revision,
        }
    }

    /// Constructs a disconnected transition.
    #[must_use]
    pub const fn disconnected(hash: BlockHash, height: u32, revision: u64) -> Self {
        Self::Disconnected {
            hash,
            height,
            revision,
        }
    }

    /// Returns the block hash affected by this correction.
    #[must_use]
    pub const fn hash(self) -> BlockHash {
        match self {
            Self::Connected { hash, .. } | Self::Disconnected { hash, .. } => hash,
        }
    }

    /// Returns the monotonic state revision.
    #[must_use]
    pub const fn revision(self) -> u64 {
        match self {
            Self::Connected { revision, .. } | Self::Disconnected { revision, .. } => revision,
        }
    }
}
