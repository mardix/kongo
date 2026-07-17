#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PORT="${KONGODB_PORT:-18093}"
SMOKE_ROOT="${KONGODB_SMOKE_ROOT:-./.smoke/audit-logs}"
DATA_DIR="${KONGODB_DATA_DIR:-${SMOKE_ROOT}/data}"
LOG_FILE="${KONGODB_SMOKE_LOG:-${SMOKE_ROOT}/logs/smoke-audit-logs.log}"
BIN="${KONGODB_BIN:-./target/debug/kongodb}"
BASE_URL="http://127.0.0.1:${PORT}"
BASE_PATH_RAW="${KONGODB_BASE_PATH:-}"
BASE_PATH="/${BASE_PATH_RAW#/}"
BASE_PATH="${BASE_PATH%/}"
if [[ "$BASE_PATH" == "/" ]]; then BASE_PATH=""; fi
GATEWAY_URL="${BASE_URL}${BASE_PATH}/gateway"

assert_contains() {
  local haystack="$1"
  local needle="$2"
  local message="$3"
  if ! grep -Fq "$needle" <<<"$haystack"; then
    echo "assertion failed: $message" >&2
    echo "response: $haystack" >&2
    exit 1
  fi
}

echo "[1/7] building kongodb"
cargo build >/dev/null

echo "[2/7] preparing smoke dirs under $SMOKE_ROOT"
rm -rf "$SMOKE_ROOT"
mkdir -p "$DATA_DIR" "$(dirname "$LOG_FILE")"

echo "[3/7] starting server on :$PORT"
KONGODB_PORT="$PORT" \
KONGODB_STORAGE_MODE="local" \
KONGODB_DATA_DIR="$DATA_DIR" \
KONGODB_BASE_PATH="$BASE_PATH" \
KONGODB_AUTH_MODE="none" \
KONGODB_WRITE_MODE="accepted" \
"$BIN" >"$LOG_FILE" 2>&1 &
PID=$!
cleanup() {
  kill "$PID" >/dev/null 2>&1 || true
}
trap cleanup EXIT

READY=false
for _ in $(seq 1 40); do
  if curl -fsS -o /dev/null "$BASE_URL/ping" 2>/dev/null; then
    READY=true
    break
  fi
  sleep 0.25
done
if [[ "$READY" != "true" ]]; then
  echo "server did not become ready on :$PORT" >&2
  cat "$LOG_FILE" >&2
  exit 1
fi

echo "[4/7] create audit database"
CREATE_DB="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"audit/main","operation":"create_db","payload":{}}')"
assert_contains "$CREATE_DB" '"status":"success"' "create_db should succeed"

echo "[5/7] append committed audit events"
INGEST="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{
  "db":"audit/main",
  "operation":"audit_ingest",
  "payload":{
    "events":[
      {"action":"user.login","ts":"2026-07-15T10:00:00Z","actor_type":"user","actor_id":"u1","target_type":"session","target_id":"s1","status":"success","source":"api","request_id":"r1","message":"Google login","data":{"provider":"google"}},
      {"action":"file.delete","ts":"2026-07-15T11:00:00Z","actor_type":"admin","actor_id":"a1","target_type":"file","target_id":"f1","status":"denied","source":"admin-ui","request_id":"r2","message":"Delete denied","data":{"reason":"policy"}}
    ]
  }
}')"
assert_contains "$INGEST" '"status":"success"' "audit_ingest should succeed"
assert_contains "$INGEST" '"count":2' "audit_ingest should append two events"
assert_contains "$INGEST" '"committed":true' "audit_ingest should default to committed acknowledgement"

echo "[6/7] query and filter audit timeline"
QUERY="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"audit/main","operation":"audit_query","payload":{"actor_id":"u1","start":"2026-07-15","end":"2026-07-15","page":1,"per_page":1}}')"
assert_contains "$QUERY" '"total_items":1' "actor/date query should match one event"
assert_contains "$QUERY" '"action":"user.login"' "query should return matching action"
assert_contains "$QUERY" '"provider":"google"' "query should decode event data"

SEARCH="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"audit/main","operation":"audit_query","payload":{"search":"denied","status":"denied","page":1,"per_page":25}}')"
assert_contains "$SEARCH" '"action":"file.delete"' "text/status query should return denied event"
assert_contains "$SEARCH" '"next_page":null' "pagination should report final page"

echo "[7/7] reject event without action"
INVALID="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"audit/main","operation":"audit_ingest","payload":{"events":[{"message":"missing action"}]}}')"
assert_contains "$INVALID" 'events[].action is required' "audit_ingest should require action"

echo "audit-logs smoke passed. log: $LOG_FILE"
