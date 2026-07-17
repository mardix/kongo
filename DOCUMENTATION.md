# Kongo

## Hybrid Database Toolkit

Kongo is a lightweight, self-hosted data platform that combines the flexibility of a document database with the power of SQLite. It provides a consistent JSON API for document storage, direct SQL access, identity records, file metadata, metrics, audit logs, full-text search, and database administration.

Built in Rust on SQLite/libSQL, Kongo is designed for applications that need a capable embedded or standalone data service without operating a large database stack. It runs locally, in Docker, or with S3-backed storage.

### Main Features

- **Hybrid Document and SQL Database**
  Store schemaless JSON documents in namespaces while retaining direct access to SQLite tables and SQL queries.

- **Complete Document Operations**
  Insert, update, delete, query, aggregate, upsert, paginate, project fields, sort, filter, join related documents, and manage TTL-based expiration.

- **SQLite Interface**
  Create and inspect user tables, execute parameterized SQL, browse records, and use supported DDL without exposing Kongo's internal tables.

- **Local and S3 Storage**
  Run entirely from local disk or use S3-compatible object storage with local hydration, WAL replication, snapshots, synchronization, and safe recovery.

- **Write Coordination**
  Per-database write coordinators serialize concurrent mutations. Requests can wait for a committed result or use asynchronous acknowledgment for higher throughput.

- **Backup and Recovery**
  Create manual or scheduled compressed backups, retain versioned snapshots, restore by backup ID, tag, timestamp, or latest version, and apply configurable retention policies.

- **Asynchronous Jobs**
  Long-running imports, exports, backups, maintenance, FTS indexing, and administrative work execute through a unified background job system.

- **JSONL Import and Export**
  Stream large local or S3-hosted JSONL datasets with compression, resumable offsets, conflict handling, field mapping, progress tracking, and worker recovery.

- **Full-Text Search**
  Search document content through SQLite FTS5 with namespace filters, pagination, projections, sorting, and background indexing.

- **Automatic Indexing**
  Query heatmaps identify frequently filtered or sorted JSON paths and create bounded expression indexes automatically. Indexes can also be managed manually.

- **Document Lookups**
  Resolve one-to-one and one-to-many relationships across namespaces using nested, dependency-aware lookups with concurrent DAG execution.

- **Metrics Event Store**
  Ingest application events and query counts, sums, averages, minimums, maximums, distinct values, time buckets, and grouped dimensions.

- **Identity Store**
  Manage users, profile information, statuses, external authentication providers, tokens, password-change requirements, bans, and lifecycle events without imposing an authentication protocol.

- **File Catalog**
  Track file identity, ownership, storage location, content type, hashes, upload timestamps, metadata, expiration, and deletion state for files stored by the application.

- **Audit Logs**
  Record and query structured audit events by actor, action, target, status, source, request ID, and timestamp.

- **Lifecycle Management**
  Support soft deletion, archival, hard purge, restoration, namespace changes, TTL expiration, database reaping, vacuuming, and statistics recomputation.

- **System Catalog and Monitoring**
  Maintain cross-database inventory, lifecycle events, historical database statistics, active connection state, process memory, request counts, latency, and rolling instance metrics.

- **Built-In Admin Interface**
  Manage multiple Kongo connections and databases through a React interface for DocumentDB, SQLiteDB, Identity, Files, Metrics, Search, Audit Logs, jobs, backups, and system monitoring.

- **Simple Security Model**
  Protect API, documentation, and administration routes with an access key, while supporting an explicit no-auth mode for trusted local development.

### Deployment Model

Kongo can operate as:

- An embedded local database service
- A self-hosted Docker application with persistent volumes
- A serverless container with S3-backed durable storage
- A lightweight database gateway for SaaS applications
- A development and administration layer over SQLite data

Kongo's goal is to provide one compact service for common application data needs while preserving SQLite's portability, reliability, and direct SQL capabilities.

## API Surface
This section describes the public HTTP routes and the access rules around them.

### Endpoints
These are the HTTP endpoints exposed by Kongodb under the configured base path.
- `POST ${KONGODB_BASE_PATH}/gateway` (default `/_/kdb/gateway`): all operations.
- `GET ${KONGODB_BASE_PATH}/ping`: service health + version.
- `GET ${KONGODB_BASE_PATH}/meta/operations`: machine-readable operation catalog.
- `GET ${KONGODB_BASE_PATH}/doc`: rendered markdown docs HTML.
- `GET ${KONGODB_BASE_PATH}/admin/`: built-in Admin UI SPA when enabled.

### Auth
These settings define how requests are authenticated and which endpoints stay open.
- Header: `X-Access-Key: <key>`.
- Browser access to `/doc` and `/admin/`: HTTP Basic username `kongodb`, password set to `KONGODB_ACCESS_KEY`.
- Use HTTPS for browser access outside localhost; HTTP Basic credentials are only transport-safe when TLS protects the connection.
- `KONGODB_AUTH_MODE=access_key` requires a non-empty `KONGODB_ACCESS_KEY`.
- `KONGODB_AUTH_MODE=none` explicitly disables authentication for trusted local development.
- `/meta/operations` follows the same access-key policy as `/gateway`.
- `/ping` remains open. `/doc` and `/admin/` share the browser access-key gate unless `KONGODB_AUTH_MODE=none`.

## Request/Response Contract
This section defines the canonical request envelope and the common success/error response shapes.

### Request
Use this envelope for all RPC calls sent to the gateway.

```json
{
  "db": "myapp.something/main",
  "operation": "query",
  "namespace": "users",
  "namespaces": ["users", "admins"],
  "payload": {}
}
```

### Rules
These rules explain how request fields are normalized and validated before dispatch.
- `db` is required for all operations except global db-list operations.
- Canonical request shape is explicit: `db` + `operation` + `namespace|namespaces` + `payload`.
- `namespace` is top-level canonical selector.
- `namespaces` (top-level) selects multiple namespaces.
- `collection` is an alias for `namespace` (top-level alias, and `payload.collection` accepted).
- If both `namespace` and `payload.collection` are provided, they must match.
- `namespace` and `namespaces` cannot both be provided.
- Optional shorthand alias is supported:
  - `operation: "query::users"` => `operation="query"`, `namespace="users"`
  - `operation: "query::*"` => `operation="query"`, `namespace="*"`
  - `operation: "query::users,admins,teams"` => `operation="query"`, `namespaces=["users","admins","teams"]`
  - shorthand cannot be combined with top-level `namespace` or `namespaces`
- Namespace policy:
  - Required: `insert`, `query`, `search`.
  - For `insert` and upsert-insert paths, namespace must be a single concrete value (no `namespace="*"` and no `namespaces:[...]`).
  - ID-targeted ops allow namespace optional (`get`, `set`, `update`, `delete`); if provided, it is strict.
- Filter/wide destructive ops require namespace unless explicit `scope=all` where supported (`update` filter mode, `delete` filter mode, `set_ttl` filter mode, namespace-level ops).
- Namespace wildcard alias:
  - `namespace: "*"` maps to `payload.scope: "all"` for operations that support `scope=all`.
  - It conflicts with `payload.scope: "collection"`.
- `_namespace` response field:
  - hidden by default
  - enabled globally with `KONGODB_RESPONSE_INCLUDE_NAMESPACE=true`
  - per-request override with `payload.include_namespace` (alias `payload.include_name`)
  - always auto-included for `get/query/search` when using `namespace="*"` or `namespaces:[...]`
- DB creation is only allowed by:
  - `create_db`
  - `insert`
  - `import_jsonl`

### Success Response
Successful operations return this envelope, with operation-specific fields nested under `data`.

```json
{
  "status": "success",
  "data": {},
  "_txn_id": "optional",
  "message": "optional",
  "committed": true,
  "is_async_ack": false,
  "ack_mode": "optional (accepted path)",
  "ack_status": "optional (accepted path)"
}
```

### Error Response
Failed operations return this error envelope with a single human-readable error string.

```json
{
  "status": "error",
  "error": "reason"
}
```

## Payload Properties (Grouped)
This section summarizes reusable payload fields shared across multiple operations.

## Payload Shape
Use this table as the quick reference for common payload keys and their meanings.

| Field | Type | Description |
|---|---|---|
| `collection` | string | Alias of top-level `namespace` |
| `namespaces` | string[] | Multi-namespace selector (top-level alias via request normalization) |
| `search` | string | Search query for `search` (alias: `q`) |
| `from_namespace` | string | Source namespace for `change_namespace` and `rename_namespace` |
| `to_namespace` | string | Target namespace for `change_namespace` |
| `to_db_path` | string | Target db for `clone_db` |
| `backup_db_path` | string | Backup/restore path |
| `backup_id` | string | Backup selector id |
| `backup_at` | string | Backup selector timestamp (RFC3339 UTC) |
| `backup_tag` | string | Backup selector tag |
| `latest` | bool | Restore selector: latest backup |
| `source_path` | string | Source path for `import_jsonl` |
| `source_hash` | string | Optional source hash/fingerprint for import dedupe/validation |
| `target_path` | string | Target path/prefix for `export_jsonl` |
| `compress` | bool | Export compression toggle (default `true`) |
| `alias_import_pk` | string\\|object\\|array | Import-time pk alias mapping |
| `drop_keys` | string[] | Import-time field paths to remove |
| `job_id` | string | Job selector for job operations |
| `job_type` | string | Optional job type filter/hint |
| `status` | string | Optional status filter for `list_jobs` |
| `on_conflict` | string | Conflict policy (op-specific) |
| `commit` | bool | Per-request write ack override: `true` committed, `false` accepted |
| `force_db` | bool | For `get` by explicit id(s), bypass pending accepted-write overlay and read only durable DB state |
| `alias` | string | Metric Events result alias; defaults to `default` for single metrics query |
| `label` | string | Metric Events result label; supports templates like `{{start YYYY-MM-DD}}` |
| `event` | string | Metric Events event selector or tracked event name |
| `events` | array | Metric Events track events array, or metrics query event-name array |
| `action` | string | Audit event action or exact action filter, e.g. `user.login` |
| `actor_type` | string | Audit actor category, e.g. `user`, `service`, or `admin` |
| `actor_id` | string | Audit actor identifier |
| `target_type` | string | Audit target/resource category, e.g. `document`, `file`, or `user` |
| `target_id` | string | Audit target/resource identifier |
| `source` | string | Audit event source, e.g. `api`, `admin-ui`, or `worker` |
| `request_id` | string | Audit correlation/request identifier |
| `ip_address` | string | Audit source IP address supplied by the application |
| `message` | string | Optional human-readable audit context |
| `_user_id` | string | Document-table user reference column. For writes it is stored outside `data`; for reads it can scope results |
| `attach_users` | bool | For `get`, `query`, and `search`, side-load Identity users referenced by returned `_user_id` values |
| `attach_user_fields` | string[] | Fields to return for attached users. Defaults to `id`, `first_name`, `last_name`, `profile_photo`; supports nested `data.*` paths |
| `user_id` | string | Identity user id selector or caller-provided user id for `user_create` |
| `email` | string | Identity user email or provider email |
| `username` | string | Identity username |
| `phone` | string | Identity user phone |
| `first_name` | string | Identity user first/given name |
| `last_name` | string | Identity user last/family name |
| `profile_photo` | string | Identity user profile image URL, file id, or storage reference |
| `provider` | string | Identity provider name, e.g. `google`, `github`, `password`, `custom` |
| `provider_user_id` | string | Stable external provider user id |
| `password_hash` | string | App-generated password hash. Kongodb never stores raw passwords |
| `password_algo` | string | Password hash algorithm label, e.g. `argon2id` |
| `requires_password_change` | bool | Identity account signal for the application to require a password change. Defaults to `false` on create |
| `token_hash` | string | App-generated token hash for reset/magic/API/session references |
| `kind` | string | Identity token kind, e.g. `password_reset`, `email_verify`, `api_key` |
| `allow_multi` | bool | Identity token option. Default `false` revokes active same-kind tokens |
| `expires_at` | string | Identity token expiration datetime, RFC3339 UTC-compatible |
| `expires_in` | int | Identity token expiration offset in seconds |
| `status_reason` | string | Identity status reason |
| `status_expires_at` | string | Scheduled identity status transition datetime, RFC3339 UTC-compatible |
| `status_expires_in` | int | Scheduled identity status transition offset in seconds |
| `status_next` | string | Identity status to apply when status expiration is reached |
| `status_next_reason` | string | Reason stored when scheduled status transition applies |
| `changed_by` | string | Optional actor/service marker for identity status changes |
| `bucket` | string | File catalog bucket/group name; defaults to `default` |
| `storage_backend` | string | File catalog backend marker, e.g. `local`, `s3`, `external` |
| `storage_path` | string | File/object location managed by the application |
| `filename` | string | Original/display filename for file metadata |
| `content_type` | string | MIME type for file metadata |
| `size_bytes` | int | File size in bytes, provided by the application |
| `sha256` | string | Optional content checksum/fingerprint |
| `owner_type` | string | File owner entity type, e.g. `user`, `invoice`, `project` |
| `owner_id` | string | File owner entity id |
| `metadata` | object | File catalog app-specific metadata |
| `uploaded_at` | string | When the app/object store received the file; defaults to server UTC now |
| `start` | string | Metric Events query start (`RFC3339` or `YYYY-MM-DD`) |
| `end` | string | Metric Events query end (`RFC3339` or `YYYY-MM-DD`) |
| `range` | string | Metric Events relative window, e.g. `24h`, `7d` |
| `interval` | string | Metric Events time bucket: `minute`, `hour`, `day`, `week`, `month`, `year` |
| `bucket_label` | string | Metric Events item bucket label template, e.g. `{{bucket HH:mm}}` |
| `metrics` | array | Metric Events metric definitions |
| `batch` | array | Multiple metric events queries in one `metrics_query` request |
| `unique_fields` | string[] | Insert-family soft uniqueness paths (dot notation) |
| `ignore_input_id` | bool | Import: ignore `_id`/`id`/`_key` from input |
| `resumable` | bool | Import job resumable flag |
| `batch_size` | int | Import batch size |
| `enable` | bool | FTS flag for `enable_fts_index` |
| `retain_segments` | int | WAL compaction retain count |
| `index_name` | string | Index name |
| `index_path` | string | JSON path used for index operations |
| `sql` | string | Direct SQL statement for `sql_execute` |
| `params` | array | Positional bind parameters for `sql_execute` |
| `id` | string | Single document id selector |
| `ids` | string[] | Multi-id selector |
| `data` | object\\|array | Main operation data payload |
| `update_data` | object | Upsert update payload |
| `insert_data` | object | Upsert/insert-if-absent insert payload |
| `expiry_behavior` | string | TTL behavior: `archive` or `delete` |
| `filter` | object | JQL filter |
| `txn_id` | string | Archive transaction selector (mapped to `_txn_id`) |
| `snapshot_id` | string | Snapshot selector for `restore_snapshot` |
| `purge` | bool | Hard-delete flag for delete/drop operations |
| `ttl_seconds` | int | TTL seconds |
| `allow_system_timestamps` | bool | Allow input `_created_at`/`_modified_at` where supported |
| `include_system_timestamps` | bool | Export toggle for system timestamps |
| `include_namespace` | bool | Include `_namespace` in response items for get/query/search (alias: `include_name`) |
| `compute` | object | Compute spec (`aggregate`/`query`) |
| `group_by` | string\\|array | Metric Events grouping fields; aggregate grouping is reserved |
| `lookups` | object | Lookup/join map |
| `lookup_depth_override` | int | Per-request lookup depth override |
| `sort` | object\\|string | Sort definition |
| `fields` | string[] | Include projection paths |
| `exclude_fields` | string[] | Exclude projection paths (`_id` always kept) |
| `limit` | int | Page size |
| `offset` | int | Page offset |
| `page` | int | Page number alias (used when `limit/offset` are not provided) |
| `per_page` | int | Page size alias (used when `limit/offset` are not provided) |
| `max_docs` | int | Write cap: `-1` all, `0` no-op, `1+` cap |
| `dry_run` | bool | Simulation mode (no write) |
| `scope` | string | `collection` (default) or `all` |
| `include_archive` | bool | Read source: include archive |
| `archive_only` | bool | Read source: archive only |
| `explain` | bool | Query explain/debug mode |
| `cache` | bool\\|int | Read cache policy (`false/0`, `true/1`, `N>1`, `-1`) |

