use fact_envelope::encode_hex;
use serde::Serialize;
use solana_domain::{AccountWrite, ExecutionStatus, MessageVersion, SolanaTransaction};
use thiserror::Error;

/// Stable Solana native parser identity.
pub const SOLANA_PARSER_VERSION: &str = "solana-native/1.0.0";

/// Explicit ingestion tier attached to every Solana fact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SolanaCoverageTier {
    /// All executed transactions, instructions, logs, and balance changes.
    AllTransactions,
    /// Only explicitly configured account-write identities.
    SelectedAccounts,
}

impl SolanaCoverageTier {
    /// Stable storage/API encoding.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AllTransactions => "all_transactions",
            Self::SelectedAccounts => "selected_accounts",
        }
    }
}

/// Validated status and lineage for one Solana fact revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SolanaFactContext {
    canonicality: String,
    commitment: String,
    revision: u64,
    coverage_tier: SolanaCoverageTier,
    source_id: Option<String>,
    source_session_id: Option<[u8; 16]>,
    observation_id: Option<[u8; 32]>,
    recorded_at_unix_ns: Option<i64>,
}

impl SolanaFactContext {
    /// Creates chain-native fact status.
    ///
    /// # Errors
    ///
    /// Rejects revision zero and contradictory canonicality/commitment pairs.
    pub fn new(
        canonicality: impl Into<String>,
        commitment: impl Into<String>,
        revision: u64,
        coverage_tier: SolanaCoverageTier,
    ) -> Result<Self, SolanaFactError> {
        if revision == 0 {
            return Err(SolanaFactError::ZeroRevision);
        }
        let canonicality = canonicality.into();
        let commitment = commitment.into();
        let consistent = match commitment.as_str() {
            "dead" => canonicality == "non_canonical",
            "received" => canonicality == "candidate",
            "processed" => matches!(canonicality.as_str(), "candidate" | "canonical"),
            "confirmed" | "finalized" => canonicality == "canonical",
            _ => false,
        };
        if !consistent {
            return Err(SolanaFactError::InvalidStatus {
                canonicality,
                commitment,
            });
        }
        Ok(Self {
            canonicality,
            commitment,
            revision,
            coverage_tier,
            source_id: None,
            source_session_id: None,
            observation_id: None,
            recorded_at_unix_ns: None,
        })
    }

    /// Attaches exact source observation lineage.
    ///
    /// # Errors
    ///
    /// Rejects blank source identity and negative receipt time.
    pub fn with_lineage(
        mut self,
        source_id: impl Into<String>,
        source_session_id: [u8; 16],
        observation_id: [u8; 32],
        recorded_at_unix_ns: i64,
    ) -> Result<Self, SolanaFactError> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() || !source_id.is_ascii() {
            return Err(SolanaFactError::EmptySourceId);
        }
        if recorded_at_unix_ns < 0 {
            return Err(SolanaFactError::InvalidRecordedTime);
        }
        self.source_id = Some(source_id);
        self.source_session_id = Some(source_session_id);
        self.observation_id = Some(observation_id);
        self.recorded_at_unix_ns = Some(recorded_at_unix_ns);
        Ok(self)
    }

    fn lineage(&self) -> Result<SolanaLineage, SolanaFactError> {
        Ok(SolanaLineage {
            source_id: self
                .source_id
                .clone()
                .ok_or(SolanaFactError::MissingLineage)?,
            source_session_id: encode_hex(
                &self
                    .source_session_id
                    .ok_or(SolanaFactError::MissingLineage)?,
            ),
            observation_id: encode_hex(
                &self.observation_id.ok_or(SolanaFactError::MissingLineage)?,
            ),
            recorded_at_unix_ns: self
                .recorded_at_unix_ns
                .ok_or(SolanaFactError::MissingLineage)?,
        })
    }
}

#[derive(Clone, Debug)]
struct SolanaLineage {
    source_id: String,
    source_session_id: String,
    observation_id: String,
    recorded_at_unix_ns: i64,
}

