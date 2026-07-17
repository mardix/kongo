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
DB="${KONGODB_SMOKE_DB:-smoke.import.s3}"
NS="${KONGODB_SMOKE_NAMESPACE:-imports_s3}"
SMOKE_ROOT="${KONGODB_SMOKE_ROOT:-./.smoke/import-s3}"
LOG_FILE="${KONGODB_SMOKE_LOG:-${SMOKE_ROOT}/import-s3.log}"
ACCESS_KEY="${KONGODB_ACCESS_KEY:-}"

S3_BUCKET="${KONGODB_S3_BUCKET:-}"
S3_PREFIX="${KONGODB_S3_PREFIX:-kongodb}"
S3_REGION="${KONGODB_S3_REGION:-us-east-1}"
S3_ENDPOINT="${KONGODB_S3_ENDPOINT:-}"

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

aws_s3api() {
  if [[ -n "$S3_ENDPOINT" ]]; then
    aws --region "$S3_REGION" --endpoint-url "$S3_ENDPOINT" s3api "$@"
  else
    aws --region "$S3_REGION" s3api "$@"
  fi
}

need_cmd curl
if ! command -v aws >/dev/null 2>&1; then
  echo "smoke-import-s3 skipped: aws CLI not installed"
  exit 0
fi
need_cmd shasum

mkdir -p "$SMOKE_ROOT"
: > "$LOG_FILE"

if [[ -z "$S3_BUCKET" ]]; then
  echo "smoke-import-s3 skipped: KONGODB_S3_BUCKET not set" | tee -a "$LOG_FILE"
  exit 0
fi

echo "[1/8] ping server" | tee -a "$LOG_FILE"
PING="$(curl -sS "$BASE_URL/ping" || true)"
assert_contains "$PING" '"status":"ok"' "server must be reachable"

echo "[2/8] create db" | tee -a "$LOG_FILE"
CREATE="$(api_post "{\"db\":\"$DB\",\"operation\":\"create_db\",\"payload\":{}}")"
assert_contains "$CREATE" '"status":"success"' "create should succeed"

NOW="$(date -u +%Y%m%dT%H%M%SZ)"
MISMATCH_KEY="${S3_PREFIX%/}/smoke/import/${DB//\//_}/mismatch_${NOW}.jsonl"
RESUME_KEY="${S3_PREFIX%/}/smoke/import/${DB//\//_}/resume_${NOW}.jsonl"
MISMATCH_URI="s3://${S3_BUCKET}/${MISMATCH_KEY}"
RESUME_URI="s3://${S3_BUCKET}/${RESUME_KEY}"

MISMATCH_FILE="${SMOKE_ROOT}/mismatch.jsonl"
RESUME_FILE="${SMOKE_ROOT}/resume.jsonl"

cat >"$MISMATCH_FILE" <<'JSONL'
{"_id":"m1","name":"Mismatch One"}
JSONL

