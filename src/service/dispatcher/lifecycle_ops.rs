// Durable document lifecycle scheduling, inspection, and serialized execution.

#[derive(Clone, Debug)]
struct DocumentLifecycleSpec {
    transition_id: String,
    name: String,
    execute_at: String,
    condition: Value,
    update: Value,
    ttl_seconds: Option<i64>,
    expiry_behavior: Option<String>,
}

fn parse_document_lifecycle(value: Option<Value>) -> AppResult<Vec<DocumentLifecycleSpec>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = match value {
        Value::Object(_) => vec![value],
        Value::Array(values) if !values.is_empty() => values,
        Value::Array(_) => {
            return Err(AppError::BadRequest(
                "lifecycle cannot be an empty array".to_string(),
            ));
        }
        _ => {
            return Err(AppError::BadRequest(
                "lifecycle must be an object or array of objects".to_string(),
            ));
        }
    };

    let mut names = HashSet::<String>::new();
    let mut specs = Vec::with_capacity(values.len());
    for value in values {
        let obj = value.as_object().ok_or_else(|| {
            AppError::BadRequest("every lifecycle item must be an object".to_string())
        })?;
        let name = obj
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AppError::BadRequest("lifecycle.name is required".to_string()))?
            .to_string();
        if !names.insert(name.clone()) {
            return Err(AppError::BadRequest(format!(
                "lifecycle name is duplicated: {name}"
            )));
        }

        let at = obj.get("at");
        let after_seconds = obj.get("after_seconds");
        if at.is_some() == after_seconds.is_some() {
            return Err(AppError::BadRequest(
                "lifecycle requires exactly one of at or after_seconds".to_string(),
            ));
        }
        let execute_at = if let Some(at) = at {
            let raw = at.as_str().ok_or_else(|| {
                AppError::BadRequest("lifecycle.at must be an RFC3339 datetime".to_string())
            })?;
            chrono::DateTime::parse_from_rfc3339(raw)
                .map_err(|_| {
                    AppError::BadRequest("lifecycle.at must be an RFC3339 datetime".to_string())
                })?
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Millis, true)
        } else {
            let seconds = after_seconds.and_then(Value::as_i64).ok_or_else(|| {
                AppError::BadRequest("lifecycle.after_seconds must be a positive integer".to_string())
            })?;
            if seconds <= 0 {
                return Err(AppError::BadRequest(
                    "lifecycle.after_seconds must be a positive integer".to_string(),
                ));
            }
            let delay = Duration::try_seconds(seconds).ok_or_else(|| {
                AppError::BadRequest(
                    "lifecycle.after_seconds produces an out-of-range datetime".to_string(),
                )
            })?;
            Utc::now()
                .checked_add_signed(delay)
                .ok_or_else(|| {
                    AppError::BadRequest(
                        "lifecycle.after_seconds produces an out-of-range datetime".to_string(),
                    )
                })?
                .to_rfc3339_opts(SecondsFormat::Millis, true)
        };

        let condition = obj.get("when").cloned().ok_or_else(|| {
            AppError::BadRequest("lifecycle.when is required".to_string())
        })?;
        if !condition.is_object() {
            return Err(AppError::BadRequest(
                "lifecycle.when must be a Filter Operators object".to_string(),
            ));
        }
        build_where(&condition)?;

        let update = obj.get("update").cloned().ok_or_else(|| {
            AppError::BadRequest("lifecycle.update is required".to_string())
        })?;
        let update_obj = update.as_object().ok_or_else(|| {
            AppError::BadRequest("lifecycle.update must be a non-empty object".to_string())
        })?;
        if update_obj.is_empty() {
            return Err(AppError::BadRequest(
                "lifecycle.update must be a non-empty object".to_string(),
            ));
        }
        if update_obj
            .keys()
            .any(|key| key == "_id" || key.starts_with("_id."))
        {
            return Err(AppError::BadRequest(
                "lifecycle.update cannot change _id".to_string(),
            ));
        }

        let ttl_seconds = obj.get("ttl_seconds").map(|value| {
            value.as_i64().ok_or_else(|| {
                AppError::BadRequest("lifecycle.ttl_seconds must be an integer".to_string())
            })
        }).transpose()?;
        if ttl_seconds.is_some_and(|value| value < 0) {
            return Err(AppError::BadRequest(
                "lifecycle.ttl_seconds must be 0 or positive".to_string(),
            ));
        }
        let expiry_behavior = obj
            .get("expiry_behavior")
            .map(|value| {
                value.as_str().ok_or_else(|| {
                    AppError::BadRequest(
                        "lifecycle.expiry_behavior must be archive or delete".to_string(),
                    )
                })
            })
            .transpose()?
            .map(str::to_ascii_lowercase);
        if expiry_behavior
            .as_deref()
            .is_some_and(|value| !matches!(value, "archive" | "delete"))
        {
            return Err(AppError::BadRequest(
                "lifecycle.expiry_behavior must be archive or delete".to_string(),
            ));
        }

        specs.push(DocumentLifecycleSpec {
            transition_id: Uuid::new_v4().simple().to_string(),
            name,
            execute_at,
            condition,
            update,
            ttl_seconds,
            expiry_behavior,
        });
    }
    Ok(specs)
}

