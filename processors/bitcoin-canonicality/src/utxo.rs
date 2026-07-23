use std::collections::HashMap;

use bitcoin_domain::{BitcoinBlock, BlockHash, OutPoint, Sats, ScriptPubkeyId, Txid};
use sha2::{Digest as _, Sha256};

use crate::StateError;

/// Reversible change to canonical UTXO state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UtxoEvent {
    /// A transaction output became available.
    Created {
        /// Created output identity.
        outpoint: OutPoint,
        /// Exact output value.
        value: Sats,
        /// Canonical digest of the output script.
        script_pubkey_id: ScriptPubkeyId,
    },
    /// A canonical transaction spent an output.
    Spent {
        /// Spent output identity.
        outpoint: OutPoint,
        /// Spending transaction.
        spending_txid: Txid,
        /// Stable input index.
        input_index: u32,
    },
    /// A disconnected transaction's spend was reversed.
    SpendReverted {
        /// Restored output identity.
        outpoint: OutPoint,
        /// Former spending transaction.
        spending_txid: Txid,
        /// Stable input index.
        input_index: u32,
    },
    /// A disconnected transaction's output was removed.
    CreationReverted {
        /// Removed output identity.
        outpoint: OutPoint,
    },
}

#[derive(Clone, Debug)]
struct UtxoEntry {
    value: Sats,
    script_pubkey_id: ScriptPubkeyId,
}

#[derive(Clone, Debug)]
enum UndoOperation {
    Created {
        outpoint: OutPoint,
    },
    Spent {
        outpoint: OutPoint,
        entry: UtxoEntry,
        spending_txid: Txid,
        input_index: u32,
    },
}

#[derive(Clone, Debug)]
struct AppliedBlock {
    hash: BlockHash,
    undo: Vec<UndoOperation>,
}

/// Canonical UTXO projection with exact per-block undo data.
#[derive(Clone, Debug, Default)]
pub struct UtxoState {
    entries: HashMap<OutPoint, UtxoEntry>,
    applied_blocks: Vec<AppliedBlock>,
}

impl UtxoState {
    /// Creates empty canonical UTXO state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies one block atomically in transaction and input order.
    ///
    /// Re-delivery of the current tip is idempotent.
    ///
    /// # Errors
    ///
    /// Rejects non-extending blocks, missing prevouts, canonical double spends,
    /// duplicate outputs, negative fees, and arithmetic overflow.
    pub fn apply_block(&mut self, block: &BitcoinBlock) -> Result<Vec<UtxoEvent>, StateError> {
        let hash = block.block_hash();
        if self
            .applied_blocks
            .last()
            .is_some_and(|tip| tip.hash == hash)
        {
            return Ok(Vec::new());
        }
        let current_tip = self.applied_blocks.last().map(|tip| tip.hash);
        if let Some(tip) = current_tip
            && block.previous_block_hash() != tip
        {
            return Err(StateError::UnexpectedConnect {
                hash,
                parent: block.previous_block_hash(),
                current_tip,
            });
        }

        let mut undo = Vec::new();
        let mut events = Vec::new();
        let result = self.apply_transactions(block, &mut undo, &mut events);
        if let Err(error) = result {
            self.rollback_partial(&undo)?;
            return Err(error);
        }
        self.applied_blocks.push(AppliedBlock { hash, undo });
        Ok(events)
    }

    /// Reverts the current tip in exact reverse operation order.
    ///
    /// # Errors
    ///
    /// Rejects out-of-order disconnects and internal undo corruption.
    pub fn revert_block(&mut self, hash: BlockHash) -> Result<Vec<UtxoEvent>, StateError> {
        let current_tip = self.applied_blocks.last().map(|tip| tip.hash);
        if current_tip != Some(hash) {
            return Err(StateError::OutOfOrderDisconnect {
                requested: hash,
                current_tip,
            });
        }
        let applied = self
            .applied_blocks
            .pop()
            .ok_or(StateError::OutOfOrderDisconnect {
                requested: hash,
                current_tip: None,
            })?;
        let mut events = Vec::with_capacity(applied.undo.len());
        for operation in applied.undo.into_iter().rev() {
            match operation {
                UndoOperation::Created { outpoint } => {
                    if self.entries.remove(&outpoint).is_none() {
                        return Err(StateError::InconsistentUtxo { outpoint });
                    }
                    events.push(UtxoEvent::CreationReverted { outpoint });
                }
                UndoOperation::Spent {
                    outpoint,
                    entry,
                    spending_txid,
                    input_index,
                } => {
                    if self.entries.insert(outpoint, entry).is_some() {
                        return Err(StateError::InconsistentUtxo { outpoint });
                    }
                    events.push(UtxoEvent::SpendReverted {
                        outpoint,
                        spending_txid,
                        input_index,
                    });
                }
            }
        }
        Ok(events)
    }

