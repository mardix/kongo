// DB and admin operation handlers extracted from dispatcher.rs.

async fn create_db(state: &AppState, db_path: &str) -> AppResult<GatewayResponse> {
    let existed_before = state.db_manager.db_exists(db_path).await?;
    let _ = state.db_manager.get_conn_with_create(db_path, true).await?;
    if let Some(catalog) = state.system_catalog.as_ref() {
        let record = collect_system_db_record(state, db_path).await?;
        let _ = catalog.upsert_db(&record).await;
        if !existed_before {
            let _ = catalog
                .insert_event(&SystemDbEventRecord {
                    db: Some(db_path.to_string()),
                    event: "db.created".to_string(),
                    level: "info".to_string(),
                    message: Some("database created".to_string()),
                    metadata: None,
                })
                .await;
        }
    }
    Ok(GatewayResponse::ok(Some(json!({
        "db": db_path,
        "created": !existed_before
    }))))
}

async fn load_db(state: &AppState, db_path: &str) -> AppResult<GatewayResponse> {
    let loaded = state.db_manager.load_db(db_path).await?;
    Ok(GatewayResponse::ok(Some(json!({
        "loaded": loaded,
        "db": db_path
    }))))
}

async fn db_exists(state: &AppState, db_path: &str) -> AppResult<GatewayResponse> {
    let exists = state.db_manager.db_exists(db_path).await?;
    Ok(GatewayResponse::ok(Some(json!({
        "db": db_path,
        "exists": exists
    }))))
}

async fn list_commands() -> AppResult<GatewayResponse> {
    let items = vec![
        "create_db",
        "db_exists",
        "list_commands",
        "list_dbs",
        "list_all_dbs",
        "system_get_inventory",
        "system_refresh_inventory",
        "system_get_db_status",
        "system_snapshot_db_stats",
        "system_query_db_stats",
        "system_list_db_events",
        "get_system_stats",
        "system_memory",
        "cleanup_temp_artifacts",
        "insert",
        "update",
        "set",
        "upsert",
        "get",
        "count",
        "query",
        "aggregate",
        "search",
        "metrics_ingest",
        "metrics_query",
        "metrics_catalog",
        "audit_ingest",
        "audit_query",
        "user_create",
        "user_get",
        "user_list",
        "user_get_details",
        "user_update",
        "user_update_status",
        "user_delete",
        "user_create_token",
        "user_link_provider",
        "user_unlink_provider",
        "file_create",
        "file_get",
        "file_list",
        "file_update",
        "file_delete",
        "sql_execute",
        "delete",
        "drop_namespace",
        "restore_archive",
        "purge_archive",
        "set_ttl",
        "change_namespace",
        "rename_namespace",
        "get_stats",
        "get_db_stats",
        "snapshot_db_stats",
        "query_db_stats",
        "get_system_config",
        "recompute_stats",
        "list_namespaces",
        "list_tables",
        "get_table_schema",
        "load_db",
        "sync_db",
        "create_snapshot",
        "list_snapshots",
        "get_sync_status",
        "verify_db",
        "restore_snapshot",
        "compact_wal",
        "clone_db",
        "create_backup",
        "restore_backup",
        "list_backups",
        "tag_backup",
        "offload_db",
        "vacuum_db",
        "reap_db",
        "create_index",
        "drop_index",
        "list_indexes",
        "reindex_fts",
        "drop_fts_index",
        "enable_fts_index",
        "export_jsonl",
        "import_jsonl",
        "get_job",
        "list_jobs",
        "continue_job",
        "abort_job",
        "transaction",
    ];
    Ok(GatewayResponse::ok(Some(json!({
        "count": items.len(),
        "items": items
    }))))
}

async fn list_dbs(state: &AppState) -> AppResult<GatewayResponse> {
    let dbs = state.db_manager.list_active_db_paths();
    let mut items = Vec::<Value>::with_capacity(dbs.len());
    for db in dbs {
        let local_size_bytes = state.db_manager.db_local_file_size_bytes(&db).await?;
        let on_local = state.db_manager.db_exists_local_only(&db).await?;
        let on_s3 = state.db_manager.db_exists_remote_only(&db).await?;
        items.push(json!({
            "db": db,
            "local_size_bytes": local_size_bytes,
            "on_local": on_local,
            "on_s3": on_s3
        }));
    }
    Ok(GatewayResponse::ok(Some(json!({
        "count": items.len(),
        "items": items
    }))))
}

async fn list_all_dbs(state: &AppState) -> AppResult<GatewayResponse> {
    let dbs = state.db_manager.list_all_db_paths().await?;
    let mut items = Vec::<Value>::with_capacity(dbs.len());
    for db in dbs {
        let loaded = state.db_manager.is_db_loaded(&db);
        let local_size_bytes = state.db_manager.db_local_file_size_bytes(&db).await?;
        let on_local = state.db_manager.db_exists_local_only(&db).await?;
        let on_s3 = state.db_manager.db_exists_remote_only(&db).await?;
        items.push(json!({
            "db": db,
            "loaded": loaded,
            "local_size_bytes": local_size_bytes,
            "on_local": on_local,
            "on_s3": on_s3
        }));
    }
    Ok(GatewayResponse::ok(Some(json!({
        "count": items.len(),
        "items": items
    }))))
}

async fn system_get_inventory(
    state: &AppState,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let limit = req.payload.limit.unwrap_or(500).clamp(1, 5000);
    let offset = req.payload.offset.unwrap_or(0).max(0);
    if let Some(catalog) = state.system_catalog.as_ref() {
        let items = catalog.list_dbs(limit, offset).await?;
        return Ok(GatewayResponse::ok(Some(json!({
            "catalog_enabled": true,
            "count": items.len(),
            "limit": limit,
            "offset": offset,
            "items": items
        }))));
    }

    let items = collect_live_inventory(state).await?;
    Ok(GatewayResponse::ok(Some(json!({
        "catalog_enabled": false,
        "count": items.len(),
        "items": items
    }))))
}

async fn system_refresh_inventory(state: &AppState) -> AppResult<GatewayResponse> {
    let catalog = require_system_catalog(state)?;
    let records = collect_system_inventory_records(state).await?;
    for record in &records {
        catalog.upsert_db(record).await?;
    }
    catalog
        .insert_event(&SystemDbEventRecord {
            db: None,
            event: "system.inventory_refreshed".to_string(),
            level: "info".to_string(),
            message: Some(format!("refreshed {} dbs", records.len())),
            metadata: Some(json!({ "count": records.len() })),
        })
        .await?;
    Ok(GatewayResponse::ok(Some(json!({
        "catalog_enabled": true,
        "refreshed": records.len(),
        "items": records.into_iter().map(system_db_record_to_json).collect::<Vec<_>>()
    }))))
}

