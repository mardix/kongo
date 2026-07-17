// Query compute, aggregation, projection, and sort helpers extracted from dispatcher.rs.

fn build_where_with_collection(
    filter: Value,
    collection: Option<String>,
) -> AppResult<(String, Vec<libsql::Value>)> {
    let compiled = build_where(&filter)?;
    let mut bind_values: Vec<libsql::Value> = Vec::new();
    let mut where_clause = format!("({})", compiled.sql);

    if let Some(collection) = collection {
        where_clause = format!("collection = ? AND {where_clause}");
        bind_values.push(libsql::Value::Text(collection));
    }
    bind_values.extend(compiled.binds);

    Ok((where_clause, bind_values))
}

fn apply_document_user_scope(
    where_clause: &mut String,
    bind_values: &mut Vec<libsql::Value>,
    user_id: Option<String>,
) {
    if let Some(user_id) = user_id {
        *where_clause = format!("_user_id = ? AND ({where_clause})");
        bind_values.insert(0, libsql::Value::Text(user_id));
    }
}

async fn execute_count(
    conn: &libsql::Connection,
    source: &str,
    where_clause: &str,
    binds: Vec<libsql::Value>,
) -> AppResult<i64> {
    let sql = format!("SELECT COUNT(*) FROM {source} WHERE {where_clause}");
    let mut rows = conn
        .query(&sql, binds)
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
        Ok(total)
    } else {
        Ok(0)
    }
}


async fn run_aggregations_sql(
    conn: &libsql::Connection,
    source: &str,
    spec: Value,
    base_filter: Option<Value>,
    collection: Option<String>,
) -> AppResult<Value> {
    let spec_obj = spec
        .as_object()
        .ok_or_else(|| AppError::BadRequest("compute must be an object".to_string()))?;
    if spec_obj.is_empty() {
        return Err(AppError::BadRequest("compute cannot be empty".to_string()));
    }

    let mut out = serde_json::Map::<String, Value>::new();
    for (name, def) in spec_obj {
        let metric = parse_compute_metric(def)?;
        let metric_filter = combine_filters(base_filter.clone(), metric.filter.clone());
        let (where_clause, binds) = build_where_with_collection(
            metric_filter.unwrap_or_else(|| json!({})),
            collection.clone(),
        )?;
        if metric.op == "$distinct" {
            let path = expect_agg_path(metric.op.as_str(), &metric.arg)?;
            let docs = fetch_docs_for_compute(conn, source, &where_clause, binds).await?;
            let mut values = if path.contains("[]") {
                let mut out = Vec::<Value>::new();
                for doc in &docs {
                    out.extend(extract_values_by_path(doc, path));
                }
                out
            } else {
                let mut out = Vec::<Value>::new();
                for doc in &docs {
                    if let Some(v) = value_by_path(doc, path) {
                        out.push(v.clone());
                    }
                }
                out
            };
            values = distinct_json_values(values)?;
            out.insert(name.clone(), Value::Array(values));
            continue;
        }
        let expr = agg_expr_sql_metric(&metric)?;
        let sql = format!("SELECT {expr} FROM {source} WHERE {where_clause}");
        let mut rows = conn
            .query(&sql, binds)
            .await
            .map_err(|e| AppError::Internal(format!("compute sql query failed: {e}")))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("compute sql row read failed: {e}")))?
        else {
            out.insert(name.clone(), Value::Null);
            continue;
        };
        let val = decode_sql_aggregate_cell(&row)?;
        out.insert(name.clone(), val);
    }
    Ok(Value::Object(out))
}

async fn fetch_docs_for_compute(
    conn: &libsql::Connection,
    source: &str,
    where_clause: &str,
    binds: Vec<libsql::Value>,
) -> AppResult<Vec<Value>> {
    let sql = format!("SELECT json(data) FROM {source} WHERE {where_clause}");
    let mut rows = conn
        .query(&sql, binds)
        .await
        .map_err(|e| AppError::Internal(format!("compute fetch query failed: {e}")))?;
    let mut docs = Vec::<Value>::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("compute fetch row read failed: {e}")))?
    {
        let raw: String = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("compute fetch row decode failed: {e}")))?;
        let doc = serde_json::from_str::<Value>(&raw)
            .map_err(|e| AppError::Internal(format!("compute fetch json decode failed: {e}")))?;
        docs.push(doc);
    }
    Ok(docs)
}

