#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PORT="${KONGODB_PORT:-18083}"
SMOKE_ROOT="${KONGODB_SMOKE_ROOT:-./.smoke/lookup}"
DATA_DIR="${KONGODB_DATA_DIR:-${SMOKE_ROOT}/data}"
LOG_FILE="${KONGODB_SMOKE_LOG:-${SMOKE_ROOT}/logs/lookup-edge.log}"
BIN="${KONGODB_BIN:-./target/debug/kongodb}"
BASE_URL="http://127.0.0.1:${PORT}"
BASE_PATH_RAW="${KONGODB_BASE_PATH:-}"
BASE_PATH="/${BASE_PATH_RAW#/}"
BASE_PATH="${BASE_PATH%/}"
if [[ "$BASE_PATH" == "/" ]]; then BASE_PATH=""; fi
GATEWAY_PATH="${BASE_PATH}/gateway"
GATEWAY_URL="${BASE_URL}${GATEWAY_PATH}"

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

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

assert_not_contains() {
  local haystack="$1"
  local needle="$2"
  local msg="$3"
  if grep -Fq "$needle" <<<"$haystack"; then
    echo "assertion failed: $msg" >&2
    echo "response: $haystack" >&2
    exit 1
  fi
}

need_cmd cargo
need_cmd curl

echo "[1/9] building kongodb"
cargo build >/dev/null

if [[ ! -x "$BIN" ]]; then
  echo "binary not found: $BIN" >&2
  exit 1
fi

echo "[2/9] starting lookup edge server"
rm -rf "$DATA_DIR"
mkdir -p "$DATA_DIR" "$(dirname "$LOG_FILE")"
KONGODB_PORT="$PORT" \
KONGODB_STORAGE_MODE="local" \
KONGODB_DATA_DIR="$DATA_DIR" \
KONGODB_BASE_PATH="$BASE_PATH" \
KONGODB_AUTH_MODE="none" \
KONGODB_QUERY_LOOKUP_MAX_DEPTH="3" \
KONGODB_QUERY_LOOKUP_UNCAPPED_OVERRIDE_ENABLED="false" \
"$BIN" >"$LOG_FILE" 2>&1 &
PID=$!
cleanup() {
  kill "$PID" >/dev/null 2>&1 || true
}
trap cleanup EXIT

for _ in $(seq 1 40); do
  if curl -sS -o /dev/null "$BASE_URL/ping"; then
    break
  fi
  sleep 0.25
done

echo "[3/9] seed collections"
curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"lkp","operation":"insert","payload":{"collection":"users","data":[{"_id":"u1","name":"User1","book_ids":["b1","b2"]}]}}' >/dev/null
curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"lkp","operation":"insert","payload":{"collection":"books","data":[{"_id":"b1","name":"Book1"},{"_id":"b2","name":"Book2"}]}}' >/dev/null
curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"lkp","operation":"insert","payload":{"collection":"n1","data":[{"_id":"a1","next":"x1"}]}}' >/dev/null
curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"lkp","operation":"insert","payload":{"collection":"n2","data":[{"_id":"x1","next":"y1"}]}}' >/dev/null
curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"lkp","operation":"insert","payload":{"collection":"n3","data":[{"_id":"y1","next":"z1"}]}}' >/dev/null
curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"lkp","operation":"insert","payload":{"collection":"n4","data":[{"_id":"z1","name":"N4"}]}}' >/dev/null

echo "[4/9] unknown alias reference rejected"
BAD_ALIAS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"lkp","operation":"query","payload":{"collection":"users","lookups":{"vendors":{"from":"books","local_field":"$lookup.missing[].id","foreign_field":"_id","match":"$in","multi":true,"limit":10}}}}')"
assert_contains "$BAD_ALIAS" '"status":"error"' "unknown alias reference should fail"
assert_contains "$BAD_ALIAS" 'unknown alias' "unknown alias message should be present"

echo "[5/9] cycle detection rejected"
CYCLE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"lkp","operation":"query","payload":{"collection":"users","lookups":{"a":{"from":"books","local_field":"$lookup.b._id","foreign_field":"_id","match":"$eq"},"b":{"from":"books","local_field":"$lookup.a._id","foreign_field":"_id","match":"$eq"}}}}')"
assert_contains "$CYCLE" '"status":"error"' "cycle should fail"
assert_contains "$CYCLE" 'cycle' "cycle message should be present"

