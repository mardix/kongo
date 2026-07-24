//! Local filesystem storage engine with cached libSQL connections per database path.

use std::{path::Path, sync::Arc};

use chrono::{NaiveDateTime, Utc};
use dashmap::DashMap;
use libsql::{Builder, Connection};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt},
    sync::{Mutex, Semaphore},
    task::JoinSet,
};

use crate::error::{AppError, AppResult};
use crate::storage::auto_index::{AutoIndexDbReport, run_auto_index_for_conn};
use crate::storage::backup::AutoBackupCycleReport;
use crate::storage::db_path::resolve_db_file;
use crate::storage::reaper::{ReaperStats, reap_conn};
use crate::storage::schema::init_schema_with_retry;

#[derive(Clone)]
pub struct LocalEngine {
    base_path: Arc<String>,
    fts_enabled: bool,
    max_active_dbs: usize,
    conns: Arc<DashMap<String, Connection>>,
    last_accessed: Arc<DashMap<String, u64>>,
    init_locks: Arc<DashMap<String, Arc<Mutex<()>>>>,
    backup_last_runs: Arc<DashMap<String, u64>>,
    backup_last_mtime: Arc<DashMap<String, u64>>,
}

impl LocalEngine {
    pub fn new(base_path: String, fts_enabled: bool, max_active_dbs: usize) -> Self {
        Self {
            base_path: Arc::new(base_path),
            fts_enabled,
            max_active_dbs: max_active_dbs.max(1),
            conns: Arc::new(DashMap::new()),
            last_accessed: Arc::new(DashMap::new()),
            init_locks: Arc::new(DashMap::new()),
            backup_last_runs: Arc::new(DashMap::new()),
            backup_last_mtime: Arc::new(DashMap::new()),
        }
    }
}

