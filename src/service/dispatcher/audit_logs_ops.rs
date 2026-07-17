// Append-only audit log ingestion and query handlers.

#[derive(Clone, Debug)]
struct PreparedAuditEvent {
    id: String,
    ts: String,
    action: String,
    actor_type: Option<String>,
    actor_id: Option<String>,
    target_type: Option<String>,
    target_id: Option<String>,
    status: String,
    source: Option<String>,
    request_id: Option<String>,
    ip_address: Option<String>,
    message: Option<String>,
    data: Value,
    size_bytes: i64,
}

async fn audit_ingest(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let events = prepare_audit_events(req.payload.events)?;
    let json_expr = json_input_expr(state.jsonb_enabled);

    for chunk in events.chunks(250) {
        let tx = conn
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("audit ingest tx begin failed: {e}")))?;
        for event in chunk {
            tx.execute(
                &format!(
                    "INSERT INTO __kdb_audit_logs (
                        id, ts, action, actor_type, actor_id, target_type, target_id,
                        status, source, request_id, ip_address, message, data, _size_bytes
                     ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, {json_expr}, ?)"
                ),
                libsql::params![
                    event.id.clone(),
                    event.ts.clone(),
                    event.action.clone(),
                    to_sql_nullable_text(event.actor_type.clone()),
                    to_sql_nullable_text(event.actor_id.clone()),
                    to_sql_nullable_text(event.target_type.clone()),
                    to_sql_nullable_text(event.target_id.clone()),
                    event.status.clone(),
                    to_sql_nullable_text(event.source.clone()),
                    to_sql_nullable_text(event.request_id.clone()),
                    to_sql_nullable_text(event.ip_address.clone()),
                    to_sql_nullable_text(event.message.clone()),
                    event.data.to_string(),
                    event.size_bytes
                ],
            )
            .await
            .map_err(|e| AppError::Internal(format!("audit event insert failed: {e}")))?;
        }
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("audit ingest tx commit failed: {e}")))?;
    }

    state
        .db_manager
        .append_wal_record(
            db_path,
            "AUDIT_INGEST",
            &json!({"count": events.len()}).to_string(),
        )
        .await?;

    Ok(GatewayResponse::ok(Some(json!({
        "count": events.len(),
        "ids": events.iter().map(|event| event.id.clone()).collect::<Vec<_>>()
    }))))
}

async fn audit_query(
    state: &AppState,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let (limit, offset, page) = resolve_pagination_args(&payload, state.query_default_limit)?;
    let mut where_parts = Vec::<String>::new();
    let mut binds = Vec::<libsql::Value>::new();

    push_audit_filter(&mut where_parts, &mut binds, "action", payload.action);
    push_audit_filter(&mut where_parts, &mut binds, "actor_type", payload.actor_type);
    push_audit_filter(&mut where_parts, &mut binds, "actor_id", payload.actor_id);
    push_audit_filter(&mut where_parts, &mut binds, "target_type", payload.target_type);
    push_audit_filter(&mut where_parts, &mut binds, "target_id", payload.target_id);
    push_audit_filter(&mut where_parts, &mut binds, "status", payload.status);
    push_audit_filter(&mut where_parts, &mut binds, "source", payload.source);
    push_audit_filter(&mut where_parts, &mut binds, "request_id", payload.request_id);

    if let Some(start) = clean_optional(payload.start) {
        where_parts.push("datetime(ts) >= datetime(?)".to_string());
        binds.push(libsql::Value::Text(parse_metric_events_datetime(
            &start, false, "start",
        )?));
    }
    if let Some(end) = clean_optional(payload.end) {
        where_parts.push("datetime(ts) <= datetime(?)".to_string());
        binds.push(libsql::Value::Text(parse_metric_events_datetime(
            &end, true, "end",
        )?));
    }
    if let Some(search) = clean_optional(payload.search) {
        let pattern = format!("%{}%", search.to_ascii_lowercase());
        where_parts.push(
            "(lower(action) LIKE ? OR lower(COALESCE(message, '')) LIKE ? OR lower(COALESCE(actor_id, '')) LIKE ? OR lower(COALESCE(target_id, '')) LIKE ?)"
                .to_string(),
        );
        for _ in 0..4 {
            binds.push(libsql::Value::Text(pattern.clone()));
        }
    }

    let where_sql = if where_parts.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", where_parts.join(" AND "))
    };

    let mut count_rows = conn
        .query(
            &format!("SELECT COUNT(*) FROM __kdb_audit_logs{where_sql}"),
            binds.clone(),
        )
        .await
        .map_err(|e| AppError::Internal(format!("audit count query failed: {e}")))?;
    let total_items = count_rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("audit count row failed: {e}")))?
        .map(|row| row.get::<i64>(0))
        .transpose()
        .map_err(|e| AppError::Internal(format!("audit count decode failed: {e}")))?
        .unwrap_or(0);
    drop(count_rows);

    binds.push(libsql::Value::Integer(limit));
    binds.push(libsql::Value::Integer(offset));
    let mut rows = conn
        .query(
            &format!(
                "SELECT id, ts, action, actor_type, actor_id, target_type, target_id,
                        status, source, request_id, ip_address, message, json(data),
                        _created_at, _size_bytes
                 FROM __kdb_audit_logs{where_sql}
                 ORDER BY ts DESC, id DESC
                 LIMIT ? OFFSET ?"
            ),
            binds,
        )
        .await
        .map_err(|e| AppError::Internal(format!("audit query failed: {e}")))?;

    let mut items = Vec::<Value>::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("audit row read failed: {e}")))?
    {
        let raw_data: String = row
            .get(12)
            .map_err(|e| AppError::Internal(format!("audit data decode failed: {e}")))?;
        let data = serde_json::from_str::<Value>(&raw_data)
            .map_err(|e| AppError::Internal(format!("audit data JSON decode failed: {e}")))?;
        items.push(json!({
            "_id": audit_row_string(&row, 0, "id")?,
            "ts": audit_row_string(&row, 1, "ts")?,
            "action": audit_row_string(&row, 2, "action")?,
            "actor_type": audit_row_optional_string(&row, 3, "actor_type")?,
            "actor_id": audit_row_optional_string(&row, 4, "actor_id")?,
            "target_type": audit_row_optional_string(&row, 5, "target_type")?,
            "target_id": audit_row_optional_string(&row, 6, "target_id")?,
            "status": audit_row_string(&row, 7, "status")?,
            "source": audit_row_optional_string(&row, 8, "source")?,
            "request_id": audit_row_optional_string(&row, 9, "request_id")?,
            "ip_address": audit_row_optional_string(&row, 10, "ip_address")?,
            "message": audit_row_optional_string(&row, 11, "message")?,
            "data": data,
            "_created_at": audit_row_string(&row, 13, "_created_at")?,
            "_size_bytes": row.get::<i64>(14).map_err(|e| AppError::Internal(format!("audit size decode failed: {e}")))?
        }));
    }

    let pagination = build_pagination(total_items, items.len(), limit, page, offset);
    let (next_offset, prev_offset) = build_offsets(total_items, items.len(), limit, offset);
    Ok(GatewayResponse::ok(Some(json!({
        "count": items.len(),
        "total_items": total_items,
        "items": items,
        "limit": limit,
        "offset": offset,
        "next_offset": next_offset,
        "prev_offset": prev_offset,
        "pagination": pagination
    }))))
}

