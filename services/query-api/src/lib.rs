#![doc = "Revision-aware REST serving layer with fail-closed dependency readiness."]

mod routes;

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    routing::{get, post},
};
use native_normalizer::ClickHouseFactStore;
use serde::Serialize;
use sqlx::Row as _;
use storage_adapters::PostgresCheckpointStore;
use storage_ports::CheckpointKind;
use tokio::{net::TcpStream, time::timeout};

pub use routes::health::{HealthResponse, ProbeStatus, ReadinessResponse};
pub use routes::replay::BitcoinRpcBackfillSource;

/// Shared REST application state.
#[derive(Clone)]
pub struct AppState {
    pub(crate) facts: Arc<ClickHouseFactStore>,
    pub(crate) readiness: Arc<DependencyReadiness>,
}

impl AppState {
    /// Binds immutable service dependencies.
    #[must_use]
    pub const fn new(facts: Arc<ClickHouseFactStore>, readiness: Arc<DependencyReadiness>) -> Self {
        Self { facts, readiness }
    }
}

/// Required dependency coordinates and durable source coverage.
#[derive(Clone, Debug)]
pub struct DependencyReadiness {
    broker_address: String,
    clickhouse_url: String,
    clickhouse_username: String,
    clickhouse_password: String,
    database_url: String,
    checkpoints: PostgresCheckpointStore,
    source_id: String,
    source_session_id: [u8; 16],
}

impl DependencyReadiness {
    /// Creates the local Phase 0 dependency probe.
    #[must_use]
    pub fn local(
        broker_address: &str,
        clickhouse_url: &str,
        database_url: &str,
        checkpoints: PostgresCheckpointStore,
        source_id: String,
        source_session_id: [u8; 16],
    ) -> Self {
        Self {
            broker_address: broker_address.to_owned(),
            clickhouse_url: clickhouse_url.trim_end_matches('/').to_owned(),
            clickhouse_username: "multichain".to_owned(),
            clickhouse_password: "local-development-only".to_owned(),
            database_url: database_url.to_owned(),
            checkpoints,
            source_id,
            source_session_id,
        }
    }

    /// Overrides local `ClickHouse` credentials for deployed environments.
    #[must_use]
    pub fn with_clickhouse_credentials(mut self, username: &str, password: &str) -> Self {
        username.clone_into(&mut self.clickhouse_username);
        password.clone_into(&mut self.clickhouse_password);
        self
    }

    /// Evaluates every required dependency and durable checkpoint now.
    pub async fn check(&self) -> ReadinessResponse {
        let broker = tcp_available(&self.broker_address).await;
        let clickhouse = reqwest_query(
            &self.clickhouse_url,
            &self.clickhouse_username,
            &self.clickhouse_password,
        )
        .await;
        let postgres = sqlx::query("SELECT 1 AS ready")
            .fetch_one(self.checkpoints.pool())
            .await
            .is_ok_and(|row| row.try_get::<i32, _>("ready").is_ok_and(|value| value == 1));
        let broker_checkpoint = self
            .checkpoints
            .load(
                CheckpointKind::Broker,
                &self.source_id,
                self.source_session_id,
            )
            .await
            .is_ok_and(|checkpoint| checkpoint.is_some());
        let archive_checkpoint = self
            .checkpoints
            .load(
                CheckpointKind::Archive,
                &self.source_id,
                self.source_session_id,
            )
            .await
            .is_ok_and(|checkpoint| checkpoint.is_some());
        ReadinessResponse {
            component: "query-api",
            ready: broker && clickhouse && postgres && broker_checkpoint && archive_checkpoint,
            broker: broker.into(),
            clickhouse: clickhouse.into(),
            postgres: postgres.into(),
            broker_checkpoint: broker_checkpoint.into(),
            archive_checkpoint: archive_checkpoint.into(),
        }
    }

    /// Returns the configured database URL for build/debug metadata without
    /// exposing credentials.
    #[must_use]
    pub fn database_scheme(&self) -> &str {
        self.database_url
            .split_once("://")
            .map_or("unknown", |(scheme, _)| scheme)
    }
}

/// Builds the complete REST router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health/live", get(routes::health::live))
        .route("/health/ready", get(routes::health::ready))
        .route("/v1/alerts/preview", post(routes::alerts::preview))
        .route("/v1/fixtures/{fixture_id}", get(routes::fixtures::fixture))
        .route(
            "/v1/lineage/facts/{fact_id}",
            get(routes::fixtures::lineage),
        )
        .route(
            "/v1/replay/bitcoin/validate",
            post(routes::replay::validate),
        )
        .with_state(state)
}

/// Builds the storage-independent alert preview boundary for focused tests and
/// local tooling.
pub fn alert_preview_router() -> Router {
    Router::new().route("/v1/alerts/preview", post(routes::alerts::preview))
}

/// Checks the four local service ports before deciding whether to run a live
/// acceptance test.
pub async fn local_infrastructure_available() -> bool {
    for address in [
        "127.0.0.1:19092",
        "127.0.0.1:18123",
        "127.0.0.1:15432",
        "127.0.0.1:19000",
    ] {
        if !tcp_available(address).await {
            return false;
        }
    }
    true
}

async fn tcp_available(address: &str) -> bool {
    matches!(
        timeout(Duration::from_millis(400), TcpStream::connect(address)).await,
        Ok(Ok(_))
    )
}

async fn reqwest_query(endpoint: &str, username: &str, password: &str) -> bool {
    reqwest::Client::new()
        .post(endpoint)
        .basic_auth(username, Some(password))
        .query(&[("query", "SELECT 1")])
        .header(reqwest::header::CONTENT_LENGTH, 0)
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}

#[derive(Debug, Serialize)]
struct ApiErrorBody {
    error: &'static str,
}
