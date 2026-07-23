#![doc = "Bounded, commit-marked local write-ahead log."]

mod error;
mod format;
mod reader;
mod recovery;
mod writer;

pub use error::WalError;
pub use recovery::{RecoveryIncident, RecoveryReport};
pub use writer::{FileWal, WalConfig};

use observation_envelope::{CollectorSequence, SourceSessionId};
use platform_proto::observation::{CommittedObservation, Observation};

/// Byte offset within a WAL segment.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WalOffset(u64);

impl WalOffset {
    /// Constructs an offset.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the underlying byte offset.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// An observation waiting to be framed by the WAL.
#[derive(Clone, Debug, PartialEq)]
pub struct UnframedObservation {
    /// Validated wire observation.
    pub observation: Observation,
}

impl UnframedObservation {
    /// Wraps a validated observation for append.
    #[must_use]
    pub const fn new(observation: Observation) -> Self {
        Self { observation }
    }
}

/// Result of an append that has not yet been group-committed.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingObservation {
    /// Observation written to the segment.
    pub observation: Observation,
    /// Offset of the observation frame.
    pub wal_offset: WalOffset,
}

/// Durable range produced by one group commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommittedRange {
    /// First collector sequence covered by the commit.
    pub first_sequence: CollectorSequence,
    /// Last collector sequence covered by the commit.
    pub last_sequence: CollectorSequence,
    /// Wall-clock time recorded after the durability barrier.
    pub durable_at_unix_ns: i64,
    /// Commit digest over the immutable observation frames and range metadata.
    pub commit_hash: [u8; 32],
}

/// Contract implemented by an observation WAL.
pub trait ObservationWal {
    /// Appends one immutable observation frame without making it publishable.
    ///
    /// # Errors
    ///
    /// Returns [`WalError`] when identity/order validation fails, the segment
    /// has insufficient capacity, or the write fails.
    fn append(&mut self, input: UnframedObservation) -> Result<PendingObservation, WalError>;

    /// Makes all pending frames durable as one immutable range.
    ///
    /// # Errors
    ///
    /// Returns [`WalError`] when no observations are pending, capacity is
    /// exhausted, or the write/durability barrier fails.
    fn group_commit(&mut self) -> Result<CommittedRange, WalError>;

    /// Reads observations covered by valid durable commit records.
    ///
    /// # Errors
    ///
    /// Returns [`WalError`] when frame, checksum, commit, or I/O validation
    /// fails.
    fn committed(&self) -> Result<Box<dyn Iterator<Item = CommittedObservation>>, WalError>;
}

#[derive(Clone, Debug)]
struct PendingFrame {
    observation: Observation,
    offset: WalOffset,
    end_offset: u64,
    frame_bytes: Vec<u8>,
}

fn validate_session(expected: SourceSessionId, observation: &Observation) -> Result<(), WalError> {
    if observation.source_session_id.as_slice() != expected.as_bytes() {
        return Err(WalError::SourceSessionMismatch);
    }
    Ok(())
}
