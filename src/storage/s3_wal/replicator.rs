//! WAL replicator: appends segments, updates manifest, and manages writer leases.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::{
    config::S3Config,
    error::{AppError, AppResult},
    storage::s3_wal::{
        lease::WriterLease,
        manifest::{Manifest, ManifestSegment},
        object_store::ObjectStore,
        segment::WalSegment,
        snapshot::SnapshotMeta,
    },
};

const MANIFEST_FILE: &str = "manifest.json";
const LEASE_FILE: &str = "lease.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncReport {
    pub tenant: String,
    pub epoch: u64,
    pub applied_seq: u64,
    pub segment_count: usize,
}

#[derive(Clone)]
pub struct Replicator {
    store: Arc<dyn ObjectStore>,
    cfg: Arc<S3Config>,
    writer_id: String,
}

impl Replicator {
    pub fn new(store: Arc<dyn ObjectStore>, cfg: Arc<S3Config>, writer_id: String) -> Self {
        Self {
            store,
            cfg,
            writer_id,
        }
    }

    pub async fn ensure_lease(&self, tenant: &str) -> AppResult<WriterLease> {
        let key = self.lease_key(tenant);
        let now = unix_now_secs();

        if let Some((raw, etag)) = self.store.get(&key).await? {
            let existing: WriterLease = serde_json::from_slice(&raw)
                .map_err(|e| AppError::Internal(format!("lease decode failed: {e}")))?;

            let expires_at = existing
                .lease_until_rfc3339
                .parse::<u64>()
                .map_err(|e| AppError::Internal(format!("lease time parse failed: {e}")))?;

            if existing.writer_id != self.writer_id && expires_at > now {
                return Err(AppError::Conflict(format!(
                    "writer lease held by {} until {}",
                    existing.writer_id, existing.lease_until_rfc3339
                )));
            }

            let renewed = WriterLease {
                tenant: tenant.to_string(),
                epoch: existing.epoch,
                writer_id: self.writer_id.clone(),
                lease_until_rfc3339: (now + self.cfg.lease_duration_secs).to_string(),
            };

            let bytes = serde_json::to_vec(&renewed)
                .map_err(|e| AppError::Internal(format!("lease encode failed: {e}")))?;
            self.store.put_if_match(&key, &bytes, Some(&etag)).await?;
            return Ok(renewed);
        }

        let created = WriterLease {
            tenant: tenant.to_string(),
            epoch: 1,
            writer_id: self.writer_id.clone(),
            lease_until_rfc3339: (now + self.cfg.lease_duration_secs).to_string(),
        };

        let bytes = serde_json::to_vec(&created)
            .map_err(|e| AppError::Internal(format!("lease encode failed: {e}")))?;
        self.store.put(&key, &bytes).await?;
        Ok(created)
    }

    pub async fn sync_once(&self, tenant: &str) -> AppResult<SyncReport> {
        let lease = self.ensure_lease(tenant).await?;
        let manifest = self.load_or_init_manifest(tenant, lease.epoch).await?;

        Ok(SyncReport {
            tenant: tenant.to_string(),
            epoch: manifest.epoch,
            applied_seq: manifest.applied_seq,
            segment_count: manifest.segments.len(),
        })
    }

