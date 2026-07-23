use async_trait::async_trait;
use bitcoin_domain::{BitcoinBlock, BlockHash, parse_block};
use futures_util::{StreamExt as _, stream};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

const MAX_IN_FLIGHT: usize = 256;

/// Validated historical replay request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackfillRequest {
    source_id: String,
    start_height: u32,
    end_height_inclusive: u32,
    max_in_flight: usize,
}

impl BackfillRequest {
    /// Validates a bounded inclusive height range.
    ///
    /// # Errors
    ///
    /// Rejects blank source identities, reversed ranges, and concurrency
    /// outside `1..=256`.
    pub fn new(
        source_id: impl Into<String>,
        start_height: u32,
        end_height_inclusive: u32,
        max_in_flight: usize,
    ) -> Result<Self, BackfillError> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() {
            return Err(BackfillError::EmptySourceId);
        }
        if start_height > end_height_inclusive {
            return Err(BackfillError::InvalidRange {
                start: start_height,
                end: end_height_inclusive,
            });
        }
        if !(1..=MAX_IN_FLIGHT).contains(&max_in_flight) {
            return Err(BackfillError::InvalidConcurrency {
                value: max_in_flight,
                maximum: MAX_IN_FLIGHT,
            });
        }
        Ok(Self {
            source_id,
            start_height,
            end_height_inclusive,
            max_in_flight,
        })
    }

    /// Returns the stable source identity.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Returns the first requested height.
    #[must_use]
    pub const fn start_height(&self) -> u32 {
        self.start_height
    }

    /// Returns the final requested height.
    #[must_use]
    pub const fn end_height_inclusive(&self) -> u32 {
        self.end_height_inclusive
    }

    /// Returns the maximum concurrent source reads.
    #[must_use]
    pub const fn max_in_flight(&self) -> usize {
        self.max_in_flight
    }

    /// Returns a deterministic identity for resume validation.
    #[must_use]
    pub fn request_hash(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(b"multichain.bitcoin.backfill.request.v1");
        digest.update(
            u64::try_from(self.source_id.len())
                .unwrap_or(u64::MAX)
                .to_le_bytes(),
        );
        digest.update(self.source_id.as_bytes());
        digest.update(self.start_height.to_le_bytes());
        digest.update(self.end_height_inclusive.to_le_bytes());
        digest.update(
            u64::try_from(self.max_in_flight)
                .unwrap_or(u64::MAX)
                .to_le_bytes(),
        );
        digest.finalize().into()
    }
}

/// Source progress bound to one immutable replay request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackfillCheckpoint {
    request_hash: [u8; 32],
    last_archived_height: Option<u32>,
    last_materialized_height: Option<u32>,
    canonical_tip_hash_at_start: BlockHash,
}

impl BackfillCheckpoint {
    /// Returns the request identity this checkpoint may resume.
    #[must_use]
    pub const fn request_hash(&self) -> [u8; 32] {
        self.request_hash
    }

    /// Returns the last height durably covered by raw archive.
    #[must_use]
    pub const fn last_archived_height(&self) -> Option<u32> {
        self.last_archived_height
    }

    /// Returns the last height durably covered by materialized facts.
    #[must_use]
    pub const fn last_materialized_height(&self) -> Option<u32> {
        self.last_materialized_height
    }

    /// Returns the source tip captured when this replay began.
    #[must_use]
    pub const fn canonical_tip_hash_at_start(&self) -> BlockHash {
        self.canonical_tip_hash_at_start
    }
}

/// Exact raw block fetched for one source-resolved height.
#[derive(Clone, Debug)]
pub struct BackfillBlock {
    source_id: String,
    height: u32,
    hash: BlockHash,
    raw_block: Vec<u8>,
    parsed: BitcoinBlock,
}

impl BackfillBlock {
    /// Returns the observer identity that produced the RPC recovery record.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Returns the source-resolved height.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the hash used to fetch the raw block.
    #[must_use]
    pub const fn hash(&self) -> BlockHash {
        self.hash
    }

    /// Returns the exact RPC block bytes.
    #[must_use]
    pub fn raw_block(&self) -> &[u8] {
        &self.raw_block
    }

