#![doc = "Dual-source Yellowstone capture with exact protobuf preservation."]

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    env,
    time::Duration,
};

use observation_envelope::SourceSessionId;
use prost::Message as _;
use reqwest::Url;
use solana_domain::{Blockhash, Pubkey, Slot};
use thiserror::Error;
use yellowstone_grpc_client::{Backoff, GeyserGrpcClient, ReconnectConfig};
use yellowstone_grpc_proto::prelude::{
    SubscribeRequest, SubscribeRequestFilterAccounts, SubscribeRequestFilterBlocksMeta,
    SubscribeRequestFilterSlots, SubscribeRequestFilterTransactions, SubscribeUpdate,
    subscribe_update::UpdateOneof,
};

const MAX_SELECTED_ACCOUNTS: usize = 1_024;
const MAX_YELLOWSTONE_MESSAGE_BYTES: usize = 32 * 1_024 * 1_024;

/// One independently identified Yellowstone provider stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct YellowstoneSourceConfig {
    source_id: String,
    provider_id: String,
    endpoint: String,
    token_env: Option<String>,
    selected_accounts: Vec<Pubkey>,
}

impl YellowstoneSourceConfig {
    /// Validates a provider configuration.
    ///
    /// # Errors
    ///
    /// Rejects blank identities, cleartext or credential-bearing endpoints,
    /// literal-looking secret names, and empty/oversized account selections.
    pub fn new(
        source_id: impl Into<String>,
        provider_id: impl Into<String>,
        endpoint: impl Into<String>,
        token_env: Option<&str>,
        selected_accounts: Vec<Pubkey>,
    ) -> Result<Self, YellowstoneConnectorError> {
        let source_id = validated_identity(source_id.into(), "source_id")?;
        let provider_id = validated_identity(provider_id.into(), "provider_id")?;
        let endpoint = endpoint.into();
        validate_endpoint(&endpoint)?;
        let token_env = token_env.map(str::to_owned);
        if let Some(name) = &token_env
            && !is_environment_name(name)
        {
            return Err(YellowstoneConnectorError::InvalidTokenEnvironment);
        }
        if selected_accounts.is_empty() || selected_accounts.len() > MAX_SELECTED_ACCOUNTS {
            return Err(YellowstoneConnectorError::InvalidAccountSelection(
                selected_accounts.len(),
            ));
        }
        let unique = selected_accounts.iter().collect::<BTreeSet<_>>();
        if unique.len() != selected_accounts.len() {
            return Err(YellowstoneConnectorError::DuplicateSelectedAccount);
        }
        Ok(Self {
            source_id,
            provider_id,
            endpoint,
            token_env,
            selected_accounts,
        })
    }

    /// Exact source identity attached to every observation.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Operational provider identity used for independence checks.
    #[must_use]
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    /// Validated TLS endpoint.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Builds the pinned Yellowstone subscription contract.
    ///
    /// Slot updates retain every commitment transition. Transaction delivery
    /// is unfiltered, while account writes are explicitly selected.
    #[must_use]
    #[allow(clippy::zero_sized_map_values)]
    pub fn subscribe_request(&self) -> SubscribeRequest {
        let mut slots = HashMap::new();
        slots.insert(
            "commitment-transitions".to_owned(),
            SubscribeRequestFilterSlots {
                filter_by_commitment: Some(false),
                interslot_updates: Some(true),
            },
        );
        let mut transactions = HashMap::new();
        transactions.insert(
            "all-transactions".to_owned(),
            SubscribeRequestFilterTransactions {
                vote: None,
                failed: None,
                signature: None,
                account_include: Vec::new(),
                account_exclude: Vec::new(),
                account_required: Vec::new(),
                token_accounts: None,
            },
        );
        let mut accounts = HashMap::new();
        accounts.insert(
            "selected-accounts".to_owned(),
            SubscribeRequestFilterAccounts {
                account: self
                    .selected_accounts
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                owner: Vec::new(),
                filters: Vec::new(),
                nonempty_txn_signature: None,
                cuckoo_accounts_filter: None,
            },
        );
        let mut blocks_meta = HashMap::new();
        blocks_meta.insert(
            "block-metadata".to_owned(),
            SubscribeRequestFilterBlocksMeta {},
        );
        SubscribeRequest {
            accounts,
            slots,
            transactions,
            transactions_status: HashMap::new(),
            blocks: HashMap::new(),
            blocks_meta,
            entry: HashMap::new(),
            commitment: None,
            accounts_data_slice: Vec::new(),
            ping: None,
            from_slot: None,
        }
    }

