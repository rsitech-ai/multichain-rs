use std::fmt::Write as _;

use bitcoin_domain::{ScriptType, parse_block, parse_transaction};
use chain_domain::BitcoinNetwork;

#[test]
fn corpus_manifest_is_complete_and_hashes_match() {
    let repository_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest = test_fixtures::bitcoin::BitcoinFixtureManifest::load(&repository_root)
        .expect("manifest loads");
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.checksum_scope, "sha256(decoded_consensus_bytes)");
    assert_eq!(
        manifest.validation_scope,
        "consensus_decode_and_merkle_commitment; proof_of_work_not_asserted"
    );
    assert_eq!(manifest.objects.len(), 18);
    assert_eq!(
        manifest
            .verify_all(&repository_root)
            .expect("all fixture checksums")
            .len(),
        18
    );
}

#[test]
fn legacy_vector_preserves_identity_amount_and_script() {
    let bytes = fixture("legacy_transaction.hex");
    let transaction = parse_transaction(&bytes).expect("legacy transaction");

    assert_eq!(
        transaction.txid().to_string(),
        "38bbcabc57b7ab88149ac7143de3c2da113894729da706cf68bc7cec77ce0d78"
    );
    assert_eq!(
        transaction.txid().to_string(),
        transaction.wtxid().to_string()
    );
    let output = transaction.output(0).expect("first output");
    assert_eq!(output.value_sats().value(), 5_000);
    assert_eq!(
        encode_hex(output.script_pubkey_id().as_bytes()),
        "6cdf10b629f2c1b8cab546ef4e4ddd850d40d17a69cc5e2c6fdb17dfda0df353"
    );
    let presentation = output.script_pubkey().presentation(BitcoinNetwork::Mainnet);
    assert_eq!(presentation.script_type, ScriptType::P2pkh);
    assert_eq!(presentation.addresses.len(), 1);
    assert_eq!(transaction.consensus_bytes(), bytes);
}

#[test]
fn witness_vectors_separate_txid_and_wtxid() {
    for (name, expected_txid, expected_wtxid) in [
        (
            "segwit_v0_transaction.hex",
            "9f969fc01aca0a12933f7d80920ed9fa2ccea2742b1bbdc60b6ed26728c4a1ed",
            "ee49d957a6d4406b3bff33fa79b84fc456b38eb38d7e2588b92c587e1c3230f9",
        ),
        (
            "taproot_transaction.hex",
            "e4ec5782f3a528ac5fe1f316e41c6bd865090e35ce16f20dad99ad74dbdab453",
            "802f3fd321bbefc67308d60a34708dc962f1f12dfc09a42091c888dbb29289b0",
        ),
    ] {
        let transaction = parse_transaction(&fixture(name)).expect(name);
        assert_eq!(transaction.txid().to_string(), expected_txid);
        assert_eq!(transaction.wtxid().to_string(), expected_wtxid);
        assert_ne!(
            transaction.txid().to_string(),
            transaction.wtxid().to_string()
        );
    }
}

#[test]
fn relationships_and_non_address_scripts_remain_chain_native() {
    let rbf = parse_transaction(&fixture("rbf_signaling.hex")).expect("RBF");
    assert!(rbf.inputs()[0].sequence < 0xffff_fffe);

    let parent = parse_transaction(&fixture("cpfp_parent.hex")).expect("parent");
    let child = parse_transaction(&fixture("cpfp_child.hex")).expect("child");
    assert_eq!(child.inputs()[0].previous_output.txid, parent.txid());

    let conflict_a = parse_transaction(&fixture("conflict_a.hex")).expect("conflict A");
    let conflict_b = parse_transaction(&fixture("conflict_b.hex")).expect("conflict B");
    assert_eq!(
        conflict_a.inputs()[0].previous_output,
        conflict_b.inputs()[0].previous_output
    );
    assert_ne!(conflict_a.txid(), conflict_b.txid());

    let non_address =
        parse_transaction(&fixture("non_address_script.hex")).expect("OP_RETURN transaction");
    let presentation = non_address.outputs()[0]
        .script_pubkey()
        .presentation(BitcoinNetwork::Mainnet);
    assert_eq!(presentation.script_type, ScriptType::OpReturn);
    assert!(presentation.addresses.is_empty());
}

#[test]
fn block_vectors_validate_merkle_roots_and_branch_links() {
    let main_1 = parse_block(&fixture("reorg_main_1.hex")).expect("main block 1");
    let main_2 = parse_block(&fixture("reorg_main_2.hex")).expect("main block 2");
    let alt_1 = parse_block(&fixture("reorg_alt_1.hex")).expect("alternate block 1");
    let alt_2 = parse_block(&fixture("reorg_alt_2.hex")).expect("alternate block 2");

    assert_eq!(main_2.previous_block_hash(), main_1.block_hash());
    assert_eq!(alt_2.previous_block_hash(), alt_1.block_hash());
    assert_ne!(main_1.block_hash(), alt_1.block_hash());
    assert_eq!(main_1.transactions().len(), 1);
    assert_eq!(main_1.consensus_bytes(), fixture("reorg_main_1.hex"));
}

fn fixture(name: &str) -> Vec<u8> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/bitcoin/objects")
        .join(name);
    decode_hex(&std::fs::read_to_string(root).expect("fixture"))
}

fn decode_hex(text: &str) -> Vec<u8> {
    let text = text.trim();
    assert!(text.len().is_multiple_of(2));
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("fixture hex"))
        .collect()
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut hex, byte| {
            write!(hex, "{byte:02x}").expect("writing into a String cannot fail");
            hex
        })
}
