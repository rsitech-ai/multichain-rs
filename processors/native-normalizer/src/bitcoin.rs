use bitcoin_domain::BitcoinBlock;
use chain_domain::BitcoinNetwork;
use fact_envelope::encode_hex;
use serde::Serialize;
use thiserror::Error;

/// Stable parser identity recorded on native Bitcoin facts.
pub const BITCOIN_PARSER_VERSION: &str = "bitcoin-native/1.0.0";

/// Validated context shared by rows derived from one block observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BitcoinFactContext {
    network: BitcoinNetwork,
    height: u32,
    revision: u64,
    canonicality: String,
    source_id: Option<String>,
    source_session_id: Option<[u8; 16]>,
    observation_id: Option<[u8; 32]>,
    recorded_at_unix_ns: Option<i64>,
}

impl BitcoinFactContext {
    /// Creates the chain-position and revision part of a fact context.
    ///
    /// # Errors
    ///
    /// Rejects revision zero and unknown canonicality values.
    pub fn new(
        network: BitcoinNetwork,
        height: u32,
        revision: u64,
        canonicality: impl Into<String>,
    ) -> Result<Self, BitcoinFactError> {
        if revision == 0 {
            return Err(BitcoinFactError::ZeroRevision);
        }
        let canonicality = canonicality.into();
        if !matches!(
            canonicality.as_str(),
            "candidate" | "canonical" | "non_canonical"
        ) {
            return Err(BitcoinFactError::InvalidCanonicality(canonicality));
        }
        Ok(Self {
            network,
            height,
            revision,
            canonicality,
            source_id: None,
            source_session_id: None,
            observation_id: None,
            recorded_at_unix_ns: None,
        })
    }

    /// Attaches exact source lineage.
    ///
    /// # Errors
    ///
    /// Rejects a blank source identity.
    pub fn with_lineage(
        mut self,
        source_id: impl Into<String>,
        source_session_id: [u8; 16],
        observation_id: [u8; 32],
        recorded_at_unix_ns: i64,
    ) -> Result<Self, BitcoinFactError> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() {
            return Err(BitcoinFactError::EmptySourceId);
        }
        self.source_id = Some(source_id);
        self.source_session_id = Some(source_session_id);
        self.observation_id = Some(observation_id);
        self.recorded_at_unix_ns = Some(recorded_at_unix_ns);
        Ok(self)
    }

    fn lineage(&self) -> Result<BitcoinLineage, BitcoinFactError> {
        Ok(BitcoinLineage {
            source_id: self
                .source_id
                .clone()
                .ok_or(BitcoinFactError::MissingLineage)?,
            source_session_id: encode_hex(
                &self
                    .source_session_id
                    .ok_or(BitcoinFactError::MissingLineage)?,
            ),
            observation_id: encode_hex(
                &self
                    .observation_id
                    .ok_or(BitcoinFactError::MissingLineage)?,
            ),
            recorded_at_unix_ns: self
                .recorded_at_unix_ns
                .ok_or(BitcoinFactError::MissingLineage)?,
        })
    }
}

#[derive(Clone, Debug)]
struct BitcoinLineage {
    source_id: String,
    source_session_id: String,
    observation_id: String,
    recorded_at_unix_ns: i64,
}

/// One normalized block and its transaction/input/output rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BitcoinFactBatch {
    /// Block row.
    pub blocks: Vec<BitcoinBlockFactRow>,
    /// Transaction rows.
    pub transactions: Vec<BitcoinTransactionFactRow>,
    /// Input rows.
    pub inputs: Vec<BitcoinInputFactRow>,
    /// Output rows.
    pub outputs: Vec<BitcoinOutputFactRow>,
}

