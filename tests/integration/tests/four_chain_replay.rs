use std::{path::Path, str::FromStr as _};

use bitcoin::{
    Amount, Block, BlockHash as NativeBlockHash, CompactTarget, Network, OutPoint, ScriptBuf,
    Sequence, Transaction, TxIn, TxMerkleNode, TxOut, Witness, absolute,
    block::{Header, Version},
    consensus::{deserialize, serialize},
    hashes::Hash as _,
    transaction,
};
use bitcoin_canonicality::BitcoinState;
use bitcoin_domain::parse_block;
use bsc_connector::RecordedBscHeads;
use chain_domain::BitcoinNetwork;
use ethereum_consensus_connector::RecordedConsensusCheckpoint;
use ethereum_reth_connector::{RecordedRethNotification, RethTransition};
use evm_canonicality::{
    BscCanonicality, BscFinalityObservation, EthereumCanonicality, ExecutionBlock,
};
use evm_domain::B256;
use native_normalizer::{
    BitcoinFactBatch, BitcoinFactContext, SolanaCoverageTier, SolanaFactBatch, SolanaFactContext,
};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use solana_canonicality::{Commitment, SolanaCanonicality};
use solana_domain::{
    AccountWrite, Blockhash, CompiledInstruction, ExecutionStatus, ForkId, Lamports,
    MessageVersion, Pubkey, Signature, Slot, SolanaMessage, SolanaTransaction, TransactionKey,
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReplayDigest {
    facts: [u8; 32],
    state: [u8; 32],
}

#[test]
fn every_chain_replays_twice_to_identical_logical_fact_and_state_hashes() {
    let first = replay_all();
    let second = replay_all();

    assert_eq!(first, second);
    assert_eq!(first.len(), 4);
    assert!(first.iter().all(|digest| digest.facts != [0; 32]));
    assert!(first.iter().all(|digest| digest.state != [0; 32]));
    for (index, left) in first.iter().enumerate() {
        assert!(
            first
                .iter()
                .skip(index + 1)
                .all(|right| left.facts != right.facts)
        );
    }
}

fn replay_all() -> Vec<ReplayDigest> {
    vec![
        replay_bitcoin(),
        replay_ethereum(),
        replay_bsc(),
        replay_solana(),
    ]
}

fn replay_bitcoin() -> ReplayDigest {
    let first = regtest_genesis();
    let second = mine_regtest_child(&first, 1);
    let first_facts = BitcoinFactBatch::from_block(
        &first,
        &BitcoinFactContext::new(BitcoinNetwork::Regtest, 0, 1, "canonical")
            .expect("context")
            .with_lineage("observer-a", [1; 16], [2; 32], 1_000)
            .expect("lineage"),
    )
    .expect("first facts");
    let second_facts = BitcoinFactBatch::from_block(
        &second,
        &BitcoinFactContext::new(BitcoinNetwork::Regtest, 1, 2, "canonical")
            .expect("context")
            .with_lineage("observer-a", [1; 16], [3; 32], 2_000)
            .expect("lineage"),
    )
    .expect("second facts");
    let facts = serde_json::to_vec(&(
        first_facts.blocks,
        first_facts.transactions,
        first_facts.inputs,
        first_facts.outputs,
        second_facts.blocks,
        second_facts.transactions,
        second_facts.inputs,
        second_facts.outputs,
    ))
    .expect("Bitcoin facts JSON");

    let mut state = BitcoinState::new(BitcoinNetwork::Regtest);
    state.observe_block(first).expect("first canonical state");
    state.observe_block(second).expect("second canonical state");
    ReplayDigest {
        facts: digest(&facts),
        state: state.state_hash(),
    }
}

fn replay_ethereum() -> ReplayDigest {
    let mut state = EthereumCanonicality::new();
    let mut logical_facts = Vec::new();
    logical_facts.extend(
        state
            .commit_segment([block(0, 1, 0), block(1, 2, 1), block(2, 3, 2)])
            .expect("execution segment"),
    );
    let consensus = RecordedConsensusCheckpoint::from_json(
        "lighthouse-eu-1",
        &fixture("ethereum/consensus-checkpoint.json"),
    )
    .expect("consensus fixture");
    logical_facts.extend(
        state
            .observe_checkpoint(consensus.checkpoint().expect("checkpoint"))
            .expect("checkpoint join"),
    );
    let reth =
        RecordedRethNotification::from_json("reth-eu-1", &fixture("ethereum/reth-reorg.json"))
            .expect("Reth fixture");
    let RethTransition::Reorged { old, new } = reth.transition() else {
        panic!("expected recorded Ethereum reorg");
    };
    logical_facts.extend(
        state
            .reorg(old.iter().copied(), new.iter().copied())
            .expect("reorg"),
    );
    let rows = logical_facts
        .iter()
        .map(|revision| {
            format!(
                "{}:{}:{}:{}:{}",
                revision.block.number,
                revision.block.hash,
                revision.status.as_str(),
                revision.revision(),
                revision.source_id.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    ReplayDigest {
        facts: digest(rows.as_bytes()),
        state: digest(state.head().expect("Ethereum head").to_string().as_bytes()),
    }
}

fn replay_bsc() -> ReplayDigest {
    let recorded = RecordedBscHeads::from_json("bsc-eu-1", &fixture("bsc/head-finalized.json"))
        .expect("BSC fixture");
    let mut state = BscCanonicality::new();
    let mut logical_facts = state
        .commit_segment([block(0, 1, 0), block(1, 2, 1), block(2, 3, 2)])
        .expect("BSC segment");
    logical_facts.push(
        state
            .observe_finalized(
                BscFinalityObservation::new(
                    recorded.finalized().hash,
                    recorded.head().hash,
                    recorded.source_id(),
                    recorded.observed_at_unix_ms(),
                )
                .expect("BSC evidence"),
            )
            .expect("BSC finality"),
    );
    let rows = logical_facts
        .iter()
        .map(|revision| {
            format!(
                "{}:{}:{}:{}:{}",
                revision.block.number,
                revision.block.hash,
                revision.status.as_str(),
                revision.revision,
                revision.source_id.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    ReplayDigest {
        facts: digest(rows.as_bytes()),
        state: digest(state.head().expect("BSC head").to_string().as_bytes()),
    }
}

fn replay_solana() -> ReplayDigest {
    let fixture: SolanaFixture =
        serde_json::from_slice(&fixture("solana/mainnet-v0-transaction.json"))
            .expect("Solana fixture");
    let fork = ForkId::new(
        Slot::new(fixture.slot),
        Blockhash::from_str(&fixture.blockhash).expect("blockhash"),
    );
    let transaction = fixture.into_transaction(fork.clone());
    let facts = SolanaFactBatch::from_transaction(
        &transaction,
        &SolanaFactContext::new(
            "canonical",
            "finalized",
            1,
            SolanaCoverageTier::AllTransactions,
        )
        .expect("context")
        .with_lineage("yellowstone-a", [4; 16], [5; 32], 3_000)
        .expect("lineage"),
    )
    .expect("Solana facts");
    let fact_bytes = serde_json::to_vec(&(
        facts.transactions,
        facts.instructions,
        facts.logs,
        facts.balance_changes,
        facts.token_balance_changes,
    ))
    .expect("Solana facts JSON");

    let account = Pubkey::new_from_array([7; 32]);
    let mut state = SolanaCanonicality::new();
    state.observe_slot(fork.clone(), None).expect("slot");
    state.activate(&fork).expect("activate");
    state
        .record_transaction(transaction.key().clone())
        .expect("transaction");
    state
        .record_account_write(
            AccountWrite::try_new(
                fork.clone(),
                account,
                Pubkey::new_from_array([8; 32]),
                Lamports::new(42),
                vec![0xaa],
                false,
                0,
                1,
            )
            .expect("account write"),
        )
        .expect("record account");
    state
        .observe_commitment(&fork, Commitment::Finalized)
        .expect("finalized");
    ReplayDigest {
        facts: digest(&fact_bytes),
        state: state.state_hash(),
    }
}

#[derive(Deserialize)]
struct SolanaFixture {
    slot: u64,
    blockhash: String,
    signature: String,
    version: u8,
    account_keys: Vec<String>,
    instructions: Vec<SolanaFixtureInstruction>,
    logs: Vec<String>,
    pre_balances: Vec<String>,
    post_balances: Vec<String>,
    fee: String,
    compute_units_consumed: u64,
    execution_status: String,
}

#[derive(Deserialize)]
struct SolanaFixtureInstruction {
    program_id_index: u8,
    accounts: Vec<u8>,
    data_base58: String,
}

impl SolanaFixture {
    fn into_transaction(self, fork: ForkId) -> SolanaTransaction {
        let message = SolanaMessage::try_new(
            match self.version {
                0 => MessageVersion::V0,
                other => panic!("unexpected Solana message version {other}"),
            },
            self.account_keys
                .iter()
                .map(|key| Pubkey::from_str(key).expect("pubkey"))
                .collect(),
            Vec::new(),
            self.instructions
                .iter()
                .map(|instruction| {
                    CompiledInstruction::try_new(
                        instruction.program_id_index,
                        instruction.accounts.clone(),
                        bs58::decode(&instruction.data_base58)
                            .into_vec()
                            .expect("instruction bytes"),
                    )
                    .expect("instruction")
                })
                .collect(),
        )
        .expect("message");
        SolanaTransaction::try_new(
            TransactionKey::new(
                Signature::from_str(&self.signature).expect("signature"),
                fork,
            ),
            message,
            Vec::new(),
            self.logs,
            decimal_lamports(&self.pre_balances),
            decimal_lamports(&self.post_balances),
            Vec::new(),
            serde_json::from_str(&format!("\"{}\"", self.fee)).expect("fee"),
            Some(self.compute_units_consumed),
            match self.execution_status.as_str() {
                "succeeded" => ExecutionStatus::Succeeded,
                other => panic!("unexpected Solana execution status {other}"),
            },
            Vec::new(),
        )
        .expect("transaction")
    }
}

fn decimal_lamports(amounts: &[String]) -> Vec<Lamports> {
    amounts
        .iter()
        .map(|amount| serde_json::from_str(&format!("\"{amount}\"")).expect("decimal lamports"))
        .collect()
}

fn block(number: u64, hash_byte: u8, parent_byte: u8) -> ExecutionBlock {
    ExecutionBlock::new(
        number,
        B256::from([hash_byte; 32]),
        B256::from([parent_byte; 32]),
    )
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn regtest_genesis() -> bitcoin_domain::BitcoinBlock {
    let block = bitcoin::blockdata::constants::genesis_block(Network::Regtest);
    parse_block(&serialize(&block)).expect("regtest genesis")
}

fn mine_regtest_child(
    parent: &bitcoin_domain::BitcoinBlock,
    marker: u8,
) -> bitcoin_domain::BitcoinBlock {
    let parent_native: Block =
        deserialize(parent.consensus_bytes()).expect("validated parent serialization");
    let coinbase = Transaction {
        version: transaction::Version::ONE,
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::from_bytes(vec![1, marker]),
            sequence: Sequence::MAX,
            witness: Witness::default(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(5_000_000_000),
            script_pubkey: ScriptBuf::new(),
        }],
    };
    let mut block = Block {
        header: Header {
            version: Version::ONE,
            prev_blockhash: NativeBlockHash::from_byte_array(*parent.block_hash().as_bytes()),
            merkle_root: TxMerkleNode::all_zeros(),
            time: parent_native.header.time.saturating_add(1),
            bits: CompactTarget::from_consensus(0x207f_ffff),
            nonce: 0,
        },
        txdata: vec![coinbase],
    };
    block.header.merkle_root = block.compute_merkle_root().expect("coinbase");
    let target = block.header.target();
    while !target.is_met_by(block.block_hash()) {
        block.header.nonce = block.header.nonce.wrapping_add(1);
    }
    parse_block(&serialize(&block)).expect("mined child")
}

fn fixture(relative: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(relative),
    )
    .expect("fixture")
}
