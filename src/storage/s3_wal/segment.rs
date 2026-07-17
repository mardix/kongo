//! Serializable WAL record/segment structures stored in object storage.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalRecord {
    pub seq: u64,
    pub ts_rfc3339: String,
    pub sql: String,
    pub args_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalSegment {
    pub tenant: String,
    pub epoch: u64,
    pub start_seq: u64,
    pub end_seq: u64,
    pub checksum: String,
    pub records: Vec<WalRecord>,
}
