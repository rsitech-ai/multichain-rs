#![doc = "Shared durable raw-observation capture boundary."]

use std::sync::Arc;

use observation_envelope::{
    Clock, CollectorSequence, ObservationBuilder, ObservationError, SourceSessionId,
};
use platform_proto::observation::CommittedObservation;
use thiserror::Error;
use wal::{ObservationWal, UnframedObservation, WalError};

const MAX_IDENTITY_BYTES: usize = 128;
const MAX_CHAIN_BYTES: usize = 32;
const MAX_NETWORK_BYTES: usize = 64;
const MAX_MESSAGE_FIELD_BYTES: usize = 128;
const MAX_SOURCE_CURSOR_BYTES: usize = 1_024;
const MAX_RAW_SOURCE_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;

/// Stable identity copied into every source observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceIdentity {
    source_id: String,
    chain: String,
    network: String,
}

impl SourceIdentity {
    /// Constructs a bounded printable source/chain/network identity.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::InvalidIdentity`] for an invalid field.
    pub fn new(
        source_id: impl Into<String>,
        chain: impl Into<String>,
        network: impl Into<String>,
    ) -> Result<Self, CaptureError> {
        let source_id = source_id.into();
        let chain = chain.into();
        let network = network.into();
        validate_graphic_ascii(&source_id, MAX_IDENTITY_BYTES, "source_id", true)?;
        validate_graphic_ascii(&chain, MAX_CHAIN_BYTES, "chain", true)?;
        validate_graphic_ascii(&network, MAX_NETWORK_BYTES, "network", true)?;
        Ok(Self {
            source_id,
            chain,
            network,
        })
    }

    /// Exact configured observer identity.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Exact chain family.
    #[must_use]
    pub fn chain(&self) -> &str {
        &self.chain
    }

    /// Exact chain network.
    #[must_use]
    pub fn network(&self) -> &str {
        &self.network
    }
}

/// One uninterrupted source connection and its next total-order position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureSession {
    id: SourceSessionId,
    next_sequence: u64,
}

impl CaptureSession {
    /// Creates a deterministic fresh session for tests and restored runtime
    /// configuration.
    ///
    /// # Panics
    ///
    /// The fixed array always satisfies the 16-byte session contract.
    #[must_use]
    pub fn with_id(id: [u8; 16]) -> Self {
        Self {
            id: SourceSessionId::try_from(id.as_slice()).expect("fixed source session ID"),
            next_sequence: 0,
        }
    }

    /// Restores a source session at the next sequence proven by WAL recovery.
    ///
    /// The caller must use the exact session ID supplied to the recovered WAL.
    ///
    /// # Panics
    ///
    /// The fixed array always satisfies the 16-byte session contract.
    #[must_use]
    pub fn resume(id: [u8; 16], next_sequence: u64) -> Self {
        Self {
            id: SourceSessionId::try_from(id.as_slice()).expect("fixed source session ID"),
            next_sequence,
        }
    }

    /// Creates a fresh session from operating-system entropy.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::Entropy`] without using a weaker fallback.
    pub fn new() -> Result<Self, CaptureError> {
        let mut id = [0_u8; 16];
        getrandom::fill(&mut id).map_err(|error| CaptureError::Entropy(error.to_string()))?;
        Ok(Self::with_id(id))
    }

    /// Fixed source-session identity.
    #[must_use]
    pub const fn id(self) -> SourceSessionId {
        self.id
    }

    /// Next collector sequence that a successful commit will consume.
    #[must_use]
    pub const fn next_sequence(self) -> u64 {
        self.next_sequence
    }

    fn pending_sequence(self) -> Result<CollectorSequence, CaptureError> {
        if self.next_sequence == u64::MAX {
            return Err(CaptureError::SequenceExhausted);
        }
        Ok(CollectorSequence::new(self.next_sequence))
    }

    fn commit_sequence(&mut self) {
        self.next_sequence = self.next_sequence.saturating_add(1);
    }
}

/// Exact source message presented to the durable capture boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawSourceMessage {
    channel: String,
    source_message_type: String,
    payload: Vec<u8>,
    source_sequence: Option<u64>,
    source_cursor: Option<Vec<u8>>,
    source_time_unix_ns: Option<i64>,
}

