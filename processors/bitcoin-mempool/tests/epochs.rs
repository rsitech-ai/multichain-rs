use bitcoin_domain::Txid;
use bitcoin_mempool::{MembershipCause, ObserverMempool, RemovalCause};

#[test]
fn accept_remove_reaccept_and_snapshot_replay_have_stable_epochs() {
    let tx_a = Txid::from_bytes([0x0a; 32]);
    let tx_b = Txid::from_bytes([0x0b; 32]);
    let mut first = ObserverMempool::new("observer-a").expect("observer");

    let accepted = first.observe_add(tx_a, 100).expect("accepted");
    assert_eq!(accepted.epoch_revision(), 1);
    assert_eq!(accepted.cause(), MembershipCause::Observed);
    assert!(first.observe_add(tx_a, 101).is_none());

    let removed = first
        .observe_remove(tx_a, 110, RemovalCause::Mined)
        .expect("removed");
    assert_eq!(removed.epoch_id(), accepted.epoch_id());
    assert_eq!(removed.epoch_revision(), 2);
    assert_eq!(removed.cause(), MembershipCause::Mined);

    let reaccepted = first.observe_add(tx_a, 120).expect("reaccepted");
    assert_ne!(reaccepted.epoch_id(), accepted.epoch_id());
    assert_eq!(reaccepted.epoch_revision(), 1);

    let snapshot = first
        .apply_snapshot(50, &[tx_b], 130)
        .expect("snapshot diff");
    assert_eq!(snapshot.len(), 2);
    assert!(snapshot.iter().all(|revision| {
        revision.cause() == MembershipCause::ReconciledSnapshot
            && revision.source_observed_at_unix_ns().is_none()
            && revision.recorded_at_unix_ns() == 130
    }));
    assert!(!first.contains(&tx_a));
    assert!(first.contains(&tx_b));
    assert!(
        first
            .apply_snapshot(50, &[tx_b], 140)
            .expect("duplicate snapshot")
            .is_empty()
    );

    let mut replay = ObserverMempool::new("observer-a").expect("observer");
    replay.observe_add(tx_a, 100);
    replay.observe_remove(tx_a, 110, RemovalCause::Mined);
    replay.observe_add(tx_a, 120);
    replay.apply_snapshot(50, &[tx_b], 130).expect("snapshot");
    assert_eq!(first.revisions(), replay.revisions());
}

#[test]
fn snapshot_sequence_regression_and_blank_source_fail_closed() {
    assert!(ObserverMempool::new(" ").is_err());
    let mut observer = ObserverMempool::new("observer-a").expect("observer");
    observer
        .apply_snapshot(10, &[], 100)
        .expect("first snapshot");
    assert!(observer.apply_snapshot(9, &[], 101).is_err());
}
