//! BackupRestore::BackupStore: where the backups are kept, picked by
//! `backup_location`. Callers see one store (the files newest first, one
//! file, delete, download, upload, the retention rules and the stats);
//! which backend holds the files is the store's business. The local
//! backend keeps the `.gz` and `.tgz` files in `public/backups/<db>`; the
//! external one (S3BackupStore) keeps them as objects under
//! `s3_backup_bucket/<db>/` in an object storage bucket
//! ([`crate::object_storage`]).

use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::{Value, json};

use crate::config::Config;
use crate::object_storage::{Backend, CorsRule, PutOptions, StorageError, s3};
use crate::site_settings::SiteSettings;
use crate::{AppError, ruby};

/// Where a backup's bytes can be had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A file on this machine (the local store's).
    File(PathBuf),
    /// A URL that serves it for a while (a remote store's presigned
    /// download).
    Url(String),
}

/// BackupRestore::BackupFile
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupFile {
    pub filename: String,
    pub size: u64,
    /// `File.mtime(path).utc`, or the object's Last-Modified.
    pub last_modified: NaiveDateTime,
    /// `source`: always the file for the local store; for a remote one
    /// only when asked for (`include_download_source`).
    pub source: Option<Source>,
}

impl BackupFile {
    /// BackupFileSerializer
    pub fn json(&self) -> Value {
        json!({
            "filename": self.filename,
            "size": self.size,
            "last_modified": crate::topic_list::time_json(self.last_modified),
        })
    }
}

/// The store's own failures, which callers answer for (Rails rescues
/// them by class).
#[derive(Debug)]
pub enum BackupStoreError {
    /// `BackupStore::StorageError`: the remote store failed, with its
    /// message.
    Storage(String),
    /// `BackupStore::BackupFileExists`
    FileExists,
    /// `NotImplementedError`: what the backend cannot do (a local store's
    /// upload URL).
    NotImplemented(&'static str),
}

impl std::fmt::Display for BackupStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackupStoreError::Storage(message) => f.write_str(message),
            BackupStoreError::FileExists => {
                f.write_str("BackupRestore::BackupStore::BackupFileExists")
            }
            BackupStoreError::NotImplemented(what) => write!(f, "NotImplementedError: {what}"),
        }
    }
}

impl std::error::Error for BackupStoreError {}

/// `rescue Aws::Errors::ServiceError => e; raise StorageError.new(...)`
fn storage(e: StorageError) -> AppError {
    match e {
        StorageError::Unsupported(u) => u.into(),
        e => BackupStoreError::Storage(e.to_string()).into(),
    }
}

/// `BackupStore#stats`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stats {
    pub used_bytes: u64,
    /// None for a remote store.
    pub free_bytes: Option<u64>,
    pub count: usize,
    pub last_backup_taken_at: Option<NaiveDateTime>,
}

/// The site's backup store (`BackupStore.create`).
#[derive(Debug, Clone)]
pub enum BackupStore {
    Local(LocalBackupStore),
    External(Box<ExternalBackupStore>),
}

/// BackupRestore::S3BackupStore: objects under `s3_backup_bucket`'s folder
/// and the site's db (`<bucket>/<folder>/<db>/<filename>`).
#[derive(Debug, Clone)]
pub struct ExternalBackupStore {
    pub(crate) backend: Backend,
    /// The bucket's folder and the site's db: what keys start with.
    pub(crate) folder: String,
    /// `s3_presigned_get_url_expires_after_seconds`
    presigned_get_expires: u64,
    /// `!s3_disable_cleanup`
    cleanup_allowed: bool,
    /// `s3_install_cors_rule`
    install_cors_rule: bool,
    /// `s3_use_acls`
    use_acls: bool,
}

/// S3BackupStore::UPLOAD_URL_EXPIRES_AFTER_SECONDS (six hours).
const UPLOAD_URL_EXPIRES_AFTER_SECONDS: u64 = 6 * 3600;

