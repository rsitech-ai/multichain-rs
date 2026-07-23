use std::{path::PathBuf, str::FromStr as _, time::Duration};

use async_trait::async_trait;
use bitcoin::hashes::Hash as _;
use bitcoin_domain::{BlockHash, Txid};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::error::RpcError;

/// Atomic node-local mempool view returned with Core's sequence.
#[derive(Clone, Debug)]
pub struct MempoolSnapshot {
    pub txids: Vec<Txid>,
    pub mempool_sequence: u64,
    pub source_payload: Vec<u8>,
}

/// One `getchaintips` result.
#[derive(Clone, Debug)]
pub struct ChainTip {
    pub hash: BlockHash,
    pub height: u64,
    pub branch_length: u64,
    pub status: String,
}

#[async_trait]
pub trait BitcoinRpc: Send + Sync {
    async fn get_raw_mempool_with_sequence(&self) -> Result<MempoolSnapshot, RpcError>;
    async fn get_raw_transaction(&self, txid: Txid) -> Result<Option<Vec<u8>>, RpcError>;
    async fn get_block_hash(&self, height: u32) -> Result<BlockHash, RpcError>;
    async fn get_block(&self, hash: BlockHash) -> Result<Vec<u8>, RpcError>;
    async fn get_best_block_hash(&self) -> Result<BlockHash, RpcError>;
    async fn get_chain_tips(&self) -> Result<Vec<ChainTip>, RpcError>;
}

/// Cookie-authenticated, allowlist-only Bitcoin Core JSON-RPC client.
pub struct CoreRpcClient {
    endpoint: String,
    cookie_path: PathBuf,
    client: reqwest::Client,
    cancellation: CancellationToken,
    maximum_attempts: u32,
}

#[derive(Deserialize)]
struct RawMempoolSnapshot {
    txids: Vec<String>,
    mempool_sequence: u64,
}

impl CoreRpcClient {
    /// Creates a client with bounded request time and retry count.
    ///
    /// # Errors
    ///
    /// Returns an error if the bounded HTTP client cannot be constructed.
    pub fn new(
        endpoint: impl Into<String>,
        cookie_path: PathBuf,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<Self, RpcError> {
        Ok(Self {
            endpoint: endpoint.into(),
            cookie_path,
            client: reqwest::Client::builder().timeout(timeout).build()?,
            cancellation,
            maximum_attempts: 3,
        })
    }

    async fn call(&self, method: &'static str, params: Value) -> Result<Value, RpcError> {
        let cookie = tokio::fs::read_to_string(&self.cookie_path)
            .await
            .map_err(RpcError::Secret)?;
        let (username, password) =
            cookie
                .trim()
                .split_once(':')
                .ok_or_else(|| RpcError::InvalidResult {
                    method,
                    message: "RPC cookie lacks user:password format".to_owned(),
                })?;
        for attempt in 0..self.maximum_attempts {
            let request = self
                .client
                .post(&self.endpoint)
                .basic_auth(username, Some(password))
                .json(&json!({"jsonrpc":"2.0","id":"multichain","method":method,"params":params}));
            let response = tokio::select! {
                () = self.cancellation.cancelled() => return Err(RpcError::Cancelled),
                result = request.send() => result,
            };
            match response {
                Ok(response) if !response.status().is_server_error() => {
                    let envelope: RpcEnvelope = response.json().await?;
                    if let Some(error) = envelope.error {
                        return Err(RpcError::Remote {
                            method,
                            code: error.code,
                            message: error.message,
                        });
                    }
                    return envelope.result.ok_or_else(|| RpcError::InvalidResult {
                        method,
                        message: "missing result".to_owned(),
                    });
                }
                Ok(_) | Err(_) if attempt + 1 < self.maximum_attempts => {}
                Ok(response) => {
                    return Err(RpcError::Transport(
                        response.error_for_status().expect_err("5xx status"),
                    ));
                }
                Err(error) => return Err(RpcError::Transport(error)),
            }
            let jitter = u64::from(std::process::id() % 17) + u64::from(attempt * 7);
            tokio::select! {
                () = self.cancellation.cancelled() => return Err(RpcError::Cancelled),
                () = tokio::time::sleep(Duration::from_millis(25 * u64::from(attempt + 1) + jitter)) => {}
            }
        }
        unreachable!("bounded loop returns on final attempt")
    }
}

#[async_trait]
impl BitcoinRpc for CoreRpcClient {
    async fn get_raw_mempool_with_sequence(&self) -> Result<MempoolSnapshot, RpcError> {
        const METHOD: &str = "getrawmempool";
        let value = self.call(METHOD, json!([false, true])).await?;
        let source_payload =
            serde_json::to_vec(&value).map_err(|error| RpcError::InvalidResult {
                method: METHOD,
                message: error.to_string(),
            })?;
        let snapshot: RawMempoolSnapshot =
            serde_json::from_value(value).map_err(|error| RpcError::InvalidResult {
                method: METHOD,
                message: error.to_string(),
            })?;
        let txids = snapshot
            .txids
            .iter()
            .map(|value| parse_txid(METHOD, value))
            .collect::<Result<_, _>>()?;
        Ok(MempoolSnapshot {
            txids,
            mempool_sequence: snapshot.mempool_sequence,
            source_payload,
        })
    }

