// MetricEvents ingestion/query handlers for append-only metrics events.

#[derive(Clone, Debug)]
struct PreparedMetricEventsEvent {
    id: String,
    event: String,
    ts: String,
    tenant_id: Option<String>,
    user_id: Option<String>,
    value: f64,
    dimensions: Value,
    metadata: Value,
    size_bytes: i64,
}

#[derive(Clone, Debug)]
struct MetricEventsQuerySpec {
    alias: String,
    label: String,
    events: Vec<String>,
    range: Option<String>,
    start: String,
    end: String,
    interval: Option<String>,
    bucket_label: Option<String>,
    filter: Value,
    groups: Vec<MetricEventsGroupSpec>,
    metrics: Vec<MetricEventsMetricSpec>,
    sort: Option<Value>,
    limit: i64,
    offset: i64,
}

#[derive(Clone, Debug)]
struct MetricEventsGroupSpec {
    field: String,
    alias: String,
    label: String,
}

#[derive(Clone, Debug)]
struct MetricEventsMetricSpec {
    op: String,
    field: String,
    alias: String,
    label: String,
}

async fn metrics_ingest(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let events = prepare_metric_events_events(req.payload.events)?;
    if events.is_empty() {
        return Err(AppError::BadRequest("events cannot be empty".to_string()));
    }

    for chunk in events.chunks(state.metric_events_insert_batch_size.max(1)) {
        let tx = conn
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("metric_events tx begin failed: {e}")))?;
        let json_expr = json_input_expr(state.jsonb_enabled);
        for item in chunk {
            tx.execute(
                &format!(
                    "INSERT INTO __kdb_metric_events
                     (id, event, ts, tenant_id, user_id, value, dimensions, metadata, _size_bytes)
                     VALUES (?, ?, ?, ?, ?, ?, {json_expr}, {json_expr}, ?)"
                ),
                libsql::params![
                    item.id.clone(),
                    item.event.clone(),
                    item.ts.clone(),
                    to_sql_nullable_text(item.tenant_id.clone()),
                    to_sql_nullable_text(item.user_id.clone()),
                    item.value,
                    item.dimensions.to_string(),
                    item.metadata.to_string(),
                    item.size_bytes
                ],
            )
            .await
            .map_err(|e| AppError::Internal(format!("metric_events insert failed: {e}")))?;
            upsert_metrics_catalog_for_event(&tx, item).await?;
        }
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("metric_events tx commit failed: {e}")))?;
    }

    state
        .db_manager
        .append_wal_record(
            db_path,
            "METRICS_PUT",
            &json!({"count": events.len()}).to_string(),
        )
        .await?;

    Ok(GatewayResponse::ok(Some(json!({
        "count": events.len(),
        "ids": events.iter().map(|e| e.id.clone()).collect::<Vec<_>>()
    }))))
}

async fn metrics_query(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let cache_meta = metric_events_cache_key(state, db_path, &payload)?;
    if let Some(meta) = cache_meta.as_ref() {
        if let Some(cached) = get_cached_read(state, meta).await {
            return Ok(GatewayResponse::ok(Some(cached)));
        }
    }

    let specs = normalize_metrics_query_payload(payload, state)?;
    let mut results = serde_json::Map::new();
    let mut seen_aliases = HashSet::<String>::new();

    for spec in specs {
        if !seen_aliases.insert(spec.alias.clone()) {
            return Err(AppError::BadRequest(format!(
                "duplicate metrics query alias: {}",
                spec.alias
            )));
        }
        let result = run_metrics_query_spec(conn, &spec).await?;
        results.insert(spec.alias.clone(), result);
    }

    let data = json!({
        "count": results.len(),
        "results": Value::Object(results)
    });
    if let Some(meta) = cache_meta {
        put_cached_read(state, &meta, data.clone()).await;
    }
    Ok(GatewayResponse::ok(Some(data)))
}

async fn metrics_catalog(
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let limit = payload.limit.unwrap_or(250).clamp(1, 1000);
    let offset = payload.offset.unwrap_or(0).max(0);
    let mut where_parts = Vec::<String>::new();
    let mut binds = Vec::<libsql::Value>::new();

    if let Some(value) = payload.catalog_type.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        where_parts.push("type = ?".to_string());
        binds.push(libsql::Value::Text(value.to_string()));
    }
    if let Some(value) = payload.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        where_parts.push("name = ?".to_string());
        binds.push(libsql::Value::Text(value.to_string()));
    }
    if let Some(value) = payload.value.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        where_parts.push("value = ?".to_string());
        binds.push(libsql::Value::Text(value.to_string()));
    }

    let where_sql = if where_parts.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", where_parts.join(" AND "))
    };
    binds.push(libsql::Value::Integer(limit));
    binds.push(libsql::Value::Integer(offset));

    let sql = format!(
        "SELECT type, name, value, created_at, updated_at
         FROM __kdb_metrics_catalog{where_sql}
         ORDER BY type ASC, name ASC, value ASC
         LIMIT ? OFFSET ?"
    );
    let mut rows = conn
        .query(&sql, binds)
        .await
        .map_err(|e| AppError::Internal(format!("metrics catalog query failed: {e}")))?;
    let mut items = Vec::<Value>::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("metrics catalog row read failed: {e}")))?
    {
        items.push(json!({
            "type": row.get::<String>(0).map_err(|e| AppError::Internal(format!("metrics catalog decode failed: {e}")))?,
            "name": row.get::<String>(1).map_err(|e| AppError::Internal(format!("metrics catalog decode failed: {e}")))?,
            "value": row.get::<String>(2).map_err(|e| AppError::Internal(format!("metrics catalog decode failed: {e}")))?,
            "created_at": row.get::<String>(3).map_err(|e| AppError::Internal(format!("metrics catalog decode failed: {e}")))?,
            "updated_at": row.get::<String>(4).map_err(|e| AppError::Internal(format!("metrics catalog decode failed: {e}")))?,
        }));
    }

    Ok(GatewayResponse::ok(Some(json!({
        "count": items.len(),
        "items": items,
        "limit": limit,
        "offset": offset
    }))))
}

