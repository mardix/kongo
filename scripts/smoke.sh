#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PORT="${KONGODB_PORT:-18080}"
SMOKE_ROOT="${KONGODB_SMOKE_ROOT:-./.smoke}"
DATA_DIR="${KONGODB_DATA_DIR:-${SMOKE_ROOT}/data}"
LOG_FILE="${KONGODB_SMOKE_LOG:-${SMOKE_ROOT}/logs/smoke.log}"
BIN="${KONGODB_BIN:-./target/debug/kongo}"
ARCHIVE_TTL="${KONGODB_ARCHIVE_TTL_SECS:-}"
BASE_URL="http://127.0.0.1:${PORT}"
BASE_PATH_RAW="${KONGODB_BASE_PATH:-}"
BASE_PATH="/${BASE_PATH_RAW#/}"
BASE_PATH="${BASE_PATH%/}"
if [[ "$BASE_PATH" == "/" ]]; then BASE_PATH=""; fi
GATEWAY_PATH="${BASE_PATH}/gateway"
GATEWAY_URL="${BASE_URL}${GATEWAY_PATH}"

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

need_cmd cargo
need_cmd curl

assert_contains() {
  local haystack="$1"
  local needle="$2"
  local msg="$3"
  if ! grep -q "$needle" <<<"$haystack"; then
    echo "assertion failed: $msg" >&2
    echo "response: $haystack" >&2
    exit 1
  fi
}

assert_not_contains() {
  local haystack="$1"
  local needle="$2"
  local msg="$3"
  if grep -q "$needle" <<<"$haystack"; then
    echo "assertion failed: $msg" >&2
    echo "response: $haystack" >&2
    exit 1
  fi
}

wait_for_job_completed() {
  local db="$1"
  local job_id="$2"
  local timeout_secs="${3:-20}"
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

echo "[1/22] building kongodb"
cargo build >/dev/null

if [[ ! -x "$BIN" ]]; then
  echo "binary not found: $BIN" >&2
  exit 1
fi

echo "[2/22] preparing data dir: $DATA_DIR"
rm -rf "$DATA_DIR"
mkdir -p "$DATA_DIR"
mkdir -p "$(dirname "$LOG_FILE")"

echo "[3/22] starting server on :$PORT"
export KONGODB_PORT="$PORT"
export KONGODB_STORAGE_MODE="local"
export KONGODB_DATA_DIR="$DATA_DIR"
export KONGODB_AUTH_MODE="none"
export KONGODB_BASE_PATH="$BASE_PATH"
export KONGODB_MAX_ACTIVE_DBS="2"
if [[ -n "$ARCHIVE_TTL" ]]; then
  export KONGODB_ARCHIVE_TTL_SECS="$ARCHIVE_TTL"
fi

"$BIN" >"$LOG_FILE" 2>&1 &
PID=$!
cleanup() {
  kill "$PID" >/dev/null 2>&1 || true
}
trap cleanup EXIT

# wait for server
for _ in $(seq 1 40); do
  if curl -sS -o /dev/null "$GATEWAY_URL" -X POST -H 'content-type: application/json' -d '{"db":"smoke","operation":"count","payload":{"collection":"users"}}' 2>/dev/null; then
    break
  fi
  sleep 0.25
done

echo "[4/22] insert"
MISSING_COUNT="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"missing/path","operation":"count","payload":{"collection":"users"}}')"
assert_contains "$MISSING_COUNT" '"status":"error"' "non-insert op must not auto-create db"
CREATE_DB="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"missing/path","operation":"create_db","payload":{}}')"
assert_contains "$CREATE_DB" '"created":true' "create_db operation should create db"
EXISTS_MISSING="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"missing/path","operation":"db_exists","payload":{}}')"
assert_contains "$EXISTS_MISSING" '"exists":true' "db_exists should be true for created db"
LRU_DB1="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"lru/db1","operation":"create_db","payload":{}}')"
assert_contains "$LRU_DB1" '"status":"success"' "lru db1 create should succeed"
LRU_DB2="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"lru/db2","operation":"create_db","payload":{}}')"
assert_contains "$LRU_DB2" '"status":"success"' "lru db2 create should evict older active db if cap reached"
LRU_DB3="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"lru/db3","operation":"create_db","payload":{}}')"
assert_contains "$LRU_DB3" '"status":"success"' "lru db3 create should evict least recently used active db"
LRU_ACTIVE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"operation":"list_dbs","payload":{}}')"
assert_contains "$LRU_ACTIVE" '"count":2' "active db count should stay capped by LRU eviction"