#[derive(Clone)]
struct ComputeMetric {
    op: String,
    arg: Value,
    distinct: bool,
    filter: Option<Value>,
}

fn parse_compute_metric(def: &Value) -> AppResult<ComputeMetric> {
    let obj = def
        .as_object()
        .ok_or_else(|| AppError::BadRequest("compute definition must be object".to_string()))?;
    let mut op_key: Option<String> = None;
    let mut op_arg: Option<Value> = None;
    for (k, v) in obj {
        if !k.starts_with('$') || k == "$filter" {
            continue;
        }
        // "$distinct" can be either:
        // 1) modifier: {"$count":"path","$distinct":true}
        // 2) operator: {"$distinct":"path[]"}
        if k == "$distinct" && v.is_boolean() {
            continue;
        }
        if op_key.is_some() {
            return Err(AppError::BadRequest(
                "compute definition must include exactly one operator".to_string(),
            ));
        }
        op_key = Some(k.clone());
        op_arg = Some(v.clone());
    }
    let op =
        op_key.ok_or_else(|| AppError::BadRequest("compute definition missing operator".to_string()))?;
    let arg = op_arg.ok_or_else(|| AppError::BadRequest("compute argument missing".to_string()))?;
    let distinct = obj
        .get("$distinct")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let filter = obj.get("$filter").cloned();
    Ok(ComputeMetric {
        op,
        arg,
        distinct,
        filter,
    })
}

fn agg_expr_sql_metric(metric: &ComputeMetric) -> AppResult<String> {
    let expr = match metric.op.as_str() {
        "$distinct" => {
            return Err(AppError::BadRequest(
                "$distinct in aggregate uses application mode".to_string(),
            ));
        }
        "$count" => {
            if metric.arg == "*" {
                if metric.distinct {
                    "COUNT(DISTINCT id)".to_string()
                } else {
                    "COUNT(*)".to_string()
                }
            } else {
                let path = expect_agg_path(metric.op.as_str(), &metric.arg)?;
                let arg = format!("json_extract(data, '{}')", sql_json_path(path)?);
                if metric.distinct {
                    format!("COUNT(DISTINCT {arg})")
                } else {
                    format!("COUNT({arg})")
                }
            }
        }
        "$sum" => {
            let path = expect_agg_path(metric.op.as_str(), &metric.arg)?;
            let arg = format!("CAST(json_extract(data, '{}') AS REAL)", sql_json_path(path)?);
            if metric.distinct {
                format!("SUM(DISTINCT {arg})")
            } else {
                format!("SUM({arg})")
            }
        }
        "$avg" => {
            let path = expect_agg_path(metric.op.as_str(), &metric.arg)?;
            let arg = format!("CAST(json_extract(data, '{}') AS REAL)", sql_json_path(path)?);
            if metric.distinct {
                format!("AVG(DISTINCT {arg})")
            } else {
                format!("AVG({arg})")
            }
        }
        "$min" => {
            let path = expect_agg_path(metric.op.as_str(), &metric.arg)?;
            format!("MIN(CAST(json_extract(data, '{}') AS REAL))", sql_json_path(path)?)
        }
        "$max" => {
            let path = expect_agg_path(metric.op.as_str(), &metric.arg)?;
            format!("MAX(CAST(json_extract(data, '{}') AS REAL))", sql_json_path(path)?)
        }
        _ => {
            return Err(AppError::BadRequest(format!(
                "unsupported compute operator: {}",
                metric.op
            )));
        }
    };
    Ok(expr)
}

fn combine_filters(base: Option<Value>, extra: Option<Value>) -> Option<Value> {
    match (base, extra) {
        (None, None) => None,
        (Some(v), None) => Some(v),
        (None, Some(v)) => Some(v),
        (Some(a), Some(b)) => Some(json!({"$and":[a,b]})),
    }
}

