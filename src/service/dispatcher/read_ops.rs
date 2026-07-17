// Read operation handlers extracted from dispatcher.rs.

async fn count(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let observed_paths = observed_query_paths(&payload);
    if !observed_paths.is_empty() {
        let _ = bump_query_heatmap(conn, &observed_paths).await;
    }
    let cache_meta = cache_key_for_read(state, db_path, "count", &payload)?;
    if let Some(meta) = cache_meta.as_ref() {
        if let Some(cached) = get_cached_read(state, meta).await {
            return Ok(GatewayResponse::ok(Some(cached)));
        }
    }
    let source = resolve_source(&payload);
    let collection = resolve_collection_scope(&payload)?;
    let (mut where_clause, mut bind_values) =
        build_where_with_collection(payload.filter.unwrap_or_else(|| json!({})), collection)?;
    apply_document_user_scope(
        &mut where_clause,
        &mut bind_values,
        clean_optional(payload.document_user_id.clone()),
    );

    let sql = format!("SELECT COUNT(*) FROM {source} WHERE {where_clause}");
    let mut rows = conn
        .query(&sql, bind_values)
        .await
        .map_err(|e| AppError::Internal(format!("count query failed: {e}")))?;

    if let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("count row read failed: {e}")))?
    {
        let total: i64 = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("count row decode failed: {e}")))?;
        let data = json!({ "count": total });
        if let Some(meta) = cache_meta {
            put_cached_read(state, &meta, data.clone()).await;
        }
        return Ok(GatewayResponse::ok(Some(data)));
    }
    let data = json!({ "count": 0 });
    if let Some(meta) = cache_meta {
        put_cached_read(state, &meta, data.clone()).await;
    }
    Ok(GatewayResponse::ok(Some(data)))
}

async fn sql_execute(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let raw_sql = payload
        .sql
        .as_ref()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| AppError::BadRequest("sql is required".to_string()))?;
    let bind_values = json_params_to_sql_values(payload.params.clone())?;
    let (sql, mode) = classify_sql_execute(&raw_sql)?;

    if matches!(mode, SqlExecuteMode::Read) {
        let cache_meta = cache_key_for_read(state, db_path, "sql_execute", &payload)?;
        if let Some(meta) = cache_meta.as_ref() {
            if let Some(cached) = get_cached_read(state, meta).await {
                return Ok(GatewayResponse::ok(Some(cached)));
            }
        }

        let mut rows = conn
            .query(&sql, bind_values)
            .await
            .map_err(|e| AppError::Internal(format!("sql_execute query failed: {e}")))?;

        let mut columns = Vec::<String>::new();
        for idx in 0..rows.column_count() {
            columns.push(
                rows.column_name(idx)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| format!("col_{idx}")),
            );
        }

        let mut items = Vec::<Value>::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("sql_execute row read failed: {e}")))?
        {
            let mut obj = serde_json::Map::new();
            for idx in 0..row.column_count() {
                let name = row
                    .column_name(idx)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| format!("col_{idx}"));
                let raw = row.get_value(idx).map_err(|e| {
                    AppError::Internal(format!("sql_execute cell read failed: {e}"))
                })?;
                obj.insert(name, libsql_value_to_json(raw));
            }
            items.push(Value::Object(obj));
        }

        let data = json!({
            "count": items.len(),
            "items": items,
            "columns": columns
        });
        if let Some(meta) = cache_meta {
            put_cached_read(state, &meta, data.clone()).await;
        }
        return Ok(GatewayResponse::ok(Some(data)));
    }

    let rows_affected = conn
        .execute(&sql, bind_values)
        .await
        .map_err(|e| AppError::Internal(format!("sql_execute execute failed: {e}")))?;
    let mut last_insert_rowid = Value::Null;
    if sql
        .split_whitespace()
        .next()
        .map(|v| v.eq_ignore_ascii_case("insert") || v.eq_ignore_ascii_case("replace"))
        .unwrap_or(false)
    {
        let mut rows = conn
            .query("SELECT last_insert_rowid()", ())
            .await
            .map_err(|e| AppError::Internal(format!("sql_execute rowid query failed: {e}")))?;
        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("sql_execute rowid read failed: {e}")))?
        {
            let value = row
                .get_value(0)
                .map_err(|e| AppError::Internal(format!("sql_execute rowid decode failed: {e}")))?;
            last_insert_rowid = libsql_value_to_json(value);
        }
    }
    Ok(GatewayResponse::ok(Some(json!({
        "rows_affected": rows_affected,
        "last_insert_rowid": last_insert_rowid
    }))))
}

