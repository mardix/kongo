//! Object-store adapters used by the s3 pipeline (local fs emulation and AWS S3 transport).

use std::sync::Arc;

use async_trait::async_trait;
use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use aws_sdk_s3::{Client, config::Region, error::ProvideErrorMetadata};
use tokio::time::{Duration, sleep};

use crate::{
    config::S3Credentials,
    error::{AppError, AppResult},
};

#[async_trait]
pub trait ObjectStore: Send + Sync {
    async fn exists(&self, key: &str) -> AppResult<bool>;
    async fn get(&self, key: &str) -> AppResult<Option<(Vec<u8>, String)>>;
    async fn put(&self, key: &str, data: &[u8]) -> AppResult<String>;
    async fn delete(&self, key: &str) -> AppResult<()>;
    async fn list_prefix(&self, prefix: &str) -> AppResult<Vec<String>>;
    async fn put_if_match(
        &self,
        key: &str,
        data: &[u8],
        expected_etag: Option<&str>,
    ) -> AppResult<String>;
    async fn get_range_limited(
        &self,
        key: &str,
        start: usize,
        len: usize,
    ) -> AppResult<Option<(Vec<u8>, String)>>;
}

#[derive(Clone)]
pub struct AwsS3ObjectStore {
    client: Client,
    bucket: String,
    base_prefix: String,
}

impl AwsS3ObjectStore {
    pub fn new(
        base_uri: &str,
        region: &str,
        endpoint: Option<&str>,
        credentials: Option<&S3Credentials>,
    ) -> AppResult<Self> {
        let (bucket, base_prefix) = split_s3_uri(base_uri)?;
        let mut builder = aws_sdk_s3::config::Builder::new()
            .behavior_version_latest()
            .region(Region::new(region.to_string()));
        if let Some(endpoint) = endpoint {
            if !endpoint.trim().is_empty() {
                builder = builder.endpoint_url(endpoint.to_string());
            }
        }
        if let Some(c) = credentials {
            let creds = Credentials::new(
                c.access_key.clone(),
                c.secret_key.clone(),
                c.session_token.clone(),
                None,
                "kongodb-static",
            );
            builder = builder.credentials_provider(SharedCredentialsProvider::new(creds));
        }

        let client = Client::from_conf(builder.build());
        Ok(Self {
            client,
            bucket,
            base_prefix,
        })
    }

    fn object_key(&self, key: &str) -> String {
        if self.base_prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}/{}", self.base_prefix.trim_end_matches('/'), key)
        }
    }

    fn normalize_etag(etag: Option<&str>) -> String {
        etag.unwrap_or("").trim_matches('"').to_string()
    }

    fn error_code_message<E: ProvideErrorMetadata>(err: &E) -> (String, String) {
        let code = err.code().unwrap_or("Unknown").to_string();
        let message = err.message().unwrap_or("no error message").to_string();
        (code, message)
    }

    pub async fn head_source_hash(&self, key: &str) -> AppResult<Option<String>> {
        let object_key = self.object_key(key);
        let out = match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(object_key.as_str())
            .send()
            .await
        {
            Ok(v) => v,
            Err(aws_sdk_s3::error::SdkError::ServiceError(se)) => {
                let (code, message) = Self::error_code_message(se.err());
                if code == "NotFound" || code == "NoSuchKey" || message.contains("404") {
                    return Ok(None);
                }
                return Err(AppError::Internal(format!(
                    "s3 head_object failed: code={code} message={message} bucket={} key={}",
                    self.bucket, object_key
                )));
            }
            Err(e) => {
                return Err(AppError::Internal(format!(
                    "s3 head_object transport/config error: bucket={} key={} err={}",
                    self.bucket, object_key, e
                )));
            }
        };

        let map = out.metadata();
        let picked = map
            .and_then(|m| m.get("source-hash").or_else(|| m.get("source_hash")))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        Ok(picked)
    }

    pub async fn get_range(&self, key: &str, start: usize) -> AppResult<Option<(Vec<u8>, String)>> {
        let object_key = self.object_key(key);
        let out = match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(object_key.as_str())
            .range(format!("bytes={start}-"))
            .send()
            .await
        {
            Ok(v) => v,
            Err(aws_sdk_s3::error::SdkError::ServiceError(se)) => {
                let (code, message) = Self::error_code_message(se.err());
                if code == "NoSuchKey" || code == "NotFound" || message.contains("404") {
                    return Ok(None);
                }
                return Err(AppError::Internal(format!(
                    "s3 get_object range failed: code={code} message={message} bucket={} key={} range=bytes={start}-",
                    self.bucket, object_key
                )));
            }
            Err(e) => {
                return Err(AppError::Internal(format!(
                    "s3 get_object range transport/config error: bucket={} key={} err={}",
                    self.bucket, object_key, e
                )));
            }
        };

        let bytes = out
            .body
            .collect()
            .await
            .map_err(|e| AppError::Internal(format!("s3 body read failed: {e}")))?
            .into_bytes()
            .to_vec();
        let etag = Self::normalize_etag(out.e_tag.as_deref());
        Ok(Some((bytes, etag)))
    }
}

