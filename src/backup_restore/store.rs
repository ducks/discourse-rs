//! BackupRestore::LocalBackupStore: the backups are the `.gz` and `.tgz`
//! files in `public/backups/<db>`, newest first. The S3 store is refused.

use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::{Value, json};

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// RailsMultisite::ConnectionManagement.current_db of a single site.
pub const CURRENT_DB: &str = "default";

/// BackupRestore::BackupFile
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupFile {
    pub filename: String,
    pub size: u64,
    /// `File.mtime(path).utc`
    pub last_modified: NaiveDateTime,
    pub path: PathBuf,
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

/// BackupRestore::LocalBackupStore
#[derive(Debug, Clone)]
pub struct LocalBackupStore {
    base_directory: PathBuf,
}

impl LocalBackupStore {
    /// `LocalBackupStore.base_directory`: `public/backups/<db>`, made when
    /// missing.
    pub fn base_directory(public_dir: &Path) -> std::io::Result<PathBuf> {
        let dir = public_dir.join("backups").join(CURRENT_DB);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// `BackupStore.create` for `backup_location` local; S3 is refused.
    pub fn create(state: &AppState, settings: &SiteSettings) -> Result<Self, AppError> {
        let location = settings.get("backup_location")?.to_s();
        if location != "local" {
            return Err(Unsupported("the S3 backup store").into());
        }
        Self::at(&state.config.public_dir)
    }

    pub fn at(public_dir: &Path) -> Result<Self, AppError> {
        Ok(Self {
            base_directory: Self::base_directory(public_dir)?,
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

    /// `files`: `*.{gz,tgz}`, the newest (to the second) first.
    pub fn files(&self) -> Result<Vec<BackupFile>, AppError> {
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
        files.sort_by(|a, b| {
            b.last_modified
                .and_utc()
                .timestamp()
                .cmp(&a.last_modified.and_utc().timestamp())
                .then_with(|| a.filename.cmp(&b.filename))
        });
        Ok(files)
    }

    /// `latest_file`
    pub fn latest_file(&self) -> Result<Option<BackupFile>, AppError> {
        Ok(self.files()?.into_iter().next())
    }

    /// `file(filename)`: the backup of that name, if there is one. A name
    /// leading outside the directory is InvalidParameters, as Rails raises.
    pub fn file(&self, filename: &str) -> Result<Option<BackupFile>, AppError> {
        let path = self.path_from_filename(filename)?;
        file_at(&path)
    }

    /// `delete_file`
    pub fn delete_file(&self, filename: &str) -> Result<(), AppError> {
        let path = self.path_from_filename(filename)?;
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
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

    /// `delete_old`: past `maximum_backups`, the oldest go (skipped in
    /// development, as Creator#delete_old returns there).
    pub fn delete_old(&self, maximum: i64) -> Result<(), AppError> {
        let files = self.files()?;
        let keep = usize::try_from(maximum.max(0)).unwrap_or(usize::MAX);
        if files.len() <= keep {
            return Ok(());
        }
        for file in &files[keep..] {
            self.delete_file(&file.filename)?;
        }
        Ok(())
    }

    /// `delete_prior_to_n_days`: backups older than `remove_older_backups`
    /// days go (blank or zero: none).
    pub fn delete_prior_to_n_days(&self, window: &str, now: DateTime<Utc>) -> Result<(), AppError> {
        let days = crate::ruby::to_i(window);
        if days <= 0 {
            return Ok(());
        }
        let cutoff = now - chrono::Duration::days(days);
        for file in self.files()? {
            if file.last_modified.and_utc() < cutoff {
                self.delete_file(&file.filename)?;
            }
        }
        Ok(())
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
        path: path.to_path_buf(),
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

    #[test]
    fn lists_newest_first_and_only_archives() {
        let public = scratch("list");
        let store = LocalBackupStore::at(&public).unwrap();
        touch(store.directory(), "old.tar.gz", 3 * 86400);
        touch(store.directory(), "new.sql.gz", 60);
        touch(store.directory(), "mid.tgz", 86400);
        touch(store.directory(), "notes.txt", 0);
        touch(store.directory(), ".hidden.gz", 0);
        let names: Vec<String> = store
            .files()
            .unwrap()
            .into_iter()
            .map(|f| f.filename)
            .collect();
        assert_eq!(names, ["new.sql.gz", "mid.tgz", "old.tar.gz"]);

        store.delete_prior_to_n_days("2", Utc::now()).unwrap();
        assert!(store.file("old.tar.gz").unwrap().is_none());
        store.delete_prior_to_n_days("", Utc::now()).unwrap();
        store.delete_old(1).unwrap();
        let names: Vec<String> = store
            .files()
            .unwrap()
            .into_iter()
            .map(|f| f.filename)
            .collect();
        assert_eq!(names, ["new.sql.gz"]);
        std::fs::remove_dir_all(public).unwrap();
    }

    #[test]
    fn merges_chunks_in_order() {
        let public = scratch("merge");
        let store = LocalBackupStore::at(&public).unwrap();
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
