use std::{sync::Arc, time::Duration};

use observation_envelope::Clock;
use platform_proto::observation::CommittedObservation;
use source_capture::{
    CaptureError, CaptureSession, DurableSourceCapture, RawSourceMessage, SourceIdentity,
};
use tempfile::tempdir;
use wal::{
    CommittedRange, FileWal, ObservationWal, PendingObservation, UnframedObservation, WalConfig,
    WalError, WalOffset,
};

#[test]
fn exact_payloads_commit_in_one_session_local_total_order() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("reth-source.wal");
    let session = CaptureSession::with_id([0x31; 16]);
    let (wal, recovery) = FileWal::open(
        &path,
        WalConfig::new(session.id(), 1024 * 1024, Duration::from_millis(1)),
        Arc::new(FixedClock),
    )
    .expect("WAL");
    assert!(recovery.incidents.is_empty());
    let mut capture = DurableSourceCapture::new(
        SourceIdentity::new("reth-eu-1", "ethereum", "mainnet").expect("identity"),
        session,
        Arc::new(FixedClock),
        wal,
        4096,
    )
    .expect("capture engine");

    let first_bytes = br#"{"kind":"committed","new":[{"number":"0x1"}]}"#;
    let first = capture
        .capture(
            RawSourceMessage::new("reth_exex", "chain_committed", first_bytes)
                .expect("message")
                .with_source_sequence(101)
                .with_source_cursor([0x01, 0x02])
                .expect("bounded cursor")
                .with_source_time_unix_ns(1_900_000_000_000_000_000),
        )
        .expect("first committed observation");
    let second_bytes = b"{malformed-but-source-exact";
    let second = capture
        .capture(
            RawSourceMessage::new("reth_exex", "chain_notification", second_bytes)
                .expect("message"),
        )
        .expect("second committed observation");

    let first = first.observation.expect("first observation");
    assert_eq!(first.collector_sequence, 0);
    assert_eq!(first.source_id, "reth-eu-1");
    assert_eq!(first.source_session_id, vec![0x31; 16]);
    assert_eq!(first.chain, "ethereum");
    assert_eq!(first.network, "mainnet");
    assert_eq!(first.channel, "reth_exex");
    assert_eq!(first.source_message_type, "chain_committed");
    assert_eq!(first.source_sequence, Some(101));
    assert_eq!(first.source_cursor, Some(vec![0x01, 0x02]));
    assert_eq!(first.source_time_unix_ns, Some(1_900_000_000_000_000_000));
    assert_eq!(first.payload, first_bytes);

    let second = second.observation.expect("second observation");
    assert_eq!(second.collector_sequence, 1);
    assert_eq!(second.payload, second_bytes);
    assert_ne!(second.observation_id, first.observation_id);

    let (_, wal) = capture.into_parts();
    let committed = wal.committed().expect("committed scan").collect::<Vec<_>>();
    assert_eq!(committed.len(), 2);
    assert_eq!(
        committed[0]
            .observation
            .as_ref()
            .expect("first durable payload")
            .payload,
        first_bytes
    );
    assert_eq!(
        committed[1]
            .observation
            .as_ref()
            .expect("second durable payload")
            .payload,
        second_bytes
    );
}

#[test]
fn reopened_wal_resumes_without_sequence_reuse() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("resumed-source.wal");
    let session_id = [0x35; 16];
    let initial_session = CaptureSession::with_id(session_id);
    let config = WalConfig::new(initial_session.id(), 1024 * 1024, Duration::from_millis(1));
    let (wal, _) = FileWal::open(&path, config.clone(), Arc::new(FixedClock)).expect("initial WAL");
    let mut initial = DurableSourceCapture::new(
        SourceIdentity::new("reth-eu-1", "ethereum", "mainnet").expect("identity"),
        initial_session,
        Arc::new(FixedClock),
        wal,
        4096,
    )
    .expect("initial capture");
    initial
        .capture(RawSourceMessage::new("reth_exex", "chain_committed", b"first").expect("message"))
        .expect("first commit");
    drop(initial);

    let (wal, recovery) = FileWal::open(&path, config, Arc::new(FixedClock)).expect("reopened WAL");
    assert!(recovery.incidents.is_empty());
    let resumed_session = CaptureSession::resume(session_id, wal.next_sequence());
    let mut resumed = DurableSourceCapture::new(
        SourceIdentity::new("reth-eu-1", "ethereum", "mainnet").expect("identity"),
        resumed_session,
        Arc::new(FixedClock),
        wal,
        4096,
    )
    .expect("resumed capture");
    let second = resumed
        .capture(RawSourceMessage::new("reth_exex", "chain_committed", b"second").expect("message"))
        .expect("second commit")
        .observation
        .expect("second observation");

    assert_eq!(second.collector_sequence, 1);
    let (_, wal) = resumed.into_parts();
    assert_eq!(wal.committed().expect("committed scan").count(), 2);
}