    /// Connects with bounded messages, keepalive, and replay-aware reconnect.
    ///
    /// # Errors
    ///
    /// Fails when the configured token environment is missing or the client
    /// cannot be configured/connected.
    pub async fn connect(&self) -> Result<GeyserGrpcClient, YellowstoneConnectorError> {
        let token = self
            .token_env
            .as_ref()
            .map(|name| {
                env::var(name)
                    .map_err(|_| YellowstoneConnectorError::MissingTokenEnvironment(name.clone()))
            })
            .transpose()?;
        let reconnect = ReconnectConfig::default()
            .with_backoff(Backoff::new(Duration::from_millis(250), 2.0, 8))
            .with_slot_retention(512);
        let builder = GeyserGrpcClient::build_from_shared(self.endpoint.clone())
            .map_err(|error| YellowstoneConnectorError::Client(error.to_string()))?
            .x_token(token)
            .map_err(|error| YellowstoneConnectorError::Client(error.to_string()))?
            .connect_timeout(Duration::from_secs(10))
            .http2_keep_alive_interval(Duration::from_secs(15))
            .keep_alive_while_idle(true)
            .max_decoding_message_size(MAX_YELLOWSTONE_MESSAGE_BYTES)
            .set_reconnect_config(reconnect);
        builder
            .connect()
            .await
            .map_err(|error| YellowstoneConnectorError::Client(error.to_string()))
    }
}

/// The required two-source deployment boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct YellowstoneDeployment {
    sources: [YellowstoneSourceConfig; 2],
}

impl YellowstoneDeployment {
    /// Proves two different source IDs, provider IDs, and endpoints.
    ///
    /// # Errors
    ///
    /// Rejects configurations that do not provide independent observation
    /// identities.
    pub fn new(
        first: YellowstoneSourceConfig,
        second: YellowstoneSourceConfig,
    ) -> Result<Self, YellowstoneConnectorError> {
        if first.source_id == second.source_id
            || first.provider_id == second.provider_id
            || first.endpoint == second.endpoint
        {
            return Err(YellowstoneConnectorError::SourcesNotIndependent);
        }
        Ok(Self {
            sources: [first, second],
        })
    }

    /// Both validated provider streams.
    #[must_use]
    pub const fn sources(&self) -> &[YellowstoneSourceConfig; 2] {
        &self.sources
    }
}

/// Coarse Yellowstone update type, before chain interpretation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum UpdateKind {
    /// Selected account write.
    Account,
    /// Slot lifecycle/commitment transition.
    Slot,
    /// Executed transaction with metadata.
    Transaction,
    /// Transaction status-only update.
    TransactionStatus,
    /// Reconstructed block.
    Block,
    /// Block metadata.
    BlockMeta,
    /// Ledger entry.
    Entry,
    /// Protocol keepalive.
    Ping,
    /// Protocol keepalive response.
    Pong,
}

/// Exact protobuf bytes and source/session ordering, persisted before parsing
/// results become facts.
#[derive(Clone, Debug, PartialEq)]
pub struct CapturedUpdate {
    source_id: String,
    source_session_id: SourceSessionId,
    source_sequence: u64,
    observed_at_unix_ns: i64,
    exact_protobuf: Vec<u8>,
    decoded: SubscribeUpdate,
    kind: UpdateKind,
}

impl CapturedUpdate {
    /// Decodes a bounded message while retaining its exact original bytes.
    ///
    /// # Errors
    ///
    /// Rejects malformed, empty, oversized, or unqualified observations.
    pub fn decode(
        source_id: impl Into<String>,
        source_session_id: SourceSessionId,
        source_sequence: u64,
        observed_at_unix_ns: i64,
        exact_protobuf: &[u8],
    ) -> Result<Self, YellowstoneConnectorError> {
        let source_id = validated_identity(source_id.into(), "source_id")?;
        if source_sequence == 0 {
            return Err(YellowstoneConnectorError::InvalidSourceSequence);
        }
        if observed_at_unix_ns < 0 {
            return Err(YellowstoneConnectorError::InvalidObservedTime);
        }
        if exact_protobuf.len() > MAX_YELLOWSTONE_MESSAGE_BYTES {
            return Err(YellowstoneConnectorError::MessageTooLarge(
                exact_protobuf.len(),
            ));
        }
        let decoded =
            SubscribeUpdate::decode(exact_protobuf).map_err(YellowstoneConnectorError::Decode)?;
        let kind = update_kind(&decoded)?;
        Ok(Self {
            source_id,
            source_session_id,
            source_sequence,
            observed_at_unix_ns,
            exact_protobuf: exact_protobuf.to_vec(),
            decoded,
            kind,
        })
    }

