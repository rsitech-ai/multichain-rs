#![doc = "Concrete broker, object-store, and database adapters."]

mod postgres;
mod redpanda;
mod s3;

pub use postgres::{MemoryCheckpointStore, PostgresCheckpointStore};
pub use redpanda::{MemoryBroker, PublishedRecord, RedpandaBroker};
pub use s3::{MemoryRawArchive, S3ArchiveConfig, S3RawArchive};

/// Stable component identifier used by health and build metadata.
pub const COMPONENT_NAME: &str = "storage-adapters";
