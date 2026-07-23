#![doc = "Chain-ID-safe shared EVM values with chain-native finality boundaries."]

use std::{collections::HashSet, fmt, str::FromStr as _};

pub use alloy::primitives::{Address, B256, U256};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Visitor};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

/// The two supported EVM mainnets. Finality remains adapter-specific.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EvmNetwork {
    /// Ethereum mainnet, chain ID 1.
    EthereumMainnet,
    /// BNB Smart Chain mainnet, chain ID 56.
    BscMainnet,
}

impl EvmNetwork {
    /// Returns the EIP-155 chain ID.
    #[must_use]
    pub const fn chain_id(self) -> u64 {
        match self {
            Self::EthereumMainnet => 1,
            Self::BscMainnet => 56,
        }
    }

    /// Returns the stable external network name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EthereumMainnet => "ethereum-mainnet",
            Self::BscMainnet => "bsc-mainnet",
        }
    }
}

impl TryFrom<u64> for EvmNetwork {
    type Error = EvmError;

    fn try_from(chain_id: u64) -> Result<Self, Self::Error> {
        match chain_id {
            1 => Ok(Self::EthereumMainnet),
            56 => Ok(Self::BscMainnet),
            _ => Err(EvmError::UnsupportedChainId(chain_id)),
        }
    }
}

/// Ethereum execution/consensus status.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EthereumStatus {
    /// Seen in an execution-node mempool.
    Pending,
    /// Included in a candidate execution segment.
    IncludedCandidate,
    /// Canonical execution head ancestry.
    CanonicalHead,
    /// Consensus safe checkpoint ancestry.
    Safe,
    /// Consensus finalized checkpoint ancestry.
    Finalized,
    /// Removed from canonical ancestry.
    Reorged,
}

impl EthereumStatus {
    /// Stable API/storage encoding.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::IncludedCandidate => "included_candidate",
            Self::CanonicalHead => "canonical_head",
            Self::Safe => "safe",
            Self::Finalized => "finalized",
            Self::Reorged => "reorged",
        }
    }
}

/// BSC execution and fast-finality status.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BscStatus {
    /// Seen in an official BSC node's local transaction pool.
    Pending,
    /// Included in a candidate BSC block.
    IncludedCandidate,
    /// Canonical BSC head ancestry.
    CanonicalHead,
    /// Proven ancestor of the BSC-native finalized tag.
    FastFinalized,
    /// Removed from canonical ancestry.
    Reorged,
}

impl BscStatus {
    /// Stable API/storage encoding.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::IncludedCandidate => "included_candidate",
            Self::CanonicalHead => "canonical_head",
            Self::FastFinalized => "fast_finalized",
            Self::Reorged => "reorged",
        }
    }
}

/// Exact 256-bit EVM integer encoded as a decimal JSON string.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EvmAmount(U256);

impl EvmAmount {
    /// Parses an unsigned base-10 value.
    ///
    /// # Errors
    ///
    /// Rejects empty, signed, non-decimal, and values outside `U256`.
    pub fn from_decimal_str(value: &str) -> Result<Self, EvmError> {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(EvmError::InvalidAmount);
        }
        U256::from_str_radix(value, 10)
            .map(Self)
            .map_err(|_| EvmError::InvalidAmount)
    }

    /// Returns the Alloy primitive without narrowing.
    #[must_use]
    pub const fn value(self) -> U256 {
        self.0
    }
}

impl From<u64> for EvmAmount {
    fn from(value: u64) -> Self {
        Self(U256::from(value))
    }
}

impl fmt::Display for EvmAmount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Serialize for EvmAmount {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for EvmAmount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct DecimalVisitor;

        impl Visitor<'_> for DecimalVisitor {
            type Value = EvmAmount;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an unsigned U256 decimal string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                EvmAmount::from_decimal_str(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(DecimalVisitor)
    }
}

/// Chain-independent transaction facts. Finality is intentionally absent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvmTransaction {
    hash: B256,
    from: Address,
    to: Option<Address>,
    value: EvmAmount,
    nonce: u64,
    gas_limit: u64,
    max_fee_per_gas: Option<EvmAmount>,
    blob_versioned_hashes: Vec<B256>,
}

