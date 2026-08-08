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
These are the HTTP endpoints exposed by Kongo under the configured base path.
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
  "payload": {
    "filter": {"status": "active"}
  }
}
```

Use `namespaces:["users","admins"]` instead of `namespace` when an operation supports a multi-namespace read. The two selectors are mutually exclusive.

### Rules
These rules explain how request fields are normalized and validated before dispatch.
- `db` is required for database-scoped operations. Commands explicitly marked global, such as instance inventory and system runtime statistics, may omit it.
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
  - Required: `insert`, `query`.
  - For `insert` and upsert-insert paths, namespace must be a single concrete value (no `namespace="*"` and no `namespaces:[...]`).
  - ID-targeted `update` and `delete` allow namespace omission; if provided, it is strict.
  - Global ID reads use `query` with `namespace="*"` and an explicit `_id` or `_id.$in` filter.
- Filter/wide destructive ops require namespace unless explicit `scope=all` where supported (`update` filter mode, `delete` filter mode, `set_ttl` filter mode, namespace-level ops).
- Namespace wildcard alias:
  - `namespace: "*"` maps to `payload.scope: "all"` for operations that support `scope=all`.
  - It conflicts with `payload.scope: "collection"`.
- `_namespace` response field:
  - hidden by default
  - enabled globally with `KONGODB_RESPONSE_INCLUDE_NAMESPACE=true`
  - per-request override with `payload.include_namespace` (alias `payload.include_name`)
  - always auto-included for `query` when using `namespace="*"` or `namespaces:[...]`
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

## Payload Properties
This section summarizes reusable payload fields shared across multiple operations.

### Payload Shape
Use this table as the quick reference for common payload keys and their meanings.

| Field | Type | Description |
|---|---|---|
| `collection` | string | Alias of top-level `namespace` |
| `namespaces` | string[] | Multi-namespace selector (top-level alias via request normalization) |
| `search` | string | Optional FTS text for `query` (alias: `q`); when present, query uses live-document FTS5 |
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
| `drop_keys` | string[] | Import-time field paths to remove |
| `job_id` | string | Job selector for job operations |
| `job_type` | string | Optional job type filter/hint |
| `status` | string | Optional status filter for `list_jobs` |
| `on_conflict` | string | Conflict policy (op-specific) |
| `commit` | bool | Per-request write ack override: `true` committed, `false` accepted |
| `force_db` | bool | For `query` with an explicit `_id`/`_id.$in` filter, bypass pending accepted-write overlay and read only durable DB state |
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
| `attach_users` | bool | For `query`, side-load Identity users referenced by returned `_user_id` values |
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
| `password_hash` | string | App-generated password hash. Kongo never stores raw passwords |
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
| `ignore_input_id` | bool | Import: ignore `_id` and `id` from input; `_key` remains ordinary document data |
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
| `lifecycle` | object or object[] | Named scheduled conditional transitions for singular `insert`, explicit-ID `update`, and singular `upsert` |
| `update_data` | object | Upsert update payload |
| `insert_data` | object | Upsert/insert-if-absent insert payload |
| `expiry_behavior` | string | TTL behavior: `archive` or `delete` |
| `filter` | object | Filter expression composed from Filter Operators |
| `txn_id` | string | Archive transaction selector (mapped to `_txn_id`) |
| `snapshot_id` | string | Snapshot selector for `restore_snapshot` |
| `purge` | bool | Hard-delete flag for delete/drop operations |
| `ttl_seconds` | int | TTL seconds |
| `transition_id` | string | Durable lifecycle transition selector |
| `document_id` | string | Lifecycle document selector; `id` is also accepted |
| `at` | string | Lifecycle execution time as RFC3339 UTC; mutually exclusive with `after_seconds` |
| `after_seconds` | int | Positive lifecycle delay resolved when the scheduling write commits |
| `when` | object | Filter Operators condition evaluated against the current document at execution time |
| `update` | object | Lifecycle patch or Mutation Operators applied when its condition matches |
| `execute_at_from` | string | Inclusive RFC3339 lower bound for `list_transitions` |
| `execute_at_to` | string | Inclusive RFC3339 upper bound for `list_transitions` |
| `allow_system_timestamps` | bool | Allow input `_created_at`/`_modified_at` where supported |
| `include_system_timestamps` | bool | Export toggle for system timestamps |
| `include_namespace` | bool | Include `_namespace` in query response items (alias: `include_name`) |
| `compute` | object | Compute spec (`aggregate`/`query`) |
| `group_by` | string\\|array | Metric Events grouping fields; aggregate grouping is reserved |
| `lookups` | object | Lookup/join map |
| `lookup_depth_override` | int | Per-request lookup depth override |
| `sort` | object\\|string | Sort definition |
| `fields` | string[] | Include projection paths |
| `exclude_fields` | string[] | Exclude projection paths (`_id` and document `_user_id` are kept when present) |
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


The table above is exhaustive. The groups below explain how the most reusable properties relate to one another. Operation-specific sections remain authoritative when a field has specialized behavior.

### Datetime Values
These rules define the accepted timestamp format for system-managed date fields.

- Kongo system timestamps are UTC.
- Accepted datetime input format for system timestamp fields is RFC3339/ISO-8601 with timezone.
- Examples:
  - `2025-12-24T23:39:26Z`
  - `2025-12-24T23:39:26.873397+00:00`
- If `_created_at` is provided and `_modified_at` is omitted (where allowed), `_modified_at` is set to `_created_at`.

### Identity and Scope
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

### Conflict and Uniqueness
These fields control insert conflicts, import merge behavior, and restore conflict handling.
- `on_conflict`
  - `insert`: `skip|error`
  - `import_jsonl`: `error|skip|replace|merge`
  - `restore_archive`: `skip|replace|patch`
- `unique_fields: string[]` (insert family soft uniqueness, dot paths supported)

### TTL, Archive, and Purge
These fields control expiry behavior, archive retention, and hard-delete semantics.
- `ttl_seconds: int`
- `expiry_behavior: "archive"|"delete"`
- `purge: bool`
- `txn_id: string` (maps to archive `_txn_id`)

### Query and Read Controls
These fields shape reads with filtering, projection, archive scope, and caching.
- `filter: object` composed from Filter Operators
- `sort: object|string`
- `limit: int`
- `offset: int`
- `fields: string[]`
- `exclude_fields: string[]`
- `include_archive: bool`
- `archive_only: bool`
- `explain: bool`
- `cache: bool|int`

### Compute Operators and Aggregate
These fields define Compute Operators for aggregate and query responses.
- `compute: object`
- `group_by: string[]` (reserved; currently not implemented)

### Lookup Operators and Joins
These fields configure lookup expansion during `query`, including FTS query mode.
- `lookups: object`
- `lookup_depth_override: int`

### Full-Text Search
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
- `drop_keys: string[]`


## Operations Cheatsheet

Use this catalog to find an operation by task. The detailed reference follows in the same order.

### Document Data

| Operation | Required input | Purpose |
|---|---|---|
| `insert` | `namespace`, `payload.data` | Create one or many documents; optionally apply TTL, identity reference, generated values, and soft uniqueness. |
| `update` | Explicit `_id` data or `filter + data` | Patch existing documents, apply Mutation Operators, or replace one known document. Never inserts. |
| `upsert` | `namespace`, `filter`, `insert_data` | Update filter matches or insert one document when none exist. |
| `count` | `namespace` or `scope:"all"` | Return only the number of matching live/archive documents. |
| `query` | `namespace`, `namespaces`, or `namespace:"*"` | Return documents with Filter Operators, pagination, sorting, projection, FTS, lookups, compute, and attachments. |
| `aggregate` | `compute`, namespace or all scope | Compute set-level counts, sums, averages, extrema, and distinct values. |
| `delete` | Exactly one of `id`, `ids`, `filter` | Soft-delete into archive by default or hard-delete with `purge:true`. |
| `set_ttl` | `ids` or `filter`, plus `ttl_seconds` | Schedule document expiration or clear an existing TTL. |
| `schedule_transition` | `document_id`, `name`, time, `when`, `update` | Create or replace a named scheduled conditional mutation. |
| `cancel_transition` | `transition_id` or `document_id + name` | Cancel one pending transition while retaining history. |
| `get_transition` | Transition selector | Inspect one transition and its execution state. |
| `list_transitions` | None | Filter and paginate lifecycle transition history. |
| `retry_transition` | Transition selector | Explicitly reopen a failed transition; failures do not auto-retry. |
| `import_jsonl` | `namespace`, `source_path` | Queue streaming/resumable JSONL ingestion from local storage or S3. |
| `export_jsonl` | Namespace or all scope | Queue filtered/projection-aware JSONL export to local storage or S3. |
| `transaction` | Top-level `data[]` | Atomically run supported insert, update, and delete operations against one database. |

### Product Stores

| Operation | Required input | Purpose |
|---|---|---|
| `metrics_ingest` | `events[]` | Append application metric events. |
| `metrics_query` | Event selector, date range, `metrics[]` | Produce bucketed and grouped metric result sets. |
| `metrics_catalog` | None | Discover registered event names and dimension paths. |
| `audit_ingest` | `events[]` | Append immutable application audit events. |
| `audit_query` | None | Search and filter the audit timeline. |
| `user_create` | None | Create Identity user metadata; Kongo stores identity state but does not authenticate. |
| `user_get` | User selector | Fetch one user by ID, email, username, or provider identity. |
| `user_query` | None | Search and paginate users. |
| `user_get_details` | User selector | Fetch a user with providers, login methods, and recent lifecycle events. |
| `user_update` | `user_id` or `id` | Update profile and application metadata. |
| `user_update_status` | User selector and `status` | Change status immediately or schedule a future transition. |
| `user_delete` | User selector | Soft-delete a user or purge all related identity state. |
| `user_create_token` | User selector, `kind`, `token_hash` | Store an application-generated token hash with expiration and single/multi-token policy. |
| `user_link_provider` | User selector, `provider`, `provider_user_id` | Link an external identity provider. |
| `user_unlink_provider` | `provider`, `provider_user_id` | Remove an external provider link. |
| `file_create` | `storage_backend`, `storage_path` | Register file/object metadata without moving bytes. |
| `file_get` | `id` | Fetch one file metadata record. |
| `file_query` | None | Search and paginate file metadata. |
| `file_update` | `id` | Update mutable file metadata. |
| `file_delete` | `id` | Soft-delete or purge file metadata. |

### Namespace Lifecycle

| Operation | Required input | Purpose |
|---|---|---|
| `list_namespaces` | None | List namespaces and their statistics. |
| `get_stats` | `namespace` | Read live/archive counts and bytes for one namespace. |
| `recompute_stats` | None | Queue a full rebuild of namespace statistics. |
| `drop_namespace` | `namespace` | Archive all namespace documents or permanently purge them. |
| `restore_archive` | `txn_id`, `ids`, or namespace/filter | Restore archived documents with a conflict policy. |
| `purge_archive` | `txn_id`, `ids`, or namespace/filter | Permanently delete selected archive rows. |
| `change_namespace` | `from_namespace`, `to_namespace` | Move selected live documents to another namespace. |
| `rename_namespace` | `from_namespace`, `to_namespace` | Rename a namespace across live and archive data. |

### Database Lifecycle and Recovery

| Operation | Required input | Purpose |
|---|---|---|
| `create_db` | `db` | Explicitly initialize a database path. |
| `db_exists` | `db` | Check local and, in S3 mode, remote existence. |
| `load_db` | `db` | Hydrate and preload an S3-backed database. |
| `offload_db` | `db` | Sync, close, and remove an S3-backed local working copy. |
| `sync_db` | `db` | Force S3 WAL/snapshot/manifest synchronization. |
| `create_snapshot` | `db` | Documented alias of `sync_db`. |
| `list_snapshots` | `db` | List versioned snapshots. |
| `restore_snapshot` | `db`; optional `snapshot_id` | Hydrate from the latest or selected snapshot. |
| `get_sync_status` | `db` | Inspect local and remote synchronization state. |
| `verify_db` | `db` | Verify referenced remote manifest, snapshot, and segment objects. |
| `compact_wal` | `db` | Compact retained WAL segment metadata. |
| `clone_db` | `db`, `to_db_path` | Copy the current database to a new path. |
| `create_backup` | `db` | Queue a compressed backup. |
| `restore_backup` | One backup selector | Restore from path, ID, tag, timestamp, or latest. |
| `list_backups` | `db` | Browse the backup catalog. |
| `tag_backup` | Backup ID or path | Set or clear a human-readable backup tag. |
| `vacuum_db` | `db` | Queue SQLite compaction. |
| `reap_db` | `db` | Run TTL/archive lifecycle processing immediately. |

### Jobs

| Operation | Required input | Purpose |
|---|---|---|
| `get_job` | `job_id` | Inspect one unified background job. |
| `list_jobs` | None | Filter and paginate background jobs. |
| `continue_job` | `job_id` | Reopen supported failed/resumable work. |
| `abort_job` | `job_id` | Mark supported work terminal and release its lease. |

### SQL

| Operation | Required input | Purpose |
|---|---|---|
| `sql_execute` | `sql` | Execute one supported parameterized read, write, or limited DDL statement. |
| `sql_list_tables` | None | List user-created tables while hiding Kongo/SQLite internals. |
| `sql_get_table_schema` | `table` | Safely inspect a user table schema without exposing arbitrary PRAGMA. |

### System, Statistics, and Indexing

| Operation | Required input | Purpose |
|---|---|---|
| `list_commands` | None | Return the public gateway command names. |
| `list_dbs` | None | List databases currently loaded by this instance. |
| `list_all_dbs` | None | Discover all known local and remote databases. |
| `system_get_inventory` | None | Read cross-database inventory from `__kdb_system.db`. |
| `system_refresh_inventory` | None | Refresh system inventory from local/S3 discovery. |
| `system_get_db_status` | `db` | Combine live database status with its system-catalog record. |
| `system_snapshot_db_stats` | Optional `db` | Snapshot active-database statistics into the system catalog. |
| `system_query_db_stats` | None | Query system-catalog database history. |
| `system_list_db_events` | None | Query database lifecycle/error events. |
| `get_system_stats` | None | Read instance uptime, requests, latency, memory, queues, and rolling windows. |
| `system_memory` | None | Read the compatibility memory/write-queue view. |
| `cleanup_temp_artifacts` | None | Remove stale internal temporary files. |
| `get_system_config` | `db` | Read per-database internal configuration. |
| `get_db_stats` | `db` | Read current in-memory counters for one database. |
| `snapshot_db_stats` | `db` | Persist one per-database counter snapshot. |
| `query_db_stats` | `db` | Query persisted per-database statistics snapshots. |
| `create_index` | `index_path` | Create a manual JSON expression index. |
| `drop_index` | `index_name` or `index_path` | Remove a manual or derived index. |
| `list_indexes` | None | List document-table indexes. |
| `enable_fts_index` | None | Toggle database-level FTS access. |
| `reindex_fts` | None | Queue FTS table creation/rebuild and backfill. |
| `drop_fts_index` | None | Queue FTS table and trigger removal. |
## Operators Cheatsheet
This section summarizes Filter Operators, Compute Operators, Generator Operators, Mutation Operators, and the relationship-specific Lookup Match Operators.

### Filter Operators
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

### Generator Operators (`data`/`insert_data`/`update_data`)
Use Generator Operators to create values during writes.

| Operation | Required Field | Description |
|---|---|---|
| `Generate value` | Exact single-key operator object | `$ts_now`, `$ts_now_ms`, `$id_uuidv4`, `$id_uuidv7`, `$id_random`, `$hash_value` |

### Mutation Operators (`update` data)
Use Mutation Operators to transform existing document values.

| Operation | Required Field | Description |
|---|---|---|
| `Mutate value` | Exact single-key operator object | `$unset`, `$inc`, `$push`, `$pop`, `$extend`, `$pull`, `$addset` |

### Lookup Match Operators (`payload.lookups`)
Use Lookup Match Operators to describe the direction of a document relationship.

| Operation | Required Field | Description |
|---|---|---|
| `$eq` | Scalar local and foreign paths | Scalar-to-scalar equality |
| `$in` | Local array path and foreign scalar path | Current array contains the foreign value |
| `$contains` | Local scalar path and foreign array path | Foreign array contains the current value |
| `$overlap` | Local and foreign array paths | The arrays share at least one value |

## Filter Operators Cheatsheet
These examples show the most common filtering patterns used with read operations.

| Operation | Required Field | Description |
|---|---|---|
| `Equality + range` | field paths + scalar/range values | `{ \"status\": {\"$eq\":\"active\"}, \"age\": {\"$gte\":18, \"$lte\":65} }` |
| `Boolean logic` | `$and/$or` arrays | Combine filters with nested logical groups |
| `Array matching` | array fields | Use `$all`, `$any`, `$none`, `$elemMatch` for collection semantics |
| `String matching` | string fields | Use `$icontains`, `$startsWith`, `$regex`, etc. |
| `Nested path + exists/type` | dot paths | Example: `profile.age`, `profile.phone`, `profile.meta` with `$between/$exists/$type` |

## Operation Reference

The reference is arranged by developer workflow. Every operation lists its purpose, valid request shape, relevant options, and examples. Fields described as top-level belong beside `db` and `operation`; all others belong inside `payload`.

---

### 1) Document Data Operations

These operations are the primary API for storing and retrieving JSON documents. They all operate on the database named by top-level `db`. Unless an operation explicitly supports global scope, it also operates on one concrete top-level `namespace`.

Use the operations in this order when learning the API:

1. `insert` creates documents without reading existing data.
2. `update` changes documents that already exist.
3. `upsert` chooses between update and insert using a filter.
4. `count` returns only the number of matching documents.
5. `query` returns documents and supports FTS, pagination, projection, lookups, and per-row computation.
6. `aggregate` computes set-level values without returning the matching documents.
7. `delete` soft-deletes or permanently purges selected documents.
8. `set_ttl` schedules or clears future document expiration.
9. Document lifecycle operations schedule, inspect, cancel, and explicitly retry conditional future mutations.
10. `import_jsonl` asynchronously ingests large JSONL files.
11. `export_jsonl` asynchronously writes selected documents to JSONL.
12. `transaction` atomically applies multiple supported document mutations to one database.

#### Shared Write Behavior

`insert`, `update`, `upsert`, `delete`, and `set_ttl` are mutations. Their common controls are:

| Property | Type | Default | Meaning |
|---|---:|---:|---|
| `commit` | bool | Runtime configuration | `true` waits for the per-database write coordinator to persist the mutation. `false` accepts and queues it. If the queue is unavailable, Kongo falls back to committed execution. |
| `dry_run` | bool | `false` | Validates and evaluates the target without changing data. The response reports the expected counts. |
| `max_docs` | int | Operation-specific | `-1` means all matches, `0` means no matched documents are changed, and a positive value caps the mutation. |

Committed mutation responses include `committed:true` and `is_async_ack:false`. Accepted responses include `committed:false`, `is_async_ack:true`, `ack_mode:"accepted"`, and `ack_status:"queued"`. Accepted `insert` and explicit-ID `update` requests return prepared documents immediately; filter-based mutations return an acknowledgement because their final targets are resolved by the write worker.

#### `insert`

Creates one or many new JSON documents in one namespace. Use `insert` when the request is inherently a create operation and existing documents should not be patched. It is also the only normal CRUD operation, besides `create_db` and `import_jsonl`, that may create a missing database.

##### Requirements

- Top-level `namespace` is required and must be one concrete namespace.
- `namespace:"*"` and `namespaces:[...]` are rejected.
- `payload.data` must be either one non-empty object or a non-empty array of objects.

##### Options

| Property | Type | Default | Description |
|---|---:|---:|---|
| `data` | object or object[] | Required | Document body or bodies. Every item must be an object. |
| `_user_id` | string | None | Stores an Identity user reference in the document table column rather than inside JSON `data`. It may also be supplied per document. |
| `ttl_seconds` | int | None | Positive lifetime for the new document. The reaper processes it after expiration. |
| `expiry_behavior` | string | `archive` | `archive` moves an expired document to the archive; `delete` removes it permanently. Unknown values normalize to `archive`. |
| `lifecycle` | object or object[] | None | On a single-document insert, atomically creates one or more named scheduled conditional transitions. |
| `allow_system_timestamps` | bool | `false` | Allows `_created_at` and `_modified_at` in input. Without it, those fields are reserved. If only `_created_at` is supplied, `_modified_at` uses the same value. |
| `unique_fields` | string[] | `[]` | Namespace-scoped soft uniqueness key. Multiple paths form one composite key. Dot paths are supported. |
| `on_conflict` | string | `skip` | With `unique_fields`, either `skip` conflicting input or return an `error`. |
| `commit` | bool | Runtime default | Selects committed or accepted acknowledgement. |
| `dry_run` | bool | `false` | Reports how many documents would be inserted or skipped. |

If `_id` is absent, Kongo generates a dashless UUIDv4. If `_id` is supplied, it must be a non-empty string. Generator Operators such as `$id_uuidv4` and `$ts_now` are expanded before persistence.

##### Insert One

```json
{
  "db": "myapp/main",
  "operation": "insert",
  "namespace": "users",
  "payload": {
    "data": {
      "email": "ada@example.com",
      "name": "Ada"
    }
  }
}
```

##### Insert Many

```json
{
  "db": "myapp/main",
  "operation": "insert",
  "namespace": "users",
  "payload": {
    "data": [
      {"email": "ada@example.com", "name": "Ada"},
      {"email": "grace@example.com", "name": "Grace"}
    ],
    "commit": false
  }
}
```

##### Insert With Identity Reference, TTL, and Generated Values

```json
{
  "db": "myapp/main",
  "operation": "insert",
  "namespace": "sessions",
  "payload": {
    "_user_id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001",
    "ttl_seconds": 7200,
    "expiry_behavior": "delete",
    "data": {
      "_id": {"$id_uuidv4": {"prefix": "session_"}},
      "created_by_app_at": {"$ts_now": true},
      "state": "active"
    }
  }
}
```

##### Insert With Composite Uniqueness

Use this form for idempotent creates based on an application key such as tenant plus email. This is not a database constraint; Kongo checks the composite values within the target namespace during insertion.

```json
{
  "db": "myapp/main",
  "operation": "insert",
  "namespace": "users",
  "payload": {
    "data": {
      "tenant": {"id": "tenant_01"},
      "profile": {"email": "ada@example.com"},
      "name": "Ada"
    },
    "unique_fields": ["tenant.id", "profile.email"],
    "on_conflict": "error"
  }
}
```

##### Response

```json
{
  "status": "success",
  "data": {
    "count": 1,
    "inserted_count": 1,
    "skipped_count": 0,
    "items": [
      {
        "_id": "7835cb6159234c49955326a93adade8f",
        "email": "ada@example.com",
        "name": "Ada",
        "_created_at": "2026-08-07T12:00:00.000Z",
        "_modified_at": "2026-08-07T12:00:00.000Z"
      }
    ]
  },
  "committed": true,
  "is_async_ack": false
}
```

#### `update`

Changes documents that already exist. Use it when the caller knows a document `_id`, has an array of explicit document IDs, or intentionally wants to patch every record matched by a filter. `update` never inserts a missing document.

By default, update data is a JSON merge patch: supplied fields are changed, untouched fields remain, and nested objects update nested values. Mutation Operators provide path-aware transformations for counters and arrays; their field keys may use dot notation.

##### Accepted Shapes

| Mode | Required input | Namespace rule | Typical use case |
|---|---|---|---|
| Single document | `data` object containing `_id` | Optional; strict when provided | Edit one known document. |
| Multiple explicit documents | `data` array; every object contains `_id` | Optional; strict when provided | Apply different patches to known documents. |
| Filter update | Non-empty `filter` plus one `data` object | Required unless `scope:"all"` | Apply one patch to matching documents. |

`payload.ids` is not accepted. For many explicit IDs, use `data:[...]`; for one shared patch, use `filter:{"_id":{"$in":[...]}}`.

##### Options

| Property | Type | Default | Description |
|---|---:|---:|---|
| `data` | object or object[] | Required | Patch object(s). Explicit-ID modes require `_id` on every object. |
| `filter` | object | None | Non-empty filter expression for shared-patch mode. It cannot be combined with an array. |
| `replace` | bool | `false` | Fully replaces one explicit-ID document while preserving `_id`. Rejected for arrays and filter mode. |
| `lifecycle` | object or object[] | None | Single explicit-ID mode only. Atomically creates or replaces named transitions after the update. Existing transitions with other names remain. |
| `max_docs` | int | All matches | Caps filter mode. `-1` all, `0` no changes, positive values cap changes. |
| `scope` | string | `collection` | Use `all` only for an intentional cross-namespace filter update. |
| `commit` | bool | Runtime default | Selects committed or accepted acknowledgement. |
| `dry_run` | bool | `false` | Validates the request and reports matched/update counts without writing. |

An explicit-ID update without a namespace searches globally by `_id`. Supplying a namespace ensures the document belongs to that namespace. Missing IDs are skipped, not created.

##### Patch One Document

```json
{
  "db": "myapp/main",
  "operation": "update",
  "namespace": "users",
  "payload": {
    "data": {
      "_id": "u1",
      "name": "Ada Lovelace",
      "profile": {"city": "London"}
    }
  }
}
```

##### Patch Multiple Explicit Documents

```json
{
  "db": "myapp/main",
  "operation": "update",
  "payload": {
    "data": [
      {"_id": "u1", "status": "active"},
      {"_id": "u2", "status": "inactive"}
    ],
    "commit": false
  }
}
```

##### Update by Filter

```json
{
  "db": "myapp/main",
  "operation": "update",
  "namespace": "users",
  "payload": {
    "filter": {
      "plan": "trial",
      "created_at": {"$lt": "2026-01-01T00:00:00Z"}
    },
    "data": {
      "plan": "expired"
    },
    "max_docs": 500,
    "dry_run": false
  }
}
```

##### Use Mutation Operators

```json
{
  "db": "myapp/main",
  "operation": "update",
  "namespace": "users",
  "payload": {
    "data": {
      "_id": "u1",
      "login_count": {"$inc": 1},
      "events": {"$push": {"type": "login"}},
      "roles": {"$addset": "editor"},
      "temporary_code": {"$unset": true}
    }
  }
}
```

##### Replace One Document

```json
{
  "db": "myapp/main",
  "operation": "update",
  "namespace": "users",
  "payload": {
    "replace": true,
    "data": {
      "_id": "u1",
      "name": "Ada",
      "plan": "pro"
    }
  }
}
```

#### `upsert`

Updates documents matched by a non-empty filter, or inserts one document when no match exists. Use it for synchronization and natural-key writes where the caller wants one operation to handle both existing and missing state.

`upsert` is intentionally singular on the insert path: `insert_data` and `update_data` are objects, not arrays. It is not a bulk-upsert operation. Use `transaction` or explicit application batching for unrelated upserts.

##### Options

| Property | Type | Default | Description |
|---|---:|---:|---|
| `filter` | object | Required | Non-empty filter expression used to find existing documents. |
| `insert_data` | object | Required | Non-empty document used only when the filter has no matches. `_id` is rejected here. |
| `update_data` | object | Required when `max_docs != 0` | Patch used only when matches exist. `_id` is rejected here. Optional for insert-if-absent mode. |
| `_user_id` | string | None | Identity reference stored on a newly inserted document. |
| `ttl_seconds` | int | None | Positive TTL applied only to the insert path. |
| `expiry_behavior` | string | `archive` | Expiry behavior applied only to the insert path. |
| `lifecycle` | object or object[] | None | Requires `max_docs:1`. Atomically schedules named transitions for the one updated or inserted document. |
| `max_docs` | int | `1` | Maximum existing matches to update. `0` updates none but still inserts when absent; `-1` updates all matches. |
| `commit` | bool | Runtime default | Selects committed or accepted acknowledgement. Accepted upserts return an acknowledgement rather than a prepared document. |
| `dry_run` | bool | `false` | Reports whether the operation would update or insert without writing. |

A literal `filter._id` or `filter._id.$eq` string becomes the inserted `_id` when no match exists. For all other filters, Kongo generates a dashless UUIDv4. `_id` remains prohibited inside both data objects to prevent contradictory identity inputs.

##### Update or Insert by Natural Key

```json
{
  "db": "myapp/main",
  "operation": "upsert",
  "namespace": "users",
  "payload": {
    "filter": {"email": "ada@example.com"},
    "insert_data": {
      "email": "ada@example.com",
      "name": "Ada",
      "login_count": 1
    },
    "update_data": {
      "last_seen": {"$ts_now": true},
      "login_count": {"$inc": 1}
    },
    "max_docs": 1
  }
}
```

##### Insert If Absent

With `max_docs:0`, existing matches remain unchanged and `update_data` is optional. A missing match is still inserted.

```json
{
  "db": "myapp/main",
  "operation": "upsert",
  "namespace": "settings",
  "payload": {
    "filter": {"key": "site_theme"},
    "insert_data": {"key": "site_theme", "value": "light"},
    "max_docs": 0
  }
}
```

##### Upsert a Known ID

```json
{
  "db": "myapp/main",
  "operation": "upsert",
  "namespace": "users",
  "payload": {
    "filter": {"_id": {"$eq": "user_external_123"}},
    "insert_data": {"name": "Ada"},
    "update_data": {"name": "Ada Lovelace"}
  }
}
```

#### `count`

Returns only the number of documents matched by namespace, filter, user scope, and archive source. Use `count` for totals, existence checks, dashboards, and pagination metadata when document bodies are unnecessary. It is cheaper and smaller than querying documents solely to count them.

##### Options

| Property | Type | Default | Description |
|---|---:|---:|---|
| `filter` | object | `{}` | Conditions composed from Filter Operators. |
| `_user_id` | string | None | Restricts results to the document-table user reference. |
| `scope` | string | `collection` | `collection` requires a namespace; `all` counts across namespaces. `namespace:"*"` normalizes to `all`. |
| `include_archive` | bool | `false` | Counts live and archived documents together. |
| `archive_only` | bool | `false` | Counts archived documents only. |
| `cache` | bool or int | Configured default | Controls read caching. See Cache Behavior. |

##### Count in One Namespace

```json
{
  "db": "myapp/main",
  "operation": "count",
  "namespace": "users",
  "payload": {
    "filter": {"status": "active"},
    "cache": true
  }
}
```

##### Count Across All Namespaces

```json
{
  "db": "myapp/main",
  "operation": "count",
  "namespace": "*",
  "payload": {
    "_user_id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001",
    "include_archive": true
  }
}
```

##### Response

```json
{
  "status": "success",
  "data": {"count": 42}
}
```

#### `query`

Returns matching documents. This is the general-purpose read operation and replaces separate get/search endpoints: use an `_id` filter for direct retrieval and `payload.search` for full-text search.

Use `query` when you need document bodies, pagination, sorting, field projection, nested lookups, per-row computed values, user attachments, archive reads, or FTS relevance.

##### Namespace Selection

| Selector | Behavior |
|---|---|
| `namespace:"users"` | Reads one namespace. |
| `namespaces:["users","admins"]` | Reads several namespaces and automatically includes `_namespace`. |
| `namespace:"*"` | Reads all namespaces and automatically includes `_namespace`. |
| `operation:"query::users"` | Shorthand for one namespace. |
| `operation:"query::users,admins"` | Shorthand for multiple namespaces. |
| `operation:"query::*"` | Shorthand for all namespaces. |

##### Options

| Property | Type | Default | Description |
|---|---:|---:|---|
| `filter` | object | `{}` | Filter expression. Use `_id`, `_id.$eq`, or `_id.$in` for direct ID retrieval. |
| `search` | string | None | Enables FTS5 over live documents. Alias: `q`. |
| `_user_id` | string | None | Restricts documents by their external user reference column. |
| `sort` | string or object | `_created_at DESC` | Ordered fields. String form supports comma-separated `path ASC|DESC`; missing direction means ascending. Dot paths are supported. |
| `limit` | int | Configured query limit | Page size for offset mode. |
| `offset` | int | `0` | Zero-based row offset. If `limit` or `offset` is supplied, offset mode takes precedence. |
| `page` | int | `1` | One-based page number when offset mode is not used. |
| `per_page` | int | Configured query limit | Page size used with `page`. |
| `fields` | string[] | All | Includes only selected paths. `_id` and `_user_id` are retained when present. |
| `exclude_fields` | string[] | `[]` | Removes selected paths after inclusion. `_id` and `_user_id` cannot be excluded. |
| `include_namespace` | bool | Configured response setting | Adds `_namespace` to items. Alias: `include_name`. Automatically enabled for multi/all namespace reads. |
| `include_archive` | bool | `false` | Reads live and archived documents. Not available in FTS mode. |
| `archive_only` | bool | `false` | Reads archived documents only. Not available in FTS mode. |
| `lookups` | object | None | Named lookup map for joining related documents. |
| `lookup_depth_override` | int | Configured maximum | Overrides lookup depth when uncapped lookups are enabled globally. |
| `compute` | object | None | Adds per-row computed fields after retrieval and lookups. |
| `attach_users` | bool | `false` | Side-loads Identity users referenced by returned `_user_id` values. |
| `attach_user_fields` | string[] | `id`, `first_name`, `last_name`, `profile_photo` | Selects top-level or nested `data.*` fields for user attachments. |
| `force_db` | bool | `false` | For exact `_id`/`_id.$in` reads, bypasses accepted-write pending state and reads durable rows only. |
| `explain` | bool | `false` | Returns the generated where SQL, bind count, and source instead of documents. |
| `cache` | bool or int | Configured default | Uses, bypasses, customizes, or invalidates read cache. |

##### Query With Filter, Sort, Projection, and Pagination

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "users",
  "payload": {
    "filter": {
      "status": "active",
      "profile.age": {"$gte": 18}
    },
    "sort": "profile.age desc, name asc",
    "fields": ["_id", "name", "profile.age", "status"],
    "exclude_fields": ["internal_notes"],
    "page": 2,
    "per_page": 25,
    "cache": true
  }
}
```

##### Retrieve IDs Globally and Include Namespace

Exact ID reads overlay pending accepted inserts and explicit-ID updates by default. Set `force_db:true` when the caller must observe only durable SQLite state.

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "*",
  "payload": {
    "filter": {
      "_id": {"$in": ["u1", "u2", "u3"]}
    },
    "force_db": false
  }
}
```

