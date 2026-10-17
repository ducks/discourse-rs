//! FileStore (lib/file_store): where uploads and their thumbnails are kept
//! and the urls they are served at. `Discourse.store` picks the store for
//! the site: the local store (files under the public directory, served at
//! `/uploads`) or, with `enable_s3_uploads` or the S3 global settings, the
//! external store (S3Store), whose files are objects in a bucket of an
//! object storage backend ([`crate::object_storage`]). Callers see the
//! store; which backend holds the objects is the store's business.

use std::path::PathBuf;

use crate::config::Config;
use crate::object_storage::{Backend, PutOptions, StorageError, s3};
use crate::site_settings::{SettingError, SiteSettings};
use crate::url::{UrlError, Urls};
use crate::{Unsupported, ruby};

/// Why the store cannot be had, or failed.
#[derive(Debug)]
pub enum StoreError {
    Setting(SettingError),
    Url(UrlError),
    Unsupported(Unsupported),
    /// `Discourse::SiteSettingMissing` / `S3Helper::SettingMissing`: a
    /// setting the external store needs is blank.
    SettingMissing(&'static str),
    Storage(StorageError),
    Io(std::io::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Setting(e) => e.fmt(f),
            StoreError::Url(e) => e.fmt(f),
            StoreError::Unsupported(e) => e.fmt(f),
            StoreError::SettingMissing(name) => write!(f, "{name}"),
            StoreError::Storage(e) => e.fmt(f),
            StoreError::Io(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<SettingError> for StoreError {
    fn from(e: SettingError) -> Self {
        StoreError::Setting(e)
    }
}

impl From<UrlError> for StoreError {
    fn from(e: UrlError) -> Self {
        StoreError::Url(e)
    }
}

impl From<StorageError> for StoreError {
    fn from(e: StorageError) -> Self {
        match e {
            StorageError::Unsupported(u) => StoreError::Unsupported(u),
            e => StoreError::Storage(e),
        }
    }
}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e)
    }
}

impl From<Unsupported> for StoreError {
    fn from(e: Unsupported) -> Self {
        StoreError::Unsupported(e)
    }
}

/// `FileStore::BaseStore#get_path_for(type, id, sha, extension)`: the path
/// within the store, a directory level deeper per 16x more ids.
pub fn get_path_for(kind: &str, id: i32, sha1: &str, extension: &str) -> String {
    let depth = if id > 0 {
        ((f64::from(id) / 1000.0).ln() / 16f64.ln()).ceil().max(0.0) as usize
    } else {
        0
    };
    let tree: String = sha1.chars().take(depth).map(|c| format!("{c}/")).collect();
    format!("{kind}/{}X/{tree}{sha1}{extension}", depth + 1)
}

/// `BaseStore::UPLOAD_PATH_REGEX` and `OPTIMIZED_IMAGE_PATH_REGEX`: the
/// store path in a stored url (`original/1X/...`, `optimized/2X/...`).
fn store_path_in(url: &str) -> Option<&str> {
    for kind in ["/original/", "/optimized/"] {
        let mut from = 0;
        while let Some(at) = url[from..].find(kind) {
            let start = from + at + 1;
            let rest = &url[start + kind.len() - 1..];
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            if digits > 0 && rest[digits..].starts_with("X/") {
                return Some(&url[start..]);
            }
            from = start;
        }
    }
    None
}

/// What storing a file gives: its url, and its etag on an external store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub url: String,
    pub etag: Option<String>,
}

/// The site's store (`Discourse.store`).
#[derive(Debug, Clone)]
pub enum FileStore {
    Local(LocalStore),
    External(Box<ExternalStore>),
}

/// `FileStore::LocalStore`: files under the public directory.
#[derive(Debug, Clone)]
pub struct LocalStore {
    public_dir: PathBuf,
    /// `RailsMultisite::ConnectionManagement.current_db`
    db: String,
    /// `Discourse.base_path`
    base_path: String,
    /// `relative_base_url`: the base path and `uploads/<db>`.
    relative_base_url: String,
    /// `absolute_base_url`
    absolute_base_url: String,
    /// `absolute_base_cdn_url` (the CDN's, when there is one).
    absolute_base_cdn_url: Option<String>,
    /// `SiteSetting.scheme`
    scheme: String,
}

/// `FileStore::S3Store`: objects in a bucket, at urls of the bucket's
/// host (`//<bucket>.<endpoint host>/<path>`) or of `s3_cdn_url`.
#[derive(Debug, Clone)]
pub struct ExternalStore {
    backend: Backend,
    /// The folder of `s3_upload_bucket` (`bucket/folder`), if any.
    folder: Option<String>,
    /// `SiteSetting::Upload.absolute_base_url`: `//<bucket>.<host>`.
    absolute_base_url: String,
    /// `SiteSetting::Upload.s3_cdn_url`
    s3_cdn_url: Option<String>,
    /// `s3_use_cdn_url_for_all_uploads`
    use_cdn_url_for_all_uploads: bool,
    /// `Rails.configuration.multisite`: paths under `uploads/<db>/`.
    multisite: bool,
    /// The tombstone prefix, the folder included.
    tombstone_prefix: String,
    /// `s3_configure_tombstone_policy`
    configure_tombstone_policy: bool,
    /// `Cache-Control` of stored objects.
    cache_control: String,
    /// `s3_use_acls`
    use_acls: bool,
    /// `s3_enable_access_control_tags`: the tag key and its public and
    /// private values.
    access_control_tag: Option<(String, String, String)>,
    /// `s3_presigned_get_url_expires_after_seconds`
    presigned_get_expires: u64,
    /// For relative urls (uploads still on this machine).
    local: LocalStore,
}

impl FileStore {
    /// `Discourse.store` for the site: the external store when S3 uploads
    /// are on (the site setting, or the S3 global settings), else the
    /// local store.
    pub fn for_site(config: &Config, settings: &SiteSettings) -> Result<FileStore, StoreError> {
        let local = LocalStore::for_site(config, settings)?;
        if !settings.get("enable_s3_uploads")?.truthy() && !config.globals.use_s3() {
            return Ok(FileStore::Local(local));
        }
        Ok(FileStore::External(Box::new(ExternalStore::for_site(
            config, settings, local,
        )?)))
    }