## Operations Cheatsheet
This cheatsheet groups the public commands by task so the operation surface is easier to scan.

### Data CRUD
These are the primary document-oriented operations used by application code.
#### Create / Update / Read

| Operation | Required Field | Description |
|---|---|---|
| `insert` | `namespace`, `payload.data(object\|array<object>)` | Insert one or many documents. Supports soft uniqueness via `unique_fields` + `on_conflict`. |
| `update` | `payload.data(object with _id)` OR `payload.filter + payload.data(object)` OR `payload.data(array<object with _id>)` | Patch one document, patch many by filter, or patch many explicit ids. `replace=true` only for single-object mode. |
| `set` | `payload.data(object with _id)` | Set one document; with `namespace` => upsert, without => update-only. |
| `upsert` | `payload.filter`, `payload.insert_data` | Update matching docs, or insert when no match. |
| `get` | `payload.id` OR `payload.ids` OR `payload.data._id` | Get one/many by id(s), optional strict namespace. |
| `count` | none | Count matched docs with optional filter/scope/archive flags. |
| `query` | `namespace` OR `namespaces` OR `namespace="*"` | List docs with filter/sort/page/projection/lookups/per-row compute. |
| `aggregate` | `payload.compute` | Set-level compute over matched rows. |
| `search` | (`namespace` OR `namespaces` OR `namespace="*"`) + `payload.search` | FTS search over live docs. |
| `metrics_ingest` | `payload.events(array<object>)` | Append metric events; defaults to accepted/queued ack unless `commit:true`. |
| `metrics_query` | `payload.event|events`, `payload.range|start+end`, `payload.metrics` | Aggregate metric events into labeled result sets. |
| `metrics_catalog` | none | List discovered metric event names and dimension paths. |
| `audit_ingest` | `payload.events(array<object>)` | Append one or more immutable application audit events. |
| `audit_query` | none | Query audit events newest first with actor, target, status, time, search, and pagination filters. |
| `user_create` | none | Create identity user metadata. Stores login info only; Kongodb does not authenticate. |
| `user_get` | `payload.user_id|id|email|username` OR `payload.provider+provider_user_id` | Fetch one identity user. |
| `user_list` | none | List/query identity users with pagination. |
| `user_get_details` | `payload.user_id|id|email|username` | Fetch one identity user with providers and recent events. |
| `user_update` | `payload.user_id|id` | Update identity profile metadata and app data. |
| `user_update_status` | `payload.user_id|id`, `payload.status` | Update app-defined user status and optionally schedule a future status transition. |
| `user_delete` | `payload.user_id|id` | Soft delete and revoke active tokens; `purge=true` hard-deletes user, providers, tokens, and events. |
| `user_create_token` | `payload.user_id|id`, `payload.kind`, `payload.token_hash` | Store app-generated token hashes; `allow_multi=false` revokes active same-kind tokens first. |
| `user_link_provider` | `payload.user_id|id`, `payload.provider`, `payload.provider_user_id` | Link Google/GitHub/custom provider identity to a user. |
| `user_unlink_provider` | `payload.provider`, `payload.provider_user_id` | Unlink a provider identity; optional `user_id` makes it strict. |
| `file_create` | `payload.storage_backend`, `payload.storage_path` | Register file/object metadata only. Kongodb does not move bytes. |
| `file_get` | `payload.id` | Fetch one file metadata record. |
| `file_list` | none | List/query file metadata with pagination and owner/bucket filters. |
| `file_update` | `payload.id` | Update mutable file metadata only. |
| `file_delete` | `payload.id` | Soft-delete metadata by default; `purge=true` hard-deletes the metadata row. |

#### Delete / Archive / TTL / Bulk Data Transfer

| Operation | Required Field | Description |
|---|---|---|
| `delete` | exactly one of `payload.id` OR `payload.ids` OR `payload.filter` | Soft delete by default: move to archive then remove from live. `purge=true` hard deletes. |
| `set_ttl` | selector (`ids` OR `filter`) + `payload.ttl_seconds` | Set/reset TTL and optional `expiry_behavior`. |
| `import_jsonl` | `namespace`, `payload.source_path` | Enqueue async JSONL import for large ingests. |
| `export_jsonl` | none | Enqueue async JSONL export for matched data. |

### Namespace Lifecycle
These operations act on namespaces as units instead of single documents.
#### Namespace Management

| Operation | Required Field | Description |
|---|---|---|
| `list_namespaces` | none | List namespaces with stats. |
| `get_stats` | `namespace` | Read live/archive stats for one namespace. |
| `recompute_stats` | none | Recompute `__kdb_system_stats` for all namespaces. |
| `drop_namespace` | `namespace` | Archive+delete namespace, or hard-delete with `purge=true`. |
| `restore_archive` | `payload.txn_id` OR `payload.ids` OR (`namespace` + `payload.filter`) | Restore from archive with conflict policy. |
| `purge_archive` | `payload.txn_id` OR `payload.ids` OR (`namespace` + `payload.filter`) | Hard-delete from archive only. |
| `change_namespace` | `payload.from_namespace`, `payload.to_namespace` | Move docs from one namespace to another. |
| `rename_namespace` | `payload.from_namespace`, `payload.to_namespace` | Rename a namespace across live and archive data. |

### Database Operations
These operations manage the database itself, including sync, backup, restore, and verification.
#### Database Lifecycle / Backup / Replication

| Operation | Required Field | Description |
|---|---|---|
| `create_db` | none | Create/init current db path. |
| `db_exists` | none | Check db existence (remote-aware in s3 mode). |
| `load_db` | none | s3 mode: preload db in-memory/local cache. |
| `offload_db` | none | s3 mode: flush/sync then unload local copy/connection. |
| `sync_db` | none | Force snapshot+manifest sync. |
| `create_snapshot` | none | Alias of `sync_db`. |
| `list_snapshots` | none | List available snapshots for current db. |
| `restore_snapshot` | none (`payload.snapshot_id` optional) | Restore local db from snapshot. |
| `get_sync_status` | none | Show local/remote sync status. |
| `verify_db` | none | Verify manifest/snapshot/segment objects. |
| `compact_wal` | none (`payload.retain_segments` optional) | Compact manifest WAL segment list. |
| `clone_db` | `payload.to_db_path` | Clone current db to target db path. |
| `create_backup` | none | Enqueue backup job for current db. |
| `restore_backup` | one of: `backup_db_path|backup_id|backup_tag|backup_at|latest=true` | Restore db from selected backup. |
| `list_backups` | none | List backup catalog entries. |
| `tag_backup` | `payload.backup_id` OR `payload.backup_db_path` | Set/clear backup tag for backup entry. |
| `vacuum_db` | none | Run SQLite `VACUUM`. |
| `reap_db` | none | Run TTL reaper immediately. |

### Jobs
These operations inspect or control asynchronous background work.
#### Background Execution

| Operation | Required Field | Description |
|---|---|---|
| `get_job` | `payload.job_id` | Get one job status/details. |
| `list_jobs` | none | List jobs with optional filters. |
| `continue_job` | `payload.job_id` | Resume/retry a resumable/failed job. |
| `abort_job` | `payload.job_id` | Abort/cancel a running/queued job. |
| `transaction` | top-level `data(array<operation>)` | Atomic operation array (currently supports nested insert/update/delete). |

### SQL Operations
These operations expose the direct SQL escape hatch and user-table discovery.
#### Direct SQL / User Tables

| Operation | Required Field | Description |
|---|---|---|
| `sql_execute` | `payload.sql` | Direct SQL execution (`SELECT`/`WITH`/`EXPLAIN`/`INSERT`/`UPDATE`/`DELETE`/`REPLACE`), plus limited DDL (`CREATE TABLE`, `CREATE INDEX`, `DROP INDEX`, `ALTER TABLE ... ADD COLUMN`) for non-`__kdb_*` objects. |
| `list_tables` | none | List user-created SQL tables for the current db. Excludes `__kdb_*` and `sqlite_*`. |
| `get_table_schema` | `payload.table` | Return schema columns for one user-created SQL table. Safely wraps `PRAGMA table_info` without enabling arbitrary `PRAGMA` in `sql_execute`. |

### Admin / System
These operations expose instance-level inventory, system config, and index/FTS controls.
#### Inventory / Config / Indexing

