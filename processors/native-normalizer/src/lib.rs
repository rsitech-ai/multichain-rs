#![doc = "Deterministic native normalization and Phase 0 `ClickHouse` projection."]

mod bitcoin;
mod evm;
mod solana;

pub use bitcoin::{
    BITCOIN_PARSER_VERSION, BitcoinBlockFactRow, BitcoinFactBatch, BitcoinFactContext,
    BitcoinFactError, BitcoinInputFactRow, BitcoinOutputFactRow, BitcoinTransactionFactRow,
};
pub use evm::{
    EVM_PARSER_VERSION, EvmBlockFactRow, EvmFactBatch, EvmFactContext, EvmFactError, EvmLogFactRow,
    EvmReceiptFactRow, EvmTransactionFactRow,
};
pub use solana::{
    SOLANA_PARSER_VERSION, SolanaAccountWriteFactRow, SolanaBalanceChangeFactRow,
    SolanaCoverageTier, SolanaFactBatch, SolanaFactContext, SolanaFactError,
    SolanaInstructionFactRow, SolanaLogFactRow, SolanaTokenBalanceChangeFactRow,
    SolanaTransactionFactRow,
};

use fact_envelope::{
    FIXTURE_PARSER_VERSION, FactError, FixtureFact, FixtureFactView, FixtureLineage,
    FixturePayload, encode_hex, fixture_fact_id, fixture_fact_key,
};
use platform_proto::{
    fact::Fact,
    observation::{CommittedObservation, Observation},
};
use reqwest::{Client, StatusCode, header::CONTENT_LENGTH};
use serde::{Deserialize, Serialize};
use solana_decoder::DecodeRevisionFactRow;
use thiserror::Error;

const FACT_SCHEMA: &str = include_str!("../../../schemas/clickhouse/001_core.sql");
/// Append-only Bitcoin fact tables and explicit current projections.
pub const BITCOIN_FACT_SCHEMA: &str = include_str!("../../../schemas/clickhouse/002_bitcoin.sql");
/// Append-only EVM fact tables keyed by EIP-155 chain ID.
pub const EVM_FACT_SCHEMA: &str = include_str!("../../../schemas/clickhouse/003_evm.sql");
/// Append-only fork-qualified Solana facts and explicit current projections.
pub const SOLANA_FACT_SCHEMA: &str = include_str!("../../../schemas/clickhouse/004_solana.sql");

/// Explicit state for a source range that cannot yet be proven complete.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GapStatus {
    /// Some collector transitions are not known and require reconciliation.
    KnownIncomplete,
}

/// Exact missing collector range discovered during replay or reconciliation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IncompleteInterval {
    /// Incident state.
    pub status: GapStatus,
    /// First missing collector sequence.
    pub missing_first: u64,
    /// Last missing collector sequence.
    pub missing_last: u64,
}

