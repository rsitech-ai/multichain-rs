use std::future::Future;

use platform_proto::observation::CommittedObservation;
use thiserror::Error;

/// Versioned raw Bitcoin observation topic.
pub const RAW_BITCOIN_OBSERVATION_TOPIC: &str = "dev.raw.bitcoin.mainnet.source.observation.v1";

/// Durable broker acknowledgement for one contiguous source range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrokerAck {
    source_session_id: [u8; 16],
    last_collector_sequence: u64,
    record_count: u64,
}

impl BrokerAck {
    /// Constructs a validated acknowledgement.
    #[must_use]
    pub const fn new(
        source_session_id: [u8; 16],
        last_collector_sequence: u64,
        record_count: u64,
    ) -> Self {
        Self {
            source_session_id,
            last_collector_sequence,
            record_count,
        }
    }

    /// Returns the acknowledged source session.
    #[must_use]
    pub const fn source_session_id(&self) -> [u8; 16] {
        self.source_session_id
    }

    /// Returns the inclusive acknowledged collector sequence.
    #[must_use]
    pub const fn last_collector_sequence(&self) -> u64 {
        self.last_collector_sequence
    }

    /// Returns the number of acknowledged records.
    #[must_use]
    pub const fn record_count(&self) -> u64 {
        self.record_count
    }
}

/// Expected broker publication failures.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum BrokerError {
    /// No durable records were supplied.
    #[error("broker batch is empty")]
    EmptyBatch,
    /// A committed envelope was missing its observation.
    #[error("committed observation at index {index} has no observation")]
    MissingObservation {
        /// Zero-based record index.
        index: usize,
    },
    /// Records did not belong to one source session.
    #[error("broker batch contains mixed source sessions")]
    MixedSourceSession,
    /// A fixed-width identifier was malformed.
    #[error("field `{field}` must be {expected} bytes, got {actual}")]
    InvalidLength {
        /// Field name.
        field: &'static str,
        /// Expected byte count.
        expected: usize,
        /// Actual byte count.
        actual: usize,
    },
    /// The broker did not durably acknowledge a record.
    #[error("broker delivery failed: {0}")]
    Delivery(String),
}

/// Publishes only WAL-committed observations to the durable event log.
pub trait BrokerPublisher: Send + Sync {
    /// Publishes a contiguous batch with the source ID as record key and the
    /// observation ID as the deterministic application event ID.
    fn publish(
        &self,
        topic: &str,
        records: &[CommittedObservation],
    ) -> impl Future<Output = Result<BrokerAck, BrokerError>> + Send;
}
