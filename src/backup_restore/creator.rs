//! BackupRestore::Creator: a backup of the site. The backup_metadata rows
//! rewritten, pg_dump of the public schema, then either the dump alone
//! (`<name>.sql.gz`) or a tar of it and the local uploads gzipped
//! (`<name>.tar.gz`), in the local store; every step logged, on
//! `/admin/backups/logs` when asked, and the logs sent to the admin who
//! asked as a system message. Cancelled through the shutdown flag.
//!
//! Refused: sites with S3 uploads (the S3 URLs in the metadata, the
//! remote uploads in the archive) and the S3 store.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};

use super::store::{CURRENT_DB, LocalBackupStore};
use crate::config::RailsEnv;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `BackupRestore::DUMP_FILE`
pub const DUMP_FILE: &str = "dump.sql.gz";
/// `Discourse::SYSTEM_USER_ID`
const SYSTEM_USER_ID: i32 = -1;
/// `Group::AUTO_GROUPS[:admins]`
const ADMINS_GROUP_ID: i32 = 1;

/// What `BackupRestore.backup!` is given.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub publish_to_message_bus: bool,
    /// `opts[:with_uploads]`: only an explicit false leaves them out.
    pub with_uploads: Option<bool>,
    pub client_id: Option<String>,
    /// `opts[:filename]`, in place of the title and timestamp.
    pub filename: Option<String>,
}

/// How a run ended: `success`, the backup's file name, the logs as saved.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub success: bool,
    pub filename: String,
    pub logs: Vec<String>,
}

/// The operation's log: saved lines (`[time] message`), and each one on
/// the logs channel for the admin who asked when publishing.
pub(crate) struct Log<'a> {
    pub(crate) state: &'a AppState,
    pub(crate) user_id: i32,
    pub(crate) operation: &'static str,
    pub(crate) publish: bool,
    pub(crate) lines: Vec<String>,
}

impl Log<'_> {
    /// `log(message)`: `Time.now.strftime("%Y-%m-%d %H:%M:%S")` (UTC here).
    pub(crate) async fn log(&mut self, message: &str) {
        let timestamp = crate::clock::now().format("%Y-%m-%d %H:%M:%S").to_string();
        tracing::info!(operation = self.operation, "{message}");
        if self.publish {
            // MessageBus's client_ids filter has no counterpart: every tab
            // of the admin hears the logs.
            let data = json!({
                "timestamp": timestamp,
                "operation": self.operation,
                "message": message,
            });
            let published = async {
                let mut conn = self.state.pool.acquire().await?;
                self.state
                    .bus
                    .publish(
                        &mut conn,
                        super::LOGS_CHANNEL,
                        &data,
                        Some(&[crate::bus::user_tag(self.user_id)]),
                    )
                    .await?;
                Ok::<_, AppError>(())
            };
            if let Err(e) = published.await {
                tracing::error!("publishing a backup log line: {e}");
            }
        }
        self.lines.push(format!("[{timestamp}] {message}"));
    }
}

/// `BackupRestore::OperationRunningError`
#[derive(Debug)]
pub struct OperationRunning;

impl std::fmt::Display for OperationRunning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BackupRestore::OperationRunningError")
    }
}

impl std::error::Error for OperationRunning {}