async fn system_get_db_status(state: &AppState, db_path: &str) -> AppResult<GatewayResponse> {
    let db = db_path.trim();
    if db.is_empty() {
        return Err(AppError::BadRequest(
            "db is required for system_get_db_status".to_string(),
        ));
    }
    let live = collect_system_db_record(state, db).await?;
    if let Some(catalog) = state.system_catalog.as_ref() {
        catalog.upsert_db(&live).await?;
        let catalog_row = catalog.get_db(db).await?;
        return Ok(GatewayResponse::ok(Some(json!({
            "catalog_enabled": true,
            "db": db,
            "live": system_db_record_to_json(live),
            "catalog": catalog_row
        }))));
    }
    Ok(GatewayResponse::ok(Some(json!({
        "catalog_enabled": false,
        "db": db,
        "live": system_db_record_to_json(live)
    }))))
}

async fn system_snapshot_db_stats(state: &AppState, db_path: &str) -> AppResult<GatewayResponse> {
    let items = write_system_stats_snapshots(
        state,
        if db_path.trim().is_empty() {
            None
        } else {
            Some(db_path.trim())
        },
    )
    .await?;
    Ok(GatewayResponse::ok(Some(json!({
        "catalog_enabled": true,
        "count": items.len(),
        "items": items
    }))))
}

pub async fn snapshot_system_catalog_tick(state: &AppState) -> AppResult<usize> {
    if state.system_catalog.is_none() {
        return Ok(0);
    }
    let items = write_system_stats_snapshots(state, None).await?;
    Ok(items.len())
}

async fn write_system_stats_snapshots(
    state: &AppState,
    target_db: Option<&str>,
) -> AppResult<Vec<Value>> {
    let catalog = require_system_catalog(state)?;
    let targets = if let Some(db) = target_db {
        vec![db.to_string()]
    } else {
        state.db_manager.list_active_db_paths()
    };
    let mut items = Vec::<Value>::new();
    for db in targets {
        let record = collect_system_db_record(state, &db).await?;
        catalog.upsert_db(&record).await?;
        let snapshot = state.db_stats_snapshot(&db);
        let stats = SystemDbStatsRecord {
            db: db.clone(),
            ts: snapshot.ts,
            requests_total: snapshot.requests_total,
            reads_total: snapshot.reads_total,
            writes_total: snapshot.writes_total,
            errors_total: snapshot.errors_total,
            in_flight: snapshot.in_flight,
            local_size_bytes: record.local_size_bytes,
            namespace_count: record.namespace_count,
            document_count: record.document_count,
            archive_count: record.archive_count,
            write_queue_depth: record.write_queue_depth,
            metadata: Some(json!({
                "loaded": record.loaded,
                "on_local": record.on_local,
                "on_s3": record.on_s3
            })),
        };
        catalog.insert_stats(&stats).await?;
        items.push(json!({
            "db": db,
            "ts": stats.ts,
            "requests_total": stats.requests_total,
            "reads_total": stats.reads_total,
            "writes_total": stats.writes_total,
            "errors_total": stats.errors_total,
            "in_flight": stats.in_flight,
            "local_size_bytes": stats.local_size_bytes,
            "namespace_count": stats.namespace_count,
            "document_count": stats.document_count,
            "archive_count": stats.archive_count,
            "write_queue_depth": stats.write_queue_depth
        }));
    }
    catalog.prune(state.system_catalog_retention_days).await?;
    Ok(items)
}

async fn system_query_db_stats(
    state: &AppState,
    db_path: &str,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let catalog = require_system_catalog(state)?;
    let payload = req.payload;
    let db = if db_path.trim().is_empty() {
        None
    } else {
        Some(db_path.trim())
    };
    let limit = payload.limit.unwrap_or(100).clamp(1, 5000);
    let offset = payload.offset.unwrap_or(0).max(0);
    let items = catalog
        .query_stats(db, payload.start, payload.end, limit, offset)
        .await?;
    Ok(GatewayResponse::ok(Some(json!({
        "catalog_enabled": true,
        "count": items.len(),
        "limit": limit,
        "offset": offset,
        "items": items
    }))))
}

async fn system_list_db_events(
    state: &AppState,
    db_path: &str,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let catalog = require_system_catalog(state)?;
    let payload = req.payload;
    let db = if db_path.trim().is_empty() {
        None
    } else {
        Some(db_path.trim())
    };
    let limit = payload.limit.unwrap_or(100).clamp(1, 5000);
    let offset = payload.offset.unwrap_or(0).max(0);
    let items = catalog.list_events(db, limit, offset).await?;
    Ok(GatewayResponse::ok(Some(json!({
        "catalog_enabled": true,
        "count": items.len(),
        "limit": limit,
        "offset": offset,
        "items": items
    }))))
}

async fn system_memory(state: &AppState) -> AppResult<GatewayResponse> {
    let mut queue_items = Vec::<Value>::new();
    let mut queued_total = 0usize;
    for q in state.write_queue_stats() {
        queued_total = queued_total.saturating_add(q.queued);
        queue_items.push(json!({
            "db": q.db,
            "queued": q.queued,
            "capacity": q.capacity,
            "idle_secs": q.idle_secs
        }));
    }
    let mem = memory_stats();
    Ok(GatewayResponse::ok(Some(json!({
        "process_memory": {
            "physical_bytes": mem.as_ref().map(|m| m.physical_mem),
            "virtual_bytes": mem.as_ref().map(|m| m.virtual_mem)
        },
        "system_stats": state.system_stats_snapshot_json(),
        "active_db_count": state.db_manager.list_active_db_paths().len(),
        "max_active_dbs": state.db_manager.max_active_dbs(),
        "db_idle_close_secs": state.db_idle_close_secs,
        "background": {
            "job_worker_concurrency": state.job_worker_concurrency,
            "sync_concurrency": state.sync_concurrency
        },
        "write_queue": {
            "enabled": state.write_queue_enabled,
            "db_queues": queue_items.len(),
            "queued_total": queued_total,
            "capacity": state.write_queue_capacity,
            "idle_secs": state.write_queue_idle_secs,
            "items": queue_items
        }
    }))))
}

async fn cleanup_temp_artifacts(
    state: &AppState,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let older_than_secs = req
        .payload
        .older_than_secs
        .and_then(|v| u64::try_from(v).ok())
        .filter(|v| *v > 0)
        .unwrap_or(600);
    let removed = state
        .db_manager
        .cleanup_temp_artifacts(older_than_secs)
        .await?;
    Ok(GatewayResponse::ok(Some(json!({
        "removed_files": removed,
        "older_than_secs": older_than_secs
    }))))
}

async fn sync_db(state: &AppState, db_path: &str) -> AppResult<GatewayResponse> {
    let report = state.db_manager.sync_db(db_path).await?;
    Ok(GatewayResponse::ok(Some(json!(report))))
}

