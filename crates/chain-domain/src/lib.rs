#![doc = "Chain-neutral network, status, and quality types."]

mod network;
mod quality;
mod status;

pub use network::{BitcoinNetwork, Chain, NetworkId, NetworkIdError};
pub use quality::QualityFlag;
pub use status::{Canonicality, Finality};

/// Stable component identifier used by health and build metadata.
pub const COMPONENT_NAME: &str = "chain-domain";
