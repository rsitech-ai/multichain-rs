#![doc = "Validated construction of replay-stable source observations."]

use std::collections::HashMap;

use platform_proto::observation::Observation;
use sha2::{Digest, Sha256};
use thiserror::Error;

const SOURCE_SESSION_ID_LENGTH: usize = 16;
const OBSERVATION_ID_LENGTH: usize = 32;
const OBSERVATION_DOMAIN: &[u8] = b"observation/v1";

/// Supplies wall and monotonic timestamps without coupling domain logic to the
/// operating-system clock.
pub trait Clock: Send + Sync {
    /// Returns Unix time in nanoseconds.
    fn wall_time_unix_ns(&self) -> i64;

    /// Returns a process-local monotonic time in nanoseconds.
    fn monotonic_ns(&self) -> u64;
}

/// Identifies one uninterrupted connector process session.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceSessionId([u8; SOURCE_SESSION_ID_LENGTH]);

impl SourceSessionId {
    /// Returns the canonical fixed-length bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; SOURCE_SESSION_ID_LENGTH] {
        &self.0
    }
}

impl TryFrom<&[u8]> for SourceSessionId {
    type Error = ObservationError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let actual = value.len();
        let bytes = value
            .try_into()
            .map_err(|_| ObservationError::InvalidLength {
                field: "source_session_id",
                expected: SOURCE_SESSION_ID_LENGTH,
                actual,
            })?;
        Ok(Self(bytes))
    }
}

/// A replay-stable observation digest.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ObservationId([u8; OBSERVATION_ID_LENGTH]);

impl ObservationId {
    /// Returns the canonical fixed-length bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; OBSERVATION_ID_LENGTH] {
        &self.0
    }
}

impl TryFrom<&[u8]> for ObservationId {
    type Error = ObservationError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let actual = value.len();
        let bytes = value
            .try_into()
            .map_err(|_| ObservationError::InvalidLength {
                field: "observation_id",
                expected: OBSERVATION_ID_LENGTH,
                actual,
            })?;
        Ok(Self(bytes))
    }
}

/// Total-order position within a source session.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CollectorSequence(u64);

