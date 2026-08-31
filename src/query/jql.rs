//! JQL compiler: translates Mongo-style JSON filters into SQLite SQL + bind values.

use serde_json::Value;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct CompiledWhere {
    pub sql: String,
    pub binds: Vec<libsql::Value>,
}

pub fn build_where(filter: &Value) -> AppResult<CompiledWhere> {
    compile_filter_on_base("data", filter, false)
}

pub fn build_where_on_base(base_expr: &str, filter: &Value) -> AppResult<CompiledWhere> {
    compile_filter_on_base(base_expr, filter, false)
}

fn compile_filter_on_base(
    base_expr: &str,
    filter: &Value,
    elem_mode: bool,
) -> AppResult<CompiledWhere> {
    if filter.is_null() {
        return Ok(CompiledWhere {
            sql: "1=1".to_string(),
            binds: vec![],
        });
    }

    let obj = filter
        .as_object()
        .ok_or_else(|| AppError::BadRequest("filter must be an object".to_string()))?;

    if obj.is_empty() {
        return Ok(CompiledWhere {
            sql: "1=1".to_string(),
            binds: vec![],
        });
    }

    let mut parts = Vec::new();
    let mut binds = Vec::new();

    for (key, value) in obj {
        match key.as_str() {
            "$and" | "$or" | "$nor" => {
                let arr = value
                    .as_array()
                    .ok_or_else(|| AppError::BadRequest(format!("{key} must be an array")))?;
                if arr.is_empty() {
                    return Err(AppError::BadRequest(format!("{key} cannot be empty")));
                }

                let mut nested_sql = Vec::new();
                for item in arr {
                    let compiled = compile_filter_on_base(base_expr, item, elem_mode)?;
                    nested_sql.push(format!("({})", compiled.sql));
                    binds.extend(compiled.binds);
                }

                if key == "$nor" {
                    parts.push(format!("NOT ({})", nested_sql.join(" OR ")));
                } else {
                    let joiner = if key == "$and" { " AND " } else { " OR " };
                    parts.push(nested_sql.join(joiner));
                }
            }
            "$not" => {
                let compiled = compile_filter_on_base(base_expr, value, elem_mode)?;
                parts.push(format!("NOT ({})", compiled.sql));
                binds.extend(compiled.binds);
            }
            _ if elem_mode && key.starts_with('$') => {
                let scalar_expr = format!("({base_expr})");
                let array_expr = format!("json_extract({base_expr}, '$')");
                let mut local_parts = Vec::new();
                compile_operator(
                    key,
                    value,
                    &scalar_expr,
                    &array_expr,
                    &mut local_parts,
                    &mut binds,
                    Some(base_expr),
                )?;
                parts.extend(local_parts);
            }
            _ => {
                let compiled = compile_field_expr_on_base(base_expr, key, value)?;
                parts.push(compiled.sql);
                binds.extend(compiled.binds);
            }
        }
    }

    Ok(CompiledWhere {
        sql: parts.join(" AND "),
        binds,
    })
}

fn compile_field_expr_on_base(
    base_expr: &str,
    path: &str,
    value: &Value,
) -> AppResult<CompiledWhere> {
    if path.matches("[]").count() > 1 {
        return compile_nested_array_field_expr(base_expr, path, value);
    }

    if let Some((array_path, remainder)) = split_first_array_path(path)? {
        let element_filter = if remainder.is_empty() {
            if value
                .as_object()
                .is_some_and(|obj| !obj.is_empty() && obj.keys().all(|key| key.starts_with('$')))
            {
                value.clone()
            } else {
                let mut equality = serde_json::Map::new();
                equality.insert("$eq".to_string(), value.clone());
                Value::Object(equality)
            }
        } else {
            let mut nested = serde_json::Map::new();
            nested.insert(remainder, value.clone());
            Value::Object(nested)
        };
        let mut elem_match = serde_json::Map::new();
        elem_match.insert("$elemMatch".to_string(), element_filter);
        return compile_field_expr_on_base(base_expr, &array_path, &Value::Object(elem_match));
    }

    let scalar_expr = json_path_expr(base_expr, path)?;
    let array_expr = json_array_expr(base_expr, path)?;

    let Some(op_obj) = value.as_object() else {
        return Ok(CompiledWhere {
            sql: format!("{} = ?", scalar_expr),
            binds: vec![json_to_sql_value(value)?],
        });
    };

    let mut parts = Vec::new();
    let mut binds = Vec::new();

    for (op, operand) in op_obj {
        compile_operator(
            op,
            operand,
            &scalar_expr,
            &array_expr,
            &mut parts,
            &mut binds,
            Some(base_expr),
        )?;
    }

    if parts.is_empty() {
        return Err(AppError::BadRequest(
            "empty field operator object".to_string(),
        ));
    }

    Ok(CompiledWhere {
        sql: parts.join(" AND "),
        binds,
    })
}