impl ExternalBackupStore {
    fn for_site(config: &Config, settings: &SiteSettings) -> Result<Self, AppError> {
        // s3_bucket_name_with_prefix: File.join(s3_backup_bucket,
        // current_db), split as S3Helper splits it.
        let bucket = settings.get("s3_backup_bucket")?.to_s().to_lowercase();
        if ruby::is_blank(&bucket) {
            // S3Helper: Discourse::InvalidParameters.new("s3_bucket_name")
            return Err(std::io::Error::other(
                "You supplied invalid parameters to the request: s3_bucket_name",
            )
            .into());
        }
        let with_db = format!("{}/{}", bucket.trim_end_matches('/'), config.site.db);
        let (name, folder) = with_db.split_once('/').unwrap_or((&with_db, ""));
        // S3Helper.s3_options(SiteSetting): the site's region, endpoint
        // and keys, never transfer acceleration.
        let s3_config = crate::file_store::s3_config(config, settings, true, name, false)
            .map_err(AppError::from)?;
        Ok(ExternalBackupStore {
            backend: Backend::S3(s3::S3::new(&s3_config).map_err(storage)?),
            folder: folder.to_string(),
            presigned_get_expires: settings
                .get("s3_presigned_get_url_expires_after_seconds")?
                .to_i()
                .max(0) as u64,
            cleanup_allowed: !settings.get("s3_disable_cleanup")?.truthy(),
            install_cors_rule: settings.get("s3_install_cors_rule")?.truthy(),
            use_acls: settings.get("s3_use_acls")?.truthy(),
        })
    }

    /// `s3_helper.object(filename)`'s key.
    fn key(&self, filename: &str) -> String {
        if self.folder.is_empty() {
            filename.to_string()
        } else {
            format!("{}/{filename}", self.folder)
        }
    }

    /// `file_regex`: a `.gz` or `.tgz` right under the folder.
    fn is_backup_key(&self, key: &str) -> bool {
        let rest = if self.folder.is_empty() {
            Some(key)
        } else {
            key.strip_prefix(&format!("{}/", self.folder))
        };
        rest.is_some_and(|name| {
            let lower = name.to_lowercase();
            !name.contains('/') && (lower.ends_with(".gz") || lower.ends_with(".tgz"))
        })
    }

    async fn unsorted_files(&self) -> Result<Vec<BackupFile>, AppError> {
        self.backend.ensure_bucket().await.map_err(storage)?;
        let prefix = if self.folder.is_empty() {
            String::new()
        } else {
            format!("{}/", self.folder)
        };
        let objects = self.backend.list(&prefix).await.map_err(storage)?;
        Ok(objects
            .into_iter()
            .filter(|o| self.is_backup_key(&o.key))
            .map(|o| BackupFile {
                filename: o.key.rsplit('/').next().unwrap_or(&o.key).to_string(),
                size: o.size,
                last_modified: o.last_modified,
                source: None,
            })
            .collect())
    }

    /// `obj.exists?` and the object's file, presigned when asked.
    async fn file(
        &self,
        filename: &str,
        with_source: bool,
    ) -> Result<Option<BackupFile>, AppError> {
        self.backend.ensure_bucket().await?;
        let key = self.key(filename);
        let Some(meta) = self.backend.head(&key).await? else {
            return Ok(None);
        };
        let source = if with_source {
            Some(Source::Url(self.backend.presigned_get(
                &key,
                self.presigned_get_expires,
                None,
            )?))
        } else {
            None
        };
        Ok(Some(BackupFile {
            filename: key.rsplit('/').next().unwrap_or(&key).to_string(),
            size: meta.size,
            last_modified: meta.last_modified,
            source,
        }))
    }

    /// The bucket the store keeps its backups in.
    pub fn backend(&self) -> &Backend {
        &self.backend
    }

    /// `S3BackupStore#temporary_upload_path`: `temp/<folder>/<hex>/<hex><ext>`.
    pub fn temporary_upload_path(&self, file_name: &str) -> String {
        crate::file_store::temporary_upload_path(file_name, &self.folder)
    }