fn lifecycle_value_from_payload(payload: &OperationPayload) -> Value {
    let mut value = serde_json::Map::new();
    if let Some(field) = payload.name.clone() {
        value.insert("name".to_string(), Value::String(field));
    }
    if let Some(field) = payload.at.clone() {
        value.insert("at".to_string(), Value::String(field));
    }
    if let Some(field) = payload.after_seconds {
        value.insert("after_seconds".to_string(), Value::from(field));
    }
    if let Some(field) = payload.when.clone() {
        value.insert("when".to_string(), field);
    }
    if let Some(field) = payload.update.clone() {
        value.insert("update".to_string(), field);
    }
    if let Some(field) = payload.ttl_seconds {
        value.insert("ttl_seconds".to_string(), Value::from(field));
    }
    if let Some(field) = payload.expiry_behavior.clone() {
        value.insert("expiry_behavior".to_string(), Value::String(field));
    }
    Value::Object(value)
}

async fn resolve_transition_document(
    conn: &libsql::Connection,
    document_id: &str,
    collection: Option<&str>,
) -> AppResult<String> {
    let mut rows = if let Some(collection) = collection {
        conn.query(
            "SELECT collection FROM __kdb_documents WHERE id = ? AND collection = ? LIMIT 1",
            libsql::params![document_id.to_string(), collection.to_string()],
        )
        .await
    } else {
        conn.query(
            "SELECT collection FROM __kdb_documents WHERE id = ? LIMIT 1",
            libsql::params![document_id.to_string()],
        )
        .await
    }
    .map_err(|e| AppError::Internal(format!("transition document lookup failed: {e}")))?;
    let row = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("transition document row failed: {e}")))?
        .ok_or_else(|| AppError::NotFound(format!("document not found: {document_id}")))?;
    row.get(0)
        .map_err(|e| AppError::Internal(format!("transition namespace decode failed: {e}")))
}

async fn upsert_transition_on_conn(
    conn: &libsql::Connection,
    document_id: &str,
    collection: &str,
    spec: &DocumentLifecycleSpec,
) -> AppResult<()> {
    let condition = spec.condition.to_string();
    let update = spec.update.to_string();
    conn.execute(
        "INSERT INTO __kdb_document_transitions (
            id, document_id, collection, name, execute_at, condition_json, update_json,
            ttl_seconds, expiry_behavior, status, attempts, last_error, skipped_reason,
            started_at, completed_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, jsonb(?), jsonb(?), ?, ?, 'pending', 0, NULL, NULL, NULL, NULL,
                   strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         ON CONFLICT(document_id, name) DO UPDATE SET
            id = excluded.id, collection = excluded.collection, execute_at = excluded.execute_at,
            condition_json = excluded.condition_json, update_json = excluded.update_json,
            ttl_seconds = excluded.ttl_seconds, expiry_behavior = excluded.expiry_behavior,
            status = 'pending', attempts = 0, last_error = NULL, skipped_reason = NULL,
            started_at = NULL, completed_at = NULL,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        libsql::params![
            spec.transition_id.clone(), document_id.to_string(), collection.to_string(),
            spec.name.clone(), spec.execute_at.clone(), condition, update,
            spec.ttl_seconds, to_sql_nullable_text(spec.expiry_behavior.clone())
        ],
    )
    .await
    .map_err(|e| AppError::Internal(format!("schedule transition failed: {e}")))?;
    Ok(())
}

