use std::fmt;

use bitcoin::{Transaction, hashes::Hash as _};

use crate::{OutPoint, ParseError, Sats, ScriptPubkey};

/// Transaction ID, excluding witness serialization.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Txid([u8; 32]);

/// Witness transaction ID.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Wtxid([u8; 32]);

/// Owned Bitcoin input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BitcoinInput {
    /// Spent outpoint.
    pub previous_output: OutPoint,
    /// Exact unlocking script.
    pub script_sig: Vec<u8>,
    /// Consensus sequence used for locktime and RBF signaling.
    pub sequence: u32,
    /// Exact witness stack items.
    pub witness: Vec<Vec<u8>>,
}

/// Owned Bitcoin output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BitcoinOutput {
    value: Sats,
    script_pubkey: ScriptPubkey,
}

/// Owned, replay-stable parsed transaction.
#[derive(Clone, Debug)]
pub struct BitcoinTransaction {
    pub(crate) inner: Transaction,
    consensus_bytes: Vec<u8>,
    inputs: Vec<BitcoinInput>,
    outputs: Vec<BitcoinOutput>,
}

impl BitcoinTransaction {
    pub(crate) fn from_transaction(inner: Transaction, consensus_bytes: Vec<u8>) -> Self {
        let inputs = inner
            .input
            .iter()
            .map(|input| BitcoinInput {
                previous_output: OutPoint {
                    txid: Txid(input.previous_output.txid.to_byte_array()),
                    vout: input.previous_output.vout,
                },
                script_sig: input.script_sig.as_bytes().to_vec(),
                sequence: input.sequence.0,
                witness: input.witness.iter().map(<[u8]>::to_vec).collect(),
            })
            .collect();
        let outputs = inner
            .output
            .iter()
            .map(|output| BitcoinOutput {
                value: Sats::new(output.value.to_sat()),
                script_pubkey: ScriptPubkey::new(output.script_pubkey.as_bytes()),
            })
            .collect();
        Self {
            inner,
            consensus_bytes,
            inputs,
            outputs,
        }
    }

    /// Returns the non-witness transaction digest.
    #[must_use]
    pub fn txid(&self) -> Txid {
        Txid(self.inner.compute_txid().to_byte_array())
    }

    /// Returns the witness-inclusive transaction digest.
    #[must_use]
    pub fn wtxid(&self) -> Wtxid {
        Wtxid(self.inner.compute_wtxid().to_byte_array())
    }

    /// Returns whether this transaction has the canonical coinbase input.
    #[must_use]
    pub fn is_coinbase(&self) -> bool {
        self.inner.is_coinbase()
    }

    /// Returns virtual transaction size in vbytes.
    #[must_use]
    pub fn virtual_size(&self) -> usize {
        self.inner.vsize()
    }

    /// Returns all owned inputs.
    #[must_use]
    pub fn inputs(&self) -> &[BitcoinInput] {
        &self.inputs
    }

    /// Returns all owned outputs.
    #[must_use]
    pub fn outputs(&self) -> &[BitcoinOutput] {
        &self.outputs
    }

    /// Returns one output by index.
    #[must_use]
    pub fn output(&self, index: usize) -> Option<&BitcoinOutput> {
        self.outputs.get(index)
    }

    /// Returns the exact consensus serialization.
    #[must_use]
    pub fn consensus_bytes(&self) -> &[u8] {
        &self.consensus_bytes
    }

    /// Sums outputs using checked `u64` arithmetic.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::AmountOverflow`] if the sum cannot fit.
    pub fn sum_outputs(&self) -> Result<Sats, ParseError> {
        self.outputs.iter().try_fold(Sats::new(0), |sum, output| {
            sum.checked_add(output.value)
                .ok_or(ParseError::AmountOverflow)
        })
    }
}

impl BitcoinOutput {
    /// Returns exact output satoshis.
    #[must_use]
    pub const fn value_sats(&self) -> Sats {
        self.value
    }

    /// Returns the canonical raw output script.
    #[must_use]
    pub const fn script_pubkey(&self) -> &ScriptPubkey {
        &self.script_pubkey
    }

    /// Returns the canonical script digest.
    #[must_use]
    pub fn script_pubkey_id(&self) -> crate::ScriptPubkeyId {
        self.script_pubkey.id()
    }
}

macro_rules! hash_type {
    ($name:ident, $bitcoin:ty) => {
        impl $name {
            /// Constructs from consensus-order digest bytes.
            #[must_use]
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            /// Returns consensus-order digest bytes.
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                <$bitcoin>::from_byte_array(self.0).fmt(formatter)
            }
        }
    };
}

hash_type!(Txid, bitcoin::Txid);
hash_type!(Wtxid, bitcoin::Wtxid);