    /// `S3BackupStore#create_multipart`: refused for a name taken, else a
    /// private multipart upload at a temporary key; its id and key.
    pub async fn create_multipart(
        &self,
        file_name: &str,
        content_type: Option<&str>,
        metadata: Vec<(String, String)>,
    ) -> Result<(String, String), AppError> {
        self.backend.ensure_bucket().await?;
        if self.backend.head(&self.key(file_name)).await?.is_some() {
            return Err(BackupStoreError::FileExists.into());
        }
        let key = self.temporary_upload_path(file_name);
        let opts = PutOptions {
            content_type: content_type.map(str::to_string),
            acl: self.use_acls.then(|| "private".to_string()),
            metadata,
            ..Default::default()
        };
        let upload_id = self.backend.create_multipart(&key, &opts).await?;
        Ok((upload_id, key))
    }

    /// `S3BackupStore#move_existing_stored_upload`: the uploaded object
    /// copied to its name in the folder (private), then deleted.
    pub async fn move_existing_stored_upload(
        &self,
        existing_key: &str,
        original_filename: &str,
    ) -> Result<(), AppError> {
        let opts = PutOptions {
            acl: self.use_acls.then(|| "private".to_string()),
            ..Default::default()
        };
        self.backend
            .copy(existing_key, &self.key(original_filename), Some(&opts))
            .await?;
        self.backend.delete(existing_key).await?;
        Ok(())
    }

    /// `generate_upload_url`: refused for a name taken, the bucket's CORS
    /// rule for browser uploads made sure of, then a presigned PUT good
    /// for six hours. Any service error is a StorageError.
    async fn generate_upload_url(&self, filename: &str) -> Result<String, AppError> {
        let key = self.key(filename);
        let generated = async {
            self.backend.ensure_bucket().await?;
            if self.backend.head(&key).await?.is_some() {
                return Ok(None);
            }
            if self.install_cors_rule {
                self.backend
                    .ensure_cors(&CorsRule::backup_direct_upload())
                    .await?;
            }
            let (url, _) = self.backend.presigned_put(
                &key,
                UPLOAD_URL_EXPIRES_AFTER_SECONDS,
                &PutOptions::default(),
            )?;
            Ok::<_, StorageError>(Some(url))
        };
        match generated.await {
            Ok(Some(url)) => Ok(url),
            Ok(None) => Err(BackupStoreError::FileExists.into()),
            Err(e) => {
                tracing::warn!("Failed to generate upload URL for S3: {e}");
                Err(storage(e))
            }
        }
    }
}

impl BackupStore {
    /// `BackupStore.create`: the store `backup_location` names.
    pub fn create(config: &Config, settings: &SiteSettings) -> Result<Self, AppError> {
        match settings.get("backup_location")?.to_s().as_str() {
            "s3" => Ok(BackupStore::External(Box::new(
                ExternalBackupStore::for_site(config, settings)?,
            ))),
            _ => Self::local(config),
        }
    }

    /// `BackupStore.create(location: LOCAL)`, whatever backup_location
    /// says.
    pub fn local(config: &Config) -> Result<Self, AppError> {
        Ok(BackupStore::Local(LocalBackupStore::at(
            &config.public_dir,
            &config.site.db,
        )?))
    }

    /// `remote?`
    pub fn remote(&self) -> bool {
        matches!(self, BackupStore::External(_))
    }

    /// `files`: `*.{gz,tgz}`, the newest (to the second) first.
    pub async fn files(&self) -> Result<Vec<BackupFile>, AppError> {
        let mut files = match self {
            BackupStore::Local(s) => s.unsorted_files()?,
            BackupStore::External(s) => s.unsorted_files().await?,
        };
        sort_newest_first(&mut files);
        Ok(files)
    }

    /// `latest_file`
    pub async fn latest_file(&self) -> Result<Option<BackupFile>, AppError> {
        Ok(self.files().await?.into_iter().next())
    }

    /// `file(filename, include_download_source:)`: the backup of that
    /// name, if there is one. A name leading outside the local store is
    /// InvalidParameters, as Rails raises.
    pub async fn file(
        &self,
        filename: &str,
        include_download_source: bool,
    ) -> Result<Option<BackupFile>, AppError> {
        match self {
            // The local file is its own source either way.
            BackupStore::Local(s) => file_at(&s.path_from_filename(filename)?),
            BackupStore::External(s) => s.file(filename, include_download_source).await,
        }
    }