fn prepare_metric_events_events(raw: Option<Vec<Value>>) -> AppResult<Vec<PreparedMetricEventsEvent>> {
    let raw = raw.ok_or_else(|| AppError::BadRequest("events is required".to_string()))?;
    if raw.is_empty() {
        return Err(AppError::BadRequest("events cannot be empty".to_string()));
    }
    let mut out = Vec::with_capacity(raw.len());
    for mut event_value in raw {
        expand_kdb_macros_in_value(&mut event_value)?;
        let obj = event_value
            .as_object()
            .ok_or_else(|| AppError::BadRequest("events items must be objects".to_string()))?;
        let id = obj
            .get("_id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("evt_{}", Uuid::new_v4().simple()));
        let event = obj
            .get("event")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AppError::BadRequest("events[].event is required".to_string()))?
            .to_string();
        let ts = match obj.get("ts").and_then(Value::as_str) {
            Some(raw) => parse_metric_events_datetime(raw, false, "events[].ts")?,
            None => Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        };
        let tenant_id = obj
            .get("tenant_id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(ToOwned::to_owned);
        let user_id = obj
            .get("user_id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(ToOwned::to_owned);
        let value = obj.get("value").and_then(Value::as_f64).unwrap_or(1.0);
        if !value.is_finite() {
            return Err(AppError::BadRequest("events[].value must be finite".to_string()));
        }
        let dimensions = obj.get("dimensions").cloned().unwrap_or_else(|| json!({}));
        if !dimensions.is_object() {
            return Err(AppError::BadRequest(
                "events[].dimensions must be an object".to_string(),
            ));
        }
        let metadata = obj.get("metadata").cloned().unwrap_or_else(|| json!({}));
        if !metadata.is_object() {
            return Err(AppError::BadRequest(
                "events[].metadata must be an object".to_string(),
            ));
        }
        let size_bytes = event_value.to_string().len() as i64;
        out.push(PreparedMetricEventsEvent {
            id,
            event,
            ts,
            tenant_id,
            user_id,
            value,
            dimensions,
            metadata,
            size_bytes,
        });
    }
    Ok(out)
}

async fn upsert_metrics_catalog_for_event(
    tx: &libsql::Transaction,
    item: &PreparedMetricEventsEvent,
) -> AppResult<()> {
    upsert_metrics_catalog_entry(tx, "event", "name", item.event.as_str()).await?;
    let mut dimensions = Vec::<String>::new();
    collect_metric_dimension_paths(&item.dimensions, "dimensions", &mut dimensions);
    for path in dimensions {
        upsert_metrics_catalog_entry(tx, "dimension", item.event.as_str(), path.as_str()).await?;
    }
    Ok(())
}

async fn upsert_metrics_catalog_entry(
    tx: &libsql::Transaction,
    catalog_type: &str,
    name: &str,
    value: &str,
) -> AppResult<()> {
    tx.execute(
        "INSERT INTO __kdb_metrics_catalog (type, name, value)
         VALUES (?, ?, ?)
         ON CONFLICT(type, name, value) DO UPDATE SET
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        libsql::params![
            catalog_type.to_string(),
            name.to_string(),
            value.to_string()
        ],
    )
    .await
    .map_err(|e| AppError::Internal(format!("metrics catalog upsert failed: {e}")))?;
    Ok(())
}

fn collect_metric_dimension_paths(value: &Value, prefix: &str, out: &mut Vec<String>) {
    let Some(obj) = value.as_object() else {
        return;
    };
    for (key, val) in obj {
        let path = format!("{prefix}.{key}");
        out.push(path.clone());
        if val.is_object() {
            collect_metric_dimension_paths(val, path.as_str(), out);
        }
    }
}

fn normalize_metrics_query_payload(
    payload: OperationPayload,
    state: &AppState,
) -> AppResult<Vec<MetricEventsQuerySpec>> {
    if let Some(batch) = payload.batch {
        if batch.is_empty() {
            return Err(AppError::BadRequest("batch cannot be empty".to_string()));
        }
        return batch
            .into_iter()
            .map(|p| normalize_metrics_query_item(p, true, state))
            .collect();
    }
    Ok(vec![normalize_metrics_query_item(payload, false, state)?])
}

fn normalize_metrics_query_item(
    payload: OperationPayload,
    batch_mode: bool,
    state: &AppState,
) -> AppResult<MetricEventsQuerySpec> {
    let alias = payload
        .alias
        .clone()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "default".to_string());
    if batch_mode && alias == "default" {
        return Err(AppError::BadRequest(
            "batch metric_events queries require alias".to_string(),
        ));
    }
    validate_output_alias(&alias, "alias")?;

    let events = normalize_metrics_query_events(payload.event.clone(), payload.events.clone())?;
    let (range, start, end) = normalize_metric_events_range(&payload)?;
    let interval = payload.interval.clone().map(|v| v.trim().to_ascii_lowercase());
    if let Some(interval) = interval.as_deref() {
        validate_metric_events_interval(interval)?;
    }
    let groups = normalize_metric_events_groups(payload.group_by.clone())?;
    let metrics = normalize_metric_events_metrics(payload.metrics.clone())?;
    let limit = payload
        .limit
        .unwrap_or(state.metric_events_query_default_limit as i64)
        .max(1)
        .min(state.metric_events_query_max_limit as i64);
    let offset = payload.offset.unwrap_or(0).max(0);
    let label_source = payload
        .label
        .clone()
        .unwrap_or_else(|| title_from_alias(alias.as_str()));
    let label = render_metric_events_label(
        label_source.as_str(),
        &start,
        &end,
        None,
        None,
        Some(alias.as_str()),
        payload.range.as_deref(),
    );

    Ok(MetricEventsQuerySpec {
        alias,
        label,
        events,
        range,
        start,
        end,
        interval,
        bucket_label: payload.bucket_label,
        filter: payload.filter.unwrap_or_else(|| json!({})),
        groups,
        metrics,
        sort: payload.sort,
        limit,
        offset,
    })
}

fn normalize_metrics_query_events(
    event: Option<String>,
    events: Option<Vec<Value>>,
) -> AppResult<Vec<String>> {
    if event.is_some() && events.is_some() {
        return Err(AppError::BadRequest(
            "use either event or events, not both".to_string(),
        ));
    }
    let mut out = Vec::<String>::new();
    if let Some(event) = event {
        out.push(event.trim().to_string());
    } else if let Some(events) = events {
        for item in events {
            let s = item
                .as_str()
                .ok_or_else(|| AppError::BadRequest("events must be array<string>".to_string()))?
                .trim()
                .to_string();
            out.push(s);
        }
    } else {
        return Err(AppError::BadRequest("event or events is required".to_string()));
    }
    out.retain(|v| !v.is_empty());
    out.sort();
    out.dedup();
    if out.is_empty() {
        return Err(AppError::BadRequest("event/events cannot be empty".to_string()));
    }
    Ok(out)
}

fn normalize_metric_events_range(payload: &OperationPayload) -> AppResult<(Option<String>, String, String)> {
    if let Some(range) = payload.range.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        let normalized = normalize_metric_events_range_token(range)?;
        let (start, end) = resolve_metric_events_range(normalized.as_str())?;
        return Ok((
            Some(normalized),
            start.to_rfc3339_opts(SecondsFormat::Millis, true),
            end.to_rfc3339_opts(SecondsFormat::Millis, true),
        ));
    }
    let start = payload
        .start
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("range or start+end is required".to_string()))?;
    let end = payload
        .end
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("range or start+end is required".to_string()))?;
    Ok((
        None,
        parse_metric_events_datetime(start, false, "start")?,
        parse_metric_events_datetime(end, true, "end")?,
    ))
}