impl RawSourceMessage {
    /// Creates a message while validating its chain-native channel/type labels.
    ///
    /// The shared hard payload bound is checked before copying the bytes. The
    /// capture engine applies its usually smaller source-specific runtime
    /// limit before writing.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::InvalidMessageField`] for blank, non-printable,
    /// or unbounded labels, and [`CaptureError::PayloadTooLarge`] for a payload
    /// above the shared hard limit.
    pub fn new(
        channel: impl Into<String>,
        source_message_type: impl Into<String>,
        payload: impl AsRef<[u8]>,
    ) -> Result<Self, CaptureError> {
        let channel = channel.into();
        let source_message_type = source_message_type.into();
        validate_graphic_ascii(&channel, MAX_MESSAGE_FIELD_BYTES, "channel", false)?;
        validate_graphic_ascii(
            &source_message_type,
            MAX_MESSAGE_FIELD_BYTES,
            "source_message_type",
            false,
        )?;
        let payload = payload.as_ref();
        if payload.len() > MAX_RAW_SOURCE_PAYLOAD_BYTES {
            return Err(CaptureError::PayloadTooLarge {
                actual: payload.len(),
                max: MAX_RAW_SOURCE_PAYLOAD_BYTES,
            });
        }
        Ok(Self {
            channel,
            source_message_type,
            payload: payload.to_vec(),
            source_sequence: None,
            source_cursor: None,
            source_time_unix_ns: None,
        })
    }

    /// Adds an optional source-native sequence.
    #[must_use]
    pub const fn with_source_sequence(mut self, source_sequence: u64) -> Self {
        self.source_sequence = Some(source_sequence);
        self
    }

    /// Adds an optional opaque reconnect/replay cursor after checking its hard
    /// bound before copying.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::SourceCursorTooLarge`] when the cursor exceeds
    /// the shared limit.
    pub fn with_source_cursor(
        mut self,
        source_cursor: impl AsRef<[u8]>,
    ) -> Result<Self, CaptureError> {
        let source_cursor = source_cursor.as_ref();
        if source_cursor.len() > MAX_SOURCE_CURSOR_BYTES {
            return Err(CaptureError::SourceCursorTooLarge);
        }
        self.source_cursor = Some(source_cursor.to_vec());
        Ok(self)
    }

    /// Adds an optional source-provided time.
    #[must_use]
    pub const fn with_source_time_unix_ns(mut self, source_time_unix_ns: i64) -> Self {
        self.source_time_unix_ns = Some(source_time_unix_ns);
        self
    }
}

/// Single-writer raw source capture that exposes only WAL-committed records.
pub struct DurableSourceCapture<W> {
    identity: SourceIdentity,
    session: CaptureSession,
    clock: Arc<dyn Clock>,
    wal: W,
    max_payload_bytes: usize,
    poisoned: bool,
}

impl<W: ObservationWal> DurableSourceCapture<W> {
    /// Binds one source session to one WAL writer.
    ///
    /// # Errors
    ///
    /// Rejects a zero payload bound.
    pub fn new(
        identity: SourceIdentity,
        session: CaptureSession,
        clock: Arc<dyn Clock>,
        wal: W,
        max_payload_bytes: usize,
    ) -> Result<Self, CaptureError> {
        if max_payload_bytes == 0 {
            return Err(CaptureError::InvalidPayloadLimit);
        }
        Ok(Self {
            identity,
            session,
            clock,
            wal,
            max_payload_bytes,
            poisoned: false,
        })
    }