    /// `delete_file`
    pub async fn delete_file(&self, filename: &str) -> Result<(), AppError> {
        match self {
            BackupStore::Local(s) => {
                let path = s.path_from_filename(filename)?;
                if path.exists() {
                    std::fs::remove_file(path)?;
                }
                Ok(())
            }
            BackupStore::External(s) => {
                s.backend.ensure_bucket().await?;
                let key = s.key(filename);
                if s.backend.head(&key).await?.is_some() {
                    s.backend.delete(&key).await?;
                }
                Ok(())
            }
        }
    }

    /// `download_file(filename, destination, failure_message)`: the
    /// backup copied to a local path. A local copy's failure is the
    /// message and why (`cp`'s error), a download's the message alone.
    pub async fn download_file(
        &self,
        filename: &str,
        destination: &Path,
        failure_message: &str,
    ) -> Result<(), AppError> {
        match self {
            BackupStore::Local(s) => {
                let source = s.path_from_filename(filename)?;
                tokio::fs::copy(&source, destination)
                    .await
                    .map(|_| ())
                    .map_err(|e| std::io::Error::other(format!("{failure_message}\n{e}")).into())
            }
            BackupStore::External(s) => {
                let key = s.key(filename);
                match s.backend.download(&key, destination).await {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        tracing::error!("downloading {key}: {e}");
                        Err(std::io::Error::other(failure_message.to_string()).into())
                    }
                }
            }
        }
    }

    /// `upload_file(filename, source_path, content_type)`: a local archive
    /// put in the store, refused when the name is taken. Only a remote
    /// store takes one: the local store's archives are made in place.
    pub async fn upload_file(
        &self,
        filename: &str,
        source: &Path,
        content_type: &str,
    ) -> Result<(), AppError> {
        match self {
            BackupStore::Local(_) => {
                Err(BackupStoreError::NotImplemented("LocalBackupStore#upload_file").into())
            }
            BackupStore::External(s) => {
                s.backend.ensure_bucket().await?;
                let key = s.key(filename);
                if s.backend.head(&key).await?.is_some() {
                    return Err(BackupStoreError::FileExists.into());
                }
                let opts = PutOptions {
                    content_type: Some(content_type.to_string()),
                    ..Default::default()
                };
                s.backend.put_file(&key, source, &opts).await?;
                Ok(())
            }
        }
    }

    /// `generate_upload_url(filename)`: where a browser puts a backup.
    pub async fn generate_upload_url(&self, filename: &str) -> Result<String, AppError> {
        match self {
            BackupStore::Local(_) => {
                Err(BackupStoreError::NotImplemented("LocalBackupStore#generate_upload_url").into())
            }
            BackupStore::External(s) => s.generate_upload_url(filename).await,
        }
    }

    /// `cleanup_allowed?`
    fn cleanup_allowed(&self) -> bool {
        match self {
            BackupStore::Local(_) => true,
            BackupStore::External(s) => s.cleanup_allowed,
        }
    }

    /// `delete_old`: past `maximum_backups`, the oldest go.
    pub async fn delete_old(&self, maximum: i64) -> Result<(), AppError> {
        if !self.cleanup_allowed() {
            return Ok(());
        }
        let files = self.files().await?;
        let keep = usize::try_from(maximum.max(0)).unwrap_or(usize::MAX);
        if files.len() <= keep {
            return Ok(());
        }
        for file in &files[keep..] {
            self.delete_file(&file.filename).await?;
        }
        Ok(())
    }

    /// `delete_prior_to_n_days`: backups older than `remove_older_backups`
    /// days go (blank or zero: none).
    pub async fn delete_prior_to_n_days(
        &self,
        window: &str,
        now: DateTime<Utc>,
    ) -> Result<(), AppError> {
        let days = crate::ruby::to_i(window);
        if days <= 0 || !self.cleanup_allowed() {
            return Ok(());
        }
        let cutoff = now - chrono::Duration::days(days);
        for file in self.files().await? {
            if file.last_modified.and_utc() < cutoff {
                self.delete_file(&file.filename).await?;
            }
        }
        Ok(())
    }

    /// `stats`: the files' sizes and count, the newest's date and, for the
    /// local store, the free space where they are.
    pub async fn stats(&self) -> Result<Stats, AppError> {
        let files = self.files().await?;
        let free_bytes = match self {
            BackupStore::Local(s) => Some(free_bytes(s.directory())?),
            BackupStore::External(_) => None,
        };
        Ok(Stats {
            used_bytes: files.iter().map(|f| f.size).sum(),
            free_bytes,
            count: files.len(),
            last_backup_taken_at: files.first().map(|f| f.last_modified),
        })
    }
}

