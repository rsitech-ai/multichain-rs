#![doc = "Exact Ethereum consensus checkpoint capture and payload-hash adapter."]

use std::str::FromStr as _;

use evm_canonicality::{EthereumCheckpoint, EthereumError};
use evm_domain::B256;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use source_runtime::{HttpRequestSpec, SourceLoopError};
use thiserror::Error;

/// Validated consensus-client source configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusSourceConfig {
    source_id: String,
    endpoint: String,
}

impl ConsensusSourceConfig {
    /// Creates a safe source configuration.
    ///
    /// # Errors
    ///
    /// Rejects blank source identity, embedded credentials, and insecure
    /// non-loopback endpoints.
    pub fn new(
        source_id: impl Into<String>,
        endpoint: impl Into<String>,
    ) -> Result<Self, ConsensusConnectorError> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() || !source_id.is_ascii() {
            return Err(ConsensusConnectorError::EmptySourceId);
        }
        let endpoint = endpoint.into();
        validate_endpoint(&endpoint)?;
        Ok(Self {
            source_id,
            endpoint,
        })
    }

    /// Exact source identity.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Validated consensus REST endpoint.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Builds independent raw Beacon API requests for head and finalized
    /// blocks.
    ///
    /// # Errors
    ///
    /// Returns an error if the validated base endpoint cannot form an HTTP
    /// polling request.
    pub fn http_poll_plan(&self) -> Result<Vec<HttpRequestSpec>, ConsensusConnectorError> {
        let base = self.endpoint.trim_end_matches('/');
        Ok(vec![
            HttpRequestSpec::get(
                format!("{base}/eth/v2/beacon/blocks/head"),
                "beacon_api",
                "beacon_block.head",
            )?,
            HttpRequestSpec::get(
                format!("{base}/eth/v2/beacon/blocks/finalized"),
                "beacon_api",
                "beacon_block.finalized",
            )?,
        ])
    }

    /// Creates an independent monotonic slot cursor.
    #[must_use]
    pub const fn cursor(&self) -> ConsensusCursor {
        ConsensusCursor { last_slot: None }
    }
}

/// Exact consensus update plus parsed payload hashes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedConsensusCheckpoint {
    source_id: String,
    slot: u64,
    head: B256,
    safe: B256,
    finalized: B256,
    raw_json: Vec<u8>,
    raw_json_sha256: [u8; 32],
}

impl RecordedConsensusCheckpoint {
    /// Parses an exact consensus source response.
    ///
    /// # Errors
    ///
    /// Rejects blank sources, malformed JSON, noncanonical slots, and invalid
    /// execution payload hashes.
    pub fn from_json(
        source_id: impl Into<String>,
        raw_json: &[u8],
    ) -> Result<Self, ConsensusConnectorError> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() || !source_id.is_ascii() {
            return Err(ConsensusConnectorError::EmptySourceId);
        }
        let raw: RawCheckpoint =
            serde_json::from_slice(raw_json).map_err(ConsensusConnectorError::InvalidJson)?;
        let digest = Sha256::digest(raw_json);
        let mut raw_json_sha256 = [0_u8; 32];
        raw_json_sha256.copy_from_slice(&digest);
        Ok(Self {
            source_id,
            slot: parse_quantity(&raw.slot)?,
            head: parse_hash(raw.head_execution_payload_hash)?,
            safe: parse_hash(raw.safe_execution_payload_hash)?,
            finalized: parse_hash(raw.finalized_execution_payload_hash)?,
            raw_json: raw_json.to_vec(),
            raw_json_sha256,
        })
    }

    /// Exact source bytes.
    #[must_use]
    pub fn raw_json(&self) -> &[u8] {
        &self.raw_json
    }

    /// Consensus slot.
    #[must_use]
    pub const fn slot(&self) -> u64 {
        self.slot
    }

    /// Builds typed evidence for the canonicality join.
    ///
    /// # Errors
    ///
    /// Propagates the source identity invariant from the canonicality domain.
    pub fn checkpoint(&self) -> Result<EthereumCheckpoint, EthereumError> {
        EthereumCheckpoint::new(self.head, self.safe, self.finalized, self.source_id.clone())
    }

    /// SHA-256 of exact source bytes.
    #[must_use]
    pub const fn raw_json_sha256(&self) -> [u8; 32] {
        self.raw_json_sha256
    }
}

