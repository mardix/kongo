// Write operation handlers extracted from dispatcher.rs.

#[derive(Clone, Debug)]
struct InsertCommonOptions {
    dry_run: bool,
    allow_system_timestamps: bool,
    unique_fields: Vec<String>,
    on_conflict: String,
}

#[derive(Clone, Debug)]
struct PreparedInsertDocuments {
    docs: Vec<Value>,
}

#[derive(Clone, Debug)]
struct DeleteCommonOptions {
    dry_run: bool,
    hard_delete: bool,
    __kdb_archive_ttl: Option<i64>,
}

#[derive(Clone, Debug)]
struct UpsertPreparedInputs {
    collection: Option<String>,
    dry_run: bool,
    max_docs: i64,
    user_id: Option<String>,
    filter: Value,
    insert_data: Value,
    update_data: Value,
}

#[derive(Clone, Debug)]
struct SetPreparedInput {
    collection: Option<String>,
    dry_run: bool,
    id: String,
    user_id: Option<String>,
    data: Value,
    data_str: String,
    size: i64,
    expires_at: Option<i64>,
    expiry_behavior: String,
}

fn parse_insert_common_options(payload: &OperationPayload) -> AppResult<InsertCommonOptions> {
    Ok(InsertCommonOptions {
        dry_run: payload.dry_run.unwrap_or(false),
        allow_system_timestamps: payload.allow_system_timestamps.unwrap_or(false),
        unique_fields: normalize_unique_fields(payload.unique_fields.clone())?,
        on_conflict: parse_insert_on_conflict(payload.on_conflict.as_deref())?,
    })
}

fn prepare_insert_documents(data: Option<Value>, bulk: bool) -> AppResult<PreparedInsertDocuments> {
    let data = data.ok_or_else(|| AppError::BadRequest("data is required".to_string()))?;
    if bulk {
        let arr = data
            .as_array()
            .ok_or_else(|| AppError::BadRequest("data must be an array for insert_bulk".to_string()))?;
        if arr.is_empty() {
            return Err(AppError::BadRequest("data cannot be empty".to_string()));
        }
        let mut docs = Vec::<Value>::with_capacity(arr.len());
        for item in arr {
            let mut doc = item
                .as_object()
                .cloned()
                .ok_or_else(|| AppError::BadRequest("all data items must be objects".to_string()))?;
            let mut v = Value::Object(std::mem::take(&mut doc));
            expand_kdb_macros_in_value(&mut v)?;
            let _ = ensure_or_get_id(&mut v)?;
            docs.push(v);
        }
        return Ok(PreparedInsertDocuments { docs });
    }

    let mut doc = require_object(Some(data), "data")?;
    expand_kdb_macros_in_value(&mut doc)?;
    let _ = ensure_or_get_id(&mut doc)?;
    Ok(PreparedInsertDocuments { docs: vec![doc] })
}

fn extract_single_write_doc(data: Option<Value>, field_name: &str) -> AppResult<(String, Value)> {
    let doc = require_object(data, field_name)?;
    let id = doc
        .get("_id")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::BadRequest(format!("{field_name}._id is required")))?
        .to_string();
    Ok((id, doc))
}

fn parse_delete_common_options(
    state: &AppState,
    payload: &OperationPayload,
) -> AppResult<DeleteCommonOptions> {
    let hard_delete = payload.purge.unwrap_or(false);
    let __kdb_archive_ttl = if hard_delete {
        None
    } else {
        delete_archive_ttl_secs(state, payload.ttl_seconds)?
    };
    Ok(DeleteCommonOptions {
        dry_run: payload.dry_run.unwrap_or(false),
        hard_delete,
        __kdb_archive_ttl,
    })
}

