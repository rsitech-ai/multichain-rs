#!/usr/bin/env bash

set -Eeuo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
readonly REPOSITORY_ROOT
readonly VALIDATOR="$REPOSITORY_ROOT/scripts/validate-project-manifest.py"

valid="$(
  python3 "$VALIDATOR" \
    "$REPOSITORY_ROOT/.rsitech/project.json" \
    --repo-root "$REPOSITORY_ROOT"
)"
jq -e '
  .readiness == "preview-ready" and
  (.manifest_hash | test("^[0-9a-f]{64}$"))
' <<<"$valid" >/dev/null

fixture_dir="$(mktemp -d "${TMPDIR:-/tmp}/multichain-manifest-contract.XXXXXX")"
cleanup() {
  if [[ "$fixture_dir" == "${TMPDIR:-/tmp}/multichain-manifest-contract."* ]]; then
    rm -rf -- "$fixture_dir"
  fi
}
trap cleanup EXIT

jq 'del(.identity.slug)' \
  "$REPOSITORY_ROOT/.rsitech/project.json" >"$fixture_dir/missing-slug.json"
if python3 "$VALIDATOR" "$fixture_dir/missing-slug.json" \
  --repo-root "$REPOSITORY_ROOT" >/dev/null 2>&1; then
  printf '%s\n' 'manifest without identity.slug unexpectedly succeeded' >&2
  exit 1
fi

jq '.capabilities[0].evidence = ["../private"]' \
  "$REPOSITORY_ROOT/.rsitech/project.json" >"$fixture_dir/unsafe-path.json"
if python3 "$VALIDATOR" "$fixture_dir/unsafe-path.json" \
  --repo-root "$REPOSITORY_ROOT" >/dev/null 2>&1; then
  printf '%s\n' 'manifest with unsafe evidence path unexpectedly succeeded' >&2
  exit 1
fi

printf '%s\n' 'project manifest contract: passed'