##### Full-Text Query

FTS mode requires `enable_fts_index` to be true for the database and a populated index created by `reindex_fts`. It searches live documents only. The default order is `_search_score ASC, _created_at DESC`; lower BM25 scores rank first.

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "articles",
  "payload": {
    "search": "sqlite AND json",
    "filter": {"status": "published"},
    "sort": "_search_score asc, _created_at desc",
    "fields": ["_id", "title", "summary", "_search_score"],
    "page": 1,
    "per_page": 20
  }
}
```

`_search_score` may only be used as a sort path during FTS mode. Archive flags are rejected in FTS mode.

##### Query With User Attachments

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "orders",
  "payload": {
    "filter": {"status": "paid"},
    "attach_users": true,
    "attach_user_fields": [
      "id",
      "first_name",
      "last_name",
      "profile_photo",
      "data.display_name"
    ]
  }
}
```

The response de-duplicates users into an attachment map:

```json
{
  "status": "success",
  "data": {
    "count": 1,
    "total_items": 1,
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
          "data": {"display_name": "Ada"}
        }
      }
    }
  }
}
```

##### Pagination Response

```json
{
  "status": "success",
  "data": {
    "count": 25,
    "total_items": 84,
    "items": [],
    "limit": 25,
    "offset": 25,
    "next_offset": 50,
    "prev_offset": 0,
    "pagination": {
      "total_items": 84,
      "count": 25,
      "per_page": 25,
      "page": 2,
      "total_pages": 4,
      "next_page": 3,
      "prev_page": 1
    }
  }
}
```

