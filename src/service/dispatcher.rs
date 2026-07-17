//! Gateway operation dispatcher and handlers for CRUD, bulk ops, TTL, and transactions.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Cursor;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Datelike, Duration, SecondsFormat, Utc};
use memory_stats::memory_stats;
use petgraph::visit::EdgeRef;
use petgraph::{algo::toposort, graph::DiGraph};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::{sync::Semaphore, task::JoinSet};
use uuid::Uuid;

use crate::{
    api::dto::{CacheHint, GatewayRequest, GatewayResponse, OperationPayload},
    error::{AppError, AppResult},
    query::jql::build_where,
    state::AppState,
    storage::auto_index::{bump_query_heatmap, create_manual_index, drop_index, list_indexes},
    storage::reaper::reap_conn,
    storage::schema::{get_bool_config, reindex_fts, set_bool_config, set_fts_index_enabled},
    storage::system_catalog::{SystemDbEventRecord, SystemDbRecord, SystemDbStatsRecord},
};

pub async fn dispatch(
    state: &AppState,
    db_path: &str,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    Box::pin(dispatch_inner(state, db_path, req, false)).await
}

pub async fn dispatch_write_worker(
    state: &AppState,
    db_path: &str,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    Box::pin(dispatch_inner(state, db_path, req, true)).await
}

