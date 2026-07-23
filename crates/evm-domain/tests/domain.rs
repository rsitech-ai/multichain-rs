use std::str::FromStr as _;

use evm_domain::{
    Address, B256, BscStatus, EthereumStatus, EvmAmount, EvmBlock, EvmError, EvmLog, EvmNetwork,
    EvmReceipt, EvmTransaction,
};

#[test]
fn chain_identity_and_u256_json_are_exact() {
    assert_eq!(
        EvmNetwork::try_from(1).expect("ethereum"),
        EvmNetwork::EthereumMainnet
    );
    assert_eq!(
        EvmNetwork::try_from(56).expect("bsc"),
        EvmNetwork::BscMainnet
    );
    assert!(EvmNetwork::try_from(0).is_err());

    let maximum = EvmAmount::from_decimal_str(
        "115792089237316195423570985008687907853269984665640564039457584007913129639935",
    )
    .expect("u256 max");
    assert_eq!(
        serde_json::to_string(&maximum).expect("serialize"),
        "\"115792089237316195423570985008687907853269984665640564039457584007913129639935\""
    );
    assert!(serde_json::from_str::<EvmAmount>("1").is_err());
    assert!(
        EvmAmount::from_decimal_str(
            "115792089237316195423570985008687907853269984665640564039457584007913129639936"
        )
        .is_err()
    );
}

#[test]
fn receipt_and_log_invariants_fail_closed() {
    let tx_hash = hash(1);
    let transaction = EvmTransaction::new(
        tx_hash,
        address(1),
        None,
        EvmAmount::from(42_u64),
        7,
        21_000,
        Some(EvmAmount::from(100_u64)),
        Vec::new(),
    );
    assert!(
        transaction.to().is_none(),
        "contract creation stays explicit"
    );
    let log =
        EvmLog::new(address(2), vec![hash(3)], vec![0xde, 0xad], tx_hash, 0).expect("valid log");
    let receipt = EvmReceipt::new(tx_hash, true, 21_000, vec![log.clone()]).expect("receipt");
    let block = EvmBlock::new(
        EvmNetwork::EthereumMainnet,
        1,
        hash(4),
        hash(0),
        vec![transaction.clone()],
        vec![receipt.clone()],
    )
    .expect("block");
    assert_eq!(block.transactions().len(), 1);

    assert!(matches!(
        EvmBlock::new(
            EvmNetwork::EthereumMainnet,
            1,
            hash(4),
            hash(0),
            vec![transaction.clone()],
            Vec::new(),
        ),
        Err(EvmError::ReceiptCardinality { .. })
    ));
    assert!(matches!(
        EvmReceipt::new(tx_hash, true, 21_000, vec![log.clone(), log]),
        Err(EvmError::DuplicateLogKey { .. })
    ));
    assert!(
        EvmBlock::new(
            EvmNetwork::BscMainnet,
            1,
            hash(4),
            hash(0),
            vec![transaction],
            vec![receipt],
        )
        .is_ok()
    );
}

#[test]
fn ethereum_and_bsc_statuses_are_distinct_chain_native_types() {
    fn ethereum_only(status: EthereumStatus) -> &'static str {
        status.as_str()
    }
    fn bsc_only(status: BscStatus) -> &'static str {
        status.as_str()
    }

    assert_eq!(ethereum_only(EthereumStatus::Safe), "safe");
    assert_eq!(ethereum_only(EthereumStatus::Finalized), "finalized");
    assert_eq!(bsc_only(BscStatus::FastFinalized), "fast_finalized");
}

fn hash(byte: u8) -> B256 {
    B256::from([byte; 32])
}

fn address(byte: u8) -> Address {
    Address::from([byte; 20])
}

#[test]
fn malformed_external_identities_are_rejected() {
    assert!(Address::from_str("0x1234").is_err());
    assert!(B256::from_str("0x1234").is_err());
}