async fn aggregate(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let cache_meta = cache_key_for_read(state, db_path, "aggregate", &payload)?;
    if let Some(meta) = cache_meta.as_ref() {
        if let Some(cached) = get_cached_read(state, meta).await {
            return Ok(GatewayResponse::ok(Some(cached)));
        }
    }

    let source = resolve_source(&payload);
    let collection = resolve_collection_scope(&payload)?;
    let (where_clause, bind_values) = build_where_with_collection(
        payload.filter.clone().unwrap_or_else(|| json!({})),
        collection.clone(),
    )?;
    let compute_spec = payload
        .compute
        .clone()
        .ok_or_else(|| AppError::BadRequest("compute is required".to_string()))?;

    if payload.group_by.is_some() {
        return Err(AppError::BadRequest(
            "group_by is not implemented yet".to_string(),
        ));
    }

    let compute = run_aggregations_sql(
        conn,
        source,
        compute_spec,
        payload.filter.clone(),
        collection.clone(),
    )
    .await?;
    let matched_count = execute_count(conn, source, &where_clause, bind_values).await?;

    let data = json!({
        "matched_count": matched_count,
        "compute": compute
    });
    if let Some(meta) = cache_meta {
        put_cached_read(state, &meta, data.clone()).await;
    }
    Ok(GatewayResponse::ok(Some(data)))
}

