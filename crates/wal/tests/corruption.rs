mod common;

use std::{
    ffi::OsString,
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use tempfile::tempdir;
use test_fixtures::clock::FakeClock;
use wal::{FileWal, ObservationWal, UnframedObservation, WalConfig, WalError};

#[test]
fn crc_mismatch_in_a_committed_observation_fails_closed() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("observer.wal");
    let config = WalConfig::new(common::session(), 64 * 1024, Duration::from_millis(10));
    let clock = Arc::new(FakeClock::new(1_784_808_789_000_000_000, 100));
    let (mut wal, _) = FileWal::open(&path, config.clone(), clock.clone()).expect("create WAL");
    let pending = wal
        .append(UnframedObservation::new(common::observation(0, b"tx-a")))
        .expect("append");
    wal.group_commit().expect("commit");
    drop(wal);

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open WAL");
    let corrupt_at = pending.wal_offset.get() + 8;
    file.seek(SeekFrom::Start(corrupt_at)).expect("seek");
    let mut byte = [0_u8; 1];
    file.read_exact(&mut byte).expect("read byte");
    byte[0] ^= 0x80;
    file.seek(SeekFrom::Start(corrupt_at)).expect("seek");
    file.write_all(&byte).expect("corrupt byte");
    file.sync_data().expect("persist corruption");

    let result = FileWal::open(&path, config, clock);
    assert!(matches!(result, Err(WalError::CommittedCorruption { .. })));
    let mut quarantine_name = OsString::from(path.as_os_str());
    quarantine_name.push(".quarantine");
    let quarantine_path = PathBuf::from(quarantine_name);
    assert!(!path.exists(), "corrupt segment must leave the active path");
    assert!(
        quarantine_path.exists(),
        "corrupt bytes must be retained for forensic inspection"
    );
}
