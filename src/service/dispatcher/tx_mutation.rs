// Transaction, TTL, mutation engine, aliasing, and shared write helpers extracted from dispatcher.rs.

async fn set_ttl(conn: &libsql::Connection, req: GatewayRequest) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let dry_run = payload.dry_run.unwrap_or(false);
    let max_docs = payload.max_docs;
    let expiry_behavior = payload
        .expiry_behavior
        .as_deref()
        .map(|v| normalized_expiry_behavior(Some(v)));
    let ttl_seconds = payload
        .ttl_seconds
        .ok_or_else(|| AppError::BadRequest("ttl_seconds is required".to_string()))?;

    let expires_at = if ttl_seconds == 0 {
        None
    } else if ttl_seconds > 0 {
        Some(unix_now_secs() + ttl_seconds)
    } else {
        return Err(AppError::BadRequest(
            "ttl_seconds must be 0 or greater".to_string(),
        ));
    };

    validate_ids_or_filter_target(&payload, "set_ttl")?;
    let collection = if payload.ids.is_some() {
        resolve_collection_scope_optional_collection(&payload)?
    } else {
        resolve_collection_scope(&payload)?
    };
    let ids = target_ids_from_payload_with_collection(
        conn,
        &payload,
        collection.as_deref(),
        max_docs,
        "set_ttl",
    )
    .await?;

    if ids.is_empty() {
        return Ok(GatewayResponse::ok(Some(json!({
            "count": 0,
            "matched_count": 0,
            "updated_count": 0,
            "ttl_seconds": ttl_seconds
        }))));
    }

    if dry_run {
        let matched = count_by_ids(conn, collection.as_deref(), &ids).await?;
        return Ok(GatewayResponse::ok(Some(json!({
            "count": matched,
            "matched_count": matched,
            "updated_count": matched,
            "ttl_seconds": ttl_seconds,
            "dry_run": true
        }))));
    }

    let placeholders = vec!["?"; ids.len()].join(", ");
    let mut binds = vec![to_sql_nullable_int(expires_at)];
    let mut set_sql = "_expires_at = ?".to_string();
    if let Some(ref behavior) = expiry_behavior {
        set_sql.push_str(", _expiry_behavior = ?");
        binds.push(libsql::Value::Text(behavior.clone()));
    }
    let where_clause = where_ids_with_scope(&mut binds, collection.as_deref(), &ids, &placeholders);
    let updated = conn
        .execute(
            &format!("UPDATE __kdb_documents SET {set_sql} WHERE {where_clause}"),
            binds,
        )
        .await
        .map_err(|e| AppError::Internal(format!("set_ttl failed: {e}")))?;

    Ok(GatewayResponse::ok(Some(json!({
        "count": updated,
        "matched_count": updated,
        "updated_count": updated,
        "ttl_seconds": ttl_seconds
    }))))
}

async fn transaction(
    state: &AppState,
    db_path: &str,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let (on_error, ops) = prepare_transaction_operations(req.payload)?;
    let count = ops.len();

    let tx = conn
        .transaction()
        .await
        .map_err(|e| AppError::Internal(format!("tx begin failed: {e}")))?;

    let mut succeeded = 0_usize;
    let mut failed = 0_usize;
    let mut skipped = 0_usize;
    let mut results = Vec::<Value>::with_capacity(count);
    for op in ops {
        let savepoint = format!("__kdb_tx_op_{}", op.index);
        if on_error == BatchOnError::Continue {
            tx.execute(&format!("SAVEPOINT {savepoint}"), ())
                .await
                .map_err(|e| AppError::Internal(format!("tx savepoint failed: {e}")))?;
        }

        match execute_transaction_operation(state, &tx, &op).await {
            Ok(TransactionOperationOutcome::Applied(data)) => {
                if on_error == BatchOnError::Continue {
                    release_transaction_savepoint(&tx, &savepoint).await?;
                }
                succeeded += 1;
                results.push(transaction_operation_result(
                    &op,
                    "success",
                    data,
                    None,
                ));
            }
            Ok(TransactionOperationOutcome::Skipped) => {
                if on_error == BatchOnError::Continue {
                    release_transaction_savepoint(&tx, &savepoint).await?;
                }
                skipped += 1;
                results.push(transaction_operation_result(
                    &op,
                    "skipped",
                    None,
                    Some(json!({
                        "code": "unique_fields_conflict",
                        "message": "insert skipped by on_conflict policy"
                    })),
                ));
            }
            Err(error) if on_error == BatchOnError::Fail => return Err(error),
            Err(error) => {
                rollback_transaction_savepoint(&tx, &savepoint).await?;
                failed += 1;
                results.push(transaction_operation_result(
                    &op,
                    "error",
                    None,
                    Some(batch_error_value(error)),
                ));
            }
        }
    }

    tx.commit()
        .await
        .map_err(|e| AppError::Internal(format!("tx commit failed: {e}")))?;

    state
        .db_manager
        .append_wal_record(
            db_path,
            "TRANSACTION",
            &json!({
                "count": count,
                "succeeded": succeeded,
                "failed": failed,
                "skipped_count": skipped
            })
            .to_string(),
        )
        .await?;

    let mut response = GatewayResponse::ok(Some(json!({
        "count": count,
        "succeeded": succeeded,
        "failed": failed,
        "skipped_count": skipped,
        "results": results,
        "message": "transaction_committed"
    })));
    if failed > 0 {
        response.status = "partial";
    }
    Ok(response)
}

#[derive(Debug)]
struct PreparedTransactionOperation {
    index: usize,
    operation: String,
    alias: Option<String>,
    namespace: Option<String>,
    payload: OperationPayload,
}

#[derive(Debug)]
enum TransactionOperationOutcome {
    Applied(Option<Value>),
    Skipped,
}

fn prepare_transaction_operations(
    mut payload: OperationPayload,
) -> AppResult<(BatchOnError, Vec<PreparedTransactionOperation>)> {
    let on_error = parse_batch_on_error(payload.on_error.as_deref(), "transaction")?;
    let operations = payload.operations.take().ok_or_else(|| {
        AppError::BadRequest("transaction requires payload.operations[]".to_string())
    })?;
    if operations.is_empty() {
        return Err(AppError::BadRequest(
            "transaction payload.operations[] cannot be empty".to_string(),
        ));
    }

    operations
        .into_iter()
        .enumerate()
        .map(|(index, operation)| prepare_transaction_operation(operation, index))
        .collect::<AppResult<Vec<_>>>()
        .map(|operations| (on_error, operations))
}

fn prepare_transaction_operation(
    operation: BatchOperation,
    index: usize,
) -> AppResult<PreparedTransactionOperation> {
    let BatchOperation {
        operation,
        alias,
        mut namespace,
        mut payload,
    } = operation;
    let mut operation = operation
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if operation.is_empty() {
        return Err(AppError::BadRequest(format!(
            "transaction operations[{index}].operation is required"
        )));
    }

    if let Some((raw_operation, raw_scope)) = operation.clone().split_once("::") {
        if namespace.is_some() {
            return Err(AppError::BadRequest(format!(
                "transaction operations[{index}] operation shorthand cannot be combined with namespace"
            )));
        }
        operation = raw_operation.trim().to_string();
        let scope = raw_scope.trim();
        if operation.is_empty() || scope.is_empty() {
            return Err(AppError::BadRequest(format!(
                "transaction operations[{index}] has invalid operation shorthand"
            )));
        }
        if scope == "*" {
            namespace = Some(NamespaceSelector::One("*".to_string()));
        } else if scope.contains(',') {
            namespace = Some(NamespaceSelector::Many(
                scope
                    .split(',')
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .collect(),
            ));
        } else {
            namespace = Some(NamespaceSelector::One(scope.to_string()));
        }
    }

    if !matches!(
        operation.as_str(),
        "insert" | "update" | "upsert" | "delete"
    ) {
        return Err(AppError::BadRequest(format!(
            "transaction operations[{index}] supports only insert/update/upsert/delete"
        )));
    }
    normalize_transaction_operation_scope(&mut payload, namespace, index)?;
    let namespace = payload.collection.clone();
    let alias = alias
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    Ok(PreparedTransactionOperation {
        index,
        operation,
        alias,
        namespace,
        payload,
    })
}

fn normalize_transaction_operation_scope(
    payload: &mut OperationPayload,
    namespace: Option<NamespaceSelector>,
    index: usize,
) -> AppResult<()> {
    let Some(namespace) = namespace else { return Ok(()); };
    if let NamespaceSelector::Many(raw_namespaces) = namespace {
        if payload.collection.is_some() {
            return Err(AppError::BadRequest(format!(
                "transaction operations[{index}] has conflicting namespace selectors"
            )));
        }
        let mut normalized = raw_namespaces
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        normalized.sort();
        normalized.dedup();
        if normalized.is_empty() {
            return Err(AppError::BadRequest(format!(
                "transaction operations[{index}].namespace array cannot be empty"
            )));
        }
        if normalized.iter().any(|value| value == "*") && normalized.len() > 1 {
            return Err(AppError::BadRequest(format!(
                "transaction operations[{index}].namespace array cannot combine '*' with other values"
            )));
        }
        if normalized.len() == 1 && normalized[0] == "*" {
            payload.scope = Some("all".to_string());
        }
        payload.namespaces = Some(normalized);
        return Ok(());
    }

    let NamespaceSelector::One(namespace) = namespace else {
        unreachable!("namespace array handled above")
    };
    let namespace = namespace.trim().to_string();
    if namespace.is_empty() {
        return Err(AppError::BadRequest(format!(
            "transaction operations[{index}].namespace cannot be empty"
        )));
    }
    if payload
        .namespaces
        .as_ref()
        .map(|values| !values.is_empty())
        .unwrap_or(false)
    {
        return Err(AppError::BadRequest(format!(
            "transaction operations[{index}] namespace conflicts with payload.namespaces"
        )));
    }
    if namespace == "*" {
        payload.collection = None;
        payload.namespaces = Some(vec!["*".to_string()]);
        payload.scope = Some("all".to_string());
    } else if let Some(existing) = payload.collection.as_deref() {
        if existing != namespace {
            return Err(AppError::BadRequest(format!(
                "transaction operations[{index}] has conflicting namespace selectors"
            )));
        }
    } else {
        payload.collection = Some(namespace);
    }
    Ok(())
}

