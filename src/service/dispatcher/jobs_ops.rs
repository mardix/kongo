// Async job, import/export, and unified job operation helpers extracted from dispatcher.rs.

async fn export_jsonl(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let _ = resolve_collection_scope_optional_collection(&payload)?;
    let _ = build_order_by(&payload.sort)?;
    if payload.archive_only.unwrap_or(false) && payload.include_archive.unwrap_or(false) {
        return Err(AppError::BadRequest(
            "include_archive and archive_only cannot both be true".to_string(),
        ));
    }
    let compress = payload.compress.unwrap_or(true);
    let include_system_timestamps = payload.include_system_timestamps.unwrap_or(true);
    let target_path =
        normalize_export_target_path(state, db_path, payload.target_path.as_deref(), compress)?;
    let payload_json = serde_json::to_string(&payload)
        .map_err(|e| AppError::Internal(format!("export payload encode failed: {e}")))?;
    let job_id = Uuid::new_v4().simple().to_string();
    conn.execute(
        "INSERT INTO __kdb_jobs (
            job_id, job_type, collection, payload_json, target_path, compress, status, resumable
         ) VALUES (?, 'export_jsonl', ?, ?, ?, ?, 'queued', 1)",
        libsql::params![
            job_id.clone(),
            payload.collection.clone(),
            payload_json,
            target_path.clone(),
            if compress { 1 } else { 0 }
        ],
    )
    .await
    .map_err(|e| AppError::Internal(format!("enqueue export job failed: {e}")))?;

    Ok(GatewayResponse::ok(Some(json!({
        "job_id": job_id,
        "status": "queued",
        "target_path": target_path,
        "compress": compress,
        "include_system_timestamps": include_system_timestamps
    }))))
}


async fn continue_export(
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let job_id = req
        .payload
        .job_id
        .ok_or_else(|| AppError::BadRequest("job_id is required".to_string()))?;
    let job = fetch_export_job(conn, &job_id).await?;
    let status = job
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if status == "aborted" || status == "completed" {
        return Err(AppError::BadRequest(format!(
            "continue_export not allowed for status={status}"
        )));
    }
    if status != "failed" {
        return Err(AppError::BadRequest(
            "continue_export requires status=failed".to_string(),
        ));
    }
    conn.execute(
        "UPDATE __kdb_jobs
         SET status = 'retrying', last_error_code = NULL, last_error_message = NULL,
             worker_id = NULL, lease_expires_at = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE job_id = ? AND job_type = 'export_jsonl'",
        libsql::params![job_id.clone()],
    )
    .await
    .map_err(|e| AppError::Internal(format!("continue_export failed: {e}")))?;
    Ok(GatewayResponse::ok(Some(json!({
        "job_id": job_id,
        "status": "retrying"
    }))))
}

async fn abort_export(
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let job_id = req
        .payload
        .job_id
        .ok_or_else(|| AppError::BadRequest("job_id is required".to_string()))?;
    let job = fetch_export_job(conn, &job_id).await?;
    let status = job
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if status == "completed" {
        return Err(AppError::BadRequest(
            "abort_export not allowed for completed jobs".to_string(),
        ));
    }
    conn.execute(
        "UPDATE __kdb_jobs
         SET status = 'aborted', worker_id = NULL, lease_expires_at = NULL, finished_at = COALESCE(finished_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE job_id = ? AND job_type = 'export_jsonl'",
        libsql::params![job_id.clone()],
    )
    .await
    .map_err(|e| AppError::Internal(format!("abort_export failed: {e}")))?;
    Ok(GatewayResponse::ok(Some(json!({
        "job_id": job_id,
        "status": "aborted"
    }))))
}

async fn import_jsonl(
    state: &AppState,
    _db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let collection = require_collection(&payload)?;
    let source_path = payload.source_path.ok_or_else(|| {
        AppError::BadRequest("source_path is required for import_jsonl".to_string())
    })?;
    let on_conflict = payload.on_conflict.unwrap_or_else(|| "error".to_string());
    if !matches!(on_conflict.as_str(), "error" | "skip" | "replace" | "merge") {
        return Err(AppError::BadRequest(
            "on_conflict must be one of: error, skip, replace, merge".to_string(),
        ));
    }
    let batch_size = payload.batch_size.unwrap_or(500).clamp(1, 10_000) as i64;
    let ignore_input_id = payload.ignore_input_id.unwrap_or(false);
    let allow_system_timestamps = payload.allow_system_timestamps.unwrap_or(false);
    let resumable = payload.resumable.unwrap_or(false);
    let alias_import_pk = parse_alias_import_pk_override(payload.alias_import_pk.as_ref())?;
    let drop_keys = parse_drop_keys_override(payload.drop_keys.as_ref())?;
    let alias_import_pk_json = if alias_import_pk.is_empty() {
        None
    } else {
        Some(
            serde_json::to_string(&alias_import_pk)
                .map_err(|e| AppError::BadRequest(format!("invalid alias_import_pk: {e}")))?,
        )
    };
    let drop_keys_json = if drop_keys.is_empty() {
        None
    } else {
        Some(
            serde_json::to_string(&drop_keys)
                .map_err(|e| AppError::BadRequest(format!("invalid drop_keys: {e}")))?,
        )
    };
    let mut source_hash = payload
        .source_hash
        .clone()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    if source_hash.is_none() && source_path.starts_with("s3://") {
        source_hash = state
            .db_manager
            .get_s3_uri_source_hash(&source_path)
            .await?;
    }

    if let Some(ref hash) = source_hash {
        let mut rows = conn
            .query(
            "SELECT job_id, status FROM __kdb_jobs
                 WHERE job_type = 'import_jsonl'
                   AND collection = ? AND source_hash = ? AND on_conflict = ? AND batch_size = ?
                   AND ignore_input_id = ?
                   AND allow_system_timestamps = ?
                   AND COALESCE(alias_import_pk,'') = COALESCE(?, '')
                   AND COALESCE(drop_keys_json,'') = COALESCE(?, '')
                   AND status IN ('queued','running','retrying','completed')
                 ORDER BY created_at DESC LIMIT 1",
                libsql::params![
                    collection.clone(),
                    hash.clone(),
                    on_conflict.clone(),
                    batch_size,
                    if ignore_input_id { 1 } else { 0 },
                    if allow_system_timestamps { 1 } else { 0 },
                    alias_import_pk_json.clone(),
                    drop_keys_json.clone()
                ],
            )
            .await
            .map_err(|e| AppError::Internal(format!("import dedupe lookup failed: {e}")))?;
        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("import dedupe row failed: {e}")))?
        {
            let job_id: String = row
                .get(0)
                .map_err(|e| AppError::Internal(format!("import dedupe decode failed: {e}")))?;
            let status: String = row
                .get(1)
                .map_err(|e| AppError::Internal(format!("import dedupe decode failed: {e}")))?;
            return Ok(GatewayResponse::ok(Some(json!({
                "job_id": job_id,
                "status": status,
                "deduped": true
            }))));
        }
    }

    let job_id = Uuid::new_v4().simple().to_string();
    conn.execute(
        "INSERT INTO __kdb_jobs (
            job_id, job_type, collection, source_path, source_hash, alias_import_pk, drop_keys_json, on_conflict, ignore_input_id, allow_system_timestamps, batch_size, resumable, status
         ) VALUES (?, 'import_jsonl', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'queued')",
        libsql::params![
            job_id.clone(),
            collection.clone(),
            source_path.clone(),
            source_hash.clone(),
            alias_import_pk_json.clone(),
            drop_keys_json.clone(),
            on_conflict.clone(),
            if ignore_input_id { 1 } else { 0 },
            if allow_system_timestamps { 1 } else { 0 },
            batch_size,
            if resumable { 1 } else { 0 }
        ],
    )
    .await
    .map_err(|e| AppError::Internal(format!("enqueue import job failed: {e}")))?;

    Ok(GatewayResponse::ok(Some(json!({
        "job_id": job_id,
        "status": "queued",
        "collection": collection,
        "source_path": source_path,
        "source_hash": source_hash,
        "alias_import_pk": alias_import_pk,
        "drop_keys": drop_keys,
        "on_conflict": on_conflict,
        "ignore_input_id": ignore_input_id,
        "allow_system_timestamps": allow_system_timestamps,
        "batch_size": batch_size,
        "resumable": resumable
    }))))
}


async fn continue_import(
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let job_id = req
        .payload
        .job_id
        .ok_or_else(|| AppError::BadRequest("job_id is required".to_string()))?;
    let job = fetch_import_job(conn, &job_id).await?;
    let status = job
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if status == "aborted" || status == "completed" {
        return Err(AppError::BadRequest(format!(
            "continue_import not allowed for status={status}"
        )));
    }
    if status != "failed" {
        return Err(AppError::BadRequest(
            "continue_import requires status=failed".to_string(),
        ));
    }
    let resumable = job
        .get("resumable")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !resumable {
        return Err(AppError::BadRequest(
            "continue_import rejected: job is not resumable".to_string(),
        ));
    }

    conn.execute(
        "UPDATE __kdb_jobs
         SET status = 'retrying', last_error_code = NULL, last_error_message = NULL,
             worker_id = NULL, lease_expires_at = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE job_id = ? AND job_type = 'import_jsonl'",
        libsql::params![job_id.clone()],
    )
    .await
    .map_err(|e| AppError::Internal(format!("continue_import failed: {e}")))?;

    Ok(GatewayResponse::ok(Some(json!({
        "job_id": job_id,
        "status": "retrying"
    }))))
}

async fn abort_import(
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let job_id = req
        .payload
        .job_id
        .ok_or_else(|| AppError::BadRequest("job_id is required".to_string()))?;
    let job = fetch_import_job(conn, &job_id).await?;
    let status = job
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if status == "completed" {
        return Err(AppError::BadRequest(
            "abort_import not allowed for completed jobs".to_string(),
        ));
    }

    conn.execute(
        "UPDATE __kdb_jobs
         SET status = 'aborted', worker_id = NULL, lease_expires_at = NULL, finished_at = COALESCE(finished_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE job_id = ? AND job_type = 'import_jsonl'",
        libsql::params![job_id.clone()],
    )
    .await
    .map_err(|e| AppError::Internal(format!("abort_import failed: {e}")))?;

    Ok(GatewayResponse::ok(Some(json!({
        "job_id": job_id,
        "status": "aborted"
    }))))
}

async fn get_job(conn: &libsql::Connection, req: GatewayRequest) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let job_id = payload
        .job_id
        .ok_or_else(|| AppError::BadRequest("job_id is required".to_string()))?;
    let job_type = resolve_job_type(conn, &job_id, payload.job_type.as_deref()).await?;
    let mut job = fetch_job_typed(conn, &job_type, &job_id).await?;
    if let Some(obj) = job.as_object_mut() {
        obj.insert("job_type".to_string(), Value::String(job_type));
    }
    Ok(GatewayResponse::ok(Some(json!({ "job": job }))))
}

