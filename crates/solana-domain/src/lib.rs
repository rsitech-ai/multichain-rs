#![doc = "Fork-qualified Solana mainnet-beta identities and bounded execution facts."]

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Visitor};
use thiserror::Error;

pub use solana_hash::Hash as Blockhash;
pub use solana_pubkey::Pubkey;
pub use solana_signature::Signature;

const MAX_ACCOUNT_KEYS: usize = 256;
const MAX_ADDRESS_TABLE_LOOKUPS: usize = 256;
const MAX_INSTRUCTIONS: usize = 4_096;
const MAX_INSTRUCTION_DATA_BYTES: usize = 1_048_576;
const MAX_TRANSACTION_BYTES: usize = 1_048_576;
const MAX_LOG_MESSAGES: usize = 16_384;
const MAX_LOG_MESSAGE_BYTES: usize = 16_384;
const MAX_ERROR_BYTES: usize = 4_096;
const MAX_ACCOUNT_DATA_BYTES: usize = 1_048_576;

/// The only Solana network accepted by this platform slice.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SolanaNetwork {
    /// Solana mainnet-beta.
    MainnetBeta,
}

impl SolanaNetwork {
    /// Stable storage and API name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        "solana-mainnet-beta"
    }
}

/// Slot number without an implied commitment or fork identity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Slot(u64);

impl Slot {
    /// Creates a slot number.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw slot number.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Exact lamport amount encoded as a decimal JSON string.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Lamports(u64);

impl Lamports {
    /// Creates an exact lamport amount.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw amount.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    /// Checked addition.
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

impl Serialize for Lamports {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for Lamports {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct DecimalVisitor;

        impl Visitor<'_> for DecimalVisitor {
            type Value = Lamports;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an unsigned u64 decimal string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(E::custom("invalid lamport decimal string"));
                }
                value.parse::<u64>().map(Lamports).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(DecimalVisitor)
    }
}

/// A specific slot fork. Slot alone is not unique while forks compete.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ForkId {
    slot: Slot,
    blockhash: Blockhash,
}

impl ForkId {
    /// Creates a fork-qualified slot identity.
    #[must_use]
    pub const fn new(slot: Slot, blockhash: Blockhash) -> Self {
        Self { slot, blockhash }
    }

    /// Slot number.
    #[must_use]
    pub const fn slot(&self) -> Slot {
        self.slot
    }

    /// Blockhash for this fork.
    #[must_use]
    pub const fn blockhash(&self) -> &Blockhash {
        &self.blockhash
    }
}

/// An executed transaction identity. Signature alone is intentionally
/// insufficient because the same signature can appear in competing forks.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TransactionKey {
    signature: Signature,
    fork_id: ForkId,
}

impl TransactionKey {
    /// Creates a fork-qualified transaction key.
    #[must_use]
    pub const fn new(signature: Signature, fork_id: ForkId) -> Self {
        Self { signature, fork_id }
    }

    /// Transaction signature.
    #[must_use]
    pub const fn signature(&self) -> &Signature {
        &self.signature
    }

    /// Fork context in which the transaction executed.
    #[must_use]
    pub const fn fork_id(&self) -> &ForkId {
        &self.fork_id
    }
}

/// Solana transaction message wire family.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MessageVersion {
    /// Legacy message without address lookup tables.
    Legacy,
    /// Version-zero message with optional address lookup tables.
    V0,
}

/// One version-zero address lookup table reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddressTableLookup {
    account_key: Pubkey,
    writable_indexes: Vec<u8>,
    readonly_indexes: Vec<u8>,
}

impl AddressTableLookup {
    /// Creates an address table reference while retaining the exact indices.
    #[must_use]
    pub const fn new(
        account_key: Pubkey,
        writable_indexes: Vec<u8>,
        readonly_indexes: Vec<u8>,
    ) -> Self {
        Self {
            account_key,
            writable_indexes,
            readonly_indexes,
        }
    }

    /// Lookup table account.
    #[must_use]
    pub const fn account_key(&self) -> &Pubkey {
        &self.account_key
    }

    /// Writable loaded-address indices.
    #[must_use]
    pub fn writable_indexes(&self) -> &[u8] {
        &self.writable_indexes
    }

    /// Read-only loaded-address indices.
    #[must_use]
    pub fn readonly_indexes(&self) -> &[u8] {
        &self.readonly_indexes
    }
}

/// One compiled outer instruction with raw program bytes retained.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledInstruction {
    program_id_index: u8,
    account_indexes: Vec<u8>,
    data: Vec<u8>,
}

