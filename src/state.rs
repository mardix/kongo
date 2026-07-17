//! Global application state shared across handlers.

use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use chrono::{SecondsFormat, Utc};
use dashmap::{DashMap, mapref::entry::Entry};
use moka::future::Cache;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::api::dto::{GatewayRequest, GatewayResponse};
use crate::error::{AppError, AppResult};
use crate::storage::manager::MultiDbManager;
use crate::storage::system_catalog::SystemCatalog;

#[derive(Clone)]
pub struct AppState {
    pub db_manager: Arc<MultiDbManager>,
    pub base_path: String,
    pub admin_ui_enabled: bool,
    pub admin_ui_dir: String,
    pub docs_enabled: bool,
    pub docs_file: String,
    pub cors_allowed_origins: Vec<String>,
    pub access_key: Option<String>,
    pub delete_default_ttl_secs: Option<i64>,
    pub __kdb_archive_ttl_secs: Option<u64>,
    pub storage_mode_is_s3: bool,
    pub backup_mode_is_s3: bool,
    pub s3_bucket: Option<String>,
    pub backup_local_path: String,
    pub backup_s3_path: Option<String>,
    pub export_local_path: String,
    pub cache_enabled: bool,
    pub cache_max_entries: u64,
    pub read_cache: Cache<String, Value>,
    pub read_cache_ttl_overrides: Arc<DashMap<String, (u64, Value)>>,
    pub cache_epochs: Arc<DashMap<String, u64>>,
    pub response_include_system_timestamps: bool,
    pub response_include_namespace: bool,
    pub jsonb_enabled: bool,
    pub query_default_limit: usize,
    pub legacy_aliases_enabled: bool,
    pub legacy_import_pk_aliases: Arc<Vec<(String, String)>>,
    pub legacy_response_aliases: Arc<Vec<(String, String)>>,
    pub query_lookup_max_depth: usize,
    pub query_lookup_uncapped_override_enabled: bool,
    pub query_lookup_max_concurrency: usize,
    pub query_lookup_default_limit: usize,
    pub operation_timeout_ms: u64,
    pub import_job_retention_days: u64,
    pub export_job_retention_days: u64,
    pub export_batch_size: usize,
    pub metric_events_cache_enabled: bool,
    pub metric_events_cache_ttl_secs: u64,
    pub metric_events_insert_batch_size: usize,
    pub metric_events_retention_days: Option<u64>,
    pub metric_events_query_default_limit: usize,
    pub metric_events_query_max_limit: usize,
    pub strict_mutation_operators: bool,
    pub write_queue_enabled: bool,
    pub write_queue_default_ack_mode: String,
    pub write_queue_capacity: usize,
    pub write_queue_idle_secs: u64,
    pub db_idle_close_secs: u64,
    pub job_worker_concurrency: usize,
    pub sync_concurrency: usize,
    pub system_catalog_retention_days: u64,
    pub system_catalog: Option<SystemCatalog>,
    write_queues: Arc<DashMap<String, mpsc::Sender<QueuedWrite>>>,
    pending_documents: Arc<DashMap<String, PendingDocument>>,
    pending_revision: Arc<AtomicU64>,
    db_stats: Arc<DashMap<String, Arc<DbRuntimeStats>>>,
    system_stats: Arc<SystemRuntimeStats>,
}

#[derive(Clone, Debug)]
pub struct PendingDocument {
    pub revision: u64,
    pub collection: Option<String>,
    pub document: Value,
}

#[derive(Clone, Debug)]
pub struct PendingWriteRevision {
    pub id: String,
    pub revision: u64,
}

pub struct PendingWritePreview {
    pub response: GatewayResponse,
    pub revisions: Vec<PendingWriteRevision>,
}

enum QueuedWriteMode {
    Accepted,
    AcceptedPrepared,
    Committed,
}

struct QueuedWrite {
    db_path: String,
    request: GatewayRequest,
    mode: QueuedWriteMode,
    response_tx: Option<oneshot::Sender<AppResult<GatewayResponse>>>,
}