    /// `FileStore::LocalStore.new` whatever the site's store is: where
    /// relative urls' files are.
    pub fn local_for_site(
        config: &Config,
        settings: &SiteSettings,
    ) -> Result<FileStore, StoreError> {
        Ok(FileStore::Local(LocalStore::for_site(config, settings)?))
    }

    /// `cdn_url`: an upload's url behind the CDN (UrlHelper.local_cdn_url
    /// for the local store, `s3_cdn_url` for the external one), as it is
    /// without one.
    pub fn cdn_url(&self, url: &str) -> String {
        match self {
            FileStore::Local(s) => s.cdn_url(url),
            FileStore::External(s) => s.cdn_url(url),
        }
    }

    /// A local store for `base_url` (`http://host`), no base path or CDN.
    #[cfg(test)]
    pub(crate) fn test_local(base_url: &str) -> FileStore {
        let relative_base_url = "/uploads/default".to_string();
        FileStore::Local(LocalStore {
            public_dir: PathBuf::from("public"),
            db: "default".into(),
            base_path: String::new(),
            absolute_base_url: format!("{base_url}{relative_base_url}"),
            absolute_base_cdn_url: None,
            scheme: base_url.split(':').next().unwrap_or("http").to_string(),
            relative_base_url,
        })
    }

    /// Whether the store's urls go through a CDN (the local store's
    /// `cdn_url`, the external store's `s3_cdn_url`).
    pub fn has_cdn(&self) -> bool {
        match self {
            FileStore::Local(s) => s.absolute_base_cdn_url.is_some(),
            FileStore::External(s) => s.s3_cdn_url.is_some(),
        }
    }

    /// `external?`
    pub fn external(&self) -> bool {
        matches!(self, FileStore::External(_))
    }

    /// `internal?`
    pub fn internal(&self) -> bool {
        !self.external()
    }

    fn local(&self) -> &LocalStore {
        match self {
            FileStore::Local(s) => s,
            FileStore::External(s) => &s.local,
        }
    }

    /// `RailsMultisite::ConnectionManagement.current_db`: the site whose
    /// uploads these are.
    pub fn db(&self) -> &str {
        &self.local().db
    }

    /// `relative_base_url` of the local store (`<base path>/uploads/<db>`),
    /// where a site's uploads kept on this machine are served.
    pub fn relative_base_url(&self) -> &str {
        &self.local().relative_base_url
    }

    /// `absolute_base_url`: the local store's `http://host/uploads/<db>`,
    /// the external store's `//<bucket>.<host>`.
    pub fn absolute_base_url(&self) -> &str {
        match self {
            FileStore::Local(s) => &s.absolute_base_url,
            FileStore::External(s) => &s.absolute_base_url,
        }
    }

    /// `get_path_for(type, id, sha, extension)` as a local url
    /// (`/uploads/<db>/...`).
    pub fn path_for_kind(&self, kind: &str, id: i32, sha1: &str, extension: &str) -> String {
        format!(
            "/uploads/{}/{}",
            self.db(),
            get_path_for(kind, id, sha1, extension)
        )
    }

    /// `store_upload`: the original, at its path, named and typed by its
    /// filename on an external store.
    pub async fn store_upload(
        &self,
        bytes: &[u8],
        upload_id: i32,
        sha1: &str,
        extension: &str,
        original_filename: &str,
    ) -> Result<Stored, StoreError> {
        match self {
            FileStore::Local(s) => {
                let path = self.path_for_kind("original", upload_id, sha1, extension);
                Ok(Stored {
                    url: s.store_file(bytes, &path).await?,
                    etag: None,
                })
            }
            FileStore::External(s) => {
                let path = get_path_for("original", upload_id, sha1, extension);
                s.store_file(bytes, &path, Some(original_filename)).await
            }
        }
    }

    /// `store_optimized_image`: a thumbnail, named for its upload's sha1,
    /// version and size.
    pub async fn store_optimized_image(
        &self,
        bytes: &[u8],
        upload_id: i32,
        upload_sha1: &str,
        suffix: &str,
    ) -> Result<Stored, StoreError> {
        match self {
            FileStore::Local(s) => {
                let path = self.path_for_kind("optimized", upload_id, upload_sha1, suffix);
                Ok(Stored {
                    url: s.store_file(bytes, &path).await?,
                    etag: None,
                })
            }
            FileStore::External(s) => {
                let path = get_path_for("optimized", upload_id, upload_sha1, suffix);
                s.store_file(bytes, &path, None).await
            }
        }
    }

    /// `path_for(upload)`: where a relative url's file lives on this
    /// machine (it may not exist), None for any other url.
    pub fn path_for(&self, url: &str) -> Option<std::path::PathBuf> {
        let local = self.local();
        (url.starts_with('/') && !url.starts_with("//"))
            .then(|| local.public_dir.join(url.trim_start_matches('/')))
    }