async fn execute_transaction_operation(
    state: &AppState,
    tx: &libsql::Transaction,
    operation: &PreparedTransactionOperation,
) -> AppResult<TransactionOperationOutcome> {
    match operation.operation.as_str() {
        "insert" => Ok(if tx_insert(tx, &operation.payload).await? {
            TransactionOperationOutcome::Applied(None)
        } else {
            TransactionOperationOutcome::Skipped
        }),
        "update" => {
            tx_update(tx, &operation.payload).await?;
            Ok(TransactionOperationOutcome::Applied(None))
        }
        "upsert" => {
            let data = tx_upsert(state, tx, &operation.payload).await?;
            Ok(TransactionOperationOutcome::Applied(Some(data)))
        }
        "delete" => {
            tx_delete(tx, &operation.payload).await?;
            Ok(TransactionOperationOutcome::Applied(None))
        }
        _ => unreachable!("transaction operation validated before execution"),
    }
}

async fn release_transaction_savepoint(
    tx: &libsql::Transaction,
    savepoint: &str,
) -> AppResult<()> {
    tx.execute(&format!("RELEASE SAVEPOINT {savepoint}"), ())
        .await
        .map_err(|e| AppError::Internal(format!("tx savepoint release failed: {e}")))?;
    Ok(())
}

async fn rollback_transaction_savepoint(
    tx: &libsql::Transaction,
    savepoint: &str,
) -> AppResult<()> {
    tx.execute(&format!("ROLLBACK TO SAVEPOINT {savepoint}"), ())
        .await
        .map_err(|e| AppError::Internal(format!("tx savepoint rollback failed: {e}")))?;
    release_transaction_savepoint(tx, savepoint).await
}

fn transaction_operation_result(
    operation: &PreparedTransactionOperation,
    status: &str,
    data: Option<Value>,
    detail: Option<Value>,
) -> Value {
    let mut result = json!({
        "index": operation.index,
        "operation": operation.operation,
        "status": status
    });
    let object = result.as_object_mut().expect("transaction result is object");
    if let Some(alias) = operation.alias.as_ref() {
        object.insert("alias".to_string(), Value::String(alias.clone()));
    }
    if let Some(namespace) = operation.namespace.as_ref() {
        object.insert("namespace".to_string(), Value::String(namespace.clone()));
    }
    if let Some(data) = data {
        object.insert("data".to_string(), data);
    }
    if let Some(detail) = detail {
        let key = if status == "error" { "error" } else { "reason" };
        object.insert(key.to_string(), detail);
    }
    result
}

async fn tx_insert(tx: &libsql::Transaction, payload: &OperationPayload) -> AppResult<bool> {
    let collection = require_collection(payload)?;
    let lifecycle_specs = parse_document_lifecycle(payload.lifecycle.clone())?;
    let mut doc = require_object(payload.data.clone(), "data")?;
    expand_kdb_macros_in_value(&mut doc)?;
    let allow_system_timestamps = payload.allow_system_timestamps.unwrap_or(false);
    let id = ensure_or_get_id(&mut doc)?;
    let unique_fields = normalize_unique_fields(payload.unique_fields.clone())?;
    let on_conflict = parse_insert_on_conflict(Some(
        payload.on_conflict.as_deref().unwrap_or("error"),
    ))?;
    if !unique_fields.is_empty() {
        let pairs = resolve_unique_pairs(&doc, &unique_fields)?;
        if !pairs.is_empty() && exists_by_unique_pairs_on_tx(tx, &collection, &pairs).await? {
            if on_conflict == "error" {
                return Err(AppError::Conflict(
                    "insert unique_fields conflict".to_string(),
                ));
            }
            return Ok(false);
        }
    }
    let (created_at, modified_at) = resolve_insert_timestamps(&doc, allow_system_timestamps)?;
    let data = doc.to_string();
    let size = data.len() as i64;
    let expires_at = ttl_to_expires_at(payload.ttl_seconds)?;
    let expiry_behavior = normalized_expiry_behavior(payload.expiry_behavior.as_deref());

    let data_expr = json_input_expr(jsonb_enabled());
    tx.execute(
        &format!(
            "INSERT INTO __kdb_documents (id, collection, data, _size_bytes, _expires_at, _expiry_behavior, _created_at, _modified_at)
             VALUES (?, ?, {data_expr}, ?, ?, ?, COALESCE(?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), COALESCE(?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')))"
        ),
        libsql::params![
            id.clone(),
            collection.clone(),
            data,
            size,
            expires_at,
            expiry_behavior,
            to_sql_nullable_text(created_at),
            to_sql_nullable_text(modified_at)
        ],
    )
    .await
    .map_err(|e| AppError::Conflict(format!("transaction insert failed: {e}")))?;
    for spec in &lifecycle_specs {
        upsert_transition_on_tx(tx, &id, &collection, spec).await?;
    }
    Ok(true)
}

async fn tx_update(tx: &libsql::Transaction, payload: &OperationPayload) -> AppResult<()> {
    let collection = require_collection(payload)?;
    let lifecycle_specs = parse_document_lifecycle(payload.lifecycle.clone())?;
    let mut data = require_object(payload.data.clone(), "data")?;
    expand_kdb_macros_in_value(&mut data)?;
    let id = data
        .get("_id")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::BadRequest("data._id is required".to_string()))?
        .to_string();
    data.as_object_mut().expect("object checked").remove("_id");
    if update_requires_mutation_engine(data.as_object().expect("object checked")) {
        let mut rows = tx
            .query(
                "SELECT rowid, json(data)
                 FROM __kdb_documents
                 WHERE collection = ? AND id = ?
                 LIMIT 1",
                libsql::params![collection.clone(), id.clone()],
            )
            .await
            .map_err(|e| AppError::Internal(format!("transaction update read failed: {e}")))?;
        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("transaction update row failed: {e}")))?
        {
            let rowid: i64 = row.get(0).map_err(|e| {
                AppError::Internal(format!("transaction update rowid decode failed: {e}"))
            })?;
            let raw: String = row.get(1).map_err(|e| {
                AppError::Internal(format!("transaction update data decode failed: {e}"))
            })?;
            let mut doc = serde_json::from_str::<Value>(&raw).map_err(|e| {
                AppError::Internal(format!("transaction update json decode failed: {e}"))
            })?;
            let strict = strict_mutation_operators_env();
            let mut patch_obj = data
                .as_object()
                .cloned()
                .ok_or_else(|| AppError::BadRequest("data must be object".to_string()))?;
            apply_mutation_patch_to_doc(&mut doc, &mut patch_obj, strict)?;
            let data_expr = json_input_expr(jsonb_enabled());
            let data_str = doc.to_string();
            tx.execute(
                &format!(
                    "UPDATE __kdb_documents
                     SET data = {data_expr},
                         _size_bytes = length(?),
                         _modified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     WHERE rowid = ?"
                ),
                libsql::params![data_str.clone(), data_str.len() as i64, rowid],
            )
            .await
            .map_err(|e| AppError::Internal(format!("transaction mutation update failed: {e}")))?;
        }
    } else {
        let patch = data.to_string();
        let patch_expr = json_patch_expr(jsonb_enabled());
        tx.execute(
            &format!(
                "UPDATE __kdb_documents
                 SET data = {patch_expr},
                     _size_bytes = length({patch_expr}),
                     _modified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE collection = ? AND id = ?"
            ),
            libsql::params![patch.clone(), patch, collection.clone(), id.clone()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("transaction update failed: {e}")))?;
    }
    if !lifecycle_specs.is_empty() {
        let mut rows = tx
            .query(
                "SELECT 1 FROM __kdb_documents WHERE collection = ? AND id = ? LIMIT 1",
                libsql::params![collection.clone(), id.clone()],
            )
            .await
            .map_err(|e| AppError::Internal(format!("transaction lifecycle target failed: {e}")))?;
        let exists = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("transaction lifecycle row failed: {e}")))?
            .is_some();
        drop(rows);
        if exists {
            for spec in &lifecycle_specs {
                upsert_transition_on_tx(tx, &id, &collection, spec).await?;
            }
        }
    }
    Ok(())
}

async fn tx_upsert(
    state: &AppState,
    tx: &libsql::Transaction,
    payload: &OperationPayload,
) -> AppResult<Value> {
    let lifecycle_specs = parse_document_lifecycle(payload.lifecycle.clone())?;
    if !lifecycle_specs.is_empty() && payload.max_docs.unwrap_or(1) != 1 {
        return Err(AppError::BadRequest(
            "upsert with lifecycle requires max_docs=1".to_string(),
        ));
    }

    let mut inner_payload = payload.clone();
    inner_payload.lifecycle = None;
    let request = GatewayRequest {
        db: None,
        operation: "upsert".to_string(),
        namespace: None,
        payload: inner_payload,
    };
    let mut response = Box::pin(upsert_inner(state, None, tx, request)).await?;

    if !lifecycle_specs.is_empty() {
        if payload.dry_run.unwrap_or(false) {
            add_lifecycle_dry_run_response(&mut response, &lifecycle_specs);
        } else {
            let document_id = response
                .data
                .as_ref()
                .and_then(|data| data.get("items"))
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("_id"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AppError::Internal(
                        "transaction upsert lifecycle result document id missing".to_string(),
                    )
                })?
                .to_string();
            let collection = resolve_transition_document(tx, &document_id, None).await?;
            for spec in &lifecycle_specs {
                upsert_transition_on_tx(tx, &document_id, &collection, spec).await?;
            }
            add_lifecycle_response(&mut response, &lifecycle_specs);
        }
    }

    let mut data = response
        .data
        .take()
        .ok_or_else(|| AppError::Internal("transaction upsert result data missing".to_string()))?;
    let inserted = data
        .get("inserted_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let updated = data
        .get("updated_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let action = if payload.dry_run.unwrap_or(false) || (inserted == 0 && updated == 0) {
        "no_change"
    } else if inserted > 0 {
        "inserted"
    } else {
        "updated"
    };
    data.as_object_mut()
        .ok_or_else(|| AppError::Internal("transaction upsert result must be object".to_string()))?
        .insert("action".to_string(), Value::String(action.to_string()));
    Ok(data)
}