async fn list_jobs(conn: &libsql::Connection, req: GatewayRequest) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let limit = payload.limit.unwrap_or(50).clamp(1, 500) as usize;
    let offset = payload.offset.unwrap_or(0).max(0) as usize;
    let status = payload
        .status
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());
    let job_type = payload.job_type.as_deref().and_then(normalize_job_type);

    let types = if let Some(t) = job_type {
        vec![t.to_string()]
    } else {
        vec![
            "import_jsonl".to_string(),
            "export_jsonl".to_string(),
            "backup_db".to_string(),
            "reindex_fts".to_string(),
            "drop_fts_index".to_string(),
            "vacuum_db".to_string(),
            "recompute_stats".to_string(),
            "replication".to_string(),
        ]
    };

    // Gather a larger page from each table, then merge-sort in memory.
    let fetch_n = i64::try_from(limit.saturating_add(offset).saturating_add(50)).unwrap_or(1000);
    let mut items = Vec::<Value>::new();
    for t in types {
        let job_ids = list_job_ids_by_type(conn, &t, status.as_deref(), fetch_n).await?;
        for job_id in job_ids {
            let mut item = fetch_job_typed(conn, &t, &job_id).await?;
            if let Some(obj) = item.as_object_mut() {
                obj.insert("job_type".to_string(), Value::String(t.clone()));
            }
            items.push(item);
        }
    }

    items.sort_by(|a, b| {
        let ac = a
            .get("created_at")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let bc = b
            .get("created_at")
            .and_then(Value::as_str)
            .unwrap_or_default();
        bc.cmp(ac)
    });

    let total_count = items.len();
    let paged = items.into_iter().skip(offset).take(limit).collect::<Vec<_>>();
    Ok(GatewayResponse::ok(Some(json!({
        "count": paged.len(),
        "total_count": total_count,
        "items": paged
    }))))
}

async fn continue_job(
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload.clone();
    let job_id = payload
        .job_id
        .clone()
        .ok_or_else(|| AppError::BadRequest("job_id is required".to_string()))?;
    let job_type = resolve_job_type(conn, &job_id, payload.job_type.as_deref()).await?;

    match job_type.as_str() {
        "import_jsonl" => continue_import(
            conn,
            GatewayRequest {
                db: None,
                operation: "continue_import".to_string(),
                namespace: None,
                namespaces: None,
                payload,
                data: None,
            },
        )
        .await,
        "export_jsonl" => continue_export(
            conn,
            GatewayRequest {
                db: None,
                operation: "continue_export".to_string(),
                namespace: None,
                namespaces: None,
                payload,
                data: None,
            },
        )
        .await,
        "backup_db" => Err(AppError::BadRequest(
            "continue_job is not supported for backup_db".to_string(),
        )),
        "reindex_fts" | "drop_fts_index" | "vacuum_db" | "recompute_stats" => {
            let affected = conn
                .execute(
                    "UPDATE __kdb_jobs
                     SET status='queued',
                         last_error_code=NULL,
                         last_error_message=NULL,
                         worker_id=NULL,
                         finished_at=NULL,
                         updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     WHERE job_id = ? AND job_type = ? AND status = 'failed'",
                    libsql::params![job_id.clone(), job_type.clone()],
                )
                .await
                .map_err(|e| {
                    AppError::Internal(format!("continue_job generic update failed: {e}"))
                })?;
            Ok(GatewayResponse::ok(Some(json!({
                "job_id": job_id,
                "status": if affected > 0 { "queued" } else { "unchanged" }
            }))))
        }
        "replication" => {
            let id = job_id.parse::<i64>().map_err(|_| {
                AppError::BadRequest("replication job_id must be an integer string".to_string())
            })?;
            let affected = conn
                .execute(
                    "UPDATE __kdb_replication_jobs
                     SET status='queued', last_error=NULL, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     WHERE id = ? AND status = 'failed'",
                    libsql::params![id],
                )
                .await
                .map_err(|e| AppError::Internal(format!("continue_job replication failed: {e}")))?;
            Ok(GatewayResponse::ok(Some(json!({
                "job_id": job_id,
                "status": if affected > 0 { "queued" } else { "unchanged" }
            }))))
        }
        _ => Err(AppError::BadRequest(format!(
            "unsupported job_type for continue_job: {job_type}"
        ))),
    }
}

async fn abort_job(conn: &libsql::Connection, req: GatewayRequest) -> AppResult<GatewayResponse> {
    let payload = req.payload.clone();
    let job_id = payload
        .job_id
        .clone()
        .ok_or_else(|| AppError::BadRequest("job_id is required".to_string()))?;
    let job_type = resolve_job_type(conn, &job_id, payload.job_type.as_deref()).await?;

    match job_type.as_str() {
        "import_jsonl" => abort_import(
            conn,
            GatewayRequest {
                db: None,
                operation: "abort_import".to_string(),
                namespace: None,
                namespaces: None,
                payload,
                data: None,
            },
        )
        .await,
        "export_jsonl" => abort_export(
            conn,
            GatewayRequest {
                db: None,
                operation: "abort_export".to_string(),
                namespace: None,
                namespaces: None,
                payload,
                data: None,
            },
        )
        .await,
        "backup_db" => {
            let affected = conn
                .execute(
                    "UPDATE __kdb_jobs
                     SET status='aborted', worker_id=NULL,
                         finished_at=COALESCE(finished_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                         updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     WHERE job_id = ? AND job_type = 'backup_db' AND status IN ('queued','retrying','failed')",
                    libsql::params![job_id.clone()],
                )
                .await
                .map_err(|e| AppError::Internal(format!("abort_job backup update failed: {e}")))?;
            if affected == 0 {
                return Err(AppError::BadRequest(
                    "abort_job for backup_db requires status queued|retrying|failed".to_string(),
                ));
            }
            Ok(GatewayResponse::ok(Some(json!({
                "job_id": job_id,
                "status": "aborted"
            }))))
        }
        "reindex_fts" | "drop_fts_index" | "vacuum_db" | "recompute_stats" => {
            let affected = conn
                .execute(
                    "UPDATE __kdb_jobs
                     SET status='aborted', worker_id=NULL,
                         finished_at=COALESCE(finished_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                         updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     WHERE job_id = ? AND job_type = ? AND status IN ('queued','retrying','failed')",
                    libsql::params![job_id.clone(), job_type.clone()],
                )
                .await
                .map_err(|e| {
                    AppError::Internal(format!("abort_job generic update failed: {e}"))
                })?;
            if affected == 0 {
                return Err(AppError::BadRequest(
                    "abort_job for queued jobs requires status queued|retrying|failed".to_string(),
                ));
            }
            Ok(GatewayResponse::ok(Some(json!({
                "job_id": job_id,
                "status": "aborted"
            }))))
        }
        "replication" => {
            let id = job_id.parse::<i64>().map_err(|_| {
                AppError::BadRequest("replication job_id must be an integer string".to_string())
            })?;
            let affected = conn
                .execute("DELETE FROM __kdb_replication_jobs WHERE id = ?", libsql::params![id])
                .await
                .map_err(|e| AppError::Internal(format!("abort_job replication failed: {e}")))?;
            if affected == 0 {
                return Err(AppError::NotFound(format!(
                    "replication job not found: {job_id}"
                )));
            }
            Ok(GatewayResponse::ok(Some(json!({
                "job_id": job_id,
                "status": "aborted"
            }))))
        }
        _ => Err(AppError::BadRequest(format!(
            "unsupported job_type for abort_job: {job_type}"
        ))),
    }
}

fn normalize_job_type(v: &str) -> Option<&'static str> {
    match v.trim() {
        "import_jsonl" | "import" => Some("import_jsonl"),
        "export_jsonl" | "export" => Some("export_jsonl"),
        "create_backup" | "backup_db" | "backup" => Some("backup_db"),
        "reindex_fts" | "fts_reindex" => Some("reindex_fts"),
        "drop_fts_index" | "fts_drop" => Some("drop_fts_index"),
        "vacuum_db" | "vacuum" => Some("vacuum_db"),
        "recompute_stats" | "stats_recompute" => Some("recompute_stats"),
        "replication" | "replication_job" | "replication__kdb_jobs" => Some("replication"),
        _ => None,
    }
}

async fn resolve_job_type(
    conn: &libsql::Connection,
    job_id: &str,
    hinted: Option<&str>,
) -> AppResult<String> {
    if let Some(t) = hinted.and_then(normalize_job_type) {
        // Validate that this job_id exists in the hinted type.
        let exists = job_exists_in_type(conn, t, job_id).await?;
        if exists {
            return Ok(t.to_string());
        }
        return Err(AppError::NotFound(format!(
            "job not found for type={t}: {job_id}"
        )));
    }

    for t in [
        "import_jsonl",
        "export_jsonl",
        "backup_db",
        "reindex_fts",
        "drop_fts_index",
        "vacuum_db",
        "recompute_stats",
        "replication",
    ] {
        if job_exists_in_type(conn, t, job_id).await? {
            return Ok(t.to_string());
        }
    }
    Err(AppError::NotFound(format!("job not found: {job_id}")))
}

async fn job_exists_in_type(conn: &libsql::Connection, t: &str, job_id: &str) -> AppResult<bool> {
    let (sql, binds) = match t {
        "import_jsonl" => (
            "SELECT 1 FROM __kdb_jobs WHERE job_id = ? AND job_type = 'import_jsonl' LIMIT 1",
            libsql::params![job_id.to_string()],
        ),
        "export_jsonl" => (
            "SELECT 1 FROM __kdb_jobs WHERE job_id = ? AND job_type = 'export_jsonl' LIMIT 1",
            libsql::params![job_id.to_string()],
        ),
        "backup_db" => (
            "SELECT 1 FROM __kdb_jobs WHERE job_id = ? AND job_type = 'backup_db' LIMIT 1",
            libsql::params![job_id.to_string()],
        ),
        "reindex_fts" => (
            "SELECT 1 FROM __kdb_jobs WHERE job_id = ? AND job_type = 'reindex_fts' LIMIT 1",
            libsql::params![job_id.to_string()],
        ),
        "drop_fts_index" => (
            "SELECT 1 FROM __kdb_jobs WHERE job_id = ? AND job_type = 'drop_fts_index' LIMIT 1",
            libsql::params![job_id.to_string()],
        ),
        "vacuum_db" => (
            "SELECT 1 FROM __kdb_jobs WHERE job_id = ? AND job_type = 'vacuum_db' LIMIT 1",
            libsql::params![job_id.to_string()],
        ),
        "recompute_stats" => (
            "SELECT 1 FROM __kdb_jobs WHERE job_id = ? AND job_type = 'recompute_stats' LIMIT 1",
            libsql::params![job_id.to_string()],
        ),
        "replication" => {
            let id = job_id.parse::<i64>().map_err(|_| {
                AppError::BadRequest("replication job_id must be an integer string".to_string())
            })?;
            (
                "SELECT 1 FROM __kdb_replication_jobs WHERE id = ? LIMIT 1",
                libsql::params![id],
            )
        }
        _ => return Ok(false),
    };
    let mut rows = conn
        .query(sql, binds)
        .await
        .map_err(|e| AppError::Internal(format!("job exists check failed: {e}")))?;
    Ok(rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("job exists row check failed: {e}")))?
        .is_some())
}