async fn dispatch_inner(
    state: &AppState,
    db_path: &str,
    mut req: GatewayRequest,
    from_write_worker: bool,
) -> AppResult<GatewayResponse> {
    if req.operation == "list_commands" {
        return list_commands().await;
    }
    if req.operation == "list_dbs" {
        return list_dbs(state).await;
    }
    if req.operation == "list_all_dbs" {
        return list_all_dbs(state).await;
    }
    if req.operation == "cleanup_temp_artifacts" {
        return cleanup_temp_artifacts(state, req).await;
    }
    if req.operation == "system_get_inventory" {
        return system_get_inventory(state, req).await;
    }
    if req.operation == "system_refresh_inventory" {
        return system_refresh_inventory(state).await;
    }
    if req.operation == "system_get_db_status" {
        return system_get_db_status(state, db_path).await;
    }
    if req.operation == "system_snapshot_db_stats" {
        return system_snapshot_db_stats(state, db_path).await;
    }
    if req.operation == "system_query_db_stats" {
        return system_query_db_stats(state, db_path, req).await;
    }
    if req.operation == "system_list_db_events" {
        return system_list_db_events(state, db_path, req).await;
    }
    if req.operation == "system_memory" || req.operation == "get_system_stats" {
        return system_memory(state).await;
    }
    if req.operation == "metrics_ingest" && req.payload.commit.is_none() {
        req.payload.commit = Some(false);
    }
    if req.operation == "audit_ingest" && req.payload.commit.is_none() {
        req.payload.commit = Some(true);
    }
    let requested_ack_mode = resolve_ack_mode(state, req.payload.commit)?;
    let mut ack_mode_fallback = false;
    if req.operation == "create_db" {
        return create_db(state, db_path).await;
    }
    if req.operation == "load_db" {
        return load_db(state, db_path).await;
    }
    if req.operation == "db_exists" {
        return db_exists(state, db_path).await;
    }
    if req.operation == "sync_db" {
        return sync_db(state, db_path).await;
    }
    if req.operation == "create_snapshot" {
        return sync_db(state, db_path).await;
    }
    if req.operation == "list_snapshots" {
        return list_snapshots(state, db_path).await;
    }
    if req.operation == "get_sync_status" {
        return get_sync_status(state, db_path).await;
    }
    if req.operation == "verify_db" {
        return verify_db(state, db_path).await;
    }
    if req.operation == "restore_snapshot" {
        return restore_snapshot(state, db_path, req).await;
    }
    if req.operation == "restore_backup" {
        return restore_backup(state, db_path, req).await;
    }
    if req.operation == "compact_wal" {
        return compact_wal(state, db_path, req).await;
    }
    if state.legacy_aliases_enabled {
        let op_name = req.operation.clone();
        normalize_request_legacy_aliases(op_name.as_str(), &mut req, state)?;
    }

    let is_write_request = request_is_write(&req);
    if !from_write_worker && is_write_request {
        match requested_ack_mode {
            AckMode::Accepted if supports_accepted_ack_request(&req) => {
                validate_accepted_preflight(&req)?;
                let use_prepared = supports_pending_prepared_preview(&req);
                let ack_preview = if use_prepared {
                    None
                } else {
                    Some(prepare_accepted_ack_preview(&mut req)?)
                };
                let queued = if use_prepared {
                    state.enqueue_prepared_write(db_path, req).await?
                } else {
                    state.try_enqueue_write(db_path, req).await?
                };
                match queued {
                    crate::state::WriteEnqueueResult::Enqueued => {
                        let mut response = GatewayResponse::ok(ack_preview);
                        response.ack_mode = Some("accepted".to_string());
                        response.ack_status = Some("queued".to_string());
                        response.committed = Some(false);
                        response.is_async_ack = Some(true);
                        return Ok(response);
                    }
                    crate::state::WriteEnqueueResult::Fallback(r) => {
                        match state.enqueue_committed_write(db_path, r).await? {
                            crate::state::WriteEnqueueResult::Committed(result) => {
                                let mut response = result?;
                                response.ack_mode = Some("accepted".to_string());
                                response.ack_status = Some("committed_fallback".to_string());
                                response.committed = Some(true);
                                response.is_async_ack = Some(true);
                                return Ok(response);
                            }
                            crate::state::WriteEnqueueResult::Fallback(r) => {
                                req = r;
                                ack_mode_fallback = true;
                            }
                            crate::state::WriteEnqueueResult::Enqueued => {
                                return Err(AppError::Internal(
                                    "accepted write fallback unexpectedly enqueued without response channel"
                                        .to_string(),
                                ));
                            }
                        }
                    }
                    crate::state::WriteEnqueueResult::Committed(result) => {
                        let mut response = result?;
                        response.ack_mode = Some("accepted".to_string());
                        response.ack_status = Some("queued".to_string());
                        response.committed = Some(false);
                        response.is_async_ack = Some(true);
                        return Ok(response);
                    }
                }
            }
            AckMode::Committed => match state.enqueue_committed_write(db_path, req).await? {
                crate::state::WriteEnqueueResult::Committed(result) => return result,
                crate::state::WriteEnqueueResult::Fallback(r) => {
                    req = r;
                }
                crate::state::WriteEnqueueResult::Enqueued => {
                    return Err(AppError::Internal(
                        "committed write unexpectedly enqueued without response channel"
                            .to_string(),
                    ));
                }
            },
            AckMode::Accepted => match state.enqueue_committed_write(db_path, req).await? {
                crate::state::WriteEnqueueResult::Committed(result) => return result,
                crate::state::WriteEnqueueResult::Fallback(r) => {
                    req = r;
                }
                crate::state::WriteEnqueueResult::Enqueued => {
                    return Err(AppError::Internal(
                        "accepted write unexpectedly enqueued without response channel".to_string(),
                    ));
                }
            },
        }
    }

    let allow_create = matches!(req.operation.as_str(), "insert" | "import_jsonl");
    let conn = state
        .db_manager
        .get_conn_with_create(db_path, allow_create)
        .await?;
    let operation = req.operation.clone();
    let payload_for_invalidation = req.payload.clone();
    let mut result = match operation.as_str() {
        "insert" => insert(state, db_path, &conn, req).await,
        "update" => update(state, &conn, req).await,
        "upsert" => upsert(state, db_path, &conn, req).await,
        "set" => set(state, &conn, req).await,
        "get_stats" => get_stats(&conn, req).await,
        "get_db_stats" => get_db_stats(state, db_path).await,
        "snapshot_db_stats" => snapshot_db_stats(state, db_path, &conn).await,
        "query_db_stats" => query_db_stats(&conn, req).await,
        "get_system_config" => get_system_config(&conn).await,
        "recompute_stats" => recompute_stats(&conn, req).await,
        "list_namespaces" => list_collections(&conn).await,
        "list_tables" => list_tables(&conn).await,
        "get_table_schema" => get_table_schema(&conn, req).await,
        "change_namespace" => change_namespace(state, db_path, &conn, req).await,
        "rename_namespace" => rename_namespace(&conn, req).await,
        "vacuum_db" => vacuum(&conn).await,
        "reap_db" => reap_db(state, db_path, &conn).await,
        "clone_db" => clone_db(state, db_path, req).await,
        "create_backup" => create_backup(state, db_path, req).await,
        "list_backups" => list_backups(&conn, req).await,
        "tag_backup" => tag_backup(&conn, req).await,
        "offload_db" => offload_db(state, db_path).await,
        "create_index" => create_index_op(&conn, req).await,
        "drop_index" => drop_index_op(&conn, req).await,
        "list_indexes" => list_index_op(&conn).await,
        "reindex_fts" => reindex_fts_op(state, &conn).await,
        "drop_fts_index" => drop_fts_index_op(&conn).await,
        "enable_fts_index" | "enable_ftx_index" => enable_fts_index_op(state, &conn, req).await,
        "delete" => delete(state, db_path, &conn, req).await,
        "drop_namespace" => drop_collection(state, db_path, &conn, req).await,
        "purge_archive" => purge_kdb_archive(&conn, req).await,
        "restore_archive" => restore_kdb_archive(&conn, req).await,
        "set_ttl" => set_ttl(&conn, req).await,
        "get" => get(state, db_path, &conn, req).await,
        "count" => count(state, db_path, &conn, req).await,
        "sql_execute" => sql_execute(state, db_path, &conn, req).await,
        "search" => search(state, db_path, &conn, req).await,
        "aggregate" => aggregate(state, db_path, &conn, req).await,
        "query" => query(state, db_path, &conn, req).await,
        "export_jsonl" => export_jsonl(state, db_path, &conn, req).await,
        "import_jsonl" => import_jsonl(state, db_path, &conn, req).await,
        "metrics_ingest" => metrics_ingest(state, db_path, &conn, req).await,
        "metrics_query" => metrics_query(state, db_path, &conn, req).await,
        "metrics_catalog" => metrics_catalog(&conn, req).await,
        "audit_ingest" => Box::pin(audit_ingest(state, db_path, &conn, req)).await,
        "audit_query" => Box::pin(audit_query(state, &conn, req)).await,
        "user_create" => user_create(state, &conn, req).await,
        "user_get" => user_get(&conn, req).await,
        "user_list" => user_list(&conn, req).await,
        "user_get_details" => user_get_details(&conn, req).await,
        "user_update" => user_update(state, &conn, req).await,
        "user_update_status" => user_update_status(&conn, req).await,
        "user_delete" => user_delete(&conn, req).await,
        "user_create_token" => user_create_token(state, &conn, req).await,
        "user_link_provider" => user_link_provider(state, &conn, req).await,
        "user_unlink_provider" => user_unlink_provider(&conn, req).await,
        "file_create" => file_create(state, &conn, req).await,
        "file_get" => file_get(&conn, req).await,
        "file_list" => file_list(&conn, req).await,
        "file_update" => file_update(state, &conn, req).await,
        "file_delete" => file_delete(&conn, req).await,
        "get_job" => get_job(&conn, req).await,
        "list_jobs" => list_jobs(&conn, req).await,
        "continue_job" => continue_job(&conn, req).await,
        "abort_job" => abort_job(&conn, req).await,
        "transaction" => transaction(state, db_path, &conn, req).await,
        other => Err(AppError::BadRequest(format!("unknown operation: {other}"))),
    };

    if let Ok(response) = result.as_mut() {
        if state.legacy_aliases_enabled {
            apply_response_legacy_aliases(response, state);
        }
    }

    if result.is_ok() && is_write_request {
        invalidate_read_cache_after_write(state, db_path, &operation, &payload_for_invalidation);
    }

    if ack_mode_fallback {
        if let Ok(response) = result.as_mut() {
            response.ack_mode = Some("accepted".to_string());
            response.ack_status = Some("committed_fallback".to_string());
            response.committed = Some(true);
            response.is_async_ack = Some(true);
        }
    }

    if let Ok(response) = result.as_mut() {
        if is_write_request && response.committed.is_none() {
            response.committed = Some(true);
        }
        if is_write_request && response.is_async_ack.is_none() {
            response.is_async_ack = Some(false);
        }
    }

    result
}

include!("dispatcher/db_ops.rs");
include!("dispatcher/write_ops.rs");
include!("dispatcher/read_ops.rs");
include!("dispatcher/archive_ops.rs");
include!("dispatcher/metric_events_ops.rs");
include!("dispatcher/audit_logs_ops.rs");
include!("dispatcher/identity_ops.rs");
include!("dispatcher/file_ops.rs");

include!("dispatcher/jobs_ops.rs");
include!("dispatcher/tx_mutation.rs");
include!("dispatcher/query_support.rs");
include!("dispatcher/lookup_support.rs");
include!("dispatcher/runtime_support.rs");
