#![doc = "Framed raw-archive records and verified manifests."]

mod frame;
mod manifest;

pub use frame::{ArchiveBatch, ArchiveError, EncodedArchive, decode_archive};
pub use manifest::{ArchiveManifest, ManifestError};

/// Stable component identifier used by health and build metadata.
pub const COMPONENT_NAME: &str = "archive-format";
