//! Backend selector and unified storage manager facade used by the service layer.

use crate::{
    config::{StorageConfig, StorageMode},
    error::AppResult,
    storage::{
        auto_index::AutoIndexDbReport,
        backup::AutoBackupCycleReport,
        local::LocalEngine,
        reaper::ReaperStats,
        s3_wal::{
            CompactWalResult, DbSnapshotsList, RemoteSyncReport, ReplicationFlushReport,
            S3WalEngine, SyncDbResult, SyncStatus, VerifyDbResult,
        },
    },
};

pub struct MultiDbManager {
    backend: StorageBackend,
    max_active_dbs: usize,
}

enum StorageBackend {
    Local(LocalEngine),
    S3(S3WalEngine),
}

impl MultiDbManager {
    pub fn new(cfg: StorageConfig, fts_enabled: bool, max_active_dbs: usize) -> Self {
        let backend = match cfg.mode {
            StorageMode::Local => {
                StorageBackend::Local(LocalEngine::new(cfg.data_dir, fts_enabled, max_active_dbs))
            }
            StorageMode::S3 => {
                let s3_cfg = cfg.s3.unwrap_or_else(|| crate::config::S3Config {
                    bucket: String::new(),
                    prefix: "kongodb".to_string(),
                    region: "us-east-1".to_string(),
                    endpoint: None,
                    credentials: None,
                    lease_duration_secs: 30,
                    segment_max_bytes: 8 * 1024 * 1024,
                    flush_interval_secs: 2,
                    preload_dbs: vec![],
                    snapshot_every_writes: 100,
                    snapshot_max_count: 64,
                    snapshot_max_age_days: 14,
                    remote_sync_enabled: true,
                    remote_sync_interval_secs: 3,
                    replication_mode: crate::config::ReplicationMode::Async,
                    safe_hydrate: true,
                    safe_hydrate_quick_check: true,
                });
                StorageBackend::S3(S3WalEngine::new(
                    cfg.data_dir,
                    s3_cfg,
                    fts_enabled,
                    max_active_dbs,
                ))
            }
        };

        Self {
            backend,
            max_active_dbs: max_active_dbs.max(1),
        }
    }

    pub async fn get_conn_with_create(
        &self,
        db_path: &str,
        allow_create: bool,
    ) -> AppResult<libsql::Connection> {
        match &self.backend {
            StorageBackend::Local(engine) => engine.get_conn(db_path, allow_create).await,
            StorageBackend::S3(engine) => engine.get_conn(db_path, allow_create).await,
        }
    }

    pub async fn db_exists(&self, db_path: &str) -> AppResult<bool> {
        match &self.backend {
            StorageBackend::Local(engine) => engine.db_exists(db_path).await,
            StorageBackend::S3(engine) => engine.db_exists(db_path).await,
        }
    }

