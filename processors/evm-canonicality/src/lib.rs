#![doc = "Chain-native Ethereum and BSC canonicality/finality state machines."]

use std::collections::{BTreeMap, HashMap};

use evm_domain::{B256, BscStatus, EthereumStatus};
use thiserror::Error;

/// Minimal execution block identity needed for canonicality.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ExecutionBlock {
    /// Execution block number.
    pub number: u64,
    /// Execution payload/block hash.
    pub hash: B256,
    /// Parent execution payload hash.
    pub parent_hash: B256,
}

impl ExecutionBlock {
    /// Constructs an execution identity.
    #[must_use]
    pub const fn new(number: u64, hash: B256, parent_hash: B256) -> Self {
        Self {
            number,
            hash,
            parent_hash,
        }
    }
}

/// Consensus-client checkpoint evidence joined by execution payload hash.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EthereumCheckpoint {
    head: B256,
    safe: B256,
    finalized: B256,
    source_id: String,
}

impl EthereumCheckpoint {
    /// Creates checkpoint evidence from one exact consensus source.
    ///
    /// # Errors
    ///
    /// Rejects a blank source identity.
    pub fn new(
        head: B256,
        safe: B256,
        finalized: B256,
        source_id: impl Into<String>,
    ) -> Result<Self, EthereumError> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() {
            return Err(EthereumError::EmptySourceId);
        }
        Ok(Self {
            head,
            safe,
            finalized,
            source_id,
        })
    }

    /// Exact consensus source identity.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }
}

/// One append-only Ethereum status revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EthereumRevision {
    /// Exact execution block.
    pub block: ExecutionBlock,
    /// Chain-native state.
    pub status: EthereumStatus,
    revision: u64,
    /// Consensus source for safe/finalized evidence.
    pub source_id: Option<String>,
}

impl EthereumRevision {
    /// Monotonic tracker revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

/// Ethereum execution ancestry joined to consensus checkpoint evidence.
#[derive(Clone, Debug, Default)]
pub struct EthereumCanonicality {
    blocks: HashMap<B256, ExecutionBlock>,
    canonical: BTreeMap<u64, B256>,
    head: Option<B256>,
    safe: Option<B256>,
    finalized: Option<B256>,
    revision: u64,
}

impl EthereumCanonicality {
    /// Creates an empty tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies a Reth committed segment in ancestor-to-descendant order.
    ///
    /// # Errors
    ///
    /// Rejects discontinuous input and segments that do not extend the
    /// current canonical head. Exact canonical replay is a no-op.
    pub fn commit_segment<I>(&mut self, blocks: I) -> Result<Vec<EthereumRevision>, EthereumError>
    where
        I: IntoIterator<Item = ExecutionBlock>,
    {
        let blocks = blocks.into_iter().collect::<Vec<_>>();
        if blocks.is_empty() {
            return Ok(Vec::new());
        }
        validate_segment(&blocks)?;
        if blocks.iter().all(|block| {
            self.canonical
                .get(&block.number)
                .is_some_and(|hash| *hash == block.hash)
        }) {
            return Ok(Vec::new());
        }
        if let Some(head_hash) = self.head {
            let head = self.block(head_hash)?;
            let first = blocks[0];
            if first.number != head.number.saturating_add(1) || first.parent_hash != head.hash {
                return Err(EthereumError::SegmentDoesNotExtendHead);
            }
        }
        let mut revisions = Vec::with_capacity(blocks.len());
        for block in blocks {
            self.blocks.insert(block.hash, block);
            self.canonical.insert(block.number, block.hash);
            self.head = Some(block.hash);
            revisions.push(self.next_revision(block, EthereumStatus::CanonicalHead, None));
        }
        Ok(revisions)
    }