impl EvmTransaction {
    /// Constructs a transaction while retaining contract creation and modern
    /// fee/blob fields.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        hash: B256,
        from: Address,
        to: Option<Address>,
        value: EvmAmount,
        nonce: u64,
        gas_limit: u64,
        max_fee_per_gas: Option<EvmAmount>,
        blob_versioned_hashes: Vec<B256>,
    ) -> Self {
        Self {
            hash,
            from,
            to,
            value,
            nonce,
            gas_limit,
            max_fee_per_gas,
            blob_versioned_hashes,
        }
    }

    /// Transaction identity.
    #[must_use]
    pub const fn hash(&self) -> B256 {
        self.hash
    }

    /// Destination, absent for contract creation.
    #[must_use]
    pub const fn to(&self) -> Option<Address> {
        self.to
    }
}

/// One ordered EVM execution log.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvmLog {
    address: Address,
    topics: Vec<B256>,
    data: Vec<u8>,
    transaction_hash: B256,
    log_index: u64,
}

impl EvmLog {
    /// Constructs a bounded log.
    ///
    /// # Errors
    ///
    /// Rejects more than four EVM topics.
    pub fn new(
        address: Address,
        topics: Vec<B256>,
        data: Vec<u8>,
        transaction_hash: B256,
        log_index: u64,
    ) -> Result<Self, EvmError> {
        if topics.len() > 4 {
            return Err(EvmError::TooManyLogTopics(topics.len()));
        }
        Ok(Self {
            address,
            topics,
            data,
            transaction_hash,
            log_index,
        })
    }

    /// Transaction identity used by the ordered log key.
    #[must_use]
    pub const fn transaction_hash(&self) -> B256 {
        self.transaction_hash
    }

    /// Receipt-global log index.
    #[must_use]
    pub const fn log_index(&self) -> u64 {
        self.log_index
    }
}

/// Transaction execution outcome and ordered logs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvmReceipt {
    transaction_hash: B256,
    success: bool,
    cumulative_gas_used: u64,
    logs: Vec<EvmLog>,
}

impl EvmReceipt {
    /// Constructs a receipt and rejects duplicate or foreign log keys.
    ///
    /// # Errors
    ///
    /// Rejects any log whose transaction hash differs from the receipt and
    /// duplicate `(transaction_hash, log_index)` keys.
    pub fn new(
        transaction_hash: B256,
        success: bool,
        cumulative_gas_used: u64,
        logs: Vec<EvmLog>,
    ) -> Result<Self, EvmError> {
        let mut keys = HashSet::with_capacity(logs.len());
        for log in &logs {
            if log.transaction_hash != transaction_hash {
                return Err(EvmError::LogTransactionMismatch);
            }
            if !keys.insert((log.transaction_hash, log.log_index)) {
                return Err(EvmError::DuplicateLogKey {
                    transaction_hash,
                    log_index: log.log_index,
                });
            }
        }
        Ok(Self {
            transaction_hash,
            success,
            cumulative_gas_used,
            logs,
        })
    }

    /// Transaction identity.
    #[must_use]
    pub const fn transaction_hash(&self) -> B256 {
        self.transaction_hash
    }
}

/// A block whose transactions and receipts have been joined and validated.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvmBlock {
    network: EvmNetwork,
    number: u64,
    hash: B256,
    parent_hash: B256,
    transactions: Vec<EvmTransaction>,
    receipts: Vec<EvmReceipt>,
}

impl EvmBlock {
    /// Constructs a validated block.
    ///
    /// # Errors
    ///
    /// Rejects receipt cardinality or transaction-order mismatches.
    pub fn new(
        network: EvmNetwork,
        number: u64,
        hash: B256,
        parent_hash: B256,
        transactions: Vec<EvmTransaction>,
        receipts: Vec<EvmReceipt>,
    ) -> Result<Self, EvmError> {
        if transactions.len() != receipts.len() {
            return Err(EvmError::ReceiptCardinality {
                transactions: transactions.len(),
                receipts: receipts.len(),
            });
        }
        for (transaction, receipt) in transactions.iter().zip(&receipts) {
            if transaction.hash != receipt.transaction_hash {
                return Err(EvmError::ReceiptTransactionMismatch);
            }
        }
        Ok(Self {
            network,
            number,
            hash,
            parent_hash,
            transactions,
            receipts,
        })
    }

    /// Ordered transactions.
    #[must_use]
    pub fn transactions(&self) -> &[EvmTransaction] {
        &self.transactions
    }
}

/// Exact recorded JSON plus its validated block identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedBlock {
    network: EvmNetwork,
    number: u64,
    block_hash: B256,
    parent_hash: B256,
    transaction_hashes: Vec<B256>,
    raw_json: Vec<u8>,
    raw_json_sha256: [u8; 32],
}