/// `sort_by { |file| -file.last_modified.to_i }`; ties by name, so the
/// order is stable.
fn sort_newest_first(files: &mut [BackupFile]) {
    files.sort_by(|a, b| {
        b.last_modified
            .and_utc()
            .timestamp()
            .cmp(&a.last_modified.and_utc().timestamp())
            .then_with(|| a.filename.cmp(&b.filename))
    });
}

/// BackupRestore::LocalBackupStore: the directory, where uploaded chunks
/// and new archives are made whatever `backup_location` says.
#[derive(Debug, Clone)]
pub struct LocalBackupStore {
    base_directory: PathBuf,
}

impl LocalBackupStore {
    /// `LocalBackupStore.base_directory`: `public/backups/<db>`, made when
    /// missing.
    pub fn base_directory(public_dir: &Path, db: &str) -> std::io::Result<PathBuf> {
        let dir = public_dir.join("backups").join(db);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// The directory of the site `db` (its `current_db`).
    pub fn at(public_dir: &Path, db: &str) -> Result<Self, AppError> {
        Ok(Self {
            base_directory: Self::base_directory(public_dir, db)?,
        })
    }

    pub fn directory(&self) -> &Path {
        &self.base_directory
    }

    /// `LocalBackupStore.chunk_path`: `tmp/<identifier>/<filename>.part<n>`.
    pub fn chunk_path(&self, identifier: &str, filename: &str, chunk_number: &str) -> PathBuf {
        self.base_directory
            .join("tmp")
            .join(identifier)
            .join(format!("{filename}.part{chunk_number}"))
    }

    /// `unsorted_files`: `*.{gz,tgz}` in the directory.
    fn unsorted_files(&self) -> Result<Vec<BackupFile>, AppError> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&self.base_directory)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            // Dir.glob skips dotfiles.
            if name.starts_with('.') || !(name.ends_with(".gz") || name.ends_with(".tgz")) {
                continue;
            }
            if let Some(file) = file_at(&entry.path())? {
                files.push(file);
            }
        }
        Ok(files)
    }

    fn path_from_filename(&self, filename: &str) -> Result<PathBuf, AppError> {
        if filename.is_empty()
            || filename.contains('/')
            || filename == "."
            || filename == ".."
            || filename.contains('\0')
        {
            // Discourse::InvalidParameters.new(:filename)
            return Err(std::io::Error::other(
                "You supplied invalid parameters to the request: filename",
            )
            .into());
        }
        Ok(self.base_directory.join(filename))
    }
}

fn file_at(path: &Path) -> Result<Option<BackupFile>, AppError> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !meta.is_file() {
        return Ok(None);
    }
    let modified: DateTime<Utc> = meta.modified()?.into();
    Ok(Some(BackupFile {
        filename: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size: meta.len(),
        last_modified: modified.naive_utc(),
        source: Some(Source::File(path.to_path_buf())),
    }))
}

/// `valid_extension?`: `.tar.gz`, `.tgz` or `.gz`.
pub fn valid_extension(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    lower.ends_with(".tar.gz") || lower.ends_with(".tgz") || lower.ends_with(".gz")
}