fn compile_nested_array_field_expr(
    base_expr: &str,
    path: &str,
    value: &Value,
) -> AppResult<CompiledWhere> {
    let pattern = array_path_fullkey_pattern(path)?;
    let alias = json_tree_alias(base_expr, path);
    let scalar_expr = format!("{alias}.atom");
    let array_expr = format!("{alias}.value");
    let Some(op_obj) = value.as_object() else {
        return Ok(CompiledWhere {
            sql: format!(
                "EXISTS (SELECT 1 FROM json_tree({base_expr}) {alias} WHERE {alias}.fullkey LIKE ? AND {scalar_expr} = ?)"
            ),
            binds: vec![libsql::Value::Text(pattern), json_to_sql_value(value)?],
        });
    };

    let mut parts = vec![format!("{alias}.fullkey LIKE ?")];
    let mut binds = vec![libsql::Value::Text(pattern)];
    for (op, operand) in op_obj {
        compile_operator(
            op,
            operand,
            &scalar_expr,
            &array_expr,
            &mut parts,
            &mut binds,
            Some(base_expr),
        )?;
    }
    if parts.len() == 1 {
        return Err(AppError::BadRequest(
            "empty field operator object".to_string(),
        ));
    }

    Ok(CompiledWhere {
        sql: format!(
            "EXISTS (SELECT 1 FROM json_tree({base_expr}) {alias} WHERE {})",
            parts.join(" AND ")
        ),
        binds,
    })
}

fn array_path_fullkey_pattern(path: &str) -> AppResult<String> {
    let mut pattern = String::from("$");
    for segment in path.split('.') {
        if let Some(field) = segment.strip_suffix("[]") {
            if field.is_empty() || field.contains(['[', ']']) {
                return Err(AppError::BadRequest(format!(
                    "invalid array filter path segment: {segment}"
                )));
            }
            pattern.push('.');
            pattern.push_str(field);
            pattern.push_str("[%]");
        } else {
            if segment.is_empty()
                || !segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err(AppError::BadRequest("invalid filter path".to_string()));
            }
            pattern.push('.');
            pattern.push_str(segment);
        }
    }
    Ok(pattern)
}

fn split_first_array_path(path: &str) -> AppResult<Option<(String, String)>> {
    let segments = path.split('.').collect::<Vec<_>>();
    if segments.iter().any(|segment| segment.is_empty()) {
        return Err(AppError::BadRequest("invalid filter path".to_string()));
    }

    let mut prefix = Vec::<&str>::new();
    for (index, segment) in segments.iter().enumerate() {
        if let Some(field) = segment.strip_suffix("[]") {
            if field.is_empty() || field.contains('[') || field.contains(']') {
                return Err(AppError::BadRequest(format!(
                    "invalid array filter path segment: {segment}"
                )));
            }
            prefix.push(field);
            return Ok(Some((prefix.join("."), segments[index + 1..].join("."))));
        }
        if segment.contains('[') || segment.contains(']') {
            return Err(AppError::BadRequest(format!(
                "invalid array filter path segment: {segment}; use []"
            )));
        }
        prefix.push(segment);
    }
    Ok(None)
}

