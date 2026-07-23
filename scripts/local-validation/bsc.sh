#!/usr/bin/env bash

set -Eeuo pipefail

SCRIPT_DIRECTORY="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIRECTORY
# shellcheck source=scripts/local-validation/common.sh
source "$SCRIPT_DIRECTORY/common.sh"

readonly BSC_VERSION="1.7.3"
readonly BSC_BINARY_NAME="geth_macos"
readonly BSC_URL="https://github.com/bnb-chain/bsc/releases/download/v1.7.3/$BSC_BINARY_NAME"
readonly BSC_SHA256="ab956d653a3361f9772e8c7ecacc7f2f56a9ae8f1708841e59a85269854a5fd6"
readonly BSC_RPC_PORT="29545"
readonly BSC_RPC_URL="http://127.0.0.1:$BSC_RPC_PORT"
readonly BSC_P2P_PORT="30382"
readonly BSC_LOCAL_CHAIN_ID="714"

wait_for_receipt() {
  local transaction_hash="$1"
  local attempt
  local receipt
  for attempt in {1..60}; do
    receipt="$(lv_rpc \
      "$BSC_RPC_URL" \
      eth_getTransactionReceipt \
      "$(jq -cn --arg hash "$transaction_hash" '[$hash]')")"
    if [[ "$(jq -r '.result.blockHash // empty' <<<"$receipt")" != "" ]]; then
      printf '%s\n' "$receipt"
      return 0
    fi
    sleep 1
  done
  printf 'BSC transaction receipt timeout: %s\n' "$transaction_hash" >&2
  return 1
}

lv_chain_init bsc
lv_require_arm64_macos
lv_require_capacity
for command_name in curl jq shasum file cargo; do
  lv_require_command "$command_name"
done
lv_require_loopback_port tcp "$BSC_RPC_PORT"
lv_require_loopback_port tcp "$BSC_P2P_PORT"
lv_require_loopback_port udp "$BSC_P2P_PORT"

bsc_binary="$LV_CHAIN_RUNTIME/bsc-geth"
lv_download_verified "$BSC_URL" "$BSC_SHA256" "$bsc_binary"
chmod +x "$bsc_binary"
binary_architecture="$(file -b "$bsc_binary")"
version_output="$("$bsc_binary" version)"
grep -Fq "Version: $BSC_VERSION" <<<"$version_output"

node_directory="$LV_CHAIN_RUNTIME/bsc-data"
password_file="$LV_CHAIN_RUNTIME/empty-password"
: >"$password_file"
account_output="$(
  "$bsc_binary" account new \
    --datadir "$node_directory" \
    --password "$password_file" \
    2>"$LV_CHAIN_EVIDENCE/account-create.log"
)"
validator_address="$(
  awk '/Public address of the key:/ { print $NF }' <<<"$account_output" |
    tr '[:upper:]' '[:lower:]'
)"
if [[ "$validator_address" != 0x* ]]; then
  printf 'BSC validator account creation failed: %s\n' "$account_output" >&2
  exit 1
fi
validator_without_prefix="${validator_address#0x}"
genesis_timestamp="$(printf '0x%x' "$(date +%s)")"
genesis_path="$LV_CHAIN_RUNTIME/genesis.json"
jq -n \
  --arg chain_id "$BSC_LOCAL_CHAIN_ID" \
  --arg timestamp "$genesis_timestamp" \
  --arg validator "$validator_without_prefix" \
  '{
    config:{
      chainId:($chain_id | tonumber),
      homesteadBlock:0,
      eip150Block:0,
      eip150Hash:"0x0000000000000000000000000000000000000000000000000000000000000000",
      eip155Block:0,
      eip158Block:0,
      byzantiumBlock:0,
      constantinopleBlock:0,
      petersburgBlock:0,
      istanbulBlock:0,
      muirGlacierBlock:0,
      ramanujanBlock:0,
      nielsBlock:0,
      mirrorSyncBlock:1,
      brunoBlock:1,
      eulerBlock:2,
      gibbsBlock:3,
      parlia:{period:1,epoch:200}
    },
    nonce:"0x0",
    timestamp:$timestamp,
    extraData:("0x" + ("0" * 64) + $validator + ("0" * 130)),
    gasLimit:"0x2625a00",
    difficulty:"0x1",
    mixHash:"0x0000000000000000000000000000000000000000000000000000000000000000",
    coinbase:"0xffffFFFfFFffffffffffffffFfFFFfffFFFfFFfE",
    alloc:{($validator):{balance:"0x3635C9ADC5DEA00000"}}
  }' >"$genesis_path"
"$bsc_binary" \
  --datadir "$node_directory" \
  init "$genesis_path" \
  >"$LV_CHAIN_EVIDENCE/genesis-init.log" 2>&1