INSERT_RES="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"insert","payload":{"collection":"users","ttl_seconds":2,"data":{"name":"Ada","age":34,"score":10}}}')"
assert_contains "$INSERT_RES" '"status":"success"' "insert should succeed"
echo "$INSERT_RES"

INSERT_TS_BLOCKED="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"insert","payload":{"collection":"users","data":{"name":"TSBlocked","_created_at":"2024-01-01T00:00:00Z"}}}')"
assert_contains "$INSERT_TS_BLOCKED" '"status":"error"' "insert should reject system timestamps by default"
assert_contains "$INSERT_TS_BLOCKED" 'allow_system_timestamps' "insert timestamp rejection should mention allow_system_timestamps"

INSERT_TS_ALLOWED="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"insert","payload":{"collection":"users","allow_system_timestamps":true,"data":{"name":"TSAllowed","_created_at":"2024-01-01T00:00:00Z"}}}')"
assert_contains "$INSERT_TS_ALLOWED" '"status":"success"' "insert should allow system timestamps when enabled"

MACRO_SET="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"set","payload":{"collection":"users","data":{"_id":{"$id_uuidv4":{"prefix":"sessions:"}},"ts":{"$ts_now":true},"sid":{"$id_random":{"len":10}}}}}')"
assert_contains "$MACRO_SET" '"status":"success"' "set with $-macros should succeed"
assert_contains "$MACRO_SET" '"sessions:' "uuid macro should apply prefix"
assert_contains "$MACRO_SET" '"ts":"' "now macro should produce timestamp"

echo "[5/22] count + aggregate"
COUNT_RES="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"count","payload":{"collection":"users","filter":{"age":{"$gte":18}}}}')"
QUERY_RES="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"query","payload":{"collection":"users","filter":{"age":{"$gte":18}},"compute":{"full_name":{"$join":["$name"]}}}}')"

echo "$COUNT_RES"
echo "$QUERY_RES"

PROJ_RES="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"query","payload":{"collection":"users","fields":["name","age"],"exclude_fields":["age","_id"]}}')"
assert_contains "$PROJ_RES" '"name":"Ada"' "query projection should include requested field"
assert_not_contains "$PROJ_RES" '"age":34' "query projection should exclude excluded field"
assert_contains "$PROJ_RES" '"_id":"' "_id must always be present in projection"

SORT_RES="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"insert","payload":{"collection":"sort_users","data":[{"name":"Alice","profile":{"age":30}},{"name":"Bob","profile":{"age":40}}]}}')"
assert_contains "$SORT_RES" '"inserted_count":2' "sort seed insert should succeed"
SORT_Q="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"query","payload":{"collection":"sort_users","sort":{"profile.age":-1}}}')"
if ! grep -Eq '"name":"Bob".*"name":"Alice"' <<<"$SORT_Q"; then
  echo "assertion failed: query sort by dot path should order desc" >&2
  echo "response: $SORT_Q" >&2
  exit 1
fi
SORT_STR_Q="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"query","payload":{"collection":"sort_users","sort":"name"}}')"
if ! grep -Eq '"name":"Alice".*"name":"Bob"' <<<"$SORT_STR_Q"; then
  echo "assertion failed: query sort string without direction should default to asc" >&2
  echo "response: $SORT_STR_Q" >&2
  exit 1
fi

