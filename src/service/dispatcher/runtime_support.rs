// Backup path, cache, ack, namespace scope, and pagination helpers extracted from dispatcher.rs.

fn default_backup_db_path(state: &AppState, db_path: &str) -> String {
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let configured = if state.backup_mode_is_s3 {
        state
            .backup_s3_path
            .as_deref()
            .unwrap_or(state.backup_local_path.as_str())
            .trim()
    } else {
        state.backup_local_path.trim()
    };
    let configured =
        if state.backup_mode_is_s3 && !configured.is_empty() && !configured.starts_with("s3://") {
            let prefix = configured.trim_matches('/');
            if let Some(bucket) = state.s3_bucket.as_deref() {
                if prefix.is_empty() {
                    format!("s3://{bucket}")
                } else {
                    format!("s3://{bucket}/{prefix}")
                }
            } else {
                configured.to_string()
            }
        } else {
            configured.to_string()
        };
    let db_slug = db_path.replace('/', "_");
    let db_hash = short_db_hash(db_path);
    let folder = format!("{db_slug}--{db_hash}");
    let filename = format!("{ts}_0001.db.zst");
    if configured.starts_with("s3://") {
        format!("{}/{folder}/{filename}", configured.trim_end_matches('/'))
    } else {
        format!("{}/{folder}/{filename}", configured.trim_end_matches('/'))
    }
}

fn json_params_to_sql_values(params: Option<Vec<Value>>) -> AppResult<Vec<libsql::Value>> {
    let mut out = Vec::<libsql::Value>::new();
    for value in params.unwrap_or_default() {
        out.push(json_param_to_sql_value(value)?);
    }
    Ok(out)
}

fn json_param_to_sql_value(v: Value) -> AppResult<libsql::Value> {
    match v {
        Value::Null => Ok(libsql::Value::Null),
        Value::Bool(b) => Ok(libsql::Value::Integer(if b { 1 } else { 0 })),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(libsql::Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(libsql::Value::Real(f))
            } else {
                Err(AppError::BadRequest(
                    "unsupported numeric sql param".to_string(),
                ))
            }
        }
        Value::String(s) => Ok(libsql::Value::Text(s)),
        Value::Array(_) | Value::Object(_) => Err(AppError::BadRequest(
            "sql params must be scalar values".to_string(),
        )),
    }
}

fn libsql_value_to_json(v: libsql::Value) -> Value {
    match v {
        libsql::Value::Null => Value::Null,
        libsql::Value::Integer(i) => json!(i),
        libsql::Value::Real(f) => json!(f),
        libsql::Value::Text(s) => Value::String(s),
        libsql::Value::Blob(bytes) => Value::String(hex_encode(&bytes)),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SqlExecuteMode {
    Read,
    Write,
}

fn classify_sql_execute(sql: &str) -> AppResult<(String, SqlExecuteMode)> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("sql cannot be empty".to_string()));
    }
    let stripped = trimmed.trim_end_matches(';').trim();
    if stripped.contains(';') {
        return Err(AppError::BadRequest(
            "sql_execute accepts a single statement only".to_string(),
        ));
    }
    let first = stripped
        .split_whitespace()
        .next()
        .map(|v| v.to_ascii_lowercase())
        .unwrap_or_default();
    match first.as_str() {
        "select" | "with" | "explain" => Ok((stripped.to_string(), SqlExecuteMode::Read)),
        "insert" | "update" | "delete" | "replace" => {
            Ok((stripped.to_string(), SqlExecuteMode::Write))
        }
        "create" | "drop" | "alter" => {
            validate_sql_execute_ddl(stripped)?;
            Ok((stripped.to_string(), SqlExecuteMode::Write))
        }
        "attach" | "detach" | "begin" | "commit" | "rollback" | "savepoint" | "release"
        | "load_extension" => Err(AppError::BadRequest(format!(
            "sql_execute does not allow `{first}`"
        ))),
        _ => Err(AppError::BadRequest(
            "sql_execute allows only single-statement SELECT, WITH, EXPLAIN, INSERT, UPDATE, DELETE, REPLACE, CREATE TABLE, CREATE INDEX, DROP INDEX, and ALTER TABLE ... ADD COLUMN".to_string(),
        )),
    }
}

