use platform_proto::observation::CommittedObservation;
use prost::Message as _;
use rdkafka::{
    ClientConfig, Message as _,
    consumer::{CommitMode, Consumer, StreamConsumer},
};
use storage_ports::RAW_BITCOIN_OBSERVATION_TOPIC;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint =
        std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://127.0.0.1:18123".to_owned());
    let username = std::env::var("CLICKHOUSE_USER").unwrap_or_else(|_| "multichain".to_owned());
    let password = std::env::var("CLICKHOUSE_PASSWORD")
        .unwrap_or_else(|_| "local-development-only".to_owned());
    let store = native_normalizer::ClickHouseFactStore::connect_with_credentials(
        &endpoint, &username, &password,
    )
    .await?;
    store.install_schema().await?;
    let brokers = std::env::var("KAFKA_BROKERS").unwrap_or_else(|_| "127.0.0.1:19092".to_owned());
    let group_id =
        std::env::var("KAFKA_GROUP_ID").unwrap_or_else(|_| "native-normalizer-v1".to_owned());
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", brokers)
        .set("group.id", group_id)
        .set("auto.offset.reset", "earliest")
        .set("enable.auto.commit", "false")
        .set("enable.auto.offset.store", "false")
        .create()?;
    consumer.subscribe(&[RAW_BITCOIN_OBSERVATION_TOPIC])?;
    println!("native-normalizer ready at {endpoint}");

    loop {
        tokio::select! {
            shutdown = tokio::signal::ctrl_c() => {
                shutdown?;
                break;
            }
            received = consumer.recv() => {
                let message = received?;
                let Some(payload) = message.payload() else {
                    eprintln!("native-normalizer skipped broker record without payload");
                    continue;
                };
                let committed = CommittedObservation::decode(payload)?;
                let fact = native_normalizer::normalize_fixture(&committed)?;
                store.insert(&fact).await?;
                consumer.commit_message(&message, CommitMode::Async)?;
            }
        }
    }
    Ok(())
}
