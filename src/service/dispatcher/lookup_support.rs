// Lookup planning and execution helpers extracted from dispatcher.rs.

#[derive(Clone, Debug)]
struct LookupSpec {
    from: String,
    local_field: String,
    foreign_field: String,
    match_op: String,
    multi: bool,
    filter: Option<Value>,
    fields: Option<Vec<String>>,
    sort: Option<Value>,
    limit: Option<i64>,
    preserve_order: bool,
    dedupe: bool,
    on_missing: String,
    strict_path: bool,
    cache_lookup: bool,
    lookups: Option<Value>,
}

#[derive(Clone, Debug)]
struct LookupResolution {
    value: Value,
    drop_parent: bool,
}

fn resolve_lookup_max_depth(state: &AppState, override_value: Option<i64>) -> AppResult<usize> {
    match override_value {
        None => Ok(state.query_lookup_max_depth),
        Some(v) if v <= 0 => Err(AppError::BadRequest(
            "lookup_depth_override must be positive".to_string(),
        )),
        Some(v) => {
            let requested = v as usize;
            if state.query_lookup_uncapped_override_enabled {
                Ok(requested)
            } else if requested > state.query_lookup_max_depth {
                Err(AppError::BadRequest(format!(
                    "lookup_depth_override exceeds max depth {}",
                    state.query_lookup_max_depth
                )))
            } else {
                Ok(requested)
            }
        }
    }
}

#[async_recursion::async_recursion]
async fn execute_lookup_scope(
    state: &AppState,
    conn: &libsql::Connection,
    docs: &mut Vec<Value>,
    roots: &mut Vec<Value>,
    mut parents: Option<&mut Vec<Value>>,
    lookups: &Value,
    depth: usize,
    max_depth: usize,
) -> AppResult<()> {
    if docs.is_empty() {
        return Ok(());
    }
    if depth > max_depth {
        return Err(AppError::BadRequest(format!(
            "lookup depth {} exceeds max depth {}",
            depth, max_depth
        )));
    }

    let specs = parse_lookup_specs(lookups, state.query_lookup_default_limit as i64)?;
    if specs.is_empty() {
        return Ok(());
    }

    let levels = lookup_topo_levels(&specs)?;
    let mut parents_owned = parents.as_ref().map(|p| (**p).clone());
    let semaphore = std::sync::Arc::new(Semaphore::new(state.query_lookup_max_concurrency));

    for level in levels {
        let docs_snapshot = docs.clone();
        let roots_snapshot = roots.clone();
        let parents_snapshot = parents_owned.clone();
        let state_snapshot = state.clone();
        let mut join_set = JoinSet::new();
        for alias in level {
            let Some(spec) = specs.get(&alias).cloned() else {
                continue;
            };
            let conn_owned = conn.clone();
            let docs_owned = docs_snapshot.clone();
            let roots_owned = roots_snapshot.clone();
            let parents_owned_level = parents_snapshot.clone();
            let sem = semaphore.clone();
            let state_owned = state_snapshot.clone();
            join_set.spawn(async move {
                let _permit = sem
                    .acquire_owned()
                    .await
                    .map_err(|_| AppError::Internal("lookup semaphore closed".to_string()))?;
                let results = compute_lookup_for_docs(
                    &conn_owned,
                    &docs_owned,
                    &roots_owned,
                    parents_owned_level.as_ref(),
                    &spec,
                    depth,
                    max_depth,
                    &state_owned,
                )
                .await?;
                Ok::<(String, Vec<LookupResolution>), AppError>((alias, results))
            });
        }

        let mut level_results = Vec::<(String, Vec<LookupResolution>)>::new();
        while let Some(task) = join_set.join_next().await {
            let output =
                task.map_err(|e| AppError::Internal(format!("lookup task join failed: {e}")))?;
            level_results.push(output?);
        }

        for (alias, results) in level_results {
            let mut drop_mask = vec![false; docs.len()];
            if results.len() != docs.len() {
                return Err(AppError::Internal(format!(
                    "lookup result size mismatch for alias {alias}"
                )));
            }
            for (idx, res) in results.into_iter().enumerate() {
                if res.drop_parent {
                    drop_mask[idx] = true;
                    continue;
                }
                let obj = docs[idx].as_object_mut().ok_or_else(|| {
                    AppError::Internal("lookup merge target must be object".to_string())
                })?;
                obj.insert(alias.clone(), res.value);
            }
            if drop_mask.iter().any(|d| *d) {
                let mut new_docs = Vec::with_capacity(docs.len());
                let mut new_roots = Vec::with_capacity(roots.len());
                let mut new_parents = parents_owned
                    .as_ref()
                    .map(|p| Vec::with_capacity(p.len()))
                    .unwrap_or_default();
                for idx in 0..docs.len() {
                    if drop_mask[idx] {
                        continue;
                    }
                    new_docs.push(docs[idx].clone());
                    new_roots.push(roots[idx].clone());
                    if let Some(ref p) = parents_owned {
                        new_parents.push(p[idx].clone());
                    }
                }
                *docs = new_docs;
                *roots = new_roots;
                if parents_owned.is_some() {
                    parents_owned = Some(new_parents);
                }
            }
        }
    }

    if let Some(parents_vec) = parents.as_deref_mut() {
        if let Some(updated) = parents_owned {
            *parents_vec = updated;
        }
    }
    Ok(())
}

