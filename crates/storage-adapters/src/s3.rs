use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::Arc,
};

use archive_format::{ArchiveManifest, EncodedArchive};
use object_store::{
    ObjectStore, ObjectStoreExt, PutMode, PutOptions, aws::AmazonS3Builder,
    path::Path as ObjectPath,
};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction};
use storage_ports::{ArchiveError, ManifestAck, RawArchive, StagedObject};
use tokio::sync::Mutex;

/// In-memory S3-compatible archive model with explicit stage, verify, and
/// manifest visibility boundaries.
#[derive(Clone, Debug, Default)]
pub struct MemoryRawArchive {
    state: Arc<Mutex<MemoryArchiveState>>,
    withhold_manifest_commits: bool,
}

#[derive(Debug, Default)]
struct MemoryArchiveState {
    staged: HashMap<String, Vec<u8>>,
    verified: HashSet<String>,
    manifests: HashMap<[u8; 32], ArchiveManifest>,
    latest_by_session: HashMap<[u8; 16], [u8; 32]>,
}

/// S3-compatible raw archive connection settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3ArchiveConfig {
    /// S3-compatible endpoint.
    pub endpoint: String,
    /// Raw archive bucket.
    pub bucket: String,
    /// Signing region.
    pub region: String,
    /// Access-key identifier.
    pub access_key_id: String,
    /// Secret access key.
    pub secret_access_key: String,
    /// Whether plain HTTP is permitted, intended only for local development.
    pub allow_http: bool,
}

/// S3-compatible exact-byte archive with `PostgreSQL` manifest serialization.
#[derive(Clone)]
pub struct S3RawArchive {
    store: Arc<dyn ObjectStore>,
    pool: PgPool,
}

impl std::fmt::Debug for S3RawArchive {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("S3RawArchive")
            .finish_non_exhaustive()
    }
}

impl S3RawArchive {
    /// Builds an S3-compatible client and binds it to the manifest ledger.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError`] when endpoint configuration is invalid.
    pub fn new(config: S3ArchiveConfig, pool: PgPool) -> Result<Self, ArchiveError> {
        let store = AmazonS3Builder::new()
            .with_endpoint(config.endpoint)
            .with_bucket_name(config.bucket)
            .with_region(config.region)
            .with_access_key_id(config.access_key_id)
            .with_secret_access_key(config.secret_access_key)
            .with_allow_http(config.allow_http)
            .with_virtual_hosted_style_request(false)
            .build()
            .map_err(object_error)?;
        Ok(Self {
            store: Arc::new(store),
            pool,
        })
    }

    /// Reads an archive object only through a committed manifest.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError`] when the manifest is absent or object readback
    /// fails validation.
    pub async fn replay_committed(
        &self,
        manifest_hash: [u8; 32],
    ) -> Result<Option<Vec<u8>>, ArchiveError> {
        let row = sqlx::query(
            "SELECT object_key, object_sha256, compressed_bytes \
             FROM archive_manifests WHERE manifest_hash = $1",
        )
        .bind(manifest_hash.to_vec())
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let object_key: String = row.try_get("object_key").map_err(database_error)?;
        let object_sha256 = fixed_database_bytes::<32>(
            row.try_get("object_sha256").map_err(database_error)?,
            "object_sha256",
        )?;
        let compressed_bytes = u64::from_be_bytes(fixed_database_bytes::<8>(
            row.try_get("compressed_bytes").map_err(database_error)?,
            "compressed_bytes",
        )?);
        let bytes = read_object(&self.store, &object_key).await?;
        validate_exact_bytes(&bytes, compressed_bytes, object_sha256)?;
        Ok(Some(bytes))
    }
}

impl MemoryRawArchive {
    /// Creates an adapter that fails the manifest visibility transition.
    #[must_use]
    pub fn withhold_manifest_commits() -> Self {
        Self {
            withhold_manifest_commits: true,
            ..Self::default()
        }
    }

    /// Stages an already encoded archive object.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError`] when staging fails.
    pub async fn stage_encoded(
        &self,
        object: EncodedArchive,
    ) -> Result<StagedObject, ArchiveError> {
        RawArchive::stage(self, object).await
    }