async fn query(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let namespace_scope = resolve_read_namespace_scope(&payload, true)?;
    let include_namespace = payload
        .include_namespace
        .unwrap_or(state.response_include_namespace)
        || namespace_scope_force_include(&namespace_scope);
    let observed_paths = observed_query_paths(&payload);
    if !observed_paths.is_empty() {
        let _ = bump_query_heatmap(conn, &observed_paths).await;
    }
    let cache_meta = cache_key_for_read(state, db_path, "query", &payload)?;
    if let Some(meta) = cache_meta.as_ref() {
        if let Some(cached) = get_cached_read(state, meta).await {
            return Ok(GatewayResponse::ok(Some(cached)));
        }
    }
    let source = resolve_source(&payload);
    let (mut where_clause, mut bind_values) = build_where_with_namespace_scope(
        payload.filter.clone().unwrap_or_else(|| json!({})),
        &namespace_scope,
    )?;
    apply_document_user_scope(
        &mut where_clause,
        &mut bind_values,
        clean_optional(payload.document_user_id.clone()),
    );

    if payload.explain.unwrap_or(false) {
        return Ok(GatewayResponse::ok(Some(json!({
            "where_sql": where_clause,
            "bind_count": bind_values.len(),
            "source": source
        }))));
    }

    let total_count = execute_count(conn, source, &where_clause, bind_values.clone()).await?;

    let (limit, offset, page) = resolve_pagination_args(&payload, state.query_default_limit)?;
    let order_by = build_order_by(&payload.sort)?;

    let mut data_binds = bind_values.clone();
    data_binds.push(libsql::Value::Integer(limit));
    data_binds.push(libsql::Value::Integer(offset));

    let sql = match (include_namespace, state.response_include_system_timestamps) {
        (true, true) => format!(
            "SELECT json(data), _user_id, collection, _created_at, _modified_at FROM {source} WHERE {where_clause} ORDER BY {order_by} LIMIT ? OFFSET ?"
        ),
        (true, false) => format!(
            "SELECT json(data), _user_id, collection FROM {source} WHERE {where_clause} ORDER BY {order_by} LIMIT ? OFFSET ?"
        ),
        (false, true) => format!(
            "SELECT json(data), _user_id, _created_at, _modified_at FROM {source} WHERE {where_clause} ORDER BY {order_by} LIMIT ? OFFSET ?"
        ),
        (false, false) => format!(
            "SELECT json(data), _user_id FROM {source} WHERE {where_clause} ORDER BY {order_by} LIMIT ? OFFSET ?"
        ),
    };

    let mut rows = conn
        .query(&sql, data_binds)
        .await
        .map_err(|e| AppError::Internal(format!("query failed: {e}")))?;

    let mut out = Vec::<Value>::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("row read failed: {e}")))?
    {
        let raw: String = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("row decode failed: {e}")))?;
        let mut value = serde_json::from_str::<Value>(&raw)
            .map_err(|e| AppError::Internal(format!("json decode failed: {e}")))?;
        let user_id: Option<String> = row
            .get(1)
            .map_err(|e| AppError::Internal(format!("query _user_id decode failed: {e}")))?;
        attach_document_user_id(&mut value, user_id);
        let mut idx = 2;
        if include_namespace {
            let ns: String = row
                .get(idx)
                .map_err(|e| AppError::Internal(format!("query namespace decode failed: {e}")))?;
            if let Some(obj) = value.as_object_mut() {
                obj.insert("_namespace".to_string(), Value::String(ns));
            }
            idx += 1;
        }
        if state.response_include_system_timestamps {
            let created_at: Option<String> = row
                .get(idx)
                .map_err(|e| AppError::Internal(format!("query created_at decode failed: {e}")))?;
            let modified_at: Option<String> = row
                .get(idx + 1)
                .map_err(|e| AppError::Internal(format!("query modified_at decode failed: {e}")))?;
            attach_system_timestamps(&mut value, created_at, modified_at);
        }
        out.push(value);
    }

    if let Some(lookups) = payload.lookups.as_ref() {
        let max_depth = resolve_lookup_max_depth(state, payload.lookup_depth_override)?;
        let mut roots = out.clone();
        execute_lookup_scope(
            state, conn, &mut out, &mut roots, None, lookups, 1, max_depth,
        )
        .await?;
        roots.clear();
    }

    if let Some(spec) = payload.compute.as_ref() {
        apply_row_compute_to_items(&mut out, spec)?;
    }

    if payload.fields.is_some() || payload.exclude_fields.is_some() {
        let mut projected = Vec::<Value>::with_capacity(out.len());
        for item in &out {
            projected.push(apply_projection(
                item,
                &payload.fields,
                &payload.exclude_fields,
            )?);
        }
        out = projected;
    }

    let pagination = build_pagination(total_count, out.len(), limit, page, offset);
    let (next_offset, prev_offset) = build_offsets(total_count, out.len(), limit, offset);
    let mut data = json!({
        "count": out.len(),
        "total_items": total_count,
        "items": out,
        "limit": limit,
        "offset": offset,
        "next_offset": next_offset,
        "prev_offset": prev_offset,
        "pagination": pagination
    });
    attach_users_if_requested(conn, &mut data, &payload).await?;
    if let Some(meta) = cache_meta {
        put_cached_read(state, &meta, data.clone()).await;
    }
    Ok(GatewayResponse::ok(Some(data)))
}

