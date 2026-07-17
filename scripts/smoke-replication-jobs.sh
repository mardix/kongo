#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PORT="${KONGODB_PORT:-18084}"
BASE_URL="http://127.0.0.1:${PORT}"
BASE_PATH_RAW="${KONGODB_BASE_PATH:-}"
BASE_PATH="/${BASE_PATH_RAW#/}"
BASE_PATH="${BASE_PATH%/}"
if [[ "$BASE_PATH" == "/" ]]; then BASE_PATH=""; fi
GATEWAY_PATH="${BASE_PATH}/gateway"
GATEWAY_URL="${BASE_URL}${GATEWAY_PATH}"
SMOKE_ROOT="${KONGODB_SMOKE_ROOT:-./.smoke/replication-jobs}"
DATA_DIR="${KONGODB_DATA_DIR:-${SMOKE_ROOT}/data}"
LOG_FILE="${KONGODB_SMOKE_LOG:-${SMOKE_ROOT}/logs/smoke-replication-jobs.log}"
BIN="${KONGODB_BIN:-./target/debug/kongodb}"
DB="${KONGODB_SMOKE_DB:-smoke.replication.jobs}"

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

echo "[2/7] preparing smoke dirs under $SMOKE_ROOT"
rm -rf "$SMOKE_ROOT"
mkdir -p "$DATA_DIR" "$(dirname "$LOG_FILE")"

echo "[3/7] starting server on :$PORT"
export KONGODB_PORT="$PORT"
export KONGODB_STORAGE_MODE="local"
export KONGODB_DATA_DIR="$DATA_DIR"
export KONGODB_AUTH_MODE="none"
export KONGODB_BASE_PATH="$BASE_PATH"
export KONGODB_REPLICATION_MODE="async"

"$BIN" >"$LOG_FILE" 2>&1 &
PID=$!
cleanup() {
  kill "$PID" >/dev/null 2>&1 || true
}
trap cleanup EXIT

ready=0
for _ in $(seq 1 40); do
  if ! kill -0 "$PID" >/dev/null 2>&1; then
    echo "assertion failed: server process exited before readiness check" >&2
    echo "server log: $LOG_FILE" >&2
    exit 1
  fi
  if grep -Eqi 'failed to bind listener|panicked at' "$LOG_FILE" 2>/dev/null; then
    echo "assertion failed: server failed during startup" >&2
    echo "server log: $LOG_FILE" >&2
    exit 1
  fi
  if curl -sS "$BASE_URL/ping" | grep -Fq '"status":"ok"'; then
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

echo "[4/7] create db + verify meta operations expose unified job ops"
CREATE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"create_db\",\"payload\":{}}")"
assert_contains "$CREATE" '"status":"success"' "create should succeed"
META="$(curl -sS "$BASE_URL/meta/operations")"
assert_contains "$META" '"list_jobs"' "meta should include list_jobs"
assert_contains "$META" '"get_job"' "meta should include get_job"
assert_contains "$META" '"continue_job"' "meta should include continue_job"
assert_contains "$META" '"abort_job"' "meta should include abort_job"

echo "[5/7] list_jobs no-op behavior for replication job type"
LIST_EMPTY="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"list_jobs\",\"payload\":{\"job_type\":\"replication\"}}")"
assert_contains "$LIST_EMPTY" '"status":"success"' "list_jobs should succeed"
assert_contains "$LIST_EMPTY" '"count":0' "list_jobs should be empty on fresh db"

echo "[6/7] guardrails"
BAD_JOB="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"get_job\",\"payload\":{\"job_id\":\"missing-job\"}}")"
assert_contains "$BAD_JOB" '"status":"error"' "unknown job should be rejected"

echo "[7/7] done"
echo "replication jobs smoke passed. log: $LOG_FILE"
