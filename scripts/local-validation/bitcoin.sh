#!/usr/bin/env bash

set -Eeuo pipefail

SCRIPT_DIRECTORY="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIRECTORY
# shellcheck source=scripts/local-validation/common.sh
source "$SCRIPT_DIRECTORY/common.sh"

readonly BITCOIN_VERSION="31.1"
readonly BITCOIN_ARCHIVE="bitcoin-31.1-arm64-apple-darwin.tar.gz"
readonly BITCOIN_URL="https://bitcoincore.org/bin/bitcoin-core-31.1/$BITCOIN_ARCHIVE"
readonly BITCOIN_SHA256="16a097c09fbd7eb78b240ce1dae123663ea2e5e377cfd6a951e71e227e23cf2f"
readonly SOURCE_IDS=(bitcoin-local-a bitcoin-local-b bitcoin-local-c)
readonly RPC_PORTS=(19443 19453 19463)
readonly P2P_PORTS=(19444 19454 19464)
readonly ZMQ_PORTS=(29432 29433 29434)

BITCOIN_CLI=""
BITCOIND=""
DATADIRS=()

btc_cli() {
  local observer_index="$1"
  shift
  "$BITCOIN_CLI" -regtest -datadir="${DATADIRS[$observer_index]}" "$@"
}

btc_wallet_cli() {
  local observer_index="$1"
  local wallet="$2"
  shift 2
  "$BITCOIN_CLI" \
    -regtest \
    -datadir="${DATADIRS[$observer_index]}" \
    "-rpcwallet=$wallet" \
    "$@"
}

btc_mempool_size() {
  local observer_index="$1"
  btc_cli "$observer_index" getmempoolinfo | jq -r '.size'
}

write_config() {
  local observer_index="$1"
  local datadir="${DATADIRS[$observer_index]}"
  mkdir -p "$datadir"
  {
    printf '%s\n' \
      'regtest=1' \
      'server=1' \
      'daemon=0' \
      'listen=1' \
      'discover=0' \
      'dnsseed=0' \
      'fixedseeds=0' \
      'txindex=1' \
      'fallbackfee=0.00001000' \
      'dbcache=128' \
      '[regtest]' \
      'rpcbind=127.0.0.1' \
      'rpcallowip=127.0.0.1' \
      'bind=127.0.0.1' \
      "port=${P2P_PORTS[$observer_index]}" \
      "rpcport=${RPC_PORTS[$observer_index]}" \
      "zmqpubrawtx=tcp://127.0.0.1:${ZMQ_PORTS[$observer_index]}" \
      "zmqpubrawblock=tcp://127.0.0.1:${ZMQ_PORTS[$observer_index]}" \
      "zmqpubsequence=tcp://127.0.0.1:${ZMQ_PORTS[$observer_index]}"
  } >"$datadir/bitcoin.conf"
}

wait_for_tip() {
  local observer_index="$1"
  local expected_hash="$2"
  lv_wait_until_equal "$expected_hash" 90 btc_cli "$observer_index" getbestblockhash
}

lv_chain_init bitcoin
lv_require_arm64_macos
lv_require_capacity
for command_name in curl jq shasum tar cargo; do
  lv_require_command "$command_name"
done
for port in "${RPC_PORTS[@]}" "${P2P_PORTS[@]}" "${ZMQ_PORTS[@]}"; do
  lv_require_loopback_port tcp "$port"
done

archive_path="$LV_CHAIN_RUNTIME/$BITCOIN_ARCHIVE"
lv_download_verified "$BITCOIN_URL" "$BITCOIN_SHA256" "$archive_path"
mkdir -p "$LV_CHAIN_RUNTIME/release"
tar -xzf "$archive_path" -C "$LV_CHAIN_RUNTIME/release"
BITCOIND="$(find "$LV_CHAIN_RUNTIME/release" -type f -path '*/bin/bitcoind' -print -quit)"
BITCOIN_CLI="$(find "$LV_CHAIN_RUNTIME/release" -type f -path '*/bin/bitcoin-cli' -print -quit)"
[[ -x "$BITCOIND" && -x "$BITCOIN_CLI" ]]

for observer_index in 0 1 2; do
  datadir="$LV_CHAIN_RUNTIME/${SOURCE_IDS[$observer_index]}"
  DATADIRS+=("$datadir")
  write_config "$observer_index"
  "$BITCOIND" \
    -datadir="$datadir" \
    -conf="$datadir/bitcoin.conf" \
    -printtoconsole=1 \
    >"$LV_CHAIN_EVIDENCE/${SOURCE_IDS[$observer_index]}.log" 2>&1 &
  lv_register_pid "$!"
done

for observer_index in 0 1 2; do
  btc_cli "$observer_index" -rpcwait -rpcwaittimeout=60 getblockchaininfo >/dev/null
done

btc_cli 1 addnode "127.0.0.1:${P2P_PORTS[0]}" onetry
btc_cli 2 addnode "127.0.0.1:${P2P_PORTS[0]}" onetry