async fn list_snapshots(state: &AppState, db_path: &str) -> AppResult<GatewayResponse> {
    let report = state.db_manager.list_db_snapshots(db_path).await?;
    Ok(GatewayResponse::ok(Some(json!(report))))
}

async fn get_sync_status(state: &AppState, db_path: &str) -> AppResult<GatewayResponse> {
    let status = state.db_manager.get_sync_status(db_path).await?;
    Ok(GatewayResponse::ok(Some(json!(status))))
}

async fn verify_db(state: &AppState, db_path: &str) -> AppResult<GatewayResponse> {
    let report = state.db_manager.verify_db(db_path).await?;
    Ok(GatewayResponse::ok(Some(json!(report))))
}

async fn restore_snapshot(
    state: &AppState,
    db_path: &str,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let snapshot_id = req.payload.snapshot_id.clone();
    let restored = state
        .db_manager
        .restore_db_snapshot(db_path, snapshot_id.as_deref())
        .await?;
    invalidate_read_cache_scope(state, db_path, None);
    Ok(GatewayResponse::ok(Some(json!({
        "db": db_path,
        "restored": restored,
        "snapshot_id": snapshot_id
    }))))
}

async fn compact_wal(
    state: &AppState,
    db_path: &str,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let retain_segments = req.payload.retain_segments.unwrap_or(1000);
    if retain_segments <= 0 {
        return Err(AppError::BadRequest(
            "retain_segments must be >= 1".to_string(),
        ));
    }
    let retain_segments = usize::try_from(retain_segments)
        .map_err(|_| AppError::BadRequest("retain_segments is too large".to_string()))?;
    let report = state
        .db_manager
        .compact_wal(db_path, retain_segments)
        .await?;
    Ok(GatewayResponse::ok(Some(json!(report))))
}

async fn create_index_op(
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let path = payload
        .index_path
        .ok_or_else(|| AppError::BadRequest("index_path is required".to_string()))?;
    let name = create_manual_index(conn, &path, payload.index_name.as_deref()).await?;
    Ok(GatewayResponse::ok(Some(json!({
        "created": true,
        "index_name": name,
        "index_path": path
    }))))
}

async fn drop_index_op(
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let dropped = drop_index(
        conn,
        payload.index_name.as_deref(),
        payload.index_path.as_deref(),
    )
    .await?;
    Ok(GatewayResponse::ok(Some(json!({
        "count": dropped.len(),
        "dropped": dropped
    }))))
}

async fn list_index_op(conn: &libsql::Connection) -> AppResult<GatewayResponse> {
    let items = list_indexes(conn).await?;
    Ok(GatewayResponse::ok(Some(json!({
        "count": items.len(),
        "items": items
    }))))
}

async fn list_tables(conn: &libsql::Connection) -> AppResult<GatewayResponse> {
    let mut rows = conn
        .query(
            "SELECT name
             FROM sqlite_master
             WHERE type = 'table'
               AND name NOT LIKE '__kdb_%'
               AND name NOT LIKE 'sqlite_%'
             ORDER BY name ASC",
            (),
        )
        .await
        .map_err(|e| AppError::Internal(format!("list_tables failed: {e}")))?;

    let mut items = Vec::<Value>::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("list_tables row read failed: {e}")))?
    {
        let name: String = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("list_tables decode failed: {e}")))?;
        items.push(json!({ "name": name }));
    }

    Ok(GatewayResponse::ok(Some(json!({
        "count": items.len(),
        "items": items
    }))))
}

async fn get_table_schema(
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let table = req
        .payload
        .table
        .or(req.payload.name)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| AppError::BadRequest("table is required".to_string()))?;
    if table.starts_with("__kdb_") || table.starts_with("sqlite_") {
        return Err(AppError::BadRequest(
            "get_table_schema cannot inspect reserved internal tables".to_string(),
        ));
    }

    let mut exists_rows = conn
        .query(
            "SELECT 1
             FROM sqlite_master
             WHERE type = 'table'
               AND name = ?
               AND name NOT LIKE '__kdb_%'
               AND name NOT LIKE 'sqlite_%'
             LIMIT 1",
            libsql::params![table.clone()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("get_table_schema lookup failed: {e}")))?;
    if exists_rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("get_table_schema lookup row read failed: {e}")))?
        .is_none()
    {
        return Err(AppError::BadRequest(format!("table not found: {table}")));
    }

    let sql = format!("PRAGMA table_info({})", quote_sql_ident(&table));
    let mut rows = conn
        .query(&sql, ())
        .await
        .map_err(|e| AppError::Internal(format!("get_table_schema failed: {e}")))?;
    let mut columns = Vec::<Value>::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("get_table_schema row read failed: {e}")))?
    {
        let cid: i64 = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("get_table_schema cid decode failed: {e}")))?;
        let name: String = row
            .get(1)
            .map_err(|e| AppError::Internal(format!("get_table_schema name decode failed: {e}")))?;
        let data_type: String = row
            .get(2)
            .map_err(|e| AppError::Internal(format!("get_table_schema type decode failed: {e}")))?;
        let notnull: i64 = row
            .get(3)
            .map_err(|e| AppError::Internal(format!("get_table_schema notnull decode failed: {e}")))?;
        let default_value = row
            .get_value(4)
            .map(libsql_value_to_json)
            .map_err(|e| AppError::Internal(format!("get_table_schema default decode failed: {e}")))?;
        let pk: i64 = row
            .get(5)
            .map_err(|e| AppError::Internal(format!("get_table_schema pk decode failed: {e}")))?;
        columns.push(json!({
            "cid": cid,
            "name": name,
            "type": data_type,
            "notnull": notnull != 0,
            "dflt_value": default_value,
            "pk": pk
        }));
    }

    Ok(GatewayResponse::ok(Some(json!({
        "table": table,
        "count": columns.len(),
        "columns": columns,
        "items": columns
    }))))
}

fn quote_sql_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

async fn reindex_fts_op(_state: &AppState, conn: &libsql::Connection) -> AppResult<GatewayResponse> {
    let job_id = Uuid::new_v4().simple().to_string();
    conn.execute(
        "INSERT INTO __kdb_jobs (
            job_id, job_type, payload_json, status, resumable
         ) VALUES (?, 'reindex_fts', ?, 'queued', 0)",
        libsql::params![job_id.clone(), json!({}).to_string()],
    )
    .await
    .map_err(|e| AppError::Internal(format!("enqueue fts reindex job failed: {e}")))?;
    Ok(GatewayResponse::ok(Some(json!({
        "job_id": job_id,
        "job_type": "reindex_fts",
        "status": "queued"
    }))))
}

