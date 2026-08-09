#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PORT="${KONGODB_PORT:-18096}"
SMOKE_ROOT="${KONGODB_SMOKE_ROOT:-./.smoke/multi-query}"
DATA_DIR="${KONGODB_DATA_DIR:-${SMOKE_ROOT}/data}"
LOG_FILE="${KONGODB_SMOKE_LOG:-${SMOKE_ROOT}/logs/smoke-multi-query.log}"
BIN="${KONGODB_BIN:-./target/debug/kongo}"
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

gateway() {
  curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "$1"
}

extract_job_id() {
  sed -n 's/.*"job_id":"\([^"]*\)".*/\1/p'
}

poll_job_completed() {
  local job_id="$1"
  for _ in $(seq 1 40); do
    local response
    response="$(gateway "{\"db\":\"multi/main\",\"operation\":\"get_job\",\"payload\":{\"job_id\":\"${job_id}\"}}")"
    if grep -Fq '"status":"completed"' <<<"$response"; then
      return 0
    fi
    if grep -Fq '"status":"failed"' <<<"$response"; then
      echo "job failed: $response" >&2
      return 1
    fi
    sleep 0.25
  done
  echo "timed out waiting for job: $job_id" >&2
  return 1
}

echo "[1/9] building kongo"
cargo build >/dev/null

echo "[2/9] preparing smoke dirs under $SMOKE_ROOT"
rm -rf "$SMOKE_ROOT"
mkdir -p "$DATA_DIR" "$(dirname "$LOG_FILE")"

echo "[3/9] starting server on :$PORT"
KONGODB_PORT="$PORT" \
KONGODB_STORAGE_MODE="local" \
KONGODB_DATA_DIR="$DATA_DIR" \
KONGODB_BASE_PATH="$BASE_PATH" \
KONGODB_AUTH_MODE="none" \
KONGODB_WRITE_MODE="committed" \
KONGODB_QUERY_MULTI_MAX_QUERIES="2" \
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

echo "[4/9] seed two namespaces"
USERS="$(gateway '{"db":"multi/main","operation":"insert","namespace":"users","payload":{"_user_id":"11111111111111111111111111111111","data":[{"_id":"u1","name":"Ada","status":"active"},{"_id":"u2","name":"Grace","status":"active"},{"_id":"u3","name":"Linus","status":"inactive"}]}}')"
IDENTITY="$(gateway '{"db":"multi/main","operation":"user_create","payload":{"user_id":"11111111111111111111111111111111","email":"ada@example.com","first_name":"Ada","last_name":"Lovelace"}}')"
ORDERS="$(gateway '{"db":"multi/main","operation":"insert","namespace":"orders","payload":{"data":[{"_id":"o1","user_id":"u1","status":"open"},{"_id":"o2","user_id":"u2","status":"closed"}]}}')"
assert_contains "$IDENTITY" '"status":"success"' "identity user create should succeed"
assert_contains "$USERS" '"status":"success"' "users insert should succeed"
assert_contains "$ORDERS" '"status":"success"' "orders insert should succeed"

echo "[5/9] enable and populate full-text search"
ENABLE_FTS="$(gateway '{"db":"multi/main","operation":"enable_fts_index","payload":{"enable":true}}')"
REINDEX_FTS="$(gateway '{"db":"multi/main","operation":"reindex_fts","payload":{}}')"
assert_contains "$ENABLE_FTS" '"fts_enabled":true' "database FTS should be enabled"
assert_contains "$REINDEX_FTS" '"job_type":"reindex_fts"' "FTS reindex should enqueue"
REINDEX_JOB_ID="$(extract_job_id <<<"$REINDEX_FTS")"
[[ -n "$REINDEX_JOB_ID" ]] || { echo "missing FTS reindex job id" >&2; exit 1; }
poll_job_completed "$REINDEX_JOB_ID"

