use bitcoin_domain::parse_block;
use chain_domain::BitcoinNetwork;
use native_normalizer::{
    BITCOIN_FACT_SCHEMA, BitcoinFactBatch, BitcoinFactContext, BitcoinFactError,
};

#[test]
fn native_block_rows_preserve_integer_amounts_identity_and_lineage() {
    let block = parse_block(&fixture("reorg_main_1.hex")).expect("block");
    let context = BitcoinFactContext::new(BitcoinNetwork::Regtest, 101, 7, "canonical")
        .expect("context")
        .with_lineage("observer-a", [0x11; 16], [0x22; 32], 1_000)
        .expect("lineage");
    let batch = BitcoinFactBatch::from_block(&block, &context).expect("facts");

    assert_eq!(batch.blocks.len(), 1);
    assert_eq!(batch.transactions.len(), block.transactions().len());
    assert_eq!(
        batch.inputs.len(),
        block
            .transactions()
            .iter()
            .map(|transaction| transaction.inputs().len())
            .sum::<usize>()
    );
    assert_eq!(
        batch.outputs.len(),
        block
            .transactions()
            .iter()
            .map(|transaction| transaction.outputs().len())
            .sum::<usize>()
    );
    assert_eq!(batch.blocks[0].block_hash, block.block_hash().to_string());
    assert_eq!(batch.blocks[0].revision, 7);
    assert_eq!(
        batch.blocks[0].source_session_id,
        "11111111111111111111111111111111"
    );
    assert_eq!(
        batch.blocks[0].observation_id,
        "2222222222222222222222222222222222222222222222222222222222222222"
    );
    assert!(batch.inputs.iter().all(|input| {
        input.source_id == "observer-a"
            && input.source_session_id == "11111111111111111111111111111111"
            && input.observation_id
                == "2222222222222222222222222222222222222222222222222222222222222222"
    }));
    assert!(batch.outputs.iter().all(|output| {
        output.source_id == "observer-a"
            && output.source_session_id == "11111111111111111111111111111111"
            && output.observation_id
                == "2222222222222222222222222222222222222222222222222222222222222222"
    }));
    assert!(
        batch
            .outputs
            .iter()
            .all(|output| output.value_sats <= 2_100_000_000_000_000)
    );
}

#[test]
fn revision_rows_are_append_only_and_current_queries_are_explicit() {
    assert!(BITCOIN_FACT_SCHEMA.contains("ENGINE = MergeTree"));
    assert!(!BITCOIN_FACT_SCHEMA.contains("ReplacingMergeTree"));
    assert!(BITCOIN_FACT_SCHEMA.contains("argMax(canonicality, revision)"));
    assert!(BITCOIN_FACT_SCHEMA.contains("bitcoin_blocks_current"));
    assert!(BITCOIN_FACT_SCHEMA.contains("bitcoin_mempool_membership_revisions"));

    assert!(matches!(
        BitcoinFactContext::new(BitcoinNetwork::Regtest, 0, 1, "canonical")
            .expect("base context")
            .with_lineage("", [0; 16], [0; 32], 0),
        Err(BitcoinFactError::EmptySourceId)
    ));
}

fn fixture(name: &str) -> Vec<u8> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/bitcoin/objects")
        .join(name);
    let text = std::fs::read_to_string(root).expect("fixture");
    (0..text.trim().len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex"))
        .collect()
}
