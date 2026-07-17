//! Storage abstraction modules for local and s3 backends.

pub mod auto_index;
pub mod backup;
pub mod db_path;
pub mod local;
pub mod manager;
pub mod reaper;
pub mod s3_wal;
pub mod schema;
pub mod system_catalog;