impl CompiledInstruction {
    /// Validates and creates an instruction.
    ///
    /// # Errors
    ///
    /// Rejects instruction data larger than the bounded ingestion contract.
    pub fn try_new(
        program_id_index: u8,
        account_indexes: Vec<u8>,
        data: Vec<u8>,
    ) -> Result<Self, SolanaError> {
        if data.len() > MAX_INSTRUCTION_DATA_BYTES {
            return Err(SolanaError::InstructionDataTooLarge(data.len()));
        }
        Ok(Self {
            program_id_index,
            account_indexes,
            data,
        })
    }

    /// Program index into the fully resolved account-key list.
    #[must_use]
    pub const fn program_id_index(&self) -> u8 {
        self.program_id_index
    }

    /// Account indices into the fully resolved account-key list.
    #[must_use]
    pub fn account_indexes(&self) -> &[u8] {
        &self.account_indexes
    }

    /// Exact opaque instruction bytes.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

/// Outer and inner indices identify a CPI instruction without flattening it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InstructionPath {
    outer_index: u16,
    inner_index: u16,
}

impl InstructionPath {
    /// Creates a CPI path.
    #[must_use]
    pub const fn new(outer_index: u16, inner_index: u16) -> Self {
        Self {
            outer_index,
            inner_index,
        }
    }

    /// Outer instruction index.
    #[must_use]
    pub const fn outer_index(self) -> u16 {
        self.outer_index
    }

    /// Inner instruction index within the outer instruction.
    #[must_use]
    pub const fn inner_index(self) -> u16 {
        self.inner_index
    }
}

/// One CPI instruction with its nested path and raw bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InnerInstruction {
    path: InstructionPath,
    instruction: CompiledInstruction,
}

impl InnerInstruction {
    /// Creates a bounded inner instruction.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`CompiledInstruction::try_new`].
    pub fn try_new(
        path: InstructionPath,
        program_id_index: u8,
        account_indexes: Vec<u8>,
        data: Vec<u8>,
    ) -> Result<Self, SolanaError> {
        Ok(Self {
            path,
            instruction: CompiledInstruction::try_new(program_id_index, account_indexes, data)?,
        })
    }

    /// Nested instruction location.
    #[must_use]
    pub const fn path(&self) -> InstructionPath {
        self.path
    }

    /// Compiled instruction payload.
    #[must_use]
    pub const fn instruction(&self) -> &CompiledInstruction {
        &self.instruction
    }
}

/// A bounded legacy or version-zero message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SolanaMessage {
    version: MessageVersion,
    static_account_keys: Vec<Pubkey>,
    address_table_lookups: Vec<AddressTableLookup>,
    instructions: Vec<CompiledInstruction>,
}

impl SolanaMessage {
    /// Validates message cardinalities.
    ///
    /// # Errors
    ///
    /// Rejects empty or oversized account lists, lookup tables on legacy
    /// messages, and oversized instruction/lookup collections.
    pub fn try_new(
        version: MessageVersion,
        static_account_keys: Vec<Pubkey>,
        address_table_lookups: Vec<AddressTableLookup>,
        instructions: Vec<CompiledInstruction>,
    ) -> Result<Self, SolanaError> {
        if static_account_keys.is_empty() || static_account_keys.len() > MAX_ACCOUNT_KEYS {
            return Err(SolanaError::InvalidAccountKeyCount(
                static_account_keys.len(),
            ));
        }
        if address_table_lookups.len() > MAX_ADDRESS_TABLE_LOOKUPS {
            return Err(SolanaError::TooManyAddressTableLookups(
                address_table_lookups.len(),
            ));
        }
        if version == MessageVersion::Legacy && !address_table_lookups.is_empty() {
            return Err(SolanaError::LegacyMessageWithAddressLookups);
        }
        if instructions.len() > MAX_INSTRUCTIONS {
            return Err(SolanaError::TooManyInstructions(instructions.len()));
        }
        Ok(Self {
            version,
            static_account_keys,
            address_table_lookups,
            instructions,
        })
    }

    /// Message wire family.
    #[must_use]
    pub const fn version(&self) -> MessageVersion {
        self.version
    }

    /// Statically encoded account keys.
    #[must_use]
    pub fn static_account_keys(&self) -> &[Pubkey] {
        &self.static_account_keys
    }

    /// Version-zero lookup references.
    #[must_use]
    pub fn address_table_lookups(&self) -> &[AddressTableLookup] {
        &self.address_table_lookups
    }

    /// Outer instructions.
    #[must_use]
    pub fn instructions(&self) -> &[CompiledInstruction] {
        &self.instructions
    }
}

/// Raw token balance delta. Amounts remain decimal strings because token
/// programs can expose values whose interpretation depends on mint metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenBalanceChange {
    account_index: u16,
    mint: Pubkey,
    pre_amount: String,
    post_amount: String,
    decimals: u8,
}

