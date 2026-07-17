//! Internal instance catalog used to track DB inventory, stats snapshots, and lifecycle events.

use std::{path::PathBuf, sync::Arc};

use chrono::{SecondsFormat, Utc};
use libsql::{Builder, Connection};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::error::{AppError, AppResult};

const SYSTEM_DB_FILENAME: &str = "__kdb_system.db";

const INIT_SQL: &str = r#"
PRAGMA auto_vacuum = INCREMENTAL;
PRAGMA busy_timeout = 5000;

CREATE TABLE IF NOT EXISTS __kdb_system_dbs (
    db TEXT PRIMARY KEY,
    status TEXT NOT NULL DEFAULT 'known',
    storage_mode TEXT NOT NULL,
    on_local INTEGER NOT NULL DEFAULT 0,
    on_s3 INTEGER NOT NULL DEFAULT 0,
    loaded INTEGER NOT NULL DEFAULT 0,
    active INTEGER NOT NULL DEFAULT 0,
    local_size_bytes INTEGER,
    remote_size_bytes INTEGER,
    namespace_count INTEGER,
    document_count INTEGER,
    archive_count INTEGER,
    pending_write_count INTEGER NOT NULL DEFAULT 0,
    write_queue_depth INTEGER NOT NULL DEFAULT 0,
    last_opened_at TEXT,
    last_closed_at TEXT,
    last_read_at TEXT,
    last_write_at TEXT,
    last_sync_at TEXT,
    last_backup_at TEXT,
    last_reaper_at TEXT,
    last_vacuum_at TEXT,
    last_error_at TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE IF NOT EXISTS __kdb_system_db_stats (
    id TEXT PRIMARY KEY,
    db TEXT NOT NULL,
    ts TEXT NOT NULL,
    requests_total INTEGER NOT NULL DEFAULT 0,
    reads_total INTEGER NOT NULL DEFAULT 0,
    writes_total INTEGER NOT NULL DEFAULT 0,
    errors_total INTEGER NOT NULL DEFAULT 0,
    in_flight INTEGER NOT NULL DEFAULT 0,
    local_size_bytes INTEGER,
    namespace_count INTEGER,
    document_count INTEGER,
    archive_count INTEGER,
    write_queue_depth INTEGER NOT NULL DEFAULT 0,
    metadata TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE IF NOT EXISTS __kdb_system_db_events (
    id TEXT PRIMARY KEY,
    db TEXT,
    ts TEXT NOT NULL,
    event TEXT NOT NULL,
    level TEXT NOT NULL DEFAULT 'info',
    message TEXT,
    metadata TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX IF NOT EXISTS idx__kdb_system_dbs_updated_at
    ON __kdb_system_dbs(updated_at DESC);
CREATE INDEX IF NOT EXISTS idx__kdb_system_db_stats_db_ts
    ON __kdb_system_db_stats(db, ts DESC);
CREATE INDEX IF NOT EXISTS idx__kdb_system_db_events_db_ts
    ON __kdb_system_db_events(db, ts DESC);
"#;

#[derive(Clone)]
pub struct SystemCatalog {
    path: Arc<PathBuf>,
    conn: Arc<Mutex<Option<Connection>>>,
}

#[derive(Debug, Clone)]
pub struct SystemDbRecord {
    pub db: String,
    pub status: String,
    pub storage_mode: String,
    pub on_local: bool,
    pub on_s3: bool,
    pub loaded: bool,
    pub active: bool,
    pub local_size_bytes: Option<u64>,
    pub remote_size_bytes: Option<u64>,
    pub namespace_count: Option<i64>,
    pub document_count: Option<i64>,
    pub archive_count: Option<i64>,
    pub pending_write_count: usize,
    pub write_queue_depth: usize,
    pub last_opened_at: Option<String>,
    pub last_closed_at: Option<String>,
    pub last_read_at: Option<String>,
    pub last_write_at: Option<String>,
    pub last_sync_at: Option<String>,
    pub last_backup_at: Option<String>,
    pub last_reaper_at: Option<String>,
    pub last_vacuum_at: Option<String>,
    pub last_error_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SystemDbStatsRecord {
    pub db: String,
    pub ts: String,
    pub requests_total: u64,
    pub reads_total: u64,
    pub writes_total: u64,
    pub errors_total: u64,
    pub in_flight: u64,
    pub local_size_bytes: Option<u64>,
    pub namespace_count: Option<i64>,
    pub document_count: Option<i64>,
    pub archive_count: Option<i64>,
    pub write_queue_depth: usize,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct SystemDbEventRecord {
    pub db: Option<String>,
    pub event: String,
    pub level: String,
    pub message: Option<String>,
    pub metadata: Option<Value>,
}

impl SystemCatalog {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: Arc::new(data_dir.into().join(SYSTEM_DB_FILENAME)),
            conn: Arc::new(Mutex::new(None)),
        }
    }

    async fn connection(&self) -> AppResult<Connection> {
        let mut guard = self.conn.lock().await;
        if let Some(conn) = guard.as_ref() {
            return Ok(conn.clone());
        }
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                AppError::Internal(format!("create system catalog dir failed: {e}"))
            })?;
        }
        let db = Builder::new_local(self.path.as_ref())
            .build()
            .await
            .map_err(|e| AppError::Internal(format!("open system catalog failed: {e}")))?;
        let conn = db
            .connect()
            .map_err(|e| AppError::Internal(format!("connect system catalog failed: {e}")))?;
        conn.execute_batch(INIT_SQL)
            .await
            .map_err(|e| AppError::Internal(format!("init system catalog failed: {e}")))?;
        *guard = Some(conn.clone());
        Ok(conn)
    }

    pub async fn upsert_db(&self, record: &SystemDbRecord) -> AppResult<()> {
        let conn = self.connection().await?;
        conn.execute(
            "INSERT INTO __kdb_system_dbs (
                db, status, storage_mode, on_local, on_s3, loaded, active,
                local_size_bytes, remote_size_bytes, namespace_count, document_count, archive_count,
                pending_write_count, write_queue_depth, last_opened_at, last_closed_at,
                last_read_at, last_write_at, last_sync_at, last_backup_at, last_reaper_at,
                last_vacuum_at, last_error_at, last_error, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(db) DO UPDATE SET
                status = excluded.status,
                storage_mode = excluded.storage_mode,
                on_local = excluded.on_local,
                on_s3 = excluded.on_s3,
                loaded = excluded.loaded,
                active = excluded.active,
                local_size_bytes = excluded.local_size_bytes,
                remote_size_bytes = excluded.remote_size_bytes,
                namespace_count = COALESCE(excluded.namespace_count, __kdb_system_dbs.namespace_count),
                document_count = COALESCE(excluded.document_count, __kdb_system_dbs.document_count),
                archive_count = COALESCE(excluded.archive_count, __kdb_system_dbs.archive_count),
                pending_write_count = excluded.pending_write_count,
                write_queue_depth = excluded.write_queue_depth,
                last_opened_at = COALESCE(excluded.last_opened_at, __kdb_system_dbs.last_opened_at),
                last_closed_at = COALESCE(excluded.last_closed_at, __kdb_system_dbs.last_closed_at),
                last_read_at = COALESCE(excluded.last_read_at, __kdb_system_dbs.last_read_at),
                last_write_at = COALESCE(excluded.last_write_at, __kdb_system_dbs.last_write_at),
                last_sync_at = COALESCE(excluded.last_sync_at, __kdb_system_dbs.last_sync_at),
                last_backup_at = COALESCE(excluded.last_backup_at, __kdb_system_dbs.last_backup_at),
                last_reaper_at = COALESCE(excluded.last_reaper_at, __kdb_system_dbs.last_reaper_at),
                last_vacuum_at = COALESCE(excluded.last_vacuum_at, __kdb_system_dbs.last_vacuum_at),
                last_error_at = excluded.last_error_at,
                last_error = excluded.last_error,
                updated_at = excluded.updated_at",
            libsql::params![
                record.db.clone(),
                record.status.clone(),
                record.storage_mode.clone(),
                bool_i64(record.on_local),
                bool_i64(record.on_s3),
                bool_i64(record.loaded),
                bool_i64(record.active),
                opt_u64_i64(record.local_size_bytes, "local_size_bytes")?,
                opt_u64_i64(record.remote_size_bytes, "remote_size_bytes")?,
                record.namespace_count,
                record.document_count,
                record.archive_count,
                usize_i64(record.pending_write_count, "pending_write_count")?,
                usize_i64(record.write_queue_depth, "write_queue_depth")?,
                record.last_opened_at.clone(),
                record.last_closed_at.clone(),
                record.last_read_at.clone(),
                record.last_write_at.clone(),
                record.last_sync_at.clone(),
                record.last_backup_at.clone(),
                record.last_reaper_at.clone(),
                record.last_vacuum_at.clone(),
                record.last_error_at.clone(),
                record.last_error.clone(),
                now_rfc3339()
            ],
        )
        .await
        .map_err(|e| AppError::Internal(format!("system catalog db upsert failed: {e}")))?;
        Ok(())
    }

    pub async fn insert_stats(&self, record: &SystemDbStatsRecord) -> AppResult<()> {
        let conn = self.connection().await?;
        conn.execute(
            "INSERT INTO __kdb_system_db_stats (
                id, db, ts, requests_total, reads_total, writes_total, errors_total,
                in_flight, local_size_bytes, namespace_count, document_count, archive_count,
                write_queue_depth, metadata
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            libsql::params![
                Uuid::new_v4().simple().to_string(),
                record.db.clone(),
                record.ts.clone(),
                u64_i64(record.requests_total, "requests_total")?,
                u64_i64(record.reads_total, "reads_total")?,
                u64_i64(record.writes_total, "writes_total")?,
                u64_i64(record.errors_total, "errors_total")?,
                u64_i64(record.in_flight, "in_flight")?,
                opt_u64_i64(record.local_size_bytes, "local_size_bytes")?,
                record.namespace_count,
                record.document_count,
                record.archive_count,
                usize_i64(record.write_queue_depth, "write_queue_depth")?,
                record.metadata.as_ref().map(|v| v.to_string())
            ],
        )
        .await
        .map_err(|e| AppError::Internal(format!("system catalog stats insert failed: {e}")))?;
        Ok(())
    }

    pub async fn insert_event(&self, record: &SystemDbEventRecord) -> AppResult<()> {
        let conn = self.connection().await?;
        conn.execute(
            "INSERT INTO __kdb_system_db_events (id, db, ts, event, level, message, metadata)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            libsql::params![
                Uuid::new_v4().simple().to_string(),
                record.db.clone(),
                now_rfc3339(),
                record.event.clone(),
                record.level.clone(),
                record.message.clone(),
                record.metadata.as_ref().map(|v| v.to_string())
            ],
        )
        .await
        .map_err(|e| AppError::Internal(format!("system catalog event insert failed: {e}")))?;
        Ok(())
    }

    pub async fn list_dbs(&self, limit: i64, offset: i64) -> AppResult<Vec<Value>> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "SELECT db, status, storage_mode, on_local, on_s3, loaded, active,
                        local_size_bytes, remote_size_bytes, namespace_count, document_count,
                        archive_count, pending_write_count, write_queue_depth, last_opened_at,
                        last_closed_at, last_read_at, last_write_at, last_sync_at, last_backup_at,
                        last_reaper_at, last_vacuum_at, last_error_at, last_error, created_at, updated_at
                 FROM __kdb_system_dbs
                 ORDER BY updated_at DESC, db ASC
                 LIMIT ? OFFSET ?",
                libsql::params![limit, offset],
            )
            .await
            .map_err(|e| AppError::Internal(format!("system catalog list dbs failed: {e}")))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("system catalog db row failed: {e}")))?
        {
            items.push(db_row_to_json(&row)?);
        }
        Ok(items)
    }

    pub async fn get_db(&self, db: &str) -> AppResult<Option<Value>> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "SELECT db, status, storage_mode, on_local, on_s3, loaded, active,
                        local_size_bytes, remote_size_bytes, namespace_count, document_count,
                        archive_count, pending_write_count, write_queue_depth, last_opened_at,
                        last_closed_at, last_read_at, last_write_at, last_sync_at, last_backup_at,
                        last_reaper_at, last_vacuum_at, last_error_at, last_error, created_at, updated_at
                 FROM __kdb_system_dbs
                 WHERE db = ?
                 LIMIT 1",
                libsql::params![db],
            )
            .await
            .map_err(|e| AppError::Internal(format!("system catalog get db failed: {e}")))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("system catalog db row failed: {e}")))?
        else {
            return Ok(None);
        };
        Ok(Some(db_row_to_json(&row)?))
    }

    pub async fn query_stats(
        &self,
        db: Option<&str>,
        start: Option<String>,
        end: Option<String>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<Value>> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "SELECT id, db, ts, requests_total, reads_total, writes_total, errors_total,
                        in_flight, local_size_bytes, namespace_count, document_count, archive_count,
                        write_queue_depth, metadata, created_at
                 FROM __kdb_system_db_stats
                 WHERE (? IS NULL OR db = ?)
                   AND (? IS NULL OR ts >= ?)
                   AND (? IS NULL OR ts <= ?)
                 ORDER BY ts DESC
                 LIMIT ? OFFSET ?",
                libsql::params![
                    db.map(str::to_string),
                    db.map(str::to_string),
                    start.clone(),
                    start,
                    end.clone(),
                    end,
                    limit,
                    offset
                ],
            )
            .await
            .map_err(|e| AppError::Internal(format!("system catalog query stats failed: {e}")))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("system catalog stats row failed: {e}")))?
        {
            items.push(stats_row_to_json(&row)?);
        }
        Ok(items)
    }

    pub async fn list_events(
        &self,
        db: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<Value>> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "SELECT id, db, ts, event, level, message, metadata, created_at
                 FROM __kdb_system_db_events
                 WHERE (? IS NULL OR db = ?)
                 ORDER BY ts DESC
                 LIMIT ? OFFSET ?",
                libsql::params![
                    db.map(str::to_string),
                    db.map(str::to_string),
                    limit,
                    offset
                ],
            )
            .await
            .map_err(|e| AppError::Internal(format!("system catalog list events failed: {e}")))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("system catalog event row failed: {e}")))?
        {
            items.push(event_row_to_json(&row)?);
        }
        Ok(items)
    }

    pub async fn prune(&self, retention_days: u64) -> AppResult<()> {
        if retention_days == 0 {
            return Ok(());
        }
        let conn = self.connection().await?;
        let days = i64::try_from(retention_days)
            .map_err(|_| AppError::BadRequest("system retention days is too large".to_string()))?;
        conn.execute(
            "DELETE FROM __kdb_system_db_stats
             WHERE ts < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ? || ' days')",
            libsql::params![-days],
        )
        .await
        .map_err(|e| AppError::Internal(format!("system catalog stats prune failed: {e}")))?;
        conn.execute(
            "DELETE FROM __kdb_system_db_events
             WHERE ts < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ? || ' days')",
            libsql::params![-days],
        )
        .await
        .map_err(|e| AppError::Internal(format!("system catalog events prune failed: {e}")))?;
        Ok(())
    }
}

