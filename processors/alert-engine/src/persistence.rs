use std::{collections::BTreeMap, future::Future, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Row as _, postgres::PgPoolOptions};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::{AlertTransition, MempoolAlertEvaluation, QuorumVbytesAboveDefinition};

const ALERT_KIND: &str = "bitcoin_mempool_quorum_vbytes_above";
const MAX_FACT_COUNT: usize = 128;
const MAX_FACT_ID_LEN: usize = 256;
const MAX_DELIVERY_BATCH: u16 = 1_000;

/// Whether an idempotent durable write inserted a new logical record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistOutcome {
    /// The transaction committed a new logical record.
    Inserted,
    /// The exact logical record already existed.
    Duplicate,
}

/// Auditable durable counts and current revision for one alert definition.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AlertAuditSummary {
    /// Immutable evaluation rows.
    pub evaluation_count: u64,
    /// All transactional outbox rows.
    pub outbox_count: u64,
    /// Outbox rows still awaiting a successful sink acknowledgement.
    pub pending_outbox_count: u64,
    /// Successfully acknowledged outbox rows.
    pub delivered_outbox_count: u64,
    /// Current monotonic input revision.
    pub last_revision: Option<u64>,
}

/// One outbox delivery retaining its deterministic idempotency identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OutboxDelivery {
    /// Sink deduplication key.
    pub idempotency_key: String,
    /// Evaluation that created this delivery.
    pub evaluation_id: String,
    /// Alert definition identity.
    pub alert_id: String,
    /// Immutable definition version.
    pub definition_version: u64,
    /// Exact serialized evaluation payload.
    pub payload: Value,
}

/// Result of one bounded outbox drain.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DeliveryReport {
    /// Rows acknowledged and moved to `delivered`.
    pub delivered: u16,
    /// Sink failures retained as pending rows with audit metadata.
    pub failed: u16,
}

/// Idempotent notification boundary used by the transactional outbox.
pub trait AlertDeliverySink: Send + Sync {
    /// Accepts one delivery. Implementations must deduplicate by
    /// [`OutboxDelivery::idempotency_key`].
    fn deliver(
        &self,
        delivery: OutboxDelivery,
    ) -> impl Future<Output = Result<(), AlertDeliveryError>> + Send;
}

/// In-memory idempotent sink used for deterministic delivery tests.
#[derive(Clone, Debug, Default)]
pub struct MemoryAlertDeliverySink {
    deliveries: Arc<Mutex<BTreeMap<String, OutboxDelivery>>>,
}

impl MemoryAlertDeliverySink {
    /// Returns accepted deliveries in idempotency-key order.
    pub async fn deliveries(&self) -> Vec<OutboxDelivery> {
        self.deliveries.lock().await.values().cloned().collect()
    }
}

impl AlertDeliverySink for MemoryAlertDeliverySink {
    fn deliver(
        &self,
        delivery: OutboxDelivery,
    ) -> impl Future<Output = Result<(), AlertDeliveryError>> + Send {
        let deliveries = Arc::clone(&self.deliveries);
        async move {
            deliveries
                .lock()
                .await
                .entry(delivery.idempotency_key.clone())
                .or_insert(delivery);
            Ok(())
        }
    }
}

/// `PostgreSQL`-backed alert definition, state, evidence, and outbox store.
#[derive(Clone, Debug)]
pub struct PostgresAlertStore {
    pool: PgPool,
}