fn prepare_upsert_inputs(payload: OperationPayload) -> AppResult<UpsertPreparedInputs> {
    let collection = resolve_collection_scope(&payload)?;
    let dry_run = payload.dry_run.unwrap_or(false);
    let max_docs = payload.max_docs.unwrap_or(1);
    if max_docs < -1 {
        return Err(AppError::BadRequest(
            "max_docs must be -1, 0, or positive".to_string(),
        ));
    }

    let filter = require_non_empty_filter(payload.filter)?;
    let payload_user_id = clean_optional(payload.document_user_id.clone());
    let mut insert_data = require_object(payload.insert_data, "insert_data")?;
    expand_kdb_macros_in_value(&mut insert_data)?;
    let user_id = normalize_document_user_id_from_doc(&mut insert_data, payload_user_id.as_deref())?;
    let update_data = if max_docs == 0 {
        payload.update_data.unwrap_or_else(|| json!({}))
    } else {
        require_object(payload.update_data, "update_data")?
    };
    let mut update_data = update_data;
    expand_kdb_macros_in_value(&mut update_data)?;

    reject_id_field(&insert_data, "insert_data")?;
    if max_docs != 0 || !update_data.is_object() {
        reject_id_field(&update_data, "update_data")?;
    }

    Ok(UpsertPreparedInputs {
        collection,
        dry_run,
        max_docs,
        user_id,
        filter,
        insert_data,
        update_data,
    })
}

fn prepare_set_input(payload: OperationPayload) -> AppResult<SetPreparedInput> {
    if payload.ids.is_some() {
        return Err(AppError::BadRequest(
            "set accepts only one document; use data._id or id".to_string(),
        ));
    }
    let collection = resolve_collection_scope_optional_collection(&payload)?;
    let dry_run = payload.dry_run.unwrap_or(false);
    let payload_user_id = clean_optional(payload.document_user_id.clone());
    let mut data = require_object(payload.data, "data")?;
    expand_kdb_macros_in_value(&mut data)?;
    let user_id = normalize_document_user_id_from_doc(&mut data, payload_user_id.as_deref())?;
    let id = data
        .get("_id")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::BadRequest("data._id is required".to_string()))?
        .to_string();
    if data.as_object().expect("object checked").len() == 1 {
        return Err(AppError::BadRequest(
            "data must include fields beyond _id".to_string(),
        ));
    }
    let data_str = data.to_string();
    let size = data_str.len() as i64;
    let expires_at = ttl_to_expires_at(payload.ttl_seconds)?;
    let expiry_behavior = normalized_expiry_behavior(payload.expiry_behavior.as_deref());
    Ok(SetPreparedInput {
        collection,
        dry_run,
        id,
        user_id,
        data,
        data_str,
        size,
        expires_at,
        expiry_behavior,
    })
}

