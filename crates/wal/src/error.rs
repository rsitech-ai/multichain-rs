use std::{io, path::PathBuf};

use thiserror::Error;

/// WAL append, durability, and recovery failures.
#[derive(Debug, Error)]
pub enum WalError {
    /// An operating-system file operation failed.
    #[error("WAL I/O failed: {0}")]
    Io(#[from] io::Error),

    /// A Protobuf frame could not be decoded.
    #[error("WAL frame at offset {offset} could not be decoded: {reason}")]
    Decode {
        /// Frame byte offset.
        offset: u64,
        /// Decoder failure.
        reason: String,
    },

    /// The configured segment bound would be exceeded.
    #[error("WAL capacity exhausted: required {required} bytes, available {available}")]
    CapacityExhausted {
        /// Bytes needed for the attempted operation.
        required: u64,
        /// Remaining bytes.
        available: u64,
    },

    /// A group commit was requested with no pending observations.
    #[error("WAL has no pending observations to commit")]
    NoPendingObservations,

    /// A segment seal was requested while observations were pending.
    #[error("WAL has pending observations that must be committed before sealing")]
    PendingObservations,

    /// No additional frames may be written after a segment seal.
    #[error("WAL segment is sealed")]
    Sealed,

    /// An append did not continue the session's total order.
    #[error("collector sequence mismatch: expected {expected}, got {actual}")]
    SequenceMismatch {
        /// Next required sequence.
        expected: u64,
        /// Supplied sequence.
        actual: u64,
    },

    /// The observation belongs to a different source session.
    #[error("observation source session does not match the WAL")]
    SourceSessionMismatch,

    /// A valid commit record covers a corrupt observation frame.
    #[error("committed WAL data is corrupt at offset {offset}")]
    CommittedCorruption {
        /// First corrupt byte range.
        offset: u64,
    },

    /// A structural frame or commit invariant failed.
    #[error("corrupt WAL frame at offset {offset}: {reason}")]
    CorruptFrame {
        /// Frame byte offset.
        offset: u64,
        /// Invariant failure.
        reason: String,
    },

    /// A corrupt segment could not be moved out of the active path.
    #[error("failed to quarantine WAL segment `{path:?}`: {source}")]
    QuarantineFailed {
        /// Intended quarantine path.
        path: PathBuf,
        /// Rename failure.
        #[source]
        source: io::Error,
    },

    /// The configured capacity cannot encode even an empty commit reserve.
    #[error("WAL maximum size {max_bytes} is too small")]
    InvalidCapacity {
        /// Configured segment size.
        max_bytes: u64,
    },
}

impl WalError {
    pub(crate) const fn requires_quarantine(&self) -> bool {
        matches!(
            self,
            Self::Decode { .. } | Self::CommittedCorruption { .. } | Self::CorruptFrame { .. }
        )
    }
}