impl PostgresAlertStore {
    /// Connects a bounded `PostgreSQL` pool.
    ///
    /// # Errors
    ///
    /// Returns [`AlertPersistenceError::Storage`] when the connection fails.
    pub async fn connect(
        database_url: &str,
        max_connections: u32,
    ) -> Result<Self, AlertPersistenceError> {
        if max_connections == 0 {
            return Err(AlertPersistenceError::InvalidConnectionLimit);
        }
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

    /// Installs the Task 15 alert schema.
    ///
    /// # Errors
    ///
    /// Returns [`AlertPersistenceError::Storage`] if any statement fails.
    pub async fn install_schema(&self) -> Result<(), AlertPersistenceError> {
        sqlx::raw_sql(include_str!("../../../schemas/postgres/002_alerts.sql"))
            .execute(&self.pool)
            .await
            .map_err(storage_error)?;
        Ok(())
    }

    /// Registers one immutable mempool alert definition version.
    ///
    /// Replaying byte-equivalent content is idempotent. Reusing the same
    /// identity and version for different content fails closed.
    ///
    /// # Errors
    ///
    /// Returns a validation, conflict, serialization, or storage failure.
    pub async fn register_mempool_definition(
        &self,
        definition_version: u64,
        definition: &QuorumVbytesAboveDefinition,
    ) -> Result<PersistOutcome, AlertPersistenceError> {
        let version = checked_positive_i64(definition_version)?;
        let definition_json = serde_json::to_value(definition).map_err(serialization_error)?;
        let definition_text =
            serde_json::to_string(&definition_json).map_err(serialization_error)?;
        let mut transaction = self.pool.begin().await.map_err(storage_error)?;
        lock_definition(&mut transaction, definition.alert_id(), version).await?;

        let existing = sqlx::query(
            "SELECT definition::text AS definition \
             FROM alert_definitions \
             WHERE alert_id = $1 AND definition_version = $2",
        )
        .bind(definition.alert_id())
        .bind(version)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage_error)?;
        if let Some(existing) = existing {
            let stored: String = existing.try_get("definition").map_err(storage_error)?;
            let stored: Value = serde_json::from_str(&stored).map_err(serialization_error)?;
            transaction.commit().await.map_err(storage_error)?;
            return if stored == definition_json {
                Ok(PersistOutcome::Duplicate)
            } else {
                Err(AlertPersistenceError::DefinitionConflict {
                    version: definition_version,
                })
            };
        }

        sqlx::query(
            "INSERT INTO alert_definitions (
                alert_id,
                definition_version,
                kind,
                definition
             ) VALUES ($1, $2, $3, $4::jsonb)",
        )
        .bind(definition.alert_id())
        .bind(version)
        .bind(ALERT_KIND)
        .bind(definition_text)
        .execute(&mut *transaction)
        .await
        .map_err(storage_error)?;
        transaction.commit().await.map_err(storage_error)?;
        Ok(PersistOutcome::Inserted)
    }

    /// Atomically appends evaluation evidence, advances current state, and
    /// creates a delivery outbox row when required.
    ///
    /// # Errors
    ///
    /// Fails before writing for invalid evidence. Revision regression or
    /// conflicting reuse rolls the entire transaction back.
    pub async fn persist_mempool_evaluation(
        &self,
        definition_version: u64,
        evaluated_at_unix_seconds: i64,
        input_fact_ids: &[&str],
        evaluation: &MempoolAlertEvaluation,
    ) -> Result<PersistOutcome, AlertPersistenceError> {
        validate_evaluation(
            definition_version,
            evaluated_at_unix_seconds,
            input_fact_ids,
            evaluation,
        )?;
        let write = EvaluationWrite::new(
            definition_version,
            evaluated_at_unix_seconds,
            input_fact_ids,
            evaluation,
        )?;

        let mut transaction = self.pool.begin().await.map_err(storage_error)?;
        lock_definition(
            &mut transaction,
            &evaluation.alert_id,
            write.definition_version,
        )
        .await?;
        ensure_definition_exists(
            &mut transaction,
            &evaluation.alert_id,
            write.definition_version,
            definition_version,
        )
        .await?;
        if check_current_state(&mut transaction, &write).await? {
            transaction.commit().await.map_err(storage_error)?;
            return Ok(PersistOutcome::Duplicate);
        }

        insert_evaluation(&mut transaction, &write).await?;
        advance_state(&mut transaction, &write).await?;
        insert_outbox(&mut transaction, &write).await?;
        transaction.commit().await.map_err(storage_error)?;
        Ok(PersistOutcome::Inserted)
    }

    /// Returns auditable counts and the current revision for an alert.
    ///
    /// # Errors
    ///
    /// Returns a storage or stored-value decoding failure.
    pub async fn audit_summary(
        &self,
        alert_id: &str,
        definition_version: u64,
    ) -> Result<AlertAuditSummary, AlertPersistenceError> {
        let version = checked_positive_i64(definition_version)?;
        let row = sqlx::query(
            "SELECT
                (
                    SELECT count(*)
                    FROM alert_evaluations
                    WHERE alert_id = $1 AND definition_version = $2
                ) AS evaluation_count,
                (
                    SELECT count(*)
                    FROM alert_outbox
                    WHERE alert_id = $1 AND definition_version = $2
                ) AS outbox_count,
                (
                    SELECT count(*)
                    FROM alert_outbox
                    WHERE alert_id = $1
                      AND definition_version = $2
                      AND status = 'pending'
                ) AS pending_outbox_count,
                (
                    SELECT count(*)
                    FROM alert_outbox
                    WHERE alert_id = $1
                      AND definition_version = $2
                      AND status = 'delivered'
                ) AS delivered_outbox_count,
                (
                    SELECT max(last_input_revision)
                    FROM alert_state
                    WHERE alert_id = $1 AND definition_version = $2
                ) AS last_revision",
        )
        .bind(alert_id)
        .bind(version)
        .fetch_one(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(AlertAuditSummary {
            evaluation_count: decode_count(&row, "evaluation_count")?,
            outbox_count: decode_count(&row, "outbox_count")?,
            pending_outbox_count: decode_count(&row, "pending_outbox_count")?,
            delivered_outbox_count: decode_count(&row, "delivered_outbox_count")?,
            last_revision: row
                .try_get::<Option<i64>, _>("last_revision")
                .map_err(storage_error)?
                .map(|value| u64::try_from(value).map_err(storage_error))
                .transpose()?,
        })
    }

    /// Delivers a bounded pending batch and records success/failure audit state.
    ///
    /// Delivery remains at-least-once: a crash after sink acknowledgement but
    /// before the status update can replay the row, so sinks must honor the
    /// idempotency key.
    ///
    /// # Errors
    ///
    /// Returns a validation, storage, or stored-payload decoding failure.
    pub async fn deliver_pending<S: AlertDeliverySink>(
        &self,
        sink: &S,
        limit: u16,
    ) -> Result<DeliveryReport, AlertPersistenceError> {
        if limit == 0 || limit > MAX_DELIVERY_BATCH {
            return Err(AlertPersistenceError::InvalidDeliveryBatch);
        }
        let rows = sqlx::query(
            "SELECT
                idempotency_key,
                evaluation_id,
                alert_id,
                definition_version,
                payload::text AS payload
             FROM alert_outbox
             WHERE status = 'pending' AND available_at <= clock_timestamp()
             ORDER BY available_at, created_at, idempotency_key
             LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?;

        let mut report = DeliveryReport::default();
        for row in rows {
            let key: String = row.try_get("idempotency_key").map_err(storage_error)?;
            let payload: String = row.try_get("payload").map_err(storage_error)?;
            let version: i64 = row.try_get("definition_version").map_err(storage_error)?;
            let delivery = OutboxDelivery {
                idempotency_key: key.clone(),
                evaluation_id: row.try_get("evaluation_id").map_err(storage_error)?,
                alert_id: row.try_get("alert_id").map_err(storage_error)?,
                definition_version: u64::try_from(version).map_err(storage_error)?,
                payload: serde_json::from_str(&payload).map_err(serialization_error)?,
            };
            match sink.deliver(delivery).await {
                Ok(()) => {
                    let updated = sqlx::query(
                        "UPDATE alert_outbox
                         SET
                            status = 'delivered',
                            delivery_attempts = delivery_attempts + 1,
                            delivered_at = clock_timestamp(),
                            last_error = NULL,
                            updated_at = clock_timestamp()
                         WHERE idempotency_key = $1 AND status = 'pending'",
                    )
                    .bind(&key)
                    .execute(&self.pool)
                    .await
                    .map_err(storage_error)?;
                    if updated.rows_affected() == 1 {
                        report.delivered = report.delivered.saturating_add(1);
                    }
                }
                Err(error) => {
                    let error = truncate_error(error.to_string());
                    sqlx::query(
                        "UPDATE alert_outbox
                         SET
                            delivery_attempts = delivery_attempts + 1,
                            last_error = $2,
                            updated_at = clock_timestamp()
                         WHERE idempotency_key = $1 AND status = 'pending'",
                    )
                    .bind(&key)
                    .bind(error)
                    .execute(&self.pool)
                    .await
                    .map_err(storage_error)?;
                    report.failed = report.failed.saturating_add(1);
                }
            }
        }
        Ok(report)
    }
}

struct EvaluationWrite<'a> {
    definition_version: i64,
    revision: i64,
    evaluated_at_unix_seconds: i64,
    fact_ids: Vec<String>,
    result: String,
    source_health: String,
    transition: &'static str,
    evaluation: &'a MempoolAlertEvaluation,
}

