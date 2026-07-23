use std::{sync::Arc, time::Duration};

use bitcoin_core_connector::{
    capture::CaptureEngine,
    config::BitcoinCoreNetwork,
    session::SourceSession,
    zmq::{ZmqNotification, parse_multipart},
};
use test_fixtures::clock::FakeClock;
use tokio::sync::mpsc;
use wal::{FileWal, WalConfig};

#[tokio::test]
async fn three_topics_share_one_bounded_total_order_allocator() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let clock = Arc::new(FakeClock::new(100, 200));
    let session = SourceSession::with_id("observer-a", [7; 16], 100);
    let (wal, _) = FileWal::open(
        temporary.path().join("observer.wal"),
        WalConfig::new(session.id(), 1_000_000, Duration::from_millis(10)),
        clock.clone(),
    )
    .expect("open WAL");
    let mut capture = CaptureEngine::new(
        "observer-a",
        BitcoinCoreNetwork::Mainnet,
        session,
        clock,
        wal,
    );
    let (sender, mut receiver) = mpsc::channel(3);
    for (topic, sequence) in [("rawtx", 4), ("sequence", 9), ("rawblock", 2)] {
        let sender = sender.clone();
        tokio::spawn(async move {
            sender
                .send(ZmqNotification {
                    topic: topic.to_owned(),
                    body: vec![u8::try_from(sequence).expect("small")],
                    transport_sequence: sequence,
                })
                .await
                .expect("bounded receiver alive");
        });
    }
    drop(sender);

    let mut observations = Vec::new();
    while let Some(notification) = receiver.recv().await {
        observations.push(capture.capture(notification).expect("durable capture"));
    }
    let sequences: Vec<_> = observations
        .iter()
        .map(|record| {
            record
                .observation
                .as_ref()
                .expect("observation")
                .collector_sequence
        })
        .collect();
    assert_eq!(sequences, vec![0, 1, 2]);
    let recovered = capture
        .capture_recovered_mempool_snapshot(br#"{"txids":[]}"#.to_vec(), 42)
        .expect("durable recovery evidence");
    let recovered = recovered.observation.expect("observation");
    assert_eq!(recovered.collector_sequence, 3);
    assert_eq!(recovered.channel, "rpc");
    assert_eq!(recovered.quality_flags, vec!["recovered_by_rpc"]);
}

#[test]
fn multipart_validation_rejects_unknown_large_or_malformed_frames() {
    assert!(parse_multipart(&[b"rawtx".as_slice(), b"body", &[1, 0, 0, 0]], 4).is_ok());
    assert!(parse_multipart(&[b"hashblock".as_slice(), b"body", &[1, 0, 0, 0]], 4).is_err());
    assert!(parse_multipart(&[b"rawtx".as_slice(), b"large", &[1, 0, 0, 0]], 4).is_err());
    assert!(parse_multipart(&[b"rawtx".as_slice(), b"body"], 4).is_err());
}
