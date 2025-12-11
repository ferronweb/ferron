#!/usr/bin/env bash

set -euo pipefail

FERRON_BIN="${FERRON_BIN:-./target/debug/ferron}"
FERRON_CONFIG="${FERRON_CONFIG:-test-script.kdl}"
SCRIPT_FILE="${SCRIPT_FILE:-scripts/example.rhai}"
FERRON_URL="${FERRON_URL:-http://127.0.0.1:8080/}"
WRK_BIN="${WRK_BIN:-wrk}"
SERVER_LOG="${SERVER_LOG:-/tmp/ferron-script-module.log}"
WRK_THREADS="${WRK_THREADS:-4}"
WRK_CONNECTIONS="${WRK_CONNECTIONS:-32}"
WRK_DURATION="${WRK_DURATION:-10s}"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: 未找到依赖命令 '$1'" >&2
    exit 1
  fi
}

require_cmd curl
require_cmd "$WRK_BIN"

CONFIG_BACKUP=$(mktemp)
SCRIPT_BACKUP=$(mktemp)
CONFIG_ORIG_EXISTS=0
SCRIPT_ORIG_EXISTS=0

if [[ -f "$FERRON_CONFIG" ]]; then
  CONFIG_ORIG_EXISTS=1
  cp "$FERRON_CONFIG" "$CONFIG_BACKUP"
fi

if [[ -f "$SCRIPT_FILE" ]]; then
  SCRIPT_ORIG_EXISTS=1
  cp "$SCRIPT_FILE" "$SCRIPT_BACKUP"
fi

log() {
  printf '[%s] %s\n' "$(date '+%H:%M:%S')" "$*" >&2
}

fail() {
  echo "error: $*" >&2
  if [[ -s "$SERVER_LOG" ]]; then
    echo "--- Ferron 日志 (最近 40 行) ---" >&2
    tail -n 40 "$SERVER_LOG" >&2 || true
  fi
  exit 1
}

