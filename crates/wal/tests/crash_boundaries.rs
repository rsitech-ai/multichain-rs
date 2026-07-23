mod common;

use std::{fs::OpenOptions, sync::Arc, time::Duration};

use tempfile::tempdir;
use test_fixtures::clock::FakeClock;
use wal::{FileWal, ObservationWal, RecoveryIncident, UnframedObservation, WalConfig};

fn setup() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    WalConfig,
    Arc<FakeClock>,
) {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("observer.wal");
    let config = WalConfig::new(common::session(), 64 * 1024, Duration::from_millis(10));
    let clock = Arc::new(FakeClock::new(1_784_808_456_000_000_000, 100));
    (directory, path, config, clock)
}

#[test]
fn observation_without_commit_is_not_publishable_after_recovery() {
    let (_directory, path, config, clock) = setup();
    let (mut wal, _) = FileWal::open(&path, config.clone(), clock.clone()).expect("create WAL");
    wal.append(UnframedObservation::new(common::observation(0, b"tx-a")))
        .expect("append");
    drop(wal);

    let (wal, report) = FileWal::open(&path, config, clock).expect("recover");
    assert_eq!(wal.committed().expect("committed").count(), 0);
    assert!(matches!(
        report.incidents.as_slice(),
        [RecoveryIncident::UncommittedTail {
            observations: 1,
            ..
        }]
    ));
}

#[test]
fn fsynced_commit_makes_every_covered_observation_publishable() {
    let (_directory, path, config, clock) = setup();
    let (mut wal, _) = FileWal::open(&path, config.clone(), clock.clone()).expect("create WAL");
    wal.append(UnframedObservation::new(common::observation(0, b"tx-a")))
        .expect("append");
    wal.append(UnframedObservation::new(common::observation(1, b"tx-b")))
        .expect("append");
    wal.group_commit().expect("commit");
    drop(wal);

    let (wal, report) = FileWal::open(&path, config, clock).expect("recover");
    assert!(report.incidents.is_empty());
    assert_eq!(wal.committed().expect("committed").count(), 2);
}

#[test]
fn truncated_final_frame_recovers_to_prior_commit_boundary() {
    let (_directory, path, config, clock) = setup();
    let (mut wal, _) = FileWal::open(&path, config.clone(), clock.clone()).expect("create WAL");
    wal.append(UnframedObservation::new(common::observation(0, b"tx-a")))
        .expect("append");
    wal.group_commit().expect("commit first");
    wal.append(UnframedObservation::new(common::observation(1, b"tx-b")))
        .expect("append uncommitted");
    let interrupted_end = wal.logical_end();
    drop(wal);

    OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open WAL")
        .set_len(interrupted_end - 3)
        .expect("simulate partial final write");

    let (wal, report) = FileWal::open(&path, config, clock).expect("recover");
    assert_eq!(wal.committed().expect("committed").count(), 1);
    assert!(
        report
            .incidents
            .iter()
            .any(|incident| matches!(incident, RecoveryIncident::TruncatedTail { .. }))
    );
}
