//! FileStore (lib/file_store): where uploads and their thumbnails are kept
//! and the urls they are served at. `Discourse.store` picks the store for
//! the site: only LocalStore (files under the public directory, served at
//! `/uploads`) is ported; S3 (enable_s3_uploads) is refused here, in one
//! place, until an S3Store is a variant of FileStore.

use std::path::PathBuf;

use crate::Unsupported;
use crate::config::Config;
use crate::site_settings::{SettingError, SiteSettings};
use crate::url::{UrlError, Urls};

/// `RailsMultisite::ConnectionManagement.current_db`
const CURRENT_DB: &str = "default";

/// Why the store cannot be had.
#[derive(Debug)]
pub enum StoreError {
    Setting(SettingError),
    Url(UrlError),
    Unsupported(Unsupported),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Setting(e) => e.fmt(f),
            StoreError::Url(e) => e.fmt(f),
            StoreError::Unsupported(e) => e.fmt(f),
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

/// The site's store (`Discourse.store`).
#[derive(Debug, Clone)]
pub enum FileStore {
    Local(LocalStore),
}

/// `FileStore::LocalStore`: files under the public directory.
#[derive(Debug, Clone)]
pub struct LocalStore {
    public_dir: PathBuf,
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

impl FileStore {
    /// `Discourse.store` for the site: the local store, or a refusal for
    /// S3.
    pub fn for_site(config: &Config, settings: &SiteSettings) -> Result<FileStore, StoreError> {
        if settings.get("enable_s3_uploads")?.truthy() || config.globals.use_s3() {
            return Err(StoreError::Unsupported(Unsupported("S3 upload stores")));
        }
        let urls = Urls { config, settings };
        let base_path = config.globals.relative_url_root().to_string();
        let relative_base_url = format!("{base_path}/uploads/{CURRENT_DB}");
        Ok(FileStore::Local(LocalStore {
            public_dir: config.public_dir.clone(),
            absolute_base_url: format!("{}{relative_base_url}", urls.base_url_no_prefix()?),
            absolute_base_cdn_url: config
                .globals
                .cdn_url()
                .map(|cdn| format!("{}{relative_base_url}", cdn.trim_end_matches('/'))),
            scheme: urls.scheme()?.to_string(),
            base_path,
            relative_base_url,
        }))
    }

    /// A local store for `base_url` (`http://host`), no base path or CDN.
    #[cfg(test)]
    pub(crate) fn test_local(base_url: &str) -> FileStore {
        let relative_base_url = format!("/uploads/{CURRENT_DB}");
        FileStore::Local(LocalStore {
            public_dir: PathBuf::from("public"),
            base_path: String::new(),
            absolute_base_url: format!("{base_url}{relative_base_url}"),
            absolute_base_cdn_url: None,
            scheme: base_url.split(':').next().unwrap_or("http").to_string(),
            relative_base_url,
        })
    }

    /// `external?`
    pub fn external(&self) -> bool {
        match self {
            FileStore::Local(_) => false,
        }
    }

    /// `relative_base_url`
    pub fn relative_base_url(&self) -> &str {
        match self {
            FileStore::Local(s) => &s.relative_base_url,
        }
    }

    /// `get_path_for(type, id, sha, extension)` with the store's prefix
    /// (`prefix_path`: `/uploads/<db>/...` for the local store).
    pub fn path_for_kind(&self, kind: &str, id: i32, sha1: &str, extension: &str) -> String {
        match self {
            FileStore::Local(_) => format!(
                "/uploads/{CURRENT_DB}/{}",
                get_path_for(kind, id, sha1, extension)
            ),
        }
    }

    /// `store_upload`: the original, at its path; its url.
    pub async fn store_upload(
        &self,
        bytes: &[u8],
        upload_id: i32,
        sha1: &str,
        extension: &str,
    ) -> std::io::Result<String> {
        let path = self.path_for_kind("original", upload_id, sha1, extension);
        self.store_file(bytes, &path).await
    }

    /// `store_optimized_image`: a thumbnail, named for its upload's sha1,
    /// version and size; its url.
    pub async fn store_optimized_image(
        &self,
        bytes: &[u8],
        upload_id: i32,
        upload_sha1: &str,
        suffix: &str,
    ) -> std::io::Result<String> {
        let path = self.path_for_kind("optimized", upload_id, upload_sha1, suffix);
        self.store_file(bytes, &path).await
    }

    /// `store_file`: written under the public directory, served at the
    /// base path and the path.
    async fn store_file(&self, bytes: &[u8], path: &str) -> std::io::Result<String> {
        match self {
            FileStore::Local(s) => {
                let target = s.public_dir.join(path.trim_start_matches('/'));
                if let Some(dir) = target.parent() {
                    tokio::fs::create_dir_all(dir).await?;
                }
                tokio::fs::write(&target, bytes).await?;
                Ok(format!("{}{path}", s.base_path))
            }
        }
    }

    /// The stored file of a url (`path_for`, then read): None when the url
    /// is not the store's or the file is missing.
    pub async fn read(&self, url: &str) -> std::io::Result<Option<Vec<u8>>> {
        match self {
            FileStore::Local(s) => {
                // path_for: a relative url, the base path in it as Rails
                // keeps it.
                if !url.starts_with('/') || url.starts_with("//") {
                    return Ok(None);
                }
                let path = s.public_dir.join(url.trim_start_matches('/'));
                match tokio::fs::read(&path).await {
                    Ok(bytes) => Ok(Some(bytes)),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(e),
                }
            }
        }
    }

    /// `has_been_uploaded?`: a url of this store, relative or absolute
    /// (protocol-relative ones take the site's scheme), or the CDN's.
    pub fn has_been_uploaded(&self, url: &str) -> bool {
        match self {
            FileStore::Local(s) => {
                if url.is_empty() {
                    return false;
                }
                if url.starts_with(&s.relative_base_url) {
                    return true;
                }
                let absolute = match url.strip_prefix("//") {
                    Some(rest) => format!("{}://{rest}", s.scheme),
                    None => url.to_string(),
                };
                absolute.starts_with(&s.absolute_base_url)
                    || s.absolute_base_cdn_url
                        .as_deref()
                        .is_some_and(|cdn| absolute.starts_with(cdn))
            }
        }
    }

    /// `download_url(upload)`
    pub fn download_url(&self, sha1: &str) -> String {
        format!("{}/{sha1}", self.relative_base_url())
    }
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
}
