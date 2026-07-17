//! Auto-index helpers for heatmap-driven index creation and manual index ops.

use std::collections::HashSet;

use libsql::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

const AUTO_PREFIX: &str = "idx_kdb_auto_";
const MANUAL_PREFIX: &str = "idx_kdb_manual_";

#[derive(Debug, Clone, Serialize, Default)]
pub struct AutoIndexDbReport {
    pub db_path: String,
    pub created_indexes: Vec<String>,
    pub considered_paths: usize,
    pub skipped_due_to_cap: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexInfo {
    pub name: String,
    pub unique: bool,
    pub origin: String,
    pub partial: bool,
}

pub async fn run_auto_index_for_conn(
    conn: &Connection,
    db_path: &str,
    min_hits: i64,
    max_indexes: usize,
    max_new_per_run: usize,
) -> AppResult<AutoIndexDbReport> {
    let existing_auto = count_indexes_with_prefix(conn, AUTO_PREFIX).await?;
    if existing_auto >= max_indexes.max(1) {
        return Ok(AutoIndexDbReport {
            db_path: db_path.to_string(),
            created_indexes: vec![],
            considered_paths: 0,
            skipped_due_to_cap: true,
        });
    }

    let remaining = max_indexes.max(1) - existing_auto;
    let target_new = remaining.min(max_new_per_run.max(1));
    let mut rows = conn
        .query(
            "SELECT path FROM __kdb_query_heatmap
             WHERE is_indexed = 0 AND hit_count >= ?
             ORDER BY hit_count DESC, path ASC
             LIMIT ?",
            libsql::params![min_hits, target_new as i64],
        )
        .await
        .map_err(|e| AppError::Internal(format!("auto-index candidates query failed: {e}")))?;

    let mut paths = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("auto-index candidates read failed: {e}")))?
    {
        let path: String = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("auto-index path decode failed: {e}")))?;
        if is_valid_index_path(&path) {
            paths.push(path);
        }
    }

    let mut created = Vec::new();
    for path in &paths {
        let index_name = make_index_name(AUTO_PREFIX, path);
        let sql = format!(
            "CREATE INDEX IF NOT EXISTS {} ON __kdb_documents((data ->> '$.{}'))",
            index_name, path
        );
        conn.execute(&sql, ()).await.map_err(|e| {
            AppError::Internal(format!("auto-index create failed for {}: {e}", path))
        })?;
        conn.execute(
            "UPDATE __kdb_query_heatmap SET is_indexed = 1 WHERE path = ?",
            libsql::params![path.clone()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("auto-index heatmap update failed: {e}")))?;
        created.push(index_name);
    }

    Ok(AutoIndexDbReport {
        db_path: db_path.to_string(),
        created_indexes: created,
        considered_paths: paths.len(),
        skipped_due_to_cap: false,
    })
}

pub async fn create_manual_index(
    conn: &Connection,
    path: &str,
    name: Option<&str>,
) -> AppResult<String> {
    if !is_valid_index_path(path) {
        return Err(AppError::BadRequest(format!(
            "invalid index path: {path}. expected dot-notation with [a-zA-Z0-9_]"
        )));
    }
    let index_name = if let Some(n) = name {
        validate_index_name(n)?;
        n.to_string()
    } else {
        make_index_name(MANUAL_PREFIX, path)
    };
    let sql = format!(
        "CREATE INDEX IF NOT EXISTS {} ON __kdb_documents((data ->> '$.{}'))",
        index_name, path
    );
    conn.execute(&sql, ())
        .await
        .map_err(|e| AppError::Internal(format!("create_index failed: {e}")))?;
    conn.execute(
        "INSERT INTO __kdb_query_heatmap(path, hit_count, last_hit, is_indexed)
         VALUES (?, 0, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 1)
         ON CONFLICT(path) DO UPDATE SET is_indexed = 1",
        libsql::params![path.to_string()],
    )
    .await
    .map_err(|e| AppError::Internal(format!("create_index heatmap update failed: {e}")))?;
    Ok(index_name)
}