| Operation | Required Field | Description |
|---|---|---|
| `list_commands` | none (global op) | List all supported gateway commands. |
| `list_dbs` | none (global op) | List loaded/open dbs in this instance. |
| `list_all_dbs` | none (global op) | List all known dbs (local + remote manifests in s3 mode). |
| `system_get_inventory` | none (global op) | List DB inventory from the internal system catalog. |
| `system_refresh_inventory` | none (global op) | Scan local/S3 DBs and refresh catalog inventory. |
| `system_get_db_status` | top-level `db` | Return live status for one DB and its catalog row when enabled. |
| `system_snapshot_db_stats` | none, or top-level `db` | Persist system catalog DB stats snapshots for active DBs or one DB. |
| `system_query_db_stats` | none | Query historical DB stats snapshots from the system catalog. |
| `system_list_db_events` | none | List DB lifecycle/error events from the system catalog. |
| `get_system_stats` | none (global op) | Show instance-local uptime, request counters, rolling windows, process memory, active DBs, and write queues. |
| `system_memory` | none (global op) | Compatibility view for process memory and per-db write queue usage; includes `system_stats`. |
| `cleanup_temp_artifacts` | none (global op) | Remove stale temp artifacts under the data dir. |
| `get_system_config` | none | Read current db internal system config values. |
| `get_db_stats` | none | Return current in-memory request counters for the current db. |
| `snapshot_db_stats` | none | Persist one current-db counter snapshot into `__kdb_db_stats_rollups`. |
| `query_db_stats` | none | Query persisted db stats snapshots with optional `start`, `end`, and `limit`. |
| `create_index` | `payload.index_path` | Create manual JSON expression index. |
| `drop_index` | `payload.index_name` OR `payload.index_path` | Drop index by name or path. |
| `list_indexes` | none | List indexes on the internal documents table. |
| `enable_fts_index` | none (`payload.enable` optional) | Toggle db-level FTS access flag. |
| `reindex_fts` | none | Enqueue FTS rebuild/backfill job. |
| `drop_fts_index` | none | Enqueue FTS drop/de-index job. |

## Operators Cheatsheet
This section summarizes the supported filter, compute, and mutation operator families.

### JQL Filter Operators
Use these operators inside `payload.filter` to match documents.

| Operation | Required Field | Description |
|---|---|---|
| `Logical` | filter object | `$and`, `$or`, `$nor`, `$not` |
| `Comparison` | field path + value | `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte`, `$between`, `$exists` |
| `Set/Array` | array/scalar depending on op | `$in`, `$nin`, `$includes`, `$nincludes`, `$all`, `$any`, `$none`, `$elemMatch`, `$size` |
| `String` | string value | `$startsWith`, `$endsWith`, `$contains`, `$ilike`, `$istartsWith`, `$iendsWith`, `$icontains`, `$regex` |
| `Type` | type token | `$type` |

### Compute Operators (`payload.compute`)
Use these operators to compute derived values in `aggregate` or per-row in `query`.

| Operation | Required Field | Description |
|---|---|---|
| `Aggregate compute` | `payload.compute` | `$count`, `$sum`, `$avg`, `$min`, `$max`, `$distinct` |
| `Query row compute` | `payload.compute` | `$count`, `$sum`, `$avg`, `$min`, `$max`, `$distinct`, `$size`, `$join` |
| `Metric options` | per metric object | `$distinct`, `$filter` |

### Write Value Operators (`data`/`insert_data`/`update_data`)
Use these operators to generate or mutate values during writes.

| Operation | Required Field | Description |
|---|---|---|
| `Generators` | operator object | `$ts_now`, `$ts_now_ms`, `$id_uuidv4`, `$id_uuidv7`, `$id_random`, `$hash_value` |
| `Mutations` | operator object | `$unset`, `$inc`, `$push`, `$pop`, `$extend`, `$pull`, `$addset` |

## JQL Cheatsheet
These examples show the most common filtering patterns used with read operations.

| Operation | Required Field | Description |
|---|---|---|
| `Equality + range` | field paths + scalar/range values | `{ \"status\": {\"$eq\":\"active\"}, \"age\": {\"$gte\":18, \"$lte\":65} }` |
| `Boolean logic` | `$and/$or` arrays | Combine filters with nested logical groups |
| `Array matching` | array fields | Use `$all`, `$any`, `$none`, `$elemMatch` for collection semantics |
| `String matching` | string fields | Use `$icontains`, `$startsWith`, `$regex`, etc. |
| `Nested path + exists/type` | dot paths | Example: `profile.age`, `profile.phone`, `profile.meta` with `$between/$exists/$type` |

## Datetime Format
These rules define the accepted timestamp format for system-managed date fields.

- Kongodb system timestamps are UTC.
- Accepted datetime input format for system timestamp fields is RFC3339/ISO-8601 with timezone.
- Examples:
  - `2025-12-24T23:39:26Z`
  - `2025-12-24T23:39:26.873397+00:00`
- If `_created_at` is provided and `_modified_at` is omitted (where allowed), `_modified_at` is set to `_created_at`.

### Identity & Scope
These fields select records and control how widely an operation can scan.
- `id: string`
- `ids: string[]`
- `scope: "collection"|"all"` (default `collection`)
- `collection: string` (alias of top-level `namespace`)

### Data Write Payloads
These fields carry document bodies and write-related execution controls.
- `data: object|array`
- `insert_data: object`
- `update_data: object`
- `max_docs: -1|0|1+`
- `dry_run: bool`

### Conflict/Uniqueness
These fields control insert conflicts, import merge behavior, and restore conflict handling.
- `on_conflict`
  - `insert`: `skip|error`
  - `import_jsonl`: `error|skip|replace|merge`
  - `restore_archive`: `skip|replace|patch`
- `unique_fields: string[]` (insert family soft uniqueness, dot paths supported)

### TTL/Archive/Purge
These fields control expiry behavior, archive retention, and hard-delete semantics.
- `ttl_seconds: int`
- `expiry_behavior: "archive"|"delete"`
- `purge: bool`
- `txn_id: string` (maps to archive `_txn_id`)

### Query/Read Controls
These fields shape reads with filtering, projection, archive scope, and caching.
- `filter: object` (JQL)
- `sort: object|string`
- `limit: int`
- `offset: int`
- `fields: string[]`
- `exclude_fields: string[]`
- `include_archive: bool`
- `archive_only: bool`
- `explain: bool`
- `cache: bool|int`

### Compute/Aggregate
These fields define derived metrics for aggregate and query responses.
- `compute: object`
- `group_by: string[]` (reserved; currently not implemented)

### Lookup/Join
These fields configure lookup expansion during query and search execution.
- `lookups: object`
- `lookup_depth_override: int`

### FTS/Search
These fields drive full-text search behavior.
- `search: string` (alias: `q`)

### DB Admin Paths
These fields are used by namespace-changing and database-management operations.
- `from_namespace: string`
- `to_namespace: string`
- `to_db_path: string`
- `backup_db_path: string`
- `snapshot_id: string`
- `retain_segments: int`

### Backup Restore Selectors
These fields select a specific backup artifact to restore from.
- `backup_id: string`
- `backup_tag: string`
- `backup_at: RFC3339 UTC string`
- `latest: bool`

### Indexing
These fields target manual index operations and FTS toggles.
- `index_name: string`
- `index_path: string`
- `enable: bool` (for `enable_fts_index`)

### Async Jobs
These fields address and filter background jobs.
- `job_id: string`
- `job_type: string`
- `status: string`
- `resumable: bool`

### Import/Export
These fields configure JSONL import/export jobs and their data transformation options.
- `source_path: string`
- `source_hash: string`
- `target_path: string`
- `compress: bool`
- `batch_size: int`
- `ignore_input_id: bool`
- `allow_system_timestamps: bool`
- `include_system_timestamps: bool` (export)
- `alias_import_pk: string|object|array`
- `drop_keys: string[]`

## Operation Reference

Top-level fields shown in each operation are the required/optional `payload` fields for that operation.
`namespace` means top-level `namespace` (or alias `collection`).

---

## 1) Data CRUD
This section documents the main document-oriented operations used by application code.

### `insert`
Insert one or many documents.
- Required payload:
  - `data` (object or array\<object>)
- Optional payload:
  - `_user_id` to store a document-table user reference outside `data`
  - `ttl_seconds`, `expiry_behavior`, `allow_system_timestamps`
  - `unique_fields`, `on_conflict(skip|error)`
  - `dry_run`
- Notes:
  - `_id` respected if provided; otherwise generated (dashless UUIDv4).
  - `_user_id` may be supplied at `payload._user_id` or inside each data object as `data._user_id`; it is stored in the document column and removed from the JSON body.
  - Soft uniqueness is namespace-scoped.
  - `unique_fields` supports one or many dot-paths and is treated as one composite uniqueness key.
  - Use `unique_fields` when you want insert-time duplicate protection without turning the whole write into an `upsert`. This is useful for idempotent create flows, imports, and natural-key checks such as email, tenant+email, or nested profile keys.

Example: insert one document
```json
{
  "db": "myapp/main",
  "operation": "insert",
  "namespace": "users",
  "payload": {
    "data": { "email": "a@b.com", "name": "Ada" }
  }
}
```

Example: insert a document linked to an Identity user
```json
{
  "db": "myapp/main",
  "operation": "insert",
  "namespace": "orders",
  "payload": {
    "_user_id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001",
    "data": {
      "total": 99.5,
      "status": "paid"
    }
  }
}
```

Example: insert many documents
```json
{
  "db": "myapp/main",
  "operation": "insert",
  "namespace": "users",
  "payload": {
    "data": [
      { "email": "a@b.com", "name": "Ada" },
      { "email": "b@b.com", "name": "Bob" }
    ]
  }
}
```

Example: insert with single-field uniqueness
```json
{
  "db": "myapp/main",
  "operation": "insert",
  "namespace": "users",
  "payload": {
    "data": { "email": "a@b.com", "name": "Ada" },
    "unique_fields": ["email"],
    "on_conflict": "skip"
  }
}
```

Example: insert with composite uniqueness
```json
{
  "db": "myapp/main",
  "operation": "insert",
  "namespace": "users",
  "payload": {
    "data": {
      "tenant_id": "tenant_01",
      "email": "a@b.com",
      "name": "Ada"
    },
    "unique_fields": ["tenant_id", "email"],
    "on_conflict": "error"
  }
}
```

Example: insert with composite dot-path uniqueness
```json
{
  "db": "myapp/main",
  "operation": "insert",
  "namespace": "users",
  "payload": {
    "data": {
      "profile": {
        "account_id": "acct_01",
        "email": "a@b.com"
      },
      "name": "Ada"
    },
    "unique_fields": ["profile.account_id", "profile.email"],
    "on_conflict": "skip"
  }
}
```

### `update`
Update one or many documents.
- Required payload:
  - one of:
    - `data` object containing `_id`
    - `filter` + `data(object)`
    - `data(array<object with _id>)`
- Optional payload:
  - `replace`, `max_docs`, `dry_run`
- Scope:
  - single-object or array mode: `namespace` optional; if provided it is strict
  - filter mode: `namespace` required unless `scope=all`
- Notes:
  - `replace=true` is only allowed for single-object mode and performs full replace except `_id`.

Example: update one document by `_id`
```json
{
  "db": "myapp/main",
  "operation": "update",
  "namespace": "users",
  "payload": { "data": { "_id": "u1", "name": "Ada L" } }
}
```

Example: update many documents by filter
```json
{
  "db": "myapp/main",
  "operation": "update",
  "namespace": "users",
  "payload": {
    "filter": { "plan": { "$eq": "trial" } },
    "data": { "plan": "pro" },
    "max_docs": 100
  }
}
```

Example: update many explicit documents by array
```json
{
  "db": "myapp/main",
  "operation": "update",
  "namespace": "users",
  "payload": {
    "data": [
      { "_id": "u1", "name": "Ada L" },
      { "_id": "u2", "name": "Bob M" }
    ]
  }
}
```

Example: replace one document with `replace=true`
```json
{
  "db": "myapp/main",
  "operation": "update",
  "namespace": "users",
  "payload": {
    "replace": true,
    "data": { "_id": "u1", "name": "Ada", "plan": "pro" }
  }
}
```

### `set`
Single `_id` set behavior.
- Required payload:
  - `data(object with _id)`
- Optional payload:
  - `ttl_seconds`, `expiry_behavior`, `dry_run`
- Behavior:
  - with `namespace`: upsert by `_id`
  - without `namespace`: update existing `_id` globally, fails if not found

Example:
```json
{
  "db": "myapp/main",
  "operation": "set",
  "payload": {
    "data": { "_id": "u1", "name": "Ada", "plan": "pro" }
  }
}
```

### `upsert`
Update by filter, or insert if no match.
- Required payload:
  - `filter`, `insert_data`
- Optional payload:
  - `update_data`, `expiry_behavior`, `max_docs`, `dry_run`

Example:
```json
{
  "db": "myapp/main",
  "operation": "upsert",
  "namespace": "users",
  "payload": {
    "filter": { "email": { "$eq": "a@b.com" } },
    "insert_data": { "email": "a@b.com", "name": "Ada" },
    "update_data": { "last_seen": { "$ts_now": true } },
    "max_docs": 1
  }
}
```

### `get`
Fetch by `id`/`ids`.
- Required payload:
  - one of `id`, `data._id`, `ids`
