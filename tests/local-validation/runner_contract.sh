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

printf '%s\n' 'local validation runner contract: passed'