LOOKUP_SEED_BOOKS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"insert","payload":{"collection":"books","data":[{"_id":"b1","name":"Book One","vendor_id":"v1"},{"_id":"b2","name":"Book Two","vendor_id":"v2"}]}}')"
assert_contains "$LOOKUP_SEED_BOOKS" '"inserted_count":2' "lookup books seed should succeed"
LOOKUP_SEED_VENDORS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"insert","payload":{"collection":"vendors_lkp","data":[{"_id":"v1","name":"Vendor One"},{"_id":"v2","name":"Vendor Two"}]}}')"
assert_contains "$LOOKUP_SEED_VENDORS" '"inserted_count":2' "lookup vendors seed should succeed"
LOOKUP_SEED_USERS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"insert","payload":{"collection":"users_lookup","data":{"name":"Reader","favorite_books":["b2","b1"]}}}')"
assert_contains "$LOOKUP_SEED_USERS" '"status":"success"' "lookup user seed should succeed"
LOOKUP_Q="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"query","payload":{"collection":"users_lookup","lookups":{"books":{"from":"books","local_field":"favorite_books[]","foreign_field":"_id","match":"$in","multi":true,"preserve_order":true,"limit":10,"fields":["_id","name","vendor_id"]},"vendors":{"from":"vendors_lkp","local_field":"$lookup.books[].vendor_id","foreign_field":"_id","match":"$in","multi":true,"limit":10}}}}')"
assert_contains "$LOOKUP_Q" '"books":' "lookup should embed books"
assert_contains "$LOOKUP_Q" '"vendors":' "forward reference lookup should embed vendors"

SQL_READ="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"sql_execute","payload":{"sql":"SELECT collection, COUNT(*) AS total FROM __kdb_documents WHERE collection = ? GROUP BY collection","params":["users"]}}')"
assert_contains "$SQL_READ" '"status":"success"' "sql_execute select should succeed"
assert_contains "$SQL_READ" '"collection":"users"' "sql_execute should return selected rows"
assert_contains "$SQL_READ" '"columns":' "sql_execute should include column metadata"
SQL_WRITE_COMMITTED="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"sql_execute","payload":{"sql":"UPDATE __kdb_documents SET _expiry_behavior = ? WHERE collection = ?","params":["delete","users"]}}')"
assert_contains "$SQL_WRITE_COMMITTED" '"status":"success"' "sql_execute update should succeed in phase 2"
assert_contains "$SQL_WRITE_COMMITTED" '"rows_affected":' "sql_execute write should return rows_affected"
assert_contains "$SQL_WRITE_COMMITTED" '"committed":true' "sql_execute committed write should mark committed=true"
assert_contains "$SQL_WRITE_COMMITTED" '"is_async_ack":false' "sql_execute committed write should mark is_async_ack=false"
SQL_WRITE_ACCEPTED="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"sql_execute","payload":{"commit":false,"sql":"UPDATE __kdb_documents SET _expiry_behavior = ? WHERE collection = ?","params":["archive","users"]}}')"
assert_contains "$SQL_WRITE_ACCEPTED" '"status":"success"' "sql_execute accepted write should enqueue"
assert_contains "$SQL_WRITE_ACCEPTED" '"ack_status":"queued"' "sql_execute accepted write should return queued ack"
assert_contains "$SQL_WRITE_ACCEPTED" '"committed":false' "sql_execute accepted write should return committed=false"
assert_contains "$SQL_WRITE_ACCEPTED" '"is_async_ack":true' "sql_execute accepted write should return is_async_ack=true"
SQL_WRITE_VERIFY=""
for _ in $(seq 1 20); do
  SQL_WRITE_VERIFY="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"sql_execute","payload":{"sql":"SELECT COUNT(*) AS total FROM __kdb_documents WHERE collection = ? AND _expiry_behavior = ?","params":["users","archive"]}}')"
  if grep -q '"total":' <<<"$SQL_WRITE_VERIFY"; then
    break
  fi
  sleep 0.25
done
assert_contains "$SQL_WRITE_VERIFY" '"total":' "sql_execute accepted write should eventually apply"
assert_not_contains "$SQL_WRITE_VERIFY" '"total":0' "sql_execute accepted write should update at least one row"

