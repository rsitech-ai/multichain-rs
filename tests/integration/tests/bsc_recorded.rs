use bsc_connector::RecordedBscHeads;
use evm_canonicality::{BscCanonicality, BscFinalityObservation, ExecutionBlock};
use evm_domain::{B256, BscStatus};

#[test]
fn official_bsc_head_and_native_finalized_tag_join_canonical_ancestry() {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/bsc/head-finalized.json"),
    )
    .expect("fixture");
    let recorded = RecordedBscHeads::from_json("bsc-eu-1", &bytes).expect("recorded");

    let mut state = BscCanonicality::new();
    state
        .commit_segment([block(0, 1, 0), block(1, 2, 1), block(2, 3, 2)])
        .expect("canonical segment");
    let revision = state
        .observe_finalized(
            BscFinalityObservation::new(
                recorded.finalized().hash,
                recorded.head().hash,
                recorded.source_id(),
                recorded.observed_at_unix_ms(),
            )
            .expect("observation"),
        )
        .expect("native finality");

    assert_eq!(revision.status, BscStatus::FastFinalized);
    assert_eq!(revision.block, recorded.finalized());
    assert_eq!(state.head(), Some(recorded.head().hash));
}

fn block(number: u64, hash_byte: u8, parent_byte: u8) -> ExecutionBlock {
    ExecutionBlock::new(number, hash(hash_byte), hash(parent_byte))
}

fn hash(byte: u8) -> B256 {
    B256::from([byte; 32])
}
