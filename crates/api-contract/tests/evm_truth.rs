use api_contract::{Completeness, EvmCoverage, EvmTruth, TruthError};
use evm_domain::EvmNetwork;

#[test]
fn evm_truth_exposes_chain_native_finality_and_coverage() {
    let ethereum = EvmTruth::new(
        EvmNetwork::EthereumMainnet,
        ["reth-eu-1", "lighthouse-eu-1"],
        10,
        "canonical",
        "safe",
        Completeness::Complete,
        EvmCoverage::new(true, false, false),
        1_000,
    )
    .expect("Ethereum");
    let bsc = EvmTruth::new(
        EvmNetwork::BscMainnet,
        ["bsc-eu-1"],
        11,
        "canonical",
        "fast_finalized",
        Completeness::Complete,
        EvmCoverage::new(true, false, false),
        1_000,
    )
    .expect("BSC");

    let ethereum_json = serde_json::to_value(ethereum).expect("json");
    let bsc_json = serde_json::to_value(bsc).expect("json");
    assert_eq!(ethereum_json["chain_id"], 1);
    assert_eq!(ethereum_json["finality"], "safe");
    assert_eq!(ethereum_json["coverage"]["traces"], false);
    assert_eq!(bsc_json["chain_id"], 56);
    assert_eq!(bsc_json["finality"], "fast_finalized");
}

#[test]
fn ethereum_and_bsc_finality_vocabularies_cannot_cross() {
    assert!(matches!(
        EvmTruth::new(
            EvmNetwork::EthereumMainnet,
            ["reth-eu-1"],
            1,
            "canonical",
            "fast_finalized",
            Completeness::Complete,
            EvmCoverage::new(true, false, false),
            1,
        ),
        Err(TruthError::InvalidEvmFinality { chain_id: 1, .. })
    ));
    assert!(matches!(
        EvmTruth::new(
            EvmNetwork::BscMainnet,
            ["bsc-eu-1"],
            1,
            "canonical",
            "safe",
            Completeness::Complete,
            EvmCoverage::new(true, false, false),
            1,
        ),
        Err(TruthError::InvalidEvmFinality { chain_id: 56, .. })
    ));
}