/// One independently insertable Solana fact batch.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SolanaFactBatch {
    /// Executed transaction rows.
    pub transactions: Vec<SolanaTransactionFactRow>,
    /// Outer and CPI instruction rows.
    pub instructions: Vec<SolanaInstructionFactRow>,
    /// Runtime log rows.
    pub logs: Vec<SolanaLogFactRow>,
    /// Lamport balance changes.
    pub balance_changes: Vec<SolanaBalanceChangeFactRow>,
    /// Raw token-unit balance changes.
    pub token_balance_changes: Vec<SolanaTokenBalanceChangeFactRow>,
    /// Selected-account writes.
    pub account_writes: Vec<SolanaAccountWriteFactRow>,
}

impl SolanaFactBatch {
    /// Derives native transaction facts without decoding program bytes.
    ///
    /// # Errors
    ///
    /// Rejects missing lineage, the selected-account tier, and index overflow.
    #[allow(clippy::too_many_lines)]
    pub fn from_transaction(
        transaction: &SolanaTransaction,
        context: &SolanaFactContext,
    ) -> Result<Self, SolanaFactError> {
        if context.coverage_tier != SolanaCoverageTier::AllTransactions {
            return Err(SolanaFactError::WrongCoverageTier);
        }
        let lineage = context.lineage()?;
        let fork = transaction.key().fork_id();
        let signature = transaction.key().signature().to_string();
        let slot = fork.slot().value();
        let blockhash = fork.blockhash().to_string();
        let (execution_status, execution_error) = match transaction.status() {
            ExecutionStatus::Succeeded => ("succeeded".to_owned(), String::new()),
            ExecutionStatus::Failed { error } => ("failed".to_owned(), error.clone()),
        };
        let transactions = vec![SolanaTransactionFactRow {
            signature: signature.clone(),
            slot,
            blockhash: blockhash.clone(),
            message_version: match transaction.message().version() {
                MessageVersion::Legacy => "legacy",
                MessageVersion::V0 => "v0",
            }
            .to_owned(),
            static_account_keys: transaction
                .message()
                .static_account_keys()
                .iter()
                .map(ToString::to_string)
                .collect(),
            address_table_lookup_accounts: transaction
                .message()
                .address_table_lookups()
                .iter()
                .map(|lookup| lookup.account_key().to_string())
                .collect(),
            raw_transaction_hex: encode_hex(transaction.raw_transaction()),
            fee: transaction.fee().value().to_string(),
            compute_units_consumed: transaction.compute_units_consumed(),
            execution_status,
            execution_error,
            canonicality: context.canonicality.clone(),
            commitment: context.commitment.clone(),
            coverage_tier: context.coverage_tier.as_str().to_owned(),
            revision: context.revision,
            source_id: lineage.source_id.clone(),
            source_session_id: lineage.source_session_id.clone(),
            observation_id: lineage.observation_id.clone(),
            parser_version: SOLANA_PARSER_VERSION.to_owned(),
            recorded_at_unix_ns: lineage.recorded_at_unix_ns,
        }];
        let mut instructions = Vec::new();
        for (index, instruction) in transaction.message().instructions().iter().enumerate() {
            instructions.push(SolanaInstructionFactRow {
                signature: signature.clone(),
                slot,
                blockhash: blockhash.clone(),
                outer_index: u16::try_from(index).map_err(|_| SolanaFactError::IndexOverflow)?,
                inner_index: None,
                program_id_index: instruction.program_id_index(),
                account_indexes: instruction.account_indexes().to_vec(),
                raw_data_hex: encode_hex(instruction.data()),
                canonicality: context.canonicality.clone(),
                commitment: context.commitment.clone(),
                revision: context.revision,
                source_id: lineage.source_id.clone(),
                source_session_id: lineage.source_session_id.clone(),
                observation_id: lineage.observation_id.clone(),
                parser_version: SOLANA_PARSER_VERSION.to_owned(),
                recorded_at_unix_ns: lineage.recorded_at_unix_ns,
            });
        }
        for inner in transaction.inner_instructions() {
            instructions.push(SolanaInstructionFactRow {
                signature: signature.clone(),
                slot,
                blockhash: blockhash.clone(),
                outer_index: inner.path().outer_index(),
                inner_index: Some(inner.path().inner_index()),
                program_id_index: inner.instruction().program_id_index(),
                account_indexes: inner.instruction().account_indexes().to_vec(),
                raw_data_hex: encode_hex(inner.instruction().data()),
                canonicality: context.canonicality.clone(),
                commitment: context.commitment.clone(),
                revision: context.revision,
                source_id: lineage.source_id.clone(),
                source_session_id: lineage.source_session_id.clone(),
                observation_id: lineage.observation_id.clone(),
                parser_version: SOLANA_PARSER_VERSION.to_owned(),
                recorded_at_unix_ns: lineage.recorded_at_unix_ns,
            });
        }
        let logs = transaction
            .logs()
            .iter()
            .enumerate()
            .map(|(index, message)| {
                Ok(SolanaLogFactRow {
                    signature: signature.clone(),
                    slot,
                    blockhash: blockhash.clone(),
                    log_index: u32::try_from(index).map_err(|_| SolanaFactError::IndexOverflow)?,
                    message: message.clone(),
                    revision: context.revision,
                    source_id: lineage.source_id.clone(),
                    source_session_id: lineage.source_session_id.clone(),
                    observation_id: lineage.observation_id.clone(),
                    parser_version: SOLANA_PARSER_VERSION.to_owned(),
                    recorded_at_unix_ns: lineage.recorded_at_unix_ns,
                })
            })
            .collect::<Result<Vec<_>, SolanaFactError>>()?;
        let balance_changes = transaction
            .pre_balances()
            .iter()
            .zip(transaction.post_balances())
            .enumerate()
            .map(|(index, (pre, post))| {
                Ok(SolanaBalanceChangeFactRow {
                    signature: signature.clone(),
                    slot,
                    blockhash: blockhash.clone(),
                    account_index: u16::try_from(index)
                        .map_err(|_| SolanaFactError::IndexOverflow)?,
                    pre_lamports: pre.value().to_string(),
                    post_lamports: post.value().to_string(),
                    revision: context.revision,
                    source_id: lineage.source_id.clone(),
                    observation_id: lineage.observation_id.clone(),
                    recorded_at_unix_ns: lineage.recorded_at_unix_ns,
                })
            })
            .collect::<Result<Vec<_>, SolanaFactError>>()?;
        let token_balance_changes = transaction
            .token_balance_changes()
            .iter()
            .map(|change| SolanaTokenBalanceChangeFactRow {
                signature: signature.clone(),
                slot,
                blockhash: blockhash.clone(),
                account_index: change.account_index(),
                mint: change.mint().to_string(),
                pre_amount: change.pre_amount().to_owned(),
                post_amount: change.post_amount().to_owned(),
                decimals: change.decimals(),
                revision: context.revision,
                source_id: lineage.source_id.clone(),
                observation_id: lineage.observation_id.clone(),
                recorded_at_unix_ns: lineage.recorded_at_unix_ns,
            })
            .collect();
        Ok(Self {
            transactions,
            instructions,
            logs,
            balance_changes,
            token_balance_changes,
            account_writes: Vec::new(),
        })
    }

