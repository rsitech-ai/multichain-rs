use async_trait::async_trait;
use bitcoin_core_connector::{
    error::RpcError,
    reconcile::{MempoolReconciler, MempoolRecoveryEvent},
    rpc::{BitcoinRpc, ChainTip, MempoolSnapshot},
};
use bitcoin_domain::{BlockHash, Txid};

struct SnapshotRpc {
    snapshot: MempoolSnapshot,
}

#[async_trait]
impl BitcoinRpc for SnapshotRpc {
    async fn get_raw_mempool_with_sequence(&self) -> Result<MempoolSnapshot, RpcError> {
        Ok(self.snapshot.clone())
    }

    async fn get_raw_transaction(&self, _txid: Txid) -> Result<Option<Vec<u8>>, RpcError> {
        unreachable!("not used")
    }

    async fn get_block(&self, _hash: BlockHash) -> Result<Vec<u8>, RpcError> {
        unreachable!("not used")
    }

    async fn get_best_block_hash(&self) -> Result<BlockHash, RpcError> {
        unreachable!("not used")
    }

    async fn get_chain_tips(&self) -> Result<Vec<ChainTip>, RpcError> {
        unreachable!("not used")
    }
}

#[tokio::test]
async fn resnapshot_converges_state_but_preserves_irrecoverable_history_gap() {
    let transient = Txid::from_bytes([9; 32]);
    let current = Txid::from_bytes([8; 32]);
    let mut reconciler = MempoolReconciler::new("observer-a", [1; 16]);
    reconciler.apply_add(transient);
    assert_eq!(reconciler.observe_sequence(10, 100), None);
    assert_eq!(
        reconciler.observe_sequence(12, 120),
        Some(MempoolRecoveryEvent::GapDetected {
            expected: 11,
            actual: 12,
        })
    );

    let recovered = reconciler
        .recover(&SnapshotRpc {
            snapshot: MempoolSnapshot {
                txids: vec![current],
                mempool_sequence: 12,
                source_payload: br#"{"txids":[],"mempool_sequence":12}"#.to_vec(),
            },
        })
        .await
        .expect("snapshot recovery");
    assert_eq!(recovered.mempool_sequence, 12);
    assert!(!recovered.source_payload.is_empty());
    assert!(!reconciler.contains(&transient));
    assert!(reconciler.contains(&current));
    assert_eq!(
        reconciler.observe_sequence(13, 140),
        Some(MempoolRecoveryEvent::StateReconciled {
            aligned_sequence: 13
        })
    );
    assert_eq!(reconciler.intervals().len(), 1);
    assert_eq!(reconciler.intervals()[0].state, "known_incomplete");
    assert_eq!(reconciler.intervals()[0].end_unix_ns, Some(140));
}
