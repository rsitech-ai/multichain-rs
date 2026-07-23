use api_contract::{Completeness, SolanaCoverage, SolanaTruth};
use solana_domain::{Blockhash, ForkId, Slot};

#[test]
fn solana_truth_is_fork_source_coverage_and_recovery_qualified() {
    let truth = SolanaTruth::new(
        ["yellowstone-a", "yellowstone-b"],
        9,
        &ForkId::new(Slot::new(434_739_559), Blockhash::new_from_array([7; 32])),
        "canonical",
        "finalized",
        Completeness::Recovered,
        SolanaCoverage::new(2, 42, false, true),
        false,
        [[8; 32]],
        1_000,
    )
    .expect("truth");
    let json = serde_json::to_value(truth).expect("JSON");

    assert_eq!(json["network"], "solana-mainnet-beta");
    assert_eq!(json["slot"], 434_739_559);
    assert!(
        json["blockhash"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert_eq!(json["source_ids"].as_array().expect("sources").len(), 2);
    assert_eq!(json["coverage"]["selected_account_filter_count"], 42);
    assert_eq!(json["coverage"]["full_account_firehose"], false);
    assert_eq!(json["recovery_observation_ids"][0], "08".repeat(32));
}

#[test]
fn contradictions_missing_provider_and_unproven_recovery_fail_closed() {
    let fork = ForkId::new(Slot::new(1), Blockhash::new_from_array([1; 32]));
    assert!(
        SolanaTruth::new(
            ["only-one"],
            1,
            &fork,
            "canonical",
            "processed",
            Completeness::Complete,
            SolanaCoverage::new(2, 0, false, false),
            false,
            std::iter::empty::<[u8; 32]>(),
            1,
        )
        .is_err()
    );
    assert!(
        SolanaTruth::new(
            ["a", "b"],
            1,
            &fork,
            "canonical",
            "dead",
            Completeness::KnownIncomplete,
            SolanaCoverage::new(2, 1, false, false),
            false,
            std::iter::empty::<[u8; 32]>(),
            1,
        )
        .is_err()
    );
    assert!(
        SolanaTruth::new(
            ["a", "b"],
            1,
            &fork,
            "non_canonical",
            "dead",
            Completeness::Complete,
            SolanaCoverage::new(2, 1, false, false),
            true,
            std::iter::empty::<[u8; 32]>(),
            1,
        )
        .is_err()
    );
    assert!(
        SolanaTruth::new(
            ["a", "b"],
            1,
            &fork,
            "canonical",
            "finalized",
            Completeness::Recovered,
            SolanaCoverage::new(2, 1, false, true),
            false,
            std::iter::empty::<[u8; 32]>(),
            1,
        )
        .is_err()
    );
}
