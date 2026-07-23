#!/usr/bin/env bash

set -Eeuo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
readonly REPOSITORY_ROOT
readonly RUNNER="$REPOSITORY_ROOT/scripts/validate-local.sh"

output="$("$RUNNER" --dry-run --chain all)"
jq -e '
  .mode == "dry_run" and
  .keep_runtime == false and
  .chains == ["bitcoin", "ethereum", "bsc", "solana", "platform"]
' <<<"$output" >/dev/null

single="$("$RUNNER" --dry-run --chain solana --keep-runtime)"
jq -e '
  .mode == "dry_run" and
  .keep_runtime == true and
  .chains == ["solana"]
' <<<"$single" >/dev/null

if "$RUNNER" --dry-run --chain unsupported >/dev/null 2>&1; then
  printf '%s\n' 'unsupported chain unexpectedly succeeded' >&2
  exit 1
fi

if "$RUNNER" --dry-run --evidence-dir relative/path >/dev/null 2>&1; then
  printf '%s\n' 'relative evidence directory unexpectedly succeeded' >&2
  exit 1
fi

bitcoin_runner="$REPOSITORY_ROOT/scripts/local-validation/bitcoin.sh"
if ! grep -Fq "'[regtest]'" "$bitcoin_runner"; then
  printf '%s\n' 'Bitcoin network-specific ports are not scoped under [regtest]' >&2
  exit 1
fi

grep -Fq 'readonly RETH_VERSION="2.2.0"' \
  "$REPOSITORY_ROOT/scripts/local-validation/ethereum.sh"
grep -Fq 'readonly BSC_VERSION="1.7.3"' \
  "$REPOSITORY_ROOT/scripts/local-validation/bsc.sh"
grep -Fq 'readonly AGAVE_VERSION="4.1.2"' \
  "$REPOSITORY_ROOT/scripts/local-validation/solana.sh"
grep -Fq 'readonly SOLANA_DYNAMIC_PORT_END="18936"' \
  "$REPOSITORY_ROOT/scripts/local-validation/solana.sh"
grep -Fq 'multichain-validation-' \
  "$REPOSITORY_ROOT/scripts/local-validation/platform.sh"
if grep -Eq '^[[:space:]]*--dev([[:space:]\\]|$)' \
  "$REPOSITORY_ROOT/scripts/local-validation/bsc.sh"; then
  printf '%s\n' 'BSC 1.7.3 no longer supports the --dev flag' >&2
  exit 1
fi

if grep -REq '\$\{[^}]+,,\}' "$REPOSITORY_ROOT/scripts/local-validation"; then
  printf '%s\n' 'Bash 4 lowercase expansion is not portable to macOS Bash 3.2' >&2
  exit 1
fi
if grep -REq '\|[[:space:]]*head([[:space:]]|$)|head -1' \
  "$REPOSITORY_ROOT/scripts/local-validation"; then
  printf '%s\n' 'head may turn a successful producer into SIGPIPE under pipefail' >&2
  exit 1
fi

# shellcheck source=scripts/local-validation/common.sh
source "$REPOSITORY_ROOT/scripts/local-validation/common.sh"
rpc_result="$(lv_rpc_result \
  '{"jsonrpc":"2.0","id":1,"result":"0xabc"}' \
  'contract success')"
[[ "$rpc_result" == "0xabc" ]]
if lv_rpc_result \
  '{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"rejected"}}' \
  'contract failure' \
  >/dev/null 2>&1; then
  printf '%s\n' 'JSON-RPC error unexpectedly produced a result' >&2
  exit 1
fi

printf '%s\n' 'local validation runner contract: passed'
