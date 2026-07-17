# Kongo

### Hybrid Database Toolkit

**Kongo** is a lightweight, self-hosted data platform that combines the flexibility of a document database with the power of SQLite. It provides one consistent JSON API for document storage, direct SQL access, identity records, file metadata, metrics, audit logs, full-text search, and database administration.

Built in Rust on SQLite/libSQL, Kongo is designed for applications that need a capable embedded or standalone data service without operating a large database stack. It runs locally, in Docker, or with S3-backed storage — and it's exposed as a single RPC-style HTTP endpoint: JSON in, JSON out.

View the full [Documentation](./DOCUMENTATION.md).

---


## Table of Contents

1. [The Stacks at a Glance](#the-stacks-at-a-glance)
2. [Main Features](#main-features)
3. [Deployment Models](#deployment-models)
4. [Quick Start](#quick-start)
5. [API Surface](#api-surface)
6. [Request / Response Contract](#request--response-contract)
7. [The Kongo Stack](#the-kongo-stack)
   - [DocumentDB (Data Stack)](#documentdb-data-stack)
   - [Identity](#identity)
   - [Files](#files)
   - [SQLiteDB (SQL Stack)](#sqlitedb-sql-stack)
   - [FTS (Full-Text Search)](#fts-full-text-search)
   - [Transaction](#transaction)
   - [Metrics](#metrics)
   - [Audit Logs](#audit-logs)
   - [Advanced (Database, Jobs, Admin, Namespace)](#advanced-database-jobs-admin-namespace)
8. [Filters, Compute & Lookups (JQL)](#filters-compute--lookups-jql)
9. [Write-Time Value Operators](#write-time-value-operators)
10. [Payload Field Reference](#payload-field-reference)
11. [Configuration Reference](#configuration-reference)
12. [Admin UI](#admin-ui)
13. [Deployment](#deployment)
14. [Examples](#examples)

---

## The Stacks at a Glance

| Stack | What it's for |
|---|---|
| **DocumentDB** | Full-featured document database — CRUD, filters, sorting, projections, lookups |
| **Identity** | Storage for auth/user records — profiles, statuses, providers, tokens |
| **Files** | File *metadata* storage (Kongo doesn't store the bytes themselves) |
| **Metrics** | Application metric events — counts, sums, averages, time buckets |
| **FTSearch** | Full-text search over documents (SQLite FTS5) |
| **SQLiteDB** | Direct, parameterized SQL access to your own tables |
| **Audit Logs** | Immutable audit event storage, queryable by actor/action/target |


## Main Features

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

## Deployment Models

Kongo can run as:

- An embedded local database service
- A self-hosted Docker application with persistent volumes
- A serverless container with S3-backed durable storage
- A lightweight database gateway for SaaS applications
- A development and administration layer over SQLite data

Kongo's goal: one compact service for common application data needs, without giving up SQLite's portability, reliability, and direct SQL access.

---


## Quick Start

```bash
docker run -d \
  --name kongo \
  -p 8080:8080 \
  -e KONGODB_ACCESS_KEY=change-me \
  -v kongo-data:/data \
  kongo
```

Check it's alive:

```bash
curl http://localhost:8080/_/kdb/ping
```

Send your first request:

```bash
curl -X POST http://localhost:8080/_/kdb/gateway \
  -H 'content-type: application/json' \
  -H 'x-access-key: change-me' \
  -d '{"db":"app/main","operation":"create_db","payload":{}}'
```

Open the Admin UI at `http://localhost:8080/_/kdb/admin/` (username `kongo`, password is your access key).

---

## API Surface

### Endpoints

| Route | Purpose |
|---|---|
| `POST ${KONGODB_BASE_PATH}/gateway` | The one true endpoint — all operations go here (default path: `/gateway`) |
| `GET ${KONGODB_BASE_PATH}/ping` | Health check + version |
| `GET ${KONGODB_BASE_PATH}/meta/operations` | Machine-readable catalog of every operation |
| `GET ${KONGODB_BASE_PATH}/doc` | Rendered docs (this file, essentially) |
| `GET ${KONGODB_BASE_PATH}/admin/` | Built-in Admin UI (SPA), toggle with `KONGODB_ADMIN_UI_ENABLED` |

### Auth

- Send `X-Access-Key: <key>` on every request.
- Browser access to `/doc` and `/admin/` uses HTTP Basic — username `kongo`, password = `KONGODB_ACCESS_KEY`.
- Use HTTPS outside localhost — Basic auth credentials are only safe over TLS.
- `KONGODB_AUTH_MODE=access_key` requires `KONGODB_ACCESS_KEY`; use `KONGODB_AUTH_MODE=none` only for trusted local development.
- `/ping` is always open. `/meta/operations` can be locked down with `KONGODB_META_REQUIRES_AUTH=true`.

---

## Request / Response Contract

### Request envelope

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "users",
  "namespaces": ["users", "admins"],
  "payload": {}
}
```

### The rules of the road

- `db` is required everywhere except global, db-list-style operations.
- `namespace` is the canonical selector. `collection` is just an alias (top-level, and `payload.collection`).
- `namespace` and `namespaces` are mutually exclusive.
- Shorthand alias works too: `"operation": "query::users"` → `operation=query`, `namespace=users`. Same for `query::*` and `query::users,admins,teams`. Shorthand can't be mixed with a top-level `namespace`/`namespaces`.
- Namespace requirements by operation type:
  - **Required:** `insert`, `query`, `search`
  - **Insert-family:** must be a single concrete namespace — no `*`, no `namespaces[]`
  - **ID-targeted** (`get`, `set`, `update`, `delete`): namespace optional, strict if given
  - **Filter/wide ops:** namespace required unless `scope=all` is explicitly supported and set
- `namespace: "*"` is shorthand for `payload.scope: "all"` (conflicts with `payload.scope: "collection"`).
- Only three operations can create a brand-new db: `create_db`, `insert`, `import_jsonl`.

### Success response

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

### Error response

```json
{
  "status": "error",
  "error": "reason"
}
```

### Datetimes

All system timestamps are UTC, RFC3339/ISO-8601 with timezone.

```
2025-12-24T23:39:26Z
2025-12-24T23:39:26.873397+00:00
```

If `_created_at` is given without `_modified_at`, the latter inherits the former.

---

## The Kongo Stack

Kongo's operations are grouped into stacks. Each one covers a distinct slice of what your app needs. Every operation below includes a full, ready-to-send example.

### DocumentDB (Data Stack)

The bread and butter — documents in, documents out.

| Op | Needs | What it does |
|---|---|---|
| `insert` | `namespace`, `data` | Insert one or many docs. Supports `unique_fields` + `on_conflict` for soft dedupe. |
| `update` | `data(with _id)` or `filter + data` or `data(array)` | Patch one doc, many by filter, or many by explicit ids. `replace=true` only works single-doc. |
| `set` | `data(with _id)` | With `namespace` → upsert by id. Without → update-only. |
| `upsert` | `filter`, `insert_data` | Update on match, insert on miss. |
| `get` | `id` / `ids` / `data._id` | Fetch by id(s). Sees pending accepted writes unless `force_db=true`. |
| `count` | — | Count matches, filter optional. |
| `query` | `namespace`/`namespaces`/`*` | Filter, sort, paginate, project, lookup, per-row compute. |
| `aggregate` | `compute` | Set-level metrics: `$count`, `$sum`, `$avg`, `$min`, `$max`, `$distinct`. |
| `delete` | one of `id` / `ids` / `filter` | Soft-delete to archive by default; `purge=true` hard-deletes. |
| `set_ttl` | `ids`/`filter` + `ttl_seconds` | Set or reset a document's TTL. |
| `import_jsonl` | `namespace`, `source_path` | Enqueue a background JSONL import job. |
| `export_jsonl` | — | Enqueue a background JSONL export job. |

**`insert`** — one or many documents:

```json
{
  "db": "myapp/main",
  "operation": "insert",
  "namespace": "users",
  "payload": {
    "data": [
      {
        "email": "a@b.com",
        "name": "Ada"
      },
      {
        "email": "b@b.com",
        "name": "Bob"
      }
    ],
    "unique_fields": ["email"],
    "on_conflict": "skip"
  }
}
```

**`update`** — patch many documents by filter:

```json
{
  "db": "myapp/main",
  "operation": "update",
  "namespace": "users",
  "payload": {
    "filter": {
      "plan": {
        "$eq": "trial"
      }
    },
    "data": {
      "plan": "pro"
    },
    "max_docs": 100
  }
}
```

**`set`** — upsert a single document by id:

```json
{
  "db": "myapp/main",
  "operation": "set",
  "namespace": "users",
  "payload": {
    "data": {
      "_id": "u1",
      "name": "Ada",
      "plan": "pro"
    }
  }
}
```

**`upsert`** — update on match, insert on miss:

```json
{
  "db": "myapp/main",
  "operation": "upsert",
  "namespace": "users",
  "payload": {
    "filter": {
      "email": {
        "$eq": "a@b.com"
      }
    },
    "insert_data": {
      "email": "a@b.com",
      "name": "Ada"
    },
    "update_data": {
      "last_seen": {
        "$ts_now": true
      }
    }
  }
}
```

**`get`** — fetch by id(s):

```json
{
  "db": "myapp/main",
  "operation": "get",
  "namespace": "users",
  "payload": {
    "ids": ["u1", "u2"],
    "fields": ["name", "email"]
  }
}
```

**`count`** — count matches:

```json
{
  "db": "myapp/main",
  "operation": "count",
  "namespace": "users",
  "payload": {
    "filter": {
      "status": {
        "$eq": "active"
      }
    }
  }
}
```

**`query`** — filter, sort, project, and compute:

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "users",
  "payload": {
    "filter": {
      "status": {
        "$eq": "active"
      }
    },
    "sort": "profile.age desc, name",
    "fields": ["name", "profile.age"],
    "compute": {
      "full_name": {
        "$join": ["$name", " (", "$profile.age", ")"]
      }
    },
    "limit": 20
  }
}
```

**`aggregate`** — set-level compute:

```json
{
  "db": "myapp/main",
  "operation": "aggregate",
  "namespace": "users",
  "payload": {
    "filter": {
      "status": {
        "$eq": "active"
      }
    },
    "compute": {
      "total": {
        "$count": "*"
      },
      "avg_age": {
        "$avg": "age"
      },
      "unique_countries": {
        "$distinct": "country"
      }
    }
  }
}
```

**`delete`** — soft-delete many documents by filter:

```json
{
  "db": "myapp/main",
  "operation": "delete",
  "namespace": "users",
  "payload": {
    "filter": {
      "status": {
        "$eq": "inactive"
      }
    },
    "max_docs": 100
  }
}
```

**`set_ttl`** — expire a document after a set time:

```json
{
  "db": "myapp/main",
  "operation": "set_ttl",
  "namespace": "users",
  "payload": {
    "ids": ["u1"],
    "ttl_seconds": 600,
    "expiry_behavior": "archive"
  }
}
```

**`import_jsonl`** — enqueue a background import job:

```json
{
  "db": "myapp/main",
  "operation": "import_jsonl",
  "namespace": "users",
  "payload": {
    "source_path": "s3://bucket/path/users.jsonl.zst",
    "on_conflict": "skip",
    "batch_size": 500
  }
}
```

**`export_jsonl`** — enqueue a background export job:

```json
{
  "db": "myapp/main",
  "operation": "export_jsonl",
  "namespace": "users",
  "payload": {
    "target_path": "s3://bucket/exports/users",
    "compress": true,
    "filter": {
      "status": {
        "$eq": "active"
      }
    }
  }
}
```

---

### Identity

Stores login-related metadata for your app. **Kongo does not authenticate anyone** — no password checks, no OAuth validation, no session issuing. It just stores the data your auth layer needs.

| Op | Needs | What it does |
|---|---|---|
| `user_create` | — | Create a user record (email, username, profile, provider link, `data`). |
| `user_get` | `user_id`/`id`/`email`/`username` or `provider`+`provider_user_id` | Fetch one user. |
| `user_get_details` | same selectors | Fetch a user plus linked providers and recent events. |
| `user_list` | — | Paginated user search/list. |
| `user_update` | `user_id`/`id` | Update profile fields + `data`. |
| `user_update_status` | `user_id`/`id`, `status` | Set status, optionally schedule an automatic future transition. |
| `user_delete` | `user_id`/`id` | Soft-delete (revokes tokens, keeps identity reserved) or `purge=true` to hard-delete everything. |
| `user_create_token` | `user_id`/`id`, `kind`, `token_hash` | Store an app-generated token hash (resets, magic links, API keys...). |
| `user_link_provider` | `user_id`/`id`, `provider`, `provider_user_id` | Link a Google/GitHub/custom identity to an existing user. |
| `user_unlink_provider` | `provider`, `provider_user_id` | Unlink a provider identity; optional `user_id` makes it strict. |

**Rule of thumb:** store `password_hash`, never raw passwords. Store `token_hash`, never raw tokens.

**`user_create`**:

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

**`user_get`**:

```json
{
  "db": "app/main",
  "operation": "user_get",
  "payload": {
    "email": "user@example.com"
  }
}
```

**`user_get_details`** — user plus linked providers and recent events:

```json
{
  "db": "app/main",
  "operation": "user_get_details",
  "payload": {
    "user_id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001"
  }
}
```

**`user_list`**:

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

**`user_update`**:

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

**`user_update_status`** — ban for two days, then auto-return to active:

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

**`user_delete`** — soft delete:

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

**`user_create_token`**:

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

**`user_link_provider`**:

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

**`user_unlink_provider`**:

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

---

### Files

Metadata only — Kongo never touches the actual bytes. Your app still owns uploading/downloading to S3, disk, etc.

| Op | Needs | What it does |
|---|---|---|
| `file_create` | `storage_backend`, `storage_path` | Register a file/object's metadata. |
| `file_get` | `id` | Fetch one file record. |
| `file_list` | — | List/search by bucket, owner, status, backend, content type. |
| `file_update` | `id` | Update mutable metadata. |
| `file_delete` | `id` | Soft-delete by default; `purge=true` hard-deletes the row. |

**`file_create`**:

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

**`file_get`**:

```json
{
  "db": "app/main",
  "operation": "file_get",
  "payload": {
    "id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001"
  }
}
```

**`file_list`** — all files attached to a user:

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

**`file_update`**:

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

**`file_delete`**:

```json
{
  "db": "app/main",
  "operation": "file_delete",
  "payload": {
    "id": "f9c1b3a9e2a84f9aa0bdb88e8c12f001",
    "purge": false
  }
}
```

---

### SQLiteDB (SQL Stack)

The escape hatch, when JQL isn't enough. `sql_execute` is gated behind `KONGODB_ENABLE_SQL_EXECUTE`.

| Op | Needs | What it does |
|---|---|---|
| `sql_execute` | `sql` | One statement: `SELECT`/`WITH`/`EXPLAIN`/`INSERT`/`UPDATE`/`DELETE`/`REPLACE`, plus limited DDL (`CREATE TABLE`/`CREATE INDEX`/`DROP INDEX`/`ALTER TABLE ... ADD COLUMN`). Blocks `__kdb_*` and `sqlite_*` objects. |
| `list_tables` | — | List your own SQL tables (internal tables excluded). |
| `get_table_schema` | `table` | Safe `PRAGMA table_info` wrapper — arbitrary `PRAGMA` is not allowed. |

**`sql_execute`** — parameterized query:

```json
{
  "db": "myapp/main",
  "operation": "sql_execute",
  "payload": {
    "sql": "SELECT collection, COUNT(*) AS total FROM __kdb_documents WHERE collection = ? GROUP BY collection",
    "params": ["users"]
  }
}
```

**`list_tables`**:

```json
{
  "db": "myapp/main",
  "operation": "list_tables",
  "payload": {}
}
```

**`get_table_schema`**:

```json
{
  "db": "myapp/main",
  "operation": "get_table_schema",
  "payload": {
    "table": "customers"
  }
}
```

---

### FTS (Full-Text Search)

Full-text search over live documents, powered by SQLite FTS5. Requires `enable_fts_index` to be run once per database before `search` will return results.

| Op | Needs | What it does |
|---|---|---|
| `search` | `namespace` + `search` | Full-text search (FTS5) over live docs, with filter/sort/pagination/projection/lookup. |
| `enable_fts_index` | — | Toggle the db-level FTS accessibility flag. |
| `reindex_fts` | — | Enqueue an async FTS rebuild/backfill job. |
| `drop_fts_index` | — | Enqueue an async FTS drop job. |

**`search`**:

```json
{
  "db": "myapp/main",
  "operation": "search",
  "namespace": "users",
  "payload": {
    "search": "ada",
    "filter": {
      "status": {
        "$eq": "active"
      }
    },
    "fields": ["name", "email"],
    "limit": 10
  }
}
```

**`enable_fts_index`**:

```json
{
  "db": "myapp/main",
  "operation": "enable_fts_index",
  "payload": {
    "enable": true
  }
}
```

**`reindex_fts`**:

```json
{
  "db": "myapp/main",
  "operation": "reindex_fts",
  "payload": {}
}
```

**`drop_fts_index`**:

```json
{
  "db": "myapp/main",
  "operation": "drop_fts_index",
  "payload": {}
}
```

---

### Transaction

Run multiple write operations atomically, as a single unit.

| Op | Needs | What it does |
|---|---|---|
| `transaction` | `data[]` (array of ops) | Run `insert`/`update`/`delete` atomically as one unit. |

**`transaction`**:

```json
{
  "db": "myapp/main",
  "operation": "transaction",
  "data": [
    {
      "operation": "insert",
      "namespace": "users",
      "payload": {
        "data": {
          "_id": "u1",
          "name": "Ada"
        }
      }
    },
    {
      "operation": "update",
      "namespace": "users",
      "payload": {
        "data": {
          "_id": "u1",
          "plan": "pro"
        }
      }
    }
  ]
}
```

---

### Metrics

A lightweight, append-only events engine for SaaS-style metrics (API calls, signups, whatever you want to count).

| Op | Needs | What it does |
|---|---|---|
| `metrics_ingest` | `events[]` | Append events. Defaults to a fast queued ack (`commit:true` to wait for durability). |
| `metrics_query` | `event`/`events`, `range` or `start`+`end`, `metrics` | Aggregate events into labeled, bucketed results. |
| `metrics_catalog` | — | List discovered event names and their dimension paths. |

Rolling ranges: `24h`, `7d`, `3days`, `2weeks`, `4months`, `1year`.
Calendar ranges: `today`, `yesterday`, `this_week`, `last_week`, `this_month`, `last_month`, `this_year`, `last_year`.
Metric ops: `count`, `sum`, `avg`, `min`, `max`, `distinct`, `count_distinct`.

**`metrics_ingest`**:

```json
{
  "db": "app/main",
  "operation": "metrics_ingest",
  "payload": {
    "events": [
      {
        "event": "api.request",
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

**`metrics_query`**:

```json
{
  "db": "app/main",
  "operation": "metrics_query",
  "payload": {
    "alias": "api_requests",
    "label": "API Requests",
    "event": "api.request",
    "range": "24h",
    "interval": "hour",
    "bucket_label": "{{bucket HH:mm}}",
    "filter": {
      "tenant_id": "tenant_123"
    },
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
    ]
  }
}
```

**`metrics_catalog`** — list dimensions for one event:

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

---

### Audit Logs

Append-only, immutable activity logging. Kongo doesn't infer who did what — your app explicitly records the events worth keeping.

| Op | Needs | What it does |
|---|---|---|
| `audit_ingest` | `events[]` (each needs `action`) | Record one or more audit events. Defaults to committed (durable) ack. |
| `audit_query` | — | Search/filter by action, actor, target, status, source, time range. |

**`audit_ingest`**:

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

**`audit_query`**:

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

---

### Advanced (Database, Jobs, Admin, Namespace)

Instance- and database-level operations: lifecycle, replication, background job control, and introspection. These are used less often day-to-day, so they're grouped together here rather than given a full example per operation.

#### Database

Whole-database lifecycle: create, replicate, back up, restore, maintain.

| Op | Needs | What it does |
|---|---|---|
| `create_db` | — | Initialize the db at its path. |
| `db_exists` | — | Check existence (remote-aware in S3 mode). |
| `load_db` / `offload_db` | — | (S3 mode) Preload into memory / flush & unload. |
| `sync_db` / `create_snapshot` | — | Force a snapshot + manifest sync. |
| `list_snapshots` / `restore_snapshot` | `snapshot_id` optional | List or restore versioned snapshots. |
| `get_sync_status` / `verify_db` | — | Check sync state / verify manifest integrity. |
| `compact_wal` | `retain_segments` optional | Compact the WAL segment list. |
| `clone_db` | `to_db_path` | Clone the current db elsewhere. |
| `create_backup` | — | Enqueue a backup job. |
| `restore_backup` | one of `backup_db_path`/`backup_id`/`backup_tag`/`backup_at`/`latest` | Restore from a backup. |
| `list_backups` / `tag_backup` | — | Browse and tag the backup catalog. |
| `vacuum_db` | — | Run SQLite `VACUUM`. |
| `reap_db` | — | Trigger the TTL reaper immediately. |

#### Jobs

Every long-running task (import, export, FTS rebuilds...) runs as a background job you can inspect and control.

| Op | Needs | What it does |
|---|---|---|
| `get_job` | `job_id` | One job's status/details. |
| `list_jobs` | — | List with optional `job_type`/`status` filters. |
| `continue_job` | `job_id` | Resume/retry a resumable or failed job. |
| `abort_job` | `job_id` | Cancel a running/queued job. |

#### Admin / System

Instance-level introspection, inventory, and index controls.

| Op | Needs | What it does |
|---|---|---|
| `list_commands` | — | Every supported operation name. |
| `list_dbs` / `list_all_dbs` | — | Loaded dbs / all known dbs. |
| `system_get_inventory` / `system_refresh_inventory` | — | Read or rebuild the system catalog. |
| `system_get_db_status` | `db` | Live status + catalog row for one db. |
| `system_snapshot_db_stats` / `system_query_db_stats` | — | Persist or query historical db stats. |
| `system_list_db_events` | — | DB lifecycle/error events. |
| `get_system_stats` / `system_memory` | — | Instance uptime, request counters, memory, queues. |
| `cleanup_temp_artifacts` | — | Remove stale temp files. |
| `get_system_config` / `get_db_stats` | — | Per-db config values / live counters. |
| `create_index` | `index_path` | Create a manual JSON index. |
| `drop_index` | `index_name`/`index_path` | Drop an index. |
| `list_indexes` | — | List all indexes. |

#### Namespace

Manage whole namespaces as units instead of individual documents.

| Op | Needs | What it does |
|---|---|---|
| `list_namespaces` | — | List namespaces + stats. |
| `get_stats` | `namespace` | Live/archive stats for one namespace. |
| `recompute_stats` | — | Rebuild global stats. |
| `drop_namespace` | `namespace` | Archive + delete, or hard-delete with `purge=true`. |
| `restore_archive` | `txn_id` / `ids` / `namespace + filter` | Restore from archive (`skip`/`replace`/`patch` conflict policy). |
| `purge_archive` | same selectors | Hard-delete from archive only. |
| `change_namespace` | `from_namespace`, `to_namespace` | Move docs between namespaces. |
| `rename_namespace` | `from_namespace`, `to_namespace` | Rename across live + archive data. |

---

## Filters, Compute & Lookups (JQL)

### Filter operators (`payload.filter`)

| Family | Operators |
|---|---|
| Logical | `$and`, `$or`, `$nor`, `$not` |
| Comparison | `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte`, `$between`, `$exists` |
| Set / array | `$in`, `$nin`, `$includes`, `$nincludes`, `$all`, `$any`, `$none`, `$elemMatch`, `$size` |
| String | `$startsWith`, `$endsWith`, `$contains`, `$ilike`, `$istartsWith`, `$iendsWith`, `$icontains`, `$regex` |
| Type | `$type` |

```json
{
  "$and": [
    {
      "status": {
        "$in": ["active", "trial"]
      }
    },
    {
      "$or": [
        {
          "plan": {
            "$eq": "pro"
          }
        },
        {
          "plan": {
            "$eq": "enterprise"
          }
        }
      ]
    }
  ]
}
```

### Compute operators (`payload.compute`)

- **Aggregate (set-level):** `$count`, `$sum`, `$avg`, `$min`, `$max`, `$distinct` — with `$distinct: true` and metric-local `$filter`.
- **Query (per-row):** all of the above, plus `$size` and `$join`. Any `$`-prefixed string token in `$join` is a path on the current row (e.g. `$profile.email`).

### Lookups (`payload.lookups`)

Join in related data, keyed by alias:

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

Supports nested lookups, context selectors (`$self`, `$parent`, `$root`, `$lookup.<alias>`), and options like `filter`, `sort`, `limit`, `dedupe`, `on_missing`.

---

## Write-Time Value Operators

Recognized inside `data`, `insert_data`, and `update_data` as single-key `$`-operator objects.

**Generators** — create values at write time:
`$ts_now`, `$ts_now_ms`, `$id_uuidv4`, `$id_uuidv7`, `$id_random`, `$hash_value`

**Mutations** — modify existing values in place:
`$unset`, `$inc`, `$push`, `$pop`, `$extend`, `$pull`, `$addset`

```json
{
  "data": {
    "_id": {
      "$id_uuidv4": {
        "prefix": "session:"
      }
    },
    "created_at": {
      "$ts_now": true
    },
    "score": {
      "$inc": 1
    },
    "tags": {
      "$addset": "beta"
    }
  }
}
```

By default unknown mutation operators silently no-op. Set `KONGODB_STRICT_MUTATIONS_OPERATORS=true` to make them return an error instead.

---

## Payload Field Reference

The most commonly used fields across operations:

| Field | Type | Meaning |
|---|---|---|
| `id` / `ids` | string / string[] | Document selector(s) |
| `data` | object/array | Main write payload |
| `insert_data` / `update_data` | object | Upsert payloads |
| `filter` | object | JQL filter |
| `sort` | object/string | Sort spec |
| `limit` / `offset` / `page` / `per_page` | int | Pagination |
| `fields` / `exclude_fields` | string[] | Projection |
| `compute` | object | Aggregate/per-row derived values |
| `lookups` | object | Join map |
| `scope` | string | `collection` (default) or `all` |
| `include_archive` / `archive_only` | bool | Read source control |
| `cache` | bool/int | `false/0` bypass, `true/1` default TTL, `N` custom TTL, `-1` invalidate |
| `dry_run` | bool | Simulate without writing |
| `purge` | bool | Hard-delete flag |
| `ttl_seconds` / `expiry_behavior` | int / `archive`\|`delete` | TTL config |
| `unique_fields` / `on_conflict` | string[] / string | Insert-time soft uniqueness |
| `commit` | bool | Per-request write ack override |
| `include_namespace` | bool | Show `_namespace` in response (alias `include_name`) |

*(See the full field table in the appendix docs for identity, files, metrics, and audit-specific fields.)*

---

## Configuration Reference

Kongo ships with sensible defaults — you can run it with zero config beyond `KONGODB_ACCESS_KEY`. Everything below is optional tuning.

A number of subsystems that used to be individually feature-gated are now **core, always-on features** with internally managed safety thresholds: SQL execution capability, FTS capability, metric events, auto-indexing, JSONB storage, the system catalog, safe hydration, temp-file cleanup, and background job workers. You no longer need to flip a switch for these — they just work.

### Server & Auth

| Variable | Default | Purpose |
|---|---|---|
| `KONGODB_PORT` | `8080` | HTTP port |
| `KONGODB_BASE_PATH` | `/_/kdb` | Prefix for every route — gateway, docs, and Admin UI all live under this |
| `KONGODB_AUTH_MODE` | `access_key` | `access_key` requires credentials; `none` explicitly enables unauthenticated local access |
| `KONGODB_ACCESS_KEY` | empty | Shared API key (`X-Access-Key` header) and Basic-auth password; required in `access_key` mode |
| `KONGODB_CORS_ALLOWED_ORIGINS` | empty | Comma-separated origins, for standalone browser clients calling the API directly. The bundled Admin UI is same-origin and doesn't need this |
| `KONGODB_MAX_REQUEST_BYTES` | `16777216` | Max accepted request body size |
| `KONGODB_OPERATION_TIMEOUT_MS` | `30000` | Max execution time per operation |

### Built-in Web Interfaces

| Variable | Default | Purpose |
|---|---|---|
| `KONGODB_ADMIN_UI_ENABLED` | `true` | Serve the Admin UI at `${KONGODB_BASE_PATH}/admin/` |
| `KONGODB_DOCS_ENABLED` | `true` | Serve rendered docs at `${KONGODB_BASE_PATH}/doc` |
| `KONGODB_DOCS_FILE` | `DOCUMENTATION.md` | Which markdown file `/doc` renders |

### Storage

| Variable | Default | Purpose |
|---|---|---|
| `KONGODB_STORAGE_MODE` | `local` | `local` — durable local `.db` files, or `s3` — local working files plus S3-backed WAL/snapshots |
| `KONGODB_DATA_DIR` | `./data` | Root directory for local database files |
| `KONGODB_S3_BUCKET` | empty | Required when `KONGODB_STORAGE_MODE=s3` |
| `KONGODB_S3_PREFIX` | `data/kongodb/data` | Object key prefix under the bucket |
| `KONGODB_S3_REGION` | `us-east-1` | S3 region |
| `KONGODB_S3_ENDPOINT` | empty | Custom S3-compatible endpoint (optional) |
| `KONGODB_S3_ACCESS_KEY` / `KONGODB_S3_SECRET_KEY` | empty | S3 credentials |
| `KONGODB_S3_SESSION_TOKEN` | empty | Optional STS session token |

### Runtime Profile & Concurrency

| Variable | Default | Purpose |
|---|---|---|
| `KONGODB_RUNTIME_PROFILE` | `balanced` | `balanced` — standard defaults · `memory` — smaller caches/queues for constrained environments · `throughput` — larger batches/concurrency for high load |
| `KONGODB_MAX_ACTIVE_DBS` | `100` | Optional override for the active/open-DB LRU cap under the chosen profile |
| `KONGODB_WORKER_CONCURRENCY` | `4` | Shared concurrency budget across the reaper, backups, jobs, and remote-sync work |

### S3 Replication & Snapshots

*(Only relevant in `s3` storage mode.)*

| Variable | Default | Purpose |
|---|---|---|
| `KONGODB_REPLICATION_MODE` | `async` | `sync` waits for remote replication before acknowledging a write; `async` flushes in the background |
| `KONGODB_PRELOAD_DBS` | empty | Comma-separated db paths to load at startup |
| `KONGODB_SNAPSHOT_EVERY_WRITES` | `100` | Create a portable snapshot after this many writes |
| `KONGODB_SNAPSHOT_RETENTION_DAYS` | `14` | How long snapshots are kept |
| `KONGODB_REMOTE_SYNC_INTERVAL_SECS` | `3` | Remote snapshot polling interval; `0` disables cross-instance polling |

### Read / Write / Query Behavior

| Variable | Default | Purpose |
|---|---|---|
| `KONGODB_CACHE_TTL_SECS` | `15` | Read cache TTL; `0` disables the read cache entirely |
| `KONGODB_WRITE_MODE` | `committed` | `direct` bypasses the write coordinator · `committed` waits for the durable result · `accepted` queues and acknowledges immediately |
| `KONGODB_QUERY_DEFAULT_LIMIT` | `50` | Default page size when `limit`/`offset` are omitted |
| `KONGODB_QUERY_LOOKUP_MAX_DEPTH` | `3` | Default max nested lookup depth |
| `KONGODB_QUERY_LOOKUP_UNCAPPED_OVERRIDE_ENABLED` | `false` | Allow a request to override the lookup depth cap |
| `KONGODB_RESPONSE_INCLUDE_SYSTEM_TIMESTAMPS` | `true` | Include `_created_at`/`_modified_at` in responses by default |
| `KONGODB_RESPONSE_INCLUDE_NAMESPACE` | `false` | Include `_namespace` in `get`/`query`/`search` item responses by default |
| `KONGODB_STRICT_MUTATIONS_OPERATORS` | `false` | `true` makes unknown/invalid mutation operators (`$inc`, `$push`, etc.) return an error instead of silently no-op'ing |

### Data Lifecycle

| Variable | Default | Purpose |
|---|---|---|
| `KONGODB_ARCHIVE_TTL_SECS` | empty | Empty = no automatic archive purge |
| `KONGODB_DELETE_DEFAULT_TTL_SECS` | empty | Empty = no default delete TTL |
| `KONGODB_SYSTEM_RETENTION_DAYS` | `14` | Retention for internal system catalog history |

### Metric Events

| Variable | Default | Purpose |
|---|---|---|
| `KONGODB_METRIC_EVENTS_CACHE_TTL_SECS` | `30` | Cache TTL for `metrics_query` results only. Set to `0` to disable just that cache — the Metrics Stack itself is always available |
| `KONGODB_METRIC_EVENTS_RETENTION_DAYS` | empty | Empty keeps raw metric events indefinitely |

### Backup, Export & Jobs

| Variable | Default | Purpose |
|---|---|---|
| `KONGODB_BACKUP_PATH` | `./backups` | Local path or full `s3://bucket/prefix`, used by both manual and automatic backups |
| `KONGODB_BACKUP_EVERY_SECS` | `0` | `0` disables automatic backups. A positive value enables change-aware backups with that value as max staleness |
| `KONGODB_BACKUP_RETENTION_DAYS` | `30` | How long backups are retained |
| `KONGODB_EXPORT_PATH` | `./exports` | Local path or full `s3://bucket/prefix` for generated JSONL exports |
| `KONGODB_JOB_RETENTION_DAYS` | `30` | Shared retention for terminal import/export job history |

### Legacy Field Aliases

| Variable | Default | Purpose |
|---|---|---|
| `KONGODB_ENABLE_LEGACY_ALIASES` | `false` | Enable legacy request/response alias normalization |
| `KONGODB_LEGACY_ALIASES_IMPORT_PK` | `_key:_id` | Request/import primary-key alias map (`from:to,...`) |
| `KONGODB_LEGACY_ALIASES_RESPONSE` | `_key:_id` | Response alias projection map (`alias:canonical,...`) |

*(Full variable list with inline comments lives in `kongo.env`.)*

---

## Admin UI

Kongo ships with a built-in Admin UI — a single-page app served directly by Kongo itself, no separate install needed.

- **URL:** `http://<host>:8080/_/kdb/admin/` (path follows `KONGODB_BASE_PATH`)
- **Toggle:** `KONGODB_ADMIN_UI_ENABLED=true` (default). Set to `false` for API-only deployments.
- **Auth:** same access key as the API, entered as HTTP Basic — username `kongo`, password = `KONGODB_ACCESS_KEY`.
- **What you can do from it:**
  - Browse databases, namespaces, and documents
  - Run and preview queries, filters, projections, and pagination against any namespace
  - Use the FTSearch workspace to search live documents and manage FTS index lifecycle
  - View the Audit Timeline and manually record audit events
  - Inspect backups, snapshots, and background jobs
  - Read the rendered docs (same content as `/doc`)

Because it's served same-origin from Kongo, you don't need to configure `KONGODB_CORS_ALLOWED_ORIGINS` just to use it — that setting is only for external browser apps calling the gateway directly.

---

## Deployment

### Build Directly From GitHub (No Repo Checkout Needed)

You don't need to clone the repo to build Kongo — Docker can build straight from the GitHub URL. Handy for spinning up a container on a remote host without pulling the source down first.

```bash
docker build -t kongo-stack:latest https://github.com/mardix/kongo.git#main
```

Then run it like any other image:

```bash
docker run -d \
  --name kongo-stack \
  -p 8080:8080 \
  -e KONGODB_ACCESS_KEY=change-me \
  -v kongo_data:/app/data \
  kongo-stack:latest
```

### Build Directly From GitHub With Docker Compose

Same idea, but as a Compose service — `build.context` points at the GitHub URL instead of a local path:

```yaml
# kongo.docker-compose.yaml

services:
  kongo:
    build:
      context: https://github.com/mardix/kongo.git#main
    restart: unless-stopped
    image: kongo:local
    container_name: kongo
    ports:
      - "8080:8080"
    env_file:
      - ./kongodb.env
    environment:
      # Access key for API and Admin UI access. Change this to a secure value in production.
      KONGODB_AUTH_MODE: access_key
      KONGODB_ACCESS_KEY: ${KONGODB_ACCESS_KEY:-change-me}

      # == Storage configuration
      # DATA_DIR is the persistent volume mount point inside the container. It must be writable by the container.
      KONGODB_DATA_DIR: /data

      # Storage: local|s3 -- Local storage is the default and requires a persistent volume. S3 storage requires AWS credentials and an S3 bucket.
      KONGODB_STORAGE_MODE: local

      # S3 storage configuration (only needed if KONGODB_STORAGE_MODE=s3)
      KONGODB_S3_BUCKET:
      KONGODB_S3_ACCESS_KEY:
      KONGODB_S3_SECRET_KEY:
      KONGODB_S3_PREFIX: data/kongo/data
      KONGODB_S3_REGION: us-east-1

      # Runtime  behavior
      # KONGODB_RUNTIME_PROFILE: memory|*balanced|throughput
      KONGODB_RUNTIME_PROFILE: balanced

      # Write behavio
      # KONGODB_WRITE_MODE: committed|accepted
      KONGODB_WRITE_MODE: committed

      # BACKUP_PATH and EXPORT_PATH are used for manual and automatic backups. They can be local paths or S3 paths (s3://bucket/prefix). If using S3, ensure the bucket exists and the container has access to it.
      KONGODB_BACKUP_PATH: /data/backups
      KONGODB_EXPORT_PATH: /data/exports

      # Only needed by separately hosted browser clients; the bundled UI is same-origin.
      # KONGODB_CORS_ALLOWED_ORIGINS:* for any
      KONGODB_CORS_ALLOWED_ORIGINS:
    volumes:
      - kongodb-data:/data

volumes:
  kongodb-data:
```

```bash
docker compose -f kongo.docker-compose.yaml up -d
```

### Building From a Local Checkout

If you do have the repo locally:

```bash
docker run -d \
  --name kongo \
  --restart unless-stopped \
  -p 127.0.0.1:8080:8080 \
  --env-file ./kongo.env.prod \
  -v kongo-data:/data \
  kongo
```

Bind to `127.0.0.1` and put a reverse proxy (Caddy/Nginx/Traefik) in front for public HTTPS.

```bash
docker compose up --build -d
curl http://localhost:8080/_/kdb/ping
```

Compose loads `kongo.env`, mounts a named volume to `/data`, and serves the Admin UI at `/_/kdb/admin/`.

### Cloud Run

Use S3-mode for durability — local container paths are ephemeral cache only:

```env
KONGODB_STORAGE_MODE=s3
KONGODB_DATA_DIR=/tmp/kongo
KONGODB_S3_BUCKET=...
KONGODB_S3_PREFIX=data/kongo/data
KONGODB_S3_REGION=...
KONGODB_S3_ACCESS_KEY=...
KONGODB_S3_SECRET_KEY=...
```

---

## Examples

### Core CRUD

```json
{
  "db": "myapp/main",
  "operation": "insert",
  "namespace": "users",
  "payload": {
    "data": {
      "name": "Ada"
    }
  }
}
```

```json
{
  "db": "myapp/main",
  "operation": "update",
  "namespace": "users",
  "payload": {
    "data": {
      "_id": "u1",
      "name": "Ada L"
    }
  }
}
```

```json
{
  "db": "myapp/main",
  "operation": "get",
  "namespace": "users",
  "payload": {
    "ids": ["u1", "u2"]
  }
}
```

```json
{
  "db": "myapp/main",
  "operation": "query",
  "namespace": "users",
  "payload": {
    "filter": {
      "age": {
        "$gte": 18
      }
    },
    "sort": "age desc",
    "limit": 20
  }
}
```

```json
{
  "db": "myapp/main",
  "operation": "search",
  "namespace": "users",
  "payload": {
    "search": "ada",
    "limit": 10
  }
}
```

### Lifecycle

```json
{
  "db": "myapp/main",
  "operation": "delete",
  "payload": {
    "id": "u1"
  }
}
```

```json
{
  "db": "myapp/main",
  "operation": "set_ttl",
  "namespace": "users",
  "payload": {
    "ids": ["u1"],
    "ttl_seconds": 600
  }
}
```

```json
{
  "db": "myapp/main",
  "operation": "restore_archive",
  "payload": {
    "txn_id": "tx123"
  }
}
```

### Database ops

```json
{
  "db": "myapp/main",
  "operation": "create_db",
  "payload": {}
}
```

```json
{
  "db": "myapp/main",
  "operation": "create_backup",
  "payload": {
    "backup_tag": "nightly"
  }
}
```

```json
{
  "db": "myapp/main",
  "operation": "restore_backup",
  "payload": {
    "backup_tag": "nightly",
    "latest": true
  }
}
```

### Shorthand alias

```json
{
  "db": "test/db02.main",
  "operation": "query::users",
  "payload": {}
}
```

```json
{
  "db": "test/db02.main",
  "operation": "search::users",
  "payload": {
    "search": "ada"
  }
}
```

---

## License

Kongodb is licensed under the **MIT License**.

Copyright (c) 2026 Mardix. All rights reserved.
