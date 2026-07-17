// Identity-store operations: stores user login metadata without authenticating users.

async fn user_create(
    state: &AppState,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let user_id = normalize_optional_id(payload.user_id)?.unwrap_or_else(|| Uuid::new_v4().simple().to_string());
    let status = non_empty_or(payload.status, "status", "active")?;
    let email = clean_optional(payload.email);
    let username = clean_optional(payload.username);
    let phone = clean_optional(payload.phone);
    let first_name = clean_optional(payload.first_name);
    let last_name = clean_optional(payload.last_name);
    let profile_photo = clean_optional(payload.profile_photo);
    let status_reason = clean_optional(payload.status_reason);
    let password_hash = clean_optional(payload.password_hash);
    let password_algo = clean_optional(payload.password_algo);
    let requires_password_change = payload.requires_password_change.unwrap_or(false);
    let provider = payload.provider;
    let provider_user_id = payload.provider_user_id;
    let data = payload.data.unwrap_or_else(|| json!({}));
    if !data.is_object() {
        return Err(AppError::BadRequest("data must be an object when provided".to_string()));
    }
    let json_expr = json_input_expr(state.jsonb_enabled);
    conn.execute(
        &format!(
            "INSERT INTO __kdb_identity_users (
                id, email, username, phone, first_name, last_name, profile_photo,
                status, status_reason, password_hash, password_algo,
                password_updated_at, requires_password_change, data
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CASE WHEN ? IS NULL THEN NULL ELSE strftime('%Y-%m-%dT%H:%M:%fZ', 'now') END, ?, {json_expr})"
        ),
        libsql::params![
            user_id.clone(),
            to_sql_nullable_text(email.clone()),
            to_sql_nullable_text(username),
            to_sql_nullable_text(phone),
            to_sql_nullable_text(first_name),
            to_sql_nullable_text(last_name),
            to_sql_nullable_text(profile_photo),
            status.clone(),
            to_sql_nullable_text(status_reason),
            to_sql_nullable_text(password_hash.clone()),
            to_sql_nullable_text(password_algo),
            to_sql_nullable_text(password_hash),
            if requires_password_change { 1 } else { 0 },
            data.to_string()
        ],
    )
    .await
    .map_err(|e| AppError::BadRequest(format!("user_create failed: {e}")))?;
    log_identity_event(
        conn,
        Some(&user_id),
        "user.created",
        json!({"status": status, "requires_password_change": requires_password_change}),
    )
    .await?;
    if clean_optional(provider.clone()).is_some()
        || clean_optional(provider_user_id.clone()).is_some()
    {
        link_identity_provider(
            state,
            conn,
            &user_id,
            provider,
            provider_user_id,
            email,
            json!({}),
        )
        .await?;
    }
    let user = fetch_identity_user(conn, &user_id).await?;
    Ok(GatewayResponse::ok(Some(json!({"item": user}))))
}

async fn user_get(conn: &libsql::Connection, req: GatewayRequest) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    if clean_optional(payload.provider.clone()).is_some()
        || clean_optional(payload.provider_user_id.clone()).is_some()
    {
        let provider = required_text(payload.provider, "provider")?;
        let provider_user_id = required_text(payload.provider_user_id, "provider_user_id")?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {} FROM __kdb_identity_users u
                     JOIN __kdb_identity_providers p ON p.user_id = u.id
                     WHERE p.provider = ? AND p.provider_user_id = ?
                     LIMIT 1",
                    identity_user_select_with_alias("u")
                ),
                libsql::params![provider, provider_user_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("user_get provider query failed: {e}")))?;
        let item = if let Some(row) = rows
            .next()
            .await
            .map_err(|e| AppError::Internal(format!("user_get provider row read failed: {e}")))?
        {
            Some(identity_user_from_row(&row)?)
        } else {
            None
        };
        return Ok(GatewayResponse::ok(Some(json!({"item": item}))));
    }

    let (where_sql, bind) = if let Some(user_id) = clean_optional(payload.user_id).or(payload.id) {
        ("id = ?", user_id)
    } else if let Some(email) = clean_optional(payload.email) {
        ("email = ?", email)
    } else if let Some(username) = clean_optional(payload.username) {
        ("username = ?", username)
    } else {
        return Err(AppError::BadRequest("user_id, id, email, or username is required".to_string()));
    };
    let mut rows = conn
        .query(
            &format!("SELECT {} FROM __kdb_identity_users WHERE {where_sql} LIMIT 1", identity_user_select()),
            libsql::params![bind],
        )
        .await
        .map_err(|e| AppError::Internal(format!("user_get query failed: {e}")))?;
    let item = if let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("user_get row read failed: {e}")))?
    {
        Some(identity_user_from_row(&row)?)
    } else {
        None
    };
    Ok(GatewayResponse::ok(Some(json!({"item": item}))))
}

