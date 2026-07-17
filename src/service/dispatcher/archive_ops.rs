// Archive and delete operation handlers extracted from dispatcher.rs.

async fn delete(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let common = parse_delete_common_options(state, &payload)?;
    let max_docs = payload.max_docs;
    let (ids, collection) = if payload.filter.is_some() {
        let collection = resolve_collection_scope(&payload)?;
        let ids = target_ids_from_payload_with_collection(
            conn,
            &payload,
            collection.as_deref(),
            max_docs,
            "delete",
        )
        .await?;
        (ids, collection)
    } else {
        let collection = resolve_collection_scope_optional_collection(&payload)?;
        let ids = apply_max_docs_to_ids(extract_ids_or_single_strict(&payload)?, max_docs)?;
        (ids, collection)
    };

    if common.dry_run {
        let matched = count_by_ids(conn, collection.as_deref(), &ids).await?;
        return Ok(GatewayResponse::ok(Some(json!({
            "count": matched,
            "matched_count": matched,
            "deleted_count": matched,
            "purge": common.hard_delete,
            "soft_delete": !common.hard_delete,
            "dry_run": true
        }))));
    }

    if common.hard_delete {
        let deleted = hard_delete_document_ids(conn, collection.as_deref(), &ids).await?;
        state
            .db_manager
            .append_wal_record(
                db_path,
                "DELETE_PURGE",
                &json!({"collection": collection, "count": deleted, "purge": true}).to_string(),
            )
            .await?;
        return Ok(GatewayResponse::ok(Some(json!({
            "count": deleted,
            "matched_count": deleted,
            "deleted_count": deleted,
            "purge": true,
            "soft_delete": false
        }))));
    }

    let result =
        __kdb_archive_and_delete_ids(conn, collection.as_deref(), &ids, common.__kdb_archive_ttl).await?;
    state
        .db_manager
        .append_wal_record(
            db_path,
            "DELETE",
            &json!({"collection": collection, "count": result.deleted_count, "_txn_id": result.txn_id, "__kdb_archive_ttl_seconds": common.__kdb_archive_ttl}).to_string(),
        )
        .await?;

    Ok(GatewayResponse {
        status: "success",
        data: Some(json!({
            "count": result.deleted_count,
            "matched_count": result.deleted_count,
            "deleted_count": result.deleted_count,
            "purge": false,
            "soft_delete": true
        })),
        txn_id: Some(result.txn_id),
        message: Some("deleted".to_string()),
        ack_mode: None,
        ack_status: None,
        committed: None,
        is_async_ack: None,
    })
}
async fn drop_collection(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let collection = require_collection(&payload)?;
    let dry_run = payload.dry_run.unwrap_or(false);
    let hard_delete = payload.purge.unwrap_or(false);
    let ids = select_namespace_target_ids(conn, &collection, payload.max_docs).await?;

    if dry_run {
        return Ok(GatewayResponse::ok(Some(json!({
            "collection": collection,
            "count": ids.len(),
            "deleted_count": ids.len(),
            "purge": hard_delete,
            "dry_run": true
        }))));
    }

    if hard_delete {
        let deleted = hard_delete_document_ids(conn, Some(collection.as_str()), &ids).await?;
        state
            .db_manager
            .append_wal_record(
                db_path,
                "DROP_COLLECTION_PURGE",
                &json!({"collection": collection, "deleted_count": deleted}).to_string(),
            )
            .await?;
        return Ok(GatewayResponse::ok(Some(json!({
            "collection": collection,
            "deleted_count": deleted,
            "purge": true
        }))));
    }

    let __kdb_archive_ttl = delete_archive_ttl_secs(state, payload.ttl_seconds)?;
    let result = __kdb_archive_and_delete_ids(conn, Some(collection.as_str()), &ids, __kdb_archive_ttl).await?;
    state
        .db_manager
        .append_wal_record(
            db_path,
            "DROP_COLLECTION",
            &json!({"collection": collection, "count": result.deleted_count, "_txn_id": result.txn_id, "__kdb_archive_ttl_seconds": __kdb_archive_ttl}).to_string(),
        )
        .await?;

    Ok(GatewayResponse {
        status: "success",
        data: Some(json!({
            "collection": collection,
            "deleted_count": result.deleted_count,
            "purge": false
        })),
        txn_id: Some(result.txn_id),
        message: Some("dropped_collection".to_string()),
        ack_mode: None,
        ack_status: None,
        committed: None,
        is_async_ack: None,
    })
}

