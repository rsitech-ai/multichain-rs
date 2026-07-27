#!/usr/bin/env bash

set -Eeuo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
readonly REPOSITORY_ROOT
readonly RUNNER="$REPOSITORY_ROOT/scripts/verify-phase1.sh"

output="$("$RUNNER" --dry-run)"
jq -e '
  .mode == "dry_run" and
  .scope == "bitcoin_phase1_local_gate" and
  .local_checks == [
    "project_manifest",
    "rust_format",
    "bitcoin_clippy",
    "bitcoin_unit_tests",
    "bitcoin_multi_observer_fixture",
    "alert_preview_http"
  ] and
  .promotion_verdict == "hold" and
  .production_blockers == [
    "three_independent_mainnet_observers",
    "production_ha_and_security",
    "load_and_soak_targets",
    "disaster_recovery_drill",
    "end_to_end_web_correction_proof"
  ]
' <<<"$output" >/dev/null

if "$RUNNER" --dry-run --evidence-dir relative/path >/dev/null 2>&1; then
  printf '%s\n' 'relative evidence directory unexpectedly succeeded' >&2
  exit 1
fi

printf '%s\n' 'phase1 local gate contract: passed'