async fn compute_lookup_for_docs(
    conn: &libsql::Connection,
    docs: &[Value],
    roots: &[Value],
    parents: Option<&Vec<Value>>,
    spec: &LookupSpec,
    depth: usize,
    max_depth: usize,
    state: &AppState,
) -> AppResult<Vec<LookupResolution>> {
    let mut out = Vec::with_capacity(docs.len());
    let mut request_cache = HashMap::<String, Vec<Value>>::new();
    for idx in 0..docs.len() {
        let self_doc = &docs[idx];
        let root_doc = &roots[idx];
        let parent_doc = parents.and_then(|p| p.get(idx));
        let resolution = resolve_lookup_for_doc(
            conn,
            self_doc,
            root_doc,
            parent_doc,
            spec,
            depth,
            max_depth,
            state,
            &mut request_cache,
        )
        .await?;
        out.push(resolution);
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
async fn resolve_lookup_for_doc(
    conn: &libsql::Connection,
    self_doc: &Value,
    root_doc: &Value,
    parent_doc: Option<&Value>,
    spec: &LookupSpec,
    depth: usize,
    max_depth: usize,
    state: &AppState,
    request_cache: &mut HashMap<String, Vec<Value>>,
) -> AppResult<LookupResolution> {
    let local_values = resolve_expr_values(
        &spec.local_field,
        self_doc,
        root_doc,
        parent_doc,
        spec.strict_path,
    )?;
    if local_values.is_empty() {
        return Ok(missing_lookup_resolution(spec));
    }

    let normalized_filter = normalize_lookup_filter(
        spec.filter.clone(),
        self_doc,
        root_doc,
        parent_doc,
        spec.strict_path,
    )?;
    let cache_key = if spec.cache_lookup {
        Some(format!(
            "{}|{}|{}|{}|{}|{}",
            spec.from,
            spec.foreign_field,
            spec.match_op,
            serde_json::to_string(&local_values).unwrap_or_default(),
            serde_json::to_string(&normalized_filter).unwrap_or_default(),
            serde_json::to_string(&spec.sort).unwrap_or_default()
        ))
    } else {
        None
    };

    let mut candidates = if let Some(key) = cache_key.as_ref() {
        if let Some(cached) = request_cache.get(key) {
            cached.clone()
        } else {
            let fetched = fetch_lookup_candidates(
                conn,
                spec,
                state.response_include_system_timestamps,
                normalized_filter,
            )
            .await?;
            request_cache.insert(key.clone(), fetched.clone());
            fetched
        }
    } else {
        fetch_lookup_candidates(
            conn,
            spec,
            state.response_include_system_timestamps,
            normalized_filter,
        )
        .await?
    };

    candidates.retain(|doc| lookup_match_doc(doc, spec, &local_values));
    if candidates.is_empty() {
        return Ok(missing_lookup_resolution(spec));
    }

    if spec.dedupe {
        dedupe_docs_by_id(&mut candidates);
    }
    if spec.preserve_order && spec.match_op == "$in" {
        preserve_lookup_order(&mut candidates, &local_values, &spec.foreign_field);
    }
    if let Some(limit) = spec.limit {
        if limit >= 0 && (candidates.len() as i64) > limit {
            candidates.truncate(limit as usize);
        }
    }

    if let Some(nested) = spec.lookups.as_ref() {
        if depth + 1 > max_depth {
            return Err(AppError::BadRequest(format!(
                "nested lookup depth {} exceeds max depth {}",
                depth + 1,
                max_depth
            )));
        }
        let mut nested_docs = candidates.clone();
        let mut nested_roots = vec![root_doc.clone(); nested_docs.len()];
        let mut nested_parents = vec![self_doc.clone(); nested_docs.len()];
        execute_lookup_scope(
            state,
            conn,
            &mut nested_docs,
            &mut nested_roots,
            Some(&mut nested_parents),
            nested,
            depth + 1,
            max_depth,
        )
        .await?;
        candidates = nested_docs;
    }

    if spec.multi {
        Ok(LookupResolution {
            value: Value::Array(candidates),
            drop_parent: false,
        })
    } else {
        let first = candidates.into_iter().next().unwrap_or(Value::Null);
        Ok(LookupResolution {
            value: first,
            drop_parent: false,
        })
    }
}

fn missing_lookup_resolution(spec: &LookupSpec) -> LookupResolution {
    match spec.on_missing.as_str() {
        "drop" => LookupResolution {
            value: Value::Null,
            drop_parent: true,
        },
        "empty" => LookupResolution {
            value: if spec.multi {
                Value::Array(vec![])
            } else {
                Value::Null
            },
            drop_parent: false,
        },
        _ => LookupResolution {
            value: Value::Null,
            drop_parent: false,
        },
    }
}

fn parse_lookup_specs(
    lookups: &Value,
    default_limit: i64,
) -> AppResult<BTreeMap<String, LookupSpec>> {
    let obj = lookups
        .as_object()
        .ok_or_else(|| AppError::BadRequest("lookups must be an object map".to_string()))?;
    let mut out = BTreeMap::<String, LookupSpec>::new();
    for (alias, raw) in obj {
        let spec_obj = raw.as_object().ok_or_else(|| {
            AppError::BadRequest(format!("lookup '{alias}' spec must be an object"))
        })?;
        let from = spec_obj
            .get("from")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| AppError::BadRequest(format!("lookup '{alias}' missing from")))?;
        let local_field = spec_obj
            .get("local_field")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| AppError::BadRequest(format!("lookup '{alias}' missing local_field")))?;
        let foreign_field = spec_obj
            .get("foreign_field")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                AppError::BadRequest(format!("lookup '{alias}' missing foreign_field"))
            })?;
        let match_op = spec_obj
            .get("match")
            .and_then(Value::as_str)
            .unwrap_or("$eq")
            .to_string();
        if !matches!(match_op.as_str(), "$eq" | "$in" | "$contains" | "$overlap") {
            return Err(AppError::BadRequest(format!(
                "lookup '{alias}' match must be one of: $eq, $in, $contains, $overlap"
            )));
        }
        let multi = spec_obj
            .get("multi")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let limit = spec_obj
            .get("limit")
            .and_then(Value::as_i64)
            .unwrap_or(default_limit);
        let fields = spec_obj
            .get("fields")
            .map(|v| {
                v.as_array()
                    .ok_or_else(|| {
                        AppError::BadRequest(format!("lookup '{alias}' fields must be array"))
                    })?
                    .iter()
                    .map(|x| {
                        x.as_str().map(|s| s.to_string()).ok_or_else(|| {
                            AppError::BadRequest(format!(
                                "lookup '{alias}' fields entries must be strings"
                            ))
                        })
                    })
                    .collect::<AppResult<Vec<String>>>()
            })
            .transpose()?;

        let on_missing = spec_obj
            .get("on_missing")
            .and_then(Value::as_str)
            .unwrap_or("null")
            .to_ascii_lowercase();
        if !matches!(on_missing.as_str(), "null" | "empty" | "drop") {
            return Err(AppError::BadRequest(format!(
                "lookup '{alias}' on_missing must be one of: null, empty, drop"
            )));
        }

        out.insert(
            alias.clone(),
            LookupSpec {
                from: from.to_string(),
                local_field: local_field.to_string(),
                foreign_field: foreign_field.to_string(),
                match_op,
                multi,
                filter: spec_obj.get("filter").cloned(),
                fields,
                sort: spec_obj.get("sort").cloned(),
                limit: Some(limit),
                preserve_order: spec_obj
                    .get("preserve_order")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                dedupe: spec_obj
                    .get("dedupe")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                on_missing,
                strict_path: spec_obj
                    .get("strict_path")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                cache_lookup: spec_obj
                    .get("cache_lookup")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                lookups: spec_obj.get("lookups").cloned(),
            },
        );
    }
    Ok(out)
}

