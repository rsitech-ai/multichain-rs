use solana_domain::{Blockhash, Slot};
use solana_yellowstone_connector::{ProviderReconciler, SourceBlockObservation};

#[test]
fn provider_views_remain_source_qualified_and_disagreement_is_explicit() {
    let mut reconciler =
        ProviderReconciler::new(["solana-eu-a", "solana-us-b"]).expect("two sources");
    let first = SourceBlockObservation::new(
        "solana-eu-a",
        Slot::new(50),
        Blockhash::new_from_array([1; 32]),
        100,
    )
    .expect("observation");
    let second = SourceBlockObservation::new(
        "solana-us-b",
        Slot::new(50),
        Blockhash::new_from_array([2; 32]),
        105,
    )
    .expect("observation");

    assert!(reconciler.observe(first).expect("first").is_none());
    let divergence = reconciler
        .observe(second)
        .expect("second")
        .expect("divergence");
    assert_eq!(divergence.slot(), Slot::new(50));
    assert_eq!(divergence.observations().len(), 2);
    assert_ne!(
        divergence.observations()[0].blockhash(),
        divergence.observations()[1].blockhash()
    );
}

#[test]
fn duplicate_source_or_regressing_observation_is_rejected() {
    assert!(ProviderReconciler::new(["same", "same"]).is_err());
    let mut reconciler = ProviderReconciler::new(["solana-eu-a", "solana-us-b"]).expect("sources");
    reconciler
        .observe(
            SourceBlockObservation::new(
                "solana-eu-a",
                Slot::new(51),
                Blockhash::new_from_array([1; 32]),
                100,
            )
            .expect("observation"),
        )
        .expect("first");
    assert!(
        reconciler
            .observe(
                SourceBlockObservation::new(
                    "solana-eu-a",
                    Slot::new(50),
                    Blockhash::new_from_array([1; 32]),
                    101,
                )
                .expect("observation"),
            )
            .is_err()
    );
}