- Optional payload:
  - `_user_id`, `attach_users`, `attach_user_fields`
  - `include_archive`, `archive_only`, `fields`, `exclude_fields`, `cache`, `force_db`
- Scope:
  - `namespace` optional; if provided it is strict
  - supports `namespace="*"` or `namespaces:[...]`
- Pending writes:
  - by default, explicit `get` by `id`/`ids` checks pending accepted `insert` and explicit-id `update` previews before reading durable DB rows
  - pass `payload.force_db=true` to bypass pending state and read only the committed DB
  - `query` and `search` remain durable-DB reads only

### `count`
Count matches.
- Required payload:
  - none
- Optional payload:
  - `filter`, `include_archive`, `archive_only`, `cache`
- Scope:
  - `namespace` or `scope=all`

### `query`
Read with filter/sort/page/projection/lookups/per-item compute.

Query/search pagination response shape:
- top-level: `count`, `total_items`, `items`, `limit`, `offset`, `next_offset`, `prev_offset`
- nested `pagination`: `total_items`, `count`, `per_page`, `page`, `total_pages`, `next_page`, `prev_page`
- Required payload:
  - none (top-level `namespace` is required)
- Top-level selector:
  - `namespace`, or `namespace="*"`, or `namespaces:[...]`
  - shorthand alias also works: `operation: "query::users"`, `query::*`, `query::users,admins`
- Optional payload:
  - `_user_id`, `attach_users`, `attach_user_fields`
  - `filter`, `sort`, `limit`, `offset`, `page`, `per_page`
  - `compute`, `lookups`, `lookup_depth_override`
  - `fields`, `exclude_fields`, `include_namespace` (`include_name`)
  - `include_archive`, `archive_only`
  - `explain`, `cache`

Example:
```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "users",
  "payload": {
    "filter": { "status": { "$eq": "active" } },
    "sort": "profile.age desc, name",
    "fields": ["name", "profile.age"],
    "compute": {
      "full_name": { "$join": ["$name", " (", "$profile.age", ")"] }
    },
    "limit": 20
  }
}
```

Example: query records and side-load linked Identity users.
```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "orders",
  "payload": {
    "filter": { "status": "paid" },
    "attach_users": true,
    "attach_user_fields": ["id", "first_name", "last_name", "profile_photo", "data.display_name"]
  }
}
```

Response includes user attachments once per `_user_id`:
```json
{
  "status": "success",
  "data": {
    "items": [
      {
        "_id": "order1",
        "_user_id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001",
        "status": "paid"
      }
    ],
    "attachments": {
      "users": {
        "f9c1b3a9e2a84f9aa0bdb88e8c12f001": {
          "id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001",
          "first_name": "Ada",
          "last_name": "Lovelace",
          "profile_photo": "s3://avatars/ada.png",
          "data": {
            "display_name": "Ada"
          }
        }
      }
    }
  }
}
```

### `aggregate`
Set-level compute over matched rows.
- Required payload:
  - `compute`
- Optional payload:
  - `filter`, `include_archive`, `archive_only`, `cache`
- Scope:
  - `namespace` or `scope=all`
- Note:
  - `group_by` exists in payload but is currently not implemented.

Example:
```json
{
  "db": "myapp/main",
  "operation": "aggregate",
  "namespace": "users",
  "payload": {
    "filter": { "status": { "$eq": "active" } },
    "compute": {
      "total": { "$count": "*" },
      "avg_age": { "$avg": "age" },
      "unique_countries": { "$distinct": "country" }
    }
  }
}
```

### `search`
FTS5 search on live docs.
- Required payload:
  - top-level `namespace` (or `namespace="*"` / `namespaces:[...]`)
  - `search`
- Shorthand alias also works: `search::users`, `search::*`, `search::users,admins`
- Optional payload:
  - `filter`, `sort`, `limit`, `offset`, `page`, `per_page`
  - `lookups`, `lookup_depth_override`
  - `fields`, `exclude_fields`, `include_namespace` (`include_name`), `cache`
- Scope:
  - namespace required
- Requires:
  - FTS capability is always available; the current DB must have `fts_enabled=true`.
  - DB-level `fts_enabled=true` (`enable_fts_index`)

### `metrics_ingest`
Append one or many metric events for lightweight SaaS metrics.
- Required payload:
  - `events` as a non-empty array
- Event fields:
  - `event` is required
  - `ts` is optional; defaults to server UTC now
  - `value` is optional; defaults to `1`
  - `tenant_id`, `user_id`, `dimensions`, `metadata` are optional
- Ack behavior:
  - defaults to `commit:false` for accepted/queued ingest
  - set `commit:true` to wait for SQLite commit before response
- Catalog behavior:
  - event names are registered in `__kdb_metrics_catalog`
  - dimension paths from `dimensions` are registered under the event name

Example:
```json
{
  "db": "app/main",
  "operation": "metrics_ingest",
  "payload": {
    "events": [
      {
        "event": "api.request",
        "ts": "2026-06-14T13:22:10Z",
        "tenant_id": "tenant_123",
        "user_id": "user_456",
        "value": 1,
        "dimensions": {
          "endpoint": "/v1/chat",
          "method": "POST",
          "status": 200,
          "duration_ms": 183
        },
        "metadata": {
          "request_id": "req_abc"
        }
      }
    ]
  }
}
```

Accepted response:
```json
{
  "status": "success",
  "data": {
    "ids": ["evt_abc"],
    "queued": true
  },
  "ack_mode": "accepted",
  "ack_status": "queued",
  "committed": false,
  "is_async_ack": true
}
```

### `metrics_query`
Aggregate metric events into one or many labeled result sets.
- Required payload:
  - `event` or `events`
  - `range` or `start` + `end`
  - `metrics`
- Date inputs:
  - RFC3339 UTC datetime
  - `YYYY-MM-DD`; `start` expands to `00:00:00Z`, `end` expands to `23:59:59Z`
- Range inputs:
  - rolling ranges: `24h`, `7d`, `3days`, `2weeks`, `4months`, `1year`
  - calendar aliases: `today`, `yesterday`, `this_week`, `last_week`, `this_month`, `last_month`, `this_year`, `last_year`
  - dash aliases are accepted and normalized to underscores, e.g. `last-month` => `last_month`
  - rolling ranges mean `now - range` to `now`; calendar aliases snap to UTC calendar boundaries
- Optional payload:
  - `alias`, `label`, `interval`, `bucket_label`
  - `filter`, `group_by`, `sort`, `limit`, `offset`
  - `batch`, `cache`
- Cache behavior:
  - enabled by default with `KONGODB_METRIC_EVENTS_CACHE_TTL_SECS=30`
  - `metrics_ingest` does not invalidate cache on every ingest
  - `payload.cache=false` bypasses cache
  - `payload.cache=N` caches for `N` seconds
  - `payload.cache=-1` invalidates metric events cache for the DB
- Metric ops:
  - `count`, `sum`, `avg`, `min`, `max`, `distinct`, `count_distinct`
- Response shape:
  - `data.results` is always keyed by result alias
  - each result includes normalized `range`, `start`, `end`, and `interval`
  - item group values live under `items[].groups`
  - computed values live under `items[].metrics`

Example:
```json
{
  "db": "app/main",
  "operation": "metrics_query",
  "payload": {
    "alias": "api_requests",
    "label": "API Requests",
    "event": "api.request",
    "start": "2026-06-14",
    "end": "2026-06-14",
    "interval": "hour",
    "bucket_label": "{{bucket HH:mm}}",
    "filter": {
      "tenant_id": "tenant_123",
      "dimensions.status": { "$gte": 200 }
    },
    "group_by": [
      {
        "field": "dimensions.endpoint",
        "alias": "endpoint",
        "label": "Endpoint"
      }
    ],
    "metrics": [
      {
        "op": "count",
        "field": "*",
        "alias": "requests",
        "label": "Requests"
      },
      {
        "op": "avg",
        "field": "dimensions.duration_ms",
        "alias": "avg_duration_ms",
        "label": "Avg duration"
      }
    ],
    "sort": "bucket asc, requests desc"
  }
}
```

Response:
```json
{
  "status": "success",
  "data": {
    "count": 1,
    "results": {
      "api_requests": {
        "alias": "api_requests",
        "label": "API Requests",
        "range": null,
        "start": "2026-06-14T00:00:00Z",
        "end": "2026-06-14T23:59:59Z",
        "interval": "hour",
        "labels": {
          "groups": {
            "bucket": "Bucket",
            "bucket_label": "Bucket Label",
            "endpoint": "Endpoint"
          },
          "metrics": {
            "requests": "Requests",
            "avg_duration_ms": "Avg duration"
          }
        },
        "count": 1,
        "items": [
          {
            "bucket": "2026-06-14T13:00:00Z",
            "bucket_label": "13:00",
            "groups": {
              "endpoint": "/v1/chat"
            },
            "metrics": {
              "requests": 120,
              "avg_duration_ms": 183.4
            }
          }
        ],
        "warnings": []
      }
    }
  }
}
```

Batch example:
```json
{
  "db": "app/main",
  "operation": "metrics_query",
  "payload": {
    "batch": [
      {
        "alias": "api_requests",
        "event": "api.request",
        "range": "24h",
        "interval": "hour",
        "metrics": [
          { "op": "count", "field": "*", "alias": "requests", "label": "Requests" }
        ]
      },
      {
        "alias": "signups",
        "event": "user.signup",
        "range": "7d",
        "interval": "day",
        "metrics": [
          { "op": "count", "field": "*", "alias": "signups", "label": "Signups" }
        ]
      }
    ]
  }
}
```

### `metrics_catalog`
List discovered metric event names and dimension paths.
- Optional payload:
  - `type`: `event` or `dimension`
  - `name`: context key; for dimensions this is the event name
  - `value`: exact catalog value
  - `limit`, `offset`
- Catalog rows:
  - events: `{ "type": "event", "name": "name", "value": "api.request" }`
  - dimensions: `{ "type": "dimension", "name": "api.request", "value": "dimensions.endpoint" }`

Example: list event names.
```json
{
  "db": "app/main",
  "operation": "metrics_catalog",
  "payload": {
    "type": "event"
  }
}
```

Example: list dimensions for one event.
```json
{
  "db": "app/main",
  "operation": "metrics_catalog",
  "payload": {
    "type": "dimension",
    "name": "api.request"
  }
}
```

### Audit Logs (`audit_*`)
Audit Logs store application-supplied activity as immutable rows. Kongodb does not infer an actor from the access key or automatically audit every gateway request; the application explicitly records the events that carry useful business context.

Internal table:
- `__kdb_audit_logs`: append-only audit events ordered by `ts`.

Rules:
- `audit_ingest` requires a non-empty `events[]` list.
- Every event requires `action`.
- `_id` defaults to a dashless UUID v4 with the `aud_` prefix.
- `ts` defaults to server UTC now and accepts RFC3339 or `YYYY-MM-DD`.
- `status` defaults to `success`; applications may use their own status vocabulary.
- `audit_ingest` defaults to `commit:true`; callers may explicitly choose accepted acknowledgement with `commit:false`.
- Audit operations do not expose update or delete commands.
- `data` can contain arbitrary JSON context.

### `audit_ingest`
Append one or more audit events.

```json
{
  "db": "app/main",
  "operation": "audit_ingest",
  "payload": {
    "commit": true,
    "events": [
      {
        "action": "user.login",
        "actor_type": "user",
        "actor_id": "user_123",
        "target_type": "session",
        "target_id": "session_456",
        "status": "success",
        "source": "api",
        "request_id": "req_789",
        "ip_address": "203.0.113.10",
        "message": "User signed in with Google",
        "data": {
          "provider": "google"
        }
      }
    ]
  }
}
```

### `audit_query`
Query immutable audit events. Results use the standard `items`, `total_items`, `limit`, `offset`, and nested `pagination` response shape.

Optional payload:
- `search`: case-insensitive search over action, message, actor id, and target id
- `action`, `actor_type`, `actor_id`
- `target_type`, `target_id`
- `start`, `end`: RFC3339 or `YYYY-MM-DD`
- `page`, `per_page`, `limit`, `offset`

```json
{
  "db": "app/main",
  "operation": "audit_query",
  "payload": {
    "action": "user.login",
    "actor_id": "user_123",
    "start": "2026-07-01",
    "end": "2026-07-31",
    "page": 1,
    "per_page": 25
  }
}
```

### Identity Store (`user_*`)
Identity operations store login-related metadata for your app. Kongodb does not authenticate users, verify passwords, validate OAuth tokens, issue sessions, or enforce app permissions.

