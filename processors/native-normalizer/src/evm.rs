use evm_domain::{EvmBlock, EvmNetwork};
use fact_envelope::encode_hex;
use serde::Serialize;
use thiserror::Error;

/// Stable native EVM parser identity.
pub const EVM_PARSER_VERSION: &str = "evm-native/1.0.0";

/// Validated lineage/status context for one EVM block observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvmFactContext {
    network: EvmNetwork,
    canonicality: String,
    finality: String,
    revision: u64,
    source_id: Option<String>,
    source_session_id: Option<[u8; 16]>,
    observation_id: Option<[u8; 32]>,
    recorded_at_unix_ns: Option<i64>,
}

impl EvmFactContext {
    /// Creates a chain-native status context.
    ///
    /// # Errors
    ///
    /// Rejects revision zero and status values outside the selected chain.
    pub fn new(
        network: EvmNetwork,
        canonicality: impl Into<String>,
        finality: impl Into<String>,
        revision: u64,
    ) -> Result<Self, EvmFactError> {
        if revision == 0 {
            return Err(EvmFactError::ZeroRevision);
        }
        let canonicality = canonicality.into();
        let finality = finality.into();
        if !matches!(
            canonicality.as_str(),
            "candidate" | "canonical" | "non_canonical"
        ) {
            return Err(EvmFactError::InvalidCanonicality(canonicality));
        }
        let valid_finality = match network {
            EvmNetwork::EthereumMainnet => matches!(
                finality.as_str(),
                "pending" | "included" | "canonical_head" | "safe" | "finalized" | "reorged"
            ),
            EvmNetwork::BscMainnet => matches!(
                finality.as_str(),
                "pending" | "included" | "canonical_head" | "fast_finalized" | "reorged"
            ),
        };
        if !valid_finality {
            return Err(EvmFactError::InvalidFinality {
                chain_id: network.chain_id(),
                finality,
            });
        }
        if (canonicality == "non_canonical") != (finality == "reorged") {
            return Err(EvmFactError::InconsistentStatus {
                canonicality,
                finality,
            });
        }
        Ok(Self {
            network,
            canonicality,
            finality,
            revision,
            source_id: None,
            source_session_id: None,
            observation_id: None,
            recorded_at_unix_ns: None,
        })
    }

    /// Attaches exact observation lineage.
    ///
    /// # Errors
    ///
    /// Rejects blank source identity.
    pub fn with_lineage(
        mut self,
        source_id: impl Into<String>,
        source_session_id: [u8; 16],
        observation_id: [u8; 32],
        recorded_at_unix_ns: i64,
    ) -> Result<Self, EvmFactError> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() || !source_id.is_ascii() {
            return Err(EvmFactError::EmptySourceId);
        }
        self.source_id = Some(source_id);
        self.source_session_id = Some(source_session_id);
        self.observation_id = Some(observation_id);
        self.recorded_at_unix_ns = Some(recorded_at_unix_ns);
        Ok(self)
    }

    fn lineage(&self) -> Result<EvmLineage, EvmFactError> {
        Ok(EvmLineage {
            source_id: self.source_id.clone().ok_or(EvmFactError::MissingLineage)?,
            source_session_id: encode_hex(
                &self.source_session_id.ok_or(EvmFactError::MissingLineage)?,
            ),
            observation_id: encode_hex(&self.observation_id.ok_or(EvmFactError::MissingLineage)?),
            recorded_at_unix_ns: self
                .recorded_at_unix_ns
                .ok_or(EvmFactError::MissingLineage)?,
        })
    }
}

#[derive(Clone, Debug)]
struct EvmLineage {
    source_id: String,
    source_session_id: String,
    observation_id: String,
    recorded_at_unix_ns: i64,
}

/// One normalized EVM block and its child facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvmFactBatch {
    /// Block row.
    pub blocks: Vec<EvmBlockFactRow>,
    /// Transaction rows.
    pub transactions: Vec<EvmTransactionFactRow>,
    /// Receipt rows.
    pub receipts: Vec<EvmReceiptFactRow>,
    /// Raw log rows.
    pub logs: Vec<EvmLogFactRow>,
}