impl TokenBalanceChange {
    /// Creates a token-balance delta. Ingestion adapters must call
    /// [`Self::validate`] before admitting untrusted values.
    #[must_use]
    pub fn new(
        account_index: u16,
        mint: Pubkey,
        pre_amount: impl Into<String>,
        post_amount: impl Into<String>,
        decimals: u8,
    ) -> Self {
        Self {
            account_index,
            mint,
            pre_amount: pre_amount.into(),
            post_amount: post_amount.into(),
            decimals,
        }
    }

    fn validate(&self) -> Result<(), SolanaError> {
        if !is_decimal(&self.pre_amount) || !is_decimal(&self.post_amount) {
            return Err(SolanaError::InvalidTokenAmount);
        }
        Ok(())
    }

    /// Account index in the resolved message keys.
    #[must_use]
    pub const fn account_index(&self) -> u16 {
        self.account_index
    }

    /// Token mint.
    #[must_use]
    pub const fn mint(&self) -> &Pubkey {
        &self.mint
    }

    /// Raw pre-execution token units.
    #[must_use]
    pub fn pre_amount(&self) -> &str {
        &self.pre_amount
    }

    /// Raw post-execution token units.
    #[must_use]
    pub fn post_amount(&self) -> &str {
        &self.post_amount
    }

    /// Mint decimals reported with the observation.
    #[must_use]
    pub const fn decimals(&self) -> u8 {
        self.decimals
    }
}

/// Transaction execution result. Failed executions remain facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionStatus {
    /// Runtime execution succeeded.
    Succeeded,
    /// Runtime execution failed with source-provided structured text retained.
    Failed {
        /// Bounded source error representation.
        error: String,
    },
}

/// Complete bounded native transaction execution facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SolanaTransaction {
    key: TransactionKey,
    message: SolanaMessage,
    inner_instructions: Vec<InnerInstruction>,
    logs: Vec<String>,
    pre_balances: Vec<Lamports>,
    post_balances: Vec<Lamports>,
    token_balance_changes: Vec<TokenBalanceChange>,
    fee: Lamports,
    compute_units_consumed: Option<u64>,
    status: ExecutionStatus,
    raw_transaction: Vec<u8>,
}

impl SolanaTransaction {
    /// Validates and creates a native transaction fact.
    ///
    /// # Errors
    ///
    /// Rejects mismatched balance vectors and oversized raw/log/error fields.
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        key: TransactionKey,
        message: SolanaMessage,
        inner_instructions: Vec<InnerInstruction>,
        logs: Vec<String>,
        pre_balances: Vec<Lamports>,
        post_balances: Vec<Lamports>,
        token_balance_changes: Vec<TokenBalanceChange>,
        fee: Lamports,
        compute_units_consumed: Option<u64>,
        status: ExecutionStatus,
        raw_transaction: Vec<u8>,
    ) -> Result<Self, SolanaError> {
        if pre_balances.len() != post_balances.len() {
            return Err(SolanaError::BalanceCardinalityMismatch {
                pre: pre_balances.len(),
                post: post_balances.len(),
            });
        }
        if inner_instructions.len() > MAX_INSTRUCTIONS {
            return Err(SolanaError::TooManyInstructions(inner_instructions.len()));
        }
        if logs.len() > MAX_LOG_MESSAGES {
            return Err(SolanaError::TooManyLogs(logs.len()));
        }
        if let Some(log) = logs.iter().find(|log| log.len() > MAX_LOG_MESSAGE_BYTES) {
            return Err(SolanaError::LogMessageTooLarge(log.len()));
        }
        if raw_transaction.len() > MAX_TRANSACTION_BYTES {
            return Err(SolanaError::TransactionTooLarge(raw_transaction.len()));
        }
        if let ExecutionStatus::Failed { error } = &status
            && (error.is_empty() || error.len() > MAX_ERROR_BYTES)
        {
            return Err(SolanaError::InvalidExecutionError(error.len()));
        }
        for change in &token_balance_changes {
            change.validate()?;
        }
        Ok(Self {
            key,
            message,
            inner_instructions,
            logs,
            pre_balances,
            post_balances,
            token_balance_changes,
            fee,
            compute_units_consumed,
            status,
            raw_transaction,
        })
    }

    /// Fork-qualified transaction identity.
    #[must_use]
    pub const fn key(&self) -> &TransactionKey {
        &self.key
    }

    /// Parsed message.
    #[must_use]
    pub const fn message(&self) -> &SolanaMessage {
        &self.message
    }

    /// CPI instructions retaining outer/inner paths.
    #[must_use]
    pub fn inner_instructions(&self) -> &[InnerInstruction] {
        &self.inner_instructions
    }

    /// Runtime log messages.
    #[must_use]
    pub fn logs(&self) -> &[String] {
        &self.logs
    }

    /// Pre-execution lamport balances.
    #[must_use]
    pub fn pre_balances(&self) -> &[Lamports] {
        &self.pre_balances
    }

    /// Post-execution lamport balances.
    #[must_use]
    pub fn post_balances(&self) -> &[Lamports] {
        &self.post_balances
    }

    /// Raw token-unit balance changes.
    #[must_use]
    pub fn token_balance_changes(&self) -> &[TokenBalanceChange] {
        &self.token_balance_changes
    }

    /// Exact charged fee.
    #[must_use]
    pub const fn fee(&self) -> Lamports {
        self.fee
    }

    /// Runtime compute units, when reported by the source.
    #[must_use]
    pub const fn compute_units_consumed(&self) -> Option<u64> {
        self.compute_units_consumed
    }

    /// Execution result, including failures.
    #[must_use]
    pub const fn status(&self) -> &ExecutionStatus {
        &self.status
    }

    /// Exact source transaction bytes where provided.
    #[must_use]
    pub fn raw_transaction(&self) -> &[u8] {
        &self.raw_transaction
    }
}

