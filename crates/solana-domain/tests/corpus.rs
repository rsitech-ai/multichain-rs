use std::str::FromStr as _;

use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use solana_domain::{
    Blockhash, CompiledInstruction, ExecutionStatus, ForkId, Lamports, MessageVersion, Pubkey,
    Signature, Slot, SolanaMessage, SolanaTransaction, TransactionKey,
};

#[derive(Deserialize)]
struct Manifest {
    schema_version: u32,
    validation_scope: String,
    fixtures: Vec<ManifestEntry>,
}

#[derive(Deserialize)]
struct ManifestEntry {
    file: String,
    sha256: String,
    slot: u64,
    blockhash: String,
    signature: String,
}

#[derive(Deserialize)]
struct Fixture {
    capture_scope: String,
    network: String,
    slot: u64,
    blockhash: String,
    signature: String,
    version: u8,
    account_keys: Vec<String>,
    address_table_lookups: Vec<serde_json::Value>,
    instructions: Vec<FixtureInstruction>,
    inner_instructions: Vec<serde_json::Value>,
    logs: Vec<String>,
    pre_balances: Vec<String>,
    post_balances: Vec<String>,
    fee: String,
    compute_units_consumed: u64,
    execution_status: String,
}

#[derive(Deserialize)]
struct FixtureInstruction {
    program_id_index: u8,
    accounts: Vec<u8>,
    data_base58: String,
}

#[test]
fn immutable_mainnet_fixture_hash_and_identity_match() {
    let root = fixture_root();
    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).expect("manifest bytes"))
            .expect("manifest JSON");
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(
        manifest.validation_scope,
        "public_mainnet_finalized_rpc_semantic_subset_plus_domain_adversarial_vectors"
    );
    assert_eq!(manifest.fixtures.len(), 1);

    let entry = &manifest.fixtures[0];
    let bytes = std::fs::read(root.join(&entry.file)).expect("fixture bytes");
    assert_eq!(encode_hex(&Sha256::digest(&bytes)), entry.sha256);
    let fixture: Fixture = serde_json::from_slice(&bytes).expect("fixture JSON");
    assert_eq!(fixture.slot, entry.slot);
    assert_eq!(fixture.blockhash, entry.blockhash);
    assert_eq!(fixture.signature, entry.signature);
    assert_eq!(fixture.capture_scope, "rpc_semantic_subset");
    assert_eq!(fixture.network, "solana-mainnet-beta");
}

#[test]
fn mainnet_fixture_replays_into_fork_qualified_native_fact() {
    let fixture: Fixture = serde_json::from_slice(
        &std::fs::read(fixture_root().join("mainnet-v0-transaction.json")).expect("fixture bytes"),
    )
    .expect("fixture JSON");
    let blockhash = Blockhash::from_str(&fixture.blockhash).expect("blockhash");
    let signature = Signature::from_str(&fixture.signature).expect("signature");
    let account_keys = fixture
        .account_keys
        .iter()
        .map(|key| Pubkey::from_str(key).expect("pubkey"))
        .collect();
    let instructions = fixture
        .instructions
        .iter()
        .map(|instruction| {
            CompiledInstruction::try_new(
                instruction.program_id_index,
                instruction.accounts.clone(),
                bs58::decode(&instruction.data_base58)
                    .into_vec()
                    .expect("instruction data"),
            )
            .expect("bounded instruction")
        })
        .collect();
    let message = SolanaMessage::try_new(
        match fixture.version {
            0 => MessageVersion::V0,
            other => panic!("unexpected message version {other}"),
        },
        account_keys,
        Vec::new(),
        instructions,
    )
    .expect("message");
    assert!(fixture.address_table_lookups.is_empty());
    assert!(fixture.inner_instructions.is_empty());
    let pre_balances = fixture
        .pre_balances
        .iter()
        .map(|amount| serde_json::from_str(&format!("\"{amount}\"")).expect("pre balance"))
        .collect();
    let post_balances = fixture
        .post_balances
        .iter()
        .map(|amount| serde_json::from_str(&format!("\"{amount}\"")).expect("post balance"))
        .collect();
    let fee = serde_json::from_str(&format!("\"{}\"", fixture.fee)).expect("fee");
    let transaction = SolanaTransaction::try_new(
        TransactionKey::new(signature, ForkId::new(Slot::new(fixture.slot), blockhash)),
        message,
        Vec::new(),
        fixture.logs,
        pre_balances,
        post_balances,
        Vec::new(),
        fee,
        Some(fixture.compute_units_consumed),
        match fixture.execution_status.as_str() {
            "succeeded" => ExecutionStatus::Succeeded,
            other => panic!("unexpected status {other}"),
        },
        Vec::new(),
    )
    .expect("native fact");

    assert_eq!(transaction.key().fork_id().slot(), Slot::new(434_739_559));
    assert_eq!(transaction.message().instructions().len(), 4);
    assert_eq!(transaction.fee(), Lamports::new(62_984));
    assert_eq!(transaction.compute_units_consumed(), Some(602));
}

#[test]
fn token_2022_and_inner_instruction_bytes_remain_opaque_and_exact() {
    let token_2022 =
        Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb").expect("Token-2022");
    let instruction = CompiledInstruction::try_new(1, vec![0], vec![0x1a, 0xde, 0xad, 0xbe, 0xef])
        .expect("instruction");

    assert_eq!(
        token_2022.to_string(),
        "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
    );
    assert_eq!(instruction.data(), &[0x1a, 0xde, 0xad, 0xbe, 0xef]);
}

fn fixture_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/solana")
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}