async fn tx_delete(tx: &libsql::Transaction, payload: &OperationPayload) -> AppResult<()> {
    let collection = resolve_collection_scope(payload)?;
    let id = extract_single_id(payload)?;
    let txn_id = Uuid::new_v4().simple().to_string();
    let mut insert_binds = vec![libsql::Value::Text(txn_id), libsql::Value::Null];
    let mut delete_binds = Vec::<libsql::Value>::new();
    let mut transition_binds = Vec::<libsql::Value>::new();
    let where_clause = if let Some(collection) = collection {
        insert_binds.push(libsql::Value::Text(collection.clone()));
        insert_binds.push(libsql::Value::Text(id.clone()));
        delete_binds.push(libsql::Value::Text(collection.clone()));
        delete_binds.push(libsql::Value::Text(id.clone()));
        transition_binds.push(libsql::Value::Text(collection.clone()));
        transition_binds.push(libsql::Value::Text(id.clone()));
        "collection = ? AND id = ?".to_string()
    } else {
        insert_binds.push(libsql::Value::Text(id.clone()));
        delete_binds.push(libsql::Value::Text(id.clone()));
        transition_binds.push(libsql::Value::Text(id.clone()));
        "id = ?".to_string()
    };

    tx.execute(
        &format!(
            "INSERT INTO __kdb_archive (id, collection, _user_id, data, _size_bytes, _created_at, _modified_at, _txn_id, _expires_at)
             SELECT id, collection, _user_id, data, _size_bytes, _created_at, _modified_at, ?, ?
             FROM __kdb_documents WHERE {}",
            where_clause
        ),
        insert_binds,
    )
    .await
    .map_err(|e| AppError::Internal(format!("transaction delete __kdb_archive failed: {e}")))?;

    tx.execute(
        &format!("UPDATE __kdb_document_transitions
         SET status='cancelled', completed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),
             updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE document_id IN (
             SELECT id FROM __kdb_documents WHERE {where_clause}
         ) AND status='pending'"),
        transition_binds,
    )
    .await
    .map_err(|e| AppError::Internal(format!("transaction transition cancel failed: {e}")))?;

    tx.execute(
        &format!("DELETE FROM __kdb_documents WHERE {}", where_clause),
        delete_binds,
    )
    .await
    .map_err(|e| AppError::Internal(format!("transaction delete failed: {e}")))?;

    Ok(())
}

#[derive(Debug)]
struct DeleteResult {
    txn_id: String,
    deleted_count: usize,
}

async fn __kdb_archive_and_delete_ids(
    conn: &libsql::Connection,
    collection: Option<&str>,
    ids: &[String],
    __kdb_archive_ttl_secs: Option<i64>,
) -> AppResult<DeleteResult> {
    if ids.is_empty() {
        return Ok(DeleteResult {
            txn_id: Uuid::new_v4().simple().to_string(),
            deleted_count: 0,
        });
    }

    let txn_id = Uuid::new_v4().simple().to_string();
    let placeholders = vec!["?"; ids.len()].join(", ");
    let __kdb_archive_expires_at = __kdb_archive_ttl_secs.map(|ttl| unix_now_secs() + ttl);

    let mut insert_binds: Vec<libsql::Value> = vec![
        libsql::Value::Text(txn_id.clone()),
        to_sql_nullable_int(__kdb_archive_expires_at),
    ];
    let mut delete_binds: Vec<libsql::Value> = Vec::new();
    let mut transition_binds: Vec<libsql::Value> = Vec::new();

    let where_clause = if let Some(collection) = collection {
        insert_binds.push(libsql::Value::Text(collection.to_string()));
        delete_binds.push(libsql::Value::Text(collection.to_string()));
        transition_binds.push(libsql::Value::Text(collection.to_string()));
        format!("collection = ? AND id IN ({})", placeholders)
    } else {
        format!("id IN ({})", placeholders)
    };

    insert_binds.extend(ids.iter().map(|id| libsql::Value::Text(id.clone())));
    delete_binds.extend(ids.iter().map(|id| libsql::Value::Text(id.clone())));
    transition_binds.extend(ids.iter().map(|id| libsql::Value::Text(id.clone())));

    let tx = conn
        .transaction()
        .await
        .map_err(|e| AppError::Internal(format!("delete tx begin failed: {e}")))?;

    tx.execute(
        &format!(
            "INSERT INTO __kdb_archive (id, collection, _user_id, data, _size_bytes, _created_at, _modified_at, _txn_id, _expires_at)
             SELECT id, collection, _user_id, data, _size_bytes, _created_at, _modified_at, ?, ?
             FROM __kdb_documents
             WHERE {}",
            where_clause
        ),
        insert_binds,
    )
    .await
    .map_err(|e| AppError::Internal(format!("__kdb_archive insert failed: {e}")))?;

    tx.execute(
        &format!(
            "UPDATE __kdb_document_transitions
             SET status='cancelled', completed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),
                 updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE document_id IN (
                 SELECT id FROM __kdb_documents WHERE {where_clause}
             ) AND status='pending'"
        ),
        transition_binds,
    )
    .await
    .map_err(|e| AppError::Internal(format!("delete transition cancellation failed: {e}")))?;

    let deleted = tx
        .execute(
            &format!("DELETE FROM __kdb_documents WHERE {}", where_clause),
            delete_binds,
        )
        .await
        .map_err(|e| AppError::Internal(format!("delete failed: {e}")))?;

    tx.commit()
        .await
        .map_err(|e| AppError::Internal(format!("delete tx commit failed: {e}")))?;

    Ok(DeleteResult {
        txn_id,
        deleted_count: deleted as usize,
    })
}

async fn update_bulk_filter_same_patch(
    state: &AppState,
    conn: &libsql::Connection,
    collection: Option<&str>,
    filter: Option<Value>,
    data: Option<Value>,
    max_docs: Option<i64>,
    dry_run: bool,
) -> AppResult<GatewayResponse> {
    let filter = require_non_empty_filter(filter)?;
    let patch = require_object(data, "data")?;
    let strict = state.strict_mutation_operators;
    let use_mutation = update_requires_mutation_engine(patch.as_object().expect("object checked"));
    let patch_obj = patch.as_object().expect("object checked");
    if patch_obj.is_empty() {
        return Err(AppError::BadRequest("data cannot be empty".to_string()));
    }

    let (where_clause, binds) =
        build_where_with_collection(filter, collection.map(ToOwned::to_owned))?;
    let matched_count = execute_count(conn, "__kdb_documents", &where_clause, binds.clone()).await?;

    let max_docs = normalize_max_docs(max_docs, matched_count)?;

    if dry_run {
        return Ok(GatewayResponse::ok(Some(json!({
            "items": [],
            "count": max_docs,
            "matched_count": matched_count,
            "updated_count": max_docs,
            "dry_run": true
        }))));
    }

    if max_docs == 0 {
        return Ok(GatewayResponse::ok(Some(json!({
            "items": [],
            "count": 0,
            "matched_count": matched_count,
            "updated_count": 0
        }))));
    }

    let items = if use_mutation {
        let rows =
            select_rowids_for_update(conn, &where_clause, binds, max_docs as i64).await?;
        let mut out = Vec::<Value>::new();
        for rowid in rows {
            let mut patch_obj = patch
                .as_object()
                .cloned()
                .ok_or_else(|| AppError::BadRequest("data must be object".to_string()))?;
            if let Some(v) = update_one_by_rowid_with_mutation(
                conn,
                rowid,
                &mut patch_obj,
                strict,
                state.jsonb_enabled,
            )
            .await?
            {
                out.push(v);
            }
        }
        out
    } else {
        let patch_str = patch.to_string();
        let mut qbinds = vec![
            libsql::Value::Text(patch_str.clone()),
            libsql::Value::Text(patch_str),
        ];
        qbinds.extend(binds);
        qbinds.push(libsql::Value::Integer(max_docs as i64));

        let mut rows = conn
            .query(
                &format!(
                    "UPDATE __kdb_documents
                     SET data = json_patch(data, ?),
                         _size_bytes = length(json_patch(data, ?)),
                         _modified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     WHERE rowid IN (
                        SELECT rowid FROM __kdb_documents WHERE {where_clause} ORDER BY rowid LIMIT ?
                     )
                     RETURNING json(data)"
                ),
                qbinds,
            )
            .await
            .map_err(|e| AppError::Internal(format!("update_bulk filter failed: {e}")))?;

        let mut out = Vec::<Value>::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("update_bulk filter row read failed: {e}")))?
        {
            let raw: String = row.get(0).map_err(|e| {
                AppError::Internal(format!("update_bulk filter row decode failed: {e}"))
            })?;
            out.push(serde_json::from_str::<Value>(&raw).map_err(|e| {
                AppError::Internal(format!("update_bulk filter json decode failed: {e}"))
            })?);
        }
        out
    };

    Ok(GatewayResponse::ok(Some(json!({
        "items": items,
        "count": items.len(),
        "matched_count": matched_count,
        "updated_count": items.len()
    }))))
}

async fn update_bulk_data_array(
    state: &AppState,
    conn: &libsql::Connection,
    collection: Option<&str>,
    data: Option<Value>,
    dry_run: bool,
) -> AppResult<GatewayResponse> {
    let mut arr = data
        .ok_or_else(|| AppError::BadRequest("data is required".to_string()))?
        .as_array()
        .cloned()
        .ok_or_else(|| AppError::BadRequest("data must be an array".to_string()))?;

    if arr.is_empty() {
        return Err(AppError::BadRequest("data cannot be empty".to_string()));
    }

    for item in &mut arr {
        expand_kdb_macros_in_value(item)?;
        let obj = item
            .as_object()
            .ok_or_else(|| AppError::BadRequest("all data items must be objects".to_string()))?;
        if obj.get("_id").and_then(Value::as_str).is_none() {
            return Err(AppError::BadRequest(
                "all data items must include _id".to_string(),
            ));
        }
    }

    if dry_run {
        return Ok(GatewayResponse::ok(Some(json!({
            "items": [],
            "count": arr.len(),
            "matched_count": arr.len(),
            "updated_count": arr.len(),
            "dry_run": true
        }))));
    }

    let mut items = Vec::<Value>::new();
    let strict = state.strict_mutation_operators;
    for item in arr {
        let mut patch = item
            .as_object()
            .cloned()
            .ok_or_else(|| AppError::BadRequest("all data items must be objects".to_string()))?;
        let id = patch
            .remove("_id")
            .and_then(|v| v.as_str().map(ToOwned::to_owned))
            .ok_or_else(|| AppError::BadRequest("all data items must include _id".to_string()))?;

        if patch.is_empty() {
            continue;
        }

        let patch_value = Value::Object(patch);
        if update_requires_mutation_engine(
            patch_value
                .as_object()
                .ok_or_else(|| AppError::BadRequest("data must be object".to_string()))?,
        ) {
            let mut patch_obj = patch_value
                .as_object()
                .cloned()
                .ok_or_else(|| AppError::BadRequest("data must be object".to_string()))?;
            if let Some(v) = update_one_with_mutation(
                conn,
                collection,
                &id,
                &mut patch_obj,
                strict,
                state.jsonb_enabled,
            )
            .await?
            {
                items.push(v);
            }
        } else {
            let patch_str = patch_value.to_string();
            let mut binds = vec![
                libsql::Value::Text(patch_str.clone()),
                libsql::Value::Text(patch_str),
            ];
            let where_clause = where_id_with_scope(&mut binds, collection, &id);
            let mut rows = conn
                .query(
                    &format!(
                        "UPDATE __kdb_documents
                         SET data = json_patch(data, ?),
                             _size_bytes = length(json_patch(data, ?)),
                             _modified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                         WHERE {where_clause}
                         RETURNING json(data)"
                    ),
                    binds,
                )
                .await
                .map_err(|e| AppError::Internal(format!("update_bulk data[] failed: {e}")))?;

            while let Some(row) = rows
                .next()
                .await
                .map_err(|e| AppError::Internal(format!("update_bulk data[] row read failed: {e}")))?
            {
                let raw: String = row.get(0).map_err(|e| {
                    AppError::Internal(format!("update_bulk data[] row decode failed: {e}"))
                })?;
                items.push(serde_json::from_str::<Value>(&raw).map_err(|e| {
                    AppError::Internal(format!("update_bulk data[] json decode failed: {e}"))
                })?);
            }
        }
    }

    Ok(GatewayResponse::ok(Some(json!({
        "items": items,
        "count": items.len(),
        "matched_count": items.len(),
        "updated_count": items.len()
    }))))
}

