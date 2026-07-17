#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PORT="${KONGODB_PORT:-18085}"
SMOKE_ROOT="${KONGODB_SMOKE_ROOT:-./.smoke/fts-jobs}"
DATA_DIR="${KONGODB_DATA_DIR:-${SMOKE_ROOT}/data}"
LOG_FILE="${KONGODB_SMOKE_LOG:-${SMOKE_ROOT}/logs/smoke-fts-jobs.log}"
BIN="${KONGODB_BIN:-./target/debug/kongodb}"
BASE_URL="http://127.0.0.1:${PORT}"
BASE_PATH_RAW="${KONGODB_BASE_PATH:-}"
BASE_PATH="/${BASE_PATH_RAW#/}"
BASE_PATH="${BASE_PATH%/}"
if [[ "$BASE_PATH" == "/" ]]; then BASE_PATH=""; fi
GATEWAY_URL="${BASE_URL}${BASE_PATH}/gateway"

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

extract_job_id() {
  sed -n 's/.*"job_id":"\([^"]*\)".*/\1/p'
}

poll_job_status() {
  local db="$1"
  local job_id="$2"
  local expected="$3"
  local tries=40
  local i=0
  while [[ $i -lt $tries ]]; do
    local out
    out="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"${db}\",\"operation\":\"get_job\",\"payload\":{\"job_id\":\"${job_id}\"}}")"
    if grep -Fq "\"status\":\"${expected}\"" <<<"$out"; then
      echo "$out"
      return 0
    fi
    if grep -Fq '"status":"failed"' <<<"$out"; then
      echo "$out" >&2
      return 1
    fi
    sleep 0.25
    i=$((i + 1))
  done
  echo "timed out waiting for job ${job_id} -> ${expected}" >&2
  return 1
}

need_cmd cargo
need_cmd curl

echo "[1/10] building kongodb"
cargo build >/dev/null

if [[ ! -x "$BIN" ]]; then
  echo "binary not found: $BIN" >&2
  exit 1
fi

echo "[2/10] preparing smoke dirs under $SMOKE_ROOT"
rm -rf "$SMOKE_ROOT"
mkdir -p "$DATA_DIR" "$(dirname "$LOG_FILE")"

echo "[3/10] starting server on :$PORT"
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

for _ in $(seq 1 40); do
  if curl -sS -o /dev/null "$BASE_URL/ping"; then
    break
  fi
  sleep 0.25
done

echo "[4/10] create db + insert baseline doc"
CREATE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"ftsdb","operation":"create_db","payload":{}}')"
assert_contains "$CREATE" '"status":"success"' "create_db should succeed"
INSERT="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"ftsdb","operation":"insert","namespace":"docs","payload":{"data":{"_id":"d1","title":"Hello FTS world"}}}')"
assert_contains "$INSERT" '"status":"success"' "insert should succeed"

echo "[5/10] get_system_config should show fts disabled by default at db-level"
CFG1="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"ftsdb","operation":"get_system_config","payload":{}}')"
assert_contains "$CFG1" '"key":"fts_enabled"' "system config should include fts_enabled"
assert_contains "$CFG1" '"value":"0"' "fts_enabled should default to 0"

echo "[6/10] search should fail while db fts flag is disabled"
SEARCH_DISABLED="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"ftsdb","operation":"search","namespace":"docs","payload":{"search":"Hello"}}')"
assert_contains "$SEARCH_DISABLED" 'search is disable: db_config.fts_enabled=false' "search disabled message should match"

echo "[7/10] enable fts flag + queue reindex job"
ENABLE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"ftsdb","operation":"enable_fts_index","payload":{"enable":true}}')"
assert_contains "$ENABLE" '"fts_enabled":true' "enable_fts_index should set flag true"
REINDEX="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"ftsdb","operation":"reindex_fts","payload":{}}')"
assert_contains "$REINDEX" '"job_type":"reindex_fts"' "reindex_fts should enqueue job"
REINDEX_JOB_ID="$(echo "$REINDEX" | extract_job_id)"
[[ -n "$REINDEX_JOB_ID" ]] || { echo "missing reindex job_id" >&2; exit 1; }
REINDEX_DONE="$(poll_job_status "ftsdb" "$REINDEX_JOB_ID" "completed")"
assert_contains "$REINDEX_DONE" '"job_type":"reindex_fts"' "get_job should return fts reindex job"

echo "[8/10] search should succeed after reindex"
SEARCH_OK="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"ftsdb","operation":"search","namespace":"docs","payload":{"search":"Hello"}}')"
assert_contains "$SEARCH_OK" '"status":"success"' "search should succeed after reindex"
assert_contains "$SEARCH_OK" '"_id":"d1"' "search should return inserted doc"

echo "[9/10] queue drop_fts_index and verify job listing"
DROP="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"ftsdb","operation":"drop_fts_index","payload":{}}')"
assert_contains "$DROP" '"job_type":"drop_fts_index"' "drop_fts_index should enqueue job"
DROP_JOB_ID="$(echo "$DROP" | extract_job_id)"
[[ -n "$DROP_JOB_ID" ]] || { echo "missing drop job_id" >&2; exit 1; }
DROP_JOB="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"ftsdb\",\"operation\":\"get_job\",\"payload\":{\"job_id\":\"$DROP_JOB_ID\"}}")"
assert_contains "$DROP_JOB" '"job_type":"drop_fts_index"' "get_job should return fts drop job"
LIST_FTS_JOBS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"ftsdb","operation":"list_jobs","payload":{"job_type":"reindex_fts","limit":5}}')"
assert_contains "$LIST_FTS_JOBS" '"job_type":"reindex_fts"' "list_jobs should include fts reindex jobs"

echo "[10/10] done"
echo "fts jobs smoke passed. log: $LOG_FILE"