fn validate_sql_execute_ddl(sql: &str) -> AppResult<()> {
    let tokens = sql.split_whitespace().collect::<Vec<_>>();
    let lowered = tokens
        .iter()
        .map(|v| v.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let Some(first) = lowered.first().map(String::as_str) else {
        return Err(AppError::BadRequest("sql cannot be empty".to_string()));
    };

    match first {
        "create" => validate_create_ddl(&tokens, &lowered),
        "drop" => validate_drop_ddl(&tokens, &lowered),
        "alter" => validate_alter_ddl(&tokens, &lowered),
        _ => Err(AppError::BadRequest(format!(
            "sql_execute does not allow `{first}`"
        ))),
    }
}

fn validate_create_ddl(tokens: &[&str], lowered: &[String]) -> AppResult<()> {
    if lowered.len() >= 2 && lowered[1] == "table" {
        let table_name_idx = if lowered.len() >= 5
            && lowered[2] == "if"
            && lowered[3] == "not"
            && lowered[4] == "exists"
        {
            5
        } else {
            2
        };
        let table_name = tokens.get(table_name_idx).ok_or_else(|| {
            AppError::BadRequest("CREATE TABLE requires a table name".to_string())
        })?;
        reject_reserved_sql_identifier(table_name, "table")?;
        return Ok(());
    }

    let index_token_idx = if lowered.len() >= 3 && lowered[1] == "unique" && lowered[2] == "index" {
        2
    } else if lowered.len() >= 2 && lowered[1] == "index" {
        1
    } else {
        return Err(AppError::BadRequest(
            "sql_execute only allows CREATE TABLE and CREATE INDEX".to_string(),
        ));
    };

    let index_name_idx = if lowered.len() > index_token_idx + 3
        && lowered[index_token_idx + 1] == "if"
        && lowered[index_token_idx + 2] == "not"
        && lowered[index_token_idx + 3] == "exists"
    {
        index_token_idx + 4
    } else {
        index_token_idx + 1
    };

    let index_name = tokens
        .get(index_name_idx)
        .ok_or_else(|| AppError::BadRequest("CREATE INDEX requires an index name".to_string()))?;
    reject_reserved_sql_identifier(index_name, "index")?;

    let on_idx = lowered
        .iter()
        .position(|v| v == "on")
        .ok_or_else(|| AppError::BadRequest("CREATE INDEX requires ON <table>".to_string()))?;
    let table_name = tokens
        .get(on_idx + 1)
        .ok_or_else(|| AppError::BadRequest("CREATE INDEX requires ON <table>".to_string()))?;
    reject_reserved_sql_identifier(table_name, "table")?;
    Ok(())
}

fn validate_drop_ddl(tokens: &[&str], lowered: &[String]) -> AppResult<()> {
    if lowered.len() < 2 || lowered[1] != "index" {
        return Err(AppError::BadRequest(
            "sql_execute only allows DROP INDEX".to_string(),
        ));
    }
    let index_name_idx = if lowered.len() >= 5
        && lowered[2] == "if"
        && lowered[3] == "exists"
    {
        4
    } else {
        2
    };
    let index_name = tokens
        .get(index_name_idx)
        .ok_or_else(|| AppError::BadRequest("DROP INDEX requires an index name".to_string()))?;
    reject_reserved_sql_identifier(index_name, "index")?;
    Ok(())
}

fn validate_alter_ddl(tokens: &[&str], lowered: &[String]) -> AppResult<()> {
    if lowered.len() < 6 || lowered[1] != "table" {
        return Err(AppError::BadRequest(
            "sql_execute only allows ALTER TABLE ... ADD COLUMN".to_string(),
        ));
    }
    let table_name = tokens
        .get(2)
        .ok_or_else(|| AppError::BadRequest("ALTER TABLE requires a table name".to_string()))?;
    reject_reserved_sql_identifier(table_name, "table")?;

    let has_add = lowered.iter().any(|v| v == "add");
    let has_column = lowered.iter().any(|v| v == "column");
    if !(has_add && has_column) {
        return Err(AppError::BadRequest(
            "sql_execute only allows ALTER TABLE ... ADD COLUMN".to_string(),
        ));
    }
    Ok(())
}

fn reject_reserved_sql_identifier(raw: &str, kind: &str) -> AppResult<()> {
    let ident = normalize_sql_identifier(raw);
    if ident.is_empty() {
        return Err(AppError::BadRequest(format!(
            "{kind} name is required"
        )));
    }
    let lower = ident.to_ascii_lowercase();
    if lower.starts_with("__kdb_") || lower.starts_with("sqlite_") {
        return Err(AppError::BadRequest(format!(
            "sql_execute cannot target reserved {kind} names with prefix __kdb_ or sqlite_"
        )));
    }
    Ok(())
}

fn normalize_sql_identifier(raw: &str) -> String {
    let trimmed = raw
        .trim()
        .trim_end_matches(',')
        .trim_end_matches('(')
        .trim_end_matches(')');
    let base = trimmed.rsplit('.').next().unwrap_or(trimmed).trim();
    if (base.starts_with('"') && base.ends_with('"'))
        || (base.starts_with('`') && base.ends_with('`'))
        || (base.starts_with('[') && base.ends_with(']'))
    {
        return base[1..base.len().saturating_sub(1)].to_string();
    }
    base.to_string()
}

async fn backup_artifact_meta(state: &AppState, backup_db_path: &str) -> AppResult<(i64, String)> {
    let bytes = if backup_db_path.starts_with("s3://") {
        state
            .db_manager
            .read_s3_uri_from_offset(backup_db_path, 0)
            .await?
    } else {
        tokio::fs::read(backup_db_path)
            .await
            .map_err(|e| AppError::Internal(format!("failed to read backup artifact: {e}")))?
    };
    let size_bytes = i64::try_from(bytes.len())
        .map_err(|_| AppError::Internal("backup artifact too large".to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let sha256 = format!("{:x}", hasher.finalize());
    Ok((size_bytes, sha256))
}

async fn upsert_kdb_backup_catalog(
    conn: &libsql::Connection,
    backup_id: &str,
    backup_db_path: &str,
    size_bytes: i64,
    sha256: &str,
    backup_tag: Option<&str>,
    source: &str,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO __kdb_backup_catalog (backup_id, backup_db_path, created_at, size_bytes, sha256, backup_tag, source)
         VALUES (?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?, ?, ?, ?)
         ON CONFLICT(backup_db_path) DO UPDATE SET
            backup_id = excluded.backup_id,
            created_at = excluded.created_at,
            size_bytes = excluded.size_bytes,
            sha256 = excluded.sha256,
            backup_tag = COALESCE(excluded.backup_tag, __kdb_backup_catalog.backup_tag),
            source = excluded.source",
        libsql::params![
            backup_id.to_string(),
            backup_db_path.to_string(),
            size_bytes,
            sha256.to_string(),
            backup_tag.map(|v| v.to_string()),
            source.to_string()
        ],
    )
    .await
    .map_err(|e| AppError::Internal(format!("backup catalog upsert failed: {e}")))?;
    Ok(())
}

async fn resolve_restore_backup_path(
    conn: &libsql::Connection,
    payload: OperationPayload,
) -> AppResult<String> {
    if let Some(path) = payload.backup_db_path.filter(|v| !v.trim().is_empty()) {
        return Ok(path);
    }

    if let Some(backup_id) = payload.backup_id.filter(|v| !v.trim().is_empty()) {
        let mut rows = conn
            .query(
                "SELECT backup_db_path FROM __kdb_backup_catalog WHERE backup_id = ? LIMIT 1",
                libsql::params![backup_id.clone()],
            )
            .await
            .map_err(|e| AppError::Internal(format!("restore backup lookup failed: {e}")))?;
        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("restore backup row failed: {e}")))?
        {
            let path: String = row
                .get(0)
                .map_err(|e| AppError::Internal(format!("restore backup decode failed: {e}")))?;
            return Ok(path);
        }
        return Err(AppError::NotFound(format!(
            "backup_id not found: {backup_id}"
        )));
    }

    if let Some(tag) = payload.backup_tag.filter(|v| !v.trim().is_empty()) {
        let mut rows = conn
            .query(
                "SELECT backup_db_path FROM __kdb_backup_catalog WHERE backup_tag = ? ORDER BY created_at DESC LIMIT 1",
                libsql::params![tag.clone()],
            )
            .await
            .map_err(|e| AppError::Internal(format!("restore backup tag lookup failed: {e}")))?;
        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("restore backup row failed: {e}")))?
        {
            let path: String = row
                .get(0)
                .map_err(|e| AppError::Internal(format!("restore backup decode failed: {e}")))?;
            return Ok(path);
        }
        return Err(AppError::NotFound(format!("backup_tag not found: {tag}")));
    }

    if let Some(at) = payload.backup_at.filter(|v| !v.trim().is_empty()) {
        let mut rows = conn
            .query(
                "SELECT backup_db_path
                 FROM __kdb_backup_catalog
                 WHERE datetime(created_at) <= datetime(?)
                 ORDER BY created_at DESC
                 LIMIT 1",
                libsql::params![at.clone()],
            )
            .await
            .map_err(|e| {
                AppError::Internal(format!("restore backup timestamp lookup failed: {e}"))
            })?;
        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("restore backup row failed: {e}")))?
        {
            let path: String = row
                .get(0)
                .map_err(|e| AppError::Internal(format!("restore backup decode failed: {e}")))?;
            return Ok(path);
        }
        return Err(AppError::NotFound(format!(
            "no backup found at or before: {at}"
        )));
    }

    if payload.latest.unwrap_or(false) {
        let mut rows = conn
            .query(
                "SELECT backup_db_path FROM __kdb_backup_catalog ORDER BY created_at DESC LIMIT 1",
                (),
            )
            .await
            .map_err(|e| AppError::Internal(format!("restore latest backup lookup failed: {e}")))?;
        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("restore latest row failed: {e}")))?
        {
            let path: String = row
                .get(0)
                .map_err(|e| AppError::Internal(format!("restore latest decode failed: {e}")))?;
            return Ok(path);
        }
        return Err(AppError::NotFound("no backups found".to_string()));
    }

    Err(AppError::BadRequest(
        "restore_backup requires one of: backup_db_path, backup_id, backup_tag, backup_at, latest=true"
            .to_string(),
    ))
}

