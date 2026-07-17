//! Manifest metadata structures describing WAL segments and replication state.

use serde::{Deserialize, Serialize};

use super::snapshot::SnapshotMeta;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub tenant: String,
    pub epoch: u64,
    pub writer_id: String,
    pub applied_seq: u64,
    pub current_snapshot_id: Option<String>,
    pub current_snapshot_key: Option<String>,
    pub snapshots: Vec<SnapshotMeta>,
    pub segments: Vec<ManifestSegment>,
    pub compaction_watermark: Option<u64>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestSegment {
    pub seq: u64,
    pub object_key: String,
    pub checksum: String,
    pub start_lsn: u64,
    pub end_lsn: u64,
    pub compressed_bytes: usize,
}
