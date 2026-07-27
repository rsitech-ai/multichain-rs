use std::time::{Duration, SystemTime, UNIX_EPOCH};

use alert_engine::{
    AlertAuditSummary, AlertPersistenceError, AlertTransition, Completeness, DegradedPolicy,
    MemoryAlertDeliverySink, MempoolAlertEvaluator, PersistOutcome, PostgresAlertStore,
    QuorumFeeBandSnapshot, QuorumVbytesAboveDefinition, SnapshotCause,
};
use tokio::time::timeout;
const DATABASE_URL: &str =
    "postgres://multichain:local-development-only@127.0.0.1:15432/multichain";

#[tokio::test]
async fn evaluation_state_and_outbox_are_atomic_idempotent_and_auditable() {
    let alert_id = unique_alert_id();
    let definition = make_definition(&alert_id, 1_000_000);
    let database_url =
        std::env::var("MULTICHAIN_TEST_DATABASE_URL").unwrap_or_else(|_| DATABASE_URL.to_owned());
    let connection = timeout(
        Duration::from_secs(2),
        PostgresAlertStore::connect(&database_url, 4),
    )
    .await;
    let store = match connection {
        Ok(Ok(store)) => store,
        Ok(Err(error)) if std::env::var_os("MULTICHAIN_REQUIRE_INFRA").is_none() => {
            eprintln!("skipping live alert persistence proof: {error}");
            return;
        }
        Ok(Err(error)) => {
            panic!("required PostgreSQL alert infrastructure is unavailable: {error}")
        }
        Err(_) if std::env::var_os("MULTICHAIN_REQUIRE_INFRA").is_none() => {
            eprintln!("skipping live alert persistence proof: connection timed out");
            return;
        }
        Err(error) => {
            panic!("required PostgreSQL alert infrastructure connection timed out: {error}")
        }
    };
    store.install_schema().await.expect("install alert schema");
    prove_definition_immutability(&store, &alert_id, &definition).await;

    let mut evaluator = MempoolAlertEvaluator::new(definition);
    let after_first = persist_and_replay_first(&store, &alert_id, &mut evaluator).await;
    reject_conflicting_revision(&store, &alert_id, after_first).await;
    persist_correction_and_reject_invalid(&store, &alert_id, &mut evaluator).await;
    prove_idempotent_delivery(&store, &alert_id).await;
}

async fn prove_definition_immutability(
    store: &PostgresAlertStore,
    alert_id: &str,
    definition: &QuorumVbytesAboveDefinition,
) {
    assert_eq!(
        store
            .register_mempool_definition(1, definition)
            .await
            .expect("register definition"),
        PersistOutcome::Inserted
    );
    assert_eq!(
        store
            .register_mempool_definition(1, definition)
            .await
            .expect("idempotent definition replay"),
        PersistOutcome::Duplicate
    );
    assert!(matches!(
        store
            .register_mempool_definition(1, &make_definition(alert_id, 2_000_000))
            .await,
        Err(AlertPersistenceError::DefinitionConflict { version: 1 })
    ));
    assert_eq!(
        store
            .register_mempool_definition(2, definition)
            .await
            .expect("register next immutable version"),
        PersistOutcome::Inserted
    );
    assert_eq!(
        store
            .audit_summary(alert_id, 2)
            .await
            .expect("new version audit is isolated"),
        AlertAuditSummary::default()
    );
}

async fn persist_and_replay_first(
    store: &PostgresAlertStore,
    alert_id: &str,
    evaluator: &mut MempoolAlertEvaluator,
) -> AlertAuditSummary {
    let first_snapshot = snapshot(10, 1_000, 1_500_000, SnapshotCause::Observed);
    let first = evaluator
        .evaluate(&first_snapshot)
        .expect("first evaluation");
    assert_eq!(first.transition, AlertTransition::Triggered);
    assert_eq!(
        store
            .persist_mempool_evaluation(
                1,
                first_snapshot.observed_at_unix_seconds(),
                &["mempool-fact-revision-10"],
                &first,
            )
            .await
            .expect("persist first evaluation"),
        PersistOutcome::Inserted
    );
    assert_eq!(
        store
            .persist_mempool_evaluation(
                1,
                first_snapshot.observed_at_unix_seconds(),
                &["mempool-fact-revision-10"],
                &first,
            )
            .await
            .expect("replay first evaluation"),
        PersistOutcome::Duplicate
    );

    let after_first = store
        .audit_summary(alert_id, 1)
        .await
        .expect("first audit summary");
    assert_eq!(after_first.evaluation_count, 1);
    assert_eq!(after_first.outbox_count, 1);
    assert_eq!(after_first.pending_outbox_count, 1);
    assert_eq!(after_first.last_revision, Some(10));
    after_first
}