Internal tables:
- `__kdb_identity_users`: local user/account metadata.
- `__kdb_identity_providers`: Google/GitHub/custom provider mappings.
- `__kdb_identity_tokens`: app-generated token hashes.
- `__kdb_identity_events`: append-only identity lifecycle events.

Rules:
- User ids default to dashless UUID v4.
- `user_create` may accept caller-provided `user_id`; it must be a 32-character dashless UUID string.
- `first_name`, `last_name`, and `profile_photo` are first-class profile columns.
- `requires_password_change` is an application-facing account signal; Kongodb stores and returns it but does not enforce login behavior.
- Presentation preferences such as `display_name`, `timezone`, and `locale` should live in `data`.
- Store `password_hash`, never raw passwords.
- Store `token_hash`, never raw reset/magic/API tokens.
- Status values are app-defined strings.
- Soft-deleted users keep email/provider identity reserved.
- `purge=true` hard-deletes the user, providers, tokens, and events.

### `user_create`
Create one identity user record.
- Optional payload:
  - `user_id`, `email`, `username`, `phone`
  - `first_name`, `last_name`, `profile_photo`
  - `status` defaults to `active`
  - `status_reason`
  - `password_hash`, `password_algo`
  - `requires_password_change` defaults to `false`
  - `provider`, `provider_user_id`
  - `data` object for app-specific metadata

Example:
```json
{
  "db": "app/main",
  "operation": "user_create",
  "payload": {
    "email": "user@example.com",
    "username": "mardix",
    "first_name": "Mardix",
    "last_name": "Example",
    "profile_photo": "s3://app-files/avatars/user.png",
    "password_hash": "$argon2id$...",
    "password_algo": "argon2id",
    "requires_password_change": true,
    "data": {
      "display_name": "Mardix",
      "role": "admin"
    }
  }
}
```

Example: create and link a provider identity.
```json
{
  "db": "app/main",
  "operation": "user_create",
  "payload": {
    "email": "user@gmail.com",
    "provider": "google",
    "provider_user_id": "10982374238947238947",
    "data": {
      "name": "Jane Doe"
    }
  }
}
```

### `user_get`
Fetch one identity user.
- Required payload:
  - one of `user_id`, `id`, `email`, `username`
  - or `provider` + `provider_user_id`

Example:
```json
{
  "db": "app/main",
  "operation": "user_get",
  "payload": {
    "email": "user@example.com"
  }
}
```

Example: provider lookup after your app validates Google/GitHub OAuth.
```json
{
  "db": "app/main",
  "operation": "user_get",
  "payload": {
    "provider": "github",
    "provider_user_id": "827364"
  }
}
```

### `user_update`
Update one identity profile. `requires_password_change` accepts both `true` and `false`, allowing the application to set the requirement and clear it after a successful password change.

```json
{
  "db": "app/main",
  "operation": "user_update",
  "payload": {
    "user_id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001",
    "requires_password_change": false
  }
}
```

### `user_list`
List/query identity users with pagination.
- Optional payload:
  - `search` or `q`: matches id, email, username, or phone
  - `status`, `email`, `username`
  - `page`, `per_page`, `limit`, `offset`

Example:
```json
{
  "db": "app/main",
  "operation": "user_list",
  "payload": {
    "search": "gmail.com",
    "status": "active",
    "page": 1,
    "per_page": 25
  }
}
```

### `user_link_provider`
Link an external provider identity to an existing local user.
- Required payload:
  - `user_id` or `id`
  - `provider`
  - `provider_user_id`
- Optional payload:
  - `email`
  - `data` object

Example:
```json
{
  "db": "app/main",
  "operation": "user_link_provider",
  "payload": {
    "user_id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001",
    "provider": "github",
    "provider_user_id": "827364",
    "email": "user@example.com",
    "data": {
      "login": "octocat"
    }
  }
}
```

### `user_unlink_provider`
Unlink one provider identity.
- Required payload:
  - `provider`
  - `provider_user_id`
- Optional payload:
  - `user_id` or `id` to make the unlink strict to that user

Example:
```json
{
  "db": "app/main",
  "operation": "user_unlink_provider",
  "payload": {
    "user_id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001",
    "provider": "github",
    "provider_user_id": "827364"
  }
}
```

### `user_update_status`
Update app-defined user status and optionally schedule a future transition.
- Required payload:
  - `user_id` or `id`
  - `status`
- Optional payload:
  - `status_reason`
  - exactly one of `status_expires_at` or `status_expires_in`
  - `status_next` is required when expiration is provided
  - `status_next_reason`
  - `changed_by`
- Reaper behavior:
  - when `status_expires_at` is reached, the reaper applies `status_next`
  - the transition logs `user.status_transitioned`

Example: ban for two days, then return to active.
```json
{
  "db": "app/main",
  "operation": "user_update_status",
  "payload": {
    "user_id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001",
    "status": "banned",
    "status_reason": "abuse",
    "status_expires_in": 172800,
    "status_next": "active",
    "status_next_reason": "temporary ban expired",
    "changed_by": "admin:42"
  }
}
```

### `user_create_token`
Store one app-generated token hash.
- Required payload:
  - `user_id` or `id`
  - `kind`
  - `token_hash`
- Optional payload:
  - exactly one of `expires_at` or `expires_in`
  - `allow_multi`, default `false`
  - `data` object
- Token behavior:
  - `allow_multi=false` revokes existing active tokens for the same `user_id + kind`
  - expired tokens are removed by the reaper

Example:
```json
{
  "db": "app/main",
  "operation": "user_create_token",
  "payload": {
    "user_id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001",
    "kind": "password_reset",
    "token_hash": "sha256:abc123...",
    "expires_in": 300
  }
}
```

### `user_delete`
Soft-delete or purge an identity user.
- Required payload:
  - `user_id` or `id`
- Optional payload:
  - `status_reason`
  - `purge`
- Soft delete behavior:
  - sets `status=deleted`
  - sets `deleted_at`
  - revokes active tokens
  - keeps email/provider mappings reserved
- Purge behavior:
  - hard-deletes user, providers, tokens, and events

Example: soft delete.
```json
{
  "db": "app/main",
  "operation": "user_delete",
  "payload": {
    "user_id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001",
    "status_reason": "user requested deletion"
  }
}
```

Example: purge.
```json
{
  "db": "app/main",
  "operation": "user_delete",
  "payload": {
    "user_id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001",
    "purge": true
  }
}
```

### File Catalog (`file_*`)
File operations store metadata for files or objects that your application uploads somewhere else. Kongodb does not upload, download, stream, move, or delete the actual bytes in this phase.

Internal table:
- `__kdb_files`: file/object metadata registry.

Rules:
- File ids default to dashless UUID v4.
- `uploaded_at` is when the app/object store received the file. If omitted, Kongodb sets it to server UTC now.
- `created_at` is when the metadata row was registered in Kongodb.
- `owner_type` + `owner_id` are optional generic attachment fields, such as `user` + `user_123` or `invoice` + `inv_001`.
- `file_delete` soft-deletes metadata by setting `status=deleted` and `deleted_at`.
- `file_delete` with `purge=true` hard-deletes the metadata row only.
- The application is responsible for actual S3/local object cleanup.

### `file_create`
Create one file metadata record.
- Required payload:
  - `storage_backend`
  - `storage_path`
- Optional payload:
  - `id` as a 32-character dashless UUID string
  - `bucket`, defaults to `default`
  - `filename`, `content_type`, `size_bytes`, `sha256`
  - `status`, defaults to `active`
  - `owner_type`, `owner_id`
  - `metadata` object
  - `uploaded_at`, `expires_at` as RFC3339 UTC-compatible datetimes

Example:
```json
{
  "db": "app/main",
  "operation": "file_create",
  "payload": {
    "bucket": "avatars",
    "storage_backend": "s3",
    "storage_path": "s3://app-files/uploads/users/u123/avatar.png",
    "filename": "avatar.png",
    "content_type": "image/png",
    "size_bytes": 182331,
    "sha256": "abc123...",
    "owner_type": "user",
    "owner_id": "u123",
    "metadata": {
      "width": 512,
      "height": 512
    }
  }
}
```

### `file_get`
Fetch one file metadata row.

Example:
```json
{
  "db": "app/main",
  "operation": "file_get",
  "payload": {
    "id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001"
  }
}
```

### `file_list`
List file metadata rows with pagination.
- Optional payload:
  - `bucket`, `status`
  - `owner_type`, `owner_id`
  - `storage_backend`, `content_type`
  - `search` or `q`
  - `page`, `per_page`, `limit`, `offset`

Example: list all files attached to a user.
```json
{
  "db": "app/main",
  "operation": "file_list",
  "payload": {
    "owner_type": "user",
    "owner_id": "u123",
    "status": "active",
    "page": 1,
    "per_page": 25
  }
}
```

### `file_update`
Update mutable metadata.
- Required payload:
  - `id`
- Optional payload:
  - `bucket`, `storage_backend`, `storage_path`
  - `filename`, `content_type`, `size_bytes`, `sha256`
  - `status`, `owner_type`, `owner_id`
  - `metadata`, `uploaded_at`, `expires_at`

Example:
```json
{
  "db": "app/main",
  "operation": "file_update",
  "payload": {
    "id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001",
    "metadata": {
      "width": 1024,
      "height": 1024,
      "variant": "retina"
    }
  }
}
```

### `file_delete`
Soft-delete or purge one file metadata row.
- Required payload:
  - `id`
- Optional payload:
  - `purge`

Example: soft delete metadata.
```json
{
  "db": "app/main",
  "operation": "file_delete",
  "payload": {
    "id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001"
  }
}
```

Example: purge metadata.
```json
{
  "db": "app/main",
  "operation": "file_delete",
  "payload": {
    "id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001",
    "purge": true
  }
}
```

### `delete`
Delete one or many documents.
- Required payload:
  - exactly one of `id`, `ids`, `filter`
- Optional payload:
  - `ttl_seconds`, `purge`, `max_docs`, `dry_run`
- Notes:
  - `purge=true`: hard delete
  - default: soft delete; matched documents move to `__kdb_archive` then leave live data
  - `id`/`ids` mode: `namespace` optional; strict if provided
  - `filter` mode: `namespace` required unless `scope=all`

Example: delete one document by `id`
```json
{
  "db": "myapp/main",
  "operation": "delete",
  "payload": {
    "id": "u1"
  }
}
```

Example: delete many documents by `ids`
```json
{
  "db": "myapp/main",
  "operation": "delete",
  "payload": {
    "ids": ["u1", "u2"]
  }
}
```

Example: delete many documents by filter
```json
{
  "db": "myapp/main",
  "operation": "delete",
  "namespace": "users",
  "payload": {
    "filter": { "status": { "$eq": "inactive" } },
    "max_docs": 100
  }
}
```

### `set_ttl`
Set/reset TTL for selected docs.
- Required payload:
  - selector: `ids|filter`
  - `ttl_seconds`
- Optional payload:
  - `expiry_behavior(archive|delete)`, `max_docs`, `dry_run`
- Scope:
  - `ids` mode: `namespace` optional; strict if provided
  - `filter` mode: `namespace` required unless `scope=all`

### `import_jsonl`
Stream/stage large JSONL ingest through a background job.
- Required payload:
  - `source_path`
- Required top-level:
  - `namespace`
- Optional payload:
  - `source_hash`, `alias_import_pk`, `drop_keys`
  - `on_conflict(error|skip|replace|merge)`
  - `ignore_input_id`, `allow_system_timestamps`
  - `batch_size`, `resumable`

### `export_jsonl`
Export matched data to JSONL through a background job.
- Required payload: none
- Optional payload:
  - `target_path`, `compress`, `include_system_timestamps`
  - `filter`, `sort`, `limit`, `offset`
  - `fields`, `exclude_fields`
  - `include_archive`, `archive_only`
- Scope:
  - `namespace` or `scope=all`

---

## 2) Namespace Lifecycle
This section documents namespace-wide stats, movement, restore, and deletion workflows.

### `list_namespaces`
Lists namespaces + stats.
- Required payload: none

### `get_stats`
Read live/archive counts and bytes for one namespace.
- Required:
  - top-level `namespace`

### `recompute_stats`
Rebuilds `__kdb_system_stats` globally.
- Required payload: none

### `drop_namespace`
Namespace drop behavior.
- Required payload:
  - none (top-level `namespace` required)
- Optional payload:
  - `ttl_seconds`, `max_docs`, `purge`, `dry_run`
- Behavior:
  - `purge=false` (default): archive + delete
  - `purge=true`: hard delete

### `restore_archive`
Restore from archive.
- Required payload:
  - one selector: `txn_id` or `ids` or `namespace/filter`
- Optional payload:
  - `on_conflict(skip|replace|patch)`, `dry_run`