async fn drop_fts_index_op(conn: &libsql::Connection) -> AppResult<GatewayResponse> {
    let job_id = Uuid::new_v4().simple().to_string();
    conn.execute(
        "INSERT INTO __kdb_jobs (
            job_id, job_type, payload_json, status, resumable
         ) VALUES (?, 'drop_fts_index', ?, 'queued', 0)",
        libsql::params![job_id.clone(), json!({}).to_string()],
    )
    .await
    .map_err(|e| AppError::Internal(format!("enqueue fts drop job failed: {e}")))?;
    Ok(GatewayResponse::ok(Some(json!({
        "job_id": job_id,
        "job_type": "drop_fts_index",
        "status": "queued"
    }))))
}

async fn enable_fts_index_op(
    _state: &AppState,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let desired = req.payload.enable.unwrap_or(true);
    // Only toggle access flag; indexing lifecycle is handled by explicit reindex/drop operations.
    set_bool_config(conn, "fts_enabled", desired).await?;
    Ok(GatewayResponse::ok(Some(json!({
        "fts_enabled": desired
    }))))
}


// Additional DB/admin handlers extracted from dispatcher.rs.

async fn get_stats(conn: &libsql::Connection, req: GatewayRequest) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let collection = require_collection(&payload)?;
    let live = stats_for_table(conn, "__kdb_documents", &collection).await?;
    let __kdb_archived = stats_for_table(conn, "__kdb_archive", &collection).await?;

    Ok(GatewayResponse::ok(Some(json!({
        "collection": collection,
        "live_count": live.0,
        "live_bytes": live.1,
        "__kdb_archive_count": __kdb_archived.0,
        "__kdb_archive_bytes": __kdb_archived.1
    }))))
}

async fn get_db_stats(state: &AppState, db_path: &str) -> AppResult<GatewayResponse> {
    Ok(GatewayResponse::ok(Some(state.db_stats_snapshot_json(db_path))))
}

async fn snapshot_db_stats(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
) -> AppResult<GatewayResponse> {
    let snapshot = state.db_stats_snapshot(db_path);
    conn.execute(
        "INSERT OR REPLACE INTO __kdb_db_stats_rollups (
            ts, requests_total, reads_total, writes_total, errors_total, in_flight, last_accessed_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?)",
        libsql::params![
            snapshot.ts.clone(),
            u64_to_i64(snapshot.requests_total, "requests_total")?,
            u64_to_i64(snapshot.reads_total, "reads_total")?,
            u64_to_i64(snapshot.writes_total, "writes_total")?,
            u64_to_i64(snapshot.errors_total, "errors_total")?,
            u64_to_i64(snapshot.in_flight, "in_flight")?,
            snapshot.last_accessed_at.clone()
        ],
    )
    .await
    .map_err(|e| AppError::Internal(format!("snapshot_db_stats insert failed: {e}")))?;

    Ok(GatewayResponse::ok(Some(json!({
        "snapshot": {
            "ts": snapshot.ts,
            "requests_total": snapshot.requests_total,
            "reads_total": snapshot.reads_total,
            "writes_total": snapshot.writes_total,
            "errors_total": snapshot.errors_total,
            "in_flight": snapshot.in_flight,
            "last_accessed_at": snapshot.last_accessed_at
        }
    }))))
}

async fn query_db_stats(
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let start = payload.start;
    let end = payload.end;
    let limit = payload.limit.unwrap_or(100).clamp(1, 1000);
    let mut rows = conn
        .query(
            "SELECT ts, requests_total, reads_total, writes_total, errors_total, in_flight, last_accessed_at
             FROM __kdb_db_stats_rollups
             WHERE (? IS NULL OR ts >= ?)
               AND (? IS NULL OR ts <= ?)
             ORDER BY ts ASC
             LIMIT ?",
            libsql::params![
                start.clone(),
                start,
                end.clone(),
                end,
                limit
            ],
        )
        .await
        .map_err(|e| AppError::Internal(format!("query_db_stats failed: {e}")))?;

    let mut items = Vec::<Value>::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("query_db_stats row read failed: {e}")))?
    {
        let ts: String = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("query_db_stats ts decode failed: {e}")))?;
        let requests_total: i64 = row.get(1).map_err(|e| {
            AppError::Internal(format!("query_db_stats requests_total decode failed: {e}"))
        })?;
        let reads_total: i64 = row.get(2).map_err(|e| {
            AppError::Internal(format!("query_db_stats reads_total decode failed: {e}"))
        })?;
        let writes_total: i64 = row.get(3).map_err(|e| {
            AppError::Internal(format!("query_db_stats writes_total decode failed: {e}"))
        })?;
        let errors_total: i64 = row.get(4).map_err(|e| {
            AppError::Internal(format!("query_db_stats errors_total decode failed: {e}"))
        })?;
        let in_flight: i64 = row.get(5).map_err(|e| {
            AppError::Internal(format!("query_db_stats in_flight decode failed: {e}"))
        })?;
        let last_accessed_at: Option<String> = row.get(6).map_err(|e| {
            AppError::Internal(format!("query_db_stats last_accessed_at decode failed: {e}"))
        })?;
        items.push(json!({
            "ts": ts,
            "requests_total": requests_total,
            "reads_total": reads_total,
            "writes_total": writes_total,
            "errors_total": errors_total,
            "in_flight": in_flight,
            "last_accessed_at": last_accessed_at
        }));
    }

    Ok(GatewayResponse::ok(Some(json!({
        "count": items.len(),
        "items": items
    }))))
}

fn u64_to_i64(value: u64, field: &str) -> AppResult<i64> {
    i64::try_from(value)
        .map_err(|_| AppError::Internal(format!("{field} is too large to persist")))
}

fn require_system_catalog(state: &AppState) -> AppResult<&crate::storage::system_catalog::SystemCatalog> {
    state.system_catalog.as_ref().ok_or_else(|| {
        AppError::Internal("system catalog is unavailable".to_string())
    })
}

async fn collect_live_inventory(state: &AppState) -> AppResult<Vec<Value>> {
    let records = collect_system_inventory_records(state).await?;
    Ok(records.into_iter().map(system_db_record_to_json).collect())
}

async fn collect_system_inventory_records(state: &AppState) -> AppResult<Vec<SystemDbRecord>> {
    let dbs = state.db_manager.list_all_db_paths().await?;
    let mut records = Vec::with_capacity(dbs.len());
    for db in dbs {
        records.push(collect_system_db_record(state, &db).await?);
    }
    Ok(records)
}