cat >"$RESUME_FILE" <<'JSONL'
{"_id":"r1","name":"Resume One"}
{"_id":"broken"
{"_id":"r2","name":"Resume Two"}
JSONL

MISMATCH_HASH="$(shasum -a 256 "$MISMATCH_FILE" | awk '{print $1}')"
RESUME_HASH="$(shasum -a 256 "$RESUME_FILE" | awk '{print $1}')"

echo "[3/8] upload s3 objects with source-hash metadata" | tee -a "$LOG_FILE"
aws_s3api put-object \
  --bucket "$S3_BUCKET" \
  --key "$MISMATCH_KEY" \
  --body "$MISMATCH_FILE" \
  --metadata "source-hash=${MISMATCH_HASH}" >/dev/null
aws_s3api put-object \
  --bucket "$S3_BUCKET" \
  --key "$RESUME_KEY" \
  --body "$RESUME_FILE" \
  --metadata "source-hash=${RESUME_HASH}" >/dev/null

echo "[4/8] source_hash mismatch should fail job" | tee -a "$LOG_FILE"
ENQ_MISMATCH="$(api_post "{\"db\":\"$DB\",\"operation\":\"import_jsonl\",\"namespace\":\"$NS\",\"payload\":{\"source_path\":\"$MISMATCH_URI\",\"source_hash\":\"deadbeef\",\"on_conflict\":\"error\",\"batch_size\":1,\"resumable\":true}}")"
assert_contains "$ENQ_MISMATCH" '"status":"success"' "mismatch enqueue should succeed"
MISMATCH_JOB="$(json_extract_first "$ENQ_MISMATCH" "job_id")"
if [[ -z "$MISMATCH_JOB" ]]; then
  echo "failed to parse mismatch job id" >&2
  exit 1
fi

MISMATCH_FAILED=0
for _ in $(seq 1 60); do
  JOB="$(api_post "{\"db\":\"$DB\",\"operation\":\"get_import_job\",\"payload\":{\"job_id\":\"$MISMATCH_JOB\"}}")"
  if grep -Fq '"status":"failed"' <<<"$JOB"; then
    assert_contains "$JOB" 'source_hash mismatch' "mismatch error message should be present"
    MISMATCH_FAILED=1
    break
  fi
  sleep 0.25
done
if [[ "$MISMATCH_FAILED" -ne 1 ]]; then
  echo "mismatch job did not fail in time" >&2
  exit 1
fi

echo "[5/8] enqueue resumable job for offset resume test" | tee -a "$LOG_FILE"
ENQ_RESUME="$(api_post "{\"db\":\"$DB\",\"operation\":\"import_jsonl\",\"namespace\":\"$NS\",\"payload\":{\"source_path\":\"$RESUME_URI\",\"source_hash\":\"$RESUME_HASH\",\"on_conflict\":\"error\",\"batch_size\":1,\"resumable\":true}}")"
assert_contains "$ENQ_RESUME" '"status":"success"' "resume enqueue should succeed"
RESUME_JOB="$(json_extract_first "$ENQ_RESUME" "job_id")"
if [[ -z "$RESUME_JOB" ]]; then
  echo "failed to parse resume job id" >&2
  exit 1
fi

FAILED_ONCE=0
for _ in $(seq 1 60); do
  JOB="$(api_post "{\"db\":\"$DB\",\"operation\":\"get_import_job\",\"payload\":{\"job_id\":\"$RESUME_JOB\"}}")"
  if grep -Fq '"status":"failed"' <<<"$JOB"; then
    assert_contains "$JOB" '"last_byte_offset":' "job should include byte offset checkpoint"
    FAILED_ONCE=1
    break
  fi
  sleep 0.25
done
if [[ "$FAILED_ONCE" -ne 1 ]]; then
  echo "resume job did not fail first attempt in time" >&2
  exit 1
fi

echo "[6/8] continue failed job" | tee -a "$LOG_FILE"
CONT="$(api_post "{\"db\":\"$DB\",\"operation\":\"continue_import\",\"payload\":{\"job_id\":\"$RESUME_JOB\"}}")"
assert_contains "$CONT" '"status":"success"' "continue_import should succeed"

COMPLETED=0
for _ in $(seq 1 60); do
  JOB="$(api_post "{\"db\":\"$DB\",\"operation\":\"get_import_job\",\"payload\":{\"job_id\":\"$RESUME_JOB\"}}")"
  if grep -Fq '"status":"completed"' <<<"$JOB"; then
    COMPLETED=1
    break
  fi
  sleep 0.25
done
if [[ "$COMPLETED" -ne 1 ]]; then
  echo "resume job did not complete after continue" >&2
  exit 1
fi

echo "[7/8] verify imported rows" | tee -a "$LOG_FILE"
COUNT="$(api_post "{\"db\":\"$DB\",\"operation\":\"count\",\"namespace\":\"$NS\",\"payload\":{}}")"
assert_contains "$COUNT" '"count":2' "resume flow should import 2 valid rows"

echo "[8/8] done" | tee -a "$LOG_FILE"
echo "import s3 smoke passed. log: $LOG_FILE"
