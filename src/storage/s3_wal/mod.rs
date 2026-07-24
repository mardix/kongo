//! S3 WAL backend: connection management plus WAL/manifest replication hooks.

pub mod lease;
pub mod manifest;
pub mod object_store;
pub mod replicator;
pub mod segment;
pub mod snapshot;

use std::{collections::HashSet, path::Path, sync::Arc};

use chrono::NaiveDateTime;
use dashmap::DashMap;
use libsql::{Builder, Connection};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::{ReplicationMode, S3Config, S3Credentials};
use crate::error::{AppError, AppResult};
use crate::storage::auto_index::{AutoIndexDbReport, run_auto_index_for_conn};
use crate::storage::backup::AutoBackupCycleReport;
use crate::storage::db_path::resolve_db_file;
use crate::storage::reaper::{ReaperStats, reap_conn};
use crate::storage::s3_wal::{manifest::Manifest, snapshot::SnapshotMeta};
use crate::storage::schema::init_schema_with_retry;
use object_store::{AwsS3ObjectStore, ObjectStore, as_arc, split_s3_uri};
use replicator::Replicator;
use tokio::{
    sync::{Mutex, Semaphore},
    task::JoinSet,
};
use uuid::Uuid;

#[derive(Clone)]
pub struct S3WalEngine {
    base_path: Arc<String>,
    cfg: Arc<S3Config>,
    fts_enabled: bool,
    max_active_dbs: usize,
    conns: Arc<DashMap<String, Connection>>,
    last_accessed: Arc<DashMap<String, u64>>,
    init_locks: Arc<DashMap<String, Arc<Mutex<()>>>>,
    write_counters: Arc<DashMap<String, u64>>,
    backup_last_runs: Arc<DashMap<String, u64>>,
    loaded_snapshot_ids: Arc<DashMap<String, String>>,
    wal_queue: Arc<tokio::sync::Mutex<Vec<QueuedWalRecord>>>,
    async_replication: bool,
    replicator: Arc<Replicator>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TierSyncStatus {
    pub manifest_exists: bool,
    pub snapshot_exists: bool,
    pub applied_seq: Option<u64>,
    pub segment_count: usize,
    pub snapshot_id: Option<String>,
    pub updated_at: Option<String>,
    pub compaction_watermark: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncStatus {
    pub db: String,
    pub local_present: bool,
    pub remote: TierSyncStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncDbResult {
    pub db: String,
    pub snapshot_id: String,
    pub applied_seq: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DbSnapshotsList {
    pub db: String,
    pub current_snapshot_id: Option<String>,
    pub snapshots: Vec<SnapshotMeta>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompactTierResult {
    pub removed_segments: usize,
    pub retained_segments: usize,
    pub compaction_watermark: Option<u64>,
    pub applied_seq: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompactWalResult {
    pub db: String,
    pub retain_segments: usize,
    pub remote: CompactTierResult,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerifyTierResult {
    pub ok: bool,
    pub manifest_exists: bool,
    pub snapshot_exists: bool,
    pub segment_total: usize,
    pub missing_segment_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerifyDbResult {
    pub db: String,
    pub ok: bool,
    pub remote: VerifyTierResult,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RemoteSyncReport {
    pub checked: usize,
    pub refreshed: usize,
    pub skipped_no_snapshot: usize,
    pub failed: usize,
    pub error_samples: Vec<String>,
}

enum RemoteSyncOne {
    Unchanged,
    SkippedNoSnapshot,
    Refreshed,
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ReplicationFlushReport {
    pub queued: usize,
    pub flushed_records: usize,
    pub dbs_flushed: usize,
    pub failed_records: usize,
    pub dead_lettered_records: usize,
    pub oldest_queued_age_secs: Option<u64>,
    pub error_samples: Vec<String>,
}

#[derive(Debug, Clone)]
struct QueuedWalRecord {
    db_path: String,
    sql: String,
    args_json: String,
}

impl S3WalEngine {
    pub fn new(base_path: String, cfg: S3Config, fts_enabled: bool, max_active_dbs: usize) -> Self {
        let cfg = Arc::new(cfg);
        let store =
            build_store(&cfg).unwrap_or_else(|e| panic!("failed building s3 object store: {e}"));
        let writer_id = format!("writer-{}", Uuid::new_v4().simple());
        let replicator = Arc::new(Replicator::new(store, cfg.clone(), writer_id));
        let async_replication = matches!(cfg.replication_mode, ReplicationMode::Async);

        Self {
            base_path: Arc::new(base_path),
            cfg,
            fts_enabled,
            max_active_dbs: max_active_dbs.max(1),
            conns: Arc::new(DashMap::new()),
            last_accessed: Arc::new(DashMap::new()),
            init_locks: Arc::new(DashMap::new()),
            write_counters: Arc::new(DashMap::new()),
            backup_last_runs: Arc::new(DashMap::new()),
            loaded_snapshot_ids: Arc::new(DashMap::new()),
            wal_queue: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            async_replication,
            replicator,
        }
    }
}

impl S3WalEngine {
    pub async fn get_conn(&self, db_path: &str, allow_create: bool) -> AppResult<Connection> {
        let (cache_key, file_path) = resolve_db_file(self.base_path.as_str(), db_path)?;

        if let Some(existing) = self.conns.get(&cache_key) {
            self.touch(&cache_key);
            return Ok(existing.clone());
        }
        self.evict_lru_if_needed(&cache_key).await?;
        let init_lock = self
            .init_locks
            .entry(cache_key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _guard = init_lock.lock().await;
        if let Some(existing) = self.conns.get(&cache_key) {
            self.touch(&cache_key);
            return Ok(existing.clone());
        }
        self.evict_lru_if_needed(&cache_key).await?;

        let mut exists = tokio::fs::try_exists(&file_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed to check db path: {e}")))?;
        if !exists {
            self.hydrate_local_from_remote(&cache_key, &file_path, None)
                .await?;
            exists = tokio::fs::try_exists(&file_path).await.map_err(|e| {
                AppError::Internal(format!("failed to check hydrated db path: {e}"))
            })?;
        }
        if !exists && !allow_create {
            return Err(AppError::NotFound(format!("db_path not found: {db_path}")));
        }

        let dir = Path::new(self.base_path.as_str());
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| AppError::Internal(format!("failed to create data dir: {e}")))?;

        if let Some(parent) = file_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AppError::Internal(format!("failed to create db path: {e}")))?;
        }

        // S3/S3-Express replication transport can be switched here by replacing
        // Builder::new_local(...) with the desired libsql builder configuration.
        let db = Builder::new_local(file_path)
            .build()
            .await
            .map_err(|e| AppError::Internal(format!("open db failed: {e}")))?;

        let conn = db
            .connect()
            .map_err(|e| AppError::Internal(format!("connect db failed: {e}")))?;

        init_schema_with_retry(&conn, self.fts_enabled).await?;

        // Ensure remote metadata is initialized before connection use.
        // In async replication mode, warm-sync errors should not fail request paths.
        // In sync mode, only lease conflicts are tolerated.
        if let Err(err) = self.replicator.sync_once(&cache_key).await {
            if self.async_replication {
                eprintln!("replication warm-sync skipped for {cache_key}: {err}");
            } else if !is_replication_lease_conflict(&err) {
                return Err(err);
            }
        }
        if let Some(snapshot_id) = self.current_remote_snapshot_id(&cache_key).await? {
            self.loaded_snapshot_ids
                .insert(cache_key.clone(), snapshot_id);
        }

        self.touch(&cache_key);
        self.conns.insert(cache_key, conn.clone());
        Ok(conn)
    }

    pub async fn db_exists(&self, db_path: &str) -> AppResult<bool> {
        let (cache_key, file_path) = resolve_db_file(self.base_path.as_str(), db_path)?;
        if self.conns.contains_key(&cache_key) {
            return Ok(true);
        }

        let local_exists = tokio::fs::try_exists(file_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed to check db path: {e}")))?;
        if local_exists {
            return Ok(true);
        }

        let manifest = manifest_key(&self.cfg.prefix, &cache_key);
        if self.replicator.blob_exists(&manifest).await? {
            return Ok(true);
        }

        Ok(false)
    }

    pub async fn db_exists_local_only(&self, db_path: &str) -> AppResult<bool> {
        let (cache_key, file_path) = resolve_db_file(self.base_path.as_str(), db_path)?;
        if self.conns.contains_key(&cache_key) {
            return Ok(true);
        }
        tokio::fs::try_exists(file_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed to check db path: {e}")))
    }

    pub async fn db_exists_remote_only(&self, db_path: &str) -> AppResult<bool> {
        let (cache_key, _) = resolve_db_file(self.base_path.as_str(), db_path)?;
        let manifest = manifest_key(&self.cfg.prefix, &cache_key);
        if self.replicator.blob_exists(&manifest).await? {
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn append_wal_record(
        &self,
        db_path: &str,
        sql: &str,
        args_json: &str,
    ) -> AppResult<()> {
        let (cache_key, _) = resolve_db_file(self.base_path.as_str(), db_path)?;
        if self.async_replication {
            self.enqueue_replication_job(&cache_key, sql, args_json)
                .await?;
            return Ok(());
        }
        self.append_records_for_db(
            &cache_key,
            vec![(sql.to_string(), args_json.to_string())],
            true,
        )
        .await
    }

    async fn append_records_for_db(
        &self,
        cache_key: &str,
        records: Vec<(String, String)>,
        snapshot_after: bool,
    ) -> AppResult<()> {
        if records.is_empty() {
            return Ok(());
        }
        let record_count = records.len() as u64;
        self.replicator.append_segment(cache_key, records).await?;
        self.write_counters
            .entry(cache_key.to_string())
            .and_modify(|v| *v += record_count)
            .or_insert(record_count);
        // Replication records currently describe completed operations rather than replayable SQL.
        // Keep a recoverable checkpoint for every replicated batch until delta replay exists.
        if snapshot_after {
            if let Some(conn) = self.conns.get(cache_key).map(|c| c.clone()) {
                self.force_sync_snapshot(&conn, cache_key).await?;
            }
        }
        Ok(())
    }

    async fn enqueue_replication_job(
        &self,
        cache_key: &str,
        sql: &str,
        args_json: &str,
    ) -> AppResult<()> {
        let conn = if let Some(conn) = self.conns.get(cache_key).map(|c| c.clone()) {
            conn
        } else {
            self.get_conn(cache_key, false).await?
        };
        let res = conn
            .execute(
                "INSERT INTO __kdb_replication_jobs (sql, args_json, status, attempts, updated_at)
             VALUES (?, ?, 'queued', 0, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                libsql::params![sql.to_string(), args_json.to_string()],
            )
            .await;
        if let Err(e) = res {
            let msg = e.to_string();
            // Do not fail request writes on transient sqlite contention in async mode.
            // Stage in-memory and let flush loop persist later.
            if is_sqlite_lock_or_busy_msg(msg.as_str()) {
                let mut q = self.wal_queue.lock().await;
                q.push(QueuedWalRecord {
                    db_path: cache_key.to_string(),
                    sql: sql.to_string(),
                    args_json: args_json.to_string(),
                });
                return Ok(());
            }
            return Err(AppError::Internal(format!(
                "enqueue replication job failed: {e}"
            )));
        }
        Ok(())
    }

    pub async fn run_ttl_reaper(
        &self,
        __kdb_archive_ttl_secs: Option<u64>,
        metric_events_retention_days: Option<u64>,
        max_concurrency: usize,
    ) -> AppResult<ReaperStats> {
        let mut total = ReaperStats::default();
        let conns: Vec<(String, Connection)> = self
            .conns
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();

        let conns = conns
            .into_iter()
            .filter(|(db_path, _)| !is_internal_db_path(db_path))
            .collect::<Vec<_>>();
        for chunk in conns.chunks(max_concurrency.max(1)) {
            let mut jobs = JoinSet::new();
            for (db_path, conn) in chunk {
                let engine = self.clone();
                let db_path = db_path.clone();
                let conn = conn.clone();
                jobs.spawn(async move {
                    let stats =
                        reap_conn(&conn, __kdb_archive_ttl_secs, metric_events_retention_days)
                            .await?;
                    if stats.has_changes() {
                        engine.force_sync_snapshot(&conn, &db_path).await?;
                    }
                    Ok::<ReaperStats, AppError>(stats)
                });
            }
            while let Some(result) = jobs.join_next().await {
                let stats = result
                    .map_err(|e| AppError::Internal(format!("reaper task join failed: {e}")))??;
                total.merge(stats);
            }
        }

        Ok(total)
    }

    pub async fn clone_db(&self, from_db_path: &str, to_db_path: &str) -> AppResult<()> {
        let (_, to_file) = resolve_db_file(self.base_path.as_str(), to_db_path)?;
        let to_exists = tokio::fs::try_exists(&to_file)
            .await
            .map_err(|e| AppError::Internal(format!("failed to check target db path: {e}")))?;
        if to_exists {
            return Err(AppError::Conflict(format!(
                "target db_path already exists: {to_db_path}"
            )));
        }
        if let Some(parent) = to_file.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AppError::Internal(format!("failed to create target db path: {e}")))?;
        }
        let conn = self.get_conn(from_db_path, false).await?;
        let quoted = sql_quote_path(&to_file)?;
        conn.execute_batch(&format!("VACUUM INTO '{quoted}';"))
            .await
            .map_err(|e| AppError::Internal(format!("clone_db vacuum-into failed: {e}")))?;
        Ok(())
    }

    pub async fn backup_db(&self, db_path: &str, backup_db_path: &str) -> AppResult<String> {
        let conn = self.get_conn(db_path, false).await?;
        self.force_sync_snapshot(&conn, db_path).await?;
        let bytes = dump_db_to_temp_bytes(self.base_path.as_ref(), db_path, &conn).await?;

        if backup_db_path.starts_with("s3://") {
            let (target_uri, generated) = resolve_backup_target_s3_uri(backup_db_path, db_path)?;
            let out = if generated || target_uri.ends_with(".zst") {
                zstd::stream::encode_all(std::io::Cursor::new(bytes), 6)
                    .map_err(|e| AppError::Internal(format!("backup zstd compress failed: {e}")))?
            } else {
                bytes
            };
            if let Err(write_err) = self.write_s3_uri(&target_uri, &out).await {
                // In rare transport timeout/ack races, object may still have been persisted.
                // If we can observe it right away, treat backup as successful.
                let exists = {
                    let (store, key) = self.regular_s3_store_for_uri(&target_uri)?;
                    store.exists(&key).await.unwrap_or(false)
                };
                if !exists {
                    return Err(write_err);
                }
            }
            return Ok(target_uri);
        } else {
            let (target_file, generated) = resolve_backup_target_file(backup_db_path, db_path)?;
            if let Some(parent) = target_file.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|e| {
                    AppError::Internal(format!("failed to create backup target dir: {e}"))
                })?;
            }
            let out = if generated || target_file.to_string_lossy().ends_with(".zst") {
                zstd::stream::encode_all(std::io::Cursor::new(bytes), 6)
                    .map_err(|e| AppError::Internal(format!("backup zstd compress failed: {e}")))?
            } else {
                bytes
            };
            tokio::fs::write(&target_file, out)
                .await
                .map_err(|e| AppError::Internal(format!("failed to write backup target: {e}")))?;
            return Ok(target_file.to_string_lossy().to_string());
        }
    }

    pub async fn restore_from_backup(
        &self,
        db_path: &str,
        backup_db_path: &str,
    ) -> AppResult<bool> {
        let raw = if backup_db_path.starts_with("s3://") {
            self.read_s3_uri(backup_db_path).await?
        } else {
            let src = std::path::PathBuf::from(backup_db_path);
            let exists = tokio::fs::try_exists(&src)
                .await
                .map_err(|e| AppError::Internal(format!("failed to check backup source: {e}")))?;
            if !exists {
                return Err(AppError::NotFound(format!(
                    "backup source not found: {}",
                    src.display()
                )));
            }
            tokio::fs::read(&src)
                .await
                .map_err(|e| AppError::Internal(format!("failed to read backup source: {e}")))?
        };
        let decoded = if backup_db_path.ends_with(".zst") {
            zstd::stream::decode_all(std::io::Cursor::new(raw))
                .map_err(|e| AppError::Internal(format!("backup zstd decode failed: {e}")))?
        } else {
            raw
        };

        let (cache_key, file_path) = resolve_db_file(self.base_path.as_str(), db_path)?;
        if let Some((_k, conn)) = self.conns.remove(&cache_key) {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(FULL);").await;
        }
        if let Some(parent) = file_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AppError::Internal(format!("failed to create db path: {e}")))?;
        }
        tokio::fs::write(&file_path, decoded)
            .await
            .map_err(|e| AppError::Internal(format!("failed to write restored db: {e}")))?;
        let wal = std::path::PathBuf::from(format!("{}-wal", file_path.display()));
        let shm = std::path::PathBuf::from(format!("{}-shm", file_path.display()));
        remove_if_exists(&wal).await?;
        remove_if_exists(&shm).await?;
        let conn = self.get_conn(db_path, false).await?;
        self.force_sync_snapshot(&conn, &cache_key).await?;
        Ok(true)
    }

    pub async fn offload_db(&self, db_path: &str) -> AppResult<()> {
        let (cache_key, file_path) = resolve_db_file(self.base_path.as_str(), db_path)?;
        let exists = tokio::fs::try_exists(&file_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed to check db path: {e}")))?;
        if !exists {
            return Err(AppError::NotFound(format!("db_path not found: {db_path}")));
        }

        if let Some((_, conn)) = self.conns.remove(&cache_key) {
            self.force_sync_snapshot(&conn, &cache_key).await?;
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(FULL);").await;
        }

        self.replicator.sync_once(&cache_key).await?;

        remove_if_exists(&file_path).await?;
        let wal = std::path::PathBuf::from(format!("{}-wal", file_path.display()));
        let shm = std::path::PathBuf::from(format!("{}-shm", file_path.display()));
        remove_if_exists(&wal).await?;
        remove_if_exists(&shm).await?;

        Ok(())
    }

    pub async fn load_db(&self, db_path: &str) -> AppResult<bool> {
        let (cache_key, _) = resolve_db_file(self.base_path.as_str(), db_path)?;
        if self.conns.contains_key(&cache_key) {
            return Ok(false);
        }
        let _ = self.get_conn(db_path, false).await?;
        Ok(true)
    }

    pub async fn sync_db(&self, db_path: &str) -> AppResult<SyncDbResult> {
        let (cache_key, _) = resolve_db_file(self.base_path.as_str(), db_path)?;
        let conn = self.get_conn(db_path, false).await?;
        self.force_sync_snapshot(&conn, &cache_key).await?;
        self.write_counters.insert(cache_key, 0);
        let status = self.get_sync_status(db_path).await?;
        Ok(SyncDbResult {
            db: db_path.to_string(),
            snapshot_id: status
                .remote
                .snapshot_id
                .unwrap_or_else(|| "unknown".to_string()),
            applied_seq: status.remote.applied_seq.unwrap_or(0),
        })
    }

    pub async fn list_db_snapshots(&self, db_path: &str) -> AppResult<DbSnapshotsList> {
        let (cache_key, _) = resolve_db_file(self.base_path.as_str(), db_path)?;
        if let Some(manifest) = self.replicator.read_manifest(&cache_key).await? {
            return Ok(DbSnapshotsList {
                db: db_path.to_string(),
                current_snapshot_id: manifest.current_snapshot_id,
                snapshots: manifest.snapshots,
            });
        }
        Ok(DbSnapshotsList {
            db: db_path.to_string(),
            current_snapshot_id: None,
            snapshots: vec![],
        })
    }

    pub async fn compact_wal(
        &self,
        db_path: &str,
        retain_segments: usize,
    ) -> AppResult<CompactWalResult> {
        let (cache_key, _) = resolve_db_file(self.base_path.as_str(), db_path)?;
        let retain_segments = retain_segments.max(1);

        let (manifest, removed) = self
            .replicator
            .compact_manifest(&cache_key, retain_segments)
            .await?;
        let remote = CompactTierResult {
            removed_segments: removed,
            retained_segments: manifest.segments.len(),
            compaction_watermark: manifest.compaction_watermark,
            applied_seq: manifest.applied_seq,
        };

        Ok(CompactWalResult {
            db: db_path.to_string(),
            retain_segments,
            remote,
        })
    }

    pub async fn get_sync_status(&self, db_path: &str) -> AppResult<SyncStatus> {
        let (cache_key, file_path) = resolve_db_file(self.base_path.as_str(), db_path)?;
        let local_present = tokio::fs::try_exists(file_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed to check db path: {e}")))?;
        let remote = self.tier_status(&cache_key, &self.replicator).await?;

        Ok(SyncStatus {
            db: db_path.to_string(),
            local_present,
            remote,
        })
    }

    pub async fn verify_db(&self, db_path: &str) -> AppResult<VerifyDbResult> {
        let (cache_key, _) = resolve_db_file(self.base_path.as_str(), db_path)?;
        let remote = self.verify_tier(&cache_key, &self.replicator).await?;
        let ok = remote.ok;

        Ok(VerifyDbResult {
            db: db_path.to_string(),
            ok,
            remote,
        })
    }

    pub async fn hydrate_db(&self, db_path: &str, snapshot_id: Option<&str>) -> AppResult<bool> {
        let (cache_key, file_path) = resolve_db_file(self.base_path.as_str(), db_path)?;
        if let Some((_k, conn)) = self.conns.remove(&cache_key) {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(FULL);").await;
        }

        let wal = std::path::PathBuf::from(format!("{}-wal", file_path.display()));
        let shm = std::path::PathBuf::from(format!("{}-shm", file_path.display()));
        remove_if_exists(&wal).await?;
        remove_if_exists(&shm).await?;

        if self.cfg.safe_hydrate {
            let parent = file_path.parent().ok_or_else(|| {
                AppError::Internal("invalid db path: missing parent directory".to_string())
            })?;
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AppError::Internal(format!("failed to create hydrate dir: {e}")))?;

            let staged_file = parent.join(format!("._tmp_hydrate_{}.db", Uuid::new_v4().simple()));
            remove_if_exists(&staged_file).await?;
            self.hydrate_local_from_remote(&cache_key, &staged_file, snapshot_id)
                .await?;

            let staged_exists = tokio::fs::try_exists(&staged_file)
                .await
                .map_err(|e| AppError::Internal(format!("failed to verify staged hydrate: {e}")))?;
            if !staged_exists {
                return Ok(false);
            }
            if self.cfg.safe_hydrate_quick_check {
                if let Err(err) = quick_check_db_file(&staged_file).await {
                    let _ = remove_if_exists(&staged_file).await;
                    return Err(err);
                }
            }

            atomic_replace_db_file(&file_path, &staged_file).await?;
        } else {
            remove_if_exists(&file_path).await?;
            self.hydrate_local_from_remote(&cache_key, &file_path, snapshot_id)
                .await?;
        }

        let exists = tokio::fs::try_exists(&file_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed to check restored db path: {e}")))?;
        if !exists {
            return Ok(false);
        }

        let _ = self.get_conn(db_path, false).await?;
        Ok(true)
    }

    pub async fn read_s3_uri(&self, uri: &str) -> AppResult<Vec<u8>> {
        let (store, key) = self.regular_s3_store_for_uri(uri)?;
        let Some((bytes, _)) = store.get(&key).await? else {
            return Err(AppError::NotFound(format!("s3 object not found: {uri}")));
        };
        Ok(bytes)
    }

    pub async fn read_s3_uri_from_offset(&self, uri: &str, offset: usize) -> AppResult<Vec<u8>> {
        let (store, key) = self.regular_s3_store_for_uri(uri)?;
        let Some((bytes, _)) = store.get_range(&key, offset).await? else {
            return Err(AppError::NotFound(format!("s3 object not found: {uri}")));
        };
        Ok(bytes)
    }

    pub async fn read_s3_uri_range(
        &self,
        uri: &str,
        offset: usize,
        len: usize,
    ) -> AppResult<Vec<u8>> {
        let (store, key) = self.regular_s3_store_for_uri(uri)?;
        let Some((bytes, _)) = store.get_range_limited(&key, offset, len).await? else {
            return Ok(Vec::new());
        };
        Ok(bytes)
    }

    pub async fn get_s3_uri_source_hash(&self, uri: &str) -> AppResult<Option<String>> {
        let (store, key) = self.regular_s3_store_for_uri(uri)?;
        store.head_source_hash(&key).await
    }

    pub async fn write_s3_uri(&self, uri: &str, bytes: &[u8]) -> AppResult<()> {
        let (store, key) = self.regular_s3_store_for_uri(uri)?;
        let _ = store.put(&key, bytes).await?;
        Ok(())
    }

    pub async fn delete_s3_uri(&self, uri: &str) -> AppResult<()> {
        let (store, key) = self.regular_s3_store_for_uri(uri)?;
        store.delete(&key).await
    }

    pub async fn run_auto_indexing(
        &self,
        min_hits: i64,
        max_indexes_per_db: usize,
        max_new_per_run: usize,
    ) -> AppResult<Vec<AutoIndexDbReport>> {
        let conns: Vec<(String, Connection)> = self
            .conns
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();
        let mut out = Vec::new();
        for (db_path, conn) in conns {
            if is_internal_db_path(&db_path) {
                continue;
            }
            out.push(
                run_auto_index_for_conn(
                    &conn,
                    &db_path,
                    min_hits,
                    max_indexes_per_db,
                    max_new_per_run,
                )
                .await?,
            );
        }
        Ok(out)
    }

    pub fn loaded_db_paths(&self) -> Vec<String> {
        self.conns
            .iter()
            .map(|entry| entry.key().clone())
            .filter(|db| !is_internal_db_path(db))
            .collect()
    }

    pub async fn list_all_db_paths(&self) -> AppResult<Vec<String>> {
        let mut dbs: HashSet<String> = self.loaded_db_paths().into_iter().collect();
        for db in self.list_local_db_paths().await? {
            if !is_internal_db_path(&db) {
                dbs.insert(db);
            }
        }
        for db in self.list_known_remote_db_paths().await? {
            if !is_internal_db_path(&db) {
                dbs.insert(db);
            }
        }
        let mut out: Vec<String> = dbs.into_iter().collect();
        out.sort();
        Ok(out)
    }

    pub async fn cleanup_temp_artifacts(&self, older_than_secs: u64) -> AppResult<u64> {
        cleanup_temp_artifacts_under_base(self.base_path.as_str(), older_than_secs).await
    }

    pub async fn close_idle_dbs(&self, idle_secs: u64) -> AppResult<usize> {
        if idle_secs == 0 {
            return Ok(0);
        }
        let threshold = unix_now_secs().saturating_sub(idle_secs);
        let candidates = self
            .last_accessed
            .iter()
            .filter_map(|entry| {
                if *entry.value() <= threshold {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let mut closed = 0usize;
        for key in candidates {
            if let Some((_k, conn)) = self.conns.remove(&key) {
                let _ = conn.execute_batch("PRAGMA wal_checkpoint(FULL);").await;
                closed += 1;
            }
            self.loaded_snapshot_ids.remove(&key);
            self.last_accessed.remove(&key);
        }
        Ok(closed)
    }

    fn touch(&self, db_path: &str) {
        self.last_accessed
            .insert(db_path.to_string(), unix_now_secs());
    }

    async fn evict_lru_if_needed(&self, opening: &str) -> AppResult<()> {
        if self.conns.contains_key(opening) {
            return Ok(());
        }
        if self.conns.len() < self.max_active_dbs {
            return Ok(());
        }

        let victim = self
            .last_accessed
            .iter()
            .filter_map(|entry| {
                let key = entry.key().clone();
                if key == opening || !self.conns.contains_key(&key) {
                    return None;
                }
                Some((key, *entry.value()))
            })
            .min_by_key(|(_, ts)| *ts)
            .map(|(key, _)| key)
            .or_else(|| {
                self.conns.iter().find_map(|entry| {
                    (entry.key().as_str() != opening).then(|| entry.key().clone())
                })
            });

        if let Some(victim) = victim {
            if let Some((_k, conn)) = self.conns.remove(&victim) {
                let _ = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);").await;
            }
            self.loaded_snapshot_ids.remove(&victim);
            self.last_accessed.remove(&victim);
        }

        if self.conns.len() >= self.max_active_dbs {
            return Err(AppError::BadRequest(format!(
                "max active dbs reached: {} (KONGODB_MAX_ACTIVE_DBS)",
                self.max_active_dbs
            )));
        }
        Ok(())
    }

    pub async fn db_local_file_size_bytes(&self, db_path: &str) -> AppResult<Option<u64>> {
        let (_, file_path) = resolve_db_file(self.base_path.as_str(), db_path)?;
        let exists = tokio::fs::try_exists(&file_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed to check db path: {e}")))?;
        if !exists {
            return Ok(None);
        }
        let meta = tokio::fs::metadata(&file_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed to read db metadata: {e}")))?;
        Ok(Some(meta.len()))
    }

    pub async fn flush_replication_queue(&self) -> AppResult<ReplicationFlushReport> {
        if !self.async_replication {
            return Ok(ReplicationFlushReport::default());
        }

        let mut dbs: HashSet<String> = self.loaded_db_paths().into_iter().collect();
        for db in self.list_local_db_paths().await? {
            if !is_internal_db_path(&db) {
                dbs.insert(db);
            }
        }
        for entry in self.wal_queue.lock().await.iter() {
            if !is_internal_db_path(&entry.db_path) {
                dbs.insert(entry.db_path.clone());
            }
        }

        let mut report = ReplicationFlushReport::default();

        for db_path in dbs {
            let conn = match self.get_conn(&db_path, false).await {
                Ok(c) => c,
                Err(err) => {
                    if report.error_samples.len() < 5 {
                        report
                            .error_samples
                            .push(format!("open {db_path} failed: {err}"));
                    }
                    continue;
                }
            };

            let mut staged = Vec::<QueuedWalRecord>::new();
            {
                let mut q = self.wal_queue.lock().await;
                let mut remain = Vec::with_capacity(q.len());
                for item in q.drain(..) {
                    if item.db_path == db_path {
                        staged.push(item);
                    } else {
                        remain.push(item);
                    }
                }
                *q = remain;
            }
            for item in staged {
                let _ = conn
                    .execute(
                        "INSERT INTO __kdb_replication_jobs (sql, args_json, status, attempts, updated_at)
                         VALUES (?, ?, 'queued', 0, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                        libsql::params![item.sql, item.args_json],
                    )
                    .await;
            }

            let total = self.count_pending_kdb_replication_jobs(&conn).await?;
            report.queued += total as usize;
            if let Some(age) = self.oldest_queued_age_secs(&conn).await? {
                report.oldest_queued_age_secs = Some(
                    report
                        .oldest_queued_age_secs
                        .map(|v| v.max(age))
                        .unwrap_or(age),
                );
            }
            if total == 0 {
                continue;
            }

            let jobs = self.fetch_pending_kdb_replication_jobs(&conn, 1000).await?;
            if jobs.is_empty() {
                continue;
            }
            let records = jobs
                .iter()
                .map(|(_, sql, args)| (sql.clone(), args.clone()))
                .collect::<Vec<_>>();
            let ids = jobs.iter().map(|(id, _, _)| *id).collect::<Vec<_>>();
            match self.append_records_for_db(&db_path, records, true).await {
                Ok(_) => {
                    self.delete_kdb_replication_jobs(&conn, &ids).await?;
                    report.flushed_records += ids.len();
                    report.dbs_flushed += 1;
                }
                Err(err) => {
                    if is_replication_lease_conflict(&err) {
                        // Another writer won the lease race; keep jobs queued and retry on next flush tick.
                        continue;
                    }
                    let dead = self
                        .bump_replication_job_attempts(&conn, &ids, &err.to_string())
                        .await?;
                    report.dead_lettered_records += dead;
                    report.failed_records += ids.len();
                    if report.error_samples.len() < 5 {
                        report.error_samples.push(format!("{db_path}: {err}"));
                    }
                }
            }
        }

        Ok(report)
    }

    async fn list_local_db_paths(&self) -> AppResult<Vec<String>> {
        let root = std::path::PathBuf::from(self.base_path.as_str());
        let root_exists = tokio::fs::try_exists(&root)
            .await
            .map_err(|e| AppError::Internal(format!("failed to check data root: {e}")))?;
        if !root_exists {
            return Ok(vec![]);
        }
        let mut stack = vec![root.clone()];
        let mut out = Vec::<String>::new();
        while let Some(dir) = stack.pop() {
            let mut rd = tokio::fs::read_dir(&dir).await.map_err(|e| {
                AppError::Internal(format!("failed to read data dir {}: {e}", dir.display()))
            })?;
            while let Some(ent) = rd
                .next_entry()
                .await
                .map_err(|e| AppError::Internal(format!("failed to read data entry: {e}")))?
            {
                let path = ent.path();
                let ft = ent
                    .file_type()
                    .await
                    .map_err(|e| AppError::Internal(format!("failed to stat path: {e}")))?;
                if ft.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !ft.is_file() || path.extension().and_then(|v| v.to_str()) != Some("db") {
                    continue;
                }
                let rel = path
                    .strip_prefix(&root)
                    .map_err(|e| AppError::Internal(format!("failed to relativize db path: {e}")))?
                    .to_string_lossy()
                    .to_string();
                if rel.is_empty() {
                    continue;
                }
                let db_path = rel.trim_end_matches(".db").replace('\\', "/");
                if is_internal_db_path(&db_path) {
                    continue;
                }
                out.push(db_path);
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    pub async fn sync_loaded_dbs_from_remote(
        &self,
        max_concurrency: usize,
    ) -> AppResult<RemoteSyncReport> {
        if !self.cfg.remote_sync_enabled {
            return Ok(RemoteSyncReport::default());
        }
        let dbs = self.loaded_db_paths();
        let mut report = RemoteSyncReport {
            checked: dbs.len(),
            ..RemoteSyncReport::default()
        };
        for chunk in dbs.chunks(max_concurrency.max(1)) {
            let mut jobs = JoinSet::new();
            for db_path in chunk {
                let engine = self.clone();
                let db_path = db_path.clone();
                jobs.spawn(async move { engine.sync_one_loaded_db_from_remote(db_path).await });
            }
            while let Some(result) = jobs.join_next().await {
                match result {
                    Ok(Ok(RemoteSyncOne::Unchanged)) => {}
                    Ok(Ok(RemoteSyncOne::SkippedNoSnapshot)) => report.skipped_no_snapshot += 1,
                    Ok(Ok(RemoteSyncOne::Refreshed)) => report.refreshed += 1,
                    Ok(Ok(RemoteSyncOne::Failed(msg))) => {
                        report.failed += 1;
                        if report.error_samples.len() < 5 {
                            report.error_samples.push(msg);
                        }
                    }
                    Ok(Err(err)) => {
                        report.failed += 1;
                        if report.error_samples.len() < 5 {
                            report.error_samples.push(err.to_string());
                        }
                    }
                    Err(err) => {
                        report.failed += 1;
                        if report.error_samples.len() < 5 {
                            report
                                .error_samples
                                .push(format!("remote sync task join failed: {err}"));
                        }
                    }
                }
            }
        }
        Ok(report)
    }

    async fn sync_one_loaded_db_from_remote(&self, db_path: String) -> AppResult<RemoteSyncOne> {
        if self.async_replication {
            let has_pending = if let Some(conn) = self.conns.get(&db_path).map(|v| v.clone()) {
                self.count_pending_kdb_replication_jobs(&conn).await? > 0
            } else {
                false
            };
            if has_pending {
                return Ok(RemoteSyncOne::Unchanged);
            }
        }
        let Some(remote_snapshot_id) = self.current_remote_snapshot_id(&db_path).await? else {
            return Ok(RemoteSyncOne::SkippedNoSnapshot);
        };
        let local_snapshot_id = self
            .loaded_snapshot_ids
            .get(&db_path)
            .map(|v| v.clone())
            .unwrap_or_default();
        if local_snapshot_id == remote_snapshot_id {
            return Ok(RemoteSyncOne::Unchanged);
        }
        match self
            .hydrate_db(&db_path, Some(remote_snapshot_id.as_str()))
            .await
        {
            Ok(true) => {
                self.loaded_snapshot_ids
                    .insert(db_path.clone(), remote_snapshot_id);
                Ok(RemoteSyncOne::Refreshed)
            }
            Ok(false) => Ok(RemoteSyncOne::Failed(format!(
                "refresh produced no local db for {db_path}"
            ))),
            Err(err) => Ok(RemoteSyncOne::Failed(format!("{db_path}: {err}"))),
        }
    }

    async fn count_pending_kdb_replication_jobs(&self, conn: &Connection) -> AppResult<i64> {
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM __kdb_replication_jobs WHERE status = 'queued'",
                (),
            )
            .await
            .map_err(|e| AppError::Internal(format!("count replication jobs failed: {e}")))?;
        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("count replication jobs row failed: {e}")))?
        {
            row.get::<i64>(0).map_err(|e| {
                AppError::Internal(format!("count replication jobs decode failed: {e}"))
            })
        } else {
            Ok(0)
        }
    }

    async fn fetch_pending_kdb_replication_jobs(
        &self,
        conn: &Connection,
        limit: usize,
    ) -> AppResult<Vec<(i64, String, String)>> {
        let mut rows = conn
            .query(
                "SELECT id, sql, args_json
                 FROM __kdb_replication_jobs
                 WHERE status = 'queued'
                 ORDER BY id ASC
                 LIMIT ?",
                libsql::params![limit as i64],
            )
            .await
            .map_err(|e| AppError::Internal(format!("fetch replication jobs failed: {e}")))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("fetch replication jobs row failed: {e}")))?
        {
            let id: i64 = row.get(0).map_err(|e| {
                AppError::Internal(format!("fetch replication jobs decode failed: {e}"))
            })?;
            let sql: String = row.get(1).map_err(|e| {
                AppError::Internal(format!("fetch replication jobs decode failed: {e}"))
            })?;
            let args_json: String = row.get(2).map_err(|e| {
                AppError::Internal(format!("fetch replication jobs decode failed: {e}"))
            })?;
            out.push((id, sql, args_json));
        }
        Ok(out)
    }

    async fn delete_kdb_replication_jobs(&self, conn: &Connection, ids: &[i64]) -> AppResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let tx = conn.transaction().await.map_err(|e| {
            AppError::Internal(format!("delete replication jobs tx begin failed: {e}"))
        })?;
        for id in ids {
            tx.execute(
                "DELETE FROM __kdb_replication_jobs WHERE id = ?",
                libsql::params![*id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("delete replication job failed: {e}")))?;
        }
        tx.commit().await.map_err(|e| {
            AppError::Internal(format!("delete replication jobs tx commit failed: {e}"))
        })
    }

    async fn bump_replication_job_attempts(
        &self,
        conn: &Connection,
        ids: &[i64],
        err: &str,
    ) -> AppResult<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        const MAX_ATTEMPTS: i64 = 8;
        let tx = conn.transaction().await.map_err(|e| {
            AppError::Internal(format!(
                "bump replication job attempts tx begin failed: {e}"
            ))
        })?;
        let mut dead_lettered = 0usize;
        for id in ids {
            let mut rows = tx
                .query(
                    "SELECT attempts FROM __kdb_replication_jobs WHERE id = ? LIMIT 1",
                    libsql::params![*id],
                )
                .await
                .map_err(|e| {
                    AppError::Internal(format!("read replication attempts failed: {e}"))
                })?;
            let attempts: i64 = if let Some(row) = rows.next().await.map_err(|e| {
                AppError::Internal(format!("read replication attempts row failed: {e}"))
            })? {
                row.get(0).map_err(|e| {
                    AppError::Internal(format!("read replication attempts decode failed: {e}"))
                })?
            } else {
                0
            };
            let next_attempts = attempts + 1;
            let status = if next_attempts >= MAX_ATTEMPTS {
                dead_lettered += 1;
                "dead_letter"
            } else {
                "queued"
            };
            tx.execute(
                "UPDATE __kdb_replication_jobs
                 SET attempts = attempts + 1,
                     status = ?,
                     last_error = ?,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE id = ?",
                libsql::params![status, err.to_string(), *id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("bump replication job attempt failed: {e}")))?;
        }
        tx.commit().await.map_err(|e| {
            AppError::Internal(format!(
                "bump replication job attempts tx commit failed: {e}"
            ))
        })?;
        Ok(dead_lettered)
    }

    async fn oldest_queued_age_secs(&self, conn: &Connection) -> AppResult<Option<u64>> {
        let mut rows = conn
            .query(
                "SELECT CAST(strftime('%s','now') AS INTEGER) - CAST(strftime('%s', MIN(created_at)) AS INTEGER)
                 FROM __kdb_replication_jobs
                 WHERE status = 'queued'",
                (),
            )
            .await
            .map_err(|e| AppError::Internal(format!("oldest queued age query failed: {e}")))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("oldest queued age row failed: {e}")))?
        else {
            return Ok(None);
        };
        let age: Option<i64> = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("oldest queued age decode failed: {e}")))?;
        Ok(age.and_then(|v| if v >= 0 { Some(v as u64) } else { None }))
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
        const BACKUP_WORKER_LEASE_TENANT: &str = "__kongodb_backup_worker__";
        match self
            .replicator
            .ensure_lease(BACKUP_WORKER_LEASE_TENANT)
            .await
        {
            Ok(_) => {}
            Err(AppError::Conflict(_)) => {
                return Ok(AutoBackupCycleReport {
                    skipped_lease: true,
                    ..AutoBackupCycleReport::default()
                });
            }
            Err(e) => return Err(e),
        }

        let dbs = self.list_known_remote_db_paths().await?;
        let mut report = AutoBackupCycleReport {
            discovered: dbs.len(),
            ..AutoBackupCycleReport::default()
        };
        let now = unix_now_secs();
        let semaphore = std::sync::Arc::new(Semaphore::new(max_concurrency.max(1)));
        let mut jobs = JoinSet::new();
        for db_path in dbs {
            let writes_since_last = self.write_counters.get(&db_path).map(|v| *v).unwrap_or(0);
            let changed = writes_since_last >= min_writes_since_backup.max(1);
            let stale = if max_staleness_secs == 0 {
                false
            } else {
                self.backup_last_runs
                    .get(&db_path)
                    .map(|v| now.saturating_sub(*v) >= max_staleness_secs)
                    .unwrap_or(true)
            };
            if min_interval_secs > 0 {
                if let Some(last) = self.backup_last_runs.get(&db_path).map(|v| *v) {
                    if now.saturating_sub(last) < min_interval_secs {
                        report.skipped_recent += 1;
                        continue;
                    }
                }
            }
            if !changed && !stale {
                report.skipped_unchanged += 1;
                continue;
            }
            report.scheduled += 1;
            let engine = self.clone();
            let db = db_path.clone();
            let target = backup_target.to_string();
            let sem = semaphore.clone();
            jobs.spawn(async move {
                let _permit = sem
                    .acquire_owned()
                    .await
                    .map_err(|_| AppError::Internal("auto backup semaphore closed".to_string()))?;
                let backup_path = engine.backup_db(&db, &target).await?;
                Ok::<(String, String), AppError>((db, backup_path))
            });
        }
        while let Some(job) = jobs.join_next().await {
            match job {
                Ok(Ok((db, backup_path))) => {
                    self.backup_last_runs.insert(db.clone(), now);
                    self.write_counters.insert(db.clone(), 0);
                    if let Err(err) = self
                        .apply_backup_retention_s3(
                            backup_target,
                            &backup_path,
                            retention_max_count,
                            retention_max_age_days,
                        )
                        .await
                    {
                        if report.error_samples.len() < 5 {
                            report.error_samples.push(format!("retention error: {err}"));
                        }
                    }
                    report.succeeded += 1;
                }
                Ok(Err(err)) => {
                    report.failed += 1;
                    if report.error_samples.len() < 5 {
                        report.error_samples.push(err.to_string());
                    }
                }
                Err(join_err) => {
                    report.failed += 1;
                    if report.error_samples.len() < 5 {
                        report
                            .error_samples
                            .push(format!("auto backup task join failed: {join_err}"));
                    }
                }
            }
        }

        Ok(report)
    }

    async fn list_known_remote_db_paths(&self) -> AppResult<Vec<String>> {
        let scan_prefix = format!("{}/", self.cfg.prefix.trim_matches('/'));
        let keys = self.replicator.list_keys(&scan_prefix).await?;
        let mut dbs = parse_db_paths_from_manifest_keys(&keys, &self.cfg.prefix);
        dbs.retain(|db| !is_internal_db_path(db));
        dbs.sort();
        dbs.dedup();
        Ok(dbs)
    }

    async fn apply_backup_retention_s3(
        &self,
        _backup_target: &str,
        written_backup_path: &str,
        max_count: usize,
        max_age_days: u64,
    ) -> AppResult<()> {
        if !written_backup_path.starts_with("s3://") {
            return Ok(());
        }
        let (bucket, object_key) = split_s3_uri(written_backup_path)?;
        let Some((prefix, _)) = object_key.rsplit_once('/') else {
            return Ok(());
        };
        let creds = self.cfg.credentials.as_ref().ok_or_else(|| {
            AppError::BadRequest(
                "missing s3 credentials (KONGODB_S3_ACCESS_KEY / KONGODB_S3_SECRET_KEY)"
                    .to_string(),
            )
        })?;
        validate_credentials(creds)?;
        let store = AwsS3ObjectStore::new(
            &format!("s3://{bucket}"),
            &self.cfg.region,
            self.cfg.endpoint.as_deref(),
            Some(creds),
        )?;
        let keys = store.list_prefix(prefix).await?;
        let mut entries = Vec::<(u64, String)>::new();
        for key in keys {
            let name = key.rsplit('/').next().unwrap_or("");
            if let Some(ts) = parse_backup_ts_from_filename(name) {
                entries.push((ts, key));
            }
        }
        if entries.is_empty() {
            return Ok(());
        }
        entries.sort_by(|a, b| b.0.cmp(&a.0));
        let now = unix_now_secs();
        let max_age_secs = max_age_days.saturating_mul(86_400);
        let capped = max_count.max(1);
        for (idx, (ts, key)) in entries.into_iter().enumerate() {
            let too_old = max_age_secs > 0 && now.saturating_sub(ts) > max_age_secs;
            let overflow = idx >= capped;
            if too_old || overflow {
                let _ = store.delete(&key).await;
            }
        }
        Ok(())
    }

    async fn hydrate_local_from_remote(
        &self,
        db_path: &str,
        local_file: &std::path::Path,
        snapshot_id: Option<&str>,
    ) -> AppResult<()> {
        let Some(manifest) = self.replicator.read_manifest(db_path).await? else {
            return Ok(());
        };
        let (resolved_snapshot_id, object_key) = resolve_manifest_snapshot(&manifest, snapshot_id)
            .ok_or_else(|| {
                let selector = snapshot_id.unwrap_or("current");
                AppError::NotFound(format!(
                    "snapshot not found in manifest for db {db_path}: {selector}"
                ))
            })?;
        if let Some(bytes) = self.replicator.get_blob(&object_key).await? {
            if let Some(parent) = local_file.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|e| {
                    AppError::Internal(format!("failed to create local hydrate dir: {e}"))
                })?;
            }
            tokio::fs::write(local_file, bytes).await.map_err(|e| {
                AppError::Internal(format!("failed writing hydrated remote db: {e}"))
            })?;
            self.loaded_snapshot_ids
                .insert(db_path.to_string(), resolved_snapshot_id);
            return Ok(());
        }
        Err(AppError::NotFound(format!(
            "snapshot object missing for db {db_path}: {object_key}"
        )))
    }

    async fn force_sync_snapshot(&self, conn: &Connection, db_path: &str) -> AppResult<()> {
        let (snapshot_meta, deleted_keys) = self
            .sync_snapshot_to_replicator(conn, db_path, &self.replicator)
            .await?;
        for key in deleted_keys {
            self.replicator.delete_blob(&key).await?;
        }
        self.loaded_snapshot_ids
            .insert(db_path.to_string(), snapshot_meta.id.clone());
        Ok(())
    }

    async fn sync_snapshot_to_replicator(
        &self,
        conn: &Connection,
        db_path: &str,
        target: &Replicator,
    ) -> AppResult<(SnapshotMeta, Vec<String>)> {
        let temp_name = format!(
            "_tmp_sync/{}_{}.db",
            db_path.replace('/', "__"),
            Uuid::new_v4().simple()
        );
        let temp_file = std::path::PathBuf::from(self.base_path.as_ref()).join(temp_name);
        if let Some(parent) = temp_file.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AppError::Internal(format!("failed to create sync temp dir: {e}")))?;
        }
        let quoted = sql_quote_path(&temp_file)?;
        conn.execute_batch(&format!("VACUUM INTO '{quoted}';"))
            .await
            .map_err(|e| AppError::Internal(format!("snapshot sync vacuum-into failed: {e}")))?;
        let bytes = tokio::fs::read(&temp_file)
            .await
            .map_err(|e| AppError::Internal(format!("snapshot sync read failed: {e}")))?;
        let applied_seq = target
            .read_manifest(db_path)
            .await?
            .map(|m| m.applied_seq)
            .unwrap_or(0);
        let snapshot_id = format!("{}-{:020}", unix_now_secs(), applied_seq);
        let versioned_key = versioned_snapshot_key(&self.cfg.prefix, db_path, &snapshot_id);
        let checksum = sha256_hex(&bytes);
        target.put_blob(&versioned_key, &bytes).await?;
        let snapshot_meta = SnapshotMeta {
            id: snapshot_id,
            tenant: db_path.to_string(),
            object_key: versioned_key,
            checksum,
            size_bytes: bytes.len(),
            created_at: unix_now_secs().to_string(),
            from_seq: 0,
            to_seq: applied_seq,
        };
        let (updated_manifest, deleted_keys) = target
            .upsert_snapshot(
                db_path,
                snapshot_meta.clone(),
                self.cfg.snapshot_max_count,
                self.cfg.snapshot_max_age_days,
            )
            .await?;
        let _ = updated_manifest;
        remove_if_exists(&temp_file).await?;
        Ok((snapshot_meta, deleted_keys))
    }

    async fn tier_status(&self, db_path: &str, repl: &Replicator) -> AppResult<TierSyncStatus> {
        let manifest = repl.read_manifest(db_path).await?;
        let manifest_exists = manifest.is_some();
        let snapshot_exists = if let Some(manifest) = manifest.as_ref() {
            if let Some((_, key)) = resolve_manifest_snapshot(manifest, None) {
                repl.blob_exists(&key).await?
            } else {
                false
            }
        } else {
            false
        };
        Ok(TierSyncStatus {
            manifest_exists,
            snapshot_exists,
            applied_seq: manifest.as_ref().map(|m| m.applied_seq),
            segment_count: manifest.as_ref().map(|m| m.segments.len()).unwrap_or(0),
            snapshot_id: manifest
                .as_ref()
                .and_then(|m| m.current_snapshot_id.clone()),
            updated_at: manifest.as_ref().map(|m| m.updated_at.clone()),
            compaction_watermark: manifest.and_then(|m| m.compaction_watermark),
        })
    }

    async fn verify_tier(&self, db_path: &str, repl: &Replicator) -> AppResult<VerifyTierResult> {
        let Some(manifest) = repl.read_manifest(db_path).await? else {
            return Ok(VerifyTierResult {
                ok: false,
                manifest_exists: false,
                snapshot_exists: false,
                segment_total: 0,
                missing_segment_keys: vec![],
            });
        };
        let snapshot_exists = if let Some((_, key)) = resolve_manifest_snapshot(&manifest, None) {
            repl.blob_exists(&key).await?
        } else {
            false
        };
        let mut missing_segment_keys = Vec::new();
        for seg in &manifest.segments {
            if !repl.blob_exists(&seg.object_key).await? {
                missing_segment_keys.push(seg.object_key.clone());
            }
        }
        let ok = snapshot_exists && missing_segment_keys.is_empty();
        Ok(VerifyTierResult {
            ok,
            manifest_exists: true,
            snapshot_exists,
            segment_total: manifest.segments.len(),
            missing_segment_keys,
        })
    }

    async fn current_remote_snapshot_id(&self, db_path: &str) -> AppResult<Option<String>> {
        Ok(self
            .replicator
            .read_manifest(db_path)
            .await?
            .and_then(|m| m.current_snapshot_id))
    }
}

fn is_replication_lease_conflict(err: &AppError) -> bool {
    match err {
        AppError::Conflict(msg) => {
            let lower = msg.to_ascii_lowercase();
            lower.contains("writer lease held by")
                || (lower.contains("conditional put failed")
                    && lower.contains("preconditionfailed")
                    && lower.contains("lease.json"))
        }
        _ => false,
    }
}

fn is_sqlite_lock_or_busy_msg(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("database is locked")
        || lower.contains("database is busy")
        || lower.contains("sqlite_busy")
}

impl S3WalEngine {
    fn regular_s3_store_for_uri(&self, uri: &str) -> AppResult<(AwsS3ObjectStore, String)> {
        let (bucket, key) = split_s3_uri(uri)?;
        if key.trim().is_empty() {
            return Err(AppError::BadRequest(
                "s3 uri must include object path".to_string(),
            ));
        }
        let creds = self.cfg.credentials.as_ref().ok_or_else(|| {
            AppError::BadRequest(
                "missing s3 credentials (KONGODB_S3_ACCESS_KEY / KONGODB_S3_SECRET_KEY)"
                    .to_string(),
            )
        })?;
        validate_credentials(creds)?;
        let base = format!("s3://{}", bucket);
        let store = AwsS3ObjectStore::new(
            &base,
            &self.cfg.region,
            self.cfg.endpoint.as_deref(),
            Some(creds),
        )?;
        Ok((store, key))
    }
}

fn validate_credentials(creds: &S3Credentials) -> AppResult<()> {
    if creds.access_key.trim().is_empty() || creds.secret_key.trim().is_empty() {
        return Err(AppError::BadRequest(
            "s3 credentials must include non-empty access_key and secret_key".to_string(),
        ));
    }
    Ok(())
}

fn build_store(cfg: &Arc<S3Config>) -> AppResult<Arc<dyn object_store::ObjectStore>> {
    let bucket = cfg.bucket.clone();
    let region = cfg.region.clone();
    let endpoint = cfg.endpoint.clone();
    let creds = cfg.credentials.as_ref().ok_or_else(|| {
        AppError::BadRequest(
            "missing s3 credentials (KONGODB_S3_ACCESS_KEY / KONGODB_S3_SECRET_KEY)".to_string(),
        )
    })?;
    validate_credentials(creds)?;
    // Keep the store rooted at the bucket; logical prefixes are already
    // applied by key builders (manifest/snapshot/wal) using cfg.prefix.
    let base_uri = format!("s3://{}", bucket);
    let _ = split_s3_uri(&base_uri)?;
    let store = AwsS3ObjectStore::new(&base_uri, &region, endpoint.as_deref(), Some(creds))?;
    Ok(as_arc(store))
}

fn sql_quote_path(path: &std::path::Path) -> AppResult<String> {
    let raw = path
        .to_str()
        .ok_or_else(|| AppError::BadRequest("target db path contains invalid utf-8".to_string()))?;
    Ok(raw.replace('\'', "''"))
}

fn versioned_snapshot_key(prefix: &str, db_path: &str, snapshot_id: &str) -> String {
    format!("{}/{}/snapshots/{}.db", prefix, db_path, snapshot_id)
}

fn resolve_manifest_snapshot(
    manifest: &Manifest,
    requested_id: Option<&str>,
) -> Option<(String, String)> {
    if let Some(id) = requested_id {
        if let Some(snapshot) = manifest.snapshots.iter().find(|snapshot| snapshot.id == id) {
            return Some((snapshot.id.clone(), snapshot.object_key.clone()));
        }
        if manifest.current_snapshot_id.as_deref() == Some(id) {
            return manifest
                .current_snapshot_key
                .as_ref()
                .map(|key| (id.to_string(), key.clone()));
        }
        return None;
    }

    let current_id = manifest.current_snapshot_id.as_ref()?;
    if let Some(key) = manifest.current_snapshot_key.as_ref() {
        return Some((current_id.clone(), key.clone()));
    }
    manifest
        .snapshots
        .iter()
        .find(|snapshot| &snapshot.id == current_id)
        .map(|snapshot| (snapshot.id.clone(), snapshot.object_key.clone()))
}

fn manifest_key(prefix: &str, db_path: &str) -> String {
    format!("{}/{}/manifest.json", prefix, db_path)
}

fn parse_db_paths_from_manifest_keys(keys: &[String], prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    let normalized_prefix = prefix.trim_matches('/');
    for key in keys {
        let raw = key.trim_start_matches('/');
        let rel = if let Some(rest) = raw.strip_prefix(normalized_prefix) {
            rest.trim_start_matches('/')
        } else {
            continue;
        };
        if !rel.ends_with("/manifest.json") {
            continue;
        }
        let db_path = rel.trim_end_matches("/manifest.json").trim_matches('/');
        if db_path.is_empty() {
            continue;
        }
        if is_internal_db_path(db_path) {
            continue;
        }
        out.push(db_path.to_string());
    }
    out
}

fn is_internal_db_path(db_path: &str) -> bool {
    let normalized = db_path.trim_matches('/');
    if normalized.is_empty() {
        return true;
    }
    if normalized.starts_with("_tmp_sync/") || normalized.starts_with("_tmp_backup/") {
        return true;
    }
    if normalized.starts_with("__kongodb_") || normalized == "__kdb_system" {
        return true;
    }
    normalized
        .split('/')
        .any(|seg| seg.starts_with("_tmp_") || seg.starts_with("._tmp_"))
}

async fn cleanup_temp_artifacts_under_base(
    base_path: &str,
    older_than_secs: u64,
) -> AppResult<u64> {
    let root = std::path::PathBuf::from(base_path);
    let exists = tokio::fs::try_exists(&root)
        .await
        .map_err(|e| AppError::Internal(format!("temp cleanup exists check failed: {e}")))?;
    if !exists {
        return Ok(0);
    }
    let threshold = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(older_than_secs.max(1)))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

    let mut removed = 0u64;
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        while let Some(ent) = rd
            .next_entry()
            .await
            .map_err(|e| AppError::Internal(format!("temp cleanup read_dir failed: {e}")))?
        {
            let path = ent.path();
            let file_type = match ent.file_type().await {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let is_temp = name.starts_with("._tmp_") || name.starts_with("_tmp_");
            if !is_temp {
                continue;
            }
            let modified = match tokio::fs::metadata(&path).await.and_then(|m| m.modified()) {
                Ok(ts) => ts,
                Err(_) => continue,
            };
            if modified > threshold {
                continue;
            }
            if tokio::fs::remove_file(&path).await.is_ok() {
                removed += 1;
            }
        }
    }
    Ok(removed)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

async fn remove_if_exists(path: &std::path::Path) -> AppResult<()> {
    let exists = tokio::fs::try_exists(path)
        .await
        .map_err(|e| AppError::Internal(format!("failed to check file before delete: {e}")))?;
    if exists {
        tokio::fs::remove_file(path)
            .await
            .map_err(|e| AppError::Internal(format!("failed to remove file: {e}")))?;
    }
    Ok(())
}

async fn atomic_replace_db_file(
    target_path: &std::path::Path,
    staged_path: &std::path::Path,
) -> AppResult<()> {
    let target_exists = tokio::fs::try_exists(target_path)
        .await
        .map_err(|e| AppError::Internal(format!("failed to check target db path: {e}")))?;
    if !target_exists {
        tokio::fs::rename(staged_path, target_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed to activate hydrated db: {e}")))?;
        return Ok(());
    }

    let parent = target_path.parent().ok_or_else(|| {
        AppError::Internal("invalid target db path: missing parent directory".to_string())
    })?;
    let backup_path = parent.join(format!("._tmp_prev_{}.db", Uuid::new_v4().simple()));
    remove_if_exists(&backup_path).await?;

    tokio::fs::rename(target_path, &backup_path)
        .await
        .map_err(|e| AppError::Internal(format!("failed to move existing db aside: {e}")))?;

    match tokio::fs::rename(staged_path, target_path).await {
        Ok(()) => {
            let _ = remove_if_exists(&backup_path).await;
            Ok(())
        }
        Err(swap_err) => {
            let _ = tokio::fs::rename(&backup_path, target_path).await;
            let _ = remove_if_exists(staged_path).await;
            Err(AppError::Internal(format!(
                "failed to swap hydrated db into place: {swap_err}"
            )))
        }
    }
}

async fn quick_check_db_file(path: &std::path::Path) -> AppResult<()> {
    let db = Builder::new_local(path)
        .build()
        .await
        .map_err(|e| AppError::Internal(format!("hydrate quick_check open failed: {e}")))?;
    let conn = db
        .connect()
        .map_err(|e| AppError::Internal(format!("hydrate quick_check connect failed: {e}")))?;
    let mut rows = conn
        .query("PRAGMA quick_check;", ())
        .await
        .map_err(|e| AppError::Internal(format!("hydrate quick_check query failed: {e}")))?;

    let mut issues = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("hydrate quick_check read failed: {e}")))?
    {
        let result: String = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("hydrate quick_check decode failed: {e}")))?;
        if !result.eq_ignore_ascii_case("ok") {
            issues.push(result);
            if issues.len() >= 5 {
                break;
            }
        }
    }
    if issues.is_empty() {
        return Ok(());
    }
    Err(AppError::Internal(format!(
        "hydrate quick_check failed: {}",
        issues.join(" | ")
    )))
}