pub enum WriteEnqueueResult {
    Enqueued,
    Committed(AppResult<GatewayResponse>),
    Fallback(GatewayRequest),
}

#[derive(Debug, Clone)]
pub struct WriteQueueStat {
    pub db: String,
    pub queued: usize,
    pub capacity: usize,
    pub idle_secs: u64,
}

#[derive(Debug)]
pub struct DbRuntimeStats {
    requests_total: AtomicU64,
    reads_total: AtomicU64,
    writes_total: AtomicU64,
    errors_total: AtomicU64,
    in_flight: AtomicU64,
    last_accessed_at: RwLock<Option<String>>,
}

#[derive(Debug, Clone)]
pub struct DbStatsSnapshot {
    pub ts: String,
    pub requests_total: u64,
    pub reads_total: u64,
    pub writes_total: u64,
    pub errors_total: u64,
    pub in_flight: u64,
    pub last_accessed_at: Option<String>,
}

pub struct DbRequestStatsGuard {
    stats: Option<Arc<DbRuntimeStats>>,
}

#[derive(Debug)]
pub struct SystemRuntimeStats {
    started_at: String,
    started_instant: Instant,
    requests_total: AtomicU64,
    reads_total: AtomicU64,
    writes_total: AtomicU64,
    admin_total: AtomicU64,
    errors_total: AtomicU64,
    in_flight: AtomicU64,
    latency_total_ms: AtomicU64,
    latency_max_ms: AtomicU64,
    buckets: Mutex<VecDeque<SystemStatsBucket>>,
}

