use native_normalizer::{
    SOLANA_FACT_SCHEMA, SolanaCoverageTier, SolanaFactBatch, SolanaFactContext,
};
use solana_domain::{
    AccountWrite, AddressTableLookup, Blockhash, CompiledInstruction, ExecutionStatus, ForkId,
    InnerInstruction, InstructionPath, Lamports, MessageVersion, Pubkey, Signature, Slot,
    SolanaMessage, SolanaTransaction, TokenBalanceChange, TransactionKey,
};

#[test]
fn fork_qualified_transaction_materializes_native_rows_with_exact_lineage() {
    let transaction = transaction();
    let context = SolanaFactContext::new(
        "canonical",
        "confirmed",
        4,
        SolanaCoverageTier::AllTransactions,
    )
    .expect("context")
    .with_lineage("yellowstone-a", [1; 16], [2; 32], 1_000)
    .expect("lineage");
    let facts = SolanaFactBatch::from_transaction(&transaction, &context).expect("facts");

    assert_eq!(facts.transactions.len(), 1);
    assert_eq!(facts.instructions.len(), 2);
    assert_eq!(facts.logs.len(), 1);
    assert_eq!(facts.balance_changes.len(), 2);
    assert_eq!(facts.token_balance_changes.len(), 1);
    assert_eq!(facts.transactions[0].slot, 50);
    assert_eq!(facts.transactions[0].commitment, "confirmed");
    assert_eq!(facts.transactions[0].source_id, "yellowstone-a");
    assert_eq!(facts.instructions[1].inner_index, Some(0));
    assert_eq!(facts.instructions[1].raw_data_hex, "09");
    assert_eq!(facts.balance_changes[0].pre_lamports, "10");
}

#[test]
fn schema_is_append_only_fork_aware_and_selected_account_bounded() {
    assert!(SOLANA_FACT_SCHEMA.contains("ENGINE = MergeTree"));
    assert!(!SOLANA_FACT_SCHEMA.contains("ReplacingMergeTree"));
    assert!(SOLANA_FACT_SCHEMA.contains("blockhash String"));
    assert!(SOLANA_FACT_SCHEMA.contains("signature String"));
    assert!(SOLANA_FACT_SCHEMA.contains("coverage_tier LowCardinality(String)"));
    assert!(SOLANA_FACT_SCHEMA.contains("argMax(commitment, revision)"));
    assert!(SOLANA_FACT_SCHEMA.contains("solana_account_writes_current"));
    assert!(SOLANA_FACT_SCHEMA.contains("raw_data_hex String"));
    assert!(SOLANA_FACT_SCHEMA.contains("solana_decoder_revisions"));
    assert!(SOLANA_FACT_SCHEMA.contains("error Nullable(String)"));
    assert!(
        SOLANA_FACT_SCHEMA.contains("ORDER BY (program_id, slot, instruction_identity, revision)")
    );
}

#[test]
fn selected_account_write_is_raw_fork_qualified_and_tier_is_enforced() {
    let fork = ForkId::new(Slot::new(51), Blockhash::new_from_array([7; 32]));
    let write = AccountWrite::try_new(
        fork,
        Pubkey::new_from_array([8; 32]),
        Pubkey::new_from_array([9; 32]),
        Lamports::new(u64::MAX),
        vec![0xde, 0xad],
        true,
        42,
        3,
    )
    .expect("write");
    let context = SolanaFactContext::new(
        "non_canonical",
        "dead",
        5,
        SolanaCoverageTier::SelectedAccounts,
    )
    .expect("context")
    .with_lineage("yellowstone-b", [3; 16], [4; 32], 2_000)
    .expect("lineage");

    let facts = SolanaFactBatch::from_account_write(&write, &context).expect("facts");
    assert_eq!(facts.account_writes.len(), 1);
    let row = &facts.account_writes[0];
    assert_eq!(row.slot, 51);
    assert_eq!(row.lamports, u64::MAX.to_string());
    assert_eq!(row.raw_data_hex, "dead");
    assert_eq!(row.commitment, "dead");
    assert_eq!(row.coverage_tier, "selected_accounts");
    assert_eq!(row.source_id, "yellowstone-b");
    assert!(row.executable);
    assert_eq!(row.write_version, 3);

    assert!(SolanaFactBatch::from_transaction(&transaction(), &context).is_err());
}

fn transaction() -> SolanaTransaction {
    let fork = ForkId::new(Slot::new(50), Blockhash::new_from_array([4; 32]));
    let message = SolanaMessage::try_new(
        MessageVersion::V0,
        vec![
            Pubkey::new_from_array([1; 32]),
            Pubkey::new_from_array([2; 32]),
        ],
        vec![AddressTableLookup::new(
            Pubkey::new_from_array([3; 32]),
            vec![0],
            Vec::new(),
        )],
        vec![CompiledInstruction::try_new(1, vec![0], vec![8]).expect("outer")],
    )
    .expect("message");
    SolanaTransaction::try_new(
        TransactionKey::new(Signature::from([5; 64]), fork),
        message,
        vec![
            InnerInstruction::try_new(InstructionPath::new(0, 0), 1, vec![0], vec![9])
                .expect("inner"),
        ],
        vec!["Program log: exact".to_owned()],
        vec![Lamports::new(10), Lamports::new(20)],
        vec![Lamports::new(9), Lamports::new(20)],
        vec![TokenBalanceChange::new(
            0,
            Pubkey::new_from_array([6; 32]),
            "18446744073709551615",
            "0",
            9,
        )],
        Lamports::new(1),
        Some(500),
        ExecutionStatus::Succeeded,
        vec![0xaa],
    )
    .expect("transaction")
}
