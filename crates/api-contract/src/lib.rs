#![doc = "Stable, source-qualified API and streaming truth contracts."]

use std::fmt::Write as _;

use bitcoin_domain::Sats;
use evm_domain::EvmNetwork;
use serde::Serialize;
use solana_domain::ForkId;
use thiserror::Error;

const MAX_SOURCE_COUNT: usize = 32;
const MAX_CURSOR_DATASET_BYTES: usize = 64;
const MAX_CURSOR_KEY_BYTES: usize = 512;

/// Reader-facing completeness state. Recovery never erases gap history.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    /// The requested range is proven complete for the named sources.
    Complete,
    /// At least one durable coverage interval remains incomplete.
    KnownIncomplete,
    /// Records were recovered and retain recovery provenance.
    Recovered,
}

/// Source-qualified Bitcoin truth metadata returned beside every subject.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BitcoinTruth {
    network: String,
    source_ids: Vec<String>,
    revision: u64,
    canonicality: String,
    finality: String,
    height: u32,
    completeness: Completeness,
    as_of_unix_ns: i64,
}

impl BitcoinTruth {
    /// Constructs a validated Bitcoin truth envelope.
    ///
    /// # Errors
    ///
    /// Rejects blank networks/sources, zero revisions, unknown statuses, and
    /// status pairs that would describe a non-canonical record as confirmed.
    #[allow(clippy::too_many_arguments)]
    pub fn new<I, S>(
        network: impl Into<String>,
        source_ids: I,
        revision: u64,
        canonicality: impl Into<String>,
        finality: impl Into<String>,
        height: u32,
        completeness: Completeness,
        as_of_unix_ns: i64,
    ) -> Result<Self, TruthError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let network = network.into();
        if network.trim().is_empty() {
            return Err(TruthError::EmptyNetwork);
        }
        if revision == 0 {
            return Err(TruthError::ZeroRevision);
        }
        let source_ids = validated_sources(source_ids)?;
        let canonicality = canonicality.into();
        let finality = finality.into();
        if !matches!(
            canonicality.as_str(),
            "candidate" | "canonical" | "non_canonical"
        ) || !matches!(
            finality.as_str(),
            "pending" | "included" | "confirmed" | "reorged"
        ) {
            return Err(TruthError::InvalidBitcoinStatus {
                canonicality,
                finality,
            });
        }
        let consistent = match canonicality.as_str() {
            "non_canonical" => finality == "reorged",
            "canonical" => matches!(finality.as_str(), "included" | "confirmed"),
            "candidate" => matches!(finality.as_str(), "pending" | "included"),
            _ => false,
        };
        if !consistent {
            return Err(TruthError::InconsistentBitcoinStatus {
                canonicality,
                finality,
            });
        }
        Ok(Self {
            network,
            source_ids,
            revision,
            canonicality,
            finality,
            height,
            completeness,
            as_of_unix_ns,
        })
    }
}

/// Bitcoin output response with exact integer amount encoded as a string.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BitcoinOutput {
    txid: String,
    vout: u32,
    value_sats: Sats,
    script_pubkey_hex: String,
    truth: BitcoinTruth,
}

/// Explicit EVM historical capability flags.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct EvmCoverage {
    /// Native blocks/transactions/receipts/logs are covered for the request.
    pub full_history: bool,
    /// Arbitrary historical state is covered by an archive node.
    pub archive_state: bool,
    /// Historical execution traces are covered.
    pub traces: bool,
}

impl EvmCoverage {
    /// Constructs explicit, independently queryable capability flags.
    #[must_use]
    pub const fn new(full_history: bool, archive_state: bool, traces: bool) -> Self {
        Self {
            full_history,
            archive_state,
            traces,
        }
    }
}

