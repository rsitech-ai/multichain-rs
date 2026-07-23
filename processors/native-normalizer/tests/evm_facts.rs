use evm_domain::{
    Address, B256, EvmAmount, EvmBlock, EvmLog, EvmNetwork, EvmReceipt, EvmTransaction,
};
use native_normalizer::{EVM_FACT_SCHEMA, EvmFactBatch, EvmFactContext};

#[test]
fn ethereum_and_bsc_facts_share_shapes_but_keep_chain_ids_and_statuses() {
    let ethereum = facts(EvmNetwork::EthereumMainnet, "finalized");
    let bsc = facts(EvmNetwork::BscMainnet, "fast_finalized");

    assert_eq!(ethereum.blocks[0].chain_id, 1);
    assert_eq!(bsc.blocks[0].chain_id, 56);
    assert_eq!(ethereum.blocks[0].finality, "finalized");
    assert_eq!(bsc.blocks[0].finality, "fast_finalized");
    assert_eq!(
        ethereum.transactions[0].value,
        "115792089237316195423570985008687907853269984665640564039457584007913129639935"
    );
    assert_eq!(ethereum.logs[0].log_index, 0);
    assert_eq!(ethereum.logs[0].source_id, "evm-source-1");
}

#[test]
fn evm_schema_is_append_only_chain_qualified_and_decoder_isolated() {
    assert!(EVM_FACT_SCHEMA.contains("ENGINE = MergeTree"));
    assert!(!EVM_FACT_SCHEMA.contains("ReplacingMergeTree"));
    assert!(EVM_FACT_SCHEMA.contains("chain_id UInt64"));
    assert!(EVM_FACT_SCHEMA.contains("value UInt256"));
    assert!(EVM_FACT_SCHEMA.contains("argMax(finality, revision)"));
    assert!(EVM_FACT_SCHEMA.contains("evm_decoder_revisions"));
    assert!(EVM_FACT_SCHEMA.contains("raw_data_hex"));
}

fn facts(network: EvmNetwork, finality: &str) -> EvmFactBatch {
    let hash = B256::from([3; 32]);
    let transaction = EvmTransaction::new(
        hash,
        Address::from([1; 20]),
        None,
        EvmAmount::from_decimal_str(
            "115792089237316195423570985008687907853269984665640564039457584007913129639935",
        )
        .expect("amount"),
        7,
        100_000,
        Some(EvmAmount::from(100_u64)),
        vec![B256::from([8; 32])],
    );
    let log = EvmLog::new(
        Address::from([2; 20]),
        vec![B256::from([9; 32])],
        vec![0xde, 0xad],
        hash,
        0,
    )
    .expect("log");
    let receipt = EvmReceipt::new(hash, true, 50_000, vec![log]).expect("receipt");
    let block = EvmBlock::new(
        network,
        100,
        B256::from([4; 32]),
        B256::from([5; 32]),
        vec![transaction],
        vec![receipt],
    )
    .expect("block");
    let context = EvmFactContext::new(network, "canonical", finality, 9)
        .expect("context")
        .with_lineage("evm-source-1", [0x11; 16], [0x22; 32], 1_000)
        .expect("lineage");
    EvmFactBatch::from_block(&block, &context).expect("facts")
}
