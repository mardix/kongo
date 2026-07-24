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
    deleted_live_documents: u64,
}

impl ReaperStats {
    pub fn has_changes(&self) -> bool {
        self.moved_to_archive > 0
            || self.deleted_from_archive > 0
            || self.deleted_metric_events > 0
            || self.transitioned_identity_statuses > 0
            || self.deleted_identity_tokens > 0
            || self.deleted_live_documents > 0
    }

    pub fn merge(&mut self, other: Self) {
        self.moved_to_archive = self.moved_to_archive.saturating_add(other.moved_to_archive);
        self.deleted_from_archive = self
            .deleted_from_archive
            .saturating_add(other.deleted_from_archive);
        self.deleted_metric_events = self
            .deleted_metric_events
            .saturating_add(other.deleted_metric_events);
        self.transitioned_identity_statuses = self
            .transitioned_identity_statuses
            .saturating_add(other.transitioned_identity_statuses);
        self.deleted_identity_tokens = self
            .deleted_identity_tokens
            .saturating_add(other.deleted_identity_tokens);
        self.deleted_live_documents = self
            .deleted_live_documents
            .saturating_add(other.deleted_live_documents);
    }
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

    let deleted_live_documents = tx
        .execute(
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

    let stats = ReaperStats {
        moved_to_archive: moved,
        deleted_from_archive: deleted_kdb_archive,
        deleted_metric_events,
        transitioned_identity_statuses,
        deleted_identity_tokens,
        deleted_live_documents,
    };

    if stats.has_changes() {
        conn.execute("PRAGMA incremental_vacuum", ())
            .await
            .map_err(|e| AppError::Internal(format!("reaper incremental vacuum failed: {e}")))?;
    }

    Ok(stats)
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::ReaperStats;

    #[test]
    fn empty_reaper_stats_have_no_changes() {
        assert!(!ReaperStats::default().has_changes());
    }

    #[test]
    fn hard_deleted_documents_count_as_changes() {
        let stats = ReaperStats {
            deleted_live_documents: 1,
            ..ReaperStats::default()
        };
        assert!(stats.has_changes());
    }

    #[test]
    fn merge_preserves_all_change_categories() {
        let mut total = ReaperStats {
            moved_to_archive: 2,
            transitioned_identity_statuses: 1,
            ..ReaperStats::default()
        };
        total.merge(ReaperStats {
            deleted_from_archive: 3,
            deleted_identity_tokens: 4,
            deleted_live_documents: 5,
            ..ReaperStats::default()
        });

        assert_eq!(total.moved_to_archive, 2);
        assert_eq!(total.deleted_from_archive, 3);
        assert_eq!(total.transitioned_identity_statuses, 1);
        assert_eq!(total.deleted_identity_tokens, 4);
        assert_eq!(total.deleted_live_documents, 5);
        assert!(total.has_changes());
    }
}
