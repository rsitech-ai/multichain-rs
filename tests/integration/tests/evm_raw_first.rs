use std::{path::Path, sync::Arc, time::Duration};

use bsc_connector::RecordedBscHeads;
use ethereum_consensus_connector::RecordedConsensusCheckpoint;
use ethereum_reth_connector::{RecordedRethNotification, RethTransition};
use observation_envelope::Clock;
use platform_proto::observation::CommittedObservation;
use prost::Message as _;
use source_capture::{CaptureSession, DurableSourceCapture, RawSourceMessage, SourceIdentity};
use storage_adapters::MemoryBroker;
use storage_ports::{
    BrokerPublisher as _, RAW_BSC_OBSERVATION_TOPIC, RAW_ETHEREUM_OBSERVATION_TOPIC,
};
use tempfile::TempDir;
use wal::{FileWal, WalConfig};

#[tokio::test]
async fn evm_sources_commit_exact_bytes_before_interpretation_and_route_by_chain() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let broker = MemoryBroker::default();
    let mut captures = EvmCaptures::new(&directory);

    verify_reth_capture(&broker, &mut captures.reth).await;
    verify_consensus_capture(&broker, &mut captures.consensus).await;
    verify_bsc_capture(&broker, &mut captures.bsc).await;
    verify_malformed_capture_and_routing(&broker, &mut captures.reth).await;
}

async fn verify_reth_capture(broker: &MemoryBroker, reth: &mut DurableSourceCapture<FileWal>) {
    let reth_bytes = fixture("ethereum/reth-reorg.json");
    let reth_committed = reth
        .capture(
            RawSourceMessage::new("reth_exex", "chain_reorged", &reth_bytes)
                .expect("Reth message")
                .with_source_sequence(2),
        )
        .expect("durable Reth observation");
    publish_twice(broker, RAW_ETHEREUM_OBSERVATION_TOPIC, &reth_committed).await;
    let reth_observation = observation(&reth_committed);
    assert_eq!(reth_observation.payload, reth_bytes);
    assert_eq!(reth_observation.chain, "ethereum");
    assert_eq!(reth_observation.channel, "reth_exex");
    let parsed_reth =
        RecordedRethNotification::from_json(&reth_observation.source_id, &reth_observation.payload)
            .expect("interpret committed Reth payload");
    assert!(matches!(
        parsed_reth.transition(),
        RethTransition::Reorged { .. }
    ));
}

async fn verify_consensus_capture(
    broker: &MemoryBroker,
    consensus: &mut DurableSourceCapture<FileWal>,
) {
    let consensus_bytes = fixture("ethereum/consensus-checkpoint.json");
    let consensus_committed = consensus
        .capture(
            RawSourceMessage::new("beacon_api", "execution_checkpoint", &consensus_bytes)
                .expect("consensus message")
                .with_source_sequence(100),
        )
        .expect("durable consensus observation");
    broker
        .publish(
            RAW_ETHEREUM_OBSERVATION_TOPIC,
            std::slice::from_ref(&consensus_committed),
        )
        .await
        .expect("publish consensus");
    let consensus_observation = observation(&consensus_committed);
    assert_eq!(consensus_observation.payload, consensus_bytes);
    assert_eq!(consensus_observation.channel, "beacon_api");
    let parsed_consensus = RecordedConsensusCheckpoint::from_json(
        &consensus_observation.source_id,
        &consensus_observation.payload,
    )
    .expect("interpret committed consensus payload");
    assert_eq!(parsed_consensus.slot(), 100);
}

async fn verify_bsc_capture(broker: &MemoryBroker, bsc: &mut DurableSourceCapture<FileWal>) {
    let bsc_bytes = fixture("bsc/head-finalized.json");
    let bsc_committed = bsc
        .capture(
            RawSourceMessage::new("json_rpc", "head_and_finalized", &bsc_bytes)
                .expect("BSC message")
                .with_source_sequence(2),
        )
        .expect("durable BSC observation");
    broker
        .publish(
            RAW_BSC_OBSERVATION_TOPIC,
            std::slice::from_ref(&bsc_committed),
        )
        .await
        .expect("publish BSC");
    let bsc_observation = observation(&bsc_committed);
    assert_eq!(bsc_observation.payload, bsc_bytes);
    assert_eq!(bsc_observation.chain, "bsc");
    let parsed_bsc =
        RecordedBscHeads::from_json(&bsc_observation.source_id, &bsc_observation.payload)
            .expect("interpret committed BSC payload");
    assert_eq!(parsed_bsc.chain_id(), 56);
}