    /// Verifies staged object evidence.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError`] for missing, truncated, or corrupted bytes.
    pub async fn verify(&self, object: &StagedObject) -> Result<(), ArchiveError> {
        RawArchive::verify(self, object).await
    }

    /// Commits a verified manifest.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError`] when verification, overlap, or chaining fails.
    pub async fn commit_manifest(
        &self,
        manifest: ArchiveManifest,
    ) -> Result<ManifestAck, ArchiveError> {
        RawArchive::commit_manifest(self, manifest).await
    }

    /// Replaces staged bytes to simulate corruption between PUT and verify.
    pub async fn replace_staged_bytes(&self, object_key: &str, bytes: Vec<u8>) {
        let mut guard = self.state.lock().await;
        guard.staged.insert(object_key.to_owned(), bytes);
        guard.verified.remove(object_key);
    }

    /// Returns replay bytes only when a committed manifest names the object.
    pub async fn replay_by_object_key(&self, object_key: &str) -> Option<Vec<u8>> {
        let guard = self.state.lock().await;
        let committed = guard
            .manifests
            .values()
            .any(|manifest| manifest.object_key == object_key);
        committed
            .then(|| guard.staged.get(object_key).cloned())
            .flatten()
    }
}

impl RawArchive for MemoryRawArchive {
    fn stage(
        &self,
        object: EncodedArchive,
    ) -> impl Future<Output = Result<StagedObject, ArchiveError>> + Send {
        let state = Arc::clone(&self.state);
        async move {
            let compressed_bytes = u64::try_from(object.compressed_bytes().len())
                .map_err(|_| ArchiveError::Storage("object length exceeds u64".to_owned()))?;
            let staged = StagedObject::new(
                object.object_key().to_owned(),
                object.object_sha256(),
                compressed_bytes,
            );
            let mut guard = state.lock().await;
            guard.staged.insert(
                object.object_key().to_owned(),
                object.into_compressed_bytes(),
            );
            guard.verified.remove(staged.object_key());
            Ok(staged)
        }
    }

    fn verify(
        &self,
        object: &StagedObject,
    ) -> impl Future<Output = Result<(), ArchiveError>> + Send {
        let state = Arc::clone(&self.state);
        let object = object.clone();
        async move {
            let mut guard = state.lock().await;
            let bytes = guard
                .staged
                .get(object.object_key())
                .ok_or_else(|| ArchiveError::MissingObject(object.object_key().to_owned()))?;
            let actual_length = u64::try_from(bytes.len())
                .map_err(|_| ArchiveError::Storage("object length exceeds u64".to_owned()))?;
            if actual_length != object.compressed_bytes() {
                return Err(ArchiveError::LengthMismatch {
                    expected: object.compressed_bytes(),
                    actual: actual_length,
                });
            }
            let actual_sha256: [u8; 32] = Sha256::digest(bytes).into();
            if actual_sha256 != object.object_sha256() {
                return Err(ArchiveError::ChecksumMismatch);
            }
            guard.verified.insert(object.object_key().to_owned());
            Ok(())
        }
    }

    fn commit_manifest(
        &self,
        manifest: ArchiveManifest,
    ) -> impl Future<Output = Result<ManifestAck, ArchiveError>> + Send {
        let state = Arc::clone(&self.state);
        let withhold = self.withhold_manifest_commits;
        async move {
            if withhold {
                return Err(ArchiveError::CommitWithheld);
            }
            let manifest_hash = manifest
                .manifest_hash()
                .map_err(|error| ArchiveError::Encoding(error.to_string()))?;
            let mut guard = state.lock().await;

            if guard.manifests.contains_key(&manifest_hash) {
                return Ok(ManifestAck::new(
                    manifest_hash,
                    manifest.last_collector_sequence,
                ));
            }
            if !guard.verified.contains(&manifest.object_key) {
                return Err(ArchiveError::NotVerified(manifest.object_key));
            }
            let bytes = guard
                .staged
                .get(&manifest.object_key)
                .ok_or_else(|| ArchiveError::MissingObject(manifest.object_key.clone()))?;
            validate_manifest_object(&manifest, bytes)?;

            if guard.manifests.values().any(|existing| {
                existing.source_session_id == manifest.source_session_id
                    && ranges_overlap(
                        existing.first_collector_sequence,
                        existing.last_collector_sequence,
                        manifest.first_collector_sequence,
                        manifest.last_collector_sequence,
                    )
            }) {
                return Err(ArchiveError::OverlappingRange);
            }
            let latest = guard
                .latest_by_session
                .get(&manifest.source_session_id)
                .copied();
            if latest != manifest.previous_manifest_hash {
                return Err(ArchiveError::ManifestChainMismatch);
            }

            guard
                .latest_by_session
                .insert(manifest.source_session_id, manifest_hash);
            guard.manifests.insert(manifest_hash, manifest.clone());
            Ok(ManifestAck::new(
                manifest_hash,
                manifest.last_collector_sequence,
            ))
        }
    }