fn require_collection(payload: &OperationPayload) -> AppResult<String> {
    if payload
        .namespaces
        .as_ref()
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        return Err(AppError::BadRequest(
            "namespace must be a single string for this operation (an array is not allowed)"
                .to_string(),
        ));
    }
    let collection = payload
        .collection
        .clone()
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| AppError::BadRequest("namespace is required".to_string()))?;
    if collection == "*" {
        return Err(AppError::BadRequest(
            "namespace='*' is not allowed for this operation".to_string(),
        ));
    }
    Ok(collection)
}

fn resolve_collection_scope(payload: &OperationPayload) -> AppResult<Option<String>> {
    let scope = payload.scope.as_deref().unwrap_or("namespace");
    match scope {
        "all" => Ok(None),
        "namespace" => Ok(Some(require_collection(payload)?)),
        _ => Err(AppError::BadRequest(
            "scope must be either 'namespace' or 'all'".to_string(),
        )),
    }
}

fn resolve_collection_scope_optional_collection(
    payload: &OperationPayload,
) -> AppResult<Option<String>> {
    let scope = payload.scope.as_deref().unwrap_or("namespace");
    match scope {
        "all" => Ok(None),
        "namespace" => Ok(payload.collection.clone().filter(|c| !c.trim().is_empty())),
        _ => Err(AppError::BadRequest(
            "scope must be either 'namespace' or 'all'".to_string(),
        )),
    }
}

fn where_id_with_scope(
    binds: &mut Vec<libsql::Value>,
    collection: Option<&str>,
    id: &str,
) -> String {
    if let Some(collection) = collection {
        binds.push(libsql::Value::Text(collection.to_string()));
        binds.push(libsql::Value::Text(id.to_string()));
        "collection = ? AND id = ?".to_string()
    } else {
        binds.push(libsql::Value::Text(id.to_string()));
        "id = ?".to_string()
    }
}

fn where_ids_with_scope(
    binds: &mut Vec<libsql::Value>,
    collection: Option<&str>,
    ids: &[String],
    placeholders: &str,
) -> String {
    if let Some(collection) = collection {
        binds.push(libsql::Value::Text(collection.to_string()));
        binds.extend(ids.iter().map(|id| libsql::Value::Text(id.clone())));
        format!("collection = ? AND id IN ({})", placeholders)
    } else {
        binds.extend(ids.iter().map(|id| libsql::Value::Text(id.clone())));
        format!("id IN ({})", placeholders)
    }
}

fn replacement_doc_from_payload(doc: &mut Value, id: &str) -> AppResult<Value> {
    let obj = doc
        .as_object_mut()
        .ok_or_else(|| AppError::BadRequest("data must be object".to_string()))?;
    obj.remove("_id");

    let mut out = serde_json::Map::new();
    out.insert("_id".to_string(), Value::String(id.to_string()));
    for (k, v) in obj.iter() {
        out.insert(k.clone(), v.clone());
    }
    Ok(Value::Object(out))
}

fn delete_archive_ttl_secs(state: &AppState, payload_ttl: Option<i64>) -> AppResult<Option<i64>> {
    let chosen = payload_ttl.or(state.delete_default_ttl_secs);
    if let Some(ttl) = chosen {
        if ttl <= 0 {
            return Err(AppError::BadRequest(
                "delete ttl_seconds must be greater than 0".to_string(),
            ));
        }
    }
    Ok(chosen)
}

fn to_sql_nullable_int(v: Option<i64>) -> libsql::Value {
    match v {
        Some(n) => libsql::Value::Integer(n),
        None => libsql::Value::Null,
    }
}

fn to_sql_nullable_text(v: Option<String>) -> libsql::Value {
    match v {
        Some(s) => libsql::Value::Text(s),
        None => libsql::Value::Null,
    }
}

fn parse_insert_on_conflict(v: Option<&str>) -> AppResult<String> {
    let policy = v.unwrap_or("skip").to_lowercase();
    if !matches!(policy.as_str(), "skip" | "error") {
        return Err(AppError::BadRequest(
            "on_conflict must be one of: skip, error".to_string(),
        ));
    }
    Ok(policy)
}

fn normalize_unique_fields(v: Option<Vec<String>>) -> AppResult<Vec<String>> {
    let Some(fields) = v else {
        return Ok(vec![]);
    };
    let mut out = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();
    for raw in fields {
        let path = raw.trim();
        if path.is_empty() {
            continue;
        }
        if !is_observable_path(path) {
            return Err(AppError::BadRequest(format!(
                "unique_fields contains invalid path: {path}"
            )));
        }
        if seen.insert(path.to_string()) {
            out.push(path.to_string());
        }
    }
    Ok(out)
}

fn resolve_unique_pairs(doc: &Value, fields: &[String]) -> AppResult<Vec<(String, Value)>> {
    let mut out = Vec::<(String, Value)>::new();
    for path in fields {
        let Some(v) = value_by_path(doc, path).cloned() else {
            continue;
        };
        match v {
            Value::Array(_) | Value::Object(_) => {
                return Err(AppError::BadRequest(format!(
                    "unique_fields path must resolve to scalar/null: {path}"
                )));
            }
            _ => out.push((path.clone(), v)),
        }
    }
    Ok(out)
}

fn unique_signature(pairs: &[(String, Value)]) -> AppResult<String> {
    serde_json::to_string(pairs)
        .map_err(|e| AppError::Internal(format!("unique signature encode failed: {e}")))
}

fn build_unique_check_query(
    collection: &str,
    pairs: &[(String, Value)],
) -> AppResult<(String, Vec<libsql::Value>)> {
    let mut binds = vec![libsql::Value::Text(collection.to_string())];
    let mut clauses = Vec::<String>::new();
    for (path, v) in pairs {
        let jp = sql_json_path(path)?;
        match v {
            Value::Null => clauses.push(format!("json_type(data, '{jp}') = 'null'")),
            Value::Bool(b) => {
                clauses.push(format!("CAST(json_extract(data, '{jp}') AS INTEGER) = ?"));
                binds.push(libsql::Value::Integer(if *b { 1 } else { 0 }));
            }
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    clauses.push(format!("CAST(json_extract(data, '{jp}') AS INTEGER) = ?"));
                    binds.push(libsql::Value::Integer(i));
                } else if let Some(f) = n.as_f64() {
                    clauses.push(format!("CAST(json_extract(data, '{jp}') AS REAL) = ?"));
                    binds.push(libsql::Value::Real(f));
                } else {
                    return Err(AppError::BadRequest(format!(
                        "unique_fields value unsupported for path: {path}"
                    )));
                }
            }
            Value::String(s) => {
                clauses.push(format!("json_extract(data, '{jp}') = ?"));
                binds.push(libsql::Value::Text(s.clone()));
            }
            Value::Array(_) | Value::Object(_) => unreachable!(),
        }
    }
    let where_clause = clauses.join(" AND ");
    Ok((
        format!(
            "SELECT 1 FROM __kdb_documents WHERE collection = ? AND {where_clause} LIMIT 1"
        ),
        binds,
    ))
}

async fn exists_by_unique_pairs(
    conn: &libsql::Connection,
    collection: &str,
    pairs: &[(String, Value)],
) -> AppResult<bool> {
    if pairs.is_empty() {
        return Ok(false);
    }
    let (sql, binds) = build_unique_check_query(collection, pairs)?;
    let mut rows = conn
        .query(&sql, binds)
        .await
        .map_err(|e| AppError::Internal(format!("unique check query failed: {e}")))?;
    Ok(rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("unique check row read failed: {e}")))?
        .is_some())
}

async fn exists_by_unique_pairs_on_tx(
    tx: &libsql::Transaction,
    collection: &str,
    pairs: &[(String, Value)],
) -> AppResult<bool> {
    if pairs.is_empty() {
        return Ok(false);
    }
    let (sql, binds) = build_unique_check_query(collection, pairs)?;
    let mut rows = tx
        .query(&sql, binds)
        .await
        .map_err(|e| AppError::Internal(format!("transaction unique check query failed: {e}")))?;
    Ok(rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("transaction unique check row failed: {e}")))?
        .is_some())
}

async fn count_bulk_unique_skips(
    conn: &libsql::Connection,
    collection: &str,
    docs: &[Value],
    unique_fields: &[String],
) -> AppResult<usize> {
    if unique_fields.is_empty() {
        return Ok(0);
    }
    let mut seen = HashSet::<String>::new();
    let mut skipped = 0usize;
    for doc in docs {
        let pairs = resolve_unique_pairs(doc, unique_fields)?;
        if pairs.is_empty() {
            continue;
        }
        let sig = unique_signature(&pairs)?;
        if seen.contains(&sig) || exists_by_unique_pairs(conn, collection, &pairs).await? {
            skipped += 1;
        } else {
            seen.insert(sig);
        }
    }
    Ok(skipped)
}

