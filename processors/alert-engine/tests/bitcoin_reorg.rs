use alert_engine::{AlertDecision, BitcoinReorgEvent, BitcoinReorgRule, Completeness, RuleError};

#[test]
fn reorg_alert_id_is_deterministic_and_revision_specific() {
    let rule = BitcoinReorgRule::new("btc-mainnet-reorg", 2).expect("rule");
    let event = BitcoinReorgEvent::new(
        "mainnet",
        "0000aa",
        840_000,
        3,
        77,
        ["observer-a", "observer-b"],
        Completeness::Complete,
    )
    .expect("event");
    let first = rule.evaluate(&event);
    let second = rule.evaluate(&event);
    assert_eq!(first, second);
    let AlertDecision::Fire(alert) = first else {
        panic!("expected alert");
    };
    assert_eq!(alert.alert_id.len(), 64);
    assert_eq!(alert.revision, 77);
    assert_eq!(alert.reorg_depth, 3);

    let json = serde_json::to_value(alert).expect("json");
    assert_eq!(json["kind"], "bitcoin_reorg");
    assert_eq!(json["source_ids"][0], "observer-a");
}

#[test]
fn threshold_and_incomplete_evidence_suppress_delivery_explicitly() {
    let rule = BitcoinReorgRule::new("btc-mainnet-reorg", 3).expect("rule");
    let shallow = BitcoinReorgEvent::new(
        "mainnet",
        "0000aa",
        1,
        2,
        1,
        ["observer-a"],
        Completeness::Complete,
    )
    .expect("event");
    assert_eq!(rule.evaluate(&shallow), AlertDecision::BelowThreshold);

    let incomplete = BitcoinReorgEvent::new(
        "mainnet",
        "0000aa",
        1,
        4,
        2,
        ["observer-a"],
        Completeness::KnownIncomplete,
    )
    .expect("event");
    assert_eq!(
        rule.evaluate(&incomplete),
        AlertDecision::SuppressedIncomplete
    );
}

#[test]
fn alert_boundaries_fail_closed() {
    assert!(matches!(
        BitcoinReorgRule::new("", 1),
        Err(RuleError::EmptyRuleId)
    ));
    assert!(matches!(
        BitcoinReorgEvent::new(
            "mainnet",
            "hash",
            1,
            1,
            0,
            std::iter::empty::<&str>(),
            Completeness::Complete,
        ),
        Err(RuleError::ZeroRevision)
    ));
}