### `purge_archive`
Hard delete from archive only.
- Required payload:
  - one selector: `txn_id` or `ids` or `namespace/filter`
- Optional payload:
  - `dry_run`

### `change_namespace`
Move docs between namespaces by updating collection value.
- Required payload:
  - `from_namespace`
  - `to_namespace`
- Optional payload:
  - `ids|filter`, `max_docs`, `dry_run`
- Notes:
  - top-level `namespace` is rejected for this operation
  - if no selector is provided, all docs from `from_namespace` move to `to_namespace`

### `rename_namespace`
Rename a namespace across live and archive data.
- Required payload:
  - `from_namespace`
  - `to_namespace`
- Notes:
  - top-level `namespace` is rejected for this operation

---

## 3) Database Operations
This section documents DB-scoped lifecycle, replication, backup, and maintenance operations.

### `create_db`
Initialize DB at `db` path.
- Required payload: none
- Optional payload: none

### `db_exists`
Check DB existence (remote-aware in s3 mode).
- Required payload: none

### `load_db` (s3)
Preload DB into active instance.
- Required payload: none

### `sync_db` (s3)
Force snapshot + manifest sync.
- Required payload: none

### `create_snapshot` (s3)
Alias of `sync_db`.

### `list_snapshots` (s3)
List versioned snapshots.
- Required payload: none

### `get_sync_status` (s3)
Inspect local/remote status.

### `verify_db` (s3)
Verify manifest/snapshot/segment object presence.

### `restore_snapshot` (s3)
Restore local db from snapshot.
- Optional payload:
  - `snapshot_id` (latest if omitted)

### `compact_wal` (s3)
Compact manifest segment list.
- Optional payload:
  - `retain_segments` (default 1000)

### `clone_db`
Clone current DB to another path.
- Required payload:
  - `to_db_path`

### `create_backup`
Create/enqueue DB backup.
- Optional payload:
- `backup_db_path`, `backup_tag`

### `restore_backup`
Restore DB from backup selector or explicit path.
- Required payload:
- one of `backup_db_path|backup_id|backup_tag|backup_at|latest=true`

### `list_backups`
List backup catalog rows.
- Optional payload:
  - `backup_tag`, `limit`, `offset`

### `tag_backup`
Set/clear backup tag.
- Required payload:
- `backup_id` or `backup_db_path`
- Optional payload:
  - `backup_tag`

### `offload_db` (s3)
Sync and unload local copy/connection.

### `vacuum_db`
Run SQLite `VACUUM`.

### `reap_db`
Run TTL reaper immediately.

---

## 4) Jobs
This section documents the shared background job control operations.

### `get_job`
Read one job row.
- Required payload:
  - `job_id`
- Optional payload:
  - `job_type`

### `list_jobs`
List job rows with optional filters.
- Required payload: none
- Optional payload:
  - `job_type`, `status`, `limit`, `offset`

### `continue_job`
Resume/retry a resumable or failed job.
- Required payload:
  - `job_id`
- Optional payload:
  - `job_type`

### `abort_job`
Abort/cancel a running or queued job.
- Required payload:
  - `job_id`
- Optional payload:
  - `job_type`

### `transaction`
Atomic op array.
- Required top-level:
  - `data` (array of operation envelopes)
- Supported nested ops currently:
  - `insert`, `update`, `delete`

Example:
```json
{
  "db": "myapp/main",
  "operation": "transaction",
  "data": [
    {
      "operation": "insert",
      "namespace": "users",
      "payload": { "data": { "_id": "u1", "name": "Ada" } }
    },
    {
      "operation": "update",
      "namespace": "users",
      "payload": { "data": { "_id": "u1", "plan": "pro" } }
    }
  ]
}
```

---

## 5) SQL Operations
This section documents direct SQL execution and SQL table discovery.

### `sql_execute`
Execute a single SQL statement directly against the current db.
- Required payload:
  - `sql`
- Optional payload:
  - `params`, `commit`
- Notes:
  - always available and protected by the normal gateway authentication
  - supports a single `SELECT`, `WITH`, `EXPLAIN`, `INSERT`, `UPDATE`, `DELETE`, or `REPLACE`
  - also supports `CREATE TABLE`, `CREATE INDEX`, `DROP INDEX`, and `ALTER TABLE ... ADD COLUMN`
  - rejects any table/index name using reserved prefixes `__kdb_` or `sqlite_`
  - write statements use the normal per-db write coordinator; `payload.commit=false` returns after queueing, while committed mode waits for the serialized result

### `list_tables`
List user-created SQL tables for the current db.
- Required payload: none
- Excludes internal `__kdb_*` tables and SQLite internal tables.

### `get_table_schema`
Return schema columns for one user-created SQL table.
- Required payload:
  - `table`
- Excludes internal `__kdb_*` tables and SQLite internal tables.
- This is the safe schema-inspection operation to use because `sql_execute` intentionally blocks arbitrary `PRAGMA`.

```json
{ "db":"myapp/main", "operation":"get_table_schema", "payload":{"table":"customers"} }
```

## 6) Admin / System
This section documents instance-level introspection, configuration, and indexing controls.

### Inventory / Config
These operations expose system inventory and per-db internal config values.

#### `list_commands`
- Global operation (db not required)
- Lists all supported gateway command names.

#### `list_dbs`
- Global operation (db not required)
- Lists currently loaded/open DBs for this instance.

#### `list_all_dbs`
- Global operation (db not required)
- Lists all known DBs.
- `local`: filesystem scan
- `s3`: union of loaded + local + remote manifests

#### `system_get_inventory`
- Global operation (db not required)
- Lists DB inventory from the internal system catalog stored at `${KONGODB_DATA_DIR}/__kdb_system.db`.
- The system catalog is always available; live discovery remains the fallback source during refreshes.
- Does not scan/refresh by default; use `system_refresh_inventory` to rebuild/update the catalog.

Example:

```json
{ "operation": "system_get_inventory", "payload": { "limit": 100, "offset": 0 } }
```

#### `system_refresh_inventory`
- Global operation (db not required)
- Scans local/S3 known DBs and upserts current state into the system catalog.
- Records a `system.inventory_refreshed` catalog event.

Example:

```json
{ "operation": "system_refresh_inventory", "payload": {} }
```

#### `system_get_db_status`
- Global operation.
- Required top-level field: `db`
- Returns live status for the DB plus its catalog row when the system catalog is enabled.

Example:

```json
{ "db": "app/main", "operation": "system_get_db_status", "payload": {} }
```

#### `system_snapshot_db_stats`
- Global operation.
- With no `db`, snapshots currently active DBs only.
- With top-level `db`, snapshots that DB.
- Writes rows to the internal system catalog `__kdb_system_db_stats`.
- The background reaper cadence also snapshots active DBs into the always-on system catalog.

Example:

```json
{ "operation": "system_snapshot_db_stats", "payload": {} }
```

#### `system_query_db_stats`
- Global operation.
- Optional top-level `db` filters to one DB.
- Optional payload:
  - `start`: RFC3339 lower-bound timestamp
  - `end`: RFC3339 upper-bound timestamp
  - `limit`: default `100`
  - `offset`: default `0`

Example:

```json
{
  "db": "app/main",
  "operation": "system_query_db_stats",
  "payload": {
    "limit": 100
  }
}
```

#### `system_list_db_events`
- Global operation.
- Optional top-level `db` filters to one DB.
- Optional payload: `limit`, `offset`

Example:

```json
{ "operation": "system_list_db_events", "payload": { "limit": 50 } }
```

#### `get_system_stats`
- Global operation (db not required)
- Shows current instance-local runtime stats.
- Stats stay in memory and reset when the process restarts.
- Includes uptime, version, request totals, in-flight requests, read/write/admin/error counts, average/max latency, and 5m/15m/30m/1h rolling windows.
- Also includes process memory, active DB count/cap, background worker concurrency, and write queue usage.

#### `system_memory`
- Global operation (db not required)
- Compatibility command for process memory and write-queue usage.
- Includes the same `system_stats` block returned by `get_system_stats`.

#### `cleanup_temp_artifacts`
- Global operation (db not required)
- Removes stale temp files under the data dir.

#### `get_system_config`
- Required payload: none
- Returns `__kdb_system_config` rows (for current db).

#### `get_db_stats`
- Required payload: none
- Returns live in-memory counters for the current db:
  - `requests_total`
  - `reads_total`
  - `writes_total`
  - `errors_total`
  - `in_flight`
  - `last_accessed_at`

Example:

```json
{ "db": "app/main", "operation": "get_db_stats", "payload": {} }
```

#### `snapshot_db_stats`
- Required payload: none
- Writes one snapshot row into `__kdb_db_stats_rollups` for the current db.
- The snapshot uses cumulative totals; interval activity is calculated by diffing two snapshots.

Example:

```json
{ "db": "app/main", "operation": "snapshot_db_stats", "payload": {} }
```

#### `query_db_stats`
- Required payload: none
- Optional payload:
  - `start`: RFC3339 lower-bound timestamp
  - `end`: RFC3339 upper-bound timestamp
  - `limit`: max rows, default `100`, max `1000`

Example:

```json
{
  "db": "app/main",
  "operation": "query_db_stats",
  "payload": {
    "start": "2026-06-20T00:00:00Z",
    "end": "2026-06-21T00:00:00Z",
    "limit": 100
  }
}
```

### Index / FTS
These operations manage or inspect manual indexes on the internal document store.

#### `create_index`
- Required payload:
  - `index_path`
- Optional payload:
  - `index_name`

#### `drop_index`
- Required payload:
  - `index_name` or `index_path`

#### `list_indexes`
- Required payload: none

### FTS Operations
These operations manage DB-level FTS enablement and async FTS lifecycle jobs.

#### `enable_fts_index`
- Optional payload:
  - `enable` (default true)
- Only toggles DB-level FTS accessibility flag.

#### `reindex_fts`
- Required payload: none
- Enqueues async rebuild/backfill job.

#### `drop_fts_index`
- Required payload: none
- Enqueues async drop job.

## Operators Reference
This section expands the supported operator families in more detail than the cheatsheet above.

## 1) JQL Filter Operators
These operators compile into SQL predicates for document filtering.

### Logical
Use logical operators to combine multiple filter clauses.
- `$and`: all sub-filters must match.
- `$or`: any sub-filter must match.
- `$nor`: none of sub-filters match.
- `$not`: negate one filter.

Example:
```json
{
  "$and": [
    { "status": { "$in": ["active", "trial"] } },
    {
      "$or": [
        { "plan": { "$eq": "pro" } },
        { "plan": { "$eq": "enterprise" } }
      ]
    }
  ]
}
```

### Comparison
Use comparison operators for equality, range, and existence checks.
- `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte`, `$between`, `$exists`

Example:
```json
{ "profile.age": { "$between": [21, 65] } }
```

### Set/Array
Use these operators for membership tests, array matching, and size checks.
- `$in`, `$nin`, `$includes`, `$nincludes`, `$all`, `$any`, `$none`, `$elemMatch`, `$size`

Example:
```json
{
  "tags": { "$all": ["beta", "paid"] },
  "roles": { "$any": ["admin", "owner"] }
}
```

### String
Use these operators for prefix, suffix, substring, case-insensitive, and regex matching.
- `$startsWith`, `$endsWith`, `$contains`
- case-insensitive: `$ilike`, `$istartsWith`, `$iendsWith`, `$icontains`
- `$regex`

### Type
Use `$type` to constrain a field by JSON value type.
- `$type`

## 2) Compute Operators (`payload.compute`)
These operators create derived values either across the whole result set or per returned row.

### Aggregate operation (set-level)
Use these metrics with the `aggregate` operation to compute values over the matched set.
Supported operators:
- `$count`, `$sum`, `$avg`, `$min`, `$max`, `$distinct`

Supported options per metric:
- `$distinct: true` (modifier for applicable metrics, e.g. `$count`)
- `$filter: { ... }` (metric-local JQL)

Examples:
```json
{
  "compute": {
    "total": { "$count": "*" },
    "total_unique_country": { "$count": "country", "$distinct": true },
    "country_values": { "$distinct": "country" },
    "active_count": { "$count": "*", "$filter": { "status": { "$eq": "active" } } }
  }
}
```

### Query operation (per-row)
Use these metrics in `query` to compute derived values for each returned item.
Supported operators:
- `$count`, `$sum`, `$avg`, `$min`, `$max`, `$distinct`, `$size`, `$join`

Examples:
```json
{
  "compute": {
    "events_count": { "$count": "events[]" },
    "unique_tags": { "$distinct": "tags[]" },
    "items_len": { "$size": "items[]" },
    "label": { "$join": ["$first_name", " ", "$last_name"] }
  }
}
```