/// One selected account-write observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountWrite {
    fork_id: ForkId,
    pubkey: Pubkey,
    owner: Pubkey,
    lamports: Lamports,
    data: Vec<u8>,
    executable: bool,
    rent_epoch: u64,
    write_version: u64,
}

impl AccountWrite {
    /// Creates a bounded account write.
    ///
    /// # Errors
    ///
    /// Rejects account payloads larger than the selected-account contract.
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        fork_id: ForkId,
        pubkey: Pubkey,
        owner: Pubkey,
        lamports: Lamports,
        data: Vec<u8>,
        executable: bool,
        rent_epoch: u64,
        write_version: u64,
    ) -> Result<Self, SolanaError> {
        if data.len() > MAX_ACCOUNT_DATA_BYTES {
            return Err(SolanaError::AccountDataTooLarge(data.len()));
        }
        Ok(Self {
            fork_id,
            pubkey,
            owner,
            lamports,
            data,
            executable,
            rent_epoch,
            write_version,
        })
    }

    /// Fork on which the write occurred.
    #[must_use]
    pub const fn fork_id(&self) -> &ForkId {
        &self.fork_id
    }

    /// Written account.
    #[must_use]
    pub const fn pubkey(&self) -> &Pubkey {
        &self.pubkey
    }

    /// Account owner program.
    #[must_use]
    pub const fn owner(&self) -> &Pubkey {
        &self.owner
    }

    /// Exact post-write lamports.
    #[must_use]
    pub const fn lamports(&self) -> Lamports {
        self.lamports
    }

    /// Exact post-write account bytes.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Executable flag.
    #[must_use]
    pub const fn executable(&self) -> bool {
        self.executable
    }

    /// Rent epoch reported by the source.
    #[must_use]
    pub const fn rent_epoch(&self) -> u64 {
        self.rent_epoch
    }

    /// Validator write version for ordering within a slot/fork.
    #[must_use]
    pub const fn write_version(&self) -> u64 {
        self.write_version
    }
}

fn is_decimal(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

/// Invalid or unsafe Solana domain input.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SolanaError {
    /// Static account-key count is zero or exceeds the wire-domain limit.
    #[error("invalid static account key count: {0}")]
    InvalidAccountKeyCount(usize),
    /// Legacy messages cannot load address-table entries.
    #[error("legacy message cannot contain address table lookups")]
    LegacyMessageWithAddressLookups,
    /// Address lookup count exceeds the ingestion bound.
    #[error("too many address table lookups: {0}")]
    TooManyAddressTableLookups(usize),
    /// Instruction count exceeds the ingestion bound.
    #[error("too many instructions: {0}")]
    TooManyInstructions(usize),
    /// Raw instruction data exceeds the ingestion bound.
    #[error("instruction data is too large: {0} bytes")]
    InstructionDataTooLarge(usize),
    /// Pre/post lamport vectors must describe the same resolved accounts.
    #[error("balance cardinality mismatch: pre={pre}, post={post}")]
    BalanceCardinalityMismatch {
        /// Number of pre-execution balances.
        pre: usize,
        /// Number of post-execution balances.
        post: usize,
    },
    /// Log count exceeds the ingestion bound.
    #[error("too many log messages: {0}")]
    TooManyLogs(usize),
    /// A log message exceeds the ingestion bound.
    #[error("log message is too large: {0} bytes")]
    LogMessageTooLarge(usize),
    /// Raw transaction bytes exceed the ingestion bound.
    #[error("transaction is too large: {0} bytes")]
    TransactionTooLarge(usize),
    /// Failed execution must contain a bounded non-empty error.
    #[error("invalid execution error length: {0}")]
    InvalidExecutionError(usize),
    /// Token amount is not an unsigned decimal string.
    #[error("invalid token amount")]
    InvalidTokenAmount,
    /// Account data exceeds the selected-account bound.
    #[error("account data is too large: {0} bytes")]
    AccountDataTooLarge(usize),
}