    pub async fn append_segment(
        &self,
        tenant: &str,
        sql_records: Vec<(String, String)>,
    ) -> AppResult<ManifestSegment> {
        if sql_records.is_empty() {
            return Err(AppError::BadRequest(
                "append_segment requires at least one record".to_string(),
            ));
        }

        let lease = self.ensure_lease(tenant).await?;
        let (mut manifest, manifest_etag) = self.load_manifest_with_etag(tenant).await?;

        if manifest.epoch == 0 {
            manifest = self.default_manifest(tenant, lease.epoch);
        }

        let start_seq = manifest.applied_seq + 1;
        let end_seq = start_seq + sql_records.len() as u64 - 1;
        let object_key = format!(
            "{}/{}/wal/{}/{:020}.json",
            self.cfg.prefix, tenant, lease.epoch, start_seq
        );

        let records = sql_records
            .into_iter()
            .enumerate()
            .map(
                |(i, (sql, args_json))| crate::storage::s3_wal::segment::WalRecord {
                    seq: start_seq + i as u64,
                    ts_rfc3339: unix_now_secs().to_string(),
                    sql,
                    args_json,
                },
            )
            .collect::<Vec<_>>();

        let segment = WalSegment {
            tenant: tenant.to_string(),
            epoch: lease.epoch,
            start_seq,
            end_seq,
            checksum: format!("{}-{}", start_seq, end_seq),
            records,
        };

        let raw_segment = serde_json::to_vec(&segment)
            .map_err(|e| AppError::Internal(format!("segment encode failed: {e}")))?;
        let seg_etag = self.store.put(&object_key, &raw_segment).await?;

        let manifest_segment = ManifestSegment {
            seq: start_seq,
            object_key,
            checksum: seg_etag.clone(),
            start_lsn: start_seq,
            end_lsn: end_seq,
            compressed_bytes: raw_segment.len(),
        };

        manifest.applied_seq = end_seq;
        manifest.writer_id = self.writer_id.clone();
        manifest.updated_at = unix_now_secs().to_string();
        manifest.segments.push(manifest_segment.clone());

        self.write_manifest(tenant, &manifest, manifest_etag.as_deref())
            .await?;

        Ok(manifest_segment)
    }

    pub async fn get_blob(&self, key: &str) -> AppResult<Option<Vec<u8>>> {
        Ok(self.store.get(key).await?.map(|(raw, _)| raw))
    }

    pub async fn blob_exists(&self, key: &str) -> AppResult<bool> {
        self.store.exists(key).await
    }

    pub async fn put_blob(&self, key: &str, bytes: &[u8]) -> AppResult<()> {
        let _ = self.store.put(key, bytes).await?;
        Ok(())
    }

    pub async fn delete_blob(&self, key: &str) -> AppResult<()> {
        self.store.delete(key).await
    }

    pub async fn list_keys(&self, prefix: &str) -> AppResult<Vec<String>> {
        self.store.list_prefix(prefix).await
    }

    pub async fn read_manifest(&self, tenant: &str) -> AppResult<Option<Manifest>> {
        let key = self.manifest_key(tenant);
        let Some((raw, _etag)) = self.store.get(&key).await? else {
            return Ok(None);
        };
        let manifest: Manifest = serde_json::from_slice(&raw)
            .map_err(|e| AppError::Internal(format!("manifest decode failed: {e}")))?;
        Ok(Some(manifest))
    }

    pub async fn upsert_snapshot(
        &self,
        tenant: &str,
        snapshot: SnapshotMeta,
        max_count: usize,
        max_age_days: u64,
    ) -> AppResult<(Manifest, Vec<String>)> {
        let lease = self.ensure_lease(tenant).await?;
        let (mut manifest, manifest_etag) = self.load_manifest_with_etag(tenant).await?;
        if manifest.epoch == 0 {
            manifest = self.default_manifest(tenant, lease.epoch);
        }

        manifest.current_snapshot_id = Some(snapshot.id.clone());
        manifest.current_snapshot_key = Some(snapshot.object_key.clone());
        manifest.snapshots.retain(|s| s.id != snapshot.id);
        manifest.snapshots.push(snapshot);
        manifest
            .snapshots
            .sort_by(|a, b| a.created_at.cmp(&b.created_at));

        let now = unix_now_secs();
        let max_age_secs = max_age_days.saturating_mul(86_400);
        let mut deleted_keys = Vec::new();

        if max_age_secs > 0 {
            manifest.snapshots.retain(|s| {
                let created = s.created_at.parse::<u64>().unwrap_or(now);
                let keep = now.saturating_sub(created) <= max_age_secs;
                if !keep {
                    deleted_keys.push(s.object_key.clone());
                }
                keep
            });
        }

        let capped = max_count.max(1);
        while manifest.snapshots.len() > capped {
            let removed = manifest.snapshots.remove(0);
            deleted_keys.push(removed.object_key);
        }

        if let Some(current_id) = manifest.current_snapshot_id.clone() {
            if let Some(found) = manifest.snapshots.iter().find(|s| s.id == current_id) {
                manifest.current_snapshot_key = Some(found.object_key.clone());
            } else if let Some(last) = manifest.snapshots.last() {
                manifest.current_snapshot_id = Some(last.id.clone());
                manifest.current_snapshot_key = Some(last.object_key.clone());
            } else {
                manifest.current_snapshot_id = None;
                manifest.current_snapshot_key = None;
            }
        }

        manifest.writer_id = self.writer_id.clone();
        manifest.updated_at = unix_now_secs().to_string();
        self.write_manifest(tenant, &manifest, manifest_etag.as_deref())
            .await?;
        Ok((manifest, deleted_keys))
    }