async fn upsert_transition_on_tx(
    tx: &libsql::Transaction,
    document_id: &str,
    collection: &str,
    spec: &DocumentLifecycleSpec,
) -> AppResult<()> {
    let condition = spec.condition.to_string();
    let update = spec.update.to_string();
    tx.execute(
        "INSERT INTO __kdb_document_transitions (
            id, document_id, collection, name, execute_at, condition_json, update_json,
            ttl_seconds, expiry_behavior, status, attempts, last_error, skipped_reason,
            started_at, completed_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, jsonb(?), jsonb(?), ?, ?, 'pending', 0, NULL, NULL, NULL, NULL,
                   strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         ON CONFLICT(document_id, name) DO UPDATE SET
            id = excluded.id, collection = excluded.collection, execute_at = excluded.execute_at,
            condition_json = excluded.condition_json, update_json = excluded.update_json,
            ttl_seconds = excluded.ttl_seconds, expiry_behavior = excluded.expiry_behavior,
            status = 'pending', attempts = 0, last_error = NULL, skipped_reason = NULL,
            started_at = NULL, completed_at = NULL,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        libsql::params![
            spec.transition_id.clone(), document_id.to_string(), collection.to_string(),
            spec.name.clone(), spec.execute_at.clone(), condition, update,
            spec.ttl_seconds, to_sql_nullable_text(spec.expiry_behavior.clone())
        ],
    )
    .await
    .map_err(|e| AppError::Internal(format!("schedule transition failed: {e}")))?;
    Ok(())
}

fn lifecycle_response(specs: &[DocumentLifecycleSpec]) -> Value {
    json!({
        "count": specs.len(),
        "transition_ids": specs.iter().map(|spec| spec.transition_id.clone()).collect::<Vec<_>>()
    })
}

fn add_lifecycle_response(response: &mut GatewayResponse, specs: &[DocumentLifecycleSpec]) {
    if let Some(Value::Object(data)) = response.data.as_mut() {
        data.insert("lifecycle".to_string(), lifecycle_response(specs));
    }
}

fn add_lifecycle_dry_run_response(response: &mut GatewayResponse, specs: &[DocumentLifecycleSpec]) {
    if let Some(Value::Object(data)) = response.data.as_mut() {
        data.insert(
            "lifecycle".to_string(),
            json!({"count": specs.len(), "transition_ids": [], "dry_run": true}),
        );
    }
}

async fn schedule_transition(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let document_id = clean_optional(payload.document_id.clone())
        .or_else(|| clean_optional(payload.id.clone()))
        .ok_or_else(|| AppError::BadRequest("document_id or id is required".to_string()))?;
    let collection = resolve_collection_scope_optional_collection(&payload)?;
    let actual_collection =
        resolve_transition_document(conn, &document_id, collection.as_deref()).await?;
    let specs = parse_document_lifecycle(Some(lifecycle_value_from_payload(&payload)))?;
    for spec in &specs {
        upsert_transition_on_conn(conn, &document_id, &actual_collection, spec).await?;
    }
    state
        .db_manager
        .append_wal_record(
            db_path,
            "DOCUMENT_TRANSITION_SCHEDULE",
            &json!({"document_id": document_id, "count": specs.len()}).to_string(),
        )
        .await?;
    Ok(GatewayResponse::ok(Some(json!({
        "document_id": document_id,
        "namespace": actual_collection,
        "lifecycle": lifecycle_response(&specs)
    }))))
}

fn transition_selector(payload: &OperationPayload) -> AppResult<(String, Vec<libsql::Value>)> {
    if let Some(id) = clean_optional(payload.transition_id.clone()) {
        return Ok(("id = ?".to_string(), vec![libsql::Value::Text(id)]));
    }
    let document_id = clean_optional(payload.document_id.clone())
        .or_else(|| clean_optional(payload.id.clone()))
        .ok_or_else(|| {
            AppError::BadRequest(
                "transition_id or document_id with name is required".to_string(),
            )
        })?;
    let name = clean_optional(payload.name.clone()).ok_or_else(|| {
        AppError::BadRequest("name is required with document_id".to_string())
    })?;
    Ok((
        "document_id = ? AND name = ?".to_string(),
        vec![libsql::Value::Text(document_id), libsql::Value::Text(name)],
    ))
}