#### `aggregate`

Computes set-level values over all documents matched by a namespace and filter. Unlike `query.compute`, which computes a value independently for each returned item, `aggregate` summarizes the whole matched set and returns no document list.

Use it for totals, sums, averages, extrema, and distinct value lists without transferring every matching document to the application.

##### Options

| Property | Type | Default | Description |
|---|---:|---:|---|
| `compute` | object | Required | Named aggregate expressions using `$count`, `$sum`, `$avg`, `$min`, `$max`, or `$distinct`. |
| `filter` | object | `{}` | Filter expression applied before aggregation. |
| `scope` | string | `collection` | Use one namespace or `all`/`namespace:"*"`. |
| `include_archive` | bool | `false` | Aggregates live and archived documents. |
| `archive_only` | bool | `false` | Aggregates archived documents only. |
| `cache` | bool or int | Configured default | Controls result caching. |
| `group_by` | any | Unsupported | Reserved; requests currently fail when it is supplied. Use `metrics_query` for grouped time-series-style metrics. |

##### Example

```json
{
  "db": "myapp/main",
  "operation": "aggregate",
  "namespace": "orders",
  "payload": {
    "filter": {"status": "paid"},
    "compute": {
      "orders": {"$count": "*"},
      "revenue": {"$sum": "total"},
      "average_order": {"$avg": "total"},
      "smallest_order": {"$min": "total"},
      "largest_order": {"$max": "total"},
      "currencies": {"$distinct": "currency"}
    },
    "cache": 60
  }
}
```

##### Response

```json
{
  "status": "success",
  "data": {
    "matched_count": 125,
    "compute": {
      "orders": 125,
      "revenue": 18420.5,
      "average_order": 147.364,
      "smallest_order": 9.99,
      "largest_order": 1250,
      "currencies": ["USD", "CAD"]
    }
  }
}
```

#### `delete`

Removes one or many live documents. By default deletion is recoverable: Kongo copies each matched document to `__kdb_archive`, preserves its original timestamps and namespace, assigns one `_txn_id` to the operation, and then removes it from the live table. Use `purge:true` only when the data must be permanently removed without entering the archive.

##### Selectors

Exactly one selector is required:

| Selector | Namespace rule | Use case |
|---|---|---|
| `id` | Optional; strict if supplied | Delete one globally unique document ID. |
| `ids` | Optional; strict if supplied | Delete several explicit IDs. |
| `filter` | Required unless `scope:"all"` | Delete documents selected by Filter Operators. |

##### Options

| Property | Type | Default | Description |
|---|---:|---:|---|
| `id` | string | None | One explicit document ID. |
| `ids` | string[] | None | Explicit document IDs. Cannot be combined with `id` or `filter`. |
| `filter` | object | None | Non-empty filter expression. |
| `purge` | bool | `false` | `false` performs a soft delete; `true` permanently deletes live rows. |
| `ttl_seconds` | int | Configured delete TTL or none | Positive retention time for newly archived rows. Ignored with `purge:true`. |
| `max_docs` | int | All selected | Caps explicit IDs or filter matches. |
| `scope` | string | `collection` | `all` permits an intentional cross-namespace filter delete. |
| `commit` | bool | Runtime default | Selects committed or accepted acknowledgement. |
| `dry_run` | bool | `false` | Reports targets without deleting or archiving them. |

##### Soft-Delete One ID Globally

```json
{
  "db": "myapp/main",
  "operation": "delete",
  "payload": {
    "id": "u1"
  }
}
```

##### Soft-Delete Explicit IDs Strictly Within a Namespace

```json
{
  "db": "myapp/main",
  "operation": "delete",
  "namespace": "users",
  "payload": {
    "ids": ["u1", "u2"],
    "ttl_seconds": 604800
  }
}
```

##### Delete by Filter With a Safety Cap

```json
{
  "db": "myapp/main",
  "operation": "delete",
  "namespace": "sessions",
  "payload": {
    "filter": {
      "status": "expired",
      "last_seen": {"$lt": "2026-01-01T00:00:00Z"}
    },
    "max_docs": 1000,
    "dry_run": true
  }
}
```

##### Permanently Purge Live Documents

```json
{
  "db": "myapp/main",
  "operation": "delete",
  "payload": {
    "ids": ["temporary-1", "temporary-2"],
    "purge": true
  }
}
```

A successful soft delete returns `_txn_id`, which can later be passed to `restore_archive` or `purge_archive`.

#### `set_ttl`

Schedules selected live documents for future expiration or clears their existing expiration. Use it when retention is decided after insertion, such as expiring sessions, temporary exports, invitations, or stale application records.

When a document reaches `_expires_at`, the reaper uses `_expiry_behavior`: `archive` moves it to `__kdb_archive`; `delete` permanently removes it. This is different from `delete.ttl_seconds`, which controls how long an already soft-deleted archive row is retained.

##### Options

| Property | Type | Default | Description |
|---|---:|---:|---|
| `ids` | string[] | Selector | Explicit IDs. Namespace is optional and strict when supplied. |
| `filter` | object | Selector | Non-empty filter expression. Namespace is required unless `scope:"all"`. |
| `ttl_seconds` | int | Required | Positive seconds schedule expiration; `0` clears `_expires_at`. Negative values are rejected. |
| `expiry_behavior` | string | Existing value | Sets `archive` or `delete`; omitted leaves each document's behavior unchanged. Unknown supplied values normalize to `archive`. |
| `max_docs` | int | All selected | Caps selected IDs or filter matches. |
| `scope` | string | `collection` | `all` permits cross-namespace filter targeting. |
| `commit` | bool | Runtime default | Selects committed or accepted acknowledgement. |
| `dry_run` | bool | `false` | Reports selected rows without changing TTL. |

`ids` and `filter` are mutually exclusive. Unlike `delete`, `set_ttl` does not accept singular `id`; use `ids:["..."]` for one document.

##### Schedule Expiration

```json
{
  "db": "myapp/main",
  "operation": "set_ttl",
  "namespace": "sessions",
  "payload": {
    "ids": ["session_1", "session_2"],
    "ttl_seconds": 3600,
    "expiry_behavior": "delete"
  }
}
```

##### Schedule by Filter

```json
{
  "db": "myapp/main",
  "operation": "set_ttl",
  "namespace": "invitations",
  "payload": {
    "filter": {"status": "unused"},
    "ttl_seconds": 86400,
    "expiry_behavior": "archive",
    "max_docs": 500
  }
}
```

##### Clear TTL

```json
{
  "db": "myapp/main",
  "operation": "set_ttl",
  "payload": {
    "ids": ["session_1"],
    "ttl_seconds": 0
  }
}
```

#### Document Lifecycle Transitions

Document lifecycle transitions are durable, named, one-time conditional mutations. Use them when a document should change later only if its state still satisfies a condition, such as expiring an unaccepted invitation, timing out an unfinished job, publishing content, or assigning a TTL after a status change.

Lifecycle transitions and TTL serve different purposes:

- TTL archives or permanently deletes a document when `_expires_at` is reached.
- A lifecycle transition evaluates current document state at `execute_at` and conditionally updates fields.
- A transition may set `ttl_seconds` and `expiry_behavior`, which delegates later expiration to the normal TTL system.

##### Lifecycle Shape

`payload.lifecycle` accepts one object or a non-empty array. Every item requires a unique `name`, exactly one time selector, a `when` Filter Operators object, and a non-empty `update` object.

| Property | Type | Required | Description |
|---|---:|---:|---|
| `name` | string | Yes | Stable name scoped to the document. Scheduling the same document/name replaces that transition; different names coexist. |
| `at` | string | One time selector | Absolute RFC3339 datetime normalized to UTC. Mutually exclusive with `after_seconds`. |
| `after_seconds` | int | One time selector | Positive delay. The clock starts when the scheduling write commits, including accepted writes. |
| `when` | object | Yes | Filter Operators evaluated against the current document inside the serialized execution transaction. `{}` means always apply while the document exists. |
| `update` | object | Yes | JSON merge patch or Mutation Operators. `_id` cannot be changed. Generator Operators are resolved when execution occurs, not when scheduled. |
| `ttl_seconds` | int | No | `1+` assigns a TTL from execution time; `0` clears the existing TTL. |
| `expiry_behavior` | string | No | Optional `archive` or `delete` behavior applied with the transition. |

##### Attach to Insert

A singular insert can create the document and its transitions in the same SQLite transaction. Bulk `data:[...]` with lifecycle is rejected.

