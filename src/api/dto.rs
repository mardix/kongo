//! Request/response DTOs for the gateway RPC contract.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayRequest {
    #[serde(rename = "db", alias = "db_path")]
    pub db: Option<String>,
    pub operation: String,
    #[serde(default, alias = "collection")]
    pub namespace: Option<String>,
    #[serde(default)]
    pub namespaces: Option<Vec<String>>,
    #[serde(default)]
    pub payload: OperationPayload,
    pub data: Option<Vec<GatewayRequest>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OperationPayload {
    pub collection: Option<String>,
    pub namespaces: Option<Vec<String>>,
    pub table: Option<String>,
    pub sql: Option<String>,
    pub params: Option<Vec<Value>>,
    #[serde(alias = "q")]
    pub search: Option<String>,
    pub from_namespace: Option<String>,
    pub to_namespace: Option<String>,
    pub to_db_path: Option<String>,
    pub backup_db_path: Option<String>,
    pub backup_id: Option<String>,
    pub backup_at: Option<String>,
    pub backup_tag: Option<String>,
    pub latest: Option<bool>,
    pub source_path: Option<String>,
    pub source_hash: Option<String>,
    pub target_path: Option<String>,
    pub compress: Option<bool>,
    pub alias_import_pk: Option<Value>,
    pub drop_keys: Option<Vec<String>>,
    pub job_id: Option<String>,
    pub job_type: Option<String>,
    pub status: Option<String>,
    pub on_conflict: Option<String>,
    pub commit: Option<bool>,
    pub alias: Option<String>,
    pub label: Option<String>,
    #[serde(rename = "type")]
    pub catalog_type: Option<String>,
    pub name: Option<String>,
    pub value: Option<String>,
    pub event: Option<String>,
    pub events: Option<Vec<Value>>,
    pub action: Option<String>,
    pub actor_type: Option<String>,
    pub actor_id: Option<String>,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub source: Option<String>,
    pub request_id: Option<String>,
    pub ip_address: Option<String>,
    pub message: Option<String>,
    pub user_id: Option<String>,
    #[serde(rename = "_user_id")]
    pub document_user_id: Option<String>,
    pub attach_users: Option<bool>,
    pub attach_user_fields: Option<Vec<String>>,
    pub email: Option<String>,
    pub username: Option<String>,
    pub phone: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub profile_photo: Option<String>,
    pub provider: Option<String>,
    pub provider_user_id: Option<String>,
    pub password_hash: Option<String>,
    pub password_algo: Option<String>,
    pub requires_password_change: Option<bool>,
    pub email_verified_at: Option<String>,
    pub phone_verified_at: Option<String>,
    pub token_hash: Option<String>,
    pub kind: Option<String>,
    pub allow_multi: Option<bool>,
    pub bucket: Option<String>,
    pub storage_backend: Option<String>,
    pub storage_path: Option<String>,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub size_bytes: Option<i64>,
    pub sha256: Option<String>,
    pub owner_type: Option<String>,
    pub owner_id: Option<String>,
    pub metadata: Option<Value>,
    pub uploaded_at: Option<String>,
    pub expires_at: Option<String>,
    pub expires_in: Option<i64>,
    pub status_reason: Option<String>,
    pub status_expires_at: Option<String>,
    pub status_expires_in: Option<i64>,
    pub status_next: Option<String>,
    pub status_next_reason: Option<String>,
    pub changed_by: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub range: Option<String>,
    pub interval: Option<String>,
    pub bucket_label: Option<String>,
    pub metrics: Option<Value>,
    pub batch: Option<Vec<OperationPayload>>,
    pub replace: Option<bool>,
    pub unique_fields: Option<Vec<String>>,
    pub ignore_input_id: Option<bool>,
    pub resumable: Option<bool>,
    pub batch_size: Option<i64>,
    pub enable: Option<bool>,
    pub retain_segments: Option<i64>,
    pub index_name: Option<String>,
    pub index_path: Option<String>,
    pub id: Option<String>,
    pub ids: Option<Vec<String>>,
    pub data: Option<Value>,
    pub update_data: Option<Value>,
    pub insert_data: Option<Value>,
    pub expiry_behavior: Option<String>,
    pub filter: Option<Value>,
    #[serde(rename = "txn_id", alias = "_txn_id")]
    pub txn_id: Option<String>,
    pub snapshot_id: Option<String>,
    pub purge: Option<bool>,
    pub ttl_seconds: Option<i64>,
    pub older_than_secs: Option<i64>,
    pub allow_system_timestamps: Option<bool>,
    pub include_system_timestamps: Option<bool>,
    #[serde(alias = "include_name")]
    pub include_namespace: Option<bool>,
    pub compute: Option<Value>,
    pub group_by: Option<Value>,
    pub lookups: Option<Value>,
    pub lookup_depth_override: Option<i64>,
    pub sort: Option<Value>,
    pub fields: Option<Vec<String>>,
    pub exclude_fields: Option<Vec<String>>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub max_docs: Option<i64>,
    pub dry_run: Option<bool>,
    pub scope: Option<String>,
    pub include_archive: Option<bool>,
    pub archive_only: Option<bool>,
    pub explain: Option<bool>,
    pub force_db: Option<bool>,
    pub cache: Option<CacheHint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CacheHint {
    Bool(bool),
    Int(i64),
}

#[derive(Debug, Serialize)]
pub struct GatewayResponse {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(rename = "_txn_id", skip_serializing_if = "Option::is_none")]
    pub txn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub committed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_async_ack: Option<bool>,
}

impl GatewayResponse {
    pub fn ok(data: Option<Value>) -> Self {
        Self {
            status: "success",
            data,
            txn_id: None,
            message: None,
            ack_mode: None,
            ack_status: None,
            committed: None,
            is_async_ack: None,
        }
    }
}
