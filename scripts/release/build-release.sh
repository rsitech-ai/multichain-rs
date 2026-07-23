#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
cd "$repo_root"

if [[ $# -ne 1 ]]; then
  echo "usage: $0 v<workspace-version>" >&2
  exit 64
fi

release_tag="$1"
if [[ ! "$release_tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "release tag must look like v0.1.0" >&2
  exit 64
fi

workspace_version="$(
  awk '
    /^\[workspace\.package\]$/ { in_workspace_package = 1; next }
    /^\[/ { in_workspace_package = 0 }
    in_workspace_package && /^version = / {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' Cargo.toml
)"

if [[ "$release_tag" != "v$workspace_version" ]]; then
  echo "tag $release_tag does not match workspace version $workspace_version" >&2
  exit 65
fi

release_notes_path="$repo_root/docs/releases/${release_tag}.md"
if [[ ! -f "$release_notes_path" ]]; then
  echo "release notes do not exist: $release_notes_path" >&2
  exit 66
fi

if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
  echo "tracked files must be clean before creating a release bundle" >&2
  exit 65
fi

for command_name in cargo git rustc shasum tar; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "required command is unavailable: $command_name" >&2
    exit 69
  fi
done

if ! cargo about --version >/dev/null 2>&1; then
  echo "cargo-about 0.9.1 or newer is required for third-party notices" >&2
  exit 69
fi

target_triple="$(rustc -Vv | awk '/^host:/ {print $2}')"
if [[ -z "$target_triple" ]]; then
  echo "unable to determine the Rust host target" >&2
  exit 70
fi

git_sha="$(git rev-parse HEAD)"
git_short_sha="$(git rev-parse --short=12 HEAD)"
source_epoch="$(git show -s --format=%ct HEAD)"
release_name="multichain-rs-${release_tag}-${target_triple}"
dist_dir="$repo_root/dist"
archive_path="$dist_dir/${release_name}.tar.gz"
checksum_path="${archive_path}.sha256"
staging_root="$(mktemp -d "${TMPDIR:-/tmp}/multichain-release.XXXXXX")"
bundle_root="$staging_root/$release_name"

cleanup() {
  if [[ -n "${staging_root:-}" && -d "$staging_root" ]]; then
    rm -rf -- "$staging_root"
  fi
}
trap cleanup EXIT

mkdir -p "$dist_dir" "$bundle_root/bin" "$bundle_root/docs"

cargo build \
  --locked \
  --release \
  --package bitcoin-core-connector \
  --package fixture-source \
  --package native-normalizer \
  --package query-api \
  --package stream-gateway \
  --bins

binaries=(
  bitcoin-core-connector
  fixture-source
  native-normalizer
  query-api
  stream-gateway
)

for binary_name in "${binaries[@]}"; do
  binary_path="$repo_root/target/release/$binary_name"
  if [[ ! -x "$binary_path" ]]; then
    echo "expected release binary is missing: $binary_path" >&2
    exit 70
  fi
  cp "$binary_path" "$bundle_root/bin/$binary_name"
done

cp LICENSE NOTICE README.md CHANGELOG.md SECURITY.md "$bundle_root/"
cp .env.example "$bundle_root/env.example"
cp -R schemas infra "$bundle_root/"
cp docs/operations/local-runtime-validation.md "$bundle_root/docs/"
cp docs/operations/four-chain-acceptance.md "$bundle_root/docs/"
cp "$release_notes_path" "$bundle_root/docs/"
cp scripts/release/smoke-bundle.sh "$bundle_root/smoke.sh"
chmod +x "$bundle_root/smoke.sh"
find "$bundle_root" -name .DS_Store -type f -delete

if find "$bundle_root" -name .DS_Store -type f -print | grep -q .; then
  echo "release staging contains Finder metadata" >&2
  exit 70
fi

cargo about generate \
  --workspace \
  --locked \
  --fail \
  --output-file "$bundle_root/THIRD_PARTY_LICENSES.html" \
  scripts/release/third-party-licenses.hbs

(
  cd "$bundle_root/bin"
  shasum -a 256 "${binaries[@]}" > ../BINARY-SHA256SUMS
)

cat > "$bundle_root/BUILD-MANIFEST.json" <<EOF
{
  "name": "multichain-rs",
  "version": "$workspace_version",
  "tag": "$release_tag",
  "git_sha": "$git_sha",
  "git_short_sha": "$git_short_sha",
  "target": "$target_triple",
  "profile": "release",
  "source_date_epoch": $source_epoch,
  "rustc": "$(rustc --version)",
  "maintainer": "RSI Tech",
  "website": "https://rsitech.ai",
  "contact": "info@rsitech.ai",
  "license": "Apache-2.0",
  "binaries": [
    "bitcoin-core-connector",
    "fixture-source",
    "native-normalizer",
    "query-api",
    "stream-gateway"
  ]
}
EOF

COPYFILE_DISABLE=1 tar -czf "$archive_path" -C "$staging_root" "$release_name"
(
  cd "$dist_dir"
  shasum -a 256 "$(basename "$archive_path")" > "$(basename "$checksum_path")"
)

echo "archive=$archive_path"
echo "checksum=$checksum_path"
