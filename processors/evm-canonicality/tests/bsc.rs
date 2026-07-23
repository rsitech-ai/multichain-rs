use evm_canonicality::{
    BscCanonicality, BscError, BscFinalityObservation, BscFinalityStatus, ExecutionBlock,
};
use evm_domain::{B256, BscStatus};

#[test]
fn bsc_finalized_tag_is_native_monotonic_ancestry() {
    let genesis = block(0, 1, 0);
    let a = block(1, 2, 1);
    let b = block(2, 3, 2);
    let mut state = BscCanonicality::new();
    state
        .commit_segment([genesis, a, b])
        .expect("canonical segment");

    let finalized = state
        .observe_finalized(
            BscFinalityObservation::new(a.hash, b.hash, "bsc-eu-1", 1_000).expect("observation"),
        )
        .expect("finality");
    assert_eq!(finalized.status, BscStatus::FastFinalized);
    assert_eq!(finalized.block.hash, a.hash);

    assert!(
        state
            .observe_finalized(
                BscFinalityObservation::new(b.hash, b.hash, "bsc-eu-1", 1_450)
                    .expect("observation"),
            )
            .is_ok()
    );
    assert_eq!(
        state.finality_health(2_000, 1_000),
        BscFinalityStatus::Healthy
    );
    assert_eq!(
        state.finality_health(3_000, 1_000),
        BscFinalityStatus::Stalled
    );
}

#[test]
fn regression_nonancestor_and_finalized_reorg_fail_closed() {
    let genesis = block(0, 1, 0);
    let a = block(1, 2, 1);
    let b = block(2, 3, 2);
    let mut state = BscCanonicality::new();
    state
        .commit_segment([genesis, a, b])
        .expect("canonical segment");
    state
        .observe_finalized(
            BscFinalityObservation::new(a.hash, b.hash, "bsc-eu-1", 1_000).expect("observation"),
        )
        .expect("finality");

    assert!(matches!(
        state.observe_finalized(
            BscFinalityObservation::new(genesis.hash, b.hash, "bsc-eu-1", 1_500)
                .expect("observation"),
        ),
        Err(BscError::FinalizedRegression { .. })
    ));
    assert!(matches!(
        state.observe_finalized(
            BscFinalityObservation::new(hash(9), b.hash, "bsc-eu-1", 1_500).expect("observation"),
        ),
        Err(BscError::UnknownBlock { .. })
    ));
    assert!(matches!(
        state.reorg([a, b], [block(1, 8, 1), block(2, 9, 8)]),
        Err(BscError::FinalizedReversalCritical { .. })
    ));
    assert_eq!(state.head(), Some(b.hash));
}

fn block(number: u64, hash_byte: u8, parent_byte: u8) -> ExecutionBlock {
    ExecutionBlock::new(number, hash(hash_byte), hash(parent_byte))
}

fn hash(byte: u8) -> B256 {
    B256::from([byte; 32])
}