async fn search(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    if !get_bool_config(conn, "fts_enabled").await?.unwrap_or(true) {
        return Err(AppError::BadRequest(
            "search is disable: db_config.fts_enabled=false".to_string(),
        ));
    }
    let payload = req.payload;
    let namespace_scope = resolve_read_namespace_scope(&payload, true)?;
    let include_namespace = payload
        .include_namespace
        .unwrap_or(state.response_include_namespace)
        || namespace_scope_force_include(&namespace_scope);
    if payload.include_archive.unwrap_or(false) || payload.archive_only.unwrap_or(false) {
        return Err(AppError::BadRequest(
            "search currently supports live documents only".to_string(),
        ));
    }

    let query_text = payload
        .search
        .clone()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest("search query is required".to_string()))?;

    let observed_paths = observed_query_paths(&payload);
    if !observed_paths.is_empty() {
        let _ = bump_query_heatmap(conn, &observed_paths).await;
    }
    let cache_meta = cache_key_for_read(state, db_path, "search", &payload)?;
    if let Some(meta) = cache_meta.as_ref() {
        if let Some(cached) = get_cached_read(state, meta).await {
            return Ok(GatewayResponse::ok(Some(cached)));
        }
    }

    let (mut where_clause, mut where_binds) = build_where_with_namespace_scope(
        payload.filter.clone().unwrap_or_else(|| json!({})),
        &namespace_scope,
    )?;
    apply_document_user_scope(
        &mut where_clause,
        &mut where_binds,
        clean_optional(payload.document_user_id.clone()),
    );

    let mut count_binds = Vec::<libsql::Value>::with_capacity(1 + where_binds.len());
    count_binds.push(libsql::Value::Text(query_text.clone()));
    count_binds.extend(where_binds.clone());

    let count_sql = format!(
        "SELECT COUNT(*)
         FROM __kdb_documents_fts
         JOIN __kdb_documents d ON d.id = __kdb_documents_fts.id
         WHERE __kdb_documents_fts.content MATCH ? AND {where_clause}"
    );
    let mut count_rows = conn
        .query(&count_sql, count_binds)
        .await
        .map_err(|e| AppError::Internal(format!("search count query failed: {e}")))?;
    let total_count = if let Some(row) = count_rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("search count row read failed: {e}")))?
    {
        row.get(0)
            .map_err(|e| AppError::Internal(format!("search count row decode failed: {e}")))?
    } else {
        0
    };

    let (limit, offset, page) = resolve_pagination_args(&payload, state.query_default_limit)?;
    let order_by = if payload.sort.is_some() {
        build_order_by(&payload.sort)?
    } else {
        "_score ASC, _created_at DESC".to_string()
    };

    let mut data_binds = Vec::<libsql::Value>::with_capacity(3 + where_binds.len());
    data_binds.push(libsql::Value::Text(query_text));
    data_binds.extend(where_binds);
    data_binds.push(libsql::Value::Integer(limit));
    data_binds.push(libsql::Value::Integer(offset));

    let sql = if state.response_include_system_timestamps && include_namespace {
        format!(
            "WITH matched AS (
                SELECT json(d.data) AS data,
                       d._user_id AS _user_id,
                       d.collection AS _namespace,
                       d._created_at AS _created_at,
                       d._modified_at AS _modified_at,
                       bm25(__kdb_documents_fts) AS _score
                FROM __kdb_documents_fts
                JOIN __kdb_documents d ON d.id = __kdb_documents_fts.id
                WHERE __kdb_documents_fts.content MATCH ? AND {where_clause}
            )
            SELECT data, _user_id, _namespace, _created_at, _modified_at, _score
            FROM matched
            ORDER BY {order_by}
            LIMIT ? OFFSET ?"
        )
    } else if state.response_include_system_timestamps {
        format!(
            "WITH matched AS (
                SELECT json(d.data) AS data,
                       d._user_id AS _user_id,
                       d._created_at AS _created_at,
                       d._modified_at AS _modified_at,
                       bm25(__kdb_documents_fts) AS _score
                FROM __kdb_documents_fts
                JOIN __kdb_documents d ON d.id = __kdb_documents_fts.id
                WHERE __kdb_documents_fts.content MATCH ? AND {where_clause}
            )
            SELECT data, _user_id, _created_at, _modified_at, _score
            FROM matched
            ORDER BY {order_by}
            LIMIT ? OFFSET ?"
        )
    } else if include_namespace {
        format!(
            "WITH matched AS (
                SELECT json(d.data) AS data,
                       d._user_id AS _user_id,
                       d.collection AS _namespace,
                       bm25(__kdb_documents_fts) AS _score
                FROM __kdb_documents_fts
                JOIN __kdb_documents d ON d.id = __kdb_documents_fts.id
                WHERE __kdb_documents_fts.content MATCH ? AND {where_clause}
            )
            SELECT data, _user_id, _namespace, _score
            FROM matched
            ORDER BY {order_by}
            LIMIT ? OFFSET ?"
        )
    } else {
        format!(
            "WITH matched AS (
                SELECT json(d.data) AS data,
                       d._user_id AS _user_id,
                       bm25(__kdb_documents_fts) AS _score
                FROM __kdb_documents_fts
                JOIN __kdb_documents d ON d.id = __kdb_documents_fts.id
                WHERE __kdb_documents_fts.content MATCH ? AND {where_clause}
            )
            SELECT data, _user_id, _score
            FROM matched
            ORDER BY {order_by}
            LIMIT ? OFFSET ?"
        )
    };

    let mut rows = conn
        .query(&sql, data_binds)
        .await
        .map_err(|e| AppError::Internal(format!("search query failed: {e}")))?;

    let mut out = Vec::<Value>::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("search row read failed: {e}")))?
    {
        let raw: String = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("search row decode failed: {e}")))?;
        let mut value = serde_json::from_str::<Value>(&raw)
            .map_err(|e| AppError::Internal(format!("search json decode failed: {e}")))?;
        let user_id: Option<String> = row
            .get(1)
            .map_err(|e| AppError::Internal(format!("search _user_id decode failed: {e}")))?;
        attach_document_user_id(&mut value, user_id);
        let mut idx = 2;
        if include_namespace {
            let ns: String = row
                .get(idx)
                .map_err(|e| AppError::Internal(format!("search namespace decode failed: {e}")))?;
            if let Some(obj) = value.as_object_mut() {
                obj.insert("_namespace".to_string(), Value::String(ns));
            }
            idx += 1;
        }
        if state.response_include_system_timestamps {
            let created_at: Option<String> = row
                .get(idx)
                .map_err(|e| AppError::Internal(format!("search created_at decode failed: {e}")))?;
            let modified_at: Option<String> = row.get(idx + 1).map_err(|e| {
                AppError::Internal(format!("search modified_at decode failed: {e}"))
            })?;
            attach_system_timestamps(&mut value, created_at, modified_at);
        }
        let score_idx = if state.response_include_system_timestamps {
            idx + 2
        } else {
            idx
        };
        let score: f64 = row
            .get(score_idx)
            .map_err(|e| AppError::Internal(format!("search score decode failed: {e}")))?;
        if let Some(obj) = value.as_object_mut() {
            obj.insert("_search_score".to_string(), json!(score));
        }
        out.push(value);
    }

    if let Some(lookups) = payload.lookups.as_ref() {
        let max_depth = resolve_lookup_max_depth(state, payload.lookup_depth_override)?;
        let mut roots = out.clone();
        execute_lookup_scope(
            state, conn, &mut out, &mut roots, None, lookups, 1, max_depth,
        )
        .await?;
        roots.clear();
    }

    if payload.fields.is_some() || payload.exclude_fields.is_some() {
        let mut projected = Vec::<Value>::with_capacity(out.len());
        for item in &out {
            projected.push(apply_projection(
                item,
                &payload.fields,
                &payload.exclude_fields,
            )?);
        }
        out = projected;
    }

    let pagination = build_pagination(total_count, out.len(), limit, page, offset);
    let (next_offset, prev_offset) = build_offsets(total_count, out.len(), limit, offset);
    let mut data = json!({
        "count": out.len(),
        "total_items": total_count,
        "items": out,
        "limit": limit,
        "offset": offset,
        "next_offset": next_offset,
        "prev_offset": prev_offset,
        "pagination": pagination
    });
    attach_users_if_requested(conn, &mut data, &payload).await?;
    if let Some(meta) = cache_meta {
        put_cached_read(state, &meta, data.clone()).await;
    }
    Ok(GatewayResponse::ok(Some(data)))
}