/// Fixture normalization and `ClickHouse` boundary failures.
#[derive(Debug, Error)]
pub enum NormalizerError {
    /// The committed record omitted its source observation.
    #[error("committed record has no observation")]
    MissingObservation,
    /// A fixed-length observation identity was malformed.
    #[error(transparent)]
    Fact(#[from] FactError),
    /// The source payload was not the expected fixture JSON.
    #[error("fixture payload is invalid: {0}")]
    InvalidPayload(#[from] serde_json::Error),
    /// Source sequence metadata and payload disagree.
    #[error("fixture source sequence mismatch: observation={observation:?}, payload={payload}")]
    SourceSequenceMismatch {
        /// Optional source-native sequence from the observation envelope.
        observation: Option<u64>,
        /// Source-native sequence decoded from the payload.
        payload: u64,
    },
    /// `ClickHouse` access or response validation failed.
    #[error("ClickHouse operation failed: {0}")]
    ClickHouse(String),
}

/// HTTP-native `ClickHouse` adapter for Phase 0 normalized facts.
#[derive(Clone, Debug)]
pub struct ClickHouseFactStore {
    endpoint: String,
    client: Client,
    username: Option<String>,
    password: Option<String>,
}

impl ClickHouseFactStore {
    /// Creates a bounded `ClickHouse` HTTP client and verifies the endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`NormalizerError`] when `ClickHouse` does not answer its ping.
    pub async fn connect(endpoint: &str) -> Result<Self, NormalizerError> {
        let store = Self {
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            client: Client::new(),
            username: None,
            password: None,
        };
        if !store.available().await {
            return Err(NormalizerError::ClickHouse(
                "ClickHouse ping did not return success".to_owned(),
            ));
        }
        Ok(store)
    }

    /// Creates a credentialed `ClickHouse` HTTP client and verifies the endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`NormalizerError`] when the credentials cannot execute a ping.
    pub async fn connect_with_credentials(
        endpoint: &str,
        username: &str,
        password: &str,
    ) -> Result<Self, NormalizerError> {
        let store = Self {
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            client: Client::new(),
            username: Some(username.to_owned()),
            password: Some(password.to_owned()),
        };
        if !store.available().await {
            return Err(NormalizerError::ClickHouse(
                "credentialed ClickHouse ping did not return success".to_owned(),
            ));
        }
        Ok(store)
    }

    /// Returns whether `ClickHouse` is reachable now.
    pub async fn available(&self) -> bool {
        self.authenticated(self.client.get(format!("{}/ping", self.endpoint)))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }

    /// Installs the idempotent Phase 0 fact schema.
    ///
    /// # Errors
    ///
    /// Returns [`NormalizerError`] on the first rejected statement.
    pub async fn install_schema(&self) -> Result<(), NormalizerError> {
        for statement in [
            FACT_SCHEMA,
            BITCOIN_FACT_SCHEMA,
            EVM_FACT_SCHEMA,
            SOLANA_FACT_SCHEMA,
        ]
        .join("\n")
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        {
            self.execute(statement).await?;
        }
        Ok(())
    }

    /// Inserts one deterministic fact revision.
    ///
    /// Replaying the same record may create another physical row until
    /// `ClickHouse` merges, but all reader queries collapse by deterministic
    /// `fact_id`; logical truth is immediately idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`NormalizerError`] if `ClickHouse` rejects the row.
    pub async fn insert(&self, fact: &FixtureFact) -> Result<(), NormalizerError> {
        let row = StoredFactRow::from(fact);
        let mut body = serde_json::to_vec(&row).map_err(NormalizerError::InvalidPayload)?;
        body.push(b'\n');
        self.request(
            "INSERT INTO multichain.fixture_facts FORMAT JSONEachRow",
            body,
            &[],
        )
        .await
        .map(|_| ())
    }

    /// Inserts one native Bitcoin block batch into append-only fact tables.
    ///
    /// Each table is sent as one `JSONEachRow` batch. Empty child collections
    /// are skipped, and an error prevents the caller from acknowledging its
    /// broker offset.
    ///
    /// # Errors
    ///
    /// Returns the first serialization, transport, or `ClickHouse` failure.
    pub async fn insert_bitcoin_batch(
        &self,
        batch: &BitcoinFactBatch,
    ) -> Result<(), NormalizerError> {
        self.insert_json_rows("multichain.bitcoin_blocks", &batch.blocks)
            .await?;
        self.insert_json_rows("multichain.bitcoin_transactions", &batch.transactions)
            .await?;
        self.insert_json_rows("multichain.bitcoin_inputs", &batch.inputs)
            .await?;
        self.insert_json_rows("multichain.bitcoin_outputs", &batch.outputs)
            .await
    }

    /// Inserts one EVM native block batch into append-only fact tables.
    ///
    /// # Errors
    ///
    /// Returns the first serialization, transport, or `ClickHouse` failure.
    pub async fn insert_evm_batch(&self, batch: &EvmFactBatch) -> Result<(), NormalizerError> {
        self.insert_json_rows("multichain.evm_blocks", &batch.blocks)
            .await?;
        self.insert_json_rows("multichain.evm_transactions", &batch.transactions)
            .await?;
        self.insert_json_rows("multichain.evm_receipts", &batch.receipts)
            .await?;
        self.insert_json_rows("multichain.evm_logs", &batch.logs)
            .await
    }

    /// Inserts one fork-qualified Solana native fact batch.
    ///
    /// # Errors
    ///
    /// Returns the first serialization, transport, or `ClickHouse` failure.
    pub async fn insert_solana_batch(
        &self,
        batch: &SolanaFactBatch,
    ) -> Result<(), NormalizerError> {
        self.insert_json_rows("multichain.solana_transactions", &batch.transactions)
            .await?;
        self.insert_json_rows("multichain.solana_instructions", &batch.instructions)
            .await?;
        self.insert_json_rows("multichain.solana_logs", &batch.logs)
            .await?;
        self.insert_json_rows("multichain.solana_balance_changes", &batch.balance_changes)
            .await?;
        self.insert_json_rows(
            "multichain.solana_token_balance_changes",
            &batch.token_balance_changes,
        )
        .await?;
        self.insert_json_rows("multichain.solana_account_writes", &batch.account_writes)
            .await
    }

    /// Inserts one append-only Solana decoder attempt.
    ///
    /// # Errors
    ///
    /// Returns serialization, transport, or `ClickHouse` failure.
    pub async fn insert_solana_decoder_revision(
        &self,
        row: &DecodeRevisionFactRow,
    ) -> Result<(), NormalizerError> {
        self.insert_json_rows(
            "multichain.solana_decoder_revisions",
            std::slice::from_ref(row),
        )
        .await
    }

    /// Removes a named fixture synchronously for isolated acceptance tests.
    ///
    /// # Errors
    ///
    /// Returns [`NormalizerError`] if the synchronous mutation fails.
    pub async fn clear_fixture(&self, fixture_id: &str) -> Result<(), NormalizerError> {
        self.request(
            "ALTER TABLE multichain.fixture_facts DELETE WHERE fixture_id = {fixture_id:String}",
            Vec::new(),
            &[("param_fixture_id", fixture_id), ("mutations_sync", "2")],
        )
        .await
        .map(|_| ())
    }

    /// Counts distinct logical fact identities for a fixture.
    ///
    /// # Errors
    ///
    /// Returns [`NormalizerError`] if `ClickHouse` cannot answer or returns an
    /// invalid count.
    pub async fn logical_count(&self, fixture_id: &str) -> Result<u64, NormalizerError> {
        let bytes = self
            .request(
                "SELECT count() FROM (\
                 SELECT fact_id FROM multichain.fixture_facts \
                 WHERE fixture_id = {fixture_id:String} GROUP BY fact_id)",
                Vec::new(),
                &[("param_fixture_id", fixture_id)],
            )
            .await?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|error| NormalizerError::ClickHouse(error.to_string()))?;
        text.trim()
            .parse()
            .map_err(|error| NormalizerError::ClickHouse(format!("invalid count: {error}")))
    }

    /// Loads the current logical fixture projection.
    ///
    /// # Errors
    ///
    /// Returns [`NormalizerError`] on transport or row decoding failure.
    pub async fn fixture(
        &self,
        fixture_id: &str,
    ) -> Result<Option<FixtureFactView>, NormalizerError> {
        self.query_one(
            "WHERE fixture_id = {fixture_id:String}",
            &[("param_fixture_id", fixture_id)],
        )
        .await
    }

    /// Loads a fact by deterministic identity for lineage inspection.
    ///
    /// # Errors
    ///
    /// Returns [`NormalizerError`] on transport or row decoding failure.
    pub async fn fact(&self, fact_id: &str) -> Result<Option<FixtureFactView>, NormalizerError> {
        self.query_one(
            "WHERE fact_id = {fact_id:String}",
            &[("param_fact_id", fact_id)],
        )
        .await
    }

    /// Loads one current projection per fixture for a stream snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`NormalizerError`] on transport or row decoding failure.
    pub async fn current_facts(&self) -> Result<Vec<FixtureFactView>, NormalizerError> {
        let query = format!(
            "{} ORDER BY fixture_id, revision DESC LIMIT 1 BY fixture_id FORMAT JSONEachRow",
            select_columns()
        );
        let bytes = self.request(&query, Vec::new(), &[]).await?;
        json_each_row(&bytes)
    }

    async fn query_one(
        &self,
        predicate: &str,
        parameters: &[(&str, &str)],
    ) -> Result<Option<FixtureFactView>, NormalizerError> {
        let query = format!(
            "{} {predicate} ORDER BY revision DESC LIMIT 1 FORMAT JSONEachRow",
            select_columns()
        );
        let bytes = self.request(&query, Vec::new(), parameters).await?;
        let rows = json_each_row(&bytes)?;
        Ok(rows.into_iter().next())
    }

    async fn execute(&self, query: &str) -> Result<(), NormalizerError> {
        self.request(query, Vec::new(), &[]).await.map(|_| ())
    }

    async fn insert_json_rows<T: Serialize>(
        &self,
        table: &str,
        rows: &[T],
    ) -> Result<(), NormalizerError> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut body = Vec::new();
        for row in rows {
            serde_json::to_writer(&mut body, row).map_err(NormalizerError::InvalidPayload)?;
            body.push(b'\n');
        }
        self.request(
            &format!("INSERT INTO {table} FORMAT JSONEachRow"),
            body,
            &[],
        )
        .await
        .map(|_| ())
    }