    /// The stored file of a url: read from this machine for a relative
    /// url, from the bucket for one of the external store's; None when the
    /// url is neither or the file is missing.
    pub async fn read(&self, url: &str) -> Result<Option<Vec<u8>>, StoreError> {
        if let Some(path) = self.path_for(url) {
            return match tokio::fs::read(&path).await {
                Ok(bytes) => Ok(Some(bytes)),
                // Rails' public/images holds the stock site images.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    Ok(crate::site_icons::stock_image(url).map(<[u8]>::to_vec))
                }
                Err(e) => Err(e.into()),
            };
        }
        match self {
            FileStore::Local(_) => Ok(None),
            FileStore::External(s) => match s.key_of(url) {
                Some(key) => Ok(s.backend.get(&key).await?),
                None => Ok(None),
            },
        }
    }

    /// The external store, for what only it does (direct uploads).
    pub fn as_external(&self) -> Option<&ExternalStore> {
        match self {
            FileStore::Local(_) => None,
            FileStore::External(s) => Some(s),
        }
    }

    /// `S3Store#move_existing_stored_upload`: an object a browser put at
    /// `existing_key` copied where the upload belongs (with the store's
    /// headers for its filename), then deleted.
    pub async fn move_existing_stored_upload(
        &self,
        existing_key: &str,
        upload_id: i32,
        sha1: &str,
        extension: &str,
        original_filename: &str,
    ) -> Result<Stored, StoreError> {
        let FileStore::External(s) = self else {
            return Err(Unsupported("moving an upload within the local store").into());
        };
        let path = get_path_for("original", upload_id, sha1, extension);
        let key = s.s3_path(&s.multisite_path(&path));
        let opts = s.put_options(original_filename, false);
        s.backend.ensure_bucket().await?;
        let etag = s.backend.copy(existing_key, &key, Some(&opts)).await?;
        s.backend.delete(existing_key).await?;
        Ok(Stored {
            url: format!("{}/{key}", s.absolute_base_url),
            etag: Some(etag),
        })
    }

    /// `S3Store#delete_file`: an object deleted outright, no tombstone.
    pub async fn delete_file(&self, key: &str) -> Result<(), StoreError> {
        match self {
            FileStore::Local(_) => Ok(()),
            FileStore::External(s) => {
                s.backend.ensure_bucket().await?;
                Ok(s.backend.delete(key).await?)
            }
        }
    }

    /// `remove_file`: the local store moves the file to the tombstone
    /// (`/uploads/` -> `/uploads/tombstone/`, replacing an older copy) and
    /// touches it, where purge_tombstone later finds it; the external one
    /// copies the object under its tombstone prefix and deletes it.
    /// Nothing for a url not the store's or a file already gone.
    pub async fn remove_file(&self, url: &str) -> Result<(), StoreError> {
        match self {
            FileStore::Local(s) => Ok(s.remove_file(url).await?),
            FileStore::External(s) => s.remove_file(url).await,
        }
    }

    /// `purge_tombstone(grace_period)`: the local store deletes tombstoned
    /// files over `days` old; the external one has the bucket expire them
    /// (`s3_configure_tombstone_policy`).
    pub async fn purge_tombstone(&self, days: i64) -> Result<(), StoreError> {
        match self {
            FileStore::Local(s) => Ok(s.purge_tombstone(days).await?),
            FileStore::External(s) => {
                if !s.configure_tombstone_policy || s.tombstone_prefix.is_empty() {
                    return Ok(());
                }
                s.backend.ensure_bucket().await?;
                s.backend
                    .update_lifecycle("purge_tombstone", days, &s.tombstone_prefix)
                    .await?;
                Ok(())
            }
        }
    }

    /// `has_been_uploaded?`: a url of this store, relative or absolute
    /// (protocol-relative ones take the site's scheme), or its CDN's.
    pub fn has_been_uploaded(&self, url: &str) -> bool {
        match self {
            FileStore::Local(s) => s.has_been_uploaded(url),
            FileStore::External(s) => s.has_been_uploaded(url),
        }
    }

    /// `download_url(upload)`: the local store's `<relative base>/<sha1>`,
    /// the external store's `short_path?dl=1`.
    pub fn download_url(&self, sha1: &str, extension: Option<&str>) -> String {
        match self {
            FileStore::Local(s) => format!("{}/{sha1}", s.relative_base_url),
            FileStore::External(s) => {
                format!("{}?dl=1", short_path(&s.local.base_path, sha1, extension))
            }
        }
    }

    /// `url_for(upload, force_download:)` on the external store: a
    /// presigned download when forced, the CDN's url when every upload
    /// goes through it, else the upload's url. The local store serves its
    /// files itself (None).
    pub fn url_for(
        &self,
        url: &str,
        original_filename: &str,
        force_download: bool,
    ) -> Result<Option<String>, StoreError> {
        let FileStore::External(s) = self else {
            return Ok(None);
        };
        if force_download {
            let Some(key) = s.key_of(url) else {
                return Ok(Some(url.to_string()));
            };
            let disposition = content_disposition("attachment", original_filename);
            return Ok(Some(s.backend.presigned_get(
                &key,
                s.presigned_get_expires,
                Some(&disposition),
            )?));
        }
        Ok(Some(if s.use_cdn_url_for_all_uploads {
            s.cdn_url(url)
        } else {
            url.to_string()
        }))
    }
}

/// `Upload.short_path(sha1:, extension:)`
pub fn short_path(base_path: &str, sha1: &str, extension: Option<&str>) -> String {
    let base62 = crate::pretty_text::helpers::base62_sha1(sha1).unwrap_or_default();
    match extension.filter(|e| !e.is_empty()) {
        Some(ext) => format!("{base_path}/uploads/short-url/{base62}.{ext}"),
        None => format!("{base_path}/uploads/short-url/{base62}"),
    }
}

impl LocalStore {
    fn for_site(config: &Config, settings: &SiteSettings) -> Result<LocalStore, StoreError> {
        let urls = Urls { config, settings };
        let base_path = config.globals.relative_url_root().to_string();
        let db = config.site.db.clone();
        let relative_base_url = format!("{base_path}/uploads/{db}");
        Ok(LocalStore {
            public_dir: config.public_dir.clone(),
            db,
            absolute_base_url: format!("{}{relative_base_url}", urls.base_url_no_prefix()?),
            absolute_base_cdn_url: config
                .globals
                .cdn_url()
                .map(|cdn| format!("{}{relative_base_url}", cdn.trim_end_matches('/'))),
            scheme: urls.scheme()?.to_string(),
            base_path,
            relative_base_url,
        })
    }

    /// UrlHelper.local_cdn_url
    fn cdn_url(&self, url: &str) -> String {
        let Some(cdn_base) = &self.absolute_base_cdn_url else {
            return url.to_string();
        };
        let asset_host = cdn_base
            .strip_suffix(&self.relative_base_url)
            .unwrap_or(cdn_base);
        if url.starts_with(&format!("/uploads/{}/", self.db)) {
            return format!("{asset_host}{url}");
        }
        let base = self
            .absolute_base_url
            .strip_suffix(&self.relative_base_url)
            .unwrap_or(&self.absolute_base_url);
        url.replacen(base, asset_host, 1)
    }

    /// `store_file`: written under the public directory, served at the
    /// base path and the path.
    async fn store_file(&self, bytes: &[u8], path: &str) -> std::io::Result<String> {
        let target = self.public_dir.join(path.trim_start_matches('/'));
        if let Some(dir) = target.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        tokio::fs::write(&target, bytes).await?;
        Ok(format!("{}{path}", self.base_path))
    }