async fn cancel_transition(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let (where_clause, binds) = transition_selector(&req.payload)?;
    let changed = conn
        .execute(
            &format!(
                "UPDATE __kdb_document_transitions
                 SET status = 'cancelled', completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE {where_clause} AND status = 'pending'"
            ),
            binds,
        )
        .await
        .map_err(|e| AppError::Internal(format!("cancel transition failed: {e}")))?;
    if changed > 0 {
        state
            .db_manager
            .append_wal_record(
                db_path,
                "DOCUMENT_TRANSITION_CANCEL",
                &json!({"count": changed}).to_string(),
            )
            .await?;
    }
    Ok(GatewayResponse::ok(Some(json!({"cancelled_count": changed}))))
}

async fn get_transition(
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let (where_clause, binds) = transition_selector(&req.payload)?;
    let mut rows = conn
        .query(
            &format!("{} WHERE {where_clause} LIMIT 1", transition_select_sql()),
            binds,
        )
        .await
        .map_err(|e| AppError::Internal(format!("get transition failed: {e}")))?;
    let row = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("get transition row failed: {e}")))?
        .ok_or_else(|| AppError::NotFound("transition not found".to_string()))?;
    Ok(GatewayResponse::ok(Some(transition_row_to_json(&row)?)))
}

fn transition_select_sql() -> &'static str {
    "SELECT id, document_id, collection, name, execute_at, json(condition_json), json(update_json),
            ttl_seconds, expiry_behavior, status, attempts, last_error, skipped_reason,
            created_at, started_at, completed_at, updated_at
     FROM __kdb_document_transitions"
}

fn transition_row_to_json(row: &libsql::Row) -> AppResult<Value> {
    let condition: String = row.get(5).map_err(|e| AppError::Internal(format!("transition condition decode failed: {e}")))?;
    let update: String = row.get(6).map_err(|e| AppError::Internal(format!("transition update decode failed: {e}")))?;
    Ok(json!({
        "transition_id": row.get::<String>(0).map_err(|e| AppError::Internal(format!("transition id decode failed: {e}")))?,
        "document_id": row.get::<String>(1).map_err(|e| AppError::Internal(format!("transition document id decode failed: {e}")))?,
        "namespace": row.get::<String>(2).map_err(|e| AppError::Internal(format!("transition namespace decode failed: {e}")))?,
        "name": row.get::<String>(3).map_err(|e| AppError::Internal(format!("transition name decode failed: {e}")))?,
        "execute_at": row.get::<String>(4).map_err(|e| AppError::Internal(format!("transition execute_at decode failed: {e}")))?,
        "when": serde_json::from_str::<Value>(&condition).map_err(|e| AppError::Internal(format!("transition condition json failed: {e}")))?,
        "update": serde_json::from_str::<Value>(&update).map_err(|e| AppError::Internal(format!("transition update json failed: {e}")))?,
        "ttl_seconds": row.get::<Option<i64>>(7).map_err(|e| AppError::Internal(format!("transition ttl decode failed: {e}")))?,
        "expiry_behavior": row.get::<Option<String>>(8).map_err(|e| AppError::Internal(format!("transition behavior decode failed: {e}")))?,
        "status": row.get::<String>(9).map_err(|e| AppError::Internal(format!("transition status decode failed: {e}")))?,
        "attempts": row.get::<i64>(10).map_err(|e| AppError::Internal(format!("transition attempts decode failed: {e}")))?,
        "last_error": row.get::<Option<String>>(11).map_err(|e| AppError::Internal(format!("transition error decode failed: {e}")))?,
        "skipped_reason": row.get::<Option<String>>(12).map_err(|e| AppError::Internal(format!("transition skip decode failed: {e}")))?,
        "created_at": row.get::<String>(13).map_err(|e| AppError::Internal(format!("transition created decode failed: {e}")))?,
        "started_at": row.get::<Option<String>>(14).map_err(|e| AppError::Internal(format!("transition started decode failed: {e}")))?,
        "completed_at": row.get::<Option<String>>(15).map_err(|e| AppError::Internal(format!("transition completed decode failed: {e}")))?,
        "updated_at": row.get::<String>(16).map_err(|e| AppError::Internal(format!("transition updated decode failed: {e}")))?
    }))
}