/// `String#parameterize`: transliterated to ASCII (accents dropped, the
/// rest a separator), lowercase, runs of anything but `[a-z0-9_-]` one
/// `-`, none at either end.
pub fn parameterize(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let mut out = String::new();
    for c in s.nfkd() {
        if unicode_normalization::char::is_combining_mark(c) {
            continue;
        }
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            if c == '-' && out.ends_with('-') {
                continue;
            }
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// `BackupRestore.current_version`: the latest migration run.
pub async fn current_version(conn: &mut sqlx::PgConnection) -> Result<String, sqlx::Error> {
    let version: Option<String> = sqlx::query_scalar("SELECT max(version) FROM schema_migrations")
        .fetch_one(conn)
        .await?;
    Ok(version.unwrap_or_else(|| "0".into()))
}

/// The connection pg_dump and psql get: the pool's database, with the
/// password from DATABASE_URL (PGPASSWORD) when it has one.
pub(crate) struct DbArgs {
    pub(crate) args: Vec<String>,
    pub(crate) password: Option<String>,
    pub(crate) database: String,
}

pub(crate) fn database_args(state: &AppState) -> DbArgs {
    let opts = state.pool.connect_options();
    let mut args = Vec::new();
    match opts.get_socket() {
        Some(socket) => args.push(format!("--host={}", socket.display())),
        None => args.push(format!("--host={}", opts.get_host())),
    }
    args.push(format!("--port={}", opts.get_port()));
    args.push(format!("--username={}", opts.get_username()));
    let database = opts.get_database().unwrap_or("discourse").to_string();
    DbArgs {
        args,
        password: url_password(&state.config.database_url),
        database,
    }
}

/// The password in a postgres URL's userinfo, percent-decoded.
fn url_password(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?']).next()?;
    let userinfo = authority.rsplit_once('@')?.0;
    let password = userinfo.split_once(':')?.1;
    (!password.is_empty()).then(|| {
        form_urlencoded::parse(format!("p={password}").as_bytes())
            .next()
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default()
    })
}

/// `Discourse::Utils.execute_command`: the command run (in `dir`), its
/// status one of `ok`, else the failure message and its output.
async fn execute(
    program: &str,
    args: &[&str],
    dir: Option<&Path>,
    ok: &[i32],
    failure_message: &str,
) -> Result<(), AppError> {
    let mut command = tokio::process::Command::new(program);
    command.args(args).kill_on_drop(true);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let out = command.output().await?;
    if out.status.code().is_some_and(|c| ok.contains(&c)) {
        return Ok(());
    }
    let mut message = failure_message.to_string();
    if message.is_empty() {
        message = format!("{program} failed");
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(std::io::Error::other(format!("{message}\n{}", stderr.trim_end())).into())
}

struct Run<'a> {
    state: &'a AppState,
    settings: SiteSettings,
    user: Option<(i32, String)>,
    with_uploads: bool,
    store: LocalBackupStore,
    tmp_directory: PathBuf,
    dump_filename: PathBuf,
    archive_directory: PathBuf,
    archive_basename: PathBuf,
    backup_filename: String,
    log: Log<'a>,
}

/// `BackupRestore::Creator.new(user_id, opts).run`. Errors inside the run
/// are logged and end it as failed, as Rails rescues them; only the setup
/// (the settings, the store) fails the call.
pub async fn run(state: &AppState, user_id: i32, opts: Options) -> Result<Outcome, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if settings.get("enable_s3_uploads")?.truthy() || state.config.globals.use_s3() {
        return Err(Unsupported("backups of a site whose uploads are on S3").into());
    }
    let store = LocalBackupStore::create(state, &settings)?;
    let user: Option<(i32, String)> =
        sqlx::query_as("SELECT id, username FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
    let has_local_uploads = has_local_uploads(&state.config.public_dir);
    let with_uploads = if opts.with_uploads == Some(false) {
        false
    } else {
        has_local_uploads || settings.get("include_s3_uploads_in_backups")?.truthy()
    };
    let timestamp = crate::clock::now().format("%Y-%m-%d-%H%M%S").to_string();
    let tmp_directory = state
        .config
        .tmp_dir
        .join("backups")
        .join(CURRENT_DB)
        .join(&timestamp);
    let archive_directory = store.directory().to_path_buf();
    let title = parameterize(&settings.get("title")?.to_s());
    let title: String = title.chars().take(64).collect();
    let name = opts.filename.clone().unwrap_or_else(|| {
        format!(
            "{}-{timestamp}",
            if title.is_empty() {
                "discourse"
            } else {
                &title
            }
        )
    });
    let version = current_version(&mut conn).await?;
    drop(conn);
    let archive_basename = archive_directory.join(format!("{name}-v{version}"));
    let base = archive_basename
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let backup_filename = if with_uploads {
        format!("{base}.tar.gz")
    } else {
        format!("{base}.sql.gz")
    };
    let mut run = Run {
        state,
        settings,
        user,
        with_uploads,
        store,
        dump_filename: tmp_directory.join(DUMP_FILE),
        tmp_directory,
        archive_directory,
        archive_basename,
        backup_filename,
        log: Log {
            state,
            user_id,
            operation: "backup",
            publish: opts.publish_to_message_bus,
            lines: Vec::new(),
        },
    };
    let success = run.run().await;
    Ok(Outcome {
        success,
        filename: run.backup_filename,
        logs: run.log.lines,
    })
}

fn has_local_uploads(public_dir: &Path) -> bool {
    std::fs::read_dir(public_dir.join("uploads").join(CURRENT_DB))
        .map(|mut d| d.next().is_some())
        .unwrap_or(false)
}

impl Run<'_> {
    async fn run(&mut self) -> bool {
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let outcome = {
            let work = self.work(stop_rx.clone());
            work.await
        };
        let _ = stop_tx.send(true);
        let success = match outcome {
            Ok(()) => true,
            Err(Stopped::Cancelled) => {
                self.log.log("Backup process was cancelled!").await;
                false
            }
            Err(Stopped::Failed(e)) => {
                self.log.log(&format!("EXCEPTION: {e}")).await;
                false
            }
        };
        self.delete_old().await;
        self.clean_up(success).await;
        self.notify_user(success).await;
        self.log.log("Finished!").await;
        // publish_completion (the backup_complete and backup_failed
        // events have no core listeners).
        self.log
            .log(if success { "[SUCCESS]" } else { "[FAILED]" })
            .await;
        success
    }

    async fn work(&mut self, stop: tokio::sync::watch::Receiver<bool>) -> Result<(), Stopped> {
        let mut conn = self.state.pool.acquire().await.map_err(Stopped::from)?;
        if super::is_operation_running(&mut conn).await? {
            return Err(Stopped::Failed(OperationRunning.into()));
        }
        let Some((_, username)) = self.user.clone() else {
            return Err(Stopped::Failed(
                std::io::Error::other("You supplied invalid parameters to the request: user_id")
                    .into(),
            ));
        };
        drop(conn);
        self.log.log("[STARTED]").await;
        self.log
            .log(&format!("'{username}' has started the backup!"))
            .await;
        self.log.log("Marking backup as running...").await;
        let cancelled = super::keep_running(self.state, stop).await?;

        let steps = async {
            for dir in [self.tmp_directory.clone(), self.archive_directory.clone()] {
                self.log
                    .log(&format!("Making sure '{}' exists...", dir.display()))
                    .await;
                std::fs::create_dir_all(&dir).map_err(AppError::from)?;
            }
            self.update_metadata().await?;
            self.dump_public_schema().await?;
            self.log.log("Finalizing backup...").await;
            if self.with_uploads {
                self.create_archive().await?;
            } else {
                self.move_dump_backup().await?;
            }
            // upload_archive: the local store keeps it where it is.
            self.log
                .log("Executing the after_create_hook for the backup...")
                .await;
            Ok::<_, AppError>(())
        };
        tokio::select! {
            result = steps => result.map_err(Stopped::Failed),
            () = cancelled => Err(Stopped::Cancelled),
        }
    }

    async fn update_metadata(&mut self) -> Result<(), AppError> {
        self.log.log("Updating metadata...").await;
        let urls = crate::url::Urls {
            config: &self.state.config,
            settings: &self.settings,
        };
        let base_url = urls.base_url()?;
        let cdn_url = urls.asset_host().map(str::to_string);
        let mut tx = self.state.pool.begin().await?;
        sqlx::query("DELETE FROM backup_metadata")
            .execute(&mut *tx)
            .await?;
        for (name, value) in [
            ("base_url", Some(base_url)),
            ("cdn_url", cdn_url),
            ("s3_base_url", None),
            ("s3_cdn_url", None),
            ("db_name", Some(CURRENT_DB.to_string())),
            // Rails.configuration.multisite, a boolean in a string column.
            ("multisite", Some("f".to_string())),
        ] {
            sqlx::query("INSERT INTO backup_metadata (name, value) VALUES ($1, $2)")
                .bind(name)
                .bind(value)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn dump_public_schema(&mut self) -> Result<(), AppError> {
        self.log
            .log("Dumping the public schema of the database...")
            .await;
        let db = database_args(self.state);
        let mut command = tokio::process::Command::new("pg_dump");
        command
            .arg("--schema=public")
            .arg("-T")
            .arg("public.pg_*")
            .arg("--exclude-table-data=public.nested_hot_post_scores")
            .arg("--exclude-table-data=public.nested_hot_score_snapshots")
            .arg(format!("--file={}", self.dump_filename.display()))
            .arg("--no-owner")
            .arg("--no-privileges")
            .arg("--verbose")
            .arg("--compress=4")
            .args(&db.args)
            .arg(&db.database)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(password) = &db.password {
            command.env("PGPASSWORD", password);
        }
        let mut child = command.spawn()?;
        // Rails reads `2>&1`: both streams are log lines.
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        for stream in [
            stdout.map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
            stderr.map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stream).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        while let Some(line) = rx.recv().await {
            let line = line.trim();
            if !line.is_empty() {
                self.log.log(line).await;
            }
        }
        let status = child.wait().await?;
        if !status.success() {
            return Err(std::io::Error::other("pg_dump failed").into());
        }
        Ok(())
    }

    async fn move_dump_backup(&mut self) -> Result<(), AppError> {
        self.log
            .log(&format!(
                "Finalizing database dump file: {}",
                self.backup_filename
            ))
            .await;
        let archive_filename = self.archive_directory.join(&self.backup_filename);
        if let Err(error) = move_file(&self.dump_filename, &archive_filename) {
            return Err(std::io::Error::other(format!(
                "Failed to move database dump file.\n{error}"
            ))
            .into());
        }
        self.remove_tmp_directory().await;
        Ok(())
    }

    async fn create_archive(&mut self) -> Result<(), AppError> {
        self.log
            .log(&format!("Creating archive: {}", self.backup_filename))
            .await;
        let tar_filename = PathBuf::from(format!("{}.tar", self.archive_basename.display()));
        let tar = tar_filename.to_string_lossy().into_owned();
        self.log
            .log("Making sure archive does not already exist...")
            .await;
        for f in [tar.clone(), format!("{tar}.gz")] {
            let _ = std::fs::remove_file(f);
        }
        self.log.log("Creating empty archive...").await;
        execute(
            "tar",
            &["--create", "--file", &tar, "--files-from", "/dev/null"],
            None,
            &[0],
            "",
        )
        .await?;
        self.log.log("Archiving data dump...").await;
        execute(
            "tar",
            &["--append", "--dereference", "--file", &tar, DUMP_FILE],
            Some(&self.tmp_directory),
            &[0],
            "Failed to archive data dump.",
        )
        .await?;
        self.log.log("Archiving uploads...").await;
        if has_local_uploads(&self.state.config.public_dir) {
            let upload_directory = format!("uploads/{CURRENT_DB}");
            let exclude = format!("--exclude={upload_directory}/optimized");
            let mut args = vec!["--append", "--dereference"];
            if !self.settings.get("include_thumbnails_in_backups")?.truthy() {
                args.push(&exclude);
            }
            args.extend(["--file", &tar, &upload_directory]);
            execute(
                "tar",
                &args,
                Some(&self.state.config.public_dir),
                &[0, 1],
                "Failed to archive uploads.",
            )
            .await?;
        } else {
            self.log
                .log("No local uploads found. Skipping archiving of local uploads...")
                .await;
        }
        self.remove_tmp_directory().await;
        self.log
            .log("Gzipping archive, this may take a while...")
            .await;
        let level = format!(
            "-{}",
            self.settings
                .get("backup_gzip_compression_level_for_uploads")?
                .to_i()
        );
        execute(
            "gzip",
            &[&level, &tar],
            None,
            &[0],
            "Failed to gzip archive.",
        )
        .await?;
        Ok(())
    }

    async fn remove_tmp_directory(&mut self) {
        self.log
            .log(&format!(
                "Removing tmp '{}' directory...",
                self.tmp_directory.display()
            ))
            .await;
        if self.tmp_directory.exists()
            && let Err(e) = std::fs::remove_dir_all(&self.tmp_directory)
        {
            self.log
                .log(&format!(
                    "Something went wrong while removing the following tmp directory: {}",
                    self.tmp_directory.display()
                ))
                .await;
            tracing::error!("{e}");
        }
    }

    /// `delete_old`: past maximum_backups the oldest go; not in
    /// development.
    async fn delete_old(&mut self) {
        if self.state.config.rails_env == RailsEnv::Development {
            return;
        }
        self.log.log("Deleting old backups...").await;
        let deleted = match self.settings.get("maximum_backups") {
            Ok(max) => self.store.delete_old(max.to_i()),
            Err(e) => Err(e.into()),
        };
        if let Err(e) = deleted {
            self.log
                .log("Something went wrong while deleting old backups.")
                .await;
            tracing::error!("{e}");
        }
    }

    async fn clean_up(&mut self, success: bool) {
        self.log.log("Cleaning stuff up...").await;
        self.log.log("Removing '.tar' leftovers...").await;
        if let Ok(entries) = std::fs::read_dir(&self.archive_directory) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".tar") && !name.starts_with('.') {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        self.log.log("Marking backup as finished...").await;
        let marked = async {
            let mut conn = self.state.pool.acquire().await?;
            super::mark_as_not_running(&mut conn).await
        };
        if let Err(e) = marked.await {
            self.log
                .log("Something went wrong while marking backup as finished.")
                .await;
            tracing::error!("{e}");
        }
        if success {
            self.log.log("Refreshing disk stats...").await;
        }
    }

    /// `notify_user`: the logs to the admin as backup_succeeded or
    /// backup_failed; none for the system user's successes, and its
    /// failures go to the admins too.
    async fn notify_user(&mut self, success: bool) {
        let Some((user_id, username)) = self.user.clone() else {
            self.log
                .log("Something went wrong while notifying user.")
                .await;
            return;
        };
        if success && user_id == SYSTEM_USER_ID {
            return;
        }
        self.log
            .log(&format!(
                "Notifying '{username}' of the end of the backup..."
            ))
            .await;
        let message_type = if success {
            "backup_succeeded"
        } else {
            "backup_failed"
        };
        if let Err(e) = self.send_logs(user_id, message_type).await {
            self.log
                .log("Something went wrong while notifying user.")
                .await;
            tracing::error!("{e}");
        }
    }

    async fn send_logs(&self, user_id: i32, message_type: &str) -> Result<(), AppError> {
        let logs = logs_markdown(self.state, &self.settings, &self.log.lines, user_id).await?;
        let host = crate::pretty_text::Host::from_state(self.state);
        let ctx = crate::posting::Ctx {
            host: &host,
            settings: &self.settings,
            config: &self.state.config,
            i18n: &self.state.i18n,
            bus: &self.state.bus,
        };
        let post_id = crate::system_message::create_from_system_user(
            &self.state.pool,
            &ctx,
            user_id,
            message_type,
            &[("logs".to_string(), logs)],
        )
        .await?;
        if user_id == SYSTEM_USER_ID {
            let mut tx = self.state.pool.begin().await?;
            let topic_id: i32 = sqlx::query_scalar("SELECT topic_id FROM posts WHERE id = $1")
                .bind(post_id)
                .fetch_one(&mut *tx)
                .await?;
            crate::pm_invite::invite_group(&mut tx, &ctx, user_id, ADMINS_GROUP_ID, topic_id, true)
                .await?;
            tx.commit().await?;
        }
        Ok(())
    }
}

/// `Discourse::Utils.logs_markdown`: the logs in a text block when they
/// fit a post, else a `log.txt.zip` upload of the user's linked as an
/// attachment, else the logs' tail.
pub(crate) async fn logs_markdown(
    state: &AppState,
    settings: &SiteSettings,
    logs: &[String],
    user_id: i32,
) -> Result<String, AppError> {
    let max = settings.get("max_post_length")?.to_i() - 250;
    let pretty = logs.join("\n");
    let length = pretty.chars().count() as i64;
    if length < max {
        return Ok(format!("```text\n{pretty}\n```\n"));
    }
    let zip = zip_one("log.txt", pretty.as_bytes())?;
    let staff: bool = sqlx::query_scalar(
        "SELECT COALESCE((SELECT admin OR moderator FROM users WHERE id = $1), FALSE)",
    )
    .bind(user_id)
    .fetch_one(&state.pool)
    .await?;
    let mut conn = state.pool.acquire().await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let outcome = crate::uploads::create(
        &mut conn,
        settings,
        &state.i18n,
        &urls,
        &crate::file_store::FileStore::for_site(&state.config, settings)?,
        &crate::uploads::NewUpload {
            user_id,
            staff,
            upload_type: "backup_logs",
            filename: "log.txt.zip",
            bytes: &zip,
            pasted: false,
            for_private_message: false,
            for_export: true,
        },
    )
    .await?;
    match outcome {
        crate::uploads::Outcome::Created(upload) => {
            let id = upload["id"].as_i64().unwrap_or_default();
            let (name, sha1, extension, size): (String, String, Option<String>, i64) =
                sqlx::query_as(
                    "SELECT original_filename, sha1, extension, filesize::bigint FROM uploads \
                     WHERE id = $1",
                )
                .bind(id as i32)
                .fetch_one(&mut *conn)
                .await?;
            // UploadMarkdown#attachment_markdown
            let base = crate::pretty_text::helpers::base62_sha1(&sha1).unwrap_or_default();
            let ext = extension
                .filter(|e| !e.is_empty())
                .map(|e| format!(".{e}"))
                .unwrap_or_default();
            let label = name.replace(['[', ']', '|'], "");
            Ok(format!(
                "[{label}|attachment](upload://{base}{ext}) ({})",
                crate::uploads::human_size(size)
            ))
        }
        crate::uploads::Outcome::Invalid(errors) => {
            tracing::warn!(
                "Failed to upload the backup logs file: {}",
                errors.join(", ")
            );
            let skip = (length - max.max(0)).max(0) as usize;
            let tail: String = pretty.chars().skip(skip).collect();
            Ok(format!("```text\n...\n{tail}\n```\n"))
        }
    }
}

/// `Compression::Zip#compress` of one file: a zip archive holding it,
/// deflated.
pub(crate) fn zip_one(name: &str, data: &[u8]) -> std::io::Result<Vec<u8>> {
    use std::io::Write;
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data)?;
    let compressed = encoder.finish()?;
    let crc = crc32fast::hash(data);
    // MS-DOS time and date of now (UTC).
    let now = crate::clock::now();
    use chrono::{Datelike, Timelike};
    let time = ((now.hour() << 11) | (now.minute() << 5) | (now.second() / 2)) as u16;
    let date = (((now.year() - 1980).max(0) as u32) << 9 | (now.month() << 5) | now.day()) as u16;
    let size = |n: usize| u32::try_from(n).map_err(std::io::Error::other);
    let (csize, usize_) = (size(compressed.len())?, size(data.len())?);
    let name_len = u16::try_from(name.len()).map_err(std::io::Error::other)?;
    let mut out = Vec::new();
    // Local file header.
    out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    out.extend_from_slice(&20u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&8u16.to_le_bytes());
    out.extend_from_slice(&time.to_le_bytes());
    out.extend_from_slice(&date.to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&csize.to_le_bytes());
    out.extend_from_slice(&usize_.to_le_bytes());
    out.extend_from_slice(&name_len.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(&compressed);
    let central_offset = size(out.len())?;
    // Central directory.
    out.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
    out.extend_from_slice(&20u16.to_le_bytes());
    out.extend_from_slice(&20u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&8u16.to_le_bytes());
    out.extend_from_slice(&time.to_le_bytes());
    out.extend_from_slice(&date.to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&csize.to_le_bytes());
    out.extend_from_slice(&usize_.to_le_bytes());
    out.extend_from_slice(&name_len.to_le_bytes());
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(name.as_bytes());
    let central_size = size(out.len())? - central_offset;
    // End of central directory.
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    Ok(out)
}

/// FileUtils.mv: a rename, or a copy and delete across file systems.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(from, to)?;
            std::fs::remove_file(from)
        }
    }
}

enum Stopped {
    Cancelled,
    Failed(AppError),
}

impl<E: Into<AppError>> From<E> for Stopped {
    fn from(e: E) -> Self {
        Stopped::Failed(e.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameterizes_titles_as_rails() {
        assert_eq!(parameterize("Discourse"), "discourse");
        assert_eq!(parameterize("My  Forum -- Crème!"), "my-forum-creme");
        assert_eq!(parameterize("__a b__"), "__a-b__");
        assert_eq!(parameterize("日本"), "");
    }

    #[test]
    fn reads_the_password_from_the_url() {
        assert_eq!(
            url_password("postgres://u:p%40ss@h:5432/db"),
            Some("p@ss".into())
        );
        assert_eq!(url_password("postgresql://localhost:5442/db?host=/x"), None);
        assert_eq!(url_password("postgres://u@h/db"), None);
    }
}