    /// `LocalStore#remove_file`
    async fn remove_file(&self, url: &str) -> std::io::Result<()> {
        if url.is_empty() || !url.starts_with(&self.relative_base_url) {
            return Ok(());
        }
        let source = self.public_dir.join(url.trim_start_matches('/'));
        if !tokio::fs::try_exists(&source).await? {
            return Ok(());
        }
        let tombstone = url.replacen("/uploads/", "/uploads/tombstone/", 1);
        let destination = self.public_dir.join(tombstone.trim_start_matches('/'));
        if let Some(dir) = destination.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        if tokio::fs::try_exists(&destination).await? {
            tokio::fs::remove_file(&destination).await?;
        }
        tokio::fs::rename(&source, &destination).await?;
        // FileUtils.touch: the move keeps the old mtime.
        let file = std::fs::File::options().write(true).open(&destination)?;
        file.set_modified(std::time::SystemTime::now())?;
        Ok(())
    }

    /// `LocalStore#purge_tombstone(grace_period)`: `find <tombstone>
    /// -mtime +<days> -type f -delete`, files a whole `days` or more past
    /// their last day removed (age in whole days over `days`),
    /// directories kept.
    async fn purge_tombstone(&self, days: i64) -> std::io::Result<()> {
        let tombstone = self
            .relative_base_url
            .replacen("/uploads/", "/uploads/tombstone/", 1);
        let dir = self.public_dir.join(tombstone.trim_start_matches('/'));
        if !tokio::fs::try_exists(&dir).await? {
            return Ok(());
        }
        let now = std::time::SystemTime::now();
        let mut pending = vec![dir];
        while let Some(dir) = pending.pop() {
            let mut entries = tokio::fs::read_dir(&dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let meta = entry.metadata().await?;
                if meta.is_dir() {
                    pending.push(entry.path());
                    continue;
                }
                if !meta.is_file() {
                    continue;
                }
                let age = now
                    .duration_since(meta.modified()?)
                    .unwrap_or_default()
                    .as_secs() as i64;
                if age / 86_400 > days {
                    tokio::fs::remove_file(entry.path()).await?;
                }
            }
        }
        Ok(())
    }

    /// `LocalStore#has_been_uploaded?`
    fn has_been_uploaded(&self, url: &str) -> bool {
        if url.is_empty() {
            return false;
        }
        if url.starts_with(&self.relative_base_url) {
            return true;
        }
        let absolute = match url.strip_prefix("//") {
            Some(rest) => format!("{}://{rest}", self.scheme),
            None => url.to_string(),
        };
        absolute.starts_with(&self.absolute_base_url)
            || self
                .absolute_base_cdn_url
                .as_deref()
                .is_some_and(|cdn| absolute.starts_with(cdn))
    }
}

/// The settings the external store is configured from: the site's
/// (`enable_s3_uploads`) or the S3 global settings (SiteSetting::Upload).
struct UploadSettings<'a> {
    config: &'a Config,
    settings: &'a SiteSettings,
    site: bool,
}

impl UploadSettings<'_> {
    /// A setting of either kind: the site setting, or GlobalSetting's
    /// `global` name.
    fn text(&self, site: &str, global: &str) -> Result<String, SettingError> {
        Ok(if self.site {
            self.settings.get(site)?.to_s().to_string()
        } else {
            self.config.globals.get(global).unwrap_or("").to_string()
        })
    }

    fn flag(&self, name: &str) -> Result<bool, SettingError> {
        Ok(if self.site {
            self.settings.get(name)?.truthy()
        } else {
            self.config.globals.get(name).is_some_and(|v| v == "true")
        })
    }
}

/// S3Helper.s3_options with S3Helper.s3_credentials, for the bucket: the
/// region and credentials of `obj` (SiteSetting or GlobalSetting), the
/// endpoint of the `s3_endpoint` setting either way.
pub fn s3_config(
    config: &Config,
    settings: &SiteSettings,
    site: bool,
    bucket: &str,
    accelerate: bool,
) -> Result<s3::S3Config, StoreError> {
    let obj = UploadSettings {
        config,
        settings,
        site,
    };
    let region = obj.text("s3_region", "s3_region")?;
    let endpoint = settings.get("s3_endpoint")?.to_s().to_string();
    let (access_key_id, secret_access_key, session_token) = if obj.flag("s3_use_iam_profile")? {
        // aws-sdk's default chain; the port reads the environment's keys.
        match (
            std::env::var("AWS_ACCESS_KEY_ID").ok(),
            std::env::var("AWS_SECRET_ACCESS_KEY").ok(),
        ) {
            (Some(key), Some(secret)) => (key, secret, std::env::var("AWS_SESSION_TOKEN").ok()),
            _ => {
                return Err(Unsupported(
                    "S3 credentials from an instance profile (s3_use_iam_profile without AWS_ACCESS_KEY_ID)",
                )
                .into());
            }
        }
    } else if !ruby::is_blank(&obj.text("s3_role_arn", "s3_role_arn")?) {
        return Err(Unsupported("S3 credentials from an assumed role (s3_role_arn)").into());
    } else {
        (
            obj.text("s3_access_key_id", "s3_access_key_id")?,
            obj.text("s3_secret_access_key", "s3_secret_access_key")?,
            None,
        )
    };
    // Config::object_storage_endpoints: where tests run the server.
    let endpoint = config
        .object_storage_endpoints
        .iter()
        .find(|(from, _)| *from == endpoint)
        .map_or(endpoint.clone(), |(_, to)| to.clone());
    Ok(s3::S3Config {
        bucket: bucket.to_string(),
        region,
        endpoint: (!ruby::is_blank(&endpoint)).then_some(endpoint),
        access_key_id,
        secret_access_key,
        session_token,
        force_path_style: config
            .globals
            .get("s3_force_path_style")
            .is_some_and(|v| v == "true"),
        dualstack: use_dualstack_endpoint(config, settings)?,
        accelerate,
    })
}

/// `SiteSetting::Upload.use_dualstack_endpoint`: S3 uploads on, no
/// endpoint, not a China region.
fn use_dualstack_endpoint(config: &Config, settings: &SiteSettings) -> Result<bool, SettingError> {
    let site = settings.get("enable_s3_uploads")?.truthy();
    if !site && !config.globals.use_s3() {
        return Ok(false);
    }
    let obj = UploadSettings {
        config,
        settings,
        site,
    };
    if !ruby::is_blank(&obj.text("s3_endpoint", "s3_endpoint")?) {
        return Ok(false);
    }
    Ok(!obj.text("s3_region", "s3_region")?.starts_with("cn-"))
}