impl LocalEngine {
    pub async fn get_conn(&self, db_path: &str, allow_create: bool) -> AppResult<Connection> {
        let (cache_key, file_path) = resolve_db_file(self.base_path.as_str(), db_path)?;

        // DashMap keeps hot connections available with very low lookup overhead.
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

        let exists = tokio::fs::try_exists(&file_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed to check db path: {e}")))?;
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

        let db = Builder::new_local(file_path)
            .build()
            .await
            .map_err(|e| AppError::Internal(format!("open db failed: {e}")))?;

        let conn = db
            .connect()
            .map_err(|e| AppError::Internal(format!("connect db failed: {e}")))?;

        init_schema_with_retry(&conn, self.fts_enabled).await?;

        self.touch(&cache_key);
        self.conns.insert(cache_key, conn.clone());
        Ok(conn)
    }

    pub async fn db_exists(&self, db_path: &str) -> AppResult<bool> {
        let (_, file_path) = resolve_db_file(self.base_path.as_str(), db_path)?;
        tokio::fs::try_exists(file_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed to check db path: {e}")))
    }

    pub async fn run_ttl_reaper(
        &self,
        __kdb_archive_ttl_secs: Option<u64>,
        metric_events_retention_days: Option<u64>,
        max_concurrency: usize,
    ) -> AppResult<ReaperStats> {
        let mut total = ReaperStats::default();
        let conns: Vec<Connection> = self
            .conns
            .iter()
            .map(|entry| entry.value().clone())
            .collect();

        for chunk in conns.chunks(max_concurrency.max(1)) {
            let mut jobs = JoinSet::new();
            for conn in chunk {
                let conn = conn.clone();
                jobs.spawn(async move {
                    reap_conn(&conn, __kdb_archive_ttl_secs, metric_events_retention_days).await
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

    pub async fn backup_db(&self, db_path: &str, backup_target: &str) -> AppResult<String> {
        if backup_target.starts_with("s3://") {
            return Err(AppError::BadRequest(
                "s3:// backup target is not supported in local mode".to_string(),
            ));
        }
        let (target_file, generated) = resolve_backup_target_file(backup_target, db_path)?;
        let target_exists = tokio::fs::try_exists(&target_file)
            .await
            .map_err(|e| AppError::Internal(format!("failed to check backup target: {e}")))?;
        if target_exists {
            return Err(AppError::Conflict(format!(
                "backup target already exists: {}",
                target_file.display()
            )));
        }
        if let Some(parent) = target_file.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AppError::Internal(format!("failed to create backup dir: {e}")))?;
        }
        let conn = self.get_conn(db_path, false).await?;
        let bytes = dump_db_to_temp_bytes(self.base_path.as_str(), db_path, &conn).await?;
        let out = if should_compress_target(&target_file, generated) {
            zstd::stream::encode_all(std::io::Cursor::new(bytes), 6)
                .map_err(|e| AppError::Internal(format!("backup zstd compress failed: {e}")))?
        } else {
            bytes
        };
        tokio::fs::write(&target_file, out)
            .await
            .map_err(|e| AppError::Internal(format!("failed to write backup target: {e}")))?;

        Ok(target_file.to_string_lossy().to_string())
    }

    pub async fn restore_from_backup(
        &self,
        db_path: &str,
        backup_db_path: &str,
    ) -> AppResult<bool> {
        if backup_db_path.starts_with("s3://") {
            return Err(AppError::BadRequest(
                "s3:// restore source is not supported in local mode".to_string(),
            ));
        }
        let source = std::path::PathBuf::from(backup_db_path);
        let exists = tokio::fs::try_exists(&source)
            .await
            .map_err(|e| AppError::Internal(format!("failed to check backup source: {e}")))?;
        if !exists {
            return Err(AppError::NotFound(format!(
                "backup source not found: {}",
                source.display()
            )));
        }
        let raw = tokio::fs::read(&source)
            .await
            .map_err(|e| AppError::Internal(format!("failed to read backup source: {e}")))?;
        let bytes = maybe_decompress_backup(&source, raw)?;

        let (cache_key, target_file) = resolve_db_file(self.base_path.as_str(), db_path)?;
        if let Some((_k, conn)) = self.conns.remove(&cache_key) {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(FULL);").await;
        }
        if let Some(parent) = target_file.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AppError::Internal(format!("failed to create db path: {e}")))?;
        }
        tokio::fs::write(&target_file, bytes)
            .await
            .map_err(|e| AppError::Internal(format!("failed to write restored db: {e}")))?;
        let wal = std::path::PathBuf::from(format!("{}-wal", target_file.display()));
        let shm = std::path::PathBuf::from(format!("{}-shm", target_file.display()));
        remove_if_exists(&wal).await?;
        remove_if_exists(&shm).await?;
        let _ = self.get_conn(db_path, false).await?;
        Ok(true)
    }

    pub async fn offload_db(&self, _db_path: &str) -> AppResult<()> {
        Err(AppError::BadRequest(
            "offload_db is only supported in s3 mode".to_string(),
        ))
    }

    pub async fn close_idle_dbs(&self, idle_secs: u64) -> AppResult<usize> {
        if idle_secs == 0 {
            return Ok(0);
        }
        let threshold = unix_now_secs().saturating_sub(idle_secs);
        let mut closed = 0usize;
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
        for key in candidates {
            if let Some((_k, conn)) = self.conns.remove(&key) {
                let _ = conn.execute_batch("PRAGMA wal_checkpoint(FULL);").await;
                closed += 1;
            }
            self.last_accessed.remove(&key);
        }
        Ok(closed)
    }

    pub async fn read_s3_uri_from_offset(&self, uri: &str, offset: usize) -> AppResult<Vec<u8>> {
        let file_path = storage_uri_to_root(self.base_path.as_str(), uri)?;
        let exists = tokio::fs::try_exists(&file_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed to check s3 uri path: {e}")))?;
        if !exists {
            return Err(AppError::NotFound(format!("s3 object not found: {uri}")));
        }
        let mut file = tokio::fs::File::open(file_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed reading s3 uri object: {e}")))?;
        file.seek(std::io::SeekFrom::Start(offset as u64))
            .await
            .map_err(|e| AppError::Internal(format!("failed seeking s3 uri object: {e}")))?;
        let mut out = Vec::<u8>::new();
        file.read_to_end(&mut out)
            .await
            .map_err(|e| AppError::Internal(format!("failed reading s3 uri object: {e}")))?;
        Ok(out)
    }

    pub async fn read_s3_uri_range(
        &self,
        uri: &str,
        offset: usize,
        len: usize,
    ) -> AppResult<Vec<u8>> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let file_path = storage_uri_to_root(self.base_path.as_str(), uri)?;
        let exists = tokio::fs::try_exists(&file_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed to check s3 uri path: {e}")))?;
        if !exists {
            return Ok(Vec::new());
        }
        let mut file = tokio::fs::File::open(file_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed reading s3 uri object: {e}")))?;
        file.seek(std::io::SeekFrom::Start(offset as u64))
            .await
            .map_err(|e| AppError::Internal(format!("failed seeking s3 uri object: {e}")))?;
        let mut out = vec![0u8; len];
        let n = file
            .read(&mut out)
            .await
            .map_err(|e| AppError::Internal(format!("failed reading s3 uri object: {e}")))?;
        out.truncate(n);
        Ok(out)
    }

    pub async fn get_s3_uri_source_hash(&self, _uri: &str) -> AppResult<Option<String>> {
        Ok(None)
    }

    pub async fn write_s3_uri(&self, uri: &str, bytes: &[u8]) -> AppResult<()> {
        let file_path = storage_uri_to_root(self.base_path.as_str(), uri)?;
        if let Some(parent) = file_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AppError::Internal(format!("failed to create s3 uri dir: {e}")))?;
        }
        tokio::fs::write(file_path, bytes)
            .await
            .map_err(|e| AppError::Internal(format!("failed writing s3 uri object: {e}")))
    }

    pub async fn delete_s3_uri(&self, uri: &str) -> AppResult<()> {
        let file_path = storage_uri_to_root(self.base_path.as_str(), uri)?;
        remove_if_exists(&file_path).await
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

    pub async fn list_all_db_paths(&self) -> AppResult<Vec<String>> {
        self.list_local_db_paths().await
    }

    pub async fn cleanup_temp_artifacts(&self, older_than_secs: u64) -> AppResult<u64> {
        cleanup_temp_artifacts_under_base(self.base_path.as_str(), older_than_secs).await
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

    pub async fn db_exists_local_only(&self, db_path: &str) -> AppResult<bool> {
        self.db_exists(db_path).await
    }

    pub async fn run_auto_backup_cycle(
        &self,
        backup_target: &str,
        max_concurrency: usize,
        min_interval_secs: u64,
        _min_writes_since_backup: u64,
        max_staleness_secs: u64,
        retention_max_count: usize,
        retention_max_age_days: u64,
    ) -> AppResult<AutoBackupCycleReport> {
        let dbs = self.list_local_db_paths().await?;
        let mut report = AutoBackupCycleReport {
            discovered: dbs.len(),
            ..AutoBackupCycleReport::default()
        };
        let now = unix_now_secs();
        let semaphore = std::sync::Arc::new(Semaphore::new(max_concurrency.max(1)));
        let mut jobs = JoinSet::new();

        for db_path in dbs {
            let mtime = self.db_mtime_secs(&db_path).await.unwrap_or(0);
            let changed = self
                .backup_last_mtime
                .get(&db_path)
                .map(|v| *v != mtime)
                .unwrap_or(true);
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
                    let mtime = self.db_mtime_secs(&db).await.unwrap_or(0);
                    self.backup_last_mtime.insert(db.clone(), mtime);
                    if let Err(err) = self
                        .apply_backup_retention_local(
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

    async fn db_mtime_secs(&self, db_path: &str) -> AppResult<u64> {
        let (_, file_path) = resolve_db_file(self.base_path.as_str(), db_path)?;
        let meta = tokio::fs::metadata(file_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed to stat db for backup: {e}")))?;
        let modified = meta
            .modified()
            .map_err(|e| AppError::Internal(format!("failed to read db mtime: {e}")))?;
        let ts = modified
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Ok(ts)
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
            let mut rd = match tokio::fs::read_dir(&dir).await {
                Ok(v) => v,
                Err(e) => {
                    return Err(AppError::Internal(format!(
                        "failed to read data dir {}: {e}",
                        dir.display()
                    )));
                }
            };
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
                if !ft.is_file() {
                    continue;
                }
                if path.extension().and_then(|v| v.to_str()) != Some("db") {
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
                let db = rel.trim_end_matches(".db").replace('\\', "/");
                if is_internal_db_path(&db) {
                    continue;
                }
                out.push(db);
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    async fn apply_backup_retention_local(
        &self,
        backup_target: &str,
        written_backup_path: &str,
        max_count: usize,
        max_age_days: u64,
    ) -> AppResult<()> {
        let target = std::path::PathBuf::from(backup_target);
        if !is_directory_target(&target, backup_target) {
            return Ok(());
        }
        let written = std::path::PathBuf::from(written_backup_path);
        let folder = written
            .parent()
            .ok_or_else(|| AppError::Internal("backup file parent missing".to_string()))?;
        if folder == target {
            return Ok(());
        }
        let mut rd = tokio::fs::read_dir(folder)
            .await
            .map_err(|e| AppError::Internal(format!("retention read dir failed: {e}")))?;
        let mut entries = Vec::<(u64, std::path::PathBuf)>::new();
        while let Some(ent) = rd
            .next_entry()
            .await
            .map_err(|e| AppError::Internal(format!("retention dir entry failed: {e}")))?
        {
            let p = ent.path();
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if let Some(ts) = parse_backup_ts_from_filename(name) {
                entries.push((ts, p));
            }
        }
        prune_backup_entries(entries, max_count, max_age_days).await
    }
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

fn sql_quote_path(path: &std::path::Path) -> AppResult<String> {
    let raw = path
        .to_str()
        .ok_or_else(|| AppError::BadRequest("target db path contains invalid utf-8".to_string()))?;
    Ok(raw.replace('\'', "''"))
}

fn resolve_backup_target_file(
    target: &str,
    db_path: &str,
) -> AppResult<(std::path::PathBuf, bool)> {
    let p = std::path::PathBuf::from(target);
    let looks_like_dir = is_directory_target(&p, target);
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

fn storage_uri_to_root(base_path: &str, uri: &str) -> AppResult<std::path::PathBuf> {
    let Some(without) = uri.strip_prefix("s3://") else {
        return Err(AppError::BadRequest(
            "s3 uri must start with s3://".to_string(),
        ));
    };
    let mut out = std::path::PathBuf::from(base_path);
    out.push("_remote_object_store");
    out.push(without);
    Ok(out)
}

fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn short_db_hash(db_path: &str) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(db_path.as_bytes());
    let hex = format!("{:x}", digest);
    hex[..16].to_string()
}

fn is_directory_target(path: &std::path::Path, raw: &str) -> bool {
    raw.ends_with('/') || path.extension().is_none()
}

fn should_compress_target(path: &std::path::Path, generated: bool) -> bool {
    generated || path.to_string_lossy().ends_with(".zst")
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

fn maybe_decompress_backup(path: &std::path::Path, bytes: Vec<u8>) -> AppResult<Vec<u8>> {
    if path.to_string_lossy().ends_with(".zst") {
        return zstd::stream::decode_all(std::io::Cursor::new(bytes))
            .map_err(|e| AppError::Internal(format!("backup zstd decode failed: {e}")));
    }
    Ok(bytes)
}

fn parse_backup_ts_from_filename(name: &str) -> Option<u64> {
    let ts_raw = name.split('_').next()?;
    let dt = NaiveDateTime::parse_from_str(ts_raw, "%Y%m%dT%H%M%SZ").ok()?;
    Some(dt.and_utc().timestamp() as u64)
}

async fn prune_backup_entries(
    mut entries: Vec<(u64, std::path::PathBuf)>,
    max_count: usize,
    max_age_days: u64,
) -> AppResult<()> {
    if entries.is_empty() {
        return Ok(());
    }
    entries.sort_by(|a, b| b.0.cmp(&a.0));
    let now = Utc::now().timestamp() as u64;
    let max_age_secs = max_age_days.saturating_mul(86_400);
    let mut keep = std::collections::HashSet::<std::path::PathBuf>::new();
    let capped = max_count.max(1);
    for (idx, (ts, path)) in entries.iter().enumerate() {
        if idx >= capped {
            continue;
        }
        if max_age_secs > 0 && now.saturating_sub(*ts) > max_age_secs {
            continue;
        }
        keep.insert(path.clone());
    }
    for (_, path) in entries {
        if keep.contains(&path) {
            continue;
        }
        remove_if_exists(&path).await?;
    }
    Ok(())
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