async fn list_transitions(
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let mut clauses = Vec::<String>::new();
    let mut binds = Vec::<libsql::Value>::new();
    for (column, value) in [
        ("document_id", clean_optional(payload.document_id.clone()).or_else(|| clean_optional(payload.id.clone()))),
        ("collection", clean_optional(payload.collection.clone())),
        ("name", clean_optional(payload.name.clone())),
        ("status", clean_optional(payload.status.clone())),
    ] {
        if let Some(value) = value {
            clauses.push(format!("{column} = ?"));
            binds.push(libsql::Value::Text(value));
        }
    }
    if let Some(value) = clean_optional(payload.execute_at_from.clone()) {
        chrono::DateTime::parse_from_rfc3339(&value).map_err(|_| AppError::BadRequest("execute_at_from must be RFC3339".to_string()))?;
        clauses.push("execute_at >= ?".to_string());
        binds.push(libsql::Value::Text(value));
    }
    if let Some(value) = clean_optional(payload.execute_at_to.clone()) {
        chrono::DateTime::parse_from_rfc3339(&value).map_err(|_| AppError::BadRequest("execute_at_to must be RFC3339".to_string()))?;
        clauses.push("execute_at <= ?".to_string());
        binds.push(libsql::Value::Text(value));
    }
    let where_clause = if clauses.is_empty() { "1=1".to_string() } else { clauses.join(" AND ") };
    let limit = payload.per_page.or(payload.limit).unwrap_or(100).clamp(1, 1000);
    let offset = if let Some(page) = payload.page {
        page.saturating_sub(1).saturating_mul(limit)
    } else {
        payload.offset.unwrap_or(0).max(0)
    };
    let mut count_rows = conn.query(
        &format!("SELECT COUNT(*) FROM __kdb_document_transitions WHERE {where_clause}"),
        binds.clone(),
    ).await.map_err(|e| AppError::Internal(format!("list transitions count failed: {e}")))?;
    let total_items: i64 = count_rows.next().await.map_err(|e| AppError::Internal(format!("list transitions count row failed: {e}")))?
        .ok_or_else(|| AppError::Internal("list transitions count missing".to_string()))?
        .get(0).map_err(|e| AppError::Internal(format!("list transitions count decode failed: {e}")))?;
    drop(count_rows);
    let mut query_binds = binds;
    query_binds.push(libsql::Value::Integer(limit));
    query_binds.push(libsql::Value::Integer(offset));
    let mut rows = conn.query(
        &format!("{} WHERE {where_clause} ORDER BY execute_at, created_at LIMIT ? OFFSET ?", transition_select_sql()),
        query_binds,
    ).await.map_err(|e| AppError::Internal(format!("list transitions failed: {e}")))?;
    let mut items = Vec::new();
    while let Some(row) = rows.next().await.map_err(|e| AppError::Internal(format!("list transitions row failed: {e}")))? {
        items.push(transition_row_to_json(&row)?);
    }
    let page = offset / limit + 1;
    let total_pages = if total_items == 0 { 0 } else { (total_items + limit - 1) / limit };
    Ok(GatewayResponse::ok(Some(json!({
        "items": items, "count": items.len(), "total_items": total_items,
        "limit": limit, "offset": offset,
        "next_offset": (offset + limit < total_items).then_some(offset + limit),
        "prev_offset": (offset > 0).then_some(offset.saturating_sub(limit)),
        "pagination": {
            "total_items": total_items, "count": items.len(), "per_page": limit,
            "page": page, "total_pages": total_pages,
            "next_page": (page < total_pages).then_some(page + 1),
            "prev_page": (page > 1).then_some(page - 1)
        }
    }))))
}