"$bsc_binary" \
  --datadir "$node_directory" \
  --networkid "$BSC_LOCAL_CHAIN_ID" \
  --mine \
  --miner.etherbase "$validator_address" \
  --unlock "$validator_address" \
  --password "$password_file" \
  --allow-insecure-unlock \
  --cache 256 \
  --http \
  --http.addr 127.0.0.1 \
  --http.port "$BSC_RPC_PORT" \
  --http.api eth,net,web3,txpool,parlia \
  --http.vhosts localhost,127.0.0.1 \
  --ipcdisable \
  --nodiscover \
  --nat none \
  --port "$BSC_P2P_PORT" \
  >"$LV_CHAIN_EVIDENCE/bsc.log" 2>&1 &
lv_register_pid "$!"
lv_wait_for_rpc "$BSC_RPC_URL" web3_clientVersion '[]' 90 >/dev/null

client_version="$(lv_rpc "$BSC_RPC_URL" web3_clientVersion '[]' | jq -r '.result')"
chain_id="$(lv_rpc "$BSC_RPC_URL" eth_chainId '[]' | jq -r '.result')"
accounts="$(lv_rpc "$BSC_RPC_URL" eth_accounts '[]' | jq -c '.result')"
from="$(jq -r '.[0] // empty' <<<"$accounts")"
to="0x000000000000000000000000000000000000dead"
[[ "$from" == 0x* && "$to" == 0x* ]]
balance_before="$(lv_rpc \
  "$BSC_RPC_URL" \
  eth_getBalance \
  "$(jq -cn --arg address "$from" '[$address,"latest"]')" |
  jq -r '.result')"
gas_price="$(lv_rpc "$BSC_RPC_URL" eth_gasPrice '[]' | jq -r '.result')"

send_response="$(lv_rpc \
  "$BSC_RPC_URL" \
  eth_sendTransaction \
  "$(jq -cn --arg from "$from" --arg to "$to" --arg gas_price "$gas_price" \
    '[{from:$from,to:$to,value:"0x1",gasPrice:$gas_price}]')")"
printf '%s\n' "$send_response" >"$LV_CHAIN_EVIDENCE/send-transaction-response.json"
if ! transaction_hash="$(lv_rpc_result "$send_response" "BSC eth_sendTransaction")"; then
  lv_chain_finish \
    "failed" \
    "official_bsc_local_parlia_rpc" \
    "$(jq -cn --arg reason "transaction_submission_rejected" \
      --argjson response "$send_response" '{reason:$reason,response:$response}')"
  exit 1
fi
if [[ "$transaction_hash" != 0x* ]]; then
  printf 'BSC returned an invalid transaction hash: %s\n' "$transaction_hash" >&2
  exit 1
fi
receipt="$(wait_for_receipt "$transaction_hash")"
receipt_block_hash="$(jq -r '.result.blockHash' <<<"$receipt")"
receipt_status="$(jq -r '.result.status' <<<"$receipt")"
[[ "$receipt_status" == "0x1" ]]
head="$(lv_rpc "$BSC_RPC_URL" eth_blockNumber '[]' | jq -r '.result')"

cargo test \
  -p bsc-connector \
  >"$LV_CHAIN_EVIDENCE/bsc-connector-tests.log" 2>&1
cargo test \
  -p evm-canonicality \
  --test bsc \
  >"$LV_CHAIN_EVIDENCE/bsc-canonicality-tests.log" 2>&1
cargo test \
  -p integration-tests \
  --test bsc_recorded \
  >"$LV_CHAIN_EVIDENCE/bsc-recorded-test.log" 2>&1

details="$(
  jq -n \
    --arg client "$client_version" \
    --arg release "$BSC_VERSION" \
    --arg binary_sha256 "$BSC_SHA256" \
    --arg binary_architecture "$binary_architecture" \
    --arg local_chain_id "$chain_id" \
    --arg validator_address "$validator_address" \
    --arg balance_before "$balance_before" \
    --arg gas_price "$gas_price" \
    --arg transaction_hash "$transaction_hash" \
    --arg receipt_block_hash "$receipt_block_hash" \
    --arg receipt_status "$receipt_status" \
    --arg head "$head" \
    '{
      client:$client,
      release:$release,
      binary_sha256:$binary_sha256,
      binary_architecture:$binary_architecture,
      local_chain_id:$local_chain_id,
      validator_address:$validator_address,
      funded_balance_before:$balance_before,
      gas_price:$gas_price,
      transaction:{
        hash:$transaction_hash,
        block_hash:$receipt_block_hash,
        status:$receipt_status
      },
      head:$head,
      official_client_runtime:"passed",
      recorded_chain_56_native_finality_test:"passed",
      local_fast_finality_runtime_proven:false,
      mainnet_chain_id_runtime_proven:false
    }'
)"
printf '%s\n' "$details" >"$LV_CHAIN_EVIDENCE/evidence.json"
lv_chain_finish \
  "passed" \
  "official_bsc_local_parlia_rpc_plus_recorded_chain56_finality" \
  "$details"
