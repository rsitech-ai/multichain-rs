use thiserror::Error;

/// Bitcoin consensus parsing and bounded-domain failures.
#[derive(Debug, Error)]
pub enum ParseError {
    /// Input exceeded the explicit parser allocation boundary.
    #[error("{kind} input is {actual} bytes; maximum is {maximum}")]
    InputTooLarge {
        /// Parsed object kind.
        kind: &'static str,
        /// Received bytes.
        actual: usize,
        /// Configured boundary.
        maximum: usize,
    },
    /// Consensus decoding failed or left invalid structure.
    #[error("bitcoin consensus decode failed: {0}")]
    Consensus(#[from] bitcoin::consensus::encode::Error),
    /// A block's header merkle root did not match its transactions.
    #[error("block merkle root does not match its transactions")]
    MerkleRootMismatch,
    /// Adding output values overflowed `u64`.
    #[error("transaction output amount overflow")]
    AmountOverflow,
    /// An outpoint was not the fixed consensus length.
    #[error("outpoint encoding must be 36 bytes, got {0}")]
    InvalidOutpointLength(usize),
}