EXPORT_PATH="${SMOKE_ROOT}/data/users_export"
EXPORT_RES="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"smoke\",\"operation\":\"export_jsonl\",\"payload\":{\"collection\":\"users\",\"target_path\":\"$EXPORT_PATH\"}}")"
assert_contains "$EXPORT_RES" '"job_id":"' "export_jsonl should enqueue job"
assert_contains "$EXPORT_RES" '"status":"queued"' "export_jsonl should return queued status"
IMPORT_PATH="${SMOKE_ROOT}/data/users_import.jsonl"
cat > "$IMPORT_PATH" <<'EOF'
{"name":"Imported One","score":11}
{"name":"Imported Two","score":22}
EOF
IMPORT_RES="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"smoke\",\"operation\":\"import_jsonl\",\"payload\":{\"collection\":\"users_import\",\"source_path\":\"$IMPORT_PATH\",\"on_conflict\":\"error\"}}")"
assert_contains "$IMPORT_RES" '"job_id":"' "import_jsonl should enqueue job"
assert_contains "$IMPORT_RES" '"status":"queued"' "import_jsonl should return queued status"

echo "[6/22] TTL move check"
sleep 3
LIVE_AFTER="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"count","payload":{"collection":"users"}}')"
ARCH_AFTER="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"count","payload":{"collection":"users","archive_only":true}}')"

echo "live after ttl:   $LIVE_AFTER"
echo "__kdb_archive after ttl:$ARCH_AFTER"

echo "compression headers:"
curl -sS -D - -o /dev/null -X POST "$GATEWAY_URL" \
  -H 'content-type: application/json' \
  -H 'accept-encoding: gzip, br' \
  -d '{"db":"smoke","operation":"count","payload":{"collection":"users"}}' \
  | grep -Ei 'content-encoding|vary|content-type' || true

echo "[7/22] update data(array<object with _id>)"
INS_BULK="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"insert","payload":{"collection":"users","data":[{"name":"U1","status":"new"},{"name":"U2","status":"new"}]}}')"
assert_contains "$INS_BULK" '"status":"success"' "insert array should succeed"
ID1="$(grep -o '"_id":"[^"]*' <<<"$INS_BULK" | sed -n '1s/"_id":"//p')"
ID2="$(grep -o '"_id":"[^"]*' <<<"$INS_BULK" | sed -n '2s/"_id":"//p')"
UPD_IDS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"smoke\",\"operation\":\"update\",\"payload\":{\"collection\":\"users\",\"data\":[{\"_id\":\"$ID1\",\"status\":\"active\"},{\"_id\":\"$ID2\",\"status\":\"active\"}]}}")"
assert_contains "$UPD_IDS" '"updated_count":2' "update data[] should update 2 rows"

echo "[8/22] update filter + data(object)"
UPD_FILTER="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"update","payload":{"collection":"users","filter":{"status":{"$eq":"active"}},"data":{"tier":"pro"},"max_docs":1}}')"
assert_contains "$UPD_FILTER" '"updated_count":1' "update by filter should respect max_docs"

echo "[9/22] update replace=true single"
UPD_ARRAY="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"smoke\",\"operation\":\"update\",\"payload\":{\"collection\":\"users\",\"replace\":true,\"data\":{\"_id\":\"$ID1\",\"name\":\"Replaced\",\"only\":\"this\"}}}")"
assert_contains "$UPD_ARRAY" '"updated_count":1' "update replace=true should update one row"

echo "[10/22] guardrails (upsert + delete selector conflict)"
UPSERT_BAD_ID="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"upsert","payload":{"collection":"users","filter":{"email":{"$eq":"x@example.com"}},"insert_data":{"_id":"bad","email":"x@example.com"},"update_data":{"name":"X"}}}')"
assert_contains "$UPSERT_BAD_ID" '"status":"error"' "upsert should reject _id in insert_data"
assert_contains "$UPSERT_BAD_ID" 'insert_data cannot contain _id' "upsert _id guardrail message"

