// File catalog operations: stores file/object metadata without moving bytes.

async fn file_create(
    state: &AppState,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let file_id = normalize_optional_file_id(payload.id)?.unwrap_or_else(|| Uuid::new_v4().simple().to_string());
    let bucket = non_empty_or(payload.bucket, "bucket", "default")?;
    let storage_backend = required_text(payload.storage_backend, "storage_backend")?;
    let storage_path = required_text(payload.storage_path, "storage_path")?;
    let filename = clean_optional(payload.filename);
    let content_type = clean_optional(payload.content_type);
    let sha256 = clean_optional(payload.sha256);
    let status = non_empty_or(payload.status, "status", "active")?;
    let owner_type = clean_optional(payload.owner_type);
    let owner_id = clean_optional(payload.owner_id);
    let metadata = payload.metadata.unwrap_or_else(|| json!({}));
    if !metadata.is_object() {
        return Err(AppError::BadRequest("metadata must be an object when provided".to_string()));
    }
    let uploaded_at = match clean_optional(payload.uploaded_at) {
        Some(raw) => Some(normalize_rfc3339_utc(&raw)?),
        None => None,
    };
    let expires_at = match clean_optional(payload.expires_at) {
        Some(raw) => Some(normalize_rfc3339_utc(&raw)?),
        None => None,
    };
    let json_expr = json_input_expr(state.jsonb_enabled);
    conn.execute(
        &format!(
            "INSERT INTO __kdb_files (
                id, bucket, storage_backend, storage_path, filename, content_type, size_bytes,
                sha256, status, owner_type, owner_id, metadata, uploaded_at, expires_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, {json_expr}, COALESCE(?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), ?)"
        ),
        libsql::params![
            file_id.clone(),
            bucket,
            storage_backend,
            storage_path,
            to_sql_nullable_text(filename),
            to_sql_nullable_text(content_type),
            payload.size_bytes.unwrap_or(0).max(0),
            to_sql_nullable_text(sha256),
            status,
            to_sql_nullable_text(owner_type),
            to_sql_nullable_text(owner_id),
            metadata.to_string(),
            to_sql_nullable_text(uploaded_at),
            to_sql_nullable_text(expires_at)
        ],
    )
    .await
    .map_err(|e| AppError::BadRequest(format!("file_create failed: {e}")))?;
    let item = fetch_file(conn, &file_id).await?;
    Ok(GatewayResponse::ok(Some(json!({"item": item}))))
}

async fn file_get(conn: &libsql::Connection, req: GatewayRequest) -> AppResult<GatewayResponse> {
    let file_id = required_text(req.payload.id, "id")?;
    let item = fetch_file_optional(conn, &file_id).await?;
    Ok(GatewayResponse::ok(Some(json!({"item": item}))))
}

async fn file_list(conn: &libsql::Connection, req: GatewayRequest) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let (limit, offset, page) = resolve_pagination_args(&payload, 25)?;
    let limit = limit.clamp(1, 200);
    let mut clauses = Vec::<String>::new();
    let mut binds = Vec::<libsql::Value>::new();

    if let Some(bucket) = clean_optional(payload.bucket) {
        clauses.push("bucket = ?".to_string());
        binds.push(libsql::Value::Text(bucket));
    }
    if let Some(status) = clean_optional(payload.status) {
        clauses.push("status = ?".to_string());
        binds.push(libsql::Value::Text(status));
    }
    if let Some(owner_type) = clean_optional(payload.owner_type) {
        clauses.push("owner_type = ?".to_string());
        binds.push(libsql::Value::Text(owner_type));
    }
    if let Some(owner_id) = clean_optional(payload.owner_id) {
        clauses.push("owner_id = ?".to_string());
        binds.push(libsql::Value::Text(owner_id));
    }
    if let Some(storage_backend) = clean_optional(payload.storage_backend) {
        clauses.push("storage_backend = ?".to_string());
        binds.push(libsql::Value::Text(storage_backend));
    }
    if let Some(content_type) = clean_optional(payload.content_type) {
        clauses.push("content_type = ?".to_string());
        binds.push(libsql::Value::Text(content_type));
    }
    if let Some(search) = clean_optional(payload.search) {
        clauses.push("(id LIKE ? OR filename LIKE ? OR storage_path LIKE ? OR owner_id LIKE ?)".to_string());
        let like = format!("%{}%", search.replace('%', "\\%").replace('_', "\\_"));
        binds.extend([
            libsql::Value::Text(like.clone()),
            libsql::Value::Text(like.clone()),
            libsql::Value::Text(like.clone()),
            libsql::Value::Text(like),
        ]);
    }

    let where_clause = if clauses.is_empty() { "1=1".to_string() } else { clauses.join(" AND ") };
    let total_items = file_count(conn, &where_clause, binds.clone()).await?;
    let mut page_binds = binds;
    page_binds.push(libsql::Value::Integer(limit));
    page_binds.push(libsql::Value::Integer(offset));
    let mut rows = conn
        .query(
            &format!(
                "SELECT {} FROM __kdb_files
                 WHERE {where_clause}
                 ORDER BY uploaded_at DESC, created_at DESC, id DESC
                 LIMIT ? OFFSET ?",
                file_select()
            ),
            page_binds,
        )
        .await
        .map_err(|e| AppError::Internal(format!("file_list query failed: {e}")))?;
    let mut items = Vec::<Value>::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("file_list row read failed: {e}")))?
    {
        items.push(file_from_row(&row)?);
    }
    let (next_offset, prev_offset) = build_offsets(total_items, items.len(), limit, offset);
    Ok(GatewayResponse::ok(Some(json!({
        "items": items,
        "count": items.len(),
        "total_items": total_items,
        "limit": limit,
        "offset": offset,
        "next_offset": next_offset,
        "prev_offset": prev_offset,
        "pagination": build_pagination(total_items, items.len(), limit, page, offset)
    }))))
}

