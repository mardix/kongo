#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PORT="${KONGODB_PORT:-18094}"
SMOKE_ROOT="${KONGODB_SMOKE_ROOT:-./.smoke/document-lifecycle}"
DATA_DIR="${KONGODB_DATA_DIR:-${SMOKE_ROOT}/data}"
LOG_FILE="${KONGODB_SMOKE_LOG:-${SMOKE_ROOT}/logs/smoke-document-lifecycle.log}"
BIN="${KONGODB_BIN:-./target/debug/kongo}"
BASE_URL="http://127.0.0.1:${PORT}"
GATEWAY_URL="${BASE_URL}/gateway"

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

echo "[1/8] building kongo"
cargo build >/dev/null

echo "[2/8] preparing smoke dirs under $SMOKE_ROOT"
rm -rf "$SMOKE_ROOT"
mkdir -p "$DATA_DIR" "$(dirname "$LOG_FILE")"

echo "[3/8] starting server on :$PORT"
KONGODB_PORT="$PORT" \
KONGODB_STORAGE_MODE="local" \
KONGODB_DATA_DIR="$DATA_DIR" \
KONGODB_BASE_PATH="" \
KONGODB_AUTH_MODE="none" \
KONGODB_WRITE_MODE="accepted" \
KONGODB_REAPER_INTERVAL_SECS="60" \
"$BIN" >"$LOG_FILE" 2>&1 &
PID=$!
cleanup() { kill "$PID" >/dev/null 2>&1 || true; }
trap cleanup EXIT

READY=false
for _ in $(seq 1 40); do
  if curl -fsS -o /dev/null "$BASE_URL/ping" 2>/dev/null; then READY=true; break; fi
  sleep 0.25
done
if [[ "$READY" != "true" ]]; then
  cat "$LOG_FILE" >&2
  exit 1
fi

echo "[4/8] create document with attached lifecycle"
gateway '{"db":"lifecycle/main","operation":"create_db","payload":{}}' >/dev/null
INSERT="$(gateway '{
  "db":"lifecycle/main","operation":"insert","namespace":"accounts",
  "payload":{"commit":true,"data":{"_id":"doc-lifecycle-1","status":"active","score":1},
    "lifecycle":{"name":"deactivate","after_seconds":3600,"when":{"status":"active"},"update":{"status":"inactive"}}}
}')"
assert_contains "$INSERT" '"status":"success"' "insert with lifecycle should succeed"
assert_contains "$INSERT" '"lifecycle":{"count":1' "insert should return lifecycle metadata"

echo "[5/8] replace named transition and execute through reap_db"
SCHEDULE="$(gateway '{
  "db":"lifecycle/main","operation":"schedule_transition","namespace":"accounts",
  "payload":{"document_id":"doc-lifecycle-1","name":"deactivate","after_seconds":1,
    "when":{"status":"active"},"update":{"status":"inactive","score":{"$inc":2}}}
}')"
assert_contains "$SCHEDULE" '"count":1' "schedule_transition should replace named transition"
LIST="$(gateway '{"db":"lifecycle/main","operation":"list_transitions","payload":{"document_id":"doc-lifecycle-1","status":"pending","page":1,"per_page":10}}')"
assert_contains "$LIST" '"total_items":1' "same document/name should have one pending transition"
sleep 1.2
REAP="$(gateway '{"db":"lifecycle/main","operation":"reap_db","payload":{"commit":true}}')"
assert_contains "$REAP" '"completed_count":1' "reap_db should execute due transition"
QUERY="$(gateway '{"db":"lifecycle/main","operation":"query","namespace":"accounts","payload":{"filter":{"_id":"doc-lifecycle-1"}}}')"
assert_contains "$QUERY" '"status":"inactive"' "transition should patch document"
assert_contains "$QUERY" '"score":3' "transition should apply mutation operators"

echo "[6/8] skip false condition and preserve terminal history"
gateway '{
  "db":"lifecycle/main","operation":"schedule_transition",
  "payload":{"document_id":"doc-lifecycle-1","name":"conditional","after_seconds":1,
    "when":{"status":"active"},"update":{"condition_ran":true}}
}' >/dev/null
sleep 1.2
gateway '{"db":"lifecycle/main","operation":"reap_db","payload":{"commit":true}}' >/dev/null
SKIPPED="$(gateway '{"db":"lifecycle/main","operation":"get_transition","payload":{"document_id":"doc-lifecycle-1","name":"conditional"}}')"
assert_contains "$SKIPPED" '"status":"skipped"' "false condition should skip transition"
assert_contains "$SKIPPED" '"skipped_reason":"condition_not_met"' "skip reason should be explicit"

