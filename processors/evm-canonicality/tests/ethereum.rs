use evm_canonicality::{
    EthereumCanonicality, EthereumCheckpoint, EthereumError, EthereumRevision, ExecutionBlock,
};
use evm_domain::{B256, EthereumStatus};

#[test]
fn committed_checkpoint_and_reorg_revisions_are_ordered() {
    let genesis = block(0, 1, 0);
    let a = block(1, 2, 1);
    let b = block(2, 3, 2);
    let c = block(2, 4, 2);
    let mut state = EthereumCanonicality::new();

    let committed = state
        .commit_segment([genesis, a, b])
        .expect("committed segment");
    assert_eq!(committed.len(), 3);
    assert!(
        committed
            .iter()
            .all(|revision| revision.status == EthereumStatus::CanonicalHead)
    );

    let checkpoint =
        EthereumCheckpoint::new(b.hash, a.hash, genesis.hash, "cl-eu-1").expect("checkpoint");
    let finality = state
        .observe_checkpoint(checkpoint)
        .expect("checkpoint revision");
    assert_eq!(
        finality
            .iter()
            .map(|revision| revision.status)
            .collect::<Vec<_>>(),
        [EthereumStatus::Safe, EthereumStatus::Finalized]
    );

    let reorg = state.reorg([b], [c]).expect("reorg");
    assert_eq!(
        reorg
            .iter()
            .map(|revision| (revision.block.hash, revision.status))
            .collect::<Vec<_>>(),
        [
            (b.hash, EthereumStatus::Reorged),
            (c.hash, EthereumStatus::CanonicalHead),
        ]
    );
    assert_eq!(state.head(), Some(c.hash));
}

#[test]
fn checkpoint_payloads_must_join_canonical_execution_ancestry() {
    let genesis = block(0, 1, 0);
    let a = block(1, 2, 1);
    let mut state = EthereumCanonicality::new();
    state.commit_segment([genesis, a]).expect("segment");

    let mismatch = EthereumCheckpoint::new(hash(9), a.hash, genesis.hash, "cl-eu-1")
        .expect("checkpoint input");
    assert!(matches!(
        state.observe_checkpoint(mismatch),
        Err(EthereumError::UnknownExecutionPayload { .. })
    ));

    let non_ancestor =
        EthereumCheckpoint::new(a.hash, genesis.hash, a.hash, "cl-eu-1").expect("checkpoint input");
    assert!(matches!(
        state.observe_checkpoint(non_ancestor),
        Err(EthereumError::InvalidCheckpointAncestry)
    ));
}

#[test]
fn duplicate_replay_is_noop_and_finalized_reversal_is_critical() {
    let genesis = block(0, 1, 0);
    let a = block(1, 2, 1);
    let mut state = EthereumCanonicality::new();
    state.commit_segment([genesis, a]).expect("segment");
    assert!(
        state
            .commit_segment([genesis, a])
            .expect("replay")
            .is_empty()
    );
    state
        .observe_checkpoint(
            EthereumCheckpoint::new(a.hash, a.hash, genesis.hash, "cl-eu-1").expect("checkpoint"),
        )
        .expect("finalize genesis");

    assert!(matches!(
        state.reorg([genesis, a], [block(0, 8, 0)]),
        Err(EthereumError::FinalizedReversalCritical { .. })
    ));
    assert_eq!(state.head(), Some(a.hash), "critical failure is atomic");
}

#[test]
fn revision_numbers_are_strictly_monotonic() {
    let mut state = EthereumCanonicality::new();
    let revisions = state
        .commit_segment([block(0, 1, 0), block(1, 2, 1)])
        .expect("segment");
    assert_eq!(
        revisions
            .iter()
            .map(EthereumRevision::revision)
            .collect::<Vec<_>>(),
        [1, 2]
    );
}

fn block(number: u64, hash_byte: u8, parent_byte: u8) -> ExecutionBlock {
    ExecutionBlock::new(number, hash(hash_byte), hash(parent_byte))
}

fn hash(byte: u8) -> B256 {
    B256::from([byte; 32])
}