async fn user_list(conn: &libsql::Connection, req: GatewayRequest) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let page = payload.page.unwrap_or(1).max(1);
    let limit = payload
        .per_page
        .or(payload.limit)
        .unwrap_or(25)
        .clamp(1, 200);
    let offset = payload.offset.unwrap_or((page - 1) * limit).max(0);
    let mut clauses = Vec::<String>::new();
    let mut binds = Vec::<libsql::Value>::new();

    if let Some(status) = clean_optional(payload.status) {
        clauses.push("status = ?".to_string());
        binds.push(libsql::Value::Text(status));
    }
    if let Some(email) = clean_optional(payload.email) {
        clauses.push("email = ?".to_string());
        binds.push(libsql::Value::Text(email));
    }
    if let Some(username) = clean_optional(payload.username) {
        clauses.push("username = ?".to_string());
        binds.push(libsql::Value::Text(username));
    }
    if let Some(q) = clean_optional(payload.search) {
        clauses.push("(email LIKE ? OR username LIKE ? OR phone LIKE ? OR first_name LIKE ? OR last_name LIKE ? OR id LIKE ?)".to_string());
        let like = format!("%{}%", q.replace('%', "\\%").replace('_', "\\_"));
        binds.extend([
            libsql::Value::Text(like.clone()),
            libsql::Value::Text(like.clone()),
            libsql::Value::Text(like.clone()),
            libsql::Value::Text(like.clone()),
            libsql::Value::Text(like.clone()),
            libsql::Value::Text(like),
        ]);
    }

    let where_clause = if clauses.is_empty() {
        "1=1".to_string()
    } else {
        clauses.join(" AND ")
    };
    let total_items = identity_count(conn, &where_clause, binds.clone()).await?;
    let mut page_binds = binds;
    page_binds.push(libsql::Value::Integer(limit));
    page_binds.push(libsql::Value::Integer(offset));
    let mut rows = conn
        .query(
            &format!(
                "SELECT {} FROM __kdb_identity_users
                 WHERE {where_clause}
                 ORDER BY created_at DESC, id DESC
                 LIMIT ? OFFSET ?",
                identity_user_select()
            ),
            page_binds,
        )
        .await
        .map_err(|e| AppError::Internal(format!("user_list query failed: {e}")))?;
    let mut items = Vec::<Value>::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("user_list row read failed: {e}")))?
    {
        let mut item = identity_user_from_row(&row)?;
        enrich_identity_user(conn, &mut item).await?;
        items.push(item);
    }
    let (next_offset, prev_offset) = build_offsets(total_items, items.len(), limit, offset);
    let total_pages = if total_items == 0 {
        0
    } else {
        (total_items + limit - 1) / limit
    };
    let next_page = if next_offset.is_null() { Value::Null } else { Value::from(page + 1) };
    let prev_page = if prev_offset.is_null() { Value::Null } else { Value::from((page - 1).max(1)) };
    Ok(GatewayResponse::ok(Some(json!({
        "items": items,
        "count": items.len(),
        "total_items": total_items,
        "limit": limit,
        "offset": offset,
        "next_offset": next_offset,
        "prev_offset": prev_offset,
        "pagination": {
            "total_items": total_items,
            "count": items.len(),
            "per_page": limit,
            "page": page,
            "total_pages": total_pages,
            "next_page": next_page,
            "prev_page": prev_page
        }
    }))))
}

async fn user_get_details(conn: &libsql::Connection, req: GatewayRequest) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let user_id = if let Some(user_id) = clean_optional(payload.user_id).or(payload.id) {
        user_id
    } else if let Some(email) = clean_optional(payload.email) {
        user_id_by_field(conn, "email", &email).await?
    } else if let Some(username) = clean_optional(payload.username) {
        user_id_by_field(conn, "username", &username).await?
    } else {
        return Err(AppError::BadRequest("user_id, id, email, or username is required".to_string()));
    };
    let mut user = fetch_identity_user(conn, &user_id).await?;
    enrich_identity_user(conn, &mut user).await?;
    let providers = fetch_identity_providers(conn, &user_id).await?;
    let events = fetch_identity_events(conn, &user_id, 50).await?;
    Ok(GatewayResponse::ok(Some(json!({
        "item": user,
        "providers": providers,
        "events": events
    }))))
}

