use alert_engine::{
    AlertTransition, Completeness, DegradedPolicy, MempoolAlertEvaluator, QuorumFeeBandSnapshot,
    QuorumVbytesAboveDefinition, SnapshotCause,
};

fn definition(
    for_evaluations: u16,
    cooldown_seconds: u64,
    degraded_policy: DegradedPolicy,
) -> QuorumVbytesAboveDefinition {
    QuorumVbytesAboveDefinition::new(
        "btc-mainnet-high-fee-pressure",
        25,
        1_000_000,
        2,
        for_evaluations,
        cooldown_seconds,
        degraded_policy,
    )
    .expect("valid definition")
}

fn snapshot(
    revision: u64,
    observed_at_unix_seconds: i64,
    vbytes: u64,
    eligible_sources: &[&str],
    unavailable_sources: &[&str],
    completeness: Completeness,
    cause: SnapshotCause,
) -> QuorumFeeBandSnapshot {
    QuorumFeeBandSnapshot::new(
        "mainnet",
        revision,
        observed_at_unix_seconds,
        25,
        vbytes,
        2,
        eligible_sources.iter().copied(),
        unavailable_sources.iter().copied(),
        completeness,
        cause,
    )
    .expect("valid snapshot")
}

#[test]
fn threshold_must_persist_before_one_deterministic_delivery() {
    let mut first = MempoolAlertEvaluator::new(definition(2, 60, DegradedPolicy::Suppress));
    let mut replay = MempoolAlertEvaluator::new(definition(2, 60, DegradedPolicy::Suppress));
    let inputs = [
        snapshot(
            10,
            1_000,
            1_100_000,
            &["observer-b", "observer-a", "observer-a"],
            &[],
            Completeness::Complete,
            SnapshotCause::Observed,
        ),
        snapshot(
            11,
            1_010,
            1_200_000,
            &["observer-a", "observer-b"],
            &[],
            Completeness::Complete,
            SnapshotCause::Observed,
        ),
        snapshot(
            12,
            1_020,
            1_300_000,
            &["observer-a", "observer-b"],
            &[],
            Completeness::Complete,
            SnapshotCause::Observed,
        ),
    ];

    let first_results = inputs
        .iter()
        .map(|input| first.evaluate(input).expect("evaluation"))
        .collect::<Vec<_>>();
    let replay_results = inputs
        .iter()
        .map(|input| replay.evaluate(input).expect("evaluation"))
        .collect::<Vec<_>>();

    assert_eq!(first_results, replay_results);
    assert_eq!(first_results[0].transition, AlertTransition::Pending);
    assert!(!first_results[0].delivery_required);
    assert_eq!(first_results[1].transition, AlertTransition::Triggered);
    assert!(first_results[1].delivery_required);
    assert_eq!(first_results[2].transition, AlertTransition::Confirmed);
    assert!(!first_results[2].delivery_required);
    assert_eq!(
        first_results[1].contributing_sources,
        ["observer-a", "observer-b"]
    );
    assert_eq!(first_results[1].evaluation_id.len(), 64);
    assert_eq!(
        first_results[1]
            .outbox_idempotency_key
            .as_deref()
            .map(str::len),
        Some(64)
    );
}

#[test]
fn same_revision_replay_never_requests_duplicate_delivery() {
    let mut evaluator =
        MempoolAlertEvaluator::new(definition(1, 60, DegradedPolicy::EvaluateHealthyQuorum));
    let input = snapshot(
        20,
        2_000,
        1_100_000,
        &["observer-a", "observer-b"],
        &["observer-c"],
        Completeness::KnownIncomplete,
        SnapshotCause::Recovered,
    );

    let triggered = evaluator.evaluate(&input).expect("trigger");
    let duplicate = evaluator.evaluate(&input).expect("idempotent replay");

    assert_eq!(triggered.transition, AlertTransition::Triggered);
    assert!(triggered.delivery_required);
    assert_eq!(duplicate.transition, AlertTransition::DuplicateRevision);
    assert!(!duplicate.delivery_required);
    assert_eq!(duplicate.evaluation_id, triggered.evaluation_id);
    assert!(duplicate.outbox_idempotency_key.is_none());
}

#[test]
fn insufficient_or_suppressed_source_coverage_is_explicitly_degraded() {
    let incomplete = snapshot(
        30,
        3_000,
        2_000_000,
        &["observer-a"],
        &["observer-b", "observer-c"],
        Completeness::KnownIncomplete,
        SnapshotCause::Observed,
    );
    let mut insufficient =
        MempoolAlertEvaluator::new(definition(1, 0, DegradedPolicy::EvaluateHealthyQuorum));
    let decision = insufficient.evaluate(&incomplete).expect("degraded");
    assert_eq!(decision.transition, AlertTransition::DegradedSource);
    assert!(!decision.delivery_required);
    assert!(!decision.active);

    let degraded_but_quorate = snapshot(
        31,
        3_010,
        2_000_000,
        &["observer-a", "observer-b"],
        &["observer-c"],
        Completeness::KnownIncomplete,
        SnapshotCause::Observed,
    );
    let mut suppress = MempoolAlertEvaluator::new(definition(1, 0, DegradedPolicy::Suppress));
    let decision = suppress
        .evaluate(&degraded_but_quorate)
        .expect("suppressed degradation");
    assert_eq!(decision.transition, AlertTransition::DegradedSource);
    assert!(!decision.delivery_required);
    assert_eq!(decision.unavailable_sources, ["observer-c"]);
}