/// Source-qualified EVM truth with chain-native finality vocabulary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EvmTruth {
    chain_id: u64,
    network: &'static str,
    source_ids: Vec<String>,
    revision: u64,
    canonicality: String,
    finality: String,
    completeness: Completeness,
    coverage: EvmCoverage,
    as_of_unix_ns: i64,
}

impl EvmTruth {
    /// Constructs validated Ethereum or BSC response truth.
    ///
    /// # Errors
    ///
    /// Rejects missing sources, zero revision, invalid canonicality, finality
    /// vocabulary from the other EVM chain, and contradictory reorg status.
    #[allow(clippy::too_many_arguments)]
    pub fn new<I, S>(
        network: EvmNetwork,
        source_ids: I,
        revision: u64,
        canonicality: impl Into<String>,
        finality: impl Into<String>,
        completeness: Completeness,
        coverage: EvmCoverage,
        as_of_unix_ns: i64,
    ) -> Result<Self, TruthError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let source_ids = validated_sources(source_ids)?;
        if revision == 0 {
            return Err(TruthError::ZeroRevision);
        }
        let canonicality = canonicality.into();
        if !matches!(
            canonicality.as_str(),
            "candidate" | "canonical" | "non_canonical"
        ) {
            return Err(TruthError::InvalidEvmCanonicality(canonicality));
        }
        let finality = finality.into();
        let valid_finality = match network {
            EvmNetwork::EthereumMainnet => matches!(
                finality.as_str(),
                "pending" | "included" | "canonical_head" | "safe" | "finalized" | "reorged"
            ),
            EvmNetwork::BscMainnet => matches!(
                finality.as_str(),
                "pending" | "included" | "canonical_head" | "fast_finalized" | "reorged"
            ),
        };
        if !valid_finality {
            return Err(TruthError::InvalidEvmFinality {
                chain_id: network.chain_id(),
                finality,
            });
        }
        if (canonicality == "non_canonical") != (finality == "reorged") {
            return Err(TruthError::InconsistentEvmStatus {
                canonicality,
                finality,
            });
        }
        Ok(Self {
            chain_id: network.chain_id(),
            network: network.as_str(),
            source_ids,
            revision,
            canonicality,
            finality,
            completeness,
            coverage,
            as_of_unix_ns,
        })
    }
}

/// Solana ingestion coverage for the initial bounded product tier.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct SolanaCoverage {
    /// Independently identified Yellowstone providers contributing truth.
    pub provider_count: u8,
    /// Every executed transaction is in scope.
    pub all_transactions: bool,
    /// Number of explicit account identities in the selected S2 tier.
    pub selected_account_filter_count: u32,
    /// A full account firehose is deliberately not claimed by v1.
    pub full_account_firehose: bool,
    /// At least one range used explicit reconstruction evidence.
    pub reconstruction: bool,
}

impl SolanaCoverage {
    /// Creates explicit Solana coverage flags.
    #[must_use]
    pub const fn new(
        provider_count: u8,
        selected_account_filter_count: u32,
        full_account_firehose: bool,
        reconstruction: bool,
    ) -> Self {
        Self {
            provider_count,
            all_transactions: true,
            selected_account_filter_count,
            full_account_firehose,
            reconstruction,
        }
    }
}

/// Source- and fork-qualified Solana response truth.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SolanaTruth {
    network: &'static str,
    source_ids: Vec<String>,
    revision: u64,
    slot: u64,
    blockhash: String,
    canonicality: String,
    commitment: String,
    completeness: Completeness,
    coverage: SolanaCoverage,
    provider_divergent: bool,
    recovery_observation_ids: Vec<String>,
    as_of_unix_ns: i64,
}