fn require_object(value: Option<Value>, name: &str) -> AppResult<Value> {
    let mut v = value.ok_or_else(|| AppError::BadRequest(format!("{name} is required")))?;
    expand_kdb_macros_in_value(&mut v)?;
    let obj = v
        .as_object()
        .ok_or_else(|| AppError::BadRequest(format!("{name} must be an object")))?;
    if obj.is_empty() {
        return Err(AppError::BadRequest(format!("{name} cannot be empty")));
    }
    Ok(v)
}

fn require_non_empty_filter(filter: Option<Value>) -> AppResult<Value> {
    let v = filter.ok_or_else(|| AppError::BadRequest("filter is required".to_string()))?;
    let obj = v
        .as_object()
        .ok_or_else(|| AppError::BadRequest("filter must be an object".to_string()))?;
    if obj.is_empty() {
        return Err(AppError::BadRequest("filter cannot be empty".to_string()));
    }
    Ok(v)
}

fn reject_id_field(v: &Value, name: &str) -> AppResult<()> {
    let obj = v
        .as_object()
        .ok_or_else(|| AppError::BadRequest(format!("{name} must be object")))?;
    if obj.contains_key("_id") {
        return Err(AppError::BadRequest(format!("{name} cannot contain _id")));
    }
    Ok(())
}

fn ensure_or_get_id(doc: &mut Value) -> AppResult<String> {
    let obj = doc
        .as_object_mut()
        .ok_or_else(|| AppError::BadRequest("data must be an object".to_string()))?;

    match obj.get("_id") {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.clone()),
        Some(_) => Err(AppError::BadRequest(
            "_id must be a non-empty string".to_string(),
        )),
        None => {
            let id = Uuid::new_v4().simple().to_string();
            obj.insert("_id".to_string(), json!(id.clone()));
            Ok(id)
        }
    }
}

fn resolve_insert_timestamps(
    doc: &Value,
    allow_system_timestamps: bool,
) -> AppResult<(Option<String>, Option<String>)> {
    let obj = doc
        .as_object()
        .ok_or_else(|| AppError::BadRequest("data must be an object".to_string()))?;
    let created_raw = obj.get("_created_at");
    let modified_raw = obj.get("_modified_at");
    let has_any = created_raw.is_some() || modified_raw.is_some();

    if has_any && !allow_system_timestamps {
        return Err(AppError::BadRequest(
            "_created_at/_modified_at are reserved; set payload.allow_system_timestamps=true to import them"
                .to_string(),
        ));
    }
    if !allow_system_timestamps {
        return Ok((None, None));
    }

    let created_at = parse_optional_utc_rfc3339(created_raw, "_created_at")?;
    let modified_at = parse_optional_utc_rfc3339(modified_raw, "_modified_at")?;

    if created_at.is_none() && modified_at.is_some() {
        return Err(AppError::BadRequest(
            "_modified_at requires _created_at when allow_system_timestamps=true".to_string(),
        ));
    }

    let modified_at = match (created_at.clone(), modified_at) {
        (Some(created), None) => Some(created),
        (_, some) => some,
    };

    Ok((created_at, modified_at))
}

fn parse_optional_utc_rfc3339(value: Option<&Value>, field: &str) -> AppResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let raw = value
        .as_str()
        .ok_or_else(|| AppError::BadRequest(format!("{field} must be RFC3339 string")))?;
    let dt = chrono::DateTime::parse_from_rfc3339(raw)
        .map_err(|_| AppError::BadRequest(format!("{field} must be valid RFC3339 datetime")))?;
    Ok(Some(
        dt.with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Millis, true),
    ))
}

fn expand_kdb_macros_in_value(value: &mut Value) -> AppResult<()> {
    match value {
        Value::Array(items) => {
            for item in items {
                expand_kdb_macros_in_value(item)?;
            }
            Ok(())
        }
        Value::Object(map) => {
            if map.is_empty() {
                return Ok(());
            }

            if map.len() == 1 {
                let macro_key = map.keys().next().cloned().unwrap_or_default();
                if macro_key.starts_with('$') && is_known_kdb_macro(&macro_key) {
                    let arg = map
                        .get(&macro_key)
                        .cloned()
                        .ok_or_else(|| AppError::Internal("macro key missing value".to_string()))?;
                    let resolved = resolve_kdb_macro(&macro_key, arg)?;
                    *value = resolved;
                    return Ok(());
                }
            }

            for v in map.values_mut() {
                expand_kdb_macros_in_value(v)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn is_known_kdb_macro(key: &str) -> bool {
    matches!(
        key,
        "$ts_now"
            | "$ts_now_ms"
            | "$id_uuidv4"
            | "$id_uuidv7"
            | "$id_random"
            | "$hash_value"
    )
}

fn resolve_kdb_macro(key: &str, arg: Value) -> AppResult<Value> {
    match key {
        "$ts_now" => resolve_kdb_now(arg),
        "$ts_now_ms" => resolve_kdb_now_ms(arg),
        "$id_uuidv4" => resolve_kdb_uuid(arg, false),
        "$id_uuidv7" => resolve_kdb_uuid(arg, true),
        "$id_random" => resolve_kdb_rand_id(arg),
        "$hash_value" => resolve_kdb_hash(arg),
        _ => Err(AppError::BadRequest(format!("unknown macro key: {key}"))),
    }
}

fn resolve_kdb_now(arg: Value) -> AppResult<Value> {
    let dt = shifted_now(arg)?;
    Ok(Value::String(dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()))
}

fn resolve_kdb_now_ms(arg: Value) -> AppResult<Value> {
    let dt = shifted_now(arg)?;
    Ok(json!(dt.timestamp_millis()))
}

fn shifted_now(arg: Value) -> AppResult<chrono::DateTime<Utc>> {
    let mut dt = Utc::now();
    if let Value::Object(shift) = arg {
        for (k, v) in shift {
            let n = v
                .as_i64()
                .ok_or_else(|| AppError::BadRequest(format!("{k} shift must be an integer")))?;
            dt = match k.as_str() {
                "days" => dt + Duration::days(n),
                "hours" => dt + Duration::hours(n),
                "minutes" => dt + Duration::minutes(n),
                "seconds" => dt + Duration::seconds(n),
                _ => {
                    return Err(AppError::BadRequest(
                        "now shift only supports days|hours|minutes|seconds".to_string(),
                    ));
                }
            };
        }
    }
    Ok(dt)
}

fn resolve_kdb_uuid(arg: Value, use_v7: bool) -> AppResult<Value> {
    if arg == Value::Bool(true) {
        let id = if use_v7 {
            Uuid::now_v7().simple().to_string()
        } else {
            Uuid::new_v4().simple().to_string()
        };
        return Ok(Value::String(id));
    }
    let options = arg.as_object().ok_or_else(|| {
        AppError::BadRequest("uuid macro must be true or options object".to_string())
    })?;

    let mut prefix = String::new();
    let mut suffix = String::new();
    let mut dash = false;
    for (k, v) in options {
        match k.as_str() {
            "prefix" => {
                prefix = v
                    .as_str()
                    .ok_or_else(|| AppError::BadRequest("uuid prefix must be string".to_string()))?
                    .to_string();
            }
            "suffix" => {
                suffix = v
                    .as_str()
                    .ok_or_else(|| AppError::BadRequest("uuid suffix must be string".to_string()))?
                    .to_string();
            }
            "dash" => {
                dash = v
                    .as_bool()
                    .ok_or_else(|| AppError::BadRequest("uuid dash must be boolean".to_string()))?;
            }
            _ => {
                return Err(AppError::BadRequest(
                    "uuid options only support prefix|suffix|dash".to_string(),
                ));
            }
        }
    }

    let base = if use_v7 {
        let u = Uuid::now_v7();
        if dash {
            u.to_string()
        } else {
            u.simple().to_string()
        }
    } else {
        let u = Uuid::new_v4();
        if dash {
            u.to_string()
        } else {
            u.simple().to_string()
        }
    };
    Ok(Value::String(format!("{prefix}{base}{suffix}")))
}

fn resolve_kdb_rand_id(arg: Value) -> AppResult<Value> {
    if arg == Value::Bool(true) {
        let raw = Uuid::new_v4().simple().to_string();
        return Ok(Value::String(raw.chars().take(12).collect()));
    }
    let options = arg.as_object().ok_or_else(|| {
        AppError::BadRequest("rand_id macro must be true or options object".to_string())
    })?;
    let mut prefix = String::new();
    let mut suffix = String::new();
    let mut len = 12usize;
    for (k, v) in options {
        match k.as_str() {
            "prefix" => {
                prefix = v
                    .as_str()
                    .ok_or_else(|| {
                        AppError::BadRequest("rand_id prefix must be string".to_string())
                    })?
                    .to_string();
            }
            "suffix" => {
                suffix = v
                    .as_str()
                    .ok_or_else(|| {
                        AppError::BadRequest("rand_id suffix must be string".to_string())
                    })?
                    .to_string();
            }
            "len" => {
                let l = v.as_u64().ok_or_else(|| {
                    AppError::BadRequest("rand_id len must be positive integer".to_string())
                })?;
                if l == 0 || l > 128 {
                    return Err(AppError::BadRequest(
                        "rand_id len must be between 1 and 128".to_string(),
                    ));
                }
                len = l as usize;
            }
            _ => {
                return Err(AppError::BadRequest(
                    "rand_id options only support prefix|suffix|len".to_string(),
                ));
            }
        }
    }
    let mut out = String::with_capacity(len);
    while out.len() < len {
        out.push_str(&Uuid::new_v4().simple().to_string());
    }
    out.truncate(len);
    Ok(Value::String(format!("{prefix}{out}{suffix}")))
}

fn resolve_kdb_hash(arg: Value) -> AppResult<Value> {
    let options = arg
        .as_object()
        .ok_or_else(|| AppError::BadRequest("hash macro must be options object".to_string()))?;

    let mut value: Option<String> = None;
    let mut algo = "sha256".to_string();
    let mut prefix = String::new();
    let mut suffix = String::new();
    let mut len: Option<usize> = None;

    for (k, v) in options {
        match k.as_str() {
            "value" => {
                value = Some(
                    v.as_str()
                        .ok_or_else(|| {
                            AppError::BadRequest("hash value must be string".to_string())
                        })?
                        .to_string(),
                );
            }
            "algo" => {
                algo = v
                    .as_str()
                    .ok_or_else(|| AppError::BadRequest("hash algo must be string".to_string()))?
                    .to_ascii_lowercase();
            }
            "prefix" => {
                prefix = v
                    .as_str()
                    .ok_or_else(|| AppError::BadRequest("hash prefix must be string".to_string()))?
                    .to_string();
            }
            "suffix" => {
                suffix = v
                    .as_str()
                    .ok_or_else(|| AppError::BadRequest("hash suffix must be string".to_string()))?
                    .to_string();
            }
            "len" => {
                let l = v.as_u64().ok_or_else(|| {
                    AppError::BadRequest("hash len must be positive integer".to_string())
                })?;
                if l == 0 || l > 64 {
                    return Err(AppError::BadRequest(
                        "hash len must be between 1 and 64".to_string(),
                    ));
                }
                len = Some(l as usize);
            }
            _ => {
                return Err(AppError::BadRequest(
                    "hash options only support value|algo|len|prefix|suffix".to_string(),
                ));
            }
        }
    }

    let value = value.ok_or_else(|| AppError::BadRequest("hash value is required".to_string()))?;
    if algo != "sha256" {
        return Err(AppError::BadRequest("hash algo must be sha256".to_string()));
    }

    let digest = Sha256::digest(value.as_bytes());
    let mut hex = format!("{digest:x}");
    if let Some(n) = len {
        hex.truncate(n);
    }
    Ok(Value::String(format!("{prefix}{hex}{suffix}")))
}

fn update_requires_mutation_engine(data: &serde_json::Map<String, Value>) -> bool {
    data.values().any(|v| match v {
        Value::Object(obj) if obj.len() == 1 => obj
            .keys()
            .next()
            .map(|k| k.starts_with('$'))
            .unwrap_or(false),
        _ => false,
    })
}

async fn update_one_with_mutation(
    conn: &libsql::Connection,
    collection: Option<&str>,
    id: &str,
    patch_obj: &mut serde_json::Map<String, Value>,
    strict: bool,
    jsonb_enabled: bool,
) -> AppResult<Option<Value>> {
    let mut binds = vec![libsql::Value::Text(id.to_string())];
    let where_clause = if let Some(c) = collection {
        binds.insert(0, libsql::Value::Text(c.to_string()));
        "collection = ? AND id = ?".to_string()
    } else {
        "id = ?".to_string()
    };

    let mut rows = conn
        .query(
            &format!("SELECT rowid, json(data) FROM __kdb_documents WHERE {where_clause} LIMIT 1"),
            binds,
        )
        .await
        .map_err(|e| AppError::Internal(format!("update mutation read failed: {e}")))?;
    let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("update mutation row read failed: {e}")))?
    else {
        return Ok(None);
    };

    let rowid: i64 = row
        .get(0)
        .map_err(|e| AppError::Internal(format!("update mutation rowid decode failed: {e}")))?;
    let raw: String = row
        .get(1)
        .map_err(|e| AppError::Internal(format!("update mutation data decode failed: {e}")))?;
    let mut doc = serde_json::from_str::<Value>(&raw)
        .map_err(|e| AppError::Internal(format!("update mutation json decode failed: {e}")))?;
    apply_mutation_patch_to_doc(&mut doc, patch_obj, strict)?;
    let updated = update_rowid_json(conn, rowid, &doc, jsonb_enabled).await?;
    Ok(Some(updated))
}

async fn update_one_by_rowid_with_mutation(
    conn: &libsql::Connection,
    rowid: i64,
    patch_obj: &mut serde_json::Map<String, Value>,
    strict: bool,
    jsonb_enabled: bool,
) -> AppResult<Option<Value>> {
    let mut rows = conn
        .query(
            "SELECT json(data) FROM __kdb_documents WHERE rowid = ? LIMIT 1",
            libsql::params![rowid],
        )
        .await
        .map_err(|e| AppError::Internal(format!("update mutation rowid read failed: {e}")))?;
    let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("update mutation rowid row failed: {e}")))?
    else {
        return Ok(None);
    };

    let raw: String = row
        .get(0)
        .map_err(|e| AppError::Internal(format!("update mutation rowid decode failed: {e}")))?;
    let mut doc = serde_json::from_str::<Value>(&raw)
        .map_err(|e| AppError::Internal(format!("update mutation rowid json decode failed: {e}")))?;
    apply_mutation_patch_to_doc(&mut doc, patch_obj, strict)?;
    let updated = update_rowid_json(conn, rowid, &doc, jsonb_enabled).await?;
    Ok(Some(updated))
}

