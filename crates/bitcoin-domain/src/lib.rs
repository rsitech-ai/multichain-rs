#![doc = "Bitcoin-native blocks, transactions, scripts, and outpoints."]

mod amount;
mod block;
mod error;
mod outpoint;
mod parse;
mod script;
mod transaction;

pub use amount::Sats;
pub use block::{BitcoinBlock, BlockHash, BlockWork};
pub use error::ParseError;
pub use outpoint::OutPoint;
pub use parse::{parse_block, parse_transaction};
pub use script::{ScriptPresentation, ScriptPubkey, ScriptPubkeyId, ScriptType};
pub use transaction::{BitcoinInput, BitcoinOutput, BitcoinTransaction, Txid, Wtxid};

/// Stable component identifier used by health and build metadata.
pub const COMPONENT_NAME: &str = "bitcoin-domain";