fn parse_metric_events_datetime(raw: &str, end_of_day: bool, field: &str) -> AppResult<String> {
    let trimmed = raw.trim();
    if trimmed.len() == 10 {
        let date = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
            .map_err(|_| AppError::BadRequest(format!("{field} must be RFC3339 or YYYY-MM-DD")))?;
        let time = if end_of_day {
            chrono::NaiveTime::from_hms_opt(23, 59, 59)
        } else {
            chrono::NaiveTime::from_hms_opt(0, 0, 0)
        }
        .ok_or_else(|| AppError::Internal("date time build failed".to_string()))?;
        let dt = chrono::DateTime::<Utc>::from_naive_utc_and_offset(date.and_time(time), Utc);
        return Ok(dt.to_rfc3339_opts(SecondsFormat::Secs, true));
    }
    let dt = chrono::DateTime::parse_from_rfc3339(trimmed)
        .map_err(|_| AppError::BadRequest(format!("{field} must be valid RFC3339 datetime")))?;
    Ok(dt
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn normalize_metric_events_range_token(raw: &str) -> AppResult<String> {
    let token = raw.trim().to_ascii_lowercase().replace('-', "_");
    match token.as_str() {
        "today" | "yesterday" | "this_week" | "last_week" | "this_month" | "last_month"
        | "this_year" | "last_year" => return Ok(token),
        _ => {}
    }

    let digit_len = token.chars().take_while(|c| c.is_ascii_digit()).count();
    if digit_len == 0 || digit_len == token.len() {
        return Err(AppError::BadRequest(
            "range must be a calendar alias or look like 24h, 7d, 3days, 2weeks, 4months"
                .to_string(),
        ));
    }
    let (num, unit) = token.split_at(digit_len);
    let n = num
        .parse::<i64>()
        .map_err(|_| AppError::BadRequest("range number is invalid".to_string()))?;
    if n <= 0 {
        return Err(AppError::BadRequest("range must be positive".to_string()));
    }
    let normalized_unit = match unit {
        "m" | "min" | "minute" | "minutes" => "minutes",
        "h" | "hour" | "hours" => "hours",
        "d" | "day" | "days" => "days",
        "w" | "week" | "weeks" => "weeks",
        "mo" | "month" | "months" => "months",
        "y" | "year" | "years" => "years",
        _ => {
            return Err(AppError::BadRequest(
                "range unit must be m|min|minutes|h|hours|d|days|w|weeks|mo|months|y|years"
                    .to_string(),
            ));
        }
    };
    Ok(format!("{n}{normalized_unit}"))
}

fn resolve_metric_events_range(
    normalized: &str,
) -> AppResult<(chrono::DateTime<Utc>, chrono::DateTime<Utc>)> {
    let now = Utc::now();
    match normalized {
        "today" => return Ok(day_bounds(now.date_naive(), 0)?),
        "yesterday" => return Ok(day_bounds(now.date_naive(), -1)?),
        "this_week" => {
            let today = now.date_naive();
            let start = today - Duration::days(today.weekday().num_days_from_monday() as i64);
            return Ok(date_span_bounds(start, start + Duration::days(6))?);
        }
        "last_week" => {
            let today = now.date_naive();
            let this_week = today - Duration::days(today.weekday().num_days_from_monday() as i64);
            let start = this_week - Duration::days(7);
            return Ok(date_span_bounds(start, start + Duration::days(6))?);
        }
        "this_month" => {
            let today = now.date_naive();
            let start = chrono::NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
                .ok_or_else(|| AppError::Internal("month start build failed".to_string()))?;
            let next = start
                .checked_add_months(chrono::Months::new(1))
                .ok_or_else(|| AppError::Internal("month end build failed".to_string()))?;
            return Ok(date_span_bounds(start, next - Duration::days(1))?);
        }
        "last_month" => {
            let today = now.date_naive();
            let this_month = chrono::NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
                .ok_or_else(|| AppError::Internal("month start build failed".to_string()))?;
            let start = this_month
                .checked_sub_months(chrono::Months::new(1))
                .ok_or_else(|| AppError::Internal("last month start build failed".to_string()))?;
            return Ok(date_span_bounds(start, this_month - Duration::days(1))?);
        }
        "this_year" => {
            let today = now.date_naive();
            let start = chrono::NaiveDate::from_ymd_opt(today.year(), 1, 1)
                .ok_or_else(|| AppError::Internal("year start build failed".to_string()))?;
            let end = chrono::NaiveDate::from_ymd_opt(today.year(), 12, 31)
                .ok_or_else(|| AppError::Internal("year end build failed".to_string()))?;
            return Ok(date_span_bounds(start, end)?);
        }
        "last_year" => {
            let today = now.date_naive();
            let year = today.year() - 1;
            let start = chrono::NaiveDate::from_ymd_opt(year, 1, 1)
                .ok_or_else(|| AppError::Internal("year start build failed".to_string()))?;
            let end = chrono::NaiveDate::from_ymd_opt(year, 12, 31)
                .ok_or_else(|| AppError::Internal("year end build failed".to_string()))?;
            return Ok(date_span_bounds(start, end)?);
        }
        _ => {}
    }

    let digit_len = normalized
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .count();
    let (num, unit) = normalized.split_at(digit_len);
    let n = num
        .parse::<i64>()
        .map_err(|_| AppError::BadRequest("range number is invalid".to_string()))?;
    let start = match unit {
        "minutes" => now - Duration::minutes(n),
        "hours" => now - Duration::hours(n),
        "days" => now - Duration::days(n),
        "weeks" => now - Duration::weeks(n),
        "months" => now
            .checked_sub_months(chrono::Months::new(n as u32))
            .ok_or_else(|| AppError::BadRequest("range months is too large".to_string()))?,
        "years" => {
            let months = n
                .checked_mul(12)
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| AppError::BadRequest("range years is too large".to_string()))?;
            now.checked_sub_months(chrono::Months::new(months))
                .ok_or_else(|| AppError::BadRequest("range years is too large".to_string()))?
        }
        _ => {
            return Err(AppError::BadRequest(
                "range unit must be minutes|hours|days|weeks|months|years".to_string(),
            ));
        }
    };
    Ok((start, now))
}

fn day_bounds(
    date: chrono::NaiveDate,
    offset_days: i64,
) -> AppResult<(chrono::DateTime<Utc>, chrono::DateTime<Utc>)> {
    let date = date + Duration::days(offset_days);
    date_span_bounds(date, date)
}

fn date_span_bounds(
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
) -> AppResult<(chrono::DateTime<Utc>, chrono::DateTime<Utc>)> {
    let start_time = chrono::NaiveTime::from_hms_opt(0, 0, 0)
        .ok_or_else(|| AppError::Internal("start time build failed".to_string()))?;
    let end_time = chrono::NaiveTime::from_hms_opt(23, 59, 59)
        .ok_or_else(|| AppError::Internal("end time build failed".to_string()))?;
    Ok((
        chrono::DateTime::<Utc>::from_naive_utc_and_offset(start.and_time(start_time), Utc),
        chrono::DateTime::<Utc>::from_naive_utc_and_offset(end.and_time(end_time), Utc),
    ))
}

fn validate_metric_events_interval(interval: &str) -> AppResult<()> {
    match interval {
        "minute" | "hour" | "day" | "week" | "month" | "year" => Ok(()),
        _ => Err(AppError::BadRequest(
            "interval must be minute|hour|day|week|month|year".to_string(),
        )),
    }
}

fn normalize_metric_events_groups(raw: Option<Value>) -> AppResult<Vec<MetricEventsGroupSpec>> {
    let Some(raw) = raw else {
        return Ok(vec![]);
    };
    let values = match raw {
        Value::String(_) | Value::Object(_) => vec![raw],
        Value::Array(arr) => arr,
        _ => {
            return Err(AppError::BadRequest(
                "group_by must be string, object, or array".to_string(),
            ));
        }
    };
    let mut out = Vec::<MetricEventsGroupSpec>::new();
    let mut seen = HashSet::<String>::new();
    for item in values {
        let (field, alias, label) = match item {
            Value::String(s) => {
                let alias = alias_from_path(s.as_str());
                let label = title_from_alias(alias.as_str());
                (s, alias, label)
            }
            Value::Object(obj) => {
                let field = obj
                    .get("field")
                    .and_then(Value::as_str)
                    .ok_or_else(|| AppError::BadRequest("group_by.field is required".to_string()))?
                    .to_string();
                let alias = obj
                    .get("alias")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| alias_from_path(field.as_str()));
                let label = obj
                    .get("label")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| title_from_alias(alias.as_str()));
                (field, alias, label)
            }
            _ => {
                return Err(AppError::BadRequest(
                    "group_by entries must be strings or objects".to_string(),
                ));
            }
        };
        validate_metric_events_field(field.as_str())?;
        validate_output_alias(alias.as_str(), "group alias")?;
        if !seen.insert(alias.clone()) {
            return Err(AppError::BadRequest(format!("duplicate group alias: {alias}")));
        }
        out.push(MetricEventsGroupSpec {
            field,
            alias,
            label,
        });
    }
    Ok(out)
}