async fn user_update(
    state: &AppState,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let user_id = required_text(payload.user_id.or(payload.id), "user_id")?;
    let mut current = fetch_identity_user(conn, &user_id).await?;
    let email = clean_optional(payload.email);
    let username = clean_optional(payload.username);
    let phone = clean_optional(payload.phone);
    let first_name = clean_optional(payload.first_name);
    let last_name = clean_optional(payload.last_name);
    let profile_photo = clean_optional(payload.profile_photo);
    let email_verified_at = clean_optional(payload.email_verified_at)
        .map(|v| normalize_rfc3339_utc(&v))
        .transpose()?;
    let phone_verified_at = clean_optional(payload.phone_verified_at)
        .map(|v| normalize_rfc3339_utc(&v))
        .transpose()?;
    let requires_password_change = payload.requires_password_change;
    let data = payload.data.unwrap_or_else(|| current.get("data").cloned().unwrap_or_else(|| json!({})));
    if !data.is_object() {
        return Err(AppError::BadRequest("data must be an object when provided".to_string()));
    }
    let json_expr = json_input_expr(state.jsonb_enabled);
    conn.execute(
        &format!(
            "UPDATE __kdb_identity_users
             SET email = COALESCE(?, email),
                 username = COALESCE(?, username),
                 phone = COALESCE(?, phone),
                 first_name = COALESCE(?, first_name),
                 last_name = COALESCE(?, last_name),
                 profile_photo = COALESCE(?, profile_photo),
                 email_verified_at = COALESCE(?, email_verified_at),
                 phone_verified_at = COALESCE(?, phone_verified_at),
                 requires_password_change = COALESCE(?, requires_password_change),
                 data = {json_expr},
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?"
        ),
        libsql::params![
            to_sql_nullable_text(email),
            to_sql_nullable_text(username),
            to_sql_nullable_text(phone),
            to_sql_nullable_text(first_name),
            to_sql_nullable_text(last_name),
            to_sql_nullable_text(profile_photo),
            to_sql_nullable_text(email_verified_at),
            to_sql_nullable_text(phone_verified_at),
            requires_password_change.map(|value| if value { 1 } else { 0 }),
            data.to_string(),
            user_id.clone()
        ],
    )
    .await
    .map_err(|e| AppError::BadRequest(format!("user_update failed: {e}")))?;
    log_identity_event(conn, Some(&user_id), "user.updated", json!({})).await?;
    current = fetch_identity_user(conn, &user_id).await?;
    enrich_identity_user(conn, &mut current).await?;
    Ok(GatewayResponse::ok(Some(json!({"item": current}))))
}

async fn user_update_status(conn: &libsql::Connection, req: GatewayRequest) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let user_id = required_text(payload.user_id.or(payload.id), "user_id")?;
    let status = required_text(payload.status, "status")?;
    let expires_at = normalize_future_time(payload.status_expires_at, payload.status_expires_in, "status")?;
    let status_next = clean_optional(payload.status_next);
    if expires_at.is_some() && status_next.is_none() {
        return Err(AppError::BadRequest("status_next is required when status expiration is provided".to_string()));
    }
    if expires_at.is_none() && status_next.is_some() {
        return Err(AppError::BadRequest("status expiration is required when status_next is provided".to_string()));
    }

    let previous = fetch_identity_user(conn, &user_id).await?;
    conn.execute(
        "UPDATE __kdb_identity_users
         SET status = ?, status_reason = ?, status_changed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             status_expires_at = ?, status_next = ?, status_next_reason = ?, status_changed_by = ?,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?",
        libsql::params![
            status.clone(),
            to_sql_nullable_text(clean_optional(payload.status_reason.clone())),
            to_sql_nullable_text(expires_at.clone()),
            to_sql_nullable_text(status_next.clone()),
            to_sql_nullable_text(clean_optional(payload.status_next_reason.clone())),
            to_sql_nullable_text(clean_optional(payload.changed_by.clone())),
            user_id.clone()
        ],
    )
    .await
    .map_err(|e| AppError::Internal(format!("user_update_status failed: {e}")))?;
    log_identity_event(
        conn,
        Some(&user_id),
        "user.status_updated",
        json!({
            "previous_status": previous.get("status").cloned().unwrap_or(Value::Null),
            "status": status,
            "status_reason": payload.status_reason,
            "status_expires_at": expires_at,
            "status_next": status_next,
            "status_next_reason": payload.status_next_reason,
            "changed_by": payload.changed_by
        }),
    )
    .await?;
    let user = fetch_identity_user(conn, &user_id).await?;
    Ok(GatewayResponse::ok(Some(json!({"item": user}))))
}

