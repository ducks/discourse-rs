//! Object storage: the remote half of the stores. `FileStore` (uploads)
//! and `BackupStore` (backups) keep their files either on this machine or
//! in a bucket of an object store, through a [`Backend`]; nothing outside
//! those two stores knows which backend is in use.
//!
//! A backend is a bucket and what can be done with its objects: put, get,
//! head, copy, delete and list, URLs signed for a browser to get or put an
//! object directly, multipart uploads a browser sends part by part, and an
//! expiry rule for a prefix. The S3 API backend ([`s3`]) speaks to any
//! S3-compatible service (AWS, Cloudflare R2, Backblaze B2, MinIO, Wasabi,
//! DigitalOcean Spaces, Garage). A service with an API of its own (Azure
//! Blob, Google Cloud Storage) is another variant of [`Backend`]: the
//! stores and their callers stay as they are.

pub mod s3;

use std::collections::BTreeMap;
use std::path::Path;

use chrono::NaiveDateTime;

/// What the store knows of an object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMeta {
    /// The object's key in the bucket.
    pub key: String,
    pub size: u64,
    /// Without the quotes S3 sends it in.
    pub etag: String,
    pub last_modified: NaiveDateTime,
    /// User metadata (`x-amz-meta-*` on S3), keys lowercase without the
    /// prefix; empty from a listing.
    pub metadata: BTreeMap<String, String>,
    /// The headers it is served with (none from a listing).
    pub content_type: Option<String>,
    pub cache_control: Option<String>,
    pub content_disposition: Option<String>,
}

/// How an object is written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PutOptions {
    pub content_type: Option<String>,
    pub cache_control: Option<String>,
    pub content_disposition: Option<String>,
    /// A canned ACL (`public-read`, `private`), for the services that keep
    /// them (`s3_use_acls`).
    pub acl: Option<String>,
    /// Object tags, form encoded (`s3_enable_access_control_tags`).
    pub tagging: Option<String>,
    /// User metadata, keys without the prefix.
    pub metadata: Vec<(String, String)>,
}

/// A CORS rule a bucket needs for browsers to reach it directly (S3CorsRulesets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorsRule {
    pub allowed_headers: Vec<String>,
    pub allowed_methods: Vec<String>,
    pub allowed_origins: Vec<String>,
    pub expose_headers: Vec<String>,
    pub max_age_seconds: Option<u32>,
}

impl CorsRule {
    /// `S3CorsRulesets::BACKUP_DIRECT_UPLOAD`
    pub fn backup_direct_upload() -> CorsRule {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect();
        CorsRule {
            allowed_headers: s(&["*"]),
            allowed_methods: s(&["GET", "HEAD", "PUT"]),
            allowed_origins: s(&["*"]),
            expose_headers: s(&["ETag"]),
            max_age_seconds: Some(3000),
        }
    }

    /// `S3CorsRulesets::DIRECT_UPLOAD`
    pub fn direct_upload() -> CorsRule {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect();
        CorsRule {
            allowed_headers: s(&[
                "Authorization",
                "Content-Disposition",
                "Content-Type",
                "X-Amz-Acl",
                "X-Amz-Meta-Sha1-Checksum",
                "X-Amz-Tagging",
            ]),
            allowed_methods: s(&["GET", "HEAD", "PUT"]),
            allowed_origins: s(&["*"]),
            expose_headers: s(&["ETag"]),
            max_age_seconds: Some(3000),
        }
    }
}

/// One rule of a bucket's lifecycle: objects under `prefix` expire
/// `days` after they were written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleRule {
    pub id: Option<String>,
    pub status: String,
    pub days: Option<i64>,
    pub prefix: Option<String>,
}

/// One uploaded part of a multipart upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub number: u16,
    pub etag: String,
}

/// Why an operation failed.
#[derive(Debug)]
pub enum StorageError {
    /// The object is not there.
    NotFound,
    /// The multipart upload is not there (`NoSuchUpload`).
    NoSuchUpload,
    /// The service refused: its HTTP status, error code and message.
    Service {
        status: u16,
        code: String,
        message: String,
    },
    /// The service could not be reached, or answered nonsense.
    Transport(String),
    /// A configuration the port does not handle yet.
    Unsupported(crate::Unsupported),
}

impl StorageError {
    /// The service's error code (`NoSuchBucket`, `AccessDenied`, ...).
    pub fn code(&self) -> &str {
        match self {
            StorageError::NotFound => "NotFound",
            StorageError::NoSuchUpload => "NoSuchUpload",
            StorageError::Service { code, .. } => code,
            StorageError::Transport(_) | StorageError::Unsupported(_) => "",
        }
    }
}

impl std::fmt::Display for StorageError {
    /// As aws-sdk's errors read: the service's message, else the error's
    /// class name (`e.message.presence || e.class.name`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::NotFound => f.write_str("Aws::S3::Errors::NotFound"),
            StorageError::NoSuchUpload => {
                f.write_str("The specified multipart upload does not exist.")
            }
            StorageError::Service { code, message, .. } => {
                if message.is_empty() {
                    write!(f, "Aws::S3::Errors::{code}")
                } else {
                    f.write_str(message)
                }
            }
            StorageError::Transport(message) => f.write_str(message),
            StorageError::Unsupported(u) => u.fmt(f),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<crate::Unsupported> for StorageError {
    fn from(u: crate::Unsupported) -> Self {
        StorageError::Unsupported(u)
    }
}

impl From<std::io::Error> for StorageError {
    fn from(e: std::io::Error) -> Self {
        StorageError::Transport(e.to_string())
    }
}

/// A bucket of an object store.
#[derive(Debug, Clone)]
pub enum Backend {
    S3(s3::S3),
}

impl Backend {
    /// The bucket's name.
    pub fn bucket(&self) -> &str {
        match self {
            Backend::S3(s) => s.bucket(),
        }
    }

