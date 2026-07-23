#!/usr/bin/env bash

set -Eeuo pipefail

SCRIPT_DIRECTORY="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIRECTORY
# shellcheck source=scripts/local-validation/common.sh
source "$SCRIPT_DIRECTORY/common.sh"

readonly AGAVE_VERSION="4.1.2"
readonly AGAVE_ARCHIVE="solana-release-aarch64-apple-darwin.tar.bz2"
readonly AGAVE_URL="https://github.com/anza-xyz/agave/releases/download/v4.1.2/$AGAVE_ARCHIVE"
readonly AGAVE_SHA256="51a44318e6fb8be0cfa69cdfdb3252f4c76a5eb2866740694e91de3d2fc5a75b"
readonly SOLANA_RPC_PORT="18899"
readonly SOLANA_RPC_URL="http://127.0.0.1:$SOLANA_RPC_PORT"
readonly SOLANA_FAUCET_PORT="18901"
readonly SOLANA_GOSSIP_PORT="18902"
readonly SOLANA_DYNAMIC_PORT_START="18910"
readonly SOLANA_DYNAMIC_PORT_END="18936"

wait_for_transaction() {
  local signature="$1"
  local response
  local attempt
  local params
  params="$(jq -cn \
    --arg signature "$signature" \
    '[$signature,{commitment:"confirmed",encoding:"json"}]')"
  for attempt in {1..60}; do
    response="$(lv_rpc "$SOLANA_RPC_URL" getTransaction "$params")"
    if [[ "$(jq -r '.result.slot // empty' <<<"$response")" != "" ]]; then
      printf '%s\n' "$response"
      return 0
    fi
    sleep 1
  done
  printf 'Solana transaction confirmation timeout: %s\n' "$signature" >&2
  return 1
}

lv_chain_init solana
lv_require_arm64_macos
lv_require_capacity
for command_name in curl jq shasum tar cargo awk; do
  lv_require_command "$command_name"
done
lv_require_loopback_port tcp "$SOLANA_RPC_PORT"
lv_require_loopback_port tcp "$SOLANA_FAUCET_PORT"
lv_require_loopback_port tcp "$SOLANA_GOSSIP_PORT"
lv_require_loopback_port udp "$SOLANA_GOSSIP_PORT"
for ((port = SOLANA_DYNAMIC_PORT_START; port <= SOLANA_DYNAMIC_PORT_END; port += 1)); do
  lv_require_loopback_port tcp "$port"
  lv_require_loopback_port udp "$port"
done

archive_path="$LV_CHAIN_RUNTIME/$AGAVE_ARCHIVE"
lv_download_verified "$AGAVE_URL" "$AGAVE_SHA256" "$archive_path"
mkdir -p "$LV_CHAIN_RUNTIME/release"
tar -xjf "$archive_path" -C "$LV_CHAIN_RUNTIME/release"

validator_binary="$(
  find "$LV_CHAIN_RUNTIME/release" \
    -type f -name solana-test-validator -perm -111 -print -quit
)"
solana_binary="$(
  find "$LV_CHAIN_RUNTIME/release" \
    -type f -name solana -perm -111 -print -quit
)"
keygen_binary="$(
  find "$LV_CHAIN_RUNTIME/release" \
    -type f -name solana-keygen -perm -111 -print -quit
)"
[[ -x "$validator_binary" && -x "$solana_binary" && -x "$keygen_binary" ]]

validator_version="$("$validator_binary" --version)"
cli_version="$("$solana_binary" --version)"
[[ "$validator_version" == *"$AGAVE_VERSION"* ]]
[[ "$cli_version" == *"$AGAVE_VERSION"* ]]

"$validator_binary" \
  --ledger "$LV_CHAIN_RUNTIME/ledger" \
  --rpc-port "$SOLANA_RPC_PORT" \
  --faucet-port "$SOLANA_FAUCET_PORT" \
  --gossip-port "$SOLANA_GOSSIP_PORT" \
  --dynamic-port-range "$SOLANA_DYNAMIC_PORT_START-$SOLANA_DYNAMIC_PORT_END" \
  --bind-address 127.0.0.1 \
  --limit-ledger-size 10000 \
  --reset \
  >"$LV_CHAIN_EVIDENCE/solana-test-validator.log" 2>&1 &
lv_register_pid "$!"
health_response="$(lv_wait_for_rpc "$SOLANA_RPC_URL" getHealth '[]' 120)"
[[ "$(jq -r '.result' <<<"$health_response")" == "ok" ]]

sender_keypair="$LV_CHAIN_RUNTIME/sender.json"
recipient_keypair="$LV_CHAIN_RUNTIME/recipient.json"
"$keygen_binary" new \
  --no-bip39-passphrase \
  --silent \
  --force \
  --outfile "$sender_keypair"
