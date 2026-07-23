use std::sync::Arc;

use native_normalizer::ClickHouseFactStore;
use query_api::{AppState, DependencyReadiness, router};
use storage_adapters::PostgresCheckpointStore;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://multichain:local-development-only@127.0.0.1:15432/multichain".to_owned()
    });
    let clickhouse_url =
        std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://127.0.0.1:18123".to_owned());
    let brokers = std::env::var("KAFKA_BROKERS").unwrap_or_else(|_| "127.0.0.1:19092".to_owned());
    let clickhouse_user =
        std::env::var("CLICKHOUSE_USER").unwrap_or_else(|_| "multichain".to_owned());
    let clickhouse_password = std::env::var("CLICKHOUSE_PASSWORD")
        .unwrap_or_else(|_| "local-development-only".to_owned());
    let facts = Arc::new(
        ClickHouseFactStore::connect_with_credentials(
            &clickhouse_url,
            &clickhouse_user,
            &clickhouse_password,
        )
        .await?,
    );
    let checkpoints = PostgresCheckpointStore::connect(&database_url, 4).await?;
    let readiness = Arc::new(
        DependencyReadiness::local(
            &brokers,
            &clickhouse_url,
            &database_url,
            checkpoints,
            "phase0-fixture-source".to_owned(),
            [0x50; 16],
        )
        .with_clickhouse_credentials(&clickhouse_user, &clickhouse_password),
    );
    let bind_address =
        std::env::var("QUERY_API_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_owned());
    let listener = TcpListener::bind(bind_address).await?;
    axum::serve(listener, router(AppState::new(facts, readiness))).await?;
    Ok(())
}
