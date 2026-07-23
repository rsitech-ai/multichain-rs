use bitcoin::consensus::{deserialize, serialize};

use crate::{BitcoinBlock, BitcoinTransaction, ParseError};

const MAX_CONSENSUS_OBJECT_BYTES: usize = 4_000_000;

/// Parses a bounded, exact Bitcoin transaction.
///
/// # Errors
///
/// Returns [`ParseError`] for oversized or malformed consensus bytes.
pub fn parse_transaction(bytes: &[u8]) -> Result<BitcoinTransaction, ParseError> {
    enforce_bound("transaction", bytes)?;
    let transaction: bitcoin::Transaction = deserialize(bytes)?;
    let canonical = serialize(&transaction);
    if canonical != bytes {
        return Err(ParseError::Consensus(
            bitcoin::consensus::encode::Error::ParseFailed(
                "transaction has non-canonical or trailing bytes",
            ),
        ));
    }
    Ok(BitcoinTransaction::from_transaction(
        transaction,
        bytes.to_vec(),
    ))
}

/// Parses a bounded Bitcoin block and validates its transaction merkle root.
///
/// # Errors
///
/// Returns [`ParseError`] for oversized or malformed bytes or a mismatched
/// merkle commitment.
pub fn parse_block(bytes: &[u8]) -> Result<BitcoinBlock, ParseError> {
    enforce_bound("block", bytes)?;
    let block: bitcoin::Block = deserialize(bytes)?;
    if serialize(&block) != bytes {
        return Err(ParseError::Consensus(
            bitcoin::consensus::encode::Error::ParseFailed(
                "block has non-canonical or trailing bytes",
            ),
        ));
    }
    if !block.check_merkle_root() {
        return Err(ParseError::MerkleRootMismatch);
    }
    Ok(BitcoinBlock::from_block(block, bytes.to_vec()))
}

fn enforce_bound(kind: &'static str, bytes: &[u8]) -> Result<(), ParseError> {
    if bytes.len() > MAX_CONSENSUS_OBJECT_BYTES {
        return Err(ParseError::InputTooLarge {
            kind,
            actual: bytes.len(),
            maximum: MAX_CONSENSUS_OBJECT_BYTES,
        });
    }
    Ok(())
}