    /// Returns the validated native block.
    #[must_use]
    pub const fn parsed(&self) -> &BitcoinBlock {
        &self.parsed
    }
}

/// Completed pass evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackfillReport {
    processed_blocks: u64,
    tip_at_start: BlockHash,
    tip_at_end: BlockHash,
}

impl BackfillReport {
    /// Returns blocks archived and materialized during this pass.
    #[must_use]
    pub const fn processed_blocks(self) -> u64 {
        self.processed_blocks
    }

    /// Returns whether the source tip changed during this pass.
    #[must_use]
    pub fn tip_changed(self) -> bool {
        self.tip_at_start != self.tip_at_end
    }

    /// Returns the source tip captured when the replay job began.
    #[must_use]
    pub const fn tip_at_start(self) -> BlockHash {
        self.tip_at_start
    }

    /// Returns the source tip observed after this pass.
    #[must_use]
    pub const fn tip_at_end(self) -> BlockHash {
        self.tip_at_end
    }
}

/// Historical Bitcoin block source.
#[async_trait]
pub trait BackfillSource: Sync {
    /// Returns the source's current canonical tip hash.
    async fn best_block_hash(&self) -> Result<BlockHash, BackfillError>;

    /// Resolves one height to the source's current canonical block hash.
    async fn block_hash(&self, height: u32) -> Result<BlockHash, BackfillError>;

    /// Fetches exact serialized block bytes by hash.
    async fn raw_block(&self, hash: BlockHash) -> Result<Vec<u8>, BackfillError>;
}

/// Raw archive, materialization, and checkpoint transaction boundary.
#[async_trait]
pub trait BackfillSink: Sync {
    /// Idempotently archives one exact source observation.
    async fn archive(&self, block: &BackfillBlock) -> Result<(), BackfillError>;

    /// Idempotently materializes facts from the archived observation path.
    async fn materialize(&self, block: &BackfillBlock) -> Result<(), BackfillError>;

    /// Atomically persists the complete checkpoint value.
    async fn persist_checkpoint(
        &self,
        checkpoint: &BackfillCheckpoint,
    ) -> Result<(), BackfillError>;
}

/// Bounded, stable-order raw-first replay coordinator.
pub struct BackfillCoordinator<'a, Source, Sink> {
    source: &'a Source,
    sink: &'a Sink,
}