async fn user_delete(conn: &libsql::Connection, req: GatewayRequest) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let user_id = required_text(payload.user_id.or(payload.id), "user_id")?;
    let purge = payload.purge.unwrap_or(false);
    let tx = conn
        .transaction()
        .await
        .map_err(|e| AppError::Internal(format!("user_delete tx begin failed: {e}")))?;
    if purge {
        let deleted_tokens = tx
            .execute("DELETE FROM __kdb_identity_tokens WHERE user_id = ?", libsql::params![user_id.clone()])
            .await
            .map_err(|e| AppError::Internal(format!("user_delete token purge failed: {e}")))?;
        let deleted_providers = tx
            .execute("DELETE FROM __kdb_identity_providers WHERE user_id = ?", libsql::params![user_id.clone()])
            .await
            .map_err(|e| AppError::Internal(format!("user_delete provider purge failed: {e}")))?;
        let deleted_events = tx
            .execute("DELETE FROM __kdb_identity_events WHERE user_id = ?", libsql::params![user_id.clone()])
            .await
            .map_err(|e| AppError::Internal(format!("user_delete event purge failed: {e}")))?;
        let deleted_users = tx
            .execute("DELETE FROM __kdb_identity_users WHERE id = ?", libsql::params![user_id.clone()])
            .await
            .map_err(|e| AppError::Internal(format!("user_delete purge failed: {e}")))?;
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("user_delete tx commit failed: {e}")))?;
        return Ok(GatewayResponse::ok(Some(json!({
            "purged": true,
            "user_id": user_id,
            "deleted_users": deleted_users,
            "deleted_tokens": deleted_tokens,
            "deleted_providers": deleted_providers,
            "deleted_events": deleted_events
        }))));
    }
    tx.execute(
        "UPDATE __kdb_identity_users
         SET status = 'deleted', status_reason = COALESCE(?, status_reason),
             status_changed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             status_expires_at = NULL, status_next = NULL, status_next_reason = NULL,
             deleted_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?",
        libsql::params![to_sql_nullable_text(clean_optional(payload.status_reason)), user_id.clone()],
    )
    .await
    .map_err(|e| AppError::Internal(format!("user_delete soft delete failed: {e}")))?;
    let revoked_tokens = tx
        .execute(
            "UPDATE __kdb_identity_tokens
             SET revoked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE user_id = ? AND used_at IS NULL AND revoked_at IS NULL",
            libsql::params![user_id.clone()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("user_delete token revoke failed: {e}")))?;
    tx.commit()
        .await
        .map_err(|e| AppError::Internal(format!("user_delete tx commit failed: {e}")))?;
    log_identity_event(conn, Some(&user_id), "user.deleted", json!({"purge": false})).await?;
    Ok(GatewayResponse::ok(Some(json!({
        "deleted": true,
        "purged": false,
        "user_id": user_id,
        "revoked_tokens": revoked_tokens
    }))))
}

async fn user_link_provider(
    state: &AppState,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let user_id = required_text(payload.user_id.or(payload.id), "user_id")?;
    let provider = required_text(payload.provider, "provider")?;
    let provider_user_id = required_text(payload.provider_user_id, "provider_user_id")?;
    let email = clean_optional(payload.email);
    let data = payload.data.unwrap_or_else(|| json!({}));
    if !data.is_object() {
        return Err(AppError::BadRequest("data must be an object when provided".to_string()));
    }
    let provider_id = link_identity_provider(
        state,
        conn,
        &user_id,
        Some(provider.clone()),
        Some(provider_user_id.clone()),
        email.clone(),
        data,
    )
    .await?;
    log_identity_event(
        conn,
        Some(&user_id),
        "user.provider_linked",
        json!({"provider_id": provider_id, "provider": provider, "provider_user_id": provider_user_id, "email": email}),
    )
    .await?;
    Ok(GatewayResponse::ok(Some(json!({
        "provider_id": provider_id,
        "user_id": user_id,
        "provider": provider,
        "provider_user_id": provider_user_id
    }))))
}

