use std::sync::Arc;

use native_normalizer::ClickHouseFactStore;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let clickhouse_url =
        std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://127.0.0.1:18123".to_owned());
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
    let listener = TcpListener::bind("127.0.0.1:8081").await?;
    axum::serve(listener, stream_gateway::router(facts)).await?;
    Ok(())
}
