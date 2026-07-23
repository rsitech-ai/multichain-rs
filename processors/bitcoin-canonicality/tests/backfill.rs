mod common;

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use bitcoin_canonicality::{
    BackfillBlock, BackfillCoordinator, BackfillError, BackfillRequest, BackfillSink,
    BackfillSource,
};
use bitcoin_domain::{BitcoinBlock, BlockHash};
use common::{coinbase, mine_block, regtest_genesis};

#[tokio::test]
async fn range_is_fetched_concurrently_but_committed_in_height_order() {
    let source = FakeSource::new(chain(8));
    let sink = FakeSink::default();
    let request = backfill_request(0, 7, 3);
    let coordinator = BackfillCoordinator::new(&source, &sink);
    let mut checkpoint = coordinator.start(&request).await.expect("start");

    let report = coordinator
        .run(&request, &mut checkpoint)
        .await
        .expect("backfill");

    assert_eq!(sink.archived(), (0..=7).collect::<Vec<_>>());
    assert_eq!(sink.materialized(), (0..=7).collect::<Vec<_>>());
    assert_eq!(source.maximum_active(), 3);
    assert_eq!(report.processed_blocks(), 8);
    assert!(!report.tip_changed());
    assert_eq!(checkpoint.last_archived_height(), Some(7));
    assert_eq!(checkpoint.last_materialized_height(), Some(7));
    assert_eq!(
        coordinator
            .run(&request, &mut checkpoint)
            .await
            .expect("completed resume")
            .processed_blocks(),
        0
    );
}

#[tokio::test]
async fn restart_resumes_after_last_materialized_height_without_logical_duplicates() {
    let source = FakeSource::new(chain(6));
    let sink = FakeSink::failing_once_at(3);
    let request = backfill_request(0, 5, 2);
    let coordinator = BackfillCoordinator::new(&source, &sink);
    let mut checkpoint = coordinator.start(&request).await.expect("start");

    assert!(matches!(
        coordinator.run(&request, &mut checkpoint).await,
        Err(BackfillError::Sink {
            stage: "materialize",
            ..
        })
    ));
    assert_eq!(checkpoint.last_archived_height(), Some(3));
    assert_eq!(checkpoint.last_materialized_height(), Some(2));

    let report = coordinator
        .run(&request, &mut checkpoint)
        .await
        .expect("resume");
    assert_eq!(report.processed_blocks(), 3);
    assert_eq!(sink.unique_archived(), (0..=5).collect::<BTreeSet<_>>());
    assert_eq!(sink.unique_materialized(), (0..=5).collect::<BTreeSet<_>>());
    assert_eq!(checkpoint.last_materialized_height(), Some(5));
}

#[tokio::test]
async fn request_mismatch_invalid_range_and_tip_change_are_explicit() {
    let source = FakeSource::new(chain(3)).with_changed_tip();
    let sink = FakeSink::default();
    let request = backfill_request(0, 2, 2);
    let coordinator = BackfillCoordinator::new(&source, &sink);
    let mut checkpoint = coordinator.start(&request).await.expect("start");
    let report = coordinator
        .run(&request, &mut checkpoint)
        .await
        .expect("backfill");
    assert!(report.tip_changed());
    assert_ne!(report.tip_at_start(), report.tip_at_end());

    let mismatched = backfill_request(1, 2, 2);
    assert!(matches!(
        coordinator.run(&mismatched, &mut checkpoint).await,
        Err(BackfillError::CheckpointRequestMismatch)
    ));
    assert!(matches!(
        BackfillRequest::new("observer-a", 3, 2, 1),
        Err(BackfillError::InvalidRange { .. })
    ));
    assert!(matches!(
        BackfillRequest::new("observer-a", 0, 2, 0),
        Err(BackfillError::InvalidConcurrency { .. })
    ));
}

fn backfill_request(start: u32, end: u32, max_in_flight: usize) -> BackfillRequest {
    BackfillRequest::new("observer-a", start, end, max_in_flight).expect("request")
}

fn chain(length: u32) -> Vec<BitcoinBlock> {
    let mut blocks = vec![regtest_genesis()];
    for height in 1..length {
        let block = mine_block(
            blocks.last().expect("genesis"),
            height,
            vec![coinbase(u8::try_from(height).expect("small fixture"), 1)],
        );
        blocks.push(block);
    }
    blocks
}

