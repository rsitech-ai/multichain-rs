use std::{
    collections::VecDeque,
    future::{Future, ready},
    sync::Arc,
    time::Duration,
};

use observation_envelope::Clock;
use platform_proto::observation::CommittedObservation;
use source_capture::{CaptureSession, DurableSourceCapture, RawSourceMessage, SourceIdentity};
use source_runtime::{
    BackoffPolicy, HttpPollingTransport, HttpRequestSpec, PollDisposition, PollEvent, RunExit,
    SourceLoop, SourceLoopConfig, SourceLoopError, SourceState, SourceTransport, TransportError,
    TransportFailureKind,
};
use storage_adapters::MemoryBroker;
use storage_ports::{BrokerAck, BrokerError, BrokerPublisher, RAW_ETHEREUM_OBSERVATION_TOPIC};
use tempfile::TempDir;
use tokio::{io::AsyncWriteExt as _, net::TcpListener};
use tokio_util::sync::CancellationToken;
use wal::{FileWal, ObservationWal as _, WalConfig};

#[tokio::test]
async fn successful_cycle_is_wal_committed_before_broker_publication() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let broker = MemoryBroker::default();
    let mut runner = runner(
        &directory,
        ScriptedTransport::new([Ok(PollEvent::success(
            "json_rpc",
            "eth_getBlockByNumber.latest",
            br#"{"jsonrpc":"2.0","id":1,"result":{"number":"0x1"}}"#,
            1,
            true,
        )
        .expect("event"))]),
        broker.clone(),
        CancellationToken::new(),
    );

    assert_eq!(
        runner.run_until_cycle_complete().await.expect("one cycle"),
        RunExit::CycleComplete
    );

    let health = runner.health();
    assert_eq!(health.state, SourceState::Healthy);
    assert_eq!(health.successful_observations, 1);
    assert_eq!(health.next_collector_sequence, 1);
    let records = broker.records().await;
    assert_eq!(records.len(), 1);
    let (_, capture, _) = runner.into_parts();
    let (_, wal) = capture.into_parts();
    assert_eq!(wal.committed().expect("committed WAL").count(), 1);
}

#[tokio::test]
async fn transient_gap_is_explicit_and_closes_only_after_recovery() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let mut runner = runner(
        &directory,
        ScriptedTransport::new([
            Err(TransportError::transient(TransportFailureKind::Network)),
            Ok(PollEvent::success(
                "json_rpc",
                "eth_getBlockByNumber.latest",
                b"{\"result\":\"recovered\"}",
                2,
                true,
            )
            .expect("event")),
        ]),
        MemoryBroker::default(),
        CancellationToken::new(),
    );

    assert_eq!(
        runner
            .run_until_cycle_complete()
            .await
            .expect("recovered cycle"),
        RunExit::CycleComplete
    );

    let health = runner.health();
    assert_eq!(health.state, SourceState::Healthy);
    assert_eq!(health.consecutive_failures, 0);
    let closed = health
        .last_closed_interval
        .as_ref()
        .expect("closed incomplete interval");
    assert_eq!(closed.failure_count, 1);
    assert_eq!(closed.reason, TransportFailureKind::Network);
    assert!(closed.closed_at_unix_ns.is_some());
    assert!(health.active_interval.is_none());
}

#[tokio::test]
async fn cancellation_interrupts_retry_backoff() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let cancellation = CancellationToken::new();
    let mut runner = runner(
        &directory,
        AlwaysTransient,
        MemoryBroker::default(),
        cancellation.clone(),
    );

    let cancel = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        cancellation.cancel();
    });
    let exit = tokio::time::timeout(Duration::from_secs(1), runner.run_until_cancelled())
        .await
        .expect("bounded shutdown")
        .expect("clean cancellation");
    cancel.await.expect("cancellation task");

    assert_eq!(exit, RunExit::Cancelled);
    assert_eq!(runner.health().state, SourceState::Stopped);
    assert!(runner.health().active_interval.is_some());
}

#[tokio::test]
async fn one_cycle_reports_cancellation_without_claiming_completion() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let mut runner = runner(
        &directory,
        AlwaysTransient,
        MemoryBroker::default(),
        cancellation,
    );

    let exit = runner
        .run_until_cycle_complete()
        .await
        .expect("clean cancellation");

    assert_eq!(exit, RunExit::Cancelled);
    assert_eq!(runner.health().state, SourceState::Stopped);
}

#[tokio::test]
async fn broker_failure_stops_with_committed_wal_available_for_replay() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let mut runner = runner(
        &directory,
        ScriptedTransport::new([Ok(PollEvent::success(
            "json_rpc",
            "eth_getBlockByNumber.latest",
            b"{\"result\":\"durable\"}",
            1,
            true,
        )
        .expect("event"))]),
        FailingBroker,
        CancellationToken::new(),
    );

    assert!(matches!(
        runner.run_until_cycle_complete().await,
        Err(SourceLoopError::Broker(BrokerError::Delivery(message)))
            if message == "test broker unavailable"
    ));
    assert_eq!(runner.health().state, SourceState::Failed);
    let (_, capture, _) = runner.into_parts();
    let (_, wal) = capture.into_parts();
    let committed = wal.committed().expect("committed WAL").collect::<Vec<_>>();
    assert_eq!(committed.len(), 1);
    assert_eq!(
        committed[0]
            .observation
            .as_ref()
            .expect("durable observation")
            .payload,
        b"{\"result\":\"durable\"}"
    );
}