UPSERT_EMPTY_FILTER="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"upsert","payload":{"collection":"users","filter":{},"insert_data":{"email":"z@example.com"},"update_data":{"name":"Z"}}}')"
assert_contains "$UPSERT_EMPTY_FILTER" '"status":"error"' "upsert should reject empty filter"

DEL_CONFLICT="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"smoke\",\"operation\":\"delete\",\"payload\":{\"collection\":\"users\",\"id\":\"$ID1\",\"ids\":[\"$ID2\"]}}")"
assert_contains "$DEL_CONFLICT" '"status":"error"' "delete should reject id+ids together"

echo "[11/22] insert composite unique_fields"
INS_ABS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"insert","payload":{"collection":"users","data":{"email":"nobody@example.com","tenant":{"id":"t1"},"name":"Nobody"},"unique_fields":["tenant.id","email"],"on_conflict":"skip"}}')"
assert_contains "$INS_ABS" '"inserted_count":1' "insert should insert first composite unique row"
INS_ABS2="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"insert","payload":{"collection":"users","data":{"email":"nobody@example.com","tenant":{"id":"t1"},"name":"Nobody Again"},"unique_fields":["tenant.id","email"],"on_conflict":"skip"}}')"
assert_contains "$INS_ABS2" '"skipped_count":1' "insert should skip composite unique conflict"

echo "[12/22] get + set"
GET_ONE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"smoke\",\"operation\":\"get\",\"payload\":{\"id\":\"$ID1\"}}")"
assert_contains "$GET_ONE" '"count":1' "get should return one row"
SET_ONE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"smoke\",\"operation\":\"set\",\"payload\":{\"data\":{\"_id\":\"$ID1\",\"flag\":\"ok\"}}}")"
assert_contains "$SET_ONE" '"updated_count":1' "set should update row by id"

echo "[13/22] set_ttl + delete soft-delete"
SET_TTL="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"smoke\",\"operation\":\"set_ttl\",\"payload\":{\"collection\":\"users\",\"ids\":[\"$ID1\"],\"ttl_seconds\":1}}")"
assert_contains "$SET_TTL" '"updated_count":1' "set_ttl should update ttl"
sleep 2
REAP_AFTER_TTL="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"count","payload":{"collection":"users","archive_only":true}}')"
assert_contains "$REAP_AFTER_TTL" '"count":' "archive count after set_ttl should be returned"
ARCH_BULK="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"smoke\",\"operation\":\"delete\",\"payload\":{\"collection\":\"users\",\"ids\":[\"$ID2\"]}}")"
assert_contains "$ARCH_BULK" '"soft_delete":true' "delete should soft-delete one row by default"

echo "[14/22] scope=all count without collection"
SCOPE_ALL="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"count","payload":{"scope":"all"}}')"
assert_contains "$SCOPE_ALL" '"status":"success"' "scope all count should work without collection"