fn decode_sql_aggregate_cell(row: &libsql::Row) -> AppResult<Value> {
    if let Ok(v) = row.get::<Option<f64>>(0) {
        return Ok(v.map_or(Value::Null, |n| json!(n)));
    }
    if let Ok(v) = row.get::<Option<i64>>(0) {
        return Ok(v.map_or(Value::Null, |n| json!(n)));
    }
    if let Ok(v) = row.get::<Option<String>>(0) {
        return Ok(v.map_or(Value::Null, Value::String));
    }
    Ok(Value::Null)
}

fn sql_json_path(path: &str) -> AppResult<String> {
    if !is_observable_path(path) {
        return Err(AppError::BadRequest(format!("invalid field path: {path}")));
    }
    Ok(format!("$.{}", path))
}

fn apply_row_compute_to_items(items: &mut [Value], spec: &Value) -> AppResult<()> {
    let spec_obj = spec
        .as_object()
        .ok_or_else(|| AppError::BadRequest("compute must be object".to_string()))?;
    for item in items {
        let obj = item
            .as_object_mut()
            .ok_or_else(|| AppError::Internal("query item must be object".to_string()))?;
        let snapshot = Value::Object(obj.clone());
        for (name, def) in spec_obj {
            let metric = parse_compute_metric(def)?;
            let v = eval_row_compute(&metric, &snapshot)?;
            obj.insert(name.clone(), v);
        }
    }
    Ok(())
}

fn eval_row_compute(metric: &ComputeMetric, doc: &Value) -> AppResult<Value> {
    if metric.op == "$join" {
        return eval_row_join(&metric.arg, doc);
    }
    if metric.op == "$size" {
        let path = metric
            .arg
            .as_str()
            .ok_or_else(|| AppError::BadRequest("$size argument must be a field path".to_string()))?;
        if path.contains("[]") {
            let values = extract_values_by_path(doc, path);
            return Ok(json!(values.len() as i64));
        }
        return Ok(match value_by_path(doc, path) {
            Some(Value::Array(arr)) => json!(arr.len() as i64),
            Some(Value::Object(obj)) => json!(obj.len() as i64),
            Some(Value::String(s)) => json!(s.chars().count() as i64),
            Some(_) => Value::Null,
            None => Value::Null,
        });
    }

    let path = metric
        .arg
        .as_str()
        .ok_or_else(|| AppError::BadRequest(format!("{} argument must be a field path", metric.op)))?;
    let (mut values, is_array_source) = row_compute_values(doc, path, metric.filter.as_ref());
    if !is_array_source {
        return Ok(Value::Null);
    }
    if metric.distinct || metric.op == "$distinct" {
        values = distinct_json_values(values)?;
    }
    match metric.op.as_str() {
        "$count" => Ok(json!(values.len() as i64)),
        "$distinct" => Ok(Value::Array(values)),
        "$sum" => {
            let mut sum = 0.0_f64;
            let mut found = false;
            for v in values {
                if let Some(n) = as_f64(&v) {
                    sum += n;
                    found = true;
                }
            }
            if !found {
                Ok(json!(0))
            } else {
                Ok(json!(sum))
            }
        }
        "$avg" => {
            let mut sum = 0.0_f64;
            let mut count = 0.0_f64;
            for v in values {
                if let Some(n) = as_f64(&v) {
                    sum += n;
                    count += 1.0;
                }
            }
            if count == 0.0 {
                Ok(Value::Null)
            } else {
                Ok(json!(sum / count))
            }
        }
        "$min" => {
            let mut min_v: Option<f64> = None;
            for v in values {
                if let Some(n) = as_f64(&v) {
                    min_v = Some(match min_v {
                        Some(cur) => cur.min(n),
                        None => n,
                    });
                }
            }
            Ok(min_v.map_or(Value::Null, |v| json!(v)))
        }
        "$max" => {
            let mut max_v: Option<f64> = None;
            for v in values {
                if let Some(n) = as_f64(&v) {
                    max_v = Some(match max_v {
                        Some(cur) => cur.max(n),
                        None => n,
                    });
                }
            }
            Ok(max_v.map_or(Value::Null, |v| json!(v)))
        }
        _ => Err(AppError::BadRequest(format!(
            "unsupported row compute operator: {}",
            metric.op
        ))),
    }
}