async fn update_rowid_json(
    conn: &libsql::Connection,
    rowid: i64,
    doc: &Value,
    jsonb_enabled: bool,
) -> AppResult<Value> {
    let data_expr = json_input_expr(jsonb_enabled);
    let data = doc.to_string();
    let mut rows = conn
        .query(
            &format!(
                "UPDATE __kdb_documents
                 SET data = {data_expr},
                     _size_bytes = length(?),
                     _modified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE rowid = ?
                 RETURNING json(data)"
            ),
            libsql::params![data.clone(), data.len() as i64, rowid],
        )
        .await
        .map_err(|e| AppError::Internal(format!("update mutation write failed: {e}")))?;
    let row = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("update mutation return row failed: {e}")))?
        .ok_or_else(|| AppError::Internal("update mutation updated row missing".to_string()))?;
    let raw: String = row
        .get(0)
        .map_err(|e| AppError::Internal(format!("update mutation return decode failed: {e}")))?;
    serde_json::from_str::<Value>(&raw)
        .map_err(|e| AppError::Internal(format!("update mutation return json decode failed: {e}")))
}

async fn select_rowids_for_update(
    conn: &libsql::Connection,
    where_clause: &str,
    mut binds: Vec<libsql::Value>,
    limit: i64,
) -> AppResult<Vec<i64>> {
    binds.push(libsql::Value::Integer(limit));
    let mut rows = conn
        .query(
            &format!(
                "SELECT rowid FROM __kdb_documents WHERE {where_clause} ORDER BY rowid LIMIT ?"
            ),
            binds,
        )
        .await
        .map_err(|e| AppError::Internal(format!("select rowids for update failed: {e}")))?;
    let mut out = Vec::<i64>::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("select rowids row failed: {e}")))?
    {
        let rowid: i64 = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("select rowids decode failed: {e}")))?;
        out.push(rowid);
    }
    Ok(out)
}

fn apply_mutation_patch_to_doc(
    doc: &mut Value,
    patch: &mut serde_json::Map<String, Value>,
    strict: bool,
) -> AppResult<()> {
    for (path, spec) in patch {
        validate_projection_path(path, "data")?;
        apply_single_mutation_field(doc, path, spec, strict)?;
    }
    Ok(())
}

fn apply_single_mutation_field(
    doc: &mut Value,
    path: &str,
    spec: &Value,
    strict: bool,
) -> AppResult<()> {
    let Some(obj) = spec.as_object() else {
        let mut v = spec.clone();
        expand_kdb_macros_in_value(&mut v)?;
        set_path(doc, path, v)?;
        return Ok(());
    };
    if obj.len() != 1 {
        let mut v = spec.clone();
        expand_kdb_macros_in_value(&mut v)?;
        set_path(doc, path, v)?;
        return Ok(());
    }
    let (op, arg) = obj
        .iter()
        .next()
        .ok_or_else(|| AppError::Internal("mutation op missing".to_string()))?;
    if !op.starts_with('$') {
        let mut v = spec.clone();
        expand_kdb_macros_in_value(&mut v)?;
        set_path(doc, path, v)?;
        return Ok(());
    }

    if is_known_kdb_macro(op) {
        let mut v = spec.clone();
        expand_kdb_macros_in_value(&mut v)?;
        set_path(doc, path, v)?;
        return Ok(());
    }

    match op.as_str() {
        "$unset" => {
            if arg == &Value::Bool(true) {
                drop_path(doc, path);
            } else if strict {
                return Err(AppError::BadRequest(format!(
                    "{path}: $unset expects true"
                )));
            }
        }
        "$inc" => {
            let delta = match parse_inc_delta(arg) {
                Ok(v) => v,
                Err(_) if !strict => return Ok(()),
                Err(e) => return Err(e),
            };
            match get_path(doc, path) {
                Some(Value::Number(cur)) => {
                    let next = number_to_f64(&cur)?.unwrap_or(0.0) + delta;
                    set_path(doc, path, number_value(next)?)?;
                }
                Some(Value::Null) | None => {
                    set_path(doc, path, number_value(delta)?)?;
                }
                Some(_) => {
                    if strict {
                        return Err(AppError::BadRequest(format!(
                            "{path}: $inc requires numeric or null field"
                        )));
                    }
                }
            }
        }
        "$push" => {
            apply_array_op(doc, path, strict, |arr| {
                arr.push(arg.clone());
                Ok(())
            })?;
        }
        "$pop" => {
            let mode = parse_pop_mode(arg, strict, path)?;
            apply_array_op(doc, path, strict, |arr| {
                if arr.is_empty() {
                    return Ok(());
                }
                if mode < 0 {
                    arr.remove(0);
                } else {
                    arr.pop();
                }
                Ok(())
            })?;
        }
        "$extend" => {
            let ext = arg.as_array().cloned();
            if ext.is_none() {
                if strict {
                    return Err(AppError::BadRequest(format!(
                        "{path}: $extend expects array"
                    )));
                }
                return Ok(());
            }
            let ext = ext.unwrap_or_default();
            apply_array_op(doc, path, strict, move |arr| {
                arr.extend(ext.clone());
                Ok(())
            })?;
        }
        "$pull" => {
            let candidates = if let Some(a) = arg.as_array() {
                a.clone()
            } else {
                vec![arg.clone()]
            };
            apply_array_op(doc, path, strict, move |arr| {
                arr.retain(|v| !candidates.iter().any(|c| c == v));
                Ok(())
            })?;
        }
        "$addset" => {
            let candidates = if let Some(a) = arg.as_array() {
                a.clone()
            } else {
                vec![arg.clone()]
            };
            apply_array_op(doc, path, strict, move |arr| {
                for c in candidates.clone() {
                    if !arr.iter().any(|v| *v == c) {
                        arr.push(c);
                    }
                }
                Ok(())
            })?;
        }
        _ => {
            if strict {
                return Err(AppError::BadRequest(format!(
                    "{path}: unknown mutation operator {op}"
                )));
            }
        }
    }

    Ok(())
}