async fn user_unlink_provider(
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let user_id = clean_optional(payload.user_id.or(payload.id));
    let provider = required_text(payload.provider, "provider")?;
    let provider_user_id = required_text(payload.provider_user_id, "provider_user_id")?;
    let mut rows = conn
        .query(
            "SELECT user_id FROM __kdb_identity_providers
             WHERE provider = ? AND provider_user_id = ?
               AND (? IS NULL OR user_id = ?)
             LIMIT 1",
            libsql::params![
                provider.clone(),
                provider_user_id.clone(),
                to_sql_nullable_text(user_id.clone()),
                to_sql_nullable_text(user_id.clone())
            ],
        )
        .await
        .map_err(|e| AppError::Internal(format!("user_unlink_provider lookup failed: {e}")))?;
    let linked_user_id = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("user_unlink_provider row read failed: {e}")))?
        .map(|row| {
            row.get::<String>(0)
                .map_err(|e| AppError::Internal(format!("user_unlink_provider decode failed: {e}")))
        })
        .transpose()?;
    drop(rows);

    let deleted = conn
        .execute(
            "DELETE FROM __kdb_identity_providers
             WHERE provider = ? AND provider_user_id = ?
               AND (? IS NULL OR user_id = ?)",
            libsql::params![
                provider.clone(),
                provider_user_id.clone(),
                to_sql_nullable_text(user_id.clone()),
                to_sql_nullable_text(user_id)
            ],
        )
        .await
        .map_err(|e| AppError::Internal(format!("user_unlink_provider failed: {e}")))?;
    if let Some(linked_user_id) = linked_user_id.as_deref() {
        log_identity_event(
            conn,
            Some(linked_user_id),
            "user.provider_unlinked",
            json!({"provider": provider, "provider_user_id": provider_user_id}),
        )
        .await?;
    }
    Ok(GatewayResponse::ok(Some(json!({
        "unlinked": deleted,
        "user_id": linked_user_id,
        "provider": provider,
        "provider_user_id": provider_user_id
    }))))
}

async fn link_identity_provider(
    state: &AppState,
    conn: &libsql::Connection,
    user_id: &str,
    provider: Option<String>,
    provider_user_id: Option<String>,
    email: Option<String>,
    data: Value,
) -> AppResult<String> {
    let provider = required_text(provider, "provider")?;
    let provider_user_id = required_text(provider_user_id, "provider_user_id")?;
    ensure_identity_user_exists(conn, user_id).await?;
    let provider_id = Uuid::new_v4().simple().to_string();
    let json_expr = json_input_expr(state.jsonb_enabled);
    conn.execute(
        &format!(
            "INSERT INTO __kdb_identity_providers (id, user_id, provider, provider_user_id, email, data)
             VALUES (?, ?, ?, ?, ?, {json_expr})"
        ),
        libsql::params![
            provider_id.clone(),
            user_id.to_string(),
            provider,
            provider_user_id,
            to_sql_nullable_text(clean_optional(email)),
            data.to_string()
        ],
    )
    .await
    .map_err(|e| AppError::BadRequest(format!("user_link_provider failed: {e}")))?;
    Ok(provider_id)
}

async fn ensure_identity_user_exists(conn: &libsql::Connection, user_id: &str) -> AppResult<()> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM __kdb_identity_users WHERE id = ? LIMIT 1",
            libsql::params![user_id.to_string()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("identity user existence check failed: {e}")))?;
    if rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("identity user existence row failed: {e}")))?
        .is_some()
    {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!("user not found: {user_id}")))
    }
}