echo "[15/22] get_stats + recompute_stats + list_namespaces"
INS_PROJECTS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"insert","payload":{"collection":"projects","data":[{"name":"P1"},{"name":"P2"}]}}')"
assert_contains "$INS_PROJECTS" '"inserted_count":2' "projects insert should succeed"
STATS_PROJECTS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"get_stats","payload":{"collection":"projects"}}')"
assert_contains "$STATS_PROJECTS" '"live_count":2' "stats should show 2 live docs"
RECOMP_STATS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"recompute_stats","payload":{"collection":"projects"}}')"
assert_contains "$RECOMP_STATS" '"job_type":"recompute_stats"' "recompute_stats should enqueue admin job"
RECOMP_JOB_ID="$(grep -o '"job_id":"[^"]*' <<<"$RECOMP_STATS" | sed -n '1s/"job_id":"//p')"
RECOMP_JOB="$(wait_for_job_completed "smoke" "$RECOMP_JOB_ID" 30)"
assert_contains "$RECOMP_JOB" '"job_type":"recompute_stats"' "recompute_stats job should complete"
LIST_COLLS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"list_namespaces","payload":{}}')"
assert_contains "$LIST_COLLS" '"collection":"projects"' "list_namespaces should include projects"
LIST_TABLES="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"list_tables","payload":{}}')"
assert_contains "$LIST_TABLES" '"status":"success"' "list_tables should succeed"
assert_not_contains "$LIST_TABLES" '__kdb_' "list_tables should exclude internal tables"
SQL_CREATE_TABLE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"sql_execute","payload":{"sql":"CREATE TABLE app_notes (id TEXT PRIMARY KEY, title TEXT)"}}')"
assert_contains "$SQL_CREATE_TABLE" '"status":"success"' "sql_execute create table should succeed"
SQL_CREATE_INDEX="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"sql_execute","payload":{"sql":"CREATE INDEX idx_app_notes_title ON app_notes(title)"}}')"
assert_contains "$SQL_CREATE_INDEX" '"status":"success"' "sql_execute create index should succeed"
SQL_ALTER_TABLE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"sql_execute","payload":{"sql":"ALTER TABLE app_notes ADD COLUMN body TEXT"}}')"
assert_contains "$SQL_ALTER_TABLE" '"status":"success"' "sql_execute alter table add column should succeed"
SQL_DROP_INDEX="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"sql_execute","payload":{"sql":"DROP INDEX idx_app_notes_title"}}')"
assert_contains "$SQL_DROP_INDEX" '"status":"success"' "sql_execute drop index should succeed"
LIST_TABLES_AFTER_DDL="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"list_tables","payload":{}}')"
assert_contains "$LIST_TABLES_AFTER_DDL" '"name":"app_notes"' "list_tables should include user-created tables"
SQL_CREATE_RESERVED="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"sql_execute","payload":{"sql":"CREATE TABLE __kdb_bad (id TEXT)"}}')"
assert_contains "$SQL_CREATE_RESERVED" '"status":"error"' "sql_execute should reject reserved __kdb_ table creation"
MOVE_DOCS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"change_namespace","payload":{"from_namespace":"projects","to_namespace":"projects___kdb_archived","max_docs":1}}')"
assert_contains "$MOVE_DOCS" '"moved_count":1' "change_namespace should move one row"
STATS_SRC_AFTER_MOVE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"get_stats","namespace":"projects","payload":{}}')"
assert_contains "$STATS_SRC_AFTER_MOVE" '"live_count":1' "source namespace stats should decrement after change_namespace"
STATS_DST_AFTER_MOVE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"get_stats","namespace":"projects___kdb_archived","payload":{}}')"
assert_contains "$STATS_DST_AFTER_MOVE" '"live_count":1' "target namespace stats should increment after change_namespace"
VACUUM_RES="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"vacuum_db","payload":{}}')"
assert_contains "$VACUUM_RES" '"job_type":"vacuum_db"' "vacuum should enqueue admin job"
VACUUM_JOB_ID="$(grep -o '"job_id":"[^"]*' <<<"$VACUUM_RES" | sed -n '1s/"job_id":"//p')"
VACUUM_JOB="$(wait_for_job_completed "smoke" "$VACUUM_JOB_ID" 30)"
assert_contains "$VACUUM_JOB" '"job_type":"vacuum_db"' "vacuum job should complete"
REAP_RES="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"reap_db","payload":{}}')"
assert_contains "$REAP_RES" '"reaped":true' "reap_db should succeed"
CLONE_RES="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"clone_db","payload":{"to_db_path":"smoke_clone"}}')"
assert_contains "$CLONE_RES" '"cloned":true' "clone_db should succeed"
CLONE_COUNT="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke_clone","operation":"count","payload":{"scope":"all"}}')"
assert_contains "$CLONE_COUNT" '"status":"success"' "cloned db should be queryable"
BACKUP_RES="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"create_backup","payload":{}}')"
assert_contains "$BACKUP_RES" '"enqueued":true' "create_backup should enqueue async backup job"
assert_contains "$BACKUP_RES" '"backup_id":"' "create_backup should return backup_id"
OFFLOAD_LOCAL="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"offload_db","payload":{}}')"
assert_contains "$OFFLOAD_LOCAL" '"status":"error"' "offload_db should error in local mode"
LOAD_LOCAL="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"load_db","payload":{}}')"
assert_contains "$LOAD_LOCAL" '"status":"error"' "load_db should error in local mode"
SYS_REFRESH="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"operation":"system_refresh_inventory","payload":{}}')"
assert_contains "$SYS_REFRESH" '"refreshed":' "system_refresh_inventory should refresh catalog"
SYS_INV="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"operation":"system_get_inventory","payload":{"limit":50}}')"
assert_contains "$SYS_INV" '"db":"smoke"' "system_get_inventory should include smoke db"
assert_not_contains "$SYS_INV" '__kdb_system' "system inventory should not expose internal catalog db"
SYS_STATUS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"system_get_db_status","payload":{}}')"
assert_contains "$SYS_STATUS" '"db":"smoke"' "system_get_db_status should include db"
SYS_SNAPSHOT="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"system_snapshot_db_stats","payload":{}}')"
assert_contains "$SYS_SNAPSHOT" '"count":1' "system_snapshot_db_stats should snapshot selected db"
SYS_QUERY_STATS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"system_query_db_stats","payload":{"limit":5}}')"
assert_contains "$SYS_QUERY_STATS" '"db":"smoke"' "system_query_db_stats should include smoke stats"
SYS_EVENTS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"operation":"system_list_db_events","payload":{"limit":20}}')"
assert_contains "$SYS_EVENTS" 'system.inventory_refreshed' "system_list_db_events should include refresh event"

