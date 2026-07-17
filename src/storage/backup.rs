//! Shared backup worker/report models used by storage backends.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, Default)]
pub struct AutoBackupCycleReport {
    pub discovered: usize,
    pub scheduled: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub skipped_recent: usize,
    pub skipped_unchanged: usize,
    pub skipped_lease: bool,
    pub error_samples: Vec<String>,
}
