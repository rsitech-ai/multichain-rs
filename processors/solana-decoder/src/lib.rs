#![doc = "Append-only, slot-qualified Solana program decoder revisions."]

use std::str::FromStr as _;

use serde::Serialize;
use serde_json::Value;
use solana_domain::Pubkey;
use thiserror::Error;

const MAX_INSTRUCTION_IDENTITY_BYTES: usize = 256;
const MAX_RAW_DATA_BYTES: usize = 1_048_576;
const MAX_DECODED_JSON_BYTES: usize = 1_048_576;
const MAX_DECODER_ERROR_BYTES: usize = 1_024;

/// One program decoder deployment over an inclusive slot range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecoderDeployment {
    program_id: Pubkey,
    decoder_version: String,
    valid_from_slot: u64,
    valid_to_slot: Option<u64>,
}

impl DecoderDeployment {
    /// Creates a bounded deployment range.
    ///
    /// # Errors
    ///
    /// Rejects blank versions and inverted ranges.
    pub fn new(
        program_id: Pubkey,
        decoder_version: impl Into<String>,
        valid_from_slot: u64,
        valid_to_slot: Option<u64>,
    ) -> Result<Self, SolanaDecoderError> {
        let decoder_version = decoder_version.into();
        if decoder_version.trim().is_empty()
            || !decoder_version.is_ascii()
            || decoder_version.len() > 128
        {
            return Err(SolanaDecoderError::InvalidDecoderVersion);
        }
        if valid_to_slot.is_some_and(|end| end < valid_from_slot) {
            return Err(SolanaDecoderError::InvalidDeploymentRange);
        }
        Ok(Self {
            program_id,
            decoder_version,
            valid_from_slot,
            valid_to_slot,
        })
    }

    /// Program identity.
    #[must_use]
    pub const fn program_id(&self) -> &Pubkey {
        &self.program_id
    }

    /// Decoder build/deployment identity.
    #[must_use]
    pub fn decoder_version(&self) -> &str {
        &self.decoder_version
    }

    /// First covered slot.
    #[must_use]
    pub const fn valid_from_slot(&self) -> u64 {
        self.valid_from_slot
    }

    /// Last covered slot, inclusive.
    #[must_use]
    pub const fn valid_to_slot(&self) -> Option<u64> {
        self.valid_to_slot
    }

    fn covers(&self, slot: u64) -> bool {
        slot >= self.valid_from_slot && self.valid_to_slot.is_none_or(|end| slot <= end)
    }

    fn overlaps(&self, other: &Self) -> bool {
        if self.program_id != other.program_id {
            return false;
        }
        let self_end = self.valid_to_slot.unwrap_or(u64::MAX);
        let other_end = other.valid_to_slot.unwrap_or(u64::MAX);
        self.valid_from_slot <= other_end && other.valid_from_slot <= self_end
    }
}

/// Outcome of one decoder attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeStatus {
    /// Decoder emitted bounded structured JSON.
    Decoded,
    /// No deployment covered this program/slot.
    Unknown,
    /// A selected decoder returned a bounded failure.
    Failed,
}

impl DecodeStatus {
    /// Stable storage encoding.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Decoded => "decoded",
            Self::Unknown => "unknown",
            Self::Failed => "failed",
        }
    }
}

/// Immutable result of one decode/replay attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeRevision {
    program_id: Pubkey,
    slot: u64,
    instruction_identity: String,
    decoder_version: Option<String>,
    status: DecodeStatus,
    decoded_json: Option<String>,
    error: Option<String>,
    raw_data: Vec<u8>,
    native_fact_id: [u8; 32],
    revision: u64,
    recorded_at_unix_ns: i64,
}

impl DecodeRevision {
    /// Program identity.
    #[must_use]
    pub const fn program_id(&self) -> &Pubkey {
        &self.program_id
    }

    /// Slot at which the instruction executed.
    #[must_use]
    pub const fn slot(&self) -> u64 {
        self.slot
    }

    /// Outer/inner instruction identity.
    #[must_use]
    pub fn instruction_identity(&self) -> &str {
        &self.instruction_identity
    }

    /// Decoder build used, absent for unknown programs.
    #[must_use]
    pub fn decoder_version(&self) -> Option<&str> {
        self.decoder_version.as_deref()
    }