echo "[16/22] ack_mode accepted + fallback metadata"
ACK_ACCEPTED="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"insert","namespace":"acktest","payload":{"commit":false,"data":{"name":"queued"}}}')"
assert_contains "$ACK_ACCEPTED" '"ack_mode":"accepted"' "commit=false should use accepted path"
assert_contains "$ACK_ACCEPTED" '"ack_status":"queued"' "accepted path should enqueue"
assert_contains "$ACK_ACCEPTED" '"is_async_ack":true' "accepted path should be marked async"
assert_contains "$ACK_ACCEPTED" '"committed":false' "accepted path should not be committed yet"

echo "[17/22] rename_namespace"
INS_RENAME="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"insert","payload":{"collection":"ns_old","data":[{"r":1},{"r":2}]}}')"
assert_contains "$INS_RENAME" '"inserted_count":2' "rename seed insert should succeed"
REN_NS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"rename_namespace","payload":{"from_namespace":"ns_old","to_namespace":"ns_new"}}')"
assert_contains "$REN_NS" '"renamed":true' "rename_namespace should succeed"
COUNT_NEW="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"count","payload":{"collection":"ns_new"}}')"
assert_contains "$COUNT_NEW" '"count":2' "renamed namespace should keep data"

echo "[18/22] drop_namespace + restore_archive(txn_id)"
ARCH_COL="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"drop_namespace","namespace":"projects","payload":{}}')"
assert_contains "$ARCH_COL" '"deleted_count":1' "drop_namespace should soft-delete remaining projects"
ARCH_TXN="$(grep -o '"_txn_id":"[^"]*' <<<"$ARCH_COL" | sed -n '1s/"_txn_id":"//p')"
RESTORE_TXN="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"smoke\",\"operation\":\"restore_archive\",\"payload\":{\"txn_id\":\"$ARCH_TXN\"}}")"
assert_contains "$RESTORE_TXN" '"restored_count":1' "restore_archive should restore __kdb_archived projects"

