use std::fmt;

use thiserror::Error;

/// Supported chain families.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Chain {
    Bitcoin,
    Ethereum,
    Solana,
    BnbSmartChain,
}

/// Bitcoin network identity required for presentation-address encoding.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BitcoinNetwork {
    Mainnet,
    Testnet,
    Signet,
    Regtest,
}

/// Validated externally visible network identifier.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct NetworkId(String);

impl NetworkId {
    /// Validates a non-empty, bounded network identifier.
    ///
    /// # Errors
    ///
    /// Returns [`NetworkIdError`] for empty, overly long, or non-ASCII values.
    pub fn new(value: impl Into<String>) -> Result<Self, NetworkIdError> {
        let value = value.into();
        if value.is_empty() {
            return Err(NetworkIdError::Empty);
        }
        if value.len() > 64 {
            return Err(NetworkIdError::TooLong(value.len()));
        }
        if !value.is_ascii() {
            return Err(NetworkIdError::NonAscii);
        }
        Ok(Self(value))
    }

    /// Returns the stable identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NetworkId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Invalid network identifier.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum NetworkIdError {
    /// Identifier was empty.
    #[error("network identifier is empty")]
    Empty,
    /// Identifier exceeded the boundary limit.
    #[error("network identifier is {0} bytes; maximum is 64")]
    TooLong(usize),
    /// Identifier was not portable ASCII.
    #[error("network identifier must be ASCII")]
    NonAscii,
}
