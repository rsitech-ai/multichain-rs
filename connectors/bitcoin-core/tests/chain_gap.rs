use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use bitcoin_core_connector::{
    error::RpcError,
    reconcile::recover_blocks_to_ancestor,
    rpc::{BitcoinRpc, ChainTip, MempoolSnapshot},
};
use bitcoin_domain::{BlockHash, Txid, parse_block};

struct BlockRpc {
    best: BlockHash,
    blocks: HashMap<BlockHash, Vec<u8>>,
}

#[async_trait]
impl BitcoinRpc for BlockRpc {
    async fn get_raw_mempool_with_sequence(&self) -> Result<MempoolSnapshot, RpcError> {
        unreachable!("not used")
    }

    async fn get_raw_transaction(&self, _txid: Txid) -> Result<Option<Vec<u8>>, RpcError> {
        unreachable!("not used")
    }

    async fn get_block_hash(&self, _height: u32) -> Result<BlockHash, RpcError> {
        unreachable!("not used")
    }

    async fn get_block(&self, hash: BlockHash) -> Result<Vec<u8>, RpcError> {
        self.blocks
            .get(&hash)
            .cloned()
            .ok_or_else(|| RpcError::InvalidResult {
                method: "getblock",
                message: "unknown test block".to_owned(),
            })
    }

    async fn get_best_block_hash(&self) -> Result<BlockHash, RpcError> {
        Ok(self.best)
    }

    async fn get_chain_tips(&self) -> Result<Vec<ChainTip>, RpcError> {
        unreachable!("not used")
    }
}

#[tokio::test]
async fn block_recovery_walks_parents_to_last_known_ancestor() {
    let first = fixture("reorg_main_1.hex");
    let second = fixture("reorg_main_2.hex");
    let first_block = parse_block(&first).expect("first block");
    let second_block = parse_block(&second).expect("second block");
    assert_eq!(second_block.previous_block_hash(), first_block.block_hash());
    let rpc = BlockRpc {
        best: second_block.block_hash(),
        blocks: HashMap::from([(second_block.block_hash(), second.clone())]),
    };
    let recovered = recover_blocks_to_ancestor(&rpc, &HashSet::from([first_block.block_hash()]))
        .await
        .expect("recover");
    assert_eq!(recovered, vec![second]);
}

fn fixture(name: &str) -> Vec<u8> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/bitcoin/objects")
        .join(name);
    let text = std::fs::read_to_string(root).expect("fixture");
    (0..text.trim().len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex"))
        .collect()
}
