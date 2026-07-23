use std::{sync::Arc, time::Duration};

use archive_writer::ArchiveCoordinator;
use fact_envelope::FixtureFact;
use fixture_source::{PHASE0_FIXTURE_ID, phase0_observation, write_fixture_to_wal};
use futures_util::StreamExt as _;
use native_normalizer::{ClickHouseFactStore, normalize_fixture};
use platform_proto::observation::CommittedObservation;
use prost::Message as _;
use query_api::{AppState, DependencyReadiness, router};
use rdkafka::{
    ClientConfig, Message as _,
    consumer::{Consumer, StreamConsumer},
};
use serde_json::Value;
use storage_adapters::{PostgresCheckpointStore, RedpandaBroker, S3ArchiveConfig, S3RawArchive};
use storage_ports::RAW_BITCOIN_OBSERVATION_TOPIC;
use stream_gateway::router as stream_router;
use tempfile::tempdir;
use tokio::{net::TcpListener, time::timeout};
use tokio_tungstenite::connect_async;
use wal::ObservationWal as _;

const DATABASE_URL: &str =
    "postgres://multichain:local-development-only@127.0.0.1:15432/multichain";
const BROKERS: &str = "127.0.0.1:19092";
const CLICKHOUSE_URL: &str = "http://127.0.0.1:18123";

#[tokio::test]
async fn phase0_synthetic() {
    if !query_api::local_infrastructure_available().await {
        assert!(
            std::env::var_os("MULTICHAIN_REQUIRE_INFRA").is_none(),
            "Phase 0 infrastructure is required but unavailable"
        );
        eprintln!("skipping Phase 0 proof: local infrastructure is unavailable");
        return;
    }

    let (committed, checkpoints) = durable_fixture().await;
    let (facts, fact) = materialize_fixture(&committed).await;
    let readiness = prove_readiness(&fact, checkpoints).await;
    let api_listener = TcpListener::bind("127.0.0.1:0").await.expect("REST bind");
    let api_address = api_listener.local_addr().expect("REST address");
    let api_state = AppState::new(Arc::clone(&facts), readiness);
    tokio::spawn(async move {
        axum::serve(api_listener, router(api_state))
            .await
            .expect("REST server");
    });
    let stream_listener = TcpListener::bind("127.0.0.1:0").await.expect("WS bind");
    let stream_address = stream_listener.local_addr().expect("WS address");
    let stream_facts = Arc::clone(&facts);
    tokio::spawn(async move {
        axum::serve(stream_listener, stream_router(stream_facts))
            .await
            .expect("stream server");
    });

    prove_rest(api_address, &fact).await;
    prove_websocket(stream_address, &fact).await;
}

async fn durable_fixture() -> (CommittedObservation, PostgresCheckpointStore) {
    let directory = tempdir().expect("temporary WAL");
    let observation = phase0_observation().expect("deterministic fixture");
    let wal = write_fixture_to_wal(directory.path(), observation)
        .expect("fixture source writes durable WAL");
    let committed = wal
        .committed()
        .expect("read committed fixture")
        .collect::<Vec<_>>();
    assert_eq!(committed.len(), 1, "one committed WAL observation");
    let checkpoints = PostgresCheckpointStore::connect(DATABASE_URL, 4)
        .await
        .expect("connect PostgreSQL");
    checkpoints.install_schema().await.expect("control schema");
    let archive = S3RawArchive::new(
        S3ArchiveConfig {
            endpoint: "http://127.0.0.1:19000".to_owned(),
            bucket: "multichain-raw".to_owned(),
            region: "us-east-1".to_owned(),
            access_key_id: "multichain".to_owned(),
            secret_access_key: "local-development-only".to_owned(),
            allow_http: true,
        },
        checkpoints.pool().clone(),
    )
    .expect("configure archive");
    let consumer = broker_consumer("phase0-e2e");
    consumer
        .subscribe(&[RAW_BITCOIN_OBSERVATION_TOPIC])
        .expect("subscribe to observations");
    let outcome = ArchiveCoordinator::new(
        RedpandaBroker::new(BROKERS, Duration::from_secs(10)).expect("broker"),
        archive.clone(),
        checkpoints.clone(),
    )
    .publish_and_archive(committed.clone())
    .await
    .expect("broker and archive visibility");
    assert_eq!(
        receive_matching(&consumer, &committed[0]).await,
        committed[0],
        "one broker logical observation"
    );
    assert!(
        archive
            .replay_committed(outcome.manifest.manifest_hash())
            .await
            .expect("archive read")
            .is_some(),
        "one committed raw archive manifest"
    );
    (committed.into_iter().next().expect("fixture"), checkpoints)
}

