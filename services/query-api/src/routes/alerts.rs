use alert_engine::{
    Completeness, DegradedPolicy, MempoolAlertEvaluation, MempoolAlertEvaluator,
    QuorumFeeBandSnapshot, QuorumVbytesAboveDefinition, SnapshotCause,
};
use axum::{
    Json,
    extract::rejection::JsonRejection,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use crate::ApiErrorBody;

const MAX_PREVIEW_REVISIONS: usize = 1_000;

#[derive(Debug, Deserialize)]
pub(crate) struct AlertPreviewRequest {
    definition: AlertDefinitionRequest,
    snapshots: Vec<FeeBandSnapshotRequest>,
}

#[derive(Debug, Deserialize)]
struct AlertDefinitionRequest {
    alert_id: String,
    min_fee_rate_sat_vb: u64,
    threshold_vbytes: u64,
    quorum_required: u16,
    for_evaluations: u16,
    cooldown_seconds: u64,
    degraded_policy: DegradedPolicyRequest,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DegradedPolicyRequest {
    Suppress,
    EvaluateHealthyQuorum,
}

impl From<DegradedPolicyRequest> for DegradedPolicy {
    fn from(value: DegradedPolicyRequest) -> Self {
        match value {
            DegradedPolicyRequest::Suppress => Self::Suppress,
            DegradedPolicyRequest::EvaluateHealthyQuorum => Self::EvaluateHealthyQuorum,
        }
    }
}

#[derive(Debug, Deserialize)]
struct FeeBandSnapshotRequest {
    network: String,
    revision: u64,
    observed_at_unix_seconds: i64,
    min_fee_rate_sat_vb: u64,
    vbytes: u64,
    quorum_required: u16,
    eligible_sources: Vec<String>,
    unavailable_sources: Vec<String>,
    completeness: CompletenessRequest,
    cause: SnapshotCauseRequest,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CompletenessRequest {
    Complete,
    KnownIncomplete,
    Recovered,
}

impl From<CompletenessRequest> for Completeness {
    fn from(value: CompletenessRequest) -> Self {
        match value {
            CompletenessRequest::Complete => Self::Complete,
            CompletenessRequest::KnownIncomplete => Self::KnownIncomplete,
            CompletenessRequest::Recovered => Self::Recovered,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SnapshotCauseRequest {
    Observed,
    Recovered,
    Correction,
}

impl From<SnapshotCauseRequest> for SnapshotCause {
    fn from(value: SnapshotCauseRequest) -> Self {
        match value {
            SnapshotCauseRequest::Observed => Self::Observed,
            SnapshotCauseRequest::Recovered => Self::Recovered,
            SnapshotCauseRequest::Correction => Self::Correction,
        }
    }
}

#[derive(Debug, Serialize)]
struct AlertPreviewResponse {
    evaluations: Vec<MempoolAlertEvaluation>,
    final_active: bool,
}

pub(crate) async fn preview(payload: Result<Json<AlertPreviewRequest>, JsonRejection>) -> Response {
    let Ok(Json(request)) = payload else {
        return invalid_preview();
    };
    let Some(response) = evaluate_preview(request) else {
        return invalid_preview();
    };
    Json(response).into_response()
}

fn evaluate_preview(request: AlertPreviewRequest) -> Option<AlertPreviewResponse> {
    if request.snapshots.is_empty() || request.snapshots.len() > MAX_PREVIEW_REVISIONS {
        return None;
    }
    let definition = QuorumVbytesAboveDefinition::new(
        request.definition.alert_id,
        request.definition.min_fee_rate_sat_vb,
        request.definition.threshold_vbytes,
        request.definition.quorum_required,
        request.definition.for_evaluations,
        request.definition.cooldown_seconds,
        request.definition.degraded_policy.into(),
    )
    .ok()?;
    let mut evaluator = MempoolAlertEvaluator::new(definition);
    let mut evaluations = Vec::with_capacity(request.snapshots.len());
    for snapshot in request.snapshots {
        let snapshot = QuorumFeeBandSnapshot::new(
            snapshot.network,
            snapshot.revision,
            snapshot.observed_at_unix_seconds,
            snapshot.min_fee_rate_sat_vb,
            snapshot.vbytes,
            snapshot.quorum_required,
            snapshot.eligible_sources,
            snapshot.unavailable_sources,
            snapshot.completeness.into(),
            snapshot.cause.into(),
        )
        .ok()?;
        evaluations.push(evaluator.evaluate(&snapshot).ok()?);
    }
    let final_active = evaluations
        .last()
        .is_some_and(|evaluation| evaluation.active);
    Some(AlertPreviewResponse {
        evaluations,
        final_active,
    })
}

fn invalid_preview() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiErrorBody {
            error: "invalid_alert_preview",
        }),
    )
        .into_response()
}
