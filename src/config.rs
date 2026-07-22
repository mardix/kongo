//! Runtime configuration model and environment variable loading for Kongodb.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KongodbConfig {
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub storage: StorageConfig,
    pub reaper: ReaperConfig,
    pub delete: DeleteConfig,
    pub backup: BackupConfig,
    pub cache: CacheConfig,
    pub response: ResponseConfig,
    pub json_storage: JsonStorageConfig,
    pub query: QueryConfig,
    pub query_lookup: QueryLookupConfig,
    pub legacy_aliases: LegacyAliasesConfig,
    pub auto_index: AutoIndexConfig,
    pub import: ImportConfig,
    pub export: ExportConfig,
    pub metric_events: MetricEventsConfig,
    pub system_catalog: SystemCatalogConfig,
    pub mutation: MutationConfig,
    pub write_queue: WriteQueueConfig,
    pub runtime: RuntimeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: [u8; 4],
    pub port: u16,
    pub base_path: String,
    pub admin_ui_enabled: bool,
    pub admin_ui_dir: String,
    pub docs_enabled: bool,
    pub docs_file: String,
    pub cors_allowed_origins: Vec<String>,
    pub max_request_bytes: usize,
    pub operation_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub mode: String,
    pub access_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub mode: StorageMode,
    pub data_dir: String,
    pub s3: Option<S3Config>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageMode {
    Local,
    S3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3Config {
    pub bucket: String,
    pub prefix: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub credentials: Option<S3Credentials>,
    pub lease_duration_secs: u64,
    pub segment_max_bytes: usize,
    pub flush_interval_secs: u64,
    pub preload_dbs: Vec<String>,
    pub snapshot_every_writes: u64,
    pub snapshot_max_count: usize,
    pub snapshot_max_age_days: u64,
    pub topology: S3Topology,
    pub remote_sync_enabled: bool,
    pub remote_sync_interval_secs: u64,
    pub replication_mode: ReplicationMode,
    pub safe_hydrate: bool,
    pub safe_hydrate_quick_check: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum S3Topology {
    Single,
    Multi,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplicationMode {
    Sync,
    Async,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3Credentials {
    pub access_key: String,
    pub secret_key: String,
    pub session_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReaperConfig {
    pub interval_secs: u64,
    pub max_concurrency: usize,
    pub __kdb_archive_ttl_secs: Option<u64>,
    pub temp_cleanup_interval_secs: u64,
    pub temp_cleanup_older_than_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteConfig {
    pub default_ttl_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupConfig {
    pub mode: BackupMode,
    pub enabled: bool,
    pub interval_secs: u64,
    pub max_concurrency: usize,
    pub min_interval_per_db_secs: u64,
    pub min_writes_since_backup: u64,
    pub max_staleness_secs: u64,
    pub max_count: usize,
    pub max_age_days: u64,
    pub local_path: String,
    pub s3_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupMode {
    Local,
    S3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    pub enabled: bool,
    pub ttl_secs: u64,
    pub max_entries: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseConfig {
    pub include_system_timestamps: bool,
    pub include_namespace: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonStorageConfig {
    pub format: JsonStorageFormat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonStorageFormat {
    Text,
    Jsonb,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryConfig {
    pub default_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyAliasesConfig {
    pub enabled: bool,
    pub import_pk: Vec<(String, String)>,
    pub response: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryLookupConfig {
    pub max_depth: usize,
    pub uncapped_override_enabled: bool,
    pub max_concurrency: usize,
    pub default_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoIndexConfig {
    pub interval_secs: u64,
    pub min_hits: i64,
    pub max_indexes_per_db: usize,
    pub max_new_per_run: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportConfig {
    pub worker_interval_secs: u64,
    pub job_retention_days: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportConfig {
    pub worker_interval_secs: u64,
    pub job_retention_days: u64,
    pub batch_size: usize,
    pub local_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricEventsConfig {
    pub cache_enabled: bool,
    pub cache_ttl_secs: u64,
    pub insert_batch_size: usize,
    pub retention_days: Option<u64>,
    pub query_default_limit: usize,
    pub query_max_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemCatalogConfig {
    pub retention_days: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationConfig {
    pub strict_mutation_operators: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteQueueConfig {
    pub enabled: bool,
    pub ack_mode: WriteAckMode,
    pub capacity: usize,
    pub idle_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub max_active_dbs: usize,
    pub db_idle_close_secs: u64,
    pub job_worker_concurrency: usize,
    pub sync_concurrency: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteAckMode {
    Committed,
    Accepted,
}

impl KongodbConfig {
    pub fn from_env() -> Self {
        let mode = match std::env::var("KONGODB_STORAGE_MODE")
            .unwrap_or_else(|_| "local".to_string())
            .as_str()
        {
            "s3" => StorageMode::S3,
            _ => StorageMode::Local,
        };
        let profile = RuntimeDefaults::from_env();
        let worker_concurrency = env_usize("KONGODB_WORKER_CONCURRENCY")
            .unwrap_or(profile.worker_concurrency)
            .max(1);
        let cache_ttl_secs = env_u64("KONGODB_CACHE_TTL_SECS").unwrap_or(15);
        let metric_cache_ttl_secs = env_u64("KONGODB_METRIC_EVENTS_CACHE_TTL_SECS").unwrap_or(30);
        let s3_topology = match std::env::var("KONGODB_S3_TOPOLOGY")
            .unwrap_or_else(|_| "single".to_string())
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "multi" => S3Topology::Multi,
            _ => S3Topology::Single,
        };
        let remote_sync_interval_secs = env_u64("KONGODB_REMOTE_SYNC_INTERVAL_SECS").unwrap_or(10);
        let remote_sync_enabled =
            matches!(s3_topology, S3Topology::Multi) && remote_sync_interval_secs > 0;
        let job_retention_days = env_u64("KONGODB_JOB_RETENTION_DAYS").unwrap_or(30);
        let backup_path =
            env_nonempty("KONGODB_BACKUP_PATH").unwrap_or_else(|| "./backups".to_string());
        let backup_every_secs = env_u64("KONGODB_BACKUP_EVERY_SECS").unwrap_or(0);
        let backup_is_s3 = backup_path.starts_with("s3://");
        let export_path =
            env_nonempty("KONGODB_EXPORT_PATH").unwrap_or_else(|| "./exports".to_string());
        let write_mode = std::env::var("KONGODB_WRITE_MODE")
            .unwrap_or_else(|_| "committed".to_string())
            .to_ascii_lowercase();

        Self {
            server: ServerConfig {
                host: [0, 0, 0, 0],
                port: env_usize("KONGODB_PORT")
                    .and_then(|v| u16::try_from(v).ok())
                    .unwrap_or(8080),
                base_path: normalize_base_path(
                    std::env::var("KONGODB_BASE_PATH").unwrap_or_else(|_| "/_/kdb".to_string()),
                ),
                admin_ui_enabled: env_bool("KONGODB_ADMIN_UI_ENABLED").unwrap_or(true),
                admin_ui_dir: "admin-ui/dist".to_string(),
                docs_enabled: env_bool("KONGODB_DOCS_ENABLED").unwrap_or(true),
                docs_file: env_nonempty("KONGODB_DOCS_FILE")
                    .unwrap_or_else(|| "DOCUMENTATION.md".to_string()),
                cors_allowed_origins: std::env::var("KONGODB_CORS_ALLOWED_ORIGINS")
                    .ok()
                    .map(|v| {
                        v.split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(ToOwned::to_owned)
                            .collect::<Vec<String>>()
                    })
                    .unwrap_or_default(),
                max_request_bytes: env_usize("KONGODB_MAX_REQUEST_BYTES")
                    .unwrap_or(16 * 1024 * 1024),
                operation_timeout_ms: env_u64("KONGODB_OPERATION_TIMEOUT_MS").unwrap_or(30_000),
            },
            auth: AuthConfig {
                mode: std::env::var("KONGODB_AUTH_MODE")
                    .unwrap_or_else(|_| "access_key".to_string())
                    .trim()
                    .to_ascii_lowercase(),
                access_key: env_nonempty("KONGODB_ACCESS_KEY"),
            },
            storage: StorageConfig {
                mode,
                data_dir: std::env::var("KONGODB_DATA_DIR")
                    .unwrap_or_else(|_| "./data".to_string()),
                s3: Some(S3Config {
                    bucket: std::env::var("KONGODB_S3_BUCKET").unwrap_or_default(),
                    prefix: std::env::var("KONGODB_S3_PREFIX")
                        .unwrap_or_else(|_| "data/kongodb/data".to_string()),
                    region: std::env::var("KONGODB_S3_REGION")
                        .unwrap_or_else(|_| "us-east-1".to_string()),
                    endpoint: env_nonempty("KONGODB_S3_ENDPOINT"),
                    credentials: env_s3_credentials("KONGODB_S3"),
                    lease_duration_secs: 30,
                    segment_max_bytes: 8 * 1024 * 1024,
                    flush_interval_secs: 2,
                    preload_dbs: std::env::var("KONGODB_PRELOAD_DBS")
                        .ok()
                        .map(|v| {
                            v.split(',')
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(ToOwned::to_owned)
                                .collect::<Vec<String>>()
                        })
                        .unwrap_or_default(),
                    snapshot_every_writes: env_u64("KONGODB_SNAPSHOT_EVERY_WRITES").unwrap_or(100),
                    snapshot_max_count: 64,
                    snapshot_max_age_days: env_u64("KONGODB_SNAPSHOT_RETENTION_DAYS").unwrap_or(14),
                    topology: s3_topology,
                    remote_sync_enabled,
                    remote_sync_interval_secs,
                    replication_mode: match std::env::var("KONGODB_REPLICATION_MODE")
                        .unwrap_or_else(|_| "async".to_string())
                        .as_str()
                    {
                        "sync" => ReplicationMode::Sync,
                        _ => ReplicationMode::Async,
                    },
                    safe_hydrate: true,
                    safe_hydrate_quick_check: true,
                }),
            },
            reaper: ReaperConfig {
                interval_secs: 60,
                max_concurrency: worker_concurrency,
                __kdb_archive_ttl_secs: env_u64("KONGODB_ARCHIVE_TTL_SECS"),
                temp_cleanup_interval_secs: 300,
                temp_cleanup_older_than_secs: 600,
            },
            delete: DeleteConfig {
                default_ttl_secs: env_u64("KONGODB_DELETE_DEFAULT_TTL_SECS"),
            },
            backup: BackupConfig {
                mode: if backup_is_s3 {
                    BackupMode::S3
                } else {
                    BackupMode::Local
                },
                enabled: backup_every_secs > 0,
                interval_secs: backup_every_secs.clamp(1, 300),
                max_concurrency: worker_concurrency,
                min_interval_per_db_secs: backup_every_secs,
                min_writes_since_backup: 1,
                max_staleness_secs: backup_every_secs,
                max_count: 200,
                max_age_days: env_u64("KONGODB_BACKUP_RETENTION_DAYS").unwrap_or(30),
                local_path: backup_path.clone(),
                s3_path: backup_is_s3.then_some(backup_path),
            },
            cache: CacheConfig {
                enabled: cache_ttl_secs > 0,
                ttl_secs: cache_ttl_secs,
                max_entries: profile.cache_max_entries,
            },
            response: ResponseConfig {
                include_system_timestamps: env_bool("KONGODB_RESPONSE_INCLUDE_SYSTEM_TIMESTAMPS")
                    .unwrap_or(true),
                include_namespace: env_bool("KONGODB_RESPONSE_INCLUDE_NAMESPACE").unwrap_or(false),
            },
            json_storage: JsonStorageConfig {
                format: JsonStorageFormat::Jsonb,
            },
            query: QueryConfig {
                default_limit: env_usize("KONGODB_QUERY_DEFAULT_LIMIT").unwrap_or(50),
            },
            legacy_aliases: LegacyAliasesConfig {
                enabled: env_bool("KONGODB_ENABLE_LEGACY_ALIASES").unwrap_or(false),
                import_pk: parse_alias_map(
                    &std::env::var("KONGODB_LEGACY_ALIASES_IMPORT_PK").unwrap_or_default(),
                ),
                response: parse_alias_map(
                    &std::env::var("KONGODB_LEGACY_ALIASES_RESPONSE").unwrap_or_default(),
                ),
            },
            query_lookup: QueryLookupConfig {
                max_depth: env_usize("KONGODB_QUERY_LOOKUP_MAX_DEPTH").unwrap_or(3),
                uncapped_override_enabled: env_bool(
                    "KONGODB_QUERY_LOOKUP_UNCAPPED_OVERRIDE_ENABLED",
                )
                .unwrap_or(false),
                max_concurrency: profile.lookup_concurrency,
                default_limit: env_usize("KONGODB_QUERY_DEFAULT_LIMIT").unwrap_or(50),
            },
            auto_index: AutoIndexConfig {
                interval_secs: 60,
                min_hits: 200,
                max_indexes_per_db: 16,
                max_new_per_run: 2,
            },
            import: ImportConfig {
                worker_interval_secs: 2,
                job_retention_days,
            },
            export: ExportConfig {
                worker_interval_secs: 2,
                job_retention_days,
                batch_size: profile.export_batch_size,
                local_path: export_path,
            },
            metric_events: MetricEventsConfig {
                cache_enabled: metric_cache_ttl_secs > 0,
                cache_ttl_secs: metric_cache_ttl_secs,
                insert_batch_size: profile.metric_insert_batch_size,
                retention_days: env_u64("KONGODB_METRIC_EVENTS_RETENTION_DAYS"),
                query_default_limit: profile.metric_query_default_limit,
                query_max_limit: profile.metric_query_max_limit,
            },
            system_catalog: SystemCatalogConfig {
                retention_days: env_u64("KONGODB_SYSTEM_RETENTION_DAYS").unwrap_or(14),
            },
            mutation: MutationConfig {
                strict_mutation_operators: env_bool("KONGODB_STRICT_MUTATIONS_OPERATORS")
                    .unwrap_or(false),
            },
            write_queue: WriteQueueConfig {
                enabled: write_mode != "direct",
                ack_mode: if write_mode == "accepted" {
                    WriteAckMode::Accepted
                } else {
                    WriteAckMode::Committed
                },
                capacity: profile.write_queue_capacity,
                idle_secs: 300,
            },
            runtime: RuntimeConfig {
                max_active_dbs: env_usize("KONGODB_MAX_ACTIVE_DBS")
                    .unwrap_or(profile.max_active_dbs),
                db_idle_close_secs: profile.db_idle_close_secs,
                job_worker_concurrency: worker_concurrency,
                sync_concurrency: worker_concurrency,
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RuntimeDefaults {
    max_active_dbs: usize,
    db_idle_close_secs: u64,
    worker_concurrency: usize,
    lookup_concurrency: usize,
    cache_max_entries: u64,
    write_queue_capacity: usize,
    export_batch_size: usize,
    metric_insert_batch_size: usize,
    metric_query_default_limit: usize,
    metric_query_max_limit: usize,
}

impl RuntimeDefaults {
    fn from_env() -> Self {
        match std::env::var("KONGODB_RUNTIME_PROFILE")
            .unwrap_or_else(|_| "balanced".to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "memory" => Self {
                max_active_dbs: 25,
                db_idle_close_secs: 300,
                worker_concurrency: 2,
                lookup_concurrency: 4,
                cache_max_entries: 2_500,
                write_queue_capacity: 2_500,
                export_batch_size: 500,
                metric_insert_batch_size: 500,
                metric_query_default_limit: 500,
                metric_query_max_limit: 5_000,
            },
            "throughput" => Self {
                max_active_dbs: 250,
                db_idle_close_secs: 1_800,
                worker_concurrency: 8,
                lookup_concurrency: 16,
                cache_max_entries: 50_000,
                write_queue_capacity: 50_000,
                export_batch_size: 5_000,
                metric_insert_batch_size: 5_000,
                metric_query_default_limit: 5_000,
                metric_query_max_limit: 50_000,
            },
            _ => Self {
                max_active_dbs: 100,
                db_idle_close_secs: 900,
                worker_concurrency: 4,
                lookup_concurrency: 8,
                cache_max_entries: 10_000,
                write_queue_capacity: 10_000,
                export_batch_size: 1_000,
                metric_insert_batch_size: 1_000,
                metric_query_default_limit: 1_000,
                metric_query_max_limit: 10_000,
            },
        }
    }
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_bool(name: &str) -> Option<bool> {
    std::env::var(name)
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok().and_then(|v| v.parse().ok())
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok().and_then(|v| v.parse().ok())
}

fn normalize_base_path(raw: String) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let with_leading = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    };
    if with_leading.len() > 1 {
        with_leading.trim_end_matches('/').to_string()
    } else {
        String::new()
    }
}

fn env_s3_credentials(prefix: &str) -> Option<S3Credentials> {
    let access_key = std::env::var(format!("{prefix}_ACCESS_KEY")).ok();
    let secret_key = std::env::var(format!("{prefix}_SECRET_KEY")).ok();
    match (access_key, secret_key) {
        (Some(access_key), Some(secret_key))
            if !access_key.trim().is_empty() && !secret_key.trim().is_empty() =>
        {
            Some(S3Credentials {
                access_key,
                secret_key,
                session_token: std::env::var(format!("{prefix}_SESSION_TOKEN")).ok(),
            })
        }
        _ => None,
    }
}

fn parse_alias_map(raw: &str) -> Vec<(String, String)> {
    raw.split(',')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, ':');
            let from = parts.next()?.trim();
            let to = parts.next()?.trim();
            if from.is_empty() || to.is_empty() {
                return None;
            }
            Some((from.to_string(), to.to_string()))
        })
        .collect()
}