fn normalize_metric_events_metrics(raw: Option<Value>) -> AppResult<Vec<MetricEventsMetricSpec>> {
    let raw = raw.ok_or_else(|| AppError::BadRequest("metrics is required".to_string()))?;
    let arr = raw
        .as_array()
        .ok_or_else(|| AppError::BadRequest("metrics must be array<object>".to_string()))?;
    if arr.is_empty() {
        return Err(AppError::BadRequest("metrics cannot be empty".to_string()));
    }
    let mut out = Vec::<MetricEventsMetricSpec>::new();
    let mut seen = HashSet::<String>::new();
    for item in arr {
        let obj = item
            .as_object()
            .ok_or_else(|| AppError::BadRequest("metrics entries must be objects".to_string()))?;
        let op = obj
            .get("op")
            .and_then(Value::as_str)
            .map(|v| v.trim().to_ascii_lowercase())
            .ok_or_else(|| AppError::BadRequest("metrics[].op is required".to_string()))?;
        match op.as_str() {
            "count" | "sum" | "avg" | "min" | "max" | "distinct" | "count_distinct" => {}
            _ => {
                return Err(AppError::BadRequest(format!(
                    "unsupported metric event metric op: {op}"
                )));
            }
        }
        let field = obj
            .get("field")
            .and_then(Value::as_str)
            .unwrap_or("*")
            .trim()
            .to_string();
        if op != "count" || field != "*" {
            validate_metric_events_field(field.as_str())?;
        }
        let alias = obj
            .get("alias")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("{}_{}", op, alias_from_path(field.as_str())));
        validate_output_alias(alias.as_str(), "metric alias")?;
        if !seen.insert(alias.clone()) {
            return Err(AppError::BadRequest(format!("duplicate metric alias: {alias}")));
        }
        let label = obj
            .get("label")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| title_from_alias(alias.as_str()));
        out.push(MetricEventsMetricSpec {
            op,
            field,
            alias,
            label,
        });
    }
    Ok(out)
}