impl<'a> EvaluationWrite<'a> {
    fn new(
        definition_version: u64,
        evaluated_at_unix_seconds: i64,
        input_fact_ids: &[&str],
        evaluation: &'a MempoolAlertEvaluation,
    ) -> Result<Self, AlertPersistenceError> {
        Ok(Self {
            definition_version: checked_positive_i64(definition_version)?,
            revision: checked_positive_i64(evaluation.revision)?,
            evaluated_at_unix_seconds,
            fact_ids: normalized_fact_ids(input_fact_ids)?,
            result: serde_json::to_string(evaluation).map_err(serialization_error)?,
            source_health: serde_json::to_string(&json!({
                "contributing_sources": evaluation.contributing_sources,
                "unavailable_sources": evaluation.unavailable_sources,
                "completeness": evaluation.completeness,
                "cause": evaluation.cause,
            }))
            .map_err(serialization_error)?,
            transition: transition_name(evaluation.transition),
            evaluation,
        })
    }
}

async fn check_current_state(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    write: &EvaluationWrite<'_>,
) -> Result<bool, AlertPersistenceError> {
    let current = sqlx::query(
        "SELECT last_input_revision, last_evaluation_id \
         FROM alert_state \
         WHERE alert_id = $1 AND definition_version = $2 \
         FOR UPDATE",
    )
    .bind(&write.evaluation.alert_id)
    .bind(write.definition_version)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(storage_error)?;
    let Some(current) = current else {
        return Ok(false);
    };
    let stored_revision: i64 = current
        .try_get("last_input_revision")
        .map_err(storage_error)?;
    let stored_evaluation_id: String = current
        .try_get("last_evaluation_id")
        .map_err(storage_error)?;
    if write.revision < stored_revision {
        return check_historical_replay(transaction, write, stored_revision).await;
    }
    if write.revision > stored_revision {
        return Ok(false);
    }
    if stored_evaluation_id != write.evaluation.evaluation_id {
        return Err(AlertPersistenceError::ConflictingRevision {
            revision: write.evaluation.revision,
        });
    }
    if evaluation_matches(
        transaction,
        &write.evaluation.evaluation_id,
        write.evaluated_at_unix_seconds,
        &write.fact_ids,
        &write.result,
    )
    .await?
    {
        Ok(true)
    } else {
        Err(AlertPersistenceError::EvaluationConflict)
    }
}

