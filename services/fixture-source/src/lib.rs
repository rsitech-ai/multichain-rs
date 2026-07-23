#![doc = "Deterministic source used to prove the Phase 0 durable vertical."]

use std::{path::Path, sync::Arc, time::Duration};

use observation_envelope::{Clock, CollectorSequence, ObservationBuilder, SourceSessionId};
use platform_proto::observation::Observation;
use thiserror::Error;
use wal::{FileWal, ObservationWal, UnframedObservation, WalConfig, WalError};

/// Stable fixture subject used by the Phase 0 acceptance gate.
pub const PHASE0_FIXTURE_ID: &str = "phase0-001";
/// Stable observer identity for the synthetic vertical.
pub const PHASE0_SOURCE_ID: &str = "phase0-fixture-source";
/// Fixed session keeps fixture identities deterministic across full replays.
pub const PHASE0_SESSION_BYTES: [u8; 16] = [0x50; 16];
/// Exact payload required by the approved implementation plan.
pub const PHASE0_PAYLOAD: &[u8] =
    br#"{"fixture_id":"phase0-001","value":"exact-source-observation","source_sequence":42}"#;

const OBSERVED_AT_UNIX_NS: i64 = 1_784_808_123_000_000_000;
const OBSERVED_AT_MONOTONIC_NS: u64 = 42;

/// Fixture source failures.
#[derive(Debug, Error)]
pub enum FixtureSourceError {
    /// Observation construction was invalid.
    #[error("fixture observation is invalid: {0}")]
    Observation(#[from] observation_envelope::ObservationError),
    /// WAL creation, append, commit, or seal failed.
    #[error("fixture WAL failed: {0}")]
    Wal(#[from] WalError),
    /// A pre-existing fixture WAL required crash repair.
    #[error("fixture WAL recovery reported {0} incident(s)")]
    Recovery(usize),
}

/// Builds the exact deterministic Phase 0 observation.
///
/// # Errors
///
/// Returns [`FixtureSourceError`] if a fixed fixture identity violates the
/// observation contract.
pub fn phase0_observation() -> Result<Observation, FixtureSourceError> {
    let session = SourceSessionId::try_from(PHASE0_SESSION_BYTES.as_slice())?;
    Ok(ObservationBuilder::new()
        .source_id(PHASE0_SOURCE_ID)
        .source_session_id(session)
        .collector_sequence(CollectorSequence::new(0))
        .chain("bitcoin")
        .network("mainnet")
        .channel("fixture")
        .source_message_type("phase0_fixture")
        .source_sequence(42)
        .observed_at_unix_ns(OBSERVED_AT_UNIX_NS)
        .observed_at_monotonic_ns(OBSERVED_AT_MONOTONIC_NS)
        .payload(PHASE0_PAYLOAD)
        .build()?)
}

/// Persists and seals the fixture in a local durable WAL.
///
/// # Errors
///
/// Returns [`FixtureSourceError`] if the WAL cannot be created or durably
/// committed.
pub fn write_fixture_to_wal(
    directory: &Path,
    observation: Observation,
) -> Result<FileWal, FixtureSourceError> {
    let session = SourceSessionId::try_from(PHASE0_SESSION_BYTES.as_slice())?;
    let config = WalConfig::new(session, 64 * 1024, Duration::from_millis(5));
    let path = directory.join("phase0-fixture.wal");
    let (mut wal, recovery) = FileWal::open(&path, config, Arc::new(FixtureClock))?;
    if !recovery.incidents.is_empty() {
        return Err(FixtureSourceError::Recovery(recovery.incidents.len()));
    }
    wal.append(UnframedObservation::new(observation))?;
    wal.group_commit()?;
    wal.seal()?;
    Ok(wal)
}

#[derive(Debug)]
struct FixtureClock;

impl Clock for FixtureClock {
    fn wall_time_unix_ns(&self) -> i64 {
        OBSERVED_AT_UNIX_NS + 1
    }

    fn monotonic_ns(&self) -> u64 {
        OBSERVED_AT_MONOTONIC_NS + 1
    }
}