    /// Makes one exact source message durable and returns only its committed
    /// envelope.
    ///
    /// Any WAL failure poisons this writer until the caller takes its parts and
    /// performs explicit recovery. Validation failures do not consume a
    /// collector sequence.
    ///
    /// # Errors
    ///
    /// Returns validation, ordering, observation, or WAL failures.
    pub fn capture(
        &mut self,
        message: RawSourceMessage,
    ) -> Result<CommittedObservation, CaptureError> {
        if self.poisoned {
            return Err(CaptureError::CaptureUnavailable);
        }
        if message.payload.len() > self.max_payload_bytes {
            return Err(CaptureError::PayloadTooLarge {
                actual: message.payload.len(),
                max: self.max_payload_bytes,
            });
        }
        if message
            .source_cursor
            .as_ref()
            .is_some_and(|cursor| cursor.len() > MAX_SOURCE_CURSOR_BYTES)
        {
            return Err(CaptureError::SourceCursorTooLarge);
        }
        let collector_sequence = self.session.pending_sequence()?;
        let mut builder = ObservationBuilder::new()
            .source_id(&self.identity.source_id)
            .source_session_id(self.session.id())
            .collector_sequence(collector_sequence)
            .chain(&self.identity.chain)
            .network(&self.identity.network)
            .channel(message.channel)
            .source_message_type(message.source_message_type)
            .observed_at_unix_ns(self.clock.wall_time_unix_ns())
            .observed_at_monotonic_ns(self.clock.monotonic_ns())
            .payload(message.payload);
        if let Some(source_sequence) = message.source_sequence {
            builder = builder.source_sequence(source_sequence);
        }
        if let Some(source_cursor) = message.source_cursor {
            builder = builder.source_cursor(source_cursor);
        }
        if let Some(source_time_unix_ns) = message.source_time_unix_ns {
            builder = builder.source_time_unix_ns(source_time_unix_ns);
        }
        let observation = builder.build()?;
        let pending = match self.wal.append(UnframedObservation::new(observation)) {
            Ok(pending) => pending,
            Err(error) => {
                self.poisoned = true;
                return Err(CaptureError::Wal(error));
            }
        };
        let committed = match self.wal.group_commit() {
            Ok(committed) => committed,
            Err(error) => {
                self.poisoned = true;
                return Err(CaptureError::Wal(error));
            }
        };
        if committed.first_sequence != collector_sequence
            || committed.last_sequence != collector_sequence
        {
            self.poisoned = true;
            return Err(CaptureError::CommitRangeMismatch {
                expected: collector_sequence.get(),
                first: committed.first_sequence.get(),
                last: committed.last_sequence.get(),
            });
        }
        self.session.commit_sequence();
        Ok(CommittedObservation {
            observation: Some(pending.observation),
            durable_at_unix_ns: committed.durable_at_unix_ns,
            wal_commit_hash: committed.commit_hash.to_vec(),
        })
    }

    /// Returns whether an ambiguous WAL failure requires explicit recovery.
    #[must_use]
    pub const fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    /// Returns the current source session.
    #[must_use]
    pub const fn session(&self) -> CaptureSession {
        self.session
    }

    /// Releases the session and WAL for shutdown/recovery.
    pub fn into_parts(self) -> (CaptureSession, W) {
        (self.session, self.wal)
    }
}

/// Durable source-capture boundary failure.
#[derive(Debug, Error)]
pub enum CaptureError {
    /// Stable identity was absent, non-printable, or too long.
    #[error("invalid source identity field `{field}`")]
    InvalidIdentity {
        /// Invalid identity field.
        field: &'static str,
    },
    /// Message channel/type was absent, non-printable, or too long.
    #[error("invalid raw source message field `{field}`")]
    InvalidMessageField {
        /// Invalid message field.
        field: &'static str,
    },
    /// Runtime payload limit must be positive.
    #[error("raw source payload limit must be positive")]
    InvalidPayloadLimit,
    /// Exact payload exceeded the source-specific bound.
    #[error("raw source payload has {actual} bytes, limit is {max}")]
    PayloadTooLarge {
        /// Exact supplied bytes.
        actual: usize,
        /// Configured limit.
        max: usize,
    },
    /// Opaque cursor exceeded the shared bound.
    #[error("raw source cursor exceeds 1024 bytes")]
    SourceCursorTooLarge,
    /// The session cannot allocate another unique order position.
    #[error("source collector sequence is exhausted")]
    SequenceExhausted,
    /// Operating-system entropy failed.
    #[error("source session entropy failed: {0}")]
    Entropy(String),
    /// Observation construction failed.
    #[error(transparent)]
    Observation(#[from] ObservationError),
    /// WAL append, commit, or committed scan failed.
    #[error("source capture WAL failed: {0}")]
    Wal(#[from] WalError),
    /// The WAL durability proof did not cover exactly the appended observation.
    #[error(
        "source capture WAL committed unexpected sequence range {first}..={last}; expected {expected}"
    )]
    CommitRangeMismatch {
        /// Collector sequence assigned to the appended observation.
        expected: u64,
        /// First sequence covered by the returned durability proof.
        first: u64,
        /// Last sequence covered by the returned durability proof.
        last: u64,
    },
    /// A durable failure requires explicit recovery before more writes.
    #[error("source capture is unavailable until WAL recovery")]
    CaptureUnavailable,
}

fn validate_graphic_ascii(
    value: &str,
    max_bytes: usize,
    field: &'static str,
    identity: bool,
) -> Result<(), CaptureError> {
    if value.is_empty()
        || value.len() > max_bytes
        || !value.bytes().all(|byte| byte.is_ascii_graphic())
    {
        if identity {
            Err(CaptureError::InvalidIdentity { field })
        } else {
            Err(CaptureError::InvalidMessageField { field })
        }
    } else {
        Ok(())
    }
}