/// `SiteSetting::Upload.enable_s3_uploads`: the site setting, or the S3
/// global settings.
pub fn s3_uploads_enabled(config: &Config, settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("enable_s3_uploads")?.truthy() || config.globals.use_s3())
}

/// `SiteSetting::Upload.s3_cdn_url`, None when blank.
pub fn upload_s3_cdn_url(
    config: &Config,
    settings: &SiteSettings,
) -> Result<Option<String>, SettingError> {
    let obj = UploadSettings {
        config,
        settings,
        site: settings.get("enable_s3_uploads")?.truthy(),
    };
    let cdn = obj.text("s3_cdn_url", "s3_cdn_url")?;
    Ok((!ruby::is_blank(&cdn)).then_some(cdn))
}

/// `SiteSetting::Upload.s3_upload_bucket` (the site's, or GlobalSetting's
/// `s3_bucket`) as S3Store#s3_bucket reads it: lowercase, its name and
/// its folder. Blank is Discourse::SiteSettingMissing.
pub fn upload_bucket(
    config: &Config,
    settings: &SiteSettings,
) -> Result<(String, Option<String>), SettingError> {
    let obj = UploadSettings {
        config,
        settings,
        site: settings.get("enable_s3_uploads")?.truthy(),
    };
    let bucket = obj.text("s3_upload_bucket", "s3_bucket")?.to_lowercase();
    if ruby::is_blank(&bucket) {
        return Err(SettingError::Invalid {
            name: "s3_upload_bucket".into(),
            value: bucket,
        });
    }
    Ok(match bucket.split_once('/') {
        Some((name, folder)) => (name.to_string(), Some(folder.to_string())),
        None => (bucket, None),
    })
}

/// `SiteSetting::Upload.absolute_base_url`: `//<bucket>.<the endpoint's
/// host>`, or AWS's regional host without an endpoint (or with one of
/// AWS's own).
pub fn upload_absolute_base_url(
    config: &Config,
    settings: &SiteSettings,
) -> Result<String, SettingError> {
    let (name, _) = upload_bucket(config, settings)?;
    let endpoint = settings.get("s3_endpoint")?.to_s().to_string();
    if !ruby::is_blank(&endpoint) && !endpoint.ends_with("amazonaws.com") {
        return Ok(format!(
            "//{name}.{}",
            endpoint.rsplit('/').next().unwrap_or("")
        ));
    }
    let obj = UploadSettings {
        config,
        settings,
        site: settings.get("enable_s3_uploads")?.truthy(),
    };
    let region = obj.text("s3_region", "s3_region")?;
    Ok(if use_dualstack_endpoint(config, settings)? {
        format!("//{name}.s3.dualstack.{region}.amazonaws.com")
    } else {
        format!("//{name}.s3.{region}.amazonaws.com.cn")
    })
}

/// `GlobalPath#upload_cdn_path`'s `Discourse.store.cdn_url(p)`, behind
/// `SiteSetting::Upload.s3_cdn_url`: the external store's mapping, the
/// local store's (no CDN of its own here: unchanged) otherwise.
pub fn upload_cdn_url(
    config: &Config,
    settings: &SiteSettings,
    url: &str,
) -> Result<String, SettingError> {
    let site = settings.get("enable_s3_uploads")?.truthy();
    if !site && !config.globals.use_s3() {
        return Ok(url.to_string());
    }
    let obj = UploadSettings {
        config,
        settings,
        site,
    };
    let cdn = obj.text("s3_cdn_url", "s3_cdn_url")?;
    if ruby::is_blank(&cdn) {
        return Ok(url.to_string());
    }
    let (_, folder) = upload_bucket(config, settings)?;
    Ok(s3_cdn_url(
        &upload_absolute_base_url(config, settings)?,
        folder.as_deref(),
        &cdn,
        url,
    ))
}

/// `S3Store#cdn_url`: the bucket's base (and folder) swapped for the CDN.
fn s3_cdn_url(absolute_base_url: &str, folder: Option<&str>, cdn: &str, url: &str) -> String {
    let schema = if url.starts_with("https://") {
        "https:"
    } else if url.starts_with("http://") {
        "http:"
    } else {
        ""
    };
    let folder = folder.map(|f| format!("{f}/")).unwrap_or_default();
    let from = file_join(&format!("{schema}{absolute_base_url}"), &folder);
    url.replacen(&from, &file_join(cdn, "/"), 1)
}

impl ExternalStore {
    fn for_site(
        config: &Config,
        settings: &SiteSettings,
        local: LocalStore,
    ) -> Result<ExternalStore, StoreError> {
        let site = settings.get("enable_s3_uploads")?.truthy();
        let obj = UploadSettings {
            config,
            settings,
            site,
        };
        let (name, folder) = upload_bucket(config, settings)?;
        // S3Helper#check_missing_site_options, for the site's settings.
        if site && !settings.get("s3_use_iam_profile")?.truthy() {
            if ruby::is_blank(&settings.get("s3_access_key_id")?.to_s()) {
                return Err(StoreError::SettingMissing("access_key_id"));
            }
            if ruby::is_blank(&settings.get("s3_secret_access_key")?.to_s()) {
                return Err(StoreError::SettingMissing("secret_access_key"));
            }
        }
        let accelerate = obj.flag("enable_s3_transfer_acceleration")?;
        let s3 = s3_config(config, settings, site, &name, accelerate)?;
        let backend = Backend::S3(s3::S3::new(&s3)?);
        let absolute_base_url = upload_absolute_base_url(config, settings)?;

        let multisite = config.site.multisite;
        // S3Helper: the tombstone prefix under the bucket's folder.
        let tombstone = if multisite {
            format!("uploads/tombstone/{}/", config.site.db)
        } else {
            "tombstone/".to_string()
        };
        let tombstone_prefix = match &folder {
            Some(folder) => format!("{}/{tombstone}", folder.trim_end_matches('/')),
            None => tombstone,
        };
        let max_age = settings.get("s3_max_age")?.to_i();
        let swr = settings.get("s3_stale_while_revalidate")?.to_i();
        let mut cache_control = format!("max-age={max_age}, public, immutable");
        if swr != S3_STALE_WHILE_REVALIDATE_DEFAULT {
            cache_control.push_str(&format!(", stale-while-revalidate={swr}"));
        }
        let access_control_tag = if settings.get("s3_enable_access_control_tags")?.truthy() {
            let key = settings
                .get("s3_access_control_tag_key")?
                .to_s()
                .to_string();
            (!ruby::is_blank(&key)).then(|| -> Result<_, StoreError> {
                Ok((
                    key,
                    settings
                        .get("s3_access_control_tag_public_value")?
                        .to_s()
                        .to_string(),
                    settings
                        .get("s3_access_control_tag_private_value")?
                        .to_s()
                        .to_string(),
                ))
            })
        } else {
            None
        }
        .transpose()?;
        let s3_cdn_url = obj.text("s3_cdn_url", "s3_cdn_url")?;
        Ok(ExternalStore {
            backend,
            folder,
            absolute_base_url,
            s3_cdn_url: (!ruby::is_blank(&s3_cdn_url)).then_some(s3_cdn_url),
            use_cdn_url_for_all_uploads: settings.get("s3_use_cdn_url_for_all_uploads")?.truthy(),
            multisite,
            tombstone_prefix,
            configure_tombstone_policy: settings.get("s3_configure_tombstone_policy")?.truthy(),
            cache_control,
            use_acls: settings.get("s3_use_acls")?.truthy(),
            access_control_tag,
            presigned_get_expires: settings
                .get("s3_presigned_get_url_expires_after_seconds")?
                .to_i()
                .max(0) as u64,
            local,
        })
    }