    /// Applies one Reth reorg atomically: old tip-down revisions precede new
    /// ancestor-up revisions.
    ///
    /// # Errors
    ///
    /// Rejects non-canonical removals, discontinuous replacements, or any
    /// attempt to reverse a finalized execution payload.
    pub fn reorg<IOld, INew>(
        &mut self,
        old: IOld,
        new: INew,
    ) -> Result<Vec<EthereumRevision>, EthereumError>
    where
        IOld: IntoIterator<Item = ExecutionBlock>,
        INew: IntoIterator<Item = ExecutionBlock>,
    {
        let old = old.into_iter().collect::<Vec<_>>();
        let new = new.into_iter().collect::<Vec<_>>();
        if old.is_empty() || new.is_empty() {
            return Err(EthereumError::EmptyReorgSegment);
        }
        validate_segment(&old)?;
        validate_segment(&new)?;
        if old[0].number != new[0].number || old[0].parent_hash != new[0].parent_hash {
            return Err(EthereumError::ReorgAncestorMismatch);
        }
        let current_head = self.head.ok_or(EthereumError::MissingCanonicalHead)?;
        if old.last().map(|block| block.hash) != Some(current_head)
            || old.iter().any(|block| {
                self.canonical
                    .get(&block.number)
                    .is_none_or(|hash| *hash != block.hash)
            })
        {
            return Err(EthereumError::ReorgDoesNotMatchCanonicalSuffix);
        }
        if let Some(finalized) = self.finalized
            && old.iter().any(|block| block.hash == finalized)
        {
            return Err(EthereumError::FinalizedReversalCritical { finalized });
        }
        if new.iter().any(|block| {
            self.canonical
                .get(&block.number)
                .is_some_and(|hash| *hash == block.hash)
        }) {
            return Err(EthereumError::ReplacementAlreadyCanonical);
        }

        let mut revisions = Vec::with_capacity(old.len() + new.len());
        for block in old.iter().rev().copied() {
            self.canonical.remove(&block.number);
            revisions.push(self.next_revision(block, EthereumStatus::Reorged, None));
        }
        for block in new.iter().copied() {
            self.blocks.insert(block.hash, block);
            self.canonical.insert(block.number, block.hash);
            revisions.push(self.next_revision(block, EthereumStatus::CanonicalHead, None));
        }
        self.head = new.last().map(|block| block.hash);
        if self.safe.is_some_and(|safe| {
            !self
                .canonical
                .values()
                .any(|canonical_hash| *canonical_hash == safe)
        }) {
            self.safe = None;
        }
        Ok(revisions)
    }

    /// Joins consensus head/safe/finalized payload hashes to canonical
    /// execution ancestry.
    ///
    /// # Errors
    ///
    /// Rejects unknown payloads, invalid ancestry, regressions, and any
    /// finalized checkpoint reversal.
    pub fn observe_checkpoint(
        &mut self,
        checkpoint: EthereumCheckpoint,
    ) -> Result<Vec<EthereumRevision>, EthereumError> {
        let head = self.canonical_block(checkpoint.head)?;
        let safe = self.canonical_block(checkpoint.safe)?;
        let finalized = self.canonical_block(checkpoint.finalized)?;
        if !self.is_ancestor(finalized.hash, safe.hash)
            || !self.is_ancestor(safe.hash, head.hash)
            || self.head != Some(head.hash)
        {
            return Err(EthereumError::InvalidCheckpointAncestry);
        }
        if let Some(previous) = self.finalized
            && previous != finalized.hash
            && (!self.is_ancestor(previous, finalized.hash)
                || self.block(previous)?.number >= finalized.number)
        {
            return Err(EthereumError::FinalizedReversalCritical {
                finalized: previous,
            });
        }
        if let Some(previous) = self.safe
            && previous != safe.hash
            && (!self.is_ancestor(previous, safe.hash)
                || self.block(previous)?.number >= safe.number)
        {
            return Err(EthereumError::SafeCheckpointRegression);
        }

        let mut revisions = Vec::with_capacity(2);
        if self.safe != Some(safe.hash) {
            self.safe = Some(safe.hash);
            revisions.push(self.next_revision(
                safe,
                EthereumStatus::Safe,
                Some(checkpoint.source_id.clone()),
            ));
        }
        if self.finalized != Some(finalized.hash) {
            self.finalized = Some(finalized.hash);
            revisions.push(self.next_revision(
                finalized,
                EthereumStatus::Finalized,
                Some(checkpoint.source_id),
            ));
        }
        Ok(revisions)
    }