async fn list_job_ids_by_type(
    conn: &libsql::Connection,
    t: &str,
    status: Option<&str>,
    limit: i64,
) -> AppResult<Vec<String>> {
    let (sql_with_status, sql_all) = match t {
        "import_jsonl" => (
            "SELECT job_id FROM __kdb_jobs WHERE job_type = 'import_jsonl' AND status = ? ORDER BY created_at DESC LIMIT ?",
            "SELECT job_id FROM __kdb_jobs WHERE job_type = 'import_jsonl' ORDER BY created_at DESC LIMIT ?",
        ),
        "export_jsonl" => (
            "SELECT job_id FROM __kdb_jobs WHERE job_type = 'export_jsonl' AND status = ? ORDER BY created_at DESC LIMIT ?",
            "SELECT job_id FROM __kdb_jobs WHERE job_type = 'export_jsonl' ORDER BY created_at DESC LIMIT ?",
        ),
        "backup_db" => (
            "SELECT job_id FROM __kdb_jobs WHERE job_type = 'backup_db' AND status = ? ORDER BY created_at DESC LIMIT ?",
            "SELECT job_id FROM __kdb_jobs WHERE job_type = 'backup_db' ORDER BY created_at DESC LIMIT ?",
        ),
        "reindex_fts" => (
            "SELECT job_id FROM __kdb_jobs WHERE job_type = 'reindex_fts' AND status = ? ORDER BY created_at DESC LIMIT ?",
            "SELECT job_id FROM __kdb_jobs WHERE job_type = 'reindex_fts' ORDER BY created_at DESC LIMIT ?",
        ),
        "drop_fts_index" => (
            "SELECT job_id FROM __kdb_jobs WHERE job_type = 'drop_fts_index' AND status = ? ORDER BY created_at DESC LIMIT ?",
            "SELECT job_id FROM __kdb_jobs WHERE job_type = 'drop_fts_index' ORDER BY created_at DESC LIMIT ?",
        ),
        "vacuum_db" => (
            "SELECT job_id FROM __kdb_jobs WHERE job_type = 'vacuum_db' AND status = ? ORDER BY created_at DESC LIMIT ?",
            "SELECT job_id FROM __kdb_jobs WHERE job_type = 'vacuum_db' ORDER BY created_at DESC LIMIT ?",
        ),
        "recompute_stats" => (
            "SELECT job_id FROM __kdb_jobs WHERE job_type = 'recompute_stats' AND status = ? ORDER BY created_at DESC LIMIT ?",
            "SELECT job_id FROM __kdb_jobs WHERE job_type = 'recompute_stats' ORDER BY created_at DESC LIMIT ?",
        ),
        "replication" => (
            "SELECT CAST(id AS TEXT) AS job_id FROM __kdb_replication_jobs WHERE status = ? ORDER BY id DESC LIMIT ?",
            "SELECT CAST(id AS TEXT) AS job_id FROM __kdb_replication_jobs ORDER BY id DESC LIMIT ?",
        ),
        _ => return Ok(Vec::new()),
    };

    let mut rows = if let Some(status) = status {
        conn.query(sql_with_status, libsql::params![status.to_string(), limit])
            .await
            .map_err(|e| AppError::Internal(format!("list jobs by type failed: {e}")))?
    } else {
        conn.query(sql_all, libsql::params![limit])
            .await
            .map_err(|e| AppError::Internal(format!("list jobs by type failed: {e}")))?
    };

    let mut ids = Vec::<String>::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("list jobs by type row failed: {e}")))?
    {
        let id: String = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("list jobs by type decode failed: {e}")))?;
        ids.push(id);
    }
    Ok(ids)
}

async fn fetch_job_typed(conn: &libsql::Connection, t: &str, job_id: &str) -> AppResult<Value> {
    match t {
        "import_jsonl" => fetch_import_job(conn, job_id).await,
        "export_jsonl" => fetch_export_job(conn, job_id).await,
        "backup_db" => fetch_backup_job(conn, job_id).await,
        "reindex_fts" | "drop_fts_index" | "vacuum_db" | "recompute_stats" => {
            fetch_generic_db_job(conn, t, job_id).await
        }
        "replication" => fetch_replication_job(conn, job_id).await,
        _ => Err(AppError::BadRequest(format!("unsupported job type: {t}"))),
    }
}

async fn enqueue_admin_job(
    conn: &libsql::Connection,
    job_type: &str,
    payload: &OperationPayload,
) -> AppResult<GatewayResponse> {
    let job_id = Uuid::new_v4().simple().to_string();
    let payload_json = serde_json::to_string(payload)
        .map_err(|e| AppError::Internal(format!("admin job payload serialize failed: {e}")))?;
    conn.execute(
        "INSERT INTO __kdb_jobs (
            job_id, job_type, payload_json, status, resumable
         ) VALUES (?, ?, ?, 'queued', 1)",
        libsql::params![job_id.clone(), job_type.to_string(), payload_json],
    )
    .await
    .map_err(|e| AppError::Internal(format!("enqueue admin job failed: {e}")))?;

    Ok(GatewayResponse::ok(Some(json!({
        "job_id": job_id,
        "job_type": job_type,
        "status": "queued"
    }))))
}

pub async fn process_admin_jobs_tick(state: &AppState) -> AppResult<usize> {
    process_loaded_db_jobs_bounded(state, |state, db_path| async move {
        let conn = match state.db_manager.get_conn_with_create(&db_path, false).await {
            Ok(v) => v,
            Err(_) => return Ok(0),
        };
        Ok(process_next_admin_job_for_db(&conn).await? as usize)
    })
    .await
}

async fn process_next_admin_job_for_db(conn: &libsql::Connection) -> AppResult<bool> {
    let worker_id = format!("worker-{}", Uuid::new_v4().simple());
    let mut rows = conn
        .query(
            "SELECT job_id, job_type
             FROM __kdb_jobs
             WHERE status IN ('queued','retrying')
               AND job_type IN ('vacuum_db','recompute_stats')
             ORDER BY created_at ASC
             LIMIT 1",
            (),
        )
        .await
        .map_err(|e| AppError::Internal(format!("admin worker query failed: {e}")))?;
    let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("admin worker row failed: {e}")))?
    else {
        return Ok(false);
    };

    let job_id: String = row
        .get(0)
        .map_err(|e| AppError::Internal(format!("admin worker decode failed: {e}")))?;
    let job_type: String = row
        .get(1)
        .map_err(|e| AppError::Internal(format!("admin worker decode failed: {e}")))?;
    drop(row);
    drop(rows);

    let claimed = conn
        .execute(
            "UPDATE __kdb_jobs
             SET status='running', worker_id=?, last_error_code=NULL, last_error_message=NULL,
                 started_at=COALESCE(started_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                 updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE job_id=? AND job_type=? AND status IN ('queued','retrying')",
            libsql::params![worker_id.clone(), job_id.clone(), job_type.clone()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("admin worker claim failed: {e}")))?;
    if claimed == 0 {
        return Ok(false);
    }

    let result_payload = match job_type.as_str() {
        "vacuum_db" => apply_vacuum(conn).await,
        "recompute_stats" => apply_recompute_stats(conn).await,
        _ => Err(AppError::BadRequest(format!(
            "unsupported admin job_type: {job_type}"
        ))),
    };

    match result_payload {
        Ok(payload) => {
            conn.execute(
                "UPDATE __kdb_jobs
                 SET status='completed', payload_json=?, worker_id=NULL,
                     finished_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                     updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE job_id=? AND job_type=?",
                libsql::params![payload.to_string(), job_id, job_type],
            )
            .await
            .map_err(|e| {
                AppError::Internal(format!("admin worker complete update failed: {e}"))
            })?;
        }
        Err(err) => {
            conn.execute(
                "UPDATE __kdb_jobs
                 SET status='failed', last_error_code='admin_job_failed', last_error_message=?, worker_id=NULL,
                     finished_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                     updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE job_id=? AND job_type=?",
                libsql::params![err.to_string(), job_id, job_type],
            )
            .await
            .map_err(|e| AppError::Internal(format!("admin worker failed update failed: {e}")))?;
        }
    }

    Ok(true)
}


pub async fn process_export_jobs_tick(state: &AppState) -> AppResult<usize> {
    process_loaded_db_jobs_bounded(state, |state, db_path| async move {
        let conn = match state.db_manager.get_conn_with_create(&db_path, false).await {
            Ok(v) => v,
            Err(_) => return Ok(0),
        };
        cleanup_export_jobs_for_db(&conn, state.export_job_retention_days).await?;
        Ok(process_next_export_job_for_db(&state, &conn).await? as usize)
    })
    .await
}

pub async fn process_backup_jobs_tick(state: &AppState) -> AppResult<usize> {
    process_loaded_db_jobs_bounded(state, |state, db_path| async move {
        let conn = match state.db_manager.get_conn_with_create(&db_path, false).await {
            Ok(v) => v,
            Err(_) => return Ok(0),
        };
        Ok(process_next_backup_job_for_db(&state, &db_path, &conn).await? as usize)
    })
    .await
}

pub async fn process_fts_jobs_tick(state: &AppState) -> AppResult<usize> {
    process_loaded_db_jobs_bounded(state, |state, db_path| async move {
        let conn = match state.db_manager.get_conn_with_create(&db_path, false).await {
            Ok(v) => v,
            Err(_) => return Ok(0),
        };
        Ok(process_next_fts_job_for_db(&state, &conn).await? as usize)
    })
    .await
}

async fn process_next_fts_job_for_db(
    _state: &AppState,
    conn: &libsql::Connection,
) -> AppResult<bool> {
    let worker_id = format!("worker-{}", Uuid::new_v4().simple());
    let mut rows = conn
        .query(
            "SELECT job_id, job_type
             FROM __kdb_jobs
             WHERE status IN ('queued','retrying')
               AND job_type IN ('reindex_fts','drop_fts_index')
             ORDER BY created_at ASC
             LIMIT 1",
            (),
        )
        .await
        .map_err(|e| AppError::Internal(format!("fts worker query failed: {e}")))?;
    let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("fts worker row failed: {e}")))?
    else {
        return Ok(false);
    };

    let job_id: String = row
        .get(0)
        .map_err(|e| AppError::Internal(format!("fts worker decode failed: {e}")))?;
    let job_type: String = row
        .get(1)
        .map_err(|e| AppError::Internal(format!("fts worker decode failed: {e}")))?;

    let claimed = conn
        .execute(
            "UPDATE __kdb_jobs
             SET status='running', worker_id=?, last_error_code=NULL, last_error_message=NULL,
                 started_at=COALESCE(started_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                 updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE job_id=? AND job_type=? AND status IN ('queued','retrying')",
            libsql::params![worker_id.clone(), job_id.clone(), job_type.clone()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("fts worker claim failed: {e}")))?;
    if claimed == 0 {
        return Ok(false);
    }

    let result_payload = match job_type.as_str() {
        "reindex_fts" => match reindex_fts(conn).await {
            Ok(indexed_count) => {
                if set_bool_config(conn, "fts_enabled", true).await.is_ok() {
                    Ok(json!({"fts_enabled": true, "indexed_count": indexed_count}))
                } else {
                    Err("set fts_enabled=true failed".to_string())
                }
            }
            Err(e) => Err(e.to_string()),
        },
        "drop_fts_index" => match set_fts_index_enabled(conn, false).await {
            Ok(_) => {
                if set_bool_config(conn, "fts_enabled", false).await.is_ok() {
                    Ok(json!({"fts_enabled": false, "dropped": true}))
                } else {
                    Err("set fts_enabled=false failed".to_string())
                }
            }
            Err(e) => Err(e.to_string()),
        },
        _ => Err(format!("unsupported fts job_type: {job_type}")),
    };

    match result_payload {
        Ok(payload) => {
            conn.execute(
                "UPDATE __kdb_jobs
                 SET status='completed', payload_json=?, worker_id=NULL,
                     finished_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                     updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE job_id=? AND job_type=?",
                libsql::params![payload.to_string(), job_id, job_type],
            )
            .await
            .map_err(|e| AppError::Internal(format!("fts worker complete update failed: {e}")))?;
        }
        Err(message) => {
            conn.execute(
                "UPDATE __kdb_jobs
                 SET status='failed', last_error_code='fts_job_failed', last_error_message=?, worker_id=NULL,
                     finished_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                     updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE job_id=? AND job_type=?",
                libsql::params![message, job_id, job_type],
            )
            .await
            .map_err(|e| AppError::Internal(format!("fts worker failed update failed: {e}")))?;
        }
    }

    Ok(true)
}

