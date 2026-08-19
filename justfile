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
    bash tests/local-validation/runner_contract.sh
    bash tests/local-validation/phase1_gate_contract.sh
    bash tests/local-validation/evm_foundation_gate_contract.sh

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

validate-local chain="all":
    ./scripts/validate-local.sh --chain "{{chain}}"

release-build version="v0.1.0":
    ./scripts/release/build-release.sh "{{version}}"

release-smoke archive checksum:
    ./scripts/release/smoke-release.sh "{{archive}}" "{{checksum}}"

verify-task4:
    MULTICHAIN_REQUIRE_INFRA=1 cargo test -p integration-tests --test wal_broker_archive -- --nocapture

verify-phase0:
    MULTICHAIN_REQUIRE_INFRA=1 cargo test -p e2e-tests --test phase0_synthetic -- --nocapture
    MULTICHAIN_REQUIRE_INFRA=1 cargo test -p fault-tests --test phase0_restart_replay -- --nocapture
    # The e2e test creates the durable rows needed by the standalone readiness route.
    log_file="$(mktemp)"; QUERY_API_BIND=127.0.0.1:18080 cargo run -p query-api >"$log_file" 2>&1 & api_pid=$!; trap 'kill "$api_pid" 2>/dev/null || true; rm -f "$log_file"' EXIT; ready=0; for _ in $(seq 1 30); do response="$(curl --fail --silent --show-error http://127.0.0.1:18080/health/ready 2>/dev/null || true)"; if [[ "$response" == *'"component":"query-api"'* && "$response" == *'"ready":true'* ]]; then ready=1; break; fi; sleep 1; done; test "$ready" -eq 1 || { cat "$log_file"; exit 1; }; echo "$response"; kill "$api_pid"; wait "$api_pid" 2>/dev/null || true; trap - EXIT; rm -f "$log_file"

verify-phase1:
    ./scripts/verify-phase1.sh

verify-evm-foundation:
    ./scripts/verify-evm-foundation.sh
