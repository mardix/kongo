#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PORT="${KONGODB_PORT:-18082}"
SMOKE_ROOT="${KONGODB_SMOKE_ROOT:-./.smoke/auth}"
DATA_DIR="${KONGODB_DATA_DIR:-${SMOKE_ROOT}/data}"
LOG_FILE="${KONGODB_SMOKE_LOG:-${SMOKE_ROOT}/logs/auth.log}"
BIN="${KONGODB_BIN:-./target/debug/kongodb}"
BASE_URL="http://127.0.0.1:${PORT}"
BASE_PATH_RAW="${KONGODB_BASE_PATH:-}"
BASE_PATH="/${BASE_PATH_RAW#/}"
BASE_PATH="${BASE_PATH%/}"
if [[ "$BASE_PATH" == "/" ]]; then BASE_PATH=""; fi
GATEWAY_PATH="${BASE_PATH}/gateway"
GATEWAY_URL="${BASE_URL}${GATEWAY_PATH}"
AUTH_KEY="smoke_secret_key"

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

need_cmd cargo
need_cmd curl

echo "[1/7] building kongodb"
cargo build >/dev/null

if [[ ! -x "$BIN" ]]; then
  echo "binary not found: $BIN" >&2
  exit 1
fi

echo "[2/7] startup auth configuration validation"
mkdir -p "$SMOKE_ROOT/logs"
set +e
KONGODB_PORT="$PORT" \
KONGODB_STORAGE_MODE="local" \
KONGODB_DATA_DIR="$DATA_DIR" \
KONGODB_BASE_PATH="$BASE_PATH" \
KONGODB_AUTH_MODE="invalid" \
KONGODB_ACCESS_KEY="" \
"$BIN" >"${SMOKE_ROOT}/logs/startup-invalid-mode.log" 2>&1
invalid_mode_rc=$?
set -e
if [[ "$invalid_mode_rc" -eq 0 ]]; then
  echo "assertion failed: server should reject an invalid auth mode" >&2
  exit 1
fi
assert_contains "$(cat "${SMOKE_ROOT}/logs/startup-invalid-mode.log")" \
  'KONGODB_AUTH_MODE must be access_key|none' \
  "invalid auth mode should report accepted values"

set +e
KONGODB_PORT="$PORT" \
KONGODB_STORAGE_MODE="local" \
KONGODB_DATA_DIR="$DATA_DIR" \
KONGODB_BASE_PATH="$BASE_PATH" \
KONGODB_AUTH_MODE="access_key" \
KONGODB_ACCESS_KEY="" \
"$BIN" >"${SMOKE_ROOT}/logs/startup-fail.log" 2>&1
rc=$?
set -e
if [[ "$rc" -eq 0 ]]; then
  echo "assertion failed: server should fail startup without auth key" >&2
  exit 1
fi

echo "[3/7] start server with access key"
rm -rf "$DATA_DIR"
mkdir -p "$DATA_DIR" "$(dirname "$LOG_FILE")"
KONGODB_PORT="$PORT" \
KONGODB_STORAGE_MODE="local" \
KONGODB_DATA_DIR="$DATA_DIR" \
KONGODB_BASE_PATH="$BASE_PATH" \
KONGODB_AUTH_MODE="access_key" \
KONGODB_ACCESS_KEY="$AUTH_KEY" \
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

echo "[4/7] gateway auth checks"
HTTP_NO_KEY="$(curl -sS -o /tmp/kdb_auth_no_key.json -w "%{http_code}" -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"authdb","operation":"create_db","payload":{}}')"
[[ "$HTTP_NO_KEY" == "401" ]] || { echo "expected 401 without key, got $HTTP_NO_KEY" >&2; exit 1; }
assert_contains "$(cat /tmp/kdb_auth_no_key.json)" '"status":"error"' "unauthorized response should be error json"

HTTP_BAD_KEY="$(curl -sS -o /tmp/kdb_auth_bad_key.json -w "%{http_code}" -X POST "$GATEWAY_URL" -H 'content-type: application/json' -H 'X-Access-Key: wrong' -d '{"db":"authdb","operation":"create_db","payload":{}}')"
[[ "$HTTP_BAD_KEY" == "401" ]] || { echo "expected 401 with wrong key, got $HTTP_BAD_KEY" >&2; exit 1; }

CREATE_OK="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -H "X-Access-Key: ${AUTH_KEY}" -d '{"db":"authdb","operation":"create_db","payload":{}}')"
assert_contains "$CREATE_OK" '"status":"success"' "create_db should succeed with valid key"

echo "[5/7] metadata always follows service authentication"
META_NO_KEY_CODE="$(curl -sS -o /tmp/kdb_meta_no_key.json -w "%{http_code}" "$BASE_URL/meta/operations")"
[[ "$META_NO_KEY_CODE" == "401" ]] || { echo "expected 401 meta without key, got $META_NO_KEY_CODE" >&2; exit 1; }
META_OK_CODE="$(curl -sS -o /tmp/kdb_meta_ok.json -w "%{http_code}" -H "X-Access-Key: ${AUTH_KEY}" "$BASE_URL/meta/operations")"
[[ "$META_OK_CODE" == "200" ]] || { echo "expected 200 meta with key, got $META_OK_CODE" >&2; exit 1; }
assert_contains "$(cat /tmp/kdb_meta_ok.json)" '"operations"' "meta should include operations"

echo "[6/7] browser docs authentication"
DOCS_NO_KEY_CODE="$(curl -sS -o /tmp/kdb_docs_no_key.html -w "%{http_code}" "$BASE_URL/doc")"
[[ "$DOCS_NO_KEY_CODE" == "401" ]] || { echo "expected 401 docs without credentials, got $DOCS_NO_KEY_CODE" >&2; exit 1; }
DOCS_OK_CODE="$(curl -sS -o /tmp/kdb_docs_ok.html -w "%{http_code}" -u "kongodb:${AUTH_KEY}" "$BASE_URL/doc")"
[[ "$DOCS_OK_CODE" == "200" ]] || { echo "expected 200 docs with Basic credentials, got $DOCS_OK_CODE" >&2; exit 1; }

echo "[7/7] done"
echo "auth smoke passed. log: $LOG_FILE"