// Additional read handlers extracted from dispatcher.rs.

async fn get(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let namespace_scope = resolve_read_namespace_scope(&payload, false)?;
    let include_namespace = payload
        .include_namespace
        .unwrap_or(state.response_include_namespace)
        || namespace_scope_force_include(&namespace_scope);
    let ids = extract_ids_or_single_strict(&payload)?;
    let source = resolve_source(&payload);
    let allow_pending = !payload.force_db.unwrap_or(false) && source == "__kdb_documents";
    let mut items = Vec::<Value>::new();
    let mut missing_ids = Vec::<String>::new();
    if allow_pending {
        for id in &ids {
            if let Some(pending) = state.get_pending_document(db_path, id) {
                if pending_matches_read_scope(&pending.collection, &namespace_scope) {
                    let mut item = pending.document.clone();
                    if include_namespace {
                        if let (Some(obj), Some(ns)) =
                            (item.as_object_mut(), pending.collection.as_ref())
                        {
                            obj.insert("_namespace".to_string(), Value::String(ns.clone()));
                        }
                    }
                    items.push(item);
                    continue;
                }
            }
            missing_ids.push(id.clone());
        }
    } else {
        missing_ids = ids.clone();
    }

    let cache_meta = if allow_pending || payload.force_db.unwrap_or(false) {
        None
    } else {
        cache_key_for_read(state, db_path, "get", &payload)?
    };
    if let Some(meta) = cache_meta.as_ref() {
        if let Some(cached) = get_cached_read(state, meta).await {
            return Ok(GatewayResponse::ok(Some(cached)));
        }
    }
    if !missing_ids.is_empty() {
        let placeholders = vec!["?"; missing_ids.len()].join(", ");
        let mut binds = Vec::<libsql::Value>::new();
        let mut where_clause = where_ids_with_namespace_scope(
            &mut binds,
            &namespace_scope,
            &missing_ids,
            &placeholders,
        );
        apply_document_user_scope(
            &mut where_clause,
            &mut binds,
            clean_optional(payload.document_user_id.clone()),
        );

        let mut rows = conn
            .query(
                &if state.response_include_system_timestamps && include_namespace {
                    format!(
                        "SELECT json(data), _user_id, collection, _created_at, _modified_at FROM {source} WHERE {where_clause} ORDER BY _created_at DESC"
                    )
                } else if state.response_include_system_timestamps {
                    format!(
                        "SELECT json(data), _user_id, _created_at, _modified_at FROM {source} WHERE {where_clause} ORDER BY _created_at DESC"
                    )
                } else if include_namespace {
                    format!("SELECT json(data), _user_id, collection FROM {source} WHERE {where_clause} ORDER BY _created_at DESC")
                } else {
                    format!("SELECT json(data), _user_id FROM {source} WHERE {where_clause} ORDER BY _created_at DESC")
                },
                binds,
            )
            .await
            .map_err(|e| AppError::Internal(format!("get query failed: {e}")))?;

        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("get row read failed: {e}")))?
        {
            let raw: String = row
                .get(0)
                .map_err(|e| AppError::Internal(format!("get row decode failed: {e}")))?;
            let mut item = serde_json::from_str::<Value>(&raw)
                .map_err(|e| AppError::Internal(format!("get json decode failed: {e}")))?;
            let user_id: Option<String> = row
                .get(1)
                .map_err(|e| AppError::Internal(format!("get _user_id decode failed: {e}")))?;
            attach_document_user_id(&mut item, user_id);
            let mut idx = 2;
            if include_namespace {
                let ns: String = row
                    .get(idx)
                    .map_err(|e| AppError::Internal(format!("get namespace decode failed: {e}")))?;
                if let Some(obj) = item.as_object_mut() {
                    obj.insert("_namespace".to_string(), Value::String(ns));
                }
                idx += 1;
            }
            if state.response_include_system_timestamps {
                let created_at: Option<String> = row.get(idx).map_err(|e| {
                    AppError::Internal(format!("get created_at decode failed: {e}"))
                })?;
                let modified_at: Option<String> = row.get(idx + 1).map_err(|e| {
                    AppError::Internal(format!("get modified_at decode failed: {e}"))
                })?;
                attach_system_timestamps(&mut item, created_at, modified_at);
            }
            items.push(item);
        }
    }

    if let Some(lookups) = payload.lookups.as_ref() {
        let max_depth = resolve_lookup_max_depth(state, payload.lookup_depth_override)?;
        let mut roots = items.clone();
        execute_lookup_scope(
            state, conn, &mut items, &mut roots, None, lookups, 1, max_depth,
        )
        .await?;
        roots.clear();
    }

    if payload.fields.is_some() || payload.exclude_fields.is_some() {
        let mut projected = Vec::<Value>::with_capacity(items.len());
        for item in &items {
            projected.push(apply_projection(
                item,
                &payload.fields,
                &payload.exclude_fields,
            )?);
        }
        items = projected;
    }
    let mut data = json!({ "items": items, "count": items.len() });
    attach_users_if_requested(conn, &mut data, &payload).await?;
    if let Some(meta) = cache_meta {
        put_cached_read(state, &meta, data.clone()).await;
    }
    Ok(GatewayResponse::ok(Some(data)))
}
