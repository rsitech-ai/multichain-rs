use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::EncodedArchive;

/// Immutable manifest committed only after an archive object is verified.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArchiveManifest {
    /// Source session covered by this object.
    pub source_session_id: [u8; 16],
    /// Inclusive first collector sequence.
    pub first_collector_sequence: u64,
    /// Inclusive last collector sequence.
    pub last_collector_sequence: u64,
    /// Deterministic raw object key.
    pub object_key: String,
    /// SHA-256 over the exact compressed object bytes.
    pub object_sha256: [u8; 32],
    /// Exact compressed object length.
    pub compressed_bytes: u64,
    /// Number of length-delimited records.
    pub record_count: u64,
    /// Hash of the previous committed manifest for this source session.
    pub previous_manifest_hash: Option<[u8; 32]>,
}

impl ArchiveManifest {
    /// Builds a manifest from an encoded object.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError`] when the object size is not representable.
    pub fn from_encoded(
        object: &EncodedArchive,
        previous_manifest_hash: Option<[u8; 32]>,
    ) -> Result<Self, ManifestError> {
        Ok(Self {
            source_session_id: object.source_session_id(),
            first_collector_sequence: object.first_collector_sequence(),
            last_collector_sequence: object.last_collector_sequence(),
            object_key: object.object_key().to_owned(),
            object_sha256: object.object_sha256(),
            compressed_bytes: u64::try_from(object.compressed_bytes().len())
                .map_err(|_| ManifestError::ObjectTooLarge)?,
            record_count: object.record_count(),
            previous_manifest_hash,
        })
    }

    /// Returns the SHA-256 digest of the stable JSON representation.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError`] if serialization fails.
    pub fn manifest_hash(&self) -> Result<[u8; 32], ManifestError> {
        let bytes = serde_json::to_vec(self)?;
        Ok(Sha256::digest(bytes).into())
    }

    /// Returns the stable JSON bytes written as the commit marker.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError`] if serialization fails.
    pub fn json_bytes(&self) -> Result<Vec<u8>, ManifestError> {
        Ok(serde_json::to_vec(self)?)
    }
}

/// Manifest construction and hashing failures.
#[derive(Debug, Error)]
pub enum ManifestError {
    /// The compressed object length was not representable.
    #[error("archive object is too large")]
    ObjectTooLarge,
    /// Stable JSON serialization failed.
    #[error("manifest serialization failed: {0}")]
    Json(#[from] serde_json::Error),
}