async fn cleanup_export_jobs_for_db(
    conn: &libsql::Connection,
    retention_days: u64,
) -> AppResult<()> {
    if retention_days == 0 {
        return Ok(());
    }
    let cutoff = format!("-{} days", retention_days);
    conn.execute(
        "DELETE FROM __kdb_jobs
         WHERE job_type = 'export_jsonl'
           AND status IN ('completed','aborted','failed')
           AND COALESCE(finished_at, updated_at) < datetime('now', ?)",
        libsql::params![cutoff],
    )
    .await
    .map_err(|e| AppError::Internal(format!("export jobs cleanup failed: {e}")))?;
    Ok(())
}

async fn process_next_export_job_for_db(
    state: &AppState,
    conn: &libsql::Connection,
) -> AppResult<bool> {
    let worker_id = format!("worker-{}", Uuid::new_v4().simple());
    let mut rows = conn
        .query(
            "SELECT job_id, payload_json, target_path, compress, exported_count, bytes_written, part_count
             FROM __kdb_jobs
             WHERE status IN ('queued','retrying')
               AND job_type = 'export_jsonl'
             ORDER BY created_at ASC
             LIMIT 1",
            (),
        )
        .await
        .map_err(|e| AppError::Internal(format!("export worker query failed: {e}")))?;
    let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("export worker row failed: {e}")))?
    else {
        return Ok(false);
    };

    let job_id: String = row
        .get(0)
        .map_err(|e| AppError::Internal(format!("export worker decode failed: {e}")))?;
    let payload_json: String = row
        .get(1)
        .map_err(|e| AppError::Internal(format!("export worker decode failed: {e}")))?;
    let target_path: String = row
        .get(2)
        .map_err(|e| AppError::Internal(format!("export worker decode failed: {e}")))?;
    let compress: i64 = row
        .get(3)
        .map_err(|e| AppError::Internal(format!("export worker decode failed: {e}")))?;
    let mut exported_count: i64 = row
        .get(4)
        .map_err(|e| AppError::Internal(format!("export worker decode failed: {e}")))?;
    let mut bytes_written: i64 = row
        .get(5)
        .map_err(|e| AppError::Internal(format!("export worker decode failed: {e}")))?;
    let mut part_count: i64 = row
        .get(6)
        .map_err(|e| AppError::Internal(format!("export worker decode failed: {e}")))?;
    let payload: OperationPayload = serde_json::from_str(&payload_json)
        .map_err(|e| AppError::Internal(format!("export worker payload decode failed: {e}")))?;

    let claimed = conn
        .execute(
            "UPDATE __kdb_jobs
             SET status='running',
                 worker_id=?,
                 lease_expires_at=datetime('now', '+30 seconds'),
                 started_at=COALESCE(started_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                 updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE job_id=? AND job_type = 'export_jsonl' AND status IN ('queued','retrying')",
            libsql::params![worker_id.clone(), job_id.clone()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("export worker claim failed: {e}")))?;
    if claimed == 0 {
        return Ok(false);
    }

    let include_system_timestamps = payload.include_system_timestamps.unwrap_or(true);
    let source = resolve_source(&payload);
    let collection = resolve_collection_scope_optional_collection(&payload)?;
    let filter = payload.filter.clone().unwrap_or_else(|| json!({}));
    let (where_clause, base_binds) = build_where_with_collection(filter, collection)?;
    let order_by = build_order_by(&payload.sort)?;
    let batch_size = state.export_batch_size as i64;
    let start_offset = payload.offset.unwrap_or(0).max(0);
    let requested_limit = payload.limit.map(|v| v.max(0));

    let work_result: AppResult<()> = async {
        loop {
            let mut step_limit = batch_size;
            if let Some(limit) = requested_limit {
                let remaining = limit - exported_count;
                if remaining <= 0 {
                    break;
                }
                step_limit = step_limit.min(remaining);
            }

            let mut binds = base_binds.clone();
            binds.push(libsql::Value::Integer(step_limit));
            binds.push(libsql::Value::Integer(start_offset + exported_count));
            let sql = if include_system_timestamps {
                format!(
                    "SELECT json(data), _created_at, _modified_at
                     FROM {source}
                     WHERE {where_clause}
                     ORDER BY {order_by}
                     LIMIT ? OFFSET ?"
                )
            } else {
                format!(
                    "SELECT json(data)
                     FROM {source}
                     WHERE {where_clause}
                     ORDER BY {order_by}
                     LIMIT ? OFFSET ?"
                )
            };

            let mut rows = conn
                .query(&sql, binds)
                .await
                .map_err(|e| AppError::Internal(format!("export batch query failed: {e}")))?;
            let mut chunk = Vec::<u8>::new();
            let mut batch_count = 0i64;
            while let Some(row) = rows
                .next()
                .await
                .map_err(|e| AppError::Internal(format!("export batch read failed: {e}")))?
            {
                let raw: String = row
                    .get(0)
                    .map_err(|e| AppError::Internal(format!("export batch decode failed: {e}")))?;
                let mut doc = serde_json::from_str::<Value>(&raw).map_err(|e| {
                    AppError::Internal(format!("export batch json decode failed: {e}"))
                })?;
                if include_system_timestamps {
                    let created_at: Option<String> = row.get(1).map_err(|e| {
                        AppError::Internal(format!("export created_at decode failed: {e}"))
                    })?;
                    let modified_at: Option<String> = row.get(2).map_err(|e| {
                        AppError::Internal(format!("export modified_at decode failed: {e}"))
                    })?;
                    attach_system_timestamps(&mut doc, created_at, modified_at);
                }
                let projected = apply_projection(&doc, &payload.fields, &payload.exclude_fields)?;
                let line = projected.to_string();
                chunk.extend_from_slice(line.as_bytes());
                chunk.push(b'\n');
                batch_count += 1;
            }

            if batch_count == 0 {
                break;
            }

            let part_bytes = if compress == 1 {
                zstd::stream::encode_all(Cursor::new(chunk), 1)
                    .map_err(|e| AppError::Internal(format!("export zstd encode failed: {e}")))?
            } else {
                chunk
            };

            part_count += 1;
            write_export_part(state, &target_path, part_count, compress == 1, &part_bytes).await?;
            exported_count += batch_count;
            bytes_written += i64::try_from(part_bytes.len()).unwrap_or(i64::MAX);

            conn.execute(
                "UPDATE __kdb_jobs
                 SET exported_count=?, bytes_written=?, part_count=?,
                     lease_expires_at=datetime('now', '+30 seconds'),
                     updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE job_id=? AND job_type = 'export_jsonl'",
                libsql::params![exported_count, bytes_written, part_count, job_id.clone()],
            )
            .await
            .map_err(|e| AppError::Internal(format!("export progress update failed: {e}")))?;

            if batch_count < step_limit {
                break;
            }
        }
        finalize_export_target(state, &target_path, part_count, compress == 1).await?;
        Ok(())
    }
    .await;

    match work_result {
        Ok(_) => {
            conn.execute(
                "UPDATE __kdb_jobs
                 SET status='completed',
                     exported_count=?, bytes_written=?, part_count=?,
                     worker_id=NULL, lease_expires_at=NULL,
                     finished_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                     updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE job_id=? AND job_type = 'export_jsonl'",
                libsql::params![exported_count, bytes_written, part_count, job_id.clone()],
            )
            .await
            .map_err(|e| AppError::Internal(format!("export completion update failed: {e}")))?;
        }
        Err(err) => {
            conn.execute(
                "UPDATE __kdb_jobs
                 SET status='failed',
                     last_error_code='export_failed',
                     last_error_message=?,
                     worker_id=NULL, lease_expires_at=NULL,
                     updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE job_id=? AND job_type = 'export_jsonl'",
                libsql::params![err.to_string(), job_id.clone()],
            )
            .await
            .map_err(|e| AppError::Internal(format!("export failure update failed: {e}")))?;
        }
    }
    Ok(true)
}

async fn write_export_part(
    state: &AppState,
    target_path: &str,
    part_no: i64,
    compressed: bool,
    bytes: &[u8],
) -> AppResult<()> {
    let part_path = export_part_path(target_path, part_no, compressed);
    if target_path.starts_with("s3://") {
        state.db_manager.write_s3_uri(&part_path, bytes).await
    } else {
        let path = PathBuf::from(&part_path);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                AppError::Internal(format!("failed to create export part dir: {e}"))
            })?;
        }
        tokio::fs::write(path, bytes)
            .await
            .map_err(|e| AppError::Internal(format!("failed to write export part: {e}")))?;
        Ok(())
    }
}

async fn finalize_export_target(
    state: &AppState,
    target_path: &str,
    part_count: i64,
    compressed: bool,
) -> AppResult<()> {
    if part_count <= 0 {
        if target_path.starts_with("s3://") {
            state.db_manager.write_s3_uri(target_path, &[]).await?;
        } else {
            let path = PathBuf::from(target_path);
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| AppError::Internal(format!("failed to create export dir: {e}")))?;
            }
            tokio::fs::write(path, &[])
                .await
                .map_err(|e| AppError::Internal(format!("failed to create export file: {e}")))?;
        }
        return Ok(());
    }

    if target_path.starts_with("s3://") {
        let mut out = Vec::<u8>::new();
        for i in 1..=part_count {
            let part_path = export_part_path(target_path, i, compressed);
            let bytes = state
                .db_manager
                .read_s3_uri_from_offset(&part_path, 0)
                .await?;
            out.extend_from_slice(&bytes);
        }
        state.db_manager.write_s3_uri(target_path, &out).await?;
        cleanup_export_parts(state, target_path, part_count, compressed).await;
        return Ok(());
    }

    let path = PathBuf::from(target_path);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Internal(format!("failed to create export dir: {e}")))?;
    }
    let mut out = tokio::fs::File::create(&path)
        .await
        .map_err(|e| AppError::Internal(format!("failed to create export file: {e}")))?;
    for i in 1..=part_count {
        let part_path = PathBuf::from(export_part_path(target_path, i, compressed));
        let bytes = tokio::fs::read(&part_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed to read export part: {e}")))?;
        out.write_all(&bytes)
            .await
            .map_err(|e| AppError::Internal(format!("failed to write export file: {e}")))?;
    }
    out.flush()
        .await
        .map_err(|e| AppError::Internal(format!("failed to flush export file: {e}")))?;
    cleanup_export_parts(state, target_path, part_count, compressed).await;
    Ok(())
}

async fn cleanup_export_parts(
    state: &AppState,
    target_path: &str,
    part_count: i64,
    compressed: bool,
) {
    for i in 1..=part_count {
        let part_path = export_part_path(target_path, i, compressed);
        if target_path.starts_with("s3://") {
            let _ = state.db_manager.delete_s3_uri(&part_path).await;
        } else {
            let _ = tokio::fs::remove_file(PathBuf::from(&part_path)).await;
        }
    }
    if !target_path.starts_with("s3://") {
        let parts_dir = PathBuf::from(format!("{target_path}.parts"));
        let _ = tokio::fs::remove_dir(parts_dir).await;
    }
}

fn export_part_path(target_path: &str, part_no: i64, compressed: bool) -> String {
    let ext = if compressed {
        ".zst.part"
    } else {
        ".jsonl.part"
    };
    format!("{target_path}.parts/{part_no:08}{ext}")
}

pub async fn process_import_jobs_tick(state: &AppState) -> AppResult<usize> {
    process_loaded_db_jobs_bounded(state, |state, db_path| async move {
        let conn = match state.db_manager.get_conn_with_create(&db_path, false).await {
            Ok(v) => v,
            Err(_) => return Ok(0),
        };
        cleanup_import_jobs_for_db(&conn, state.import_job_retention_days).await?;
        Ok(process_next_import_job_for_db(&state, &db_path, &conn).await? as usize)
    })
    .await
}

