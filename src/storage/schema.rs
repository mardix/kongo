//! SQL schema and trigger bootstrap script executed when opening a new database file.

use libsql::Connection;
use tokio::time::{Duration, sleep};

use crate::error::{AppError, AppResult};

pub const LATEST_SCHEMA_VERSION: i64 = 1;

pub const INIT_SQL: &str = r#"
PRAGMA auto_vacuum = INCREMENTAL;

CREATE TABLE IF NOT EXISTS __kdb_documents (
    id TEXT PRIMARY KEY,
    collection TEXT NOT NULL,
    _user_id TEXT,
    data ANY NOT NULL,
    _size_bytes INTEGER,
    _expires_at INTEGER,
    _expiry_behavior TEXT NOT NULL DEFAULT 'archive' CHECK (_expiry_behavior IN ('archive', 'delete')),
    _created_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    _modified_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE IF NOT EXISTS __kdb_archive (
    id TEXT,
    collection TEXT,
    _user_id TEXT,
    data ANY,
    _size_bytes INTEGER,
    _created_at TEXT,
    _modified_at TEXT,
    _archived_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    _txn_id TEXT NOT NULL,
    _expires_at INTEGER,
    _restore_failed INTEGER DEFAULT 0,
    _restore_reason TEXT
) STRICT;

CREATE TABLE IF NOT EXISTS __kdb_system_stats (
    collection TEXT PRIMARY KEY,
    total_count INTEGER DEFAULT 0,
    total_bytes INTEGER DEFAULT 0
) STRICT;

CREATE TABLE IF NOT EXISTS __kdb_query_heatmap (
    path TEXT PRIMARY KEY,
    hit_count INTEGER DEFAULT 0,
    last_hit TEXT,
    is_indexed INTEGER DEFAULT 0
) STRICT;

CREATE TABLE IF NOT EXISTS __kdb_system_meta (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    schema_version INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    created_by_version TEXT NOT NULL,
    last_migrated_at TEXT NOT NULL,
    last_migrated_by_version TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS __kdb_system_config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE IF NOT EXISTS __kdb_db_stats_rollups (
    ts TEXT PRIMARY KEY,
    requests_total INTEGER NOT NULL DEFAULT 0,
    reads_total INTEGER NOT NULL DEFAULT 0,
    writes_total INTEGER NOT NULL DEFAULT 0,
    errors_total INTEGER NOT NULL DEFAULT 0,
    in_flight INTEGER NOT NULL DEFAULT 0,
    last_accessed_at TEXT
) STRICT;

CREATE TABLE IF NOT EXISTS __kdb_backup_catalog (
    backup_id TEXT PRIMARY KEY,
    backup_db_path TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    sha256 TEXT NOT NULL,
    backup_tag TEXT,
    source TEXT NOT NULL DEFAULT 'manual'
) STRICT;

CREATE TABLE IF NOT EXISTS __kdb_replication_jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    sql TEXT NOT NULL,
    args_json TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE IF NOT EXISTS __kdb_metric_events (
    id TEXT PRIMARY KEY,
    event TEXT NOT NULL,
    ts TEXT NOT NULL,
    tenant_id TEXT,
    user_id TEXT,
    value REAL NOT NULL DEFAULT 1,
    dimensions ANY,
    metadata ANY,
    _created_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    _size_bytes INTEGER
) STRICT;

CREATE TABLE IF NOT EXISTS __kdb_metrics_catalog (
    type TEXT NOT NULL,
    name TEXT NOT NULL,
    value TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (type, name, value)
) STRICT;

CREATE TABLE IF NOT EXISTS __kdb_identity_users (
    id TEXT PRIMARY KEY,
    email TEXT UNIQUE,
    username TEXT UNIQUE,
    phone TEXT,
    first_name TEXT,
    last_name TEXT,
    profile_photo TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    status_reason TEXT,
    status_changed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    status_expires_at TEXT,
    status_next TEXT,
    status_next_reason TEXT,
    status_changed_by TEXT,
    password_hash TEXT,
    password_algo TEXT,
    password_updated_at TEXT,
    requires_password_change INTEGER NOT NULL DEFAULT 0 CHECK (requires_password_change IN (0, 1)),
    email_verified_at TEXT,
    phone_verified_at TEXT,
    last_login_at TEXT,
    data ANY,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    deleted_at TEXT
) STRICT;

CREATE TABLE IF NOT EXISTS __kdb_identity_tokens (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    token_hash TEXT NOT NULL,
    expires_at TEXT,
    used_at TEXT,
    revoked_at TEXT,
    data ANY,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY(user_id) REFERENCES __kdb_identity_users(id) ON DELETE CASCADE
) STRICT;

CREATE TABLE IF NOT EXISTS __kdb_identity_providers (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    provider_user_id TEXT NOT NULL,
    email TEXT,
    data ANY,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(provider, provider_user_id),
    FOREIGN KEY(user_id) REFERENCES __kdb_identity_users(id) ON DELETE CASCADE
) STRICT;

CREATE TABLE IF NOT EXISTS __kdb_identity_events (
    id TEXT PRIMARY KEY,
    user_id TEXT,
    event TEXT NOT NULL,
    data ANY,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE IF NOT EXISTS __kdb_files (
    id TEXT PRIMARY KEY,
    bucket TEXT NOT NULL DEFAULT 'default',
    storage_backend TEXT NOT NULL,
    storage_path TEXT NOT NULL,
    filename TEXT,
    content_type TEXT,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    sha256 TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    owner_type TEXT,
    owner_id TEXT,
    metadata ANY,
    uploaded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    deleted_at TEXT,
    expires_at TEXT
) STRICT;

CREATE INDEX IF NOT EXISTS idx__kdb_backup_catalog_created_at
    ON __kdb_backup_catalog(created_at DESC);
CREATE INDEX IF NOT EXISTS idx__kdb_backup_catalog_backup_tag
    ON __kdb_backup_catalog(backup_tag, created_at DESC)
    WHERE backup_tag IS NOT NULL;

CREATE TABLE IF NOT EXISTS __kdb_jobs (
    job_id TEXT PRIMARY KEY,
    job_type TEXT NOT NULL,
    collection TEXT,
    source_path TEXT,
    source_hash TEXT,
    alias_import_pk TEXT,
    drop_keys_json TEXT,
    on_conflict TEXT,
    ignore_input_id INTEGER NOT NULL DEFAULT 0,
    allow_system_timestamps INTEGER NOT NULL DEFAULT 0,
    batch_size INTEGER,
    payload_json TEXT,
    target_path TEXT,
    compress INTEGER NOT NULL DEFAULT 1,
    requested_backup_db_path TEXT,
    backup_tag TEXT,
    backup_id TEXT,
    backup_db_path TEXT,
    size_bytes INTEGER,
    sha256 TEXT,
    status TEXT NOT NULL,
    resumable INTEGER NOT NULL DEFAULT 0,
    last_error_code TEXT,
    last_error_message TEXT,
    read_count INTEGER NOT NULL DEFAULT 0,
    inserted_count INTEGER NOT NULL DEFAULT 0,
    updated_count INTEGER NOT NULL DEFAULT 0,
    skipped_count INTEGER NOT NULL DEFAULT 0,
    error_count INTEGER NOT NULL DEFAULT 0,
    last_line_no INTEGER NOT NULL DEFAULT 0,
    last_byte_offset INTEGER NOT NULL DEFAULT 0,
    exported_count INTEGER NOT NULL DEFAULT 0,
    bytes_written INTEGER NOT NULL DEFAULT 0,
    part_count INTEGER NOT NULL DEFAULT 0,
    worker_id TEXT,
    lease_expires_at TEXT,
    started_at TEXT,
    finished_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX IF NOT EXISTS idx__kdb_jobs_type_status_created
    ON __kdb_jobs(job_type, status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx__kdb_jobs_status_created
    ON __kdb_jobs(status, created_at DESC);

CREATE INDEX IF NOT EXISTS idx__kdb_replication_jobs_status_id
    ON __kdb_replication_jobs(status, id);

CREATE INDEX IF NOT EXISTS idx__kdb_metric_events_event_ts
    ON __kdb_metric_events(event, ts);
CREATE INDEX IF NOT EXISTS idx__kdb_metric_events_tenant_event_ts
    ON __kdb_metric_events(tenant_id, event, ts)
    WHERE tenant_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx__kdb_metric_events_user_event_ts
    ON __kdb_metric_events(user_id, event, ts)
    WHERE user_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx__kdb_metrics_catalog_type_name
    ON __kdb_metrics_catalog(type, name, value);
CREATE INDEX IF NOT EXISTS idx__kdb_identity_users_status_expires
    ON __kdb_identity_users(status_expires_at)
    WHERE status_expires_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx__kdb_identity_tokens_user_kind_active
    ON __kdb_identity_tokens(user_id, kind, revoked_at, used_at, expires_at);
CREATE INDEX IF NOT EXISTS idx__kdb_identity_tokens_expires
    ON __kdb_identity_tokens(expires_at)
    WHERE expires_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx__kdb_identity_providers_user
    ON __kdb_identity_providers(user_id);
CREATE INDEX IF NOT EXISTS idx__kdb_identity_events_user_created
    ON __kdb_identity_events(user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx__kdb_files_owner
    ON __kdb_files(owner_type, owner_id, status, uploaded_at DESC)
    WHERE owner_type IS NOT NULL AND owner_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx__kdb_files_bucket_status_uploaded
    ON __kdb_files(bucket, status, uploaded_at DESC);
CREATE INDEX IF NOT EXISTS idx__kdb_files_expires
    ON __kdb_files(expires_at)
    WHERE expires_at IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx___kdb_documents_expires_at
    ON __kdb_documents(_expires_at) WHERE _expires_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx___kdb_documents_collection_user
    ON __kdb_documents(collection, _user_id)
    WHERE _user_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx___kdb_archive_expires_at
    ON __kdb_archive(_expires_at) WHERE _expires_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx___kdb_archive_collection_user
    ON __kdb_archive(collection, _user_id)
    WHERE _user_id IS NOT NULL;

DROP TRIGGER IF EXISTS trg_insert;
CREATE TRIGGER trg_insert AFTER INSERT ON __kdb_documents BEGIN
    INSERT INTO __kdb_system_stats (collection, total_count, total_bytes)
    VALUES (NEW.collection, 1, NEW._size_bytes)
    ON CONFLICT(collection) DO UPDATE SET
        total_count = total_count + 1,
        total_bytes = total_bytes + NEW._size_bytes;
END;

DROP TRIGGER IF EXISTS trg_delete;
CREATE TRIGGER trg_delete AFTER DELETE ON __kdb_documents BEGIN
    UPDATE __kdb_system_stats
    SET total_count = total_count - 1,
        total_bytes = total_bytes - OLD._size_bytes
    WHERE collection = OLD.collection;
END;

DROP TRIGGER IF EXISTS trg_update;
CREATE TRIGGER trg_update AFTER UPDATE OF data, _size_bytes ON __kdb_documents BEGIN
    UPDATE __kdb_system_stats
    SET total_bytes = total_bytes - OLD._size_bytes + NEW._size_bytes
    WHERE collection = NEW.collection;
END;
"#;

const AUDIT_LOGS_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS __kdb_audit_logs (
    id TEXT PRIMARY KEY,
    ts TEXT NOT NULL,
    action TEXT NOT NULL,
    actor_type TEXT,
    actor_id TEXT,
    target_type TEXT,
    target_id TEXT,
    status TEXT NOT NULL DEFAULT 'success',
    source TEXT,
    request_id TEXT,
    ip_address TEXT,
    message TEXT,
    data ANY NOT NULL,
    _created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    _size_bytes INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX IF NOT EXISTS idx__kdb_audit_logs_ts
    ON __kdb_audit_logs(ts DESC);
CREATE INDEX IF NOT EXISTS idx__kdb_audit_logs_action_ts
    ON __kdb_audit_logs(action, ts DESC);
CREATE INDEX IF NOT EXISTS idx__kdb_audit_logs_actor_ts
    ON __kdb_audit_logs(actor_type, actor_id, ts DESC)
    WHERE actor_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx__kdb_audit_logs_target_ts
    ON __kdb_audit_logs(target_type, target_id, ts DESC)
    WHERE target_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx__kdb_audit_logs_status_ts
    ON __kdb_audit_logs(status, ts DESC);
"#;

async fn ensure_audit_logs_schema(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(AUDIT_LOGS_SQL)
        .await
        .map(|_| ())
        .map_err(|e| AppError::Internal(format!("audit logs schema init failed: {e}")))
}

/// Ensures the single-row internal metadata record exists for this database.
pub async fn ensure_system_meta(conn: &Connection, default_fts_enabled: bool) -> AppResult<()> {
    let version = env!("CARGO_PKG_VERSION");
    conn.execute(
        "INSERT OR IGNORE INTO __kdb_system_meta (
            id, schema_version, created_at, created_by_version, last_migrated_at, last_migrated_by_version
         ) VALUES (
            1, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?
         )",
        libsql::params![LATEST_SCHEMA_VERSION, version, version],
    )
    .await
    .map_err(|e| AppError::Internal(format!("system meta init failed: {e}")))?;

    conn.execute(
        "INSERT OR IGNORE INTO __kdb_system_config (key, value, updated_at)
         VALUES ('fts_enabled', '0', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        (),
    )
    .await
    .map_err(|e| AppError::Internal(format!("system config init failed: {e}")))?;

    let db_enabled = get_bool_config(conn, "fts_enabled").await?.unwrap_or(false);
    let effective_enabled = default_fts_enabled && db_enabled;
    set_fts_index_enabled(conn, effective_enabled).await?;

    if db_enabled != effective_enabled {
        set_bool_config(conn, "fts_enabled", effective_enabled).await?;
    }

    Ok(())
}

/// Adds identity columns introduced during the current pre-1.0 schema version.
async fn ensure_identity_schema(conn: &Connection) -> AppResult<()> {
    let mut rows = conn
        .query("PRAGMA table_info('__kdb_identity_users')", ())
        .await
        .map_err(|e| AppError::Internal(format!("identity schema inspect failed: {e}")))?;
    let mut has_requires_password_change = false;
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("identity schema row read failed: {e}")))?
    {
        let name: String = row.get(1).map_err(|e| {
            AppError::Internal(format!("identity schema column decode failed: {e}"))
        })?;
        if name == "requires_password_change" {
            has_requires_password_change = true;
            break;
        }
    }
    drop(rows);
    if !has_requires_password_change {
        conn.execute(
            "ALTER TABLE __kdb_identity_users
             ADD COLUMN requires_password_change INTEGER NOT NULL DEFAULT 0
             CHECK (requires_password_change IN (0, 1))",
            (),
        )
        .await
        .map_err(|e| AppError::Internal(format!("identity schema update failed: {e}")))?;
    }
    Ok(())
}

/// Initializes schema and system metadata with retry/backoff for transient sqlite lock contention.
pub async fn init_schema_with_retry(conn: &Connection, default_fts_enabled: bool) -> AppResult<()> {
    const MAX_ATTEMPTS: usize = 8;
    const BASE_BACKOFF_MS: u64 = 25;

    // Keep lock waits bounded for this connection while still allowing brief contention to settle.
    conn.execute_batch("PRAGMA busy_timeout = 5000;")
        .await
        .map_err(|e| AppError::Internal(format!("pragma busy_timeout failed: {e}")))?;

    for attempt in 0..MAX_ATTEMPTS {
        let init_res = conn.execute_batch(INIT_SQL).await;
        match init_res {
            Ok(_) => match async {
                ensure_identity_schema(conn).await?;
                ensure_audit_logs_schema(conn).await?;
                ensure_system_meta(conn, default_fts_enabled).await
            }
            .await
            {
                Ok(_) => return Ok(()),
                Err(e) => {
                    let msg = e.to_string();
                    if !is_lock_or_busy_msg(msg.as_str()) || attempt + 1 == MAX_ATTEMPTS {
                        return Err(e);
                    }
                }
            },
            Err(e) => {
                let msg = e.to_string();
                if !is_lock_or_busy_msg(msg.as_str()) || attempt + 1 == MAX_ATTEMPTS {
                    return Err(AppError::Internal(format!("schema init failed: {e}")));
                }
            }
        }
        let backoff = BASE_BACKOFF_MS.saturating_mul(1u64 << attempt.min(6));
        sleep(Duration::from_millis(backoff)).await;
    }

    Err(AppError::Internal(
        "schema init failed: database remained locked/busy".to_string(),
    ))
}

fn is_lock_or_busy_msg(msg: &str) -> bool {
    let s = msg.to_ascii_lowercase();
    s.contains("database is locked") || s.contains("database is busy") || s.contains("sqlite_busy")
}

#[cfg(test)]
mod tests {
    use super::*;
    use libsql::Builder;
    use uuid::Uuid;

    #[tokio::test]
    async fn identity_schema_adds_password_change_flag_to_existing_table() {
        let path = std::env::temp_dir().join(format!(
            "kongodb_identity_schema_{}.db",
            Uuid::new_v4().simple()
        ));
        let db = Builder::new_local(&path).build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute_batch(
            "CREATE TABLE __kdb_identity_users (
                id TEXT PRIMARY KEY
             ) STRICT;",
        )
        .await
        .unwrap();

        ensure_identity_schema(&conn).await.unwrap();
        ensure_identity_schema(&conn).await.unwrap();
        conn.execute(
            "INSERT INTO __kdb_identity_users (id) VALUES ('user-1')",
            (),
        )
        .await
        .unwrap();
        let mut rows = conn
            .query(
                "SELECT requires_password_change FROM __kdb_identity_users WHERE id = 'user-1'",
                (),
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(row.get::<i64>(0).unwrap(), 0);

        drop(rows);
        drop(conn);
        drop(db);
        let _ = tokio::fs::remove_file(path).await;
    }

    #[tokio::test]
    async fn blank_database_initializes_audit_logs_schema() {
        let path = std::env::temp_dir().join(format!(
            "kongodb_audit_schema_{}.db",
            Uuid::new_v4().simple()
        ));
        let db = Builder::new_local(&path).build().await.unwrap();
        let conn = db.connect().unwrap();

        init_schema_with_retry(&conn, false).await.unwrap();
        let mut rows = conn
            .query(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = '__kdb_audit_logs'",
                (),
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(row.get::<String>(0).unwrap(), "__kdb_audit_logs");

        drop(rows);
        drop(conn);
        drop(db);
        let _ = tokio::fs::remove_file(path).await;
    }
}

pub async fn get_bool_config(conn: &Connection, key: &str) -> AppResult<Option<bool>> {
    let mut rows = conn
        .query(
            "SELECT value FROM __kdb_system_config WHERE key = ? LIMIT 1",
            libsql::params![key],
        )
        .await
        .map_err(|e| AppError::Internal(format!("system config read failed: {e}")))?;
    if let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("system config row read failed: {e}")))?
    {
        let raw: String = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("system config row decode failed: {e}")))?;
        let normalized = raw.trim().to_ascii_lowercase();
        let enabled = normalized == "1" || normalized == "true";
        return Ok(Some(enabled));
    }
    Ok(None)
}

pub async fn set_bool_config(conn: &Connection, key: &str, value: bool) -> AppResult<()> {
    conn.execute(
        "INSERT INTO __kdb_system_config (key, value, updated_at)
         VALUES (?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         ON CONFLICT(key) DO UPDATE SET
           value = excluded.value,
           updated_at = excluded.updated_at",
        libsql::params![key, if value { "1" } else { "0" }],
    )
    .await
    .map_err(|e| AppError::Internal(format!("system config write failed: {e}")))?;
    Ok(())
}

pub async fn set_fts_index_enabled(conn: &Connection, enabled: bool) -> AppResult<()> {
    if enabled {
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS __kdb_documents_fts USING fts5(id UNINDEXED, content);
             CREATE TRIGGER IF NOT EXISTS trg_fts_insert AFTER INSERT ON __kdb_documents BEGIN
               INSERT INTO __kdb_documents_fts(rowid, id, content) VALUES (NEW.rowid, NEW.id, json(NEW.data));
             END;
             CREATE TRIGGER IF NOT EXISTS trg_fts_delete AFTER DELETE ON __kdb_documents BEGIN
               DELETE FROM __kdb_documents_fts WHERE rowid = OLD.rowid;
             END;
             CREATE TRIGGER IF NOT EXISTS trg_fts_update AFTER UPDATE OF data, id ON __kdb_documents BEGIN
               DELETE FROM __kdb_documents_fts WHERE rowid = OLD.rowid;
               INSERT INTO __kdb_documents_fts(rowid, id, content) VALUES (NEW.rowid, NEW.id, json(NEW.data));
             END;",
        )
        .await
        .map_err(|e| AppError::Internal(format!("enable fts index failed: {e}")))?;
    } else {
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS trg_fts_insert;
             DROP TRIGGER IF EXISTS trg_fts_delete;
             DROP TRIGGER IF EXISTS trg_fts_update;
             DROP TABLE IF EXISTS __kdb_documents_fts;",
        )
        .await
        .map_err(|e| AppError::Internal(format!("disable fts index failed: {e}")))?;
    }
    Ok(())
}

pub async fn reindex_fts(conn: &Connection) -> AppResult<u64> {
    set_fts_index_enabled(conn, true).await?;
    conn.execute("DELETE FROM __kdb_documents_fts", ())
        .await
        .map_err(|e| AppError::Internal(format!("fts clear failed: {e}")))?;
    conn.execute(
        "INSERT INTO __kdb_documents_fts(rowid, id, content)
         SELECT rowid, id, json(data) FROM __kdb_documents",
        (),
    )
    .await
    .map_err(|e| AppError::Internal(format!("fts reindex failed: {e}")))
}
