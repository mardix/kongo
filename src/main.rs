//! Binary entrypoint: loads config, starts background tasks, and serves HTTP routes.

mod api;
mod config;
mod error;
mod query;
mod service;
mod state;
mod storage;

use std::net::SocketAddr;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use crate::api::router::build_router;
use crate::config::{
    BackupMode, JsonStorageFormat, KongodbConfig, ReplicationMode, S3Topology, StorageMode,
    WriteAckMode,
};
use crate::state::AppState;
use crate::storage::manager::MultiDbManager;
use crate::storage::system_catalog::SystemCatalog;

#[tokio::main]
async fn main() {
    if let Ok(raw) = std::env::var("KONGODB_S3_TOPOLOGY") {
        let normalized = raw.trim().to_ascii_lowercase();
        if normalized != "single" && normalized != "multi" {
            eprintln!(
                "startup error: KONGODB_S3_TOPOLOGY must be single|multi, got: {}",
                raw.trim()
            );
            std::process::exit(2);
        }
    }
    let cfg = KongodbConfig::from_env();
    let runtime_access_key = match cfg.auth.mode.as_str() {
        "access_key" => match cfg.auth.access_key.clone() {
            Some(key) => Some(key),
            None => {
                eprintln!(
                    "startup error: KONGODB_ACCESS_KEY is required when KONGODB_AUTH_MODE=access_key"
                );
                std::process::exit(2);
            }
        },
        "none" => None,
        other => {
            eprintln!("startup error: KONGODB_AUTH_MODE must be access_key|none, got: {other}");
            std::process::exit(2);
        }
    };
    if matches!(cfg.backup.mode, BackupMode::S3) && !matches!(cfg.storage.mode, StorageMode::S3) {
        eprintln!("startup error: an s3:// KONGODB_BACKUP_PATH requires KONGODB_STORAGE_MODE=s3");
        std::process::exit(2);
    }
    let mode = cfg.storage.mode.clone();
    let preload_dbs = cfg
        .storage
        .s3
        .as_ref()
        .map(|c| c.preload_dbs.clone())
        .unwrap_or_default();
    let manager = MultiDbManager::new(cfg.storage.clone(), true, cfg.runtime.max_active_dbs);
    let system_catalog = Some(SystemCatalog::new(cfg.storage.data_dir.clone()));
    let app_state = AppState::new(
        manager,
        cfg.server.base_path.clone(),
        cfg.server.admin_ui_enabled,
        cfg.server.admin_ui_dir.clone(),
        cfg.server.docs_enabled,
        cfg.server.docs_file.clone(),
        cfg.server.cors_allowed_origins.clone(),
        runtime_access_key,
        cfg.delete.default_ttl_secs.map(|v| v as i64),
        cfg.reaper.__kdb_archive_ttl_secs,
        matches!(mode, StorageMode::S3),
        matches!(cfg.backup.mode, BackupMode::S3),
        cfg.storage
            .s3
            .as_ref()
            .map(|s| s.bucket.trim().to_string())
            .filter(|v| !v.is_empty()),
        cfg.backup.local_path.clone(),
        cfg.backup.s3_path.clone(),
        cfg.export.local_path.clone(),
        cfg.cache.enabled,
        cfg.cache.ttl_secs,
        cfg.cache.max_entries,
        cfg.response.include_system_timestamps,
        cfg.response.include_namespace,
        matches!(cfg.json_storage.format, JsonStorageFormat::Jsonb),
        cfg.query.default_limit,
        cfg.legacy_aliases.enabled,
        cfg.legacy_aliases.import_pk.clone(),
        cfg.legacy_aliases.response.clone(),
        cfg.query_lookup.max_depth,
        cfg.query_lookup.uncapped_override_enabled,
        cfg.query_lookup.max_concurrency,
        cfg.query_lookup.default_limit,
        cfg.server.operation_timeout_ms,
        cfg.import.job_retention_days,
        cfg.export.job_retention_days,
        cfg.export.batch_size,
        cfg.metric_events.cache_enabled,
        cfg.metric_events.cache_ttl_secs,
        cfg.metric_events.insert_batch_size,
        cfg.metric_events.retention_days,
        cfg.metric_events.query_default_limit,
        cfg.metric_events.query_max_limit,
        cfg.mutation.strict_mutation_operators,
        cfg.write_queue.enabled,
        match cfg.write_queue.ack_mode {
            WriteAckMode::Accepted => "accepted".to_string(),
            WriteAckMode::Committed => "committed".to_string(),
        },
        cfg.write_queue.capacity,
        cfg.write_queue.idle_secs,
        cfg.runtime.db_idle_close_secs,
        cfg.runtime.job_worker_concurrency,
        cfg.runtime.sync_concurrency,
        cfg.system_catalog.retention_days,
        system_catalog,
    );
    let reaper_manager = app_state.db_manager.clone();
    let reaper_state = app_state.clone();
    let reaper_interval_secs = cfg.reaper.interval_secs.max(1);
    let reaper_concurrency = cfg.reaper.max_concurrency.max(1);
    let __kdb_archive_ttl_secs = cfg.reaper.__kdb_archive_ttl_secs;
    let metric_events_retention_days = cfg.metric_events.retention_days;
    let temp_cleanup_interval_secs = cfg.reaper.temp_cleanup_interval_secs.max(1);
    let temp_cleanup_older_than_secs = cfg.reaper.temp_cleanup_older_than_secs.max(1);
    let db_idle_close_secs = cfg.runtime.db_idle_close_secs;
    let auto_index_cfg = cfg.auto_index.clone();
    let backup_cfg = cfg.backup.clone();
    let import_cfg = cfg.import.clone();
    let export_cfg = cfg.export.clone();
    let replication_async = cfg
        .storage
        .s3
        .as_ref()
        .map(|s| matches!(s.replication_mode, ReplicationMode::Async))
        .unwrap_or(false);
    let replication_flush_secs = cfg
        .storage
        .s3
        .as_ref()
        .map(|s| s.flush_interval_secs.max(1))
        .unwrap_or(2);
    let remote_sync_enabled = cfg
        .storage
        .s3
        .as_ref()
        .map(|s| s.remote_sync_enabled)
        .unwrap_or(false);
    let s3_topology = cfg
        .storage
        .s3
        .as_ref()
        .map(|s| s.topology.clone())
        .unwrap_or(S3Topology::Single);
    let remote_sync_interval_secs = cfg
        .storage
        .s3
        .as_ref()
        .map(|s| s.remote_sync_interval_secs.max(1))
        .unwrap_or(10);
    let sync_concurrency = cfg.runtime.sync_concurrency.max(1);
    if matches!(mode, StorageMode::S3) {
        eprintln!(
            "s3 topology={} remote_sync={} interval_secs={}",
            match s3_topology {
                S3Topology::Single => "single",
                S3Topology::Multi => "multi",
            },
            remote_sync_enabled,
            remote_sync_interval_secs
        );
    }
    let backup_target = if matches!(cfg.backup.mode, BackupMode::S3) {
        let raw = backup_cfg
            .s3_path
            .clone()
            .unwrap_or_else(|| "data/kongodb/backups".to_string());
        if raw.starts_with("s3://") {
            raw
        } else {
            let maybe_bucket = cfg
                .storage
                .s3
                .as_ref()
                .map(|s| s.bucket.trim().to_string())
                .filter(|b| !b.is_empty());
            if let Some(bucket) = maybe_bucket {
                format!("s3://{}/{}", bucket, raw.trim_matches('/'))
            } else {
                raw
            }
        }
    } else {
        backup_cfg.local_path.clone()
    };

    match app_state
        .db_manager
        .cleanup_temp_artifacts(temp_cleanup_older_than_secs)
        .await
    {
        Ok(removed) if removed > 0 => {
            eprintln!("startup temp-cleanup removed_files={removed}");
        }
        Ok(_) => {}
        Err(err) => eprintln!("startup temp-cleanup error: {err}"),
    }

    tokio::spawn(async move {
        let running = Arc::new(AtomicBool::new(false));
        let mut error_streak = 0u32;
        let mut last_temp_cleanup_at = unix_now_secs();
        loop {
            tokio::time::sleep(next_worker_sleep(reaper_interval_secs, error_streak)).await;
            if running.swap(true, Ordering::SeqCst) {
                continue;
            }
            match reaper_manager
                .run_ttl_reaper(
                    __kdb_archive_ttl_secs,
                    metric_events_retention_days,
                    reaper_concurrency,
                )
                .await
            {
                Ok(_) => error_streak = 0,
                Err(err) => {
                    error_streak = (error_streak + 1).min(6);
                    eprintln!("reaper error: {err}");
                }
            }
            if unix_now_secs().saturating_sub(last_temp_cleanup_at) >= temp_cleanup_interval_secs {
                match reaper_manager
                    .cleanup_temp_artifacts(temp_cleanup_older_than_secs)
                    .await
                {
                    Ok(removed) if removed > 0 => {
                        eprintln!("temp-cleanup removed_files={removed}");
                    }
                    Ok(_) => {}
                    Err(err) => eprintln!("temp-cleanup error: {err}"),
                }
                last_temp_cleanup_at = unix_now_secs();
            }
            if db_idle_close_secs > 0 {
                match reaper_manager.close_idle_dbs(db_idle_close_secs).await {
                    Ok(closed) if closed > 0 => eprintln!("idle-db-close closed={closed}"),
                    Ok(_) => {}
                    Err(err) => eprintln!("idle-db-close error: {err}"),
                }
            }
            match crate::service::dispatcher::snapshot_system_catalog_tick(&reaper_state).await {
                Ok(count) if count > 0 => eprintln!("system-catalog snapshots={count}"),
                Ok(_) => {}
                Err(err) => eprintln!("system-catalog snapshot error: {err}"),
            }
            running.store(false, Ordering::SeqCst);
        }
    });

    {
        let auto_index_manager = app_state.db_manager.clone();
        tokio::spawn(async move {
            let running = Arc::new(AtomicBool::new(false));
            let mut error_streak = 0u32;
            loop {
                tokio::time::sleep(next_worker_sleep(
                    auto_index_cfg.interval_secs.max(1),
                    error_streak,
                ))
                .await;
                if running.swap(true, Ordering::SeqCst) {
                    continue;
                }
                match auto_index_manager
                    .run_auto_indexing(
                        auto_index_cfg.min_hits,
                        auto_index_cfg.max_indexes_per_db,
                        auto_index_cfg.max_new_per_run,
                    )
                    .await
                {
                    Ok(reports) => {
                        error_streak = 0;
                        for r in reports {
                            if !r.created_indexes.is_empty() {
                                eprintln!(
                                    "auto-index created for {}: {}",
                                    r.db_path,
                                    r.created_indexes.join(", ")
                                );
                            }
                        }
                    }
                    Err(err) => {
                        error_streak = (error_streak + 1).min(6);
                        eprintln!("auto-index error: {err}")
                    }
                }
                running.store(false, Ordering::SeqCst);
            }
        });
    }

    if backup_cfg.enabled {
        let backup_manager = app_state.db_manager.clone();
        tokio::spawn(async move {
            let running = Arc::new(AtomicBool::new(false));
            let mut error_streak = 0u32;
            loop {
                tokio::time::sleep(next_worker_sleep(
                    backup_cfg.interval_secs.max(1),
                    error_streak,
                ))
                .await;
                if running.swap(true, Ordering::SeqCst) {
                    continue;
                }
                match backup_manager
                    .run_auto_backup_cycle(
                        &backup_target,
                        backup_cfg.max_concurrency,
                        backup_cfg.min_interval_per_db_secs,
                        backup_cfg.min_writes_since_backup,
                        backup_cfg.max_staleness_secs,
                        backup_cfg.max_count,
                        backup_cfg.max_age_days,
                    )
                    .await
                {
                    Ok(report) => {
                        error_streak = 0;
                        if report.skipped_lease {
                            eprintln!("auto-backup skipped: lease held by another instance");
                            running.store(false, Ordering::SeqCst);
                            continue;
                        }
                        eprintln!(
                            "auto-backup cycle: discovered={} scheduled={} ok={} failed={} skipped_recent={}",
                            report.discovered,
                            report.scheduled,
                            report.succeeded,
                            report.failed,
                            report.skipped_recent
                        );
                        if report.skipped_unchanged > 0 {
                            eprintln!("auto-backup skipped unchanged={}", report.skipped_unchanged);
                        }
                        if !report.error_samples.is_empty() {
                            eprintln!("auto-backup errors: {}", report.error_samples.join(" | "));
                        }
                    }
                    Err(err) => {
                        error_streak = (error_streak + 1).min(6);
                        eprintln!("auto-backup error: {err}")
                    }
                }
                running.store(false, Ordering::SeqCst);
            }
        });
    }

    if matches!(mode, StorageMode::S3) && remote_sync_enabled {
        let remote_sync_manager = app_state.db_manager.clone();
        tokio::spawn(async move {
            let running = Arc::new(AtomicBool::new(false));
            let mut error_streak = 0u32;
            loop {
                tokio::time::sleep(next_worker_sleep(remote_sync_interval_secs, error_streak))
                    .await;
                if running.swap(true, Ordering::SeqCst) {
                    continue;
                }
                match remote_sync_manager
                    .sync_loaded_dbs_from_remote(sync_concurrency)
                    .await
                {
                    Ok(report) => {
                        error_streak = 0;
                        if report.refreshed > 0 {
                            eprintln!(
                                "remote-sync refreshed={} checked={} failed={} skipped_no_snapshot={}",
                                report.refreshed,
                                report.checked,
                                report.failed,
                                report.skipped_no_snapshot
                            );
                        }
                        if !report.error_samples.is_empty() {
                            eprintln!("remote-sync errors: {}", report.error_samples.join(" | "));
                        }
                    }
                    Err(err) => {
                        error_streak = (error_streak + 1).min(6);
                        eprintln!("remote-sync error: {err}");
                    }
                }
                running.store(false, Ordering::SeqCst);
            }
        });
    }

    if matches!(mode, StorageMode::S3) && replication_async {
        let replication_manager = app_state.db_manager.clone();
        tokio::spawn(async move {
            let running = Arc::new(AtomicBool::new(false));
            let mut error_streak = 0u32;
            loop {
                tokio::time::sleep(next_worker_sleep(replication_flush_secs, error_streak)).await;
                if running.swap(true, Ordering::SeqCst) {
                    continue;
                }
                match replication_manager.flush_replication_queue().await {
                    Ok(report) => {
                        error_streak = 0;
                        if report.queued > 0 {
                            eprintln!(
                                "replication-flush queued={} flushed={} dbs={} failed={}",
                                report.queued,
                                report.flushed_records,
                                report.dbs_flushed,
                                report.failed_records
                            );
                        }
                        if !report.error_samples.is_empty() {
                            eprintln!(
                                "replication-flush errors: {}",
                                report.error_samples.join(" | ")
                            );
                        }
                    }
                    Err(err) => {
                        error_streak = (error_streak + 1).min(6);
                        eprintln!("replication-flush error: {err}");
                    }
                }
                running.store(false, Ordering::SeqCst);
            }
        });
    }

    {
        let import_state = app_state.clone();
        tokio::spawn(async move {
            let running = Arc::new(AtomicBool::new(false));
            let mut error_streak = 0u32;
            loop {
                tokio::time::sleep(next_worker_sleep(
                    import_cfg.worker_interval_secs.max(1),
                    error_streak,
                ))
                .await;
                if running.swap(true, Ordering::SeqCst) {
                    continue;
                }
                match crate::service::dispatcher::process_import_jobs_tick(&import_state).await {
                    Ok(_) => error_streak = 0,
                    Err(err) => {
                        error_streak = (error_streak + 1).min(6);
                        eprintln!("import-worker error: {err}");
                    }
                }
                running.store(false, Ordering::SeqCst);
            }
        });
    }

    {
        let export_state = app_state.clone();
        tokio::spawn(async move {
            let running = Arc::new(AtomicBool::new(false));
            let mut error_streak = 0u32;
            loop {
                tokio::time::sleep(next_worker_sleep(
                    export_cfg.worker_interval_secs.max(1),
                    error_streak,
                ))
                .await;
                if running.swap(true, Ordering::SeqCst) {
                    continue;
                }
                match crate::service::dispatcher::process_export_jobs_tick(&export_state).await {
                    Ok(_) => error_streak = 0,
                    Err(err) => {
                        error_streak = (error_streak + 1).min(6);
                        eprintln!("export-worker error: {err}");
                    }
                }
                running.store(false, Ordering::SeqCst);
            }
        });
    }

    {
        let backup_jobs_state = app_state.clone();
        tokio::spawn(async move {
            let running = Arc::new(AtomicBool::new(false));
            let mut error_streak = 0u32;
            loop {
                tokio::time::sleep(next_worker_sleep(2, error_streak)).await;
                if running.swap(true, Ordering::SeqCst) {
                    continue;
                }
                match crate::service::dispatcher::process_backup_jobs_tick(&backup_jobs_state).await
                {
                    Ok(_) => error_streak = 0,
                    Err(err) => {
                        error_streak = (error_streak + 1).min(6);
                        eprintln!("backup-jobs worker error: {err}");
                    }
                }
                running.store(false, Ordering::SeqCst);
            }
        });
    }

    {
        let fts_jobs_state = app_state.clone();
        tokio::spawn(async move {
            let running = Arc::new(AtomicBool::new(false));
            let mut error_streak = 0u32;
            loop {
                tokio::time::sleep(next_worker_sleep(2, error_streak)).await;
                if running.swap(true, Ordering::SeqCst) {
                    continue;
                }
                match crate::service::dispatcher::process_fts_jobs_tick(&fts_jobs_state).await {
                    Ok(_) => error_streak = 0,
                    Err(err) => {
                        error_streak = (error_streak + 1).min(6);
                        eprintln!("fts-jobs worker error: {err}");
                    }
                }
                running.store(false, Ordering::SeqCst);
            }
        });
    }

    {
        let admin_jobs_state = app_state.clone();
        tokio::spawn(async move {
            let running = Arc::new(AtomicBool::new(false));
            let mut error_streak = 0u32;
            loop {
                tokio::time::sleep(next_worker_sleep(2, error_streak)).await;
                if running.swap(true, Ordering::SeqCst) {
                    continue;
                }
                match crate::service::dispatcher::process_admin_jobs_tick(&admin_jobs_state).await {
                    Ok(_) => error_streak = 0,
                    Err(err) => {
                        error_streak = (error_streak + 1).min(6);
                        eprintln!("admin-jobs worker error: {err}");
                    }
                }
                running.store(false, Ordering::SeqCst);
            }
        });
    }

    if matches!(mode, StorageMode::S3) {
        for db_path in preload_dbs {
            if let Err(err) = app_state.db_manager.load_db(&db_path).await {
                eprintln!("load_db error for {db_path}: {err}");
            }
        }
    }

    let app = build_router(app_state, cfg.server.max_request_bytes);
    let addr = SocketAddr::from((cfg.server.host, cfg.server.port));

    println!("Kongodb listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind listener");

    axum::serve(listener, app).await.expect("server crashed");
}

fn next_worker_sleep(base_secs: u64, error_streak: u32) -> Duration {
    let base = base_secs.max(1);
    let backoff = 1u64 << error_streak.min(3);
    let scaled = base.saturating_mul(backoff).max(1);
    let jitter = (scaled / 5).max(1);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let span = jitter.saturating_mul(2).saturating_add(1);
    let delta = (nanos % span) as i64 - jitter as i64;
    let secs = (scaled as i64 + delta).max(1) as u64;
    Duration::from_secs(secs)
}

fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