async fn reject_conflicting_revision(
    store: &PostgresAlertStore,
    alert_id: &str,
    expected_summary: AlertAuditSummary,
) {
    let mut conflicting_evaluator =
        MempoolAlertEvaluator::new(make_definition(alert_id, 1_000_000));
    let conflicting = conflicting_evaluator
        .evaluate(&snapshot(10, 1_000, 1_600_000, SnapshotCause::Correction))
        .expect("conflicting evaluation");
    assert!(matches!(
        store
            .persist_mempool_evaluation(
                1,
                1_000,
                &["different-fact-for-revision-10"],
                &conflicting,
            )
            .await,
        Err(AlertPersistenceError::ConflictingRevision { revision: 10 })
    ));
    assert_eq!(
        store
            .audit_summary(alert_id, 1)
            .await
            .expect("unchanged summary after conflict"),
        expected_summary
    );
}

async fn persist_correction_and_reject_invalid(
    store: &PostgresAlertStore,
    alert_id: &str,
    evaluator: &mut MempoolAlertEvaluator,
) {
    let correction_snapshot = snapshot(11, 1_001, 900_000, SnapshotCause::Correction);
    let correction = evaluator
        .evaluate(&correction_snapshot)
        .expect("correction evaluation");
    assert_eq!(correction.transition, AlertTransition::Corrected);
    assert_eq!(
        store
            .persist_mempool_evaluation(
                1,
                correction_snapshot.observed_at_unix_seconds(),
                &["mempool-fact-revision-11"],
                &correction,
            )
            .await
            .expect("persist correction"),
        PersistOutcome::Inserted
    );

    let historical_snapshot = snapshot(10, 1_000, 1_500_000, SnapshotCause::Observed);
    let historical = MempoolAlertEvaluator::new(make_definition(alert_id, 1_000_000))
        .evaluate(&historical_snapshot)
        .expect("historical evaluation");
    assert_eq!(
        store
            .persist_mempool_evaluation(
                1,
                historical_snapshot.observed_at_unix_seconds(),
                &["mempool-fact-revision-10"],
                &historical,
            )
            .await
            .expect("exact historical replay"),
        PersistOutcome::Duplicate
    );

    let invalid_snapshot = snapshot(12, 1_061, 1_500_000, SnapshotCause::Observed);
    let mut invalid = evaluator
        .evaluate(&invalid_snapshot)
        .expect("next trigger evaluation");
    invalid.outbox_idempotency_key = None;
    assert!(matches!(
        store
            .persist_mempool_evaluation(
                1,
                invalid_snapshot.observed_at_unix_seconds(),
                &["mempool-fact-revision-12"],
                &invalid,
            )
            .await,
        Err(AlertPersistenceError::MissingOutboxKey)
    ));

    let before_delivery = store
        .audit_summary(alert_id, 1)
        .await
        .expect("pre-delivery audit");
    assert_eq!(before_delivery.evaluation_count, 2);
    assert_eq!(before_delivery.outbox_count, 2);
    assert_eq!(before_delivery.pending_outbox_count, 2);
    assert_eq!(before_delivery.last_revision, Some(11));
}

async fn prove_idempotent_delivery(store: &PostgresAlertStore, alert_id: &str) {
    let sink = MemoryAlertDeliverySink::default();
    let first_delivery = store
        .deliver_pending(&sink, 100)
        .await
        .expect("deliver pending");
    let replay_delivery = store
        .deliver_pending(&sink, 100)
        .await
        .expect("replay delivery");
    assert!(first_delivery.delivered >= 2);
    assert_eq!(first_delivery.failed, 0);
    assert_eq!(replay_delivery.delivered, 0);
    assert_eq!(
        sink.deliveries()
            .await
            .iter()
            .filter(|delivery| delivery.alert_id == alert_id)
            .count(),
        2
    );

    let final_summary = store.audit_summary(alert_id, 1).await.expect("final audit");
    assert_eq!(final_summary.pending_outbox_count, 0);
    assert_eq!(final_summary.delivered_outbox_count, 2);
}

fn make_definition(alert_id: &str, threshold_vbytes: u64) -> QuorumVbytesAboveDefinition {
    QuorumVbytesAboveDefinition::new(
        alert_id,
        25,
        threshold_vbytes,
        2,
        1,
        60,
        DegradedPolicy::Suppress,
    )
    .expect("valid definition")
}

fn snapshot(
    revision: u64,
    observed_at_unix_seconds: i64,
    vbytes: u64,
    cause: SnapshotCause,
) -> QuorumFeeBandSnapshot {
    QuorumFeeBandSnapshot::new(
        "mainnet",
        revision,
        observed_at_unix_seconds,
        25,
        vbytes,
        2,
        ["observer-a", "observer-b", "observer-c"],
        std::iter::empty::<&str>(),
        Completeness::Complete,
        cause,
    )
    .expect("valid snapshot")
}

fn unique_alert_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    format!("btc-mainnet-postgres-{nanos}-{}", std::process::id())
}
