#![doc = "Versioned EVM decoder selection isolated from immutable native facts."]

use std::fmt::Write as _;

use evm_domain::Address;
use serde::Serialize;
use thiserror::Error;

/// One ABI/decoder deployment over an inclusive historical block range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecoderDeployment {
    chain_id: u64,
    address: Address,
    valid_from_block: u64,
    valid_to_block: Option<u64>,
    version: String,
}

impl DecoderDeployment {
    /// Creates a deployment boundary.
    ///
    /// # Errors
    ///
    /// Rejects unsupported chains, blank versions, and inverted ranges.
    pub fn new(
        chain_id: u64,
        address: Address,
        valid_from_block: u64,
        valid_to_block: Option<u64>,
        version: impl Into<String>,
    ) -> Result<Self, DecoderError> {
        validate_chain(chain_id)?;
        if valid_to_block.is_some_and(|end| end < valid_from_block) {
            return Err(DecoderError::InvalidDeploymentRange);
        }
        let version = version.into();
        if version.trim().is_empty() {
            return Err(DecoderError::EmptyVersion);
        }
        Ok(Self {
            chain_id,
            address,
            valid_from_block,
            valid_to_block,
            version,
        })
    }

    /// Stable deployment version.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    fn contains(&self, block_number: u64) -> bool {
        block_number >= self.valid_from_block
            && self.valid_to_block.is_none_or(|end| block_number <= end)
    }

    fn overlaps(&self, other: &Self) -> bool {
        if self.chain_id != other.chain_id || self.address != other.address {
            return false;
        }
        let self_end = self.valid_to_block.unwrap_or(u64::MAX);
        let other_end = other.valid_to_block.unwrap_or(u64::MAX);
        self.valid_from_block <= other_end && other.valid_from_block <= self_end
    }
}

/// Immutable native subject presented to a separately replayable decoder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeDecodeSubject {
    chain_id: u64,
    address: Address,
    block_number: u64,
    native_fact_id: String,
    selector: [u8; 4],
    raw_data: Vec<u8>,
}

impl NativeDecodeSubject {
    /// Creates a decode subject without interpreting the raw bytes.
    ///
    /// # Errors
    ///
    /// Rejects unsupported chains and blank native fact identity.
    pub fn new(
        chain_id: u64,
        address: Address,
        block_number: u64,
        native_fact_id: impl Into<String>,
        selector: [u8; 4],
        raw_data: Vec<u8>,
    ) -> Result<Self, DecoderError> {
        validate_chain(chain_id)?;
        let native_fact_id = native_fact_id.into();
        if native_fact_id.trim().is_empty() {
            return Err(DecoderError::EmptyNativeFactId);
        }
        Ok(Self {
            chain_id,
            address,
            block_number,
            native_fact_id,
            selector,
            raw_data,
        })
    }
}

/// Outcome stored separately from native EVM facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecodeStatus {
    /// A registered decoder produced structured JSON.
    Decoded,
    /// A registered decoder failed; native bytes remain available.
    Failed,
    /// No deployment covered the subject.
    Unknown,
}

/// One append-only decoder output revision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DecodeRevision {
    /// Deterministic key shared by retries/revisions.
    pub replay_key: String,
    /// Chain ID.
    pub chain_id: u64,
    /// Contract address.
    pub contract_address: String,
    /// Native fact identity, never replaced by decoded identity.
    pub native_fact_id: String,
    /// Four-byte selector as lowercase hex.
    pub selector_hex: String,
    /// Exact native bytes as lowercase hex.
    pub raw_data_hex: String,
    /// Selected decoder version, absent for unknown contracts.
    pub decoder_version: Option<String>,
    /// Decode outcome.
    pub status: DecodeStatus,
    /// Structured output only on success.
    pub decoded_json: Option<String>,
    /// Bounded error text only on failure.
    pub error: Option<String>,
    /// Append-only revision.
    pub revision: u64,
}

/// Historical decoder deployment registry.
#[derive(Clone, Debug, Default)]
pub struct DecoderRegistry {
    deployments: Vec<DecoderDeployment>,
}