async fn verify_malformed_capture_and_routing(
    broker: &MemoryBroker,
    reth: &mut DurableSourceCapture<FileWal>,
) {
    let malformed_bytes = b"{malformed-reth-source-payload";
    let malformed_committed = reth
        .capture(
            RawSourceMessage::new("reth_exex", "chain_notification", malformed_bytes)
                .expect("malformed source message"),
        )
        .expect("malformed bytes still become durable replay truth");
    broker
        .publish(
            RAW_ETHEREUM_OBSERVATION_TOPIC,
            std::slice::from_ref(&malformed_committed),
        )
        .await
        .expect("publish malformed durable observation");
    let malformed_observation = observation(&malformed_committed);
    assert_eq!(malformed_observation.payload, malformed_bytes);
    assert!(
        RecordedRethNotification::from_json(
            &malformed_observation.source_id,
            &malformed_observation.payload,
        )
        .is_err()
    );

    let records = broker.records().await;
    assert_eq!(records.len(), 4);
    assert_eq!(records[0].topic, RAW_ETHEREUM_OBSERVATION_TOPIC);
    assert_eq!(records[1].topic, RAW_ETHEREUM_OBSERVATION_TOPIC);
    assert_eq!(records[2].topic, RAW_BSC_OBSERVATION_TOPIC);
    assert_eq!(records[3].topic, RAW_ETHEREUM_OBSERVATION_TOPIC);
    let replayed =
        CommittedObservation::decode(records[3].value.as_slice()).expect("broker replay record");
    assert_eq!(observation(&replayed).payload, malformed_bytes);
}

struct EvmCaptures {
    reth: DurableSourceCapture<FileWal>,
    consensus: DurableSourceCapture<FileWal>,
    bsc: DurableSourceCapture<FileWal>,
}

impl EvmCaptures {
    fn new(directory: &TempDir) -> Self {
        Self {
            reth: capture_engine(
                directory,
                "reth",
                SourceIdentity::new("reth-eu-1", "ethereum", "mainnet").expect("Reth identity"),
                [0x41; 16],
            ),
            consensus: capture_engine(
                directory,
                "consensus",
                SourceIdentity::new("lighthouse-eu-1", "ethereum", "mainnet")
                    .expect("consensus identity"),
                [0x42; 16],
            ),
            bsc: capture_engine(
                directory,
                "bsc",
                SourceIdentity::new("bsc-eu-1", "bsc", "mainnet").expect("BSC identity"),
                [0x43; 16],
            ),
        }
    }
}

async fn publish_twice(broker: &MemoryBroker, topic: &str, committed: &CommittedObservation) {
    for _ in 0..2 {
        broker
            .publish(topic, std::slice::from_ref(committed))
            .await
            .expect("idempotent publish");
    }
    assert_eq!(broker.records().await.len(), 1);
}

fn capture_engine(
    directory: &TempDir,
    name: &str,
    identity: SourceIdentity,
    session_bytes: [u8; 16],
) -> DurableSourceCapture<FileWal> {
    let session = CaptureSession::with_id(session_bytes);
    let (wal, recovery) = FileWal::open(
        directory.path().join(format!("{name}.wal")),
        WalConfig::new(session.id(), 1024 * 1024, Duration::from_millis(1)),
        Arc::new(FixedClock),
    )
    .expect("WAL");
    assert!(recovery.incidents.is_empty());
    DurableSourceCapture::new(identity, session, Arc::new(FixedClock), wal, 1024 * 1024)
        .expect("capture engine")
}

fn observation(committed: &CommittedObservation) -> &platform_proto::observation::Observation {
    committed.observation.as_ref().expect("observation")
}

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures")
            .join(path),
    )
    .expect("fixture")
}

#[derive(Debug)]
struct FixedClock;

impl Clock for FixedClock {
    fn wall_time_unix_ns(&self) -> i64 {
        1_900_000_000_000_000_200
    }

    fn monotonic_ns(&self) -> u64 {
        200
    }
}