fn distinct_json_values(values: Vec<Value>) -> AppResult<Vec<Value>> {
    let mut out = Vec::<Value>::new();
    let mut seen = HashSet::<String>::new();
    for v in values {
        let key = serde_json::to_string(&v)
            .map_err(|e| AppError::Internal(format!("compute distinct encode failed: {e}")))?;
        if seen.insert(key) {
            out.push(v);
        }
    }
    Ok(out)
}

fn eval_row_join(arg: &Value, doc: &Value) -> AppResult<Value> {
    let parts = arg
        .as_array()
        .ok_or_else(|| AppError::BadRequest("$join argument must be an array".to_string()))?;
    let mut out = String::new();
    for part in parts {
        match part {
            Value::String(s) if s.starts_with('$') => {
                let path = s.trim_start_matches('$');
                if let Some(v) = value_by_path(doc, path) {
                    out.push_str(&value_to_join_string(v));
                }
            }
            Value::String(s) => out.push_str(s),
            other => out.push_str(&value_to_join_string(other)),
        }
    }
    Ok(Value::String(out))
}

fn value_to_join_string(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(v).unwrap_or_default(),
    }
}

fn row_compute_values(doc: &Value, path: &str, metric_filter: Option<&Value>) -> (Vec<Value>, bool) {
    if path.contains("[]") {
        let mut values = extract_values_by_path(doc, path);
        if let Some(f) = metric_filter {
            values.retain(|v| row_filter_matches(v, f));
        }
        return (values, true);
    }
    let Some(v) = value_by_path(doc, path) else {
        return (vec![], false);
    };
    if let Some(arr) = v.as_array() {
        let mut values = arr.clone();
        if let Some(f) = metric_filter {
            values.retain(|v| row_filter_matches(v, f));
        }
        return (values, true);
    }
    (vec![v.clone()], false)
}

fn row_filter_matches(candidate: &Value, filter: &Value) -> bool {
    let Some(obj) = filter.as_object() else {
        return false;
    };
    for (k, v) in obj {
        if k == "$and" {
            let Some(arr) = v.as_array() else {
                return false;
            };
            if !arr.iter().all(|f| row_filter_matches(candidate, f)) {
                return false;
            }
            continue;
        }
        if k == "$or" {
            let Some(arr) = v.as_array() else {
                return false;
            };
            if !arr.iter().any(|f| row_filter_matches(candidate, f)) {
                return false;
            }
            continue;
        }
        let Some(actual) = candidate.get(k) else {
            return false;
        };
        if let Some(spec) = v.as_object() {
            for (op, expected) in spec {
                let ok = match op.as_str() {
                    "$eq" => actual == expected,
                    "$ne" => actual != expected,
                    "$in" => expected
                        .as_array()
                        .map(|arr| arr.iter().any(|x| x == actual))
                        .unwrap_or(false),
                    "$nin" => expected
                        .as_array()
                        .map(|arr| arr.iter().all(|x| x != actual))
                        .unwrap_or(false),
                    _ => false,
                };
                if !ok {
                    return false;
                }
            }
        } else if actual != v {
            return false;
        }
    }
    true
}

fn expect_agg_path<'a>(op: &str, arg: &'a Value) -> AppResult<&'a str> {
    arg.as_str()
        .ok_or_else(|| AppError::BadRequest(format!("{op} argument must be a field path")))
}

fn value_by_path<'a>(doc: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = doc;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

fn json_input_expr(jsonb_enabled: bool) -> &'static str {
    if jsonb_enabled { "jsonb(?)" } else { "json(?)" }
}

fn json_patch_expr(jsonb_enabled: bool) -> &'static str {
    if jsonb_enabled {
        "jsonb(json_patch(data, ?))"
    } else {
        "json_patch(data, ?)"
    }
}

fn jsonb_enabled() -> bool {
    true
}

