#!/usr/bin/env bash

set -Eeuo pipefail

LV_REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
readonly LV_REPOSITORY_ROOT
readonly LV_MIN_FREE_KIB="${LV_MIN_FREE_KIB:-6291456}"
readonly LV_CURL_CONNECT_TIMEOUT_SECONDS="${LV_CURL_CONNECT_TIMEOUT_SECONDS:-10}"
readonly LV_CURL_MAX_TIME_SECONDS="${LV_CURL_MAX_TIME_SECONDS:-120}"

LV_CHAIN=""
LV_CHAIN_RUNTIME=""
LV_CHAIN_EVIDENCE=""
LV_CHAIN_RESULT=""
LV_CHAIN_FINISHED=0
LV_PIDS=()
LV_CONTAINERS=()

lv_require_command() {
  local command_name="$1"
  command -v "$command_name" >/dev/null 2>&1 || {
    printf 'required command is unavailable: %s\n' "$command_name" >&2
    return 1
  }
}

lv_available_kib() {
  df -Pk "$LV_REPOSITORY_ROOT" | awk 'NR == 2 { print $4 }'
}

lv_require_capacity() {
  local available_kib
  available_kib="$(lv_available_kib)"
  if (( available_kib < LV_MIN_FREE_KIB )); then
    printf 'insufficient free space: available_kib=%s required_kib=%s\n' \
      "$available_kib" "$LV_MIN_FREE_KIB" >&2
    return 1
  fi
}

lv_require_arm64_macos() {
  [[ "$(uname -s)" == "Darwin" && "$(uname -m)" == "arm64" ]] || {
    printf 'local validation requires Darwin arm64; found %s %s\n' \
      "$(uname -s)" "$(uname -m)" >&2
    return 1
  }
}

lv_require_loopback_port() {
  local protocol="$1"
  local port="$2"
  local selector
  case "$protocol" in
    tcp) selector="-iTCP:${port}" ;;
    udp) selector="-iUDP:${port}" ;;
    *)
      printf 'unsupported port protocol: %s\n' "$protocol" >&2
      return 1
      ;;
  esac
  if lsof -nP "$selector" 2>/dev/null | awk 'NR > 1 { found = 1 } END { exit !found }'; then
    printf 'loopback %s port is already occupied: %s\n' "$protocol" "$port" >&2
    lsof -nP "$selector" >&2 || true
    return 1
  fi
}

lv_sha256() {
  shasum -a 256 "$1" | awk '{ print $1 }'
}

lv_download_verified() {
  local url="$1"
  local expected_sha256="$2"
  local destination="$3"
  local actual_sha256

  curl \
    --fail \
    --location \
    --silent \
    --show-error \
    --connect-timeout "$LV_CURL_CONNECT_TIMEOUT_SECONDS" \
    --max-time "$LV_CURL_MAX_TIME_SECONDS" \
    --output "$destination" \
    "$url"
  actual_sha256="$(lv_sha256 "$destination")"
  if [[ "$actual_sha256" != "$expected_sha256" ]]; then
    printf 'checksum mismatch for %s: expected=%s actual=%s\n' \
      "$url" "$expected_sha256" "$actual_sha256" >&2
    return 1
  fi
}

lv_rpc() {
  local url="$1"
  local method="$2"
  local params="${3:-[]}"
  curl \
    --fail \
    --silent \
    --show-error \
    --connect-timeout 2 \
    --max-time 10 \
    --header 'content-type: application/json' \
    --data "$(jq -cn --arg method "$method" --argjson params "$params" \
      '{jsonrpc:"2.0",id:1,method:$method,params:$params}')" \
    "$url"
}

lv_rpc_result() {
  local response="$1"
  local context="$2"
  local error

  if ! jq -e '(.error == null) and (.result != null)' <<<"$response" >/dev/null; then
    error="$(jq -c '.error // .' <<<"$response" 2>/dev/null || printf '%s' "$response")"
    printf '%s failed: %s\n' "$context" "$error" >&2
    return 1
  fi
  jq -r '.result | if type == "string" then . else tojson end' <<<"$response"
}

lv_wait_for_rpc() {
  local url="$1"
  local method="$2"
  local params="${3:-[]}"
  local attempts="${4:-60}"
  local response
  local attempt
  for ((attempt = 1; attempt <= attempts; attempt += 1)); do
    if response="$(lv_rpc "$url" "$method" "$params" 2>/dev/null)" &&
      [[ "$(jq -r '.error // empty' <<<"$response")" == "" ]]; then
      printf '%s\n' "$response"
      return 0
    fi
    sleep 1
  done
  printf 'RPC readiness timeout: url=%s method=%s attempts=%s\n' \
    "$url" "$method" "$attempts" >&2
  return 1
}