    /// Exact provider bytes for WAL/archive persistence.
    #[must_use]
    pub fn exact_protobuf(&self) -> &[u8] {
        &self.exact_protobuf
    }

    /// Stable configured observer identity.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Connector session identity.
    #[must_use]
    pub const fn source_session_id(&self) -> SourceSessionId {
        self.source_session_id
    }

    /// Monotonic provider-frame position within the session.
    #[must_use]
    pub const fn source_sequence(&self) -> u64 {
        self.source_sequence
    }

    /// Local receipt time.
    #[must_use]
    pub const fn observed_at_unix_ns(&self) -> i64 {
        self.observed_at_unix_ns
    }

    /// Wire update category.
    #[must_use]
    pub const fn kind(&self) -> UpdateKind {
        self.kind
    }

    /// Source-reported slot when this update kind carries one.
    #[must_use]
    pub fn slot(&self) -> Option<u64> {
        match self.decoded.update_oneof.as_ref()? {
            UpdateOneof::Account(update) => Some(update.slot),
            UpdateOneof::Slot(update) => Some(update.slot),
            UpdateOneof::Transaction(update) => Some(update.slot),
            UpdateOneof::TransactionStatus(update) => Some(update.slot),
            UpdateOneof::Block(update) => Some(update.slot),
            UpdateOneof::BlockMeta(update) => Some(update.slot),
            UpdateOneof::Entry(update) => Some(update.slot),
            UpdateOneof::Ping(_) | UpdateOneof::Pong(_) => None,
        }
    }

    /// Parsed Yellowstone message. Exact bytes remain the replay truth.
    #[must_use]
    pub const fn decoded(&self) -> &SubscribeUpdate {
        &self.decoded
    }
}

fn update_kind(update: &SubscribeUpdate) -> Result<UpdateKind, YellowstoneConnectorError> {
    match update.update_oneof.as_ref() {
        Some(UpdateOneof::Account(_)) => Ok(UpdateKind::Account),
        Some(UpdateOneof::Slot(_)) => Ok(UpdateKind::Slot),
        Some(UpdateOneof::Transaction(_)) => Ok(UpdateKind::Transaction),
        Some(UpdateOneof::TransactionStatus(_)) => Ok(UpdateKind::TransactionStatus),
        Some(UpdateOneof::Block(_)) => Ok(UpdateKind::Block),
        Some(UpdateOneof::BlockMeta(_)) => Ok(UpdateKind::BlockMeta),
        Some(UpdateOneof::Entry(_)) => Ok(UpdateKind::Entry),
        Some(UpdateOneof::Ping(_)) => Ok(UpdateKind::Ping),
        Some(UpdateOneof::Pong(_)) => Ok(UpdateKind::Pong),
        None => Err(YellowstoneConnectorError::EmptyUpdate),
    }
}

/// Explicit sequence gap inside one source session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceGap {
    expected_sequence: u64,
    observed_sequence: u64,
}

impl SourceGap {
    /// First missing source sequence.
    #[must_use]
    pub const fn expected_sequence(self) -> u64 {
        self.expected_sequence
    }

    /// First sequence seen after the gap.
    #[must_use]
    pub const fn observed_sequence(self) -> u64 {
        self.observed_sequence
    }
}

/// Session-local source cursor. Reconnection never pretends sequence
/// continuity across sessions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionCursor {
    source_id: String,
    source_session_id: SourceSessionId,
    last_sequence: u64,
}

impl SessionCursor {
    /// Starts a new source session.
    ///
    /// # Errors
    ///
    /// Rejects blank source identities.
    pub fn start(
        source_id: impl Into<String>,
        source_session_id: SourceSessionId,
    ) -> Result<Self, YellowstoneConnectorError> {
        Ok(Self {
            source_id: validated_identity(source_id.into(), "source_id")?,
            source_session_id,
            last_sequence: 0,
        })
    }