impl<'a, Source, Sink> BackfillCoordinator<'a, Source, Sink>
where
    Source: BackfillSource,
    Sink: BackfillSink,
{
    /// Borrows one source and one durable sink.
    #[must_use]
    pub const fn new(source: &'a Source, sink: &'a Sink) -> Self {
        Self { source, sink }
    }

    /// Captures the initial tip and creates an empty replay checkpoint.
    ///
    /// # Errors
    ///
    /// Returns a source failure when the initial tip cannot be observed.
    pub async fn start(
        &self,
        request: &BackfillRequest,
    ) -> Result<BackfillCheckpoint, BackfillError> {
        Ok(BackfillCheckpoint {
            request_hash: request.request_hash(),
            last_archived_height: None,
            last_materialized_height: None,
            canonical_tip_hash_at_start: self.source.best_block_hash().await?,
        })
    }

    /// Fetches concurrently and commits archive/materialization in stable order.
    ///
    /// The passed checkpoint is advanced only after its complete next value is
    /// durably persisted by the sink.
    ///
    /// # Errors
    ///
    /// Rejects checkpoint/request mismatch, invalid source bytes, source
    /// failures, and sink stage failures.
    pub async fn run(
        &self,
        request: &BackfillRequest,
        checkpoint: &mut BackfillCheckpoint,
    ) -> Result<BackfillReport, BackfillError> {
        if checkpoint.request_hash != request.request_hash() {
            return Err(BackfillError::CheckpointRequestMismatch);
        }
        let first_height = match checkpoint.last_materialized_height {
            Some(height) if height >= request.end_height_inclusive => None,
            Some(height) => Some(height + 1),
            None => Some(request.start_height),
        };
        let heights = first_height
            .into_iter()
            .flat_map(|height| height..=request.end_height_inclusive);
        let fetches = stream::iter(heights)
            .map(|height| self.fetch_block(request.source_id(), height))
            .buffered(request.max_in_flight);
        futures_util::pin_mut!(fetches);

        let mut processed_blocks = 0_u64;
        while let Some(block) = fetches.next().await {
            let block = block?;
            if checkpoint
                .last_archived_height
                .is_none_or(|height| height < block.height)
            {
                self.sink
                    .archive(&block)
                    .await
                    .map_err(|error| error.with_stage("archive"))?;
                let mut next = checkpoint.clone();
                next.last_archived_height = Some(block.height);
                self.sink
                    .persist_checkpoint(&next)
                    .await
                    .map_err(|error| error.with_stage("checkpoint"))?;
                *checkpoint = next;
            }

            self.sink
                .materialize(&block)
                .await
                .map_err(|error| error.with_stage("materialize"))?;
            let mut next = checkpoint.clone();
            next.last_materialized_height = Some(block.height);
            self.sink
                .persist_checkpoint(&next)
                .await
                .map_err(|error| error.with_stage("checkpoint"))?;
            *checkpoint = next;
            processed_blocks = processed_blocks.saturating_add(1);
        }

        let tip_at_end = self.source.best_block_hash().await?;
        Ok(BackfillReport {
            processed_blocks,
            tip_at_start: checkpoint.canonical_tip_hash_at_start,
            tip_at_end,
        })
    }

    async fn fetch_block(
        &self,
        source_id: &str,
        height: u32,
    ) -> Result<BackfillBlock, BackfillError> {
        let hash = self.source.block_hash(height).await?;
        let raw_block = self.source.raw_block(hash).await?;
        let parsed = parse_block(&raw_block).map_err(|error| BackfillError::InvalidBlock {
            height,
            message: error.to_string(),
        })?;
        let actual = parsed.block_hash();
        if actual != hash {
            return Err(BackfillError::BlockHashMismatch {
                height,
                expected: hash,
                actual,
            });
        }
        Ok(BackfillBlock {
            source_id: source_id.to_owned(),
            height,
            hash,
            raw_block,
            parsed,
        })
    }
}

/// Historical replay validation or durability failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum BackfillError {
    /// Source identity was blank.
    #[error("backfill source_id must not be empty")]
    EmptySourceId,
    /// Inclusive height range was reversed.
    #[error("invalid backfill range {start}..={end}")]
    InvalidRange {
        /// Requested first height.
        start: u32,
        /// Requested final height.
        end: u32,
    },
    /// Requested concurrency was zero or exceeded the hard bound.
    #[error("invalid max_in_flight {value}; expected 1..={maximum}")]
    InvalidConcurrency {
        /// Rejected value.
        value: usize,
        /// Hard upper bound.
        maximum: usize,
    },
    /// The checkpoint belongs to a different immutable request.
    #[error("checkpoint request hash does not match the replay request")]
    CheckpointRequestMismatch,
    /// Source lookup or transport failed.
    #[error("backfill source failed: {0}")]
    Source(String),
    /// A durable output stage failed.
    #[error("backfill {stage} stage failed: {message}")]
    Sink {
        /// Stable stage name.
        stage: &'static str,
        /// Redacted failure description.
        message: String,
    },
    /// Exact source bytes failed native parsing.
    #[error("backfill block at height {height} is invalid: {message}")]
    InvalidBlock {
        /// Source-resolved height.
        height: u32,
        /// Native parser failure.
        message: String,
    },
    /// Raw bytes did not match the hash used for retrieval.
    #[error("backfill block hash mismatch at height {height}: expected {expected}, got {actual}")]
    BlockHashMismatch {
        /// Source-resolved height.
        height: u32,
        /// Hash returned for the height.
        expected: BlockHash,
        /// Hash parsed from exact bytes.
        actual: BlockHash,
    },
}

impl BackfillError {
    /// Constructs a redacted sink-stage error.
    #[must_use]
    pub fn sink(stage: &'static str, message: impl Into<String>) -> Self {
        Self::Sink {
            stage,
            message: message.into(),
        }
    }

    fn with_stage(self, stage: &'static str) -> Self {
        match self {
            Self::Sink { message, .. } => Self::Sink { stage, message },
            other => Self::Sink {
                stage,
                message: other.to_string(),
            },
        }
    }
}