fn parse_inc_delta(arg: &Value) -> AppResult<f64> {
    if *arg == Value::Bool(true) {
        return Ok(1.0);
    }
    if let Some(v) = arg.as_i64() {
        return Ok(v as f64);
    }
    if let Some(v) = arg.as_u64() {
        return Ok(v as f64);
    }
    if let Some(v) = arg.as_f64() {
        return Ok(v);
    }
    Err(AppError::BadRequest(
        "$inc expects true or numeric delta".to_string(),
    ))
}

fn parse_pop_mode(arg: &Value, strict: bool, path: &str) -> AppResult<i64> {
    if arg == &Value::Bool(true) || arg == &json!(1) {
        return Ok(1);
    }
    if arg == &json!(-1) {
        return Ok(-1);
    }
    if strict {
        return Err(AppError::BadRequest(format!(
            "{path}: $pop expects 1|-1|true"
        )));
    }
    Ok(1)
}

fn apply_array_op<F>(doc: &mut Value, path: &str, strict: bool, mut f: F) -> AppResult<()>
where
    F: FnMut(&mut Vec<Value>) -> AppResult<()>,
{
    match get_path(doc, path) {
        Some(Value::Array(_)) => {}
        Some(Value::Null) | None => {
            set_path(doc, path, Value::Array(vec![]))?;
        }
        Some(_) => {
            if strict {
                return Err(AppError::BadRequest(format!(
                    "{path}: target must be array|null|missing"
                )));
            }
            return Ok(());
        }
    }

    let arr = get_path_mut(doc, path).and_then(Value::as_array_mut).ok_or_else(|| {
        AppError::Internal(format!("failed to access array field for path: {path}"))
    })?;
    f(arr)
}

fn get_path(root: &Value, path: &str) -> Option<Value> {
    let mut cur = root;
    for seg in path.split('.').filter(|s| !s.is_empty()) {
        cur = cur.get(seg)?;
    }
    Some(cur.clone())
}

fn get_path_mut<'a>(root: &'a mut Value, path: &str) -> Option<&'a mut Value> {
    let mut cur = root;
    for seg in path.split('.').filter(|s| !s.is_empty()) {
        cur = cur.get_mut(seg)?;
    }
    Some(cur)
}

fn set_path(root: &mut Value, path: &str, value: Value) -> AppResult<()> {
    if !root.is_object() {
        *root = Value::Object(serde_json::Map::new());
    }
    let mut parts = path.split('.').filter(|s| !s.is_empty()).peekable();
    let mut cur = root
        .as_object_mut()
        .ok_or_else(|| AppError::Internal("set_path root object expected".to_string()))?;
    while let Some(seg) = parts.next() {
        if parts.peek().is_none() {
            cur.insert(seg.to_string(), value);
            return Ok(());
        }
        let next = cur
            .entry(seg.to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !next.is_object() {
            *next = Value::Object(serde_json::Map::new());
        }
        cur = next
            .as_object_mut()
            .ok_or_else(|| AppError::Internal("set_path failed".to_string()))?;
    }
    Ok(())
}

fn number_to_f64(n: &serde_json::Number) -> AppResult<Option<f64>> {
    if let Some(v) = n.as_f64() {
        return Ok(Some(v));
    }
    Ok(None)
}

fn number_value(v: f64) -> AppResult<Value> {
    if v.fract() == 0.0 && v >= i64::MIN as f64 && v <= i64::MAX as f64 {
        return Ok(Value::Number(serde_json::Number::from(v as i64)));
    }
    let Some(n) = serde_json::Number::from_f64(v) else {
        return Err(AppError::BadRequest(
            "numeric operation produced non-finite value".to_string(),
        ));
    };
    Ok(Value::Number(n))
}

fn extract_single_id(payload: &OperationPayload) -> AppResult<String> {
    if let Some(id) = &payload.id {
        if !id.trim().is_empty() {
            return Ok(id.clone());
        }
    }

    if let Some(data) = &payload.data {
        if let Some(obj) = data.as_object() {
            if let Some(id) = obj.get("_id").and_then(Value::as_str) {
                if !id.trim().is_empty() {
                    return Ok(id.to_string());
                }
            }
        }
    }

    Err(AppError::BadRequest("_id is required".to_string()))
}

fn extract_ids_or_single(payload: &OperationPayload) -> AppResult<Vec<String>> {
    if let Some(ids) = payload.ids.as_ref() {
        if ids.is_empty() {
            return Err(AppError::BadRequest("ids cannot be empty".to_string()));
        }
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if id.trim().is_empty() {
                return Err(AppError::BadRequest(
                    "ids must not contain empty values".to_string(),
                ));
            }
            out.push(id.clone());
        }
        return Ok(out);
    }

    Ok(vec![extract_single_id(payload)?])
}

fn validate_ids_or_filter_target(payload: &OperationPayload, op_name: &str) -> AppResult<()> {
    if payload.ids.is_some() && payload.filter.is_some() {
        return Err(AppError::BadRequest(
            "ids and filter cannot be provided together".to_string(),
        ));
    }
    if payload.ids.is_none() && payload.filter.is_none() {
        return Err(AppError::BadRequest(format!(
            "{op_name} requires ids or filter"
        )));
    }
    Ok(())
}

async fn target_ids_from_payload_with_collection(
    conn: &libsql::Connection,
    payload: &OperationPayload,
    collection: Option<&str>,
    max_docs: Option<i64>,
    op_name: &str,
) -> AppResult<Vec<String>> {
    validate_ids_or_filter_target(payload, op_name)?;
    if let Some(ids) = payload.ids.as_ref() {
        return apply_max_docs_to_ids(ids.clone(), max_docs);
    }
    let filter = require_non_empty_filter(payload.filter.clone())?;
    select_ids_by_filter(conn, collection, filter, max_docs).await
}

fn extract_ids_or_single_strict(payload: &OperationPayload) -> AppResult<Vec<String>> {
    if payload.ids.is_some()
        && (payload.id.is_some()
            || payload
                .data
                .as_ref()
                .and_then(Value::as_object)
                .and_then(|o| o.get("_id"))
                .is_some())
    {
        return Err(AppError::BadRequest(
            "provide either id (or data._id) OR ids, not both".to_string(),
        ));
    }
    extract_ids_or_single(payload)
}

fn apply_max_docs_to_ids(mut ids: Vec<String>, max_docs: Option<i64>) -> AppResult<Vec<String>> {
    if let Some(m) = max_docs {
        if m < -1 {
            return Err(AppError::BadRequest(
                "max_docs must be -1, 0, or positive".to_string(),
            ));
        }
        if m == 0 {
            ids.clear();
        } else if m > 0 {
            let m = m as usize;
            if ids.len() > m {
                ids.truncate(m);
            }
        }
    }

    Ok(ids)
}

fn normalize_max_docs(max_docs: Option<i64>, matched: i64) -> AppResult<usize> {
    let limit = max_docs.unwrap_or(matched);
    if limit < -1 {
        return Err(AppError::BadRequest(
            "max_docs must be -1, 0, or positive".to_string(),
        ));
    }
    if limit == -1 {
        return Ok(matched as usize);
    }
    Ok(std::cmp::min(limit as usize, matched as usize))
}

async fn select_ids_by_filter(
    conn: &libsql::Connection,
    collection: Option<&str>,
    filter: Value,
    max_docs: Option<i64>,
) -> AppResult<Vec<String>> {
    let (where_clause, mut binds) =
        build_where_with_collection(filter, collection.map(ToOwned::to_owned))?;
    let mut sql = format!("SELECT id FROM __kdb_documents WHERE {where_clause} ORDER BY rowid");
    if let Some(max_docs) = max_docs {
        if max_docs < -1 {
            return Err(AppError::BadRequest(
                "max_docs must be -1, 0, or positive".to_string(),
            ));
        }
        if max_docs == -1 {
            // no LIMIT
        } else {
            sql.push_str(" LIMIT ?");
            binds.push(libsql::Value::Integer(max_docs));
        }
    }

    let mut rows = conn
        .query(&sql, binds)
        .await
        .map_err(|e| AppError::Internal(format!("select ids by filter failed: {e}")))?;

    let mut ids = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("select ids row read failed: {e}")))?
    {
        ids.push(
            row.get::<String>(0)
                .map_err(|e| AppError::Internal(format!("select ids row decode failed: {e}")))?,
        );
    }

    Ok(ids)
}

async fn select_namespace_target_ids(
    conn: &libsql::Connection,
    collection: &str,
    max_docs: Option<i64>,
) -> AppResult<Vec<String>> {
    select_ids_by_filter(conn, Some(collection), json!({}), max_docs).await
}

async fn count_by_ids(
    conn: &libsql::Connection,
    collection: Option<&str>,
    ids: &[String],
) -> AppResult<usize> {
    if ids.is_empty() {
        return Ok(0);
    }

    let placeholders = vec!["?"; ids.len()].join(", ");
    let mut binds: Vec<libsql::Value> = Vec::new();
    let where_clause = where_ids_with_scope(&mut binds, collection, ids, &placeholders);

    let mut rows = conn
        .query(
            &format!("SELECT COUNT(*) FROM __kdb_documents WHERE {}", where_clause),
            binds,
        )
        .await
        .map_err(|e| AppError::Internal(format!("count_by_ids failed: {e}")))?;

    if let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("count_by_ids row read failed: {e}")))?
    {
        let count: i64 = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("count_by_ids row decode failed: {e}")))?;
        Ok(count as usize)
    } else {
        Ok(0)
    }
}

async fn count_and_bytes_by_ids(
    conn: &libsql::Connection,
    collection: Option<&str>,
    ids: &[String],
) -> AppResult<(i64, i64)> {
    if ids.is_empty() {
        return Ok((0, 0));
    }

    let placeholders = vec!["?"; ids.len()].join(", ");
    let mut binds: Vec<libsql::Value> = Vec::new();
    let where_clause = where_ids_with_scope(&mut binds, collection, ids, &placeholders);

    let mut rows = conn
        .query(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(_size_bytes), 0) FROM __kdb_documents WHERE {}",
                where_clause
            ),
            binds,
        )
        .await
        .map_err(|e| AppError::Internal(format!("count_and_bytes_by_ids failed: {e}")))?;

    if let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("count_and_bytes_by_ids row read failed: {e}")))?
    {
        let count: i64 = row.get(0).map_err(|e| {
            AppError::Internal(format!("count_and_bytes_by_ids count decode failed: {e}"))
        })?;
        let bytes: i64 = row.get(1).map_err(|e| {
            AppError::Internal(format!("count_and_bytes_by_ids bytes decode failed: {e}"))
        })?;
        Ok((count, bytes))
    } else {
        Ok((0, 0))
    }
}

