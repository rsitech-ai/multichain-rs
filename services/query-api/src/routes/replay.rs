use async_trait::async_trait;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use bitcoin_canonicality::{BackfillError, BackfillRequest, BackfillSource};
use bitcoin_core_connector::rpc::BitcoinRpc;
use bitcoin_domain::BlockHash;
use serde::{Deserialize, Serialize};

use crate::ApiErrorBody;

#[derive(Debug, Deserialize)]
pub(crate) struct BitcoinReplayRequest {
    source_id: String,
    start_height: u32,
    end_height_inclusive: u32,
    max_in_flight: usize,
}

#[derive(Debug, Serialize)]
struct BitcoinReplayValidation {
    request_hash: String,
    source_id: String,
    start_height: u32,
    end_height_inclusive: u32,
    block_count: u64,
    max_in_flight: usize,
}

pub(crate) async fn validate(Json(request): Json<BitcoinReplayRequest>) -> Response {
    match validated_response(request) {
        Ok(response) => Json(response).into_response(),
        Err(_) => (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorBody {
                error: "invalid_bitcoin_replay_request",
            }),
        )
            .into_response(),
    }
}

fn validated_response(
    request: BitcoinReplayRequest,
) -> Result<BitcoinReplayValidation, BackfillError> {
    let request = BackfillRequest::new(
        request.source_id,
        request.start_height,
        request.end_height_inclusive,
        request.max_in_flight,
    )?;
    let block_count =
        u64::from(request.end_height_inclusive() - request.start_height()).saturating_add(1);
    Ok(BitcoinReplayValidation {
        request_hash: encode_hex(&request.request_hash()),
        source_id: request.source_id().to_owned(),
        start_height: request.start_height(),
        end_height_inclusive: request.end_height_inclusive(),
        block_count,
        max_in_flight: request.max_in_flight(),
    })
}

/// Query-service adapter from the allowlisted Bitcoin RPC port to backfill.
pub struct BitcoinRpcBackfillSource<R> {
    rpc: R,
}

impl<R> BitcoinRpcBackfillSource<R> {
    #[must_use]
    pub const fn new(rpc: R) -> Self {
        Self { rpc }
    }
}

#[async_trait]
impl<R> BackfillSource for BitcoinRpcBackfillSource<R>
where
    R: BitcoinRpc,
{
    async fn best_block_hash(&self) -> Result<BlockHash, BackfillError> {
        self.rpc
            .get_best_block_hash()
            .await
            .map_err(|error| source_error(&error))
    }

    async fn block_hash(&self, height: u32) -> Result<BlockHash, BackfillError> {
        self.rpc
            .get_block_hash(height)
            .await
            .map_err(|error| source_error(&error))
    }

    async fn raw_block(&self, hash: BlockHash) -> Result<Vec<u8>, BackfillError> {
        self.rpc
            .get_block(hash)
            .await
            .map_err(|error| source_error(&error))
    }
}

fn source_error(error: &bitcoin_core_connector::error::RpcError) -> BackfillError {
    BackfillError::Source(error.to_string())
}

fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut hex, byte| {
            write!(hex, "{byte:02x}").expect("writing into a String cannot fail");
            hex
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_validation_is_bounded_and_replay_stable() {
        let first = validated_response(BitcoinReplayRequest {
            source_id: "observer-eu-1".to_owned(),
            start_height: 100,
            end_height_inclusive: 200,
            max_in_flight: 4,
        })
        .expect("valid request");
        let second = validated_response(BitcoinReplayRequest {
            source_id: "observer-eu-1".to_owned(),
            start_height: 100,
            end_height_inclusive: 200,
            max_in_flight: 4,
        })
        .expect("same request");
        assert_eq!(first.request_hash, second.request_hash);
        assert_eq!(first.block_count, 101);
        assert_eq!(first.request_hash.len(), 64);

        assert!(
            validated_response(BitcoinReplayRequest {
                source_id: String::new(),
                start_height: 0,
                end_height_inclusive: 1,
                max_in_flight: 1,
            })
            .is_err()
        );
    }
}