async fn retry_transition(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let (where_clause, mut binds) = transition_selector(&payload)?;
    let execute_at = match (payload.at.as_deref(), payload.after_seconds) {
        (Some(_), Some(_)) => return Err(AppError::BadRequest("at and after_seconds cannot both be provided".to_string())),
        (Some(at), None) => chrono::DateTime::parse_from_rfc3339(at)
            .map_err(|_| AppError::BadRequest("at must be RFC3339".to_string()))?
            .with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true),
        (None, Some(seconds)) if seconds > 0 => {
            let delay = Duration::try_seconds(seconds).ok_or_else(|| {
                AppError::BadRequest("after_seconds produces an out-of-range datetime".to_string())
            })?;
            Utc::now()
                .checked_add_signed(delay)
                .ok_or_else(|| AppError::BadRequest("after_seconds produces an out-of-range datetime".to_string()))?
                .to_rfc3339_opts(SecondsFormat::Millis, true)
        },
        (None, Some(_)) => return Err(AppError::BadRequest("after_seconds must be positive".to_string())),
        (None, None) => Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
    };
    let mut all_binds = vec![libsql::Value::Text(execute_at)];
    all_binds.append(&mut binds);
    let changed = conn.execute(
        &format!("UPDATE __kdb_document_transitions
                 SET execute_at = ?, status = 'pending', last_error = NULL, skipped_reason = NULL,
                     started_at = NULL, completed_at = NULL,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE {where_clause} AND status = 'failed'"),
        all_binds,
    ).await.map_err(|e| AppError::Internal(format!("retry transition failed: {e}")))?;
    if changed > 0 {
        state
            .db_manager
            .append_wal_record(
                db_path,
                "DOCUMENT_TRANSITION_RETRY",
                &json!({"count": changed}).to_string(),
            )
            .await?;
    }
    Ok(GatewayResponse::ok(Some(json!({"retried_count": changed}))))
}

fn merge_json_patch(target: &mut Value, patch: &Value) {
    match patch {
        Value::Object(patch_obj) => {
            if !target.is_object() {
                *target = json!({});
            }
            let target_obj = target.as_object_mut().expect("object initialized");
            for (key, value) in patch_obj {
                if value.is_null() {
                    target_obj.remove(key);
                } else {
                    merge_json_patch(target_obj.entry(key.clone()).or_insert(Value::Null), value);
                }
            }
        }
        value => *target = value.clone(),
    }
}