async fn collect_system_db_record(state: &AppState, db: &str) -> AppResult<SystemDbRecord> {
    let loaded = state.db_manager.is_db_loaded(db);
    let local_size_bytes = state.db_manager.db_local_file_size_bytes(db).await?;
    let on_local = state.db_manager.db_exists_local_only(db).await?;
    let on_s3 = state.db_manager.db_exists_remote_only(db).await?;
    let queue_depth = state
        .write_queue_stats()
        .into_iter()
        .find(|item| item.db == db)
        .map(|item| item.queued)
        .unwrap_or(0);
    let pending_write_count = state.pending_write_count_for_db(db);
    let runtime = state.db_stats_snapshot(db);
    let mut namespace_count = None;
    let mut document_count = None;
    let mut archive_count = None;
    let mut last_error_at = None;
    let mut last_error = None;

    if loaded {
        match state.db_manager.get_conn_with_create(db, false).await {
            Ok(conn) => match collect_loaded_db_counts(&conn).await {
                Ok(counts) => {
                    namespace_count = Some(counts.0);
                    document_count = Some(counts.1);
                    archive_count = Some(counts.2);
                }
                Err(err) => {
                    last_error_at = Some(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true));
                    last_error = Some(err.to_string());
                }
            },
            Err(err) => {
                last_error_at = Some(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true));
                last_error = Some(err.to_string());
            }
        }
    }

    Ok(SystemDbRecord {
        db: db.to_string(),
        status: if last_error.is_some() { "error" } else { "known" }.to_string(),
        storage_mode: if state.storage_mode_is_s3 { "s3" } else { "local" }.to_string(),
        on_local,
        on_s3,
        loaded,
        active: loaded,
        local_size_bytes,
        remote_size_bytes: None,
        namespace_count,
        document_count,
        archive_count,
        pending_write_count,
        write_queue_depth: queue_depth,
        last_opened_at: if loaded { runtime.last_accessed_at.clone() } else { None },
        last_closed_at: None,
        last_read_at: runtime.last_accessed_at.clone(),
        last_write_at: if runtime.writes_total > 0 {
            runtime.last_accessed_at.clone()
        } else {
            None
        },
        last_sync_at: None,
        last_backup_at: None,
        last_reaper_at: None,
        last_vacuum_at: None,
        last_error_at,
        last_error,
    })
}

async fn collect_loaded_db_counts(conn: &libsql::Connection) -> AppResult<(i64, i64, i64)> {
    let (namespace_count, document_count) = {
        let mut rows = conn
            .query(
                "SELECT COUNT(DISTINCT collection), COUNT(*) FROM __kdb_documents",
                (),
            )
            .await
            .map_err(|e| AppError::Internal(format!("system db counts query failed: {e}")))?;
        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("system db counts row failed: {e}")))?
        {
            let ns: i64 = row
                .get(0)
                .map_err(|e| AppError::Internal(format!("system namespace count decode failed: {e}")))?;
            let docs: i64 = row
                .get(1)
                .map_err(|e| AppError::Internal(format!("system document count decode failed: {e}")))?;
            (ns, docs)
        } else {
            (0, 0)
        }
    };
    let archive_count = {
        let mut rows = conn
            .query("SELECT COUNT(*) FROM __kdb_archive", ())
            .await
            .map_err(|e| AppError::Internal(format!("system archive count query failed: {e}")))?;
        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("system archive count row failed: {e}")))?
        {
            row.get(0)
                .map_err(|e| AppError::Internal(format!("system archive count decode failed: {e}")))?
        } else {
            0
        }
    };
    Ok((namespace_count, document_count, archive_count))
}

fn system_db_record_to_json(record: SystemDbRecord) -> Value {
    json!({
        "db": record.db,
        "status": record.status,
        "storage_mode": record.storage_mode,
        "on_local": record.on_local,
        "on_s3": record.on_s3,
        "loaded": record.loaded,
        "active": record.active,
        "local_size_bytes": record.local_size_bytes,
        "remote_size_bytes": record.remote_size_bytes,
        "namespace_count": record.namespace_count,
        "document_count": record.document_count,
        "archive_count": record.archive_count,
        "pending_write_count": record.pending_write_count,
        "write_queue_depth": record.write_queue_depth,
        "last_opened_at": record.last_opened_at,
        "last_closed_at": record.last_closed_at,
        "last_read_at": record.last_read_at,
        "last_write_at": record.last_write_at,
        "last_sync_at": record.last_sync_at,
        "last_backup_at": record.last_backup_at,
        "last_reaper_at": record.last_reaper_at,
        "last_vacuum_at": record.last_vacuum_at,
        "last_error_at": record.last_error_at,
        "last_error": record.last_error
    })
}

async fn get_system_config(conn: &libsql::Connection) -> AppResult<GatewayResponse> {
    let mut rows = conn
        .query(
            "SELECT key, value, updated_at
             FROM __kdb_system_config
             ORDER BY key ASC",
            (),
        )
        .await
        .map_err(|e| AppError::Internal(format!("get_system_config failed: {e}")))?;

    let mut items = Vec::<Value>::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("get_system_config row read failed: {e}")))?
    {
        let key: String = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("get_system_config decode failed: {e}")))?;
        let value: String = row
            .get(1)
            .map_err(|e| AppError::Internal(format!("get_system_config decode failed: {e}")))?;
        let updated_at: String = row
            .get(2)
            .map_err(|e| AppError::Internal(format!("get_system_config decode failed: {e}")))?;
        items.push(json!({
            "key": key,
            "value": value,
            "updated_at": updated_at
        }));
    }

    Ok(GatewayResponse::ok(Some(json!({
        "count": items.len(),
        "items": items
    }))))
}

async fn recompute_stats(
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    enqueue_admin_job(conn, "recompute_stats", &req.payload).await
}

async fn list_collections(conn: &libsql::Connection) -> AppResult<GatewayResponse> {
    let sql = "
        WITH all_collections AS (
            SELECT collection FROM __kdb_documents
            UNION
            SELECT collection FROM __kdb_archive
        )
        SELECT
            c.collection,
            COALESCE(ss.total_count, 0) AS live_count,
            COALESCE(ss.total_bytes, 0) AS live_bytes,
            COALESCE(a.__kdb_archive_count, 0) AS __kdb_archive_count,
            COALESCE(a.__kdb_archive_bytes, 0) AS __kdb_archive_bytes
        FROM all_collections c
        LEFT JOIN __kdb_system_stats ss ON ss.collection = c.collection
        LEFT JOIN (
            SELECT collection, COUNT(*) AS __kdb_archive_count, COALESCE(SUM(_size_bytes), 0) AS __kdb_archive_bytes
            FROM __kdb_archive
            GROUP BY collection
        ) a ON a.collection = c.collection
        ORDER BY c.collection";

    let mut rows = conn
        .query(sql, ())
        .await
        .map_err(|e| AppError::Internal(format!("list_collections failed: {e}")))?;

    let mut items = Vec::<Value>::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("list_collections row read failed: {e}")))?
    {
        let collection: String = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("collection decode failed: {e}")))?;
        let live_count: i64 = row
            .get(1)
            .map_err(|e| AppError::Internal(format!("live_count decode failed: {e}")))?;
        let live_bytes: i64 = row
            .get(2)
            .map_err(|e| AppError::Internal(format!("live_bytes decode failed: {e}")))?;
        let __kdb_archive_count: i64 = row
            .get(3)
            .map_err(|e| AppError::Internal(format!("__kdb_archive_count decode failed: {e}")))?;
        let __kdb_archive_bytes: i64 = row
            .get(4)
            .map_err(|e| AppError::Internal(format!("__kdb_archive_bytes decode failed: {e}")))?;
        items.push(json!({
            "collection": collection,
            "live_count": live_count,
            "live_bytes": live_bytes,
            "__kdb_archive_count": __kdb_archive_count,
            "__kdb_archive_bytes": __kdb_archive_bytes
        }));
    }

    Ok(GatewayResponse::ok(Some(json!({
        "items": items,
        "count": items.len()
    }))))
}