fn strict_mutation_operators_env() -> bool {
    std::env::var("KONGODB_STRICT_MUTATIONS_OPERATORS")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn apply_projection(
    doc: &Value,
    fields: &Option<Vec<String>>,
    exclude_fields: &Option<Vec<String>>,
) -> AppResult<Value> {
    if !doc.is_object() {
        return Ok(doc.clone());
    }

    let mut out = if let Some(include_paths) = fields {
        if include_paths.is_empty() {
            return Err(AppError::BadRequest(
                "fields cannot be empty when provided".to_string(),
            ));
        }
        let mut obj = serde_json::Map::new();
        if let Some(id) = doc.get("_id") {
            obj.insert("_id".to_string(), id.clone());
        }
        if let Some(user_id) = doc.get("_user_id") {
            obj.insert("_user_id".to_string(), user_id.clone());
        }
        for path in include_paths {
            validate_projection_path(path, "fields")?;
            if path == "_id" || path == "_user_id" {
                continue;
            }
            if let Some(v) = value_by_path(doc, path) {
                set_value_by_path(&mut obj, path, v.clone())?;
            }
        }
        Value::Object(obj)
    } else {
        doc.clone()
    };

    if let Some(excludes) = exclude_fields {
        if excludes.is_empty() {
            return Err(AppError::BadRequest(
                "exclude_fields cannot be empty when provided".to_string(),
            ));
        }
        for path in excludes {
            validate_projection_path(path, "exclude_fields")?;
            if path == "_id" || path == "_user_id" {
                continue;
            }
            remove_value_by_path(&mut out, path)?;
        }
        if let Some(id) = doc.get("_id") {
            if let Some(obj) = out.as_object_mut() {
                obj.insert("_id".to_string(), id.clone());
            }
        }
        if let Some(user_id) = doc.get("_user_id") {
            if let Some(obj) = out.as_object_mut() {
                obj.insert("_user_id".to_string(), user_id.clone());
            }
        }
    }

    Ok(out)
}

fn attach_system_timestamps(
    doc: &mut Value,
    created_at: Option<String>,
    modified_at: Option<String>,
) {
    let Some(obj) = doc.as_object_mut() else {
        return;
    };
    if let Some(created) = created_at {
        obj.insert("_created_at".to_string(), Value::String(created));
    }
    if let Some(modified) = modified_at {
        obj.insert("_modified_at".to_string(), Value::String(modified));
    }
}

fn attach_document_user_id(doc: &mut Value, user_id: Option<String>) {
    let Some(user_id) = user_id else {
        return;
    };
    let Some(obj) = doc.as_object_mut() else {
        return;
    };
    obj.insert("_user_id".to_string(), Value::String(user_id));
}

fn normalize_document_user_id_from_doc(
    doc: &mut Value,
    payload_user_id: Option<&str>,
) -> AppResult<Option<String>> {
    let doc_user_id = match doc.as_object_mut().and_then(|obj| obj.remove("_user_id")) {
        Some(Value::String(v)) => clean_optional(Some(v)),
        Some(Value::Null) | None => None,
        Some(_) => {
            return Err(AppError::BadRequest(
                "_user_id must be a non-empty string when provided".to_string(),
            ));
        }
    };
    let payload_user_id = payload_user_id.and_then(|v| clean_optional(Some(v.to_string())));
    if let (Some(a), Some(b)) = (&payload_user_id, &doc_user_id) {
        if a != b {
            return Err(AppError::BadRequest(
                "payload._user_id and data._user_id cannot differ".to_string(),
            ));
        }
    }
    Ok(payload_user_id.or(doc_user_id))
}

async fn attach_users_if_requested(
    conn: &libsql::Connection,
    data: &mut Value,
    payload: &OperationPayload,
) -> AppResult<()> {
    if !payload.attach_users.unwrap_or(false) {
        return Ok(());
    }
    let Some(items) = data.get("items").and_then(Value::as_array) else {
        return Ok(());
    };
    let users = load_user_attachments(conn, items, payload.attach_user_fields.clone()).await?;
    if users.is_empty() {
        return Ok(());
    }
    let Some(obj) = data.as_object_mut() else {
        return Ok(());
    };
    let attachments = obj
        .entry("attachments".to_string())
        .or_insert_with(|| json!({}));
    let Some(attachments_obj) = attachments.as_object_mut() else {
        return Ok(());
    };
    attachments_obj.insert("users".to_string(), Value::Object(users));
    Ok(())
}

async fn load_user_attachments(
    conn: &libsql::Connection,
    items: &[Value],
    fields: Option<Vec<String>>,
) -> AppResult<serde_json::Map<String, Value>> {
    let mut ids = items
        .iter()
        .filter_map(|item| item.get("_user_id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Ok(serde_json::Map::new());
    }
    let fields = normalize_attach_user_fields(fields)?;
    let placeholders = vec!["?"; ids.len()].join(", ");
    let binds = ids
        .iter()
        .map(|id| libsql::Value::Text(id.clone()))
        .collect::<Vec<_>>();
    let mut rows = conn
        .query(
            &format!(
                "SELECT {} FROM __kdb_identity_users WHERE id IN ({placeholders})",
                identity_user_select()
            ),
            binds,
        )
        .await
        .map_err(|e| AppError::Internal(format!("attach users query failed: {e}")))?;
    let mut users = serde_json::Map::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("attach users row read failed: {e}")))?
    {
        let user = identity_user_from_row(&row)?;
        let id = user
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Internal("attached user missing id".to_string()))?
            .to_string();
        users.insert(id, project_attached_user(&user, &fields)?);
    }
    Ok(users)
}