```json
{
  "db": "myapp/main",
  "operation": "insert",
  "namespace": "invitations",
  "payload": {
    "commit": true,
    "data": {
      "email": "user@example.com",
      "status": "pending"
    },
    "lifecycle": {
      "name": "expire_invitation",
      "after_seconds": 86400,
      "when": {
        "accepted_at": {"$exists": false},
        "status": "pending"
      },
      "update": {
        "status": "expired",
        "expired_at": {"$ts_now": true}
      }
    }
  }
}
```

##### Attach Multiple Transitions

An explicit-ID update may atomically alter a document and schedule multiple independent transitions. Filter updates and update arrays cannot carry lifecycle definitions.

```json
{
  "db": "myapp/main",
  "operation": "update",
  "namespace": "content",
  "payload": {
    "data": {
      "_id": "article_1",
      "status": "scheduled"
    },
    "lifecycle": [
      {
        "name": "publish",
        "at": "2026-08-10T14:00:00Z",
        "when": {"status": "scheduled"},
        "update": {"status": "published", "published_at": {"$ts_now": true}}
      },
      {
        "name": "expire",
        "at": "2026-09-10T14:00:00Z",
        "when": {"status": "published"},
        "update": {"status": "expired"},
        "ttl_seconds": 604800,
        "expiry_behavior": "archive"
      }
    ]
  }
}
```

`upsert` also accepts lifecycle when `max_docs` is exactly `1`. The transition is attached to the one updated or inserted document. `max_docs:0`, `-1`, and values greater than one are rejected when lifecycle is present.

Inside `transaction`, supported singular `insert` and explicit-ID `update` entries may also carry lifecycle definitions; their document mutation and transition rows commit or roll back together. A transactional soft delete cancels pending transitions for its document.

Committed write responses include scheduling metadata:

```json
{
  "status": "success",
  "data": {
    "items": [{"_id": "article_1", "status": "scheduled"}],
    "count": 1,
    "lifecycle": {
      "count": 2,
      "transition_ids": ["9f...", "42..."]
    }
  },
  "committed": true,
  "is_async_ack": false
}
```

Accepted writes return prepared document acknowledgement data but do not promise transition IDs before the queued write commits. Use `list_transitions` with the document ID or use committed mode when the IDs are needed immediately.

##### `schedule_transition`

Creates or replaces one named transition without otherwise modifying the document. `document_id` or `id` is required. Namespace is optional because document IDs are global; when supplied, it is a strict ownership check.

```json
{
  "db": "myapp/main",
  "operation": "schedule_transition",
  "namespace": "orders",
  "payload": {
    "document_id": "order_1",
    "name": "cancel_unpaid",
    "after_seconds": 1800,
    "when": {"payment.status": {"$ne": "paid"}},
    "update": {
      "status": "cancelled",
      "cancelled_at": {"$ts_now": true},
      "events": {"$push": {"type": "payment_timeout"}}
    }
  }
}
```

##### `get_transition` and `list_transitions`

Select one transition with `transition_id`, or with `document_id` plus `name`:

```json
{
  "db": "myapp/main",
  "operation": "get_transition",
  "payload": {
    "document_id": "order_1",
    "name": "cancel_unpaid"
  }
}
```

List operations support `document_id`, `namespace`, `name`, `status`, `execute_at_from`, `execute_at_to`, and standard `limit/offset` or `page/per_page` pagination:

```json
{
  "db": "myapp/main",
  "operation": "list_transitions",
  "payload": {
    "status": "failed",
    "execute_at_from": "2026-08-01T00:00:00Z",
    "execute_at_to": "2026-09-01T00:00:00Z",
    "page": 1,
    "per_page": 50
  }
}
```

Transition items expose `transition_id`, `document_id`, `namespace`, `name`, `execute_at`, `when`, `update`, TTL options, `status`, `attempts`, error/skip diagnostics, and timestamps.

##### `cancel_transition` and `retry_transition`

Cancellation only changes a `pending` transition and retains it as `cancelled` history:

```json
{
  "db": "myapp/main",
  "operation": "cancel_transition",
  "payload": {"transition_id": "transition_1"}
}
```

Execution failures remain `failed`; there is no automatic retry. Explicit retry changes only a failed transition back to `pending`. Omit the time selector to retry immediately, or provide a new `at` or positive `after_seconds`:

```json
{
  "db": "myapp/main",
  "operation": "retry_transition",
  "payload": {
    "transition_id": "transition_1",
    "after_seconds": 60
  }
}
```

##### Execution and Status Rules

The background reaper invokes a bounded lifecycle pass for active databases. Each database pass selects at most 100 due rows and submits execution through the per-database committed write coordinator. `reap_db` runs both TTL maintenance and due lifecycle transitions immediately.

| Status | Meaning |
|---|---|
| `pending` | Waiting for `execute_at`. |
| `running` | Claimed inside the serialized database transaction. |
| `completed` | Condition matched and the update committed. |
| `skipped` | Document was missing or `when` no longer matched; see `skipped_reason`. |
| `failed` | Evaluation or update failed; inspect `last_error` and use explicit retry if appropriate. |
| `cancelled` | Pending execution was intentionally disabled. |

Soft delete and TTL archive cancel pending transitions. Hard delete and archive purge permanently remove their transition rows. Restoring an archived document does not reactivate cancelled transitions. `change_namespace` and `rename_namespace` update the stored transition namespace. Replacing a document preserves existing transitions unless `payload.lifecycle` replaces the same document/name.

#### `import_jsonl`

Creates a background job that streams newline-delimited JSON into one namespace in bounded batches. Use it instead of `insert` for large files, resumable ingestion, S3 sources, or migrations that need conflict and field-cleanup policies.

The gateway records the job and returns immediately. Background workers claim it through `__kdb_jobs`, persist progress after each batch, and allow another worker to continue a resumable failed job from its recorded line/byte offset.

##### Requirements

- One concrete top-level `namespace` is required and may be created by the import.
- `source_path` is required.
- Every decompressed line must be one valid UTF-8 JSON object.
- Local `.jsonl`, local `.jsonl.zst`, `s3://...jsonl`, and `s3://...jsonl.zst` sources are supported.

##### Options

| Property | Type | Default | Description |
|---|---:|---:|---|
| `source_path` | string | Required | Local path or `s3://bucket/key`. Compression is detected from `.zst`. |
| `source_hash` | string | S3 metadata when present | Caller-provided source identity used for duplicate-job detection. For S3, Kongo also validates it against `x-amz-meta-source-hash`; local imports treat it as a caller-supplied identity. |
| `on_conflict` | string | `error` | `_id` conflict policy: `error`, `skip`, `replace`, or `merge`. |
| `ignore_input_id` | bool | `false` | Removes incoming `_id` and `id`, then generates a new `_id`. `_key` remains normal data. |
| `drop_keys` | string[] | `[]` | Removes named top-level or dot-path fields before persistence. `_id` cannot be dropped through this option. |
| `allow_system_timestamps` | bool | `false` | Accepts imported `_created_at` and `_modified_at`. If only `_created_at` exists, it is also used as `_modified_at`. |
| `batch_size` | int | `500` | Documents per committed import batch, clamped to `1..10000`. |
| `resumable` | bool | `false` | Allows a failed job to be reopened with `continue_job`. |

If a source hash matches an equivalent active or completed import for the same namespace and import options, Kongo returns the existing job with `deduped:true` instead of enqueuing a duplicate.

##### Import a Local File

```json
{
  "db": "myapp/main",
  "operation": "import_jsonl",
  "namespace": "users",
  "payload": {
    "source_path": "/data/imports/users.jsonl",
    "on_conflict": "merge",
    "batch_size": 1000,
    "resumable": true
  }
}
```

##### Import a Compressed S3 Migration

```json
{
  "db": "myapp/main",
  "operation": "import_jsonl",
  "namespace": "users",
  "payload": {
    "source_path": "s3://migration-bucket/exports/users.jsonl.zst",
    "source_hash": "upload-2026-08-07-users-v3",
    "on_conflict": "replace",
    "ignore_input_id": false,
    "drop_keys": ["legacy.password", "temporary_flag"],
    "allow_system_timestamps": true,
    "batch_size": 2000,
    "resumable": true
  }
}
```

##### Response and Job Follow-Up

```json
{
  "status": "success",
  "data": {
    "job_id": "17ed650141934293b15200810a0d83f3",
    "status": "queued",
    "collection": "users",
    "source_path": "/data/imports/users.jsonl",
    "on_conflict": "merge",
    "ignore_input_id": false,
    "allow_system_timestamps": false,
    "batch_size": 1000,
    "resumable": true
  }
}
```

Use `get_job` to inspect progress, `continue_job` to reopen a resumable failed import, and `abort_job` to make it terminal.

#### `export_jsonl`

Creates a background job that queries documents and writes newline-delimited JSON in bounded parts before finalizing one output object. Use it for data portability, migrations, analytics handoff, offline processing, and large downloads that should not block the gateway request.

##### Scope and Output

- Select one namespace with `namespace`, or all namespaces with `namespace:"*"`/`scope:"all"`.
- If `target_path` is omitted, Kongo generates a path under the configured export destination.
- A local target writes to local storage.
- An `s3://bucket/prefix/object` target uses configured S3 credentials.
- `compress:true` produces `.jsonl.zst`; `compress:false` produces `.jsonl`.

##### Options

| Property | Type | Default | Description |
|---|---:|---:|---|
| `target_path` | string | Generated | Local path or S3 URI. Kongo adds the canonical extension when needed. |
| `compress` | bool | `true` | Writes Zstandard-compressed JSONL when true. |
| `include_system_timestamps` | bool | `true` | Includes `_created_at` and `_modified_at` in each exported object. |
| `filter` | object | `{}` | Filter expression applied to the export source. |
| `sort` | string or object | `_created_at DESC` | Stable export order; dot paths are supported. |
| `limit` | int | Unlimited | Maximum documents to export. |
| `offset` | int | `0` | Starting offset. |
| `fields` | string[] | All | Include projection. `_id` is retained. |
| `exclude_fields` | string[] | `[]` | Exclude projection. `_id` is retained. |
| `include_archive` | bool | `false` | Exports live and archived documents. |
| `archive_only` | bool | `false` | Exports only archived documents. Cannot be combined with `include_archive:true`. |

`page` and `per_page` are accepted by the common payload shape, but export execution uses `limit` and `offset`; use those fields for deterministic exports.

##### Export One Namespace to the Configured Destination

```json
{
  "db": "myapp/main",
  "operation": "export_jsonl",
  "namespace": "users",
  "payload": {
    "filter": {"status": "active"},
    "sort": "_created_at asc",
    "fields": ["_id", "email", "name"],
    "compress": true,
    "include_system_timestamps": true
  }
}
```

##### Export Archive Data to S3

```json
{
  "db": "myapp/main",
  "operation": "export_jsonl",
  "namespace": "users",
  "payload": {
    "target_path": "s3://exports-bucket/kongo/users-archive",
    "archive_only": true,
    "sort": "_created_at asc",
    "limit": 1000000,
    "offset": 0,
    "exclude_fields": ["password", "ssn"],
    "compress": true
  }
}
```

##### Response

```json
{
  "status": "success",
  "data": {
    "job_id": "dd21d1525b4544c4b70916572dcb30ea",
    "status": "queued",
    "target_path": "/data/exports/20260807T120000Z__myapp_main.jsonl.zst",
    "compress": true,
    "include_system_timestamps": true
  }
}
```

Use the unified job operations to monitor, continue, or abort export work.

#### `transaction`

Applies a sequence of document mutations as one atomic database transaction. Use it when several related inserts, updates, or deletes must either all commit or all roll back. Nested operations may target different namespaces, but they always use the single database selected by the outer `db` field.

Unlike the other document operations, transaction entries are provided in top-level `data`, not `payload.data`.

##### Requirements

- Top-level `db` selects the database for every nested operation.
- Top-level `data` must be a non-empty array of operation envelopes.
- Supported nested operations are `insert`, `update`, and `delete`.
- Each nested operation supplies its own `namespace` and `payload` as required by that operation.
- A nested operation cannot override the outer database.
- If any nested operation fails, the entire transaction is rolled back.

##### Example

```json
{
  "db": "myapp/main",
  "operation": "transaction",
  "data": [
    {
      "operation": "insert",
      "namespace": "users",
      "payload": {
        "data": {"_id": "u1", "name": "Ada"}
      }
    },
    {
      "operation": "update",
      "namespace": "accounts",
      "payload": {
        "data": {"_id": "account-1", "owner_id": "u1"}
      }
    }
  ]
}
```

Use `transaction` for short, related mutation sets. Large ingestion remains better suited to `insert` with array data or the resumable `import_jsonl` job.

#### Document Operators and Query Shaping

The document API uses four named operator families:

| Family | Used in | Purpose |
|---|---|---|
| Filter Operators | `payload.filter` and lookup `filter` | Select documents by field values and logical conditions. |
| Compute Operators | `payload.compute` | Produce aggregate values or fields derived from each returned document. |
| Generator Operators | Write data objects | Generate timestamps, identifiers, and hashes before persistence. |
| Mutation Operators | `update` data objects | Transform existing numbers, arrays, and fields without replacing them manually. |

Lookup Match Operators are a smaller relationship-specific family used by `payload.lookups.*.match`. Projection is not an operator family: `fields` and `exclude_fields` shape the response after lookup and compute processing.

##### Filter Operators

A filter is an object whose ordinary keys are document field paths and whose `$` keys are Filter Operators. Dot notation addresses nested object fields. Multiple fields in one object are implicitly joined with AND.

```json
{
  "status": "active",
  "profile.age": {"$gte": 18}
}
```

The example above means `status` equals `active` **and** `profile.age` is at least `18`. Explicit equality with `$eq` is equivalent to a scalar value.

###### Logical Filter Operators

| Operator | Operand | Definition | Example |
|---|---|---|---|
| `$and` | Non-empty filter array | Every child filter must match. | `{"$and":[{"status":"active"},{"age":{"$gte":18}}]}` |
| `$or` | Non-empty filter array | At least one child filter must match. | `{"$or":[{"plan":"pro"},{"plan":"team"}]}` |
| `$nor` | Non-empty filter array | None of the child filters may match. | `{"$nor":[{"status":"banned"},{"status":"deleted"}]}` |
| `$not` | One filter object | Negates the nested filter. | `{"$not":{"profile.country":"US"}}` |

###### Comparison Filter Operators

| Operator | Operand | Definition | Example |
|---|---|---|---|
| `$eq` | Any scalar value | Field equals the operand. | `{"status":{"$eq":"active"}}` |
| `$ne` | Any scalar value | Field does not equal the operand. | `{"status":{"$ne":"deleted"}}` |
| `$gt` | Comparable scalar | Field is greater than the operand. | `{"score":{"$gt":100}}` |
| `$gte` | Comparable scalar | Field is greater than or equal to the operand. | `{"profile.age":{"$gte":18}}` |
| `$lt` | Comparable scalar | Field is less than the operand. | `{"price":{"$lt":50}}` |
| `$lte` | Comparable scalar | Field is less than or equal to the operand. | `{"attempts":{"$lte":3}}` |
| `$between` | Exactly two values | Field is inclusively between the lower and upper operands. | `{"profile.age":{"$between":[18,65]}}` |
| `$exists` | Boolean | `true` requires a non-null path; `false` requires a missing or null path. | `{"profile.phone":{"$exists":true}}` |