    /// `S3Helper#get_path_for_s3_upload`: a path under the bucket's
    /// folder (temporary uploads name theirs already).
    fn s3_path(&self, path: &str) -> String {
        match &self.folder {
            Some(folder)
                if !path.starts_with(folder.as_str())
                    && !path.starts_with(&format!("temp/{folder}")) =>
            {
                format!("{}/{path}", folder.trim_end_matches('/'))
            }
            _ => path.to_string(),
        }
    }

    /// `upload_path` before a path, under multisite.
    fn multisite_path(&self, path: &str) -> String {
        if self.multisite {
            format!("uploads/{}/{path}", self.local.db)
        } else {
            path.to_string()
        }
    }

    /// The object key of one of the store's urls (`signed_url_for_path`'s
    /// `path.sub(absolute_base_url + "/", "")`, or its CDN url's path).
    fn key_of(&self, url: &str) -> Option<String> {
        let absolute = match url.split_once("//") {
            Some((scheme, rest)) if scheme.is_empty() || scheme.ends_with(':') => {
                format!("//{rest}")
            }
            _ => return None,
        };
        if let Some(key) = absolute.strip_prefix(&format!("{}/", self.absolute_base_url)) {
            return Some(key.to_string());
        }
        let cdn = self.s3_cdn_url.as_deref()?;
        let cdn = cdn.split_once("//").map_or(cdn, |(_, rest)| rest);
        let key = absolute
            .strip_prefix("//")?
            .strip_prefix(cdn.trim_end_matches('/'))?
            .strip_prefix('/')?;
        Some(self.s3_path(key))
    }

    /// `S3Store#default_s3_options(secure:)`: the canned ACL and the
    /// access control tag.
    fn access_options(&self, secure: bool) -> (Option<String>, Option<String>) {
        let acl = self
            .use_acls
            .then(|| if secure { "private" } else { "public-read" }.to_string());
        let tagging = self
            .access_control_tag
            .as_ref()
            .map(|(key, public, private)| {
                let value = if secure { private } else { public };
                form_urlencoded::Serializer::new(String::new())
                    .append_pair(key, value)
                    .finish()
            });
        (acl, tagging)
    }

    /// How `store_file` writes an object named `filename`: cache control,
    /// type and disposition by its name, ACL and tag.
    fn put_options(&self, filename: &str, secure: bool) -> PutOptions {
        let (acl, tagging) = self.access_options(secure);
        let disposition = content_disposition_for(filename);
        PutOptions {
            content_type: mime_type(filename).map(str::to_string),
            cache_control: Some(self.cache_control.clone()),
            content_disposition: (!disposition.is_empty()).then_some(disposition),
            acl,
            tagging,
            metadata: Vec::new(),
        }
    }

    /// The bucket the store keeps its objects in.
    pub fn backend(&self) -> &Backend {
        &self.backend
    }

    /// `S3Store#temporary_upload_path`: `temp/[<folder>/]uploads/<db>/<hex>/<hex><ext>`,
    /// where a browser puts a file before it is an upload.
    pub fn temporary_upload_path(&self, file_name: &str) -> String {
        let upload_path = format!("uploads/{}", self.local.db);
        let prefix = match &self.folder {
            Some(folder) => format!("{}/{upload_path}", folder.trim_end_matches('/')),
            None => upload_path,
        };
        temporary_upload_path(file_name, &prefix)
    }

    /// `S3Store#signed_request_for_temporary_upload`: a temporary key and
    /// a PUT presigned for ten minutes with the metadata given, private.
    pub fn signed_request_for_temporary_upload(
        &self,
        file_name: &str,
        metadata: Vec<(String, String)>,
    ) -> Result<(String, std::collections::BTreeMap<String, String>), StoreError> {
        let key = self.temporary_upload_path(file_name);
        let (acl, tagging) = self.access_options(true);
        let opts = PutOptions {
            acl,
            tagging,
            metadata,
            ..Default::default()
        };
        Ok(self
            .backend
            .presigned_put(&key, UPLOAD_URL_EXPIRES_AFTER_SECONDS, &opts)?)
    }

    /// `S3Store#create_multipart`: a multipart upload at a temporary key,
    /// private; its id and key.
    pub async fn create_multipart(
        &self,
        file_name: &str,
        content_type: Option<&str>,
        metadata: Vec<(String, String)>,
    ) -> Result<(String, String), StoreError> {
        let key = self.temporary_upload_path(file_name);
        let (acl, tagging) = self.access_options(true);
        let opts = PutOptions {
            content_type: content_type.map(str::to_string),
            acl,
            tagging,
            metadata,
            ..Default::default()
        };
        let upload_id = self.backend.create_multipart(&key, &opts).await?;
        Ok((upload_id, key))
    }

    /// `signed_url_for_path(key, include_content_disposition: false)`.
    pub fn presigned_get(&self, key: &str) -> Result<String, StoreError> {
        Ok(self
            .backend
            .presigned_get(key, self.presigned_get_expires, None)?)
    }

