//! Axum handlers for gateway, ping, docs, and meta endpoints.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, header::HeaderName},
    response::Html,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use pulldown_cmark::{Options, Parser, html};
use serde_json::json;
use std::{fs, time::Duration};

use crate::{
    api::dto::{GatewayRequest, GatewayResponse},
    error::{AppError, AppResult},
    service::dispatcher,
    state::{AppState, SystemRequestKind},
};

pub async fn gateway(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut request): Json<GatewayRequest>,
) -> AppResult<Json<GatewayResponse>> {
    validate_access_key(&state, &headers)?;
    normalize_request_operation_alias(&mut request)?;
    normalize_request_namespace(&mut request)?;
    let is_global_op = matches!(
        request.operation.as_str(),
        "list_commands"
            | "list_dbs"
            | "list_all_dbs"
            | "cleanup_temp_artifacts"
            | "system_get_inventory"
            | "system_refresh_inventory"
            | "system_get_db_status"
            | "system_snapshot_db_stats"
            | "system_query_db_stats"
            | "system_list_db_events"
            | "system_memory"
            | "get_system_stats"
    );
    let is_write = dispatcher::request_is_write(&request);
    let system_stats_guard = state.begin_system_request(if is_global_op {
        SystemRequestKind::Admin
    } else if is_write {
        SystemRequestKind::Write
    } else {
        SystemRequestKind::Read
    });
    let db_path = if is_global_op {
        request.db.take().unwrap_or_default()
    } else {
        request
            .db
            .take()
            .ok_or_else(|| AppError::BadRequest("db is required".to_string()))?
    };
    let stats_guard = if is_global_op {
        None
    } else {
        Some(state.begin_db_request(db_path.as_str(), is_write))
    };
    let fut = dispatcher::dispatch(&state, &db_path, request);
    let result =
        match tokio::time::timeout(Duration::from_millis(state.operation_timeout_ms), fut).await {
            Ok(res) => res.map(Json),
            Err(_) => Err(AppError::Timeout(format!(
                "operation timed out after {} ms",
                state.operation_timeout_ms
            ))),
        };
    if let Some(guard) = stats_guard {
        guard.finish(result.is_ok());
    }
    system_stats_guard.finish(result.is_ok());
    result
}

fn normalize_request_operation_alias(req: &mut GatewayRequest) -> AppResult<()> {
    let operation = req.operation.clone();
    if let Some((raw_op, raw_scope)) = operation.split_once("::") {
        let op = raw_op.trim();
        let scope = raw_scope.trim();
        if op.is_empty() {
            return Err(AppError::BadRequest(
                "operation shorthand must include an operation before '::'".to_string(),
            ));
        }
        if scope.is_empty() {
            return Err(AppError::BadRequest(
                "operation shorthand must include a namespace selector after '::'".to_string(),
            ));
        }
        if req
            .namespace
            .as_ref()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
            || req
                .namespaces
                .as_ref()
                .map(|v| v.iter().any(|item| !item.trim().is_empty()))
                .unwrap_or(false)
        {
            return Err(AppError::BadRequest(
                "operation shorthand cannot be combined with top-level namespace/namespaces"
                    .to_string(),
            ));
        }

        req.operation = op.to_string();
        if scope == "*" {
            req.namespace = Some("*".to_string());
            req.namespaces = None;
        } else if scope.contains(',') {
            let namespaces = scope
                .split(',')
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .collect::<Vec<_>>();
            if namespaces.is_empty() {
                return Err(AppError::BadRequest(
                    "operation shorthand namespaces cannot be empty".to_string(),
                ));
            }
            req.namespace = None;
            req.namespaces = Some(namespaces);
        } else {
            req.namespace = Some(scope.to_string());
            req.namespaces = None;
        }
    }

    if let Some(children) = req.data.as_mut() {
        for child in children {
            normalize_request_operation_alias(child)?;
        }
    }
    Ok(())
}