    /// Attempt result.
    #[must_use]
    pub const fn status(&self) -> DecodeStatus {
        self.status
    }

    /// Canonical compact JSON on success.
    #[must_use]
    pub fn decoded_json(&self) -> Option<&str> {
        self.decoded_json.as_deref()
    }

    /// Bounded decoder failure.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Exact opaque instruction bytes.
    #[must_use]
    pub fn raw_data(&self) -> &[u8] {
        &self.raw_data
    }

    /// Native instruction fact identity.
    #[must_use]
    pub const fn native_fact_id(&self) -> &[u8; 32] {
        &self.native_fact_id
    }

    /// Append-only attempt revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Local materialization time.
    #[must_use]
    pub const fn recorded_at_unix_ns(&self) -> i64 {
        self.recorded_at_unix_ns
    }

    /// Converts the immutable attempt into its exact append-only storage row.
    #[must_use]
    pub fn fact_row(&self) -> DecodeRevisionFactRow {
        DecodeRevisionFactRow {
            program_id: self.program_id.to_string(),
            slot: self.slot,
            instruction_identity: self.instruction_identity.clone(),
            decoder_version: self.decoder_version.clone(),
            decode_status: self.status.as_str().to_owned(),
            decoded_json: self.decoded_json.clone(),
            error: self.error.clone(),
            raw_data_hex: encode_hex(&self.raw_data),
            native_fact_id: encode_hex(&self.native_fact_id),
            revision: self.revision,
            recorded_at_unix_ns: self.recorded_at_unix_ns,
        }
    }
}

/// `solana_decoder_revisions` insert row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DecodeRevisionFactRow {
    pub program_id: String,
    pub slot: u64,
    pub instruction_identity: String,
    pub decoder_version: Option<String>,
    pub decode_status: String,
    pub decoded_json: Option<String>,
    pub error: Option<String>,
    pub raw_data_hex: String,
    pub native_fact_id: String,
    pub revision: u64,
    pub recorded_at_unix_ns: i64,
}

/// Slot-qualified registry and append-only revision allocator.
#[derive(Clone, Debug, Default)]
pub struct DecoderRegistry {
    deployments: Vec<DecoderDeployment>,
    revision: u64,
}