#[derive(Debug, Clone)]
struct SystemStatsBucket {
    epoch_minute: u64,
    ts: String,
    requests: u64,
    reads: u64,
    writes: u64,
    admin: u64,
    errors: u64,
    latency_total_ms: u64,
    latency_max_ms: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum SystemRequestKind {
    Read,
    Write,
    Admin,
}

pub struct SystemRequestStatsGuard {
    stats: Arc<SystemRuntimeStats>,
    kind: SystemRequestKind,
    started_at: Instant,
}

impl AppState {
    pub fn new(
        manager: MultiDbManager,
        base_path: String,
        admin_ui_enabled: bool,
        admin_ui_dir: String,
        docs_enabled: bool,
        docs_file: String,
        cors_allowed_origins: Vec<String>,
        access_key: Option<String>,
        delete_default_ttl_secs: Option<i64>,
        __kdb_archive_ttl_secs: Option<u64>,
        storage_mode_is_s3: bool,
        backup_mode_is_s3: bool,
        s3_bucket: Option<String>,
        backup_local_path: String,
        backup_s3_path: Option<String>,
        export_local_path: String,
        cache_enabled: bool,
        cache_ttl_secs: u64,
        cache_max_entries: u64,
        response_include_system_timestamps: bool,
        response_include_namespace: bool,
        jsonb_enabled: bool,
        query_default_limit: usize,
        legacy_aliases_enabled: bool,
        legacy_import_pk_aliases: Vec<(String, String)>,
        legacy_response_aliases: Vec<(String, String)>,
        query_lookup_max_depth: usize,
        query_lookup_uncapped_override_enabled: bool,
        query_lookup_max_concurrency: usize,
        query_lookup_default_limit: usize,
        operation_timeout_ms: u64,
        import_job_retention_days: u64,
        export_job_retention_days: u64,
        export_batch_size: usize,
        metric_events_cache_enabled: bool,
        metric_events_cache_ttl_secs: u64,
        metric_events_insert_batch_size: usize,
        metric_events_retention_days: Option<u64>,
        metric_events_query_default_limit: usize,
        metric_events_query_max_limit: usize,
        strict_mutation_operators: bool,
        write_queue_enabled: bool,
        write_queue_default_ack_mode: String,
        write_queue_capacity: usize,
        write_queue_idle_secs: u64,
        db_idle_close_secs: u64,
        job_worker_concurrency: usize,
        sync_concurrency: usize,
        system_catalog_retention_days: u64,
        system_catalog: Option<SystemCatalog>,
    ) -> Self {
        let read_cache = Cache::builder()
            .max_capacity(cache_max_entries)
            .time_to_live(std::time::Duration::from_secs(cache_ttl_secs.max(1)))
            .build();
        Self {
            db_manager: Arc::new(manager),
            base_path,
            admin_ui_enabled,
            admin_ui_dir,
            docs_enabled,
            docs_file,
            cors_allowed_origins,
            access_key,
            delete_default_ttl_secs,
            __kdb_archive_ttl_secs,
            storage_mode_is_s3,
            backup_mode_is_s3,
            s3_bucket,
            backup_local_path,
            backup_s3_path,
            export_local_path,
            cache_enabled,
            cache_max_entries: cache_max_entries.max(1),
            read_cache,
            read_cache_ttl_overrides: Arc::new(DashMap::new()),
            cache_epochs: Arc::new(DashMap::new()),
            response_include_system_timestamps,
            response_include_namespace,
            jsonb_enabled,
            query_default_limit: query_default_limit.max(1),
            legacy_aliases_enabled,
            legacy_import_pk_aliases: Arc::new(legacy_import_pk_aliases),
            legacy_response_aliases: Arc::new(legacy_response_aliases),
            query_lookup_max_depth: query_lookup_max_depth.max(1),
            query_lookup_uncapped_override_enabled,
            query_lookup_max_concurrency: query_lookup_max_concurrency.max(1),
            query_lookup_default_limit: query_lookup_default_limit.max(1),
            operation_timeout_ms: operation_timeout_ms.max(1),
            import_job_retention_days,
            export_job_retention_days,
            export_batch_size: export_batch_size.max(1),
            metric_events_cache_enabled,
            metric_events_cache_ttl_secs: metric_events_cache_ttl_secs.max(1),
            metric_events_insert_batch_size: metric_events_insert_batch_size.max(1),
            metric_events_retention_days,
            metric_events_query_default_limit: metric_events_query_default_limit.max(1),
            metric_events_query_max_limit: metric_events_query_max_limit.max(1),
            strict_mutation_operators,
            write_queue_enabled,
            write_queue_default_ack_mode,
            write_queue_capacity: write_queue_capacity.max(1),
            write_queue_idle_secs,
            db_idle_close_secs,
            job_worker_concurrency: job_worker_concurrency.max(1),
            sync_concurrency: sync_concurrency.max(1),
            system_catalog_retention_days,
            system_catalog,
            write_queues: Arc::new(DashMap::new()),
            pending_documents: Arc::new(DashMap::new()),
            pending_revision: Arc::new(AtomicU64::new(1)),
            db_stats: Arc::new(DashMap::new()),
            system_stats: Arc::new(SystemRuntimeStats::default()),
        }
    }

    pub fn begin_system_request(&self, kind: SystemRequestKind) -> SystemRequestStatsGuard {
        self.system_stats.begin_request(kind)
    }

    pub fn system_stats_snapshot_json(&self) -> Value {
        self.system_stats.snapshot_json()
    }

    pub fn begin_db_request(&self, db_path: &str, is_write: bool) -> DbRequestStatsGuard {
        let stats = self.db_stats_for(db_path);
        stats.requests_total.fetch_add(1, Ordering::Relaxed);
        if is_write {
            stats.writes_total.fetch_add(1, Ordering::Relaxed);
        } else {
            stats.reads_total.fetch_add(1, Ordering::Relaxed);
        }
        stats.in_flight.fetch_add(1, Ordering::Relaxed);
        stats.set_last_accessed_at(now_rfc3339());
        DbRequestStatsGuard { stats: Some(stats) }
    }

    pub fn db_stats_snapshot(&self, db_path: &str) -> DbStatsSnapshot {
        self.db_stats_for(db_path).snapshot()
    }

    pub fn db_stats_snapshot_json(&self, db_path: &str) -> Value {
        let snapshot = self.db_stats_snapshot(db_path);
        json!({
            "ts": snapshot.ts,
            "requests_total": snapshot.requests_total,
            "reads_total": snapshot.reads_total,
            "writes_total": snapshot.writes_total,
            "errors_total": snapshot.errors_total,
            "in_flight": snapshot.in_flight,
            "last_accessed_at": snapshot.last_accessed_at
        })
    }

