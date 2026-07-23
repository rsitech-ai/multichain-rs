use std::fmt;

use bitcoin::{Block, CompactTarget, consensus::Params, hashes::Hash as _};
use chain_domain::BitcoinNetwork;

use crate::BitcoinTransaction;

/// Bitcoin block header digest.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BlockHash([u8; 32]);

/// Comparable accumulated proof-of-work value.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct BlockWork([u8; 32]);

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

    /// Returns the compact proof-of-work target encoded by the header.
    #[must_use]
    pub fn compact_target(&self) -> u32 {
        self.inner.header.bits.to_consensus()
    }

    /// Returns the block-header timestamp.
    #[must_use]
    pub fn timestamp(&self) -> u32 {
        self.inner.header.time
    }

    /// Validates this header against the required compact target.
    #[must_use]
    pub fn has_valid_pow_for(&self, required_compact_target: u32) -> bool {
        self.inner
            .header
            .validate_pow(CompactTarget::from_consensus(required_compact_target).into())
            .is_ok()
    }

    /// Returns the proof-of-work represented by this block's target.
    #[must_use]
    pub fn work(&self) -> BlockWork {
        BlockWork(self.inner.header.work().to_be_bytes())
    }

    /// Calculates the target required for the next block.
    ///
    /// `epoch_start` is required only at a difficulty-adjustment boundary on
    /// networks where retargeting is enabled.
    #[must_use]
    pub fn next_required_target(
        &self,
        network: BitcoinNetwork,
        next_height: u32,
        epoch_start: Option<&Self>,
    ) -> Option<u32> {
        let params = Params::new(bitcoin_network(network));
        let interval = u32::try_from(params.difficulty_adjustment_interval()).ok()?;
        if params.no_pow_retargeting || !next_height.is_multiple_of(interval) {
            return Some(self.compact_target());
        }
        let epoch_start = epoch_start?;
        Some(
            CompactTarget::from_header_difficulty_adjustment(
                epoch_start.inner.header,
                self.inner.header,
                params,
            )
            .to_consensus(),
        )
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

impl BlockWork {
    /// Returns zero accumulated work.
    #[must_use]
    pub const fn zero() -> Self {
        Self([0; 32])
    }

    /// Adds two work values, returning `None` if the 256-bit sum overflows.
    #[must_use]
    pub fn checked_add(self, other: Self) -> Option<Self> {
        let mut sum = [0_u8; 32];
        let mut carry = 0_u16;
        for index in (0..32).rev() {
            let value = u16::from(self.0[index]) + u16::from(other.0[index]) + carry;
            sum[index] = value.to_le_bytes()[0];
            carry = value >> 8;
        }
        (carry == 0).then_some(Self(sum))
    }

    /// Returns big-endian work bytes.
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

const fn bitcoin_network(network: BitcoinNetwork) -> bitcoin::Network {
    match network {
        BitcoinNetwork::Mainnet => bitcoin::Network::Bitcoin,
        BitcoinNetwork::Testnet => bitcoin::Network::Testnet,
        BitcoinNetwork::Signet => bitcoin::Network::Signet,
        BitcoinNetwork::Regtest => bitcoin::Network::Regtest,
    }
}