fn prepare_audit_events(raw: Option<Vec<Value>>) -> AppResult<Vec<PreparedAuditEvent>> {
    let raw = raw.ok_or_else(|| AppError::BadRequest("events is required".to_string()))?;
    if raw.is_empty() {
        return Err(AppError::BadRequest("events cannot be empty".to_string()));
    }
    let mut events = Vec::<PreparedAuditEvent>::with_capacity(raw.len());
    let mut ids = HashSet::<String>::new();
    for mut value in raw {
        expand_kdb_macros_in_value(&mut value)?;
        let object = value
            .as_object()
            .ok_or_else(|| AppError::BadRequest("events items must be objects".to_string()))?;
        let id = match object.get("_id") {
            Some(Value::String(value)) if !value.trim().is_empty() => value.trim().to_string(),
            Some(_) => {
                return Err(AppError::BadRequest(
                    "events[]._id must be a non-empty string".to_string(),
                ));
            }
            None => format!("aud_{}", Uuid::new_v4().simple()),
        };
        if !ids.insert(id.clone()) {
            return Err(AppError::BadRequest(format!("duplicate audit event _id: {id}")));
        }
        let action = audit_object_optional_string(object, "action")
            .ok_or_else(|| AppError::BadRequest("events[].action is required".to_string()))?;
        let ts = match audit_object_optional_string(object, "ts") {
            Some(raw) => parse_metric_events_datetime(&raw, false, "events[].ts")?,
            None => Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        };
        let status = audit_object_optional_string(object, "status")
            .unwrap_or_else(|| "success".to_string());
        let data = object.get("data").cloned().unwrap_or_else(|| json!({}));
        let size_bytes = i64::try_from(value.to_string().len()).unwrap_or(i64::MAX);
        events.push(PreparedAuditEvent {
            id,
            ts,
            action,
            actor_type: audit_object_optional_string(object, "actor_type"),
            actor_id: audit_object_optional_string(object, "actor_id"),
            target_type: audit_object_optional_string(object, "target_type"),
            target_id: audit_object_optional_string(object, "target_id"),
            status,
            source: audit_object_optional_string(object, "source"),
            request_id: audit_object_optional_string(object, "request_id"),
            ip_address: audit_object_optional_string(object, "ip_address"),
            message: audit_object_optional_string(object, "message"),
            data,
            size_bytes,
        });
    }
    Ok(events)
}

fn push_audit_filter(
    where_parts: &mut Vec<String>,
    binds: &mut Vec<libsql::Value>,
    column: &str,
    value: Option<String>,
) {
    if let Some(value) = clean_optional(value) {
        where_parts.push(format!("{column} = ?"));
        binds.push(libsql::Value::Text(value));
    }
}

fn audit_object_optional_string(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn audit_row_string(row: &libsql::Row, index: i32, field: &str) -> AppResult<String> {
    row.get(index)
        .map_err(|e| AppError::Internal(format!("audit {field} decode failed: {e}")))
}

fn audit_row_optional_string(
    row: &libsql::Row,
    index: i32,
    field: &str,
) -> AppResult<Option<String>> {
    row.get(index)
        .map_err(|e| AppError::Internal(format!("audit {field} decode failed: {e}")))
}

#[cfg(test)]
mod audit_logs_tests {
    use super::*;

    #[test]
    fn prepares_audit_defaults_and_rejects_missing_action() {
        let events = prepare_audit_events(Some(vec![json!({"action": "user.login"})])).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].id.starts_with("aud_"));
        assert_eq!(events[0].status, "success");

        let error = prepare_audit_events(Some(vec![json!({"message": "missing"})])).unwrap_err();
        assert!(error.to_string().contains("action is required"));
    }
}
