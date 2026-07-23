use std::{collections::HashMap, future::Future, sync::Arc};

use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use storage_ports::{CheckpointError, CheckpointKind, CheckpointStore, DurableCheckpoint};
use tokio::sync::Mutex;

type MemoryCheckpointKey = (CheckpointKind, String, [u8; 16]);
type MemoryCheckpointMap = HashMap<MemoryCheckpointKey, DurableCheckpoint>;

/// In-memory checkpoint store with transactional monotonicity semantics.
#[derive(Clone, Debug, Default)]
pub struct MemoryCheckpointStore {
    checkpoints: Arc<Mutex<MemoryCheckpointMap>>,
}

/// PostgreSQL-backed control-plane checkpoint store.
#[derive(Clone, Debug)]
pub struct PostgresCheckpointStore {
    pool: PgPool,
}

impl PostgresCheckpointStore {
    /// Connects a bounded `PostgreSQL` pool.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError`] when the pool cannot connect.
    pub async fn connect(
        database_url: &str,
        max_connections: u32,
    ) -> Result<Self, CheckpointError> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .connect(database_url)
            .await
            .map_err(storage_error)?;
        Ok(Self { pool })
    }

    /// Wraps an existing pool.
    #[must_use]
    pub const fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Returns the underlying pool for manifest coordination.
    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Installs the Task 4 control schema.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError`] if any statement fails.
    pub async fn install_schema(&self) -> Result<(), CheckpointError> {
        sqlx::raw_sql(include_str!("../../../schemas/postgres/001_control.sql"))
            .execute(&self.pool)
            .await
            .map_err(storage_error)?;
        Ok(())
    }

    /// Loads a durable checkpoint.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError`] when storage access or decoding fails.
    pub async fn load(
        &self,
        kind: CheckpointKind,
        source_id: &str,
        source_session_id: [u8; 16],
    ) -> Result<Option<DurableCheckpoint>, CheckpointError> {
        CheckpointStore::load(self, kind, source_id, source_session_id).await
    }

    /// Advances a durable checkpoint transactionally.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError`] for storage failures, regression, or source
    /// session mismatch.
    pub async fn advance(
        &self,
        kind: CheckpointKind,
        source_id: &str,
        checkpoint: DurableCheckpoint,
    ) -> Result<DurableCheckpoint, CheckpointError> {
        CheckpointStore::advance(self, kind, source_id, checkpoint).await
    }
}

impl MemoryCheckpointStore {
    /// Loads a checkpoint without requiring trait import at call sites.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError`] when stored state is invalid.
    pub async fn load(
        &self,
        kind: CheckpointKind,
        source_id: &str,
        source_session_id: [u8; 16],
    ) -> Result<Option<DurableCheckpoint>, CheckpointError> {
        CheckpointStore::load(self, kind, source_id, source_session_id).await
    }

    /// Advances a checkpoint without requiring trait import at call sites.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError`] for regression or source-session mismatch.
    pub async fn advance(
        &self,
        kind: CheckpointKind,
        source_id: &str,
        checkpoint: DurableCheckpoint,
    ) -> Result<DurableCheckpoint, CheckpointError> {
        CheckpointStore::advance(self, kind, source_id, checkpoint).await
    }
}

impl CheckpointStore for MemoryCheckpointStore {
    fn load(
        &self,
        kind: CheckpointKind,
        source_id: &str,
        source_session_id: [u8; 16],
    ) -> impl Future<Output = Result<Option<DurableCheckpoint>, CheckpointError>> + Send {
        let checkpoints = Arc::clone(&self.checkpoints);
        let source_id = source_id.to_owned();
        async move {
            Ok(checkpoints
                .lock()
                .await
                .get(&(kind, source_id, source_session_id))
                .copied())
        }
    }

    fn advance(
        &self,
        kind: CheckpointKind,
        source_id: &str,
        checkpoint: DurableCheckpoint,
    ) -> impl Future<Output = Result<DurableCheckpoint, CheckpointError>> + Send {
        let checkpoints = Arc::clone(&self.checkpoints);
        let source_id = source_id.to_owned();
        async move {
            let mut guard = checkpoints.lock().await;
            let key = (kind, source_id, checkpoint.source_session_id());
            let next = DurableCheckpoint::advance(guard.get(&key), checkpoint)?;
            guard.insert(key, next);
            Ok(next)
        }
    }
}

