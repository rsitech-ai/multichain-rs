#!/usr/bin/env bash

set -Eeuo pipefail

SCRIPT_DIRECTORY="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIRECTORY
# shellcheck source=scripts/local-validation/common.sh
source "$SCRIPT_DIRECTORY/common.sh"

readonly COMPOSE_FILE="$LV_REPOSITORY_ROOT/infra/compose.yaml"
readonly PLATFORM_PORTS=(19092 19644 18123 15432 19000 19001 18080)
readonly CLICKHOUSE_LOCAL_USER="multichain"
readonly CLICKHOUSE_LOCAL_PASSWORD="local-development-only"
COMPOSE_PROJECT=""

platform_compose() {
  docker compose \
    --project-name "$COMPOSE_PROJECT" \
    --file "$COMPOSE_FILE" \
    "$@"
}

platform_cleanup() {
  local exit_code=$?
  local compose_cleanup_status="not_started"
  local compose_cleanup_exit_code=0
  local cleanup_tmp
  trap - ERR EXIT

  if [[ -n "$COMPOSE_PROJECT" && "${LV_KEEP_RUNTIME:-0}" != "1" ]]; then
    compose_cleanup_status="cleaned"
    if ! platform_compose \
      --profile setup \
      down \
      --volumes \
      --remove-orphans \
      >"$LV_CHAIN_EVIDENCE/compose-down.log" 2>&1; then
      compose_cleanup_status="failed"
      compose_cleanup_exit_code=1
      exit_code=1
      lv_chain_finish \
        "failed" \
        "platform_cleanup" \
        "$(jq -cn \
          --arg reason "compose_down_failed" \
          --arg project "$COMPOSE_PROJECT" \
          '{reason:$reason,compose_project:$project}')"
    fi
  elif [[ "${LV_KEEP_RUNTIME:-0}" == "1" ]]; then
    compose_cleanup_status="retained_by_request"
  fi

  lv_chain_cleanup
  if [[ -f "$LV_CHAIN_EVIDENCE/cleanup.json" ]]; then
    cleanup_tmp="$LV_CHAIN_EVIDENCE/cleanup.json.tmp"
    jq \
      --arg compose_project "$COMPOSE_PROJECT" \
      --arg compose_cleanup_status "$compose_cleanup_status" \
      --argjson compose_cleanup_exit_code "$compose_cleanup_exit_code" \
      '. + {
        compose_project:$compose_project,
        compose_cleanup_status:$compose_cleanup_status,
        compose_cleanup_exit_code:$compose_cleanup_exit_code
      }' \
      "$LV_CHAIN_EVIDENCE/cleanup.json" \
      >"$cleanup_tmp"
    mv "$cleanup_tmp" "$LV_CHAIN_EVIDENCE/cleanup.json"
  fi
  exit "$exit_code"
}

wait_for_query_api() {
  local response
  local attempt
  for attempt in {1..60}; do
    if response="$(
      curl \
        --fail \
        --silent \
        --show-error \
        --connect-timeout 2 \
        --max-time 5 \
        http://127.0.0.1:18080/health/ready \
        2>/dev/null
    )" &&
      jq -e '.component == "query-api" and .ready == true' \
        <<<"$response" >/dev/null; then
      printf '%s\n' "$response"
      return 0
    fi
    sleep 1
  done
  printf '%s\n' 'query-api readiness timeout' >&2
  return 1
}

lv_chain_init platform
COMPOSE_PROJECT="multichain-validation-$(date -u +%Y%m%dt%H%M%Sz)-$$"
trap 'platform_cleanup' EXIT
lv_require_capacity
for command_name in docker curl jq cargo; do
  lv_require_command "$command_name"
done
docker info >/dev/null
docker compose version >/dev/null
docker compose --file "$COMPOSE_FILE" config --quiet
for port in "${PLATFORM_PORTS[@]}"; do
  lv_require_loopback_port tcp "$port"
done

platform_compose \
  up \
  --detach \
  --wait \
  >"$LV_CHAIN_EVIDENCE/compose-up.log" 2>&1
platform_compose \
  --profile setup \
  run \
  --rm \
  minio-init \
  >"$LV_CHAIN_EVIDENCE/minio-init.log" 2>&1