SERVER_PID=""
WRK_PARAMS=()
if [[ $# -gt 0 ]]; then
  WRK_PARAMS=("$@")
else
  WRK_PARAMS=(-t"$WRK_THREADS" -c"$WRK_CONNECTIONS" -d"$WRK_DURATION" "$FERRON_URL")
fi

cleanup() {
  if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" >/dev/null 2>&1; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  if [[ $CONFIG_ORIG_EXISTS -eq 1 ]]; then
    mv "$CONFIG_BACKUP" "$FERRON_CONFIG"
  else
    rm -f "$FERRON_CONFIG"
  fi
  if [[ $SCRIPT_ORIG_EXISTS -eq 1 ]]; then
    mv "$SCRIPT_BACKUP" "$SCRIPT_FILE"
  else
    rm -f "$SCRIPT_FILE"
  fi
  rm -f "$CONFIG_BACKUP" "$SCRIPT_BACKUP"
}
trap cleanup EXIT

write_file() {
  local path="$1"
  local content="$2"
  printf '%s\n' "$content" >"$path"
}

stop_server() {
  if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" >/dev/null 2>&1; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  SERVER_PID=""
}

wait_for_ready() {
  for _ in $(seq 1 30); do
    if curl -sS -o /dev/null "$FERRON_URL"; then
      return 0
    fi
    if ! kill -0 "$SERVER_PID" >/dev/null 2>&1; then
      fail "Ferron 提前退出，请检查 $SERVER_LOG"
    fi
    sleep 1
  done
  fail "等待 Ferron 启动超时 ($FERRON_URL)"
}

start_server() {
  local scenario="$1"
  local config_content="$2"
  local script_content="$3"

  stop_server
  write_file "$FERRON_CONFIG" "$config_content"
  write_file "$SCRIPT_FILE" "$script_content"

  log "启动 Ferron（场景：$scenario）"
  "$FERRON_BIN" --config "$FERRON_CONFIG" >"$SERVER_LOG" 2>&1 &
  SERVER_PID=$!
  wait_for_ready
}

RESULT_STATUS=""
RESULT_HEADERS=""
RESULT_BODY=""

collect_response() {
  RESULT_HEADERS=$(mktemp)
  RESULT_BODY=$(mktemp)
  RESULT_STATUS=$(curl -sS -D "$RESULT_HEADERS" -o "$RESULT_BODY" -w "%{http_code}" "$FERRON_URL")
}

assert_status() {
  local expected="$1"
  if [[ "$RESULT_STATUS" != "$expected" ]]; then
    fail "期望 HTTP $expected，实际 $RESULT_STATUS"
  fi
}

assert_status_ge() {
  local expected="$1"
  if (( RESULT_STATUS < expected )); then
    fail "期望 HTTP >= $expected，实际 $RESULT_STATUS"
  fi
}

assert_header_equals() {
  local name="$1"
  local value="$2"
  if ! tr -d '\r' <"$RESULT_HEADERS" | grep -qi "^$name: $value\$"; then
    fail "响应头 $name 不符合预期（缺少 $value）"
  fi
}

assert_body_contains() {
  local text="$1"
  if ! grep -q "$text" "$RESULT_BODY"; then
    fail "响应体未包含：$text"
  fi
}

WRK_LAST_OUTPUT=""
run_wrk() {
  log "运行 wrk: ${WRK_PARAMS[*]}"
  local tmp
  tmp=$(mktemp)
  set +e
  "$WRK_BIN" "${WRK_PARAMS[@]}" | tee "$tmp"
  local status=${PIPESTATUS[0]}
  set -e
  WRK_LAST_OUTPUT=$(cat "$tmp")
  rm -f "$tmp"
  if (( status != 0 )); then
    fail "wrk 执行失败（状态 $status）"
  fi
}

response_ready_config() {
  cat <<'KDL'
globals {
  log "access.log"
  error_log "error.log"
}

:8080 {
  module "script-exec" {
    script "test-script" {
      file "scripts/example.rhai"
      trigger "on_response_ready"
      reload_on_change #true
      limits {
        max_operations 200_000
        max_call_depth 32
        max_exec_time "50ms"
      }
      failure_policy "skip"
    }
  }

  root "wwwroot"
}
KDL
}

response_script() {
  local version="$1"
  cat <<SCRIPT
log("info", "response hook for version $version");
response.set_header("X-Script-Source", "file");
response.set_header("X-Script-Version", "$version");
response.set_status(200);
response.set_body("Hello version $version");
SCRIPT
}

request_start_config() {
  cat <<'KDL'
globals {
  log "access.log"
  error_log "error.log"
}

:8080 {
  module "script-exec" {
    script "test-script" {
      file "scripts/example.rhai"
      trigger "on_request_start"
      limits {
        max_operations 200_000
        max_call_depth 32
        max_exec_time "50ms"
      }
      failure_policy "block"
    }
  }

  root "wwwroot"
}
KDL
}

request_start_script() {
  cat <<'SCRIPT'
log("info", "denying request at request_start");
deny(403, "blocked by request_start script");
SCRIPT
}

limit_config() {
  local policy="$1"
  cat <<KDL
globals {
  log "access.log"
  error_log "error.log"
}

:8080 {
  module "script-exec" {
    script "test-script" {
      file "scripts/example.rhai"
      trigger "on_response_ready"
      limits {
        max_operations 1_000
        max_call_depth 8
        max_exec_time "10ms"
      }
      failure_policy "$policy"
    }
  }

  root "wwwroot"
}
KDL
}

limit_script() {
  cat <<'SCRIPT'
let i = 0;
while true {
  i += 1;
}
// 永远不会执行到这里
response.set_body("should not reach");
SCRIPT
}

scenario_response_ready() {
  log "=== 场景 1：on_response_ready + reload_on_change ==="
  start_server "response_ready" "$(response_ready_config)" "$(response_script "1.0")"
  collect_response
  assert_status 200
  assert_header_equals "X-Script-Source" "file"
  assert_header_equals "X-Script-Version" "1.0"
  assert_body_contains "Hello version 1.0"

  log "验证 reload_on_change，热更新脚本版本号"
  write_file "$SCRIPT_FILE" "$(response_script "2.0")"
  sleep 1
  collect_response
  assert_header_equals "X-Script-Version" "2.0"
  assert_body_contains "Hello version 2.0"

  run_wrk
  stop_server
}

scenario_request_start() {
  log "=== 场景 2：on_request_start 拒绝请求 ==="
  start_server "request_start" "$(request_start_config)" "$(request_start_script)"
  collect_response
  assert_status 403
  assert_body_contains "blocked by request_start script"

  run_wrk
  if ! grep -q 'Non-2xx or 3xx responses:' <<<"$WRK_LAST_OUTPUT"; then
    fail "wrk 输出未显示 403（Non-2xx or 3xx responses）"
  fi
  stop_server
}

scenario_limit_skip() {
  log "=== 场景 3：limits + failure_policy=skip ==="
  start_server "limits-skip" "$(limit_config "skip")" "$(limit_script)"
  collect_response
  assert_status 200
  assert_body_contains "Ferron is installed successfully"
  stop_server
}

scenario_limit_block() {
  log "=== 场景 4：limits + failure_policy=block ==="
  start_server "limits-block" "$(limit_config "block")" "$(limit_script)"
  collect_response
  assert_status_ge 500
  stop_server
}

scenario_response_ready
scenario_request_start
scenario_limit_skip
scenario_limit_block

log "全部脚本参数场景验证完成"