impl DecoderRegistry {
    /// Creates an empty decoder registry.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            deployments: Vec::new(),
            revision: 0,
        }
    }

    /// Creates v1 SPL Token and Token-2022 deployment entries.
    ///
    /// # Errors
    ///
    /// Fails only if a built-in program identity is invalid.
    pub fn standard_v1() -> Result<Self, SolanaDecoderError> {
        let mut registry = Self::new();
        let token = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
            .map_err(|_| SolanaDecoderError::InvalidBuiltInProgram)?;
        let token_2022 = Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb")
            .map_err(|_| SolanaDecoderError::InvalidBuiltInProgram)?;
        registry.register(DecoderDeployment::new(token, "spl-token/v1", 0, None)?)?;
        registry.register(DecoderDeployment::new(
            token_2022,
            "spl-token-2022/v1",
            0,
            None,
        )?)?;
        Ok(registry)
    }

    /// Adds a non-overlapping deployment.
    ///
    /// # Errors
    ///
    /// Rejects another deployment covering the same program and slot.
    pub fn register(&mut self, deployment: DecoderDeployment) -> Result<(), SolanaDecoderError> {
        if self
            .deployments
            .iter()
            .any(|existing| existing.overlaps(&deployment))
        {
            return Err(SolanaDecoderError::OverlappingDeployment);
        }
        self.deployments.push(deployment);
        self.deployments.sort_by(|left, right| {
            left.program_id
                .to_bytes()
                .cmp(&right.program_id.to_bytes())
                .then(left.valid_from_slot.cmp(&right.valid_from_slot))
        });
        Ok(())
    }

    /// Finds the decoder covering a program/slot.
    #[must_use]
    pub fn deployment_for(&self, program_id: &Pubkey, slot: u64) -> Option<&DecoderDeployment> {
        self.deployments
            .iter()
            .find(|deployment| deployment.program_id == *program_id && deployment.covers(slot))
    }

    /// Appends one decoded, failed, or unknown revision.
    ///
    /// The callback is never invoked when no deployment covers the
    /// program/slot.
    ///
    /// # Errors
    ///
    /// Rejects invalid identities, oversized raw/decoded data, negative time,
    /// and revision exhaustion.
    #[allow(clippy::too_many_arguments)]
    pub fn decode<F>(
        &mut self,
        program_id: &Pubkey,
        slot: u64,
        instruction_identity: impl Into<String>,
        raw_data: &[u8],
        native_fact_id: [u8; 32],
        recorded_at_unix_ns: i64,
        decoder: F,
    ) -> Result<DecodeRevision, SolanaDecoderError>
    where
        F: FnOnce(&[u8]) -> Result<Value, String>,
    {
        let instruction_identity = instruction_identity.into();
        if instruction_identity.trim().is_empty()
            || instruction_identity.len() > MAX_INSTRUCTION_IDENTITY_BYTES
            || !instruction_identity.is_ascii()
        {
            return Err(SolanaDecoderError::InvalidInstructionIdentity);
        }
        if raw_data.len() > MAX_RAW_DATA_BYTES {
            return Err(SolanaDecoderError::RawDataTooLarge(raw_data.len()));
        }
        if native_fact_id == [0; 32] {
            return Err(SolanaDecoderError::InvalidNativeFactId);
        }
        if recorded_at_unix_ns < 0 {
            return Err(SolanaDecoderError::InvalidRecordedTime);
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(SolanaDecoderError::RevisionOverflow)?;
        let deployment = self.deployment_for(program_id, slot).cloned();
        let (decoder_version, status, decoded_json, error) = if let Some(deployment) = deployment {
            match decoder(raw_data) {
                Ok(value) => {
                    let json =
                        serde_json::to_string(&value).map_err(SolanaDecoderError::Serialize)?;
                    if json.len() > MAX_DECODED_JSON_BYTES {
                        return Err(SolanaDecoderError::DecodedJsonTooLarge(json.len()));
                    }
                    (
                        Some(deployment.decoder_version),
                        DecodeStatus::Decoded,
                        Some(json),
                        None,
                    )
                }
                Err(error) => (
                    Some(deployment.decoder_version),
                    DecodeStatus::Failed,
                    None,
                    Some(truncate_error(error)),
                ),
            }
        } else {
            (None, DecodeStatus::Unknown, None, None)
        };
        self.revision = revision;
        Ok(DecodeRevision {
            program_id: *program_id,
            slot,
            instruction_identity,
            decoder_version,
            status,
            decoded_json,
            error,
            raw_data: raw_data.to_vec(),
            native_fact_id,
            revision,
            recorded_at_unix_ns,
        })
    }
}

fn truncate_error(mut error: String) -> String {
    if error.len() <= MAX_DECODER_ERROR_BYTES {
        return error;
    }
    let mut boundary = MAX_DECODER_ERROR_BYTES;
    while !error.is_char_boundary(boundary) {
        boundary -= 1;
    }
    error.truncate(boundary);
    error
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

/// Decoder registry or attempt failure.
#[derive(Debug, Error)]
pub enum SolanaDecoderError {
    /// Decoder version is blank, non-ASCII, or oversized.
    #[error("invalid decoder version")]
    InvalidDecoderVersion,
    /// Deployment end precedes its start.
    #[error("invalid decoder deployment range")]
    InvalidDeploymentRange,
    /// Two deployments overlap for one program.
    #[error("overlapping decoder deployment")]
    OverlappingDeployment,
    /// A built-in program ID could not be parsed.
    #[error("invalid built-in Solana program")]
    InvalidBuiltInProgram,
    /// Instruction identity is blank, non-ASCII, or oversized.
    #[error("invalid instruction identity")]
    InvalidInstructionIdentity,
    /// Native instruction bytes exceed the decoder boundary.
    #[error("raw instruction data is too large: {0} bytes")]
    RawDataTooLarge(usize),
    /// Native fact identity is zero.
    #[error("invalid native fact identity")]
    InvalidNativeFactId,
    /// Attempt time cannot be negative.
    #[error("invalid decode recorded time")]
    InvalidRecordedTime,
    /// Successful decoder JSON exceeds the storage boundary.
    #[error("decoded JSON is too large: {0} bytes")]
    DecodedJsonTooLarge(usize),
    /// JSON serialization failed.
    #[error("decoded JSON serialization failed: {0}")]
    Serialize(#[source] serde_json::Error),
    /// Revision allocator exhausted.
    #[error("decoder revision overflow")]
    RevisionOverflow,
}