    fn latest_manifest_hash(
        &self,
        source_session_id: [u8; 16],
    ) -> impl Future<Output = Result<Option<[u8; 32]>, ArchiveError>> + Send {
        let state = Arc::clone(&self.state);
        async move {
            Ok(state
                .lock()
                .await
                .latest_by_session
                .get(&source_session_id)
                .copied())
        }
    }
}

impl RawArchive for S3RawArchive {
    fn stage(
        &self,
        object: EncodedArchive,
    ) -> impl Future<Output = Result<StagedObject, ArchiveError>> + Send {
        let store = Arc::clone(&self.store);
        async move {
            let compressed_bytes = u64::try_from(object.compressed_bytes().len())
                .map_err(|_| ArchiveError::Storage("object length exceeds u64".to_owned()))?;
            let staged = StagedObject::new(
                object.object_key().to_owned(),
                object.object_sha256(),
                compressed_bytes,
            );
            put_create_or_verify(
                &store,
                staged.object_key(),
                object.compressed_bytes().to_vec(),
                staged.compressed_bytes(),
                staged.object_sha256(),
            )
            .await?;
            Ok(staged)
        }
    }

    fn verify(
        &self,
        object: &StagedObject,
    ) -> impl Future<Output = Result<(), ArchiveError>> + Send {
        let store = Arc::clone(&self.store);
        let object = object.clone();
        async move {
            let path = object_path(object.object_key())?;
            let metadata = store.head(&path).await.map_err(object_error)?;
            let actual_length = metadata.size;
            if actual_length != object.compressed_bytes() {
                return Err(ArchiveError::LengthMismatch {
                    expected: object.compressed_bytes(),
                    actual: actual_length,
                });
            }
            let bytes = store
                .get(&path)
                .await
                .map_err(object_error)?
                .bytes()
                .await
                .map_err(object_error)?;
            validate_exact_bytes(
                bytes.as_ref(),
                object.compressed_bytes(),
                object.object_sha256(),
            )
        }
    }

    fn commit_manifest(
        &self,
        manifest: ArchiveManifest,
    ) -> impl Future<Output = Result<ManifestAck, ArchiveError>> + Send {
        let store = Arc::clone(&self.store);
        let pool = self.pool.clone();
        async move {
            let (manifest_hash, manifest_json) = prepare_manifest_commit(&store, &manifest).await?;
            let mut transaction = pool.begin().await.map_err(database_error)?;
            lock_manifest_session(&mut transaction, manifest.source_session_id).await?;
            if let Some(ack) = existing_manifest_ack(&mut transaction, manifest_hash).await? {
                transaction.commit().await.map_err(database_error)?;
                return Ok(ack);
            }
            validate_database_manifest_chain(&mut transaction, &manifest).await?;
            let manifest_key = manifest_object_key(manifest.source_session_id, manifest_hash);
            put_create_or_verify(
                &store,
                &manifest_key,
                manifest_json.clone(),
                u64::try_from(manifest_json.len())
                    .map_err(|_| ArchiveError::Storage("manifest length exceeds u64".to_owned()))?,
                Sha256::digest(&manifest_json).into(),
            )
            .await?;
            insert_manifest_and_head(&mut transaction, &manifest, manifest_hash).await?;
            transaction.commit().await.map_err(database_error)?;
            Ok(ManifestAck::new(
                manifest_hash,
                manifest.last_collector_sequence,
            ))
        }
    }