async fn run_metrics_query_spec(
    conn: &libsql::Connection,
    spec: &MetricEventsQuerySpec,
) -> AppResult<Value> {
    let (where_clause, mut binds) = build_metric_events_where(spec)?;
    let mut selects = Vec::<String>::new();
    let mut group_exprs = Vec::<String>::new();
    if let Some(interval) = spec.interval.as_deref() {
        let expr = metric_events_bucket_expr(interval)?;
        selects.push(format!("{expr} AS bucket"));
        group_exprs.push(expr);
    }
    for group in &spec.groups {
        let expr = metric_events_field_expr(group.field.as_str())?;
        selects.push(format!("{expr} AS {}", quote_ident(group.alias.as_str())?));
        group_exprs.push(expr);
    }
    for metric in &spec.metrics {
        selects.push(format!(
            "{} AS {}",
            metric_events_metric_expr(metric)?,
            quote_ident(metric.alias.as_str())?
        ));
    }
    let group_by = if group_exprs.is_empty() {
        String::new()
    } else {
        format!(" GROUP BY {}", group_exprs.join(", "))
    };
    let order_by = metric_events_order_by(spec)?;
    binds.push(libsql::Value::Integer(spec.limit));
    binds.push(libsql::Value::Integer(spec.offset));
    let sql = format!(
        "SELECT {} FROM __kdb_metric_events WHERE {where_clause}{group_by} ORDER BY {order_by} LIMIT ? OFFSET ?",
        selects.join(", ")
    );

    let mut rows = conn
        .query(&sql, binds)
        .await
        .map_err(|e| AppError::Internal(format!("metrics query failed: {e}")))?;
    let mut items = Vec::<Value>::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("metric_events row read failed: {e}")))?
    {
        let mut idx = 0usize;
        let mut item = serde_json::Map::new();
        if spec.interval.is_some() {
            let bucket: Option<String> = row
                .get(idx as i32)
                .map_err(|e| AppError::Internal(format!("metric_events bucket decode failed: {e}")))?;
            idx += 1;
            let bucket = bucket.unwrap_or_default();
            item.insert("bucket".to_string(), Value::String(bucket.clone()));
            let label_template = spec
                .bucket_label
                .as_deref()
                .unwrap_or_else(|| default_bucket_label_template(spec.interval.as_deref()));
            item.insert(
                "bucket_label".to_string(),
                Value::String(render_metric_events_label(
                    label_template,
                    spec.start.as_str(),
                    spec.end.as_str(),
                    Some(bucket.as_str()),
                    None,
                    Some(spec.alias.as_str()),
                    None,
                )),
            );
        }
        let mut groups = serde_json::Map::new();
        for group in &spec.groups {
            let raw = row
                .get_value(idx as i32)
                .map_err(|e| AppError::Internal(format!("metric_events group decode failed: {e}")))?;
            idx += 1;
            groups.insert(group.alias.clone(), libsql_value_to_json(raw));
        }
        item.insert("groups".to_string(), Value::Object(groups));

        let mut metrics = serde_json::Map::new();
        for metric in &spec.metrics {
            let raw = row
                .get_value(idx as i32)
                .map_err(|e| AppError::Internal(format!("metric event metric decode failed: {e}")))?;
            idx += 1;
            let value = if metric.op == "distinct" {
                match libsql_value_to_json(raw) {
                    Value::String(s) => serde_json::from_str::<Value>(&s).unwrap_or(Value::Array(vec![])),
                    Value::Null => Value::Array(vec![]),
                    other => other,
                }
            } else {
                libsql_value_to_json(raw)
            };
            metrics.insert(metric.alias.clone(), value);
        }
        item.insert("metrics".to_string(), Value::Object(metrics));
        items.push(Value::Object(item));
    }

    Ok(json!({
        "alias": spec.alias,
        "label": spec.label,
        "range": spec.range,
        "start": spec.start,
        "end": spec.end,
        "interval": spec.interval,
        "labels": metric_events_result_labels(spec),
        "count": items.len(),
        "items": items,
        "warnings": metric_events_warnings(spec)
    }))
}

