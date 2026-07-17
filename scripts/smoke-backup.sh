#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PORT="${KONGODB_PORT:-18081}"
BASE_URL="http://127.0.0.1:${PORT}"
BASE_PATH_RAW="${KONGODB_BASE_PATH:-}"
BASE_PATH="/${BASE_PATH_RAW#/}"
BASE_PATH="${BASE_PATH%/}"
if [[ "$BASE_PATH" == "/" ]]; then BASE_PATH=""; fi
GATEWAY_PATH="${BASE_PATH}/gateway"
GATEWAY_URL="${BASE_URL}${GATEWAY_PATH}"
SMOKE_ROOT="${KONGODB_SMOKE_ROOT:-./.smoke/backup}"
DATA_DIR="${KONGODB_DATA_DIR:-${SMOKE_ROOT}/data}"
BACKUP_DIR="${KONGODB_BACKUP_PATH:-${SMOKE_ROOT}/backups}"
LOG_FILE="${KONGODB_SMOKE_LOG:-${SMOKE_ROOT}/logs/smoke-backup.log}"
BIN="${KONGODB_BIN:-./target/debug/kongodb}"
DB="${KONGODB_SMOKE_DB:-smoke.backup.main}"
NS="${KONGODB_SMOKE_NAMESPACE:-backup_users}"

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

json_extract_first() {
  local body="$1"
  local key="$2"
  grep -o "\"${key}\":\"[^\"]*\"" <<<"$body" | head -n1 | sed "s/\"${key}\":\"//;s/\"$//"
}

wait_for_job_completed() {
  local db="$1"
  local job_id="$2"
  local timeout_secs="${3:-30}"
  local started
  started="$(date +%s)"
  while true; do
    local now
    now="$(date +%s)"
    if (( now - started > timeout_secs )); then
      echo "timed out waiting for job completion: $job_id" >&2
      return 1
    fi
    local body
    body="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$db\",\"operation\":\"get_job\",\"payload\":{\"job_id\":\"$job_id\"}}")"
    if grep -q '"status":"completed"' <<<"$body"; then
      echo "$body"
      return 0
    fi
    if grep -q '"status":"failed"' <<<"$body"; then
      echo "$body" >&2
      return 1
    fi
    sleep 0.25
  done
}

need_cmd cargo
need_cmd curl
need_cmd find

echo "[1/8] building kongodb"
cargo build >/dev/null

if [[ ! -x "$BIN" ]]; then
  echo "binary not found: $BIN" >&2
  exit 1
fi

echo "[2/8] preparing smoke dirs under $SMOKE_ROOT"
rm -rf "$SMOKE_ROOT"
mkdir -p "$DATA_DIR" "$BACKUP_DIR" "$(dirname "$LOG_FILE")"

echo "[3/8] starting server on :$PORT"
export KONGODB_PORT="$PORT"
export KONGODB_STORAGE_MODE="local"
export KONGODB_DATA_DIR="$DATA_DIR"
export KONGODB_AUTH_MODE="none"
export KONGODB_BASE_PATH="$BASE_PATH"
export KONGODB_BACKUP_EVERY_SECS="1"
export KONGODB_BACKUP_PATH="$BACKUP_DIR"

"$BIN" >"$LOG_FILE" 2>&1 &
PID=$!
cleanup() {
  kill "$PID" >/dev/null 2>&1 || true
}
trap cleanup EXIT

ready=0
for _ in $(seq 1 40); do
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

echo "[4/8] seed db"
CREATE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"create_db\",\"payload\":{}}")"
assert_contains "$CREATE" '"status":"success"' "create_db should succeed"
INS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"insert\",\"namespace\":\"$NS\",\"payload\":{\"data\":{\"name\":\"backup-smoke\"}}}")"
assert_contains "$INS" '"status":"success"' "insert should succeed"
BASE_SET="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"set\",\"namespace\":\"$NS\",\"payload\":{\"data\":{\"_id\":\"restore-doc\",\"value\":\"before\"}}}")"
assert_contains "$BASE_SET" '"status":"success"' "set baseline should succeed"

echo "[5/8] manual create_backup"
MANUAL="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"create_backup\",\"payload\":{}}")"
assert_contains "$MANUAL" '"status":"success"' "create_backup should succeed"
JOB_ID="$(json_extract_first "$MANUAL" "job_id")"
if [[ -z "$JOB_ID" ]]; then
  echo "assertion failed: backup job_id should be present" >&2
  echo "$MANUAL" >&2
  exit 1
fi
BACKUP_JOB="$(wait_for_job_completed "$DB" "$JOB_ID" 40)"
BACKUP_PATH="$(json_extract_first "$BACKUP_JOB" "backup_db_path")"
if [[ -z "$BACKUP_PATH" ]]; then
  echo "assertion failed: backup_db_path should be present" >&2
  echo "$BACKUP_JOB" >&2
  exit 1
fi
if [[ ! -f "$BACKUP_PATH" ]]; then
  echo "assertion failed: backup_db_path file should exist: $BACKUP_PATH" >&2
  exit 1
fi

if ! grep -Eq '.*/[a-zA-Z0-9._-]+--[a-f0-9]{16}/[0-9]{8}T[0-9]{6}Z_0001\.db\.zst$' <<<"$BACKUP_PATH"; then
  echo "assertion failed: backup_db_path should match slug--hash/timestamp format" >&2
  echo "path: $BACKUP_PATH" >&2
  exit 1
fi

echo "[6/8] restore_backup from artifact"
MUTATE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"set\",\"namespace\":\"$NS\",\"payload\":{\"data\":{\"_id\":\"restore-doc\",\"value\":\"after\"}}}")"
assert_contains "$MUTATE" '"status":"success"' "mutate after backup should succeed"
RESTORE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"restore_backup\",\"payload\":{\"backup_db_path\":\"$BACKUP_PATH\"}}")"
assert_contains "$RESTORE" '"status":"success"' "restore_backup should succeed"
GET_AFTER="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"get\",\"payload\":{\"id\":\"restore-doc\"}}")"
assert_contains "$GET_AFTER" '"value":"before"' "restore_backup should restore pre-mutation state"

echo "[7/8] auto-backup worker"
before_count="$(find "$BACKUP_DIR" -type f -name '*.db.zst' | wc -l | tr -d ' ')"
sleep 3
after_count="$(find "$BACKUP_DIR" -type f -name '*.db.zst' | wc -l | tr -d ' ')"
if [[ "$after_count" -le "$before_count" ]]; then
  echo "assertion failed: auto-backup worker should create additional backups ($before_count -> $after_count)" >&2
  exit 1
fi

echo "[8/8] done"
echo "backup smoke passed. log: $LOG_FILE"