    async fn request(
        &self,
        query: &str,
        body: Vec<u8>,
        parameters: &[(&str, &str)],
    ) -> Result<Vec<u8>, NormalizerError> {
        let mut request = self
            .authenticated(self.client.post(&self.endpoint))
            .query(&[("query", query)])
            .header(CONTENT_LENGTH, body.len());
        for (key, value) in parameters {
            request = request.query(&[(*key, *value)]);
        }
        let response = request.body(body).send().await.map_err(clickhouse_error)?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(clickhouse_error)?;
        if status != StatusCode::OK {
            return Err(NormalizerError::ClickHouse(format!(
                "HTTP {status}: {}",
                String::from_utf8_lossy(&bytes)
            )));
        }
        Ok(bytes.to_vec())
    }

    fn authenticated(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.username.as_ref() {
            Some(username) => request.basic_auth(username, self.password.as_ref()),
            None => request,
        }
    }
}

/// Deterministically decodes one committed fixture observation.
///
/// # Errors
///
/// Returns [`NormalizerError`] for missing context, invalid JSON, identity
/// length violations, or source-sequence disagreement.
pub fn normalize_fixture(committed: &CommittedObservation) -> Result<FixtureFact, NormalizerError> {
    let observation = committed
        .observation
        .as_ref()
        .ok_or(NormalizerError::MissingObservation)?;
    let payload: FixturePayload = serde_json::from_slice(&observation.payload)?;
    if observation.source_sequence != Some(payload.source_sequence) {
        return Err(NormalizerError::SourceSequenceMismatch {
            observation: observation.source_sequence,
            payload: payload.source_sequence,
        });
    }
    build_fact(committed, observation, payload)
}