fn resolve_backup_target_file(
    target: &str,
    db_path: &str,
) -> AppResult<(std::path::PathBuf, bool)> {
    let p = std::path::PathBuf::from(target);
    let looks_like_dir = target.ends_with('/') || p.extension().is_none();
    if looks_like_dir {
        let db_slug = db_path.replace('/', "_");
        let db_hash = short_db_hash(db_path);
        let folder = format!("{db_slug}--{db_hash}");
        let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        let filename = format!("{ts}_0001.db.zst");
        return Ok((p.join(folder).join(filename), true));
    }
    Ok((p, false))
}

fn resolve_backup_target_s3_uri(target: &str, db_path: &str) -> AppResult<(String, bool)> {
    let (bucket, key) = split_s3_uri(target)?;
    let looks_like_dir = target.ends_with('/') || !key.contains('.');
    if !looks_like_dir {
        return Ok((target.to_string(), false));
    }
    let db_slug = db_path.replace('/', "_");
    let db_hash = short_db_hash(db_path);
    let folder = format!("{db_slug}--{db_hash}");
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let filename = format!("{ts}_0001.db.zst");
    let prefix = key.trim_matches('/');
    let object_key = if prefix.is_empty() {
        format!("{folder}/{filename}")
    } else {
        format!("{prefix}/{folder}/{filename}")
    };
    Ok((format!("s3://{bucket}/{object_key}"), true))
}