    fn db_stats_for(&self, db_path: &str) -> Arc<DbRuntimeStats> {
        if let Some(existing) = self.db_stats.get(db_path) {
            return existing.clone();
        }
        let stats = Arc::new(DbRuntimeStats::default());
        self.db_stats
            .entry(db_path.to_string())
            .or_insert_with(|| stats.clone())
            .clone()
    }

    pub fn current_collection_epoch(&self, db_path: &str, collection: &str) -> u64 {
        let key = format!("c|{db_path}|{collection}");
        self.cache_epochs.get(&key).map(|v| *v).unwrap_or(0)
    }

    pub fn bump_collection_epoch(&self, db_path: &str, collection: &str) {
        let key = format!("c|{db_path}|{collection}");
        self.cache_epochs
            .entry(key)
            .and_modify(|v| *v += 1)
            .or_insert(1);
    }

    pub fn current_any_scope_epoch(&self, db_path: &str) -> u64 {
        let key = format!("a|{db_path}");
        self.cache_epochs.get(&key).map(|v| *v).unwrap_or(0)
    }

    pub fn bump_any_scope_epoch(&self, db_path: &str) {
        let key = format!("a|{db_path}");
        self.cache_epochs
            .entry(key)
            .and_modify(|v| *v += 1)
            .or_insert(1);
    }

    pub fn current_broadcast_epoch(&self, db_path: &str) -> u64 {
        let key = format!("b|{db_path}");
        self.cache_epochs.get(&key).map(|v| *v).unwrap_or(0)
    }

    pub fn bump_broadcast_epoch(&self, db_path: &str) {
        let key = format!("b|{db_path}");
        self.cache_epochs
            .entry(key)
            .and_modify(|v| *v += 1)
            .or_insert(1);
    }

    pub fn get_ttl_override_cache(&self, key: &str) -> Option<Value> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let entry = self.read_cache_ttl_overrides.get(key)?;
        if entry.value().0 > now {
            return Some(entry.value().1.clone());
        }
        drop(entry);
        self.read_cache_ttl_overrides.remove(key);
        None
    }