/// Reports a non-contiguous collector interval instead of silently accepting it.
#[must_use]
pub const fn detect_collector_gap(
    previous_sequence: u64,
    next_sequence: u64,
) -> Option<IncompleteInterval> {
    let Some(expected) = previous_sequence.checked_add(1) else {
        return None;
    };
    if next_sequence <= expected {
        return None;
    }
    Some(IncompleteInterval {
        status: GapStatus::KnownIncomplete,
        missing_first: expected,
        missing_last: next_sequence - 1,
    })
}

/// Finds every missing interval in sorted or unsorted collector coverage.
///
/// Duplicate sequence observations are harmless. Adding the missing sequence
/// and rerunning this function closes the incident deterministically.
#[must_use]
pub fn detect_sequence_gaps(sequences: &[u64]) -> Vec<IncompleteInterval> {
    let mut ordered = sequences.to_vec();
    ordered.sort_unstable();
    ordered.dedup();
    ordered
        .windows(2)
        .filter_map(|pair| detect_collector_gap(pair[0], pair[1]))
        .collect()
}

/// Lightweight availability check used by fault tests and readiness.
pub async fn clickhouse_available(endpoint: &str) -> bool {
    ClickHouseFactStore {
        endpoint: endpoint.trim_end_matches('/').to_owned(),
        client: Client::new(),
        username: None,
        password: None,
    }
    .available()
    .await
}