    /// `S3Store#store_file`: the object written with its cache control,
    /// type and disposition (by its filename), ACL and tags; its url
    /// (`absolute_base_url/<key>`) and etag.
    async fn store_file(
        &self,
        bytes: &[u8],
        path: &str,
        filename: Option<&str>,
    ) -> Result<Stored, StoreError> {
        let filename = match filename.filter(|f| !ruby::is_blank(f)) {
            Some(f) => f.to_string(),
            None => path.rsplit('/').next().unwrap_or(path).to_string(),
        };
        let opts = self.put_options(&filename, false);
        let key = self.s3_path(&self.multisite_path(path));
        self.backend.ensure_bucket().await?;
        let etag = self.backend.put(&key, bytes.to_vec(), &opts).await?;
        Ok(Stored {
            url: format!("{}/{key}", self.absolute_base_url),
            etag: Some(etag),
        })
    }

    /// `S3Store#remove_file`: one of the store's objects copied to the
    /// tombstone, then deleted.
    async fn remove_file(&self, url: &str) -> Result<(), StoreError> {
        if !self.has_been_uploaded(url) {
            return Ok(());
        }
        let Some(path) = store_path_in(url) else {
            return Ok(());
        };
        let source = self.s3_path(&self.multisite_path(path));
        let tombstone = format!("{}{path}", self.tombstone_prefix);
        self.backend.ensure_bucket().await?;
        match self.backend.copy(&source, &tombstone, None).await {
            Ok(_) | Err(StorageError::NotFound) => {}
            Err(e) if e.code() == "NoSuchKey" => {}
            Err(e) => return Err(e.into()),
        }
        self.backend.delete(&source).await?;
        Ok(())
    }

    /// `S3Store#has_been_uploaded?`
    fn has_been_uploaded(&self, url: &str) -> bool {
        if url.is_empty() {
            return false;
        }
        let Some(parsed_path) = url_path(url) else {
            return false;
        };
        let base_hostname = host_of(&self.absolute_base_url).unwrap_or_default();
        if !base_hostname.is_empty() && url.contains(&base_hostname) {
            return match &self.folder {
                Some(folder) => parsed_path.starts_with(&format!("/{folder}")),
                None => true,
            };
        }
        let Some(cdn) = &self.s3_cdn_url else {
            return false;
        };
        let cdn_hostname = host_of(cdn).unwrap_or_default();
        let cdn_path = url_path(cdn).unwrap_or_default();
        !cdn_hostname.is_empty()
            && url.contains(&cdn_hostname)
            && (cdn_path.is_empty() || parsed_path.starts_with(&cdn_path))
    }

    /// `S3Store#cdn_url`: the bucket's base (and folder) swapped for
    /// `s3_cdn_url`.
    fn cdn_url(&self, url: &str) -> String {
        match &self.s3_cdn_url {
            Some(cdn) => s3_cdn_url(&self.absolute_base_url, self.folder.as_deref(), cdn, url),
            None => url.to_string(),
        }
    }
}

/// S3Helper::UPLOAD_URL_EXPIRES_AFTER_SECONDS: presigned uploads last ten
/// minutes.
pub const UPLOAD_URL_EXPIRES_AFTER_SECONDS: u64 = 600;

/// `BaseStore.temporary_upload_path(file_name, folder_prefix:)`:
/// `temp/<prefix>/<hex>/<hex><extname>`, the name itself left out.
pub fn temporary_upload_path(file_name: &str, folder_prefix: &str) -> String {
    let hex = || {
        let bytes: [u8; 16] = rand::random();
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    };
    let mut parts = vec!["temp".to_string()];
    if !folder_prefix.is_empty() {
        parts.push(folder_prefix.trim_matches('/').to_string());
    }
    parts.push(hex());
    parts.push(format!("{}{}", hex(), extname(file_name)));
    parts.join("/")
}

/// Ruby's `File.extname`: the last extension of the basename (`.` for a
/// name ending in one), none for a dotfile.
pub fn extname(file_name: &str) -> &str {
    let base = file_name.rsplit('/').next().unwrap_or(file_name);
    let trimmed = base.trim_start_matches('.');
    match trimmed.rfind('.') {
        Some(i) => &trimmed[i..],
        None => "",
    }
}

/// SiteSetting.defaults[:s3_stale_while_revalidate]
const S3_STALE_WHILE_REVALIDATE_DEFAULT: i64 = 31_556_952;

/// Ruby's `File.join(a, b)`: one slash between them.
fn file_join(a: &str, b: &str) -> String {
    if b.is_empty() {
        return format!("{}/", a.trim_end_matches('/'));
    }
    format!("{}/{}", a.trim_end_matches('/'), b.trim_start_matches('/'))
}

/// A url's host (`URI.parse(url).hostname`), protocol-relative too.
fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("//")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = match host.rsplit_once(':') {
        Some((h, port)) if port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => host,
    };
    Some(host.to_lowercase())
}

/// A url's path (`URI.parse(url).path`); None when it does not parse.
fn url_path(url: &str) -> Option<String> {
    if url.contains(' ') {
        return None;
    }
    let without_query = url.split(['?', '#']).next().unwrap_or("");
    match without_query.split_once("//") {
        Some((scheme, rest)) if scheme.is_empty() || scheme.ends_with(':') => Some(
            rest.find('/')
                .map(|i| rest[i..].to_string())
                .unwrap_or_default(),
        ),
        _ => Some(without_query.to_string()),
    }
}

/// `FileHelper.supported_video`
const SUPPORTED_VIDEO: [&str; 8] = ["mov", "mp4", "webm", "ogv", "m4v", "3gp", "avi", "mpeg"];
/// `FileHelper.supported_audio`
const SUPPORTED_AUDIO: [&str; 11] = [
    "mp3", "ogg", "oga", "opus", "wav", "m4a", "m4b", "m4p", "m4r", "aac", "flac",
];
/// `FileHelper.supported_images`
const SUPPORTED_IMAGES: [&str; 11] = [
    "jpg", "jpeg", "png", "gif", "svg", "ico", "webp", "avif", "heic", "heif", "jxl",
];

fn ends_with_any(filename: &str, extensions: &[&str]) -> bool {
    let lower = filename.to_lowercase();
    extensions
        .iter()
        .any(|ext| lower.ends_with(&format!(".{ext}")))
}