echo "[6/9] strict_path and on_missing behavior"
STRICT_FAIL="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"lkp","operation":"query","payload":{"collection":"users","lookups":{"books":{"from":"books","local_field":"missing.path","foreign_field":"_id","match":"$eq","strict_path":true}}}}')"
assert_contains "$STRICT_FAIL" '"status":"error"' "strict_path should fail on missing path"

ON_MISSING_EMPTY="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"lkp","operation":"query","payload":{"collection":"users","lookups":{"none_books":{"from":"books","local_field":"missing[]","foreign_field":"_id","match":"$in","multi":true,"limit":5,"on_missing":"empty"}}}}')"
assert_contains "$ON_MISSING_EMPTY" '"none_books":[]' "on_missing=empty should return empty array"

ON_MISSING_DROP="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"lkp","operation":"query","payload":{"collection":"users","lookups":{"none_books":{"from":"books","local_field":"missing[]","foreign_field":"_id","match":"$in","multi":true,"limit":5,"on_missing":"drop"}}}}')"
assert_contains "$ON_MISSING_DROP" '"count":0' "on_missing=drop should remove parent rows"

echo "[7/9] depth cap and uncapped override"
DEPTH_FAIL="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"lkp","operation":"query","payload":{"collection":"n1","lookup_depth_override":4,"lookups":{"l2":{"from":"n2","local_field":"next","foreign_field":"_id","match":"$eq","lookups":{"l3":{"from":"n3","local_field":"next","foreign_field":"_id","match":"$eq","lookups":{"l4":{"from":"n4","local_field":"next","foreign_field":"_id","match":"$eq"}}}}}}}}')"
assert_contains "$DEPTH_FAIL" '"status":"error"' "depth override over cap should fail when uncapped disabled"
assert_contains "$DEPTH_FAIL" 'exceeds max depth' "depth cap message should be present"

kill "$PID" >/dev/null 2>&1 || true
KONGODB_PORT="$PORT" \
KONGODB_STORAGE_MODE="local" \
KONGODB_DATA_DIR="$DATA_DIR" \
KONGODB_BASE_PATH="$BASE_PATH" \
KONGODB_AUTH_MODE="none" \
KONGODB_QUERY_LOOKUP_MAX_DEPTH="3" \
KONGODB_QUERY_LOOKUP_UNCAPPED_OVERRIDE_ENABLED="true" \
"$BIN" >"$LOG_FILE" 2>&1 &
PID=$!
for _ in $(seq 1 40); do
  if curl -sS -o /dev/null "$BASE_URL/ping"; then
    break
  fi
  sleep 0.25
done

DEPTH_OK="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"lkp","operation":"query","payload":{"collection":"n1","lookup_depth_override":4,"lookups":{"l2":{"from":"n2","local_field":"next","foreign_field":"_id","match":"$eq","lookups":{"l3":{"from":"n3","local_field":"next","foreign_field":"_id","match":"$eq","lookups":{"l4":{"from":"n4","local_field":"next","foreign_field":"_id","match":"$eq","fields":["_id","name"]}}}}}}}}')"
assert_contains "$DEPTH_OK" '"status":"success"' "uncapped override should allow deeper lookup"
assert_contains "$DEPTH_OK" '"l4"' "deep nested alias should be present"

echo "[8/9] get supports lookups + fields + exclude_fields"
GET_LOOKUP_PROJ="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"lkp","operation":"get","payload":{"collection":"users","id":"u1","lookups":{"books":{"from":"books","local_field":"book_ids[]","foreign_field":"_id","match":"$in","multi":true,"limit":10,"fields":["_id","name","vendor_id"]},"vendors":{"from":"books","local_field":"$lookup.books[].vendor_id","foreign_field":"_id","match":"$in","multi":true,"limit":10,"fields":["_id","name"]}},"fields":["name","books","vendors"],"exclude_fields":["books.vendor_id"]}}')"
assert_contains "$GET_LOOKUP_PROJ" '"status":"success"' "get lookup+projection should succeed"
assert_contains "$GET_LOOKUP_PROJ" '"books":' "get should include books lookup"
assert_contains "$GET_LOOKUP_PROJ" '"vendors":' "get should include sibling lookup"
assert_not_contains "$GET_LOOKUP_PROJ" '"vendor_id"' "exclude_fields should remove nested vendor_id from books"

echo "[9/9] done"
echo "lookup edge smoke passed. log: $LOG_FILE"