impl SolanaTruth {
    /// Constructs bounded Solana mainnet-beta truth.
    ///
    /// # Errors
    ///
    /// Rejects missing provider independence, revision zero, contradictory
    /// fork/commitment states, unsupported full-firehose claims, unqualified
    /// divergence, and recovery without observation evidence.
    #[allow(clippy::too_many_arguments)]
    pub fn new<I, S, R>(
        source_ids: I,
        revision: u64,
        fork_id: &ForkId,
        canonicality: impl Into<String>,
        commitment: impl Into<String>,
        completeness: Completeness,
        coverage: SolanaCoverage,
        provider_divergent: bool,
        recovery_observation_ids: R,
        as_of_unix_ns: i64,
    ) -> Result<Self, TruthError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
        R: IntoIterator<Item = [u8; 32]>,
    {
        let source_ids = validated_sources(source_ids)?;
        if revision == 0 {
            return Err(TruthError::ZeroRevision);
        }
        if coverage.provider_count != 2 || usize::from(coverage.provider_count) != source_ids.len()
        {
            return Err(TruthError::InvalidSolanaProviderCount);
        }
        if coverage.selected_account_filter_count > 1_024 || coverage.full_account_firehose {
            return Err(TruthError::UnsupportedSolanaCoverage);
        }
        let canonicality = canonicality.into();
        let commitment = commitment.into();
        if !matches!(
            canonicality.as_str(),
            "candidate" | "canonical" | "non_canonical"
        ) || !matches!(
            commitment.as_str(),
            "received" | "processed" | "confirmed" | "finalized" | "dead"
        ) {
            return Err(TruthError::InvalidSolanaStatus {
                canonicality,
                commitment,
            });
        }
        let consistent = match commitment.as_str() {
            "dead" => canonicality == "non_canonical",
            "received" => canonicality == "candidate",
            "processed" => matches!(canonicality.as_str(), "candidate" | "canonical"),
            "confirmed" | "finalized" => canonicality == "canonical",
            _ => false,
        };
        if !consistent {
            return Err(TruthError::InconsistentSolanaStatus {
                canonicality,
                commitment,
            });
        }
        if provider_divergent && completeness == Completeness::Complete {
            return Err(TruthError::UnqualifiedSolanaDivergence);
        }
        let recovery_observation_ids = recovery_observation_ids
            .into_iter()
            .map(|identity| encode_hex(&identity))
            .collect::<Vec<_>>();
        let has_recovery = !recovery_observation_ids.is_empty();
        if coverage.reconstruction != has_recovery
            || (completeness == Completeness::Recovered) != has_recovery
        {
            return Err(TruthError::InvalidSolanaRecoveryEvidence);
        }
        Ok(Self {
            network: "solana-mainnet-beta",
            source_ids,
            revision,
            slot: fork_id.slot().value(),
            blockhash: fork_id.blockhash().to_string(),
            canonicality,
            commitment,
            completeness,
            coverage,
            provider_divergent,
            recovery_observation_ids,
            as_of_unix_ns,
        })
    }
}

impl BitcoinOutput {
    /// Constructs a validated output response.
    ///
    /// # Errors
    ///
    /// Rejects malformed transaction IDs and script encodings.
    pub fn new(
        txid: impl Into<String>,
        vout: u32,
        value_sats: Sats,
        script_pubkey_hex: impl Into<String>,
        truth: BitcoinTruth,
    ) -> Result<Self, TruthError> {
        let txid = txid.into();
        if !is_lower_hex(&txid, 64) {
            return Err(TruthError::InvalidTxid);
        }
        let script_pubkey_hex = script_pubkey_hex.into();
        if !script_pubkey_hex.len().is_multiple_of(2)
            || (!script_pubkey_hex.is_empty() && !script_pubkey_hex.bytes().all(is_lower_hex_digit))
        {
            return Err(TruthError::InvalidScript);
        }
        Ok(Self {
            txid,
            vout,
            value_sats,
            script_pubkey_hex,
            truth,
        })
    }
}

