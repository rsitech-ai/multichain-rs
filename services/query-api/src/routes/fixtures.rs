use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use fact_envelope::{FixtureFactView, FixtureLineage};

use crate::{ApiErrorBody, AppState};

pub(crate) async fn fixture(
    State(state): State<AppState>,
    Path(fixture_id): Path<String>,
) -> Response {
    match state.facts.fixture(&fixture_id).await {
        Ok(Some(fact)) => Json(fact).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorBody {
                error: "fixture_not_found",
            }),
        )
            .into_response(),
        Err(_) => upstream_error(),
    }
}

pub(crate) async fn lineage(
    State(state): State<AppState>,
    Path(fact_id): Path<String>,
) -> Response {
    match state.facts.fact(&fact_id).await {
        Ok(Some(FixtureFactView { lineage, .. })) => {
            Json::<FixtureLineage>(lineage).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorBody {
                error: "fact_not_found",
            }),
        )
            .into_response(),
        Err(_) => upstream_error(),
    }
}

fn upstream_error() -> Response {
    (
        StatusCode::BAD_GATEWAY,
        Json(ApiErrorBody {
            error: "fact_store_unavailable",
        }),
    )
        .into_response()
}