    /// Current canonical execution head.
    #[must_use]
    pub const fn head(&self) -> Option<B256> {
        self.head
    }

    fn canonical_block(&self, hash: B256) -> Result<ExecutionBlock, EthereumError> {
        let block = self
            .blocks
            .get(&hash)
            .copied()
            .ok_or(EthereumError::UnknownExecutionPayload { hash })?;
        if self.canonical.get(&block.number) != Some(&hash) {
            return Err(EthereumError::ExecutionPayloadNotCanonical { hash });
        }
        Ok(block)
    }

    fn block(&self, hash: B256) -> Result<ExecutionBlock, EthereumError> {
        self.blocks
            .get(&hash)
            .copied()
            .ok_or(EthereumError::UnknownExecutionPayload { hash })
    }

    fn is_ancestor(&self, ancestor: B256, descendant: B256) -> bool {
        let Ok(ancestor_block) = self.block(ancestor) else {
            return false;
        };
        let Ok(mut current) = self.block(descendant) else {
            return false;
        };
        while current.number > ancestor_block.number {
            let Ok(parent) = self.block(current.parent_hash) else {
                return false;
            };
            current = parent;
        }
        current.hash == ancestor
    }

    fn next_revision(
        &mut self,
        block: ExecutionBlock,
        status: EthereumStatus,
        source_id: Option<String>,
    ) -> EthereumRevision {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("revision exhaustion is operationally unreachable");
        EthereumRevision {
            block,
            status,
            revision: self.revision,
            source_id,
        }
    }
}

/// Ethereum canonicality/finality invariant failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum EthereumError {
    /// Consensus source identity was blank.
    #[error("consensus source identity must not be empty")]
    EmptySourceId,
    /// Segment had a number or parent discontinuity.
    #[error("execution segment is not contiguous")]
    DiscontinuousSegment,
    /// Committed segment did not extend the current head.
    #[error("committed segment does not extend canonical execution head")]
    SegmentDoesNotExtendHead,
    /// Reorg lacked one of its required sides.
    #[error("reorg requires non-empty old and new segments")]
    EmptyReorgSegment,
    /// Old and replacement branches did not share the same parent/height.
    #[error("reorg branches do not share an ancestor boundary")]
    ReorgAncestorMismatch,
    /// No canonical head exists.
    #[error("canonical execution head is absent")]
    MissingCanonicalHead,
    /// Removed segment was not the exact canonical suffix.
    #[error("reorg removal does not match the canonical suffix")]
    ReorgDoesNotMatchCanonicalSuffix,
    /// A replacement block was already canonical.
    #[error("replacement segment contains an already canonical block")]
    ReplacementAlreadyCanonical,
    /// Consensus referenced an unknown execution payload hash.
    #[error("unknown execution payload {hash}")]
    UnknownExecutionPayload {
        /// Unknown hash.
        hash: B256,
    },
    /// Consensus referenced a known non-canonical payload.
    #[error("execution payload {hash} is not canonical")]
    ExecutionPayloadNotCanonical {
        /// Non-canonical hash.
        hash: B256,
    },
    /// Head/safe/finalized did not form canonical ancestry.
    #[error("consensus checkpoints do not form canonical ancestry")]
    InvalidCheckpointAncestry,
    /// Safe checkpoint moved backward or to another branch.
    #[error("safe checkpoint regressed")]
    SafeCheckpointRegression,
    /// Finalized ancestry would be reversed.
    #[error("critical finalized checkpoint reversal at {finalized}")]
    FinalizedReversalCritical {
        /// Previously finalized payload.
        finalized: B256,
    },
}