async fn change_namespace(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    if payload
        .collection
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_some()
    {
        return Err(AppError::BadRequest(
            "change_namespace requires payload.from_namespace and payload.to_namespace; top-level namespace is not allowed".to_string(),
        ));
    }
    let from_namespace = payload
        .from_namespace
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| AppError::BadRequest("from_namespace is required".to_string()))?
        .to_string();
    let to_namespace = payload
        .to_namespace
        .clone()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| AppError::BadRequest("to_namespace is required".to_string()))?;
    let dry_run = payload.dry_run.unwrap_or(false);
    let max_docs = payload.max_docs;

    if payload.ids.is_some() && payload.filter.is_some() {
        return Err(AppError::BadRequest(
            "ids and filter cannot be provided together".to_string(),
        ));
    }

    if from_namespace == to_namespace {
        return Ok(GatewayResponse::ok(Some(json!({
            "from_namespace": from_namespace,
            "to_namespace": to_namespace,
            "matched_count": 0,
            "moved_count": 0,
            "moved_bytes": 0
        }))));
    }

    let ids = if let Some(ids) = payload.ids {
        apply_max_docs_to_ids(ids, max_docs)?
    } else if let Some(filter) = payload.filter {
        select_ids_by_filter(conn, Some(from_namespace.as_str()), filter, max_docs).await?
    } else {
        select_ids_by_filter(
            conn,
            Some(from_namespace.as_str()),
            json!({}),
            max_docs.or(Some(-1)),
        )
        .await?
    };

    if ids.is_empty() {
        return Ok(GatewayResponse::ok(Some(json!({
            "from_namespace": from_namespace,
            "to_namespace": to_namespace,
            "matched_count": 0,
            "moved_count": 0,
            "moved_bytes": 0
        }))));
    }

    let (matched_count, moved_bytes) =
        count_and_bytes_by_ids(conn, Some(from_namespace.as_str()), &ids).await?;

    if dry_run {
        return Ok(GatewayResponse::ok(Some(json!({
            "from_namespace": from_namespace,
            "to_namespace": to_namespace,
            "matched_count": matched_count,
            "moved_count": matched_count,
            "moved_bytes": moved_bytes,
            "dry_run": true
        }))));
    }

    if matched_count == 0 {
        return Ok(GatewayResponse::ok(Some(json!({
            "from_namespace": from_namespace,
            "to_namespace": to_namespace,
            "matched_count": 0,
            "moved_count": 0,
            "moved_bytes": 0
        }))));
    }

    let tx = conn
        .transaction()
        .await
        .map_err(|e| AppError::Internal(format!("change_namespace tx begin failed: {e}")))?;

    let placeholders = vec!["?"; ids.len()].join(", ");
    let mut update_binds = vec![
        libsql::Value::Text(to_namespace.clone()),
        libsql::Value::Text(from_namespace.clone()),
    ];
    update_binds.extend(ids.iter().map(|id| libsql::Value::Text(id.clone())));
    let moved_count = tx
        .execute(
            &format!(
                "UPDATE __kdb_documents
                 SET collection = ?,
                     _modified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE collection = ? AND id IN ({placeholders})"
            ),
            update_binds,
        )
        .await
        .map_err(|e| AppError::Internal(format!("change_namespace update failed: {e}")))?;

    tx.execute(
        "UPDATE __kdb_system_stats
         SET total_count = total_count - ?, total_bytes = total_bytes - ?
         WHERE collection = ?",
        libsql::params![moved_count as i64, moved_bytes, from_namespace.clone()],
    )
    .await
    .map_err(|e| AppError::Internal(format!("change_namespace source stats update failed: {e}")))?;

    tx.execute(
        "INSERT INTO __kdb_system_stats (collection, total_count, total_bytes)
         VALUES (?, ?, ?)
         ON CONFLICT(collection) DO UPDATE
         SET total_count = total_count + excluded.total_count,
             total_bytes = total_bytes + excluded.total_bytes",
        libsql::params![to_namespace.clone(), moved_count as i64, moved_bytes],
    )
    .await
    .map_err(|e| AppError::Internal(format!("change_namespace target stats update failed: {e}")))?;

    tx.commit()
        .await
        .map_err(|e| AppError::Internal(format!("change_namespace tx commit failed: {e}")))?;

    state
        .db_manager
        .append_wal_record(
            db_path,
            "CHANGE_NAMESPACE",
            &json!({
                "from_namespace": from_namespace,
                "to_namespace": to_namespace,
                "moved_count": moved_count,
                "moved_bytes": moved_bytes
            })
            .to_string(),
        )
        .await?;

    Ok(GatewayResponse::ok(Some(json!({
        "from_namespace": from_namespace,
        "to_namespace": to_namespace,
        "matched_count": matched_count,
        "moved_count": moved_count,
        "moved_bytes": moved_bytes
    }))))
}

async fn rename_namespace(
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    if payload
        .collection
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_some()
    {
        return Err(AppError::BadRequest(
            "rename_namespace requires payload.from_namespace and payload.to_namespace; top-level namespace is not allowed".to_string(),
        ));
    }
    let from_namespace = payload
        .from_namespace
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| AppError::BadRequest("from_namespace is required".to_string()))?
        .to_string();
    let to_namespace = payload
        .to_namespace
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| AppError::BadRequest("to_namespace is required".to_string()))?
        .to_string();
    if from_namespace == to_namespace {
        return Err(AppError::BadRequest(
            "to_namespace must be different from from_namespace".to_string(),
        ));
    }

    let tx = conn
        .transaction()
        .await
        .map_err(|e| AppError::Internal(format!("rename_namespace tx begin failed: {e}")))?;
    let live_renamed = tx
        .execute(
            "UPDATE __kdb_documents
             SET collection = ?, _modified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE collection = ?",
            libsql::params![to_namespace.clone(), from_namespace.clone()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("rename_namespace live update failed: {e}")))?;
    let __kdb_archive_renamed = tx
        .execute(
            "UPDATE __kdb_archive SET collection = ? WHERE collection = ?",
            libsql::params![to_namespace.clone(), from_namespace.clone()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("rename_namespace __kdb_archive update failed: {e}")))?;
    tx.commit()
        .await
        .map_err(|e| AppError::Internal(format!("rename_namespace tx commit failed: {e}")))?;

    let _ = apply_recompute_stats(conn).await?;
    Ok(GatewayResponse::ok(Some(json!({
        "renamed": true,
        "from_namespace": from_namespace,
        "to_namespace": to_namespace,
        "live_renamed": live_renamed,
        "__kdb_archive_renamed": __kdb_archive_renamed
    }))))
}