async fn run_one_document_transition(
    state: &AppState,
    conn: &libsql::Connection,
    transition_id: &str,
) -> AppResult<&'static str> {
    conn.execute_batch("BEGIN IMMEDIATE").await.map_err(|e| AppError::Internal(format!("transition tx begin failed: {e}")))?;
    let result: AppResult<&'static str> = async {
        let claimed = conn.execute(
            "UPDATE __kdb_document_transitions
             SET status = 'running', attempts = attempts + 1,
                 started_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ? AND status = 'pending'
               AND execute_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            libsql::params![transition_id.to_string()],
        ).await.map_err(|e| AppError::Internal(format!("transition claim failed: {e}")))?;
        if claimed == 0 {
            return Ok("ignored");
        }
        let mut rows = conn.query(
            "SELECT document_id, collection, json(condition_json), json(update_json), ttl_seconds, expiry_behavior
             FROM __kdb_document_transitions WHERE id = ? LIMIT 1",
            libsql::params![transition_id.to_string()],
        ).await.map_err(|e| AppError::Internal(format!("transition read failed: {e}")))?;
        let row = rows.next().await.map_err(|e| AppError::Internal(format!("transition row failed: {e}")))?
            .ok_or_else(|| AppError::Internal("claimed transition disappeared".to_string()))?;
        let document_id: String = row.get(0).map_err(|e| AppError::Internal(format!("transition document decode failed: {e}")))?;
        let collection: String = row.get(1).map_err(|e| AppError::Internal(format!("transition collection decode failed: {e}")))?;
        let condition_raw: String = row.get(2).map_err(|e| AppError::Internal(format!("transition condition decode failed: {e}")))?;
        let update_raw: String = row.get(3).map_err(|e| AppError::Internal(format!("transition update decode failed: {e}")))?;
        let ttl_seconds: Option<i64> = row.get(4).map_err(|e| AppError::Internal(format!("transition ttl decode failed: {e}")))?;
        let expiry_behavior: Option<String> = row.get(5).map_err(|e| AppError::Internal(format!("transition behavior decode failed: {e}")))?;
        drop(rows);

        let mut doc_rows = conn.query(
            "SELECT rowid, json(data) FROM __kdb_documents WHERE id = ? AND collection = ? LIMIT 1",
            libsql::params![document_id.clone(), collection.clone()],
        ).await.map_err(|e| AppError::Internal(format!("transition document read failed: {e}")))?;
        let Some(doc_row) = doc_rows.next().await.map_err(|e| AppError::Internal(format!("transition document row failed: {e}")))? else {
            drop(doc_rows);
            conn.execute(
                "UPDATE __kdb_document_transitions SET status='skipped', skipped_reason='document_not_found',
                 completed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'), updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?",
                libsql::params![transition_id.to_string()],
            ).await.map_err(|e| AppError::Internal(format!("transition skip failed: {e}")))?;
            return Ok("skipped");
        };
        let rowid: i64 = doc_row.get(0).map_err(|e| AppError::Internal(format!("transition rowid decode failed: {e}")))?;
        let doc_raw: String = doc_row.get(1).map_err(|e| AppError::Internal(format!("transition data decode failed: {e}")))?;
        drop(doc_rows);

        let condition: Value = serde_json::from_str(&condition_raw).map_err(|e| AppError::Internal(format!("transition condition json failed: {e}")))?;
        let compiled = build_where(&condition)?;
        let mut condition_binds = vec![libsql::Value::Integer(rowid)];
        condition_binds.extend(compiled.binds);
        let mut condition_rows = conn.query(
            &format!("SELECT 1 FROM __kdb_documents WHERE rowid = ? AND ({}) LIMIT 1", compiled.sql),
            condition_binds,
        ).await.map_err(|e| AppError::Internal(format!("transition condition failed: {e}")))?;
        let condition_met = condition_rows.next().await.map_err(|e| AppError::Internal(format!("transition condition row failed: {e}")))?.is_some();
        drop(condition_rows);
        if !condition_met {
            conn.execute(
                "UPDATE __kdb_document_transitions SET status='skipped', skipped_reason='condition_not_met',
                 completed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'), updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?",
                libsql::params![transition_id.to_string()],
            ).await.map_err(|e| AppError::Internal(format!("transition skip failed: {e}")))?;
            return Ok("skipped");
        }

        let mut doc: Value = serde_json::from_str(&doc_raw).map_err(|e| AppError::Internal(format!("transition document json failed: {e}")))?;
        let mut update: Value = serde_json::from_str(&update_raw).map_err(|e| AppError::Internal(format!("transition update json failed: {e}")))?;
        expand_kdb_macros_in_value(&mut update)?;
        let update_obj = update.as_object_mut().ok_or_else(|| AppError::BadRequest("transition update must be object".to_string()))?;
        if update_requires_mutation_engine(update_obj) {
            apply_mutation_patch_to_doc(&mut doc, update_obj, state.strict_mutation_operators)?;
        } else {
            merge_json_patch(&mut doc, &update);
        }
        let doc_string = doc.to_string();
        let expires_at = match ttl_seconds {
            Some(0) | None => None,
            Some(seconds) => Some(
                unix_now_secs().checked_add(seconds).ok_or_else(|| {
                    AppError::BadRequest("transition TTL is out of range".to_string())
                })?,
            ),
        };
        conn.execute(
            &format!("UPDATE __kdb_documents
                      SET data = {}, _size_bytes = ?, _modified_at = strftime('%Y-%m-%dT%H:%M:%fZ','now'),
                          _expires_at = CASE WHEN ? IS NULL THEN _expires_at ELSE ? END,
                          _expiry_behavior = COALESCE(?, _expiry_behavior)
                      WHERE rowid = ?", json_input_expr(state.jsonb_enabled)),
            libsql::params![doc_string.clone(), doc_string.len() as i64,
                ttl_seconds, expires_at, to_sql_nullable_text(expiry_behavior), rowid],
        ).await.map_err(|e| AppError::Internal(format!("transition document update failed: {e}")))?;
        conn.execute(
            "UPDATE __kdb_document_transitions SET status='completed', skipped_reason=NULL, last_error=NULL,
             completed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'), updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?",
            libsql::params![transition_id.to_string()],
        ).await.map_err(|e| AppError::Internal(format!("transition completion failed: {e}")))?;
        Ok("completed")
    }.await;

    match result {
        Ok(status) => {
            conn.execute_batch("COMMIT").await.map_err(|e| AppError::Internal(format!("transition tx commit failed: {e}")))?;
            Ok(status)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK").await;
            conn.execute(
                "UPDATE __kdb_document_transitions
                 SET status='failed', attempts=attempts+1, last_error=?,
                     completed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'), updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
                 WHERE id=? AND status='pending'",
                libsql::params![error.to_string(), transition_id.to_string()],
            ).await.map_err(|e| AppError::Internal(format!("transition failure mark failed: {e}")))?;
            Ok("failed")
        }
    }
}