/// Explicit source semantics for a Bitcoin mempool query.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MempoolView {
    /// One observer's node-local mempool.
    Source {
        /// Exact observer identity.
        source_id: String,
    },
    /// Membership reported by any eligible observer.
    Union {
        /// Health-eligible observers included in the computation.
        eligible_source_ids: Vec<String>,
    },
    /// Membership reported by every eligible observer.
    Intersection {
        /// Health-eligible observers included in the computation.
        eligible_source_ids: Vec<String>,
    },
    /// Membership reported by at least `threshold` eligible observers.
    Quorum {
        /// Minimum number of agreeing observers.
        threshold: usize,
        /// Health-eligible observers included in the computation.
        eligible_source_ids: Vec<String>,
    },
}

impl MempoolView {
    /// Constructs one observer-local view.
    ///
    /// # Errors
    ///
    /// Rejects blank source identities.
    pub fn source(source_id: impl Into<String>) -> Result<Self, TruthError> {
        let source_id = source_id.into();
        validate_source(&source_id)?;
        Ok(Self::Source { source_id })
    }

    /// Constructs a health-qualified union.
    ///
    /// # Errors
    ///
    /// Rejects an empty or invalid source set.
    pub fn union<I, S>(source_ids: I) -> Result<Self, TruthError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Ok(Self::Union {
            eligible_source_ids: validated_sources(source_ids)?,
        })
    }

    /// Constructs a health-qualified intersection.
    ///
    /// # Errors
    ///
    /// Rejects an empty or invalid source set.
    pub fn intersection<I, S>(source_ids: I) -> Result<Self, TruthError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Ok(Self::Intersection {
            eligible_source_ids: validated_sources(source_ids)?,
        })
    }

    /// Constructs a health-qualified quorum.
    ///
    /// # Errors
    ///
    /// Rejects invalid source sets and thresholds outside `1..=sources`.
    pub fn quorum<I, S>(threshold: usize, source_ids: I) -> Result<Self, TruthError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let eligible_source_ids = validated_sources(source_ids)?;
        if threshold == 0 || threshold > eligible_source_ids.len() {
            return Err(TruthError::InvalidQuorum {
                threshold,
                eligible_sources: eligible_source_ids.len(),
            });
        }
        Ok(Self::Quorum {
            threshold,
            eligible_source_ids,
        })
    }
}

/// Opaque stable pagination position.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageCursor {
    dataset: String,
    last_key: String,
    revision: u64,
}

impl PageCursor {
    /// Creates a bounded cursor.
    ///
    /// # Errors
    ///
    /// Rejects empty/oversized fields and revision zero.
    pub fn new(
        dataset: impl Into<String>,
        last_key: impl Into<String>,
        revision: u64,
    ) -> Result<Self, TruthError> {
        let dataset = dataset.into();
        let last_key = last_key.into();
        if dataset.is_empty() || dataset.len() > MAX_CURSOR_DATASET_BYTES {
            return Err(TruthError::InvalidCursor);
        }
        if last_key.is_empty() || last_key.len() > MAX_CURSOR_KEY_BYTES {
            return Err(TruthError::InvalidCursor);
        }
        if dataset.contains('\0') || last_key.contains('\0') || revision == 0 {
            return Err(TruthError::InvalidCursor);
        }
        Ok(Self {
            dataset,
            last_key,
            revision,
        })
    }

    /// Encodes the cursor with an integrity checksum.
    #[must_use]
    pub fn encode(&self) -> String {
        let payload = format!("{}\0{}\0{}", self.dataset, self.last_key, self.revision);
        let digest = blake3::hash(payload.as_bytes());
        format!(
            "{}.{}",
            encode_hex(payload.as_bytes()),
            encode_hex(&digest.as_bytes()[..16])
        )
    }

