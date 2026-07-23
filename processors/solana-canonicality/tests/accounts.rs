use solana_canonicality::SolanaCanonicality;
use solana_domain::{AccountWrite, Blockhash, ForkId, Lamports, Pubkey, Slot};

fn fork(slot: u64, marker: u8) -> ForkId {
    ForkId::new(Slot::new(slot), Blockhash::new_from_array([marker; 32]))
}

fn write(fork_id: ForkId, amount: u64, marker: u8, version: u64) -> AccountWrite {
    AccountWrite::try_new(
        fork_id,
        Pubkey::new_from_array([7; 32]),
        Pubkey::new_from_array([8; 32]),
        Lamports::new(amount),
        vec![marker],
        false,
        0,
        version,
    )
    .expect("write")
}

#[test]
fn switching_forks_reverts_then_applies_account_writes_exactly() {
    let root = fork(100, 1);
    let first = fork(101, 2);
    let competing = fork(101, 3);
    let account = Pubkey::new_from_array([7; 32]);
    let mut state = SolanaCanonicality::new();
    state.observe_slot(root.clone(), None).expect("root");
    state
        .observe_slot(first.clone(), Some(root.clone()))
        .expect("first");
    state
        .observe_slot(competing.clone(), Some(root.clone()))
        .expect("competing");
    state.activate(&root).expect("activate root");
    let root_hash = state.state_hash();

    state
        .record_account_write(write(first.clone(), 10, 0xaa, 1))
        .expect("first write");
    state.activate(&first).expect("activate first");
    let first_hash = state.state_hash();
    assert_eq!(
        state.account(&account).expect("first state").lamports(),
        Lamports::new(10)
    );

    state
        .record_account_write(write(competing.clone(), 20, 0xbb, 1))
        .expect("competing write");
    state.activate(&competing).expect("activate competing");
    assert_eq!(
        state.account(&account).expect("competing state").lamports(),
        Lamports::new(20)
    );
    assert_ne!(state.state_hash(), first_hash);

    state.mark_dead(&competing).expect("dead competing fork");
    assert_eq!(state.active_tip(), Some(&root));
    assert!(state.account(&account).is_none());
    assert_eq!(state.state_hash(), root_hash);

    state.activate(&first).expect("reactivate surviving fork");
    assert_eq!(state.state_hash(), first_hash);
}

#[test]
fn write_versions_are_strictly_ordered_and_invalid_writes_are_atomic() {
    let root = fork(200, 1);
    let account = Pubkey::new_from_array([7; 32]);
    let mut state = SolanaCanonicality::new();
    state.observe_slot(root.clone(), None).expect("root");
    state.activate(&root).expect("active");
    state
        .record_account_write(write(root.clone(), 10, 1, 5))
        .expect("first");
    let before = state.state_hash();

    assert!(
        state
            .record_account_write(write(root.clone(), 20, 2, 5))
            .is_err()
    );
    assert!(state.record_account_write(write(root, 20, 2, 4)).is_err());
    assert_eq!(state.state_hash(), before);
    assert_eq!(
        state.account(&account).expect("unchanged").lamports(),
        Lamports::new(10)
    );
}