fn build_metric_events_where(spec: &MetricEventsQuerySpec) -> AppResult<(String, Vec<libsql::Value>)> {
    let mut parts = vec!["ts >= ?".to_string(), "ts <= ?".to_string()];
    let mut binds = vec![
        libsql::Value::Text(spec.start.clone()),
        libsql::Value::Text(spec.end.clone()),
    ];
    if spec.events.len() == 1 {
        parts.push("event = ?".to_string());
        binds.push(libsql::Value::Text(spec.events[0].clone()));
    } else {
        parts.push(format!("event IN ({})", vec!["?"; spec.events.len()].join(", ")));
        binds.extend(spec.events.iter().cloned().map(libsql::Value::Text));
    }
    let filter = compile_metric_events_filter(&spec.filter)?;
    if filter.sql != "1=1" {
        parts.push(format!("({})", filter.sql));
        binds.extend(filter.binds);
    }
    Ok((parts.join(" AND "), binds))
}

fn compile_metric_events_filter(filter: &Value) -> AppResult<CompiledMetricEventsWhere> {
    let obj = filter
        .as_object()
        .ok_or_else(|| AppError::BadRequest("filter must be an object".to_string()))?;
    if obj.is_empty() {
        return Ok(CompiledMetricEventsWhere {
            sql: "1=1".to_string(),
            binds: vec![],
        });
    }
    let mut parts = Vec::<String>::new();
    let mut binds = Vec::<libsql::Value>::new();
    for (key, value) in obj {
        match key.as_str() {
            "$and" | "$or" => {
                let arr = value
                    .as_array()
                    .ok_or_else(|| AppError::BadRequest(format!("{key} must be an array")))?;
                if arr.is_empty() {
                    return Err(AppError::BadRequest(format!("{key} cannot be empty")));
                }
                let mut nested = Vec::<String>::new();
                for item in arr {
                    let compiled = compile_metric_events_filter(item)?;
                    nested.push(format!("({})", compiled.sql));
                    binds.extend(compiled.binds);
                }
                parts.push(nested.join(if key == "$and" { " AND " } else { " OR " }));
            }
            _ => compile_metric_events_filter_field(key, value, &mut parts, &mut binds)?,
        }
    }
    Ok(CompiledMetricEventsWhere {
        sql: parts.join(" AND "),
        binds,
    })
}

struct CompiledMetricEventsWhere {
    sql: String,
    binds: Vec<libsql::Value>,
}

fn compile_metric_events_filter_field(
    field: &str,
    value: &Value,
    parts: &mut Vec<String>,
    binds: &mut Vec<libsql::Value>,
) -> AppResult<()> {
    let expr = metric_events_field_expr(field)?;
    if let Some(obj) = value.as_object() {
        for (op, operand) in obj {
            match op.as_str() {
                "$eq" => {
                    parts.push(format!("{expr} = ?"));
                    binds.push(json_scalar_to_sql_value(operand)?);
                }
                "$ne" => {
                    parts.push(format!("{expr} != ?"));
                    binds.push(json_scalar_to_sql_value(operand)?);
                }
                "$gt" | "$gte" | "$lt" | "$lte" => {
                    let sql_op = match op.as_str() {
                        "$gt" => ">",
                        "$gte" => ">=",
                        "$lt" => "<",
                        "$lte" => "<=",
                        _ => unreachable!(),
                    };
                    parts.push(format!("{expr} {sql_op} ?"));
                    binds.push(json_scalar_to_sql_value(operand)?);
                }
                "$in" | "$nin" => {
                    let arr = operand
                        .as_array()
                        .ok_or_else(|| AppError::BadRequest(format!("{op} must be an array")))?;
                    if arr.is_empty() {
                        return Err(AppError::BadRequest(format!("{op} cannot be empty")));
                    }
                    let placeholders = vec!["?"; arr.len()].join(", ");
                    parts.push(format!(
                        "{expr} {} ({placeholders})",
                        if op == "$in" { "IN" } else { "NOT IN" }
                    ));
                    for item in arr {
                        binds.push(json_scalar_to_sql_value(item)?);
                    }
                }
                "$between" => {
                    let arr = operand
                        .as_array()
                        .ok_or_else(|| AppError::BadRequest("$between must be an array".to_string()))?;
                    if arr.len() != 2 {
                        return Err(AppError::BadRequest(
                            "$between requires exactly 2 values".to_string(),
                        ));
                    }
                    parts.push(format!("{expr} BETWEEN ? AND ?"));
                    binds.push(json_scalar_to_sql_value(&arr[0])?);
                    binds.push(json_scalar_to_sql_value(&arr[1])?);
                }
                _ => {
                    return Err(AppError::BadRequest(format!(
                        "unsupported metric_events filter operator: {op}"
                    )));
                }
            }
        }
        return Ok(());
    }
    parts.push(format!("{expr} = ?"));
    binds.push(json_scalar_to_sql_value(value)?);
    Ok(())
}