lv_wait_until_equal() {
  local expected="$1"
  local attempts="$2"
  shift 2
  local actual
  local attempt
  for ((attempt = 1; attempt <= attempts; attempt += 1)); do
    actual="$("$@" 2>/dev/null || true)"
    if [[ "$actual" == "$expected" ]]; then
      return 0
    fi
    sleep 1
  done
  printf 'condition timeout: expected=%s actual=%s command=%q\n' \
    "$expected" "$actual" "$*" >&2
  return 1
}

lv_register_pid() {
  local pid="$1"
  kill -0 "$pid" 2>/dev/null || {
    printf 'cannot register inactive pid: %s\n' "$pid" >&2
    return 1
  }
  LV_PIDS+=("$pid")
}

lv_register_container() {
  local container="$1"
  [[ "$container" == multichain-validation-* ]] || {
    printf 'refusing to register non-task container: %s\n' "$container" >&2
    return 1
  }
  LV_CONTAINERS+=("$container")
}

lv_chain_init() {
  LV_CHAIN="$1"
  [[ -n "${LV_RUN_ROOT:-}" && -n "${LV_EVIDENCE_ROOT:-}" ]] || {
    printf 'LV_RUN_ROOT and LV_EVIDENCE_ROOT are required\n' >&2
    return 1
  }
  [[ "$LV_RUN_ROOT" == "${TMPDIR:-/tmp}/multichain-local-validation."* ]] || {
    printf 'unsafe runtime root: %s\n' "$LV_RUN_ROOT" >&2
    return 1
  }
  LV_CHAIN_RUNTIME="$LV_RUN_ROOT/$LV_CHAIN"
  LV_CHAIN_EVIDENCE="$LV_EVIDENCE_ROOT/$LV_CHAIN"
  LV_CHAIN_RESULT="$LV_EVIDENCE_ROOT/results/$LV_CHAIN.json"
  mkdir -p "$LV_CHAIN_RUNTIME" "$LV_CHAIN_EVIDENCE" "$LV_EVIDENCE_ROOT/results"
  trap 'lv_chain_error "$?" "$LINENO"' ERR
  trap 'lv_chain_cleanup' EXIT
}

lv_chain_finish() {
  local status="$1"
  local scope="$2"
  local details_json="$3"
  local ended_at
  ended_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  jq -n \
    --arg chain "$LV_CHAIN" \
    --arg status "$status" \
    --arg scope "$scope" \
    --arg ended_at "$ended_at" \
    --argjson details "$details_json" \
    '{
      chain: $chain,
      status: $status,
      scope: $scope,
      ended_at: $ended_at,
      details: $details
    }' >"$LV_CHAIN_RESULT"
  LV_CHAIN_FINISHED=1
}

lv_chain_error() {
  local exit_code="$1"
  local line="$2"
  trap - ERR
  if (( LV_CHAIN_FINISHED == 0 )) && [[ -n "$LV_CHAIN_RESULT" ]]; then
    lv_chain_finish \
      "failed" \
      "local_runtime" \
      "$(jq -cn --arg reason "command_failed" --argjson exit_code "$exit_code" \
        --argjson line "$line" '{reason:$reason,exit_code:$exit_code,line:$line}')"
  fi
  return "$exit_code"
}

lv_chain_cleanup() {
  local pid
  local container
  local stopped_pids=0
  local removed_containers=0
  trap - ERR
  if [[ "${LV_KEEP_RUNTIME:-0}" == "1" ]]; then
    if [[ -n "$LV_CHAIN_EVIDENCE" ]]; then
      jq -n \
        --arg status "retained_by_request" \
        --argjson process_count "${#LV_PIDS[@]}" \
        --argjson container_count "${#LV_CONTAINERS[@]}" \
        '{status:$status,process_count:$process_count,container_count:$container_count}' \
        >"$LV_CHAIN_EVIDENCE/cleanup.json"
    fi
    return
  fi
  for pid in "${LV_PIDS[@]:-}"; do
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      for _ in {1..20}; do
        kill -0 "$pid" 2>/dev/null || break
        sleep 0.1
      done
      kill -KILL "$pid" 2>/dev/null || true
      stopped_pids=$((stopped_pids + 1))
    fi
  done
  for container in "${LV_CONTAINERS[@]:-}"; do
    if [[ -n "$container" ]]; then
      docker rm --force "$container" >/dev/null 2>&1 || true
      removed_containers=$((removed_containers + 1))
    fi
  done
  if [[ -n "$LV_CHAIN_EVIDENCE" ]]; then
    jq -n \
      --arg status "cleaned" \
      --argjson stopped_pids "$stopped_pids" \
      --argjson removed_containers "$removed_containers" \
      '{status:$status,stopped_pids:$stopped_pids,removed_containers:$removed_containers}' \
      >"$LV_CHAIN_EVIDENCE/cleanup.json"
  fi
}