impl CollectorSequence {
    /// Constructs a sequence value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the underlying sequence value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Observation construction failures.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ObservationError {
    /// A required string was empty or whitespace-only.
    #[error("required observation field `{0}` is empty")]
    EmptyField(&'static str),

    /// A required builder field was not provided.
    #[error("required observation field `{0}` is missing")]
    MissingField(&'static str),

    /// A fixed-length identifier had the wrong size.
    #[error("field `{field}` must be {expected} bytes, got {actual}")]
    InvalidLength {
        /// Field name.
        field: &'static str,
        /// Required length.
        expected: usize,
        /// Supplied length.
        actual: usize,
    },
}

/// Constructs an immutable wire observation while enforcing identity rules.
#[derive(Clone, Debug, Default)]
pub struct ObservationBuilder {
    source_id: Option<String>,
    source_session_id: Option<SourceSessionId>,
    collector_sequence: Option<CollectorSequence>,
    chain: Option<String>,
    network: Option<String>,
    channel: Option<String>,
    source_message_type: Option<String>,
    source_sequence: Option<u64>,
    source_cursor: Option<Vec<u8>>,
    observed_at_unix_ns: Option<i64>,
    observed_at_monotonic_ns: Option<u64>,
    source_time_unix_ns: Option<i64>,
    payload: Option<Vec<u8>>,
}

impl ObservationBuilder {
    /// Creates an empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the stable observer identifier.
    #[must_use]
    pub fn source_id(mut self, value: impl Into<String>) -> Self {
        self.source_id = Some(value.into());
        self
    }

    /// Sets the connector session identifier.
    #[must_use]
    pub const fn source_session_id(mut self, value: SourceSessionId) -> Self {
        self.source_session_id = Some(value);
        self
    }

    /// Sets the total-order sequence within the connector session.
    #[must_use]
    pub const fn collector_sequence(mut self, value: CollectorSequence) -> Self {
        self.collector_sequence = Some(value);
        self
    }

    /// Sets the chain family.
    #[must_use]
    pub fn chain(mut self, value: impl Into<String>) -> Self {
        self.chain = Some(value.into());
        self
    }

    /// Sets the chain network.
    #[must_use]
    pub fn network(mut self, value: impl Into<String>) -> Self {
        self.network = Some(value.into());
        self
    }

    /// Sets the source channel.
    #[must_use]
    pub fn channel(mut self, value: impl Into<String>) -> Self {
        self.channel = Some(value.into());
        self
    }

    /// Sets the source-native message type.
    #[must_use]
    pub fn source_message_type(mut self, value: impl Into<String>) -> Self {
        self.source_message_type = Some(value.into());
        self
    }

    /// Sets an optional source-native sequence.
    #[must_use]
    pub const fn source_sequence(mut self, value: u64) -> Self {
        self.source_sequence = Some(value);
        self
    }

    /// Sets an optional opaque source cursor.
    #[must_use]
    pub fn source_cursor(mut self, value: impl AsRef<[u8]>) -> Self {
        self.source_cursor = Some(value.as_ref().to_vec());
        self
    }

    /// Sets the wall-clock observation time.
    #[must_use]
    pub const fn observed_at_unix_ns(mut self, value: i64) -> Self {
        self.observed_at_unix_ns = Some(value);
        self
    }

    /// Sets the monotonic observation time.
    #[must_use]
    pub const fn observed_at_monotonic_ns(mut self, value: u64) -> Self {
        self.observed_at_monotonic_ns = Some(value);
        self
    }

    /// Sets the optional source-provided time.
    #[must_use]
    pub const fn source_time_unix_ns(mut self, value: i64) -> Self {
        self.source_time_unix_ns = Some(value);
        self
    }

    /// Sets the exact source payload bytes.
    #[must_use]
    pub fn payload(mut self, value: impl AsRef<[u8]>) -> Self {
        self.payload = Some(value.as_ref().to_vec());
        self
    }

    /// Validates the builder and computes the payload and observation digests.
    ///
    /// # Errors
    ///
    /// Returns [`ObservationError`] when a required field is absent, empty, or
    /// has an invalid fixed length.
    pub fn build(self) -> Result<Observation, ObservationError> {
        let source_id = required_text(self.source_id, "source_id")?;
        let source_session_id = required(self.source_session_id, "source_session_id")?;
        let collector_sequence = required(self.collector_sequence, "collector_sequence")?;
        let chain = required_text(self.chain, "chain")?;
        let network = required_text(self.network, "network")?;
        let channel = required_text(self.channel, "channel")?;
        let source_message_type = required_text(self.source_message_type, "source_message_type")?;
        let observed_at_unix_ns = required(self.observed_at_unix_ns, "observed_at_unix_ns")?;
        let observed_at_monotonic_ns =
            required(self.observed_at_monotonic_ns, "observed_at_monotonic_ns")?;
        let payload = required(self.payload, "payload")?;

        let payload_hash: [u8; 32] = Sha256::digest(&payload).into();
        let mut observation_hasher = blake3::Hasher::new();
        observation_hasher.update(OBSERVATION_DOMAIN);
        observation_hasher.update(source_id.as_bytes());
        observation_hasher.update(source_session_id.as_bytes());
        observation_hasher.update(&collector_sequence.get().to_be_bytes());
        observation_hasher.update(&payload_hash);
        let observation_id = observation_hasher.finalize();

        Ok(Observation {
            schema_version: 1,
            observation_id: observation_id.as_bytes().to_vec(),
            source_id,
            source_session_id: source_session_id.as_bytes().to_vec(),
            collector_sequence: collector_sequence.get(),
            chain,
            network,
            channel,
            source_message_type,
            source_sequence: self.source_sequence,
            source_cursor: self.source_cursor,
            observed_at_unix_ns,
            observed_at_monotonic_ns,
            source_time_unix_ns: self.source_time_unix_ns,
            payload_hash: payload_hash.to_vec(),
            payload,
            attributes: HashMap::default(),
            quality_flags: Vec::new(),
        })
    }
}

fn required<T>(value: Option<T>, field: &'static str) -> Result<T, ObservationError> {
    value.ok_or(ObservationError::MissingField(field))
}

fn required_text(value: Option<String>, field: &'static str) -> Result<String, ObservationError> {
    let value = required(value, field)?;
    if value.trim().is_empty() {
        return Err(ObservationError::EmptyField(field));
    }
    Ok(value)
}