`$join` token rule:
- Any string token that starts with `$` is treated as a path on the current row.
- Example: `$profile.email`.

## 3) Lookup Operators (`payload.lookups`)
This section describes the lookup system used to enrich query and search results.

Lookup spec supports:
- `from`, `local_field`, `foreign_field`
- `match`: `$eq` (default), `$in`, `$contains`, `$overlap`
- `multi`, `filter`, `fields`, `sort`, `limit`
- `preserve_order`, `dedupe`, `on_missing`, `strict_path`, `cache_lookup`
- nested `lookups`

Context selectors in lookup paths:
- `$self.<path>` current document
- `$parent.<path>` parent context
- `$root.<path>` root row
- `$lookup.<alias>...` another lookup alias in current scope

Example:
```json
{
  "lookups": {
    "books": {
      "from": "books",
      "local_field": "favorite_books[]",
      "foreign_field": "_id",
      "match": "$in",
      "multi": true,
      "fields": ["_id", "title"]
    }
  }
}
```

## 4) Value Operators in Write Payloads (`data`, `insert_data`, `update_data`)
This section describes the special value objects recognized during writes.

These are recognized when a value is a single-key object with a `$` operator key.

### Generator operators
Generator operators create timestamps, ids, and hashes at write time.
- `$ts_now`: UTC timestamp string (optionally shifted by `{days,hours,minutes,seconds}`)
- `$ts_now_ms`: UTC epoch milliseconds (optionally shifted)
- `$id_uuidv4`, `$id_uuidv7`: UUID generation (options: `prefix`, `suffix`, `dash`)
- `$id_random`: short random ID (options: `len`, `prefix`, `suffix`)
- `$hash_value`: hash from input (supports options)

Example:
```json
{
  "data": {
    "_id": { "$id_uuidv4": { "prefix": "session:" } },
    "created_at": { "$ts_now": true }
  }
}
```

### Mutation operators
Mutation operators modify existing values without replacing the whole field manually.
- `$unset`: remove field
- `$inc`: increment/decrement number
- `$push`: append item to array
- `$pop`: pop from end
- `$extend`: append many items to array
- `$pull`: remove matching item(s)
- `$addset`: add item only if missing

Example:
```json
{
  "data": {
    "score": { "$inc": 1 },
    "events": { "$push": { "type": "login" } },
    "tags": { "$addset": "beta" }
  }
}
```

Strictness:
- `KONGODB_STRICT_MUTATIONS_OPERATORS=false` (default): invalid/unknown mutation ops become no-op.
- `KONGODB_STRICT_MUTATIONS_OPERATORS=true`: invalid/unknown mutation ops return `400`.

## Query/Sort/Projection Notes
These notes clarify how sorting, projection, and cache behavior work at query time.

### Sort
Sort clauses can be written in object form or string form.
- Object form:
```json
{ "profile.age": -1, "name": 1 }
```
- String form:
```json
"profile.age desc, name asc"
```
- If direction omitted, default is ascending.

### Projection
Projection controls which fields are returned in read responses.
- `fields`: include list.
- `exclude_fields`: exclude list.
- `_id` is always returned.

## Cache Behavior (`payload.cache`)
These flags control whether reads use cache, bypass it, or invalidate it before execution.

Read operations (`get`, `count`, `query`, `search`):
- `false` or `0`: bypass cache
- `true` or `1`: use default TTL
- `N > 1`: use per-request TTL seconds
- `-1`: invalidate relevant cache scope and run uncached

## Storage Modes (High Level)
These are the supported runtime storage backends for database files and remote sync behavior.

- `local`: filesystem `.db` files under `KONGODB_DATA_DIR`.
- `s3`: object-store mode with WAL/manifest/snapshots in a single remote S3 tier.

## Deployment
This section covers the common deployment paths and how Kongodb stores data in each environment.

### Docker Data Persistence
The Docker image defaults `KONGODB_DATA_DIR` to `/data` and declares `/data` as a volume. Local backup/export defaults are also moved under `/data` inside the image:

```env
KONGODB_DATA_DIR=/data
KONGODB_BACKUP_PATH=/data/backups
KONGODB_EXPORT_PATH=/data/exports
```

You can launch the image without manually creating a volume. Docker will create an anonymous volume for `/data`, but that is harder to inspect, backup, or reuse. For real usage, prefer a named volume or host-mounted path.

For durable local-container storage, mount a volume:

```bash
docker run \
  -p 8080:8080 \
  -v kongodb-data:/data \
  kongodb
```

Docker creates the named volume automatically if it does not exist.

Runtime environment variables keep precedence over baked-in env files, so this is also valid:

```bash
docker run \
  -p 8080:8080 \
  -e KONGODB_DATA_DIR=/var/lib/kongodb \
  -v kongodb-data:/var/lib/kongodb \
  kongodb
```

For a self-managed server, a host-mounted path is often easier to back up than a Docker named volume:

```bash
mkdir -p /srv/kongodb/data

docker run -d \
  --name kongodb \
  --restart unless-stopped \
  -p 127.0.0.1:8080:8080 \
  --env-file ./kongodb.env.prod \
  -v /srv/kongodb/data:/data \
  kongodb
```

Binding to `127.0.0.1` keeps Kongodb private to the host so a reverse proxy such as Caddy, Nginx, or Traefik can terminate HTTPS publicly.

### Docker Compose
The repository includes [`docker-compose.yaml`](/Users/mardix/Dropbox/Projects/kongodb/docker-compose.yaml) as a local durable example.

Start it with:

```bash
docker compose up --build -d
```

The compose file:

- builds the local Dockerfile
- builds and serves the Admin UI at `http://localhost:8080/_/kdb/admin/`
- loads `kongodb.env`
- overrides Docker-specific settings such as `KONGODB_DATA_DIR=/data`
- stores DB files, local backups, and local exports under `/data`
- creates a named volume `kongodb-data`
- mounts that volume to `/data`

Test the service:

```bash
curl http://localhost:8080/_/kdb/ping
```

Open the bundled Admin UI at `http://localhost:8080/_/kdb/admin/`. When auth is
enabled, use `kongodb` as the browser prompt username and the configured
`KONGODB_ACCESS_KEY` as its password.

Gateway example:

```bash
curl -X POST http://localhost:8080/_/kdb/gateway \
  -H 'content-type: application/json' \
  -H 'x-access-key: change-me' \
  -d '{"db":"app/main","operation":"create_db","payload":{}}'
```

For production, replace `change-me`, review `KONGODB_BASE_PATH`, and consider using a host-mounted path instead of a named volume if your backup tooling expects normal filesystem paths.

### Cloud Run
On Cloud Run, use `KONGODB_STORAGE_MODE=s3` for durable storage. Container-local paths such as `/tmp/kongodb` or `/data` are instance-local cache only.

Recommended Cloud Run shape:

```env
KONGODB_STORAGE_MODE=s3
KONGODB_DATA_DIR=/tmp/kongodb
KONGODB_S3_BUCKET=...
KONGODB_S3_PREFIX=data/kongodb/data
KONGODB_S3_REGION=...
KONGODB_S3_ACCESS_KEY=...
KONGODB_S3_SECRET_KEY=...
```

## Configuration

[`kongodb.env`](/Users/mardix/Dropbox/Projects/kongodb/kongodb.env) is the canonical environment template. Kongodb exposes deployment choices, API semantics, retention, and bounded resource controls; low-level worker thresholds use internal defaults selected by `KONGODB_RUNTIME_PROFILE`.

SQL execution, FTS capability, metric events, auto-indexing, JSONB storage, the system catalog, safe hydration, temporary-file cleanup, and background job workers are always enabled. Per-DB `fts_enabled` still controls whether a specific database can be searched.

### Server, Web, and Authentication

| Config Name | Default | Description |
|---|---:|---|
| `KONGODB_PORT` | `8080` | HTTP listen port. |
| `KONGODB_BASE_PATH` | `/_/kdb` | Prefix for `/gateway`, `/ping`, `/meta/operations`, `/doc`, and `/admin/`. |
| `KONGODB_AUTH_MODE` | `access_key` | Authentication policy: `access_key` requires credentials; `none` explicitly permits unauthenticated access. Invalid values fail startup. |
| `KONGODB_ACCESS_KEY` | empty | `X-Access-Key` value and browser Basic-auth password. Required when `KONGODB_AUTH_MODE=access_key`. |
| `KONGODB_CORS_ALLOWED_ORIGINS` | empty | Comma-separated origins for standalone browser clients. The bundled Admin UI is same-origin. |
| `KONGODB_MAX_REQUEST_BYTES` | `16777216` | Maximum HTTP request body bytes. |
| `KONGODB_OPERATION_TIMEOUT_MS` | `30000` | Gateway operation timeout in milliseconds. |
| `KONGODB_ADMIN_UI_ENABLED` | `true` | Serve the bundled SPA at `${KONGODB_BASE_PATH}/admin/`. |
| `KONGODB_DOCS_ENABLED` | `true` | Serve rendered Markdown at `${KONGODB_BASE_PATH}/doc`. |
| `KONGODB_DOCS_FILE` | `DOCUMENTATION.md` | Markdown source rendered by `/doc`. |

`/gateway`, `/meta/operations`, `/doc`, and `/admin/` follow service authentication. `/ping` remains open.

Production:

```env
KONGODB_AUTH_MODE=access_key
KONGODB_ACCESS_KEY=a-long-random-secret
```

Trusted local development:

```env
KONGODB_AUTH_MODE=none
KONGODB_ACCESS_KEY=
```

Kongo fails at startup when `KONGODB_AUTH_MODE` is invalid or when `access_key` mode has no key. In `none` mode, any configured access key is ignored.

### Storage and S3

| Config Name | Default | Description |
|---|---:|---|
| `KONGODB_STORAGE_MODE` | `local` | `local` or `s3`. |
| `KONGODB_DATA_DIR` | `./data` | Local durable root or S3-mode working-file root. Docker uses `/data`. |
| `KONGODB_S3_BUCKET` | empty | S3 bucket required in `s3` mode. |
| `KONGODB_S3_PREFIX` | `data/kongodb/data` | Base object prefix for database artifacts. |
| `KONGODB_S3_REGION` | `us-east-1` | S3 region. |
| `KONGODB_S3_ENDPOINT` | empty | Optional custom S3-compatible endpoint. |
| `KONGODB_S3_ACCESS_KEY` | empty | S3 access key. |
| `KONGODB_S3_SECRET_KEY` | empty | S3 secret key. |
| `KONGODB_S3_SESSION_TOKEN` | empty | Optional temporary-credential token. |

### Runtime and Concurrency

| Config Name | Default | Description |
|---|---:|---|
| `KONGODB_RUNTIME_PROFILE` | `balanced` | `memory`, `balanced`, or `throughput`; controls internal cache, queue, batch, idle-close, lookup, and concurrency defaults. |
| `KONGODB_MAX_ACTIVE_DBS` | profile (`100`) | Optional profile override for hot/open DB connections. Additional DBs evict least-recently-used connections. |
| `KONGODB_WORKER_CONCURRENCY` | profile (`4`) | Shared DB-work concurrency for reaper, backups, jobs, and remote sync. |

Profile defaults:

| Profile | Active DBs | Worker Concurrency | Read Cache Entries | Write Queue | Export/Metric Batch |
|---|---:|---:|---:|---:|---:|
| `memory` | `25` | `2` | `2500` | `2500` | `500` |
| `balanced` | `100` | `4` | `10000` | `10000` | `1000` |
| `throughput` | `250` | `8` | `50000` | `50000` | `5000` |

### Replication and Snapshots

| Config Name | Default | Description |
|---|---:|---|
| `KONGODB_REPLICATION_MODE` | `async` | `sync` waits for remote persistence; `async` flushes through the background replication worker. |
| `KONGODB_PRELOAD_DBS` | empty | Comma-separated S3-mode DB paths loaded at startup. |
| `KONGODB_SNAPSHOT_EVERY_WRITES` | `100` | Versioned snapshot cadence per DB write count. |
| `KONGODB_SNAPSHOT_RETENTION_DAYS` | `14` | Versioned snapshot age retention. `current.db` is not rotated. |
| `KONGODB_REMOTE_SYNC_INTERVAL_SECS` | `3` | Cross-instance remote snapshot polling interval; `0` disables polling. |

Writer leases, WAL segment size, flush cadence, safe hydrate, integrity checks, snapshot count cap, and temporary-artifact cleanup use fixed safe defaults.

### Reads, Writes, and Responses