echo "[7/8] cancel pending transition and enforce retry guard"
UPDATE_ATTACHED="$(gateway '{
  "db":"lifecycle/main","operation":"update","namespace":"accounts",
  "payload":{"commit":true,"data":{"_id":"doc-lifecycle-1","reviewed":true},
    "lifecycle":{"name":"update-attached","after_seconds":3600,"when":{},"update":{"review_expired":true}}}
}')"
assert_contains "$UPDATE_ATTACHED" '"lifecycle":{"count":1' "explicit update should attach lifecycle"
UPSERT_ATTACHED="$(gateway '{
  "db":"lifecycle/main","operation":"upsert","namespace":"accounts",
  "payload":{"commit":true,"filter":{"_id":"doc-lifecycle-2"},
    "insert_data":{"status":"new"},"update_data":{"status":"existing"},"max_docs":1,
    "lifecycle":{"name":"upsert-attached","after_seconds":3600,"when":{},"update":{"status":"aged"}}}
}')"
assert_contains "$UPSERT_ATTACHED" '"lifecycle":{"count":1' "singular upsert should attach lifecycle"
gateway '{"db":"lifecycle/main","operation":"delete","namespace":"wrong-namespace","payload":{"id":"doc-lifecycle-2","purge":true,"commit":true}}' >/dev/null
STRICT_TRANSITION="$(gateway '{"db":"lifecycle/main","operation":"get_transition","payload":{"document_id":"doc-lifecycle-2","name":"upsert-attached"}}')"
assert_contains "$STRICT_TRANSITION" '"status":"pending"' "wrong-namespace purge must preserve transition and document"
TX_ATTACHED="$(gateway '{
  "db":"lifecycle/main","operation":"transaction","data":[
    {"operation":"insert","payload":{"collection":"accounts","data":{"_id":"doc-lifecycle-tx","status":"new"},
      "lifecycle":{"name":"tx-attached","after_seconds":3600,"when":{},"update":{"status":"aged"}}}}
  ],"payload":{"commit":true}
}')"
assert_contains "$TX_ATTACHED" '"transaction_committed"' "transaction insert with lifecycle should commit"
TX_TRANSITION="$(gateway '{"db":"lifecycle/main","operation":"get_transition","payload":{"document_id":"doc-lifecycle-tx","name":"tx-attached"}}')"
assert_contains "$TX_TRANSITION" '"status":"pending"' "transaction should persist lifecycle atomically"
gateway '{
  "db":"lifecycle/main","operation":"schedule_transition",
  "payload":{"document_id":"doc-lifecycle-1","name":"cancel-me","after_seconds":3600,
    "when":{},"update":{"cancelled_transition_ran":true}}
}' >/dev/null
CANCEL="$(gateway '{"db":"lifecycle/main","operation":"cancel_transition","payload":{"document_id":"doc-lifecycle-1","name":"cancel-me"}}')"
assert_contains "$CANCEL" '"cancelled_count":1' "cancel should update pending transition"
RETRY="$(gateway '{"db":"lifecycle/main","operation":"retry_transition","payload":{"document_id":"doc-lifecycle-1","name":"cancel-me"}}')"
assert_contains "$RETRY" '"retried_count":0' "only failed transitions should retry"

echo "[8/8] soft delete cancels pending lifecycle"
gateway '{
  "db":"lifecycle/main","operation":"schedule_transition",
  "payload":{"document_id":"doc-lifecycle-1","name":"delete-cancel","after_seconds":3600,
    "when":{},"update":{"should_not_run":true}}
}' >/dev/null
gateway '{"db":"lifecycle/main","operation":"delete","payload":{"id":"doc-lifecycle-1","commit":true}}' >/dev/null
DELETED_TRANSITION="$(gateway '{"db":"lifecycle/main","operation":"get_transition","payload":{"document_id":"doc-lifecycle-1","name":"delete-cancel"}}')"
assert_contains "$DELETED_TRANSITION" '"status":"cancelled"' "soft delete should cancel pending lifecycle"

echo "document lifecycle smoke passed. log: $LOG_FILE"