    /// Advances the cursor and reports a gap without hiding it.
    ///
    /// # Errors
    ///
    /// Rejects duplicate or regressing sequences.
    pub fn observe(
        &mut self,
        source_sequence: u64,
    ) -> Result<Option<SourceGap>, YellowstoneConnectorError> {
        if source_sequence == 0 || source_sequence <= self.last_sequence {
            return Err(YellowstoneConnectorError::InvalidSourceSequence);
        }
        let expected = self
            .last_sequence
            .checked_add(1)
            .ok_or(YellowstoneConnectorError::SequenceOverflow)?;
        self.last_sequence = source_sequence;
        Ok((source_sequence != expected).then_some(SourceGap {
            expected_sequence: expected,
            observed_sequence: source_sequence,
        }))
    }

    /// Begins a genuinely new connector session.
    ///
    /// # Errors
    ///
    /// Rejects reuse of the old session identity.
    pub fn reconnect(
        &mut self,
        source_session_id: SourceSessionId,
    ) -> Result<(), YellowstoneConnectorError> {
        if source_session_id == self.source_session_id {
            return Err(YellowstoneConnectorError::ReusedSessionId);
        }
        self.source_session_id = source_session_id;
        self.last_sequence = 0;
        Ok(())
    }

    /// Stable source identity.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Current connector session.
    #[must_use]
    pub const fn source_session_id(&self) -> SourceSessionId {
        self.source_session_id
    }
}

/// Source-qualified blockhash observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceBlockObservation {
    source_id: String,
    slot: Slot,
    blockhash: Blockhash,
    observed_at_unix_ns: i64,
}

impl SourceBlockObservation {
    /// Creates a provider-specific block observation.
    ///
    /// # Errors
    ///
    /// Rejects blank source IDs and negative receipt times.
    pub fn new(
        source_id: impl Into<String>,
        slot: Slot,
        blockhash: Blockhash,
        observed_at_unix_ns: i64,
    ) -> Result<Self, YellowstoneConnectorError> {
        if observed_at_unix_ns < 0 {
            return Err(YellowstoneConnectorError::InvalidObservedTime);
        }
        Ok(Self {
            source_id: validated_identity(source_id.into(), "source_id")?,
            slot,
            blockhash,
            observed_at_unix_ns,
        })
    }

    /// Source identity.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Observed slot.
    #[must_use]
    pub const fn slot(&self) -> Slot {
        self.slot
    }

    /// Observed blockhash.
    #[must_use]
    pub const fn blockhash(&self) -> &Blockhash {
        &self.blockhash
    }

    /// Local receipt time.
    #[must_use]
    pub const fn observed_at_unix_ns(&self) -> i64 {
        self.observed_at_unix_ns
    }
}

/// Two-source disagreement at one slot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderDivergence {
    slot: Slot,
    observations: Vec<SourceBlockObservation>,
}

impl ProviderDivergence {
    /// Divergent slot.
    #[must_use]
    pub const fn slot(&self) -> Slot {
        self.slot
    }

    /// Exact source-qualified claims in source-ID order.
    #[must_use]
    pub fn observations(&self) -> &[SourceBlockObservation] {
        &self.observations
    }
}

/// Health-independent comparison state. Consumers decide whether divergence
/// is actionable based on source coverage and health.
#[derive(Clone, Debug)]
pub struct ProviderReconciler {
    sources: BTreeSet<String>,
    last_slot: BTreeMap<String, Slot>,
    by_slot: BTreeMap<Slot, BTreeMap<String, SourceBlockObservation>>,
}

impl ProviderReconciler {
    /// Creates a two-source reconciler.
    ///
    /// # Errors
    ///
    /// Requires exactly two distinct non-empty source IDs.
    pub fn new<const N: usize>(sources: [&str; N]) -> Result<Self, YellowstoneConnectorError> {
        if N != 2 {
            return Err(YellowstoneConnectorError::SourcesNotIndependent);
        }
        let sources = sources
            .into_iter()
            .map(|source| validated_identity(source.to_owned(), "source_id"))
            .collect::<Result<BTreeSet<_>, _>>()?;
        if sources.len() != 2 {
            return Err(YellowstoneConnectorError::SourcesNotIndependent);
        }
        Ok(Self {
            sources,
            last_slot: BTreeMap::new(),
            by_slot: BTreeMap::new(),
        })
    }

