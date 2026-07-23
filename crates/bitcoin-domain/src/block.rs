use std::fmt;

use bitcoin::{Block, hashes::Hash as _};

use crate::BitcoinTransaction;

/// Bitcoin block header digest.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BlockHash([u8; 32]);

/// Owned Bitcoin block with validated merkle commitment.
#[derive(Clone, Debug)]
pub struct BitcoinBlock {
    inner: Block,
    consensus_bytes: Vec<u8>,
    transactions: Vec<BitcoinTransaction>,
}

impl BitcoinBlock {
    pub(crate) fn from_block(inner: Block, consensus_bytes: Vec<u8>) -> Self {
        let transactions = inner
            .txdata
            .iter()
            .cloned()
            .map(|transaction| {
                let bytes = bitcoin::consensus::serialize(&transaction);
                BitcoinTransaction::from_transaction(transaction, bytes)
            })
            .collect();
        Self {
            inner,
            consensus_bytes,
            transactions,
        }
    }

    /// Returns the block-header digest.
    #[must_use]
    pub fn block_hash(&self) -> BlockHash {
        BlockHash(self.inner.block_hash().to_byte_array())
    }

    /// Returns the parent block digest from the header.
    #[must_use]
    pub fn previous_block_hash(&self) -> BlockHash {
        BlockHash(self.inner.header.prev_blockhash.to_byte_array())
    }

    /// Returns the validated transactions.
    #[must_use]
    pub fn transactions(&self) -> &[BitcoinTransaction] {
        &self.transactions
    }

    /// Returns exact block consensus bytes.
    #[must_use]
    pub fn consensus_bytes(&self) -> &[u8] {
        &self.consensus_bytes
    }
}

impl BlockHash {
    /// Constructs from consensus-order digest bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns consensus-order digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for BlockHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        bitcoin::BlockHash::from_byte_array(self.0).fmt(formatter)
    }
}
