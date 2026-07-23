#![allow(dead_code)]

use bitcoin::{
    Amount, Block, BlockHash, CompactTarget, Network, OutPoint, ScriptBuf, Sequence, Transaction,
    TxIn, TxMerkleNode, TxOut, Witness, absolute,
    block::{Header, Version},
    consensus::{deserialize, serialize},
    hashes::Hash as _,
    transaction,
};
use bitcoin_domain::{BitcoinBlock, parse_block};

pub fn regtest_genesis() -> BitcoinBlock {
    let block = bitcoin::blockdata::constants::genesis_block(Network::Regtest);
    parse_block(&serialize(&block)).expect("valid regtest genesis")
}

pub fn mine_child(parent: &BitcoinBlock, marker: u32) -> BitcoinBlock {
    let parent_native: Block =
        deserialize(parent.consensus_bytes()).expect("validated parent serialization");
    mine_block(parent, marker, parent_native.txdata)
}

pub fn mine_block(
    parent: &BitcoinBlock,
    marker: u32,
    transactions: Vec<Transaction>,
) -> BitcoinBlock {
    let parent_native: Block =
        deserialize(parent.consensus_bytes()).expect("validated parent serialization");
    let mut block = Block {
        header: Header {
            version: Version::ONE,
            prev_blockhash: native_hash(parent.block_hash()),
            merkle_root: TxMerkleNode::all_zeros(),
            time: parent_native.header.time.saturating_add(marker.max(1)),
            bits: CompactTarget::from_consensus(0x207f_ffff),
            nonce: 0,
        },
        txdata: transactions,
    };
    block.header.merkle_root = block.compute_merkle_root().expect("coinbase transaction");
    let target = block.header.target();
    while !target.is_met_by(block.block_hash()) {
        block.header.nonce = block.header.nonce.wrapping_add(1);
    }
    parse_block(&serialize(&block)).expect("valid mined child")
}

pub fn coinbase(marker: u8, value_sats: u64) -> Transaction {
    Transaction {
        version: transaction::Version::ONE,
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::from_bytes(vec![1, marker]),
            sequence: Sequence::MAX,
            witness: Witness::default(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(value_sats),
            script_pubkey: ScriptBuf::new(),
        }],
    }
}

pub fn spend(previous_output: OutPoint, outputs: &[u64]) -> Transaction {
    Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output,
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::default(),
        }],
        output: outputs
            .iter()
            .copied()
            .map(|value| TxOut {
                value: Amount::from_sat(value),
                script_pubkey: ScriptBuf::new(),
            })
            .collect(),
    }
}

pub fn invalidate_pow(block: &BitcoinBlock) -> BitcoinBlock {
    let mut native: Block =
        deserialize(block.consensus_bytes()).expect("validated block serialization");
    let target = native.header.target();
    loop {
        native.header.nonce = native.header.nonce.wrapping_add(1);
        if !target.is_met_by(native.block_hash()) {
            return parse_block(&serialize(&native)).expect("merkle-valid invalid-PoW block");
        }
    }
}

fn native_hash(hash: bitcoin_domain::BlockHash) -> BlockHash {
    BlockHash::from_byte_array(*hash.as_bytes())
}
