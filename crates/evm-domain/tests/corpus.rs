use evm_domain::{EvmError, EvmNetwork, RecordedBlock};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::fmt::Write as _;

#[derive(Debug, Deserialize)]
struct Manifest {
    fixtures: Vec<Fixture>,
}

#[test]
fn recorded_json_rejects_noncanonical_quantities_and_schema_drift() {
    let noncanonical = br#"{"number":"0x00","hash":"0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3","parentHash":"0x0000000000000000000000000000000000000000000000000000000000000000","transactions":[],"source":"fixture","capture_scope":"test"}"#;
    assert!(matches!(
        RecordedBlock::from_json(EvmNetwork::EthereumMainnet, noncanonical),
        Err(EvmError::InvalidQuantity(_))
    ));

    let unknown_field = br#"{"number":"0x0","hash":"0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3","parentHash":"0x0000000000000000000000000000000000000000000000000000000000000000","transactions":[],"source":"fixture","capture_scope":"test","silentFallback":true}"#;
    assert!(matches!(
        RecordedBlock::from_json(EvmNetwork::EthereumMainnet, unknown_field),
        Err(EvmError::InvalidJson(_))
    ));
}

#[derive(Debug, Deserialize)]
struct Fixture {
    file: String,
    sha256: String,
    chain_id: u64,
    block_hash: String,
}

#[test]
fn immutable_mainnet_fixture_manifest_matches_semantics_and_bytes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/evm");
    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).expect("manifest bytes"))
            .expect("manifest");
    assert_eq!(manifest.fixtures.len(), 2);

    for fixture in manifest.fixtures {
        let bytes = std::fs::read(root.join(&fixture.file)).expect("fixture bytes");
        let digest = Sha256::digest(&bytes);
        let digest_hex = digest
            .iter()
            .fold(String::with_capacity(64), |mut encoded, byte| {
                write!(encoded, "{byte:02x}").expect("writing into a String cannot fail");
                encoded
            });
        assert_eq!(digest_hex, fixture.sha256);
        let expected_network = EvmNetwork::try_from(fixture.chain_id).expect("known chain");
        let block = RecordedBlock::from_json(expected_network, &bytes).expect("recorded block");
        assert_eq!(block.network(), expected_network);
        assert_eq!(block.block_hash().to_string(), fixture.block_hash);
        assert_eq!(block.number(), 0);
        assert_eq!(block.raw_json(), bytes);
    }
}
