#!/usr/bin/env bash

set -Eeuo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPOSITORY_ROOT
readonly LOCAL_CHECKS=(
  rust_format
  evm_clippy
  source_capture
  ethereum_recorded
  bsc_recorded
  evm_raw_first
)
readonly PRODUCTION_BLOCKERS=(
  owned_reth_consensus_pair
  official_bsc_mainnet_node
  independent_secondary_sources
  live_reorg_and_finality_fault_proof
  load_soak_and_disaster_recovery
)

dry_run=0
evidence_dir=""

usage() {
  cat <<'USAGE'
Usage: scripts/verify-evm-foundation.sh [--dry-run] [--evidence-dir ABSOLUTE_PATH]

Runs the locally owned Ethereum/BSC raw-first foundation checks and writes
evidence. A passing local gate does not promote either chain: owned mainnet
nodes, independent reconciliation sources, live fault proof, and operational
qualification remain HOLD gates.
USAGE
}

while (($# > 0)); do
  case "$1" in
    --dry-run)
      dry_run=1
      shift
      ;;
    --evidence-dir)
      (($# >= 2)) || {
        printf '%s\n' '--evidence-dir requires a value' >&2
        exit 64
      }
      evidence_dir="$2"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      printf 'unknown argument: %s\n' "$1" >&2
      usage >&2
      exit 64
      ;;
  esac
done

if [[ -n "$evidence_dir" && "$evidence_dir" != /* ]]; then
  printf 'evidence directory must be absolute: %s\n' "$evidence_dir" >&2
  exit 64
fi

json_array() {
  if (($# == 0)); then
    printf '%s\n' '[]'
    return
  fi
  printf '%s\n' "$@" | jq -R . | jq -s .
}

if ((dry_run == 1)); then
  jq -n \
    --arg mode "dry_run" \
    --arg scope "evm_raw_first_local_gate" \
    --argjson local_checks "$(json_array "${LOCAL_CHECKS[@]}")" \
    --arg promotion_verdict "hold" \
    --argjson production_blockers "$(json_array "${PRODUCTION_BLOCKERS[@]}")" \
    '{
      mode: $mode,
      scope: $scope,
      local_checks: $local_checks,
      promotion_verdict: $promotion_verdict,
      production_blockers: $production_blockers
    }'
  exit 0
fi

for required in cargo git jq rustc tee; do
  command -v "$required" >/dev/null 2>&1 || {
    printf 'required command is unavailable: %s\n' "$required" >&2
    exit 69
  }
done

build_sha="$(git -C "$REPOSITORY_ROOT" rev-parse HEAD)"
if [[ -z "$evidence_dir" ]]; then
  evidence_dir="$REPOSITORY_ROOT/artifacts/certification/$build_sha/evm-foundation-local"
fi
mkdir -p "$evidence_dir/logs"

completed_checks=()
run_check() {
  local check_name="$1"
  shift
  local log_file="$evidence_dir/logs/$check_name.log"
  printf 'EVM foundation local check: %s\n' "$check_name"
  if ! "$@" 2>&1 | tee "$log_file"; then
    jq -n \
      --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
      --arg build_sha "$build_sha" \
      --arg failed_check "$check_name" \
      --argjson completed_checks "$(json_array "${completed_checks[@]}")" \
      '{
        generated_at: $generated_at,
        build_sha: $build_sha,
        scope: "evm_raw_first_local_gate",
        local_gate: "failed",
        failed_check: $failed_check,
        completed_checks: $completed_checks,
        promotion_verdict: "hold"
      }' >"$evidence_dir/result.json"
    return 1
  fi
  completed_checks+=("$check_name")
}

cd "$REPOSITORY_ROOT"
run_check rust_format cargo fmt --all -- --check
run_check evm_clippy \
  cargo clippy \
  -p source-capture \
  -p storage-ports \
  -p ethereum-reth-connector \
  -p ethereum-consensus-connector \
  -p bsc-connector \
  -p evm-canonicality \
  -p integration-tests \
  --all-targets --all-features -- -D warnings
run_check source_capture cargo test -p source-capture
run_check ethereum_recorded \
  cargo test -p integration-tests --test ethereum_recorded
run_check bsc_recorded \
  cargo test -p integration-tests --test bsc_recorded
run_check evm_raw_first \
  cargo test -p integration-tests --test evm_raw_first

working_tree_dirty=false
if [[ -n "$(git status --short)" ]]; then
  working_tree_dirty=true
fi
rust_toolchain="$(rustc --version)"
jq -n \
  --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg build_sha "$build_sha" \
  --arg rust_toolchain "$rust_toolchain" \
  --argjson working_tree_dirty "$working_tree_dirty" \
  --argjson completed_checks "$(json_array "${completed_checks[@]}")" \
  --argjson production_blockers "$(json_array "${PRODUCTION_BLOCKERS[@]}")" \
  '{
    generated_at: $generated_at,
    build_sha: $build_sha,
    rust_toolchain: $rust_toolchain,
    working_tree_dirty: $working_tree_dirty,
    scope: "evm_raw_first_local_gate",
    local_gate: "passed",
    completed_checks: $completed_checks,
    promotion_verdict: "hold",
    production_blockers: $production_blockers
  }' >"$evidence_dir/result.json"

printf 'EVM foundation local evidence: %s\n' "$evidence_dir/result.json"
jq . "$evidence_dir/result.json"
