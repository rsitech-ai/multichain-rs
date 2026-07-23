use solana_domain::Pubkey;
use solana_yellowstone_connector::{YellowstoneDeployment, YellowstoneSourceConfig};

fn source(source_id: &str, provider_id: &str, endpoint: &str) -> YellowstoneSourceConfig {
    YellowstoneSourceConfig::new(
        source_id,
        provider_id,
        endpoint,
        Some("YELLOWSTONE_TOKEN"),
        vec![Pubkey::new_from_array([7; 32])],
    )
    .expect("valid source")
}

#[test]
fn deployment_requires_two_independent_source_and_provider_identities() {
    let first = source(
        "solana-eu-a",
        "provider-a",
        "https://yellowstone-a.example.com",
    );
    let second = source(
        "solana-us-b",
        "provider-b",
        "https://yellowstone-b.example.com",
    );
    let deployment = YellowstoneDeployment::new(first.clone(), second).expect("independent");

    assert_eq!(deployment.sources().len(), 2);
    assert!(
        YellowstoneDeployment::new(
            first.clone(),
            source(
                "solana-eu-a",
                "provider-b",
                "https://yellowstone-b.example.com"
            )
        )
        .is_err()
    );
    assert!(
        YellowstoneDeployment::new(
            first.clone(),
            source(
                "solana-us-b",
                "provider-a",
                "https://yellowstone-b.example.com"
            )
        )
        .is_err()
    );
    assert!(YellowstoneDeployment::new(first.clone(), first).is_err());
}

#[test]
fn endpoints_secrets_and_account_firehose_fail_closed() {
    assert!(
        YellowstoneSourceConfig::new(
            "",
            "provider",
            "https://yellowstone.example.com",
            None,
            Vec::new(),
        )
        .is_err()
    );
    assert!(
        YellowstoneSourceConfig::new(
            "source",
            "provider",
            "http://yellowstone.example.com",
            None,
            Vec::new(),
        )
        .is_err()
    );
    assert!(
        YellowstoneSourceConfig::new(
            "source",
            "provider",
            "https://token@yellowstone.example.com?x-token=secret",
            None,
            Vec::new(),
        )
        .is_err()
    );
    assert!(
        YellowstoneSourceConfig::new(
            "source",
            "provider",
            "https://yellowstone.example.com",
            Some("literal-token-value"),
            Vec::new(),
        )
        .is_err()
    );
    assert!(
        YellowstoneSourceConfig::new(
            "source",
            "provider",
            "https://yellowstone.example.com",
            None,
            vec![Pubkey::new_from_array([1; 32]); 1_025],
        )
        .is_err()
    );
}

#[test]
fn request_subscribes_to_all_transactions_but_only_selected_accounts() {
    let config = source(
        "solana-eu-a",
        "provider-a",
        "https://yellowstone-a.example.com",
    );
    let request = config.subscribe_request();

    assert!(request.slots.contains_key("commitment-transitions"));
    assert!(request.transactions.contains_key("all-transactions"));
    assert!(request.blocks_meta.contains_key("block-metadata"));
    let accounts = request
        .accounts
        .get("selected-accounts")
        .expect("selected accounts");
    assert_eq!(accounts.account.len(), 1);
    assert!(accounts.owner.is_empty());
    assert_eq!(request.commitment, None);
    assert_eq!(request.from_slot, None);
}
