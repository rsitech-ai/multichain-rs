use crate::{ParseError, Txid};

/// Canonical Bitcoin spend reference.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OutPoint {
    /// Referenced transaction.
    pub txid: Txid,
    /// Referenced output index.
    pub vout: u32,
}

impl OutPoint {
    /// Encodes the fixed 36-byte consensus outpoint.
    #[must_use]
    pub fn consensus_bytes(self) -> [u8; 36] {
        let mut encoded = [0_u8; 36];
        encoded[..32].copy_from_slice(self.txid.as_bytes());
        encoded[32..].copy_from_slice(&self.vout.to_le_bytes());
        encoded
    }

    /// Decodes a fixed consensus outpoint.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError`] unless exactly 36 bytes are supplied.
    pub fn from_consensus_bytes(bytes: &[u8]) -> Result<Self, ParseError> {
        if bytes.len() != 36 {
            return Err(ParseError::InvalidOutpointLength(bytes.len()));
        }
        let mut txid = [0_u8; 32];
        txid.copy_from_slice(&bytes[..32]);
        let mut vout = [0_u8; 4];
        vout.copy_from_slice(&bytes[32..]);
        Ok(Self {
            txid: Txid::from_bytes(txid),
            vout: u32::from_le_bytes(vout),
        })
    }
}