impl EvmFactBatch {
    /// Derives chain-qualified native facts from one validated EVM block.
    ///
    /// # Errors
    ///
    /// Rejects context/network mismatch, missing lineage, and index overflow.
    pub fn from_block(block: &EvmBlock, context: &EvmFactContext) -> Result<Self, EvmFactError> {
        if block.network() != context.network {
            return Err(EvmFactError::NetworkMismatch);
        }
        let lineage = context.lineage()?;
        let chain_id = context.network.chain_id();
        let block_hash = block.hash().to_string();
        let blocks = vec![EvmBlockFactRow {
            chain_id,
            block_hash: block_hash.clone(),
            parent_block_hash: block.parent_hash().to_string(),
            block_number: block.number(),
            canonicality: context.canonicality.clone(),
            finality: context.finality.clone(),
            revision: context.revision,
            source_id: lineage.source_id.clone(),
            source_session_id: lineage.source_session_id.clone(),
            observation_id: lineage.observation_id.clone(),
            parser_version: EVM_PARSER_VERSION.to_owned(),
            recorded_at_unix_ns: lineage.recorded_at_unix_ns,
        }];
        let mut transactions = Vec::with_capacity(block.transactions().len());
        let mut receipts = Vec::with_capacity(block.receipts().len());
        let mut logs = Vec::new();
        for (transaction_index, (transaction, receipt)) in block
            .transactions()
            .iter()
            .zip(block.receipts())
            .enumerate()
        {
            let transaction_hash = transaction.hash().to_string();
            transactions.push(EvmTransactionFactRow {
                chain_id,
                block_hash: block_hash.clone(),
                block_number: block.number(),
                transaction_index: u32::try_from(transaction_index)
                    .map_err(|_| EvmFactError::IndexOverflow)?,
                transaction_hash: transaction_hash.clone(),
                sender: transaction.from().to_string(),
                recipient: transaction.to().map(|address| address.to_string()),
                value: transaction.value().to_string(),
                nonce: transaction.nonce(),
                gas_limit: transaction.gas_limit(),
                max_fee_per_gas: transaction
                    .max_fee_per_gas()
                    .map(|amount| amount.to_string()),
                blob_versioned_hashes: transaction
                    .blob_versioned_hashes()
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                canonicality: context.canonicality.clone(),
                finality: context.finality.clone(),
                revision: context.revision,
                source_id: lineage.source_id.clone(),
                source_session_id: lineage.source_session_id.clone(),
                observation_id: lineage.observation_id.clone(),
                parser_version: EVM_PARSER_VERSION.to_owned(),
                recorded_at_unix_ns: lineage.recorded_at_unix_ns,
            });
            receipts.push(EvmReceiptFactRow {
                chain_id,
                block_hash: block_hash.clone(),
                transaction_hash: transaction_hash.clone(),
                success: receipt.success(),
                cumulative_gas_used: receipt.cumulative_gas_used(),
                canonicality: context.canonicality.clone(),
                finality: context.finality.clone(),
                revision: context.revision,
                source_id: lineage.source_id.clone(),
                source_session_id: lineage.source_session_id.clone(),
                observation_id: lineage.observation_id.clone(),
                parser_version: EVM_PARSER_VERSION.to_owned(),
                recorded_at_unix_ns: lineage.recorded_at_unix_ns,
            });
            for log in receipt.logs() {
                logs.push(EvmLogFactRow {
                    chain_id,
                    block_hash: block_hash.clone(),
                    transaction_hash: transaction_hash.clone(),
                    log_index: log.log_index(),
                    address: log.address().to_string(),
                    topics: log.topics().iter().map(ToString::to_string).collect(),
                    raw_data_hex: encode_hex(log.data()),
                    canonicality: context.canonicality.clone(),
                    finality: context.finality.clone(),
                    revision: context.revision,
                    source_id: lineage.source_id.clone(),
                    source_session_id: lineage.source_session_id.clone(),
                    observation_id: lineage.observation_id.clone(),
                    parser_version: EVM_PARSER_VERSION.to_owned(),
                    recorded_at_unix_ns: lineage.recorded_at_unix_ns,
                });
            }
        }
        Ok(Self {
            blocks,
            transactions,
            receipts,
            logs,
        })
    }
}

/// `evm_blocks` insert row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EvmBlockFactRow {
    pub chain_id: u64,
    pub block_hash: String,
    pub parent_block_hash: String,
    pub block_number: u64,
    pub canonicality: String,
    pub finality: String,
    pub revision: u64,
    pub source_id: String,
    pub source_session_id: String,
    pub observation_id: String,
    pub parser_version: String,
    pub recorded_at_unix_ns: i64,
}

/// `evm_transactions` insert row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EvmTransactionFactRow {
    pub chain_id: u64,
    pub block_hash: String,
    pub block_number: u64,
    pub transaction_index: u32,
    pub transaction_hash: String,
    pub sender: String,
    pub recipient: Option<String>,
    pub value: String,
    pub nonce: u64,
    pub gas_limit: u64,
    pub max_fee_per_gas: Option<String>,
    pub blob_versioned_hashes: Vec<String>,
    pub canonicality: String,
    pub finality: String,
    pub revision: u64,
    pub source_id: String,
    pub source_session_id: String,
    pub observation_id: String,
    pub parser_version: String,
    pub recorded_at_unix_ns: i64,
}

/// `evm_receipts` insert row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EvmReceiptFactRow {
    pub chain_id: u64,
    pub block_hash: String,
    pub transaction_hash: String,
    pub success: bool,
    pub cumulative_gas_used: u64,
    pub canonicality: String,
    pub finality: String,
    pub revision: u64,
    pub source_id: String,
    pub source_session_id: String,
    pub observation_id: String,
    pub parser_version: String,
    pub recorded_at_unix_ns: i64,
}

/// `evm_logs` insert row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EvmLogFactRow {
    pub chain_id: u64,
    pub block_hash: String,
    pub transaction_hash: String,
    pub log_index: u64,
    pub address: String,
    pub topics: Vec<String>,
    pub raw_data_hex: String,
    pub canonicality: String,
    pub finality: String,
    pub revision: u64,
    pub source_id: String,
    pub source_session_id: String,
    pub observation_id: String,
    pub parser_version: String,
    pub recorded_at_unix_ns: i64,
}

/// EVM fact boundary failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum EvmFactError {
    #[error("EVM fact revision must be positive")]
    ZeroRevision,
    #[error("invalid EVM canonicality {0}")]
    InvalidCanonicality(String),
    #[error("invalid finality {finality} for chain ID {chain_id}")]
    InvalidFinality { chain_id: u64, finality: String },
    #[error("inconsistent EVM status {canonicality}/{finality}")]
    InconsistentStatus {
        canonicality: String,
        finality: String,
    },
    #[error("EVM fact source identity must not be empty")]
    EmptySourceId,
    #[error("EVM fact context is missing source lineage")]
    MissingLineage,
    #[error("EVM fact context does not match block network")]
    NetworkMismatch,
    #[error("EVM transaction index exceeds u32")]
    IndexOverflow,
}