fn normalize_request_namespace(req: &mut GatewayRequest) -> AppResult<()> {
    if let Some(raw_list) = req.namespaces.clone() {
        let namespaces = raw_list
            .into_iter()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>();
        if namespaces.is_empty() {
            return Err(AppError::BadRequest(
                "namespaces cannot be empty".to_string(),
            ));
        }
        if req
            .namespace
            .as_ref()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
        {
            return Err(AppError::BadRequest(
                "namespace and namespaces cannot both be provided".to_string(),
            ));
        }
        if req
            .payload
            .collection
            .as_ref()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
        {
            return Err(AppError::BadRequest(
                "payload.collection cannot be used with namespaces".to_string(),
            ));
        }
        let has_star = namespaces.iter().any(|v| v == "*");
        if has_star && namespaces.len() > 1 {
            return Err(AppError::BadRequest(
                "namespaces cannot include '*' with other values".to_string(),
            ));
        }
        if has_star {
            match req.payload.scope.as_deref() {
                Some("collection") => {
                    return Err(AppError::BadRequest(
                        "namespace='*' conflicts with payload.scope='collection'".to_string(),
                    ));
                }
                Some("all") | None => req.payload.scope = Some("all".to_string()),
                Some(other) => {
                    return Err(AppError::BadRequest(format!(
                        "invalid scope value: {other}"
                    )));
                }
            }
            req.payload.namespaces = Some(vec!["*".to_string()]);
            req.payload.collection = None;
        } else {
            req.payload.namespaces = Some(namespaces);
            req.payload.collection = None;
        }
        req.namespace = None;
    }

    if let Some(ns) = req.namespace.clone().filter(|v| !v.trim().is_empty()) {
        if req
            .payload
            .namespaces
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            return Err(AppError::BadRequest(
                "namespace and namespaces cannot both be provided".to_string(),
            ));
        }
        if ns == "*" {
            match req.payload.scope.as_deref() {
                Some("collection") => {
                    return Err(AppError::BadRequest(
                        "namespace='*' conflicts with payload.scope='collection'".to_string(),
                    ));
                }
                Some("all") | None => {
                    req.payload.scope = Some("all".to_string());
                }
                Some(other) => {
                    return Err(AppError::BadRequest(format!(
                        "invalid scope value: {other}"
                    )));
                }
            }
            req.payload.collection = None;
            req.payload.namespaces = Some(vec!["*".to_string()]);
        } else if let Some(existing) = req
            .payload
            .collection
            .as_ref()
            .filter(|v| !v.trim().is_empty())
        {
            if existing != &ns {
                return Err(AppError::BadRequest(
                    "namespace/collection mismatch between top-level and payload".to_string(),
                ));
            }
        } else {
            req.payload.collection = Some(ns);
        }
    }

    if let Some(children) = req.data.as_mut() {
        for child in children {
            normalize_request_namespace(child)?;
        }
    }
    Ok(())
}

pub(crate) fn validate_access_key(state: &AppState, headers: &HeaderMap) -> AppResult<()> {
    let Some(expected) = state.access_key.as_ref() else {
        return Ok(());
    };

    let header_name = HeaderName::from_static("x-access-key");
    let header_value = headers.get(&header_name).and_then(|v| v.to_str().ok());
    let basic_value = basic_auth_password(headers);
    let provided = header_value.or(basic_value.as_deref());

    let Some(provided) = provided else {
        return Err(AppError::Unauthorized(
            "missing X-Access-Key or HTTP Basic credentials".to_string(),
        ));
    };

    if constant_time_eq(expected.as_bytes(), provided.as_bytes()) {
        Ok(())
    } else {
        Err(AppError::Unauthorized("invalid X-Access-Key".to_string()))
    }
}