impl RecordedBlock {
    /// Parses the bounded semantic block fixture while retaining exact bytes.
    ///
    /// # Errors
    ///
    /// Rejects malformed JSON, quantities, hashes, or non-empty unknown
    /// transaction encodings.
    pub fn from_json(network: EvmNetwork, raw_json: &[u8]) -> Result<Self, EvmError> {
        let raw: RawBlock = serde_json::from_slice(raw_json).map_err(EvmError::InvalidJson)?;
        let number = parse_quantity_u64(&raw.number)?;
        let block_hash =
            B256::from_str(&raw.hash).map_err(|_| EvmError::InvalidBlockHash(raw.hash))?;
        let parent_hash = B256::from_str(&raw.parent_hash)
            .map_err(|_| EvmError::InvalidBlockHash(raw.parent_hash))?;
        let transaction_hashes = raw
            .transactions
            .iter()
            .map(|hash| {
                B256::from_str(hash).map_err(|_| EvmError::InvalidTransactionHash(hash.to_owned()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let digest = Sha256::digest(raw_json);
        let mut raw_json_sha256 = [0_u8; 32];
        raw_json_sha256.copy_from_slice(&digest);
        Ok(Self {
            network,
            number,
            block_hash,
            parent_hash,
            transaction_hashes,
            raw_json: raw_json.to_vec(),
            raw_json_sha256,
        })
    }

    /// Network selected by the source connector.
    #[must_use]
    pub const fn network(&self) -> EvmNetwork {
        self.network
    }

    /// Block number parsed from an RPC quantity.
    #[must_use]
    pub const fn number(&self) -> u64 {
        self.number
    }

    /// Block hash.
    #[must_use]
    pub const fn block_hash(&self) -> B256 {
        self.block_hash
    }

    /// Exact recorded JSON bytes.
    #[must_use]
    pub fn raw_json(&self) -> &[u8] {
        &self.raw_json
    }

    /// SHA-256 of exact recorded JSON bytes.
    #[must_use]
    pub const fn raw_json_sha256(&self) -> [u8; 32] {
        self.raw_json_sha256
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBlock {
    number: String,
    hash: String,
    #[serde(rename = "parentHash")]
    parent_hash: String,
    transactions: Vec<String>,
    #[serde(rename = "source")]
    _source: String,
    #[serde(rename = "capture_scope")]
    _capture_scope: String,
}

/// EVM domain boundary failure.
#[derive(Debug, Error)]
pub enum EvmError {
    /// Chain ID is outside the supported production scope.
    #[error("unsupported EVM chain ID {0}")]
    UnsupportedChainId(u64),
    /// Decimal amount was malformed or outside `U256`.
    #[error("invalid unsigned U256 decimal amount")]
    InvalidAmount,
    /// More topics were supplied than an EVM log can contain.
    #[error("EVM log contains {0} topics; maximum is 4")]
    TooManyLogTopics(usize),
    /// A log referred to another transaction.
    #[error("log transaction hash does not match receipt")]
    LogTransactionMismatch,
    /// A receipt contained the same ordered log key twice.
    #[error("duplicate log key {transaction_hash}/{log_index}")]
    DuplicateLogKey {
        /// Transaction hash.
        transaction_hash: B256,
        /// Receipt-global log position.
        log_index: u64,
    },
    /// Transactions and receipts differed in count.
    #[error("block has {transactions} transactions and {receipts} receipts")]
    ReceiptCardinality {
        /// Transaction count.
        transactions: usize,
        /// Receipt count.
        receipts: usize,
    },
    /// Receipt order/hash did not match transaction order.
    #[error("receipt transaction hash does not match transaction order")]
    ReceiptTransactionMismatch,
    /// JSON payload was malformed or violated its exact schema.
    #[error("invalid recorded EVM JSON: {0}")]
    InvalidJson(serde_json::Error),
    /// Block hash was malformed.
    #[error("invalid EVM block hash {0}")]
    InvalidBlockHash(String),
    /// Transaction hash was malformed.
    #[error("invalid EVM transaction hash {0}")]
    InvalidTransactionHash(String),
    /// JSON-RPC quantity was non-canonical or outside `u64`.
    #[error("invalid EVM quantity {0}")]
    InvalidQuantity(String),
}

fn parse_quantity_u64(value: &str) -> Result<u64, EvmError> {
    let digits = value
        .strip_prefix("0x")
        .ok_or_else(|| EvmError::InvalidQuantity(value.to_owned()))?;
    if digits.is_empty()
        || (digits.len() > 1 && digits.starts_with('0'))
        || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(EvmError::InvalidQuantity(value.to_owned()));
    }
    u64::from_str_radix(digits, 16).map_err(|_| EvmError::InvalidQuantity(value.to_owned()))
}