async fn vacuum(conn: &libsql::Connection) -> AppResult<GatewayResponse> {
    enqueue_admin_job(conn, "vacuum_db", &OperationPayload::default()).await
}

async fn apply_recompute_stats(conn: &libsql::Connection) -> AppResult<Value> {
    conn.execute("DELETE FROM __kdb_system_stats", ())
        .await
        .map_err(|e| AppError::Internal(format!("recompute_stats clear failed: {e}")))?;
    conn.execute(
        "INSERT INTO __kdb_system_stats (collection, total_count, total_bytes)
         SELECT collection, COUNT(*), COALESCE(SUM(_size_bytes), 0)
         FROM __kdb_documents
         GROUP BY collection",
        (),
    )
    .await
    .map_err(|e| AppError::Internal(format!("recompute_stats rebuild failed: {e}")))?;
    Ok(json!({
        "scope": "all",
        "recomputed": true
    }))
}

async fn apply_vacuum(conn: &libsql::Connection) -> AppResult<Value> {
    let before_pages = pragma_i64(conn, "page_count").await?;
    let before_free = pragma_i64(conn, "freelist_count").await?;
    conn.execute("VACUUM", ())
        .await
        .map_err(|e| AppError::Internal(format!("vacuum failed: {e}")))?;
    let after_pages = pragma_i64(conn, "page_count").await?;
    let after_free = pragma_i64(conn, "freelist_count").await?;

    Ok(json!({
        "vacuumed": true,
        "before": {"page_count": before_pages, "freelist_count": before_free},
        "after": {"page_count": after_pages, "freelist_count": after_free}
    }))
}

async fn reap_db(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
) -> AppResult<GatewayResponse> {
    let stats = reap_conn(
        conn,
        state.__kdb_archive_ttl_secs,
        state.metric_events_retention_days,
    )
    .await?;
    if stats.has_changes() {
        state
            .db_manager
            .append_wal_record(
                db_path,
                "REAP_DB",
                &json!({
                    "moved_to_archive": stats.moved_to_archive,
                    "deleted_from_archive": stats.deleted_from_archive,
                    "deleted_metric_events": stats.deleted_metric_events,
                    "transitioned_identity_statuses": stats.transitioned_identity_statuses,
                    "deleted_identity_tokens": stats.deleted_identity_tokens
                })
                .to_string(),
            )
            .await?;
    }
    Ok(GatewayResponse::ok(Some(json!({
        "reaped": true,
        "moved_to_archive": stats.moved_to_archive,
        "deleted_from_archive": stats.deleted_from_archive,
        "deleted_metric_events": stats.deleted_metric_events,
        "transitioned_identity_statuses": stats.transitioned_identity_statuses,
        "deleted_identity_tokens": stats.deleted_identity_tokens
    }))))
}

async fn clone_db(
    state: &AppState,
    db_path: &str,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let to_db_path = payload
        .to_db_path
        .ok_or_else(|| AppError::BadRequest("to_db_path is required".to_string()))?;
    state.db_manager.clone_db(db_path, &to_db_path).await?;
    Ok(GatewayResponse::ok(Some(json!({
        "cloned": true,
        "from_db": db_path,
        "to_db_path": to_db_path
    }))))
}

async fn create_backup(
    state: &AppState,
    db_path: &str,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let requested_backup_db_path = payload
        .backup_db_path
        .unwrap_or_else(|| default_backup_db_path(state, db_path));
    let backup_tag = payload
        .backup_tag
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let backup_id = Uuid::new_v4().simple().to_string();
    let job_id = Uuid::new_v4().simple().to_string();
    let conn = state
        .db_manager
        .get_conn_with_create(db_path, false)
        .await?;
    conn.execute(
        "INSERT INTO __kdb_jobs (
            job_id, job_type, requested_backup_db_path, backup_tag, status, backup_id
         ) VALUES (?, 'backup_db', ?, ?, 'queued', ?)",
        libsql::params![
            job_id.clone(),
            requested_backup_db_path.clone(),
            backup_tag.clone(),
            backup_id.clone()
        ],
    )
    .await
    .map_err(|e| AppError::Internal(format!("enqueue backup job failed: {e}")))?;

    let bg_state = state.clone();
    let bg_db_path = db_path.to_string();
    tokio::spawn(async move {
        let conn = match bg_state
            .db_manager
            .get_conn_with_create(&bg_db_path, false)
            .await
        {
            Ok(v) => v,
            Err(_) => return,
        };
        let _ = process_next_backup_job_for_db(&bg_state, &bg_db_path, &conn).await;
    });

    Ok(GatewayResponse::ok(Some(json!({
        "enqueued": true,
        "job_id": job_id,
        "backup_id": backup_id,
        "requested_backup_db_path": requested_backup_db_path,
        "backup_tag": backup_tag
    }))))
}