async fn check_historical_replay(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    write: &EvaluationWrite<'_>,
    current_revision: i64,
) -> Result<bool, AlertPersistenceError> {
    let historical_id = sqlx::query_scalar::<_, String>(
        "SELECT evaluation_id
         FROM alert_evaluations
         WHERE alert_id = $1
           AND definition_version = $2
           AND input_revision = $3",
    )
    .bind(&write.evaluation.alert_id)
    .bind(write.definition_version)
    .bind(write.revision)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(storage_error)?;
    let Some(historical_id) = historical_id else {
        return Err(AlertPersistenceError::RevisionRegression {
            current: u64::try_from(current_revision).map_err(storage_error)?,
            attempted: write.evaluation.revision,
        });
    };
    if historical_id != write.evaluation.evaluation_id {
        return Err(AlertPersistenceError::ConflictingRevision {
            revision: write.evaluation.revision,
        });
    }
    if evaluation_matches(
        transaction,
        &historical_id,
        write.evaluated_at_unix_seconds,
        &write.fact_ids,
        &write.result,
    )
    .await?
    {
        Ok(true)
    } else {
        Err(AlertPersistenceError::EvaluationConflict)
    }
}

async fn insert_evaluation(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    write: &EvaluationWrite<'_>,
) -> Result<(), AlertPersistenceError> {
    sqlx::query(
        "INSERT INTO alert_evaluations (
            evaluation_id,
            alert_id,
            definition_version,
            kind,
            network,
            input_revision,
            input_fact_ids,
            source_health,
            result,
            transition,
            evaluated_at_unix_seconds
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8::jsonb, $9::jsonb, $10, $11
         )",
    )
    .bind(&write.evaluation.evaluation_id)
    .bind(&write.evaluation.alert_id)
    .bind(write.definition_version)
    .bind(write.evaluation.kind)
    .bind(&write.evaluation.network)
    .bind(write.revision)
    .bind(&write.fact_ids)
    .bind(&write.source_health)
    .bind(&write.result)
    .bind(write.transition)
    .bind(write.evaluated_at_unix_seconds)
    .execute(&mut **transaction)
    .await
    .map_err(storage_error)?;
    Ok(())
}

