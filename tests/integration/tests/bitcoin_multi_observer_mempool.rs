use bitcoin_domain::Txid;
use bitcoin_mempool::{
    CoverageQuality, MempoolViewPolicy, ObserverHealth, ObserverMempool, aggregate_membership,
};

#[test]
fn three_observer_views_match_source_qualified_fixture() {
    let txid = Txid::from_bytes([0x42; 32]);
    let mut observer_a = ObserverMempool::new("observer-a").expect("observer a");
    let mut observer_b = ObserverMempool::new("observer-b").expect("observer b");
    let mut observer_c = ObserverMempool::new("observer-c").expect("observer c");
    observer_a.set_health(ObserverHealth::Healthy, 0);
    observer_b.set_health(ObserverHealth::Healthy, 0);
    observer_c.set_health(ObserverHealth::Offline, 0);
    observer_a.observe_add(txid, 200);
    observer_b.observe_add(txid, 100);

    let aggregate = aggregate_membership(
        txid,
        &[&observer_a, &observer_b, &observer_c],
        &MempoolViewPolicy::Intersection,
        1_000_000,
    );
    assert!(aggregate.policy_satisfied);
    assert_eq!(aggregate.quality, CoverageQuality::Degraded);

    let actual = serde_json::json!({
        "txid": txid.to_string(),
        "healthy_source_count": aggregate.healthy_source_count,
        "present_source_count": aggregate.present_source_count,
        "present_sources": aggregate.present_sources,
        "policy": "intersection_healthy",
        "policy_satisfied": aggregate.policy_satisfied,
        "platform_first_seen_at_unix_ns": aggregate.platform_first_seen_at_unix_ns,
        "quality": "degraded"
    });
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/bitcoin/expected/mempool-three-observers.json"
    ))
    .expect("expected fixture");
    assert_eq!(actual, expected);
}
