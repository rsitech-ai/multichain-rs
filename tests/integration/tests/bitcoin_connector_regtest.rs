use std::{path::PathBuf, time::Duration};

use bitcoin_core_connector::rpc::{BitcoinRpc, CoreRpcClient};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn bitcoin_connector_regtest() {
    let Ok(endpoint) = std::env::var("BITCOIN_REGTEST_RPC_URL") else {
        eprintln!("SKIP: set BITCOIN_REGTEST_RPC_URL and BITCOIN_REGTEST_COOKIE");
        return;
    };
    let Ok(cookie) = std::env::var("BITCOIN_REGTEST_COOKIE") else {
        eprintln!("SKIP: set BITCOIN_REGTEST_RPC_URL and BITCOIN_REGTEST_COOKIE");
        return;
    };
    let client = CoreRpcClient::new(
        endpoint,
        PathBuf::from(cookie),
        Duration::from_secs(5),
        CancellationToken::new(),
    )
    .expect("RPC client");

    let best = client.get_best_block_hash().await.expect("best block");
    let block = client.get_block(best).await.expect("raw best block");
    assert!(!block.is_empty());
    let tips = client.get_chain_tips().await.expect("chain tips");
    assert!(tips.iter().any(|tip| tip.hash == best));
    let snapshot = client
        .get_raw_mempool_with_sequence()
        .await
        .expect("atomic mempool snapshot");
    assert!(!snapshot.source_payload.is_empty());
}
