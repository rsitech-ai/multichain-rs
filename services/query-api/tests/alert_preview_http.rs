use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use query_api::alert_preview_router;
use serde_json::{Value, json};
use tower::ServiceExt as _;

#[tokio::test]
async fn preview_evaluates_ordered_revisions_without_external_storage() {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/alerts/preview")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "definition": {
                    "alert_id": "btc-mainnet-high-fee-pressure",
                    "min_fee_rate_sat_vb": 25,
                    "threshold_vbytes": 1_000_000,
                    "quorum_required": 2,
                    "for_evaluations": 2,
                    "cooldown_seconds": 60,
                    "degraded_policy": "suppress"
                },
                "snapshots": [
                    {
                        "network": "mainnet",
                        "revision": 1,
                        "observed_at_unix_seconds": 1_000,
                        "min_fee_rate_sat_vb": 25,
                        "vbytes": 1_100_000,
                        "quorum_required": 2,
                        "eligible_sources": ["observer-b", "observer-a"],
                        "unavailable_sources": [],
                        "completeness": "complete",
                        "cause": "observed"
                    },
                    {
                        "network": "mainnet",
                        "revision": 2,
                        "observed_at_unix_seconds": 1_010,
                        "min_fee_rate_sat_vb": 25,
                        "vbytes": 1_200_000,
                        "quorum_required": 2,
                        "eligible_sources": ["observer-a", "observer-b"],
                        "unavailable_sources": [],
                        "completeness": "complete",
                        "cause": "observed"
                    }
                ]
            })
            .to_string(),
        ))
        .expect("request");

    let response = alert_preview_router()
        .oneshot(request)
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 256 * 1024)
        .await
        .expect("body");
    let body: Value = serde_json::from_slice(&body).expect("json");

    assert_eq!(body["evaluations"][0]["transition"], "pending");
    assert_eq!(body["evaluations"][1]["transition"], "triggered");
    assert_eq!(body["evaluations"][1]["delivery_required"], true);
    assert_eq!(
        body["evaluations"][1]["contributing_sources"][0],
        "observer-a"
    );
    assert_eq!(body["final_active"], true);
}

#[tokio::test]
async fn preview_rejects_unknown_enums_and_conflicting_revisions() {
    let malformed_enum = json!({
        "definition": {
            "alert_id": "btc-mainnet-high-fee-pressure",
            "min_fee_rate_sat_vb": 25,
            "threshold_vbytes": 1_000_000,
            "quorum_required": 2,
            "for_evaluations": 1,
            "cooldown_seconds": 0,
            "degraded_policy": "invent_global_truth"
        },
        "snapshots": []
    });
    assert_invalid(malformed_enum).await;

    let conflicting_revision = json!({
        "definition": {
            "alert_id": "btc-mainnet-high-fee-pressure",
            "min_fee_rate_sat_vb": 25,
            "threshold_vbytes": 1_000_000,
            "quorum_required": 2,
            "for_evaluations": 1,
            "cooldown_seconds": 0,
            "degraded_policy": "suppress"
        },
        "snapshots": [
            {
                "network": "mainnet",
                "revision": 1,
                "observed_at_unix_seconds": 1_000,
                "min_fee_rate_sat_vb": 25,
                "vbytes": 1_100_000,
                "quorum_required": 2,
                "eligible_sources": ["observer-a", "observer-b"],
                "unavailable_sources": [],
                "completeness": "complete",
                "cause": "observed"
            },
            {
                "network": "mainnet",
                "revision": 1,
                "observed_at_unix_seconds": 1_000,
                "min_fee_rate_sat_vb": 25,
                "vbytes": 900_000,
                "quorum_required": 2,
                "eligible_sources": ["observer-a", "observer-b"],
                "unavailable_sources": [],
                "completeness": "complete",
                "cause": "correction"
            }
        ]
    });
    assert_invalid(conflicting_revision).await;
}

async fn assert_invalid(payload: Value) {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/alerts/preview")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(payload.to_string()))
        .expect("request");
    let response = alert_preview_router()
        .oneshot(request)
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), 16 * 1024)
        .await
        .expect("body");
    assert_eq!(
        serde_json::from_slice::<Value>(&body).expect("json"),
        json!({"error": "invalid_alert_preview"})
    );
}
