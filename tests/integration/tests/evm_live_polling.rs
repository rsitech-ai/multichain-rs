use std::{sync::Arc, time::Duration};

use bsc_connector::{BscNodeConfig, BscNodeKind};
use ethereum_consensus_connector::ConsensusSourceConfig;
use ethereum_reth_connector::RethSourceConfig;
use observation_envelope::Clock;
use platform_proto::observation::CommittedObservation;
use prost::Message as _;
use source_capture::{CaptureSession, DurableSourceCapture, SourceIdentity};
use source_runtime::{BackoffPolicy, HttpPollingTransport, SourceLoop, SourceLoopConfig};
use storage_adapters::MemoryBroker;
use storage_ports::{RAW_BSC_OBSERVATION_TOPIC, RAW_ETHEREUM_OBSERVATION_TOPIC};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
};
use tokio_util::sync::CancellationToken;
use wal::{FileWal, WalConfig};

const RESPONSE_BODY: &[u8] = br#"{"jsonrpc":"2.0","id":1,"result":{"proof":"exact-source-bytes"}}"#;

#[tokio::test]
async fn owned_evm_http_sources_flow_exact_responses_through_wal_to_chain_topics() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let broker = MemoryBroker::default();

    let (reth_endpoint, reth_server) = loopback_server(4).await;
    let reth_plan = RethSourceConfig::new("reth-eu-1", reth_endpoint, 1)
        .expect("Reth config")
        .http_poll_plan()
        .expect("Reth plan");
    run_plan(
        &directory,
        "reth",
        SourceIdentity::new("reth-eu-1", "ethereum", "mainnet").expect("identity"),
        [0x51; 16],
        reth_plan,
        broker.clone(),
        RAW_ETHEREUM_OBSERVATION_TOPIC,
    )
    .await;
    reth_server.await.expect("Reth server");

    let (consensus_endpoint, consensus_server) = loopback_server(2).await;
    let consensus_plan = ConsensusSourceConfig::new("lighthouse-eu-1", consensus_endpoint)
        .expect("consensus config")
        .http_poll_plan()
        .expect("consensus plan");
    run_plan(
        &directory,
        "consensus",
        SourceIdentity::new("lighthouse-eu-1", "ethereum", "mainnet").expect("identity"),
        [0x52; 16],
        consensus_plan,
        broker.clone(),
        RAW_ETHEREUM_OBSERVATION_TOPIC,
    )
    .await;
    consensus_server.await.expect("consensus server");

    let (bsc_endpoint, bsc_server) = loopback_server(6).await;
    let bsc_plan = BscNodeConfig::new("bsc-eu-1", bsc_endpoint, 56, BscNodeKind::OfficialBsc)
        .expect("BSC config")
        .http_poll_plan()
        .expect("BSC plan");
    run_plan(
        &directory,
        "bsc",
        SourceIdentity::new("bsc-eu-1", "bsc", "mainnet").expect("identity"),
        [0x53; 16],
        bsc_plan,
        broker.clone(),
        RAW_BSC_OBSERVATION_TOPIC,
    )
    .await;
    bsc_server.await.expect("BSC server");

    let records = broker.records().await;
    assert_eq!(records.len(), 12);
    assert!(
        records[..6]
            .iter()
            .all(|record| record.topic == RAW_ETHEREUM_OBSERVATION_TOPIC)
    );
    assert!(
        records[6..]
            .iter()
            .all(|record| record.topic == RAW_BSC_OBSERVATION_TOPIC)
    );
    for record in records {
        let committed =
            CommittedObservation::decode(record.value.as_slice()).expect("committed observation");
        assert_eq!(
            committed.observation.expect("observation").payload,
            RESPONSE_BODY
        );
    }
}

async fn run_plan(
    directory: &TempDir,
    name: &str,
    identity: SourceIdentity,
    session_bytes: [u8; 16],
    plan: Vec<source_runtime::HttpRequestSpec>,
    broker: MemoryBroker,
    topic: &str,
) {
    let session = CaptureSession::with_id(session_bytes);
    let (wal, recovery) = FileWal::open(
        directory.path().join(format!("{name}.wal")),
        WalConfig::new(session.id(), 1024 * 1024, Duration::from_millis(1)),
        Arc::new(FixedClock),
    )
    .expect("WAL");
    assert!(recovery.incidents.is_empty());
    let capture =
        DurableSourceCapture::new(identity, session, Arc::new(FixedClock), wal, 1024 * 1024)
            .expect("capture");
    let transport =
        HttpPollingTransport::new(plan, Duration::from_secs(2), 1024 * 1024).expect("transport");
    let config = SourceLoopConfig::new(
        Duration::from_millis(10),
        BackoffPolicy::new(Duration::from_millis(10), Duration::from_millis(50)).expect("backoff"),
    )
    .expect("loop config");
    let mut source_loop = SourceLoop::new(
        transport,
        capture,
        broker,
        topic,
        Arc::new(FixedClock),
        CancellationToken::new(),
        config,
    )
    .expect("source loop");

    assert_eq!(
        source_loop
            .run_until_cycle_complete()
            .await
            .expect("polling cycle"),
        source_runtime::RunExit::CycleComplete
    );
}

async fn loopback_server(response_count: usize) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback listener");
    let endpoint = format!("http://{}", listener.local_addr().expect("address"));
    let server = tokio::spawn(async move {
        for _ in 0..response_count {
            let (mut socket, _) = listener.accept().await.expect("request");
            read_http_request(&mut socket).await;
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                RESPONSE_BODY.len()
            );
            socket
                .write_all(header.as_bytes())
                .await
                .expect("response header");
            socket
                .write_all(RESPONSE_BODY)
                .await
                .expect("response body");
        }
    });
    (endpoint, server)
}

async fn read_http_request(socket: &mut tokio::net::TcpStream) {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = socket.read(&mut chunk).await.expect("request bytes");
        assert!(read > 0, "request ended before headers");
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let body_start = header_end + 4;
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(str::trim)
                        .and_then(|value| value.parse::<usize>().ok())
                })
                .unwrap_or(0);
            while bytes.len() < body_start + content_length {
                let read = socket.read(&mut chunk).await.expect("request body");
                assert!(read > 0, "request ended before body");
                bytes.extend_from_slice(&chunk[..read]);
            }
            return;
        }
    }
}

#[derive(Debug)]
struct FixedClock;

impl Clock for FixedClock {
    fn wall_time_unix_ns(&self) -> i64 {
        1_900_000_000_000_000_500
    }

    fn monotonic_ns(&self) -> u64 {
        500
    }
}