    pub async fn append_wal_record(
        &self,
        db_path: &str,
        sql: &str,
        args_json: &str,
    ) -> AppResult<()> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.append_wal_record(db_path, sql, args_json).await,
            StorageBackend::Local(_) => Ok(()),
        }
    }

    pub async fn run_ttl_reaper(
        &self,
        __kdb_archive_ttl_secs: Option<u64>,
        metric_events_retention_days: Option<u64>,
        max_concurrency: usize,
    ) -> AppResult<ReaperStats> {
        match &self.backend {
            StorageBackend::Local(engine) => {
                engine
                    .run_ttl_reaper(
                        __kdb_archive_ttl_secs,
                        metric_events_retention_days,
                        max_concurrency,
                    )
                    .await
            }
            StorageBackend::S3(engine) => {
                engine
                    .run_ttl_reaper(
                        __kdb_archive_ttl_secs,
                        metric_events_retention_days,
                        max_concurrency,
                    )
                    .await
            }
        }
    }

    pub async fn clone_db(&self, from_db_path: &str, to_db_path: &str) -> AppResult<()> {
        match &self.backend {
            StorageBackend::Local(engine) => engine.clone_db(from_db_path, to_db_path).await,
            StorageBackend::S3(engine) => engine.clone_db(from_db_path, to_db_path).await,
        }
    }

    pub async fn backup_db_with_result(
        &self,
        db_path: &str,
        backup_db_path: &str,
    ) -> AppResult<String> {
        match &self.backend {
            StorageBackend::Local(engine) => engine.backup_db(db_path, backup_db_path).await,
            StorageBackend::S3(engine) => engine.backup_db(db_path, backup_db_path).await,
        }
    }

    pub async fn restore_from_backup(
        &self,
        db_path: &str,
        backup_db_path: &str,
    ) -> AppResult<bool> {
        match &self.backend {
            StorageBackend::Local(engine) => {
                engine.restore_from_backup(db_path, backup_db_path).await
            }
            StorageBackend::S3(engine) => engine.restore_from_backup(db_path, backup_db_path).await,
        }
    }

    pub async fn offload_db(&self, db_path: &str) -> AppResult<()> {
        match &self.backend {
            StorageBackend::Local(engine) => engine.offload_db(db_path).await,
            StorageBackend::S3(engine) => engine.offload_db(db_path).await,
        }
    }

    pub async fn load_db(&self, db_path: &str) -> AppResult<bool> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.load_db(db_path).await,
            StorageBackend::Local(_) => Err(crate::error::AppError::BadRequest(
                "load_db is only supported in s3 mode".to_string(),
            )),
        }
    }

    pub async fn sync_db(&self, db_path: &str) -> AppResult<SyncDbResult> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.sync_db(db_path).await,
            StorageBackend::Local(_) => Err(crate::error::AppError::BadRequest(
                "sync_db is only supported in s3 mode".to_string(),
            )),
        }
    }

    pub async fn list_db_snapshots(&self, db_path: &str) -> AppResult<DbSnapshotsList> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.list_db_snapshots(db_path).await,
            StorageBackend::Local(_) => Err(crate::error::AppError::BadRequest(
                "list_db_snapshots is only supported in s3 mode".to_string(),
            )),
        }
    }

    pub async fn compact_wal(
        &self,
        db_path: &str,
        retain_segments: usize,
    ) -> AppResult<CompactWalResult> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.compact_wal(db_path, retain_segments).await,
            StorageBackend::Local(_) => Err(crate::error::AppError::BadRequest(
                "compact_wal is only supported in s3 mode".to_string(),
            )),
        }
    }

    pub async fn get_sync_status(&self, db_path: &str) -> AppResult<SyncStatus> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.get_sync_status(db_path).await,
            StorageBackend::Local(_) => Err(crate::error::AppError::BadRequest(
                "get_sync_status is only supported in s3 mode".to_string(),
            )),
        }
    }

    pub async fn verify_db(&self, db_path: &str) -> AppResult<VerifyDbResult> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.verify_db(db_path).await,
            StorageBackend::Local(_) => Err(crate::error::AppError::BadRequest(
                "verify_db is only supported in s3 mode".to_string(),
            )),
        }
    }

    pub async fn restore_db_snapshot(
        &self,
        db_path: &str,
        snapshot_id: Option<&str>,
    ) -> AppResult<bool> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.hydrate_db(db_path, snapshot_id).await,
            StorageBackend::Local(_) => Err(crate::error::AppError::BadRequest(
                "restore_db_snapshot is only supported in s3 mode".to_string(),
            )),
        }
    }

    pub async fn read_s3_uri_from_offset(&self, uri: &str, offset: usize) -> AppResult<Vec<u8>> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.read_s3_uri_from_offset(uri, offset).await,
            StorageBackend::Local(engine) => engine.read_s3_uri_from_offset(uri, offset).await,
        }
    }

    pub async fn read_s3_uri_range(
        &self,
        uri: &str,
        offset: usize,
        len: usize,
    ) -> AppResult<Vec<u8>> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.read_s3_uri_range(uri, offset, len).await,
            StorageBackend::Local(engine) => engine.read_s3_uri_range(uri, offset, len).await,
        }
    }

    pub async fn get_s3_uri_source_hash(&self, uri: &str) -> AppResult<Option<String>> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.get_s3_uri_source_hash(uri).await,
            StorageBackend::Local(engine) => engine.get_s3_uri_source_hash(uri).await,
        }
    }

    pub async fn write_s3_uri(&self, uri: &str, bytes: &[u8]) -> AppResult<()> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.write_s3_uri(uri, bytes).await,
            StorageBackend::Local(engine) => engine.write_s3_uri(uri, bytes).await,
        }
    }

    pub async fn delete_s3_uri(&self, uri: &str) -> AppResult<()> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.delete_s3_uri(uri).await,
            StorageBackend::Local(engine) => engine.delete_s3_uri(uri).await,
        }
    }

    pub async fn run_auto_indexing(
        &self,
        min_hits: i64,
        max_indexes_per_db: usize,
        max_new_per_run: usize,
    ) -> AppResult<Vec<AutoIndexDbReport>> {
        match &self.backend {
            StorageBackend::S3(engine) => {
                engine
                    .run_auto_indexing(min_hits, max_indexes_per_db, max_new_per_run)
                    .await
            }
            StorageBackend::Local(engine) => {
                engine
                    .run_auto_indexing(min_hits, max_indexes_per_db, max_new_per_run)
                    .await
            }
        }
    }

    pub async fn run_auto_backup_cycle(
        &self,
        backup_target: &str,
        max_concurrency: usize,
        min_interval_secs: u64,
        min_writes_since_backup: u64,
        max_staleness_secs: u64,
        retention_max_count: usize,
        retention_max_age_days: u64,
    ) -> AppResult<AutoBackupCycleReport> {
        match &self.backend {
            StorageBackend::S3(engine) => {
                engine
                    .run_auto_backup_cycle(
                        backup_target,
                        max_concurrency,
                        min_interval_secs,
                        min_writes_since_backup,
                        max_staleness_secs,
                        retention_max_count,
                        retention_max_age_days,
                    )
                    .await
            }
            StorageBackend::Local(engine) => {
                engine
                    .run_auto_backup_cycle(
                        backup_target,
                        max_concurrency,
                        min_interval_secs,
                        min_writes_since_backup,
                        max_staleness_secs,
                        retention_max_count,
                        retention_max_age_days,
                    )
                    .await
            }
        }
    }

    pub fn loaded_db_paths(&self) -> Vec<String> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.loaded_db_paths(),
            StorageBackend::Local(engine) => engine.loaded_db_paths(),
        }
    }

    pub fn list_active_db_paths(&self) -> Vec<String> {
        self.loaded_db_paths()
    }

    pub fn is_db_loaded(&self, db_path: &str) -> bool {
        self.loaded_db_paths().iter().any(|v| v == db_path)
    }

    pub async fn list_all_db_paths(&self) -> AppResult<Vec<String>> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.list_all_db_paths().await,
            StorageBackend::Local(engine) => engine.list_all_db_paths().await,
        }
    }

    pub async fn db_local_file_size_bytes(&self, db_path: &str) -> AppResult<Option<u64>> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.db_local_file_size_bytes(db_path).await,
            StorageBackend::Local(engine) => engine.db_local_file_size_bytes(db_path).await,
        }
    }

    pub async fn db_exists_local_only(&self, db_path: &str) -> AppResult<bool> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.db_exists_local_only(db_path).await,
            StorageBackend::Local(engine) => engine.db_exists_local_only(db_path).await,
        }
    }

    pub async fn db_exists_remote_only(&self, db_path: &str) -> AppResult<bool> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.db_exists_remote_only(db_path).await,
            StorageBackend::Local(_) => Ok(false),
        }
    }

    pub async fn sync_loaded_dbs_from_remote(
        &self,
        max_concurrency: usize,
    ) -> AppResult<RemoteSyncReport> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.sync_loaded_dbs_from_remote(max_concurrency).await,
            StorageBackend::Local(_) => Ok(RemoteSyncReport::default()),
        }
    }

    pub async fn flush_replication_queue(&self) -> AppResult<ReplicationFlushReport> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.flush_replication_queue().await,
            StorageBackend::Local(_) => Ok(ReplicationFlushReport::default()),
        }
    }

    pub async fn cleanup_temp_artifacts(&self, older_than_secs: u64) -> AppResult<u64> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.cleanup_temp_artifacts(older_than_secs).await,
            StorageBackend::Local(engine) => engine.cleanup_temp_artifacts(older_than_secs).await,
        }
    }

    pub async fn close_idle_dbs(&self, idle_secs: u64) -> AppResult<usize> {
        match &self.backend {
            StorageBackend::S3(engine) => engine.close_idle_dbs(idle_secs).await,
            StorageBackend::Local(engine) => engine.close_idle_dbs(idle_secs).await,
        }
    }

    pub fn max_active_dbs(&self) -> usize {
        self.max_active_dbs
    }
}