async fn file_update(
    state: &AppState,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let file_id = required_text(payload.id, "id")?;
    let metadata = payload.metadata.unwrap_or(Value::Null);
    if !metadata.is_null() && !metadata.is_object() {
        return Err(AppError::BadRequest("metadata must be an object when provided".to_string()));
    }
    let uploaded_at = match clean_optional(payload.uploaded_at) {
        Some(raw) => Some(normalize_rfc3339_utc(&raw)?),
        None => None,
    };
    let expires_at = match clean_optional(payload.expires_at) {
        Some(raw) => Some(normalize_rfc3339_utc(&raw)?),
        None => None,
    };
    let json_expr = json_input_expr(state.jsonb_enabled);
    let changed = conn
        .execute(
            &format!(
                "UPDATE __kdb_files
                 SET bucket = COALESCE(?, bucket),
                     storage_backend = COALESCE(?, storage_backend),
                     storage_path = COALESCE(?, storage_path),
                     filename = COALESCE(?, filename),
                     content_type = COALESCE(?, content_type),
                     size_bytes = COALESCE(?, size_bytes),
                     sha256 = COALESCE(?, sha256),
                     status = COALESCE(?, status),
                     owner_type = COALESCE(?, owner_type),
                     owner_id = COALESCE(?, owner_id),
                     metadata = CASE WHEN ? IS NULL THEN metadata ELSE {json_expr} END,
                     uploaded_at = COALESCE(?, uploaded_at),
                     expires_at = COALESCE(?, expires_at),
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE id = ?"
            ),
            libsql::params![
                to_sql_nullable_text(clean_optional(payload.bucket)),
                to_sql_nullable_text(clean_optional(payload.storage_backend)),
                to_sql_nullable_text(clean_optional(payload.storage_path)),
                to_sql_nullable_text(clean_optional(payload.filename)),
                to_sql_nullable_text(clean_optional(payload.content_type)),
                payload.size_bytes.map(|v| v.max(0)),
                to_sql_nullable_text(clean_optional(payload.sha256)),
                to_sql_nullable_text(clean_optional(payload.status)),
                to_sql_nullable_text(clean_optional(payload.owner_type)),
                to_sql_nullable_text(clean_optional(payload.owner_id)),
                if metadata.is_null() { libsql::Value::Null } else { libsql::Value::Text(metadata.to_string()) },
                metadata.to_string(),
                to_sql_nullable_text(uploaded_at),
                to_sql_nullable_text(expires_at),
                file_id.clone()
            ],
        )
        .await
        .map_err(|e| AppError::BadRequest(format!("file_update failed: {e}")))?;
    if changed == 0 {
        return Err(AppError::BadRequest("file not found".to_string()));
    }
    let item = fetch_file(conn, &file_id).await?;
    Ok(GatewayResponse::ok(Some(json!({"item": item}))))
}

async fn file_delete(conn: &libsql::Connection, req: GatewayRequest) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let file_id = required_text(payload.id, "id")?;
    let purge = payload.purge.unwrap_or(false);
    let changed = if purge {
        conn.execute("DELETE FROM __kdb_files WHERE id = ?", libsql::params![file_id.clone()])
            .await
            .map_err(|e| AppError::Internal(format!("file_delete purge failed: {e}")))?
    } else {
        conn.execute(
            "UPDATE __kdb_files
             SET status = 'deleted', deleted_at = COALESCE(deleted_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?",
            libsql::params![file_id.clone()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("file_delete failed: {e}")))?
    };
    Ok(GatewayResponse::ok(Some(json!({
        "id": file_id,
        "deleted": changed > 0,
        "purged": purge,
        "bytes_deleted": false
    }))))
}

