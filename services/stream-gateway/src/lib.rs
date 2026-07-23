#![doc = "Bounded Phase 0 WebSocket snapshot gateway."]

pub mod protocol;

use std::sync::Arc;

use axum::{
    Router,
    extract::{State, WebSocketUpgrade, ws::Message},
    response::Response,
    routing::get,
};
use native_normalizer::ClickHouseFactStore;

/// Builds the Phase 0 WebSocket router.
pub fn router(facts: Arc<ClickHouseFactStore>) -> Router {
    Router::new()
        .route("/v1/stream", get(upgrade))
        .with_state(facts)
}

async fn upgrade(
    State(facts): State<Arc<ClickHouseFactStore>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade.on_upgrade(move |mut socket| async move {
        let Ok(current) = facts.current_facts().await else {
            return;
        };
        let snapshot = protocol::StreamFrame::snapshot(current);
        let Ok(payload) = serde_json::to_string(&snapshot) else {
            return;
        };
        let _ = socket.send(Message::Text(payload.into())).await;
    })
}