#[tokio::test]
async fn chunked_http_body_is_rejected_at_the_configured_bound() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback listener");
    let address = listener.local_addr().expect("listener address");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("request");
        let response =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\n1234\r\n4\r\n5678\r\n0\r\n\r\n";
        socket.write_all(response).await.expect("response");
    });
    let request = HttpRequestSpec::get(
        format!("http://{address}/oversized"),
        "beacon_api",
        "beacon_block.head",
    )
    .expect("request");
    let mut transport =
        HttpPollingTransport::new([request], Duration::from_secs(1), 6).expect("transport");

    assert!(matches!(
        transport.next_event().await,
        Err(TransportError::ResponseTooLarge { max: 6 })
    ));
    server.await.expect("server task");
}

#[tokio::test]
async fn retryable_http_status_retains_exact_body_for_wal_capture() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback listener");
    let address = listener.local_addr().expect("listener address");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("request");
        let body = b"{\"error\":\"node warming up\"}";
        let header = format!(
            "HTTP/1.1 503 Service Unavailable\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        socket
            .write_all(header.as_bytes())
            .await
            .expect("response header");
        socket.write_all(body).await.expect("response body");
    });
    let request = HttpRequestSpec::get(
        format!("http://{address}/head"),
        "beacon_api",
        "beacon_block.head",
    )
    .expect("request");
    let mut transport =
        HttpPollingTransport::new([request], Duration::from_secs(1), 1024).expect("transport");

    let event = transport.next_event().await.expect("retryable event");

    assert_eq!(
        event.disposition,
        PollDisposition::RetryableFailure {
            kind: TransportFailureKind::HttpRetryable
        }
    );
    assert_eq!(
        event.message,
        RawSourceMessage::new(
            "beacon_api",
            "beacon_block.head",
            b"{\"error\":\"node warming up\"}"
        )
        .expect("expected message")
        .with_source_sequence(0)
    );
    server.await.expect("server task");
}

fn runner<T, B>(
    directory: &TempDir,
    transport: T,
    broker: B,
    cancellation: CancellationToken,
) -> SourceLoop<T, FileWal, B>
where
    T: SourceTransport,
    B: BrokerPublisher,
{
    let session = CaptureSession::with_id([0x61; 16]);
    let (wal, recovery) = FileWal::open(
        directory.path().join("source.wal"),
        WalConfig::new(session.id(), 1024 * 1024, Duration::from_millis(1)),
        Arc::new(FixedClock),
    )
    .expect("WAL");
    assert!(recovery.incidents.is_empty());
    let capture = DurableSourceCapture::new(
        SourceIdentity::new("reth-eu-1", "ethereum", "mainnet").expect("identity"),
        session,
        Arc::new(FixedClock),
        wal,
        4096,
    )
    .expect("capture");
    SourceLoop::new(
        transport,
        capture,
        broker,
        RAW_ETHEREUM_OBSERVATION_TOPIC,
        Arc::new(FixedClock),
        cancellation,
        SourceLoopConfig::new(
            Duration::from_millis(1),
            BackoffPolicy::new(Duration::from_millis(50), Duration::from_millis(50))
                .expect("backoff"),
        )
        .expect("loop config"),
    )
    .expect("runner")
}

struct ScriptedTransport {
    events: VecDeque<Result<PollEvent, TransportError>>,
}

impl ScriptedTransport {
    fn new(events: impl IntoIterator<Item = Result<PollEvent, TransportError>>) -> Self {
        Self {
            events: events.into_iter().collect(),
        }
    }
}

impl SourceTransport for ScriptedTransport {
    fn next_event(&mut self) -> impl Future<Output = Result<PollEvent, TransportError>> + Send {
        ready(
            self.events
                .pop_front()
                .expect("script contains enough events"),
        )
    }
}

struct AlwaysTransient;

impl SourceTransport for AlwaysTransient {
    fn next_event(&mut self) -> impl Future<Output = Result<PollEvent, TransportError>> + Send {
        ready(Err(TransportError::transient(
            TransportFailureKind::Timeout,
        )))
    }
}

struct FailingBroker;

impl BrokerPublisher for FailingBroker {
    fn publish(
        &self,
        _topic: &str,
        _records: &[CommittedObservation],
    ) -> impl Future<Output = Result<BrokerAck, BrokerError>> + Send {
        ready(Err(BrokerError::Delivery(
            "test broker unavailable".to_owned(),
        )))
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
