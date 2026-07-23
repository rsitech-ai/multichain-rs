#!/usr/bin/env bash

set -Eeuo pipefail

SCRIPT_DIRECTORY="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIRECTORY
# shellcheck source=scripts/local-validation/common.sh
source "$SCRIPT_DIRECTORY/common.sh"

readonly RETH_VERSION="2.2.0"
readonly RETH_ARCHIVE="reth-v2.2.0-aarch64-apple-darwin.tar.gz"
readonly RETH_URL="https://github.com/paradigmxyz/reth/releases/download/v2.2.0/$RETH_ARCHIVE"
readonly RETH_SHA256="7ae83603a2ceafab6c5c9dde5337840ed7e4270f309030379727a86337d2c1c5"
readonly RETH_RPC_PORT="19545"
readonly RETH_RPC_URL="http://127.0.0.1:$RETH_RPC_PORT"
readonly DEV_ACCOUNT_A="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
readonly DEV_ACCOUNT_B="0x70997970C51812dc3A010C7d01b50e0d17dc79C8"

RETH_BINARY=""
RETH_PID=""

start_reth() {
  local log_name="$1"
  "$RETH_BINARY" node \
    --dev \
    --dev.block-time 1s \
    --datadir "$LV_CHAIN_RUNTIME/reth-data" \
    --http \
    --http.addr 127.0.0.1 \
    --http.port "$RETH_RPC_PORT" \
    --http.api eth,net,web3,txpool,rpc,reth \
    --disable-discovery \
    >"$LV_CHAIN_EVIDENCE/$log_name" 2>&1 &
  RETH_PID="$!"
  lv_register_pid "$RETH_PID"
  lv_wait_for_rpc "$RETH_RPC_URL" web3_clientVersion '[]' 90 >/dev/null
}

wait_for_receipt() {
  local transaction_hash="$1"
  local attempt
  local receipt
  for attempt in {1..60}; do
    receipt="$(lv_rpc \
      "$RETH_RPC_URL" \
      eth_getTransactionReceipt \
      "$(jq -cn --arg hash "$transaction_hash" '[$hash]')")"
    if [[ "$(jq -r '.result.blockHash // empty' <<<"$receipt")" != "" ]]; then
      printf '%s\n' "$receipt"
      return 0
    fi
    sleep 1
  done
  printf 'Reth transaction receipt timeout: %s\n' "$transaction_hash" >&2
  return 1
}

lv_chain_init ethereum
lv_require_arm64_macos
lv_require_capacity
for command_name in curl jq shasum tar cargo; do
  lv_require_command "$command_name"
done
lv_require_loopback_port tcp "$RETH_RPC_PORT"

archive_path="$LV_CHAIN_RUNTIME/$RETH_ARCHIVE"
lv_download_verified "$RETH_URL" "$RETH_SHA256" "$archive_path"
mkdir -p "$LV_CHAIN_RUNTIME/release"
tar -xzf "$archive_path" -C "$LV_CHAIN_RUNTIME/release"
RETH_BINARY="$(find "$LV_CHAIN_RUNTIME/release" -type f -name reth -perm -111 -print -quit)"
[[ -x "$RETH_BINARY" ]]

version_line="$("$RETH_BINARY" --version)"
[[ "$version_line" == *"$RETH_VERSION"* ]]
start_reth reth-first.log

client_version="$(lv_rpc "$RETH_RPC_URL" web3_clientVersion '[]' | jq -r '.result')"
chain_id="$(lv_rpc "$RETH_RPC_URL" eth_chainId '[]' | jq -r '.result')"
accounts="$(lv_rpc "$RETH_RPC_URL" eth_accounts '[]' | jq -c '.result')"
dev_account_a_lower="$(printf '%s' "$DEV_ACCOUNT_A" | tr '[:upper:]' '[:lower:]')"
dev_account_b_lower="$(printf '%s' "$DEV_ACCOUNT_B" | tr '[:upper:]' '[:lower:]')"
jq -e \
  --arg first "$dev_account_a_lower" \
  --arg second "$dev_account_b_lower" \
  'map(ascii_downcase) | index($first) != null and index($second) != null' \
  <<<"$accounts" >/dev/null
