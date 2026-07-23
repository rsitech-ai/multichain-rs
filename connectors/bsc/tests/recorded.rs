use bsc_connector::{BscConnectorError, BscNodeConfig, BscNodeKind, RecordedBscHeads};

#[test]
fn config_requires_official_bsc_client_and_chain_id_56() {
    assert!(
        BscNodeConfig::new(
            "bsc-eu-1",
            "http://127.0.0.1:8545",
            56,
            BscNodeKind::OfficialBsc,
        )
        .is_ok()
    );
    assert!(matches!(
        BscNodeConfig::new(
            "bsc-eu-1",
            "http://127.0.0.1:8545",
            1,
            BscNodeKind::OfficialBsc,
        ),
        Err(BscConnectorError::WrongChainId(1))
    ));
    assert!(matches!(
        BscNodeConfig::new(
            "bsc-eu-1",
            "http://127.0.0.1:8545",
            56,
            BscNodeKind::GenericEthereum,
        ),
        Err(BscConnectorError::WrongClient)
    ));
}

#[test]
fn recorded_head_and_native_finalized_tag_retain_exact_source_bytes() {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/bsc/head-finalized.json"),
    )
    .expect("fixture");
    let recorded = RecordedBscHeads::from_json("bsc-eu-1", &bytes).expect("recorded");
    assert_eq!(recorded.raw_json(), bytes);
    assert_eq!(recorded.chain_id(), 56);
    assert_eq!(recorded.head().number, 2);
    assert_eq!(recorded.finalized().number, 1);
    assert_eq!(recorded.observed_at_unix_ms(), 1_721_000_000_000);
}

#[test]
fn malformed_or_wrong_chain_observations_fail_closed() {
    assert!(RecordedBscHeads::from_json("bsc-eu-1", b"{}").is_err());
    let wrong_chain = br#"{"chainId":"0x1","client":"bnb-chain/bsc","head":{"number":"0x0","hash":"0x0101010101010101010101010101010101010101010101010101010101010101","parentHash":"0x0000000000000000000000000000000000000000000000000000000000000000"},"finalized":{"number":"0x0","hash":"0x0101010101010101010101010101010101010101010101010101010101010101","parentHash":"0x0000000000000000000000000000000000000000000000000000000000000000"},"observedAtUnixMs":1}"#;
    assert!(matches!(
        RecordedBscHeads::from_json("bsc-eu-1", wrong_chain),
        Err(BscConnectorError::WrongChainId(1))
    ));
}