fn metric_events_bucket_expr(interval: &str) -> AppResult<String> {
    Ok(match interval {
        "minute" => "strftime('%Y-%m-%dT%H:%M:00Z', ts)".to_string(),
        "hour" => "strftime('%Y-%m-%dT%H:00:00Z', ts)".to_string(),
        "day" => "strftime('%Y-%m-%dT00:00:00Z', ts)".to_string(),
        "week" => "strftime('%Y-W%W', ts)".to_string(),
        "month" => "strftime('%Y-%m', ts)".to_string(),
        "year" => "strftime('%Y', ts)".to_string(),
        _ => return Err(AppError::BadRequest("invalid interval".to_string())),
    })
}

fn metric_events_metric_expr(metric: &MetricEventsMetricSpec) -> AppResult<String> {
    if metric.op == "count" && metric.field == "*" {
        return Ok("COUNT(*)".to_string());
    }
    let expr = metric_events_field_expr(metric.field.as_str())?;
    Ok(match metric.op.as_str() {
        "count" => format!("COUNT({expr})"),
        "sum" => format!("SUM(CAST({expr} AS REAL))"),
        "avg" => format!("AVG(CAST({expr} AS REAL))"),
        "min" => format!("MIN(CAST({expr} AS REAL))"),
        "max" => format!("MAX(CAST({expr} AS REAL))"),
        "distinct" => format!("json_group_array(DISTINCT {expr})"),
        "count_distinct" => format!("COUNT(DISTINCT {expr})"),
        _ => return Err(AppError::BadRequest("invalid metric op".to_string())),
    })
}

fn metric_events_field_expr(field: &str) -> AppResult<String> {
    validate_metric_events_field(field)?;
    match field {
        "event" | "ts" | "tenant_id" | "user_id" | "value" => Ok(field.to_string()),
        _ if field.starts_with("dimensions.") => Ok(format!(
            "json_extract(dimensions, '{}')",
            sql_json_path(field.trim_start_matches("dimensions."))?
        )),
        _ if field.starts_with("metadata.") => Ok(format!(
            "json_extract(metadata, '{}')",
            sql_json_path(field.trim_start_matches("metadata."))?
        )),
        _ => Err(AppError::BadRequest(format!(
            "metric event field must be event|ts|tenant_id|user_id|value|dimensions.*|metadata.*: {field}"
        ))),
    }
}

fn metric_events_order_by(spec: &MetricEventsQuerySpec) -> AppResult<String> {
    let Some(sort) = spec.sort.as_ref() else {
        return Ok(if spec.interval.is_some() {
            "bucket ASC".to_string()
        } else {
            "1 ASC".to_string()
        });
    };
    let mut allowed = HashSet::<String>::new();
    if spec.interval.is_some() {
        allowed.insert("bucket".to_string());
    }
    for group in &spec.groups {
        allowed.insert(group.alias.clone());
    }
    for metric in &spec.metrics {
        allowed.insert(metric.alias.clone());
    }
    let pairs = match sort {
        Value::String(s) => parse_sort_string(s)?,
        Value::Object(obj) => {
            let mut out = Vec::<(String, &'static str)>::new();
            for (k, v) in obj {
                out.push((k.clone(), parse_sort_dir(v)?));
            }
            out
        }
        _ => {
            return Err(AppError::BadRequest(
                "sort must be string or object".to_string(),
            ));
        }
    };
    let mut parts = Vec::<String>::new();
    for (name, dir) in pairs {
        if !allowed.contains(&name) {
            return Err(AppError::BadRequest(format!(
                "metric_events sort field is not in bucket/group aliases/metric aliases: {name}"
            )));
        }
        parts.push(format!("{} {dir}", quote_ident(name.as_str())?));
    }
    Ok(parts.join(", "))
}

fn metric_events_result_labels(spec: &MetricEventsQuerySpec) -> Value {
    let mut groups = serde_json::Map::new();
    if spec.interval.is_some() {
        groups.insert("bucket".to_string(), Value::String("Bucket".to_string()));
        groups.insert(
            "bucket_label".to_string(),
            Value::String("Bucket Label".to_string()),
        );
    }
    for group in &spec.groups {
        groups.insert(group.alias.clone(), Value::String(group.label.clone()));
    }
    let mut metrics = serde_json::Map::new();
    for metric in &spec.metrics {
        metrics.insert(metric.alias.clone(), Value::String(metric.label.clone()));
    }
    json!({
        "groups": Value::Object(groups),
        "metrics": Value::Object(metrics)
    })
}

fn metric_events_warnings(spec: &MetricEventsQuerySpec) -> Vec<Value> {
    if spec.events.len() > 1 && !spec.groups.iter().any(|g| g.field == "event") {
        return vec![json!({
            "code": "multiple_events_without_event_group",
            "message": "multiple events are aggregated together; add group_by:\"event\" to split by event"
        })];
    }
    vec![]
}

fn validate_metric_events_field(path: &str) -> AppResult<()> {
    if path == "*" {
        return Ok(());
    }
    if path.trim().is_empty() || path.contains("[]") {
        return Err(AppError::BadRequest(format!("invalid metric event field: {path}")));
    }
    for segment in path.split('.') {
        if segment.is_empty()
            || !segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(AppError::BadRequest(format!(
                "invalid metric event field segment: {segment}"
            )));
        }
    }
    Ok(())
}

fn validate_output_alias(alias: &str, field_name: &str) -> AppResult<()> {
    if alias.trim().is_empty() {
        return Err(AppError::BadRequest(format!("{field_name} cannot be empty")));
    }
    if !alias
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(AppError::BadRequest(format!(
            "{field_name} must contain only letters, digits, and underscore"
        )));
    }
    Ok(())
}