fn compile_operator(
    op: &str,
    operand: &Value,
    scalar_expr: &str,
    array_expr: &str,
    parts: &mut Vec<String>,
    binds: &mut Vec<libsql::Value>,
    elem_base_expr: Option<&str>,
) -> AppResult<()> {
    match op {
        "$eq" => {
            parts.push(format!("{} = ?", scalar_expr));
            binds.push(json_to_sql_value(operand)?);
        }
        "$ne" => {
            parts.push(format!("{} != ?", scalar_expr));
            binds.push(json_to_sql_value(operand)?);
        }
        "$gt" => {
            parts.push(format!("{} > ?", scalar_expr));
            binds.push(json_to_sql_value(operand)?);
        }
        "$gte" => {
            parts.push(format!("{} >= ?", scalar_expr));
            binds.push(json_to_sql_value(operand)?);
        }
        "$lt" => {
            parts.push(format!("{} < ?", scalar_expr));
            binds.push(json_to_sql_value(operand)?);
        }
        "$lte" => {
            parts.push(format!("{} <= ?", scalar_expr));
            binds.push(json_to_sql_value(operand)?);
        }
        "$exists" => {
            let exists = operand
                .as_bool()
                .ok_or_else(|| AppError::BadRequest("$exists value must be boolean".to_string()))?;
            if exists {
                parts.push(format!("{} IS NOT NULL", scalar_expr));
            } else {
                parts.push(format!("{} IS NULL", scalar_expr));
            }
        }
        "$startsWith" => {
            let s = expect_string(op, operand)?;
            parts.push(format!("{} LIKE ?", scalar_expr));
            binds.push(libsql::Value::Text(format!("{}%", s)));
        }
        "$endsWith" => {
            let s = expect_string(op, operand)?;
            parts.push(format!("{} LIKE ?", scalar_expr));
            binds.push(libsql::Value::Text(format!("%{}", s)));
        }
        "$contains" => {
            let s = expect_string(op, operand)?;
            parts.push(format!("{} LIKE ?", scalar_expr));
            binds.push(libsql::Value::Text(format!("%{}%", s)));
        }
        "$ilike" => {
            let s = expect_string(op, operand)?;
            parts.push(format!("LOWER({}) LIKE LOWER(?)", scalar_expr));
            binds.push(libsql::Value::Text(s.to_string()));
        }
        "$istartsWith" => {
            let s = expect_string(op, operand)?;
            parts.push(format!("LOWER({}) LIKE LOWER(?)", scalar_expr));
            binds.push(libsql::Value::Text(format!("{}%", s)));
        }
        "$iendsWith" => {
            let s = expect_string(op, operand)?;
            parts.push(format!("LOWER({}) LIKE LOWER(?)", scalar_expr));
            binds.push(libsql::Value::Text(format!("%{}", s)));
        }
        "$icontains" => {
            let s = expect_string(op, operand)?;
            parts.push(format!("LOWER({}) LIKE LOWER(?)", scalar_expr));
            binds.push(libsql::Value::Text(format!("%{}%", s)));
        }
        "$regex" => {
            let pattern = expect_string(op, operand)?;
            parts.push(format!("{} REGEXP ?", scalar_expr));
            binds.push(libsql::Value::Text(pattern.to_string()));
        }
        "$in" | "$nin" => {
            let arr = expect_array(op, operand)?;
            if arr.is_empty() {
                return Err(AppError::BadRequest(format!("{op} cannot be empty")));
            }
            let placeholders = vec!["?"; arr.len()].join(", ");
            if op == "$in" {
                parts.push(format!("{} IN ({})", scalar_expr, placeholders));
            } else {
                parts.push(format!("{} NOT IN ({})", scalar_expr, placeholders));
            }
            for item in arr {
                binds.push(json_to_sql_value(item)?);
            }
        }
        "$between" => {
            let arr = expect_array(op, operand)?;
            if arr.len() != 2 {
                return Err(AppError::BadRequest(
                    "$between value must contain exactly 2 items".to_string(),
                ));
            }
            parts.push(format!("{} BETWEEN ? AND ?", scalar_expr));
            binds.push(json_to_sql_value(&arr[0])?);
            binds.push(json_to_sql_value(&arr[1])?);
        }
        "$size" => {
            let size_expr = format!("json_array_length({})", array_expr);
            compile_size_predicate(&size_expr, operand, parts, binds)?;
        }
        "$type" => {
            let t = expect_string(op, operand)?;
            let type_expr = format!("json_type({})", array_expr);
            compile_type_predicate(&type_expr, t, parts, binds)?;
        }
        "$includes" => {
            parts.push(format!(
                "EXISTS (SELECT 1 FROM json_each({}) je WHERE je.value = ?)",
                array_expr
            ));
            binds.push(json_to_sql_value(operand)?);
        }
        "$nincludes" => {
            parts.push(format!(
                "NOT EXISTS (SELECT 1 FROM json_each({}) je WHERE je.value = ?)",
                array_expr
            ));
            binds.push(json_to_sql_value(operand)?);
        }
        "$any" => {
            let arr = expect_array(op, operand)?;
            if arr.is_empty() {
                return Err(AppError::BadRequest("$any cannot be empty".to_string()));
            }
            let placeholders = vec!["?"; arr.len()].join(", ");
            parts.push(format!(
                "EXISTS (SELECT 1 FROM json_each({}) je WHERE je.value IN ({}))",
                array_expr, placeholders
            ));
            for item in arr {
                binds.push(json_to_sql_value(item)?);
            }
        }
        "$all" => {
            let arr = expect_array(op, operand)?;
            if arr.is_empty() {
                return Err(AppError::BadRequest("$all cannot be empty".to_string()));
            }
            let mut all_parts = Vec::new();
            for _ in arr {
                all_parts.push(format!(
                    "EXISTS (SELECT 1 FROM json_each({}) je WHERE je.value = ?)",
                    array_expr
                ));
            }
            parts.push(format!("({})", all_parts.join(" AND ")));
            for item in arr {
                binds.push(json_to_sql_value(item)?);
            }
        }
        "$none" => {
            let arr = expect_array(op, operand)?;
            if arr.is_empty() {
                return Err(AppError::BadRequest("$none cannot be empty".to_string()));
            }
            let placeholders = vec!["?"; arr.len()].join(", ");
            parts.push(format!(
                "NOT EXISTS (SELECT 1 FROM json_each({}) je WHERE je.value IN ({}))",
                array_expr, placeholders
            ));
            for item in arr {
                binds.push(json_to_sql_value(item)?);
            }
        }
        "$elemMatch" => {
            elem_base_expr.ok_or_else(|| {
                AppError::BadRequest("$elemMatch not available at this context".to_string())
            })?;
            let compiled = compile_elem_match(array_expr, operand)?;
            parts.push(compiled.sql);
            binds.extend(compiled.binds);
        }
        _ => {
            return Err(AppError::BadRequest(format!("unsupported operator: {op}")));
        }
    }

    Ok(())
}

