use solana_canonicality::{PreExecutionObservation, ReconstructionEvidence};
use solana_domain::{Blockhash, ForkId, Signature, Slot, TransactionKey};

#[test]
fn recovery_requires_independent_source_and_exact_evidence() {
    let fork = ForkId::new(Slot::new(300), Blockhash::new_from_array([3; 32]));
    let evidence =
        ReconstructionEvidence::new("yellowstone-a", "yellowstone-b", fork.clone(), [9; 32])
            .expect("independent evidence");
    assert_eq!(evidence.fork_id(), &fork);
    assert_eq!(evidence.recovery_observation_id(), &[9; 32]);
    assert!(ReconstructionEvidence::new("same", "same", fork.clone(), [9; 32]).is_err());
    assert!(ReconstructionEvidence::new("a", "b", fork, [0; 32]).is_err());
}

#[test]
fn pre_execution_signal_never_becomes_executed_without_fork_join() {
    let signature = Signature::from([4; 64]);
    let signal = PreExecutionObservation::new("yellowstone-a", signature, 1_000).expect("signal");
    assert!(!signal.is_executed());

    let key = TransactionKey::new(
        signature,
        ForkId::new(Slot::new(301), Blockhash::new_from_array([5; 32])),
    );
    let joined = signal.join_execution(key.clone()).expect("execution join");
    assert!(joined.is_executed());
    assert_eq!(joined.transaction_key(), Some(&key));
    assert!(joined.join_execution(key).is_err());
}