    pub fn put_ttl_override_cache(&self, key: String, value: Value, ttl_secs: u64) {
        if self.read_cache_ttl_overrides.len() as u64 >= self.cache_max_entries {
            if let Some(first) = self.read_cache_ttl_overrides.iter().next() {
                let evict_key = first.key().clone();
                drop(first);
                self.read_cache_ttl_overrides.remove(&evict_key);
            }
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let expires_at = now.saturating_add(ttl_secs.max(1));
        self.read_cache_ttl_overrides
            .insert(key, (expires_at, value));
    }

    pub async fn try_enqueue_write(
        &self,
        db_path: &str,
        mut request: GatewayRequest,
    ) -> AppResult<WriteEnqueueResult> {
        if !self.write_queue_enabled {
            return Ok(WriteEnqueueResult::Fallback(request));
        }

        request.payload.commit = Some(true);
        let sender = self.get_or_create_write_sender(db_path);
        match sender.try_send(QueuedWrite {
            db_path: db_path.to_string(),
            request,
            mode: QueuedWriteMode::Accepted,
            response_tx: None,
        }) {
            Ok(_) => Ok(WriteEnqueueResult::Enqueued),
            Err(tokio::sync::mpsc::error::TrySendError::Full(item)) => {
                Ok(WriteEnqueueResult::Fallback(item.request))
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(item)) => {
                self.write_queues.remove(db_path);
                Ok(WriteEnqueueResult::Fallback(item.request))
            }
        }
    }

    pub async fn enqueue_committed_write(
        &self,
        db_path: &str,
        mut request: GatewayRequest,
    ) -> AppResult<WriteEnqueueResult> {
        if !self.write_queue_enabled {
            return Ok(WriteEnqueueResult::Fallback(request));
        }

        request.payload.commit = Some(true);
        let fallback_request = request.clone();
        let sender = self.get_or_create_write_sender(db_path);
        let (response_tx, response_rx) = oneshot::channel();
        match sender
            .send(QueuedWrite {
                db_path: db_path.to_string(),
                request,
                mode: QueuedWriteMode::Committed,
                response_tx: Some(response_tx),
            })
            .await
        {
            Ok(_) => match response_rx.await {
                Ok(result) => Ok(WriteEnqueueResult::Committed(result)),
                Err(_) => Err(AppError::Internal(
                    "write coordinator closed before returning a response".to_string(),
                )),
            },
            Err(err) => {
                drop(err);
                self.write_queues.remove(db_path);
                Ok(WriteEnqueueResult::Fallback(fallback_request))
            }
        }
    }

    pub async fn enqueue_prepared_write(
        &self,
        db_path: &str,
        mut request: GatewayRequest,
    ) -> AppResult<WriteEnqueueResult> {
        if !self.write_queue_enabled {
            return Ok(WriteEnqueueResult::Fallback(request));
        }

        request.payload.commit = Some(true);
        let fallback_request = request.clone();
        let sender = self.get_or_create_write_sender(db_path);
        let (response_tx, response_rx) = oneshot::channel();
        match sender
            .send(QueuedWrite {
                db_path: db_path.to_string(),
                request,
                mode: QueuedWriteMode::AcceptedPrepared,
                response_tx: Some(response_tx),
            })
            .await
        {
            Ok(_) => match response_rx.await {
                Ok(result) => Ok(WriteEnqueueResult::Committed(result)),
                Err(_) => Err(AppError::Internal(
                    "write coordinator closed before returning prepared response".to_string(),
                )),
            },
            Err(err) => {
                drop(err);
                self.write_queues.remove(db_path);
                Ok(WriteEnqueueResult::Fallback(fallback_request))
            }
        }
    }

    pub fn put_pending_document(
        &self,
        db_path: &str,
        id: &str,
        collection: Option<String>,
        document: Value,
    ) -> u64 {
        let revision = self.pending_revision.fetch_add(1, Ordering::Relaxed);
        self.pending_documents.insert(
            pending_document_key(db_path, id),
            PendingDocument {
                revision,
                collection,
                document,
            },
        );
        revision
    }

    pub fn get_pending_document(&self, db_path: &str, id: &str) -> Option<PendingDocument> {
        self.pending_documents
            .get(&pending_document_key(db_path, id))
            .map(|doc| doc.clone())
    }

    pub fn clear_pending_if_current(&self, db_path: &str, revisions: &[PendingWriteRevision]) {
        for item in revisions {
            let key = pending_document_key(db_path, &item.id);
            let should_remove = self
                .pending_documents
                .get(&key)
                .map(|doc| doc.revision == item.revision)
                .unwrap_or(false);
            if should_remove {
                self.pending_documents.remove(&key);
            }
        }
    }

    pub fn pending_write_count_for_db(&self, db_path: &str) -> usize {
        let prefix = format!("{db_path}\u{1f}");
        self.pending_documents
            .iter()
            .filter(|entry| entry.key().starts_with(&prefix))
            .count()
    }

    fn get_or_create_write_sender(&self, db_path: &str) -> mpsc::Sender<QueuedWrite> {
        let (tx, mut rx) = mpsc::channel::<QueuedWrite>(self.write_queue_capacity.max(1));
        match self.write_queues.entry(db_path.to_string()) {
            Entry::Occupied(existing) => return existing.get().clone(),
            Entry::Vacant(slot) => {
                slot.insert(tx.clone());
            }
        }
        let state = self.clone();
        let worker_db_path = db_path.to_string();
        let idle_secs = self.write_queue_idle_secs;
        tokio::spawn(async move {
            loop {
                let next = if idle_secs == 0 {
                    rx.recv().await
                } else {
                    match tokio::time::timeout(std::time::Duration::from_secs(idle_secs), rx.recv())
                        .await
                    {
                        Ok(item) => item,
                        Err(_) => break,
                    }
                };
                let Some(item) = next else {
                    break;
                };
                if item
                    .response_tx
                    .as_ref()
                    .map(|tx| tx.is_closed())
                    .unwrap_or(false)
                {
                    continue;
                }
                let db_path = item.db_path.clone();
                match item.mode {
                    QueuedWriteMode::AcceptedPrepared => {
                        let request = item.request;
                        let preview = crate::service::dispatcher::prepare_pending_write_preview(
                            &state, &db_path, &request,
                        )
                        .await;
                        let preview = match preview {
                            Ok(preview) => preview,
                            Err(err) => {
                                if let Some(response_tx) = item.response_tx {
                                    let _ = response_tx.send(Err(err));
                                }
                                continue;
                            }
                        };
                        let revisions = preview.revisions.clone();
                        if let Some(response_tx) = item.response_tx {
                            let _ = response_tx.send(Ok(preview.response));
                        }
                        let result = crate::service::dispatcher::dispatch_write_worker(
                            &state, &db_path, request,
                        )
                        .await;
                        if result.is_ok() {
                            state.clear_pending_if_current(&db_path, &revisions);
                        } else if let Err(err) = result {
                            state.clear_pending_if_current(&db_path, &revisions);
                            eprintln!("write-queue apply error for {}: {}", db_path, err);
                        }
                    }
                    QueuedWriteMode::Accepted | QueuedWriteMode::Committed => {
                        let result = crate::service::dispatcher::dispatch_write_worker(
                            &state,
                            &db_path,
                            item.request,
                        )
                        .await;
                        if let Some(response_tx) = item.response_tx {
                            let _ = response_tx.send(result);
                        } else if let Err(err) = result {
                            eprintln!("write-queue apply error for {}: {}", db_path, err);
                        }
                    }
                }
            }
            state.write_queues.remove(&worker_db_path);
        });
        tx
    }

    pub fn write_queue_stats(&self) -> Vec<WriteQueueStat> {
        let mut out = Vec::with_capacity(self.write_queues.len());
        for entry in self.write_queues.iter() {
            let sender = entry.value();
            let capacity = sender.max_capacity();
            let queued = capacity.saturating_sub(sender.capacity());
            out.push(WriteQueueStat {
                db: entry.key().clone(),
                queued,
                capacity,
                idle_secs: self.write_queue_idle_secs,
            });
        }
        out
    }
}

impl DbRuntimeStats {
    fn set_last_accessed_at(&self, ts: String) {
        if let Ok(mut guard) = self.last_accessed_at.write() {
            *guard = Some(ts);
        }
    }