    async fn get_raw_transaction(&self, txid: Txid) -> Result<Option<Vec<u8>>, RpcError> {
        const METHOD: &str = "getrawtransaction";
        match self.call(METHOD, json!([txid.to_string(), false])).await {
            Ok(value) => decode_hex_value(METHOD, &value).map(Some),
            Err(RpcError::Remote { code: -5, .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn get_block_hash(&self, height: u32) -> Result<BlockHash, RpcError> {
        const METHOD: &str = "getblockhash";
        let value = self.call(METHOD, json!([height])).await?;
        parse_block_hash(
            METHOD,
            value.as_str().ok_or_else(|| RpcError::InvalidResult {
                method: METHOD,
                message: "block hash is not a string".to_owned(),
            })?,
        )
    }

    async fn get_block(&self, hash: BlockHash) -> Result<Vec<u8>, RpcError> {
        const METHOD: &str = "getblock";
        decode_hex_value(
            METHOD,
            &self.call(METHOD, json!([hash.to_string(), 0])).await?,
        )
    }

    async fn get_best_block_hash(&self) -> Result<BlockHash, RpcError> {
        const METHOD: &str = "getbestblockhash";
        let value = self.call(METHOD, json!([])).await?;
        parse_block_hash(
            METHOD,
            value.as_str().ok_or_else(|| RpcError::InvalidResult {
                method: METHOD,
                message: "block hash is not a string".to_owned(),
            })?,
        )
    }

    async fn get_chain_tips(&self) -> Result<Vec<ChainTip>, RpcError> {
        const METHOD: &str = "getchaintips";
        #[derive(Deserialize)]
        struct RawTip {
            height: u64,
            hash: String,
            branchlen: u64,
            status: String,
        }
        let tips: Vec<RawTip> = serde_json::from_value(self.call(METHOD, json!([])).await?)
            .map_err(|error| RpcError::InvalidResult {
                method: METHOD,
                message: error.to_string(),
            })?;
        tips.into_iter()
            .map(|tip| {
                Ok(ChainTip {
                    hash: parse_block_hash(METHOD, &tip.hash)?,
                    height: tip.height,
                    branch_length: tip.branchlen,
                    status: tip.status,
                })
            })
            .collect()
    }
}

#[derive(Deserialize)]
struct RpcEnvelope {
    result: Option<Value>,
    error: Option<RpcRemoteError>,
}

#[derive(Deserialize)]
struct RpcRemoteError {
    code: i64,
    message: String,
}

fn parse_txid(method: &'static str, value: &str) -> Result<Txid, RpcError> {
    let txid = bitcoin::Txid::from_str(value).map_err(|error| RpcError::InvalidResult {
        method,
        message: error.to_string(),
    })?;
    Ok(Txid::from_bytes(txid.to_byte_array()))
}

fn parse_block_hash(method: &'static str, value: &str) -> Result<BlockHash, RpcError> {
    let hash = bitcoin::BlockHash::from_str(value).map_err(|error| RpcError::InvalidResult {
        method,
        message: error.to_string(),
    })?;
    Ok(BlockHash::from_bytes(hash.to_byte_array()))
}

fn decode_hex_value(method: &'static str, value: &Value) -> Result<Vec<u8>, RpcError> {
    let text = value.as_str().ok_or_else(|| RpcError::InvalidResult {
        method,
        message: "hex result is not a string".to_owned(),
    })?;
    if !text.len().is_multiple_of(2) {
        return Err(RpcError::InvalidResult {
            method,
            message: "hex result has odd length".to_owned(),
        });
    }
    (0..text.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&text[index..index + 2], 16).map_err(|error| {
                RpcError::InvalidResult {
                    method,
                    message: error.to_string(),
                }
            })
        })
        .collect()
}