async fn advance_state(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    write: &EvaluationWrite<'_>,
) -> Result<(), AlertPersistenceError> {
    sqlx::query(
        "INSERT INTO alert_state (
            alert_id,
            definition_version,
            last_evaluation_id,
            last_input_revision,
            active,
            consecutive_true_evaluations,
            last_evaluated_at_unix_seconds,
            last_triggered_at_unix_seconds
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7,
            CASE WHEN $8 = 'triggered' THEN $7 ELSE NULL END
         )
         ON CONFLICT (alert_id, definition_version)
         DO UPDATE SET
            last_evaluation_id = EXCLUDED.last_evaluation_id,
            last_input_revision = EXCLUDED.last_input_revision,
            active = EXCLUDED.active,
            consecutive_true_evaluations =
                EXCLUDED.consecutive_true_evaluations,
            last_evaluated_at_unix_seconds =
                EXCLUDED.last_evaluated_at_unix_seconds,
            last_triggered_at_unix_seconds =
                CASE
                    WHEN $8 = 'triggered'
                    THEN EXCLUDED.last_evaluated_at_unix_seconds
                    ELSE alert_state.last_triggered_at_unix_seconds
                END,
            updated_at = clock_timestamp()",
    )
    .bind(&write.evaluation.alert_id)
    .bind(write.definition_version)
    .bind(&write.evaluation.evaluation_id)
    .bind(write.revision)
    .bind(write.evaluation.active)
    .bind(i32::from(write.evaluation.consecutive_true_evaluations))
    .bind(write.evaluated_at_unix_seconds)
    .bind(write.transition)
    .execute(&mut **transaction)
    .await
    .map_err(storage_error)?;
    Ok(())
}

async fn insert_outbox(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    write: &EvaluationWrite<'_>,
) -> Result<(), AlertPersistenceError> {
    let Some(idempotency_key) = &write.evaluation.outbox_idempotency_key else {
        return Ok(());
    };
    sqlx::query(
        "INSERT INTO alert_outbox (
            idempotency_key,
            evaluation_id,
            alert_id,
            definition_version,
            payload
         ) VALUES ($1, $2, $3, $4, $5::jsonb)",
    )
    .bind(idempotency_key)
    .bind(&write.evaluation.evaluation_id)
    .bind(&write.evaluation.alert_id)
    .bind(write.definition_version)
    .bind(&write.result)
    .execute(&mut **transaction)
    .await
    .map_err(storage_error)?;
    Ok(())
}

async fn lock_definition(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    alert_id: &str,
    version: i64,
) -> Result<(), AlertPersistenceError> {
    let lock_key = format!("{alert_id}:{version}");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(lock_key)
        .execute(&mut **transaction)
        .await
        .map_err(storage_error)?;
    Ok(())
}

async fn ensure_definition_exists(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    alert_id: &str,
    version: i64,
    public_version: u64,
) -> Result<(), AlertPersistenceError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1
            FROM alert_definitions
            WHERE alert_id = $1
              AND definition_version = $2
              AND status = 'enabled'
        )",
    )
    .bind(alert_id)
    .bind(version)
    .fetch_one(&mut **transaction)
    .await
    .map_err(storage_error)?;
    if !exists {
        return Err(AlertPersistenceError::DefinitionUnavailable {
            version: public_version,
        });
    }
    Ok(())
}