    fn latest_manifest_hash(
        &self,
        source_session_id: [u8; 16],
    ) -> impl Future<Output = Result<Option<[u8; 32]>, ArchiveError>> + Send {
        let pool = self.pool.clone();
        async move {
            let row = sqlx::query(
                "SELECT manifest_hash FROM archive_manifest_heads \
                 WHERE source_session_id = $1",
            )
            .bind(source_session_id.to_vec())
            .fetch_optional(&pool)
            .await
            .map_err(database_error)?;
            row.map(|row| {
                fixed_database_bytes::<32>(
                    row.try_get("manifest_hash").map_err(database_error)?,
                    "manifest_hash",
                )
            })
            .transpose()
        }
    }
}

async fn prepare_manifest_commit(
    store: &Arc<dyn ObjectStore>,
    manifest: &ArchiveManifest,
) -> Result<([u8; 32], Vec<u8>), ArchiveError> {
    let object_bytes = read_object(store, &manifest.object_key).await?;
    validate_manifest_object(manifest, &object_bytes)?;
    let manifest_hash = manifest
        .manifest_hash()
        .map_err(|error| ArchiveError::Encoding(error.to_string()))?;
    let manifest_json = manifest
        .json_bytes()
        .map_err(|error| ArchiveError::Encoding(error.to_string()))?;
    Ok((manifest_hash, manifest_json))
}

async fn lock_manifest_session(
    transaction: &mut Transaction<'_, Postgres>,
    source_session_id: [u8; 16],
) -> Result<(), ArchiveError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(encode($1, 'hex'), 0))")
        .bind(source_session_id.to_vec())
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
    Ok(())
}

async fn existing_manifest_ack(
    transaction: &mut Transaction<'_, Postgres>,
    manifest_hash: [u8; 32],
) -> Result<Option<ManifestAck>, ArchiveError> {
    let row = sqlx::query(
        "SELECT last_collector_sequence FROM archive_manifests \
         WHERE manifest_hash = $1",
    )
    .bind(manifest_hash.to_vec())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?;
    row.map(|row| {
        let last = u64::from_be_bytes(fixed_database_bytes::<8>(
            row.try_get("last_collector_sequence")
                .map_err(database_error)?,
            "last_collector_sequence",
        )?);
        Ok(ManifestAck::new(manifest_hash, last))
    })
    .transpose()
}

async fn validate_database_manifest_chain(
    transaction: &mut Transaction<'_, Postgres>,
    manifest: &ArchiveManifest,
) -> Result<(), ArchiveError> {
    let head = sqlx::query(
        "SELECT manifest_hash, last_collector_sequence \
         FROM archive_manifest_heads \
         WHERE source_session_id = $1 FOR UPDATE",
    )
    .bind(manifest.source_session_id.to_vec())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?;
    let Some(head) = head else {
        return if manifest.previous_manifest_hash.is_none() {
            Ok(())
        } else {
            Err(ArchiveError::ManifestChainMismatch)
        };
    };
    let current_hash = fixed_database_bytes::<32>(
        head.try_get("manifest_hash").map_err(database_error)?,
        "manifest_hash",
    )?;
    let current_last = u64::from_be_bytes(fixed_database_bytes::<8>(
        head.try_get("last_collector_sequence")
            .map_err(database_error)?,
        "last_collector_sequence",
    )?);
    if manifest.previous_manifest_hash != Some(current_hash) {
        return Err(ArchiveError::ManifestChainMismatch);
    }
    if manifest.first_collector_sequence <= current_last {
        return Err(ArchiveError::OverlappingRange);
    }
    Ok(())
}

async fn insert_manifest_and_head(
    transaction: &mut Transaction<'_, Postgres>,
    manifest: &ArchiveManifest,
    manifest_hash: [u8; 32],
) -> Result<(), ArchiveError> {
    sqlx::query(
        "INSERT INTO archive_manifests (
            manifest_hash, source_session_id, first_collector_sequence,
            last_collector_sequence, object_key, object_sha256,
            compressed_bytes, record_count, previous_manifest_hash
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(manifest_hash.to_vec())
    .bind(manifest.source_session_id.to_vec())
    .bind(manifest.first_collector_sequence.to_be_bytes().to_vec())
    .bind(manifest.last_collector_sequence.to_be_bytes().to_vec())
    .bind(&manifest.object_key)
    .bind(manifest.object_sha256.to_vec())
    .bind(manifest.compressed_bytes.to_be_bytes().to_vec())
    .bind(manifest.record_count.to_be_bytes().to_vec())
    .bind(manifest.previous_manifest_hash.map(|hash| hash.to_vec()))
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    sqlx::query(
        "INSERT INTO archive_manifest_heads (
            source_session_id, manifest_hash, last_collector_sequence
         ) VALUES ($1, $2, $3)
         ON CONFLICT (source_session_id)
         DO UPDATE SET
            manifest_hash = EXCLUDED.manifest_hash,
            last_collector_sequence = EXCLUDED.last_collector_sequence,
            updated_at = clock_timestamp()",
    )
    .bind(manifest.source_session_id.to_vec())
    .bind(manifest_hash.to_vec())
    .bind(manifest.last_collector_sequence.to_be_bytes().to_vec())
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    Ok(())
}

