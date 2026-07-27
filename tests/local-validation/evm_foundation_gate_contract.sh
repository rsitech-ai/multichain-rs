#!/usr/bin/env bash

set -Eeuo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
readonly REPOSITORY_ROOT
readonly RUNNER="$REPOSITORY_ROOT/scripts/verify-evm-foundation.sh"

output="$("$RUNNER" --dry-run)"
jq -e '
  .mode == "dry_run" and
  .scope == "evm_raw_first_local_gate" and
  .local_checks == [
    "rust_format",
    "evm_clippy",
    "source_capture",
    "source_runtime",
    "connector_live_plans",
    "ethereum_recorded",
    "bsc_recorded",
    "evm_raw_first",
    "evm_live_polling"
  ] and
  .promotion_verdict == "hold" and
  .production_blockers == [
    "owned_reth_consensus_pair",
    "official_bsc_mainnet_node",
    "independent_secondary_sources",
    "live_reorg_and_finality_fault_proof",
    "load_soak_and_disaster_recovery"
  ]
' <<<"$output" >/dev/null

if "$RUNNER" --dry-run --evidence-dir relative/path >/dev/null 2>&1; then
  printf '%s\n' 'relative evidence directory unexpectedly succeeded' >&2
  exit 1
fi

printf '%s\n' 'EVM foundation local gate contract: passed'
