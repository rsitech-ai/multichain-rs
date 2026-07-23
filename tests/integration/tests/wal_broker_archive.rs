use std::{collections::HashMap, sync::Arc, time::Duration};

use archive_format::decode_archive;
use archive_writer::ArchiveCoordinator;
use observation_envelope::{CollectorSequence, ObservationBuilder, SourceSessionId};
use platform_proto::observation::CommittedObservation;
use prost::Message as _;
use rdkafka::{
    ClientConfig, Message as _,
    consumer::{Consumer, StreamConsumer},
};
use storage_adapters::{
    MemoryBroker, MemoryCheckpointStore, MemoryRawArchive, PostgresCheckpointStore, RedpandaBroker,
    S3ArchiveConfig, S3RawArchive,
};
use storage_ports::{
    CheckpointKind, RAW_BITCOIN_OBSERVATION_TOPIC, SealedWalSegment, ensure_reclaimable,
};
use tempfile::tempdir;
use test_fixtures::clock::FakeClock;
use tokio::{net::TcpStream, time::timeout};
use wal::{FileWal, ObservationWal, UnframedObservation, WalConfig};

const DATABASE_URL: &str =
    "postgres://multichain:local-development-only@127.0.0.1:15432/multichain";
const BROKERS: &str = "127.0.0.1:19092";

#[tokio::test]
async fn wal_to_broker_to_verified_archive_round_trip() {
    if !infra_available().await {
        assert!(
            std::env::var_os("MULTICHAIN_REQUIRE_INFRA").is_none(),
            "Task 4 infrastructure is required but unavailable"
        );
        eprintln!("skipping live Task 4 proof: local infrastructure is unavailable");
        return;
    }

    let directory = tempdir().expect("temporary WAL directory");
    let wal_path = directory.path().join("observer.wal");
    let session = SourceSessionId::try_from([0x43_u8; 16].as_slice()).expect("session");
    let clock = Arc::new(FakeClock::new(1_784_808_123_000_000_000, 10));
    let config = WalConfig::new(session, 64 * 1024, Duration::from_millis(10));
    let (mut wal, report) = FileWal::open(&wal_path, config, clock).expect("create WAL");
    assert!(report.incidents.is_empty());
    wal.append(UnframedObservation::new(observation(session, 0, b"tx-a")))
        .expect("append first");
    wal.append(UnframedObservation::new(observation(session, 1, b"tx-b")))
        .expect("append second");
    wal.group_commit().expect("commit WAL");
    wal.seal().expect("seal WAL");
    let committed = wal
        .committed()
        .expect("read committed WAL")
        .collect::<Vec<_>>();

    let checkpoint_store = PostgresCheckpointStore::connect(DATABASE_URL, 4)
        .await
        .expect("connect PostgreSQL");
    checkpoint_store
        .install_schema()
        .await
        .expect("install control schema");
    let consumer = consumer().expect("create consumer");
    consumer
        .subscribe(&[RAW_BITCOIN_OBSERVATION_TOPIC])
        .expect("subscribe before publish");
    let archive = S3RawArchive::new(
        S3ArchiveConfig {
            endpoint: "http://127.0.0.1:19000".to_owned(),
            bucket: "multichain-raw".to_owned(),
            region: "us-east-1".to_owned(),
            access_key_id: "multichain".to_owned(),
            secret_access_key: "local-development-only".to_owned(),
            allow_http: true,
        },
        checkpoint_store.pool().clone(),
    )
    .expect("configure MinIO archive");
    let coordinator = ArchiveCoordinator::new(
        RedpandaBroker::new(BROKERS, Duration::from_secs(10)).expect("configure producer"),
        archive.clone(),
        checkpoint_store.clone(),
    );

    let outcome = coordinator
        .publish_and_archive(committed.clone())
        .await
        .expect("publish and archive");
    let broker_records = receive_expected(&consumer, &committed).await;
    assert_eq!(broker_records, committed);

    let archive_bytes = archive
        .replay_committed(outcome.manifest.manifest_hash())
        .await
        .expect("read committed archive")
        .expect("manifest is replayable");
    assert_eq!(
        decode_archive(&archive_bytes).expect("decode archive"),
        committed
    );

    let broker_checkpoint = checkpoint_store
        .load(
            CheckpointKind::Broker,
            "btc-observer-integration",
            [0x43; 16],
        )
        .await
        .expect("load broker checkpoint");
    let archive_checkpoint = checkpoint_store
        .load(
            CheckpointKind::Archive,
            "btc-observer-integration",
            [0x43; 16],
        )
        .await
        .expect("load archive checkpoint");
    ensure_reclaimable(
        &SealedWalSegment::new([0x43; 16], 1),
        broker_checkpoint.as_ref(),
        archive_checkpoint.as_ref(),
    )
    .expect("both durable checkpoints cover sealed WAL");

    prove_withheld_archive_retains_wal(&committed, &wal_path).await;
}