fn short_db_hash(db_path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(db_path.as_bytes());
    let hex = format!("{:x}", hasher.finalize());
    hex[..16].to_string()
}

fn normalize_export_target_path(
    state: &AppState,
    db_path: &str,
    target_path: Option<&str>,
    compress: bool,
) -> AppResult<String> {
    let mut path = if let Some(tp) = target_path.map(str::trim).filter(|v| !v.is_empty()) {
        tp.to_string()
    } else {
        default_export_target_path(state, db_path)
    };
    if path.starts_with("s3://") {
        path = path.trim_end_matches('/').to_string();
    }
    if compress {
        if path.ends_with(".jsonl") {
            path.push_str(".zst");
        } else if !path.ends_with(".zst") {
            path.push_str(".jsonl.zst");
        }
    } else if !path.ends_with(".jsonl") {
        path.push_str(".jsonl");
    }
    Ok(path)
}

fn default_export_target_path(state: &AppState, db_path: &str) -> String {
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let configured = state.export_local_path.trim();
    let db_slug = db_path.replace('/', "_");
    let db_hash = short_db_hash(db_path);
    let filename = format!("{ts}__{db_slug}--{db_hash}");
    format!("{}/{}", configured.trim_end_matches('/'), filename)
}

#[derive(Clone, Copy)]
enum CachePolicy {
    Default,
    CustomTtl(u64),
    Invalidate,
}

#[derive(Debug, Clone)]
enum NamespaceReadScope {
    None,
    One(String),
    Many(Vec<String>),
    All,
}

struct CacheKeyMeta {
    key: String,
    policy: CachePolicy,
}

