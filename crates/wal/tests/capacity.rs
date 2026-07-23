mod common;

use std::{sync::Arc, time::Duration};

use tempfile::tempdir;
use test_fixtures::clock::FakeClock;
use wal::{FileWal, ObservationWal, UnframedObservation, WalConfig, WalError};

#[test]
fn capacity_exhaustion_rejects_before_overwriting_committed_data() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("observer.wal");
    let config = WalConfig::new(common::session(), 1024, Duration::from_millis(10));
    let clock = Arc::new(FakeClock::new(1_784_809_000_000_000_000, 100));
    let (mut wal, _) = FileWal::open(&path, config.clone(), clock.clone()).expect("create WAL");
    wal.append(UnframedObservation::new(common::observation(0, b"small")))
        .expect("append first");
    wal.group_commit().expect("commit first");

    let oversized = vec![0x5a; 2048];
    let result = wal.append(UnframedObservation::new(common::observation(1, oversized)));
    assert!(matches!(result, Err(WalError::CapacityExhausted { .. })));
    drop(wal);

    let (wal, report) = FileWal::open(&path, config, clock).expect("recover");
    assert!(report.incidents.is_empty());
    assert_eq!(wal.committed().expect("committed").count(), 1);
}
