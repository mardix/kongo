#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PORT="${KONGODB_PORT:-18092}"
SMOKE_ROOT="${KONGODB_SMOKE_ROOT:-./.smoke/identity}"
DATA_DIR="${KONGODB_DATA_DIR:-${SMOKE_ROOT}/data}"
LOG_FILE="${KONGODB_SMOKE_LOG:-${SMOKE_ROOT}/logs/smoke-identity.log}"
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

echo "[4/7] create identity database"
CREATE_DB="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"identity/main","operation":"create_db","payload":{}}')"
assert_contains "$CREATE_DB" '"status":"success"' "create_db should succeed"

echo "[5/7] create identity requiring password change"
CREATE_USER="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{
  "db":"identity/main",
  "operation":"user_create",
  "payload":{
    "email":"password-change@example.com",
    "first_name":"Password",
    "last_name":"Change",
    "requires_password_change":true
  }
}')"
assert_contains "$CREATE_USER" '"status":"success"' "user_create should succeed"
assert_contains "$CREATE_USER" '"requires_password_change":true' "user_create should return the enabled flag"
USER_ID="$(sed -n 's/.*"id":"\([^"]*\)".*/\1/p' <<<"$CREATE_USER")"
[[ -n "$USER_ID" ]] || { echo "failed to extract created user id" >&2; exit 1; }

LIST_USERS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"identity/main","operation":"user_list","payload":{}}')"
assert_contains "$LIST_USERS" '"requires_password_change":true' "user_list should return the enabled flag"

echo "[6/7] clear password-change requirement"
UPDATE_USER="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"identity/main\",\"operation\":\"user_update\",\"payload\":{\"user_id\":\"$USER_ID\",\"requires_password_change\":false}}")"
assert_contains "$UPDATE_USER" '"status":"success"' "user_update should succeed"
assert_contains "$UPDATE_USER" '"requires_password_change":false' "user_update should clear the flag"

echo "[7/7] fetch identity with cleared requirement"
GET_USER="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"identity/main\",\"operation\":\"user_get\",\"payload\":{\"user_id\":\"$USER_ID\"}}")"
assert_contains "$GET_USER" '"requires_password_change":false' "user_get should return the cleared flag"

echo "identity smoke passed. log: $LOG_FILE"