    fn snapshot(&self) -> DbStatsSnapshot {
        DbStatsSnapshot {
            ts: now_rfc3339(),
            requests_total: self.requests_total.load(Ordering::Relaxed),
            reads_total: self.reads_total.load(Ordering::Relaxed),
            writes_total: self.writes_total.load(Ordering::Relaxed),
            errors_total: self.errors_total.load(Ordering::Relaxed),
            in_flight: self.in_flight.load(Ordering::Relaxed),
            last_accessed_at: self.last_accessed_at.read().ok().and_then(|v| v.clone()),
        }
    }
}

impl Default for DbRuntimeStats {
    fn default() -> Self {
        Self {
            requests_total: AtomicU64::new(0),
            reads_total: AtomicU64::new(0),
            writes_total: AtomicU64::new(0),
            errors_total: AtomicU64::new(0),
            in_flight: AtomicU64::new(0),
            last_accessed_at: RwLock::new(None),
        }
    }
}

impl DbRequestStatsGuard {
    pub fn finish(mut self, success: bool) {
        if let Some(stats) = self.stats.take() {
            if !success {
                stats.errors_total.fetch_add(1, Ordering::Relaxed);
            }
            stats.in_flight.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl Drop for DbRequestStatsGuard {
    fn drop(&mut self) {
        if let Some(stats) = self.stats.take() {
            stats.in_flight.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl SystemRuntimeStats {
    fn begin_request(self: &Arc<Self>, kind: SystemRequestKind) -> SystemRequestStatsGuard {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        match kind {
            SystemRequestKind::Read => {
                self.reads_total.fetch_add(1, Ordering::Relaxed);
            }
            SystemRequestKind::Write => {
                self.writes_total.fetch_add(1, Ordering::Relaxed);
            }
            SystemRequestKind::Admin => {
                self.admin_total.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.in_flight.fetch_add(1, Ordering::Relaxed);
        SystemRequestStatsGuard {
            stats: self.clone(),
            kind,
            started_at: Instant::now(),
        }
    }

    fn finish_request(&self, kind: SystemRequestKind, success: bool, elapsed_ms: u64) {
        if !success {
            self.errors_total.fetch_add(1, Ordering::Relaxed);
        }
        self.in_flight.fetch_sub(1, Ordering::Relaxed);
        self.latency_total_ms
            .fetch_add(elapsed_ms, Ordering::Relaxed);
        update_atomic_max(&self.latency_max_ms, elapsed_ms);
        self.record_bucket(kind, success, elapsed_ms);
    }

    fn record_bucket(&self, kind: SystemRequestKind, success: bool, elapsed_ms: u64) {
        let epoch_minute = current_epoch_minute();
        let mut buckets = match self.buckets.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        if let Some(bucket) = buckets
            .iter_mut()
            .find(|bucket| bucket.epoch_minute == epoch_minute)
        {
            bucket.apply(kind, success, elapsed_ms);
        } else {
            let mut bucket = SystemStatsBucket::new(epoch_minute);
            bucket.apply(kind, success, elapsed_ms);
            buckets.push_back(bucket);
        }
        while buckets.len() > 60 {
            buckets.pop_front();
        }
        while buckets
            .front()
            .map(|bucket| bucket.epoch_minute.saturating_add(60) < epoch_minute)
            .unwrap_or(false)
        {
            buckets.pop_front();
        }
    }

    fn snapshot_json(&self) -> Value {
        let total = self.requests_total.load(Ordering::Relaxed);
        let errors = self.errors_total.load(Ordering::Relaxed);
        let latency_total = self.latency_total_ms.load(Ordering::Relaxed);
        let buckets = self
            .buckets
            .lock()
            .map(|guard| guard.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let uptime_seconds = self.started_instant.elapsed().as_secs();
        json!({
            "scope": "instance",
            "durability": "memory",
            "cluster_aware": false,
            "service": {
                "version": env!("CARGO_PKG_VERSION"),
                "started_at": self.started_at,
                "uptime_seconds": uptime_seconds,
                "hostname": std::env::var("HOSTNAME").ok()
            },
            "requests": {
                "total": total,
                "reads": self.reads_total.load(Ordering::Relaxed),
                "writes": self.writes_total.load(Ordering::Relaxed),
                "admin": self.admin_total.load(Ordering::Relaxed),
                "errors": errors,
                "in_flight": self.in_flight.load(Ordering::Relaxed),
                "error_rate": ratio(errors, total),
                "avg_latency_ms": if total == 0 { 0.0 } else { latency_total as f64 / total as f64 },
                "max_latency_ms": self.latency_max_ms.load(Ordering::Relaxed)
            },
            "windows": {
                "5m": summarize_window(&buckets, 5),
                "15m": summarize_window(&buckets, 15),
                "30m": summarize_window(&buckets, 30),
                "1h": summarize_window(&buckets, 60)
            },
            "buckets": buckets.iter().map(SystemStatsBucket::to_json).collect::<Vec<_>>()
        })
    }
}

impl Default for SystemRuntimeStats {
    fn default() -> Self {
        Self {
            started_at: now_rfc3339(),
            started_instant: Instant::now(),
            requests_total: AtomicU64::new(0),
            reads_total: AtomicU64::new(0),
            writes_total: AtomicU64::new(0),
            admin_total: AtomicU64::new(0),
            errors_total: AtomicU64::new(0),
            in_flight: AtomicU64::new(0),
            latency_total_ms: AtomicU64::new(0),
            latency_max_ms: AtomicU64::new(0),
            buckets: Mutex::new(VecDeque::with_capacity(60)),
        }
    }
}

impl SystemStatsBucket {
    fn new(epoch_minute: u64) -> Self {
        Self {
            epoch_minute,
            ts: epoch_minute_to_rfc3339(epoch_minute),
            requests: 0,
            reads: 0,
            writes: 0,
            admin: 0,
            errors: 0,
            latency_total_ms: 0,
            latency_max_ms: 0,
        }
    }

    fn apply(&mut self, kind: SystemRequestKind, success: bool, elapsed_ms: u64) {
        self.requests = self.requests.saturating_add(1);
        match kind {
            SystemRequestKind::Read => self.reads = self.reads.saturating_add(1),
            SystemRequestKind::Write => self.writes = self.writes.saturating_add(1),
            SystemRequestKind::Admin => self.admin = self.admin.saturating_add(1),
        }
        if !success {
            self.errors = self.errors.saturating_add(1);
        }
        self.latency_total_ms = self.latency_total_ms.saturating_add(elapsed_ms);
        self.latency_max_ms = self.latency_max_ms.max(elapsed_ms);
    }

    fn to_json(&self) -> Value {
        json!({
            "ts": self.ts,
            "requests": self.requests,
            "reads": self.reads,
            "writes": self.writes,
            "admin": self.admin,
            "errors": self.errors,
            "avg_latency_ms": if self.requests == 0 { 0.0 } else { self.latency_total_ms as f64 / self.requests as f64 },
            "max_latency_ms": self.latency_max_ms
        })
    }
}

impl SystemRequestStatsGuard {
    pub fn finish(self, success: bool) {
        let elapsed_ms = self
            .started_at
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        self.stats.finish_request(self.kind, success, elapsed_ms);
    }
}

fn summarize_window(buckets: &[SystemStatsBucket], minutes: u64) -> Value {
    let current = current_epoch_minute();
    let min_epoch = current.saturating_sub(minutes.saturating_sub(1));
    let mut summary = SystemStatsBucket::new(min_epoch);
    summary.ts = epoch_minute_to_rfc3339(min_epoch);
    for bucket in buckets
        .iter()
        .filter(|bucket| bucket.epoch_minute >= min_epoch)
    {
        summary.requests = summary.requests.saturating_add(bucket.requests);
        summary.reads = summary.reads.saturating_add(bucket.reads);
        summary.writes = summary.writes.saturating_add(bucket.writes);
        summary.admin = summary.admin.saturating_add(bucket.admin);
        summary.errors = summary.errors.saturating_add(bucket.errors);
        summary.latency_total_ms = summary
            .latency_total_ms
            .saturating_add(bucket.latency_total_ms);
        summary.latency_max_ms = summary.latency_max_ms.max(bucket.latency_max_ms);
    }
    json!({
        "minutes": minutes,
        "requests": summary.requests,
        "reads": summary.reads,
        "writes": summary.writes,
        "admin": summary.admin,
        "errors": summary.errors,
        "requests_per_minute": if minutes == 0 { 0.0 } else { summary.requests as f64 / minutes as f64 },
        "error_rate": ratio(summary.errors, summary.requests),
        "avg_latency_ms": if summary.requests == 0 { 0.0 } else { summary.latency_total_ms as f64 / summary.requests as f64 },
        "max_latency_ms": summary.latency_max_ms
    })
}

fn update_atomic_max(target: &AtomicU64, value: u64) {
    let mut current = target.load(Ordering::Relaxed);
    while value > current {
        match target.compare_exchange(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
}

fn ratio(part: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        part as f64 / total as f64
    }
}

fn current_epoch_minute() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() / 60)
        .unwrap_or(0)
}

fn epoch_minute_to_rfc3339(epoch_minute: u64) -> String {
    let ts = (epoch_minute.saturating_mul(60)).min(i64::MAX as u64) as i64;
    chrono::DateTime::<Utc>::from_timestamp(ts, 0)
        .unwrap_or_else(Utc::now)
        .to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn pending_document_key(db_path: &str, id: &str) -> String {
    format!("{db_path}\u{1f}{id}")
}