async fn user_create_token(
    state: &AppState,
    conn: &libsql::Connection,
    req: GatewayRequest,
) -> AppResult<GatewayResponse> {
    let payload = req.payload;
    let user_id = required_text(payload.user_id.or(payload.id), "user_id")?;
    let kind = required_text(payload.kind, "kind")?;
    let token_hash = required_text(payload.token_hash, "token_hash")?;
    let expires_at = normalize_future_time(payload.expires_at, payload.expires_in, "token")?;
    let allow_multi = payload.allow_multi.unwrap_or(false);
    let data = payload.data.unwrap_or_else(|| json!({}));
    if !data.is_object() {
        return Err(AppError::BadRequest("data must be an object when provided".to_string()));
    }
    let token_id = Uuid::new_v4().simple().to_string();
    let tx = conn
        .transaction()
        .await
        .map_err(|e| AppError::Internal(format!("user_create_token tx begin failed: {e}")))?;
    if !allow_multi {
        tx.execute(
            "UPDATE __kdb_identity_tokens
             SET revoked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE user_id = ? AND kind = ? AND used_at IS NULL AND revoked_at IS NULL",
            libsql::params![user_id.clone(), kind.clone()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("user_create_token revoke previous failed: {e}")))?;
    }
    let json_expr = json_input_expr(state.jsonb_enabled);
    tx.execute(
        &format!(
            "INSERT INTO __kdb_identity_tokens (id, user_id, kind, token_hash, expires_at, data)
             VALUES (?, ?, ?, ?, ?, {json_expr})"
        ),
        libsql::params![
            token_id.clone(),
            user_id.clone(),
            kind.clone(),
            token_hash,
            to_sql_nullable_text(expires_at.clone()),
            data.to_string()
        ],
    )
    .await
    .map_err(|e| AppError::BadRequest(format!("user_create_token failed: {e}")))?;
    tx.commit()
        .await
        .map_err(|e| AppError::Internal(format!("user_create_token tx commit failed: {e}")))?;
    log_identity_event(
        conn,
        Some(&user_id),
        "user.token_created",
        json!({"token_id": token_id, "kind": kind, "expires_at": expires_at, "allow_multi": allow_multi}),
    )
    .await?;
    Ok(GatewayResponse::ok(Some(json!({
        "token_id": token_id,
        "user_id": user_id,
        "kind": kind,
        "expires_at": expires_at,
        "allow_multi": allow_multi
    }))))
}

async fn fetch_identity_user(conn: &libsql::Connection, user_id: &str) -> AppResult<Value> {
    let mut rows = conn
        .query(
            &format!("SELECT {} FROM __kdb_identity_users WHERE id = ? LIMIT 1", identity_user_select()),
            libsql::params![user_id.to_string()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("identity user fetch failed: {e}")))?;
    let row = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("identity user row read failed: {e}")))?
        .ok_or_else(|| AppError::BadRequest(format!("user not found: {user_id}")))?;
    identity_user_from_row(&row)
}

async fn identity_count(
    conn: &libsql::Connection,
    where_clause: &str,
    binds: Vec<libsql::Value>,
) -> AppResult<i64> {
    let mut rows = conn
        .query(
            &format!("SELECT COUNT(*) FROM __kdb_identity_users WHERE {where_clause}"),
            binds,
        )
        .await
        .map_err(|e| AppError::Internal(format!("user_list count failed: {e}")))?;
    if let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("user_list count row failed: {e}")))?
    {
        row.get::<i64>(0)
            .map_err(|e| AppError::Internal(format!("user_list count decode failed: {e}")))
    } else {
        Ok(0)
    }
}

async fn user_id_by_field(conn: &libsql::Connection, field: &str, value: &str) -> AppResult<String> {
    let sql = match field {
        "email" => "SELECT id FROM __kdb_identity_users WHERE email = ? LIMIT 1",
        "username" => "SELECT id FROM __kdb_identity_users WHERE username = ? LIMIT 1",
        _ => return Err(AppError::BadRequest("unsupported identity selector".to_string())),
    };
    let mut rows = conn
        .query(sql, libsql::params![value.to_string()])
        .await
        .map_err(|e| AppError::Internal(format!("identity lookup failed: {e}")))?;
    let row = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("identity lookup row failed: {e}")))?
        .ok_or_else(|| AppError::BadRequest("user not found".to_string()))?;
    row.get::<String>(0)
        .map_err(|e| AppError::Internal(format!("identity lookup decode failed: {e}")))
}

