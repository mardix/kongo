//! Snapshot metadata model for checkpointed database state in the WAL pipeline.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMeta {
    pub id: String,
    pub tenant: String,
    pub object_key: String,
    pub checksum: String,
    pub size_bytes: usize,
    pub created_at: String,
    pub from_seq: u64,
    pub to_seq: u64,
}
