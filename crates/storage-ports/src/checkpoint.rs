use std::future::Future;

use thiserror::Error;

/// Durable downstream sink whose coverage gates WAL reclamation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CheckpointKind {
    /// Durable Redpanda/Kafka acknowledgement.
    Broker,
    /// Verified raw object plus committed manifest.
    Archive,
}

/// Monotonic coverage for one source session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DurableCheckpoint {
    source_session_id: [u8; 16],
    last_collector_sequence: u64,
}

impl DurableCheckpoint {
    /// Constructs checkpoint coverage.
    #[must_use]
    pub const fn new(source_session_id: [u8; 16], last_collector_sequence: u64) -> Self {
        Self {
            source_session_id,
            last_collector_sequence,
        }
    }

    /// Returns the covered source session.
    #[must_use]
    pub const fn source_session_id(&self) -> [u8; 16] {
        self.source_session_id
    }

    /// Returns the inclusive covered collector sequence.
    #[must_use]
    pub const fn last_collector_sequence(&self) -> u64 {
        self.last_collector_sequence
    }

    /// Validates a monotonic checkpoint transition.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError`] for a session change or regression.
    pub fn advance(current: Option<&Self>, candidate: Self) -> Result<Self, CheckpointError> {
        if let Some(current) = current {
            if current.source_session_id != candidate.source_session_id {
                return Err(CheckpointError::SourceSessionMismatch);
            }
            if candidate.last_collector_sequence < current.last_collector_sequence {
                return Err(CheckpointError::Regression {
                    current: current.last_collector_sequence,
                    candidate: candidate.last_collector_sequence,
                });
            }
        }
        Ok(candidate)
    }
}

/// Identity and terminal range of one sealed WAL segment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SealedWalSegment {
    source_session_id: [u8; 16],
    last_collector_sequence: u64,
}

impl SealedWalSegment {
    /// Constructs sealed segment metadata.
    #[must_use]
    pub const fn new(source_session_id: [u8; 16], last_collector_sequence: u64) -> Self {
        Self {
            source_session_id,
            last_collector_sequence,
        }
    }
}

/// Determines whether broker and archive proof cover a sealed segment.
///
/// # Errors
///
/// Fails closed for missing, mismatched, or insufficient coverage.
pub fn ensure_reclaimable(
    segment: &SealedWalSegment,
    broker: Option<&DurableCheckpoint>,
    archive: Option<&DurableCheckpoint>,
) -> Result<(), ReclaimBlocker> {
    let broker = broker.ok_or(ReclaimBlocker::MissingBrokerCheckpoint)?;
    validate_coverage(segment, broker, CheckpointKind::Broker)?;
    let archive = archive.ok_or(ReclaimBlocker::MissingArchiveCheckpoint)?;
    validate_coverage(segment, archive, CheckpointKind::Archive)
}

fn validate_coverage(
    segment: &SealedWalSegment,
    checkpoint: &DurableCheckpoint,
    kind: CheckpointKind,
) -> Result<(), ReclaimBlocker> {
    if checkpoint.source_session_id != segment.source_session_id {
        return Err(ReclaimBlocker::SourceSessionMismatch { checkpoint: kind });
    }
    if checkpoint.last_collector_sequence < segment.last_collector_sequence {
        return Err(ReclaimBlocker::InsufficientCoverage {
            checkpoint: kind,
            required: segment.last_collector_sequence,
            actual: checkpoint.last_collector_sequence,
        });
    }
    Ok(())
}

/// Fail-closed reasons a sealed WAL segment must be retained.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ReclaimBlocker {
    /// Broker coverage is absent.
    #[error("broker checkpoint is missing")]
    MissingBrokerCheckpoint,
    /// Verified archive coverage is absent.
    #[error("archive checkpoint is missing")]
    MissingArchiveCheckpoint,
    /// Coverage belongs to a different source session.
    #[error("{checkpoint:?} checkpoint belongs to a different source session")]
    SourceSessionMismatch {
        /// Mismatched checkpoint.
        checkpoint: CheckpointKind,
    },
    /// Coverage does not reach the segment end.
    #[error("{checkpoint:?} checkpoint covers {actual}, but segment requires {required}")]
    InsufficientCoverage {
        /// Insufficient checkpoint.
        checkpoint: CheckpointKind,
        /// Segment terminal sequence.
        required: u64,
        /// Checkpoint terminal sequence.
        actual: u64,
    },
}

/// Durable checkpoint persistence failures.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CheckpointError {
    /// A checkpoint attempted to change source session.
    #[error("checkpoint source session mismatch")]
    SourceSessionMismatch,
    /// A checkpoint attempted to move backwards.
    #[error("checkpoint regression from {current} to {candidate}")]
    Regression {
        /// Existing durable sequence.
        current: u64,
        /// Proposed durable sequence.
        candidate: u64,
    },
    /// Storage failed.
    #[error("checkpoint storage failed: {0}")]
    Storage(String),
}

/// Persists independently monotonic broker and archive coverage.
pub trait CheckpointStore: Send + Sync {
    /// Loads checkpoint coverage for a source.
    fn load(
        &self,
        kind: CheckpointKind,
        source_id: &str,
        source_session_id: [u8; 16],
    ) -> impl Future<Output = Result<Option<DurableCheckpoint>, CheckpointError>> + Send;

    /// Advances coverage transactionally and rejects regression.
    fn advance(
        &self,
        kind: CheckpointKind,
        source_id: &str,
        checkpoint: DurableCheckpoint,
    ) -> impl Future<Output = Result<DurableCheckpoint, CheckpointError>> + Send;
}