impl CheckpointStore for PostgresCheckpointStore {
    fn load(
        &self,
        kind: CheckpointKind,
        source_id: &str,
        source_session_id: [u8; 16],
    ) -> impl Future<Output = Result<Option<DurableCheckpoint>, CheckpointError>> + Send {
        let pool = self.pool.clone();
        let source_id = source_id.to_owned();
        async move {
            let row = sqlx::query(
                "SELECT source_session_id, last_collector_sequence \
                 FROM source_checkpoints \
                 WHERE checkpoint_kind = $1 AND source_id = $2 \
                   AND source_session_id = $3",
            )
            .bind(checkpoint_kind(kind))
            .bind(source_id)
            .bind(source_session_id.to_vec())
            .fetch_optional(&pool)
            .await
            .map_err(storage_error)?;
            row.map(|row| decode_checkpoint(&row)).transpose()
        }
    }

    fn advance(
        &self,
        kind: CheckpointKind,
        source_id: &str,
        checkpoint: DurableCheckpoint,
    ) -> impl Future<Output = Result<DurableCheckpoint, CheckpointError>> + Send {
        let pool = self.pool.clone();
        let source_id = source_id.to_owned();
        async move {
            let mut transaction = pool.begin().await.map_err(storage_error)?;
            let lock_key = format!(
                "{}:{source_id}:{}",
                checkpoint_kind(kind),
                session_hex(checkpoint.source_session_id())
            );
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(lock_key)
                .execute(&mut *transaction)
                .await
                .map_err(storage_error)?;
            let current = sqlx::query(
                "SELECT source_session_id, last_collector_sequence \
                 FROM source_checkpoints \
                 WHERE checkpoint_kind = $1 AND source_id = $2 \
                   AND source_session_id = $3 \
                 FOR UPDATE",
            )
            .bind(checkpoint_kind(kind))
            .bind(&source_id)
            .bind(checkpoint.source_session_id().to_vec())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(storage_error)?
            .map(|row| decode_checkpoint(&row))
            .transpose()?;
            let next = DurableCheckpoint::advance(current.as_ref(), checkpoint)?;
            sqlx::query(
                "INSERT INTO source_checkpoints (
                    checkpoint_kind,
                    source_id,
                    source_session_id,
                    last_collector_sequence
                 ) VALUES ($1, $2, $3, $4)
                 ON CONFLICT (checkpoint_kind, source_id, source_session_id)
                 DO UPDATE SET
                    last_collector_sequence = EXCLUDED.last_collector_sequence,
                    updated_at = clock_timestamp()",
            )
            .bind(checkpoint_kind(kind))
            .bind(source_id)
            .bind(next.source_session_id().to_vec())
            .bind(next.last_collector_sequence().to_be_bytes().to_vec())
            .execute(&mut *transaction)
            .await
            .map_err(storage_error)?;
            transaction.commit().await.map_err(storage_error)?;
            Ok(next)
        }
    }
}

const fn checkpoint_kind(kind: CheckpointKind) -> &'static str {
    match kind {
        CheckpointKind::Broker => "broker",
        CheckpointKind::Archive => "archive",
    }
}

fn decode_checkpoint(row: &sqlx::postgres::PgRow) -> Result<DurableCheckpoint, CheckpointError> {
    let session: Vec<u8> = row.try_get("source_session_id").map_err(storage_error)?;
    let sequence: Vec<u8> = row
        .try_get("last_collector_sequence")
        .map_err(storage_error)?;
    let source_session_id: [u8; 16] = session.try_into().map_err(|value: Vec<u8>| {
        CheckpointError::Storage(format!(
            "stored source session has {} bytes, expected 16",
            value.len()
        ))
    })?;
    let last_collector_sequence =
        u64::from_be_bytes(sequence.try_into().map_err(|value: Vec<u8>| {
            CheckpointError::Storage(format!(
                "stored collector sequence has {} bytes, expected 8",
                value.len()
            ))
        })?);
    Ok(DurableCheckpoint::new(
        source_session_id,
        last_collector_sequence,
    ))
}

fn storage_error(error: impl std::fmt::Display) -> CheckpointError {
    CheckpointError::Storage(error.to_string())
}

fn session_hex(bytes: [u8; 16]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}