| Config Name | Default | Description |
|---|---:|---|
| `KONGODB_CACHE_TTL_SECS` | `15` | Default read-cache TTL; `0` disables the read cache. |
| `KONGODB_WRITE_MODE` | `committed` | `direct`, `committed`, or `accepted`. `direct` bypasses the coordinator; `committed` waits; `accepted` queues and acknowledges. Request `payload.commit` overrides committed vs accepted. |
| `KONGODB_QUERY_DEFAULT_LIMIT` | `50` | Default query and per-lookup limit. |
| `KONGODB_QUERY_LOOKUP_MAX_DEPTH` | `3` | Maximum nested lookup depth. |
| `KONGODB_QUERY_LOOKUP_UNCAPPED_OVERRIDE_ENABLED` | `false` | Allows explicit request-level lookup depth override beyond the configured cap. |
| `KONGODB_RESPONSE_INCLUDE_SYSTEM_TIMESTAMPS` | `true` | Include `_created_at` and `_modified_at`. |
| `KONGODB_RESPONSE_INCLUDE_NAMESPACE` | `false` | Include `_namespace` by default. |
| `KONGODB_STRICT_MUTATIONS_OPERATORS` | `false` | Reject invalid mutation operators and operand types instead of leaving them unchanged. |

### Lifecycle, Metrics, and System History

| Config Name | Default | Description |
|---|---:|---|
| `KONGODB_ARCHIVE_TTL_SECS` | empty | Archive retention before permanent purge; empty retains until explicit purge. |
| `KONGODB_DELETE_DEFAULT_TTL_SECS` | empty | Default soft-delete TTL when the request does not provide one. |
| `KONGODB_SYSTEM_RETENTION_DAYS` | `14` | Historical system-catalog stats/event retention. Inventory rows remain. |
| `KONGODB_METRIC_EVENTS_CACHE_TTL_SECS` | `30` | Metrics-query cache TTL; `0` disables metrics caching. |
| `KONGODB_METRIC_EVENTS_RETENTION_DAYS` | empty | Raw metric-event retention; empty keeps events indefinitely. |

### Backup, Export, and Jobs

| Config Name | Default | Description |
|---|---:|---|
| `KONGODB_BACKUP_PATH` | `./backups` | Manual/automatic backup destination: local path or full `s3://bucket/prefix`. |
| `KONGODB_BACKUP_EVERY_SECS` | `0` | Change-aware automatic backup maximum staleness; `0` disables only automatic backups. |
| `KONGODB_BACKUP_RETENTION_DAYS` | `30` | Backup artifact age retention. An internal count cap remains as a safety bound. |
| `KONGODB_EXPORT_PATH` | `./exports` | Generated export destination: local path or full `s3://bucket/prefix`. |
| `KONGODB_JOB_RETENTION_DAYS` | `30` | Shared terminal import/export job-history retention. |

Import, export, backup, FTS, and admin job workers run automatically with bounded internal polling and profile-based batches.

### Legacy Aliases

| Config Name | Default | Description |
|---|---:|---|
| `KONGODB_ENABLE_LEGACY_ALIASES` | `false` | Enable migration-oriented request/response alias mapping. |
| `KONGODB_LEGACY_ALIASES_IMPORT_PK` | `_key:_id` | Import/write primary-key aliases. |
| `KONGODB_LEGACY_ALIASES_RESPONSE` | `_key:_id` | Response aliases added from canonical fields. |

## Quick Smoke
These are the main smoke scripts used to validate the service in local and s3-backed scenarios.

Run full smoke:
```bash
./scripts/smoke.sh
```

Auth smoke:
```bash
./scripts/smoke-auth.sh
```

S3 import smoke (requires running s3-mode server + AWS CLI + `KONGODB_S3_*`):
```bash
./scripts/smoke-import-s3.sh
```

Snapshot smoke (requires running s3-mode server):
```bash
./scripts/smoke-snapshot.sh
```

Safe hydrate anti-wipe smoke (requires running s3 mode server + AWS CLI):
```bash
./scripts/smoke-safe-hydrate.sh
```

## Complete Operation Examples
These examples show end-to-end request bodies for the most common operation families.

All examples use `db: \"myapp/main\"`. Add `X-Access-Key` header in real calls.

### Core CRUD Examples
These examples cover the main insert, update, read, aggregate, and search flows.

```json
{ "db":"myapp/main", "operation":"insert", "namespace":"users", "payload":{"data":{"name":"Ada"}} }
{ "db":"myapp/main", "operation":"insert", "namespace":"users", "payload":{"data":[{"name":"Ada"},{"name":"Bob"}],"unique_fields":["email"],"on_conflict":"skip"} }
{ "db":"myapp/main", "operation":"update", "namespace":"users", "payload":{"data":{"_id":"u1","name":"Ada L"}} }
{ "db":"myapp/main", "operation":"update", "namespace":"users", "payload":{"filter":{"_id":{"$in":["u1","u2"]}},"data":{"plan":"pro"}} }
{ "db":"myapp/main", "operation":"update", "namespace":"users", "payload":{"replace":true,"data":{"_id":"u1","name":"Ada","plan":"pro"}} }
{ "db":"myapp/main", "operation":"set", "namespace":"users", "payload":{"data":{"_id":"u1","name":"Ada"}} }
{ "db":"myapp/main", "operation":"upsert", "namespace":"users", "payload":{"filter":{"email":{"$eq":"a@b.com"}},"insert_data":{"email":"a@b.com"},"update_data":{"last_seen":{"$ts_now":true}}} }
{ "db":"myapp/main", "operation":"get", "namespace":"users", "payload":{"ids":["u1","u2"],"fields":["name","email"]} }
{ "db":"myapp/main", "operation":"count", "namespace":"users", "payload":{"filter":{"status":{"$eq":"active"}}} }
{ "db":"myapp/main", "operation":"query", "namespace":"users", "payload":{"filter":{"age":{"$gte":18}},"sort":"age desc","limit":20} }
{ "db":"myapp/main", "operation":"aggregate", "namespace":"users", "payload":{"compute":{"total":{"$count":"*"},"avg_age":{"$avg":"age"}}} }
{ "db":"myapp/main", "operation":"search", "namespace":"users", "payload":{"search":"ada","limit":10} }
{ "db":"myapp/main", "operation":"metrics_ingest", "payload":{"events":[{"event":"api.request","dimensions":{"endpoint":"/v1/chat","duration_ms":120}}]} }
{ "db":"myapp/main", "operation":"metrics_query", "payload":{"event":"api.request","range":"24h","interval":"hour","metrics":[{"op":"count","field":"*","alias":"requests","label":"Requests"}]} }
```

### Lifecycle/Archive Examples
These examples cover soft delete, namespace drop, TTL, restore, purge, and namespace changes.

```json
{ "db":"myapp/main", "operation":"delete", "payload":{"id":"u1"} }
{ "db":"myapp/main", "operation":"delete", "namespace":"users", "payload":{"filter":{"status":{"$eq":"inactive"}},"max_docs":100} }
{ "db":"myapp/main", "operation":"delete", "payload":{"ids":["u1","u2"]} }
{ "db":"myapp/main", "operation":"drop_namespace", "namespace":"users", "payload":{"ttl_seconds":3600} }
{ "db":"myapp/main", "operation":"set_ttl", "namespace":"users", "payload":{"ids":["u1"],"ttl_seconds":600,"expiry_behavior":"archive"} }
{ "db":"myapp/main", "operation":"restore_archive", "payload":{"txn_id":"tx123","on_conflict":"skip"} }
{ "db":"myapp/main", "operation":"purge_archive", "payload":{"txn_id":"tx123"} }
{ "db":"myapp/main", "operation":"change_namespace", "payload":{"from_namespace":"users","to_namespace":"users___kdb_archived","filter":{"status":{"$eq":"inactive"}}} }
```

### Stats/System/Indexes/FTS Examples
These examples cover stats reads, system config, indexing, and FTS controls.

```json
{ "db":"myapp/main", "operation":"get_stats", "namespace":"users", "payload":{} }
{ "db":"myapp/main", "operation":"get_system_config", "payload":{} }
{ "db":"myapp/main", "operation":"recompute_stats", "payload":{} }
{ "db":"myapp/main", "operation":"list_namespaces", "payload":{} }
{ "db":"myapp/main", "operation":"create_index", "payload":{"index_path":"profile.email"} }
{ "db":"myapp/main", "operation":"drop_index", "payload":{"index_path":"profile.email"} }
{ "db":"myapp/main", "operation":"list_indexes", "payload":{} }
{ "db":"myapp/main", "operation":"enable_fts_index", "payload":{"enable":true} }
{ "db":"myapp/main", "operation":"reindex_fts", "payload":{} }
{ "db":"myapp/main", "operation":"drop_fts_index", "payload":{} }
```

### Database Operations Examples
These examples cover DB creation, replication, backup, snapshot, and maintenance commands.

```json
{ "db":"myapp/main", "operation":"create_db", "payload":{} }
{ "db":"myapp/main", "operation":"db_exists", "payload":{} }
{ "operation":"list_commands", "payload":{} }
{ "operation":"list_dbs", "payload":{} }
{ "operation":"list_all_dbs", "payload":{} }
{ "operation":"system_memory", "payload":{} }
{ "db":"myapp/main", "operation":"load_db", "payload":{} }
{ "db":"myapp/main", "operation":"list_tables", "payload":{} }
{ "db":"myapp/main", "operation":"sync_db", "payload":{} }
{ "db":"myapp/main", "operation":"create_snapshot", "payload":{} }
{ "db":"myapp/main", "operation":"list_snapshots", "payload":{} }
{ "db":"myapp/main", "operation":"get_sync_status", "payload":{} }
{ "db":"myapp/main", "operation":"verify_db", "payload":{} }
{ "db":"myapp/main", "operation":"restore_snapshot", "payload":{"snapshot_id":"20260304T010203Z"} }
{ "db":"myapp/main", "operation":"compact_wal", "payload":{"retain_segments":500} }
{ "db":"myapp/main", "operation":"clone_db", "payload":{"to_db_path":"myapp/main_clone"} }
{ "db":"myapp/main", "operation":"create_backup", "payload":{"backup_tag":"nightly"} }
{ "db":"myapp/main", "operation":"restore_backup", "payload":{"backup_tag":"nightly","latest":true} }
{ "db":"myapp/main", "operation":"list_backups", "payload":{"limit":20} }
{ "db":"myapp/main", "operation":"tag_backup", "payload":{"backup_id":"bkp_123","backup_tag":"gold"} }
{ "db":"myapp/main", "operation":"offload_db", "payload":{} }
{ "db":"myapp/main", "operation":"vacuum_db", "payload":{} }
{ "db":"myapp/main", "operation":"reap_db", "payload":{} }
```

### Import/Export/Jobs/Transaction Examples
These examples cover async jobs, direct SQL, and transactional request batches.

```json
{ "db":"myapp/main", "operation":"import_jsonl", "namespace":"users", "payload":{"source_path":"s3://bucket/path/users.jsonl.zst","on_conflict":"skip"} }
{ "db":"myapp/main", "operation":"export_jsonl", "namespace":"users", "payload":{"target_path":"s3://bucket/exports/users","compress":true} }
{ "db":"myapp/main", "operation":"get_job", "payload":{"job_id":"job_123"} }
{ "db":"myapp/main", "operation":"list_jobs", "payload":{"job_type":"import_jsonl","status":"failed"} }
{ "db":"myapp/main", "operation":"continue_job", "payload":{"job_id":"job_123"} }
{ "db":"myapp/main", "operation":"abort_job", "payload":{"job_id":"job_123"} }
{ "db":"myapp/main", "operation":"sql_execute", "payload":{"sql":"SELECT collection, COUNT(*) AS total FROM __kdb_documents WHERE collection = ? GROUP BY collection","params":["users"]} }
{ "db":"myapp/main", "operation":"transaction", "data":[{"operation":"insert","namespace":"users","payload":{"data":{"_id":"u1","name":"Ada"}}},{"operation":"update","namespace":"users","payload":{"data":{"_id":"u1","plan":"pro"}}}] }
```

### Shorthand Alias Examples
These examples show the optional `operation::namespace` shorthand supported at the request edge.

```json
{ "db":"test/db02.main", "operation":"query::users", "payload":{} }
{ "db":"test/db02.main", "operation":"query::*", "payload":{} }
{ "db":"test/db02.main", "operation":"query::users,admins,teams", "payload":{} }
{ "db":"test/db02.main", "operation":"search::users", "payload":{"search":"ada"} }
```

## Notes
These are final reminders about current behavior, reserved semantics, and implementation limits.

- All system timestamps are UTC.
- `group_by` exists in the payload but is not implemented yet.
- `search` only targets live documents.
- `namespace` is the canonical name; `collection` is an alias.
