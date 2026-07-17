#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

BASE_URL="${KONGODB_SMOKE_URL:-http://127.0.0.1:8080}"
BASE_PATH_RAW="${KONGODB_BASE_PATH:-}"
BASE_PATH="/${BASE_PATH_RAW#/}"
BASE_PATH="${BASE_PATH%/}"
if [[ "$BASE_PATH" == "/" ]]; then BASE_PATH=""; fi
GATEWAY_PATH="${BASE_PATH}/gateway"
GATEWAY_URL="${BASE_URL}${GATEWAY_PATH}"
DB="${KONGODB_SMOKE_DB:-smoke.snapshot.main}"
NS="${KONGODB_SMOKE_NAMESPACE:-snap_users}"
SMOKE_ROOT="${KONGODB_SMOKE_ROOT:-./.smoke/snapshot}"
LOG_FILE="${KONGODB_SMOKE_LOG:-${SMOKE_ROOT}/snapshot.log}"
ACCESS_KEY="${KONGODB_ACCESS_KEY:-}"

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

api_post() {
  local body="$1"
  if [[ -n "$ACCESS_KEY" ]]; then
    curl -sS -X POST "$GATEWAY_URL" \
      -H 'content-type: application/json' \
      -H "X-Access-Key: $ACCESS_KEY" \
      -d "$body"
  else
    curl -sS -X POST "$GATEWAY_URL" \
      -H 'content-type: application/json' \
      -d "$body"
  fi
}

need_cmd curl
mkdir -p "$(dirname "$LOG_FILE")"
: > "$LOG_FILE"

echo "[1/7] ping server" | tee -a "$LOG_FILE"
PING="$(curl -sS "$BASE_URL/ping" || true)"
if ! grep -Fq '"status":"ok"' <<<"$PING"; then
  echo "snapshot smoke skipped: server not reachable at $BASE_URL" | tee -a "$LOG_FILE"
  exit 0
fi

echo "[2/7] create db and seed one row" | tee -a "$LOG_FILE"
CREATE="$(api_post "{\"db\":\"$DB\",\"operation\":\"create_db\",\"payload\":{}}")"
assert_contains "$CREATE" '"status":"success"' "create should succeed"
INSERT="$(api_post "{\"db\":\"$DB\",\"operation\":\"insert\",\"namespace\":\"$NS\",\"payload\":{\"data\":{\"name\":\"snap-user\"}}}")"
assert_contains "$INSERT" '"status":"success"' "insert should succeed"

echo "[3/7] create_snapshot (alias of sync_db)" | tee -a "$LOG_FILE"
SNAP1="$(api_post "{\"db\":\"$DB\",\"operation\":\"create_snapshot\",\"payload\":{}}")"
if grep -Fq '"status":"error"' <<<"$SNAP1"; then
  assert_contains "$SNAP1" "only supported in s3 mode" "snapshot smoke requires s3 mode"
  echo "snapshot smoke skipped: server is not in s3 mode" | tee -a "$LOG_FILE"
  exit 0
fi
assert_contains "$SNAP1" '"status":"success"' "create_snapshot should succeed"
assert_contains "$SNAP1" '"snapshot_id":"' "create_snapshot must return snapshot_id"

echo "[4/7] list_snapshots" | tee -a "$LOG_FILE"
LIST1="$(api_post "{\"db\":\"$DB\",\"operation\":\"list_snapshots\",\"payload\":{}}")"
assert_contains "$LIST1" '"status":"success"' "list_snapshots should succeed"
assert_contains "$LIST1" '"snapshots":[' "list_snapshots should include snapshots array"

SNAP_ID="$(grep -o '"snapshot_id":"[^"]*"' <<<"$SNAP1" | head -n1 | sed 's/"snapshot_id":"//;s/"$//')"
if [[ -z "$SNAP_ID" ]]; then
  echo "failed to parse snapshot_id from create_snapshot response" >&2
  echo "$SNAP1" >&2
  exit 1
fi

echo "[5/7] mutate data then create second snapshot" | tee -a "$LOG_FILE"
SET1="$(api_post "{\"db\":\"$DB\",\"operation\":\"set\",\"namespace\":\"$NS\",\"payload\":{\"data\":{\"_id\":\"seed-fixed\",\"state\":\"after-snap-1\"}}}")"
assert_contains "$SET1" '"status":"success"' "set should succeed"
SNAP2="$(api_post "{\"db\":\"$DB\",\"operation\":\"sync_db\",\"payload\":{}}")"
assert_contains "$SNAP2" '"status":"success"' "sync_db should succeed"

echo "[6/7] restore_snapshot using first snapshot_id" | tee -a "$LOG_FILE"
RESTORE="$(api_post "{\"db\":\"$DB\",\"operation\":\"restore_snapshot\",\"payload\":{\"snapshot_id\":\"$SNAP_ID\"}}")"
assert_contains "$RESTORE" '"status":"success"' "restore_snapshot should succeed"
assert_contains "$RESTORE" '"restored":true' "restore_snapshot should return restored=true"

echo "[7/7] final snapshot list" | tee -a "$LOG_FILE"
LIST2="$(api_post "{\"db\":\"$DB\",\"operation\":\"list_snapshots\",\"payload\":{}}")"
assert_contains "$LIST2" '"status":"success"' "final list_snapshots should succeed"

echo "snapshot smoke passed. log: $LOG_FILE"