    /// Decodes and validates a cursor.
    ///
    /// # Errors
    ///
    /// Rejects malformed, modified, or out-of-bounds cursors.
    pub fn decode(encoded: &str) -> Result<Self, TruthError> {
        let (payload_hex, checksum_hex) =
            encoded.split_once('.').ok_or(TruthError::InvalidCursor)?;
        if checksum_hex.len() != 32 || encoded.matches('.').count() != 1 {
            return Err(TruthError::InvalidCursor);
        }
        let payload = decode_hex(payload_hex)?;
        let checksum = decode_hex(checksum_hex)?;
        let digest = blake3::hash(&payload);
        if checksum.as_slice() != &digest.as_bytes()[..16] {
            return Err(TruthError::InvalidCursor);
        }
        let payload = String::from_utf8(payload).map_err(|_| TruthError::InvalidCursor)?;
        let mut fields = payload.split('\0');
        let dataset = fields.next().ok_or(TruthError::InvalidCursor)?;
        let last_key = fields.next().ok_or(TruthError::InvalidCursor)?;
        let revision = fields
            .next()
            .ok_or(TruthError::InvalidCursor)?
            .parse()
            .map_err(|_| TruthError::InvalidCursor)?;
        if fields.next().is_some() {
            return Err(TruthError::InvalidCursor);
        }
        Self::new(dataset, last_key, revision)
    }
}

/// Why a streaming client must fetch a fresh snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResyncReason {
    /// One or more ordered revisions were not delivered.
    SequenceGap,
    /// The server cannot resume from a cursor it no longer retains.
    CursorExpired,
    /// The requested schema is incompatible with this stream.
    SchemaChanged,
}

/// Bounded control frame for stream recovery.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StreamControl {
    #[serde(rename = "type")]
    frame_type: &'static str,
    channel: String,
    last_delivered_sequence: u64,
    current_sequence: u64,
    reason: ResyncReason,
}

impl StreamControl {
    /// Constructs an explicit resynchronization instruction.
    ///
    /// # Errors
    ///
    /// Rejects blank channels or a sequence range that contains no gap.
    pub fn resync_required(
        channel: impl Into<String>,
        last_delivered_sequence: u64,
        current_sequence: u64,
        reason: ResyncReason,
    ) -> Result<Self, TruthError> {
        let channel = channel.into();
        if channel.trim().is_empty()
            || current_sequence <= last_delivered_sequence.saturating_add(1)
        {
            return Err(TruthError::InvalidResyncRange);
        }
        Ok(Self {
            frame_type: "resync_required",
            channel,
            last_delivered_sequence,
            current_sequence,
            reason,
        })
    }
}

