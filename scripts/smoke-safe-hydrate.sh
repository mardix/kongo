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
DB="${KONGODB_SMOKE_DB:-smoke.safe_hydrate.main}"
NS="${KONGODB_SMOKE_NAMESPACE:-safe_hydrate_ns}"
SMOKE_ROOT="${KONGODB_SMOKE_ROOT:-./.smoke/safe-hydrate}"
LOG_FILE="${KONGODB_SMOKE_LOG:-${SMOKE_ROOT}/safe-hydrate.log}"
ACCESS_KEY="${KONGODB_ACCESS_KEY:-}"

S3_BUCKET="${KONGODB_S3_BUCKET:-}"
S3_PREFIX="${KONGODB_S3_PREFIX:-kongodb}"
S3_REGION="${KONGODB_S3_REGION:-us-east-1}"
S3_ENDPOINT="${KONGODB_S3_ENDPOINT:-}"
S3_ACCESS_KEY="${KONGODB_S3_ACCESS_KEY:-}"
S3_SECRET_KEY="${KONGODB_S3_SECRET_KEY:-}"
S3_SESSION_TOKEN="${KONGODB_S3_SESSION_TOKEN:-}"

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

aws_s3api_hot() {
  local bucket="$1"
  local region="$2"
  local endpoint="$3"
  local access_key="$4"
  local secret_key="$5"
  local session_token="$6"
  shift 6

  if [[ -n "$session_token" ]]; then
    if [[ -n "$endpoint" ]]; then
      AWS_ACCESS_KEY_ID="$access_key" AWS_SECRET_ACCESS_KEY="$secret_key" AWS_SESSION_TOKEN="$session_token" \
        aws --region "$region" --endpoint-url "$endpoint" s3api "$@"
    else
      AWS_ACCESS_KEY_ID="$access_key" AWS_SECRET_ACCESS_KEY="$secret_key" AWS_SESSION_TOKEN="$session_token" \
        aws --region "$region" s3api "$@"
    fi
  else
    if [[ -n "$endpoint" ]]; then
      AWS_ACCESS_KEY_ID="$access_key" AWS_SECRET_ACCESS_KEY="$secret_key" \
        aws --region "$region" --endpoint-url "$endpoint" s3api "$@"
    else
      AWS_ACCESS_KEY_ID="$access_key" AWS_SECRET_ACCESS_KEY="$secret_key" \
        aws --region "$region" s3api "$@"
    fi
  fi
}

need_cmd curl
mkdir -p "$SMOKE_ROOT"
if ! command -v aws >/dev/null 2>&1; then
  echo "smoke-safe-hydrate skipped: aws CLI not installed" | tee -a "$LOG_FILE"
  exit 0
fi
: > "$LOG_FILE"

if [[ -z "$S3_BUCKET" ]]; then
  echo "smoke-safe-hydrate skipped: KONGODB_S3_BUCKET not set" | tee -a "$LOG_FILE"
  exit 0
fi

HOT_BUCKET="$S3_BUCKET"
HOT_PREFIX="$S3_PREFIX"
HOT_REGION="$S3_REGION"
HOT_ENDPOINT="$S3_ENDPOINT"
HOT_ACCESS_KEY="$S3_ACCESS_KEY"
HOT_SECRET_KEY="$S3_SECRET_KEY"
HOT_SESSION_TOKEN="$S3_SESSION_TOKEN"
HOT_LABEL="s3"
if [[ -z "$HOT_BUCKET" || -z "$HOT_ACCESS_KEY" || -z "$HOT_SECRET_KEY" ]]; then
  echo "smoke-safe-hydrate skipped: missing remote s3 bucket/credentials for ${HOT_LABEL}" | tee -a "$LOG_FILE"
  exit 0
fi

echo "[1/7] ping server" | tee -a "$LOG_FILE"
PING="$(curl -sS "$BASE_URL/ping" || true)"
if ! grep -Fq '"status":"ok"' <<<"$PING"; then
  echo "smoke-safe-hydrate skipped: server not reachable at $BASE_URL" | tee -a "$LOG_FILE"
  exit 0
fi

echo "[2/7] create db + seed stable row" | tee -a "$LOG_FILE"
CREATE="$(api_post "{\"db\":\"$DB\",\"operation\":\"create_db\",\"payload\":{}}")"
assert_contains "$CREATE" '"status":"success"' "create should succeed"
SET_ROW="$(api_post "{\"db\":\"$DB\",\"operation\":\"set\",\"namespace\":\"$NS\",\"payload\":{\"data\":{\"_id\":\"safe-hydrate-row\",\"name\":\"before-corrupt-restore\"}}}")"
assert_contains "$SET_ROW" '"status":"success"' "set should succeed"

echo "[3/7] sync snapshot to remote" | tee -a "$LOG_FILE"
SYNC="$(api_post "{\"db\":\"$DB\",\"operation\":\"sync_db\",\"payload\":{}}")"
if grep -Fq '"status":"error"' <<<"$SYNC"; then
  assert_contains "$SYNC" "only supported in s3 mode" "safe hydrate smoke requires s3 mode"
  echo "smoke-safe-hydrate skipped: server is not in s3 mode" | tee -a "$LOG_FILE"
  exit 0
fi
assert_contains "$SYNC" '"status":"success"' "sync_db should succeed"

echo "[4/7] corrupt remote current snapshot object" | tee -a "$LOG_FILE"
CORRUPT_FILE="${SMOKE_ROOT}/corrupt-current.db"
printf 'not-a-sqlite-database' > "$CORRUPT_FILE"
HOT_KEY="${HOT_PREFIX%/}/${DB}/snapshots/current.db"
aws_s3api_hot "$HOT_BUCKET" "$HOT_REGION" "$HOT_ENDPOINT" "$HOT_ACCESS_KEY" "$HOT_SECRET_KEY" "$HOT_SESSION_TOKEN" \
  put-object \
  --bucket "$HOT_BUCKET" \
  --key "$HOT_KEY" \
  --body "$CORRUPT_FILE" >/dev/null

echo "[5/7] restore should fail quick_check" | tee -a "$LOG_FILE"
RESTORE="$(api_post "{\"db\":\"$DB\",\"operation\":\"restore_snapshot\",\"payload\":{}}")"
assert_contains "$RESTORE" '"status":"error"' "restore_snapshot should fail with corrupt snapshot"
assert_contains "$RESTORE" 'quick_check' "restore failure should come from quick_check gate"

echo "[6/7] verify local row still exists (anti-wipe)" | tee -a "$LOG_FILE"
GET_ROW="$(api_post "{\"db\":\"$DB\",\"operation\":\"get\",\"payload\":{\"id\":\"safe-hydrate-row\"}}")"
assert_contains "$GET_ROW" '"status":"success"' "get should succeed after failed restore"
assert_contains "$GET_ROW" '"count":1' "row should still exist after failed restore"
assert_contains "$GET_ROW" 'before-corrupt-restore' "row value should remain unchanged"

echo "[7/7] done" | tee -a "$LOG_FILE"
echo "safe hydrate smoke passed. log: $LOG_FILE"
