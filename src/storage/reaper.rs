//! TTL reaper logic: expires live docs into archive and prunes expired archive rows.

use std::time::{SystemTime, UNIX_EPOCH};

use libsql::Connection;
use uuid::Uuid;

use crate::error::{AppError, AppResult};

#[derive(Debug, Default, Clone, Copy)]
pub struct ReaperStats {
    pub moved_to_archive: u64,
    pub deleted_from_archive: u64,
    pub deleted_metric_events: u64,
    pub transitioned_identity_statuses: u64,
    pub deleted_identity_tokens: u64,
}

pub async fn reap_conn(
    conn: &Connection,
    __kdb_archive_ttl_secs: Option<u64>,
    metric_events_retention_days: Option<u64>,
) -> AppResult<ReaperStats> {
    let now = unix_now_secs() as i64;
    let __kdb_archive_expires_at = __kdb_archive_ttl_secs.map(|s| now + s as i64);
    let txn_id = Uuid::new_v4().simple().to_string();

    let tx = conn
        .transaction()
        .await
        .map_err(|e| AppError::Internal(format!("reaper tx begin failed: {e}")))?;

    let moved = tx
        .execute(
            "INSERT INTO __kdb_archive (id, collection, data, _size_bytes, _created_at, _modified_at, _archived_at, _txn_id, _expires_at)
             SELECT id, collection, data, _size_bytes, _created_at, _modified_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?, ?
             FROM __kdb_documents
             WHERE _expires_at IS NOT NULL
               AND _expires_at <= ?
               AND lower(coalesce(_expiry_behavior, 'archive')) <> 'delete'",
            libsql::params![txn_id, __kdb_archive_expires_at, now],
        )
        .await
        .map_err(|e| AppError::Internal(format!("reaper __kdb_archive move failed: {e}")))?;

    tx.execute(
        "DELETE FROM __kdb_documents
         WHERE _expires_at IS NOT NULL
           AND _expires_at <= ?
           AND lower(coalesce(_expiry_behavior, 'archive')) <> 'delete'",
        libsql::params![now],
    )
    .await
    .map_err(|e| AppError::Internal(format!("reaper live __kdb_archive-delete failed: {e}")))?;

    tx.execute(
        "DELETE FROM __kdb_documents
         WHERE _expires_at IS NOT NULL
           AND _expires_at <= ?
           AND lower(coalesce(_expiry_behavior, 'archive')) = 'delete'",
        libsql::params![now],
    )
    .await
    .map_err(|e| AppError::Internal(format!("reaper live hard-delete failed: {e}")))?;

    let deleted_kdb_archive = tx
        .execute(
            "DELETE FROM __kdb_archive WHERE _expires_at IS NOT NULL AND _expires_at <= ?",
            libsql::params![now],
        )
        .await
        .map_err(|e| AppError::Internal(format!("reaper __kdb_archive delete failed: {e}")))?;

    let deleted_metric_events = if let Some(days) = metric_events_retention_days.filter(|d| *d > 0)
    {
        let cutoff = format!("-{} days", days);
        tx.execute(
            "DELETE FROM __kdb_metric_events
             WHERE ts < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?)",
            libsql::params![cutoff],
        )
        .await
        .map_err(|e| AppError::Internal(format!("reaper metric_events delete failed: {e}")))?
    } else {
        0
    };

    let mut expired_status_rows = tx
        .query(
            "SELECT id, status, status_next, status_next_reason
             FROM __kdb_identity_users
             WHERE status_expires_at IS NOT NULL
               AND status_expires_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
               AND status_next IS NOT NULL",
            (),
        )
        .await
        .map_err(|e| AppError::Internal(format!("reaper identity status query failed: {e}")))?;
    let mut status_events = Vec::<(String, String, String, Option<String>)>::new();
    while let Some(row) = expired_status_rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("reaper identity status row failed: {e}")))?
    {
        status_events.push((
            row.get::<String>(0).map_err(|e| {
                AppError::Internal(format!("reaper identity id decode failed: {e}"))
            })?,
            row.get::<String>(1).map_err(|e| {
                AppError::Internal(format!("reaper identity status decode failed: {e}"))
            })?,
            row.get::<String>(2).map_err(|e| {
                AppError::Internal(format!("reaper identity next status decode failed: {e}"))
            })?,
            row.get::<Option<String>>(3).map_err(|e| {
                AppError::Internal(format!("reaper identity next reason decode failed: {e}"))
            })?,
        ));
    }
    drop(expired_status_rows);

    let transitioned_identity_statuses = tx
        .execute(
            "UPDATE __kdb_identity_users
             SET status = status_next,
                 status_reason = status_next_reason,
                 status_changed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 status_expires_at = NULL,
                 status_next = NULL,
                 status_next_reason = NULL,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE status_expires_at IS NOT NULL
               AND status_expires_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
               AND status_next IS NOT NULL",
            (),
        )
        .await
        .map_err(|e| AppError::Internal(format!("reaper identity status update failed: {e}")))?;

    for (user_id, previous_status, status, reason) in status_events {
        let data = serde_json::json!({
            "previous_status": previous_status,
            "status": status,
            "reason": reason,
            "source": "reaper"
        });
        tx.execute(
            "INSERT INTO __kdb_identity_events (id, user_id, event, data)
             VALUES (?, ?, 'user.status_transitioned', json(?))",
            libsql::params![
                Uuid::new_v4().simple().to_string(),
                user_id,
                data.to_string()
            ],
        )
        .await
        .map_err(|e| AppError::Internal(format!("reaper identity event insert failed: {e}")))?;
    }

    let deleted_identity_tokens = tx
        .execute(
            "DELETE FROM __kdb_identity_tokens
             WHERE expires_at IS NOT NULL
               AND expires_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            (),
        )
        .await
        .map_err(|e| AppError::Internal(format!("reaper identity token delete failed: {e}")))?;

    tx.commit()
        .await
        .map_err(|e| AppError::Internal(format!("reaper tx commit failed: {e}")))?;

    conn.execute("PRAGMA incremental_vacuum", ())
        .await
        .map_err(|e| AppError::Internal(format!("reaper incremental vacuum failed: {e}")))?;

    Ok(ReaperStats {
        moved_to_archive: moved,
        deleted_from_archive: deleted_kdb_archive,
        deleted_metric_events,
        transitioned_identity_statuses,
        deleted_identity_tokens,
    })
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
