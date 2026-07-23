#![doc = "Broker, object-store, and checkpoint boundary traits."]

mod broker;
mod checkpoint;
mod object_store;

pub use broker::{BrokerAck, BrokerError, BrokerPublisher, RAW_BITCOIN_OBSERVATION_TOPIC};
pub use checkpoint::{
    CheckpointError, CheckpointKind, CheckpointStore, DurableCheckpoint, ReclaimBlocker,
    SealedWalSegment, ensure_reclaimable,
};
pub use object_store::{ArchiveError, ManifestAck, RawArchive, StagedObject};

/// Stable component identifier used by health and build metadata.
pub const COMPONENT_NAME: &str = "storage-ports";