pub async fn drop_index(
    conn: &Connection,
    index_name: Option<&str>,
    index_path: Option<&str>,
) -> AppResult<Vec<String>> {
    let mut targets = Vec::new();
    if let Some(name) = index_name {
        validate_index_name(name)?;
        targets.push(name.to_string());
    } else if let Some(path) = index_path {
        if !is_valid_index_path(path) {
            return Err(AppError::BadRequest(format!("invalid index path: {path}")));
        }
        targets.push(make_index_name(MANUAL_PREFIX, path));
        targets.push(make_index_name(AUTO_PREFIX, path));
    } else {
        return Err(AppError::BadRequest(
            "drop_index requires index_name or index_path".to_string(),
        ));
    }

    let mut dropped = Vec::new();
    for t in dedupe(targets) {
        let sql = format!("DROP INDEX IF EXISTS {}", t);
        conn.execute(&sql, ())
            .await
            .map_err(|e| AppError::Internal(format!("drop_index failed: {e}")))?;
        dropped.push(t);
    }

    if let Some(path) = index_path {
        conn.execute(
            "UPDATE __kdb_query_heatmap SET is_indexed = 0 WHERE path = ?",
            libsql::params![path.to_string()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("drop_index heatmap update failed: {e}")))?;
    }

    Ok(dropped)
}

pub async fn bump_query_heatmap(conn: &Connection, paths: &[String]) -> AppResult<()> {
    if paths.is_empty() {
        return Ok(());
    }
    for p in dedupe(paths.to_vec()) {
        if !is_valid_index_path(&p) {
            continue;
        }
        conn.execute(
            "INSERT INTO __kdb_query_heatmap(path, hit_count, last_hit, is_indexed)
             VALUES (?, 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 0)
             ON CONFLICT(path) DO UPDATE SET
               hit_count = hit_count + 1,
               last_hit = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            libsql::params![p],
        )
        .await
        .map_err(|e| AppError::Internal(format!("heatmap upsert failed: {e}")))?;
    }
    Ok(())
}

pub async fn list_indexes(conn: &Connection) -> AppResult<Vec<IndexInfo>> {
    let mut rows = conn
        .query("PRAGMA index_list('__kdb_documents')", ())
        .await
        .map_err(|e| AppError::Internal(format!("list_indexes failed: {e}")))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("list_indexes read failed: {e}")))?
    {
        let name: String = row
            .get(1)
            .map_err(|e| AppError::Internal(format!("list_indexes decode(name) failed: {e}")))?;
        let unique_i: i64 = row
            .get(2)
            .map_err(|e| AppError::Internal(format!("list_indexes decode(unique) failed: {e}")))?;
        let origin: String = row
            .get(3)
            .map_err(|e| AppError::Internal(format!("list_indexes decode(origin) failed: {e}")))?;
        let partial_i: i64 = row
            .get(4)
            .map_err(|e| AppError::Internal(format!("list_indexes decode(partial) failed: {e}")))?;
        out.push(IndexInfo {
            name,
            unique: unique_i != 0,
            origin,
            partial: partial_i != 0,
        });
    }
    Ok(out)
}

async fn count_indexes_with_prefix(conn: &Connection, prefix: &str) -> AppResult<usize> {
    let mut rows = conn
        .query("PRAGMA index_list('__kdb_documents')", ())
        .await
        .map_err(|e| AppError::Internal(format!("index_list failed: {e}")))?;
    let mut count = 0usize;
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("index_list read failed: {e}")))?
    {
        let name: String = row
            .get(1)
            .map_err(|e| AppError::Internal(format!("index_list decode failed: {e}")))?;
        if name.starts_with(prefix) {
            count += 1;
        }
    }
    Ok(count)
}

pub fn is_valid_index_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    for seg in path.split('.') {
        if seg.is_empty() {
            return false;
        }
        if !seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return false;
        }
    }
    true
}

pub fn make_index_name(prefix: &str, path: &str) -> String {
    let mut h = Sha256::new();
    h.update(path.as_bytes());
    let digest = format!("{:x}", h.finalize());
    format!("{prefix}{}", &digest[..16])
}

fn validate_index_name(name: &str) -> AppResult<()> {
    if name.is_empty() {
        return Err(AppError::BadRequest(
            "index_name cannot be empty".to_string(),
        ));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(AppError::BadRequest(
            "index_name must use [a-zA-Z0-9_]".to_string(),
        ));
    }
    Ok(())
}

fn dedupe(items: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for i in items {
        if seen.insert(i.clone()) {
            out.push(i);
        }
    }
    out
}
