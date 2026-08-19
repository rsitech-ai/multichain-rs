#!/usr/bin/env bash

set -Eeuo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPOSITORY_ROOT
readonly MANIFEST_VALIDATOR="$REPOSITORY_ROOT/scripts/validate-project-manifest.py"
readonly LOCAL_CHECKS=(
  rust_format
  bitcoin_clippy
  bitcoin_unit_tests
  bitcoin_multi_observer_fixture
  alert_preview_http
  alert_postgres_transaction
)
readonly PRODUCTION_BLOCKERS=(
  three_independent_mainnet_observers
  production_ha_and_security
  load_and_soak_targets
  disaster_recovery_drill
  end_to_end_web_correction_proof
)

dry_run=0
evidence_dir=""

usage() {
  cat <<'USAGE'
Usage: scripts/verify-phase1.sh [--dry-run] [--evidence-dir ABSOLUTE_PATH]

Runs the locally owned Bitcoin Phase 1 checks and writes evidence. A passing
local gate does not promote Phase 1: production observer, HA/security,
performance, disaster-recovery, and web correction evidence remain HOLD gates.
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
    --arg scope "bitcoin_phase1_local_gate" \
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

for required in cargo git jq python3 tee; do
  command -v "$required" >/dev/null 2>&1 || {
    printf 'required command is unavailable: %s\n' "$required" >&2
    exit 69
  }
done
if [[ ! -f "$MANIFEST_VALIDATOR" ]]; then
  printf 'project manifest validator is unavailable: %s\n' "$MANIFEST_VALIDATOR" >&2
  exit 69
fi

build_sha="$(git -C "$REPOSITORY_ROOT" rev-parse HEAD)"
if [[ -z "$evidence_dir" ]]; then
  evidence_dir="$REPOSITORY_ROOT/artifacts/certification/$build_sha/phase1-local"
fi
mkdir -p "$evidence_dir/logs"

completed_checks=()
run_check() {
  local check_name="$1"
  shift
  local log_file="$evidence_dir/logs/$check_name.log"
  printf 'phase1 local check: %s\n' "$check_name"
  if ! "$@" 2>&1 | tee "$log_file"; then
    jq -n \
      --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
      --arg build_sha "$build_sha" \
      --arg failed_check "$check_name" \
      --argjson completed_checks "$(json_array "${completed_checks[@]}")" \
      '{
        generated_at: $generated_at,
        build_sha: $build_sha,
        scope: "bitcoin_phase1_local_gate",
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
run_check bitcoin_clippy \
  cargo clippy \
  -p bitcoin-domain \
  -p bitcoin-canonicality \
  -p bitcoin-mempool \
  -p alert-engine \
  -p query-api \
  --all-targets --all-features -- -D warnings
run_check bitcoin_unit_tests \
  cargo test \
  -p bitcoin-domain \
  -p bitcoin-canonicality \
  -p bitcoin-mempool \
  -p alert-engine
run_check bitcoin_multi_observer_fixture \
  cargo test -p integration-tests --test bitcoin_multi_observer_mempool
run_check alert_preview_http \
  cargo test -p query-api --test alert_preview_http
run_check alert_postgres_transaction \
  env MULTICHAIN_REQUIRE_INFRA=1 \
  cargo test -p alert-engine --test postgres_alert_outbox

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
    scope: "bitcoin_phase1_local_gate",
    local_gate: "passed",
    completed_checks: $completed_checks,
    promotion_verdict: "hold",
    production_blockers: $production_blockers
  }' >"$evidence_dir/result.json"

printf 'phase1 local evidence: %s\n' "$evidence_dir/result.json"
jq . "$evidence_dir/result.json"