echo "[6/9] execute structured and FTS queries with independent shaping"
SUCCESS_REQUEST='{
  "db":"multi/main",
  "operation":"multi_query",
  "payload":{
    "operations":[
      {"alias":"users","namespace":"users","payload":{"filter":{"status":"active"},"sort":"name asc","fields":["_id","name","orders"],"lookups":{"orders":{"from":"orders","local_field":"_id","foreign_field":"user_id","match":"$eq","multi":true,"limit":10}},"attach_users":true,"page":1,"per_page":1,"cache":true}},
      {"alias":"search_users","namespace":"users","payload":{"search":"Ada","fields":["_id","name","_search_score"],"limit":10}}
    ]
  }
}'
SUCCESS="$(gateway "$SUCCESS_REQUEST")"
SUCCESS_CACHED="$(gateway "$SUCCESS_REQUEST")"
MULTI_NAMESPACE="$(gateway '{"db":"multi/main","operation":"query","namespace":["users","orders"],"payload":{"sort":"_id asc","fields":["_id"],"limit":10}}')"
assert_contains "$SUCCESS" '"status":"success"' "multi_query should succeed"
assert_contains "$SUCCESS" '"alias":"users"' "users result should retain alias"
assert_contains "$SUCCESS" '"alias":"search_users"' "FTS result should retain alias"
assert_contains "$SUCCESS" '"_search_score"' "FTS child should use the normal search engine"
assert_contains "$SUCCESS" '"orders":[' "lookup output should remain available per child"
assert_contains "$SUCCESS" '"attachments":{"users"' "attached users should remain available per child"
assert_contains "$SUCCESS" '"first_name":"Ada"' "attached user fields should be returned"
assert_contains "$SUCCESS" '"total_items":2' "users child should preserve total count"
assert_contains "$SUCCESS" '"per_page":1' "users child should preserve pagination"
assert_contains "$SUCCESS" '"succeeded":2' "both children should succeed"
assert_contains "$SUCCESS" '"failed":0' "successful batch should report no failures"
assert_contains "$SUCCESS_CACHED" '"succeeded":2' "repeated cache-enabled child queries should succeed"
assert_contains "$MULTI_NAMESPACE" '"status":"success"' "namespace array query should succeed"
assert_contains "$MULTI_NAMESPACE" '"_namespace":"users"' "namespace array query should expose the source namespace"
assert_contains "$MULTI_NAMESPACE" '"_namespace":"orders"' "namespace array query should include every selected namespace"

echo "[7/9] continue after one runtime query error"
PARTIAL="$(gateway '{
  "db":"multi/main",
  "operation":"multi_query",
  "payload":{
    "on_error":"continue",
    "operations":[
      {"alias":"users","namespace":"users","payload":{"limit":1}},
      {"alias":"broken","namespace":"orders","payload":{"sort":"name sideways"}}
    ]
  }
}')"
assert_contains "$PARTIAL" '"status":"partial"' "continue mode should return partial status"
assert_contains "$PARTIAL" '"succeeded":1' "partial batch should report success count"
assert_contains "$PARTIAL" '"failed":1' "partial batch should report failure count"
assert_contains "$PARTIAL" '"alias":"broken"' "failed child should retain alias"
assert_contains "$PARTIAL" '"status":"error"' "failed child should expose error status"
assert_contains "$PARTIAL" '"code":"bad_request"' "failed child should expose stable error code"

echo "[8/9] fail fast on runtime errors"
FAIL_FAST="$(gateway '{
  "db":"multi/main",
  "operation":"multi_query",
  "payload":{
    "operations":[
      {"alias":"users","namespace":"users","payload":{"limit":1}},
      {"alias":"broken","namespace":"orders","payload":{"sort":"name sideways"}}
    ]
  }
}')"
assert_contains "$FAIL_FAST" '"status":"error"' "default fail mode should fail request"
assert_contains "$FAIL_FAST" 'sort direction' "fail-fast error should preserve query error"

echo "[9/9] enforce structural validation and configured batch cap"
DUPLICATE="$(gateway '{"db":"multi/main","operation":"multi_query","payload":{"operations":[{"alias":"same","namespace":"users","payload":{}},{"alias":"same","namespace":"orders","payload":{}}]}}')"
MISSING_SCOPE="$(gateway '{"db":"multi/main","operation":"multi_query","payload":{"operations":[{"alias":"users","payload":{}}]}}')"
LEGACY_NAMESPACES="$(gateway '{"db":"multi/main","operation":"multi_query","payload":{"operations":[{"alias":"users","namespaces":["users"],"payload":{}}]}}')"
OVERSIZED="$(gateway '{"db":"multi/main","operation":"multi_query","payload":{"operations":[{"alias":"one","namespace":"users","payload":{}},{"alias":"two","namespace":"orders","payload":{}},{"alias":"three","namespace":"users","payload":{}}]}}')"
LEGACY_QUERIES="$(gateway '{"db":"multi/main","operation":"multi_query","payload":{"queries":[{"alias":"users","namespace":"users","payload":{}}]}}')"
assert_contains "$DUPLICATE" 'alias must be unique' "duplicate aliases should be rejected"
assert_contains "$MISSING_SCOPE" 'requires namespace' "child namespace should be required"
assert_contains "$LEGACY_NAMESPACES" 'requires namespace' "removed child namespaces selector should be rejected"
assert_contains "$OVERSIZED" 'configured maximum of 2' "configured child cap should be enforced"
assert_contains "$LEGACY_QUERIES" 'multi_query operations is required' "removed payload.queries shape should be rejected"

echo "multi-query smoke passed. log: $LOG_FILE"