async fn enrich_identity_user(conn: &libsql::Connection, user: &mut Value) -> AppResult<()> {
    let Some(user_id) = user.get("id").and_then(Value::as_str).map(str::to_string) else {
        return Ok(());
    };
    let providers = fetch_identity_providers(conn, &user_id).await?;
    let mut methods = Vec::<Value>::new();
    if user.get("password_algo").and_then(Value::as_str).is_some() {
        methods.push(Value::String("password".to_string()));
    }
    for provider in &providers {
        if let Some(name) = provider.get("provider").and_then(Value::as_str) {
            methods.push(Value::String(name.to_string()));
        }
    }
    methods.sort_by(|a, b| a.as_str().unwrap_or("").cmp(b.as_str().unwrap_or("")));
    methods.dedup();
    let email_verified = user.get("email_verified_at").and_then(Value::as_str).is_some();
    if let Some(obj) = user.as_object_mut() {
        obj.insert("login_methods".to_string(), Value::Array(methods));
        obj.insert("email_verified".to_string(), Value::Bool(email_verified));
        obj.insert("provider_count".to_string(), Value::from(providers.len()));
    }
    Ok(())
}

async fn fetch_identity_providers(conn: &libsql::Connection, user_id: &str) -> AppResult<Vec<Value>> {
    let mut rows = conn
        .query(
            "SELECT id, user_id, provider, provider_user_id, email, json(data), created_at, updated_at
             FROM __kdb_identity_providers
             WHERE user_id = ?
             ORDER BY created_at DESC",
            libsql::params![user_id.to_string()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("identity providers query failed: {e}")))?;
    let mut items = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("identity provider row failed: {e}")))?
    {
        let raw_data: Option<String> = row
            .get(5)
            .map_err(|e| AppError::Internal(format!("identity provider data decode failed: {e}")))?;
        items.push(json!({
            "id": row.get::<String>(0).map_err(|e| AppError::Internal(format!("identity provider decode failed: {e}")))?,
            "user_id": row.get::<String>(1).map_err(|e| AppError::Internal(format!("identity provider decode failed: {e}")))?,
            "provider": row.get::<String>(2).map_err(|e| AppError::Internal(format!("identity provider decode failed: {e}")))?,
            "provider_user_id": row.get::<String>(3).map_err(|e| AppError::Internal(format!("identity provider decode failed: {e}")))?,
            "email": row.get::<Option<String>>(4).map_err(|e| AppError::Internal(format!("identity provider decode failed: {e}")))?,
            "data": raw_data.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()).unwrap_or_else(|| json!({})),
            "created_at": row.get::<String>(6).map_err(|e| AppError::Internal(format!("identity provider decode failed: {e}")))?,
            "updated_at": row.get::<String>(7).map_err(|e| AppError::Internal(format!("identity provider decode failed: {e}")))?
        }));
    }
    Ok(items)
}

async fn fetch_identity_events(conn: &libsql::Connection, user_id: &str, limit: i64) -> AppResult<Vec<Value>> {
    let mut rows = conn
        .query(
            "SELECT id, user_id, event, json(data), created_at
             FROM __kdb_identity_events
             WHERE user_id = ?
             ORDER BY created_at DESC
             LIMIT ?",
            libsql::params![user_id.to_string(), limit],
        )
        .await
        .map_err(|e| AppError::Internal(format!("identity events query failed: {e}")))?;
    let mut items = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("identity event row failed: {e}")))?
    {
        let raw_data: Option<String> = row
            .get(3)
            .map_err(|e| AppError::Internal(format!("identity event data decode failed: {e}")))?;
        items.push(json!({
            "id": row.get::<String>(0).map_err(|e| AppError::Internal(format!("identity event decode failed: {e}")))?,
            "user_id": row.get::<Option<String>>(1).map_err(|e| AppError::Internal(format!("identity event decode failed: {e}")))?,
            "event": row.get::<String>(2).map_err(|e| AppError::Internal(format!("identity event decode failed: {e}")))?,
            "data": raw_data.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()).unwrap_or_else(|| json!({})),
            "created_at": row.get::<String>(4).map_err(|e| AppError::Internal(format!("identity event decode failed: {e}")))?
        }));
    }
    Ok(items)
}

fn identity_user_select() -> &'static str {
    "id, email, username, phone, first_name, last_name, profile_photo, status, status_reason, status_changed_at, status_expires_at,
     status_next, status_next_reason, status_changed_by, password_algo, password_updated_at,
     requires_password_change, email_verified_at, phone_verified_at, last_login_at, json(data), created_at, updated_at, deleted_at"
}