Multiple operators on one field are also implicitly joined with AND:

```json
{
  "profile.age": {
    "$gte": 18,
    "$lt": 65
  }
}
```

###### Membership and Array Filter Operators

| Operator | Operand | Definition | Example |
|---|---|---|---|
| `$in` | Non-empty array | Scalar field equals any operand value. | `{"status":{"$in":["active","trial"]}}` |
| `$nin` | Non-empty array | Scalar field equals none of the operand values. | `{"status":{"$nin":["deleted","banned"]}}` |
| `$includes` | One value | Array field contains the value. | `{"tags":{"$includes":"paid"}}` |
| `$nincludes` | One value | Array field does not contain the value. | `{"roles":{"$nincludes":"blocked"}}` |
| `$all` | Non-empty array | Array field contains every supplied value. | `{"tags":{"$all":["paid","beta"]}}` |
| `$any` | Non-empty array | Array field contains at least one supplied value. | `{"roles":{"$any":["admin","owner"]}}` |
| `$none` | Non-empty array | Array field contains none of the supplied values. | `{"flags":{"$none":["fraud","blocked"]}}` |
| `$elemMatch` | Filter object | At least one array element satisfies the complete nested filter. | `{"items":{"$elemMatch":{"sku":"A1","qty":{"$gte":2}}}}` |
| `$size` | Integer or comparison object | Array length equals or compares against the operand. | `{"roles":{"$size":{"$gte":2}}}` |

`$size` accepts an integer directly or `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, and `$lte`:

```json
{
  "members": {"$size": 3},
  "events": {"$size": {"$gt": 0}}
}
```

###### String Filter Operators

| Operator | Operand | Definition | Example |
|---|---|---|---|
| `$startsWith` | String | Prefix match using SQLite `LIKE`. | `{"email":{"$startsWith":"admin@"}}` |
| `$endsWith` | String | Suffix match using SQLite `LIKE`. | `{"email":{"$endsWith":"@example.com"}}` |
| `$contains` | String | Substring match using SQLite `LIKE`. | `{"title":{"$contains":"SQLite"}}` |
| `$ilike` | SQL LIKE pattern | Case-insensitive pattern match; `%` and `_` retain LIKE semantics. | `{"email":{"$ilike":"%@example.com"}}` |
| `$istartsWith` | String | Case-insensitive prefix match. | `{"name":{"$istartsWith":"ada"}}` |
| `$iendsWith` | String | Case-insensitive suffix match. | `{"filename":{"$iendsWith":".jsonl"}}` |
| `$icontains` | String | Case-insensitive substring match. | `{"title":{"$icontains":"database"}}` |
| `$regex` | Regex pattern string | Matches using SQLite's registered `REGEXP` function. | `{"code":{"$regex":"^[A-Z]{3}-[0-9]+$"}}` |

###### Type Filter Operator

| Operator | Operand | Definition | Example |
|---|---|---|---|
| `$type` | Type token | Requires the field to have the selected JSON type. | `{"profile":{"$type":"object"}}` |

Supported type tokens are `number`, `boolean`, `string`, `array`, `object`, `null`, `integer`, `real`, `text`, `true`, and `false`.

###### Complete Filter Request and Response

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "users",
  "payload": {
    "filter": {
      "$and": [
        {"status": {"$in": ["active", "trial"]}},
        {"profile.age": {"$between": [18, 65]}},
        {"roles": {"$any": ["admin", "owner"]}}
      ]
    },
    "sort": "profile.age asc",
    "limit": 2
  }
}
```

```json
{
  "status": "success",
  "data": {
    "count": 2,
    "total_items": 2,
    "items": [
      {"_id": "u1", "status": "active", "profile": {"age": 31}, "roles": ["admin"]},
      {"_id": "u2", "status": "trial", "profile": {"age": 44}, "roles": ["owner"]}
    ],
    "limit": 2,
    "offset": 0,
    "next_offset": null,
    "prev_offset": null,
    "pagination": {
      "total_items": 2,
      "count": 2,
      "per_page": 2,
      "page": 1,
      "total_pages": 1,
      "next_page": null,
      "prev_page": null
    }
  }
}
```

##### Sorting

Sorting accepts an object or a comma-separated string. Dot paths are supported, and a missing direction means ascending.

```json
{
  "sort": {
    "profile.age": -1,
    "name": 1
  }
}
```

```json
{
  "sort": "profile.age desc, name asc"
}
```

```json
{
  "sort": "first_name, last_name"
}
```

The last form is normalized to `first_name ASC, last_name ASC`. When `sort` is omitted, document queries default to `_created_at DESC`. FTS queries default to `_search_score ASC, _created_at DESC`.

##### Projection

Projection shapes returned documents without modifying stored data. It runs after lookups and per-row Compute Operators, so projected responses can include or remove lookup aliases and computed fields.

| Property | Behavior |
|---|---|
| `fields` | Starts with only the listed paths, then preserves `_id` and `_user_id` when present. The array must not be empty. |
| `exclude_fields` | Removes listed paths from the included or full document. The array must not be empty. |
| Both | `fields` is applied first, then `exclude_fields`. |
| Dot paths | Preserve the nested object structure rather than flattening keys. |
| Protected fields | `_id` and `_user_id` cannot be excluded. |

###### `fields`: Inclusion Projection

`fields` changes the response from "return the full document" to "return only these paths." It accepts a non-empty array of strings.

```json
{
  "fields": ["name", "profile.city", "settings.theme"]
}
```

Rules:

- `_id` and `_user_id` are added automatically when present, even if they are absent from `fields`.
- A missing selected path is ignored; it is not returned as `null`.
- Duplicate paths are harmless but unnecessary.
- Selecting an object path returns that complete object. Selecting one nested leaf rebuilds only the object structure needed for that leaf.
- An empty `fields:[]` array is rejected. Omit `fields` to return every available field.

For this source value:

```json
{
  "_id": "u1",
  "profile": {
    "city": "London",
    "country": "GB",
    "preferences": {"theme": "light", "density": "compact"}
  }
}
```

`"fields":["profile"]` returns the whole profile:

```json
{
  "_id": "u1",
  "profile": {
    "city": "London",
    "country": "GB",
    "preferences": {"theme": "light", "density": "compact"}
  }
}
```

`"fields":["profile.city","profile.preferences.theme"]` returns only those leaves while retaining nesting:

```json
{
  "_id": "u1",
  "profile": {
    "city": "London",
    "preferences": {"theme": "light"}
  }
}
```

###### `exclude_fields`: Exclusion Projection

`exclude_fields` starts from the document currently available to projection and removes the listed paths. It accepts a non-empty array of strings.

```json
{
  "exclude_fields": ["password", "security.ssn", "internal.notes"]
}
```

Rules:

- Missing exclusion paths are ignored.
- Excluding a parent object removes that entire object.
- Excluding a nested leaf leaves its sibling fields intact.
- `_id` and `_user_id` are restored after exclusion and therefore cannot be removed.
- An empty `exclude_fields:[]` array is rejected. Omit it when no fields need to be removed.

For this source value:

```json
{
  "_id": "u1",
  "profile": {"city": "London", "country": "GB"},
  "security": {"ssn": "hidden", "mfa": true}
}
```

`"exclude_fields":["profile.country","security.ssn"]` returns:

```json
{
  "_id": "u1",
  "profile": {"city": "London"},
  "security": {"mfa": true}
}
```

###### Combining `fields` and `exclude_fields`

When both properties are present, Kongo first builds the inclusion projection and then removes exclusions. This is useful when a broad object is convenient to include but a few nested fields must remain private.

```json
{
  "fields": ["name", "profile", "security"],
  "exclude_fields": ["profile.date_of_birth", "security.ssn"]
}
```

`exclude_fields` cannot restore a path omitted by `fields`; it only removes data from the inclusion result.

###### Nested Objects and Arrays

Projection dot notation traverses nested **objects**. Arrays are projected as complete values rather than element-by-element schemas.

Given:

```json
{
  "_id": "order-1",
  "items": [
    {"sku": "A1", "qty": 2, "cost": 10},
    {"sku": "B2", "qty": 1, "cost": 20}
  ]
}
```

Use `"fields":["items"]` to return the full array:

```json
{
  "_id": "order-1",
  "items": [
    {"sku": "A1", "qty": 2, "cost": 10},
    {"sku": "B2", "qty": 1, "cost": 20}
  ]
}
```

Projection paths such as `items[].sku` do not reshape every array element. If element-level shaping is required, store the shape directly, use a lookup whose related documents have their own `fields`, or transform the response in the application.

###### System and Query-Generated Fields

Projection runs after Kongo attaches query-visible metadata, lookups, and per-row Compute Operators.

| Field | Projection behavior |
|---|---|
| `_id` | Always preserved. |
| `_user_id` | Always preserved when the document has one. |
| `_created_at`, `_modified_at` | Ordinary selectable/excludable paths. They only exist when system timestamps are enabled. |
| `_namespace` | Ordinary selectable/excludable path. It is automatically attached for multi-namespace/all-namespace reads before projection. |
| `_search_score` | Ordinary selectable path created by FTS mode. Include it explicitly when using `fields`. |
| Lookup aliases | Available to `fields` and `exclude_fields` because lookups run first. |
| Computed names | Available to `fields` and `exclude_fields` because Compute Operators run first. |

Example FTS projection:

```json
{
  "search": "distributed storage",
  "fields": ["title", "summary", "_search_score"]
}
```

Returned item:

```json
{
  "_id": "article-1",
  "title": "Distributed Storage",
  "summary": "A practical overview",
  "_search_score": -2.741
}
```

###### Projection Processing Order

For document queries, response shaping occurs in this order:

1. Kongo retrieves and decorates the base documents with configured timestamps and namespace metadata.
2. Pending accepted-write state overlays exact-ID reads when applicable.
3. Lookup Operators attach related documents.
4. Per-row Compute Operators add derived fields.
5. `fields` creates the inclusion projection.
6. `exclude_fields` removes paths from that result.
7. `attach_users` builds the separate `data.attachments.users` map using `attach_user_fields`.

Top-level `fields` and `exclude_fields` do not project the user attachment map. Use `attach_user_fields` for attached Identity records.

###### Projection by Operation

| Context | Supported controls | Notes |
|---|---|---|
| `query` | `fields`, `exclude_fields` | Applies to every returned document. |
| FTS through `query` | `fields`, `exclude_fields` | Include `_search_score` explicitly when using inclusion projection. |
| `export_jsonl` | `fields`, `exclude_fields` | Shapes every exported record before JSONL encoding. |
| Individual lookup | `fields` | Shapes each foreign document under that alias; `exclude_fields` is not a lookup property. |
| `attach_users` | `attach_user_fields` | Uses its own attachment-specific projection. |

###### Include Only Selected Fields

Given this stored document:

```json
{
  "_id": "u1",
  "_user_id": "identity-1",
  "name": "Ada",
  "email": "ada@example.com",
  "profile": {"age": 36, "city": "London"},
  "password": "hidden"
}
```

Request:

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "users",
  "payload": {
    "fields": ["name", "profile.city"]
  }
}
```

Returned item:

```json
{
  "_id": "u1",
  "_user_id": "identity-1",
  "name": "Ada",
  "profile": {"city": "London"}
}
```

###### Exclude Sensitive Fields

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "users",
  "payload": {
    "exclude_fields": ["password", "profile.age"]
  }
}
```

Returned item:

```json
{
  "_id": "u1",
  "_user_id": "identity-1",
  "name": "Ada",
  "email": "ada@example.com",
  "profile": {"city": "London"}
}
```

###### Combine Include and Exclude Projection

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "users",
  "payload": {
    "fields": ["name", "email", "profile"],
    "exclude_fields": ["email", "profile.age"]
  }
}
```

Returned item:

```json
{
  "_id": "u1",
  "_user_id": "identity-1",
  "name": "Ada",
  "profile": {"city": "London"}
}
```

###### Project Lookup and Computed Fields

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "orders",
  "payload": {
    "lookups": {
      "customer": {
        "from": "users",
        "local_field": "customer_id",
        "foreign_field": "_id",
        "fields": ["_id", "name"]
      }
    },
    "compute": {
      "item_count": {"$size": "items[]"}
    },
    "fields": ["number", "customer", "item_count"]
  }
}
```

Returned item:

```json
{
  "_id": "order-1",
  "number": "INV-1001",
  "customer": {"_id": "u1", "name": "Ada"},
  "item_count": 3
}
```

Projection is available on `query`, FTS query mode, and `export_jsonl`. Lookup-level `fields` applies only to the related documents returned under that lookup alias.

##### Lookup Operators

Lookups enrich each query result with documents from another namespace in the same database. `payload.lookups` is an object map: each key is the unique response alias, and each value is a lookup specification.

`local_field` always reads from the current result context. `foreign_field` always reads from candidate documents in the `from` namespace.

###### Lookup Properties

| Property | Type | Default | Description |
|---|---:|---:|---|
| `from` | string | Required | Namespace containing related documents. |
| `local_field` | string | Required | Path resolved from the current, parent, root, or completed lookup context. Use `[]` to flatten an array. |
| `foreign_field` | string | Required | Path on documents in `from`. Use `[]` to flatten a foreign array. |
| `match` | string | `$eq` | Lookup Match Operator: `$eq`, `$in`, `$contains`, or `$overlap`. |
| `multi` | bool | `false` | `false` returns the first match or `null`; `true` returns an array. |
| `filter` | object | None | Additional Filter Operators applied to the foreign namespace before relationship matching. Context tokens are resolved per current document. |
| `fields` | string[] | All | Projection applied to each returned foreign document. |
| `sort` | string or object | `_created_at DESC` | Orders foreign candidates before first-match selection and limiting. |
| `limit` | int | Configured query limit | Caps matched documents for this alias. |
| `preserve_order` | bool | `false` | With `$in`, orders results according to values in `local_field`. |
| `dedupe` | bool | `true` | Removes duplicate related documents by `_id`. |
| `on_missing` | string | `null` | `null` attaches `null`; `empty` attaches `[]` for `multi:true`; `drop` removes the parent result. |
| `strict_path` | bool | `false` | Rejects the query when a referenced context path is missing instead of treating it as no match. |
| `cache_lookup` | bool | `true` | Reuses identical foreign candidate reads within the current request. This is request-local lookup caching. |
| `lookups` | object | None | Nested lookup map evaluated against matched foreign documents. |

###### `from`: Foreign Namespace