/// `FileHelper.is_inline_safe?`: images but SVG, PDFs, video and audio.
pub fn is_inline_safe(filename: &str) -> bool {
    !filename.is_empty()
        && (ends_with_any(
            filename,
            &SUPPORTED_IMAGES
                .iter()
                .copied()
                .filter(|e| *e != "svg")
                .collect::<Vec<_>>(),
        ) || ends_with_any(filename, &["pdf"])
            || ends_with_any(filename, &SUPPORTED_VIDEO)
            || ends_with_any(filename, &SUPPORTED_AUDIO))
}

/// `FileHelper.is_supported_media?`: an image, video or audio file.
pub fn is_supported_media(filename: &str) -> bool {
    ends_with_any(filename, &SUPPORTED_IMAGES)
        || ends_with_any(filename, &SUPPORTED_VIDEO)
        || ends_with_any(filename, &SUPPORTED_AUDIO)
}

/// `S3Store.content_disposition_for(filename)`: inline for what is safe
/// to show, else attachment; the filename shortened to keep the header
/// within 1600 bytes.
pub fn content_disposition_for(filename: &str) -> String {
    if filename.trim().is_empty() {
        return String::new();
    }
    let disposition = if is_inline_safe(filename) {
        "inline"
    } else {
        "attachment"
    };
    const MAX: usize = 1_600;
    let header = content_disposition(disposition, filename);
    if header.len() <= MAX {
        return header;
    }
    let (basename, extension) = match filename.rfind('.') {
        Some(i) if i > 0 => (&filename[..i], &filename[i..]),
        _ => (filename, ""),
    };
    let chars: Vec<char> = basename.chars().collect();
    let (mut low, mut high) = (1usize, chars.len());
    while low < high {
        let mid = (low + high).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect::<String>() + extension;
        if content_disposition(disposition, &candidate).len() <= MAX {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    let name: String = chars[..low].iter().collect::<String>() + extension;
    content_disposition(disposition, &name)
}

/// `ActionDispatch::Http::ContentDisposition.format(disposition:,
/// filename:)`
pub fn content_disposition(disposition: &str, filename: &str) -> String {
    fn escape(s: &str, keep: impl Fn(u8) -> bool) -> String {
        let mut out = String::new();
        for c in s.chars() {
            let mut buf = [0u8; 4];
            let bytes = c.encode_utf8(&mut buf).as_bytes();
            if bytes.len() == 1 && keep(bytes[0]) {
                out.push(c);
            } else {
                for b in bytes {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
        }
        out
    }
    // TRADITIONAL_ESCAPED_CHAR, over I18n.transliterate.
    let ascii = escape(&crate::posting::text::transliterate(filename), |b| {
        b == b' ' || b.is_ascii_alphanumeric() || b"!#$+.^_`|~-".contains(&b)
    });
    // RFC_5987_ESCAPED_CHAR
    let utf8 = escape(filename, |b| {
        b.is_ascii_alphanumeric() || b"!#$&+.^_`|~-".contains(&b)
    });
    format!("{disposition}; filename=\"{ascii}\"; filename*=UTF-8''{utf8}")
}

/// `MiniMime.lookup_by_filename(filename)&.content_type` for the types
/// uploads take.
pub fn mime_type(filename: &str) -> Option<&'static str> {
    let ext = filename.rsplit_once('.')?.1.to_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" | "jpe" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "ico" => "image/vnd.microsoft.icon",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "jxl" => "image/jxl",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "ogv" => "video/ogg",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/x-wav",
        "m4a" => "audio/mp4",
        "flac" => "audio/flac",
        "aac" => "audio/aac",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "js" => "application/javascript",
        "css" => "text/css",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_deepen_with_the_id() {
        assert_eq!(
            get_path_for("original", 36, "abcdef", ".gif"),
            "original/1X/abcdef.gif"
        );
        assert_eq!(
            get_path_for("optimized", 5000, "abcdef", "_2_10x10.gif"),
            "optimized/2X/a/abcdef_2_10x10.gif"
        );
    }

    #[test]
    fn extnames_as_ruby_reads_them() {
        assert_eq!(extname("fresh.tar.gz"), ".gz");
        assert_eq!(extname("static.gif"), ".gif");
        assert_eq!(extname(".bashrc"), "");
        assert_eq!(extname("noext"), "");
        assert_eq!(extname("dot."), ".");
        let path = temporary_upload_path("a.png", "uploads/default");
        let parts: Vec<&str> = path.split('/').collect();
        assert_eq!(&parts[..3], ["temp", "uploads", "default"]);
        assert_eq!(parts[3].len(), 32);
        assert!(parts[4].ends_with(".png") && parts[4].len() == 36);
    }

    #[test]
    fn store_paths_in_urls() {
        assert_eq!(
            store_path_in("//bucket.host/folder/original/1X/abc.png"),
            Some("original/1X/abc.png")
        );
        assert_eq!(
            store_path_in("//bucket.host/optimized/2X/a/abc_2_10x10.png"),
            Some("optimized/2X/a/abc_2_10x10.png")
        );
        assert_eq!(store_path_in("//bucket.host/original/x.png"), None);
    }

    #[test]
    fn content_dispositions_as_rails_formats_them() {
        assert_eq!(
            content_disposition_for("cat pic.png"),
            "inline; filename=\"cat pic.png\"; filename*=UTF-8''cat%20pic.png"
        );
        assert_eq!(
            content_disposition_for("doc.html"),
            "attachment; filename=\"doc.html\"; filename*=UTF-8''doc.html"
        );
        assert_eq!(
            content_disposition("attachment", "é.txt"),
            "attachment; filename=\"e.txt\"; filename*=UTF-8''%C3%A9.txt"
        );
        let long = format!("{}.pdf", "a".repeat(2000));
        assert!(content_disposition_for(&long).len() <= 1600);
        assert_eq!(content_disposition_for(""), "");
    }

    #[test]
    fn hosts_and_paths_of_urls() {
        assert_eq!(
            host_of("//bucket.minio.localhost:9122/x").as_deref(),
            Some("bucket.minio.localhost")
        );
        assert_eq!(
            url_path("//b.host/a/b.png?x=1").as_deref(),
            Some("/a/b.png")
        );
        assert_eq!(url_path("/uploads/x").as_deref(), Some("/uploads/x"));
        assert_eq!(file_join("https://cdn.test", "/"), "https://cdn.test/");
        assert_eq!(file_join("//b.host", "folder/"), "//b.host/folder/");
        assert_eq!(file_join("//b.host", ""), "//b.host/");
    }
}