fn identity_user_select_with_alias(alias: &str) -> String {
    identity_user_select()
        .split(',')
        .map(|part| {
            let part = part.trim();
            if part == "json(data)" {
                format!("json({alias}.data)")
            } else {
                format!("{alias}.{part}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn identity_user_from_row(row: &libsql::Row) -> AppResult<Value> {
    let raw_data: Option<String> = row
        .get(20)
        .map_err(|e| AppError::Internal(format!("identity user data decode failed: {e}")))?;
    let data = raw_data
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or_else(|| json!({}));
    Ok(json!({
        "id": row.get::<String>(0).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "email": row.get::<Option<String>>(1).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "username": row.get::<Option<String>>(2).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "phone": row.get::<Option<String>>(3).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "first_name": row.get::<Option<String>>(4).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "last_name": row.get::<Option<String>>(5).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "profile_photo": row.get::<Option<String>>(6).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "status": row.get::<String>(7).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "status_reason": row.get::<Option<String>>(8).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "status_changed_at": row.get::<String>(9).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "status_expires_at": row.get::<Option<String>>(10).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "status_next": row.get::<Option<String>>(11).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "status_next_reason": row.get::<Option<String>>(12).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "status_changed_by": row.get::<Option<String>>(13).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "password_algo": row.get::<Option<String>>(14).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "password_updated_at": row.get::<Option<String>>(15).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "requires_password_change": row.get::<i64>(16).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))? != 0,
        "email_verified_at": row.get::<Option<String>>(17).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "phone_verified_at": row.get::<Option<String>>(18).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "last_login_at": row.get::<Option<String>>(19).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "data": data,
        "created_at": row.get::<String>(21).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "updated_at": row.get::<String>(22).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?,
        "deleted_at": row.get::<Option<String>>(23).map_err(|e| AppError::Internal(format!("identity user decode failed: {e}")))?
    }))
}

async fn log_identity_event(
    conn: &libsql::Connection,
    user_id: Option<&str>,
    event: &str,
    data: Value,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO __kdb_identity_events (id, user_id, event, data)
         VALUES (?, ?, ?, json(?))",
        libsql::params![
            Uuid::new_v4().simple().to_string(),
            to_sql_nullable_text(user_id.map(str::to_string)),
            event.to_string(),
            data.to_string()
        ],
    )
    .await
    .map_err(|e| AppError::Internal(format!("identity event insert failed: {e}")))?;
    Ok(())
}

fn normalize_future_time(value: Option<String>, seconds: Option<i64>, label: &str) -> AppResult<Option<String>> {
    match (clean_optional(value), seconds) {
        (Some(_), Some(_)) => Err(AppError::BadRequest(format!("{label}_expires_at and {label}_expires_in cannot both be provided"))),
        (Some(raw), None) => Ok(Some(normalize_rfc3339_utc(&raw)?)),
        (None, Some(secs)) => {
            if secs <= 0 {
                return Err(AppError::BadRequest(format!("{label}_expires_in must be greater than 0")));
            }
            Ok(Some((Utc::now() + Duration::seconds(secs)).to_rfc3339_opts(SecondsFormat::Millis, true)))
        }
        (None, None) => Ok(None),
    }
}

fn normalize_rfc3339_utc(raw: &str) -> AppResult<String> {
    let dt = chrono::DateTime::parse_from_rfc3339(raw)
        .map_err(|_| AppError::BadRequest("datetime must be RFC3339".to_string()))?;
    Ok(dt.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn normalize_optional_id(id: Option<String>) -> AppResult<Option<String>> {
    let Some(id) = clean_optional(id) else {
        return Ok(None);
    };
    if id.chars().all(|c| c.is_ascii_hexdigit()) && id.len() == 32 {
        Ok(Some(id.to_ascii_lowercase()))
    } else {
        Err(AppError::BadRequest("user_id must be a 32-character dashless uuid string".to_string()))
    }
}

fn required_text(value: Option<String>, name: &str) -> AppResult<String> {
    clean_optional(value).ok_or_else(|| AppError::BadRequest(format!("{name} is required")))
}

fn non_empty_or(value: Option<String>, name: &str, default: &str) -> AppResult<String> {
    match clean_optional(value) {
        Some(v) => Ok(v),
        None if !default.is_empty() => Ok(default.to_string()),
        None => Err(AppError::BadRequest(format!("{name} is required"))),
    }
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}
