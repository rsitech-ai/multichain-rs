use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use bitcoin_core_connector::{
    capture::CaptureEngine,
    config::{BitcoinCoreNetwork, ObserverConfig},
    reconcile::{MempoolReconciler, MempoolRecoveryEvent},
    rpc::CoreRpcClient,
    session::SourceSession,
    zmq::{ZmqNotification, receive_topic},
};
use observation_envelope::Clock;
use storage_adapters::RedpandaBroker;
use storage_ports::{BrokerPublisher, RAW_BITCOIN_OBSERVATION_TOPIC};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wal::{FileWal, WalConfig};

struct SystemClock {
    started: Instant,
}

impl Clock for SystemClock {
    fn wall_time_unix_ns(&self) -> i64 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        i64::try_from(nanos).unwrap_or(i64::MAX)
    }

    fn monotonic_ns(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source_id = required_env("BITCOIN_SOURCE_ID")?;
    let network = BitcoinCoreNetwork::parse(&required_env("BITCOIN_NETWORK")?)?;
    let rpc_endpoint = required_env("BITCOIN_RPC_ENDPOINT")?;
    let zmq_endpoint = required_env("BITCOIN_ZMQ_ENDPOINT")?;
    let cookie_path = PathBuf::from(required_env("BITCOIN_RPC_COOKIE")?);
    let wal_base = PathBuf::from(required_env("BITCOIN_WAL_PATH")?);
    let brokers = required_env("REDPANDA_BROKERS")?;
    let config = ObserverConfig {
        source_id: source_id.clone(),
        network,
        rpc_endpoint: rpc_endpoint.clone(),
        zmq_endpoints: vec![zmq_endpoint.clone()],
        rpc_cookie_path: cookie_path.clone(),
        wallet_rpc_enabled: false,
        wal_path: wal_base.clone(),
        max_message_bytes: 4_000_000,
    };
    config.validate()?;

    let clock = Arc::new(SystemClock {
        started: Instant::now(),
    });
    let session = SourceSession::new(source_id.clone(), clock.wall_time_unix_ns())?;
    let mut wal_path = wal_base.into_os_string();
    wal_path.push(".");
    wal_path.push(session_suffix(session.id().as_bytes()));
    wal_path.push(".wal");
    let (wal, recovery) = FileWal::open(
        PathBuf::from(wal_path),
        WalConfig::new(session.id(), 512 * 1024 * 1024, Duration::from_millis(10)),
        clock.clone(),
    )?;
    if recovery.logical_end != 0 {
        return Err("new source session unexpectedly reused a committed WAL segment".into());
    }
    let mut capture = CaptureEngine::new(&source_id, network, session, clock.clone(), wal);
    let mut reconciler = MempoolReconciler::new(&source_id, *capture.session().id().as_bytes());
    let rpc = CoreRpcClient::new(
        rpc_endpoint,
        cookie_path,
        Duration::from_secs(10),
        CancellationToken::new(),
    )?;
    let broker = RedpandaBroker::new(&brokers, Duration::from_secs(10))?;
    let topic = match network {
        BitcoinCoreNetwork::Mainnet => RAW_BITCOIN_OBSERVATION_TOPIC,
        BitcoinCoreNetwork::Regtest => "dev.raw.bitcoin.regtest.source.observation.v1",
    };

    let cancellation = CancellationToken::new();
    let (sender, mut receiver) = mpsc::channel::<ZmqNotification>(1_024);
    for source_topic in ["rawtx", "rawblock", "sequence"] {
        let endpoint = zmq_endpoint.clone();
        let sender = sender.clone();
        let cancellation = cancellation.clone();
        tokio::spawn(async move {
            if let Err(error) =
                receive_topic(&endpoint, source_topic, 4_000_000, sender, cancellation).await
            {
                eprintln!("ZMQ receiver {source_topic} stopped: {error}");
            }
        });
    }
    drop(sender);

    let mut topic_sequences = HashMap::<String, u32>::new();
    loop {
        let notification = tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal?;
                cancellation.cancel();
                break;
            }
            value = receiver.recv() => value.ok_or("all ZMQ receivers stopped")?,
        };
        if let Some(previous) =
            topic_sequences.insert(notification.topic.clone(), notification.transport_sequence)
        {
            let expected = previous.wrapping_add(1);
            if notification.transport_sequence != expected && notification.topic == "sequence" {
                let now = clock.wall_time_unix_ns();
                let _ = reconciler.observe_sequence(u64::from(previous), now);
                if matches!(
                    reconciler.observe_sequence(u64::from(notification.transport_sequence), now),
                    Some(MempoolRecoveryEvent::GapDetected { .. })
                ) {
                    let recovered = reconciler.recover(&rpc).await?;
                    let committed = capture.capture_recovered_mempool_snapshot(
                        recovered.source_payload,
                        recovered.mempool_sequence,
                    )?;
                    broker.publish(topic, &[committed]).await?;
                }
            }
        }
        let committed = capture.capture(notification)?;
        broker.publish(topic, &[committed]).await?;
    }
    Ok(())
}

fn required_env(name: &'static str) -> Result<String, Box<dyn std::error::Error>> {
    let value = std::env::var(name).map_err(|_| format!("required environment variable {name}"))?;
    if value.trim().is_empty() {
        return Err(format!("required environment variable {name} is empty").into());
    }
    Ok(value)
}

fn session_suffix(bytes: &[u8; 16]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(32), |mut value, byte| {
            write!(value, "{byte:02x}").expect("writing into a String cannot fail");
            value
        })
}