btc_cli 0 createwallet validator_a >/dev/null
btc_cli 2 createwallet validator_c >/dev/null
miner_a="$(btc_wallet_cli 0 validator_a getnewaddress)"
miner_c="$(btc_wallet_cli 2 validator_c getnewaddress)"
btc_cli 0 generatetoaddress 101 "$miner_a" >/dev/null
shared_hash="$(btc_cli 0 getbestblockhash)"
wait_for_tip 1 "$shared_hash"
wait_for_tip 2 "$shared_hash"

btc_cli 2 setnetworkactive false >/dev/null
destination="$(btc_wallet_cli 0 validator_a getnewaddress)"
txid="$(btc_wallet_cli 0 validator_a sendtoaddress "$destination" 1.0)"
lv_wait_until_equal "1" 30 btc_mempool_size 1
count_a="$(btc_mempool_size 0)"
count_b="$(btc_mempool_size 1)"
count_c="$(btc_mempool_size 2)"
[[ "$count_a" == "1" && "$count_b" == "1" && "$count_c" == "0" ]]

raw_transaction="$(btc_cli 0 getrawtransaction "$txid")"
btc_cli 2 sendrawtransaction "$raw_transaction" >/dev/null
lv_wait_until_equal "1" 30 btc_mempool_size 2
reconciled_count_c="$(btc_mempool_size 2)"

btc_cli 0 generatetoaddress 2 "$miner_a" >/dev/null
branch_a_hash="$(btc_cli 0 getbestblockhash)"
wait_for_tip 1 "$branch_a_hash"
btc_cli 2 generatetoaddress 3 "$miner_c" >/dev/null
branch_c_hash="$(btc_cli 2 getbestblockhash)"
[[ "$branch_a_hash" != "$branch_c_hash" ]]

btc_cli 2 setnetworkactive true >/dev/null
btc_cli 2 addnode "127.0.0.1:${P2P_PORTS[0]}" onetry
wait_for_tip 0 "$branch_c_hash"
wait_for_tip 1 "$branch_c_hash"

cookie_path="${DATADIRS[0]}/regtest/.cookie"
[[ -s "$cookie_path" ]]
BITCOIN_REGTEST_RPC_URL="http://127.0.0.1:${RPC_PORTS[0]}" \
BITCOIN_REGTEST_COOKIE="$cookie_path" \
  cargo test \
    -p integration-tests \
    --test bitcoin_connector_regtest \
    -- \
    --nocapture \
    >"$LV_CHAIN_EVIDENCE/connector-test.log" 2>&1

version_line="$("$BITCOIND" --version | sed -n '1p')"
final_height="$(btc_cli 0 getblockcount)"
final_hash="$(btc_cli 0 getbestblockhash)"
chain_tips="$(btc_cli 0 getchaintips)"

details="$(
  jq -n \
    --arg client "$version_line" \
    --arg release "$BITCOIN_VERSION" \
    --arg archive_sha256 "$BITCOIN_SHA256" \
    --arg shared_hash "$shared_hash" \
    --arg transaction_id "$txid" \
    --arg branch_a_hash "$branch_a_hash" \
    --arg branch_c_hash "$branch_c_hash" \
    --arg final_hash "$final_hash" \
    --argjson final_height "$final_height" \
    --argjson divergent_mempool_counts "$(jq -cn \
      --argjson a "$count_a" \
      --argjson b "$count_b" \
      --argjson c "$count_c" \
      '{a:$a,b:$b,c:$c}')" \
    --argjson reconciled_count_c "$reconciled_count_c" \
    --argjson source_ids "$(printf '%s\n' "${SOURCE_IDS[@]}" | jq -R . | jq -s .)" \
    --argjson rpc_ports "$(printf '%s\n' "${RPC_PORTS[@]}" | jq -R 'tonumber' | jq -s .)" \
    --argjson zmq_ports "$(printf '%s\n' "${ZMQ_PORTS[@]}" | jq -R 'tonumber' | jq -s .)" \
    --argjson chain_tips "$chain_tips" \
    '{
      client:$client,
      release:$release,
      archive_sha256:$archive_sha256,
      source_ids:$source_ids,
      rpc_ports:$rpc_ports,
      zmq_ports:$zmq_ports,
      initial_shared_tip:$shared_hash,
      mempool_divergence:$divergent_mempool_counts,
      reconciliation:{transaction_id:$transaction_id,observer_c_count:$reconciled_count_c},
      reorg:{old_branch_tip:$branch_a_hash,heavier_branch_tip:$branch_c_hash},
      final_tip:{height:$final_height,hash:$final_hash},
      chain_tips:$chain_tips,
      connector_test:"passed"
    }'
)"
printf '%s\n' "$details" >"$LV_CHAIN_EVIDENCE/evidence.json"
lv_chain_finish "passed" "three_observer_regtest_rpc_zmq_reorg" "$details"
