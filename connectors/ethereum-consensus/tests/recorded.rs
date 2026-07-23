use ethereum_consensus_connector::{
    ConsensusConnectorError, ConsensusSourceConfig, RecordedConsensusCheckpoint,
};

#[test]
fn checkpoint_retains_exact_bytes_and_builds_typed_join_evidence() {
    let bytes = fixture("consensus-checkpoint.json");
    let recorded =
        RecordedConsensusCheckpoint::from_json("lighthouse-eu-1", &bytes).expect("recorded");
    assert_eq!(recorded.raw_json(), bytes);
    let checkpoint = recorded.checkpoint().expect("checkpoint");
    assert_eq!(checkpoint.source_id(), "lighthouse-eu-1");
}

#[test]
fn slot_regression_and_unsafe_configuration_fail_closed() {
    let config =
        ConsensusSourceConfig::new("lighthouse-eu-1", "http://127.0.0.1:5052").expect("config");
    let bytes = fixture("consensus-checkpoint.json");
    let first = RecordedConsensusCheckpoint::from_json(config.source_id(), &bytes).expect("first");
    let mut cursor = config.cursor();
    cursor.observe(&first).expect("advance");
    assert!(matches!(
        cursor.observe(&first),
        Err(ConsensusConnectorError::SlotRegression { .. })
    ));
    assert!(ConsensusSourceConfig::new("", "http://127.0.0.1:5052").is_err());
    assert!(
        ConsensusSourceConfig::new("lighthouse-eu-1", "http://user:secret@example.com").is_err()
    );
    assert!(
        ConsensusSourceConfig::new("lighthouse-eu-1", "http://localhost.evil.example").is_err()
    );
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/ethereum")
            .join(name),
    )
    .expect("fixture")
}
