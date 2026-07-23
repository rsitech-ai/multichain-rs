use std::collections::HashSet;

use bitcoin_domain::{BlockHash, Txid, parse_block};
use platform_proto::control::CoverageInterval;

use crate::{error::RpcError, rpc::BitcoinRpc};

/// Observable recovery control events; no missing transition is fabricated.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MempoolRecoveryEvent {
    GapDetected { expected: u64, actual: u64 },
    StateReconciled { aligned_sequence: u64 },
}

/// Atomic recovery snapshot and exact RPC result bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveredMempoolSnapshot {
    pub mempool_sequence: u64,
    pub source_payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReconcileState {
    Healthy,
    Gapped,
    AwaitingAlignment,
}

/// Node-local mempool state and immutable incomplete-interval history.
pub struct MempoolReconciler {
    source_id: String,
    source_session_id: [u8; 16],
    last_sequence: Option<u64>,
    state: ReconcileState,
    current: HashSet<Txid>,
    open_started_at: Option<i64>,
    intervals: Vec<CoverageInterval>,
}

impl MempoolReconciler {
    #[must_use]
    pub fn new(source_id: impl Into<String>, source_session_id: [u8; 16]) -> Self {
        Self {
            source_id: source_id.into(),
            source_session_id,
            last_sequence: None,
            state: ReconcileState::Healthy,
            current: HashSet::new(),
            open_started_at: None,
            intervals: Vec::new(),
        }
    }

    pub fn observe_sequence(
        &mut self,
        actual: u64,
        observed_at_unix_ns: i64,
    ) -> Option<MempoolRecoveryEvent> {
        if self.state == ReconcileState::AwaitingAlignment {
            self.state = ReconcileState::Healthy;
            self.last_sequence = Some(actual);
            self.close_interval(observed_at_unix_ns);
            return Some(MempoolRecoveryEvent::StateReconciled {
                aligned_sequence: actual,
            });
        }
        let event = self.last_sequence.and_then(|last| {
            let expected = last.wrapping_add(1);
            (actual != expected).then_some(MempoolRecoveryEvent::GapDetected { expected, actual })
        });
        self.last_sequence = Some(actual);
        if event.is_some() {
            self.state = ReconcileState::Gapped;
            self.open_started_at = Some(observed_at_unix_ns);
        }
        event
    }

    pub fn apply_add(&mut self, txid: Txid) {
        self.current.insert(txid);
    }

    pub fn apply_remove(&mut self, txid: &Txid) {
        self.current.remove(txid);
    }

    /// Replaces current state from an atomic Core snapshot, preserving the gap.
    ///
    /// # Errors
    ///
    /// Returns the underlying allowlisted RPC failure.
    pub async fn recover<R: BitcoinRpc>(
        &mut self,
        rpc: &R,
    ) -> Result<RecoveredMempoolSnapshot, RpcError> {
        let snapshot = rpc.get_raw_mempool_with_sequence().await?;
        self.current = snapshot.txids.into_iter().collect();
        self.last_sequence = Some(snapshot.mempool_sequence);
        self.state = ReconcileState::AwaitingAlignment;
        Ok(RecoveredMempoolSnapshot {
            mempool_sequence: snapshot.mempool_sequence,
            source_payload: snapshot.source_payload,
        })
    }

    #[must_use]
    pub fn contains(&self, txid: &Txid) -> bool {
        self.current.contains(txid)
    }

    #[must_use]
    pub fn intervals(&self) -> &[CoverageInterval] {
        &self.intervals
    }

    fn close_interval(&mut self, ended_at_unix_ns: i64) {
        if let Some(started_at_unix_ns) = self.open_started_at.take() {
            self.intervals.push(CoverageInterval {
                source_id: self.source_id.clone(),
                source_session_id: self.source_session_id.to_vec(),
                start_unix_ns: started_at_unix_ns,
                end_unix_ns: Some(ended_at_unix_ns),
                state: "known_incomplete".to_owned(),
                cause: "zmq_or_mempool_sequence_gap".to_owned(),
                repair_evidence_observation_ids: Vec::new(),
            });
        }
    }
}

/// Walks parent hashes from Core's best block to a known durable ancestor.
///
/// # Errors
///
/// Returns an RPC or Bitcoin block-validation failure.
pub async fn recover_blocks_to_ancestor<R, S>(
    rpc: &R,
    known_blocks: &HashSet<BlockHash, S>,
) -> Result<Vec<Vec<u8>>, RpcError>
where
    R: BitcoinRpc,
    S: std::hash::BuildHasher,
{
    let mut cursor = rpc.get_best_block_hash().await?;
    let mut recovered = Vec::new();
    while !known_blocks.contains(&cursor) {
        let bytes = rpc.get_block(cursor).await?;
        let block = parse_block(&bytes).map_err(|error| RpcError::InvalidResult {
            method: "getblock",
            message: error.to_string(),
        })?;
        cursor = block.previous_block_hash();
        recovered.push(bytes);
    }
    recovered.reverse();
    Ok(recovered)
}
