use std::{
    collections::HashSet,
    net::IpAddr,
    path::{Path, PathBuf},
};

use crate::error::ConfigError;

/// Deployment class controls quorum validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Environment {
    Development,
    Production,
}

/// Networks supported by the dedicated Bitcoin Core connector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BitcoinCoreNetwork {
    Mainnet,
    Regtest,
}

impl BitcoinCoreNetwork {
    /// Parses only the networks this dedicated binary may observe.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::UnsupportedNetwork`] for every other network.
    pub fn parse(value: &str) -> Result<Self, ConfigError> {
        match value {
            "mainnet" => Ok(Self::Mainnet),
            "regtest" => Ok(Self::Regtest),
            _ => Err(ConfigError::UnsupportedNetwork),
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Regtest => "regtest",
        }
    }
}

/// One independently stateful Bitcoin Core observer.
#[derive(Clone, Debug)]
pub struct ObserverConfig {
    pub source_id: String,
    pub network: BitcoinCoreNetwork,
    pub rpc_endpoint: String,
    pub zmq_endpoints: Vec<String>,
    pub rpc_cookie_path: PathBuf,
    pub wallet_rpc_enabled: bool,
    pub wal_path: PathBuf,
    pub max_message_bytes: usize,
}

impl ObserverConfig {
    /// Validates local-secret and private-network boundaries.
    ///
    /// # Errors
    ///
    /// Returns the first unsafe or incomplete setting.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.source_id.trim().is_empty() {
            return Err(ConfigError::EmptySourceId);
        }
        validate_endpoint("RPC", &self.rpc_endpoint)?;
        for endpoint in &self.zmq_endpoints {
            validate_endpoint("ZMQ", endpoint)?;
        }
        if self.wallet_rpc_enabled {
            return Err(ConfigError::WalletRpcEnabled);
        }
        if self.rpc_cookie_path.as_os_str().is_empty() {
            return Err(ConfigError::MissingRpcSecret);
        }
        Ok(())
    }

    #[must_use]
    pub fn wal_path(&self) -> &Path {
        &self.wal_path
    }
}

/// Validates cross-observer safety rules.
///
/// # Errors
///
/// Returns the first invalid observer, identity collision, state collision, or
/// insufficient production quorum.
pub fn validate_deployment(
    environment: Environment,
    observers: &[ObserverConfig],
) -> Result<(), ConfigError> {
    let mut sources = HashSet::new();
    let mut wal_paths = HashSet::new();
    for observer in observers {
        observer.validate()?;
        if !sources.insert(observer.source_id.clone()) {
            return Err(ConfigError::DuplicateSourceId(observer.source_id.clone()));
        }
        if !wal_paths.insert(observer.wal_path.clone()) {
            return Err(ConfigError::SharedWalPath(
                observer.wal_path.display().to_string(),
            ));
        }
    }
    let production_mainnet = environment == Environment::Production
        && observers
            .iter()
            .all(|observer| observer.network == BitcoinCoreNetwork::Mainnet);
    if production_mainnet && observers.len() < 3 {
        return Err(ConfigError::InsufficientProductionObservers(
            observers.len(),
        ));
    }
    Ok(())
}

fn validate_endpoint(kind: &'static str, value: &str) -> Result<(), ConfigError> {
    let authority = value
        .split_once("://")
        .map_or(value, |(_, authority)| authority);
    let host = authority
        .rsplit_once(':')
        .map_or(authority, |(host, _)| host)
        .trim_matches(['[', ']']);
    let safe = host.parse::<IpAddr>().is_ok_and(|ip| match ip {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local(),
    });
    if safe {
        Ok(())
    } else {
        Err(ConfigError::UnsafeEndpoint {
            kind,
            value: value.to_owned(),
        })
    }
}