    /// Records a source claim and emits disagreement only once both claims for
    /// the slot exist.
    ///
    /// # Errors
    ///
    /// Rejects unknown sources and source-local slot regression.
    pub fn observe(
        &mut self,
        observation: SourceBlockObservation,
    ) -> Result<Option<ProviderDivergence>, YellowstoneConnectorError> {
        if !self.sources.contains(observation.source_id()) {
            return Err(YellowstoneConnectorError::UnknownSource);
        }
        if self
            .last_slot
            .get(observation.source_id())
            .is_some_and(|last| observation.slot() < *last)
        {
            return Err(YellowstoneConnectorError::SlotRegression);
        }
        self.last_slot
            .insert(observation.source_id.clone(), observation.slot());
        let slot = observation.slot();
        let claims = self.by_slot.entry(slot).or_default();
        claims.insert(observation.source_id.clone(), observation);
        if claims.len() != 2 {
            return Ok(None);
        }
        let observations = claims.values().cloned().collect::<Vec<_>>();
        if observations[0].blockhash == observations[1].blockhash {
            return Ok(None);
        }
        Ok(Some(ProviderDivergence { slot, observations }))
    }
}

fn validated_identity(
    value: String,
    field: &'static str,
) -> Result<String, YellowstoneConnectorError> {
    if value.trim().is_empty() || !value.is_ascii() {
        return Err(YellowstoneConnectorError::InvalidIdentity(field));
    }
    Ok(value)
}

fn validate_endpoint(endpoint: &str) -> Result<(), YellowstoneConnectorError> {
    let parsed = Url::parse(endpoint).map_err(|_| YellowstoneConnectorError::InvalidEndpoint)?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.host_str().is_none()
    {
        return Err(YellowstoneConnectorError::InvalidEndpoint);
    }
    Ok(())
}

fn is_environment_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(b'A'..=b'Z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Yellowstone configuration, capture, or reconciliation failure.
#[derive(Debug, Error)]
pub enum YellowstoneConnectorError {
    /// Source/provider identity is missing or non-ASCII.
    #[error("invalid {0}")]
    InvalidIdentity(&'static str),
    /// Endpoint must be credential-free HTTPS.
    #[error("invalid Yellowstone endpoint")]
    InvalidEndpoint,
    /// Token references must name environment variables, not contain tokens.
    #[error("invalid token environment variable name")]
    InvalidTokenEnvironment,
    /// Configured token environment variable is absent.
    #[error("required token environment variable is missing: {0}")]
    MissingTokenEnvironment(String),
    /// Account selection must be non-empty and bounded.
    #[error("invalid selected-account count: {0}")]
    InvalidAccountSelection(usize),
    /// Account filters must not repeat identities.
    #[error("duplicate selected account")]
    DuplicateSelectedAccount,
    /// Two sources do not prove independent identities.
    #[error("Yellowstone sources are not independent")]
    SourcesNotIndependent,
    /// Transport/client setup failed.
    #[error("Yellowstone client failure: {0}")]
    Client(String),
    /// Incoming provider message exceeds the configured hard limit.
    #[error("Yellowstone message is too large: {0} bytes")]
    MessageTooLarge(usize),
    /// Protobuf decoding failed.
    #[error("invalid Yellowstone protobuf: {0}")]
    Decode(#[source] prost::DecodeError),
    /// A decoded message contains no update.
    #[error("Yellowstone message contains no update")]
    EmptyUpdate,
    /// Source sequence must be positive and monotonic.
    #[error("invalid source sequence")]
    InvalidSourceSequence,
    /// Sequence arithmetic overflowed.
    #[error("source sequence overflow")]
    SequenceOverflow,
    /// Source receipt time cannot be negative.
    #[error("invalid observed time")]
    InvalidObservedTime,
    /// Reconnect must allocate a new session ID.
    #[error("source session ID was reused")]
    ReusedSessionId,
    /// Reconciler received a source outside its configured pair.
    #[error("unknown Yellowstone source")]
    UnknownSource,
    /// A source-local slot claim regressed.
    #[error("source slot regressed")]
    SlotRegression,
}