fn observation(
    session: SourceSessionId,
    sequence: u64,
    payload: &[u8],
) -> platform_proto::observation::Observation {
    ObservationBuilder::new()
        .source_id("btc-observer-integration")
        .source_session_id(session)
        .collector_sequence(CollectorSequence::new(sequence))
        .chain("bitcoin")
        .network("mainnet")
        .channel("rawtx")
        .source_message_type("rawtx")
        .observed_at_unix_ns(
            1_784_808_000_000_000_000 + i64::try_from(sequence).expect("test sequence fits i64"),
        )
        .observed_at_monotonic_ns(10_000 + sequence)
        .payload(payload)
        .build()
        .expect("valid observation")
}

async fn infra_available() -> bool {
    for address in ["127.0.0.1:19092", "127.0.0.1:19000", "127.0.0.1:15432"] {
        if !matches!(
            timeout(Duration::from_millis(250), TcpStream::connect(address)).await,
            Ok(Ok(_))
        ) {
            return false;
        }
    }
    true
}

fn consumer() -> Result<StreamConsumer, rdkafka::error::KafkaError> {
    ClientConfig::new()
        .set("bootstrap.servers", BROKERS)
        .set(
            "group.id",
            format!("task4-integration-{}", std::process::id()),
        )
        .set("auto.offset.reset", "earliest")
        .set("enable.auto.commit", "false")
        .create()
}

async fn receive_expected(
    consumer: &StreamConsumer,
    expected: &[CommittedObservation],
) -> Vec<CommittedObservation> {
    let expected_by_id = expected
        .iter()
        .map(|record| {
            let observation = record.observation.as_ref().expect("observation");
            (observation.observation_id.clone(), record.clone())
        })
        .collect::<HashMap<_, _>>();
    let mut received = HashMap::new();

    timeout(Duration::from_secs(15), async {
        while received.len() < expected_by_id.len() {
            let message = consumer.recv().await.expect("consume broker record");
            let Some(payload) = message.payload() else {
                continue;
            };
            let record = CommittedObservation::decode(payload).expect("decode broker record");
            let observation = record.observation.as_ref().expect("observation");
            if expected_by_id.contains_key(&observation.observation_id) {
                received.insert(observation.observation_id.clone(), record);
            }
        }
    })
    .await
    .expect("receive expected broker records before timeout");

    expected
        .iter()
        .map(|record| {
            let id = &record
                .observation
                .as_ref()
                .expect("observation")
                .observation_id;
            received.remove(id).expect("expected broker record")
        })
        .collect()
}

async fn prove_withheld_archive_retains_wal(
    records: &[CommittedObservation],
    wal_path: &std::path::Path,
) {
    let checkpoints = MemoryCheckpointStore::default();
    let coordinator = ArchiveCoordinator::new(
        MemoryBroker::default(),
        MemoryRawArchive::withhold_manifest_commits(),
        checkpoints.clone(),
    );
    assert!(
        coordinator
            .publish_and_archive(records.to_vec())
            .await
            .is_err()
    );
    let broker = checkpoints
        .load(
            CheckpointKind::Broker,
            "btc-observer-integration",
            [0x43; 16],
        )
        .await
        .expect("load broker checkpoint");
    let archive = checkpoints
        .load(
            CheckpointKind::Archive,
            "btc-observer-integration",
            [0x43; 16],
        )
        .await
        .expect("load archive checkpoint");
    assert!(
        ensure_reclaimable(
            &SealedWalSegment::new([0x43; 16], 1),
            broker.as_ref(),
            archive.as_ref(),
        )
        .is_err()
    );
    assert!(
        wal_path.exists(),
        "WAL must remain while archive is uncovered"
    );
}