`from` names the namespace containing candidate related documents. Lookups stay inside the database selected by the outer request; they cannot join across database files.

```json
{
  "lookups": {
    "customer": {
      "from": "users",
      "local_field": "customer_id",
      "foreign_field": "_id"
    }
  }
}
```

Here the root query may read `orders`, while `customer` reads candidates from the `users` namespace in the same database.

###### `local_field`: Current-Side Values

`local_field` resolves values from the current result context. A plain path is equivalent to `$self.<path>`.

```json
{
  "local_field": "customer_id"
}
```

For arrays, add `[]` to flatten their values for matching:

```json
{
  "local_field": "favorite_books[]"
}
```

Context prefixes allow nested and dependency-aware paths:

```json
{
  "local_field": "$root.tenant_id"
}
```

```json
{
  "local_field": "$parent.vendor_id"
}
```

```json
{
  "local_field": "$lookup.items[].product_id"
}
```

###### `foreign_field`: Related-Side Values

`foreign_field` is always evaluated on candidate documents from `from`.

```json
{
  "foreign_field": "_id"
}
```

Nested foreign object paths use dot notation:

```json
{
  "foreign_field": "identity.external_id"
}
```

Flatten a foreign array when matching one local value against its members:

```json
{
  "foreign_field": "member_ids[]",
  "match": "$contains"
}
```

Candidates missing the foreign path simply do not match. `strict_path` applies to current-context paths and dynamic filter tokens, not to every candidate's foreign path.

###### `multi`: Response Cardinality

`multi:false` returns one object: the first relationship match after lookup sorting, or `null` when no match exists.

```json
{
  "customer": {"_id": "u1", "name": "Ada"}
}
```

`multi:true` returns an array, including an empty array when `on_missing:"empty"` is selected.

```json
{
  "books": [
    {"_id": "b1", "title": "SQLite Internals"},
    {"_id": "b2", "title": "Rust Services"}
  ]
}
```

Use `multi:false` for one-to-one or many-to-one relationships. Use `multi:true` for one-to-many and many-to-many relationships.

###### `filter`: Restricting Foreign Candidates

Lookup `filter` applies Filter Operators only to the `from` namespace. It is evaluated before relationship matching. Static values and context-derived values may be combined.

```json
{
  "lookups": {
    "current_membership": {
      "from": "memberships",
      "local_field": "_id",
      "foreign_field": "user_id",
      "multi": false,
      "filter": {
        "tenant_id": "$root.tenant_id",
        "status": {"$eq": "active"}
      }
    }
  }
}
```

Response item:

```json
{
  "_id": "u1",
  "tenant_id": "tenant-a",
  "current_membership": {
    "_id": "membership-1",
    "user_id": "u1",
    "tenant_id": "tenant-a",
    "status": "active"
  }
}
```

Any string filter value beginning with `$root.`, `$parent.`, `$self.`, or `$lookup.` is resolved from that context before the foreign query runs.

###### `fields`: Lookup-Level Projection

Lookup `fields` applies inclusion projection to each matched foreign document before it is attached. `_id` remains protected. Lookup candidate loading does not attach the document table's external `_user_id` column to related documents.

```json
{
  "lookups": {
    "customer": {
      "from": "users",
      "local_field": "customer_id",
      "foreign_field": "_id",
      "fields": ["_id", "name", "profile.avatar"]
    }
  }
}
```

Response field:

```json
{
  "customer": {
    "_id": "u1",
    "name": "Ada",
    "profile": {"avatar": "s3://avatars/ada.png"}
  }
}
```

The outer query can still include or exclude the complete `customer` alias afterward. Lookup specifications do not support `exclude_fields`; use an explicit `fields` allowlist for related documents.

Lookup-level projection is applied before that related document's nested `lookups` execute. Include every field needed by nested `local_field` paths:

```json
{
  "fields": ["_id", "name", "vendor_id"],
  "lookups": {
    "vendor": {
      "from": "vendors",
      "local_field": "vendor_id",
      "foreign_field": "_id"
    }
  }
}
```

Omitting `vendor_id` from this lookup-level `fields` array would leave the nested vendor lookup without its local value.

###### `sort` and `limit`: Choosing Related Results

`sort` orders foreign candidates before `multi:false` chooses its first match and before `limit` truncates a multi-result. It supports the same object and string syntax as query sorting.

```json
{
  "lookups": {
    "latest_logins": {
      "from": "login_events",
      "local_field": "_id",
      "foreign_field": "user_id",
      "multi": true,
      "sort": "created_at desc",
      "limit": 3
    }
  }
}
```

Response field:

```json
{
  "latest_logins": [
    {"_id": "login-9", "created_at": "2026-08-07T12:00:00Z"},
    {"_id": "login-8", "created_at": "2026-08-06T18:00:00Z"},
    {"_id": "login-7", "created_at": "2026-08-05T09:30:00Z"}
  ]
}
```

When omitted, `limit` uses `KONGODB_QUERY_DEFAULT_LIMIT`. Keep lookup limits bounded because the cap applies per root document and per lookup alias.

###### `preserve_order` and `dedupe`

`preserve_order:true` is meaningful for `$in`: it reorders matched foreign documents according to the flattened local values. It is useful for ordered ID lists such as favorites, playlists, or manually ranked content.

`dedupe:true` is the default and removes duplicate matched documents by `_id`. Set `dedupe:false` only when repeated lookup rows are intentionally meaningful. Documents without `_id` cannot be de-duplicated by this mechanism.

```json
{
  "local_field": "playlist_track_ids[]",
  "foreign_field": "_id",
  "match": "$in",
  "multi": true,
  "preserve_order": true,
  "dedupe": true
}
```

###### `on_missing`: No-Match Behavior

`on_missing` applies when the local path resolves no values or no foreign document matches.

| Value | `multi:false` | `multi:true` | Parent result |
|---|---|---|---|
| `null` | Alias is `null` | Alias is `null` | Kept |
| `empty` | Alias is `null` | Alias is `[]` | Kept |
| `drop` | Not returned | Not returned | Removed from query items |

Examples:

```json
{
  "customer": null
}
```

```json
{
  "books": []
}
```

With `on_missing:"drop"`, a root document without the relationship is removed after lookup processing. This behaves like a required relationship and can reduce `data.count` for the returned page even though `data.total_items` was calculated from the base query.

###### `strict_path`: Missing Context Validation

With `strict_path:false` (default), a missing local/context path produces no values and follows `on_missing`. With `strict_path:true`, a missing `local_field` or a missing context token used by lookup `filter` rejects the request.

```json
{
  "local_field": "$root.required_customer_id",
  "strict_path": true
}
```

Use strict paths when a missing relationship key indicates malformed data. Keep the default for optional relationships.

###### `cache_lookup`: Request-Local Reuse

With `cache_lookup:true` (default), identical candidate reads inside one query request are reused. The cache key includes the foreign namespace, foreign path, match mode, resolved local values, resolved filter, and sort definition.

This cache:

- exists only for the current request;
- is separate from `payload.cache`, which caches complete read responses;
- does not persist across requests or instances;
- is useful when many root rows resolve the same relationship values.

Set `cache_lookup:false` when each lookup execution must independently read candidates during the request.

###### `lookups`: Nested Relationships

Nested `lookups` run against each document matched by the containing lookup. Each nested alias is attached to that related document, not directly to the root item.

```json
{
  "lookups": {
    "items": {
      "from": "order_items",
      "local_field": "_id",
      "foreign_field": "order_id",
      "multi": true,
      "lookups": {
        "product": {
          "from": "products",
          "local_field": "product_id",
          "foreign_field": "_id",
          "multi": false,
          "lookups": {
            "vendor": {
              "from": "vendors",
              "local_field": "vendor_id",
              "foreign_field": "_id",
              "multi": false,
              "fields": ["_id", "name"]
            }
          }
        }
      }
    }
  }
}
```

Response item:

```json
{
  "_id": "order-1",
  "items": [
    {
      "_id": "line-1",
      "order_id": "order-1",
      "product_id": "product-9",
      "product": {
        "_id": "product-9",
        "vendor_id": "vendor-7",
        "vendor": {
          "_id": "vendor-7",
          "name": "Systems House"
        }
      }
    }
  ]
}
```

###### Lookup Depth

Depth counts nested lookup scopes, not the number of aliases or dependency edges:

| Depth | Example in the nested request above |
|---:|---|
| `1` | Root alias `items` |
| `2` | Nested alias `items.product` |
| `3` | Nested alias `items.product.vendor` |

The default `KONGODB_QUERY_LOOKUP_MAX_DEPTH=3` allows that complete example. A fourth nested scope is rejected by default.

Independent sibling aliases remain at the same depth. A forward dependency such as `vendors` reading `$lookup.books[]` also remains at the same nesting depth because it changes execution order, not structural nesting.

To request a different maximum for one query:

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "orders",
  "payload": {
    "lookup_depth_override": 5,
    "lookups": {
      "level_1": {
        "from": "one",
        "local_field": "one_id",
        "foreign_field": "_id",
        "lookups": {
          "level_2": {
            "from": "two",
            "local_field": "two_id",
            "foreign_field": "_id"
          }
        }
      }
    }
  }
}
```

Override rules:

- `lookup_depth_override` must be a positive integer.
- When `KONGODB_QUERY_LOOKUP_UNCAPPED_OVERRIDE_ENABLED=false`, the request value may lower the effective depth but cannot exceed `KONGODB_QUERY_LOOKUP_MAX_DEPTH`.
- When `KONGODB_QUERY_LOOKUP_UNCAPPED_OVERRIDE_ENABLED=true`, the request may set a higher finite depth.
- The overall request still remains subject to `KONGODB_OPERATION_TIMEOUT_MS`, lookup limits, and bounded lookup concurrency.
- Cycles and unknown `$lookup.<alias>` references are rejected regardless of depth settings.

###### Lookup Match Operators

| Operator | Typical direction | Meaning | Example paths |
|---|---|---|---|
| `$eq` | Scalar to scalar | Current value equals foreign value. | `customer_id` to `_id` |
| `$in` | Current array to foreign scalar | Foreign value occurs in the current values. | `favorite_books[]` to `_id` |
| `$contains` | Current scalar to foreign array | Foreign array contains the current value. | `skill_id` to `skill_ids[]` |
| `$overlap` | Current array to foreign array | At least one flattened value exists on both sides. | `tag_ids[]` to `tag_ids[]` |

The match names communicate relationship direction. Internally, the selected local and foreign paths are flattened as requested and compared for intersecting values.

###### Lookup Path Contexts

| Prefix | Resolves from | Typical use |
|---|---|---|
| No prefix or `$self.` | Current document at this lookup level | Join a row to its direct related data. |
| `$parent.` | Document that produced the current nested lookup row | Use an outer matched document inside a nested lookup. |
| `$root.` | Original root query document | Refer back to the root from any nested depth. |
| `$lookup.<alias>.` | A completed lookup alias in the same scope | Build a lookup from another lookup result, including forward references. |

Array traversal uses `[]`, for example `$lookup.items[].product_id`. Sibling aliases that do not depend on each other run concurrently. References create a dependency graph; Kongo topologically schedules them, permits forward references, and rejects unknown aliases or cycles.

###### One-to-One Lookup With `$eq`

Orders contain `customer_id`, while users expose `_id`:

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "orders",
  "payload": {
    "lookups": {
      "customer": {
        "from": "users",
        "local_field": "customer_id",
        "foreign_field": "_id",
        "match": "$eq",
        "multi": false,
        "fields": ["_id", "name", "email"]
      }
    }
  }
}
```

Response item:

```json
{
  "_id": "order-1",
  "customer_id": "u1",
  "total": 125,
  "customer": {
    "_id": "u1",
    "name": "Ada",
    "email": "ada@example.com"
  }
}
```

###### One-to-Many Lookup With `$in` and Preserved Order

A user stores book IDs in `favorite_books`. The `[]` suffix flattens that local array.

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "users",
  "payload": {
    "lookups": {
      "books": {
        "from": "books",
        "local_field": "favorite_books[]",
        "foreign_field": "_id",
        "match": "$in",
        "multi": true,
        "preserve_order": true,
        "dedupe": true,
        "fields": ["_id", "title"]
      }
    }
  }
}
```

Response item:

```json
{
  "_id": "u1",
  "favorite_books": ["b3", "b1"],
  "books": [
    {"_id": "b3", "title": "Distributed Systems"},
    {"_id": "b1", "title": "SQLite Internals"}
  ]
}
```

###### Foreign Array Lookup With `$contains`

The current document has one `skill_id`; each team has a `skill_ids` array.

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "profiles",
  "payload": {
    "lookups": {
      "matching_teams": {
        "from": "teams",
        "local_field": "skill_id",
        "foreign_field": "skill_ids[]",
        "match": "$contains",
        "multi": true,
        "fields": ["_id", "name", "skill_ids"]
      }
    }
  }
}
```

Response item:

```json
{
  "_id": "profile-1",
  "skill_id": "rust",
  "matching_teams": [
    {"_id": "team-1", "name": "Platform", "skill_ids": ["rust", "sql"]},
    {"_id": "team-3", "name": "Storage", "skill_ids": ["rust", "s3"]}
  ]
}
```

###### Array-to-Array Lookup With `$overlap`

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "articles",
  "payload": {
    "lookups": {
      "related": {
        "from": "articles",
        "local_field": "tag_ids[]",
        "foreign_field": "tag_ids[]",
        "match": "$overlap",
        "multi": true,
        "filter": {"status": "published"},
        "fields": ["_id", "title", "tag_ids"],
        "limit": 3
      }
    }
  }
}
```

Response item:

```json
{
  "_id": "article-1",
  "tag_ids": ["sqlite", "rust"],
  "related": [
    {"_id": "article-2", "title": "Fast Local Storage", "tag_ids": ["sqlite"]},
    {"_id": "article-3", "title": "Async Rust", "tag_ids": ["rust", "tokio"]}
  ]
}
```

An explicit lookup `filter` is the appropriate way to remove the current document from a self-lookup when the application has a suitable distinguishing field.

###### Nested Lookup With Root and Parent Context

This query resolves order items, then resolves each item's product. The nested product filter also requires the product tenant to equal the root order tenant.

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "orders",
  "payload": {
    "lookups": {
      "items": {
        "from": "order_items",
        "local_field": "_id",
        "foreign_field": "order_id",
        "match": "$eq",
        "multi": true,
        "lookups": {
          "product": {
            "from": "products",
            "local_field": "$self.product_id",
            "foreign_field": "_id",
            "match": "$eq",
            "filter": {
              "tenant_id": "$root.tenant_id",
              "vendor_id": "$parent.vendor_id"
            },
            "fields": ["_id", "name", "vendor_id"]
          }
        }
      }
    }
  }
}
```

Response item:

