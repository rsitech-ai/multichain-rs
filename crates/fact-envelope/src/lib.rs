#![doc = "Append-only normalized fact contract types."]

use platform_proto::fact::Fact;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable component identifier used by health and build metadata.
pub const COMPONENT_NAME: &str = "fact-envelope";
/// Stable parser identity recorded in every Phase 0 fixture fact.
pub const FIXTURE_PARSER_VERSION: &str = "fixture-native/1.0.0";
const FACT_KEY_DOMAIN: &[u8] = b"fixture/v1";
const FACT_ID_DOMAIN: &[u8] = b"fact/v1";

/// Exact JSON payload emitted by the deterministic Phase 0 source.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct FixturePayload {
    /// Stable fixture subject identifier.
    pub fixture_id: String,
    /// Exact source value used by the vertical-slice assertion.
    pub value: String,
    /// Source-native sequence carried in the payload.
    pub source_sequence: u64,
}

/// Query-safe fixture fact projection with its wire envelope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureFact {
    /// Append-only platform fact envelope.
    pub envelope: Fact,
    /// Decoded fixture payload.
    pub payload: FixturePayload,
    /// Lowercase hexadecimal fact identifier.
    pub fact_id_hex: String,
    /// Lowercase hexadecimal logical fact key.
    pub fact_key_hex: String,
    /// Lowercase hexadecimal input observation identifier.
    pub observation_id_hex: String,
    /// Stable source name.
    pub source_id: String,
    /// Connector session from the source observation.
    pub source_session_id: [u8; 16],
}

/// Reader-facing fixture fact and lineage projection.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct FixtureFactView {
    /// Stable fact identifier.
    pub fact_id: String,
    /// Stable logical fact key.
    pub fact_key: String,
    /// Append-only revision.
    pub revision: u64,
    /// Fixture subject identifier.
    pub fixture_id: String,
    /// Exact fixture value.
    pub value: String,
    /// Source-native sequence.
    pub source_sequence: u64,
    /// Chain family.
    pub chain: String,
    /// Chain network.
    pub network: String,
    /// Canonicality state.
    pub canonicality: String,
    /// Parser build identity.
    pub parser_version: String,
    /// Stable observer identity.
    pub source_id: String,
    /// Input lineage.
    pub lineage: FixtureLineage,
}

/// Minimal source lineage returned by REST and stream surfaces.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct FixtureLineage {
    /// Source observation digest.
    pub observation_id: String,
    /// Connector session bytes as lowercase hexadecimal.
    pub source_session_id: String,
}

/// Invalid deterministic fact construction.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum FactError {
    /// A fixed-length identity had an invalid byte count.
    #[error("{field} must contain {expected} bytes, got {actual}")]
    InvalidIdentityLength {
        /// Identity field.
        field: &'static str,
        /// Required length.
        expected: usize,
        /// Actual length.
        actual: usize,
    },
}

/// Computes the Phase 0 logical key.
#[must_use]
pub fn fixture_fact_key(fixture_id: &str) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(FACT_KEY_DOMAIN);
    hasher.update(fixture_id.as_bytes());
    *hasher.finalize().as_bytes()
}

/// Computes a replay-stable fact revision identity.
#[must_use]
pub fn fixture_fact_id(fact_key: [u8; 32], revision: u64, observation_id: [u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(FACT_ID_DOMAIN);
    hasher.update(&fact_key);
    hasher.update(&revision.to_be_bytes());
    hasher.update(&observation_id);
    *hasher.finalize().as_bytes()
}

/// Converts exact bytes to stable lowercase hexadecimal.
#[must_use]
pub fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}