async fn fetch_kdb_documents_by_ids(
    conn: &libsql::Connection,
    collection: Option<&str>,
    ids: &[String],
    include_system_timestamps: bool,
) -> AppResult<Vec<Value>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders = vec!["?"; ids.len()].join(", ");
    let mut binds: Vec<libsql::Value> = Vec::new();
    let where_clause = where_ids_with_scope(&mut binds, collection, ids, &placeholders);

    let sql = if include_system_timestamps {
        format!("SELECT json(data), _user_id, _created_at, _modified_at FROM __kdb_documents WHERE {where_clause}")
    } else {
        format!("SELECT json(data), _user_id FROM __kdb_documents WHERE {where_clause}")
    };

    let mut rows = conn
        .query(&sql, binds)
        .await
        .map_err(|e| AppError::Internal(format!("fetch documents failed: {e}")))?;

    let mut by_id = HashMap::<String, Value>::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("fetch documents row read failed: {e}")))?
    {
        let raw: String = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("fetch documents row decode failed: {e}")))?;
        let mut item = serde_json::from_str::<Value>(&raw)
            .map_err(|e| AppError::Internal(format!("fetch documents json decode failed: {e}")))?;
        let user_id: Option<String> = row.get(1).map_err(|e| {
            AppError::Internal(format!("fetch documents _user_id decode failed: {e}"))
        })?;
        attach_document_user_id(&mut item, user_id);
        if include_system_timestamps {
            let created_at: Option<String> = row.get(2).map_err(|e| {
                AppError::Internal(format!("fetch documents created_at decode failed: {e}"))
            })?;
            let modified_at: Option<String> = row.get(3).map_err(|e| {
                AppError::Internal(format!("fetch documents modified_at decode failed: {e}"))
            })?;
            attach_system_timestamps(&mut item, created_at, modified_at);
        }
        if let Some(id) = item.get("_id").and_then(Value::as_str) {
            by_id.insert(id.to_string(), item);
        }
    }

    let mut ordered = Vec::<Value>::new();
    for id in ids {
        if let Some(item) = by_id.get(id) {
            ordered.push(item.clone());
        }
    }
    Ok(ordered)
}

async fn hard_delete_document_ids(
    conn: &libsql::Connection,
    collection: Option<&str>,
    ids: &[String],
) -> AppResult<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders = vec!["?"; ids.len()].join(", ");
    let mut binds: Vec<libsql::Value> = Vec::new();
    let where_clause = where_ids_with_scope(&mut binds, collection, ids, &placeholders);
    let tx = conn
        .transaction()
        .await
        .map_err(|e| AppError::Internal(format!("hard delete tx begin failed: {e}")))?;
    tx.execute(
        &format!(
            "DELETE FROM __kdb_document_transitions WHERE document_id IN (
                 SELECT id FROM __kdb_documents WHERE {where_clause}
             )"
        ),
        binds.clone(),
    )
    .await
    .map_err(|e| AppError::Internal(format!("hard delete transitions failed: {e}")))?;
    let deleted = tx
        .execute(
            &format!("DELETE FROM __kdb_documents WHERE {where_clause}"),
            binds,
        )
        .await
        .map_err(|e| AppError::Internal(format!("hard delete failed: {e}")))?;
    tx.commit()
        .await
        .map_err(|e| AppError::Internal(format!("hard delete tx commit failed: {e}")))?;
    Ok(deleted as usize)
}

async fn stats_for_table(
    conn: &libsql::Connection,
    table: &str,
    collection: &str,
) -> AppResult<(i64, i64)> {
    let mut rows = conn
        .query(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(_size_bytes), 0) FROM {table} WHERE collection = ?"
            ),
            libsql::params![collection],
        )
        .await
        .map_err(|e| AppError::Internal(format!("stats query failed: {e}")))?;
    if let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("stats row read failed: {e}")))?
    {
        let count: i64 = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("stats count decode failed: {e}")))?;
        let bytes: i64 = row
            .get(1)
            .map_err(|e| AppError::Internal(format!("stats bytes decode failed: {e}")))?;
        Ok((count, bytes))
    } else {
        Ok((0, 0))
    }
}

async fn count_restorable_kdb_archive_rows(
    conn: &libsql::Connection,
    where_clause: &str,
    binds: Vec<libsql::Value>,
) -> AppResult<usize> {
    let mut rows = conn
        .query(
            &format!(
                "SELECT COUNT(*) FROM __kdb_archive a
                 WHERE {where_clause}
                   AND NOT EXISTS (SELECT 1 FROM __kdb_documents d WHERE d.id = a.id)"
            ),
            binds,
        )
        .await
        .map_err(|e| AppError::Internal(format!("restorable count failed: {e}")))?;
    if let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("restorable count row read failed: {e}")))?
    {
        let count: i64 = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("restorable count decode failed: {e}")))?;
        Ok(count as usize)
    } else {
        Ok(0)
    }
}

async fn pragma_i64(conn: &libsql::Connection, name: &str) -> AppResult<i64> {
    let mut rows = conn
        .query(&format!("PRAGMA {name}"), ())
        .await
        .map_err(|e| AppError::Internal(format!("pragma {name} failed: {e}")))?;
    if let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("pragma {name} row read failed: {e}")))?
    {
        row.get(0)
            .map_err(|e| AppError::Internal(format!("pragma {name} decode failed: {e}")))
    } else {
        Ok(0)
    }
}

fn build_kdb_archive_target_where(
    payload: &OperationPayload,
    allow_empty_filter: bool,
) -> AppResult<(String, Vec<libsql::Value>)> {
    let has_txn = payload
        .txn_id
        .as_ref()
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    let has_ids = payload.ids.as_ref().map(|v| !v.is_empty()).unwrap_or(false);
    let has_collection = payload
        .collection
        .as_ref()
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    let has_filter = payload.filter.is_some();

    let mode_count = (has_txn as u8) + (has_ids as u8) + ((has_collection || has_filter) as u8);
    if mode_count != 1 {
        return Err(AppError::BadRequest(
            "provide exactly one target mode: txn_id OR ids OR namespace/filter".to_string(),
        ));
    }

    if has_txn {
        return Ok((
            "_txn_id = ?".to_string(),
            vec![libsql::Value::Text(
                payload.txn_id.clone().expect("checked non-empty"),
            )],
        ));
    }

    if has_ids {
        let ids = payload.ids.clone().expect("checked");
        let placeholders = vec!["?"; ids.len()].join(", ");
        let binds = ids
            .into_iter()
            .map(libsql::Value::Text)
            .collect::<Vec<libsql::Value>>();
        return Ok((format!("id IN ({placeholders})"), binds));
    }

    let scope = payload.scope.as_deref().unwrap_or("namespace");
    let collection = match scope {
        "all" => None,
        "namespace" => {
            if has_collection {
                payload.collection.clone()
            } else {
                None
            }
        }
        _ => {
            return Err(AppError::BadRequest(
                "scope must be either 'namespace' or 'all'".to_string(),
            ));
        }
    };
    if scope == "namespace" && collection.is_none() && has_filter {
        return Err(AppError::BadRequest(
            "namespace is required for filter mode unless scope='all'".to_string(),
        ));
    }

    let filter = payload.filter.clone().unwrap_or_else(|| json!({}));
    if !allow_empty_filter && collection.is_none() {
        let is_empty = filter.as_object().map(|m| m.is_empty()).unwrap_or(false);
        if is_empty {
            return Err(AppError::BadRequest(
                "filter cannot be empty when scope='all'".to_string(),
            ));
        }
    }
    build_where_with_collection(filter, collection)
}

fn ttl_to_expires_at(ttl_seconds: Option<i64>) -> AppResult<Option<i64>> {
    match ttl_seconds {
        None => Ok(None),
        Some(ttl) if ttl <= 0 => Err(AppError::BadRequest(
            "ttl_seconds must be greater than 0".to_string(),
        )),
        Some(ttl) => Ok(Some(unix_now_secs() + ttl)),
    }
}

fn normalized_expiry_behavior(value: Option<&str>) -> String {
    match value.map(str::trim).map(str::to_ascii_lowercase) {
        Some(v) if v == "delete" => "delete".to_string(),
        _ => "archive".to_string(),
    }
}

fn unix_now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn resolve_source(params: &OperationPayload) -> &'static str {
    if params.archive_only.unwrap_or(false) {
        "__kdb_archive"
    } else if params.include_archive.unwrap_or(false) {
        "(SELECT * FROM __kdb_documents UNION ALL SELECT * FROM __kdb_archive)"
    } else {
        "__kdb_documents"
    }
}

#[cfg(test)]
mod transaction_tests {
    use super::*;

    fn payload(value: Value) -> OperationPayload {
        serde_json::from_value(value).expect("valid operation payload")
    }

    #[test]
    fn transaction_operations_default_to_fail_and_normalize_scope() {
        let request = payload(json!({
            "operations": [
                {
                    "alias": "create-user",
                    "operation": "insert::users",
                    "payload": {"data": {"name": "Ada"}}
                }
            ]
        }));

        let (on_error, operations) =
            prepare_transaction_operations(request).expect("valid transaction");
        assert_eq!(on_error, BatchOnError::Fail);
        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].operation, "insert");
        assert_eq!(operations[0].alias.as_deref(), Some("create-user"));
        assert_eq!(operations[0].payload.collection.as_deref(), Some("users"));
    }

    #[test]
    fn transaction_operations_validate_policy_and_operation() {
        let invalid_policy = payload(json!({
            "on_error": "skip",
            "operations": [{"operation": "insert", "namespace": "users", "payload": {}}]
        }));
        assert!(prepare_transaction_operations(invalid_policy).is_err());

        let missing_operation = payload(json!({
            "operations": [{"namespace": "users", "payload": {}}]
        }));
        assert!(prepare_transaction_operations(missing_operation).is_err());

        let upsert = payload(json!({
            "operations": [{
                "alias": "ensure-user",
                "operation": "upsert",
                "namespace": "users",
                "payload": {
                    "filter": {"email": "ada@example.com"},
                    "insert_data": {"email": "ada@example.com"},
                    "update_data": {"visits": {"$inc": 1}}
                }
            }]
        }));
        let (_, operations) =
            prepare_transaction_operations(upsert).expect("transaction upsert is supported");
        assert_eq!(operations[0].operation, "upsert");
        assert_eq!(operations[0].namespace.as_deref(), Some("users"));
    }
}