async fn insert(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let collection = require_collection(&payload)?;
    let common = parse_insert_common_options(&payload)?;
    let bulk = matches!(payload.data.as_ref(), Some(Value::Array(_)));
    let ttl_seconds = payload.ttl_seconds;
    let payload_user_id = clean_optional(payload.document_user_id.clone());
    let expiry_behavior = normalized_expiry_behavior(payload.expiry_behavior.as_deref());
    let mut docs = prepare_insert_documents(payload.data, bulk)?.docs;
    let mut doc_user_ids = Vec::<Option<String>>::with_capacity(docs.len());
    for doc in &mut docs {
        doc_user_ids.push(normalize_document_user_id_from_doc(
            doc,
            payload_user_id.as_deref(),
        )?);
    }

    if common.dry_run {
        let skipped_count = if common.unique_fields.is_empty() {
            0
        } else {
            count_bulk_unique_skips(conn, &collection, &docs, &common.unique_fields).await?
        };
        let inserted_count = docs.len().saturating_sub(skipped_count);
        return Ok(GatewayResponse::ok(Some(json!({
            "items": docs,
            "count": inserted_count,
            "inserted_count": inserted_count,
            "skipped_count": skipped_count,
            "dry_run": true
        }))));
    }

    let mut docs_to_insert = Vec::<(Value, Option<String>)>::new();
    let mut skipped_count = 0usize;
    let mut seen_unique = HashSet::<String>::new();
    for (idx, doc) in docs.iter().enumerate() {
        if !common.unique_fields.is_empty() {
            let pairs = resolve_unique_pairs(doc, &common.unique_fields)?;
            if !pairs.is_empty() {
                let sig = unique_signature(&pairs)?;
                if seen_unique.contains(&sig)
                    || exists_by_unique_pairs(conn, &collection, &pairs).await?
                {
                    if common.on_conflict == "error" {
                        return Err(AppError::Conflict("insert unique_fields conflict".to_string()));
                    }
                    skipped_count += 1;
                    continue;
                }
                seen_unique.insert(sig);
            }
        }
        docs_to_insert.push((doc.clone(), doc_user_ids.get(idx).cloned().flatten()));
    }

    let tx = conn
        .transaction()
        .await
        .map_err(|e| AppError::Internal(format!("tx begin failed: {e}")))?;

    let expires_at = ttl_to_expires_at(ttl_seconds)?;
    let mut kept_ids = Vec::<String>::new();
    for (doc, user_id) in &docs_to_insert {
        let id = doc
            .get("_id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Internal("generated _id missing".to_string()))?;
        let (created_at, modified_at) =
            resolve_insert_timestamps(doc, common.allow_system_timestamps)?;
        let data = doc.to_string();
        let size = data.len() as i64;
        let data_expr = json_input_expr(state.jsonb_enabled);
        tx.execute(
            &format!(
                "INSERT INTO __kdb_documents (id, collection, _user_id, data, _size_bytes, _expires_at, _expiry_behavior, _created_at, _modified_at)
                 VALUES (?, ?, ?, {data_expr}, ?, ?, ?, COALESCE(?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), COALESCE(?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')))"
            ),
            libsql::params![
                id.to_string(),
                collection.clone(),
                to_sql_nullable_text(user_id.clone()),
                data,
                size,
                expires_at,
                expiry_behavior.clone(),
                to_sql_nullable_text(created_at),
                to_sql_nullable_text(modified_at)
            ],
        )
        .await
        .map_err(|e| AppError::Conflict(format!("insert failed: {e}")))?;
        kept_ids.push(id.to_string());
    }

    tx.commit()
        .await
        .map_err(|e| AppError::Internal(format!("tx commit failed: {e}")))?;

    state
        .db_manager
        .append_wal_record(
            db_path,
            "INSERT",
            &json!({"collection": collection, "count": kept_ids.len()}).to_string(),
        )
        .await?;

    let items = fetch_kdb_documents_by_ids(
        conn,
        Some(collection.as_str()),
        &kept_ids,
        state.response_include_system_timestamps,
    )
    .await?;

    Ok(GatewayResponse::ok(Some(json!({
        "items": items,
        "count": items.len(),
        "inserted_count": items.len(),
        "skipped_count": skipped_count
    }))))
}

async fn update(
    state: &AppState,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let dry_run = payload.dry_run.unwrap_or(false);
    let replace = payload.replace.unwrap_or(false);
    if payload.ids.is_some() {
        return Err(AppError::BadRequest(
            "update does not accept ids; use filter with _id.$in or data array with _id"
                .to_string(),
        ));
    }

    match (&payload.filter, &payload.data) {
        (None, Some(Value::Object(_))) => {
            let collection = resolve_collection_scope_optional_collection(&payload)?;
            if replace {
                let (id, mut data) = extract_single_write_doc(payload.data, "data")?;
                if dry_run {
                    let matched = count_by_ids(conn, collection.as_deref(), &[id]).await?;
                    return Ok(GatewayResponse::ok(Some(json!({
                        "items": [],
                        "count": matched,
                        "matched_count": matched,
                        "updated_count": matched,
                        "dry_run": true
                    }))));
                }
                let replacement = replacement_doc_from_payload(&mut data, &id)?;
                let replacement_str = replacement.to_string();
                let mut binds = vec![
                    libsql::Value::Text(replacement_str.clone()),
                    libsql::Value::Text(replacement_str),
                ];
                let where_clause = where_id_with_scope(&mut binds, collection.as_deref(), &id);
                let mut rows = conn
                    .query(
                        &format!(
                            "UPDATE __kdb_documents
                             SET data = ?,
                                 _size_bytes = length(?),
                                 _modified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                             WHERE {where_clause}
                             RETURNING json(data)"
                        ),
                        binds,
                    )
                    .await
                    .map_err(|e| AppError::Internal(format!("update replace failed: {e}")))?;
                let mut items = Vec::<Value>::new();
                while let Some(row) = rows
                    .next()
                    .await
                    .map_err(|e| AppError::Internal(format!("update replace row read failed: {e}")))?
                {
                    let raw: String = row.get(0).map_err(|e| {
                        AppError::Internal(format!("update replace row decode failed: {e}"))
                    })?;
                    items.push(serde_json::from_str::<Value>(&raw).map_err(|e| {
                        AppError::Internal(format!("update replace json decode failed: {e}"))
                    })?);
                }
                return Ok(GatewayResponse::ok(Some(json!({
                    "items": items,
                    "count": items.len(),
                    "matched_count": items.len(),
                    "updated_count": items.len()
                }))));
            }

            let (id, mut data) = extract_single_write_doc(payload.data, "data")?;
            data.as_object_mut().expect("object checked").remove("_id");
            if data.as_object().expect("object checked").is_empty() {
                return Err(AppError::BadRequest(
                    "data must include fields to update".to_string(),
                ));
            }
            if dry_run {
                let matched = count_by_ids(conn, collection.as_deref(), &[id.clone()]).await?;
                return Ok(GatewayResponse::ok(Some(json!({
                    "items": [],
                    "count": matched as usize,
                    "matched_count": matched,
                    "updated_count": matched,
                    "dry_run": true
                }))));
            }
            let strict = state.strict_mutation_operators;
            let items = if update_requires_mutation_engine(data.as_object().expect("object checked")) {
                let mut patch_obj = data.as_object().cloned().ok_or_else(|| {
                    AppError::BadRequest("data must be object".to_string())
                })?;
                let updated = update_one_with_mutation(
                    conn,
                    collection.as_deref(),
                    &id,
                    &mut patch_obj,
                    strict,
                    state.jsonb_enabled,
                )
                .await?;
                match updated {
                    Some(v) => vec![v],
                    None => vec![],
                }
            } else {
                let patch = data.to_string();
                let patch_expr = json_patch_expr(jsonb_enabled());
                let mut binds = vec![
                    libsql::Value::Text(patch.clone()),
                    libsql::Value::Text(patch),
                ];
                let where_clause = where_id_with_scope(&mut binds, collection.as_deref(), &id);
                let mut rows = conn
                    .query(
                        &format!(
                            "UPDATE __kdb_documents
                             SET data = {patch_expr},
                                 _size_bytes = length({patch_expr}),
                                 _modified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                             WHERE {where_clause}
                             RETURNING json(data)"
                        ),
                        binds,
                    )
                    .await
                    .map_err(|e| AppError::Internal(format!("update failed: {e}")))?;
                let mut out = Vec::<Value>::new();
                while let Some(row) = rows
                    .next()
                    .await
                    .map_err(|e| AppError::Internal(format!("update row read failed: {e}")))?
                {
                    let raw: String = row.get(0).map_err(|e| {
                        AppError::Internal(format!("update row decode failed: {e}"))
                    })?;
                    out.push(serde_json::from_str::<Value>(&raw).map_err(|e| {
                        AppError::Internal(format!("update json decode failed: {e}"))
                    })?);
                }
                out
            };
            Ok(GatewayResponse::ok(Some(json!({
                "items": items,
                "count": items.len(),
                "matched_count": items.len(),
                "updated_count": items.len()
            }))))
        }
        (Some(_), Some(Value::Object(_))) => {
            if replace {
                return Err(AppError::BadRequest(
                    "replace=true is only allowed for single-document update by _id".to_string(),
                ));
            }
            let collection = resolve_collection_scope(&payload)?;
            update_bulk_filter_same_patch(
                state,
                conn,
                collection.as_deref(),
                payload.filter,
                payload.data,
                payload.max_docs,
                dry_run,
            )
            .await
        }
        (None, Some(Value::Array(_))) => {
            if replace {
                return Err(AppError::BadRequest(
                    "replace=true is only allowed for single-document update by _id".to_string(),
                ));
            }
            let collection = resolve_collection_scope_optional_collection(&payload)?;
            update_bulk_data_array(state, conn, collection.as_deref(), payload.data, dry_run).await
        }
        (Some(_), Some(Value::Array(_))) => Err(AppError::BadRequest(
            "update cannot use filter when data is an array".to_string(),
        )),
        _ => Err(AppError::BadRequest("invalid update payload".to_string())),
    }
}

async fn set(
    state: &AppState,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let prepared = prepare_set_input(req.payload)?;
    let collection = prepared.collection;
    let dry_run = prepared.dry_run;
    let data = prepared.data;
    let id = prepared.id;
    let user_id = prepared.user_id;
    let data_str = prepared.data_str;
    let size = prepared.size;
    let expires_at = prepared.expires_at;
    let expiry_behavior = prepared.expiry_behavior;

    if dry_run {
        let matched = count_by_ids(conn, collection.as_deref(), std::slice::from_ref(&id)).await?;
        let would_insert = collection.is_some() && matched == 0;
        return Ok(GatewayResponse::ok(Some(json!({
            "items": [data],
            "count": 1,
            "matched_count": matched,
            "updated_count": matched,
            "inserted_count": if would_insert { 1 } else { 0 },
            "dry_run": true
        }))));
    }

    if let Some(collection) = collection {
        let updated = if update_requires_mutation_engine(data.as_object().expect("object checked")) {
            let mut patch_obj = data
                .as_object()
                .cloned()
                .ok_or_else(|| AppError::BadRequest("data must be object".to_string()))?;
            if update_one_with_mutation(
                conn,
                Some(collection.as_str()),
                &id,
                &mut patch_obj,
                state.strict_mutation_operators,
                state.jsonb_enabled,
            )
            .await?
            .is_some()
            {
                1
            } else {
                0
            }
        } else {
            let patch_expr = json_patch_expr(state.jsonb_enabled);
            conn.execute(
                &format!(
                    "UPDATE __kdb_documents
                     SET data = {patch_expr},
                         _user_id = COALESCE(?, _user_id),
                         _size_bytes = length({patch_expr}),
                         _modified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     WHERE collection = ? AND id = ?"
                ),
                libsql::params![
                    data_str.clone(),
                    to_sql_nullable_text(user_id.clone()),
                    data_str.clone(),
                    collection.clone(),
                    id.clone()
                ],
            )
            .await
            .map_err(|e| AppError::Internal(format!("set update failed: {e}")))?
        };

        if updated > 0 {
            let items = fetch_kdb_documents_by_ids(
                conn,
                Some(collection.as_str()),
                std::slice::from_ref(&id),
                state.response_include_system_timestamps,
            )
            .await?;
            return Ok(GatewayResponse::ok(Some(json!({
                "items": items,
                "count": items.len(),
                "matched_count": items.len(),
                "updated_count": items.len(),
                "inserted_count": 0
            }))));
        }

        let data_expr = json_input_expr(state.jsonb_enabled);
        conn.execute(
            &format!(
                "INSERT INTO __kdb_documents (id, collection, _user_id, data, _size_bytes, _expires_at, _expiry_behavior)
                 VALUES (?, ?, ?, {data_expr}, ?, ?, ?)"
            ),
            libsql::params![
                id.clone(),
                collection.clone(),
                to_sql_nullable_text(user_id.clone()),
                data_str,
                size,
                expires_at,
                expiry_behavior
            ],
        )
        .await
        .map_err(|e| AppError::Conflict(format!("set insert failed: {e}")))?;

        let items = fetch_kdb_documents_by_ids(
            conn,
            Some(collection.as_str()),
            std::slice::from_ref(&id),
            state.response_include_system_timestamps,
        )
        .await?;
        return Ok(GatewayResponse::ok(Some(json!({
            "items": items,
            "count": 1,
            "matched_count": 0,
            "updated_count": 0,
            "inserted_count": 1
        }))));
    }

    let updated = if update_requires_mutation_engine(data.as_object().expect("object checked")) {
        let mut patch_obj = data
            .as_object()
            .cloned()
            .ok_or_else(|| AppError::BadRequest("data must be object".to_string()))?;
        if update_one_with_mutation(
            conn,
            None,
            &id,
            &mut patch_obj,
            state.strict_mutation_operators,
            state.jsonb_enabled,
        )
        .await?
        .is_some()
        {
            1
        } else {
            0
        }
    } else {
        let patch_expr = json_patch_expr(state.jsonb_enabled);
        conn.execute(
            &format!(
                "UPDATE __kdb_documents
                 SET data = {patch_expr},
                     _user_id = COALESCE(?, _user_id),
                     _size_bytes = length({patch_expr}),
                     _modified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE id = ?"
            ),
            libsql::params![
                data_str.clone(),
                to_sql_nullable_text(user_id.clone()),
                data_str,
                id.clone()
            ],
        )
        .await
        .map_err(|e| AppError::Internal(format!("set update failed: {e}")))?
    };
    if updated == 0 {
        return Err(AppError::NotFound(
            "document not found for set without collection".to_string(),
        ));
    }
    let items = fetch_kdb_documents_by_ids(
        conn,
        None,
        std::slice::from_ref(&id),
        state.response_include_system_timestamps,
    )
    .await?;
    Ok(GatewayResponse::ok(Some(json!({
        "items": items,
        "count": 1,
        "matched_count": 1,
        "updated_count": 1,
        "inserted_count": 0
    }))))
}


// Additional write handlers extracted from dispatcher.rs.

async fn upsert(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let insert_expires_at = ttl_to_expires_at(payload.ttl_seconds)?;
    let insert_expiry_behavior = normalized_expiry_behavior(payload.expiry_behavior.as_deref());
    let prepared = prepare_upsert_inputs(payload)?;
    let collection = prepared.collection;
    let dry_run = prepared.dry_run;
    let max_docs = prepared.max_docs;
    let user_id = prepared.user_id;
    let mut insert_data = prepared.insert_data;
    let update_data = prepared.update_data;
    let filter = prepared.filter;

    let (where_clause, binds) = build_where_with_collection(filter, collection.clone())?;
    let matched_count = execute_count(conn, "__kdb_documents", &where_clause, binds.clone()).await?;

    if matched_count > 0 {
        let effective = normalize_max_docs(Some(max_docs), matched_count)?;

        if dry_run || effective == 0 {
            return Ok(GatewayResponse::ok(Some(json!({
                "items": [],
                "count": 0,
                "matched_count": matched_count,
                "updated_count": effective,
                "inserted_count": 0,
                "dry_run": dry_run
            }))));
        }

        let mut items = Vec::<Value>::new();
        if update_requires_mutation_engine(
            update_data
                .as_object()
                .ok_or_else(|| AppError::BadRequest("update_data must be object".to_string()))?,
        ) {
            let rowids =
                select_rowids_for_update(conn, &where_clause, binds, effective as i64).await?;
            for rowid in rowids {
                let mut patch_obj = update_data
                    .as_object()
                    .cloned()
                    .ok_or_else(|| AppError::BadRequest("update_data must be object".to_string()))?;
                if let Some(v) = update_one_by_rowid_with_mutation(
                    conn,
                    rowid,
                    &mut patch_obj,
                    state.strict_mutation_operators,
                    state.jsonb_enabled,
                )
                .await?
                {
                    items.push(v);
                }
            }
        } else {
            let patch = update_data.to_string();
            let patch_expr = json_patch_expr(state.jsonb_enabled);
            let mut qbinds = vec![
                libsql::Value::Text(patch.clone()),
                libsql::Value::Text(patch),
            ];
            qbinds.extend(binds);
            qbinds.push(libsql::Value::Integer(effective as i64));

            let mut rows = conn
                .query(
                    &format!(
                        "UPDATE __kdb_documents
                         SET data = {patch_expr},
                             _size_bytes = length({patch_expr}),
                             _modified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                         WHERE rowid IN (
                            SELECT rowid FROM __kdb_documents WHERE {where_clause} ORDER BY rowid LIMIT ?
                         )
                         RETURNING json(data)"
                    ),
                    qbinds,
                )
                .await
                .map_err(|e| AppError::Internal(format!("upsert update failed: {e}")))?;

            while let Some(row) = rows
                .next()
                .await
                .map_err(|e| AppError::Internal(format!("upsert row read failed: {e}")))?
            {
                let raw: String = row
                    .get(0)
                    .map_err(|e| AppError::Internal(format!("upsert row decode failed: {e}")))?;
                items.push(serde_json::from_str::<Value>(&raw).map_err(|e| {
                    AppError::Internal(format!("upsert json decode failed: {e}"))
                })?);
            }
        }

        return Ok(GatewayResponse::ok(Some(json!({
            "items": items,
            "count": items.len(),
            "matched_count": matched_count,
            "updated_count": items.len(),
            "inserted_count": 0
        }))));
    }

    let id = Uuid::new_v4().simple().to_string();
    insert_data
        .as_object_mut()
        .expect("object checked")
        .insert("_id".to_string(), json!(id));

    if dry_run {
        return Ok(GatewayResponse::ok(Some(json!({
            "items": [insert_data],
            "count": 1,
            "matched_count": 0,
            "updated_count": 0,
            "inserted_count": 1,
            "dry_run": true
        }))));
    }

    let data = insert_data.to_string();
    let size = data.len() as i64;
    let new_id = insert_data
        .get("_id")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Internal("generated _id missing".to_string()))?;

    let insert_collection = collection.clone().ok_or_else(|| {
        AppError::BadRequest("collection is required for upsert insert path".to_string())
    })?;

    let data_expr = json_input_expr(state.jsonb_enabled);
    conn.execute(
        &format!(
            "INSERT INTO __kdb_documents (id, collection, _user_id, data, _size_bytes, _expires_at, _expiry_behavior) VALUES (?, ?, ?, {data_expr}, ?, ?, ?)"
        ),
        libsql::params![
            new_id.to_string(),
            insert_collection.clone(),
            to_sql_nullable_text(user_id),
            data,
            size,
            insert_expires_at,
            insert_expiry_behavior
        ],
    )
    .await
    .map_err(|e| AppError::Conflict(format!("upsert insert failed: {e}")))?;

    state
        .db_manager
        .append_wal_record(
            db_path,
            "UPSERT_INSERT",
            &json!({"collection": insert_collection, "id": new_id}).to_string(),
        )
        .await?;

    let new_id_owned = new_id.to_string();
    let items = fetch_kdb_documents_by_ids(
        conn,
        Some(insert_collection.as_str()),
        std::slice::from_ref(&new_id_owned),
        state.response_include_system_timestamps,
    )
    .await?;

    Ok(GatewayResponse::ok(Some(json!({
        "items": items,
        "count": 1,
        "matched_count": 0,
        "updated_count": 0,
        "inserted_count": 1
    }))))
}