    /// Returns a deterministic digest of sorted canonical UTXOs.
    #[must_use]
    pub fn state_hash(&self) -> [u8; 32] {
        let mut entries: Vec<_> = self.entries.iter().collect();
        entries.sort_unstable_by_key(|(outpoint, _)| outpoint.consensus_bytes());
        let mut digest = Sha256::new();
        for (outpoint, entry) in entries {
            digest.update(outpoint.consensus_bytes());
            digest.update(entry.value.value().to_le_bytes());
            digest.update(entry.script_pubkey_id.as_bytes());
        }
        digest.finalize().into()
    }

    /// Returns the number of currently unspent outputs.
    #[must_use]
    pub fn utxo_count(&self) -> usize {
        self.entries.len()
    }

    fn apply_transactions(
        &mut self,
        block: &BitcoinBlock,
        undo: &mut Vec<UndoOperation>,
        events: &mut Vec<UtxoEvent>,
    ) -> Result<(), StateError> {
        let block_hash = block.block_hash();
        for transaction in block.transactions() {
            let txid = transaction.txid();
            let mut input_sats = 0_u64;
            if !transaction.is_coinbase() {
                for (input_index, input) in transaction.inputs().iter().enumerate() {
                    let input_index = u32::try_from(input_index)
                        .map_err(|_| StateError::AmountOverflow { block_hash, txid })?;
                    let entry = self.entries.remove(&input.previous_output).ok_or(
                        StateError::MissingPrevout {
                            block_hash,
                            txid,
                            input_index,
                            outpoint: input.previous_output,
                        },
                    )?;
                    input_sats = input_sats
                        .checked_add(entry.value.value())
                        .ok_or(StateError::AmountOverflow { block_hash, txid })?;
                    undo.push(UndoOperation::Spent {
                        outpoint: input.previous_output,
                        entry,
                        spending_txid: txid,
                        input_index,
                    });
                    events.push(UtxoEvent::Spent {
                        outpoint: input.previous_output,
                        spending_txid: txid,
                        input_index,
                    });
                }
            }

            let output_sats = transaction
                .outputs()
                .iter()
                .try_fold(0_u64, |total, output| {
                    total
                        .checked_add(output.value_sats().value())
                        .ok_or(StateError::AmountOverflow { block_hash, txid })
                })?;
            if !transaction.is_coinbase() && output_sats > input_sats {
                return Err(StateError::NegativeFee {
                    block_hash,
                    txid,
                    input_sats,
                    output_sats,
                });
            }

            for (output_index, output) in transaction.outputs().iter().enumerate() {
                let output_index = u32::try_from(output_index)
                    .map_err(|_| StateError::AmountOverflow { block_hash, txid })?;
                let outpoint = OutPoint {
                    txid,
                    vout: output_index,
                };
                let entry = UtxoEntry {
                    value: output.value_sats(),
                    script_pubkey_id: output.script_pubkey_id(),
                };
                if self.entries.contains_key(&outpoint) {
                    return Err(StateError::DuplicateOutpoint {
                        block_hash,
                        txid,
                        output_index,
                        outpoint,
                    });
                }
                self.entries.insert(outpoint, entry);
                undo.push(UndoOperation::Created { outpoint });
                events.push(UtxoEvent::Created {
                    outpoint,
                    value: output.value_sats(),
                    script_pubkey_id: output.script_pubkey_id(),
                });
            }
        }
        Ok(())
    }

    fn rollback_partial(&mut self, undo: &[UndoOperation]) -> Result<(), StateError> {
        for operation in undo.iter().rev() {
            match operation {
                UndoOperation::Created { outpoint } => {
                    if self.entries.remove(outpoint).is_none() {
                        return Err(StateError::InconsistentUtxo {
                            outpoint: *outpoint,
                        });
                    }
                }
                UndoOperation::Spent {
                    outpoint, entry, ..
                } => {
                    if self.entries.insert(*outpoint, entry.clone()).is_some() {
                        return Err(StateError::InconsistentUtxo {
                            outpoint: *outpoint,
                        });
                    }
                }
            }
        }
        Ok(())
    }
}