fn build_fact(
    committed: &CommittedObservation,
    observation: &Observation,
    payload: FixturePayload,
) -> Result<FixtureFact, NormalizerError> {
    let observation_id: [u8; 32] =
        observation
            .observation_id
            .as_slice()
            .try_into()
            .map_err(|_| FactError::InvalidIdentityLength {
                field: "observation_id",
                expected: 32,
                actual: observation.observation_id.len(),
            })?;
    let source_session_id: [u8; 16] = observation
        .source_session_id
        .as_slice()
        .try_into()
        .map_err(|_| FactError::InvalidIdentityLength {
            field: "source_session_id",
            expected: 16,
            actual: observation.source_session_id.len(),
        })?;
    let fact_key = fixture_fact_key(&payload.fixture_id);
    let revision = 1;
    let fact_id = fixture_fact_id(fact_key, revision, observation_id);
    let normalized_payload =
        serde_json::to_vec(&payload).map_err(NormalizerError::InvalidPayload)?;
    let envelope = Fact {
        schema_version: 1,
        fact_id: fact_id.to_vec(),
        fact_key: fact_key.to_vec(),
        revision,
        chain: observation.chain.clone(),
        network: observation.network.clone(),
        fact_type: "fixture".to_owned(),
        valid_from_unix_ns: observation.observed_at_unix_ns,
        recorded_at_unix_ns: committed.durable_at_unix_ns,
        canonicality: "not_applicable".to_owned(),
        finality: "not_applicable".to_owned(),
        source_observation_ids: vec![observation_id.to_vec()],
        parser_version: FIXTURE_PARSER_VERSION.to_owned(),
        quality_flags: observation.quality_flags.clone(),
        payload: normalized_payload,
        supersedes_fact_id: None,
    };
    Ok(FixtureFact {
        envelope,
        payload,
        fact_id_hex: encode_hex(&fact_id),
        fact_key_hex: encode_hex(&fact_key),
        observation_id_hex: encode_hex(&observation_id),
        source_id: observation.source_id.clone(),
        source_session_id,
    })
}

#[derive(Debug, Deserialize, Serialize)]
struct StoredFactRow {
    fact_id: String,
    fact_key: String,
    revision: u64,
    fixture_id: String,
    value: String,
    source_sequence: u64,
    chain: String,
    network: String,
    canonicality: String,
    parser_version: String,
    source_id: String,
    source_session_id: String,
    observation_id: String,
    valid_from_unix_ns: i64,
    recorded_at_unix_ns: i64,
}

impl From<&FixtureFact> for StoredFactRow {
    fn from(fact: &FixtureFact) -> Self {
        Self {
            fact_id: fact.fact_id_hex.clone(),
            fact_key: fact.fact_key_hex.clone(),
            revision: fact.envelope.revision,
            fixture_id: fact.payload.fixture_id.clone(),
            value: fact.payload.value.clone(),
            source_sequence: fact.payload.source_sequence,
            chain: fact.envelope.chain.clone(),
            network: fact.envelope.network.clone(),
            canonicality: fact.envelope.canonicality.clone(),
            parser_version: fact.envelope.parser_version.clone(),
            source_id: fact.source_id.clone(),
            source_session_id: encode_hex(&fact.source_session_id),
            observation_id: fact.observation_id_hex.clone(),
            valid_from_unix_ns: fact.envelope.valid_from_unix_ns,
            recorded_at_unix_ns: fact.envelope.recorded_at_unix_ns,
        }
    }
}

impl From<StoredFactRow> for FixtureFactView {
    fn from(row: StoredFactRow) -> Self {
        Self {
            fact_id: row.fact_id,
            fact_key: row.fact_key,
            revision: row.revision,
            fixture_id: row.fixture_id,
            value: row.value,
            source_sequence: row.source_sequence,
            chain: row.chain,
            network: row.network,
            canonicality: row.canonicality,
            parser_version: row.parser_version,
            source_id: row.source_id,
            lineage: FixtureLineage {
                observation_id: row.observation_id,
                source_session_id: row.source_session_id,
            },
        }
    }
}

fn select_columns() -> &'static str {
    "SELECT fact_id, fact_key, revision, fixture_id, value, source_sequence, \
     chain, network, canonicality, parser_version, source_id, source_session_id, \
     observation_id, valid_from_unix_ns, recorded_at_unix_ns \
     FROM multichain.fixture_facts"
}

fn json_each_row(bytes: &[u8]) -> Result<Vec<FixtureFactView>, NormalizerError> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<StoredFactRow>(line)
                .map(Into::into)
                .map_err(NormalizerError::InvalidPayload)
        })
        .collect()
}

fn clickhouse_error(error: impl std::fmt::Display) -> NormalizerError {
    NormalizerError::ClickHouse(error.to_string())
}