fn compile_elem_match(array_expr: &str, operand: &Value) -> AppResult<CompiledWhere> {
    let alias = json_each_alias(array_expr);
    let inner = compile_filter_on_base(&format!("{alias}.value"), operand, true)?;
    Ok(CompiledWhere {
        sql: format!(
            "EXISTS (SELECT 1 FROM json_each({}) {} WHERE {})",
            array_expr, alias, inner.sql
        ),
        binds: inner.binds,
    })
}

fn json_each_alias(array_expr: &str) -> String {
    // Stable FNV-1a keeps nested json_each aliases unique without compiler-global state.
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in array_expr.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("je_{hash:x}")
}

fn json_tree_alias(base_expr: &str, path: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in format!("{base_expr}:{path}").as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("jt_{hash:x}")
}

fn compile_size_predicate(
    size_expr: &str,
    operand: &Value,
    parts: &mut Vec<String>,
    binds: &mut Vec<libsql::Value>,
) -> AppResult<()> {
    if let Some(n) = operand.as_i64() {
        parts.push(format!("{} = ?", size_expr));
        binds.push(libsql::Value::Integer(n));
        return Ok(());
    }

    let obj = operand.as_object().ok_or_else(|| {
        AppError::BadRequest("$size must be integer or operator object".to_string())
    })?;

    for (op, v) in obj {
        let n = v.as_i64().ok_or_else(|| {
            AppError::BadRequest(format!("$size operator {op} requires integer value"))
        })?;
        match op.as_str() {
            "$eq" => parts.push(format!("{} = ?", size_expr)),
            "$ne" => parts.push(format!("{} != ?", size_expr)),
            "$gt" => parts.push(format!("{} > ?", size_expr)),
            "$gte" => parts.push(format!("{} >= ?", size_expr)),
            "$lt" => parts.push(format!("{} < ?", size_expr)),
            "$lte" => parts.push(format!("{} <= ?", size_expr)),
            _ => {
                return Err(AppError::BadRequest(format!(
                    "unsupported $size operator: {op}"
                )));
            }
        }
        binds.push(libsql::Value::Integer(n));
    }

    Ok(())
}