async fn process_loaded_db_jobs_bounded<F, Fut>(state: &AppState, f: F) -> AppResult<usize>
where
    F: Fn(AppState, String) -> Fut + Copy + Send + Sync + 'static,
    Fut: std::future::Future<Output = AppResult<usize>> + Send + 'static,
{
    let loaded = state.db_manager.loaded_db_paths();
    let concurrency = state.job_worker_concurrency.max(1);
    let mut processed = 0usize;
    for chunk in loaded.chunks(concurrency) {
        let mut jobs = JoinSet::new();
        for db_path in chunk {
            jobs.spawn(f(state.clone(), db_path.clone()));
        }
        while let Some(result) = jobs.join_next().await {
            match result {
                Ok(Ok(count)) => processed += count,
                Ok(Err(err)) => return Err(err),
                Err(err) => {
                    return Err(AppError::Internal(format!(
                        "job worker task join failed: {err}"
                    )));
                }
            }
        }
    }
    Ok(processed)
}

async fn cleanup_import_jobs_for_db(
    conn: &libsql::Connection,
    retention_days: u64,
) -> AppResult<()> {
    if retention_days == 0 {
        return Ok(());
    }
    let cutoff = format!("-{} days", retention_days);
    conn.execute(
        "DELETE FROM __kdb_jobs
         WHERE job_type = 'import_jsonl'
           AND status IN ('completed','aborted','failed')
           AND COALESCE(finished_at, updated_at) < datetime('now', ?)",
        libsql::params![cutoff],
    )
    .await
    .map_err(|e| AppError::Internal(format!("import jobs cleanup failed: {e}")))?;
    Ok(())
}

async fn process_next_import_job_for_db(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
) -> AppResult<bool> {
    let worker_id = format!("worker-{}", Uuid::new_v4().simple());
    let mut rows = conn
        .query(
            "SELECT job_id, collection, source_path, on_conflict, batch_size, resumable,
                    ignore_input_id, allow_system_timestamps,
                    last_line_no, last_byte_offset, read_count, inserted_count, updated_count, skipped_count, error_count, source_hash
                    , alias_import_pk, drop_keys_json
             FROM __kdb_jobs
             WHERE status IN ('queued','retrying')
               AND job_type = 'import_jsonl'
             ORDER BY created_at ASC
             LIMIT 1",
            (),
        )
        .await
        .map_err(|e| AppError::Internal(format!("import worker query failed: {e}")))?;
    let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("import worker row failed: {e}")))?
    else {
        return Ok(false);
    };

    let job_id: String = row
        .get(0)
        .map_err(|e| AppError::Internal(format!("import worker decode failed: {e}")))?;
    let collection: String = row
        .get(1)
        .map_err(|e| AppError::Internal(format!("import worker decode failed: {e}")))?;
    let source_path: String = row
        .get(2)
        .map_err(|e| AppError::Internal(format!("import worker decode failed: {e}")))?;
    let on_conflict: String = row
        .get(3)
        .map_err(|e| AppError::Internal(format!("import worker decode failed: {e}")))?;
    let batch_size: i64 = row
        .get(4)
        .map_err(|e| AppError::Internal(format!("import worker decode failed: {e}")))?;
    let _resumable: i64 = row
        .get(5)
        .map_err(|e| AppError::Internal(format!("import worker decode failed: {e}")))?;
    let ignore_input_id: i64 = row
        .get(6)
        .map_err(|e| AppError::Internal(format!("import worker decode failed: {e}")))?;
    let allow_system_timestamps: i64 = row
        .get(7)
        .map_err(|e| AppError::Internal(format!("import worker decode failed: {e}")))?;
    let mut last_line_no: i64 = row
        .get(8)
        .map_err(|e| AppError::Internal(format!("import worker decode failed: {e}")))?;
    let mut last_byte_offset: i64 = row
        .get(9)
        .map_err(|e| AppError::Internal(format!("import worker decode failed: {e}")))?;
    let mut read_count: i64 = row
        .get(10)
        .map_err(|e| AppError::Internal(format!("import worker decode failed: {e}")))?;
    let mut inserted_count: i64 = row
        .get(11)
        .map_err(|e| AppError::Internal(format!("import worker decode failed: {e}")))?;
    let mut updated_count: i64 = row
        .get(12)
        .map_err(|e| AppError::Internal(format!("import worker decode failed: {e}")))?;
    let mut skipped_count: i64 = row
        .get(13)
        .map_err(|e| AppError::Internal(format!("import worker decode failed: {e}")))?;
    let mut error_count: i64 = row
        .get(14)
        .map_err(|e| AppError::Internal(format!("import worker decode failed: {e}")))?;
    let source_hash: Option<String> = row
        .get(15)
        .map_err(|e| AppError::Internal(format!("import worker decode failed: {e}")))?;
    let alias_import_pk_json: Option<String> = row
        .get(16)
        .map_err(|e| AppError::Internal(format!("import worker decode failed: {e}")))?;
    let drop_keys_json: Option<String> = row
        .get(17)
        .map_err(|e| AppError::Internal(format!("import worker decode failed: {e}")))?;
    let alias_import_pk = parse_alias_import_pk_json(alias_import_pk_json.as_deref())?;
    let drop_keys = parse_drop_keys_json(drop_keys_json.as_deref())?;

    let claimed = conn
        .execute(
            "UPDATE __kdb_jobs
         SET status='running',
             worker_id=?,
             lease_expires_at=datetime('now', '+30 seconds'),
             started_at=COALESCE(started_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE job_id=? AND job_type = 'import_jsonl' AND status IN ('queued','retrying')",
            libsql::params![worker_id.clone(), job_id.clone()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("import worker claim failed: {e}")))?;
    if claimed == 0 {
        return Ok(false);
    }

    let batch_cap = (batch_size.max(1) as usize).min(10_000);
    let mut batch_docs = Vec::<Value>::with_capacity(batch_cap);
    let allow_system_timestamps = allow_system_timestamps == 1;
    let source_is_zst = source_path.to_ascii_lowercase().ends_with(".zst");

    let import_result: AppResult<()> = async {
        if source_path.starts_with("s3://") {
            if let Some(expected) = source_hash
                .as_ref()
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
            {
                let meta_hash = state
                    .db_manager
                    .get_s3_uri_source_hash(&source_path)
                    .await?;
                if let Some(actual) = meta_hash
                    .as_ref()
                    .map(|v| v.trim())
                    .filter(|v| !v.is_empty())
                {
                    if actual != expected {
                        return Err(AppError::Conflict(format!(
                            "source_hash mismatch: expected {expected}, found metadata {actual}"
                        )));
                    }
                }
            }
            if source_is_zst {
                let temp_path = download_and_decompress_s3_jsonl_to_temp(
                    state,
                    &source_path,
                    &job_id,
                    4 * 1024 * 1024,
                )
                .await?;
                process_local_jsonl_file(
                    conn,
                    &collection,
                    &on_conflict,
                    &temp_path,
                    batch_cap,
                    &alias_import_pk,
                    &drop_keys,
                    ignore_input_id == 1,
                    allow_system_timestamps,
                    &job_id,
                    &mut batch_docs,
                    &mut read_count,
                    &mut inserted_count,
                    &mut updated_count,
                    &mut skipped_count,
                    &mut error_count,
                    &mut last_line_no,
                    &mut last_byte_offset,
                    true,
                )
                .await?;
                let _ = tokio::fs::remove_file(temp_path).await;
            } else {
                process_s3_jsonl_stream(
                    state,
                    conn,
                    &collection,
                    &on_conflict,
                    &source_path,
                    batch_cap,
                    &alias_import_pk,
                    &drop_keys,
                    ignore_input_id == 1,
                    allow_system_timestamps,
                    &job_id,
                    &mut batch_docs,
                    &mut read_count,
                    &mut inserted_count,
                    &mut updated_count,
                    &mut skipped_count,
                    &mut error_count,
                    &mut last_line_no,
                    &mut last_byte_offset,
                    4 * 1024 * 1024,
                )
                .await?;
            }
        } else {
            let source_for_parse = if source_is_zst {
                let temp_path = decompress_local_zst_to_temp_jsonl(&source_path, &job_id).await?;
                let out = temp_path.to_string_lossy().to_string();
                last_byte_offset = 0;
                out
            } else {
                source_path.clone()
            };
            process_local_jsonl_file(
                conn,
                &collection,
                &on_conflict,
                &source_for_parse,
                batch_cap,
                &alias_import_pk,
                &drop_keys,
                ignore_input_id == 1,
                allow_system_timestamps,
                &job_id,
                &mut batch_docs,
                &mut read_count,
                &mut inserted_count,
                &mut updated_count,
                &mut skipped_count,
                &mut error_count,
                &mut last_line_no,
                &mut last_byte_offset,
                source_is_zst,
            )
            .await?;
            if source_is_zst {
                let _ = tokio::fs::remove_file(source_for_parse).await;
            }
        }

        if !batch_docs.is_empty() {
            let (ins, upd, skip) = apply_import_batch(
                conn,
                &collection,
                &batch_docs,
                &on_conflict,
                false,
                allow_system_timestamps,
            )
            .await?;
            read_count += batch_docs.len() as i64;
            inserted_count += ins;
            updated_count += upd;
            skipped_count += skip;
            batch_docs.clear();
        }
        Ok(())
    }
    .await;

    match import_result {
        Ok(_) => {
            conn.execute(
                "UPDATE __kdb_jobs
                 SET status='completed',
                     read_count=?, inserted_count=?, updated_count=?, skipped_count=?, error_count=?,
                     last_line_no=?, last_byte_offset=?,
                     worker_id=NULL, lease_expires_at=NULL,
                     finished_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                     updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE job_id=? AND job_type = 'import_jsonl'",
                libsql::params![
                    read_count,
                    inserted_count,
                    updated_count,
                    skipped_count,
                    error_count,
                    last_line_no,
                    last_byte_offset,
                    job_id.clone()
                ],
            )
            .await
            .map_err(|e| AppError::Internal(format!("import completion update failed: {e}")))?;
            invalidate_read_cache_scope(state, db_path, Some(&collection));
            state
                .db_manager
                .append_wal_record(
                    db_path,
                    "IMPORT_JSONL",
                    &json!({
                        "job_id": job_id,
                        "collection": collection,
                        "source_path": source_path,
                        "read_count": read_count,
                        "inserted_count": inserted_count,
                        "updated_count": updated_count,
                        "skipped_count": skipped_count,
                        "on_conflict": on_conflict
                    })
                    .to_string(),
                )
                .await?;
        }
        Err(err) => {
            error_count += 1;
            conn.execute(
                "UPDATE __kdb_jobs
                 SET status='failed',
                     error_count=?,
                     read_count=?, inserted_count=?, updated_count=?, skipped_count=?,
                     last_line_no=?, last_byte_offset=?,
                     last_error_code='import_failed',
                     last_error_message=?,
                     worker_id=NULL, lease_expires_at=NULL,
                     updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE job_id=? AND job_type = 'import_jsonl'",
                libsql::params![
                    error_count,
                    read_count,
                    inserted_count,
                    updated_count,
                    skipped_count,
                    last_line_no,
                    last_byte_offset,
                    err.to_string(),
                    job_id.clone()
                ],
            )
            .await
            .map_err(|e| AppError::Internal(format!("import failure update failed: {e}")))?;
        }
    }

    Ok(true)
}

