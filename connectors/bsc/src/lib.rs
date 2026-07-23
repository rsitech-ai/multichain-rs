#![doc = "Official BNB Smart Chain node observation adapter."]

use std::str::FromStr as _;

use evm_canonicality::ExecutionBlock;
use evm_domain::B256;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

/// Client family configured behind the BSC connector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BscNodeKind {
    /// Official `bnb-chain/bsc` client.
    OfficialBsc,
    /// Generic Ethereum execution client, rejected for BSC truth.
    GenericEthereum,
}

/// Validated official BSC node configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BscNodeConfig {
    source_id: String,
    endpoint: String,
}

impl BscNodeConfig {
    /// Creates a BSC mainnet source.
    ///
    /// # Errors
    ///
    /// Requires chain ID 56, the official BSC client kind, a non-empty source
    /// identity, and a safe endpoint.
    pub fn new(
        source_id: impl Into<String>,
        endpoint: impl Into<String>,
        chain_id: u64,
        node_kind: BscNodeKind,
    ) -> Result<Self, BscConnectorError> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() || !source_id.is_ascii() {
            return Err(BscConnectorError::EmptySourceId);
        }
        if chain_id != 56 {
            return Err(BscConnectorError::WrongChainId(chain_id));
        }
        if node_kind != BscNodeKind::OfficialBsc {
            return Err(BscConnectorError::WrongClient);
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

    /// Validated official-node endpoint.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

/// Exact paired head and BSC-native finalized-tag response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedBscHeads {
    source_id: String,
    chain_id: u64,
    head: ExecutionBlock,
    finalized: ExecutionBlock,
    observed_at_unix_ms: u64,
    raw_json: Vec<u8>,
    raw_json_sha256: [u8; 32],
}

impl RecordedBscHeads {
    /// Parses exact official-node evidence.
    ///
    /// # Errors
    ///
    /// Rejects wrong chain/client identity, malformed quantities/hashes/JSON,
    /// blank sources, and a finalized height above the observed head.
    pub fn from_json(
        source_id: impl Into<String>,
        raw_json: &[u8],
    ) -> Result<Self, BscConnectorError> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() || !source_id.is_ascii() {
            return Err(BscConnectorError::EmptySourceId);
        }
        let raw: RawHeads =
            serde_json::from_slice(raw_json).map_err(BscConnectorError::InvalidJson)?;
        let chain_id = parse_quantity(&raw.chain_id)?;
        if chain_id != 56 {
            return Err(BscConnectorError::WrongChainId(chain_id));
        }
        if raw.client != "bnb-chain/bsc" {
            return Err(BscConnectorError::WrongClient);
        }
        let head = parse_block(raw.head)?;
        let finalized = parse_block(raw.finalized)?;
        if finalized.number > head.number {
            return Err(BscConnectorError::FinalizedAboveHead);
        }
        if raw.observed_at_unix_ms == 0 {
            return Err(BscConnectorError::InvalidObservationTime);
        }
        let digest = Sha256::digest(raw_json);
        let mut raw_json_sha256 = [0_u8; 32];
        raw_json_sha256.copy_from_slice(&digest);
        Ok(Self {
            source_id,
            chain_id,
            head,
            finalized,
            observed_at_unix_ms: raw.observed_at_unix_ms,
            raw_json: raw_json.to_vec(),
            raw_json_sha256,
        })
    }

    /// Exact source bytes.
    #[must_use]
    pub fn raw_json(&self) -> &[u8] {
        &self.raw_json
    }

    /// Exact official-node source identity.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Enforced chain ID.
    #[must_use]
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Observed canonical head.
    #[must_use]
    pub const fn head(&self) -> ExecutionBlock {
        self.head
    }

    /// Observed BSC-native finalized block.
    #[must_use]
    pub const fn finalized(&self) -> ExecutionBlock {
        self.finalized
    }

    /// Local platform observation time.
    #[must_use]
    pub const fn observed_at_unix_ms(&self) -> u64 {
        self.observed_at_unix_ms
    }

    /// SHA-256 of exact source bytes.
    #[must_use]
    pub const fn raw_json_sha256(&self) -> [u8; 32] {
        self.raw_json_sha256
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHeads {
    #[serde(rename = "chainId")]
    chain_id: String,
    client: String,
    head: RawBlock,
    finalized: RawBlock,
    #[serde(rename = "observedAtUnixMs")]
    observed_at_unix_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBlock {
    number: String,
    hash: String,
    #[serde(rename = "parentHash")]
    parent_hash: String,
}

/// BSC connector boundary failure.
#[derive(Debug, Error)]
pub enum BscConnectorError {
    /// Source identity was blank.
    #[error("BSC source identity must not be empty")]
    EmptySourceId,
    /// Chain ID was not 56.
    #[error("BSC connector requires chain ID 56, got {0}")]
    WrongChainId(u64),
    /// Client was not official `bnb-chain/bsc`.
    #[error("BSC connector requires official bnb-chain/bsc client semantics")]
    WrongClient,
    /// Endpoint was unsafe.
    #[error("BSC endpoint is unsafe")]
    UnsafeEndpoint,
    /// JSON was malformed.
    #[error("invalid BSC observation JSON: {0}")]
    InvalidJson(serde_json::Error),
    /// Quantity was malformed.
    #[error("invalid BSC quantity {0}")]
    InvalidQuantity(String),
    /// Block hash was malformed.
    #[error("invalid BSC block hash {0}")]
    InvalidHash(String),
    /// Finalized height exceeded head height.
    #[error("BSC finalized height is above observed head")]
    FinalizedAboveHead,
    /// Observation time was invalid.
    #[error("BSC observation time must be positive")]
    InvalidObservationTime,
}

fn parse_block(raw: RawBlock) -> Result<ExecutionBlock, BscConnectorError> {
    Ok(ExecutionBlock::new(
        parse_quantity(&raw.number)?,
        B256::from_str(&raw.hash).map_err(|_| BscConnectorError::InvalidHash(raw.hash))?,
        B256::from_str(&raw.parent_hash)
            .map_err(|_| BscConnectorError::InvalidHash(raw.parent_hash))?,
    ))
}

fn parse_quantity(value: &str) -> Result<u64, BscConnectorError> {
    let digits = value
        .strip_prefix("0x")
        .ok_or_else(|| BscConnectorError::InvalidQuantity(value.to_owned()))?;
    if digits.is_empty()
        || (digits.len() > 1 && digits.starts_with('0'))
        || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(BscConnectorError::InvalidQuantity(value.to_owned()));
    }
    u64::from_str_radix(digits, 16)
        .map_err(|_| BscConnectorError::InvalidQuantity(value.to_owned()))
}

fn validate_endpoint(endpoint: &str) -> Result<(), BscConnectorError> {
    if endpoint.contains('@') || endpoint.contains(char::is_whitespace) {
        return Err(BscConnectorError::UnsafeEndpoint);
    }
    let secure = endpoint.starts_with("https://") || endpoint.starts_with("wss://");
    let loopback = ["http://", "ws://"].iter().any(|scheme| {
        endpoint.strip_prefix(scheme).is_some_and(|rest| {
            let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
            let host = if let Some(ipv6) = authority.strip_prefix('[') {
                ipv6.split_once(']').map_or("", |(host, _)| host)
            } else {
                authority
                    .split_once(':')
                    .map_or(authority, |(host, _)| host)
            };
            matches!(host, "127.0.0.1" | "localhost" | "::1")
        })
    });
    if endpoint.is_empty() || (!secure && !loopback) {
        return Err(BscConnectorError::UnsafeEndpoint);
    }
    Ok(())
}