fn validate_segment(blocks: &[ExecutionBlock]) -> Result<(), EthereumError> {
    for pair in blocks.windows(2) {
        if pair[1].number != pair[0].number.saturating_add(1) || pair[1].parent_hash != pair[0].hash
        {
            return Err(EthereumError::DiscontinuousSegment);
        }
    }
    Ok(())
}

/// One exact official-node observation of BSC head and native finalized tag.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BscFinalityObservation {
    finalized: B256,
    head: B256,
    source_id: String,
    observed_at_unix_ms: u64,
}

impl BscFinalityObservation {
    /// Creates source-qualified fast-finality evidence.
    ///
    /// # Errors
    ///
    /// Rejects blank source identity and zero observation time.
    pub fn new(
        finalized: B256,
        head: B256,
        source_id: impl Into<String>,
        observed_at_unix_ms: u64,
    ) -> Result<Self, BscError> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() || !source_id.is_ascii() {
            return Err(BscError::EmptySourceId);
        }
        if observed_at_unix_ms == 0 {
            return Err(BscError::InvalidObservationTime);
        }
        Ok(Self {
            finalized,
            head,
            source_id,
            observed_at_unix_ms,
        })
    }
}

/// Current health of the BSC native finalized tag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BscFinalityStatus {
    /// No finalized observation has arrived.
    Unknown,
    /// Finalized tag advanced within the configured bound.
    Healthy,
    /// Source observations continue but finalized ancestry has not advanced.
    Stalled,
}

/// One append-only BSC canonicality/finality revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BscRevision {
    /// Exact execution block.
    pub block: ExecutionBlock,
    /// BSC-native status.
    pub status: BscStatus,
    /// Monotonic tracker revision.
    pub revision: u64,
    /// Exact evidence source for finalized revisions.
    pub source_id: Option<String>,
}

/// BSC canonical execution ancestry with a separate fast-finality adapter.
#[derive(Clone, Debug, Default)]
pub struct BscCanonicality {
    blocks: HashMap<B256, ExecutionBlock>,
    canonical: BTreeMap<u64, B256>,
    head: Option<B256>,
    finalized: Option<B256>,
    revision: u64,
    last_observed_at_unix_ms: Option<u64>,
    last_finality_change_at_unix_ms: Option<u64>,
}

impl BscCanonicality {
    /// Creates an empty BSC tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies a canonical BSC segment in ancestor-to-descendant order.
    ///
    /// # Errors
    ///
    /// Rejects discontinuous segments or segments that do not extend the
    /// current canonical head. Exact replay is a no-op.
    pub fn commit_segment<I>(&mut self, blocks: I) -> Result<Vec<BscRevision>, BscError>
    where
        I: IntoIterator<Item = ExecutionBlock>,
    {
        let blocks = blocks.into_iter().collect::<Vec<_>>();
        if blocks.is_empty() {
            return Ok(Vec::new());
        }
        validate_bsc_segment(&blocks)?;
        if blocks.iter().all(|block| {
            self.canonical
                .get(&block.number)
                .is_some_and(|hash| *hash == block.hash)
        }) {
            return Ok(Vec::new());
        }
        if let Some(head_hash) = self.head {
            let head = self.block(head_hash)?;
            let first = blocks[0];
            if first.number != head.number.saturating_add(1) || first.parent_hash != head.hash {
                return Err(BscError::SegmentDoesNotExtendHead);
            }
        }
        let mut revisions = Vec::with_capacity(blocks.len());
        for block in blocks {
            self.blocks.insert(block.hash, block);
            self.canonical.insert(block.number, block.hash);
            self.head = Some(block.hash);
            revisions.push(self.next_revision(block, BscStatus::CanonicalHead, None));
        }
        Ok(revisions)
    }

