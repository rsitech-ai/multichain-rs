use ethereum_reth_connector::{
    RecordedRethNotification, RethConnectorError, RethSourceConfig, RethTransition,
};

#[test]
fn recorded_reorg_retains_exact_bytes_and_segment_order() {
    let bytes = fixture("reth-reorg.json");
    let notification =
        RecordedRethNotification::from_json("reth-eu-1", &bytes).expect("notification");
    assert_eq!(notification.raw_json(), bytes);
    let RethTransition::Reorged { old, new } = notification.transition() else {
        panic!("expected reorg");
    };
    assert_eq!(old.len(), 1);
    assert_eq!(new.len(), 1);
    assert_eq!(old[0].number, 2);
    assert_eq!(new[0].number, 2);
    assert_ne!(old[0].hash, new[0].hash);
}

#[test]
fn source_config_is_ethereum_only_and_endpoint_safe() {
    assert!(RethSourceConfig::new("reth-eu-1", "http://127.0.0.1:8545", 1).is_ok());
    assert!(matches!(
        RethSourceConfig::new("reth-eu-1", "http://127.0.0.1:8545", 56),
        Err(RethConnectorError::WrongChainId(56))
    ));
    assert!(matches!(
        RethSourceConfig::new("reth-eu-1", "http://user:secret@example.com", 1),
        Err(RethConnectorError::UnsafeEndpoint)
    ));
    assert!(matches!(
        RethSourceConfig::new("reth-eu-1", "http://127.0.0.1.evil.example", 1),
        Err(RethConnectorError::UnsafeEndpoint)
    ));
}

#[test]
fn malformed_or_unknown_notification_fails_closed() {
    assert!(RecordedRethNotification::from_json("reth-eu-1", b"{}").is_err());
    assert!(RecordedRethNotification::from_json("reth-eu-1", br#"{"kind":"invented"}"#).is_err());
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/ethereum")
            .join(name),
    )
    .expect("fixture")
}