    /// The bucket made when it is not there yet (S3Helper#s3_bucket).
    pub async fn ensure_bucket(&self) -> Result<(), StorageError> {
        match self {
            Backend::S3(s) => s.ensure_bucket().await,
        }
    }

    /// An object written whole; its etag.
    pub async fn put(
        &self,
        key: &str,
        body: Vec<u8>,
        opts: &PutOptions,
    ) -> Result<String, StorageError> {
        match self {
            Backend::S3(s) => s.put(key, body, opts).await,
        }
    }

    /// A local file written as an object, in parts when it is large.
    pub async fn put_file(
        &self,
        key: &str,
        path: &Path,
        opts: &PutOptions,
    ) -> Result<String, StorageError> {
        match self {
            Backend::S3(s) => s.put_file(key, path, opts).await,
        }
    }

    /// An object's bytes; None when it is not there.
    pub async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        match self {
            Backend::S3(s) => s.get(key).await,
        }
    }

    /// An object written to a local file.
    pub async fn download(&self, key: &str, destination: &Path) -> Result<(), StorageError> {
        match self {
            Backend::S3(s) => s.download(key, destination).await,
        }
    }

    /// An object's size, etag, date and metadata; None when it is not
    /// there.
    pub async fn head(&self, key: &str) -> Result<Option<ObjectMeta>, StorageError> {
        match self {
            Backend::S3(s) => s.head(key).await,
        }
    }

    /// An object deleted (none there is fine).
    pub async fn delete(&self, key: &str) -> Result<(), StorageError> {
        match self {
            Backend::S3(s) => s.delete(key).await,
        }
    }

    /// An object copied within the bucket; with options, the copy takes
    /// them instead of the source's metadata. The copy's etag.
    pub async fn copy(
        &self,
        source: &str,
        destination: &str,
        replace: Option<&PutOptions>,
    ) -> Result<String, StorageError> {
        match self {
            Backend::S3(s) => s.copy(source, destination, replace).await,
        }
    }

    /// The objects under a prefix, in key order.
    pub async fn list(&self, prefix: &str) -> Result<Vec<ObjectMeta>, StorageError> {
        match self {
            Backend::S3(s) => s.list(prefix).await,
        }
    }

    /// A URL that gets the object for `expires_in` seconds, served with
    /// that Content-Disposition when one is given.
    pub fn presigned_get(
        &self,
        key: &str,
        expires_in: u64,
        response_content_disposition: Option<&str>,
    ) -> Result<String, StorageError> {
        match self {
            Backend::S3(s) => s.presigned_get(key, expires_in, response_content_disposition),
        }
    }

    /// A URL a browser puts the object at, for `expires_in` seconds, and
    /// the headers it must send with it.
    pub fn presigned_put(
        &self,
        key: &str,
        expires_in: u64,
        opts: &PutOptions,
    ) -> Result<(String, BTreeMap<String, String>), StorageError> {
        match self {
            Backend::S3(s) => s.presigned_put(key, expires_in, opts),
        }
    }

    /// A multipart upload started; its id.
    pub async fn create_multipart(
        &self,
        key: &str,
        opts: &PutOptions,
    ) -> Result<String, StorageError> {
        match self {
            Backend::S3(s) => s.create_multipart(key, opts).await,
        }
    }

    /// A URL a browser puts one part at.
    pub fn presign_part(
        &self,
        key: &str,
        upload_id: &str,
        part_number: u16,
        expires_in: u64,
    ) -> Result<String, StorageError> {
        match self {
            Backend::S3(s) => s.presign_part(key, upload_id, part_number, expires_in),
        }
    }

    /// The parts uploaded so far (up to `max_parts`); NoSuchUpload when
    /// the upload is gone.
    pub async fn list_parts(
        &self,
        key: &str,
        upload_id: &str,
        max_parts: u16,
    ) -> Result<Vec<Part>, StorageError> {
        match self {
            Backend::S3(s) => s.list_parts(key, upload_id, max_parts).await,
        }
    }

    /// A multipart upload completed from its parts.
    pub async fn complete_multipart(
        &self,
        key: &str,
        upload_id: &str,
        parts: &[Part],
    ) -> Result<(), StorageError> {
        match self {
            Backend::S3(s) => s.complete_multipart(key, upload_id, parts).await,
        }
    }

    /// A multipart upload abandoned, its parts dropped.
    pub async fn abort_multipart(&self, key: &str, upload_id: &str) -> Result<(), StorageError> {
        match self {
            Backend::S3(s) => s.abort_multipart(key, upload_id).await,
        }
    }

    /// The CORS rule added unless the bucket has one like it; whether it
    /// was written.
    pub async fn ensure_cors(&self, rule: &CorsRule) -> Result<bool, StorageError> {
        match self {
            Backend::S3(s) => s.ensure_cors(rule).await,
        }
    }

    /// The bucket's expiry rules.
    pub async fn lifecycle_rules(&self) -> Result<Vec<LifecycleRule>, StorageError> {
        match self {
            Backend::S3(s) => s.lifecycle_rules().await,
        }
    }

    /// Objects under `prefix` expire `days` after they were written (the
    /// rule `id` replaced, the bucket's others kept).
    pub async fn update_lifecycle(
        &self,
        id: &str,
        days: i64,
        prefix: &str,
    ) -> Result<(), StorageError> {
        match self {
            Backend::S3(s) => s.update_lifecycle(id, days, prefix).await,
        }
    }
}