fn db_row_to_json(row: &libsql::Row) -> AppResult<Value> {
    Ok(json!({
        "db": row.get::<String>(0).map_err(decode_err)?,
        "status": row.get::<String>(1).map_err(decode_err)?,
        "storage_mode": row.get::<String>(2).map_err(decode_err)?,
        "on_local": row.get::<i64>(3).map_err(decode_err)? != 0,
        "on_s3": row.get::<i64>(4).map_err(decode_err)? != 0,
        "loaded": row.get::<i64>(5).map_err(decode_err)? != 0,
        "active": row.get::<i64>(6).map_err(decode_err)? != 0,
        "local_size_bytes": row.get::<Option<i64>>(7).map_err(decode_err)?,
        "remote_size_bytes": row.get::<Option<i64>>(8).map_err(decode_err)?,
        "namespace_count": row.get::<Option<i64>>(9).map_err(decode_err)?,
        "document_count": row.get::<Option<i64>>(10).map_err(decode_err)?,
        "archive_count": row.get::<Option<i64>>(11).map_err(decode_err)?,
        "pending_write_count": row.get::<i64>(12).map_err(decode_err)?,
        "write_queue_depth": row.get::<i64>(13).map_err(decode_err)?,
        "last_opened_at": row.get::<Option<String>>(14).map_err(decode_err)?,
        "last_closed_at": row.get::<Option<String>>(15).map_err(decode_err)?,
        "last_read_at": row.get::<Option<String>>(16).map_err(decode_err)?,
        "last_write_at": row.get::<Option<String>>(17).map_err(decode_err)?,
        "last_sync_at": row.get::<Option<String>>(18).map_err(decode_err)?,
        "last_backup_at": row.get::<Option<String>>(19).map_err(decode_err)?,
        "last_reaper_at": row.get::<Option<String>>(20).map_err(decode_err)?,
        "last_vacuum_at": row.get::<Option<String>>(21).map_err(decode_err)?,
        "last_error_at": row.get::<Option<String>>(22).map_err(decode_err)?,
        "last_error": row.get::<Option<String>>(23).map_err(decode_err)?,
        "created_at": row.get::<String>(24).map_err(decode_err)?,
        "updated_at": row.get::<String>(25).map_err(decode_err)?
    }))
}

