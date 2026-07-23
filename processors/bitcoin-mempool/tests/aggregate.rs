use bitcoin_domain::Txid;
use bitcoin_mempool::{
    CoverageQuality, MempoolViewPolicy, ObserverHealth, ObserverMempool, aggregate_membership,
};

#[test]
fn aggregate_policies_use_only_healthy_observers() {
    let txid = Txid::from_bytes([7; 32]);
    let mut a = observer("a", ObserverHealth::Healthy, 0);
    let mut b = observer("b", ObserverHealth::Healthy, 0);
    let mut c = observer("c", ObserverHealth::Healthy, 0);
    a.observe_add(txid, 300);
    b.observe_add(txid, 200);

    let observers = [&a, &b, &c];
    assert!(
        aggregate_membership(txid, &observers, &MempoolViewPolicy::Union, 1_000).policy_satisfied
    );
    assert!(
        !aggregate_membership(txid, &observers, &MempoolViewPolicy::Intersection, 1_000)
            .policy_satisfied
    );
    assert!(
        aggregate_membership(
            txid,
            &observers,
            &MempoolViewPolicy::Quorum { required: 2 },
            1_000
        )
        .policy_satisfied
    );

    c.set_health(ObserverHealth::Offline, 0);
    let offline =
        aggregate_membership(txid, &[&a, &b, &c], &MempoolViewPolicy::Intersection, 1_000);
    assert!(offline.policy_satisfied);
    assert_eq!(offline.healthy_source_count, 2);
    assert_eq!(offline.quality, CoverageQuality::Degraded);
}

#[test]
fn gaps_clock_skew_and_zero_healthy_sources_degrade_without_false_success() {
    let txid = Txid::from_bytes([8; 32]);
    let mut a = observer("a", ObserverHealth::Healthy, 5_000);
    let mut b = observer("b", ObserverHealth::Gapped, 0);
    let mut c = observer("c", ObserverHealth::Healthy, 0);
    a.observe_add(txid, 100);
    b.observe_add(txid, 50);

    let aggregate = aggregate_membership(
        txid,
        &[&a, &b, &c],
        &MempoolViewPolicy::Quorum { required: 2 },
        1_000,
    );
    assert!(!aggregate.policy_satisfied);
    assert_eq!(aggregate.healthy_source_count, 2);
    assert_eq!(aggregate.present_source_count, 1);
    assert_eq!(aggregate.platform_first_seen_at_unix_ns, Some(50));
    assert!(aggregate.clock_untrusted);
    assert_eq!(aggregate.quality, CoverageQuality::Degraded);

    a.set_health(ObserverHealth::Offline, 0);
    c.set_health(ObserverHealth::Offline, 0);
    let unavailable = aggregate_membership(txid, &[&a, &b, &c], &MempoolViewPolicy::Union, 1_000);
    assert!(!unavailable.policy_satisfied);
    assert_eq!(unavailable.healthy_source_count, 0);
    assert_eq!(unavailable.quality, CoverageQuality::Unavailable);
}

fn observer(source_id: &str, health: ObserverHealth, clock_offset_ns: i64) -> ObserverMempool {
    let mut observer = ObserverMempool::new(source_id).expect("observer");
    observer.set_health(health, clock_offset_ns);
    observer
}