#[async_trait]
impl ObjectStore for AwsS3ObjectStore {
    async fn exists(&self, key: &str) -> AppResult<bool> {
        let object_key = self.object_key(key);
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(object_key.as_str())
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(aws_sdk_s3::error::SdkError::ServiceError(se)) => {
                let (code, message) = Self::error_code_message(se.err());
                if code == "NotFound" || code == "NoSuchKey" || message.contains("404") {
                    Ok(false)
                } else {
                    Err(AppError::Internal(format!(
                        "s3 head_object failed: code={code} message={message} bucket={} key={}",
                        self.bucket, object_key
                    )))
                }
            }
            Err(e) => Err(AppError::Internal(format!(
                "s3 head_object transport/config error: bucket={} key={} err={}",
                self.bucket, object_key, e
            ))),
        }
    }

    async fn get(&self, key: &str) -> AppResult<Option<(Vec<u8>, String)>> {
        let object_key = self.object_key(key);
        let mut attempt: usize = 0;
        let out = loop {
            let req = self
                .client
                .get_object()
                .bucket(&self.bucket)
                .key(object_key.as_str())
                .send()
                .await;
            match req {
                Ok(v) => break v,
                Err(aws_sdk_s3::error::SdkError::ServiceError(se)) => {
                    let (code, message) = Self::error_code_message(se.err());
                    if code == "NoSuchKey" || code == "NotFound" || message.contains("404") {
                        return Ok(None);
                    }
                    return Err(AppError::Internal(format!(
                        "s3 get_object failed: code={code} message={message} bucket={} key={}",
                        self.bucket, object_key
                    )));
                }
                Err(e) => {
                    let msg = e.to_string();
                    let retryable = msg.to_ascii_lowercase().contains("dispatch failure");
                    if retryable && attempt < 2 {
                        attempt += 1;
                        sleep(Duration::from_millis(80 * attempt as u64)).await;
                        continue;
                    }
                    return Err(AppError::Internal(format!(
                        "s3 get_object transport/config error: bucket={} key={} err={}",
                        self.bucket, object_key, e
                    )));
                }
            }
        };

        let bytes = out
            .body
            .collect()
            .await
            .map_err(|e| AppError::Internal(format!("s3 body read failed: {e}")))?
            .into_bytes()
            .to_vec();
        let etag = Self::normalize_etag(out.e_tag.as_deref());
        Ok(Some((bytes, etag)))
    }

    async fn put(&self, key: &str, data: &[u8]) -> AppResult<String> {
        let object_key = self.object_key(key);
        let out = match self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(object_key.as_str())
            .body(aws_sdk_s3::primitives::ByteStream::from(data.to_vec()))
            .send()
            .await
        {
            Ok(v) => v,
            Err(aws_sdk_s3::error::SdkError::ServiceError(se)) => {
                let (code, message) = Self::error_code_message(se.err());
                return Err(AppError::Internal(format!(
                    "s3 put_object failed: code={code} message={message} bucket={} key={}",
                    self.bucket, object_key
                )));
            }
            Err(e) => {
                return Err(AppError::Internal(format!(
                    "s3 put_object transport/config error: bucket={} key={} err={}",
                    self.bucket, object_key, e
                )));
            }
        };
        Ok(Self::normalize_etag(out.e_tag.as_deref()))
    }

    async fn get_range_limited(
        &self,
        key: &str,
        start: usize,
        len: usize,
    ) -> AppResult<Option<(Vec<u8>, String)>> {
        if len == 0 {
            return Ok(Some((Vec::new(), String::new())));
        }
        let end = start.saturating_add(len.saturating_sub(1));
        let object_key = self.object_key(key);
        let out = match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(object_key.as_str())
            .range(format!("bytes={start}-{end}"))
            .send()
            .await
        {
            Ok(v) => v,
            Err(aws_sdk_s3::error::SdkError::ServiceError(se)) => {
                let (code, message) = Self::error_code_message(se.err());
                if code == "NoSuchKey"
                    || code == "NotFound"
                    || code == "InvalidRange"
                    || message.contains("404")
                    || message.contains("416")
                {
                    return Ok(None);
                }
                return Err(AppError::Internal(format!(
                    "s3 get_object range failed: code={code} message={message} bucket={} key={} range=bytes={start}-{end}",
                    self.bucket, object_key
                )));
            }
            Err(e) => {
                return Err(AppError::Internal(format!(
                    "s3 get_object range transport/config error: bucket={} key={} err={}",
                    self.bucket, object_key, e
                )));
            }
        };

        let bytes = out
            .body
            .collect()
            .await
            .map_err(|e| AppError::Internal(format!("s3 body read failed: {e}")))?
            .into_bytes()
            .to_vec();
        let etag = Self::normalize_etag(out.e_tag.as_deref());
        Ok(Some((bytes, etag)))
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        let object_key = self.object_key(key);
        match self
            .client
            .delete_object()
            .bucket(&self.bucket)
            .key(object_key.as_str())
            .send()
            .await
        {
            Ok(_) => Ok(()),
            Err(aws_sdk_s3::error::SdkError::ServiceError(se)) => {
                let (code, message) = Self::error_code_message(se.err());
                if code == "NoSuchKey" || code == "NotFound" || message.contains("404") {
                    return Ok(());
                }
                Err(AppError::Internal(format!(
                    "s3 delete_object failed: code={code} message={message} bucket={} key={}",
                    self.bucket, object_key
                )))
            }
            Err(e) => Err(AppError::Internal(format!(
                "s3 delete_object transport/config error: bucket={} key={} err={}",
                self.bucket, object_key, e
            ))),
        }
    }

    async fn list_prefix(&self, prefix: &str) -> AppResult<Vec<String>> {
        let object_prefix = self.object_key(prefix);
        let mut continuation: Option<String> = None;
        let mut out = Vec::<String>::new();

        loop {
            let mut req = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(object_prefix.as_str());
            if let Some(token) = continuation.as_ref() {
                req = req.continuation_token(token);
            }
            let resp = req.send().await.map_err(|e| {
                AppError::Internal(format!(
                    "s3 list_objects_v2 failed: bucket={} prefix={} err={e}",
                    self.bucket, object_prefix
                ))
            })?;

            for item in resp.contents() {
                if let Some(key) = item.key() {
                    if self.base_prefix.is_empty() {
                        out.push(key.to_string());
                    } else {
                        let base = self.base_prefix.trim_end_matches('/');
                        if let Some(stripped) = key.strip_prefix(base) {
                            out.push(stripped.trim_start_matches('/').to_string());
                        } else {
                            out.push(key.to_string());
                        }
                    }
                }
            }

            if resp.is_truncated().unwrap_or(false) {
                continuation = resp
                    .next_continuation_token()
                    .map(|s| s.to_string())
                    .or_else(|| continuation.take());
                if continuation.is_none() {
                    break;
                }
            } else {
                break;
            }
        }

        Ok(out)
    }

    async fn put_if_match(
        &self,
        key: &str,
        data: &[u8],
        expected_etag: Option<&str>,
    ) -> AppResult<String> {
        let object_key = self.object_key(key);
        let mut req = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(object_key.as_str())
            .body(aws_sdk_s3::primitives::ByteStream::from(data.to_vec()));

        if let Some(etag) = expected_etag {
            req = req.if_match(etag.to_string());
        }

        let out = match req.send().await {
            Ok(v) => v,
            Err(aws_sdk_s3::error::SdkError::ServiceError(se)) => {
                let (code, message) = Self::error_code_message(se.err());
                return Err(AppError::Conflict(format!(
                    "s3 conditional put failed: code={code} message={message} bucket={} key={}",
                    self.bucket, object_key
                )));
            }
            Err(e) => {
                return Err(AppError::Conflict(format!(
                    "s3 conditional put transport/config error: bucket={} key={} err={}",
                    self.bucket, object_key, e
                )));
            }
        };
        Ok(Self::normalize_etag(out.e_tag.as_deref()))
    }
}

pub fn split_s3_uri(uri: &str) -> AppResult<(String, String)> {
    let without = uri
        .strip_prefix("s3://")
        .ok_or_else(|| AppError::BadRequest("s3 uri must start with s3://".to_string()))?;
    let mut parts = without.splitn(2, '/');
    let bucket = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest("s3 uri missing bucket".to_string()))?;
    let prefix = parts
        .next()
        .unwrap_or_default()
        .trim_matches('/')
        .to_string();
    Ok((bucket.to_string(), prefix))
}

pub fn as_arc<T: ObjectStore + 'static>(store: T) -> Arc<dyn ObjectStore> {
    Arc::new(store)
}
