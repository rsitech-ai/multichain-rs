use std::time::Duration;

use fixture_source::{PHASE0_FIXTURE_ID, phase0_observation, write_fixture_to_wal};
use native_normalizer::{
    ClickHouseFactStore, GapStatus, detect_collector_gap, detect_sequence_gaps, normalize_fixture,
};
use platform_proto::observation::CommittedObservation;
use prost::Message as _;
use rdkafka::{
    ClientConfig, Message as _,
    consumer::{Consumer, StreamConsumer},
};
use storage_adapters::RedpandaBroker;
use storage_ports::{BrokerPublisher as _, RAW_BITCOIN_OBSERVATION_TOPIC};
use tempfile::tempdir;
use tokio::time::timeout;
use wal::ObservationWal as _;

const CLICKHOUSE_URL: &str = "http://127.0.0.1:18123";
const BROKERS: &str = "127.0.0.1:19092";

#[tokio::test]
async fn phase0_restart_replay() {
    if !query_api::local_infrastructure_available().await {
        assert!(
            std::env::var_os("MULTICHAIN_REQUIRE_INFRA").is_none(),
            "Phase 0 infrastructure is required but unavailable"
        );
        eprintln!("skipping Phase 0 fault proof: ClickHouse is unavailable");
        return;
    }

    let directory = tempdir().expect("temporary WAL");
    let wal = write_fixture_to_wal(
        directory.path(),
        phase0_observation().expect("deterministic fixture"),
    )
    .expect("durable fixture");
    let committed = wal
        .committed()
        .expect("committed records")
        .next()
        .expect("fixture record");

    let group_id = format!("phase0-restart-{}", std::process::id());
    let first_consumer = broker_consumer(&group_id);
    first_consumer
        .subscribe(&[RAW_BITCOIN_OBSERVATION_TOPIC])
        .expect("first normalizer subscribes");
    RedpandaBroker::new(BROKERS, Duration::from_secs(10))
        .expect("broker publisher")
        .publish(
            RAW_BITCOIN_OBSERVATION_TOPIC,
            std::slice::from_ref(&committed),
        )
        .await
        .expect("publish fixture");
    let first_receipt = receive_matching(&first_consumer, &committed).await;
    let prepared_before_crash =
        normalize_fixture(&first_receipt).expect("broker receipt prepares deterministic fact");
    drop(prepared_before_crash);
    drop(first_consumer);

    let restarted_consumer = broker_consumer(&group_id);
    restarted_consumer
        .subscribe(&[RAW_BITCOIN_OBSERVATION_TOPIC])
        .expect("restarted normalizer subscribes");
    let replayed_receipt = receive_matching(&restarted_consumer, &committed).await;
    let prepared_after_restart =
        normalize_fixture(&replayed_receipt).expect("restart replays the same broker record");

    let store = ClickHouseFactStore::connect_with_credentials(
        CLICKHOUSE_URL,
        "multichain",
        "local-development-only",
    )
    .await
    .expect("ClickHouse");
    store.install_schema().await.expect("schema");
    store
        .clear_fixture(PHASE0_FIXTURE_ID)
        .await
        .expect("clean fixture");
    store.insert(&prepared_after_restart).await.expect("commit");
    store.insert(&prepared_after_restart).await.expect("replay");
    assert_eq!(
        store
            .logical_count(PHASE0_FIXTURE_ID)
            .await
            .expect("logical count"),
        1
    );

    let incident = detect_collector_gap(42, 44).expect("gap is explicit");
    assert_eq!(incident.status, GapStatus::KnownIncomplete);
    assert_eq!(incident.missing_first, 43);
    assert_eq!(incident.missing_last, 43);
    assert!(detect_collector_gap(42, 43).is_none());
    assert_eq!(detect_sequence_gaps(&[42, 44]), vec![incident]);
    assert!(
        detect_sequence_gaps(&[44, 42, 43]).is_empty(),
        "archive_manifest_gaps: 0 after the explicit repair fixture"
    );
}

fn broker_consumer(group_id: &str) -> StreamConsumer {
    ClientConfig::new()
        .set("bootstrap.servers", BROKERS)
        .set("group.id", group_id)
        .set("auto.offset.reset", "earliest")
        .set("enable.auto.commit", "false")
        .set("enable.auto.offset.store", "false")
        .create()
        .expect("broker consumer")
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