fn basic_auth_password(headers: &HeaderMap) -> Option<String> {
    let encoded = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Basic ")?;
    let decoded = STANDARD.decode(encoded).ok()?;
    let decoded = std::str::from_utf8(&decoded).ok()?;
    decoded
        .split_once(':')
        .and_then(|(username, password)| (username == "kongodb").then(|| password.to_string()))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub async fn ping() -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "service": "kongo",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

pub async fn docs(State(state): State<AppState>) -> AppResult<Html<String>> {
    let md = fs::read_to_string(&state.docs_file).map_err(|e| {
        AppError::Internal(format!(
            "docs file read failed: path={} err={e}",
            state.docs_file
        ))
    })?;
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(&md, options);
    let mut body = String::new();
    html::push_html(&mut body, parser);
    let page = format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Kongodb Docs</title>
  <style>
    * {{
      box-sizing: border-box;
    }}

    body {{
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "Noto Sans", Helvetica, Arial, sans-serif;
      line-height: 1.6;
      margin: 0;
      background: #f7f8fa;
      color: #1f2937;
      padding: 1rem;
    }}

    main {{
      max-width: 1000px;
      margin: 0 auto;
      background: #ffffff;
      border: 1px solid #e5e7eb;
      border-radius: 12px;
      padding: 2rem;
      box-shadow: 0 1px 2px rgba(0,0,0,0.05);
    }}

    @media (max-width: 640px) {{
      main {{
        padding: 1.25rem;
        border-radius: 10px;
      }}
      body {{
        padding: 0.5rem;
      }}
    }}

    /* Typography */
    h1, h2, h3, h4, h5, h6 {{
      margin-top: 1.5em;
      margin-bottom: 0.5em;
      font-weight: 600;
      line-height: 1.25;
    }}

    h1 {{
        font-size: 4.25em;
        border-bottom: 2px solid #e5e7eb;
        padding-bottom: 0.3em;
        margin-top: 0;
        color: #082f49;
    }}

    h2 {{
      font-size: 1.75em;
      border-bottom: 1px solid #e5e7eb;
      padding-bottom: 0.2em;
    }}
    h3 {{ font-size: 1.5em; }}
    h4 {{ font-size: 1.25em; }}
    h5 {{ font-size: 1.1em; }}
    h6 {{ font-size: 1em; color: #6b7280; }}

    p {{
      margin: 1em 0;
    }}

    /* Links */
    a {{
      color: #0b57d0;
      text-decoration: none;
      transition: color 0.2s ease;
    }}

    a:hover {{
      color: #0842a0;
      text-decoration: underline;
    }}

    /* Code */
    code {{
        font-family: ui-monospace, "SF Mono", SFMono-Regular, Menlo, Consolas, monospace;
        font-size: 0.875em;
        background: #f1f5f9;
        padding: 0.2em 0.4em;
        border-radius: 0px;
        color: #075985;

    }}

    pre {{
        background: #0f172a;
        color: #dbe4ff;
        font-size: 16px;
        padding: 1rem;
        border-radius: 10px;
        overflow-x: auto;
        /* line-height: 1.45; */
        margin: 1.25em 0;
    }}

    pre code {{
      background: none;
      padding: 0;
      color: inherit;
      font-size: 0.85em;
    }}

    /* Blockquotes */
    blockquote {{
      margin: 1em 0;
      padding: 0.5em 1em;
      border-left: 4px solid #e5e7eb;
      background: #f9fafb;
      border-radius: 0 6px 6px 0;
      color: #4b5563;
    }}

    /* Lists */
    ul, ol {{
      margin: 1em 0;
      padding-left: 2em;
    }}

    li {{
      margin: 0.25em 0;
    }}

    li > ul, li > ol {{
      margin: 0.25em 0;
    }}

    /* Tables */
    table {{
      border-collapse: collapse;
      width: 100%;
      margin: 1.25em 0;
      overflow-x: auto;
      display: block;
    }}

    th, td {{
      border: 1px solid #e5e7eb;
      padding: 10px 12px;
      text-align: left;
    }}

    th {{
      background: #f9fafb;
      font-weight: 600;
    }}

    tr:nth-child(even) {{
      background: #fafbfc;
    }}

    /* Images */
    img {{
      max-width: 100%;
      height: auto;
      border-radius: 8px;
    }}

    /* Horizontal rule */
    hr {{
      border: none;
      border-top: 2px solid #e5e7eb;
      margin: 2em 0;
    }}

    /* Task lists */
    input[type="checkbox"] {{
      margin-right: 0.5em;
    }}
  </style>
</head>
<body>
  <main>{}</main>
</body>
</html>"#,
        body
    );
    Ok(Html(page))
}

pub async fn meta_operations(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    validate_access_key(&state, &headers)?;
    let payload = format!(
        r#"{{
  "service":"kongodb",
  "version":"{}",
  "endpoint":"{}",
  "operations":{{
    "create_db":{{"writes":true,"required":[],"optional":[],"description":"Create/initialize db. Missing db is only creatable via create_db, insert, or import_jsonl."}},
    "db_exists":{{"writes":false,"required":[],"optional":[],"description":"Check whether db exists (remote-aware in s3 mode: local or remote)"}},
    "list_commands":{{"writes":false,"required":[],"optional":[],"description":"List all supported gateway operations as plain command names"}},
    "list_dbs":{{"writes":false,"required":[],"optional":[],"description":"List currently loaded/open dbs in this instance, including location flags (on_local,on_s3) and local file size (bytes) when present"}},
    "list_all_dbs":{{"writes":false,"required":[],"optional":[],"description":"List all known dbs. local mode: filesystem scan; s3 mode: union of loaded, local files, and remote manifests. Includes location flags (on_local,on_s3) and loaded flag"}},
    "system_get_inventory":{{"writes":false,"required":[],"optional":["limit","offset"],"description":"List DB inventory from the always-on internal system catalog"}},
    "system_refresh_inventory":{{"writes":true,"required":[],"optional":[],"description":"Scan local/S3 DBs and update the internal system catalog inventory"}},
    "system_get_db_status":{{"writes":false,"required":["db"],"optional":[],"description":"Return live status for one DB and its system-catalog row"}},
    "system_snapshot_db_stats":{{"writes":true,"required":[],"optional":["db"],"description":"Persist stats snapshots for active DBs, or one DB when db is provided"}},
    "system_query_db_stats":{{"writes":false,"required":[],"optional":["db","start","end","limit","offset"],"description":"Query historical DB stats snapshots from the internal system catalog"}},
    "system_list_db_events":{{"writes":false,"required":[],"optional":["db","limit","offset"],"description":"List DB lifecycle/error events from the internal system catalog"}},
    "get_system_stats":{{"writes":false,"required":[],"optional":[],"description":"Instance-local in-memory uptime, request counters, rolling windows, process memory, active DBs, and write queue usage"}},
    "system_memory":{{"writes":false,"required":[],"optional":[],"description":"Process memory usage + write queue usage for this instance, including system_stats"}},
    "cleanup_temp_artifacts":{{"writes":true,"required":[],"optional":["older_than_secs(default=600)"],"description":"Delete stale temp files (`._tmp_*`,`_tmp_*`) under data dir"}},
    "insert":{{"writes":true,"required":["namespace","data(object|array<object>)"],"optional":["_user_id","ttl_seconds","expiry_behavior(archive|delete)","allow_system_timestamps","unique_fields(composite)","on_conflict(skip|error)","commit","dry_run"],"description":"Insert one or many documents. _user_id can be supplied in payload or per data object and is stored as a document column. commit=false returns prepared pending items with generated ids/timestamps before the queued write commits"}},
    "update":{{"writes":true,"required":["data(object with _id)|filter+data(object)|data(array<object with _id>)"],"optional":["namespace|scope=all","replace(single only)","max_docs","commit","dry_run"],"description":"Update one or many documents. replace=true is only allowed for single-document update by _id. commit=false explicit-id updates return prepared pending items; filter updates return queued acknowledgement"}},
    "set":{{"writes":true,"required":["data(object with _id)"],"optional":["namespace","_user_id","ttl_seconds","expiry_behavior(archive|delete)","dry_run"],"description":"Set one document by _id: with namespace => upsert; without namespace => update-only (not-found if missing). _user_id is stored as a document column"}},
    "upsert":{{"writes":true,"required":["filter","insert_data"],"optional":["_user_id","update_data","expiry_behavior(archive|delete)","max_docs","dry_run"],"description":"Update matches or insert on miss. _user_id applies to the insert path"}},
    "get_stats":{{"writes":false,"required":["namespace"],"optional":[],"description":"Live/archive stats for one namespace"}},
    "get_db_stats":{{"writes":false,"required":[],"optional":[],"description":"Return current in-memory request counters for the current db"}},
    "snapshot_db_stats":{{"writes":true,"required":[],"optional":[],"description":"Persist one current-db stats snapshot into __kdb_db_stats_rollups"}},
    "query_db_stats":{{"writes":false,"required":[],"optional":["start","end","limit(default=100,max=1000)"],"description":"Query persisted db stats snapshots from __kdb_db_stats_rollups"}},
    "get_system_config":{{"writes":false,"required":[],"optional":[],"description":"List key/value rows from the internal system config table for current db"}},
    "recompute_stats":{{"writes":true,"required":[],"optional":[],"description":"Enqueue admin job to recompute __kdb_system_stats globally"}},
    "list_namespaces":{{"writes":false,"required":[],"optional":[],"description":"List namespaces with live/archive stats"}},
    "list_tables":{{"writes":false,"required":[],"optional":[],"description":"List user-created tables for the current db (excludes __kdb_* and sqlite_* tables)"}},
    "get_table_schema":{{"writes":false,"required":["table"],"optional":[],"description":"Return schema columns for one user-created SQL table. Safely wraps PRAGMA table_info without exposing PRAGMA through sql_execute"}},
    "change_namespace":{{"writes":true,"required":["from_namespace","to_namespace"],"optional":["ids|filter|max_docs|dry_run"],"description":"Move documents from one namespace to another by updating namespace and stats"}},
    "rename_namespace":{{"writes":true,"required":["from_namespace","to_namespace"],"optional":[],"description":"Rename a namespace across live+__kdb_archive tables and refresh stats"}},
    "vacuum_db":{{"writes":true,"required":[],"optional":[],"description":"Enqueue admin job to run SQLite VACUUM (rebuild/compact database file)"}},
    "reap_db":{{"writes":true,"required":[],"optional":[],"description":"Run TTL reaper now for current db"}},
    "load_db":{{"writes":true,"required":[],"optional":[],"description":"s3 only: load and warm db into memory/cache without querying. Returns loaded=true if newly loaded, false if already loaded"}},
    "sync_db":{{"writes":true,"required":[],"optional":[],"description":"s3 only: force snapshot+manifest sync for current db"}},
    "create_snapshot":{{"writes":true,"required":[],"optional":[],"description":"Alias of sync_db"}},
    "list_snapshots":{{"writes":false,"required":[],"optional":[],"description":"s3 only: list available versioned snapshots for current db"}},
    "compact_wal":{{"writes":true,"required":[],"optional":["retain_segments"],"description":"s3 only: compact manifest segment list and retain latest N segments (default 1000)"}},
    "get_sync_status":{{"writes":false,"required":[],"optional":[],"description":"s3 only: inspect local/remote manifest+snapshot status for current db"}},
    "verify_db":{{"writes":false,"required":[],"optional":[],"description":"s3 only: verify manifest/snapshot/segment object presence for current db"}},
    "restore_snapshot":{{"writes":true,"required":[],"optional":["snapshot_id"],"description":"s3 only: drop local copy and restore current db from snapshot. snapshot_id omitted => latest"}},
    "restore_backup":{{"writes":true,"required":["backup_db_path|backup_id|backup_tag|backup_at|latest=true"],"optional":[],"description":"Restore db from catalog selector or explicit backup path (local path or s3:// in s3 mode). Supports .zst backups"}},
    "create_index":{{"writes":true,"required":["index_path"],"optional":["index_name"],"description":"Create a manual expression index on documents JSON path"}},
    "drop_index":{{"writes":true,"required":["index_name|index_path"],"optional":[],"description":"Drop index by explicit index_name, or by index_path (manual+auto derived names)"}},
    "list_indexes":{{"writes":false,"required":[],"optional":[],"description":"List indexes currently defined for the internal documents table"}},
    "reindex_fts":{{"writes":true,"required":[],"optional":[],"description":"Enqueue async FTS rebuild job for current db (create/rebuild + backfill)"}},
    "drop_fts_index":{{"writes":true,"required":[],"optional":[],"description":"Enqueue async FTS drop job for current db (drop FTS table/triggers)"}},
    "enable_fts_index":{{"writes":true,"required":[],"optional":["enable(default=true)"],"description":"Set db-level FTS access flag only; indexing lifecycle is controlled by reindex_fts/drop_fts_index"}},
    "clone_db":{{"writes":true,"required":["to_db_path"],"optional":[],"description":"Clone current db into to_db_path using snapshot copy"}},
    "create_backup":{{"writes":true,"required":[],"optional":["backup_db_path","backup_tag"],"description":"Enqueue async backup job for current db. backup_db_path can be local path or s3://bucket/object-path. Directory/prefix targets generate compressed .db.zst backups"}},
    "list_backups":{{"writes":false,"required":[],"optional":["backup_tag","limit","offset"],"description":"List backup catalog entries for current db"}},
    "tag_backup":{{"writes":true,"required":["backup_id|backup_db_path"],"optional":["backup_tag"],"description":"Set or clear backup tag for a catalog entry"}},
    "offload_db":{{"writes":true,"required":[],"optional":[],"description":"s3 only: flush/sync, close connection, remove local db files"}},
    "delete":{{"writes":true,"required":["id|ids|filter"],"optional":["namespace|scope=all","ttl_seconds","purge","max_docs","dry_run"],"description":"Soft delete by default: move matched documents to __kdb_archive and remove from live data. purge=true hard-deletes instead"}},
    "drop_namespace":{{"writes":true,"required":["namespace"],"optional":["ttl_seconds","max_docs","purge","dry_run"],"description":"Archive+delete namespace or hard delete when purge=true"}},
    "set_ttl":{{"writes":true,"required":["ids|filter","ttl_seconds"],"optional":["namespace|scope=all","expiry_behavior(archive|delete)","max_docs","dry_run"],"description":"Set/reset document TTL and optional expiry behavior. ids mode allows optional namespace; filter mode requires namespace unless scope=all"}},
    "purge_archive":{{"writes":true,"required":["txn_id|ids|namespace/filter"],"optional":["dry_run"],"description":"Hard delete documents from archive only (txn_id maps to archive _txn_id)"}},
    "restore_archive":{{"writes":true,"required":["txn_id|ids|namespace/filter"],"optional":["on_conflict(skip|replace|patch)","dry_run"],"description":"Restore documents from archive. on_conflict controls skip/replace/patch; txn_id maps to archive _txn_id"}},
    "get":{{"writes":false,"required":["id|data._id|ids"],"optional":["namespace|namespaces|scope=all","_user_id","attach_users","attach_user_fields(default=id,first_name,last_name,profile_photo)","include_archive","archive_only","fields","exclude_fields","include_namespace(include_name)","cache","force_db"],"description":"Read one/many by id(s). namespace optional; if provided, match must satisfy namespace + id(s). By default, explicit id reads check pending accepted insert/update previews first; force_db=true reads durable DB rows only. attach_users side-loads Identity users into attachments.users"}},
    "count":{{"writes":false,"required":[],"optional":["namespace|scope=all","filter","include_archive","archive_only","cache"],"description":"Count matching documents"}},
    "aggregate":{{"writes":false,"required":["compute"],"optional":["namespace|scope=all","filter","include_archive","archive_only","cache"],"description":"Set-level compute metrics over matched documents"}},
    "search":{{"writes":false,"required":["namespace|namespaces|namespace='*'","search"],"optional":["_user_id","attach_users","attach_user_fields(default=id,first_name,last_name,profile_photo)","filter","sort(object|string)","limit","offset","page","per_page","lookups(map)","lookup_depth_override","fields","exclude_fields","include_namespace(include_name)","cache"],"description":"FTS search over live documents using SQLite FTS5 MATCH (requires enabled FTS). attach_users side-loads Identity users into attachments.users"}},
    "query":{{"writes":false,"required":["namespace|namespaces|namespace='*'"],"optional":["_user_id","attach_users","attach_user_fields(default=id,first_name,last_name,profile_photo)","filter","sort(object|string)","limit","offset","page","per_page","compute","lookups(map)","lookup_depth_override","fields","exclude_fields","include_namespace(include_name)","include_archive","archive_only","explain","cache"],"description":"Query documents with optional per-item compute, lookup joins, and user attachments"}},
    "metrics_ingest":{{"writes":true,"required":["events(array<object>)"],"optional":["commit(default=false)"],"description":"Append metric events. events[] requires event; ts defaults to server UTC now; value defaults to 1; dimensions/metadata must be objects when provided"}},
    "metrics_query":{{"writes":false,"required":["event|events","range|start+end","metrics"],"optional":["alias","label","interval","bucket_label","filter","group_by","sort","limit","offset","batch"],"description":"Aggregate metric events into named result sets with groups and metrics labels"}},
    "metrics_catalog":{{"writes":false,"required":[],"optional":["type(event|dimension)","name","value","limit","offset"],"description":"List discovered metrics catalog entries. Ingest registers event names and dimension paths as type/name/value rows"}},
    "audit_ingest":{{"writes":true,"required":["events(array<object>)"],"optional":["commit"],"description":"Append immutable audit events. events[] requires action; ts defaults to server UTC now; actor, target, status, request metadata, message, and data are optional"}},
    "audit_query":{{"writes":false,"required":[],"optional":["search","action","actor_type","actor_id","target_type","target_id","status","source","request_id","start","end","limit","offset","page","per_page"],"description":"Query append-only audit logs newest first with structured filters and pagination"}},
    "user_create":{{"writes":true,"required":[],"optional":["user_id","email","username","phone","first_name","last_name","profile_photo","status(default=active)","status_reason","password_hash","password_algo","requires_password_change(default=false)","provider","provider_user_id","data"],"description":"Create identity user metadata. Does not authenticate; password_hash/token values are app-provided"}},
    "user_get":{{"writes":false,"required":["user_id|id|email|username|provider+provider_user_id"],"optional":[],"description":"Fetch one identity user by local id, email, username, or provider mapping"}},
    "user_list":{{"writes":false,"required":[],"optional":["search|q","status","email","username","limit","offset","page","per_page"],"description":"List/query identity users with pagination"}},
    "user_get_details":{{"writes":false,"required":["user_id|id|email|username"],"optional":[],"description":"Fetch one identity user with login methods, providers, and recent identity events"}},
    "user_update":{{"writes":true,"required":["user_id|id"],"optional":["email","username","phone","first_name","last_name","profile_photo","requires_password_change","email_verified_at","phone_verified_at","data"],"description":"Update identity user profile metadata and app data; does not authenticate"}},
    "user_update_status":{{"writes":true,"required":["user_id|id","status"],"optional":["status_reason","status_expires_at|status_expires_in","status_next","status_next_reason","changed_by"],"description":"Update app-defined user status and optionally schedule a future transition; logs an identity event"}},
    "user_delete":{{"writes":true,"required":["user_id|id"],"optional":["purge","status_reason"],"description":"Soft delete user by default and revoke active tokens. purge=true hard-deletes user, providers, tokens, and events"}},
    "user_create_token":{{"writes":true,"required":["user_id|id","kind","token_hash"],"optional":["expires_at|expires_in","allow_multi(default=false)","data"],"description":"Store an app-generated token hash. allow_multi=false revokes active tokens for the same user+kind before insert"}},
    "user_link_provider":{{"writes":true,"required":["user_id|id","provider","provider_user_id"],"optional":["email","data"],"description":"Link an OAuth/custom provider identity to a local user"}},
    "user_unlink_provider":{{"writes":true,"required":["provider","provider_user_id"],"optional":["user_id|id"],"description":"Unlink one provider identity; user_id makes the unlink strict when provided"}},
    "file_create":{{"writes":true,"required":["storage_backend","storage_path"],"optional":["id(dashless uuid4)","bucket(default=default)","filename","content_type","size_bytes","sha256","status(default=active)","owner_type","owner_id","metadata","uploaded_at(default=now)","expires_at"],"description":"Create file/object metadata only. Kongodb does not upload, download, or delete file bytes"}},
    "file_get":{{"writes":false,"required":["id"],"optional":[],"description":"Fetch one file metadata row by id"}},
    "file_list":{{"writes":false,"required":[],"optional":["bucket","status","owner_type","owner_id","storage_backend","content_type","search|q","limit","offset","page","per_page"],"description":"List/query file metadata rows with pagination"}},
    "file_update":{{"writes":true,"required":["id"],"optional":["bucket","storage_backend","storage_path","filename","content_type","size_bytes","sha256","status","owner_type","owner_id","metadata","uploaded_at","expires_at"],"description":"Update mutable file metadata only; does not move object bytes"}},
    "file_delete":{{"writes":true,"required":["id"],"optional":["purge"],"description":"Soft delete file metadata by default (status=deleted). purge=true hard-deletes metadata row only"}},
    "sql_execute":{{"writes":true,"required":["sql"],"optional":["params","commit"],"description":"Config-gated direct SQL execution. Supports a single SELECT/WITH/EXPLAIN/INSERT/UPDATE/DELETE/REPLACE statement, plus CREATE TABLE, CREATE INDEX, DROP INDEX, and ALTER TABLE ... ADD COLUMN for non-__kdb_* objects; write statements follow normal commit=false accepted-ack queue behavior"}},
    "export_jsonl":{{"writes":true,"required":[],"optional":["target_path","compress(default=true)","include_system_timestamps(default=true)","namespace|scope=all","filter","sort(object|string)","limit","offset","page","per_page","fields","exclude_fields","include_archive","archive_only"],"description":"Create async JSONL export job and return job_id + resolved target_path"}},
    "import_jsonl":{{"writes":true,"required":["namespace","source_path"],"optional":["source_hash","alias_import_pk","drop_keys","on_conflict(error|skip|replace|merge)","ignore_input_id","allow_system_timestamps","batch_size","resumable"],"description":"Create async JSONL import job and return job_id. Background worker streams batches into namespace"}},
    "get_job":{{"writes":false,"required":["job_id"],"optional":["job_type(import_jsonl|export_jsonl|create_backup|reindex_fts|drop_fts_index|vacuum_db|recompute_stats|replication)"],"description":"Get one async job by id"}},
    "list_jobs":{{"writes":false,"required":[],"optional":["job_type(import_jsonl|export_jsonl|create_backup|reindex_fts|drop_fts_index|vacuum_db|recompute_stats|replication)","status","limit","offset"],"description":"List async jobs across supported job types"}},
    "continue_job":{{"writes":true,"required":["job_id"],"optional":["job_type(import_jsonl|export_jsonl|create_backup|reindex_fts|drop_fts_index|vacuum_db|recompute_stats|replication)"],"description":"Resume/retry a job when supported by its job type"}},
    "abort_job":{{"writes":true,"required":["job_id"],"optional":["job_type(import_jsonl|export_jsonl|create_backup|reindex_fts|drop_fts_index|vacuum_db|recompute_stats|replication)"],"description":"Abort/cancel a job when supported by its job type"}},
    "transaction":{{"writes":true,"required":["data(array<operation>)"],"optional":[],"description":"Atomic operation array"}}
  }}
}}"#,
        env!("CARGO_PKG_VERSION"),
        format!("{}{}", state.base_path, "/gateway")
    );
    let value = serde_json::from_str(&payload).unwrap_or_else(
        |_| json!({"status":"error","error":"meta/operations payload build failed"}),
    );
    Ok(Json(value))
}
