#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <archive.tar.gz> <archive.tar.gz.sha256>" >&2
  exit 64
fi

archive_path="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
checksum_path="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"

if [[ ! -f "$archive_path" ]]; then
  echo "archive does not exist: $archive_path" >&2
  exit 66
fi
if [[ ! -f "$checksum_path" ]]; then
  echo "checksum does not exist: $checksum_path" >&2
  exit 66
fi

archive_name="$(basename "$archive_path")"
checksum_name="$(basename "$checksum_path")"
checksum_dir="$(dirname "$checksum_path")"

if ! grep -q "  $archive_name\$" "$checksum_path"; then
  echo "checksum file does not name $archive_name" >&2
  exit 65
fi

(
  cd "$checksum_dir"
  shasum -a 256 -c "$checksum_name"
)

if tar -tzf "$archive_path" | grep -Eq '(^/|(^|/)\.\.(/|$))'; then
  echo "archive contains an unsafe path" >&2
  exit 65
fi

extract_root="$(mktemp -d "${TMPDIR:-/tmp}/multichain-release-smoke.XXXXXX")"
cleanup() {
  if [[ -n "${extract_root:-}" && -d "$extract_root" ]]; then
    rm -rf -- "$extract_root"
  fi
}
trap cleanup EXIT

tar -xzf "$archive_path" -C "$extract_root"
bundle_count="$(find "$extract_root" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')"
if [[ "$bundle_count" != "1" ]]; then
  echo "archive must contain exactly one top-level directory" >&2
  exit 65
fi

bundle_root="$(find "$extract_root" -mindepth 1 -maxdepth 1 -type d -print)"
"$bundle_root/smoke.sh"

echo "multichain release archive smoke: PASS"