    /// Validates and appends BSC-native finalized-tag evidence.
    ///
    /// # Errors
    ///
    /// Rejects unknown/non-canonical payloads, source-head mismatch, timestamp
    /// regression, and finalized ancestry regression.
    pub fn observe_finalized(
        &mut self,
        observation: BscFinalityObservation,
    ) -> Result<BscRevision, BscError> {
        if self
            .last_observed_at_unix_ms
            .is_some_and(|previous| observation.observed_at_unix_ms <= previous)
        {
            return Err(BscError::ObservationTimeRegression);
        }
        let head = self.canonical_block(observation.head)?;
        let finalized = self.canonical_block(observation.finalized)?;
        if self.head != Some(head.hash) || !self.is_ancestor(finalized.hash, head.hash) {
            return Err(BscError::InvalidFinalizedAncestry);
        }
        if let Some(previous_hash) = self.finalized
            && previous_hash != finalized.hash
        {
            let previous = self.block(previous_hash)?;
            if finalized.number <= previous.number
                || !self.is_ancestor(previous.hash, finalized.hash)
            {
                return Err(BscError::FinalizedRegression {
                    previous: previous.hash,
                    observed: finalized.hash,
                });
            }
        }
        self.last_observed_at_unix_ms = Some(observation.observed_at_unix_ms);
        if self.finalized != Some(finalized.hash) {
            self.finalized = Some(finalized.hash);
            self.last_finality_change_at_unix_ms = Some(observation.observed_at_unix_ms);
        }
        Ok(self.next_revision(
            finalized,
            BscStatus::FastFinalized,
            Some(observation.source_id),
        ))
    }

    /// Applies a BSC reorg below the separate finality boundary.
    ///
    /// # Errors
    ///
    /// Rejects non-canonical suffixes, branch mismatches, and any removal that
    /// contains the BSC-native finalized block.
    pub fn reorg<IOld, INew>(&mut self, old: IOld, new: INew) -> Result<Vec<BscRevision>, BscError>
    where
        IOld: IntoIterator<Item = ExecutionBlock>,
        INew: IntoIterator<Item = ExecutionBlock>,
    {
        let old = old.into_iter().collect::<Vec<_>>();
        let new = new.into_iter().collect::<Vec<_>>();
        if old.is_empty() || new.is_empty() {
            return Err(BscError::EmptyReorgSegment);
        }
        validate_bsc_segment(&old)?;
        validate_bsc_segment(&new)?;
        if old[0].number != new[0].number || old[0].parent_hash != new[0].parent_hash {
            return Err(BscError::ReorgAncestorMismatch);
        }
        if old.last().map(|block| block.hash) != self.head
            || old.iter().any(|block| {
                self.canonical
                    .get(&block.number)
                    .is_none_or(|hash| *hash != block.hash)
            })
        {
            return Err(BscError::ReorgDoesNotMatchCanonicalSuffix);
        }
        if let Some(finalized) = self.finalized
            && old.iter().any(|block| block.hash == finalized)
        {
            return Err(BscError::FinalizedReversalCritical { finalized });
        }
        let mut revisions = Vec::with_capacity(old.len() + new.len());
        for block in old.iter().rev().copied() {
            self.canonical.remove(&block.number);
            revisions.push(self.next_revision(block, BscStatus::Reorged, None));
        }
        for block in new.iter().copied() {
            self.blocks.insert(block.hash, block);
            self.canonical.insert(block.number, block.hash);
            revisions.push(self.next_revision(block, BscStatus::CanonicalHead, None));
        }
        self.head = new.last().map(|block| block.hash);
        Ok(revisions)
    }

    /// Evaluates finalized-tag liveness without mutating chain truth.
    #[must_use]
    pub fn finality_health(&self, now_unix_ms: u64, maximum_stall_ms: u64) -> BscFinalityStatus {
        match self.last_finality_change_at_unix_ms {
            None => BscFinalityStatus::Unknown,
            Some(changed_at) if now_unix_ms.saturating_sub(changed_at) > maximum_stall_ms => {
                BscFinalityStatus::Stalled
            }
            Some(_) => BscFinalityStatus::Healthy,
        }
    }

