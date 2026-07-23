use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::AppState;

/// Liveness response independent of downstream dependency state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct HealthResponse {
    /// Stable service identity used by probes and operators.
    pub component: &'static str,
    /// The process is serving requests.
    pub live: bool,
}

/// Fail-closed readiness response with every required boundary named.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ReadinessResponse {
    /// Stable service identity used to reject cross-service probe collisions.
    pub component: &'static str,
    /// All required dependencies and durable checkpoints are available.
    pub ready: bool,
    /// Redpanda/Kafka TCP endpoint is reachable.
    pub broker: ProbeStatus,
    /// `ClickHouse` answers an authenticated query.
    pub clickhouse: ProbeStatus,
    /// `PostgreSQL` answers a control-plane query.
    pub postgres: ProbeStatus,
    /// Durable broker coverage exists for the configured source session.
    pub broker_checkpoint: ProbeStatus,
    /// Durable archive coverage exists for the configured source session.
    pub archive_checkpoint: ProbeStatus,
}

/// Reader-facing state of one required readiness dependency.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeStatus {
    /// The boundary is currently proven available.
    Ready,
    /// The boundary is absent, inaccessible, or invalid.
    Unavailable,
}

impl From<bool> for ProbeStatus {
    fn from(value: bool) -> Self {
        if value {
            Self::Ready
        } else {
            Self::Unavailable
        }
    }
}

pub(crate) async fn live() -> Json<HealthResponse> {
    Json(HealthResponse {
        component: "query-api",
        live: true,
    })
}

pub(crate) async fn ready(State(state): State<AppState>) -> Response {
    let snapshot = state.readiness.check().await;
    let status = if snapshot.ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(snapshot)).into_response()
}
