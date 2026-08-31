//! Gateway operation dispatcher and handlers for CRUD, bulk ops, TTL, and transactions.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::io::Cursor;
use std::path::PathBuf;
use std::pin::Pin;
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
    api::dto::{
        BatchOperation, CacheHint, GatewayRequest, GatewayResponse, NamespaceSelector,
        OperationPayload,
    },
    error::{AppError, AppResult},
    query::jql::{build_where, build_where_on_base},
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
    heap_future(dispatch_inner(state, db_path, req, false)).await
}

pub async fn dispatch_write_worker(
    state: &AppState,
    db_path: &str,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    heap_future(dispatch_inner(state, db_path, req, true)).await
}

fn heap_future<'a, T>(
    future: impl Future<Output = T> + Send + 'a,
) -> Pin<Box<dyn Future<Output = T> + Send + 'a>> {
    Box::pin(future)
}

async fn dispatch_inner(
    state: &AppState,
    db_path: &str,
    mut req: GatewayRequest,
    from_write_worker: bool,
) -> AppResult<GatewayResponse> {
    if req.operation == "list_commands" {
        return heap_future(list_commands()).await;
    }
    if req.operation == "list_dbs" {
        return heap_future(list_dbs(state)).await;
    }
    if req.operation == "list_all_dbs" {
        return heap_future(list_all_dbs(state)).await;
    }
    if req.operation == "cleanup_temp_artifacts" {
        return heap_future(cleanup_temp_artifacts(state, req)).await;
    }
    if req.operation == "system_get_inventory" {
        return heap_future(system_get_inventory(state, req)).await;
    }
    if req.operation == "system_refresh_inventory" {
        return heap_future(system_refresh_inventory(state)).await;
    }
    if req.operation == "system_get_db_status" {
        return heap_future(system_get_db_status(state, db_path)).await;
    }
    if req.operation == "system_snapshot_db_stats" {
        return heap_future(system_snapshot_db_stats(state, db_path)).await;
    }
    if req.operation == "system_query_db_stats" {
        return heap_future(system_query_db_stats(state, db_path, req)).await;
    }
    if req.operation == "system_list_db_events" {
        return heap_future(system_list_db_events(state, db_path, req)).await;
    }
    if req.operation == "system_memory" || req.operation == "get_system_stats" {
        return heap_future(system_memory(state)).await;
    }
    if req.operation == "__run_document_transitions" && !from_write_worker {
        return Err(AppError::BadRequest("unknown operation".to_string()));
    }
    if req.payload.lifecycle.is_some()
        && !matches!(req.operation.as_str(), "insert" | "update" | "upsert")
    {
        return Err(AppError::BadRequest(
            "payload.lifecycle is only supported by insert, update, and upsert".to_string(),
        ));
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
        return heap_future(create_db(state, db_path)).await;
    }
    if req.operation == "load_db" {
        return heap_future(load_db(state, db_path)).await;
    }
    if req.operation == "db_exists" {
        return heap_future(db_exists(state, db_path)).await;
    }
    if req.operation == "sync_db" {
        return heap_future(sync_db(state, db_path)).await;
    }
    if req.operation == "create_snapshot" {
        return heap_future(sync_db(state, db_path)).await;
    }
    if req.operation == "list_snapshots" {
        return heap_future(list_snapshots(state, db_path)).await;
    }
    if req.operation == "get_sync_status" {
        return heap_future(get_sync_status(state, db_path)).await;
    }
    if req.operation == "verify_db" {
        return heap_future(verify_db(state, db_path)).await;
    }
    if req.operation == "restore_snapshot" {
        return heap_future(restore_snapshot(state, db_path, req)).await;
    }
    if req.operation == "restore_backup" {
        return heap_future(restore_backup(state, db_path, req)).await;
    }
    if req.operation == "compact_wal" {
        return heap_future(compact_wal(state, db_path, req)).await;
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
                    heap_future(state.enqueue_prepared_write(db_path, req)).await?
                } else {
                    heap_future(state.try_enqueue_write(db_path, req)).await?
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
                        match heap_future(state.enqueue_committed_write(db_path, r)).await? {
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
            AckMode::Committed => {
                match heap_future(state.enqueue_committed_write(db_path, req)).await? {
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
                }
            }
            AckMode::Accepted => match heap_future(state.enqueue_committed_write(db_path, req))
                .await?
            {
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
    let conn = heap_future(state.db_manager.get_conn_with_create(db_path, allow_create)).await?;
    let operation = req.operation.clone();
    let payload_for_invalidation = req.payload.clone();
    let mut result = match operation.as_str() {
        "insert" => heap_future(insert(state, db_path, &conn, req)).await,
        "update" => heap_future(update(state, db_path, &conn, req)).await,
        "upsert" => heap_future(upsert(state, db_path, &conn, req)).await,
        "get_namespace_stats" => heap_future(get_namespace_stats(&conn, req)).await,
        "get_data_count" => heap_future(get_data_count(&conn, req)).await,
        "get_db_stats" => heap_future(get_db_stats(state, db_path)).await,
        "snapshot_db_stats" => heap_future(snapshot_db_stats(state, db_path, &conn)).await,
        "query_db_stats" => heap_future(query_db_stats(&conn, req)).await,
        "get_system_config" => heap_future(get_system_config(&conn)).await,
        "recompute_stats" => heap_future(recompute_stats(&conn, req)).await,
        "list_namespaces" => heap_future(list_collections(&conn)).await,
        "sql_list_tables" => heap_future(sql_list_tables(&conn)).await,
        "sql_get_table_schema" => heap_future(sql_get_table_schema(&conn, req)).await,
        "change_namespace" => heap_future(change_namespace(state, db_path, &conn, req)).await,
        "rename_namespace" => heap_future(rename_namespace(&conn, req)).await,
        "vacuum_db" => heap_future(vacuum(&conn)).await,
        "reap_db" => heap_future(reap_db(state, db_path, &conn)).await,
        "clone_db" => heap_future(clone_db(state, db_path, req)).await,
        "create_backup" => heap_future(create_backup(state, db_path, req)).await,
        "list_backups" => heap_future(list_backups(&conn, req)).await,
        "tag_backup" => heap_future(tag_backup(&conn, req)).await,
        "offload_db" => heap_future(offload_db(state, db_path)).await,
        "create_index" => heap_future(create_index_op(&conn, req)).await,
        "drop_index" => heap_future(drop_index_op(&conn, req)).await,
        "list_indexes" => heap_future(list_index_op(&conn)).await,
        "reindex_fts" => heap_future(reindex_fts_op(state, &conn)).await,
        "drop_fts_index" => heap_future(drop_fts_index_op(&conn)).await,
        "enable_fts_index" => heap_future(enable_fts_index_op(state, &conn, req)).await,
        "delete" => heap_future(delete(state, db_path, &conn, req)).await,
        "drop_namespace" => heap_future(drop_collection(state, db_path, &conn, req)).await,
        "purge_archive" => heap_future(purge_kdb_archive(&conn, req)).await,
        "restore_archive" => heap_future(restore_kdb_archive(&conn, req)).await,
        "set_ttl" => heap_future(set_ttl(&conn, req)).await,
        "schedule_transition" => heap_future(schedule_transition(state, db_path, &conn, req)).await,
        "cancel_transition" => heap_future(cancel_transition(state, db_path, &conn, req)).await,
        "get_transition" => heap_future(get_transition(&conn, req)).await,
        "list_transitions" => heap_future(list_transitions(&conn, req)).await,
        "retry_transition" => heap_future(retry_transition(state, db_path, &conn, req)).await,
        "__run_document_transitions" => {
            heap_future(process_due_document_transitions(state, db_path, &conn)).await
        }
        "count" => heap_future(count(state, db_path, &conn, req)).await,
        "sql_execute" => heap_future(sql_execute(state, db_path, &conn, req)).await,
        "aggregate" => heap_future(aggregate(state, db_path, &conn, req)).await,
        "query" => query(state, db_path, &conn, req).await,
        "multi_query" => multi_query(state, db_path, &conn, req).await,
        "export_jsonl" => heap_future(export_jsonl(state, db_path, &conn, req)).await,
        "import_jsonl" => heap_future(import_jsonl(state, db_path, &conn, req)).await,
        "metrics_ingest" => heap_future(metrics_ingest(state, db_path, &conn, req)).await,
        "metrics_query" => heap_future(metrics_query(state, db_path, &conn, req)).await,
        "metrics_catalog" => heap_future(metrics_catalog(&conn, req)).await,
        "audit_ingest" => heap_future(audit_ingest(state, db_path, &conn, req)).await,
        "audit_query" => heap_future(audit_query(state, &conn, req)).await,
        "user_create" => heap_future(user_create(state, &conn, req)).await,
        "user_get" => heap_future(user_get(&conn, req)).await,
        "user_query" => heap_future(user_query(&conn, req)).await,
        "user_get_details" => heap_future(user_get_details(&conn, req)).await,
        "user_update" => heap_future(user_update(state, &conn, req)).await,
        "user_update_status" => heap_future(user_update_status(&conn, req)).await,
        "user_delete" => heap_future(user_delete(&conn, req)).await,
        "user_create_token" => heap_future(user_create_token(state, &conn, req)).await,
        "user_link_provider" => heap_future(user_link_provider(state, &conn, req)).await,
        "user_unlink_provider" => heap_future(user_unlink_provider(&conn, req)).await,
        "file_create" => heap_future(file_create(state, &conn, req)).await,
        "file_get" => heap_future(file_get(&conn, req)).await,
        "file_query" => heap_future(file_query(&conn, req)).await,
        "file_update" => heap_future(file_update(state, &conn, req)).await,
        "file_delete" => heap_future(file_delete(&conn, req)).await,
        "get_job" => heap_future(get_job(&conn, req)).await,
        "list_jobs" => heap_future(list_jobs(&conn, req)).await,
        "continue_job" => heap_future(continue_job(&conn, req)).await,
        "abort_job" => heap_future(abort_job(&conn, req)).await,
        "transaction" => heap_future(transaction(state, db_path, &conn, req)).await,
        other => Err(AppError::BadRequest(format!("unknown operation: {other}"))),
    };

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
include!("dispatcher/lifecycle_ops.rs");

include!("dispatcher/jobs_ops.rs");
include!("dispatcher/tx_mutation.rs");
include!("dispatcher/query_support.rs");
include!("dispatcher/lookup_support.rs");
include!("dispatcher/runtime_support.rs");