async fn process_local_jsonl_file(
    conn: &libsql::Connection,
    collection: &str,
    on_conflict: &str,
    source_path: &str,
    batch_cap: usize,
    alias_import_pk: &[(String, String)],
    drop_keys: &[String],
    ignore_input_id: bool,
    allow_system_timestamps: bool,
    job_id: &str,
    batch_docs: &mut Vec<Value>,
    read_count: &mut i64,
    inserted_count: &mut i64,
    updated_count: &mut i64,
    skipped_count: &mut i64,
    error_count: &mut i64,
    last_line_no: &mut i64,
    last_byte_offset: &mut i64,
    line_resume_mode: bool,
) -> AppResult<()> {
    let resume_line_checkpoint = *last_line_no;
    let source = tokio::fs::File::open(source_path)
        .await
        .map_err(|e| AppError::BadRequest(format!("source_path open failed: {e}")))?;
    let mut reader = BufReader::new(source);
    if !line_resume_mode && *last_byte_offset > 0 {
        reader
            .seek(std::io::SeekFrom::Start(*last_byte_offset as u64))
            .await
            .map_err(|e| AppError::Internal(format!("import seek failed: {e}")))?;
    }
    let mut buf = Vec::<u8>::new();
    loop {
        buf.clear();
        let n = reader
            .read_until(b'\n', &mut buf)
            .await
            .map_err(|e| AppError::Internal(format!("source read failed: {e}")))?;
        if n == 0 {
            break;
        }
        *last_byte_offset += n as i64;
        *last_line_no += 1;
        if line_resume_mode && *last_line_no <= resume_line_checkpoint {
            continue;
        }
        let line_bytes = if buf.last() == Some(&b'\n') {
            &buf[..buf.len().saturating_sub(1)]
        } else {
            &buf[..]
        };
        let line = decode_jsonl_line(line_bytes, *last_line_no)?;
        enqueue_import_line(
            line,
            batch_docs,
            alias_import_pk,
            drop_keys,
            ignore_input_id,
        )?;
        if batch_docs.len() >= batch_cap {
            let (ins, upd, skip) = apply_import_batch(
                conn,
                collection,
                batch_docs,
                on_conflict,
                false,
                allow_system_timestamps,
            )
            .await?;
            *read_count += batch_docs.len() as i64;
            *inserted_count += ins;
            *updated_count += upd;
            *skipped_count += skip;
            batch_docs.clear();
            persist_import_progress(
                conn,
                job_id,
                *read_count,
                *inserted_count,
                *updated_count,
                *skipped_count,
                *error_count,
                *last_line_no,
                *last_byte_offset,
            )
            .await?;
        }
    }
    Ok(())
}

async fn process_s3_jsonl_stream(
    state: &AppState,
    conn: &libsql::Connection,
    collection: &str,
    on_conflict: &str,
    source_path: &str,
    batch_cap: usize,
    alias_import_pk: &[(String, String)],
    drop_keys: &[String],
    ignore_input_id: bool,
    allow_system_timestamps: bool,
    job_id: &str,
    batch_docs: &mut Vec<Value>,
    read_count: &mut i64,
    inserted_count: &mut i64,
    updated_count: &mut i64,
    skipped_count: &mut i64,
    error_count: &mut i64,
    last_line_no: &mut i64,
    last_byte_offset: &mut i64,
    chunk_bytes: usize,
) -> AppResult<()> {
    let mut fetch_offset = usize::try_from((*last_byte_offset).max(0)).unwrap_or(0);
    let mut carry = Vec::<u8>::new();
    loop {
        let chunk = state
            .db_manager
            .read_s3_uri_range(source_path, fetch_offset, chunk_bytes)
            .await?;
        if chunk.is_empty() {
            break;
        }
        let base = fetch_offset.saturating_sub(carry.len());
        fetch_offset += chunk.len();
        let mut data = Vec::with_capacity(carry.len() + chunk.len());
        data.extend_from_slice(&carry);
        data.extend_from_slice(&chunk);
        let mut consumed = 0usize;
        while let Some(rel) = data[consumed..].iter().position(|b| *b == b'\n') {
            let end = consumed + rel;
            let line_bytes = &data[consumed..end];
            consumed = end + 1;
            *last_byte_offset = i64::try_from(base + consumed).unwrap_or(i64::MAX);
            *last_line_no += 1;
            let line = decode_jsonl_line(line_bytes, *last_line_no)?;
            enqueue_import_line(
                line,
                batch_docs,
                alias_import_pk,
                drop_keys,
                ignore_input_id,
            )?;
            if batch_docs.len() >= batch_cap {
                let (ins, upd, skip) = apply_import_batch(
                    conn,
                    collection,
                    batch_docs,
                    on_conflict,
                    false,
                    allow_system_timestamps,
                )
                .await?;
                *read_count += batch_docs.len() as i64;
                *inserted_count += ins;
                *updated_count += upd;
                *skipped_count += skip;
                batch_docs.clear();
                persist_import_progress(
                    conn,
                    job_id,
                    *read_count,
                    *inserted_count,
                    *updated_count,
                    *skipped_count,
                    *error_count,
                    *last_line_no,
                    *last_byte_offset,
                )
                .await?;
            }
        }
        carry = data[consumed..].to_vec();
    }
    if !carry.is_empty() {
        *last_line_no += 1;
        let line = decode_jsonl_line(&carry, *last_line_no)?;
        enqueue_import_line(
            line,
            batch_docs,
            alias_import_pk,
            drop_keys,
            ignore_input_id,
        )?;
        *last_byte_offset += i64::try_from(carry.len()).unwrap_or(0);
    }
    Ok(())
}

async fn decompress_local_zst_to_temp_jsonl(source_path: &str, job_id: &str) -> AppResult<PathBuf> {
    let temp_dir = std::env::temp_dir().join("kongodb_import");
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|e| AppError::Internal(format!("failed to create temp dir: {e}")))?;
    let out_path = temp_dir.join(format!("{job_id}.jsonl"));
    let src = source_path.to_string();
    let dst = out_path.clone();
    tokio::task::spawn_blocking(move || -> AppResult<()> {
        let input = std::fs::File::open(&src)
            .map_err(|e| AppError::BadRequest(format!("source_path open failed: {e}")))?;
        let mut decoder = zstd::stream::read::Decoder::new(input)
            .map_err(|e| AppError::BadRequest(format!("zstd decode failed: {e}")))?;
        let mut output = std::fs::File::create(&dst)
            .map_err(|e| AppError::Internal(format!("failed to create temp jsonl: {e}")))?;
        std::io::copy(&mut decoder, &mut output)
            .map_err(|e| AppError::BadRequest(format!("zstd decode failed: {e}")))?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("zstd decode task failed: {e}")))??;
    Ok(out_path)
}

async fn download_and_decompress_s3_jsonl_to_temp(
    state: &AppState,
    source_path: &str,
    job_id: &str,
    chunk_bytes: usize,
) -> AppResult<String> {
    let temp_dir = std::env::temp_dir().join("kongodb_import");
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|e| AppError::Internal(format!("failed to create temp dir: {e}")))?;
    let zst_path = temp_dir.join(format!("{job_id}.jsonl.zst"));
    let mut out = tokio::fs::File::create(&zst_path)
        .await
        .map_err(|e| AppError::Internal(format!("failed to create temp zst: {e}")))?;
    let mut offset = 0usize;
    loop {
        let chunk = state
            .db_manager
            .read_s3_uri_range(source_path, offset, chunk_bytes)
            .await?;
        if chunk.is_empty() {
            break;
        }
        out.write_all(&chunk)
            .await
            .map_err(|e| AppError::Internal(format!("failed writing temp zst: {e}")))?;
        offset += chunk.len();
    }
    out.flush()
        .await
        .map_err(|e| AppError::Internal(format!("failed flushing temp zst: {e}")))?;
    let jsonl_path = decompress_local_zst_to_temp_jsonl(
        zst_path
            .to_str()
            .ok_or_else(|| AppError::Internal("invalid temp zst path".to_string()))?,
        &format!("{job_id}_s3"),
    )
    .await?;
    let _ = tokio::fs::remove_file(zst_path).await;
    Ok(jsonl_path.to_string_lossy().to_string())
}

fn decode_jsonl_line<'a>(line_bytes: &'a [u8], line_no: i64) -> AppResult<&'a str> {
    let line = std::str::from_utf8(line_bytes)
        .map_err(|e| AppError::BadRequest(format!("invalid UTF-8 at line {line_no}: {e}")))?;
    Ok(line.trim_end_matches('\r'))
}

fn enqueue_import_line(
    line: &str,
    batch_docs: &mut Vec<Value>,
    alias_import_pk: &[(String, String)],
    drop_keys: &[String],
    ignore_input_id: bool,
) -> AppResult<()> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let mut doc = serde_json::from_str::<Value>(trimmed)
        .map_err(|e| AppError::BadRequest(format!("invalid JSONL row: {e}")))?;
    if !doc.is_object() {
        return Err(AppError::BadRequest(
            "each JSONL row must be a JSON object".to_string(),
        ));
    }
    expand_kdb_macros_in_value(&mut doc)?;
    apply_import_aliases_and_drop_keys(&mut doc, alias_import_pk, drop_keys)?;
    if ignore_input_id {
        scrub_import_identity_keys(&mut doc)?;
    }
    ensure_or_get_id(&mut doc)?;
    batch_docs.push(doc);
    Ok(())
}

fn scrub_import_identity_keys(doc: &mut Value) -> AppResult<()> {
    let obj = doc
        .as_object_mut()
        .ok_or_else(|| AppError::BadRequest("import row must be an object".to_string()))?;
    obj.remove("_id");
    obj.remove("id");
    obj.remove("_key");
    Ok(())
}

fn apply_import_aliases_and_drop_keys(
    doc: &mut Value,
    alias_import_pk: &[(String, String)],
    drop_keys: &[String],
) -> AppResult<()> {
    let obj = doc
        .as_object_mut()
        .ok_or_else(|| AppError::BadRequest("import row must be an object".to_string()))?;

    for (from, to) in alias_import_pk {
        if from.trim().is_empty() || to.trim().is_empty() || from == to {
            continue;
        }
        let to_exists = obj.contains_key(to);
        if let Some(v) = obj.remove(from) {
            if !to_exists {
                obj.insert(to.clone(), v);
            }
        }
    }

    for path in drop_keys {
        if path == "_id" {
            return Err(AppError::BadRequest(
                "drop_keys cannot include _id".to_string(),
            ));
        }
        drop_path(doc, path);
    }
    Ok(())
}

fn drop_path(root: &mut Value, path: &str) {
    let mut parts = path.split('.').filter(|p| !p.is_empty()).peekable();
    if parts.peek().is_none() {
        return;
    }
    let mut cur = root;
    while let Some(part) = parts.next() {
        let is_last = parts.peek().is_none();
        if is_last {
            if let Some(obj) = cur.as_object_mut() {
                obj.remove(part);
            }
            return;
        }
        let Some(next) = cur.as_object_mut().and_then(|o| o.get_mut(part)) else {
            return;
        };
        cur = next;
    }
}