#[test]
fn invalid_identity_message_and_payload_bounds_fail_before_wal_writes() {
    assert!(matches!(
        SourceIdentity::new("", "ethereum", "mainnet"),
        Err(CaptureError::InvalidIdentity { field: "source_id" })
    ));
    assert!(matches!(
        RawSourceMessage::new("", "chain_committed", b"{}"),
        Err(CaptureError::InvalidMessageField { field: "channel" })
    ));
    let oversized_payload = vec![0_u8; 64 * 1024 * 1024 + 1];
    assert!(matches!(
        RawSourceMessage::new("reth_exex", "chain_committed", &oversized_payload),
        Err(CaptureError::PayloadTooLarge {
            actual: 67_108_865,
            max: 67_108_864
        })
    ));
    let oversized_cursor = vec![0_u8; 1_025];
    assert!(matches!(
        RawSourceMessage::new("reth_exex", "chain_committed", b"{}")
            .expect("message")
            .with_source_cursor(&oversized_cursor),
        Err(CaptureError::SourceCursorTooLarge)
    ));

    let directory = tempdir().expect("temporary directory");
    let session = CaptureSession::with_id([0x32; 16]);
    let (wal, _) = FileWal::open(
        directory.path().join("bounded-source.wal"),
        WalConfig::new(session.id(), 1024 * 1024, Duration::from_millis(1)),
        Arc::new(FixedClock),
    )
    .expect("WAL");
    let mut capture = DurableSourceCapture::new(
        SourceIdentity::new("bsc-eu-1", "bsc", "mainnet").expect("identity"),
        session,
        Arc::new(FixedClock),
        wal,
        4,
    )
    .expect("capture engine");
    assert!(matches!(
        capture.capture(
            RawSourceMessage::new("rpc", "head_and_finalized", b"12345").expect("message")
        ),
        Err(CaptureError::PayloadTooLarge { actual: 5, max: 4 })
    ));

    let committed = capture
        .capture(RawSourceMessage::new("rpc", "head_and_finalized", b"1234").expect("message"))
        .expect("bounded observation")
        .observation
        .expect("observation");
    assert_eq!(committed.collector_sequence, 0);

    let (_, wal) = capture.into_parts();
    assert_eq!(wal.committed().expect("committed scan").count(), 1);
}

#[test]
fn ambiguous_wal_failure_poisoning_prevents_sequence_reuse() {
    let session = CaptureSession::with_id([0x33; 16]);
    let mut capture = DurableSourceCapture::new(
        SourceIdentity::new("reth-eu-1", "ethereum", "mainnet").expect("identity"),
        session,
        Arc::new(FixedClock),
        FailingCommitWal,
        1024,
    )
    .expect("capture engine");

    assert!(matches!(
        capture.capture(
            RawSourceMessage::new("reth_exex", "chain_committed", b"{}").expect("message")
        ),
        Err(CaptureError::Wal(WalError::NoPendingObservations))
    ));
    assert!(capture.is_poisoned());
    assert_eq!(capture.session().next_sequence(), 0);
    assert!(matches!(
        capture.capture(
            RawSourceMessage::new("reth_exex", "chain_committed", b"{}").expect("message")
        ),
        Err(CaptureError::CaptureUnavailable)
    ));
}

#[test]
fn inconsistent_commit_range_poisoning_prevents_unproven_publication() {
    let session = CaptureSession::with_id([0x34; 16]);
    let mut capture = DurableSourceCapture::new(
        SourceIdentity::new("reth-eu-1", "ethereum", "mainnet").expect("identity"),
        session,
        Arc::new(FixedClock),
        InconsistentCommitWal,
        1024,
    )
    .expect("capture engine");

    assert!(matches!(
        capture.capture(
            RawSourceMessage::new("reth_exex", "chain_committed", b"{}").expect("message")
        ),
        Err(CaptureError::CommitRangeMismatch {
            expected: 0,
            first: 1,
            last: 1
        })
    ));
    assert!(capture.is_poisoned());
    assert_eq!(capture.session().next_sequence(), 0);
}

#[derive(Debug)]
struct FixedClock;

impl Clock for FixedClock {
    fn wall_time_unix_ns(&self) -> i64 {
        1_900_000_000_000_000_100
    }

    fn monotonic_ns(&self) -> u64 {
        100
    }
}

struct FailingCommitWal;

impl ObservationWal for FailingCommitWal {
    fn append(&mut self, input: UnframedObservation) -> Result<PendingObservation, WalError> {
        Ok(PendingObservation {
            observation: input.observation,
            wal_offset: WalOffset::new(0),
        })
    }

    fn group_commit(&mut self) -> Result<CommittedRange, WalError> {
        Err(WalError::NoPendingObservations)
    }

    fn committed(&self) -> Result<Box<dyn Iterator<Item = CommittedObservation>>, WalError> {
        Ok(Box::new(std::iter::empty()))
    }
}

struct InconsistentCommitWal;

impl ObservationWal for InconsistentCommitWal {
    fn append(&mut self, input: UnframedObservation) -> Result<PendingObservation, WalError> {
        Ok(PendingObservation {
            observation: input.observation,
            wal_offset: WalOffset::new(0),
        })
    }

    fn group_commit(&mut self) -> Result<CommittedRange, WalError> {
        Ok(CommittedRange {
            first_sequence: observation_envelope::CollectorSequence::new(1),
            last_sequence: observation_envelope::CollectorSequence::new(1),
            durable_at_unix_ns: 1_900_000_000_000_000_100,
            commit_hash: [0x44; 32],
        })
    }

    fn committed(&self) -> Result<Box<dyn Iterator<Item = CommittedObservation>>, WalError> {
        Ok(Box::new(std::iter::empty()))
    }
}
