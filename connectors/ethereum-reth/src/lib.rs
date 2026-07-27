#![doc = "Exact Reth execution notification capture and replay adapter."]

use std::str::FromStr as _;

use evm_canonicality::ExecutionBlock;
use evm_domain::B256;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use source_runtime::{HttpRequestSpec, SourceLoopError};
use thiserror::Error;

/// Validated Ethereum execution source configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RethSourceConfig {
    source_id: String,
    endpoint: String,
}

impl RethSourceConfig {
    /// Creates an Ethereum-only source configuration.
    ///
    /// # Errors
    ///
    /// Rejects blank identities, chain IDs other than 1, credentials embedded
    /// in URLs, and cleartext non-loopback endpoints.
    pub fn new(
        source_id: impl Into<String>,
        endpoint: impl Into<String>,
        chain_id: u64,
    ) -> Result<Self, RethConnectorError> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() || !source_id.is_ascii() {
            return Err(RethConnectorError::EmptySourceId);
        }
        if chain_id != 1 {
            return Err(RethConnectorError::WrongChainId(chain_id));
        }
        let endpoint = endpoint.into();
        validate_endpoint(&endpoint)?;
        Ok(Self {
            source_id,
            endpoint,
        })
    }

    /// Exact observer identity.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Validated Reth endpoint.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Builds the raw-first JSON-RPC polling cycle for chain identity and
    /// execution canonicality states.
    ///
    /// # Errors
    ///
    /// Rejects WebSocket-only endpoints because transport changes must remain
    /// explicit.
    pub fn http_poll_plan(&self) -> Result<Vec<HttpRequestSpec>, RethConnectorError> {
        let endpoint = self.endpoint.as_str();
        let block_params = |tag: &str| serde_json::json!([tag, false]);
        Ok(vec![
            HttpRequestSpec::json_rpc(
                endpoint,
                "json_rpc",
                "eth_chainId",
                "eth_chainId",
                &serde_json::json!([]),
                1,
            )?,
            HttpRequestSpec::json_rpc(
                endpoint,
                "json_rpc",
                "eth_getBlockByNumber.latest",
                "eth_getBlockByNumber",
                &block_params("latest"),
                2,
            )?,
            HttpRequestSpec::json_rpc(
                endpoint,
                "json_rpc",
                "eth_getBlockByNumber.safe",
                "eth_getBlockByNumber",
                &block_params("safe"),
                3,
            )?,
            HttpRequestSpec::json_rpc(
                endpoint,
                "json_rpc",
                "eth_getBlockByNumber.finalized",
                "eth_getBlockByNumber",
                &block_params("finalized"),
                4,
            )?,
        ])
    }
}

/// Execution change represented by a Reth ExEx/RPC notification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RethTransition {
    /// New canonical segment in ancestor-to-descendant order.
    Committed {
        /// New segment.
        new: Vec<ExecutionBlock>,
    },
    /// Old canonical suffix and its replacement.
    Reorged {
        /// Removed suffix in ancestor-to-descendant order.
        old: Vec<ExecutionBlock>,
        /// Replacement in ancestor-to-descendant order.
        new: Vec<ExecutionBlock>,
    },
    /// Reverted canonical suffix.
    Reverted {
        /// Removed suffix in ancestor-to-descendant order.
        old: Vec<ExecutionBlock>,
    },
}

/// Exact recorded Reth notification plus parsed transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedRethNotification {
    source_id: String,
    raw_json: Vec<u8>,
    raw_json_sha256: [u8; 32],
    transition: RethTransition,
}

