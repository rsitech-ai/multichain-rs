use std::future::Future;

use archive_format::{ArchiveManifest, EncodedArchive};
use thiserror::Error;

/// Metadata returned after staging exact compressed bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedObject {
    object_key: String,
    object_sha256: [u8; 32],
    compressed_bytes: u64,
}

impl StagedObject {
    /// Constructs staged object metadata.
    #[must_use]
    pub fn new(object_key: String, object_sha256: [u8; 32], compressed_bytes: u64) -> Self {
        Self {
            object_key,
            object_sha256,
            compressed_bytes,
        }
    }

    /// Returns the raw object key.
    #[must_use]
    pub fn object_key(&self) -> &str {
        &self.object_key
    }

    /// Returns the expected object checksum.
    #[must_use]
    pub const fn object_sha256(&self) -> [u8; 32] {
        self.object_sha256
    }

    /// Returns the expected object length.
    #[must_use]
    pub const fn compressed_bytes(&self) -> u64 {
        self.compressed_bytes
    }
}

/// Durable acknowledgement for a committed manifest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManifestAck {
    manifest_hash: [u8; 32],
    last_collector_sequence: u64,
}

impl ManifestAck {
    /// Constructs a manifest acknowledgement.
    #[must_use]
    pub const fn new(manifest_hash: [u8; 32], last_collector_sequence: u64) -> Self {
        Self {
            manifest_hash,
            last_collector_sequence,
        }
    }

    /// Returns the stable manifest hash.
    #[must_use]
    pub const fn manifest_hash(&self) -> [u8; 32] {
        self.manifest_hash
    }

    /// Returns the inclusive archived sequence.
    #[must_use]
    pub const fn last_collector_sequence(&self) -> u64 {
        self.last_collector_sequence
    }
}

/// Expected object/archive failures.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ArchiveError {
    /// Object bytes failed encoding.
    #[error("archive encoding failed: {0}")]
    Encoding(String),
    /// Staged bytes were absent.
    #[error("staged object `{0}` is missing")]
    MissingObject(String),
    /// HEAD/readback length differed from the manifest.
    #[error("object length mismatch: expected {expected}, got {actual}")]
    LengthMismatch {
        /// Manifest length.
        expected: u64,
        /// Observed length.
        actual: u64,
    },
    /// HEAD/readback checksum differed from the manifest.
    #[error("object checksum mismatch")]
    ChecksumMismatch,
    /// A manifest was attempted before successful verification.
    #[error("object `{0}` has not been verified")]
    NotVerified(String),
    /// A different manifest already covered an overlapping source range.
    #[error("manifest overlaps a non-identical committed range")]
    OverlappingRange,
    /// The manifest chain did not extend the latest committed manifest.
    #[error("manifest does not extend the current manifest chain")]
    ManifestChainMismatch,
    /// The adapter intentionally withheld commit acknowledgement.
    #[error("manifest commit was withheld")]
    CommitWithheld,
    /// Object storage failed.
    #[error("object storage failed: {0}")]
    Storage(String),
}

/// Permanent raw archive boundary.
pub trait RawArchive: Send + Sync {
    /// Finds an already committed exact object for a source range.
    ///
    /// This makes a full at-least-once replay idempotent even after the
    /// session manifest head has advanced.
    fn committed_range(
        &self,
        source_session_id: [u8; 16],
        first_collector_sequence: u64,
        last_collector_sequence: u64,
        object_sha256: [u8; 32],
    ) -> impl Future<Output = Result<Option<ManifestAck>, ArchiveError>> + Send;

    /// Stages exact compressed bytes without making them replayable.
    fn stage(
        &self,
        object: EncodedArchive,
    ) -> impl Future<Output = Result<StagedObject, ArchiveError>> + Send;

    /// Verifies staged object length and SHA-256 using HEAD/readback evidence.
    fn verify(
        &self,
        object: &StagedObject,
    ) -> impl Future<Output = Result<(), ArchiveError>> + Send;

    /// Commits the JSON manifest as the replay visibility boundary.
    fn commit_manifest(
        &self,
        manifest: ArchiveManifest,
    ) -> impl Future<Output = Result<ManifestAck, ArchiveError>> + Send;

    /// Loads the latest committed manifest hash for a source session.
    fn latest_manifest_hash(
        &self,
        source_session_id: [u8; 16],
    ) -> impl Future<Output = Result<Option<[u8; 32]>, ArchiveError>> + Send;
}