/// `valid_filename?`: `\A[a-zA-Z0-9\._\-]+\z`.
pub fn valid_filename(filename: &str) -> bool {
    !filename.is_empty()
        && filename
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `DiskSpace.free(path)`: `df -Pk`'s available kilobytes, in bytes.
pub fn free_bytes(path: &Path) -> Result<u64, AppError> {
    let out = std::process::Command::new("df")
        .arg("-Pk")
        .arg(path)
        .output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let kb = text
        .lines()
        .nth(1)
        .and_then(|l| l.split_whitespace().nth(3))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    Ok(kb * 1024)
}

/// `HandleChunkUpload#merge_chunks` for the backup chunks merger: parts 1
/// to `chunks` concatenated into `<dir>/<filename>` through a `.tmp`, then
/// the upload's tmp directory removed.
pub fn merge_chunks(
    store: &LocalBackupStore,
    filename: &str,
    identifier: &str,
    chunks: i64,
) -> Result<(), AppError> {
    use std::io::Write;
    let upload_path = store.directory().join(filename);
    let tmp_upload_path = store.directory().join(format!("{filename}.tmp"));
    let tmp_directory = store
        .chunk_path(identifier, filename, "0")
        .parent()
        .map(Path::to_path_buf);
    // Rails deletes both and stops at the first missing one.
    if std::fs::remove_file(&upload_path).is_ok() {
        let _ = std::fs::remove_file(&tmp_upload_path);
    }
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&tmp_upload_path)?;
    for n in 1..=chunks {
        let bytes = std::fs::read(store.chunk_path(identifier, filename, &n.to_string()))?;
        out.write_all(&bytes)?;
    }
    drop(out);
    std::fs::rename(&tmp_upload_path, &upload_path)?;
    if let Some(dir) = tmp_directory {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str, secs_ago: u64) {
        let path = dir.join(name);
        std::fs::write(&path, name).unwrap();
        let at = std::time::SystemTime::now() - std::time::Duration::from_secs(secs_ago);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "discourse-rs-backup-store-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    async fn names(store: &BackupStore) -> Vec<String> {
        store
            .files()
            .await
            .unwrap()
            .into_iter()
            .map(|f| f.filename)
            .collect()
    }

    #[tokio::test]
    async fn lists_newest_first_and_only_archives() {
        let public = scratch("list");
        let local = LocalBackupStore::at(&public, "default").unwrap();
        touch(local.directory(), "old.tar.gz", 3 * 86400);
        touch(local.directory(), "new.sql.gz", 60);
        touch(local.directory(), "mid.tgz", 86400);
        touch(local.directory(), "notes.txt", 0);
        touch(local.directory(), ".hidden.gz", 0);
        let store = BackupStore::Local(local);
        assert_eq!(names(&store).await, ["new.sql.gz", "mid.tgz", "old.tar.gz"]);
        let stats = store.stats().await.unwrap();
        assert_eq!(stats.count, 3);
        assert_eq!(
            stats.used_bytes,
            ("new.sql.gz".len() + "mid.tgz".len() + "old.tar.gz".len()) as u64
        );
        assert!(stats.free_bytes.is_some());

        store.delete_prior_to_n_days("2", Utc::now()).await.unwrap();
        assert!(store.file("old.tar.gz", false).await.unwrap().is_none());
        store.delete_prior_to_n_days("", Utc::now()).await.unwrap();
        store.delete_old(1).await.unwrap();
        assert_eq!(names(&store).await, ["new.sql.gz"]);

        let copy = public.join("copy.sql.gz");
        store
            .download_file("new.sql.gz", &copy, "Failed to copy")
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "new.sql.gz");
        let missing = store
            .download_file("gone.sql.gz", &copy, "Failed to copy")
            .await
            .unwrap_err();
        assert!(missing.to_string().starts_with("Failed to copy\n"));
        std::fs::remove_dir_all(public).unwrap();
    }

    #[test]
    fn merges_chunks_in_order() {
        let public = scratch("merge");
        let store = LocalBackupStore::at(&public, "default").unwrap();
        for (n, part) in [(1, "first "), (2, "second")] {
            let path = store.chunk_path("id-1", "up.tar.gz", &n.to_string());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, part).unwrap();
        }
        merge_chunks(&store, "up.tar.gz", "id-1", 2).unwrap();
        assert_eq!(
            std::fs::read_to_string(store.directory().join("up.tar.gz")).unwrap(),
            "first second"
        );
        assert!(!store.directory().join("tmp/id-1").exists());
        std::fs::remove_dir_all(public).unwrap();
    }

    #[test]
    fn filenames_and_extensions() {
        assert!(valid_filename(
            "discourse-2026-10-01-033000-v20260925054715.tar.gz"
        ));
        assert!(!valid_filename("back up.tar.gz"));
        assert!(!valid_filename("a/b.tar.gz"));
        assert!(valid_extension("x.TGZ"));
        assert!(valid_extension("x.sql.gz"));
        assert!(!valid_extension("x.zip"));
    }
}
