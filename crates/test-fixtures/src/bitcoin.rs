use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

/// Immutable corpus manifest.
#[derive(Clone, Debug, Deserialize)]
pub struct BitcoinFixtureManifest {
    /// Manifest schema revision.
    pub schema_version: u32,
    /// Checksum semantics.
    pub checksum_scope: String,
    /// Validation guarantees made by this generated corpus.
    pub validation_scope: String,
    /// Corpus objects.
    pub objects: Vec<BitcoinFixtureObject>,
}

/// One immutable serialized Bitcoin object.
#[derive(Clone, Debug, Deserialize)]
pub struct BitcoinFixtureObject {
    /// Stable fixture name.
    pub name: String,
    /// Path relative to the Bitcoin fixture directory.
    pub path: String,
    /// Semantic object type.
    pub kind: String,
    /// Provenance class.
    pub source: String,
    /// License/provenance statement.
    pub license: String,
    /// SHA-256 over decoded consensus bytes.
    pub sha256: String,
    /// Expected parse result.
    pub expected: String,
    /// Optional expected transaction ID.
    pub expected_txid: Option<String>,
    /// Optional expected witness transaction ID.
    pub expected_wtxid: Option<String>,
    /// Optional expected block hash.
    pub expected_block_hash: Option<String>,
}

/// Fixture loading or integrity failure.
#[derive(Debug, Error)]
pub enum BitcoinFixtureError {
    /// File access failed.
    #[error("fixture file access failed: {0}")]
    Io(#[from] std::io::Error),
    /// Manifest JSON was invalid.
    #[error("fixture manifest is invalid: {0}")]
    Json(#[from] serde_json::Error),
    /// Hex object was malformed.
    #[error("fixture `{name}` contains invalid hex")]
    InvalidHex {
        /// Fixture name.
        name: String,
    },
    /// Decoded checksum did not match the manifest.
    #[error("fixture `{name}` checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// Fixture name.
        name: String,
        /// Manifest checksum.
        expected: String,
        /// Actual checksum.
        actual: String,
    },
}

impl BitcoinFixtureManifest {
    /// Loads the manifest from a repository root.
    ///
    /// # Errors
    ///
    /// Returns [`BitcoinFixtureError`] for inaccessible or invalid JSON.
    pub fn load(repository_root: &Path) -> Result<Self, BitcoinFixtureError> {
        let path = fixture_root(repository_root).join("manifest.json");
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    }

    /// Loads and validates every decoded consensus object.
    ///
    /// # Errors
    ///
    /// Returns [`BitcoinFixtureError`] on invalid hex or checksum drift.
    pub fn verify_all(&self, repository_root: &Path) -> Result<Vec<Vec<u8>>, BitcoinFixtureError> {
        self.objects
            .iter()
            .map(|object| object.load_verified(repository_root))
            .collect()
    }
}

impl BitcoinFixtureObject {
    /// Loads one decoded object and verifies its manifest checksum.
    ///
    /// # Errors
    ///
    /// Returns [`BitcoinFixtureError`] on file, hex, or checksum failure.
    pub fn load_verified(&self, repository_root: &Path) -> Result<Vec<u8>, BitcoinFixtureError> {
        let text = fs::read_to_string(fixture_root(repository_root).join(&self.path))?;
        let bytes = decode_hex(&self.name, text.trim())?;
        let actual = encode_hex(&Sha256::digest(&bytes));
        if actual != self.sha256 {
            return Err(BitcoinFixtureError::ChecksumMismatch {
                name: self.name.clone(),
                expected: self.sha256.clone(),
                actual,
            });
        }
        Ok(bytes)
    }
}

fn fixture_root(repository_root: &Path) -> PathBuf {
    repository_root.join("tests/fixtures/bitcoin")
}

fn decode_hex(name: &str, text: &str) -> Result<Vec<u8>, BitcoinFixtureError> {
    if !text.len().is_multiple_of(2) {
        return Err(BitcoinFixtureError::InvalidHex {
            name: name.to_owned(),
        });
    }
    (0..text.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&text[index..index + 2], 16).map_err(|_| {
                BitcoinFixtureError::InvalidHex {
                    name: name.to_owned(),
                }
            })
        })
        .collect()
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut hex, byte| {
            write!(hex, "{byte:02x}").expect("writing into a String cannot fail");
            hex
        })
}