echo "[19/22] drop_namespace (__kdb_archive mode) + purge_archive(txn_id)"
DROP_COL="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"drop_namespace","payload":{"collection":"projects","ttl_seconds":30}}')"
assert_contains "$DROP_COL" '"purge":false' "drop_namespace should __kdb_archive when purge=false"
DROP_TXN="$(grep -o '"_txn_id":"[^"]*' <<<"$DROP_COL" | sed -n '1s/"_txn_id":"//p')"
PURGE_TXN="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"smoke\",\"operation\":\"purge_archive\",\"payload\":{\"txn_id\":\"$DROP_TXN\"}}")"
assert_contains "$PURGE_TXN" '"purged_count":1' "purge_archive should hard-delete __kdb_archived rows by txn_id"

echo "[20/22] drop_namespace (purge=true hard-delete)"
INS_TMP="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"insert","payload":{"collection":"tmpdrop","data":[{"a":1},{"a":2}]}}')"
assert_contains "$INS_TMP" '"inserted_count":2' "tmpdrop insert should succeed"
DROP_HARD="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"drop_namespace","payload":{"collection":"tmpdrop","purge":true}}')"
assert_contains "$DROP_HARD" '"purge":true' "drop_namespace purge=true should hard delete"
assert_contains "$DROP_HARD" '"deleted_count":2' "drop_namespace purge=true should delete rows"

echo "[21/22] purge_archive(ids[])"
INS_PURGE_IDS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"insert","payload":{"collection":"purgeids","data":[{"x":1},{"x":2}]}}')"
PID1="$(grep -o '"_id":"[^"]*' <<<"$INS_PURGE_IDS" | sed -n '1s/"_id":"//p')"
PID2="$(grep -o '"_id":"[^"]*' <<<"$INS_PURGE_IDS" | sed -n '2s/"_id":"//p')"
ARCH_PURGE_IDS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"drop_namespace","namespace":"purgeids","payload":{}}')"
assert_contains "$ARCH_PURGE_IDS" '"deleted_count":2' "drop_namespace should soft-delete purgeids docs"
PURGE_IDS="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"smoke\",\"operation\":\"purge_archive\",\"payload\":{\"ids\":[\"$PID1\",\"$PID2\"]}}")"
assert_contains "$PURGE_IDS" '"purged_count":2' "purge_archive ids should delete __kdb_archive docs"

echo "[22/22] restore_archive(collection/filter) conflict-skip"
INS_RESTORE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"insert","payload":{"collection":"restorec","data":[{"z":1},{"z":2}]}}')"
RID1="$(grep -o '"_id":"[^"]*' <<<"$INS_RESTORE" | sed -n '1s/"_id":"//p')"
ARCH_RESTORE="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"drop_namespace","namespace":"restorec","payload":{}}')"
assert_contains "$ARCH_RESTORE" '"deleted_count":2' "drop_namespace should soft-delete restorec docs"
INS_CONFLICT="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d "{\"db\":\"smoke\",\"operation\":\"insert\",\"payload\":{\"collection\":\"restorec\",\"data\":{\"_id\":\"$RID1\",\"z\":100}}}")"
assert_contains "$INS_CONFLICT" '"status":"success"' "conflict seed insert should succeed"
RESTORE_FILTER="$(curl -sS -X POST "$GATEWAY_URL" -H 'content-type: application/json' -d '{"db":"smoke","operation":"restore_archive","payload":{"collection":"restorec"}}')"
assert_contains "$RESTORE_FILTER" '"skipped_conflicts":1' "restore_archive should skip conflicting ids"

echo
echo "manual local call examples:"
echo "curl -sS -X POST ${GATEWAY_URL} -H 'content-type: application/json' -d '{\"db\":\"myapp.something/main\",\"operation\":\"count\",\"payload\":{\"collection\":\"users\"}}'"
echo "curl -sS -X POST ${GATEWAY_URL} -H 'content-type: application/json' -d '{\"db\":\"myapp.something/main\",\"operation\":\"query\",\"payload\":{\"collection\":\"users\",\"filter\":{\"age\":{\"\$gte\":18}},\"compute\":{\"full_name\":{\"\$join\":[\"\$first_name\",\" \",\"\$last_name\"]}}}}'"

echo
echo "done. server log: $LOG_FILE"