fn lookup_topo_levels(specs: &BTreeMap<String, LookupSpec>) -> AppResult<Vec<Vec<String>>> {
    let mut graph = DiGraph::<String, ()>::new();
    let mut idx = HashMap::new();
    for alias in specs.keys() {
        let node = graph.add_node(alias.clone());
        idx.insert(alias.clone(), node);
    }

    for (alias, spec) in specs {
        let mut deps = HashSet::new();
        collect_lookup_alias_refs(&Value::String(spec.local_field.clone()), &mut deps);
        if let Some(filter) = &spec.filter {
            collect_lookup_alias_refs(filter, &mut deps);
        }
        for dep in deps {
            if dep == *alias {
                return Err(AppError::BadRequest(format!(
                    "lookup '{alias}' cannot depend on itself"
                )));
            }
            let dep_idx = idx.get(&dep).ok_or_else(|| {
                AppError::BadRequest(format!("lookup '{alias}' references unknown alias '{dep}'"))
            })?;
            let alias_idx = idx.get(alias).expect("node exists");
            graph.add_edge(*dep_idx, *alias_idx, ());
        }
    }

    let order = toposort(&graph, None)
        .map_err(|_| AppError::BadRequest("lookup dependency cycle detected".to_string()))?;
    let mut level_map = HashMap::<String, usize>::new();
    for node in order {
        let alias = graph
            .node_weight(node)
            .ok_or_else(|| AppError::Internal("lookup topo node missing".to_string()))?
            .clone();
        let mut max_parent = 0usize;
        for edge in graph.edges_directed(node, petgraph::Direction::Incoming) {
            let parent_alias = graph
                .node_weight(edge.source())
                .ok_or_else(|| AppError::Internal("lookup topo edge source missing".to_string()))?;
            let p = level_map.get(parent_alias).copied().unwrap_or(0);
            if p + 1 > max_parent {
                max_parent = p + 1;
            }
        }
        level_map.insert(alias, max_parent);
    }

    let mut grouped: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for alias in specs.keys() {
        let lvl = level_map.get(alias).copied().unwrap_or(0);
        grouped.entry(lvl).or_default().push(alias.clone());
    }
    Ok(grouped.into_values().collect())
}