/// Per-source monotonic consensus slot cursor.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConsensusCursor {
    last_slot: Option<u64>,
}

impl ConsensusCursor {
    /// Advances after validating strict slot monotonicity.
    ///
    /// # Errors
    ///
    /// Rejects duplicates and regressions so callers explicitly deduplicate
    /// observation IDs before state application.
    pub fn observe(
        &mut self,
        checkpoint: &RecordedConsensusCheckpoint,
    ) -> Result<(), ConsensusConnectorError> {
        if let Some(last_slot) = self.last_slot
            && checkpoint.slot <= last_slot
        {
            return Err(ConsensusConnectorError::SlotRegression {
                previous: last_slot,
                observed: checkpoint.slot,
            });
        }
        self.last_slot = Some(checkpoint.slot);
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCheckpoint {
    slot: String,
    #[serde(rename = "headExecutionPayloadHash")]
    head_execution_payload_hash: String,
    #[serde(rename = "safeExecutionPayloadHash")]
    safe_execution_payload_hash: String,
    #[serde(rename = "finalizedExecutionPayloadHash")]
    finalized_execution_payload_hash: String,
}

/// Consensus connector boundary failure.
#[derive(Debug, Error)]
pub enum ConsensusConnectorError {
    /// Source identity was blank.
    #[error("consensus source identity must not be empty")]
    EmptySourceId,
    /// Endpoint exposed credentials or insecure remote cleartext.
    #[error("consensus endpoint is unsafe")]
    UnsafeEndpoint,
    /// JSON payload was malformed.
    #[error("invalid consensus checkpoint JSON: {0}")]
    InvalidJson(serde_json::Error),
    /// Slot quantity was noncanonical.
    #[error("invalid consensus slot quantity {0}")]
    InvalidQuantity(String),
    /// Execution payload hash was malformed.
    #[error("invalid consensus execution payload hash {0}")]
    InvalidHash(String),
    /// Slot did not advance.
    #[error("consensus slot regressed from {previous} to {observed}")]
    SlotRegression {
        /// Previously accepted slot.
        previous: u64,
        /// Observed slot.
        observed: u64,
    },
    /// HTTP polling configuration was invalid.
    #[error("invalid consensus HTTP polling plan: {0}")]
    HttpPlan(#[from] SourceLoopError),
}

fn parse_quantity(value: &str) -> Result<u64, ConsensusConnectorError> {
    let digits = value
        .strip_prefix("0x")
        .ok_or_else(|| ConsensusConnectorError::InvalidQuantity(value.to_owned()))?;
    if digits.is_empty()
        || (digits.len() > 1 && digits.starts_with('0'))
        || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ConsensusConnectorError::InvalidQuantity(value.to_owned()));
    }
    u64::from_str_radix(digits, 16)
        .map_err(|_| ConsensusConnectorError::InvalidQuantity(value.to_owned()))
}

fn parse_hash(value: String) -> Result<B256, ConsensusConnectorError> {
    B256::from_str(&value).map_err(|_| ConsensusConnectorError::InvalidHash(value))
}

fn validate_endpoint(endpoint: &str) -> Result<(), ConsensusConnectorError> {
    if endpoint.contains('@') || endpoint.contains(char::is_whitespace) {
        return Err(ConsensusConnectorError::UnsafeEndpoint);
    }
    let secure = endpoint.starts_with("https://");
    let loopback = endpoint.strip_prefix("http://").is_some_and(|rest| {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let host = if let Some(ipv6) = authority.strip_prefix('[') {
            ipv6.split_once(']').map_or("", |(host, _)| host)
        } else {
            authority
                .split_once(':')
                .map_or(authority, |(host, _)| host)
        };
        matches!(host, "127.0.0.1" | "localhost" | "::1")
    });
    if endpoint.is_empty() || (!secure && !loopback) {
        return Err(ConsensusConnectorError::UnsafeEndpoint);
    }
    Ok(())
}