fn normalize_attach_user_fields(fields: Option<Vec<String>>) -> AppResult<Vec<String>> {
    let fields = fields.unwrap_or_else(|| {
        vec![
            "id".to_string(),
            "first_name".to_string(),
            "last_name".to_string(),
            "profile_photo".to_string(),
        ]
    });
    if fields.is_empty() {
        return Err(AppError::BadRequest(
            "attach_user_fields cannot be empty when provided".to_string(),
        ));
    }
    let mut out = Vec::new();
    for field in fields {
        let field = field.trim().to_string();
        if field.is_empty() {
            return Err(AppError::BadRequest(
                "attach_user_fields contains empty field".to_string(),
            ));
        }
        if matches!(field.as_str(), "password_hash" | "token_hash") {
            return Err(AppError::BadRequest(format!(
                "attach_user_fields cannot include sensitive field: {field}"
            )));
        }
        validate_projection_path(&field, "attach_user_fields")?;
        out.push(field);
    }
    Ok(out)
}

fn project_attached_user(user: &Value, fields: &[String]) -> AppResult<Value> {
    let mut out = serde_json::Map::new();
    for field in fields {
        if let Some(value) = value_by_path(user, field) {
            set_value_by_path(&mut out, field, value.clone())?;
        }
    }
    if !out.contains_key("id") {
        if let Some(id) = user.get("id") {
            out.insert("id".to_string(), id.clone());
        }
    }
    Ok(Value::Object(out))
}

fn validate_projection_path(path: &str, field_name: &str) -> AppResult<()> {
    if path.trim().is_empty() {
        return Err(AppError::BadRequest(format!(
            "{field_name} contains empty path"
        )));
    }
    if path.split('.').any(|p| p.trim().is_empty()) {
        return Err(AppError::BadRequest(format!(
            "{field_name} contains invalid path: {path}"
        )));
    }
    Ok(())
}

fn set_value_by_path(
    out: &mut serde_json::Map<String, Value>,
    path: &str,
    value: Value,
) -> AppResult<()> {
    let mut parts = path.split('.').peekable();
    let mut current = out;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            current.insert(part.to_string(), value);
            return Ok(());
        }
        let next = current
            .entry(part.to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !next.is_object() {
            *next = Value::Object(serde_json::Map::new());
        }
        current = next
            .as_object_mut()
            .ok_or_else(|| AppError::Internal("projection path build failed".to_string()))?;
    }
    Ok(())
}

