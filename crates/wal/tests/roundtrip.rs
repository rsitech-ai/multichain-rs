mod common;

use std::{sync::Arc, time::Duration};

use observation_envelope::CollectorSequence;
use tempfile::tempdir;
use test_fixtures::clock::FakeClock;
use wal::{FileWal, ObservationWal, UnframedObservation, WalConfig, WalError};

#[test]
fn committed_observations_round_trip_with_durability_evidence() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("observer.wal");
    let clock = Arc::new(FakeClock::new(1_784_808_123_000_000_000, 99));
    let config = WalConfig::new(common::session(), 64 * 1024, Duration::from_millis(10));

    let (mut wal, report) =
        FileWal::open(&path, config.clone(), clock.clone()).expect("create WAL");
    assert!(report.incidents.is_empty());

    let first = wal
        .append(UnframedObservation::new(common::observation(0, b"tx-a")))
        .expect("append first");
    let second = wal
        .append(UnframedObservation::new(common::observation(1, b"tx-b")))
        .expect("append second");
    assert!(second.wal_offset > first.wal_offset);

    let committed_range = wal.group_commit().expect("group commit");
    assert_eq!(committed_range.first_sequence, CollectorSequence::new(0));
    assert_eq!(committed_range.last_sequence, CollectorSequence::new(1));
    assert_eq!(
        committed_range.durable_at_unix_ns,
        1_784_808_123_000_000_000
    );
    assert_eq!(committed_range.commit_hash.len(), 32);

    drop(wal);

    let (wal, report) = FileWal::open(&path, config, clock).expect("recover WAL");
    assert!(report.incidents.is_empty());
    let records = wal.committed().expect("read committed").collect::<Vec<_>>();
    assert_eq!(records.len(), 2);
    assert_eq!(
        records
            .iter()
            .map(|record| {
                record
                    .observation
                    .as_ref()
                    .expect("observation")
                    .collector_sequence
            })
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert!(records.iter().all(|record| {
        record.durable_at_unix_ns == 1_784_808_123_000_000_000
            && record.wal_commit_hash == committed_range.commit_hash
    }));
}

#[test]
fn sealed_segment_is_verified_and_cannot_accept_more_frames() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("sealed.wal");
    let clock = Arc::new(FakeClock::new(1_784_808_123_000_000_000, 99));
    let config = WalConfig::new(common::session(), 64 * 1024, Duration::from_millis(10));
    let (mut wal, _) = FileWal::open(&path, config.clone(), clock.clone()).expect("create WAL");
    wal.append(UnframedObservation::new(common::observation(0, b"tx-a")))
        .expect("append");
    wal.group_commit().expect("commit");
    let seal_hash = wal.seal().expect("seal segment");
    assert_eq!(seal_hash.len(), 32);
    assert!(matches!(
        wal.append(UnframedObservation::new(common::observation(1, b"tx-b"))),
        Err(WalError::Sealed)
    ));
    drop(wal);

    let (mut wal, report) = FileWal::open(&path, config, clock).expect("recover sealed WAL");
    assert!(report.incidents.is_empty());
    assert_eq!(wal.committed().expect("committed").count(), 1);
    assert!(matches!(wal.group_commit(), Err(WalError::Sealed)));
}