impl DecoderRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a non-overlapping deployment range.
    ///
    /// # Errors
    ///
    /// Rejects overlap for the same chain and contract.
    pub fn register(&mut self, deployment: DecoderDeployment) -> Result<(), DecoderError> {
        if self
            .deployments
            .iter()
            .any(|current| current.overlaps(&deployment))
        {
            return Err(DecoderError::OverlappingDeployment);
        }
        self.deployments.push(deployment);
        self.deployments.sort_by_key(|deployment| {
            (
                deployment.chain_id,
                deployment.address,
                deployment.valid_from_block,
            )
        });
        Ok(())
    }

    /// Resolves the decoder valid at one exact historical block.
    #[must_use]
    pub fn resolve(
        &self,
        chain_id: u64,
        address: Address,
        block_number: u64,
    ) -> Option<&DecoderDeployment> {
        self.deployments.iter().find(|deployment| {
            deployment.chain_id == chain_id
                && deployment.address == address
                && deployment.contains(block_number)
        })
    }

    /// Records a success or failure from the resolved deployment.
    ///
    /// # Errors
    ///
    /// Rejects revision zero, missing deployments, and serialized output that
    /// exceeds the bounded decoder record.
    pub fn record_outcome(
        &self,
        subject: &NativeDecodeSubject,
        outcome: Result<serde_json::Value, String>,
        revision: u64,
    ) -> Result<DecodeRevision, DecoderError> {
        if revision == 0 {
            return Err(DecoderError::ZeroRevision);
        }
        let deployment = self
            .resolve(subject.chain_id, subject.address, subject.block_number)
            .ok_or(DecoderError::NoDeployment)?;
        let (status, decoded_json, error) = match outcome {
            Ok(value) => {
                let value = serde_json::to_string(&value)
                    .map_err(|_| DecoderError::InvalidDecodedOutput)?;
                if value.len() > 1_048_576 {
                    return Err(DecoderError::DecodedOutputTooLarge);
                }
                (DecodeStatus::Decoded, Some(value), None)
            }
            Err(error) => (
                DecodeStatus::Failed,
                None,
                Some(error.chars().take(1_024).collect()),
            ),
        };
        Ok(build_revision(
            subject,
            Some(deployment.version.clone()),
            status,
            decoded_json,
            error,
            revision,
        ))
    }

    /// Records a raw subject for which no decoder is deployed.
    ///
    /// # Errors
    ///
    /// Rejects revision zero.
    pub fn record_unknown(
        &self,
        subject: &NativeDecodeSubject,
        revision: u64,
    ) -> Result<DecodeRevision, DecoderError> {
        if revision == 0 {
            return Err(DecoderError::ZeroRevision);
        }
        Ok(build_revision(
            subject,
            None,
            DecodeStatus::Unknown,
            None,
            None,
            revision,
        ))
    }
}

/// Decoder registry/output boundary failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DecoderError {
    /// Only Ethereum and BSC mainnet are supported.
    #[error("unsupported decoder chain ID {0}")]
    UnsupportedChainId(u64),
    /// Historical range was inverted.
    #[error("decoder deployment range is inverted")]
    InvalidDeploymentRange,
    /// Version identity was blank.
    #[error("decoder version must not be empty")]
    EmptyVersion,
    /// Deployment overlaps an existing historical range.
    #[error("decoder deployment overlaps an existing range")]
    OverlappingDeployment,
    /// Native fact identity was blank.
    #[error("native fact identity must not be empty")]
    EmptyNativeFactId,
    /// Revision zero is reserved.
    #[error("decoder revision must be positive")]
    ZeroRevision,
    /// No deployment covers the subject.
    #[error("no decoder deployment covers the subject")]
    NoDeployment,
    /// Structured output could not be serialized.
    #[error("decoded output could not be serialized")]
    InvalidDecodedOutput,
    /// Structured output exceeded one MiB.
    #[error("decoded output exceeds one MiB")]
    DecodedOutputTooLarge,
}

fn build_revision(
    subject: &NativeDecodeSubject,
    decoder_version: Option<String>,
    status: DecodeStatus,
    decoded_json: Option<String>,
    error: Option<String>,
    revision: u64,
) -> DecodeRevision {
    let logical_key = format!(
        "{}\0{}\0{}\0{}",
        subject.chain_id,
        subject.address,
        subject.native_fact_id,
        encode_hex(&subject.selector)
    );
    DecodeRevision {
        replay_key: encode_hex(blake3::hash(logical_key.as_bytes()).as_bytes()),
        chain_id: subject.chain_id,
        contract_address: subject.address.to_string(),
        native_fact_id: subject.native_fact_id.clone(),
        selector_hex: encode_hex(&subject.selector),
        raw_data_hex: encode_hex(&subject.raw_data),
        decoder_version,
        status,
        decoded_json,
        error,
        revision,
    }
}

fn validate_chain(chain_id: u64) -> Result<(), DecoderError> {
    if matches!(chain_id, 1 | 56) {
        Ok(())
    } else {
        Err(DecoderError::UnsupportedChainId(chain_id))
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut hex, byte| {
            write!(hex, "{byte:02x}").expect("writing into a String cannot fail");
            hex
        })
}