fn parse_alias_import_pk_override(raw: Option<&Value>) -> AppResult<Vec<(String, String)>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    match raw {
        Value::String(s) => parse_alias_pairs_csv(s),
        Value::Object(map) => {
            let mut out = Vec::<(String, String)>::new();
            for (from, to_val) in map {
                let to = to_val.as_str().ok_or_else(|| {
                    AppError::BadRequest(
                        "alias_import_pk object values must be strings".to_string(),
                    )
                })?;
                if from.trim().is_empty() || to.trim().is_empty() {
                    continue;
                }
                out.push((from.trim().to_string(), to.trim().to_string()));
            }
            Ok(out)
        }
        Value::Array(items) => {
            let mut out = Vec::<(String, String)>::new();
            for item in items {
                match item {
                    Value::Object(obj) => {
                        let from = obj.get("from").and_then(Value::as_str).ok_or_else(|| {
                            AppError::BadRequest(
                                "alias_import_pk array object needs from/to".to_string(),
                            )
                        })?;
                        let to = obj.get("to").and_then(Value::as_str).ok_or_else(|| {
                            AppError::BadRequest(
                                "alias_import_pk array object needs from/to".to_string(),
                            )
                        })?;
                        if !from.trim().is_empty() && !to.trim().is_empty() {
                            out.push((from.trim().to_string(), to.trim().to_string()));
                        }
                    }
                    Value::Array(pair) if pair.len() == 2 => {
                        let from = pair[0].as_str().ok_or_else(|| {
                            AppError::BadRequest(
                                "alias_import_pk pair[0] must be string".to_string(),
                            )
                        })?;
                        let to = pair[1].as_str().ok_or_else(|| {
                            AppError::BadRequest(
                                "alias_import_pk pair[1] must be string".to_string(),
                            )
                        })?;
                        if !from.trim().is_empty() && !to.trim().is_empty() {
                            out.push((from.trim().to_string(), to.trim().to_string()));
                        }
                    }
                    _ => {
                        return Err(AppError::BadRequest(
                            "alias_import_pk array entries must be {from,to} or [from,to]"
                                .to_string(),
                        ));
                    }
                }
            }
            Ok(out)
        }
        _ => Err(AppError::BadRequest(
            "alias_import_pk must be string, object, or array".to_string(),
        )),
    }
}

fn parse_alias_pairs_csv(raw: &str) -> AppResult<Vec<(String, String)>> {
    let mut out = Vec::<(String, String)>::new();
    for pair in raw.split(',') {
        let p = pair.trim();
        if p.is_empty() {
            continue;
        }
        let mut parts = p.splitn(2, ':');
        let from = parts.next().unwrap_or_default().trim();
        let to = parts.next().unwrap_or_default().trim();
        if from.is_empty() || to.is_empty() {
            return Err(AppError::BadRequest(
                "alias_import_pk csv format must be from:to,from2:to2".to_string(),
            ));
        }
        out.push((from.to_string(), to.to_string()));
    }
    Ok(out)
}

fn parse_drop_keys_override(raw: Option<&Vec<String>>) -> AppResult<Vec<String>> {
    let Some(keys) = raw else {
        return Ok(Vec::new());
    };
    let mut out = Vec::<String>::new();
    for key in keys {
        let k = key.trim();
        if k.is_empty() {
            continue;
        }
        if k == "_id" {
            return Err(AppError::BadRequest(
                "drop_keys cannot include _id".to_string(),
            ));
        }
        out.push(k.to_string());
    }
    Ok(out)
}

fn parse_alias_import_pk_json(raw: Option<&str>) -> AppResult<Vec<(String, String)>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let val = serde_json::from_str::<Value>(trimmed)
        .map_err(|e| AppError::Internal(format!("invalid alias_import_pk in job: {e}")))?;
    parse_alias_import_pk_override(Some(&val))
}

fn parse_drop_keys_json(raw: Option<&str>) -> AppResult<Vec<String>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let keys = serde_json::from_str::<Vec<String>>(trimmed)
        .map_err(|e| AppError::Internal(format!("invalid drop_keys_json in job: {e}")))?;
    parse_drop_keys_override(Some(&keys))
}

async fn persist_import_progress(
    conn: &libsql::Connection,
    job_id: &str,
    read_count: i64,
    inserted_count: i64,
    updated_count: i64,
    skipped_count: i64,
    error_count: i64,
    last_line_no: i64,
    last_byte_offset: i64,
) -> AppResult<()> {
    conn.execute(
        "UPDATE __kdb_jobs
         SET read_count=?, inserted_count=?, updated_count=?, skipped_count=?, error_count=?,
             last_line_no=?, last_byte_offset=?,
             lease_expires_at=datetime('now', '+30 seconds'),
             updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE job_id=? AND job_type = 'import_jsonl'",
        libsql::params![
            read_count,
            inserted_count,
            updated_count,
            skipped_count,
            error_count,
            last_line_no,
            last_byte_offset,
            job_id.to_string()
        ],
    )
    .await
    .map_err(|e| AppError::Internal(format!("import progress update failed: {e}")))?;
    Ok(())
}

async fn fetch_export_job(conn: &libsql::Connection, job_id: &str) -> AppResult<Value> {
    let mut rows = conn
        .query(
            "SELECT job_id, collection, payload_json, target_path, compress, resumable, status,
                    last_error_code, last_error_message,
                    exported_count, bytes_written, part_count,
                    worker_id, lease_expires_at, started_at, finished_at, created_at, updated_at
             FROM __kdb_jobs WHERE job_id = ? AND job_type = 'export_jsonl' LIMIT 1",
            libsql::params![job_id.to_string()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("get_export_job failed: {e}")))?;
    let row = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("get_export_job row failed: {e}")))?
        .ok_or_else(|| AppError::NotFound(format!("export job not found: {job_id}")))?;

    let job_id: String = row
        .get(0)
        .map_err(|e| AppError::Internal(format!("get_export_job decode failed: {e}")))?;
    let collection: Option<String> = row
        .get(1)
        .map_err(|e| AppError::Internal(format!("get_export_job decode failed: {e}")))?;
    let payload_json: String = row
        .get(2)
        .map_err(|e| AppError::Internal(format!("get_export_job decode failed: {e}")))?;
    let target_path: String = row
        .get(3)
        .map_err(|e| AppError::Internal(format!("get_export_job decode failed: {e}")))?;
    let compress: i64 = row
        .get(4)
        .map_err(|e| AppError::Internal(format!("get_export_job decode failed: {e}")))?;
    let resumable: i64 = row
        .get(5)
        .map_err(|e| AppError::Internal(format!("get_export_job decode failed: {e}")))?;
    let status: String = row
        .get(6)
        .map_err(|e| AppError::Internal(format!("get_export_job decode failed: {e}")))?;
    let last_error_code: Option<String> = row
        .get(7)
        .map_err(|e| AppError::Internal(format!("get_export_job decode failed: {e}")))?;
    let last_error_message: Option<String> = row
        .get(8)
        .map_err(|e| AppError::Internal(format!("get_export_job decode failed: {e}")))?;
    let exported_count: i64 = row
        .get(9)
        .map_err(|e| AppError::Internal(format!("get_export_job decode failed: {e}")))?;
    let bytes_written: i64 = row
        .get(10)
        .map_err(|e| AppError::Internal(format!("get_export_job decode failed: {e}")))?;
    let part_count: i64 = row
        .get(11)
        .map_err(|e| AppError::Internal(format!("get_export_job decode failed: {e}")))?;
    let worker_id: Option<String> = row
        .get(12)
        .map_err(|e| AppError::Internal(format!("get_export_job decode failed: {e}")))?;
    let lease_expires_at: Option<String> = row
        .get(13)
        .map_err(|e| AppError::Internal(format!("get_export_job decode failed: {e}")))?;
    let started_at: Option<String> = row
        .get(14)
        .map_err(|e| AppError::Internal(format!("get_export_job decode failed: {e}")))?;
    let finished_at: Option<String> = row
        .get(15)
        .map_err(|e| AppError::Internal(format!("get_export_job decode failed: {e}")))?;
    let created_at: String = row
        .get(16)
        .map_err(|e| AppError::Internal(format!("get_export_job decode failed: {e}")))?;
    let updated_at: String = row
        .get(17)
        .map_err(|e| AppError::Internal(format!("get_export_job decode failed: {e}")))?;
    let payload: Value = serde_json::from_str(&payload_json)
        .map_err(|e| AppError::Internal(format!("get_export_job payload decode failed: {e}")))?;

    Ok(json!({
        "job_id": job_id,
        "collection": collection,
        "payload": payload,
        "target_path": target_path,
        "compress": compress == 1,
        "resumable": resumable == 1,
        "status": status,
        "last_error_code": last_error_code,
        "last_error_message": last_error_message,
        "exported_count": exported_count,
        "bytes_written": bytes_written,
        "part_count": part_count,
        "worker_id": worker_id,
        "lease_expires_at": lease_expires_at,
        "started_at": started_at,
        "finished_at": finished_at,
        "created_at": created_at,
        "updated_at": updated_at
    }))
}

async fn fetch_import_job(conn: &libsql::Connection, job_id: &str) -> AppResult<Value> {
    let mut rows = conn
        .query(
            "SELECT job_id, collection, source_path, source_hash, on_conflict, batch_size, resumable, status,
                    ignore_input_id, allow_system_timestamps,
                    last_error_code, last_error_message,
                    read_count, inserted_count, updated_count, skipped_count, error_count,
                    last_line_no, last_byte_offset, worker_id, lease_expires_at, alias_import_pk, drop_keys_json,
                    started_at, finished_at, created_at, updated_at
             FROM __kdb_jobs WHERE job_id = ? AND job_type = 'import_jsonl' LIMIT 1",
            libsql::params![job_id.to_string()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("get_import_job failed: {e}")))?;
    let row = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("get_import_job row failed: {e}")))?
        .ok_or_else(|| AppError::NotFound(format!("import job not found: {job_id}")))?;

    let job_id: String = row
        .get(0)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let collection: String = row
        .get(1)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let source_path: String = row
        .get(2)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let source_hash: Option<String> = row
        .get(3)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let on_conflict: String = row
        .get(4)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let batch_size: i64 = row
        .get(5)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let resumable: i64 = row
        .get(6)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let status: String = row
        .get(7)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let ignore_input_id: i64 = row
        .get(8)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let allow_system_timestamps: i64 = row
        .get(9)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let last_error_code: Option<String> = row
        .get(10)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let last_error_message: Option<String> = row
        .get(11)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let read_count: i64 = row
        .get(12)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let inserted_count: i64 = row
        .get(13)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let updated_count: i64 = row
        .get(14)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let skipped_count: i64 = row
        .get(15)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let error_count: i64 = row
        .get(16)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let last_line_no: i64 = row
        .get(17)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let last_byte_offset: i64 = row
        .get(18)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let worker_id: Option<String> = row
        .get(19)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let lease_expires_at: Option<String> = row
        .get(20)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let alias_import_pk_json: Option<String> = row
        .get(21)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let drop_keys_json: Option<String> = row
        .get(22)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let started_at: Option<String> = row
        .get(23)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let finished_at: Option<String> = row
        .get(24)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let created_at: String = row
        .get(25)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let updated_at: String = row
        .get(26)
        .map_err(|e| AppError::Internal(format!("get_import_job decode failed: {e}")))?;
    let alias_import_pk = parse_alias_import_pk_json(alias_import_pk_json.as_deref())?;
    let drop_keys = parse_drop_keys_json(drop_keys_json.as_deref())?;

    Ok(json!({
        "job_id": job_id,
        "collection": collection,
        "source_path": source_path,
        "source_hash": source_hash,
        "alias_import_pk": alias_import_pk,
        "drop_keys": drop_keys,
        "on_conflict": on_conflict,
        "ignore_input_id": ignore_input_id == 1,
        "allow_system_timestamps": allow_system_timestamps == 1,
        "batch_size": batch_size,
        "resumable": resumable == 1,
        "status": status,
        "last_error_code": last_error_code,
        "last_error_message": last_error_message,
        "read_count": read_count,
        "inserted_count": inserted_count,
        "updated_count": updated_count,
        "skipped_count": skipped_count,
        "error_count": error_count,
        "last_line_no": last_line_no,
        "last_byte_offset": last_byte_offset,
        "worker_id": worker_id,
        "lease_expires_at": lease_expires_at,
        "started_at": started_at,
        "finished_at": finished_at,
        "created_at": created_at,
        "updated_at": updated_at,
    }))
}