    /// Current BSC canonical head.
    #[must_use]
    pub const fn head(&self) -> Option<B256> {
        self.head
    }

    fn canonical_block(&self, hash: B256) -> Result<ExecutionBlock, BscError> {
        let block = self.block(hash)?;
        if self.canonical.get(&block.number) != Some(&hash) {
            return Err(BscError::BlockNotCanonical { hash });
        }
        Ok(block)
    }

    fn block(&self, hash: B256) -> Result<ExecutionBlock, BscError> {
        self.blocks
            .get(&hash)
            .copied()
            .ok_or(BscError::UnknownBlock { hash })
    }

    fn is_ancestor(&self, ancestor: B256, descendant: B256) -> bool {
        let Ok(ancestor_block) = self.block(ancestor) else {
            return false;
        };
        let Ok(mut current) = self.block(descendant) else {
            return false;
        };
        while current.number > ancestor_block.number {
            let Ok(parent) = self.block(current.parent_hash) else {
                return false;
            };
            current = parent;
        }
        current.hash == ancestor
    }

    fn next_revision(
        &mut self,
        block: ExecutionBlock,
        status: BscStatus,
        source_id: Option<String>,
    ) -> BscRevision {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("revision exhaustion is operationally unreachable");
        BscRevision {
            block,
            status,
            revision: self.revision,
            source_id,
        }
    }
}

/// BSC canonicality/finality invariant failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum BscError {
    /// Source identity was blank.
    #[error("BSC source identity must not be empty")]
    EmptySourceId,
    /// Observation time was zero.
    #[error("BSC observation time must be positive")]
    InvalidObservationTime,
    /// Observation time did not advance.
    #[error("BSC finalized observation time regressed")]
    ObservationTimeRegression,
    /// Segment was not contiguous.
    #[error("BSC execution segment is not contiguous")]
    DiscontinuousSegment,
    /// Segment did not extend the canonical head.
    #[error("BSC segment does not extend canonical head")]
    SegmentDoesNotExtendHead,
    /// Block hash is unknown.
    #[error("unknown BSC block {hash}")]
    UnknownBlock {
        /// Unknown hash.
        hash: B256,
    },
    /// Block exists but is not canonical.
    #[error("BSC block {hash} is not canonical")]
    BlockNotCanonical {
        /// Non-canonical hash.
        hash: B256,
    },
    /// Finalized block was not an ancestor of the observed/current head.
    #[error("BSC finalized tag does not belong to canonical head ancestry")]
    InvalidFinalizedAncestry,
    /// Finalized tag moved backward or across branches.
    #[error("BSC finalized tag regressed from {previous} to {observed}")]
    FinalizedRegression {
        /// Previously finalized hash.
        previous: B256,
        /// Observed hash.
        observed: B256,
    },
    /// Reorg lacked one side.
    #[error("BSC reorg requires non-empty old and new segments")]
    EmptyReorgSegment,
    /// Reorg branch boundary mismatched.
    #[error("BSC reorg branches do not share an ancestor boundary")]
    ReorgAncestorMismatch,
    /// Removed branch was not the canonical suffix.
    #[error("BSC reorg removal does not match canonical suffix")]
    ReorgDoesNotMatchCanonicalSuffix,
    /// Reorg attempted to remove the native finalized block.
    #[error("critical BSC finalized reversal at {finalized}")]
    FinalizedReversalCritical {
        /// Previously finalized block.
        finalized: B256,
    },
}

fn validate_bsc_segment(blocks: &[ExecutionBlock]) -> Result<(), BscError> {
    for pair in blocks.windows(2) {
        if pair[1].number != pair[0].number.saturating_add(1) || pair[1].parent_hash != pair[0].hash
        {
            return Err(BscError::DiscontinuousSegment);
        }
    }
    Ok(())
}