async fn process_next_backup_job_for_db(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
) -> AppResult<bool> {
    let (job_id, requested_backup_db_path, backup_tag, backup_id) = {
        let mut rows = conn
            .query(
                "SELECT job_id, requested_backup_db_path, backup_tag, backup_id
                 FROM __kdb_jobs
                 WHERE status IN ('queued','retrying')
                   AND job_type = 'backup_db'
                 ORDER BY created_at ASC
                 LIMIT 1",
                (),
            )
            .await
            .map_err(|e| AppError::Internal(format!("backup worker query failed: {e}")))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("backup worker row failed: {e}")))?
        else {
            return Ok(false);
        };

        let job_id: String = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("backup worker decode failed: {e}")))?;
        let requested_backup_db_path: String = row
            .get(1)
            .map_err(|e| AppError::Internal(format!("backup worker decode failed: {e}")))?;
        let backup_tag: Option<String> = row
            .get(2)
            .map_err(|e| AppError::Internal(format!("backup worker decode failed: {e}")))?;
        let backup_id: String = row
            .get(3)
            .map_err(|e| AppError::Internal(format!("backup worker decode failed: {e}")))?;
        (job_id, requested_backup_db_path, backup_tag, backup_id)
    };

    let worker_id = format!("worker-{}", Uuid::new_v4().simple());
    let claimed = conn
        .execute(
            "UPDATE __kdb_jobs
             SET status='running',
                 worker_id=?,
                 started_at=COALESCE(started_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                 updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE job_id=? AND job_type = 'backup_db' AND status IN ('queued','retrying')",
            libsql::params![worker_id, job_id.clone()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("backup worker claim failed: {e}")))?;
    if claimed == 0 {
        return Ok(false);
    }

    let work_result: AppResult<()> = async {
        let written = state
            .db_manager
            .backup_db_with_result(db_path, &requested_backup_db_path)
            .await?;

        let mut size_bytes: Option<i64> = None;
        let mut sha256: Option<String> = None;
        let mut warning: Option<String> = None;

        match backup_artifact_meta(state, &written).await {
            Ok((size, hash)) => {
                size_bytes = Some(size);
                sha256 = Some(hash);
            }
            Err(e) => {
                warning = Some(format!("backup artifact metadata unavailable: {e}"));
            }
        }

        if let (Some(size), Some(hash)) = (size_bytes, sha256.clone()) {
            if let Err(e) = upsert_kdb_backup_catalog(
                conn,
                &backup_id,
                &written,
                size,
                &hash,
                backup_tag.as_deref(),
                "manual",
            )
            .await
            {
                warning = Some(match warning {
                    Some(prev) => format!("{prev}; backup catalog update failed: {e}"),
                    None => format!("backup catalog update failed: {e}"),
                });
            }
        }

            conn.execute(
                "UPDATE __kdb_jobs
                 SET status='completed',
                     backup_db_path=?, size_bytes=?, sha256=?,
                     last_error_code=?, last_error_message=?,
                     worker_id=NULL,
                     finished_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                     updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE job_id=? AND job_type = 'backup_db'",
                libsql::params![
                    written,
                    size_bytes,
                sha256,
                warning
                    .as_ref()
                    .map(|_| "backup_completed_with_warnings".to_string()),
                warning,
                job_id.clone()
            ],
        )
        .await
        .map_err(|e| AppError::Internal(format!("backup completion update failed: {e}")))?;
        Ok(())
    }
    .await;

    if let Err(err) = work_result {
        let _ = conn
            .execute(
                "UPDATE __kdb_jobs
                 SET status='failed',
                     last_error_code='backup_failed',
                     last_error_message=?,
                     worker_id=NULL,
                     finished_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                     updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE job_id=? AND job_type = 'backup_db'",
                libsql::params![err.to_string(), job_id],
            )
            .await;
    }
    Ok(true)
}

async fn restore_backup(
    state: &AppState,
    db_path: &str,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let conn = state
        .db_manager
        .get_conn_with_create(db_path, false)
        .await?;
    let backup_db_path = resolve_restore_backup_path(&conn, req.payload).await?;
    let restored = state
        .db_manager
        .restore_from_backup(db_path, &backup_db_path)
        .await?;
    invalidate_read_cache_scope(state, db_path, None);
    Ok(GatewayResponse::ok(Some(json!({
        "restored": restored,
        "db": db_path,
        "backup_db_path": backup_db_path
    }))))
}

async fn list_backups(
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let limit = payload.limit.unwrap_or(50).clamp(1, 500);
    let offset = payload.offset.unwrap_or(0).max(0);
    let tag = payload
        .backup_tag
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let mut rows = if let Some(tag) = tag {
        conn.query(
            "SELECT backup_id, backup_db_path, created_at, size_bytes, sha256, backup_tag, source
             FROM __kdb_backup_catalog
             WHERE backup_tag = ?
             ORDER BY created_at DESC
             LIMIT ? OFFSET ?",
            libsql::params![tag, limit, offset],
        )
        .await
        .map_err(|e| AppError::Internal(format!("list_backups failed: {e}")))?
    } else {
        conn.query(
            "SELECT backup_id, backup_db_path, created_at, size_bytes, sha256, backup_tag, source
             FROM __kdb_backup_catalog
             ORDER BY created_at DESC
             LIMIT ? OFFSET ?",
            libsql::params![limit, offset],
        )
        .await
        .map_err(|e| AppError::Internal(format!("list_backups failed: {e}")))?
    };
    let mut items = Vec::<Value>::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("list_backups row failed: {e}")))?
    {
        let backup_id: String = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("list_backups decode failed: {e}")))?;
        let backup_db_path: String = row
            .get(1)
            .map_err(|e| AppError::Internal(format!("list_backups decode failed: {e}")))?;
        let created_at: String = row
            .get(2)
            .map_err(|e| AppError::Internal(format!("list_backups decode failed: {e}")))?;
        let size_bytes: i64 = row
            .get(3)
            .map_err(|e| AppError::Internal(format!("list_backups decode failed: {e}")))?;
        let sha256: String = row
            .get(4)
            .map_err(|e| AppError::Internal(format!("list_backups decode failed: {e}")))?;
        let backup_tag: Option<String> = row
            .get(5)
            .map_err(|e| AppError::Internal(format!("list_backups decode failed: {e}")))?;
        let source: String = row
            .get(6)
            .map_err(|e| AppError::Internal(format!("list_backups decode failed: {e}")))?;
        items.push(json!({
            "backup_id": backup_id,
            "backup_db_path": backup_db_path,
            "created_at": created_at,
            "size_bytes": size_bytes,
            "sha256": sha256,
            "backup_tag": backup_tag,
            "source": source
        }));
    }
    Ok(GatewayResponse::ok(Some(json!({
        "count": items.len(),
        "items": items
    }))))
}


async fn tag_backup(conn: &libsql::Connection, req: GatewayRequest) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let backup_tag = payload
        .backup_tag
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let affected = if let Some(backup_id) = payload.backup_id.filter(|v| !v.trim().is_empty()) {
        conn.execute(
            "UPDATE __kdb_backup_catalog SET backup_tag = ? WHERE backup_id = ?",
            libsql::params![backup_tag.clone(), backup_id.clone()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("tag_backup failed: {e}")))?
    } else if let Some(path) = payload.backup_db_path.filter(|v| !v.trim().is_empty()) {
        conn.execute(
            "UPDATE __kdb_backup_catalog SET backup_tag = ? WHERE backup_db_path = ?",
            libsql::params![backup_tag.clone(), path.clone()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("tag_backup failed: {e}")))?
    } else {
        return Err(AppError::BadRequest(
            "tag_backup requires backup_id or backup_db_path".to_string(),
        ));
    };
    Ok(GatewayResponse::ok(Some(json!({
        "updated": affected,
        "backup_tag": backup_tag
    }))))
}

async fn offload_db(state: &AppState, db_path: &str) -> AppResult<GatewayResponse> {
    state.db_manager.offload_db(db_path).await?;
    Ok(GatewayResponse::ok(Some(json!({
        "offloaded": true,
        "db": db_path
    }))))
}
