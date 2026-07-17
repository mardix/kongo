//! Writer lease model used to coordinate a single active WAL writer.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriterLease {
    pub tenant: String,
    pub epoch: u64,
    pub writer_id: String,
    pub lease_until_rfc3339: String,
}

impl WriterLease {}