async fn evaluation_matches(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    evaluation_id: &str,
    evaluated_at_unix_seconds: i64,
    input_fact_ids: &[String],
    result: &str,
) -> Result<bool, AlertPersistenceError> {
    let row = sqlx::query(
        "SELECT
            input_fact_ids,
            evaluated_at_unix_seconds,
            result::text AS result
         FROM alert_evaluations
         WHERE evaluation_id = $1",
    )
    .bind(evaluation_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(storage_error)?
    .ok_or(AlertPersistenceError::EvaluationConflict)?;
    let stored_fact_ids: Vec<String> = row.try_get("input_fact_ids").map_err(storage_error)?;
    let stored_time: i64 = row
        .try_get("evaluated_at_unix_seconds")
        .map_err(storage_error)?;
    let stored_result: String = row.try_get("result").map_err(storage_error)?;
    let stored_result: Value = serde_json::from_str(&stored_result).map_err(serialization_error)?;
    let result: Value = serde_json::from_str(result).map_err(serialization_error)?;
    Ok(stored_fact_ids == input_fact_ids
        && stored_time == evaluated_at_unix_seconds
        && stored_result == result)
}

fn validate_evaluation(
    definition_version: u64,
    evaluated_at_unix_seconds: i64,
    input_fact_ids: &[&str],
    evaluation: &MempoolAlertEvaluation,
) -> Result<(), AlertPersistenceError> {
    checked_positive_i64(definition_version)?;
    checked_positive_i64(evaluation.revision)?;
    if evaluated_at_unix_seconds < 0 {
        return Err(AlertPersistenceError::InvalidEvaluationTime);
    }
    normalized_fact_ids(input_fact_ids)?;
    if !valid_identity(&evaluation.alert_id, 128)
        || !valid_identity(&evaluation.network, 64)
        || evaluation.kind != ALERT_KIND
        || !valid_hash(&evaluation.evaluation_id)
    {
        return Err(AlertPersistenceError::InvalidEvaluation);
    }
    match (
        evaluation.delivery_required,
        evaluation.outbox_idempotency_key.as_deref(),
    ) {
        (true, None) => Err(AlertPersistenceError::MissingOutboxKey),
        (false, Some(_)) => Err(AlertPersistenceError::UnexpectedOutboxKey),
        (_, Some(key)) if !valid_hash(key) => Err(AlertPersistenceError::InvalidOutboxKey),
        _ => Ok(()),
    }
}

fn normalized_fact_ids(input_fact_ids: &[&str]) -> Result<Vec<String>, AlertPersistenceError> {
    if input_fact_ids.is_empty() || input_fact_ids.len() > MAX_FACT_COUNT {
        return Err(AlertPersistenceError::InvalidInputFacts);
    }
    let mut ids = input_fact_ids
        .iter()
        .map(|id| (*id).to_owned())
        .collect::<Vec<_>>();
    if ids.iter().any(|id| !valid_identity(id, MAX_FACT_ID_LEN)) {
        return Err(AlertPersistenceError::InvalidInputFacts);
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

fn checked_positive_i64(value: u64) -> Result<i64, AlertPersistenceError> {
    let value = i64::try_from(value).map_err(|_| AlertPersistenceError::ValueOutOfRange)?;
    if value == 0 {
        return Err(AlertPersistenceError::ValueOutOfRange);
    }
    Ok(value)
}

fn valid_identity(value: &str, max_len: usize) -> bool {
    !value.is_empty() && value.len() <= max_len && value.bytes().all(|byte| byte.is_ascii_graphic())
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

const fn transition_name(transition: AlertTransition) -> &'static str {
    match transition {
        AlertTransition::Pending => "pending",
        AlertTransition::Triggered => "triggered",
        AlertTransition::Confirmed => "confirmed",
        AlertTransition::Corrected => "corrected",
        AlertTransition::Retracted => "retracted",
        AlertTransition::DegradedSource => "degraded_source",
        AlertTransition::BelowThreshold => "below_threshold",
        AlertTransition::CooldownSuppressed => "cooldown_suppressed",
        AlertTransition::DuplicateRevision => "duplicate_revision",
    }
}

fn decode_count(row: &sqlx::postgres::PgRow, column: &str) -> Result<u64, AlertPersistenceError> {
    let value: i64 = row.try_get(column).map_err(storage_error)?;
    u64::try_from(value).map_err(storage_error)
}

fn truncate_error(mut error: String) -> String {
    const MAX_ERROR_BYTES: usize = 1_024;
    if error.len() > MAX_ERROR_BYTES {
        let mut boundary = MAX_ERROR_BYTES;
        while !error.is_char_boundary(boundary) {
            boundary -= 1;
        }
        error.truncate(boundary);
    }
    error
}

fn storage_error(error: impl std::fmt::Display) -> AlertPersistenceError {
    AlertPersistenceError::Storage(error.to_string())
}

fn serialization_error(error: impl std::fmt::Display) -> AlertPersistenceError {
    AlertPersistenceError::Serialization(error.to_string())
}

/// Durable alert validation, conflict, serialization, or database failure.
#[derive(Debug, Error)]
pub enum AlertPersistenceError {
    /// Database pool size must be positive.
    #[error("alert database connection limit must be positive")]
    InvalidConnectionLimit,
    /// Unsigned identity cannot fit the `PostgreSQL` signed integer boundary.
    #[error("alert numeric value is zero or outside PostgreSQL BIGINT range")]
    ValueOutOfRange,
    /// Evaluation time cannot predate the Unix epoch.
    #[error("alert evaluation time must not be negative")]
    InvalidEvaluationTime,
    /// Input fact IDs are missing, invalid, or exceed their bounded contract.
    #[error("alert evaluation requires 1 to 128 bounded ASCII input fact IDs")]
    InvalidInputFacts,
    /// Evaluation identity or fixed contract fields are invalid.
    #[error("alert evaluation contract is invalid")]
    InvalidEvaluation,
    /// Delivery-required evaluation omitted its deterministic key.
    #[error("delivery-required alert evaluation is missing its outbox key")]
    MissingOutboxKey,
    /// Non-delivery evaluation unexpectedly supplied an outbox key.
    #[error("non-delivery alert evaluation must not include an outbox key")]
    UnexpectedOutboxKey,
    /// Outbox identity is not a lowercase 32-byte hex digest.
    #[error("alert outbox key must be a lowercase 32-byte hex digest")]
    InvalidOutboxKey,
    /// Definition version already exists with different content.
    #[error("alert definition version {version} already has different content")]
    DefinitionConflict {
        /// Immutable conflicting version.
        version: u64,
    },
    /// Definition is absent or is not enabled.
    #[error("alert definition version {version} is unavailable")]
    DefinitionUnavailable {
        /// Requested definition version.
        version: u64,
    },
    /// Current state is ahead of the attempted input revision.
    #[error("alert input revision regressed from {current} to {attempted}")]
    RevisionRegression {
        /// Current durable revision.
        current: u64,
        /// Attempted older revision.
        attempted: u64,
    },
    /// A revision was reused for different deterministic evidence.
    #[error("alert input revision {revision} conflicts with durable evidence")]
    ConflictingRevision {
        /// Conflicting revision.
        revision: u64,
    },
    /// The evaluation identity matched but its non-hashed audit inputs differed.
    #[error("alert evaluation replay conflicts with durable audit inputs")]
    EvaluationConflict,
    /// Delivery batch is zero or exceeds the bounded limit.
    #[error("alert delivery batch must be between 1 and 1000")]
    InvalidDeliveryBatch,
    /// JSON conversion failed.
    #[error("alert serialization failed: {0}")]
    Serialization(String),
    /// `PostgreSQL` operation failed.
    #[error("alert storage failed: {0}")]
    Storage(String),
}

/// Delivery adapter failure retained in the outbox audit trail.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("alert delivery failed: {message}")]
pub struct AlertDeliveryError {
    message: String,
}

impl AlertDeliveryError {
    /// Constructs a bounded delivery failure.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_error;

    #[test]
    fn delivery_errors_are_utf8_safe_and_bounded() {
        let truncated = truncate_error("€".repeat(500));

        assert!(truncated.len() <= 1_024);
        assert!(truncated.chars().all(|character| character == '€'));
    }
}
