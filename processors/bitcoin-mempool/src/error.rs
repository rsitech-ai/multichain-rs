use thiserror::Error;

use bitcoin_domain::Txid;

/// Source-local mempool state validation failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MempoolError {
    /// Observer identity was blank.
    #[error("observer source_id must not be empty")]
    EmptySourceId,
    /// Atomic snapshot sequence moved backwards.
    #[error("snapshot sequence regressed from {current} to {attempted}")]
    SnapshotSequenceRegression {
        /// Last accepted sequence.
        current: u64,
        /// Rejected sequence.
        attempted: u64,
    },
    /// One snapshot sequence was associated with two different memberships.
    #[error("snapshot sequence {sequence} was replayed with different membership")]
    SnapshotSequenceConflict {
        /// Conflicting snapshot sequence.
        sequence: u64,
    },
    /// Requested package root is not currently present.
    #[error("transaction {txid} is absent from the source mempool")]
    UnknownTransaction {
        /// Missing transaction.
        txid: Txid,
    },
    /// Direct replacement evidence did not match a conflicting transaction.
    #[error("replacement evidence for {txid} does not name a known conflict")]
    InvalidReplacementEvidence {
        /// New transaction.
        txid: Txid,
    },
    /// Package fee sum exceeded `u64`.
    #[error("package fee sum overflowed")]
    FeeOverflow,
    /// Package virtual-size sum exceeded `u64`.
    #[error("package virtual-size sum overflowed")]
    VirtualSizeOverflow,
    /// Transaction virtual size could not fit the canonical representation.
    #[error("transaction {txid} virtual size exceeds u64")]
    TransactionTooLarge {
        /// Oversized transaction.
        txid: Txid,
    },
    /// A package has zero virtual size.
    #[error("package virtual size is zero")]
    ZeroVirtualSize,
}