fn cache_key_for_read(
    state: &AppState,
    db_path: &str,
    operation: &str,
    payload: &OperationPayload,
) -> AppResult<Option<CacheKeyMeta>> {
    if !state.cache_enabled {
        return Ok(None);
    }
    let policy = parse_cache_policy(payload)?;
    let Some(policy) = policy else {
        return Ok(None);
    };
    if matches!(policy, CachePolicy::Invalidate) {
        invalidate_read_cache_scope(state, db_path, payload.collection.as_deref());
        return Ok(None);
    }

    let collection = payload
        .collection
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    let broadcast_epoch = state.current_broadcast_epoch(db_path);
    let scope_epoch = if let Some(c) = collection {
        format!("c:{c}:{}", state.current_collection_epoch(db_path, c))
    } else {
        format!("a:{}", state.current_any_scope_epoch(db_path))
    };
    let payload_json = serde_json::to_string(payload)
        .map_err(|e| AppError::Internal(format!("cache key serialize failed: {e}")))?;
    Ok(Some(CacheKeyMeta {
        key: format!("{db_path}|b:{broadcast_epoch}|{scope_epoch}|{operation}|{payload_json}"),
        policy,
    }))
}

fn is_write_operation(operation: &str) -> bool {
    !matches!(
        operation,
        "load_db"
            | "db_exists"
            | "list_commands"
            | "list_dbs"
            | "list_all_dbs"
            | "system_get_inventory"
            | "system_get_db_status"
            | "system_query_db_stats"
            | "system_list_db_events"
            | "get_system_stats"
            | "system_memory"
            | "get_sync_status"
            | "verify_db"
            | "get"
            | "count"
            | "search"
            | "aggregate"
            | "metrics_query"
            | "metrics_catalog"
            | "audit_query"
            | "user_get"
            | "user_list"
            | "user_get_details"
            | "file_get"
            | "file_list"
            | "query"
            | "list_indexes"
            | "list_backups"
            | "export_jsonl"
            | "get_job"
            | "list_jobs"
            | "get_stats"
            | "get_db_stats"
            | "query_db_stats"
            | "get_system_config"
            | "list_namespaces"
            | "list_tables"
            | "get_table_schema"
    )
}

