#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PORT="${KONGODB_PORT:-18090}"
SMOKE_ROOT="${KONGODB_SMOKE_ROOT:-./.smoke/metric-events}"
DATA_DIR="${KONGODB_DATA_DIR:-${SMOKE_ROOT}/data}"
LOG_FILE="${KONGODB_SMOKE_LOG:-${SMOKE_ROOT}/logs/smoke-metric-events.log}"
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
  local msg="$3"
  if ! grep -Fq "$needle" <<<"$haystack"; then
    echo "assertion failed: $msg" >&2
    echo "response: $haystack" >&2
    exit 1
  fi
}

echo "[1/8] building kongodb"
cargo build >/dev/null

echo "[2/8] preparing smoke dirs under $SMOKE_ROOT"
rm -rf "$SMOKE_ROOT"
mkdir -p "$DATA_DIR" "$(dirname "$LOG_FILE")"

echo "[3/8] starting server on :$PORT"
KONGODB_PORT="$PORT" \
KONGODB_STORAGE_MODE="local" \
KONGODB_DATA_DIR="$DATA_DIR" \
KONGODB_BASE_PATH="$BASE_PATH" \
KONGODB_AUTH_MODE="none" \
KONGODB_WRITE_MODE="committed" \
"$BIN" >"$LOG_FILE" 2>&1 &
PID=$!
cleanup() {
  kill "$PID" >/dev/null 2>&1 || true
}
trap cleanup EXIT

ready=0
for _ in $(seq 1 40); do
  if curl -sS -o /dev/null "$BASE_URL/ping"; then
    ready=1
    break
  fi
  sleep 0.25
done
if [[ "$ready" -ne 1 ]]; then
  echo "assertion failed: server did not become ready on $BASE_URL" >&2
  echo "server log: $LOG_FILE" >&2
  exit 1
fi

echo "[4/8] create db"
CREATE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"metric-events/main","operation":"create_db","payload":{}}')"
assert_contains "$CREATE" '"status":"success"' "create_db should succeed"

echo "[5/8] committed metrics_ingest"
TRACK="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{
  "db":"metric-events/main",
  "operation":"metrics_ingest",
  "payload":{
    "commit":true,
    "events":[
      {"event":"api.request","ts":"2026-06-14T13:05:00Z","tenant_id":"t1","user_id":"u1","value":1,"dimensions":{"endpoint":"/v1/chat","status":200,"duration_ms":100},"metadata":{"request_id":"r1"}},
      {"event":"api.request","ts":"2026-06-14T13:10:00Z","tenant_id":"t1","user_id":"u2","value":1,"dimensions":{"endpoint":"/v1/chat","status":200,"duration_ms":200},"metadata":{"request_id":"r2"}},
      {"event":"api.request","ts":"2026-06-14T14:10:00Z","tenant_id":"t1","user_id":"u2","value":1,"dimensions":{"endpoint":"/v1/upload","status":500,"duration_ms":300},"metadata":{"request_id":"r3"}}
    ]
  }
}')"
assert_contains "$TRACK" '"status":"success"' "metrics_ingest should succeed"
assert_contains "$TRACK" '"count":3' "metrics_ingest should insert 3"

echo "[6/8] query grouped hourly metrics"
QUERY="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{
  "db":"metric-events/main",
  "operation":"metrics_query",
  "payload":{
    "alias":"api_requests",
    "label":"API Requests",
    "event":"api.request",
    "start":"2026-06-14",
    "end":"2026-06-14",
    "interval":"hour",
    "bucket_label":"{{bucket HH:mm}}",
    "filter":{"tenant_id":"t1"},
    "group_by":[{"field":"dimensions.endpoint","alias":"endpoint","label":"Endpoint"}],
    "metrics":[
      {"op":"count","field":"*","alias":"requests","label":"Requests"},
      {"op":"avg","field":"dimensions.duration_ms","alias":"avg_duration_ms","label":"Avg duration"},
      {"op":"count_distinct","field":"user_id","alias":"unique_users","label":"Unique users"}
    ],
    "sort":"bucket asc, requests desc"
  }
}')"
assert_contains "$QUERY" '"results":{"api_requests"' "metrics_query should use results alias"
assert_contains "$QUERY" '"labels":{"groups"' "metrics_query should include grouped labels"
assert_contains "$QUERY" '"requests":2' "first hour should count 2 requests"
assert_contains "$QUERY" '"bucket_label":"13:00"' "bucket_label should render hour"
assert_contains "$QUERY" '"interval":"hour"' "result should include normalized interval"

echo "[7/8] batch metrics_query with readable/alias ranges"
BATCH="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{
  "db":"metric-events/main",
  "operation":"metrics_query",
  "payload":{
    "batch":[
      {"alias":"all_requests","event":"api.request","range":"3days","metrics":[{"op":"count","field":"*","alias":"requests","label":"Requests"}]},
      {"alias":"endpoints","event":"api.request","range":"this-month","metrics":[{"op":"distinct","field":"dimensions.endpoint","alias":"endpoint_list","label":"Endpoints"}]}
    ]
  }
}')"
assert_contains "$BATCH" '"count":2' "batch should return two result sets"
assert_contains "$BATCH" '"all_requests"' "batch should include all_requests"
assert_contains "$BATCH" '"endpoints"' "batch should include endpoints"
assert_contains "$BATCH" '"range":"3days"' "readable rolling range should normalize"
assert_contains "$BATCH" '"range":"this_month"' "dash alias should normalize to underscore"

echo "[8/8] default metrics_ingest is accepted/queued"
ASYNC="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{
  "db":"metric-events/main",
  "operation":"metrics_ingest",
  "payload":{
    "events":[{"event":"auth.login","tenant_id":"t1","dimensions":{"provider":"google"}}]
  }
}')"
assert_contains "$ASYNC" '"committed":false' "metrics_ingest should default to accepted ack"
assert_contains "$ASYNC" '"is_async_ack":true' "metrics_ingest should report async ack"
assert_contains "$ASYNC" '"ack_status":"queued"' "metrics_ingest should queue"

CACHED="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{
  "db":"metric-events/main",
  "operation":"metrics_query",
  "payload":{
    "event":"api.request",
    "start":"2026-06-14",
    "end":"2026-06-14",
    "metrics":[{"op":"count","field":"*","alias":"requests","label":"Requests"}]
  }
}')"
assert_contains "$CACHED" '"requests":3' "metrics_query should cache-capable default query"

INVALIDATE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{
  "db":"metric-events/main",
  "operation":"metrics_query",
  "payload":{
    "cache":-1,
    "event":"api.request",
    "start":"2026-06-14",
    "end":"2026-06-14",
    "metrics":[{"op":"count","field":"*","alias":"requests","label":"Requests"}]
  }
}')"
assert_contains "$INVALIDATE" '"status":"success"' "cache=-1 invalidation should succeed"

echo "metric-events smoke passed. log: $LOG_FILE"
