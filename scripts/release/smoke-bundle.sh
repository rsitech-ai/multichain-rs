#!/usr/bin/env bash
set -euo pipefail

bundle_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

required_files=(
  BUILD-MANIFEST.json
  BINARY-SHA256SUMS
  CHANGELOG.md
  LICENSE
  NOTICE
  README.md
  SECURITY.md
  THIRD_PARTY_LICENSES.html
  env.example
)

for relative_path in "${required_files[@]}"; do
  if [[ ! -f "$bundle_root/$relative_path" ]]; then
    echo "release bundle is missing $relative_path" >&2
    exit 1
  fi
done

binaries=(
  bitcoin-core-connector
  fixture-source
  native-normalizer
  query-api
  stream-gateway
)

for binary_name in "${binaries[@]}"; do
  if [[ ! -x "$bundle_root/bin/$binary_name" ]]; then
    echo "release bundle is missing executable bin/$binary_name" >&2
    exit 1
  fi
done

(
  cd "$bundle_root/bin"
  shasum -a 256 -c ../BINARY-SHA256SUMS
)

fixture_output="$("$bundle_root/bin/fixture-source")"
if [[ "$fixture_output" != *'"fixture_id":"phase0-001"'* ]]; then
  echo "fixture-source did not emit the expected deterministic observation" >&2
  exit 1
fi

if connector_output="$("$bundle_root/bin/bitcoin-core-connector" 2>&1)"; then
  echo "bitcoin-core-connector unexpectedly started without configuration" >&2
  exit 1
fi
if [[ "$connector_output" != *"BITCOIN_SOURCE_ID"* ]]; then
  echo "bitcoin-core-connector did not fail on its first required boundary input" >&2
  exit 1
fi

if ! grep -q '"license": "Apache-2.0"' "$bundle_root/BUILD-MANIFEST.json"; then
  echo "build manifest does not identify Apache-2.0" >&2
  exit 1
fi

echo "multichain release bundle smoke: PASS"