struct FakeSource {
    blocks: BTreeMap<u32, BitcoinBlock>,
    active: AtomicUsize,
    maximum_active: AtomicUsize,
    best_calls: AtomicUsize,
    changed_tip: bool,
}

impl FakeSource {
    fn new(blocks: Vec<BitcoinBlock>) -> Self {
        Self {
            blocks: blocks
                .into_iter()
                .enumerate()
                .map(|(height, block)| (u32::try_from(height).expect("fixture height"), block))
                .collect(),
            active: AtomicUsize::new(0),
            maximum_active: AtomicUsize::new(0),
            best_calls: AtomicUsize::new(0),
            changed_tip: false,
        }
    }

    fn with_changed_tip(mut self) -> Self {
        self.changed_tip = true;
        self
    }

    fn maximum_active(&self) -> usize {
        self.maximum_active.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl BackfillSource for FakeSource {
    async fn best_block_hash(&self) -> Result<BlockHash, BackfillError> {
        let call = self.best_calls.fetch_add(1, Ordering::SeqCst);
        if self.changed_tip && call > 0 {
            return Ok(BlockHash::from_bytes([0x55; 32]));
        }
        Ok(self
            .blocks
            .last_key_value()
            .expect("non-empty fixture")
            .1
            .block_hash())
    }

    async fn block_hash(&self, height: u32) -> Result<BlockHash, BackfillError> {
        Ok(self
            .blocks
            .get(&height)
            .ok_or_else(|| BackfillError::Source("missing height".to_owned()))?
            .block_hash())
    }

    async fn raw_block(&self, hash: BlockHash) -> Result<Vec<u8>, BackfillError> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum_active.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(5)).await;
        let result = self
            .blocks
            .values()
            .find(|block| block.block_hash() == hash)
            .map(|block| block.consensus_bytes().to_vec())
            .ok_or_else(|| BackfillError::Source("missing block".to_owned()));
        self.active.fetch_sub(1, Ordering::SeqCst);
        result
    }
}

#[derive(Default)]
struct FakeSink {
    archived: Mutex<Vec<u32>>,
    materialized: Mutex<Vec<u32>>,
    checkpoints: Mutex<Vec<(Option<u32>, Option<u32>)>>,
    fail_once_at: Mutex<Option<u32>>,
}

impl FakeSink {
    fn failing_once_at(height: u32) -> Self {
        Self {
            fail_once_at: Mutex::new(Some(height)),
            ..Self::default()
        }
    }

    fn archived(&self) -> Vec<u32> {
        self.archived.lock().expect("archive lock").clone()
    }

    fn materialized(&self) -> Vec<u32> {
        self.materialized.lock().expect("materialize lock").clone()
    }

    fn unique_archived(&self) -> BTreeSet<u32> {
        self.archived().into_iter().collect()
    }

    fn unique_materialized(&self) -> BTreeSet<u32> {
        self.materialized().into_iter().collect()
    }
}

#[async_trait]
impl BackfillSink for FakeSink {
    async fn archive(&self, block: &BackfillBlock) -> Result<(), BackfillError> {
        assert_eq!(block.source_id(), "observer-a");
        self.archived
            .lock()
            .expect("archive lock")
            .push(block.height());
        Ok(())
    }

    async fn materialize(&self, block: &BackfillBlock) -> Result<(), BackfillError> {
        let mut fail_once = self.fail_once_at.lock().expect("failure lock");
        if *fail_once == Some(block.height()) {
            *fail_once = None;
            return Err(BackfillError::sink("materialize", "injected failure"));
        }
        drop(fail_once);
        self.materialized
            .lock()
            .expect("materialize lock")
            .push(block.height());
        Ok(())
    }

    async fn persist_checkpoint(
        &self,
        checkpoint: &bitcoin_canonicality::BackfillCheckpoint,
    ) -> Result<(), BackfillError> {
        self.checkpoints.lock().expect("checkpoint lock").push((
            checkpoint.last_archived_height(),
            checkpoint.last_materialized_height(),
        ));
        Ok(())
    }
}
