use bitcoin_domain::{BlockHash, OutPoint, Txid};
use thiserror::Error;

/// Candidate-chain validation failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum StateError {
    /// The first observed block was not the selected network's genesis block.
    #[error("block {hash} is not the expected network genesis block")]
    InvalidGenesis {
        /// Rejected block hash.
        hash: BlockHash,
    },
    /// The block does not satisfy the target required by its parent chain.
    #[error("block {hash} has invalid proof of work")]
    InvalidProofOfWork {
        /// Rejected block hash.
        hash: BlockHash,
    },
    /// The candidate's parent is absent from the local DAG.
    #[error("block {hash} has unknown parent {parent}")]
    UnknownParent {
        /// Rejected block hash.
        hash: BlockHash,
        /// Missing parent hash.
        parent: BlockHash,
    },
    /// A retarget boundary could not be found in the candidate branch.
    #[error("block {hash} is missing difficulty epoch boundary at height {height}")]
    MissingDifficultyBoundary {
        /// Rejected block hash.
        hash: BlockHash,
        /// Expected epoch-boundary height.
        height: u32,
    },
    /// Accumulated 256-bit chain work overflowed.
    #[error("accumulated work overflowed for block {hash}")]
    WorkOverflow {
        /// Rejected block hash.
        hash: BlockHash,
    },
    /// The internal candidate DAG became inconsistent.
    #[error("candidate DAG is inconsistent at block {hash}")]
    InconsistentDag {
        /// Block involved in the invariant failure.
        hash: BlockHash,
    },
    /// A connected block does not extend the current canonical tip.
    #[error("block {hash} does not extend current canonical tip")]
    UnexpectedConnect {
        /// Rejected block hash.
        hash: BlockHash,
        /// Parent declared by the rejected block.
        parent: BlockHash,
        /// Current canonical UTXO tip.
        current_tip: Option<BlockHash>,
    },
    /// A transaction references an output absent from canonical UTXO state.
    #[error("transaction {txid} input {input_index} references missing prevout")]
    MissingPrevout {
        /// Block containing the transaction.
        block_hash: BlockHash,
        /// Spending transaction.
        txid: Txid,
        /// Stable input index.
        input_index: u32,
        /// Missing output.
        outpoint: OutPoint,
    },
    /// A transaction would overwrite an unspent output identity.
    #[error("transaction {txid} output {output_index} already exists")]
    DuplicateOutpoint {
        /// Block containing the transaction.
        block_hash: BlockHash,
        /// Creating transaction.
        txid: Txid,
        /// Stable output index.
        output_index: u32,
        /// Colliding output.
        outpoint: OutPoint,
    },
    /// Transaction outputs exceed its resolved inputs.
    #[error("transaction {txid} creates {output_sats} sats from {input_sats} input sats")]
    NegativeFee {
        /// Block containing the transaction.
        block_hash: BlockHash,
        /// Invalid transaction.
        txid: Txid,
        /// Resolved input value.
        input_sats: u64,
        /// Created output value.
        output_sats: u64,
    },
    /// Satoshi or output-index arithmetic overflowed.
    #[error("amount or output index overflowed in transaction {txid}")]
    AmountOverflow {
        /// Block containing the transaction.
        block_hash: BlockHash,
        /// Transaction being applied.
        txid: Txid,
    },
    /// A disconnect did not target the current canonical UTXO tip.
    #[error("cannot disconnect {requested}; current UTXO tip is {current_tip:?}")]
    OutOfOrderDisconnect {
        /// Requested block hash.
        requested: BlockHash,
        /// Current canonical UTXO tip.
        current_tip: Option<BlockHash>,
    },
    /// An internal UTXO undo invariant failed.
    #[error("UTXO undo invariant failed for outpoint {outpoint:?}")]
    InconsistentUtxo {
        /// Outpoint involved in the invariant failure.
        outpoint: OutPoint,
    },
}