#[test]
fn correction_below_threshold_retracts_active_alert() {
    let mut evaluator = MempoolAlertEvaluator::new(definition(1, 0, DegradedPolicy::Suppress));
    let trigger = evaluator
        .evaluate(&snapshot(
            40,
            4_000,
            1_500_000,
            &["observer-a", "observer-b", "observer-c"],
            &[],
            Completeness::Complete,
            SnapshotCause::Observed,
        ))
        .expect("trigger");
    let correction = evaluator
        .evaluate(&snapshot(
            41,
            4_001,
            900_000,
            &["observer-a", "observer-b", "observer-c"],
            &[],
            Completeness::Recovered,
            SnapshotCause::Correction,
        ))
        .expect("correction");

    assert_eq!(trigger.transition, AlertTransition::Triggered);
    assert_eq!(correction.transition, AlertTransition::Corrected);
    assert!(correction.delivery_required);
    assert!(!correction.active);
    assert_ne!(trigger.evaluation_id, correction.evaluation_id);
}

#[test]
fn cooldown_suppresses_retrigger_delivery_until_window_expires() {
    let mut evaluator = MempoolAlertEvaluator::new(definition(1, 60, DegradedPolicy::Suppress));
    let trigger = evaluator
        .evaluate(&snapshot(
            50,
            5_000,
            1_500_000,
            &["observer-a", "observer-b"],
            &[],
            Completeness::Complete,
            SnapshotCause::Observed,
        ))
        .expect("first trigger");
    let retract = evaluator
        .evaluate(&snapshot(
            51,
            5_010,
            500_000,
            &["observer-a", "observer-b"],
            &[],
            Completeness::Complete,
            SnapshotCause::Observed,
        ))
        .expect("retract");
    let cooldown = evaluator
        .evaluate(&snapshot(
            52,
            5_020,
            1_500_000,
            &["observer-a", "observer-b"],
            &[],
            Completeness::Complete,
            SnapshotCause::Observed,
        ))
        .expect("cooldown");
    let retrigger = evaluator
        .evaluate(&snapshot(
            53,
            5_061,
            1_500_000,
            &["observer-a", "observer-b"],
            &[],
            Completeness::Complete,
            SnapshotCause::Observed,
        ))
        .expect("retrigger");

    assert_eq!(trigger.transition, AlertTransition::Triggered);
    assert_eq!(retract.transition, AlertTransition::Retracted);
    assert_eq!(cooldown.transition, AlertTransition::CooldownSuppressed);
    assert!(!cooldown.delivery_required);
    assert_eq!(retrigger.transition, AlertTransition::Triggered);
    assert!(retrigger.delivery_required);
}

#[test]
fn conflicting_duplicate_revision_and_out_of_order_time_fail_closed() {
    let mut evaluator = MempoolAlertEvaluator::new(definition(1, 0, DegradedPolicy::Suppress));
    evaluator
        .evaluate(&snapshot(
            60,
            6_000,
            1_500_000,
            &["observer-a", "observer-b"],
            &[],
            Completeness::Complete,
            SnapshotCause::Observed,
        ))
        .expect("first");

    assert!(
        evaluator
            .evaluate(&snapshot(
                60,
                6_000,
                500_000,
                &["observer-a", "observer-b"],
                &[],
                Completeness::Complete,
                SnapshotCause::Observed,
            ))
            .is_err()
    );
    assert!(
        evaluator
            .evaluate(&snapshot(
                61,
                5_999,
                1_500_000,
                &["observer-a", "observer-b"],
                &[],
                Completeness::Complete,
                SnapshotCause::Observed,
            ))
            .is_err()
    );
}

#[test]
fn definition_changes_and_contradictory_source_evidence_cannot_alias() {
    let input = snapshot(
        70,
        7_000,
        1_500_000,
        &["observer-a", "observer-b"],
        &[],
        Completeness::Complete,
        SnapshotCause::Observed,
    );
    let mut immediate = MempoolAlertEvaluator::new(definition(1, 0, DegradedPolicy::Suppress));
    let mut persistent = MempoolAlertEvaluator::new(definition(2, 0, DegradedPolicy::Suppress));
    let immediate = immediate.evaluate(&input).expect("immediate");
    let persistent = persistent.evaluate(&input).expect("persistent");

    assert_eq!(immediate.transition, AlertTransition::Triggered);
    assert_eq!(persistent.transition, AlertTransition::Pending);
    assert_ne!(immediate.evaluation_id, persistent.evaluation_id);

    assert!(
        QuorumFeeBandSnapshot::new(
            "mainnet",
            71,
            7_001,
            25,
            1_500_000,
            2,
            ["observer-a", "observer-b"],
            ["observer-b"],
            Completeness::KnownIncomplete,
            SnapshotCause::Observed,
        )
        .is_err()
    );
    assert!(
        QuorumFeeBandSnapshot::new(
            "mainnet",
            72,
            7_002,
            25,
            1_500_000,
            2,
            ["observer a", "observer-b"],
            std::iter::empty::<&str>(),
            Completeness::Complete,
            SnapshotCause::Observed,
        )
        .is_err()
    );
    assert!(
        QuorumFeeBandSnapshot::new(
            "mainnet",
            73,
            -1,
            25,
            1_500_000,
            2,
            ["observer-a", "observer-b"],
            std::iter::empty::<&str>(),
            Completeness::Complete,
            SnapshotCause::Observed,
        )
        .is_err()
    );
}