async fn process_due_document_transitions(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
) -> AppResult<GatewayResponse> {
    let mut rows = conn.query(
        "SELECT id FROM __kdb_document_transitions
         WHERE status='pending' AND execute_at <= strftime('%Y-%m-%dT%H:%M:%fZ','now')
         ORDER BY execute_at, created_at LIMIT 100",
        (),
    ).await.map_err(|e| AppError::Internal(format!("due transitions query failed: {e}")))?;
    let mut ids = Vec::new();
    while let Some(row) = rows.next().await.map_err(|e| AppError::Internal(format!("due transitions row failed: {e}")))? {
        ids.push(row.get::<String>(0).map_err(|e| AppError::Internal(format!("due transition id decode failed: {e}")))?);
    }
    drop(rows);
    let mut completed = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    for id in &ids {
        match run_one_document_transition(state, conn, id).await? {
            "completed" => completed += 1,
            "skipped" => skipped += 1,
            "failed" => failed += 1,
            _ => {}
        }
    }
    if !ids.is_empty() {
        state
            .db_manager
            .append_wal_record(
                db_path,
                "DOCUMENT_TRANSITION_EXECUTE",
                &json!({
                    "claimed_count": ids.len(), "completed_count": completed,
                    "skipped_count": skipped, "failed_count": failed
                })
                .to_string(),
            )
            .await?;
    }
    Ok(GatewayResponse::ok(Some(json!({
        "claimed_count": ids.len(), "completed_count": completed,
        "skipped_count": skipped, "failed_count": failed
    }))))
}

pub async fn process_document_transitions_tick(state: &AppState) -> AppResult<usize> {
    let dbs = state.db_manager.list_active_db_paths();
    let mut processed = 0usize;
    for db in dbs {
        let request = GatewayRequest {
            db: Some(db.clone()), operation: "__run_document_transitions".to_string(),
            namespace: None, namespaces: None, payload: OperationPayload::default(), data: None,
        };
        match state.enqueue_committed_write(&db, request).await? {
            crate::state::WriteEnqueueResult::Committed(result) => {
                let response = result?;
                processed += response.data.as_ref().and_then(|value| value.get("claimed_count")).and_then(Value::as_u64).unwrap_or(0) as usize;
            }
            crate::state::WriteEnqueueResult::Fallback(_) => {
                let conn = state.db_manager.get_conn_with_create(&db, false).await?;
                let response = process_due_document_transitions(state, &db, &conn).await?;
                processed += response.data.as_ref().and_then(|value| value.get("claimed_count")).and_then(Value::as_u64).unwrap_or(0) as usize;
            }
            crate::state::WriteEnqueueResult::Enqueued => {}
        }
    }
    Ok(processed)
}

#[cfg(test)]
mod document_lifecycle_tests {
    use super::*;

    #[test]
    fn lifecycle_accepts_one_or_multiple_named_specs() {
        let one = parse_document_lifecycle(Some(json!({
            "name": "expire",
            "after_seconds": 60,
            "when": {"status": "pending"},
            "update": {"status": "expired"}
        })))
        .unwrap();
        assert_eq!(one.len(), 1);

        let multiple = parse_document_lifecycle(Some(json!([
            {"name":"publish","after_seconds":60,"when":{},"update":{"status":"published"}},
            {"name":"archive","after_seconds":120,"when":{},"update":{"status":"archived"}}
        ])))
        .unwrap();
        assert_eq!(multiple.len(), 2);
    }

    #[test]
    fn lifecycle_rejects_ambiguous_time_and_duplicate_names() {
        let ambiguous = parse_document_lifecycle(Some(json!({
            "name": "expire",
            "at": "2026-08-09T00:00:00Z",
            "after_seconds": 60,
            "when": {},
            "update": {"status": "expired"}
        })))
        .unwrap_err();
        assert!(ambiguous.to_string().contains("exactly one"));

        let duplicate = parse_document_lifecycle(Some(json!([
            {"name":"same","after_seconds":60,"when":{},"update":{"a":1}},
            {"name":"same","after_seconds":120,"when":{},"update":{"b":2}}
        ])))
        .unwrap_err();
        assert!(duplicate.to_string().contains("duplicated"));
    }
}