impl RecordedRethNotification {
    /// Parses a bounded notification while retaining exact input bytes.
    ///
    /// # Errors
    ///
    /// Rejects blank source IDs, malformed JSON/quantities/hashes, unknown
    /// kinds, and empty transition sides.
    pub fn from_json(
        source_id: impl Into<String>,
        raw_json: &[u8],
    ) -> Result<Self, RethConnectorError> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() || !source_id.is_ascii() {
            return Err(RethConnectorError::EmptySourceId);
        }
        let raw: RawNotification =
            serde_json::from_slice(raw_json).map_err(RethConnectorError::InvalidJson)?;
        let transition = match raw {
            RawNotification::Committed { new } => RethTransition::Committed {
                new: parse_blocks(new)?,
            },
            RawNotification::Reorged { old, new } => RethTransition::Reorged {
                old: parse_blocks(old)?,
                new: parse_blocks(new)?,
            },
            RawNotification::Reverted { old } => RethTransition::Reverted {
                old: parse_blocks(old)?,
            },
        };
        match &transition {
            RethTransition::Committed { new } if new.is_empty() => {
                return Err(RethConnectorError::EmptyTransition);
            }
            RethTransition::Reorged { old, new } if old.is_empty() || new.is_empty() => {
                return Err(RethConnectorError::EmptyTransition);
            }
            RethTransition::Reverted { old } if old.is_empty() => {
                return Err(RethConnectorError::EmptyTransition);
            }
            _ => {}
        }
        let digest = Sha256::digest(raw_json);
        let mut raw_json_sha256 = [0_u8; 32];
        raw_json_sha256.copy_from_slice(&digest);
        Ok(Self {
            source_id,
            raw_json: raw_json.to_vec(),
            raw_json_sha256,
            transition,
        })
    }

    /// Exact recorded bytes.
    #[must_use]
    pub fn raw_json(&self) -> &[u8] {
        &self.raw_json
    }

    /// Parsed transition.
    #[must_use]
    pub const fn transition(&self) -> &RethTransition {
        &self.transition
    }

    /// Exact source identity.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// SHA-256 of the exact source bytes.
    #[must_use]
    pub const fn raw_json_sha256(&self) -> [u8; 32] {
        self.raw_json_sha256
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RawNotification {
    Committed {
        new: Vec<RawBlock>,
    },
    Reorged {
        old: Vec<RawBlock>,
        new: Vec<RawBlock>,
    },
    Reverted {
        old: Vec<RawBlock>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBlock {
    number: String,
    hash: String,
    #[serde(rename = "parentHash")]
    parent_hash: String,
}

/// Reth source/configuration boundary failure.
#[derive(Debug, Error)]
pub enum RethConnectorError {
    /// Source identity was blank.
    #[error("Reth source identity must not be empty")]
    EmptySourceId,
    /// Connector was configured for a non-Ethereum chain.
    #[error("Reth Ethereum connector requires chain ID 1, got {0}")]
    WrongChainId(u64),
    /// Endpoint exposed credentials or insecure remote cleartext.
    #[error("Reth endpoint is unsafe")]
    UnsafeEndpoint,
    /// Notification JSON was malformed.
    #[error("invalid Reth notification JSON: {0}")]
    InvalidJson(serde_json::Error),
    /// Block quantity was malformed.
    #[error("invalid Reth block quantity {0}")]
    InvalidQuantity(String),
    /// Hash was malformed.
    #[error("invalid Reth execution hash {0}")]
    InvalidHash(String),
    /// Transition had no blocks.
    #[error("Reth transition sides must not be empty")]
    EmptyTransition,
    /// HTTP polling configuration was invalid.
    #[error("invalid Reth HTTP polling plan: {0}")]
    HttpPlan(#[from] SourceLoopError),
}

fn parse_blocks(raw: Vec<RawBlock>) -> Result<Vec<ExecutionBlock>, RethConnectorError> {
    raw.into_iter()
        .map(|block| {
            Ok(ExecutionBlock::new(
                parse_quantity(&block.number)?,
                B256::from_str(&block.hash)
                    .map_err(|_| RethConnectorError::InvalidHash(block.hash))?,
                B256::from_str(&block.parent_hash)
                    .map_err(|_| RethConnectorError::InvalidHash(block.parent_hash))?,
            ))
        })
        .collect()
}

fn parse_quantity(value: &str) -> Result<u64, RethConnectorError> {
    let digits = value
        .strip_prefix("0x")
        .ok_or_else(|| RethConnectorError::InvalidQuantity(value.to_owned()))?;
    if digits.is_empty()
        || (digits.len() > 1 && digits.starts_with('0'))
        || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(RethConnectorError::InvalidQuantity(value.to_owned()));
    }
    u64::from_str_radix(digits, 16)
        .map_err(|_| RethConnectorError::InvalidQuantity(value.to_owned()))
}

fn validate_endpoint(endpoint: &str) -> Result<(), RethConnectorError> {
    if endpoint.contains('@') || endpoint.contains(char::is_whitespace) {
        return Err(RethConnectorError::UnsafeEndpoint);
    }
    let secure = endpoint.starts_with("https://") || endpoint.starts_with("wss://");
    let loopback = is_loopback_endpoint(endpoint, &["http://", "ws://"]);
    if endpoint.is_empty() || (!secure && !loopback) {
        return Err(RethConnectorError::UnsafeEndpoint);
    }
    Ok(())
}

fn is_loopback_endpoint(endpoint: &str, schemes: &[&str]) -> bool {
    schemes.iter().any(|scheme| {
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
    })
}
