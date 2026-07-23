#!/usr/bin/env bash

set -Eeuo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPOSITORY_ROOT
readonly SUPPORTED_CHAINS=(bitcoin ethereum bsc solana platform)

chain="all"
dry_run=0
keep_runtime=0
evidence_dir=""

usage() {
  cat <<'USAGE'
Usage: scripts/validate-local.sh [--chain NAME] [--dry-run] [--keep-runtime]
                                 [--evidence-dir ABSOLUTE_PATH]

NAME is one of: all, bitcoin, ethereum, bsc, solana, platform.
USAGE
}

is_supported_chain() {
  local candidate="$1"
  local supported
  for supported in "${SUPPORTED_CHAINS[@]}"; do
    [[ "$candidate" == "$supported" ]] && return 0
  done
  return 1
}

while (($# > 0)); do
  case "$1" in
    --chain)
      (($# >= 2)) || {
        printf '%s\n' '--chain requires a value' >&2
        exit 64
      }
      chain="$2"
      shift 2
      ;;
    --dry-run)
      dry_run=1
      shift
      ;;
    --keep-runtime)
      keep_runtime=1
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

if [[ "$chain" != "all" ]] && ! is_supported_chain "$chain"; then
  printf 'unsupported local validation chain: %s\n' "$chain" >&2
  exit 64
fi

if [[ -n "$evidence_dir" && "$evidence_dir" != /* ]]; then
  printf 'evidence directory must be absolute: %s\n' "$evidence_dir" >&2
  exit 64
fi

selected=()
if [[ "$chain" == "all" ]]; then
  selected=("${SUPPORTED_CHAINS[@]}")
else
  selected=("$chain")
fi

if (( dry_run == 1 )); then
  jq -n \
    --arg mode "dry_run" \
    --argjson chains "$(printf '%s\n' "${selected[@]}" | jq -R . | jq -s .)" \
    --argjson keep_runtime "$keep_runtime" \
    '{mode:$mode,chains:$chains,keep_runtime:($keep_runtime == 1)}'
  exit 0
fi

for required in bash curl jq shasum df lsof; do
  command -v "$required" >/dev/null 2>&1 || {
    printf 'required command is unavailable: %s\n' "$required" >&2
    exit 69
  }
done

timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
if [[ -z "$evidence_dir" ]]; then
  evidence_dir="$REPOSITORY_ROOT/artifacts/local-validation/$timestamp"
fi
mkdir -p "$evidence_dir/results"

run_root="$(mktemp -d "${TMPDIR:-/tmp}/multichain-local-validation.XXXXXX")"
export LV_RUN_ROOT="$run_root"
export LV_EVIDENCE_ROOT="$evidence_dir"
export LV_KEEP_RUNTIME="$keep_runtime"

cleanup() {
  local exit_code=$?
  if (( keep_runtime == 0 )) &&
    [[ "$run_root" == "${TMPDIR:-/tmp}/multichain-local-validation."* ]]; then
    rm -rf -- "$run_root"
  fi
  exit "$exit_code"
}
trap cleanup EXIT

failures=0
for selected_chain in "${selected[@]}"; do
  runner="$REPOSITORY_ROOT/scripts/local-validation/$selected_chain.sh"
  if [[ ! -x "$runner" ]]; then
    jq -n \
      --arg chain "$selected_chain" \
      --arg status "failed" \
      --arg scope "local_runtime" \
      --arg reason "runner_missing_or_not_executable" \
      '{chain:$chain,status:$status,scope:$scope,details:{reason:$reason}}' \
      >"$evidence_dir/results/$selected_chain.json"
    failures=1
    continue
  fi
  if ! "$runner"; then
    failures=1
  fi
done

jq -s \
  --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  '{
    generated_at: $generated_at,
    results: .,
    passed: (all(.[]; .status == "passed"))
  }' \
  "$evidence_dir"/results/*.json >"$evidence_dir/summary.json"

mkdir -p "$REPOSITORY_ROOT/artifacts/local-validation"
ln -sfn "$evidence_dir" "$REPOSITORY_ROOT/artifacts/local-validation/latest"
printf 'local validation evidence: %s\n' "$evidence_dir"
jq . "$evidence_dir/summary.json"

if (( failures != 0 )); then
  exit 1
fi
