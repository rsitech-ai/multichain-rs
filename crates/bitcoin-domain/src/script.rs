use bitcoin::{Address, ScriptBuf};
use chain_domain::BitcoinNetwork;
use sha2::{Digest as _, Sha256};

/// Canonical raw output script.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ScriptPubkey(Vec<u8>);

/// Stable SHA-256 identity of exact script bytes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ScriptPubkeyId([u8; 32]);

/// Recognized presentation shape without implying ownership.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ScriptType {
    P2pkh,
    P2sh,
    SegwitV0,
    Taproot,
    OpReturn,
    Other,
}

/// Optional network-aware presentation derived from a canonical script.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptPresentation {
    /// Recognized script family.
    pub script_type: ScriptType,
    /// Zero or one standard address encodings.
    pub addresses: Vec<String>,
}

impl ScriptPubkey {
    /// Owns exact script bytes.
    #[must_use]
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    /// Returns exact script bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Computes the canonical script identity.
    #[must_use]
    pub fn id(&self) -> ScriptPubkeyId {
        ScriptPubkeyId(Sha256::digest(&self.0).into())
    }

    /// Derives optional presentation data for a selected network.
    #[must_use]
    pub fn presentation(&self, network: BitcoinNetwork) -> ScriptPresentation {
        let script = ScriptBuf::from_bytes(self.0.clone());
        let script_type = if script.is_p2pkh() {
            ScriptType::P2pkh
        } else if script.is_p2sh() {
            ScriptType::P2sh
        } else if script.is_p2wpkh() || script.is_p2wsh() {
            ScriptType::SegwitV0
        } else if script.is_p2tr() {
            ScriptType::Taproot
        } else if script.is_op_return() {
            ScriptType::OpReturn
        } else {
            ScriptType::Other
        };
        let addresses = Address::from_script(&script, bitcoin_network(network))
            .map(|address| vec![address.to_string()])
            .unwrap_or_default();
        ScriptPresentation {
            script_type,
            addresses,
        }
    }
}

impl ScriptPubkeyId {
    /// Returns the fixed digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

const fn bitcoin_network(network: BitcoinNetwork) -> bitcoin::Network {
    match network {
        BitcoinNetwork::Mainnet => bitcoin::Network::Bitcoin,
        BitcoinNetwork::Testnet => bitcoin::Network::Testnet,
        BitcoinNetwork::Signet => bitcoin::Network::Signet,
        BitcoinNetwork::Regtest => bitcoin::Network::Regtest,
    }
}
