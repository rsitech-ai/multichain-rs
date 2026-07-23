use solana_domain::{
    AccountWrite, AddressTableLookup, Blockhash, CompiledInstruction, ExecutionStatus, ForkId,
    InnerInstruction, InstructionPath, Lamports, MessageVersion, Pubkey, Signature, Slot,
    SolanaMessage, SolanaTransaction, TokenBalanceChange, TransactionKey,
};

fn key() -> TransactionKey {
    TransactionKey::new(
        Signature::from([9; 64]),
        ForkId::new(Slot::new(100), Blockhash::new_from_array([8; 32])),
    )
}

#[test]
fn versioned_transaction_retains_alt_cpi_logs_balances_and_failure() {
    let message = SolanaMessage::try_new(
        MessageVersion::V0,
        vec![
            Pubkey::new_from_array([1; 32]),
            Pubkey::new_from_array([2; 32]),
        ],
        vec![AddressTableLookup::new(
            Pubkey::new_from_array([3; 32]),
            vec![0],
            vec![1],
        )],
        vec![CompiledInstruction::try_new(1, vec![0], vec![7, 8]).expect("instruction")],
    )
    .expect("message");
    let inner =
        InnerInstruction::try_new(InstructionPath::new(0, 0), 1, vec![0], vec![9]).expect("inner");
    let expected_key = key();
    let transaction = SolanaTransaction::try_new(
        expected_key.clone(),
        message,
        vec![inner],
        vec!["Program log: nested failure".to_owned()],
        vec![Lamports::new(10)],
        vec![Lamports::new(4)],
        vec![TokenBalanceChange::new(
            0,
            Pubkey::new_from_array([4; 32]),
            "100",
            "75",
            6,
        )],
        Lamports::new(6),
        Some(1_234),
        ExecutionStatus::Failed {
            error: "custom program error: 0x1".to_owned(),
        },
        vec![1, 2, 3],
    )
    .expect("transaction");

    assert_eq!(transaction.key(), &expected_key);
    assert_eq!(transaction.message().version(), MessageVersion::V0);
    assert_eq!(transaction.message().address_table_lookups().len(), 1);
    assert_eq!(
        transaction.inner_instructions()[0].path(),
        InstructionPath::new(0, 0)
    );
    assert_eq!(transaction.logs().len(), 1);
    assert_eq!(transaction.fee(), Lamports::new(6));
    assert_eq!(transaction.compute_units_consumed(), Some(1_234));
    assert!(matches!(
        transaction.status(),
        ExecutionStatus::Failed { .. }
    ));
    assert_eq!(transaction.raw_transaction(), &[1, 2, 3]);
}

#[test]
fn boundary_mismatches_and_oversized_values_fail_closed() {
    let message = SolanaMessage::try_new(
        MessageVersion::Legacy,
        vec![Pubkey::new_from_array([1; 32])],
        Vec::new(),
        Vec::new(),
    )
    .expect("message");
    let result = SolanaTransaction::try_new(
        key(),
        message,
        Vec::new(),
        Vec::new(),
        vec![Lamports::new(1), Lamports::new(2)],
        vec![Lamports::new(1)],
        Vec::new(),
        Lamports::new(0),
        None,
        ExecutionStatus::Succeeded,
        Vec::new(),
    );
    assert!(result.is_err());

    assert!(CompiledInstruction::try_new(0, Vec::new(), vec![0; 1_048_577]).is_err());
    assert!(
        AccountWrite::try_new(
            ForkId::new(Slot::new(1), Blockhash::new_from_array([1; 32])),
            Pubkey::new_from_array([2; 32]),
            Pubkey::new_from_array([3; 32]),
            Lamports::new(1),
            vec![0; 1_048_577],
            false,
            0,
            1,
        )
        .is_err()
    );
}
