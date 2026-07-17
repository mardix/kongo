#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PORT="${KONGODB_PORT:-18086}"
BASE_URL="http://127.0.0.1:${PORT}"
BASE_PATH_RAW="${KONGODB_BASE_PATH:-}"
BASE_PATH="/${BASE_PATH_RAW#/}"
BASE_PATH="${BASE_PATH%/}"
if [[ "$BASE_PATH" == "/" ]]; then BASE_PATH=""; fi
GATEWAY_PATH="${BASE_PATH}/gateway"
GATEWAY_URL="${BASE_URL}${GATEWAY_PATH}"
SMOKE_ROOT="${KONGODB_SMOKE_ROOT:-./.smoke/export-import-zst}"
DATA_DIR="${KONGODB_DATA_DIR:-${SMOKE_ROOT}/data}"
LOG_FILE="${KONGODB_SMOKE_LOG:-${SMOKE_ROOT}/logs/smoke-export-import-zst.log}"
BIN="${KONGODB_BIN:-./target/debug/kongodb}"
DB="${KONGODB_SMOKE_DB:-smoke.export.import.zst}"
SRC_NS="${KONGODB_SMOKE_SOURCE_NS:-users_src}"
DST_NS="${KONGODB_SMOKE_TARGET_NS:-users_dst}"
EXPORT_PREFIX="${SMOKE_ROOT}/exports/users_export"

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

echo "[1/8] building kongodb"
cargo build >/dev/null

if [[ ! -x "$BIN" ]]; then
  echo "binary not found: $BIN" >&2
  exit 1
fi

echo "[2/8] preparing smoke dirs"
rm -rf "$SMOKE_ROOT"
mkdir -p "$DATA_DIR" "$(dirname "$LOG_FILE")" "$(dirname "$EXPORT_PREFIX")"

echo "[3/8] starting server on :$PORT"
export KONGODB_PORT="$PORT"
export KONGODB_STORAGE_MODE="local"
export KONGODB_DATA_DIR="$DATA_DIR"
export KONGODB_BASE_PATH="$BASE_PATH"
export KONGODB_AUTH_MODE="none"
export KONGODB_EXPORT_PATH="${KONGODB_EXPORT_PATH:-${SMOKE_ROOT}/exports}"

"$BIN" >"$LOG_FILE" 2>&1 &
PID=$!
cleanup() {
  kill "$PID" >/dev/null 2>&1 || true
}
trap cleanup EXIT

for _ in $(seq 1 40); do
  if curl -sS "$BASE_URL/ping" | grep -Fq '"status":"ok"'; then
    break
  fi
  sleep 0.25
done

echo "[4/8] seed source namespace"
CREATE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"create_db\",\"payload\":{}}")"
assert_contains "$CREATE" '"status":"success"' "create should succeed"
SEED="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"insert\",\"namespace\":\"$SRC_NS\",\"payload\":{\"data\":[{\"_id\":\"u1\",\"name\":\"Alice\",\"role\":\"admin\"},{\"_id\":\"u2\",\"name\":\"Bob\",\"role\":\"owner\"},{\"_id\":\"u3\",\"name\":\"Eve\",\"role\":\"viewer\"}]}}")"
assert_contains "$SEED" '"inserted_count":3' "seed insert should insert 3 rows"

echo "[5/8] enqueue export_jsonl (zst default)"
ENQ_EXPORT="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"export_jsonl\",\"namespace\":\"$SRC_NS\",\"payload\":{\"target_path\":\"$EXPORT_PREFIX\",\"sort\":\"name asc\"}}")"
assert_contains "$ENQ_EXPORT" '"status":"success"' "export enqueue should succeed"
EXPORT_JOB_ID="$(json_extract_first "$ENQ_EXPORT" "job_id")"
EXPORT_PATH="$(json_extract_first "$ENQ_EXPORT" "target_path")"
if [[ -z "$EXPORT_JOB_ID" || -z "$EXPORT_PATH" ]]; then
  echo "failed to parse export enqueue response" >&2
  echo "$ENQ_EXPORT" >&2
  exit 1
fi
assert_contains "$EXPORT_PATH" ".jsonl.zst" "export target path should be zst by default"

wait_for_job_completed "$DB" "$EXPORT_JOB_ID" 40 >/dev/null
if [[ ! -f "$EXPORT_PATH" ]]; then
  echo "export target file missing: $EXPORT_PATH" >&2
  exit 1
fi
if [[ -d "${EXPORT_PATH}.parts" ]]; then
  echo "export parts directory should have been cleaned: ${EXPORT_PATH}.parts" >&2
  exit 1
fi

echo "[6/8] import from exported .jsonl.zst"
ENQ_IMPORT="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"import_jsonl\",\"namespace\":\"$DST_NS\",\"payload\":{\"source_path\":\"$EXPORT_PATH\",\"on_conflict\":\"error\",\"ignore_input_id\":true,\"allow_system_timestamps\":true}}")"
assert_contains "$ENQ_IMPORT" '"status":"success"' "import enqueue should succeed"
IMPORT_JOB_ID="$(json_extract_first "$ENQ_IMPORT" "job_id")"
if [[ -z "$IMPORT_JOB_ID" ]]; then
  echo "failed to parse import enqueue response" >&2
  echo "$ENQ_IMPORT" >&2
  exit 1
fi

wait_for_job_completed "$DB" "$IMPORT_JOB_ID" 40 >/dev/null

echo "[7/8] verify roundtrip count"
SRC_COUNT="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"count\",\"namespace\":\"$SRC_NS\",\"payload\":{}}")"
DST_COUNT="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"count\",\"namespace\":\"$DST_NS\",\"payload\":{}}")"
assert_contains "$SRC_COUNT" '"count":3' "source count should be 3"
assert_contains "$DST_COUNT" '"count":3' "destination count should be 3"

echo "[8/8] done"
echo "export/import zst smoke passed. log: $LOG_FILE"