fn stats_row_to_json(row: &libsql::Row) -> AppResult<Value> {
    let metadata_raw: Option<String> = row.get(13).map_err(decode_err)?;
    Ok(json!({
        "id": row.get::<String>(0).map_err(decode_err)?,
        "db": row.get::<String>(1).map_err(decode_err)?,
        "ts": row.get::<String>(2).map_err(decode_err)?,
        "requests_total": row.get::<i64>(3).map_err(decode_err)?,
        "reads_total": row.get::<i64>(4).map_err(decode_err)?,
        "writes_total": row.get::<i64>(5).map_err(decode_err)?,
        "errors_total": row.get::<i64>(6).map_err(decode_err)?,
        "in_flight": row.get::<i64>(7).map_err(decode_err)?,
        "local_size_bytes": row.get::<Option<i64>>(8).map_err(decode_err)?,
        "namespace_count": row.get::<Option<i64>>(9).map_err(decode_err)?,
        "document_count": row.get::<Option<i64>>(10).map_err(decode_err)?,
        "archive_count": row.get::<Option<i64>>(11).map_err(decode_err)?,
        "write_queue_depth": row.get::<i64>(12).map_err(decode_err)?,
        "metadata": parse_json_opt(metadata_raw)?,
        "created_at": row.get::<String>(14).map_err(decode_err)?
    }))
}