fn quote_ident(name: &str) -> AppResult<String> {
    validate_output_alias(name, "identifier")?;
    Ok(format!("\"{}\"", name.replace('"', "\"\"")))
}

fn alias_from_path(path: &str) -> String {
    if path == "*" {
        return "all".to_string();
    }
    path.split('.')
        .last()
        .filter(|v| !v.is_empty())
        .unwrap_or(path)
        .replace('-', "_")
}

fn title_from_alias(alias: &str) -> String {
    alias
        .split('_')
        .filter(|v| !v.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn default_bucket_label_template(interval: Option<&str>) -> &'static str {
    match interval {
        Some("minute") | Some("hour") => "{{bucket HH:mm}}",
        Some("day") => "{{bucket YYYY-MM-DD}}",
        Some("week") => "{{bucket YYYY-[W]WW}}",
        Some("month") => "{{bucket YYYY-MM}}",
        Some("year") => "{{bucket YYYY}}",
        _ => "{{bucket}}",
    }
}

fn render_metric_events_label(
    template: &str,
    start: &str,
    end: &str,
    bucket: Option<&str>,
    event: Option<&str>,
    alias: Option<&str>,
    range: Option<&str>,
) -> String {
    let mut out = template.to_string();
    for token in ["start", "end", "bucket"] {
        let value = match token {
            "start" => Some(start),
            "end" => Some(end),
            "bucket" => bucket,
            _ => None,
        };
        if let Some(value) = value {
            out = render_date_token(out, token, value);
        }
    }
    if let Some(event) = event {
        out = out.replace("{{event}}", event);
    }
    if let Some(alias) = alias {
        out = out.replace("{{alias}}", alias);
    }
    if let Some(range) = range {
        out = out.replace("{{range}}", range);
    }
    out
}

fn render_date_token(mut input: String, token: &str, value: &str) -> String {
    let plain = format!("{{{{{token}}}}}");
    input = input.replace(plain.as_str(), value);
    let formats = [
        ("YYYY-MM-DD HH:mm", "%Y-%m-%d %H:%M"),
        ("YYYY-MM-DD", "%Y-%m-%d"),
        ("YYYY-MM", "%Y-%m"),
        ("YYYY", "%Y"),
        ("HH:mm", "%H:%M"),
        ("YYYY-[W]WW", "%Y-W%W"),
    ];
    for (macro_fmt, chrono_fmt) in formats {
        let needle = format!("{{{{{token} {macro_fmt}}}}}");
        if input.contains(needle.as_str()) {
            let rendered = chrono::DateTime::parse_from_rfc3339(value)
                .map(|dt| dt.with_timezone(&Utc).format(chrono_fmt).to_string())
                .unwrap_or_else(|_| value.to_string());
            input = input.replace(needle.as_str(), rendered.as_str());
        }
    }
    input
}

fn json_scalar_to_sql_value(v: &Value) -> AppResult<libsql::Value> {
    match v {
        Value::Null => Ok(libsql::Value::Null),
        Value::Bool(b) => Ok(libsql::Value::Integer(if *b { 1 } else { 0 })),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(libsql::Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(libsql::Value::Real(f))
            } else {
                Err(AppError::BadRequest("unsupported numeric value".to_string()))
            }
        }
        Value::String(s) => Ok(libsql::Value::Text(s.clone())),
        Value::Array(_) | Value::Object(_) => Err(AppError::BadRequest(
            "filter scalar comparisons do not support array/object values".to_string(),
        )),
    }
}

fn metric_events_cache_key(
    state: &AppState,
    db_path: &str,
    payload: &OperationPayload,
) -> AppResult<Option<CacheKeyMeta>> {
    let policy = parse_metric_events_cache_policy(state, payload)?;
    let Some(policy) = policy else {
        return Ok(None);
    };
    if matches!(policy, CachePolicy::Invalidate) {
        state.bump_broadcast_epoch(db_path);
        return Ok(None);
    }

    let mut key_payload = payload.clone();
    key_payload.cache = None;
    let payload_json = serde_json::to_string(&key_payload)
        .map_err(|e| AppError::Internal(format!("metric_events cache key serialize failed: {e}")))?;
    let epoch = state.current_broadcast_epoch(db_path);
    Ok(Some(CacheKeyMeta {
        key: format!("{db_path}|metric_events|b:{epoch}|metrics_query|{payload_json}"),
        policy,
    }))
}

fn parse_metric_events_cache_policy(
    state: &AppState,
    payload: &OperationPayload,
) -> AppResult<Option<CachePolicy>> {
    match payload.cache.as_ref() {
        None => {
            if state.metric_events_cache_enabled {
                Ok(Some(CachePolicy::CustomTtl(
                    state.metric_events_cache_ttl_secs.max(1),
                )))
            } else {
                Ok(None)
            }
        }
        Some(CacheHint::Bool(false)) => Ok(None),
        Some(CacheHint::Bool(true)) => Ok(Some(CachePolicy::CustomTtl(
            state.metric_events_cache_ttl_secs.max(1),
        ))),
        Some(CacheHint::Int(-1)) => Ok(Some(CachePolicy::Invalidate)),
        Some(CacheHint::Int(v)) if *v <= 0 => Ok(None),
        Some(CacheHint::Int(1)) => Ok(Some(CachePolicy::CustomTtl(
            state.metric_events_cache_ttl_secs.max(1),
        ))),
        Some(CacheHint::Int(v)) => {
            let ttl = u64::try_from(*v).map_err(|_| {
                AppError::BadRequest("cache must be false|0 or true|1+ seconds".to_string())
            })?;
            Ok(Some(CachePolicy::CustomTtl(ttl)))
        }
    }
}