async fn fetch_file(conn: &libsql::Connection, file_id: &str) -> AppResult<Value> {
    fetch_file_optional(conn, file_id)
        .await?
        .ok_or_else(|| AppError::BadRequest("file not found".to_string()))
}

async fn fetch_file_optional(conn: &libsql::Connection, file_id: &str) -> AppResult<Option<Value>> {
    let mut rows = conn
        .query(
            &format!("SELECT {} FROM __kdb_files WHERE id = ? LIMIT 1", file_select()),
            libsql::params![file_id.to_string()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("file_get query failed: {e}")))?;
    if let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("file_get row read failed: {e}")))?
    {
        Ok(Some(file_from_row(&row)?))
    } else {
        Ok(None)
    }
}

async fn file_count(conn: &libsql::Connection, where_clause: &str, binds: Vec<libsql::Value>) -> AppResult<i64> {
    let mut rows = conn
        .query(&format!("SELECT COUNT(*) FROM __kdb_files WHERE {where_clause}"), binds)
        .await
        .map_err(|e| AppError::Internal(format!("file_list count failed: {e}")))?;
    let row = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("file_list count row failed: {e}")))?
        .ok_or_else(|| AppError::Internal("file_list count returned no row".to_string()))?;
    row.get::<i64>(0)
        .map_err(|e| AppError::Internal(format!("file_list count decode failed: {e}")))
}

fn file_select() -> &'static str {
    "id, bucket, storage_backend, storage_path, filename, content_type, size_bytes, sha256, status,
     owner_type, owner_id, metadata, uploaded_at, created_at, updated_at, deleted_at, expires_at"
}

fn file_from_row(row: &libsql::Row) -> AppResult<Value> {
    let metadata_raw = row
        .get::<Option<String>>(11)
        .map_err(|e| AppError::Internal(format!("file metadata decode failed: {e}")))?
        .unwrap_or_else(|| "{}".to_string());
    let metadata = serde_json::from_str::<Value>(&metadata_raw).unwrap_or_else(|_| json!({}));
    Ok(json!({
        "id": row.get::<String>(0).map_err(|e| AppError::Internal(format!("file decode failed: {e}")))?,
        "bucket": row.get::<String>(1).map_err(|e| AppError::Internal(format!("file decode failed: {e}")))?,
        "storage_backend": row.get::<String>(2).map_err(|e| AppError::Internal(format!("file decode failed: {e}")))?,
        "storage_path": row.get::<String>(3).map_err(|e| AppError::Internal(format!("file decode failed: {e}")))?,
        "filename": row.get::<Option<String>>(4).map_err(|e| AppError::Internal(format!("file decode failed: {e}")))?,
        "content_type": row.get::<Option<String>>(5).map_err(|e| AppError::Internal(format!("file decode failed: {e}")))?,
        "size_bytes": row.get::<i64>(6).map_err(|e| AppError::Internal(format!("file decode failed: {e}")))?,
        "sha256": row.get::<Option<String>>(7).map_err(|e| AppError::Internal(format!("file decode failed: {e}")))?,
        "status": row.get::<String>(8).map_err(|e| AppError::Internal(format!("file decode failed: {e}")))?,
        "owner_type": row.get::<Option<String>>(9).map_err(|e| AppError::Internal(format!("file decode failed: {e}")))?,
        "owner_id": row.get::<Option<String>>(10).map_err(|e| AppError::Internal(format!("file decode failed: {e}")))?,
        "metadata": metadata,
        "uploaded_at": row.get::<String>(12).map_err(|e| AppError::Internal(format!("file decode failed: {e}")))?,
        "created_at": row.get::<String>(13).map_err(|e| AppError::Internal(format!("file decode failed: {e}")))?,
        "updated_at": row.get::<String>(14).map_err(|e| AppError::Internal(format!("file decode failed: {e}")))?,
        "deleted_at": row.get::<Option<String>>(15).map_err(|e| AppError::Internal(format!("file decode failed: {e}")))?,
        "expires_at": row.get::<Option<String>>(16).map_err(|e| AppError::Internal(format!("file decode failed: {e}")))?
    }))
}

fn normalize_optional_file_id(id: Option<String>) -> AppResult<Option<String>> {
    let Some(id) = clean_optional(id) else {
        return Ok(None);
    };
    if id.chars().all(|c| c.is_ascii_hexdigit()) && id.len() == 32 {
        Ok(Some(id.to_ascii_lowercase()))
    } else {
        Err(AppError::BadRequest("id must be a 32-character dashless uuid string".to_string()))
    }
}