impl BitcoinFactBatch {
    /// Derives deterministic native facts from one merkle-validated block.
    ///
    /// # Errors
    ///
    /// Rejects missing lineage and indices or virtual sizes outside the
    /// canonical integer representation.
    pub fn from_block(
        block: &BitcoinBlock,
        context: &BitcoinFactContext,
    ) -> Result<Self, BitcoinFactError> {
        let lineage = context.lineage()?;
        let network = network_name(context.network);
        let block_hash = block.block_hash().to_string();
        let transaction_count = u32::try_from(block.transactions().len())
            .map_err(|_| BitcoinFactError::IndexOverflow)?;
        let blocks = vec![BitcoinBlockFactRow {
            network: network.to_owned(),
            block_hash: block_hash.clone(),
            parent_block_hash: block.previous_block_hash().to_string(),
            height: context.height,
            block_time: block.timestamp(),
            transaction_count,
            canonicality: context.canonicality.clone(),
            revision: context.revision,
            source_id: lineage.source_id.clone(),
            source_session_id: lineage.source_session_id.clone(),
            observation_id: lineage.observation_id.clone(),
            parser_version: BITCOIN_PARSER_VERSION.to_owned(),
            recorded_at_unix_ns: lineage.recorded_at_unix_ns,
        }];
        let mut transactions = Vec::with_capacity(block.transactions().len());
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        for (transaction_index, transaction) in block.transactions().iter().enumerate() {
            let transaction_index =
                u32::try_from(transaction_index).map_err(|_| BitcoinFactError::IndexOverflow)?;
            let txid = transaction.txid().to_string();
            transactions.push(BitcoinTransactionFactRow {
                network: network.to_owned(),
                block_hash: block_hash.clone(),
                height: context.height,
                transaction_index,
                txid: txid.clone(),
                wtxid: transaction.wtxid().to_string(),
                virtual_size: u64::try_from(transaction.virtual_size())
                    .map_err(|_| BitcoinFactError::VirtualSizeOverflow)?,
                canonicality: context.canonicality.clone(),
                revision: context.revision,
                source_id: lineage.source_id.clone(),
                source_session_id: lineage.source_session_id.clone(),
                observation_id: lineage.observation_id.clone(),
                parser_version: BITCOIN_PARSER_VERSION.to_owned(),
                recorded_at_unix_ns: lineage.recorded_at_unix_ns,
            });
            for (input_index, input) in transaction.inputs().iter().enumerate() {
                inputs.push(BitcoinInputFactRow {
                    network: network.to_owned(),
                    block_hash: block_hash.clone(),
                    txid: txid.clone(),
                    input_index: u32::try_from(input_index)
                        .map_err(|_| BitcoinFactError::IndexOverflow)?,
                    previous_txid: input.previous_output.txid.to_string(),
                    previous_vout: input.previous_output.vout,
                    sequence: input.sequence,
                    canonicality: context.canonicality.clone(),
                    revision: context.revision,
                    source_id: lineage.source_id.clone(),
                    source_session_id: lineage.source_session_id.clone(),
                    observation_id: lineage.observation_id.clone(),
                    parser_version: BITCOIN_PARSER_VERSION.to_owned(),
                    recorded_at_unix_ns: lineage.recorded_at_unix_ns,
                });
            }
            for (output_index, output) in transaction.outputs().iter().enumerate() {
                outputs.push(BitcoinOutputFactRow {
                    network: network.to_owned(),
                    block_hash: block_hash.clone(),
                    txid: txid.clone(),
                    output_index: u32::try_from(output_index)
                        .map_err(|_| BitcoinFactError::IndexOverflow)?,
                    value_sats: output.value_sats().value(),
                    script_pubkey_id: encode_hex(output.script_pubkey_id().as_bytes()),
                    script_pubkey_hex: encode_hex(output.script_pubkey().as_bytes()),
                    canonicality: context.canonicality.clone(),
                    revision: context.revision,
                    source_id: lineage.source_id.clone(),
                    source_session_id: lineage.source_session_id.clone(),
                    observation_id: lineage.observation_id.clone(),
                    parser_version: BITCOIN_PARSER_VERSION.to_owned(),
                    recorded_at_unix_ns: lineage.recorded_at_unix_ns,
                });
            }
        }
        Ok(Self {
            blocks,
            transactions,
            inputs,
            outputs,
        })
    }
}

/// `bitcoin_blocks` insert row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BitcoinBlockFactRow {
    /// Network identifier.
    pub network: String,
    /// Block hash.
    pub block_hash: String,
    /// Parent block hash.
    pub parent_block_hash: String,
    /// Candidate height.
    pub height: u32,
    /// Header timestamp in seconds.
    pub block_time: u32,
    /// Native transaction count.
    pub transaction_count: u32,
    /// Candidate/canonical/non-canonical.
    pub canonicality: String,
    /// Append-only revision.
    pub revision: u64,
    /// Observer identity.
    pub source_id: String,
    /// Source session as lowercase hex.
    pub source_session_id: String,
    /// Input observation as lowercase hex.
    pub observation_id: String,
    /// Parser build identity.
    pub parser_version: String,
    /// Platform record time.
    pub recorded_at_unix_ns: i64,
}