fn compile_type_predicate(
    type_expr: &str,
    operand: &str,
    parts: &mut Vec<String>,
    binds: &mut Vec<libsql::Value>,
) -> AppResult<()> {
    match operand {
        "number" => parts.push(format!(
            "({} = 'integer' OR {} = 'real')",
            type_expr, type_expr
        )),
        "boolean" => parts.push(format!(
            "({} = 'true' OR {} = 'false')",
            type_expr, type_expr
        )),
        "string" => {
            parts.push(format!("{} = ?", type_expr));
            binds.push(libsql::Value::Text("text".to_string()));
        }
        "array" | "object" | "null" | "integer" | "real" | "text" | "true" | "false" => {
            parts.push(format!("{} = ?", type_expr));
            binds.push(libsql::Value::Text(operand.to_string()));
        }
        _ => {
            return Err(AppError::BadRequest(format!(
                "unsupported $type value: {operand}"
            )));
        }
    }
    Ok(())
}

fn expect_string<'a>(op: &str, operand: &'a Value) -> AppResult<&'a str> {
    operand
        .as_str()
        .ok_or_else(|| AppError::BadRequest(format!("{op} value must be string")))
}

fn expect_array<'a>(op: &str, operand: &'a Value) -> AppResult<&'a Vec<Value>> {
    operand
        .as_array()
        .ok_or_else(|| AppError::BadRequest(format!("{op} value must be array")))
}

fn json_path_expr(base_expr: &str, path: &str) -> AppResult<String> {
    validate_path(path)?;
    Ok(format!("{base_expr} ->> '$.{path}'"))
}

fn json_array_expr(base_expr: &str, path: &str) -> AppResult<String> {
    validate_path(path)?;
    Ok(format!("json_extract({base_expr}, '$.{path}')"))
}

fn validate_path(path: &str) -> AppResult<()> {
    if path.is_empty() {
        return Err(AppError::BadRequest(
            "filter path cannot be empty".to_string(),
        ));
    }

    for segment in path.split('.') {
        if segment.is_empty() {
            return Err(AppError::BadRequest("invalid filter path".to_string()));
        }
        if !segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(AppError::BadRequest(format!(
                "invalid filter path segment: {segment}"
            )));
        }
    }

    Ok(())
}

fn json_to_sql_value(v: &Value) -> AppResult<libsql::Value> {
    match v {
        Value::Null => Ok(libsql::Value::Null),
        Value::Bool(b) => Ok(libsql::Value::Integer(if *b { 1 } else { 0 })),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(libsql::Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(libsql::Value::Real(f))
            } else {
                Err(AppError::BadRequest(
                    "unsupported numeric value".to_string(),
                ))
            }
        }
        Value::String(s) => Ok(libsql::Value::Text(s.clone())),
        Value::Array(_) | Value::Object(_) => Err(AppError::BadRequest(
            "nested array/object values are not supported in scalar comparisons".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::build_where;
    use serde_json::json;

    #[test]
    fn array_paths_compile_to_independent_element_traversals() {
        let compiled = build_where(&json!({
            "people[].name": "Ada",
            "people[].age": {"$gte": 18}
        }))
        .expect("array paths should compile");
        assert_eq!(compiled.sql.matches("EXISTS").count(), 2);
        assert_eq!(compiled.binds.len(), 2);
    }

    #[test]
    fn nested_array_paths_compile_recursively() {
        let compiled = build_where(&json!({
            "departments[].teams[].members[].name": "Ada"
        }))
        .expect("nested array paths should compile");
        assert!(compiled.sql.contains("json_tree"));
        assert!(!compiled.sql.contains("json_each"));
        assert!(matches!(
            compiled.binds.first(),
            Some(libsql::Value::Text(pattern))
                if pattern == "$.departments[%].teams[%].members[%].name"
        ));
        assert_eq!(compiled.binds.len(), 2);
    }

    #[test]
    fn scalar_array_paths_support_operator_objects() {
        let compiled = build_where(&json!({"scores[]": {"$gte": 90}}))
            .expect("scalar array operator should compile");
        assert!(compiled.sql.contains(".value"));
        assert_eq!(compiled.binds.len(), 1);
    }
}