fn event_row_to_json(row: &libsql::Row) -> AppResult<Value> {
    let metadata_raw: Option<String> = row.get(6).map_err(decode_err)?;
    Ok(json!({
        "id": row.get::<String>(0).map_err(decode_err)?,
        "db": row.get::<Option<String>>(1).map_err(decode_err)?,
        "ts": row.get::<String>(2).map_err(decode_err)?,
        "event": row.get::<String>(3).map_err(decode_err)?,
        "level": row.get::<String>(4).map_err(decode_err)?,
        "message": row.get::<Option<String>>(5).map_err(decode_err)?,
        "metadata": parse_json_opt(metadata_raw)?,
        "created_at": row.get::<String>(7).map_err(decode_err)?
    }))
}

fn parse_json_opt(raw: Option<String>) -> AppResult<Value> {
    match raw {
        Some(v) => serde_json::from_str(&v)
            .map_err(|e| AppError::Internal(format!("system catalog json decode failed: {e}"))),
        None => Ok(Value::Null),
    }
}

fn bool_i64(v: bool) -> i64 {
    if v { 1 } else { 0 }
}

fn opt_u64_i64(value: Option<u64>, field: &str) -> AppResult<Option<i64>> {
    value.map(|v| u64_i64(v, field)).transpose()
}

fn u64_i64(value: u64, field: &str) -> AppResult<i64> {
    i64::try_from(value).map_err(|_| AppError::Internal(format!("{field} is too large to persist")))
}

fn usize_i64(value: usize, field: &str) -> AppResult<i64> {
    i64::try_from(value).map_err(|_| AppError::Internal(format!("{field} is too large to persist")))
}

fn decode_err(e: libsql::Error) -> AppError {
    AppError::Internal(format!("system catalog row decode failed: {e}"))
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
