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
SMOKE_ROOT="${KONGODB_SMOKE_ROOT:-./.smoke/import-jobs}"
DATA_DIR="${KONGODB_DATA_DIR:-${SMOKE_ROOT}/data}"
LOG_FILE="${KONGODB_SMOKE_LOG:-${SMOKE_ROOT}/logs/import-jobs.log}"
BIN="${KONGODB_BIN:-./target/debug/kongodb}"
DB="${KONGODB_SMOKE_DB:-smoke.import.jobs}"
NS="${KONGODB_SMOKE_NAMESPACE:-imports_users}"

GOOD_JSONL="${SMOKE_ROOT}/good.jsonl"
BAD_JSONL="${SMOKE_ROOT}/bad.jsonl"

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

json_extract_first() {
  local body="$1"
  local key="$2"
  grep -o "\"${key}\":\"[^\"]*\"" <<<"$body" | head -n1 | sed "s/\"${key}\":\"//;s/\"$//"
}

need_cmd cargo
need_cmd curl

echo "[1/9] building kongodb"
cargo build >/dev/null

if [[ ! -x "$BIN" ]]; then
  echo "binary not found: $BIN" >&2
  exit 1
fi

echo "[2/9] preparing smoke dirs"
rm -rf "$SMOKE_ROOT"
mkdir -p "$DATA_DIR" "$(dirname "$LOG_FILE")"

cat >"$GOOD_JSONL" <<'JSONL'
{"_key":"u1","name":"Alice","legacy":{"drop":"x"}}
{"_key":"u2","name":"Bob","legacy":{"drop":"y"}}
JSONL

cat >"$BAD_JSONL" <<'JSONL'
{"_key":"b1","name":"Bad One"}
{"_key":"broken"
{"_key":"b2","name":"Bad Two"}
JSONL

echo "[3/9] starting server on :$PORT"
export KONGODB_PORT="$PORT"
export KONGODB_STORAGE_MODE="local"
export KONGODB_DATA_DIR="$DATA_DIR"
export KONGODB_BASE_PATH="$BASE_PATH"
export KONGODB_AUTH_MODE="none"

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

echo "[4/9] create db"
CREATE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"create_db\",\"payload\":{}}")"
assert_contains "$CREATE" '"status":"success"' "create should succeed"

echo "[5/9] enqueue good import job"
ENQ_GOOD="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"import_jsonl\",\"namespace\":\"$NS\",\"payload\":{\"source_path\":\"$GOOD_JSONL\",\"on_conflict\":\"error\",\"batch_size\":2,\"alias_import_pk\":\"_key:_id\",\"drop_keys\":[\"legacy.drop\"]}}")"
assert_contains "$ENQ_GOOD" '"status":"success"' "good import enqueue should succeed"
GOOD_JOB_ID="$(json_extract_first "$ENQ_GOOD" "job_id")"
if [[ -z "$GOOD_JOB_ID" ]]; then
  echo "failed to parse good import job_id" >&2
  echo "$ENQ_GOOD" >&2
  exit 1
fi

GOOD_DONE=0
for _ in $(seq 1 40); do
  JOB="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"get_job\",\"payload\":{\"job_id\":\"$GOOD_JOB_ID\"}}")"
  assert_contains "$JOB" '"status":"success"' "get_job should succeed"
  if grep -Fq '"status":"completed"' <<<"$JOB"; then
    GOOD_DONE=1
    break
  fi
  sleep 0.25
done
if [[ "$GOOD_DONE" -ne 1 ]]; then
  echo "good import job did not complete in time" >&2
  exit 1
fi

GET_U1="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"get\",\"namespace\":\"$NS\",\"payload\":{\"id\":\"u1\"}}")"
assert_contains "$GET_U1" '"status":"success"' "get u1 should succeed"
assert_contains "$GET_U1" '"_id":"u1"' "alias_import_pk should map _key to _id"
assert_not_contains "$GET_U1" '"drop":"x"' "drop_keys should remove nested path"

echo "[6/9] enqueue bad resumable import job"
ENQ_BAD="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"import_jsonl\",\"namespace\":\"$NS\",\"payload\":{\"source_path\":\"$BAD_JSONL\",\"on_conflict\":\"error\",\"batch_size\":1,\"alias_import_pk\":\"_key:_id\",\"resumable\":true}}")"
assert_contains "$ENQ_BAD" '"status":"success"' "bad import enqueue should succeed"
BAD_JOB_ID="$(json_extract_first "$ENQ_BAD" "job_id")"
if [[ -z "$BAD_JOB_ID" ]]; then
  echo "failed to parse bad import job_id" >&2
  echo "$ENQ_BAD" >&2
  exit 1
fi

BAD_FAILED=0
for _ in $(seq 1 40); do
  JOB="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"get_job\",\"payload\":{\"job_id\":\"$BAD_JOB_ID\"}}")"
  if grep -Fq '"status":"failed"' <<<"$JOB"; then
    BAD_FAILED=1
    break
  fi
  sleep 0.25
done
if [[ "$BAD_FAILED" -ne 1 ]]; then
  echo "bad import job did not fail in time" >&2
  exit 1
fi

echo "[7/9] continue failed resumable import"
CONT="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"continue_job\",\"payload\":{\"job_id\":\"$BAD_JOB_ID\"}}")"
assert_contains "$CONT" '"status":"success"' "continue_job should succeed"
assert_contains "$CONT" '"retrying"' "continue_job should set retrying"

POST_CONTINUE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"get_job\",\"payload\":{\"job_id\":\"$BAD_JOB_ID\"}}")"
assert_contains "$POST_CONTINUE" "\"job_id\":\"$BAD_JOB_ID\"" "get_job should still resolve continued import job"

echo "[8/9] abort failed job"
ABORT="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"abort_job\",\"payload\":{\"job_id\":\"$BAD_JOB_ID\"}}")"
assert_contains "$ABORT" '"status":"success"' "abort_job should succeed"
assert_contains "$ABORT" '"aborted"' "abort_job should set aborted"

FINAL="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"$DB\",\"operation\":\"get_job\",\"payload\":{\"job_id\":\"$BAD_JOB_ID\"}}")"
assert_contains "$FINAL" '"status":"aborted"' "job should be aborted"

echo "[9/9] done"
echo "import jobs smoke passed. log: $LOG_FILE"