async fn purge_kdb_archive(
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let (where_clause, binds) = build_kdb_archive_target_where(&payload, true)?;
    let dry_run = payload.dry_run.unwrap_or(false);
    let matched = execute_count(conn, "__kdb_archive", &where_clause, binds.clone()).await?;

    if dry_run {
        return Ok(GatewayResponse::ok(Some(json!({
            "matched_count": matched,
            "purged_count": matched,
            "dry_run": true
        }))));
    }

    let deleted = conn
        .execute(&format!("DELETE FROM __kdb_archive WHERE {where_clause}"), binds)
        .await
        .map_err(|e| AppError::Internal(format!("purge failed: {e}")))?;

    Ok(GatewayResponse::ok(Some(json!({
        "matched_count": matched,
        "purged_count": deleted
    }))))
}

async fn restore_kdb_archive(
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let dry_run = payload.dry_run.unwrap_or(false);
    let on_conflict = payload
        .on_conflict
        .as_deref()
        .unwrap_or("skip")
        .to_ascii_lowercase();
    if !matches!(on_conflict.as_str(), "skip" | "replace" | "patch") {
        return Err(AppError::BadRequest(
            "restore on_conflict must be one of: skip, replace, patch".to_string(),
        ));
    }

    let (where_clause, binds) = build_kdb_archive_target_where(&payload, false)?;
    let matched = execute_count(conn, "__kdb_archive", &where_clause, binds.clone()).await?;

    if dry_run {
        let restorable = if on_conflict == "skip" {
            count_restorable_kdb_archive_rows(conn, &where_clause, binds.clone()).await?
        } else {
            matched as usize
        };
        return Ok(GatewayResponse::ok(Some(json!({
            "matched_count": matched,
            "restored_count": restorable,
            "skipped_conflicts": matched.saturating_sub(restorable as i64),
            "on_conflict": on_conflict,
            "dry_run": true
        }))));
    }

    let tx = conn
        .transaction()
        .await
        .map_err(|e| AppError::Internal(format!("restore tx begin failed: {e}")))?;

    let mut rows = tx
        .query(
            &format!(
                "SELECT id, collection, _user_id, json(data), _size_bytes, _expires_at, _created_at, _modified_at
                 FROM __kdb_archive WHERE {where_clause}"
            ),
            binds,
        )
        .await
        .map_err(|e| AppError::Internal(format!("restore select failed: {e}")))?;

    let mut targets = Vec::<ArchiveRow>::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("restore select row read failed: {e}")))?
    {
        targets.push(ArchiveRow {
            id: row
                .get(0)
                .map_err(|e| AppError::Internal(format!("restore id decode failed: {e}")))?,
            collection: row.get(1).map_err(|e| {
                AppError::Internal(format!("restore collection decode failed: {e}"))
            })?,
            user_id: row
                .get(2)
                .map_err(|e| AppError::Internal(format!("restore _user_id decode failed: {e}")))?,
            data: row
                .get(3)
                .map_err(|e| AppError::Internal(format!("restore data decode failed: {e}")))?,
            size_bytes: row
                .get(4)
                .map_err(|e| AppError::Internal(format!("restore size decode failed: {e}")))?,
            expires_at: row
                .get(5)
                .map_err(|e| AppError::Internal(format!("restore expires decode failed: {e}")))?,
            created_at: row
                .get(6)
                .map_err(|e| AppError::Internal(format!("restore created decode failed: {e}")))?,
            modified_at: row
                .get(7)
                .map_err(|e| AppError::Internal(format!("restore modified decode failed: {e}")))?,
        });
    }

    let mut restored_count = 0usize;
    let mut skipped_conflicts = 0usize;

    for row in &targets {
        let mut exists_rows = tx
            .query(
                "SELECT 1 FROM __kdb_documents WHERE id = ? LIMIT 1",
                libsql::params![row.id.clone()],
            )
            .await
            .map_err(|e| AppError::Internal(format!("restore exists check failed: {e}")))?;
        let exists = exists_rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("restore exists row read failed: {e}")))?
            .is_some();

        if exists && on_conflict == "skip" {
            tx.execute(
                "UPDATE __kdb_archive
                 SET _restore_failed = 1, _restore_reason = ?
                 WHERE id = ?",
                libsql::params!["conflict:skip".to_string(), row.id.clone()],
            )
            .await
            .map_err(|e| {
                AppError::Internal(format!("restore __kdb_archive conflict mark failed: {e}"))
            })?;
            skipped_conflicts += 1;
            continue;
        }

        if exists && on_conflict == "replace" {
            tx.execute(
                "UPDATE __kdb_documents
                 SET collection = ?,
                     _user_id = ?,
                     data = ?,
                     _size_bytes = ?,
                     _expires_at = ?,
                     _expiry_behavior = 'archive',
                     _created_at = ?,
                     _modified_at = ?
                 WHERE id = ?",
                libsql::params![
                    row.collection.clone(),
                    to_sql_nullable_text(row.user_id.clone()),
                    row.data.clone(),
                    row.size_bytes,
                    row.expires_at,
                    row.created_at.clone(),
                    row.modified_at
                        .clone()
                        .unwrap_or_else(|| row.created_at.clone()),
                    row.id.clone()
                ],
            )
            .await
            .map_err(|e| AppError::Internal(format!("restore replace update failed: {e}")))?;
        } else if exists && on_conflict == "patch" {
            let patch_expr = json_patch_expr(jsonb_enabled());
            tx.execute(
                &format!(
                    "UPDATE __kdb_documents
                     SET data = {patch_expr},
                         _user_id = COALESCE(?, _user_id),
                         _size_bytes = length({patch_expr}),
                         _expires_at = ?,
                         _expiry_behavior = 'archive',
                         _modified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     WHERE id = ?"
                ),
                libsql::params![
                    row.data.clone(),
                    to_sql_nullable_text(row.user_id.clone()),
                    row.data.clone(),
                    row.expires_at,
                    row.id.clone()
                ],
            )
            .await
            .map_err(|e| AppError::Internal(format!("restore patch update failed: {e}")))?;
        } else {
            let data_expr = json_input_expr(jsonb_enabled());
            tx.execute(
                &format!(
                    "INSERT INTO __kdb_documents (id, collection, _user_id, data, _size_bytes, _expires_at, _expiry_behavior, _created_at, _modified_at)
                     VALUES (?, ?, ?, {data_expr}, ?, ?, 'archive', ?, ?)"
                ),
                libsql::params![
                    row.id.clone(),
                    row.collection.clone(),
                    to_sql_nullable_text(row.user_id.clone()),
                    row.data.clone(),
                    row.size_bytes,
                    row.expires_at,
                    row.created_at.clone(),
                    row.modified_at
                        .clone()
                        .unwrap_or_else(|| row.created_at.clone())
                ],
            )
            .await
            .map_err(|e| AppError::Internal(format!("restore insert failed: {e}")))?;
        }

        tx.execute(
            "DELETE FROM __kdb_archive WHERE id = ?",
            libsql::params![row.id.clone()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("restore archive cleanup failed: {e}")))?;
        restored_count += 1;
    }

    tx.commit()
        .await
        .map_err(|e| AppError::Internal(format!("restore tx commit failed: {e}")))?;

    Ok(GatewayResponse::ok(Some(json!({
        "matched_count": matched,
        "restored_count": restored_count,
        "skipped_conflicts": skipped_conflicts,
        "on_conflict": on_conflict
    }))))
}

struct ArchiveRow {
    id: String,
    collection: String,
    user_id: Option<String>,
    data: String,
    size_bytes: i64,
    expires_at: Option<i64>,
    created_at: String,
    modified_at: Option<String>,
}