/// API contract validation failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum TruthError {
    /// Network identity was blank.
    #[error("network must not be empty")]
    EmptyNetwork,
    /// At least one exact source identity is required.
    #[error("at least one source is required")]
    MissingSources,
    /// Source identity was blank or exceeded its bound.
    #[error("source identity must be non-empty ASCII and at most 128 bytes")]
    EmptySourceId,
    /// The source set exceeded the API bound.
    #[error("source count {0} exceeds {MAX_SOURCE_COUNT}")]
    TooManySources(usize),
    /// Revision zero is reserved.
    #[error("revision must be positive")]
    ZeroRevision,
    /// Bitcoin canonicality or finality text was unknown.
    #[error("invalid bitcoin status pair {canonicality}/{finality}")]
    InvalidBitcoinStatus {
        /// Supplied canonicality.
        canonicality: String,
        /// Supplied finality.
        finality: String,
    },
    /// EVM canonicality text was unknown.
    #[error("invalid EVM canonicality {0}")]
    InvalidEvmCanonicality(String),
    /// Finality text did not belong to the selected EVM chain.
    #[error("invalid finality {finality} for EVM chain ID {chain_id}")]
    InvalidEvmFinality {
        /// Selected EIP-155 chain ID.
        chain_id: u64,
        /// Supplied finality.
        finality: String,
    },
    /// EVM canonicality/finality contradicted each other.
    #[error("inconsistent EVM status pair {canonicality}/{finality}")]
    InconsistentEvmStatus {
        /// Supplied canonicality.
        canonicality: String,
        /// Supplied finality.
        finality: String,
    },
    /// Solana truth requires exactly two independent configured providers.
    #[error("Solana truth requires exactly two source identities")]
    InvalidSolanaProviderCount,
    /// Solana v1 cannot claim unbounded/full account coverage.
    #[error("unsupported Solana coverage claim")]
    UnsupportedSolanaCoverage,
    /// Solana canonicality or commitment text was unknown.
    #[error("invalid Solana status pair {canonicality}/{commitment}")]
    InvalidSolanaStatus {
        /// Supplied canonicality.
        canonicality: String,
        /// Supplied commitment.
        commitment: String,
    },
    /// Solana canonicality and commitment contradicted each other.
    #[error("inconsistent Solana status pair {canonicality}/{commitment}")]
    InconsistentSolanaStatus {
        /// Supplied canonicality.
        canonicality: String,
        /// Supplied commitment.
        commitment: String,
    },
    /// A divergent provider view cannot be called complete.
    #[error("provider divergence requires incomplete truth")]
    UnqualifiedSolanaDivergence,
    /// Recovery status must cite exact observation identities.
    #[error("invalid Solana recovery evidence")]
    InvalidSolanaRecoveryEvidence,
    /// Bitcoin canonicality and finality contradicted each other.
    #[error("inconsistent bitcoin status pair {canonicality}/{finality}")]
    InconsistentBitcoinStatus {
        /// Supplied canonicality.
        canonicality: String,
        /// Supplied finality.
        finality: String,
    },
    /// Quorum threshold was outside the eligible source count.
    #[error("quorum threshold {threshold} is invalid for {eligible_sources} eligible sources")]
    InvalidQuorum {
        /// Supplied threshold.
        threshold: usize,
        /// Number of eligible sources.
        eligible_sources: usize,
    },
    /// Transaction identity was not canonical lowercase hex.
    #[error("txid must be 64 lowercase hexadecimal characters")]
    InvalidTxid,
    /// Script bytes were not canonical lowercase hex.
    #[error("script_pubkey_hex must contain complete lowercase hexadecimal bytes")]
    InvalidScript,
    /// Cursor was malformed, modified, or outside its bounds.
    #[error("invalid pagination cursor")]
    InvalidCursor,
    /// Stream resync sequences did not describe a gap.
    #[error("resync control frame requires an explicit sequence gap")]
    InvalidResyncRange,
}

fn validated_sources<I, S>(source_ids: I) -> Result<Vec<String>, TruthError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut source_ids = source_ids.into_iter().map(Into::into).collect::<Vec<_>>();
    if source_ids.is_empty() {
        return Err(TruthError::MissingSources);
    }
    if source_ids.len() > MAX_SOURCE_COUNT {
        return Err(TruthError::TooManySources(source_ids.len()));
    }
    for source_id in &source_ids {
        validate_source(source_id)?;
    }
    source_ids.sort_unstable();
    source_ids.dedup();
    Ok(source_ids)
}

fn validate_source(source_id: &str) -> Result<(), TruthError> {
    if source_id.trim().is_empty() || source_id.len() > 128 || !source_id.is_ascii() {
        return Err(TruthError::EmptySourceId);
    }
    Ok(())
}

fn is_lower_hex(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len && value.bytes().all(is_lower_hex_digit)
}

const fn is_lower_hex_digit(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut hex, byte| {
            write!(hex, "{byte:02x}").expect("writing into a String cannot fail");
            hex
        })
}

fn decode_hex(value: &str) -> Result<Vec<u8>, TruthError> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(is_lower_hex_digit) {
        return Err(TruthError::InvalidCursor);
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|digits| {
            let text = std::str::from_utf8(digits).map_err(|_| TruthError::InvalidCursor)?;
            u8::from_str_radix(text, 16).map_err(|_| TruthError::InvalidCursor)
        })
        .collect()
}