fn validate_manifest_object(manifest: &ArchiveManifest, bytes: &[u8]) -> Result<(), ArchiveError> {
    let actual_length = u64::try_from(bytes.len())
        .map_err(|_| ArchiveError::Storage("object length exceeds u64".to_owned()))?;
    if actual_length != manifest.compressed_bytes {
        return Err(ArchiveError::LengthMismatch {
            expected: manifest.compressed_bytes,
            actual: actual_length,
        });
    }
    let actual_sha256: [u8; 32] = Sha256::digest(bytes).into();
    if actual_sha256 != manifest.object_sha256 {
        return Err(ArchiveError::ChecksumMismatch);
    }
    Ok(())
}

const fn ranges_overlap(first_a: u64, last_a: u64, first_b: u64, last_b: u64) -> bool {
    first_a <= last_b && first_b <= last_a
}

async fn put_create_or_verify(
    store: &Arc<dyn ObjectStore>,
    key: &str,
    bytes: Vec<u8>,
    expected_length: u64,
    expected_sha256: [u8; 32],
) -> Result<(), ArchiveError> {
    let path = object_path(key)?;
    let options = PutOptions {
        mode: PutMode::Create,
        ..PutOptions::default()
    };
    if store.put_opts(&path, bytes.into(), options).await.is_err() {
        let existing = read_object(store, key).await?;
        validate_exact_bytes(&existing, expected_length, expected_sha256)?;
    }
    Ok(())
}

async fn read_object(store: &Arc<dyn ObjectStore>, key: &str) -> Result<Vec<u8>, ArchiveError> {
    let path = object_path(key)?;
    Ok(store
        .get(&path)
        .await
        .map_err(object_error)?
        .bytes()
        .await
        .map_err(object_error)?
        .to_vec())
}

fn validate_exact_bytes(
    bytes: &[u8],
    expected_length: u64,
    expected_sha256: [u8; 32],
) -> Result<(), ArchiveError> {
    let actual_length = u64::try_from(bytes.len())
        .map_err(|_| ArchiveError::Storage("object length exceeds u64".to_owned()))?;
    if actual_length != expected_length {
        return Err(ArchiveError::LengthMismatch {
            expected: expected_length,
            actual: actual_length,
        });
    }
    let actual_sha256: [u8; 32] = Sha256::digest(bytes).into();
    if actual_sha256 != expected_sha256 {
        return Err(ArchiveError::ChecksumMismatch);
    }
    Ok(())
}

fn object_path(key: &str) -> Result<ObjectPath, ArchiveError> {
    ObjectPath::parse(key).map_err(|error| ArchiveError::Storage(error.to_string()))
}

fn object_error(error: impl std::fmt::Display) -> ArchiveError {
    ArchiveError::Storage(error.to_string())
}

fn database_error(error: impl std::fmt::Display) -> ArchiveError {
    ArchiveError::Storage(error.to_string())
}

fn fixed_database_bytes<const N: usize>(
    value: Vec<u8>,
    field: &'static str,
) -> Result<[u8; N], ArchiveError> {
    value.try_into().map_err(|value: Vec<u8>| {
        ArchiveError::Storage(format!(
            "stored {field} has {} bytes, expected {N}",
            value.len()
        ))
    })
}

fn manifest_object_key(source_session_id: [u8; 16], manifest_hash: [u8; 32]) -> String {
    format!(
        "manifests/source_session={}/manifest-{}.json",
        hex(&source_session_id),
        hex(&manifest_hash)
    )
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}