async fn materialize_fixture(
    committed: &CommittedObservation,
) -> (Arc<ClickHouseFactStore>, FixtureFact) {
    let facts = Arc::new(
        ClickHouseFactStore::connect_with_credentials(
            CLICKHOUSE_URL,
            "multichain",
            "local-development-only",
        )
        .await
        .expect("ClickHouse"),
    );
    facts.install_schema().await.expect("fact schema");
    facts
        .clear_fixture(PHASE0_FIXTURE_ID)
        .await
        .expect("clean fixture");
    let fact = normalize_fixture(committed).expect("normalize exact observation");
    facts.insert(&fact).await.expect("insert fact");
    facts
        .insert(&fact)
        .await
        .expect("full replay is idempotent");
    assert_eq!(
        facts
            .logical_count(PHASE0_FIXTURE_ID)
            .await
            .expect("count facts"),
        1,
        "logical_fact_duplicates: 0"
    );
    (facts, fact)
}

async fn prove_readiness(
    fact: &FixtureFact,
    checkpoints: PostgresCheckpointStore,
) -> Arc<DependencyReadiness> {
    let readiness = Arc::new(DependencyReadiness::local(
        BROKERS,
        CLICKHOUSE_URL,
        DATABASE_URL,
        checkpoints,
        fact.source_id.clone(),
        fact.source_session_id,
    ));
    assert!(readiness.check().await.ready);
    let missing_coverage = DependencyReadiness::local(
        BROKERS,
        CLICKHOUSE_URL,
        DATABASE_URL,
        PostgresCheckpointStore::connect(DATABASE_URL, 1)
            .await
            .expect("secondary PostgreSQL probe"),
        "missing-source".to_owned(),
        [0x99; 16],
    );
    assert!(
        !missing_coverage.check().await.ready,
        "readiness must fail closed without durable checkpoints"
    );
    readiness
}

async fn prove_rest(api_address: std::net::SocketAddr, fact: &FixtureFact) {
    let client = reqwest::Client::new();
    let ready: Value = client
        .get(format!("http://{api_address}/health/ready"))
        .send()
        .await
        .expect("ready request")
        .error_for_status()
        .expect("ready response")
        .json()
        .await
        .expect("ready JSON");
    assert_eq!(ready["ready"], true);
    let fixture: Value = client
        .get(format!(
            "http://{api_address}/v1/fixtures/{PHASE0_FIXTURE_ID}"
        ))
        .send()
        .await
        .expect("fixture request")
        .error_for_status()
        .expect("fixture response")
        .json()
        .await
        .expect("fixture JSON");
    assert_eq!(fixture["fact_id"], fact.fact_id_hex);
    assert_eq!(
        fixture["lineage"]["observation_id"],
        fact.observation_id_hex
    );

    let lineage: Value = client
        .get(format!(
            "http://{api_address}/v1/lineage/facts/{}",
            fact.fact_id_hex
        ))
        .send()
        .await
        .expect("lineage request")
        .error_for_status()
        .expect("lineage response")
        .json()
        .await
        .expect("lineage JSON");
    assert_eq!(lineage["observation_id"], fact.observation_id_hex);
}

async fn prove_websocket(stream_address: std::net::SocketAddr, fact: &FixtureFact) {
    let (mut socket, _) = connect_async(format!("ws://{stream_address}/v1/stream"))
        .await
        .expect("WebSocket connect");
    let frame = timeout(Duration::from_secs(5), socket.next())
        .await
        .expect("snapshot timeout")
        .expect("snapshot frame")
        .expect("valid WebSocket frame");
    let snapshot: Value =
        serde_json::from_slice(&frame.into_data()).expect("snapshot JSON payload");
    assert_eq!(snapshot["type"], "snapshot");
    assert_eq!(snapshot["facts"][0]["fact_id"], fact.fact_id_hex);
}

fn broker_consumer(group: &str) -> StreamConsumer {
    ClientConfig::new()
        .set("bootstrap.servers", BROKERS)
        .set("group.id", format!("{group}-{}", std::process::id()))
        .set("auto.offset.reset", "earliest")
        .set("enable.auto.commit", "false")
        .create()
        .expect("create broker consumer")
}

async fn receive_matching(
    consumer: &StreamConsumer,
    expected: &CommittedObservation,
) -> CommittedObservation {
    let expected_id = &expected
        .observation
        .as_ref()
        .expect("expected observation")
        .observation_id;
    timeout(Duration::from_secs(15), async {
        loop {
            let message = consumer.recv().await.expect("broker receive");
            let Some(payload) = message.payload() else {
                continue;
            };
            let record = CommittedObservation::decode(payload).expect("broker protobuf");
            let id = &record
                .observation
                .as_ref()
                .expect("broker observation")
                .observation_id;
            if id == expected_id {
                return record;
            }
        }
    })
    .await
    .expect("matching broker record before timeout")
}