```json
{
  "_id": "order-1",
  "tenant_id": "tenant-a",
  "vendor_id": "vendor-7",
  "items": [
    {
      "_id": "line-1",
      "order_id": "order-1",
      "product_id": "product-9",
      "product": {
        "_id": "product-9",
        "name": "Storage Adapter",
        "vendor_id": "vendor-7"
      }
    }
  ]
}
```

At this nested level, `$self` is the order item, `$parent` is the root order that produced the items lookup, and `$root` is also the original order. At deeper levels, `$parent` and `$root` differ.

###### Forward Lookup Dependency

The alias order in the request does not control execution. Here `vendors` appears first but waits for `books` because its local path references `$lookup.books`.

```json
{
  "lookups": {
    "vendors": {
      "from": "vendors",
      "local_field": "$lookup.books[].vendor_id",
      "foreign_field": "_id",
      "match": "$in",
      "multi": true,
      "fields": ["_id", "name"]
    },
    "books": {
      "from": "books",
      "local_field": "favorite_books[]",
      "foreign_field": "_id",
      "match": "$in",
      "multi": true,
      "fields": ["_id", "title", "vendor_id"]
    }
  }
}
```

Response item:

```json
{
  "_id": "u1",
  "favorite_books": ["b1", "b2"],
  "books": [
    {"_id": "b1", "title": "SQLite Internals", "vendor_id": "v1"},
    {"_id": "b2", "title": "Rust Services", "vendor_id": "v2"}
  ],
  "vendors": [
    {"_id": "v1", "name": "Northwind Press"},
    {"_id": "v2", "name": "Systems House"}
  ]
}
```

##### Compute Operators

Compute Operators have two execution modes:

- `aggregate` applies them across all documents selected by the operation filter and returns one result object.
- `query` applies them independently to array/object/string values in each fetched document and adds the named values to that item before projection.

Each compute definition must contain exactly one primary Compute Operator. `$distinct:true` and `$filter:{...}` are modifiers rather than additional primary operators.

| Operator | Aggregate behavior | Per-row `query` behavior | Example definition |
|---|---|---|---|
| `$count` | Counts rows with `"*"` or non-null field values. | Counts values extracted from an array path. | `"total":{"$count":"*"}` or `"items_count":{"$count":"items[]"}` |
| `$sum` | Sums a numeric field across matching rows. | Sums numeric values in an array path. | `"revenue":{"$sum":"amount"}` |
| `$avg` | Averages a numeric field across matching rows. | Averages numeric values in an array path. | `"average":{"$avg":"scores[]"}` |
| `$min` | Returns the minimum numeric field value. | Returns the minimum numeric array value. | `"minimum":{"$min":"scores[]"}` |
| `$max` | Returns the maximum numeric field value. | Returns the maximum numeric array value. | `"maximum":{"$max":"scores[]"}` |
| `$distinct` | Returns unique values from a field or flattened `[]` path. | Returns unique values from an array path. | `"countries":{"$distinct":"country"}` |
| `$size` | Not supported by `aggregate`. | Returns array length, object key count, string character count, or `null` for unsupported/missing values. | `"item_count":{"$size":"items"}` |
| `$join` | Not supported by `aggregate`. | Concatenates literals and `$field.path` references into a string. | `"full_name":{"$join":["$first_name"," ","$last_name"]}` |

###### Compute Modifiers

| Modifier | Definition | Example |
|---|---|---|
| `$distinct:true` | De-duplicates values before `$count`, `$sum`, or `$avg`. | `{"$count":"country","$distinct":true}` |
| `$filter:{...}` | Applies a metric-local filter. `aggregate` accepts the complete Filter Operator set. Per-row `query` array filtering accepts `$and`, `$or`, direct equality, `$eq`, `$ne`, `$in`, and `$nin`. | `{"$count":"events[]","$filter":{"status":"ok"}}` |

###### Aggregate Compute Request and Response

```json
{
  "db": "myapp/main",
  "operation": "aggregate",
  "namespace": "orders",
  "payload": {
    "filter": {"status": "paid"},
    "compute": {
      "orders": {"$count": "*"},
      "revenue": {"$sum": "amount"},
      "average_order": {"$avg": "amount"},
      "customers": {"$count": "customer_id", "$distinct": true},
      "countries": {"$distinct": "shipping.country"}
    }
  }
}
```

```json
{
  "status": "success",
  "data": {
    "orders": 42,
    "revenue": 8200.5,
    "average_order": 195.25,
    "customers": 31,
    "countries": ["US", "CA", "GB"]
  }
}
```

###### Per-Row Compute Request and Response

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "users",
  "payload": {
    "compute": {
      "full_name": {"$join": ["$first_name", " ", "$last_name"]},
      "score_total": {"$sum": "scores[]"},
      "score_average": {"$avg": "scores[]"},
      "tag_count": {"$size": "tags"},
      "unique_tags": {"$distinct": "tags[]"}
    },
    "fields": ["first_name", "last_name", "full_name", "score_total", "score_average", "tag_count", "unique_tags"]
  }
}
```

Returned item:

```json
{
  "_id": "u1",
  "first_name": "Ada",
  "last_name": "Lovelace",
  "full_name": "Ada Lovelace",
  "score_total": 270,
  "score_average": 90,
  "tag_count": 3,
  "unique_tags": ["math", "systems"]
}
```

##### Generator Operators

Generator Operators are exact single-key objects embedded anywhere in `data`, `insert_data`, or `update_data`. Kongo resolves recognized operators before persistence. A normal document key that merely starts with `$` is not treated as a generator unless the complete single-key object matches a supported operator.

| Operator | Operand | Definition | Example |
|---|---|---|---|
| `$ts_now` | `true`, scalar, or shift object | Current UTC RFC3339 timestamp. A shift object accepts signed `days`, `hours`, `minutes`, and `seconds`. | `{"$ts_now":{"days":1,"minutes":30}}` |
| `$ts_now_ms` | `true`, scalar, or shift object | Current UTC Unix epoch milliseconds after applying the same optional shift. | `{"$ts_now_ms":{"seconds":-30}}` |
| `$id_uuidv4` | `true` or options object | Random UUIDv4. Options: `prefix`, `suffix`, `dash`; `dash` defaults to `false`. | `{"$id_uuidv4":{"prefix":"session:","dash":false}}` |
| `$id_uuidv7` | `true` or options object | Time-ordered UUIDv7 with the same options. | `{"$id_uuidv7":{"prefix":"evt_"}}` |
| `$id_random` | `true` or options object | Random hexadecimal identifier. Default length is `12`; options are `len` from `1` to `128`, `prefix`, and `suffix`. | `{"$id_random":{"len":8,"prefix":"tmp_"}}` |
| `$hash_value` | Options object | SHA-256 hash of required string `value`. Options: `algo:"sha256"`, `len` from `1` to `64`, `prefix`, and `suffix`. | `{"$hash_value":{"value":"Ada","len":12}}` |

Complete write:

```json
{
  "db": "myapp/main",
  "operation": "insert",
  "namespace": "sessions",
  "payload": {
    "data": {
      "_id": {"$id_uuidv4": {"prefix": "session:"}},
      "event_id": {"$id_uuidv7": true},
      "short_code": {"$id_random": {"len": 8, "prefix": "code_"}},
      "created_at": {"$ts_now": true},
      "expires_at": {"$ts_now": {"hours": 2}},
      "created_ms": {"$ts_now_ms": true},
      "email_hash": {"$hash_value": {"value": "ada@example.com", "len": 16}}
    }
  }
}
```

Representative returned item:

```json
{
  "_id": "session:550e8400e29b41d4a716446655440000",
  "event_id": "0198fc3d47b77a90b37cc77f7d7d40c1",
  "short_code": "code_9f31a72c",
  "created_at": "2026-08-07T15:00:00Z",
  "expires_at": "2026-08-07T17:00:00Z",
  "created_ms": 1786114800000,
  "email_hash": "b5fc85e55755f9e0"
}
```

Generated values vary per execution. The concrete values above illustrate output shape only.

##### Mutation Operators

Mutation Operators are intended for `update`. They operate on the current stored document and support dot-path keys such as `profile.login_count`. Each operator object must be the complete value assigned to that path.

| Operator | Operand | Behavior | Example |
|---|---|---|---|
| `$unset` | `true` | Removes the field completely. Other operands are ignored in permissive mode. | `"profile.legacy":{"$unset":true}` |
| `$inc` | `true` or signed number | Adds the operand; `true` means `1`. Missing or null fields start at the delta. | `"score":{"$inc":-2}` |
| `$push` | Any value | Appends one value to an array. Missing or null fields become arrays. | `"events":{"$push":{"type":"login"}}` |
| `$pop` | `true`, `1`, or `-1` | Removes the last item for `true`/`1`, or the first item for `-1`. | `"queue":{"$pop":-1}` |
| `$extend` | Array | Appends all operand items to the target array. | `"tags":{"$extend":["paid","beta"]}` |
| `$pull` | One value or array | Removes every target item equal to any operand value. | `"tags":{"$pull":["old","blocked"]}` |
| `$addset` | One value or array | Appends only values not already present by JSON equality. | `"roles":{"$addset":"editor"}` |

Given this document:

```json
{
  "_id": "u1",
  "score": 10,
  "tags": ["new", "beta"],
  "events": [],
  "profile": {"legacy": true}
}
```

Request:

```json
{
  "db": "myapp/main",
  "operation": "update",
  "namespace": "users",
  "payload": {
    "data": {
      "_id": "u1",
      "score": {"$inc": 5},
      "tags": {"$addset": ["beta", "paid"]},
      "events": {"$push": {"type": "login"}},
      "profile.legacy": {"$unset": true}
    }
  }
}
```

Returned updated item:

```json
{
  "_id": "u1",
  "score": 15,
  "tags": ["new", "beta", "paid"],
  "events": [{"type": "login"}],
  "profile": {}
}
```

Mutation strictness is controlled by `KONGODB_STRICT_MUTATIONS_OPERATORS`:

- `false` (default): an unknown Mutation Operator, most invalid operands, or an incompatible existing field type leaves that field unchanged rather than failing the entire update. Array operators initialize missing and null targets as arrays. An unrecognized `$pop` operand is normalized to an end-pop.
- `true`: unknown operators, invalid operands, and incompatible target types reject the request.

---

### 2) Metrics Events
#### `metrics_ingest`
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

#### `metrics_query`
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

#### `metrics_catalog`
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

### 3) Audit Logs
Audit Logs store application-supplied activity as immutable rows. Kongo does not infer an actor from the access key or automatically audit every gateway request; the application explicitly records the events that carry useful business context.

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

#### `audit_ingest`
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

#### `audit_query`
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

### 4) Identity Store
Identity operations store login-related metadata for your app. Kongo does not authenticate users, verify passwords, validate OAuth tokens, issue sessions, or enforce app permissions.

Internal tables:
- `__kdb_identity_users`: local user/account metadata.
- `__kdb_identity_providers`: Google/GitHub/custom provider mappings.
- `__kdb_identity_tokens`: app-generated token hashes.
- `__kdb_identity_events`: append-only identity lifecycle events.

Rules:
- User ids default to dashless UUID v4.
- `user_create` may accept caller-provided `user_id`; it must be a 32-character dashless UUID string.
- `first_name`, `last_name`, and `profile_photo` are first-class profile columns.
- `requires_password_change` is an application-facing account signal; Kongo stores and returns it but does not enforce login behavior.
- Presentation preferences such as `display_name`, `timezone`, and `locale` should live in `data`.
- Store `password_hash`, never raw passwords.
- Store `token_hash`, never raw reset/magic/API tokens.
- Status values are app-defined strings.
- Soft-deleted users keep email/provider identity reserved.
- `purge=true` hard-deletes the user, providers, tokens, and events.

#### `user_create`
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

#### `user_get`
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

#### `user_get_details`

Fetches one Identity user together with linked providers, available login methods, and recent identity lifecycle events. Use it for an account-management or administrative detail page where `user_get` alone would require several additional requests.

- Required payload:
  - one of `user_id`, `id`, `email`, or `username`
- Response includes:
  - the user profile and status
  - linked provider records
  - inferred login methods, such as password, Google, or GitHub
  - recent identity events

```json
{
  "db": "app/main",
  "operation": "user_get_details",
  "payload": {
    "user_id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001"
  }
}
```

#### `user_update`
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

#### `user_query`
List/query identity users with pagination.
- Optional payload:
  - `search` or `q`: matches id, email, username, or phone
  - `status`, `email`, `username`
  - `page`, `per_page`, `limit`, `offset`

Example:
```json
{
  "db": "app/main",
  "operation": "user_query",
  "payload": {
    "search": "gmail.com",
    "status": "active",
    "page": 1,
    "per_page": 25
  }
}
```

#### `user_link_provider`
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

#### `user_unlink_provider`
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

#### `user_update_status`
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

#### `user_create_token`
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

#### `user_delete`
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

### 5) File Catalog
File operations store metadata for files or objects that your application uploads somewhere else. Kongo does not upload, download, stream, move, or delete the actual bytes in this phase.

Internal table:
- `__kdb_files`: file/object metadata registry.

Rules:
- File ids default to dashless UUID v4.
- `uploaded_at` is when the app/object store received the file. If omitted, Kongo sets it to server UTC now.
- `created_at` is when the metadata row was registered in Kongo.
- `owner_type` + `owner_id` are optional generic attachment fields, such as `user` + `user_123` or `invoice` + `inv_001`.
- `file_delete` soft-deletes metadata by setting `status=deleted` and `deleted_at`.
- `file_delete` with `purge=true` hard-deletes the metadata row only.
- The application is responsible for actual S3/local object cleanup.

#### `file_create`
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

#### `file_get`
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

#### `file_query`
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
  "operation": "file_query",
  "payload": {
    "owner_type": "user",
    "owner_id": "u123",
    "status": "active",
    "page": 1,
    "per_page": 25
  }
}
```

#### `file_update`
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

#### `file_delete`
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

### 6) Namespace Lifecycle
This section documents namespace-wide stats, movement, restore, and deletion workflows.

#### `list_namespaces`
Lists namespaces + stats.
- Required payload: none

#### `get_stats`
Read live/archive counts and bytes for one namespace.
- Required:
  - top-level `namespace`

#### `recompute_stats`
Rebuilds `__kdb_system_stats` globally.
- Required payload: none

#### `drop_namespace`
Namespace drop behavior.
- Required payload:
  - none (top-level `namespace` required)
- Optional payload:
  - `ttl_seconds`, `max_docs`, `purge`, `dry_run`
- Behavior:
  - `purge=false` (default): archive + delete
  - `purge=true`: hard delete

#### `restore_archive`
Restore from archive.
- Required payload:
  - one selector: `txn_id` or `ids` or `namespace/filter`
- Optional payload:
  - `on_conflict(skip|replace|patch)`, `dry_run`

#### `purge_archive`
Hard delete from archive only.
- Required payload:
  - one selector: `txn_id` or `ids` or `namespace/filter`