    pub async fn compact_manifest(
        &self,
        tenant: &str,
        retain_segments: usize,
    ) -> AppResult<(Manifest, usize)> {
        let lease = self.ensure_lease(tenant).await?;
        let (mut manifest, manifest_etag) = self.load_manifest_with_etag(tenant).await?;
        if manifest.epoch == 0 {
            manifest = self.default_manifest(tenant, lease.epoch);
        }

        let total_before = manifest.segments.len();
        if total_before <= retain_segments {
            return Ok((manifest, 0));
        }

        let remove_count = total_before - retain_segments;
        let removed = manifest.segments.drain(0..remove_count).collect::<Vec<_>>();
        let watermark = removed.iter().map(|s| s.end_lsn).max().unwrap_or(0);
        manifest.compaction_watermark = Some(watermark);
        manifest.writer_id = self.writer_id.clone();
        manifest.updated_at = unix_now_secs().to_string();
        self.write_manifest(tenant, &manifest, manifest_etag.as_deref())
            .await?;
        Ok((manifest, remove_count))
    }

    async fn load_or_init_manifest(&self, tenant: &str, epoch: u64) -> AppResult<Manifest> {
        let (manifest, manifest_etag) = self.load_manifest_with_etag(tenant).await?;

        if manifest.epoch != 0 {
            return Ok(manifest);
        }

        let created = self.default_manifest(tenant, epoch);
        self.write_manifest(tenant, &created, manifest_etag.as_deref())
            .await?;
        Ok(created)
    }

    async fn load_manifest_with_etag(&self, tenant: &str) -> AppResult<(Manifest, Option<String>)> {
        let key = self.manifest_key(tenant);
        let Some((raw, etag)) = self.store.get(&key).await? else {
            return Ok((self.default_manifest(tenant, 0), None));
        };

        let manifest: Manifest = serde_json::from_slice(&raw)
            .map_err(|e| AppError::Internal(format!("manifest decode failed: {e}")))?;

        Ok((manifest, Some(etag)))
    }

    async fn write_manifest(
        &self,
        tenant: &str,
        manifest: &Manifest,
        expected_etag: Option<&str>,
    ) -> AppResult<()> {
        let key = self.manifest_key(tenant);
        let raw = serde_json::to_vec(manifest)
            .map_err(|e| AppError::Internal(format!("manifest encode failed: {e}")))?;

        self.store.put_if_match(&key, &raw, expected_etag).await?;
        Ok(())
    }

    fn default_manifest(&self, tenant: &str, epoch: u64) -> Manifest {
        Manifest {
            tenant: tenant.to_string(),
            epoch,
            writer_id: self.writer_id.clone(),
            applied_seq: 0,
            current_snapshot_id: None,
            current_snapshot_key: None,
            snapshots: vec![],
            segments: vec![],
            compaction_watermark: None,
            updated_at: unix_now_secs().to_string(),
        }
    }

    fn manifest_key(&self, tenant: &str) -> String {
        format!("{}/{tenant}/{MANIFEST_FILE}", self.cfg.prefix)
    }

    fn lease_key(&self, tenant: &str) -> String {
        format!("{}/{tenant}/{LEASE_FILE}", self.cfg.prefix)
    }
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