"$keygen_binary" new \
  --no-bip39-passphrase \
  --silent \
  --force \
  --outfile "$recipient_keypair"
sender="$("$keygen_binary" pubkey "$sender_keypair")"
recipient="$("$keygen_binary" pubkey "$recipient_keypair")"

slot_before="$(lv_rpc \
  "$SOLANA_RPC_URL" \
  getSlot \
  '[{"commitment":"confirmed"}]' |
  jq -r '.result')"
latest_blockhash_response="$(lv_rpc \
  "$SOLANA_RPC_URL" \
  getLatestBlockhash \
  '[{"commitment":"confirmed"}]')"
latest_blockhash="$(jq -r '.result.value.blockhash' <<<"$latest_blockhash_response")"

"$solana_binary" \
  airdrop 10 "$sender" \
  --url "$SOLANA_RPC_URL" \
  --commitment confirmed \
  >"$LV_CHAIN_EVIDENCE/airdrop.log" 2>&1
sender_balance_before="$(
  "$solana_binary" balance "$sender" \
    --url "$SOLANA_RPC_URL" \
    --lamports |
    awk '{ print $1 }'
)"

transfer_output="$(
  "$solana_binary" \
    transfer "$recipient" 1 \
    --from "$sender_keypair" \
    --fee-payer "$sender_keypair" \
    --allow-unfunded-recipient \
    --url "$SOLANA_RPC_URL" \
    --commitment confirmed
)"
printf '%s\n' "$transfer_output" >"$LV_CHAIN_EVIDENCE/transfer.log"
signature="$(
  awk '/Signature:/ { print $2 }' <<<"$transfer_output" |
    tail -1
)"
if [[ -z "$signature" ]]; then
  printf 'Solana transfer did not return a signature: %s\n' "$transfer_output" >&2
  exit 1
fi

transaction_response="$(wait_for_transaction "$signature")"
printf '%s\n' "$transaction_response" >"$LV_CHAIN_EVIDENCE/transaction.json"
transaction_slot="$(jq -r '.result.slot' <<<"$transaction_response")"
transaction_error="$(jq -c '.result.meta.err' <<<"$transaction_response")"
[[ "$transaction_error" == "null" ]]
slot_after="$(lv_rpc \
  "$SOLANA_RPC_URL" \
  getSlot \
  '[{"commitment":"confirmed"}]' |
  jq -r '.result')"
recipient_balance="$(
  "$solana_binary" balance "$recipient" \
    --url "$SOLANA_RPC_URL" \
    --lamports |
    awk '{ print $1 }'
)"

cargo test \
  -p solana-domain \
  -p solana-yellowstone-connector \
  -p solana-canonicality \
  -p solana-decoder \
  >"$LV_CHAIN_EVIDENCE/solana-contract-tests.log" 2>&1
cargo test \
  -p integration-tests \
  --test four_chain_replay \
  >"$LV_CHAIN_EVIDENCE/four-chain-replay.log" 2>&1

details="$(
  jq -n \
    --arg validator_version "$validator_version" \
    --arg cli_version "$cli_version" \
    --arg release "$AGAVE_VERSION" \
    --arg archive_sha256 "$AGAVE_SHA256" \
    --arg health "$(jq -r '.result' <<<"$health_response")" \
    --arg sender "$sender" \
    --arg recipient "$recipient" \
    --arg sender_balance_before "$sender_balance_before" \
    --arg recipient_balance "$recipient_balance" \
    --arg latest_blockhash "$latest_blockhash" \
    --arg signature "$signature" \
    --argjson slot_before "$slot_before" \
    --argjson transaction_slot "$transaction_slot" \
    --argjson slot_after "$slot_after" \
    '{
      validator_version:$validator_version,
      cli_version:$cli_version,
      release:$release,
      archive_sha256:$archive_sha256,
      health:$health,
      sender:$sender,
      recipient:$recipient,
      sender_balance_before_lamports:$sender_balance_before,
      recipient_balance_lamports:$recipient_balance,
      latest_blockhash:$latest_blockhash,
      transfer_signature:$signature,
      slot_before:$slot_before,
      transaction_slot:$transaction_slot,
      slot_after:$slot_after,
      transaction_error:null,
      local_validator_runtime:"passed",
      recorded_fork_and_yellowstone_contracts:"passed",
      independent_yellowstone_sources_runtime_proven:false,
      mainnet_beta_runtime_proven:false
    }'
)"
printf '%s\n' "$details" >"$LV_CHAIN_EVIDENCE/evidence.json"
lv_chain_finish \
  "passed" \
  "agave_local_validator_transfer_plus_recorded_mainnet_semantics" \
  "$details"