async fn fetch_backup_job(conn: &libsql::Connection, job_id: &str) -> AppResult<Value> {
    let mut rows = conn
        .query(
            "SELECT job_id, requested_backup_db_path, backup_tag, status, last_error_code, last_error_message,
                    backup_id, backup_db_path, size_bytes, sha256, worker_id, started_at, finished_at, created_at, updated_at
             FROM __kdb_jobs WHERE job_id = ? AND job_type = 'backup_db' LIMIT 1",
            libsql::params![job_id.to_string()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("get_backup_job failed: {e}")))?;
    let row = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("get_backup_job row failed: {e}")))?
        .ok_or_else(|| AppError::NotFound(format!("backup job not found: {job_id}")))?;

    Ok(json!({
        "job_id": row.get::<String>(0).map_err(|e| AppError::Internal(format!("get_backup_job decode failed: {e}")))?,
        "requested_backup_db_path": row.get::<String>(1).map_err(|e| AppError::Internal(format!("get_backup_job decode failed: {e}")))?,
        "backup_tag": row.get::<Option<String>>(2).map_err(|e| AppError::Internal(format!("get_backup_job decode failed: {e}")))?,
        "status": row.get::<String>(3).map_err(|e| AppError::Internal(format!("get_backup_job decode failed: {e}")))?,
        "last_error_code": row.get::<Option<String>>(4).map_err(|e| AppError::Internal(format!("get_backup_job decode failed: {e}")))?,
        "last_error_message": row.get::<Option<String>>(5).map_err(|e| AppError::Internal(format!("get_backup_job decode failed: {e}")))?,
        "backup_id": row.get::<Option<String>>(6).map_err(|e| AppError::Internal(format!("get_backup_job decode failed: {e}")))?,
        "backup_db_path": row.get::<Option<String>>(7).map_err(|e| AppError::Internal(format!("get_backup_job decode failed: {e}")))?,
        "size_bytes": row.get::<Option<i64>>(8).map_err(|e| AppError::Internal(format!("get_backup_job decode failed: {e}")))?,
        "sha256": row.get::<Option<String>>(9).map_err(|e| AppError::Internal(format!("get_backup_job decode failed: {e}")))?,
        "worker_id": row.get::<Option<String>>(10).map_err(|e| AppError::Internal(format!("get_backup_job decode failed: {e}")))?,
        "started_at": row.get::<Option<String>>(11).map_err(|e| AppError::Internal(format!("get_backup_job decode failed: {e}")))?,
        "finished_at": row.get::<Option<String>>(12).map_err(|e| AppError::Internal(format!("get_backup_job decode failed: {e}")))?,
        "created_at": row.get::<String>(13).map_err(|e| AppError::Internal(format!("get_backup_job decode failed: {e}")))?,
        "updated_at": row.get::<String>(14).map_err(|e| AppError::Internal(format!("get_backup_job decode failed: {e}")))?,
    }))
}

async fn fetch_generic_db_job(conn: &libsql::Connection, job_type: &str, job_id: &str) -> AppResult<Value> {
    let mut rows = conn
        .query(
            "SELECT job_id, job_type, status, payload_json, last_error_code, last_error_message,
                    worker_id, started_at, finished_at, created_at, updated_at
             FROM __kdb_jobs WHERE job_id = ? AND job_type = ? LIMIT 1",
            libsql::params![job_id.to_string(), job_type.to_string()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("get_generic_job failed: {e}")))?;
    let row = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("get_generic_job row failed: {e}")))?
        .ok_or_else(|| AppError::NotFound(format!("job not found: {job_id}")))?;

    let payload_raw: Option<String> = row
        .get(3)
        .map_err(|e| AppError::Internal(format!("get_generic_job decode failed: {e}")))?;
    let payload = payload_raw
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|e| AppError::Internal(format!("get_generic_job payload decode failed: {e}")))?
        .unwrap_or_else(|| json!({}));

    Ok(json!({
        "job_id": row.get::<String>(0).map_err(|e| AppError::Internal(format!("get_generic_job decode failed: {e}")))?,
        "job_type": row.get::<String>(1).map_err(|e| AppError::Internal(format!("get_generic_job decode failed: {e}")))?,
        "status": row.get::<String>(2).map_err(|e| AppError::Internal(format!("get_generic_job decode failed: {e}")))?,
        "payload": payload,
        "last_error_code": row.get::<Option<String>>(4).map_err(|e| AppError::Internal(format!("get_generic_job decode failed: {e}")))?,
        "last_error_message": row.get::<Option<String>>(5).map_err(|e| AppError::Internal(format!("get_generic_job decode failed: {e}")))?,
        "worker_id": row.get::<Option<String>>(6).map_err(|e| AppError::Internal(format!("get_generic_job decode failed: {e}")))?,
        "started_at": row.get::<Option<String>>(7).map_err(|e| AppError::Internal(format!("get_generic_job decode failed: {e}")))?,
        "finished_at": row.get::<Option<String>>(8).map_err(|e| AppError::Internal(format!("get_generic_job decode failed: {e}")))?,
        "created_at": row.get::<String>(9).map_err(|e| AppError::Internal(format!("get_generic_job decode failed: {e}")))?,
        "updated_at": row.get::<String>(10).map_err(|e| AppError::Internal(format!("get_generic_job decode failed: {e}")))?,
    }))
}

async fn fetch_replication_job(conn: &libsql::Connection, job_id: &str) -> AppResult<Value> {
    let id = job_id.parse::<i64>().map_err(|_| {
        AppError::BadRequest("replication job_id must be an integer string".to_string())
    })?;
    let mut rows = conn
        .query(
            "SELECT id, status, attempts, last_error, created_at, updated_at
             FROM __kdb_replication_jobs WHERE id = ? LIMIT 1",
            libsql::params![id],
        )
        .await
        .map_err(|e| AppError::Internal(format!("get_replication_job failed: {e}")))?;
    let row = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("get_replication_job row failed: {e}")))?
        .ok_or_else(|| AppError::NotFound(format!("replication job not found: {job_id}")))?;
    Ok(json!({
        "job_id": row.get::<i64>(0).map_err(|e| AppError::Internal(format!("get_replication_job decode failed: {e}")))?.to_string(),
        "status": row.get::<String>(1).map_err(|e| AppError::Internal(format!("get_replication_job decode failed: {e}")))?,
        "attempts": row.get::<i64>(2).map_err(|e| AppError::Internal(format!("get_replication_job decode failed: {e}")))?,
        "last_error_message": row.get::<Option<String>>(3).map_err(|e| AppError::Internal(format!("get_replication_job decode failed: {e}")))?,
        "created_at": row.get::<String>(4).map_err(|e| AppError::Internal(format!("get_replication_job decode failed: {e}")))?,
        "updated_at": row.get::<String>(5).map_err(|e| AppError::Internal(format!("get_replication_job decode failed: {e}")))?,
    }))
}

async fn apply_import_batch(
    conn: &libsql::Connection,
    collection: &str,
    docs: &[Value],
    on_conflict: &str,
    dry_run: bool,
    allow_system_timestamps: bool,
) -> AppResult<(i64, i64, i64)> {
    if dry_run {
        return Ok((docs.len() as i64, 0, 0));
    }

    let tx = conn
        .transaction()
        .await
        .map_err(|e| AppError::Internal(format!("import tx begin failed: {e}")))?;

    let mut inserted_count = 0i64;
    let mut updated_count = 0i64;
    let mut skipped_count = 0i64;

    let jsonb_enabled = jsonb_enabled();
    let data_expr = json_input_expr(jsonb_enabled);
    let merge_conflict_expr = if jsonb_enabled {
        "jsonb(json_patch(__kdb_documents.data, excluded.data))"
    } else {
        "json_patch(__kdb_documents.data, excluded.data)"
    };

    for doc in docs {
        let id = doc
            .get("_id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Internal("import row missing _id".to_string()))?;
        let mut stored_doc = doc.clone();
        let user_id = normalize_document_user_id_from_doc(&mut stored_doc, None)?;
        let (created_at, modified_at) = resolve_insert_timestamps(&stored_doc, allow_system_timestamps)?;
        let data = stored_doc.to_string();
        let size = data.len() as i64;

        let affected = match on_conflict {
            "error" => tx
                .execute(
                    &format!(
                        "INSERT INTO __kdb_documents (id, collection, _user_id, data, _size_bytes, _created_at, _modified_at)
                         VALUES (?, ?, ?, {data_expr}, ?, COALESCE(?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), COALESCE(?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')))"
                    ),
                    libsql::params![
                        id.to_string(),
                        collection.to_string(),
                        to_sql_nullable_text(user_id.clone()),
                        data,
                        size,
                        to_sql_nullable_text(created_at.clone()),
                        to_sql_nullable_text(modified_at.clone())
                    ],
                )
                .await
                .map_err(|e| AppError::Conflict(format!("import_jsonl conflict: {e}")))?,
            "skip" => tx
                .execute(
                    &format!(
                        "INSERT OR IGNORE INTO __kdb_documents (id, collection, _user_id, data, _size_bytes, _created_at, _modified_at)
                         VALUES (?, ?, ?, {data_expr}, ?, COALESCE(?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), COALESCE(?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')))"
                    ),
                    libsql::params![
                        id.to_string(),
                        collection.to_string(),
                        to_sql_nullable_text(user_id.clone()),
                        data,
                        size,
                        to_sql_nullable_text(created_at.clone()),
                        to_sql_nullable_text(modified_at.clone())
                    ],
                )
                .await
                .map_err(|e| AppError::Internal(format!("import_jsonl skip mode failed: {e}")))?,
            "replace" => tx
                .execute(
                    &format!(
                        "INSERT OR REPLACE INTO __kdb_documents (id, collection, _user_id, data, _size_bytes, _created_at, _modified_at)
                         VALUES (?, ?, ?, {data_expr}, ?, COALESCE(?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), COALESCE(?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')))"
                    ),
                    libsql::params![
                        id.to_string(),
                        collection.to_string(),
                        to_sql_nullable_text(user_id.clone()),
                        data,
                        size,
                        to_sql_nullable_text(created_at.clone()),
                        to_sql_nullable_text(modified_at.clone())
                    ],
                )
                .await
                .map_err(|e| AppError::Internal(format!("import_jsonl replace mode failed: {e}")))?,
            "merge" => tx
                .execute(
                    &format!(
                        "INSERT INTO __kdb_documents (id, collection, _user_id, data, _size_bytes, _created_at, _modified_at)
                         VALUES (?, ?, ?, {data_expr}, ?, COALESCE(?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), COALESCE(?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')))
                         ON CONFLICT(id) DO UPDATE SET
                            _user_id = COALESCE(excluded._user_id, __kdb_documents._user_id),
                            data = {merge_conflict_expr},
                            _size_bytes = length({merge_conflict_expr}),
                            _modified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')"
                    ),
                    libsql::params![
                        id.to_string(),
                        collection.to_string(),
                        to_sql_nullable_text(user_id),
                        data,
                        size,
                        to_sql_nullable_text(created_at),
                        to_sql_nullable_text(modified_at)
                    ],
                )
                .await
                .map_err(|e| AppError::Internal(format!("import_jsonl merge mode failed: {e}")))?,
            _ => {
                return Err(AppError::BadRequest(
                    "on_conflict must be one of: error, skip, replace, merge".to_string(),
                ));
            }
        };

        match on_conflict {
            "skip" => {
                if affected > 0 {
                    inserted_count += 1;
                } else {
                    skipped_count += 1;
                }
            }
            "replace" | "merge" => {
                // SQLite reports one affected row for both insert and update in these modes.
                updated_count += 1;
            }
            _ => inserted_count += 1,
        }
    }

    tx.commit()
        .await
        .map_err(|e| AppError::Internal(format!("import tx commit failed: {e}")))?;

    Ok((inserted_count, updated_count, skipped_count))
}
