#![doc = "Bitcoin candidate-chain tracking and reversible canonical state."]

mod backfill;
mod checkpoint;
mod dag;
mod error;
mod state;
mod transition;
mod utxo;

pub use checkpoint::StateCheckpoint;
pub use dag::CanonicalityState;
pub use error::StateError;
pub use state::{BitcoinState, StateUpdate};
pub use transition::BlockTransition;
pub use utxo::{UtxoEvent, UtxoState};

/// Stable component identifier used by health and build metadata.
pub const COMPONENT_NAME: &str = "bitcoin-canonicality";
pub use backfill::{
    BackfillBlock, BackfillCheckpoint, BackfillCoordinator, BackfillError, BackfillReport,
    BackfillRequest, BackfillSink, BackfillSource,
};
