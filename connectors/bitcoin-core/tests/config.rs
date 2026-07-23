use std::path::PathBuf;

use bitcoin_core_connector::{
    config::{BitcoinCoreNetwork, Environment, ObserverConfig, validate_deployment},
    error::ConfigError,
};

fn observer(source: &str, wal: &str) -> ObserverConfig {
    ObserverConfig {
        source_id: source.to_owned(),
        network: BitcoinCoreNetwork::Mainnet,
        rpc_endpoint: "http://127.0.0.1:8332".to_owned(),
        zmq_endpoints: vec!["tcp://10.0.0.2:28332".to_owned()],
        rpc_cookie_path: PathBuf::from("/run/secrets/bitcoin-cookie"),
        wallet_rpc_enabled: false,
        wal_path: PathBuf::from(wal),
        max_message_bytes: 4_000_000,
    }
}

#[test]
fn unsafe_or_privileged_configuration_fails_closed() {
    let mut value = observer("observer-a", "/var/lib/multichain/a.wal");
    value.rpc_endpoint = "http://203.0.113.4:8332".to_owned();
    assert!(matches!(
        value.validate(),
        Err(ConfigError::UnsafeEndpoint { kind: "RPC", .. })
    ));

    let mut value = observer("observer-a", "/var/lib/multichain/a.wal");
    value.zmq_endpoints = vec!["tcp://0.0.0.0:28332".to_owned()];
    assert!(matches!(
        value.validate(),
        Err(ConfigError::UnsafeEndpoint { kind: "ZMQ", .. })
    ));

    let mut value = observer("observer-a", "/var/lib/multichain/a.wal");
    value.wallet_rpc_enabled = true;
    assert_eq!(value.validate(), Err(ConfigError::WalletRpcEnabled));

    let mut value = observer("observer-a", "/var/lib/multichain/a.wal");
    value.rpc_cookie_path = PathBuf::new();
    assert_eq!(value.validate(), Err(ConfigError::MissingRpcSecret));
    assert_eq!(
        BitcoinCoreNetwork::parse("testnet"),
        Err(ConfigError::UnsupportedNetwork)
    );
}

#[test]
fn deployment_requires_independent_identity_state_and_mainnet_quorum() {
    let a = observer("observer-a", "/var/lib/multichain/a.wal");
    let mut duplicate_source = observer("observer-a", "/var/lib/multichain/b.wal");
    assert!(matches!(
        validate_deployment(
            Environment::Development,
            &[a.clone(), duplicate_source.clone()]
        ),
        Err(ConfigError::DuplicateSourceId(_))
    ));

    duplicate_source.source_id = "observer-b".to_owned();
    duplicate_source.wal_path = a.wal_path.clone();
    assert!(matches!(
        validate_deployment(Environment::Development, &[a.clone(), duplicate_source]),
        Err(ConfigError::SharedWalPath(_))
    ));
    assert_eq!(
        validate_deployment(Environment::Production, &[a]),
        Err(ConfigError::InsufficientProductionObservers(1))
    );
}