redpanda_health="$(
  curl \
    --fail \
    --silent \
    --show-error \
    http://127.0.0.1:19644/v1/status/ready
)"
clickhouse_health="$(
  curl \
    --fail \
    --silent \
    --show-error \
    --header "X-ClickHouse-User: $CLICKHOUSE_LOCAL_USER" \
    --header "X-ClickHouse-Key: $CLICKHOUSE_LOCAL_PASSWORD" \
    http://127.0.0.1:18123/ping
)"
minio_health="$(
  curl \
    --fail \
    --silent \
    --show-error \
    http://127.0.0.1:19000/minio/health/ready
)"
postgres_container="$(platform_compose ps --quiet postgres)"
docker exec "$postgres_container" \
  pg_isready \
  --username multichain \
  --dbname multichain \
  >"$LV_CHAIN_EVIDENCE/postgres-health.log"
printf '%s\n' "$redpanda_health" >"$LV_CHAIN_EVIDENCE/redpanda-health.json"
printf '%s\n' "$clickhouse_health" >"$LV_CHAIN_EVIDENCE/clickhouse-health.txt"
printf '%s\n' "$minio_health" >"$LV_CHAIN_EVIDENCE/minio-health.txt"

services_json='[]'
for service in redpanda clickhouse postgres minio; do
  container_id="$(platform_compose ps --quiet "$service")"
  service_json="$(
    docker inspect "$container_id" |
      jq \
        --arg service "$service" \
        '.[0] | {
          service:$service,
          container_name:(.Name | ltrimstr("/")),
          image_ref:.Config.Image,
          image_id:.Image,
          health:(.State.Health.Status // .State.Status)
        }'
  )"
  services_json="$(
    jq \
      --argjson service "$service_json" \
      '. + [$service]' \
      <<<"$services_json"
  )"
done
printf '%s\n' "$services_json" >"$LV_CHAIN_EVIDENCE/services.json"

redpanda_version="$(
  docker exec "$(platform_compose ps --quiet redpanda)" rpk version |
    awk 'NR == 1 { first = $0 } END { print first }'
)"
clickhouse_version="$(
  docker exec "$(platform_compose ps --quiet clickhouse)" \
    clickhouse-client --version
)"
postgres_version="$(
  docker exec "$postgres_container" postgres --version
)"
minio_version="$(
  docker exec "$(platform_compose ps --quiet minio)" minio --version |
    awk 'NR == 1 { first = $0 } END { print first }'
)"

MULTICHAIN_REQUIRE_INFRA=1 cargo test \
  -p integration-tests \
  --test wal_broker_archive \
  -- \
  --nocapture \
  >"$LV_CHAIN_EVIDENCE/wal-broker-archive.log" 2>&1
MULTICHAIN_REQUIRE_INFRA=1 cargo test \
  -p e2e-tests \
  --test phase0_synthetic \
  -- \
  --nocapture \
  >"$LV_CHAIN_EVIDENCE/phase0-synthetic.log" 2>&1
MULTICHAIN_REQUIRE_INFRA=1 cargo test \
  -p fault-tests \
  --test phase0_restart_replay \
  -- \
  --nocapture \
  >"$LV_CHAIN_EVIDENCE/phase0-restart-replay.log" 2>&1

QUERY_API_BIND=127.0.0.1:18080 cargo run \
  -p query-api \
  >"$LV_CHAIN_EVIDENCE/query-api.log" 2>&1 &
query_api_pid="$!"
lv_register_pid "$query_api_pid"
query_api_health="$(wait_for_query_api)"
printf '%s\n' "$query_api_health" >"$LV_CHAIN_EVIDENCE/query-api-health.json"

details="$(
  jq -n \
    --arg compose_project "$COMPOSE_PROJECT" \
    --arg redpanda_version "$redpanda_version" \
    --arg clickhouse_version "$clickhouse_version" \
    --arg postgres_version "$postgres_version" \
    --arg minio_version "$minio_version" \
    --argjson services "$services_json" \
    --argjson query_api_health "$query_api_health" \
    '{
      compose_project:$compose_project,
      services:$services,
      versions:{
        redpanda:$redpanda_version,
        clickhouse:$clickhouse_version,
        postgres:$postgres_version,
        minio:$minio_version
      },
      infrastructure_health:"passed",
      wal_broker_archive:"passed",
      raw_archive_replay:"passed",
      clickhouse_fact_idempotence:"passed",
      postgres_checkpoints:"passed",
      rest_and_websocket_e2e:"passed",
      restart_replay_and_gap_repair:"passed",
      query_api_health:$query_api_health,
      production_ha_topology_proven:false
    }'
)"
printf '%s\n' "$details" >"$LV_CHAIN_EVIDENCE/evidence.json"
lv_chain_finish \
  "passed" \
  "task_scoped_compose_durable_pipeline_and_restart_replay" \
  "$details"