pub(crate) fn request_is_write(req: &GatewayRequest) -> bool {
    if req.operation != "sql_execute" {
        return is_write_operation(&req.operation);
    }
    req.payload
        .sql
        .as_deref()
        .and_then(|sql| classify_sql_execute(sql).ok().map(|(_, mode)| mode))
        .map(|mode| matches!(mode, SqlExecuteMode::Write))
        .unwrap_or(false)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AckMode {
    Committed,
    Accepted,
}

fn resolve_ack_mode(state: &AppState, commit: Option<bool>) -> AppResult<AckMode> {
    if let Some(commit) = commit {
        return Ok(if commit {
            AckMode::Committed
        } else {
            AckMode::Accepted
        });
    }
    match state.write_queue_default_ack_mode.as_str() {
        "committed" => Ok(AckMode::Committed),
        "accepted" => Ok(AckMode::Accepted),
        other => Err(AppError::BadRequest(format!(
            "resolved write mode must be committed|accepted, got: {other}"
        ))),
    }
}

fn supports_accepted_ack(operation: &str) -> bool {
    if !is_write_operation(operation) {
        return false;
    }
    !matches!(
        operation,
        "import_jsonl"
            | "export_jsonl"
            | "create_backup"
            | "reindex_fts"
            | "drop_fts_index"
            | "enable_fts_index"
            | "enable_ftx_index"
            | "continue_job"
            | "abort_job"
            | "list_jobs"
            | "get_job"
            | "vacuum_db"
            | "recompute_stats"
            | "snapshot_db_stats"
            | "user_create"
            | "user_update"
            | "user_update_status"
            | "user_delete"
            | "user_create_token"
            | "user_link_provider"
            | "user_unlink_provider"
            | "file_create"
            | "file_update"
            | "file_delete"
    )
}

fn supports_accepted_ack_request(req: &GatewayRequest) -> bool {
    if !request_is_write(req) {
        return false;
    }
    if req.operation == "sql_execute" {
        return true;
    }
    supports_accepted_ack(req.operation.as_str())
}

fn parse_namespaces(raw: Option<Vec<String>>) -> AppResult<Option<Vec<String>>> {
    let Some(values) = raw else {
        return Ok(None);
    };
    let mut out = values
        .into_iter()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>();
    if out.is_empty() {
        return Err(AppError::BadRequest("namespaces cannot be empty".to_string()));
    }
    out.sort();
    out.dedup();
    let has_star = out.iter().any(|v| v == "*");
    if has_star && out.len() > 1 {
        return Err(AppError::BadRequest(
            "namespaces cannot include '*' with other values".to_string(),
        ));
    }
    Ok(Some(out))
}

fn resolve_read_namespace_scope(
    payload: &OperationPayload,
    require_namespace: bool,
) -> AppResult<NamespaceReadScope> {
    let list = parse_namespaces(payload.namespaces.clone())?;
    let single = payload.collection.clone().filter(|c| !c.trim().is_empty());
    if list.is_some() && single.is_some() {
        return Err(AppError::BadRequest(
            "namespace and namespaces cannot both be provided".to_string(),
        ));
    }
    if let Some(list) = list {
        if list.len() == 1 && list[0] == "*" {
            return Ok(NamespaceReadScope::All);
        }
        return Ok(NamespaceReadScope::Many(list));
    }
    if let Some(single) = single {
        if single == "*" {
            return Ok(NamespaceReadScope::All);
        }
        return Ok(NamespaceReadScope::One(single));
    }
    if payload.scope.as_deref() == Some("all") {
        return Ok(NamespaceReadScope::All);
    }
    if require_namespace {
        return Err(AppError::BadRequest("namespace is required".to_string()));
    }
    Ok(NamespaceReadScope::None)
}

fn namespace_scope_force_include(scope: &NamespaceReadScope) -> bool {
    matches!(scope, NamespaceReadScope::All | NamespaceReadScope::Many(_))
}

fn pending_matches_read_scope(collection: &Option<String>, scope: &NamespaceReadScope) -> bool {
    match scope {
        NamespaceReadScope::None | NamespaceReadScope::All => true,
        NamespaceReadScope::One(ns) => collection.as_deref() == Some(ns.as_str()),
        NamespaceReadScope::Many(list) => collection
            .as_ref()
            .map(|ns| list.iter().any(|item| item == ns))
            .unwrap_or(false),
    }
}

fn build_where_with_namespace_scope(
    filter: Value,
    scope: &NamespaceReadScope,
) -> AppResult<(String, Vec<libsql::Value>)> {
    let compiled = build_where(&filter)?;
    let mut bind_values: Vec<libsql::Value> = Vec::new();
    let mut where_clause = format!("({})", compiled.sql);

    match scope {
        NamespaceReadScope::None | NamespaceReadScope::All => {}
        NamespaceReadScope::One(ns) => {
            where_clause = format!("collection = ? AND {where_clause}");
            bind_values.push(libsql::Value::Text(ns.clone()));
        }
        NamespaceReadScope::Many(list) => {
            let placeholders = vec!["?"; list.len()].join(", ");
            where_clause = format!("collection IN ({placeholders}) AND {where_clause}");
            bind_values.extend(list.iter().cloned().map(libsql::Value::Text));
        }
    }
    bind_values.extend(compiled.binds);
    Ok((where_clause, bind_values))
}

fn where_ids_with_namespace_scope(
    binds: &mut Vec<libsql::Value>,
    scope: &NamespaceReadScope,
    ids: &[String],
    id_placeholders: &str,
) -> String {
    match scope {
        NamespaceReadScope::None | NamespaceReadScope::All => {
            binds.extend(ids.iter().cloned().map(libsql::Value::Text));
            format!("id IN ({id_placeholders})")
        }
        NamespaceReadScope::One(ns) => {
            binds.push(libsql::Value::Text(ns.clone()));
            binds.extend(ids.iter().cloned().map(libsql::Value::Text));
            format!("collection = ? AND id IN ({id_placeholders})")
        }
        NamespaceReadScope::Many(list) => {
            let ns_placeholders = vec!["?"; list.len()].join(", ");
            binds.extend(list.iter().cloned().map(libsql::Value::Text));
            binds.extend(ids.iter().cloned().map(libsql::Value::Text));
            format!("collection IN ({ns_placeholders}) AND id IN ({id_placeholders})")
        }
    }
}

fn resolve_pagination_args(
    payload: &OperationPayload,
    default_limit: usize,
) -> AppResult<(i64, i64, i64)> {
    let default_limit = i64::try_from(default_limit).unwrap_or(50).max(1);

    if payload.limit.is_some() || payload.offset.is_some() {
        let limit = payload.limit.unwrap_or(default_limit).max(1);
        let offset = payload.offset.unwrap_or(0).max(0);
        let page = (offset / limit) + 1;
        return Ok((limit, offset, page));
    }

    let limit = payload.per_page.unwrap_or(default_limit).max(1);
    let page = payload.page.unwrap_or(1);
    if page <= 0 {
        return Err(AppError::BadRequest(
            "page must be >= 1".to_string(),
        ));
    }
    let offset = (page - 1).saturating_mul(limit);
    Ok((limit, offset, page))
}

fn build_pagination(total_count: i64, count: usize, limit: i64, page: i64, offset: i64) -> Value {
    let count_i64 = i64::try_from(count).unwrap_or(i64::MAX);
    let progressed = offset.saturating_add(count_i64);
    let has_next = progressed < total_count;
    let has_prev = page > 1;
    let next_page = if has_next {
        Some(page.saturating_add(1))
    } else {
        None
    };
    let prev_page = if has_prev {
        Some(page.saturating_sub(1))
    } else {
        None
    };
    let total_pages = if total_count <= 0 {
        0
    } else {
        ((total_count - 1) / limit) + 1
    };
    json!({
        "total_items": total_count,
        "count": count,
        "per_page": limit,
        "page": page,
        "total_pages": total_pages,
        "next_page": next_page,
        "prev_page": prev_page
    })
}

fn build_offsets(total_count: i64, count: usize, limit: i64, offset: i64) -> (Value, Value) {
    let count_i64 = i64::try_from(count).unwrap_or(i64::MAX);
    let progressed = offset.saturating_add(count_i64);
    let has_next = progressed < total_count;
    let has_prev = offset > 0;
    let next_offset = if has_next {
        Value::from(progressed)
    } else {
        Value::Null
    };
    let prev_offset = if has_prev {
        Value::from(offset.saturating_sub(limit))
    } else {
        Value::Null
    };
    (next_offset, prev_offset)
}

fn validate_accepted_preflight(req: &GatewayRequest) -> AppResult<()> {
    match req.operation.as_str() {
        "insert" => {
            let data = req
                .payload
                .data
                .as_ref()
                .ok_or_else(|| AppError::BadRequest("data is required".to_string()))?;
            if data.is_object() {
            } else if let Some(arr) = data.as_array() {
                if arr.is_empty() {
                    return Err(AppError::BadRequest("data cannot be empty".to_string()));
                }
                if arr.iter().any(|v| !v.is_object()) {
                    return Err(AppError::BadRequest(
                        "data items must be objects".to_string(),
                    ));
                }
            } else {
                return Err(AppError::BadRequest(
                    "data must be an object or array<object>".to_string(),
                ));
            }
        }
        "update" | "set" => {
            let data = req
                .payload
                .data
                .as_ref()
                .ok_or_else(|| AppError::BadRequest("data is required".to_string()))?;
            if req.operation == "set" {
                if !data.is_object() {
                    return Err(AppError::BadRequest("data must be an object".to_string()));
                }
                return Ok(());
            }
            if data.is_object() {
            } else if let Some(arr) = data.as_array() {
                if arr.is_empty() {
                    return Err(AppError::BadRequest("data cannot be empty".to_string()));
                }
                if arr.iter().any(|v| !v.is_object()) {
                    return Err(AppError::BadRequest(
                        "data items must be objects".to_string(),
                    ));
                }
            } else {
                return Err(AppError::BadRequest(
                    "data must be an object or array<object>".to_string(),
                ));
            }
        }
        "sql_execute" => {
            let sql = req
                .payload
                .sql
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("sql is required".to_string()))?;
            let _ = classify_sql_execute(sql)?;
            let _ = json_params_to_sql_values(req.payload.params.clone())?;
        }
        "metrics_ingest" => {
            let events = req
                .payload
                .events
                .as_ref()
                .ok_or_else(|| AppError::BadRequest("events is required".to_string()))?;
            if events.is_empty() {
                return Err(AppError::BadRequest("events cannot be empty".to_string()));
            }
            if events.iter().any(|v| !v.is_object()) {
                return Err(AppError::BadRequest(
                    "events items must be objects".to_string(),
                ));
            }
        }
        "audit_ingest" => {
            let events = req
                .payload
                .events
                .as_ref()
                .ok_or_else(|| AppError::BadRequest("events is required".to_string()))?;
            if events.is_empty() {
                return Err(AppError::BadRequest("events cannot be empty".to_string()));
            }
            if events.iter().any(|v| !v.is_object()) {
                return Err(AppError::BadRequest(
                    "events items must be objects".to_string(),
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

fn prepare_accepted_ack_preview(req: &mut GatewayRequest) -> AppResult<Value> {
    let mut ids = Vec::<String>::new();
    match req.operation.as_str() {
        "insert" => {
            let data = req
                .payload
                .data
                .as_mut()
                .ok_or_else(|| AppError::BadRequest("data is required".to_string()))?;
            if let Some(doc) = data.as_object_mut() {
                let id = match doc.get("_id") {
                    Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
                    Some(_) => {
                        return Err(AppError::BadRequest(
                            "_id must be a non-empty string".to_string(),
                        ))
                    }
                    None => {
                        let generated_id = Uuid::new_v4().simple().to_string();
                        doc.insert("_id".to_string(), Value::String(generated_id.clone()));
                        generated_id
                    }
                };
                ids.push(id);
            } else if let Some(arr) = data.as_array_mut() {
                for item in arr.iter_mut() {
                    let doc = item.as_object_mut().ok_or_else(|| {
                        AppError::BadRequest("data items must be objects".to_string())
                    })?;
                    let id = match doc.get("_id") {
                        Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
                        Some(_) => {
                            return Err(AppError::BadRequest(
                                "_id must be a non-empty string".to_string(),
                            ))
                        }
                        None => {
                            let generated_id = Uuid::new_v4().simple().to_string();
                            doc.insert("_id".to_string(), Value::String(generated_id.clone()));
                            generated_id
                        }
                    };
                    ids.push(id);
                }
            } else {
                return Err(AppError::BadRequest(
                    "data must be an object or array<object>".to_string(),
                ));
            }
        }
        "update" | "set" => {
            if let Some(obj) = req.payload.data.as_ref().and_then(|v| v.as_object()) {
                if let Some(id) = obj
                    .get("_id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                {
                    ids.push(id.to_string());
                }
            } else if let Some(arr) = req.payload.data.as_ref().and_then(|v| v.as_array()) {
                for item in arr {
                    if let Some(id) = item
                        .as_object()
                        .and_then(|o| o.get("_id"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                    {
                        ids.push(id.to_string());
                    }
                }
            }
        }
        "metrics_ingest" => {
            let events = req
                .payload
                .events
                .as_mut()
                .ok_or_else(|| AppError::BadRequest("events is required".to_string()))?;
            let count = events.len();
            for event in events.iter_mut() {
                let obj = event
                    .as_object_mut()
                    .ok_or_else(|| AppError::BadRequest("events items must be objects".to_string()))?;
                let id = match obj.get("_id") {
                    Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
                    Some(_) => {
                        return Err(AppError::BadRequest(
                            "event _id must be a non-empty string".to_string(),
                        ))
                    }
                    None => {
                        let generated_id = format!("evt_{}", Uuid::new_v4().simple());
                        obj.insert("_id".to_string(), Value::String(generated_id.clone()));
                        generated_id
                    }
                };
                ids.push(id);
            }
            ids.sort();
            ids.dedup();
            return Ok(json!({
                "ids": ids,
                "count": count,
                "queued": true
            }));
        }
        "audit_ingest" => {
            let events = req
                .payload
                .events
                .as_mut()
                .ok_or_else(|| AppError::BadRequest("events is required".to_string()))?;
            let count = events.len();
            for event in events.iter_mut() {
                let obj = event
                    .as_object_mut()
                    .ok_or_else(|| AppError::BadRequest("events items must be objects".to_string()))?;
                let id = match obj.get("_id") {
                    Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
                    Some(_) => {
                        return Err(AppError::BadRequest(
                            "audit event _id must be a non-empty string".to_string(),
                        ))
                    }
                    None => {
                        let generated_id = format!("aud_{}", Uuid::new_v4().simple());
                        obj.insert("_id".to_string(), Value::String(generated_id.clone()));
                        generated_id
                    }
                };
                ids.push(id);
            }
            ids.sort();
            ids.dedup();
            return Ok(json!({
                "ids": ids,
                "count": count,
                "queued": true
            }));
        }
        _ => {
            if let Some(id) = req
                .payload
                .id
                .as_ref()
                .filter(|s| !s.trim().is_empty())
                .cloned()
            {
                ids.push(id);
            }
            if let Some(payload_ids) = req.payload.ids.as_ref() {
                ids.extend(
                    payload_ids
                        .iter()
                        .filter(|s| !s.trim().is_empty())
                        .cloned(),
                );
            }
            if ids.is_empty() {
                if let Some(id) = req
                    .payload
                    .data
                    .as_ref()
                    .and_then(|v| v.as_object())
                    .and_then(|o| o.get("_id"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                {
                    ids.push(id.to_string());
                }
            }
        }
    }

    ids.sort();
    ids.dedup();
    Ok(json!({
        "ids": ids,
        "queued": true
    }))
}

fn parse_cache_policy(payload: &OperationPayload) -> AppResult<Option<CachePolicy>> {
    match payload.cache.as_ref() {
        None => Ok(Some(CachePolicy::Default)),
        Some(CacheHint::Bool(false)) => Ok(None),
        Some(CacheHint::Bool(true)) => Ok(Some(CachePolicy::Default)),
        Some(CacheHint::Int(-1)) => Ok(Some(CachePolicy::Invalidate)),
        Some(CacheHint::Int(v)) if *v <= 0 => Ok(None),
        Some(CacheHint::Int(1)) => Ok(Some(CachePolicy::Default)),
        Some(CacheHint::Int(v)) => {
            let ttl = u64::try_from(*v).map_err(|_| {
                AppError::BadRequest("cache must be false|0 or true|1+ seconds".to_string())
            })?;
            Ok(Some(CachePolicy::CustomTtl(ttl)))
        }
    }
}

fn invalidate_read_cache_scope(state: &AppState, db_path: &str, collection: Option<&str>) {
    let collection = collection.map(str::trim).filter(|c| !c.is_empty());
    if let Some(c) = collection {
        state.bump_collection_epoch(db_path, c);
        state.bump_any_scope_epoch(db_path);
    } else {
        state.bump_broadcast_epoch(db_path);
        state.bump_any_scope_epoch(db_path);
    }
}

fn invalidate_read_cache_after_write(
    state: &AppState,
    db_path: &str,
    operation: &str,
    payload: &OperationPayload,
) {
    if operation == "change_namespace" || operation == "rename_namespace" {
        let source = payload
            .from_namespace
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty());
        let target = payload
            .to_namespace
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty());

        if let Some(c) = source {
            invalidate_read_cache_scope(state, db_path, Some(c));
        }
        if let Some(c) = target {
            invalidate_read_cache_scope(state, db_path, Some(c));
        }
        if source.is_none() && target.is_none() {
            invalidate_read_cache_scope(state, db_path, None);
        }
        return;
    }

    let collection = payload
        .collection
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());

    let is_multi_scope = matches!(
        operation,
        "transaction"
            | "restore_archive"
            | "purge_archive"
            | "recompute_stats"
            | "offload_db"
    ) || collection.is_none();

    if is_multi_scope {
        invalidate_read_cache_scope(state, db_path, None);
        return;
    }

    if let Some(c) = collection {
        invalidate_read_cache_scope(state, db_path, Some(c));
    }
}

async fn get_cached_read(state: &AppState, meta: &CacheKeyMeta) -> Option<Value> {
    match meta.policy {
        CachePolicy::Default => state.read_cache.get(&meta.key).await,
        CachePolicy::CustomTtl(_) => state.get_ttl_override_cache(&meta.key),
        CachePolicy::Invalidate => None,
    }
}

async fn put_cached_read(state: &AppState, meta: &CacheKeyMeta, data: Value) {
    match meta.policy {
        CachePolicy::Default => {
            state.read_cache.insert(meta.key.clone(), data).await;
        }
        CachePolicy::CustomTtl(ttl_secs) => {
            state.put_ttl_override_cache(meta.key.clone(), data, ttl_secs);
        }
        CachePolicy::Invalidate => {}
    }
}

fn supports_pending_prepared_preview(req: &GatewayRequest) -> bool {
    match req.operation.as_str() {
        "insert" => true,
        "update" => req.payload.filter.is_none() && update_payload_has_explicit_ids(&req.payload),
        _ => false,
    }
}

fn update_payload_has_explicit_ids(payload: &OperationPayload) -> bool {
    match payload.data.as_ref() {
        Some(Value::Object(obj)) => obj
            .get("_id")
            .and_then(Value::as_str)
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false),
        Some(Value::Array(items)) => {
            !items.is_empty()
                && items.iter().all(|item| {
                    item.as_object()
                        .and_then(|obj| obj.get("_id"))
                        .and_then(Value::as_str)
                        .map(|v| !v.trim().is_empty())
                        .unwrap_or(false)
                })
        }
        _ => false,
    }
}

pub async fn prepare_pending_write_preview(
    state: &AppState,
    db_path: &str,
    req: &GatewayRequest,
) -> AppResult<crate::state::PendingWritePreview> {
    match req.operation.as_str() {
        "insert" => prepare_pending_insert_preview(state, db_path, req).await,
        "update" => prepare_pending_update_preview(state, db_path, req).await,
        other => Err(AppError::BadRequest(format!(
            "pending preview is not supported for operation: {other}"
        ))),
    }
}

async fn prepare_pending_insert_preview(
    state: &AppState,
    db_path: &str,
    req: &GatewayRequest,
) -> AppResult<crate::state::PendingWritePreview> {
    let payload = req.payload.clone();
    let collection = require_collection(&payload)?;
    let payload_user_id = clean_optional(payload.document_user_id.clone());
    let bulk = matches!(payload.data.as_ref(), Some(Value::Array(_)));
    let mut docs = prepare_insert_documents(payload.data, bulk)?.docs;
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut items = Vec::with_capacity(docs.len());
    let mut revisions = Vec::with_capacity(docs.len());
    let mut ids = Vec::with_capacity(docs.len());

    for doc in &mut docs {
        let user_id = normalize_document_user_id_from_doc(doc, payload_user_id.as_deref())?;
        let id = doc
            .get("_id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Internal("prepared insert missing _id".to_string()))?
            .to_string();
        let mut item = doc.clone();
        attach_document_user_id(&mut item, user_id.clone());
        attach_system_timestamps(&mut item, Some(now.clone()), Some(now.clone()));
        let revision =
            state.put_pending_document(db_path, &id, Some(collection.clone()), item.clone());
        revisions.push(crate::state::PendingWriteRevision {
            id: id.clone(),
            revision,
        });
        ids.push(id);
        items.push(item);
    }

    let mut response = GatewayResponse::ok(Some(json!({
        "items": items,
        "ids": ids,
        "count": ids.len(),
        "inserted_count": ids.len(),
        "skipped_count": 0,
        "queued": true,
        "pending": true
    })));
    response.committed = Some(false);
    response.is_async_ack = Some(true);
    response.ack_mode = Some("accepted".to_string());
    response.ack_status = Some("queued".to_string());
    Ok(crate::state::PendingWritePreview {
        response,
        revisions,
    })
}

async fn prepare_pending_update_preview(
    state: &AppState,
    db_path: &str,
    req: &GatewayRequest,
) -> AppResult<crate::state::PendingWritePreview> {
    let payload = req.payload.clone();
    let collection = resolve_collection_scope_optional_collection(&payload)?;
    let conn = state.db_manager.get_conn_with_create(db_path, false).await?;
    let docs = match payload.data.clone() {
        Some(Value::Object(obj)) => vec![Value::Object(obj)],
        Some(Value::Array(items)) => items,
        _ => {
            return Err(AppError::BadRequest(
                "update data must be an object or array<object>".to_string(),
            ))
        }
    };
    let mut items = Vec::with_capacity(docs.len());
    let mut ids = Vec::with_capacity(docs.len());
    let mut revisions = Vec::with_capacity(docs.len());
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

    for patch_doc in docs {
        let mut patch_obj = patch_doc
            .as_object()
            .cloned()
            .ok_or_else(|| AppError::BadRequest("update data items must be objects".to_string()))?;
        let id = patch_obj
            .get("_id")
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| AppError::BadRequest("update data._id is required".to_string()))?
            .to_string();
        patch_obj.remove("_id");
        if patch_obj.is_empty() {
            return Err(AppError::BadRequest(
                "update data must include fields beyond _id".to_string(),
            ));
        }
        let mut base = if let Some(pending) = state.get_pending_document(db_path, &id) {
            if pending_matches_scope(&pending.collection, collection.as_deref()) {
                pending.document
            } else {
                continue;
            }
        } else {
            let durable = fetch_kdb_documents_by_ids(
                &conn,
                collection.as_deref(),
                std::slice::from_ref(&id),
                true,
            )
            .await?;
            let Some(doc) = durable.into_iter().next() else {
                continue;
            };
            doc
        };

        if payload.replace.unwrap_or(false) {
            let mut replacement = Value::Object(patch_obj);
            base = replacement_doc_from_payload(&mut replacement, &id)?;
        } else if update_requires_mutation_engine(&patch_obj) {
            apply_mutation_patch_to_doc(&mut base, &mut patch_obj, state.strict_mutation_operators)?;
        } else {
            apply_merge_patch_object(&mut base, &patch_obj)?;
        }
        attach_system_timestamps(&mut base, None, Some(now.clone()));
        let revision = state.put_pending_document(db_path, &id, collection.clone(), base.clone());
        revisions.push(crate::state::PendingWriteRevision {
            id: id.clone(),
            revision,
        });
        ids.push(id);
        items.push(base);
    }

    let mut response = GatewayResponse::ok(Some(json!({
        "items": items,
        "ids": ids,
        "count": ids.len(),
        "matched_count": ids.len(),
        "updated_count": ids.len(),
        "queued": true,
        "pending": true
    })));
    response.committed = Some(false);
    response.is_async_ack = Some(true);
    response.ack_mode = Some("accepted".to_string());
    response.ack_status = Some("queued".to_string());
    Ok(crate::state::PendingWritePreview {
        response,
        revisions,
    })
}

fn pending_matches_scope(pending_collection: &Option<String>, requested: Option<&str>) -> bool {
    match requested {
        Some(ns) => pending_collection.as_deref() == Some(ns),
        None => true,
    }
}

fn apply_merge_patch_object(doc: &mut Value, patch: &serde_json::Map<String, Value>) -> AppResult<()> {
    for (path, value) in patch {
        validate_projection_path(path, "data")?;
        let mut value = value.clone();
        expand_kdb_macros_in_value(&mut value)?;
        if value.is_null() {
            drop_path(doc, path);
        } else {
            set_path(doc, path, value)?;
        }
    }
    Ok(())
}
