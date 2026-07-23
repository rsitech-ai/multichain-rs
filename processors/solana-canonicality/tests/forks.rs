use solana_canonicality::{Commitment, SolanaCanonicality};
use solana_domain::{Blockhash, ForkId, Signature, Slot, TransactionKey};

fn fork(slot: u64, marker: u8) -> ForkId {
    ForkId::new(Slot::new(slot), Blockhash::new_from_array([marker; 32]))
}

#[test]
fn competing_slots_and_same_signature_remain_fork_qualified() {
    let root = fork(10, 1);
    let first = fork(11, 2);
    let competing = fork(11, 3);
    let mut state = SolanaCanonicality::new();
    state.observe_slot(root.clone(), None).expect("root");
    state
        .observe_slot(first.clone(), Some(root.clone()))
        .expect("first fork");
    state
        .observe_slot(competing.clone(), Some(root))
        .expect("competing fork");

    let signature = Signature::from([9; 64]);
    let first_key = TransactionKey::new(signature, first);
    let competing_key = TransactionKey::new(signature, competing);
    state
        .record_transaction(first_key.clone())
        .expect("first transaction");
    state
        .record_transaction(competing_key.clone())
        .expect("competing transaction");

    assert_ne!(first_key, competing_key);
    assert!(state.contains_transaction(&first_key));
    assert!(state.contains_transaction(&competing_key));
}

#[test]
fn commitment_upgrades_are_monotonic_idempotent_and_revisioned() {
    let root = fork(20, 1);
    let child = fork(21, 2);
    let mut state = SolanaCanonicality::new();
    state.observe_slot(root.clone(), None).expect("root");
    state
        .observe_slot(child.clone(), Some(root))
        .expect("child");

    let processed = state
        .observe_commitment(&child, Commitment::Processed)
        .expect("processed")
        .expect("revision");
    assert_eq!(processed.revision(), 1);
    assert_eq!(processed.from(), Commitment::Received);
    assert_eq!(processed.to(), Commitment::Processed);
    assert!(
        state
            .observe_commitment(&child, Commitment::Processed)
            .expect("duplicate")
            .is_none()
    );
    assert!(
        state
            .observe_commitment(&child, Commitment::Received)
            .is_err()
    );
    state
        .observe_commitment(&child, Commitment::Confirmed)
        .expect("confirmed");
    state
        .observe_commitment(&child, Commitment::Finalized)
        .expect("finalized");
    assert!(state.observe_commitment(&child, Commitment::Dead).is_err());
}

#[test]
fn unknown_parent_and_dead_parent_are_rejected_atomically() {
    let mut state = SolanaCanonicality::new();
    let root = fork(30, 1);
    let unknown = fork(29, 9);
    assert!(state.observe_slot(root.clone(), Some(unknown)).is_err());
    state.observe_slot(root.clone(), None).expect("root");
    state
        .observe_commitment(&root, Commitment::Dead)
        .expect("dead");
    assert!(state.observe_slot(fork(31, 2), Some(root)).is_err());
}

#[test]
fn finalized_descendant_prevents_ancestor_death_without_partial_mutation() {
    let root = fork(40, 1);
    let child = fork(41, 2);
    let mut state = SolanaCanonicality::new();
    state.observe_slot(root.clone(), None).expect("root");
    state
        .observe_slot(child.clone(), Some(root.clone()))
        .expect("child");
    state
        .observe_commitment(&child, Commitment::Finalized)
        .expect("finalized child");

    assert!(state.mark_dead(&root).is_err());
    assert_eq!(state.commitment(&root), Some(Commitment::Received));
    assert_eq!(state.commitment(&child), Some(Commitment::Finalized));
}