/// `bitcoin_transactions` insert row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BitcoinTransactionFactRow {
    /// Network identifier.
    pub network: String,
    /// Inclusion block hash.
    pub block_hash: String,
    /// Candidate height.
    pub height: u32,
    /// Stable transaction position.
    pub transaction_index: u32,
    /// Non-witness identity.
    pub txid: String,
    /// Witness identity.
    pub wtxid: String,
    /// Virtual bytes.
    pub virtual_size: u64,
    /// Inclusion canonicality.
    pub canonicality: String,
    /// Append-only revision.
    pub revision: u64,
    /// Observer identity.
    pub source_id: String,
    /// Source session as lowercase hex.
    pub source_session_id: String,
    /// Input observation.
    pub observation_id: String,
    /// Parser build identity.
    pub parser_version: String,
    /// Platform record time.
    pub recorded_at_unix_ns: i64,
}

/// `bitcoin_inputs` insert row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BitcoinInputFactRow {
    /// Network identifier.
    pub network: String,
    /// Inclusion block hash.
    pub block_hash: String,
    /// Spending transaction.
    pub txid: String,
    /// Stable input position.
    pub input_index: u32,
    /// Referenced transaction.
    pub previous_txid: String,
    /// Referenced output index.
    pub previous_vout: u32,
    /// Consensus sequence.
    pub sequence: u32,
    /// Inclusion canonicality.
    pub canonicality: String,
    /// Append-only revision.
    pub revision: u64,
    /// Observer identity.
    pub source_id: String,
    /// Source session as lowercase hex.
    pub source_session_id: String,
    /// Input observation.
    pub observation_id: String,
    /// Parser build identity.
    pub parser_version: String,
    /// Platform record time.
    pub recorded_at_unix_ns: i64,
}

/// `bitcoin_outputs` insert row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BitcoinOutputFactRow {
    /// Network identifier.
    pub network: String,
    /// Inclusion block hash.
    pub block_hash: String,
    /// Creating transaction.
    pub txid: String,
    /// Stable output position.
    pub output_index: u32,
    /// Exact satoshi amount.
    pub value_sats: u64,
    /// Canonical script digest.
    pub script_pubkey_id: String,
    /// Exact script bytes.
    pub script_pubkey_hex: String,
    /// Inclusion canonicality.
    pub canonicality: String,
    /// Append-only revision.
    pub revision: u64,
    /// Observer identity.
    pub source_id: String,
    /// Source session as lowercase hex.
    pub source_session_id: String,
    /// Input observation.
    pub observation_id: String,
    /// Parser build identity.
    pub parser_version: String,
    /// Platform record time.
    pub recorded_at_unix_ns: i64,
}

/// Native Bitcoin fact construction failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum BitcoinFactError {
    /// Revision zero is reserved.
    #[error("bitcoin fact revision must be positive")]
    ZeroRevision,
    /// Canonicality text is not part of the v1 contract.
    #[error("invalid bitcoin canonicality {0}")]
    InvalidCanonicality(String),
    /// Source identity is blank.
    #[error("bitcoin fact source_id must not be empty")]
    EmptySourceId,
    /// Source lineage was not attached.
    #[error("bitcoin fact context is missing source lineage")]
    MissingLineage,
    /// A transaction, input, or output index exceeded `u32`.
    #[error("bitcoin fact index exceeds u32")]
    IndexOverflow,
    /// Virtual size exceeded `u64`.
    #[error("bitcoin transaction virtual size exceeds u64")]
    VirtualSizeOverflow,
}

#[must_use]
fn network_name(network: BitcoinNetwork) -> &'static str {
    match network {
        BitcoinNetwork::Mainnet => "mainnet",
        BitcoinNetwork::Testnet => "testnet",
        BitcoinNetwork::Signet => "signet",
        BitcoinNetwork::Regtest => "regtest",
    }
}
