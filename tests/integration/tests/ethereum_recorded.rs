use ethereum_consensus_connector::RecordedConsensusCheckpoint;
use ethereum_reth_connector::{RecordedRethNotification, RethTransition};
use evm_canonicality::{EthereumCanonicality, ExecutionBlock};
use evm_domain::{B256, EthereumStatus};

#[test]
fn recorded_reth_and_consensus_evidence_join_before_ordered_reorg() {
    let mut state = EthereumCanonicality::new();
    state
        .commit_segment([block(0, 1, 0), block(1, 2, 1), block(2, 3, 2)])
        .expect("execution segment");

    let consensus = RecordedConsensusCheckpoint::from_json(
        "lighthouse-eu-1",
        &fixture("consensus-checkpoint.json"),
    )
    .expect("consensus evidence");
    let finality = state
        .observe_checkpoint(consensus.checkpoint().expect("typed checkpoint"))
        .expect("payload join");
    assert_eq!(
        finality
            .iter()
            .map(|revision| revision.status)
            .collect::<Vec<_>>(),
        [EthereumStatus::Safe, EthereumStatus::Finalized]
    );

    let reth = RecordedRethNotification::from_json("reth-eu-1", &fixture("reth-reorg.json"))
        .expect("Reth evidence");
    let RethTransition::Reorged { old, new } = reth.transition() else {
        panic!("expected reorg");
    };
    let revisions = state
        .reorg(old.iter().copied(), new.iter().copied())
        .expect("ordered reorg");
    assert_eq!(
        revisions
            .iter()
            .map(|revision| revision.status)
            .collect::<Vec<_>>(),
        [EthereumStatus::Reorged, EthereumStatus::CanonicalHead]
    );
    assert_eq!(state.head(), Some(hash(4)));
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/ethereum")
            .join(name),
    )
    .expect("fixture")
}

fn block(number: u64, hash_byte: u8, parent_byte: u8) -> ExecutionBlock {
    ExecutionBlock::new(number, hash(hash_byte), hash(parent_byte))
}

fn hash(byte: u8) -> B256 {
    B256::from([byte; 32])
}
