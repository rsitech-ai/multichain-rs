use thiserror::Error;

/// Fail-closed observer configuration error.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("source_id must not be empty")]
    EmptySourceId,
    #[error("{kind} endpoint must use a private numeric IP: {value}")]
    UnsafeEndpoint { kind: &'static str, value: String },
    #[error("wallet RPC must remain disabled")]
    WalletRpcEnabled,
    #[error("RPC cookie/secret reference is required")]
    MissingRpcSecret,
    #[error("bitcoin-core connector only supports mainnet or regtest")]
    UnsupportedNetwork,
    #[error("duplicate source_id `{0}`")]
    DuplicateSourceId(String),
    #[error("WAL path is shared by multiple observers: {0}")]
    SharedWalPath(String),
    #[error("production mainnet requires at least three observers, got {0}")]
    InsufficientProductionObservers(usize),
}

/// Invalid Bitcoin Core multipart notification.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ZmqFrameError {
    #[error("ZMQ notification must contain exactly topic, body, and sequence frames")]
    InvalidPartCount,
    #[error("unsupported ZMQ topic")]
    UnsupportedTopic,
    #[error("ZMQ topic frame is not valid ASCII")]
    InvalidTopic,
    #[error("ZMQ transport sequence must be four little-endian bytes")]
    InvalidSequence,
    #[error("ZMQ {topic} body is {actual} bytes; maximum is {maximum}")]
    BodyTooLarge {
        topic: String,
        actual: usize,
        maximum: usize,
    },
}

/// Allowlisted RPC failure.
#[derive(Debug, Error)]
pub enum RpcError {
    #[error("RPC secret could not be read: {0}")]
    Secret(#[source] std::io::Error),
    #[error("RPC transport failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("RPC request was cancelled")]
    Cancelled,
    #[error("RPC method `{method}` returned code {code}: {message}")]
    Remote {
        method: &'static str,
        code: i64,
        message: String,
    },
    #[error("RPC method `{method}` returned an invalid result: {message}")]
    InvalidResult {
        method: &'static str,
        message: String,
    },
}

/// Durable hot-path capture failure.
#[derive(Debug, Error)]
pub enum CaptureError {
    #[error(transparent)]
    Observation(#[from] observation_envelope::ObservationError),
    #[error(transparent)]
    Wal(#[from] wal::WalError),
    #[error("durable WAL range did not expose collector sequence {0}")]
    MissingCommittedObservation(u64),
}