fn remove_value_by_path(doc: &mut Value, path: &str) -> AppResult<()> {
    if !doc.is_object() {
        return Ok(());
    }
    let mut parts = path.split('.').peekable();
    let mut current = doc
        .as_object_mut()
        .ok_or_else(|| AppError::Internal("projection remove failed".to_string()))?;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            current.remove(part);
            return Ok(());
        }
        let Some(next) = current.get_mut(part) else {
            return Ok(());
        };
        let Some(next_obj) = next.as_object_mut() else {
            return Ok(());
        };
        current = next_obj;
    }
    Ok(())
}

fn build_order_by(sort: &Option<Value>) -> AppResult<String> {
    let Some(spec) = sort else {
        return Ok("_created_at DESC".to_string());
    };
    let mut parts = Vec::new();
    let mut seen = HashSet::<String>::new();

    match spec {
        Value::Object(obj) => {
            if obj.is_empty() {
                return Ok("_created_at DESC".to_string());
            }
            for (path, dir) in obj {
                let key = path.to_string();
                if !seen.insert(key.clone()) {
                    return Err(AppError::BadRequest(format!("duplicate sort path: {key}")));
                }
                let direction = parse_sort_dir(dir)?;
                let expr = sort_expr(path)?;
                parts.push(format!("{expr} {direction}"));
            }
        }
        Value::String(s) => {
            for (path, direction) in parse_sort_string(s)? {
                if !seen.insert(path.clone()) {
                    return Err(AppError::BadRequest(format!("duplicate sort path: {path}")));
                }
                let expr = sort_expr(&path)?;
                parts.push(format!("{expr} {direction}"));
            }
        }
        _ => {
            return Err(AppError::BadRequest(
                "sort must be an object or string".to_string(),
            ));
        }
    }

    Ok(parts.join(", "))
}

fn parse_sort_dir(v: &Value) -> AppResult<&'static str> {
    if let Some(n) = v.as_i64() {
        return Ok(if n < 0 { "DESC" } else { "ASC" });
    }
    if let Some(s) = v.as_str() {
        return match s.to_ascii_lowercase().as_str() {
            "asc" | "1" => Ok("ASC"),
            "desc" | "-1" => Ok("DESC"),
            _ => Err(AppError::BadRequest(
                "sort direction must be 1|-1|asc|desc".to_string(),
            )),
        };
    }
    Err(AppError::BadRequest(
        "sort direction must be 1|-1|asc|desc".to_string(),
    ))
}

fn sort_expr(path: &str) -> AppResult<String> {
    match path {
        "id" | "_id" => Ok("id".to_string()),
        "collection" | "_collection" => Ok("collection".to_string()),
        "_created_at" | "_modified_at" | "_expires_at" | "_size_bytes" | "_expiry_behavior"
        | "_txn_id" => Ok(path.to_string()),
        _ => {
            validate_projection_path(path, "sort")?;
            Ok(format!("json_extract(data, '$.{path}')"))
        }
    }
}

fn parse_sort_string(input: &str) -> AppResult<Vec<(String, &'static str)>> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "sort string cannot be empty".to_string(),
        ));
    }
    let mut out = Vec::<(String, &'static str)>::new();
    for raw in trimmed.split(',') {
        let token = raw.trim();
        if token.is_empty() {
            return Err(AppError::BadRequest(
                "sort string contains empty token".to_string(),
            ));
        }
        let mut segs = token.split_whitespace();
        let path = segs
            .next()
            .ok_or_else(|| AppError::BadRequest("sort token missing path".to_string()))?;
        validate_projection_path(path, "sort")?;
        let direction = match segs.next() {
            None => "ASC",
            Some(dir) => match dir.to_ascii_lowercase().as_str() {
                "asc" => "ASC",
                "desc" => "DESC",
                _ => {
                    return Err(AppError::BadRequest(
                        "sort direction must be ASC or DESC in string mode".to_string(),
                    ));
                }
            },
        };
        if segs.next().is_some() {
            return Err(AppError::BadRequest(
                "sort string token has too many parts".to_string(),
            ));
        }
        out.push((path.to_string(), direction));
    }
    Ok(out)
}
