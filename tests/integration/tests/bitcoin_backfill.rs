use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use bitcoin_canonicality::{
    BackfillBlock, BackfillCheckpoint, BackfillCoordinator, BackfillError, BackfillRequest,
    BackfillSink,
};
use bitcoin_core_connector::{
    error::RpcError,
    rpc::{BitcoinRpc, ChainTip, MempoolSnapshot},
};
use bitcoin_domain::{BlockHash, Txid, parse_block};
use query_api::BitcoinRpcBackfillSource;

type ArchivedBlocks = Arc<Mutex<Vec<(u32, BlockHash, Vec<u8>)>>>;

struct RecordedRpc {
    heights: HashMap<u32, BlockHash>,
    blocks: HashMap<BlockHash, Vec<u8>>,
    best: BlockHash,
}

#[async_trait]
impl BitcoinRpc for RecordedRpc {
    async fn get_raw_mempool_with_sequence(&self) -> Result<MempoolSnapshot, RpcError> {
        unreachable!("not used by backfill")
    }

    async fn get_raw_transaction(&self, _txid: Txid) -> Result<Option<Vec<u8>>, RpcError> {
        unreachable!("not used by backfill")
    }

    async fn get_block_hash(&self, height: u32) -> Result<BlockHash, RpcError> {
        self.heights
            .get(&height)
            .copied()
            .ok_or_else(|| RpcError::InvalidResult {
                method: "getblockhash",
                message: "height absent from recorded source".to_owned(),
            })
    }

    async fn get_block(&self, hash: BlockHash) -> Result<Vec<u8>, RpcError> {
        self.blocks
            .get(&hash)
            .cloned()
            .ok_or_else(|| RpcError::InvalidResult {
                method: "getblock",
                message: "hash absent from recorded source".to_owned(),
            })
    }

    async fn get_best_block_hash(&self) -> Result<BlockHash, RpcError> {
        Ok(self.best)
    }

    async fn get_chain_tips(&self) -> Result<Vec<ChainTip>, RpcError> {
        unreachable!("not used by backfill")
    }
}

#[derive(Clone, Default)]
struct RecordedSink {
    archived: ArchivedBlocks,
    materialized: Arc<Mutex<Vec<(u32, BlockHash)>>>,
    checkpoints: Arc<Mutex<Vec<BackfillCheckpoint>>>,
}

#[async_trait]
impl BackfillSink for RecordedSink {
    async fn archive(&self, block: &BackfillBlock) -> Result<(), BackfillError> {
        assert_eq!(block.source_id(), "recorded-observer");
        self.archived.lock().expect("archive lock").push((
            block.height(),
            block.hash(),
            block.raw_block().to_vec(),
        ));
        Ok(())
    }

    async fn materialize(&self, block: &BackfillBlock) -> Result<(), BackfillError> {
        self.materialized
            .lock()
            .expect("materialize lock")
            .push((block.height(), block.parsed().block_hash()));
        Ok(())
    }

    async fn persist_checkpoint(
        &self,
        checkpoint: &BackfillCheckpoint,
    ) -> Result<(), BackfillError> {
        self.checkpoints
            .lock()
            .expect("checkpoint lock")
            .push(checkpoint.clone());
        Ok(())
    }
}

#[tokio::test]
async fn recorded_rpc_range_flows_raw_first_through_backfill_adapter() {
    let first_bytes = fixture("reorg_main_1.hex");
    let second_bytes = fixture("reorg_main_2.hex");
    let first = parse_block(&first_bytes).expect("first block");
    let second = parse_block(&second_bytes).expect("second block");
    assert_eq!(second.previous_block_hash(), first.block_hash());
    let rpc = RecordedRpc {
        heights: HashMap::from([(1, first.block_hash()), (2, second.block_hash())]),
        blocks: HashMap::from([
            (first.block_hash(), first_bytes.clone()),
            (second.block_hash(), second_bytes.clone()),
        ]),
        best: second.block_hash(),
    };
    let source = BitcoinRpcBackfillSource::new(rpc);
    let sink = RecordedSink::default();
    let request = BackfillRequest::new("recorded-observer", 1, 2, 2).expect("request");
    let coordinator = BackfillCoordinator::new(&source, &sink);
    let mut checkpoint = coordinator.start(&request).await.expect("start");

    let report = coordinator
        .run(&request, &mut checkpoint)
        .await
        .expect("backfill");

    assert_eq!(report.processed_blocks(), 2);
    assert!(!report.tip_changed());
    assert_eq!(
        *sink.archived.lock().expect("archive lock"),
        vec![
            (1, first.block_hash(), first_bytes),
            (2, second.block_hash(), second_bytes),
        ]
    );
    assert_eq!(
        *sink.materialized.lock().expect("materialize lock"),
        vec![(1, first.block_hash()), (2, second.block_hash())]
    );
    assert_eq!(checkpoint.last_archived_height(), Some(2));
    assert_eq!(checkpoint.last_materialized_height(), Some(2));
    assert_eq!(sink.checkpoints.lock().expect("checkpoint lock").len(), 4);
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
