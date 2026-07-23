set shell := ["bash", "-euo", "pipefail", "-c"]

check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace --all-targets
    buf lint
    buf breaking schemas/protobuf --against tests/contracts/platform-v1-baseline.binpb
    cargo deny check
    gitleaks detect --no-banner
    docker compose -f infra/compose.yaml config --quiet

infra-up:
    docker compose -f infra/compose.yaml up -d --wait
    docker compose -f infra/compose.yaml --profile setup run --rm minio-init

infra-health:
    curl --fail --silent --show-error http://127.0.0.1:19644/v1/status/ready
    curl --fail --silent --show-error http://127.0.0.1:18123/ping
    pg_isready -h 127.0.0.1 -p 15432 -U multichain -d multichain
    curl --fail --silent --show-error http://127.0.0.1:19000/minio/health/ready

infra-down:
    docker compose -f infra/compose.yaml --profile setup down --volumes --remove-orphans

verify-task4:
    MULTICHAIN_REQUIRE_INFRA=1 cargo test -p integration-tests --test wal_broker_archive -- --nocapture
