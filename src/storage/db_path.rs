//! Validation and normalization of `db_path` into a safe on-disk sqlite file path.

use std::path::PathBuf;

use crate::error::{AppError, AppResult};

pub fn resolve_db_file(base_path: &str, db_path: &str) -> AppResult<(String, PathBuf)> {
    let normalized = normalize_db_path(db_path)?;

    let mut out = PathBuf::from(base_path);
    let mut parts = normalized.split('/').peekable();

    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            out.push(format!("{}.db", part));
        } else {
            out.push(part);
        }
    }

    Ok((normalized, out))
}

fn normalize_db_path(input: &str) -> AppResult<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("db_path is required".to_string()));
    }

    if trimmed.starts_with('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err(AppError::BadRequest("invalid db_path".to_string()));
    }

    let mut segments = Vec::new();
    for seg in trimmed.split('/') {
        if seg.is_empty() {
            return Err(AppError::BadRequest("invalid db_path".to_string()));
        }
        if !seg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        {
            return Err(AppError::BadRequest(format!(
                "invalid db_path segment: {seg}"
            )));
        }
        segments.push(seg.to_string());
    }

    Ok(segments.join("/"))
}