fn collect_lookup_alias_refs(value: &Value, out: &mut HashSet<String>) {
    match value {
        Value::String(s) => {
            if let Some(rest) = s.strip_prefix("$lookup.") {
                if let Some(alias) = rest.split(['.', '[']).next() {
                    if !alias.trim().is_empty() {
                        out.insert(alias.to_string());
                    }
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_lookup_alias_refs(item, out);
            }
        }
        Value::Object(obj) => {
            for (k, v) in obj {
                collect_lookup_alias_refs(&Value::String(k.clone()), out);
                collect_lookup_alias_refs(v, out);
            }
        }
        _ => {}
    }
}

fn normalize_lookup_filter(
    filter: Option<Value>,
    self_doc: &Value,
    root_doc: &Value,
    parent_doc: Option<&Value>,
    strict_path: bool,
) -> AppResult<Option<Value>> {
    let Some(mut filter) = filter else {
        return Ok(None);
    };
    resolve_lookup_filter_tokens(&mut filter, self_doc, root_doc, parent_doc, strict_path)?;
    Ok(Some(filter))
}

fn resolve_lookup_filter_tokens(
    value: &mut Value,
    self_doc: &Value,
    root_doc: &Value,
    parent_doc: Option<&Value>,
    strict_path: bool,
) -> AppResult<()> {
    match value {
        Value::String(s) if s.starts_with('$') => {
            let vals = resolve_expr_values(s, self_doc, root_doc, parent_doc, strict_path)?;
            *value = if vals.is_empty() {
                Value::Null
            } else if vals.len() == 1 {
                vals[0].clone()
            } else {
                Value::Array(vals)
            };
            Ok(())
        }
        Value::Array(arr) => {
            for item in arr {
                resolve_lookup_filter_tokens(item, self_doc, root_doc, parent_doc, strict_path)?;
            }
            Ok(())
        }
        Value::Object(obj) => {
            for v in obj.values_mut() {
                resolve_lookup_filter_tokens(v, self_doc, root_doc, parent_doc, strict_path)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

async fn fetch_lookup_candidates(
    conn: &libsql::Connection,
    spec: &LookupSpec,
    include_system_timestamps: bool,
    filter: Option<Value>,
) -> AppResult<Vec<Value>> {
    let (where_clause, binds) =
        build_where_with_collection(filter.unwrap_or_else(|| json!({})), Some(spec.from.clone()))?;
    let order_by = build_order_by(&spec.sort)?;
    let sql = if include_system_timestamps {
        format!(
            "SELECT json(data), _created_at, _modified_at FROM __kdb_documents WHERE {where_clause} ORDER BY {order_by}"
        )
    } else {
        format!("SELECT json(data) FROM __kdb_documents WHERE {where_clause} ORDER BY {order_by}")
    };
    let mut rows = conn
        .query(&sql, binds)
        .await
        .map_err(|e| AppError::Internal(format!("lookup query failed: {e}")))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("lookup row read failed: {e}")))?
    {
        let raw: String = row
            .get(0)
            .map_err(|e| AppError::Internal(format!("lookup row decode failed: {e}")))?;
        let mut value = serde_json::from_str::<Value>(&raw)
            .map_err(|e| AppError::Internal(format!("lookup json decode failed: {e}")))?;
        if include_system_timestamps {
            let created_at: Option<String> = row
                .get(1)
                .map_err(|e| AppError::Internal(format!("lookup created_at decode failed: {e}")))?;
            let modified_at: Option<String> = row.get(2).map_err(|e| {
                AppError::Internal(format!("lookup modified_at decode failed: {e}"))
            })?;
            attach_system_timestamps(&mut value, created_at, modified_at);
        }
        if let Some(fields) = &spec.fields {
            value = apply_projection(&value, &Some(fields.clone()), &None)?;
        }
        out.push(value);
    }
    Ok(out)
}

fn lookup_match_doc(doc: &Value, spec: &LookupSpec, local_values: &[Value]) -> bool {
    let foreign_values = extract_values_by_path(doc, &normalize_expr_path(&spec.foreign_field));
    if foreign_values.is_empty() {
        return false;
    }
    match spec.match_op.as_str() {
        "$in" | "$eq" | "$contains" | "$overlap" => foreign_values
            .iter()
            .any(|fv| local_values.iter().any(|lv| lv == fv)),
        _ => false,
    }
}

fn dedupe_docs_by_id(docs: &mut Vec<Value>) {
    let mut seen = HashSet::new();
    docs.retain(|d| {
        if let Some(id) = d.get("_id").and_then(Value::as_str) {
            if !seen.insert(id.to_string()) {
                return false;
            }
        }
        true
    });
}

fn preserve_lookup_order(docs: &mut Vec<Value>, local_values: &[Value], foreign_field: &str) {
    let path = normalize_expr_path(foreign_field);
    let mut rank = HashMap::<String, usize>::new();
    for (i, v) in local_values.iter().enumerate() {
        rank.entry(v.to_string()).or_insert(i);
    }
    docs.sort_by_key(|doc| {
        let fv = extract_values_by_path(doc, &path)
            .into_iter()
            .next()
            .unwrap_or(Value::Null)
            .to_string();
        rank.get(&fv).copied().unwrap_or(usize::MAX)
    });
}

fn resolve_expr_values(
    expr: &str,
    self_doc: &Value,
    root_doc: &Value,
    parent_doc: Option<&Value>,
    strict_path: bool,
) -> AppResult<Vec<Value>> {
    let (base, path) = if let Some(rest) = expr.strip_prefix("$root.") {
        (root_doc, rest.to_string())
    } else if let Some(rest) = expr.strip_prefix("$parent.") {
        let parent = parent_doc.ok_or_else(|| {
            AppError::BadRequest(format!("path '{expr}' requires parent context"))
        })?;
        (parent, rest.to_string())
    } else if let Some(rest) = expr.strip_prefix("$self.") {
        (self_doc, rest.to_string())
    } else if let Some(rest) = expr.strip_prefix("$lookup.") {
        (self_doc, rest.to_string())
    } else {
        (self_doc, expr.to_string())
    };
    let values = extract_values_by_path(base, &path);
    if values.is_empty() && strict_path {
        return Err(AppError::BadRequest(format!(
            "strict_path missing value for '{expr}'"
        )));
    }
    Ok(values)
}

fn normalize_expr_path(path: &str) -> String {
    path.strip_prefix("$self.")
        .or_else(|| path.strip_prefix("$root."))
        .or_else(|| path.strip_prefix("$parent."))
        .or_else(|| path.strip_prefix("$lookup."))
        .unwrap_or(path)
        .to_string()
}

fn extract_values_by_path(base: &Value, path: &str) -> Vec<Value> {
    if path.trim().is_empty() {
        return vec![base.clone()];
    }
    let mut current = vec![base.clone()];
    for part in path.split('.') {
        if part.is_empty() {
            continue;
        }
        let is_array = part.ends_with("[]");
        let field = part.strip_suffix("[]").unwrap_or(part);
        let mut next = Vec::new();
        for node in current {
            if let Some(v) = node.get(field) {
                if is_array {
                    if let Some(arr) = v.as_array() {
                        next.extend(arr.iter().cloned());
                    }
                } else {
                    next.push(v.clone());
                }
            }
        }
        current = next;
        if current.is_empty() {
            break;
        }
    }
    current
}

fn observed_query_paths(payload: &OperationPayload) -> Vec<String> {
    let mut out = HashSet::<String>::new();
    if let Some(filter) = payload.filter.as_ref() {
        collect_filter_paths(filter, &mut out);
    }
    collect_sort_paths(payload.sort.as_ref(), &mut out);
    let mut v = out.into_iter().collect::<Vec<_>>();
    v.sort();
    v
}

fn collect_filter_paths(node: &Value, out: &mut HashSet<String>) {
    let Some(obj) = node.as_object() else {
        return;
    };
    for (k, v) in obj {
        if k.starts_with('$') {
            match k.as_str() {
                "$and" | "$or" | "$nor" => {
                    if let Some(arr) = v.as_array() {
                        for item in arr {
                            collect_filter_paths(item, out);
                        }
                    }
                }
                "$not" => collect_filter_paths(v, out),
                _ => {}
            }
            continue;
        }
        if is_observable_path(k) {
            out.insert(k.clone());
        }
        if let Some(op_obj) = v.as_object() {
            if let Some(elem_match) = op_obj.get("$elemMatch") {
                collect_filter_paths(elem_match, out);
            }
        }
    }
}

fn collect_sort_paths(sort: Option<&Value>, out: &mut HashSet<String>) {
    let Some(sort) = sort else {
        return;
    };
    if let Some(obj) = sort.as_object() {
        for k in obj.keys() {
            if is_observable_path(k) {
                out.insert(k.clone());
            }
        }
        return;
    }
    if let Some(s) = sort.as_str() {
        for part in s.split(',') {
            let token = part.trim();
            if token.is_empty() {
                continue;
            }
            let path = token.split_whitespace().next().unwrap_or("");
            if is_observable_path(path) {
                out.insert(path.to_string());
            }
        }
    }
}

fn is_observable_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    for seg in path.split('.') {
        if seg.is_empty() {
            return false;
        }
        if !seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return false;
        }
    }
    true
}