    /// Derives one selected-account write fact.
    ///
    /// # Errors
    ///
    /// Rejects missing lineage and use of the all-transaction tier.
    pub fn from_account_write(
        write: &AccountWrite,
        context: &SolanaFactContext,
    ) -> Result<Self, SolanaFactError> {
        if context.coverage_tier != SolanaCoverageTier::SelectedAccounts {
            return Err(SolanaFactError::WrongCoverageTier);
        }
        let lineage = context.lineage()?;
        Ok(Self {
            account_writes: vec![SolanaAccountWriteFactRow {
                pubkey: write.pubkey().to_string(),
                slot: write.fork_id().slot().value(),
                blockhash: write.fork_id().blockhash().to_string(),
                owner: write.owner().to_string(),
                lamports: write.lamports().value().to_string(),
                raw_data_hex: encode_hex(write.data()),
                executable: write.executable(),
                rent_epoch: write.rent_epoch(),
                write_version: write.write_version(),
                canonicality: context.canonicality.clone(),
                commitment: context.commitment.clone(),
                coverage_tier: context.coverage_tier.as_str().to_owned(),
                revision: context.revision,
                source_id: lineage.source_id,
                source_session_id: lineage.source_session_id,
                observation_id: lineage.observation_id,
                parser_version: SOLANA_PARSER_VERSION.to_owned(),
                recorded_at_unix_ns: lineage.recorded_at_unix_ns,
            }],
            ..Self::default()
        })
    }
}