fn short_db_hash(db_path: &str) -> String {
    let digest = Sha256::digest(db_path.as_bytes());
    let hex = format!("{:x}", digest);
    hex[..16].to_string()
}

fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn parse_backup_ts_from_filename(name: &str) -> Option<u64> {
    let ts_raw = name.split('_').next()?;
    let dt = NaiveDateTime::parse_from_str(ts_raw, "%Y%m%dT%H%M%SZ").ok()?;
    Some(dt.and_utc().timestamp() as u64)
}

async fn dump_db_to_temp_bytes(
    base_path: &str,
    db_path: &str,
    conn: &Connection,
) -> AppResult<Vec<u8>> {
    let temp_name = format!(
        "_tmp_backup/{}_{}.db",
        db_path.replace('/', "__"),
        uuid::Uuid::new_v4().simple()
    );
    let temp_file = std::path::PathBuf::from(base_path).join(temp_name);
    if let Some(parent) = temp_file.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Internal(format!("failed to create temp backup dir: {e}")))?;
    }
    let quoted = sql_quote_path(&temp_file)?;
    conn.execute_batch(&format!("VACUUM INTO '{quoted}';"))
        .await
        .map_err(|e| AppError::Internal(format!("backup_db vacuum-into failed: {e}")))?;
    let bytes = tokio::fs::read(&temp_file)
        .await
        .map_err(|e| AppError::Internal(format!("failed to read temp backup file: {e}")))?;
    remove_if_exists(&temp_file).await?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::resolve_manifest_snapshot;
    use crate::storage::s3_wal::{manifest::Manifest, snapshot::SnapshotMeta};

    fn manifest() -> Manifest {
        let snapshot = SnapshotMeta {
            id: "snap-2".to_string(),
            tenant: "app/main".to_string(),
            object_key: "data/app/main/snapshots/snap-2.db".to_string(),
            checksum: "checksum".to_string(),
            size_bytes: 42,
            created_at: "2".to_string(),
            from_seq: 0,
            to_seq: 2,
        };
        Manifest {
            tenant: "app/main".to_string(),
            epoch: 1,
            writer_id: "writer-test".to_string(),
            applied_seq: 2,
            current_snapshot_id: Some(snapshot.id.clone()),
            current_snapshot_key: Some(snapshot.object_key.clone()),
            snapshots: vec![snapshot],
            segments: vec![],
            compaction_watermark: None,
            updated_at: "2".to_string(),
        }
    }

    #[test]
    fn resolves_current_snapshot_from_manifest_pointer() {
        let resolved = resolve_manifest_snapshot(&manifest(), None);
        assert_eq!(
            resolved,
            Some((
                "snap-2".to_string(),
                "data/app/main/snapshots/snap-2.db".to_string()
            ))
        );
    }

    #[test]
    fn resolves_requested_versioned_snapshot() {
        let resolved = resolve_manifest_snapshot(&manifest(), Some("snap-2"));
        assert_eq!(
            resolved.as_ref().map(|value| value.0.as_str()),
            Some("snap-2")
        );
    }

    #[test]
    fn rejects_snapshot_missing_from_manifest() {
        assert_eq!(
            resolve_manifest_snapshot(&manifest(), Some("missing")),
            None
        );
    }
}