- Optional payload:
  - `dry_run`

#### `change_namespace`
Move docs between namespaces by updating collection value.
- Required payload:
  - `from_namespace`
  - `to_namespace`
- Optional payload:
  - `ids|filter`, `max_docs`, `dry_run`
- Notes:
  - top-level `namespace` is rejected for this operation
  - if no selector is provided, all docs from `from_namespace` move to `to_namespace`

#### `rename_namespace`
Rename a namespace across live and archive data.
- Required payload:
  - `from_namespace`
  - `to_namespace`
- Notes:
  - top-level `namespace` is rejected for this operation

---

### 7) Database Operations
This section documents DB-scoped lifecycle, replication, backup, and maintenance operations.

#### `create_db`
Initialize DB at `db` path.
- Required payload: none
- Optional payload: none

#### `db_exists`
Check DB existence (remote-aware in s3 mode).
- Required payload: none

#### `load_db` (s3)
Preload DB into active instance.
- Required payload: none

#### `sync_db` (s3)
Force snapshot + manifest sync.
- Required payload: none

#### `create_snapshot` (s3)
Alias of `sync_db`.

#### `list_snapshots` (s3)
List versioned snapshots.
- Required payload: none

#### `get_sync_status` (s3)
Inspect local/remote status.

#### `verify_db` (s3)
Verify manifest/snapshot/segment object presence.

#### `restore_snapshot` (s3)
Restore local db from snapshot.
- Optional payload:
  - `snapshot_id` (latest if omitted)

#### `compact_wal` (s3)
Compact manifest segment list.
- Optional payload:
  - `retain_segments` (default 1000)

#### `clone_db`
Clone current DB to another path.
- Required payload:
  - `to_db_path`

#### `create_backup`
Create/enqueue DB backup.
- Optional payload:
- `backup_db_path`, `backup_tag`

#### `restore_backup`
Restore DB from backup selector or explicit path.
- Required payload:
- one of `backup_db_path|backup_id|backup_tag|backup_at|latest=true`

#### `list_backups`
List backup catalog rows.
- Optional payload:
  - `backup_tag`, `limit`, `offset`

#### `tag_backup`
Set/clear backup tag.
- Required payload:
- `backup_id` or `backup_db_path`
- Optional payload:
  - `backup_tag`

#### `offload_db` (s3)
Sync and unload local copy/connection.

#### `vacuum_db`
Run SQLite `VACUUM`.

#### `reap_db`
Run TTL/archive cleanup and due document lifecycle transitions immediately.

The response includes normal TTL/archive counters plus `document_transitions` with `claimed_count`, `completed_count`, `skipped_count`, and `failed_count` for the bounded lifecycle pass.

---

### 8) Jobs
This section documents the shared background job control operations.

#### `get_job`
Read one job row.
- Required payload:
  - `job_id`
- Optional payload:
  - `job_type`

#### `list_jobs`
List job rows with optional filters.
- Required payload: none
- Optional payload:
  - `job_type`, `status`, `limit`, `offset`

#### `continue_job`
Resume/retry a resumable or failed job.
- Required payload:
  - `job_id`
- Optional payload:
  - `job_type`

#### `abort_job`
Abort/cancel a running or queued job.
- Required payload:
  - `job_id`
- Optional payload:
  - `job_type`

---

### 9) SQL Operations
This section documents direct SQL execution and SQL table discovery.

#### `sql_execute`
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

#### `sql_list_tables`
List user-created SQL tables for the current db.
- Required payload: none
- Excludes internal `__kdb_*` tables and SQLite internal tables.

#### `sql_get_table_schema`
Return schema columns for one user-created SQL table.
- Required payload:
  - `table`
- Excludes internal `__kdb_*` tables and SQLite internal tables.
- This is the safe schema-inspection operation to use because `sql_execute` intentionally blocks arbitrary `PRAGMA`.

```json
{ "db":"myapp/main", "operation":"sql_get_table_schema", "payload":{"table":"customers"} }
```

### 10) Admin and System Operations
This section documents instance-level introspection, configuration, and indexing controls.

#### Inventory / Config
These operations expose system inventory and per-db internal config values.

##### `list_commands`
- Global operation (db not required)
- Lists all supported gateway command names.

##### `list_dbs`
- Global operation (db not required)
- Lists currently loaded/open DBs for this instance.

##### `list_all_dbs`
- Global operation (db not required)
- Lists all known DBs.
- `local`: filesystem scan
- `s3`: union of loaded + local + remote manifests

##### `system_get_inventory`
- Global operation (db not required)
- Lists DB inventory from the internal system catalog stored at `${KONGODB_DATA_DIR}/__kdb_system.db`.
- The system catalog is always available; live discovery remains the fallback source during refreshes.
- Does not scan/refresh by default; use `system_refresh_inventory` to rebuild/update the catalog.

Example:

```json
{ "operation": "system_get_inventory", "payload": { "limit": 100, "offset": 0 } }
```

##### `system_refresh_inventory`
- Global operation (db not required)
- Scans local/S3 known DBs and upserts current state into the system catalog.
- Records a `system.inventory_refreshed` catalog event.

Example:

```json
{ "operation": "system_refresh_inventory", "payload": {} }
```

##### `system_get_db_status`
- Global operation.
- Required top-level field: `db`
- Returns live status for the DB plus its catalog row when the system catalog is enabled.

Example:

```json
{ "db": "app/main", "operation": "system_get_db_status", "payload": {} }
```

##### `system_snapshot_db_stats`
- Global operation.
- With no `db`, snapshots currently active DBs only.
- With top-level `db`, snapshots that DB.
- Writes rows to the internal system catalog `__kdb_system_db_stats`.
- The background reaper cadence also snapshots active DBs into the always-on system catalog.

Example:

```json
{ "operation": "system_snapshot_db_stats", "payload": {} }
```

##### `system_query_db_stats`
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

##### `system_list_db_events`
- Global operation.
- Optional top-level `db` filters to one DB.
- Optional payload: `limit`, `offset`

Example:

```json
{ "operation": "system_list_db_events", "payload": { "limit": 50 } }
```

##### `get_system_stats`
- Global operation (db not required)
- Shows current instance-local runtime stats.
- Stats stay in memory and reset when the process restarts.
- Includes uptime, version, request totals, in-flight requests, read/write/admin/error counts, average/max latency, and 5m/15m/30m/1h rolling windows.
- Also includes process memory, active DB count/cap, background worker concurrency, and write queue usage.

##### `system_memory`
- Global operation (db not required)
- Compatibility command for process memory and write-queue usage.
- Includes the same `system_stats` block returned by `get_system_stats`.

##### `cleanup_temp_artifacts`
- Global operation (db not required)
- Removes stale temp files under the data dir.

##### `get_system_config`
- Required payload: none
- Returns `__kdb_system_config` rows (for current db).

##### `get_db_stats`
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

##### `snapshot_db_stats`
- Required payload: none
- Writes one snapshot row into `__kdb_db_stats_rollups` for the current db.
- The snapshot uses cumulative totals; interval activity is calculated by diffing two snapshots.

Example:

```json
{ "db": "app/main", "operation": "snapshot_db_stats", "payload": {} }
```

##### `query_db_stats`
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

#### Index / FTS
These operations manage or inspect manual indexes on the internal document store.

##### `create_index`
- Required payload:
  - `index_path`
- Optional payload:
  - `index_name`

##### `drop_index`
- Required payload:
  - `index_name` or `index_path`

##### `list_indexes`
- Required payload: none

#### FTS Operations
These operations manage DB-level FTS enablement and async FTS lifecycle jobs.

##### `enable_fts_index`
- Optional payload:
  - `enable` (default true)
- Only toggles DB-level FTS accessibility flag.

##### `reindex_fts`
- Required payload: none
- Enqueues async rebuild/backfill job.

##### `drop_fts_index`
- Required payload: none
- Enqueues async drop job.

## Cache Behavior (`payload.cache`)
These flags control whether reads use cache, bypass it, or invalidate it before execution.

Cached document reads (`count`, `query`, `aggregate`):
- `false` or `0`: bypass cache
- `true` or `1`: use default TTL
- `N > 1`: use per-request TTL seconds
- `-1`: invalidate relevant cache scope and run uncached

## Storage Modes (High Level)
These are the supported runtime storage backends for database files and remote sync behavior.

- `local`: filesystem `.db` files under `KONGODB_DATA_DIR`.
- `s3`: object-store mode with WAL/manifest/snapshots in a single remote S3 tier.

## Deployment
This section covers the common deployment paths and how Kongo stores data in each environment.

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

Binding to `127.0.0.1` keeps Kongo private to the host so a reverse proxy such as Caddy, Nginx, or Traefik can terminate HTTPS publicly.

### Docker Compose
The repository includes [`compose.yaml`](/Users/mardix/Dropbox/Projects/kongodb/compose.yaml) as a local durable example.

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

[`kongodb.env`](/Users/mardix/Dropbox/Projects/kongodb/kongodb.env) is the canonical environment template. Kongo exposes deployment choices, API semantics, retention, and bounded resource controls; low-level worker thresholds use internal defaults selected by `KONGODB_RUNTIME_PROFILE`.

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
| `KONGODB_S3_TOPOLOGY` | `single` | `single` disables periodic remote polling for one active instance; `multi` enables cross-instance manifest polling. Writer leases remain active in both modes. |
| `KONGODB_REPLICATION_MODE` | `async` | `sync` waits for remote persistence; `async` flushes through the background replication worker. |
| `KONGODB_PRELOAD_DBS` | empty | Comma-separated S3-mode DB paths loaded at startup. |
| `KONGODB_SNAPSHOT_EVERY_WRITES` | `100` | Target versioned snapshot cadence. Recovery currently keeps a safe checkpoint per replicated batch because replication segments are operation metadata rather than replayable database deltas. |
| `KONGODB_SNAPSHOT_RETENTION_DAYS` | `14` | Versioned snapshot age retention. The manifest points directly to the current versioned snapshot; no duplicate `current.db` is written. |
| `KONGODB_REMOTE_SYNC_INTERVAL_SECS` | `10` | Cross-instance manifest polling interval in `multi` topology. Ignored in `single`; `0` disables polling. |

Writer leases, WAL segment size, flush cadence, safe hydrate, integrity checks, snapshot count cap, and temporary-artifact cleanup use fixed safe defaults.

In S3 mode, `manifest.current_snapshot_key` is authoritative. Snapshot bytes are uploaded once to `snapshots/<snapshot_id>.db`; hydration, verification, and restore resolve the object through the manifest. `single` topology is recommended when only one Kongo process serves the S3 prefix because it avoids continuous manifest GET requests.

The TTL reaper checks active databases on its fixed interval but only publishes an S3 checkpoint when it actually archives, deletes, expires, or transitions data. An idle reaper run does not upload a snapshot.

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
| `KONGODB_STRICT_MUTATIONS_OPERATORS` | `false` | Reject invalid Mutation Operators and operand types instead of leaving them unchanged. |

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

## Request Cookbook
These compact requests are intended for copy/paste and complement the detailed operation reference above. Consult the relevant operation section for option semantics, validation rules, and complete variants.

All examples use `db: "myapp/main"`. Add the `X-Access-Key` header in authenticated deployments.

### Document Data Quick Requests
These examples provide compact forms of the document operations documented in detail earlier.

```jsonl
{ "db":"myapp/main", "operation":"insert", "namespace":"users", "payload":{"data":{"name":"Ada"}} }
{ "db":"myapp/main", "operation":"insert", "namespace":"users", "payload":{"data":[{"name":"Ada"},{"name":"Bob"}],"unique_fields":["email"],"on_conflict":"skip"} }
{ "db":"myapp/main", "operation":"update", "namespace":"users", "payload":{"data":{"_id":"u1","name":"Ada L"}} }
{ "db":"myapp/main", "operation":"update", "namespace":"users", "payload":{"filter":{"_id":{"$in":["u1","u2"]}},"data":{"plan":"pro"}} }
{ "db":"myapp/main", "operation":"update", "namespace":"users", "payload":{"replace":true,"data":{"_id":"u1","name":"Ada","plan":"pro"}} }
{ "db":"myapp/main", "operation":"upsert", "namespace":"users", "payload":{"filter":{"email":{"$eq":"a@b.com"}},"insert_data":{"email":"a@b.com"},"update_data":{"last_seen":{"$ts_now":true}}} }
{ "db":"myapp/main", "operation":"query", "namespace":"*", "payload":{"filter":{"_id":{"$in":["u1","u2"]}},"fields":["name","email"]} }
{ "db":"myapp/main", "operation":"count", "namespace":"users", "payload":{"filter":{"status":{"$eq":"active"}}} }
{ "db":"myapp/main", "operation":"query", "namespace":"users", "payload":{"filter":{"age":{"$gte":18}},"sort":"age desc","limit":20} }
{ "db":"myapp/main", "operation":"aggregate", "namespace":"users", "payload":{"compute":{"total":{"$count":"*"},"avg_age":{"$avg":"age"}}} }
{ "db":"myapp/main", "operation":"query", "namespace":"users", "payload":{"search":"ada","limit":10} }
{ "db":"myapp/main", "operation":"metrics_ingest", "payload":{"events":[{"event":"api.request","dimensions":{"endpoint":"/v1/chat","duration_ms":120}}]} }
{ "db":"myapp/main", "operation":"metrics_query", "payload":{"event":"api.request","range":"24h","interval":"hour","metrics":[{"op":"count","field":"*","alias":"requests","label":"Requests"}]} }
```

### Lifecycle/Archive Examples
These examples cover soft delete, namespace drop, TTL, restore, purge, and namespace changes.

```jsonl
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

```jsonl
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

```jsonl
{ "db":"myapp/main", "operation":"create_db", "payload":{} }
{ "db":"myapp/main", "operation":"db_exists", "payload":{} }
{ "operation":"list_commands", "payload":{} }
{ "operation":"list_dbs", "payload":{} }
{ "operation":"list_all_dbs", "payload":{} }
{ "operation":"system_memory", "payload":{} }
{ "db":"myapp/main", "operation":"load_db", "payload":{} }
{ "db":"myapp/main", "operation":"sql_list_tables", "payload":{} }
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

```jsonl
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

```jsonl
{ "db":"test/db02.main", "operation":"query::users", "payload":{} }
{ "db":"test/db02.main", "operation":"query::*", "payload":{} }
{ "db":"test/db02.main", "operation":"query::users,admins,teams", "payload":{} }
{ "db":"test/db02.main", "operation":"query::users", "payload":{"search":"ada"} }
```

## Notes
These are final reminders about current behavior, reserved semantics, and implementation limits.

- All system timestamps are UTC.
- `group_by` exists in the payload but is not implemented yet.
- `query` with `payload.search` only targets live documents.
- `_key` has no special meaning and is stored, filtered, and returned like ordinary document data.
- `namespace` is the canonical name; `collection` is an alias.
