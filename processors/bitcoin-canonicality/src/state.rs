use bitcoin_domain::{BitcoinBlock, BlockHash};
use chain_domain::BitcoinNetwork;

use crate::{
    BlockTransition, CanonicalityState, StateCheckpoint, StateError, UtxoEvent, UtxoState,
};

/// One canonical correction and its ordered UTXO consequences.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateUpdate {
    transition: BlockTransition,
    utxo_events: Vec<UtxoEvent>,
}

impl StateUpdate {
    /// Returns the canonical block transition.
    #[must_use]
    pub const fn transition(&self) -> BlockTransition {
        self.transition
    }

    /// Returns ordered UTXO events caused by the transition.
    #[must_use]
    pub fn utxo_events(&self) -> &[UtxoEvent] {
        &self.utxo_events
    }
}

/// Transactional composition of candidate-DAG and canonical UTXO state.
#[derive(Clone, Debug)]
pub struct BitcoinState {
    dag: CanonicalityState,
    utxos: UtxoState,
}

impl BitcoinState {
    /// Creates empty state for one Bitcoin network.
    #[must_use]
    pub fn new(network: BitcoinNetwork) -> Self {
        Self {
            dag: CanonicalityState::new(network),
            utxos: UtxoState::new(),
        }
    }

    /// Observes one block and atomically applies every resulting correction.
    ///
    /// # Errors
    ///
    /// Returns validation or state errors without mutating the committed DAG or
    /// UTXO projection.
    pub fn observe_block(&mut self, block: BitcoinBlock) -> Result<Vec<StateUpdate>, StateError> {
        let mut next_dag = self.dag.clone();
        let mut next_utxos = self.utxos.clone();
        let transitions = next_dag.observe_block(block)?;
        let mut updates = Vec::with_capacity(transitions.len());

        for transition in transitions {
            let hash = transition.hash();
            let utxo_events = match transition {
                BlockTransition::Connected { .. } => {
                    let connected = next_dag.block(hash)?;
                    next_utxos.apply_block(&connected)?
                }
                BlockTransition::Disconnected { .. } => next_utxos.revert_block(hash)?,
            };
            updates.push(StateUpdate {
                transition,
                utxo_events,
            });
        }

        self.dag = next_dag;
        self.utxos = next_utxos;
        Ok(updates)
    }

    /// Returns the current canonical tip.
    #[must_use]
    pub const fn canonical_tip(&self) -> Option<BlockHash> {
        self.dag.canonical_tip()
    }

    /// Returns the deterministic canonical UTXO digest.
    #[must_use]
    pub fn state_hash(&self) -> [u8; 32] {
        self.utxos.state_hash()
    }

    /// Returns the current canonical UTXO count.
    #[must_use]
    pub fn utxo_count(&self) -> usize {
        self.utxos.utxo_count()
    }

    /// Returns accepted candidate block count, including side branches.
    #[must_use]
    pub fn candidate_block_count(&self) -> usize {
        self.dag.block_count()
    }

    /// Captures source progress and current state as one immutable value.
    #[must_use]
    pub fn checkpoint(&self, consumer_offset: u64) -> StateCheckpoint {
        StateCheckpoint::new(
            consumer_offset,
            self.canonical_tip(),
            self.dag.revision(),
            self.state_hash(),
        )
    }
}