head_before="$(lv_rpc "$RETH_RPC_URL" eth_blockNumber '[]' | jq -r '.result')"
gas_price="$(lv_rpc "$RETH_RPC_URL" eth_gasPrice '[]' | jq -r '.result')"

send_response="$(lv_rpc \
  "$RETH_RPC_URL" \
  eth_sendTransaction \
  "$(jq -cn \
    --arg from "$DEV_ACCOUNT_A" \
    --arg to "$DEV_ACCOUNT_B" \
    --arg gas_price "$gas_price" \
    '[{from:$from,to:$to,value:"0x1",gasPrice:$gas_price}]')")"
printf '%s\n' "$send_response" >"$LV_CHAIN_EVIDENCE/send-transaction-response.json"
if ! transaction_hash="$(lv_rpc_result "$send_response" "Reth eth_sendTransaction")"; then
  lv_chain_finish \
    "failed" \
    "reth_dev_rpc" \
    "$(jq -cn --arg reason "transaction_submission_rejected" \
      --argjson response "$send_response" '{reason:$reason,response:$response}')"
  exit 1
fi
if [[ "$transaction_hash" != 0x* ]]; then
  printf 'Reth returned an invalid transaction hash: %s\n' "$transaction_hash" >&2
  exit 1
fi
receipt="$(wait_for_receipt "$transaction_hash")"
receipt_block_hash="$(jq -r '.result.blockHash' <<<"$receipt")"
receipt_status="$(jq -r '.result.status' <<<"$receipt")"
[[ "$receipt_status" == "0x1" ]]
head_after_transaction="$(lv_rpc "$RETH_RPC_URL" eth_blockNumber '[]' | jq -r '.result')"

kill "$RETH_PID"
wait "$RETH_PID" || true
for _ in {1..30}; do
  kill -0 "$RETH_PID" 2>/dev/null || break
  sleep 0.1
done
lv_require_loopback_port tcp "$RETH_RPC_PORT"
start_reth reth-restart.log
head_after_restart="$(lv_rpc "$RETH_RPC_URL" eth_blockNumber '[]' | jq -r '.result')"
[[ "$((16#${head_after_restart#0x}))" -ge "$((16#${head_after_transaction#0x}))" ]]

cargo test \
  -p ethereum-reth-connector \
  -p ethereum-consensus-connector \
  -p evm-canonicality \
  >"$LV_CHAIN_EVIDENCE/ethereum-contract-tests.log" 2>&1
cargo test \
  -p integration-tests \
  --test ethereum_recorded \
  >"$LV_CHAIN_EVIDENCE/ethereum-recorded-test.log" 2>&1

details="$(
  jq -n \
    --arg client "$client_version" \
    --arg release "$RETH_VERSION" \
    --arg archive_sha256 "$RETH_SHA256" \
    --arg local_chain_id "$chain_id" \
    --argjson accounts "$accounts" \
    --arg head_before "$head_before" \
    --arg gas_price "$gas_price" \
    --arg transaction_hash "$transaction_hash" \
    --arg receipt_block_hash "$receipt_block_hash" \
    --arg receipt_status "$receipt_status" \
    --arg head_after_transaction "$head_after_transaction" \
    --arg head_after_restart "$head_after_restart" \
    '{
      client:$client,
      release:$release,
      archive_sha256:$archive_sha256,
      local_chain_id:$local_chain_id,
      dev_accounts:$accounts,
      head_before:$head_before,
      gas_price:$gas_price,
      transaction:{
        hash:$transaction_hash,
        block_hash:$receipt_block_hash,
        status:$receipt_status
      },
      head_after_transaction:$head_after_transaction,
      head_after_restart:$head_after_restart,
      runtime_restart:"passed",
      recorded_reth_consensus_test:"passed",
      mainnet_finality_runtime_proven:false
    }'
)"
printf '%s\n' "$details" >"$LV_CHAIN_EVIDENCE/evidence.json"
lv_chain_finish "passed" "reth_dev_rpc_restart_plus_recorded_el_cl_semantics" "$details"