/// `solana_transactions` insert row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SolanaTransactionFactRow {
    pub signature: String,
    pub slot: u64,
    pub blockhash: String,
    pub message_version: String,
    pub static_account_keys: Vec<String>,
    pub address_table_lookup_accounts: Vec<String>,
    pub raw_transaction_hex: String,
    pub fee: String,
    pub compute_units_consumed: Option<u64>,
    pub execution_status: String,
    pub execution_error: String,
    pub canonicality: String,
    pub commitment: String,
    pub coverage_tier: String,
    pub revision: u64,
    pub source_id: String,
    pub source_session_id: String,
    pub observation_id: String,
    pub parser_version: String,
    pub recorded_at_unix_ns: i64,
}

/// `solana_instructions` insert row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SolanaInstructionFactRow {
    pub signature: String,
    pub slot: u64,
    pub blockhash: String,
    pub outer_index: u16,
    pub inner_index: Option<u16>,
    pub program_id_index: u8,
    pub account_indexes: Vec<u8>,
    pub raw_data_hex: String,
    pub canonicality: String,
    pub commitment: String,
    pub revision: u64,
    pub source_id: String,
    pub source_session_id: String,
    pub observation_id: String,
    pub parser_version: String,
    pub recorded_at_unix_ns: i64,
}

/// `solana_logs` insert row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SolanaLogFactRow {
    pub signature: String,
    pub slot: u64,
    pub blockhash: String,
    pub log_index: u32,
    pub message: String,
    pub revision: u64,
    pub source_id: String,
    pub source_session_id: String,
    pub observation_id: String,
    pub parser_version: String,
    pub recorded_at_unix_ns: i64,
}

/// `solana_balance_changes` insert row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SolanaBalanceChangeFactRow {
    pub signature: String,
    pub slot: u64,
    pub blockhash: String,
    pub account_index: u16,
    pub pre_lamports: String,
    pub post_lamports: String,
    pub revision: u64,
    pub source_id: String,
    pub observation_id: String,
    pub recorded_at_unix_ns: i64,
}

/// `solana_token_balance_changes` insert row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SolanaTokenBalanceChangeFactRow {
    pub signature: String,
    pub slot: u64,
    pub blockhash: String,
    pub account_index: u16,
    pub mint: String,
    pub pre_amount: String,
    pub post_amount: String,
    pub decimals: u8,
    pub revision: u64,
    pub source_id: String,
    pub observation_id: String,
    pub recorded_at_unix_ns: i64,
}

/// `solana_account_writes` insert row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SolanaAccountWriteFactRow {
    pub pubkey: String,
    pub slot: u64,
    pub blockhash: String,
    pub owner: String,
    pub lamports: String,
    pub raw_data_hex: String,
    pub executable: bool,
    pub rent_epoch: u64,
    pub write_version: u64,
    pub canonicality: String,
    pub commitment: String,
    pub coverage_tier: String,
    pub revision: u64,
    pub source_id: String,
    pub source_session_id: String,
    pub observation_id: String,
    pub parser_version: String,
    pub recorded_at_unix_ns: i64,
}

/// Solana native fact boundary failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SolanaFactError {
    /// Revision zero is reserved.
    #[error("Solana fact revision must be positive")]
    ZeroRevision,
    /// Canonicality and commitment contradict each other.
    #[error("invalid Solana status {canonicality}/{commitment}")]
    InvalidStatus {
        /// Supplied canonicality.
        canonicality: String,
        /// Supplied commitment.
        commitment: String,
    },
    /// Source identity is invalid.
    #[error("Solana fact source identity must not be empty")]
    EmptySourceId,
    /// Receipt time cannot be negative.
    #[error("Solana fact recorded time is invalid")]
    InvalidRecordedTime,
    /// Lineage was not attached.
    #[error("Solana fact context is missing source lineage")]
    MissingLineage,
    /// Transaction and account-write facts use different explicit tiers.
    #[error("wrong Solana coverage tier for fact")]
    WrongCoverageTier,
    /// A vector index exceeded the storage contract.
    #[error("Solana fact index exceeds storage width")]
    IndexOverflow,
}
