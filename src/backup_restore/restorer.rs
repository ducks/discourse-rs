//! BackupRestore::Restorer: a backup put in place of the site's database
//! and uploads. The archive unpacked in tmp/restores, its schema version
//! checked, the site read only and the job queue paused while the public
//! schema's tables move to `backup` and the dump is loaded with psql;
//! then the restored settings, the uploads copied and remapped to this
//! site, and the logs to the admin as a system message. Any failure moves
//! the old tables back (`rollback`).
//!
//! Where the port can't do what Rails does, it says so in the logs as
//! Rails does when that step fails: `rake db:seed` ("Failed to run seed
//! fu") and recompiling theme fields ("Something went wrong while
//! clearing theme cache."). A backup whose schema needs migrations fails
//! the restore (the port has no db:migrate), as do S3 stores and S3
//! uploads.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::creator::{Log, OperationRunning, database_args, logs_markdown};
use super::store::LocalBackupStore;
use crate::config::RailsEnv;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `Discourse::SYSTEM_USER_ID`
const SYSTEM_USER_ID: i32 = -1;
/// `SidekiqPauser::PAUSED_KEY`, the job queue's pause.
pub const PAUSED_KEY: &str = "sidekiq_is_paused_v2";
/// `BackupMetadata::LAST_RESTORE_DATE`
pub const LAST_RESTORE_DATE: &str = "last_restore_date";
/// `DatabaseRestorer::DROP_BACKUP_SCHEMA_AFTER_DAYS`
const DROP_BACKUP_SCHEMA_AFTER_DAYS: i64 = 7;
/// `BackupFileHandler::OLD_DUMP_FILENAME`
const OLD_DUMP_FILENAME: &str = "dump.sql";

/// What spawn_backup_restore's restore is given.
#[derive(Debug, Clone)]
pub struct Options {
    pub filename: Option<String>,
    pub client_id: Option<String>,
    /// `opts.fetch(:disable_emails, true)`
    pub disable_emails: bool,
}

/// How a restore ended.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub success: bool,
    pub logs: Vec<String>,
}

/// Why `Restorer.new` refuses before anything runs.
#[derive(Debug)]
pub enum Refused {
    /// BackupRestore::RestoreDisabledError: allow_restore is off.
    RestoreDisabled,
    /// Discourse::InvalidParameters(:user_id)
    NoUser,
    /// BackupRestore::FilenameMissingError
    FilenameMissing,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Refused::RestoreDisabled => "BackupRestore::RestoreDisabledError",
            Refused::NoUser => "You supplied invalid parameters to the request: user_id",
            Refused::FilenameMissing => "BackupRestore::FilenameMissingError",
        })
    }
}

impl std::error::Error for Refused {}

/// Compression::Strategy::ExtractFailed: the dump is bigger than
/// decompressed_backup_max_file_size_mb.
#[derive(Debug)]
struct ExtractFailed;

impl std::fmt::Display for ExtractFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Compression::Strategy::ExtractFailed")
    }
}

impl std::error::Error for ExtractFailed {}

enum Stopped {
    Cancelled,
    Failed(AppError),
}

impl<E: Into<AppError>> From<E> for Stopped {
    fn from(e: E) -> Self {
        Stopped::Failed(e.into())
    }
}

fn error(message: impl Into<String>) -> AppError {
    std::io::Error::other(message.into()).into()
}

struct UserInfo {
    username: String,
    email: String,
}

struct Restore<'a> {
    state: &'a AppState,
    user_id: i32,
    user: UserInfo,
    filename: String,
    disable_emails: bool,
    log: Log<'a>,
    tmp_directory: Option<PathBuf>,
    db_was_changed: bool,
    created_functions: Vec<String>,
    readonly_mode_was_enabled: bool,
    readonly_alive: Option<tokio::sync::watch::Sender<bool>>,
    paused_alive: Option<tokio::sync::watch::Sender<bool>>,
}

/// `BackupRestore::Restorer.new(...).run`. The refusals of `new` (restore
/// not allowed, no such user, no file name) come back as errors, nothing
/// logged; everything after is logged and ends as `Outcome`.
pub async fn run(state: &AppState, user_id: i32, opts: Options) -> Result<Outcome, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if state.config.rails_env != RailsEnv::Development && !settings.get("allow_restore")?.truthy() {
        return Err(Refused::RestoreDisabled.into());
    }
    let user: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT u.username, e.email FROM users u \
         LEFT JOIN user_emails e ON e.user_id = u.id AND e.primary WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((username, email)) = user else {
        return Err(Refused::NoUser.into());
    };
    let Some(filename) = opts.filename.clone() else {
        return Err(Refused::FilenameMissing.into());
    };
    // SystemInterface.new: was the site read only already?
    let readonly_mode_was_enabled = crate::readonly::mode(&mut conn).await?.site_readonly();
    drop(conn);
    let mut restore = Restore {
        state,
        user_id,
        user: UserInfo {
            username,
            email: email.unwrap_or_default(),
        },
        filename,
        disable_emails: opts.disable_emails,
        log: Log {
            state,
            user_id,
            operation: "restore",
            // Logger: published with a user and a client id.
            publish: opts.client_id.as_deref().is_some_and(|c| !c.is_empty()),
            lines: Vec::new(),
        },
        tmp_directory: None,
        db_was_changed: false,
        created_functions: Vec::new(),
        readonly_mode_was_enabled,
        readonly_alive: None,
        paused_alive: None,
    };
    let success = restore.run().await;
    Ok(Outcome {
        success,
        logs: restore.log.lines,
    })
}

impl Restore<'_> {
    async fn run(&mut self) -> bool {
        self.log.log("[STARTED]").await;
        let username = self.user.username.clone();
        self.log
            .log(&format!("'{username}' has started the restore!"))
            .await;
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let outcome = self.work(stop_rx).await;
        let _ = stop_tx.send(true);
        let success = match outcome {
            Ok(()) => true,
            Err(Stopped::Cancelled) => {
                self.log.log("Restore process was cancelled!").await;
                self.rollback().await;
                false
            }
            Err(Stopped::Failed(e)) if e.to_string() == ExtractFailed.to_string() => {
                self.log
                    .log(
                        "ERROR: The uncompressed file is too big. Consider increasing the hidden \
                         \"decompressed_backup_max_file_size_mb\" setting.",
                    )
                    .await;
                self.rollback().await;
                false
            }
            Err(Stopped::Failed(e)) => {
                self.log.log(&format!("EXCEPTION: {e}")).await;
                self.rollback().await;
                false
            }
        };
        self.clean_up().await;
        self.notify_user(success).await;
        self.log.log("Finished!").await;
        self.log
            .log(if success { "[SUCCESS]" } else { "[FAILED]" })
            .await;
        success
    }

    async fn work(&mut self, stop: tokio::sync::watch::Receiver<bool>) -> Result<(), Stopped> {
        {
            let mut conn = self.state.pool.acquire().await?;
            if super::is_operation_running(&mut conn).await? {
                return Err(Stopped::Failed(OperationRunning.into()));
            }
        }
        self.log.log("Marking restore as running...").await;
        let cancelled = super::keep_running(self.state, stop).await?;
        tokio::select! {
            result = self.steps() => result.map_err(Stopped::Failed),
            () = cancelled => Err(Stopped::Cancelled),
        }
    }

    async fn steps(&mut self) -> Result<(), AppError> {
        let (tmp_directory, db_dump_path) = self.decompress().await?;
        self.validate_metadata(&tmp_directory).await?;
        self.enable_readonly_mode().await?;
        self.pause_jobs().await?;
        self.wait_for_jobs().await?;
        self.flush_redis().await?;
        self.clear_job_queues().await?;
        self.restore_database(&db_dump_path).await?;
        self.reload_site_settings().await?;
        // run_seed_fu: rake db:seed is Rails'; Rails logs this when the
        // seeding raises, and goes on.
        self.log.log("Running seed fu...").await;
        tracing::warn!("db:seed after a restore is not ported");
        self.log.log("Failed to run seed fu").await;
        self.disable_readonly_mode().await;
        // Category, stats and translation caches: the port reads them from
        // the database each time.
        self.log.log("Clearing category cache...").await;
        self.log.log("Reloading translations...").await;
        self.restore_uploads(&tmp_directory).await?;
        self.log.log("Clearing emoji cache...").await;
        self.clear_theme_cache().await;
        self.log.log("Executing the after_restore_hook...").await;
        Ok(())
    }

    /// BackupFileHandler#decompress: the archive copied to
    /// `tmp/restores/<db>/<time>`, unpacked, the dump found and gunzipped
    /// (no bigger than decompressed_backup_max_file_size_mb).
    async fn decompress(&mut self) -> Result<(PathBuf, PathBuf), AppError> {
        let timestamp = crate::clock::now().format("%Y-%m-%d-%H%M%S").to_string();
        let tmp_directory = self
            .state
            .config
            .tmp_dir
            .join("restores")
            .join(&self.state.config.site.db)
            .join(timestamp);
        self.log
            .log(&format!(
                "Making sure {} exists...",
                tmp_directory.display()
            ))
            .await;
        std::fs::create_dir_all(&tmp_directory)?;
        self.tmp_directory = Some(tmp_directory.clone());
        let archive_path = tmp_directory.join(&self.filename);
        let settings = self.settings().await?;
        let store = LocalBackupStore::create(self.state, &settings)?;
        self.log.log("Copying archive to tmp directory...").await;
        let source = store.directory().join(&self.filename);
        if let Err(e) = std::fs::copy(&source, &archive_path) {
            return Err(error(format!(
                "Failed to copy archive to tmp directory.\n{e}"
            )));
        }
        let is_archive = !self.filename.ends_with(".sql.gz");
        if is_archive {
            self.log
                .log("Unzipping archive, this may take a while...")
                .await;
            let out = tokio::process::Command::new("tar")
                .arg("--extract")
                .arg("--gzip")
                .arg("--file")
                .arg(&archive_path)
                .arg("--directory")
                .arg(&tmp_directory)
                // A bug between v2.6.0.beta1 and v2.6.0.beta2 archived
                // uploads with their full path (GNU tar's option).
                .arg("--transform")
                .arg("s|var/www/discourse/public/uploads/|uploads/|")
                .kill_on_drop(true)
                .output()
                .await?;
            if !out.status.success() {
                return Err(error(format!(
                    "Failed to decompress archive.\n{}",
                    String::from_utf8_lossy(&out.stderr).trim_end()
                )));
            }
        }
        let mut db_dump_path = if is_archive {
            let old = tmp_directory.join(OLD_DUMP_FILENAME);
            if old.exists() {
                old
            } else {
                tmp_directory.join(super::creator::DUMP_FILE)
            }
        } else {
            archive_path.clone()
        };
        if db_dump_path.extension().is_some_and(|e| e == "gz") {
            self.log.log("Extracting dump file...").await;
            let max_mb = settings.get("decompressed_backup_max_file_size_mb")?.to_i();
            let target = db_dump_path.with_extension("");
            gunzip_capped(&db_dump_path, &target, max_mb.max(0) as u64 * 1024 * 1024).await?;
            db_dump_path = target;
        }
        Ok((tmp_directory, db_dump_path))
    }

    /// MetaDataHandler#validate: meta.json's version, else the file name's
    /// `-v<14 digits>`; newer than this schema is refused.
    async fn validate_metadata(&mut self, tmp_directory: &Path) -> Result<(), AppError> {
        let mut conn = self.state.pool.acquire().await?;
        let current: i64 = super::creator::current_version(&mut conn)
            .await?
            .parse()
            .unwrap_or(0);
        drop(conn);
        let meta = tmp_directory.join("meta.json");
        let version: serde_json::Value = if meta.exists() {
            let text = std::fs::read_to_string(&meta)?;
            let parsed: serde_json::Value =
                serde_json::from_str(&text).map_err(|_| error("Failed to parse metadata file."))?;
            if parsed.as_object().is_none_or(|o| o.is_empty()) {
                return Err(error("Failed to load metadata file."));
            }
            parsed["version"].clone()
        } else {
            static VERSION: std::sync::LazyLock<regex::Regex> =
                std::sync::LazyLock::new(|| regex::Regex::new(r"-v(\d{14})").unwrap());
            match VERSION.captures(&self.filename) {
                Some(c) => serde_json::json!(c[1].parse::<i64>().unwrap_or(0)),
                None => return Err(error("Migration version is missing from the filename.")),
            }
        };
        let Some(version) = version.as_i64() else {
            return Err(error("Version is not in a valid format."));
        };
        self.log.log("Validating metadata...").await;
        self.log.log(&format!("  Current version: {current}")).await;
        self.log
            .log(&format!("  Restored version: {version}"))
            .await;
        if version > current {
            return Err(error(
                "You're trying to restore a more recent version of the schema. \
                 You should migrate first!",
            ));
        }
        Ok(())
    }

    /// `Discourse.enable_readonly_mode` (READONLY_MODE_KEY, kept alive)
    /// unless the site was read only already.
    async fn enable_readonly_mode(&mut self) -> Result<(), AppError> {
        if self.readonly_mode_was_enabled {
            return Ok(());
        }
        self.log.log("Enabling readonly mode...").await;
        let mut conn = self.state.pool.acquire().await?;
        crate::readonly::enable(
            &mut conn,
            &self.state.bus,
            crate::readonly::READONLY_MODE_KEY,
        )
        .await?;
        // Discourse.keep_readonly_mode: EXPIRE every 30 s, so a key that is
        // gone stays gone.
        self.readonly_alive = Some(keep_alive(
            self.state,
            "expiring_keys",
            crate::readonly::READONLY_MODE_KEY,
            60,
        ));
        Ok(())
    }

    async fn disable_readonly_mode(&mut self) {
        if self.readonly_mode_was_enabled {
            return;
        }
        self.log.log("Disabling readonly mode...").await;
        if let Some(alive) = self.readonly_alive.take() {
            let _ = alive.send(true);
        }
        let disabled = async {
            let mut conn = self.state.pool.acquire().await?;
            crate::readonly::disable(
                &mut conn,
                &self.state.bus,
                crate::readonly::READONLY_MODE_KEY,
            )
            .await
        };
        if let Err(e) = disabled.await {
            self.log
                .log("Something went wrong while disabling readonly mode.")
                .await;
            tracing::error!("{e}");
        }
    }

    /// `Sidekiq.pause!("restore")`: the worker takes no job while the key
    /// lives (60 s, its lease extended).
    async fn pause_jobs(&mut self) -> Result<(), AppError> {
        let mut conn = self.state.pool.acquire().await?;
        if crate::owned_schema::cached_get(&mut conn, PAUSED_KEY)
            .await?
            .is_some()
        {
            return Ok(());
        }
        self.log.log("Pausing sidekiq...").await;
        crate::owned_schema::cached_setex(&mut conn, PAUSED_KEY, 60, "restore").await?;
        self.paused_alive = Some(keep_alive(self.state, "cached_values", PAUSED_KEY, 60));
        Ok(())
    }

    async fn unpause_jobs(&mut self) {
        let unpaused = async {
            let mut conn = self.state.pool.acquire().await?;
            if crate::owned_schema::cached_get(&mut conn, PAUSED_KEY)
                .await?
                .is_none()
            {
                return Ok::<_, AppError>(false);
            }
            Ok(true)
        };
        match unpaused.await {
            Ok(false) => return,
            Ok(true) => {}
            Err(e) => {
                tracing::error!("{e}");
                return;
            }
        }
        self.log.log("Unpausing sidekiq...").await;
        if let Some(alive) = self.paused_alive.take() {
            let _ = alive.send(true);
        }
        let deleted = async {
            let mut conn = self.state.pool.acquire().await?;
            crate::owned_schema::cached_del(&mut conn, PAUSED_KEY).await
        };
        if let Err(e) = deleted.await {
            self.log
                .log("Something went wrong while unpausing Sidekiq.")
                .await;
            tracing::error!("{e}");
        }
    }

    /// SystemInterface#wait_for_sidekiq: up to a minute, checked every six
    /// seconds, for the jobs being run to finish.
    async fn wait_for_jobs(&mut self) -> Result<(), AppError> {
        let max_wait_seconds = 60.0_f64;
        let wait_seconds = 6.0_f64;
        self.log
            .log(&format!(
                "Waiting up to {} seconds for Sidekiq to finish running jobs...",
                max_wait_seconds as i64
            ))
            .await;
        let max_iterations = (max_wait_seconds / wait_seconds).ceil() as i64;
        let mut iterations = 1;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs_f64(wait_seconds)).await;
            let running: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM discourse_rs.jobs \
                 WHERE failed_at IS NULL AND locked_until > clock_timestamp())",
            )
            .fetch_one(&self.state.pool)
            .await?;
            if !running {
                return Ok(());
            }
            iterations += 1;
            if iterations > max_iterations {
                return Err(error(
                    "Sidekiq did not finish running all the jobs in the allowed time!",
                ));
            }
            self.log
                .log(&format!(
                    "Waiting for sidekiq to finish running jobs... #{iterations}"
                ))
                .await;
        }
    }

    /// SystemInterface#flush_redis: every key of the site but the pause.
    /// As in Rails, the running flag and the read-only key go too (their
    /// keep-alives only extend a key that is still there).
    async fn flush_redis(&mut self) -> Result<(), AppError> {
        let mut tx = self.state.pool.begin().await?;
        sqlx::query("DELETE FROM discourse_rs.cached_values WHERE key <> $1")
            .bind(PAUSED_KEY)
            .execute(&mut *tx)
            .await?;
        for table in [
            "expiring_keys",
            "server_sessions",
            "rate_limits",
            "presence_clients",
            "badge_queue",
        ] {
            sqlx::query(&format!("DELETE FROM discourse_rs.{table}"))
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// SystemInterface#clear_sidekiq_queues: queued, scheduled, retried
    /// and dead jobs; not those running.
    async fn clear_job_queues(&mut self) -> Result<(), AppError> {
        sqlx::query(
            "DELETE FROM discourse_rs.jobs \
             WHERE locked_until IS NULL OR locked_until <= clock_timestamp() OR failed_at IS NOT NULL",
        )
        .execute(&self.state.pool)
        .await?;
        Ok(())
    }

    /// DatabaseRestorer#restore.
    async fn restore_database(&mut self, db_dump_path: &Path) -> Result<(), AppError> {
        move_tables_between_schemas(self.state, "public", "backup").await?;
        self.db_was_changed = true;
        self.create_missing_discourse_functions(db_dump_path)
            .await?;
        self.restore_dump(db_dump_path).await?;
        self.migrate_database().await?;
        self.log.log("Reconnecting to the database...").await;
        update_last_restore_date(&self.state.pool).await?;
        Ok(())
    }

    /// The `discourse_functions.raise_*_readonly()` functions the dump's
    /// triggers call and this database lacks, made for the load (Rails
    /// makes those of every migration's dropped tables and columns) and
    /// dropped again in clean_up.
    async fn create_missing_discourse_functions(&mut self, dump: &Path) -> Result<(), AppError> {
        self.log
            .log("Creating missing functions in the discourse_functions schema...")
            .await;
        static TRIGGER: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(
                r"^CREATE TRIGGER \S+ BEFORE .*? ON public\.(\w+) .*EXECUTE (?:FUNCTION|PROCEDURE) discourse_functions\.(raise_\w+_readonly)\(\);",
            )
            .unwrap()
        });
        static COLUMN: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| regex::Regex::new(r" UPDATE OF (\w+) ").unwrap());
        let mut wanted: Vec<(String, String, Option<String>)> = Vec::new();
        let file = tokio::fs::File::open(dump).await?;
        let mut lines = BufReader::new(file).split(b'\n');
        while let Some(line) = lines.next_segment().await? {
            if !line.starts_with(b"CREATE TRIGGER ") {
                continue;
            }
            let line = String::from_utf8_lossy(&line);
            if let Some(c) = TRIGGER.captures(&line) {
                let column = COLUMN.captures(&line).map(|c| c[1].to_string());
                wanted.push((c[2].to_string(), c[1].to_string(), column));
            }
        }
        let existing: Vec<String> = sqlx::query_scalar(
            "SELECT routine_name::text FROM information_schema.routines \
             WHERE routine_type = 'FUNCTION' AND specific_schema = 'discourse_functions'",
        )
        .fetch_all(&self.state.pool)
        .await?;
        for (function, table, column) in wanted {
            if existing.contains(&function) || self.created_functions.contains(&function) {
                continue;
            }
            // Migration::BaseDropper.create_readonly_function
            let message = match &column {
                Some(column) => format!("Discourse: {column} in {table} is readonly"),
                None => format!("Discourse: {table} is read only"),
            };
            sqlx::query("CREATE SCHEMA IF NOT EXISTS discourse_functions")
                .execute(&self.state.pool)
                .await?;
            sqlx::query(&format!(
                "CREATE OR REPLACE FUNCTION discourse_functions.{function}() RETURNS trigger AS $rcr$ \
                   BEGIN \
                     RAISE EXCEPTION '{}'; \
                   END \
                 $rcr$ LANGUAGE plpgsql",
                message.replace('\'', "''")
            ))
            .execute(&self.state.pool)
            .await?;
            self.created_functions.push(function);
        }
        Ok(())
    }

    /// DatabaseRestorer#restore_dump: the dump, less what this server's
    /// psql must not run (sed_command), loaded with psql in one
    /// transaction; each line psql prints is logged.
    async fn restore_dump(&mut self, dump: &Path) -> Result<(), AppError> {
        self.log
            .log("Restoring dump file... (this may take a while)")
            .await;
        let db = database_args(self.state);
        let mut command = tokio::process::Command::new("psql");
        command
            .arg(format!("--dbname={}", db.database))
            .arg("--single-transaction")
            .arg("--variable=ON_ERROR_STOP=1")
            .args(&db.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(password) = &db.password {
            command.env("PGPASSWORD", password);
        }
        let mut child = command.spawn()?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| error("psql has no stdin"))?;
        let dump = dump.to_path_buf();
        let feeder = tokio::spawn(async move {
            let nonce = crate::accounts::random_hex();
            stdin
                .write_all(format!("\\restrict {nonce}\n").as_bytes())
                .await?;
            let file = tokio::fs::File::open(&dump).await?;
            let mut lines = BufReader::new(file).split(b'\n');
            let mut filter = DumpFilter::default();
            while let Some(line) = lines.next_segment().await? {
                if filter.keep(&line) {
                    stdin.write_all(&line).await?;
                    stdin.write_all(b"\n").await?;
                }
            }
            stdin
                .write_all(format!("\\unrestrict {nonce}\n").as_bytes())
                .await?;
            stdin.shutdown().await?;
            Ok::<_, std::io::Error>(())
        });
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        for stream in [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
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
        let mut last_line = String::new();
        while let Some(line) = rx.recv().await {
            last_line = line.clone();
            let line = line.trim();
            if !line.is_empty() {
                self.log.log(line).await;
            }
        }
        let status = child.wait().await?;
        // A psql that stopped early closes its stdin: the feeder's error
        // then says nothing psql didn't.
        let fed = feeder.await.map_err(std::io::Error::other)?;
        if !status.success() {
            return Err(error(format!("psql failed: {last_line}")));
        }
        fed?;
        Ok(())
    }

    /// DatabaseRestorer#migrate_database: `rake db:migrate` is Rails'. The
    /// restored schema must already have every migration the port was
    /// built against (the check it starts with); then there is nothing to
    /// run and rake prints nothing.
    async fn migrate_database(&mut self) -> Result<(), AppError> {
        self.log.log("Migrating the database...").await;
        let mut conn = self.state.pool.acquire().await?;
        match crate::schema::verify(&mut conn).await {
            Ok(_) => {}
            Err(crate::schema::SchemaError::Missing(missing)) => {
                tracing::error!(
                    "the restored schema lacks {} migrations (first {})",
                    missing.len(),
                    missing.first().map(String::as_str).unwrap_or("-")
                );
                return Err(Unsupported(
                    "restoring a backup whose schema needs migrations (db:migrate)",
                )
                .into());
            }
            Err(crate::schema::SchemaError::Db(e)) => return Err(e.into()),
        }
        self.log.log("").await;
        Ok(())
    }

    async fn settings(&self) -> Result<SiteSettings, AppError> {
        let mut conn = self.state.pool.acquire().await?;
        Ok(SiteSettings::load(
            &mut conn,
            &self.state.site_setting_defs,
            &self.state.config.globals,
        )
        .await?)
    }

    /// The restored user with the admin's email, as User.find_by_email.
    async fn restored_user(&self) -> Result<Option<(i32, String)>, AppError> {
        Ok(sqlx::query_as(
            "SELECT u.id, u.username FROM user_emails e JOIN users u ON u.id = e.user_id \
             WHERE lower(e.email) = lower($1) ORDER BY e.primary DESC LIMIT 1",
        )
        .bind(&self.user.email)
        .fetch_optional(&self.state.pool)
        .await?)
    }

    /// Restorer#reload_site_settings: outgoing mail off for non-staff,
    /// logged as the admin (as restored) or the system user.
    async fn reload_site_settings(&mut self) -> Result<(), AppError> {
        self.log.log("Reloading site settings...").await;
        let settings = self.settings().await?;
        if self.disable_emails && settings.get("disable_emails")?.to_s() == "no" {
            self.log
                .log("Disabling outgoing emails for non-staff users...")
                .await;
            let actor = self
                .restored_user()
                .await?
                .map(|(id, _)| id)
                .unwrap_or(SYSTEM_USER_ID);
            let def = self
                .state
                .site_setting_defs
                .get("disable_emails")
                .ok_or_else(|| error("no disable_emails setting"))?;
            let previous = settings.get("disable_emails")?.clone();
            let mut tx = self.state.pool.begin().await?;
            crate::site_setting_update::set_and_log(
                &mut tx,
                &self.state.bus,
                def,
                &previous,
                &crate::site_settings::Value::Str("non-staff".into()),
                actor,
            )
            .await?;
            tx.commit().await?;
        }
        Ok(())
    }

    /// UploadsRestorer#restore.
    async fn restore_uploads(&mut self, tmp_directory: &Path) -> Result<(), AppError> {
        let mut directories: Vec<PathBuf> = match std::fs::read_dir(tmp_directory.join("uploads")) {
            Ok(entries) => entries
                .flatten()
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    !name.starts_with('.') && !name.starts_with("PaxHeaders")
                })
                .map(|e| e.path())
                .collect(),
            Err(_) => Vec::new(),
        };
        if directories.len() > 1 {
            return Err(error(
                "Could not find uploads, because the uploads directory contains multiple folders.",
            ));
        }
        let Some(tmp_uploads_path) = directories.pop() else {
            return Ok(());
        };
        let settings = self.settings().await?;
        if settings.get("enable_s3_uploads")?.truthy() || self.state.config.globals.use_s3() {
            return Err(Unsupported("restoring uploads into S3").into());
        }
        let previous_db_name = metadata_value(&self.state.pool, "db_name")
            .await?
            .unwrap_or_else(|| {
                tmp_uploads_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
        let has_optimized = tmp_uploads_path.join("optimized").exists();
        self.remap_uploads(&previous_db_name).await;
        self.log
            .log("Restoring uploads, this may take a while...")
            .await;
        let target = self
            .state
            .config
            .public_dir
            .join("uploads")
            .join(&self.state.config.site.db);
        if let Err(e) = copy_tree(&tmp_uploads_path, &target) {
            return Err(error(format!("Failed to copy uploads.\n{e}")));
        }
        if !has_optimized {
            self.log.log("Optimizing site icons...").await;
            sqlx::query("TRUNCATE TABLE optimized_images")
                .execute(&self.state.pool)
                .await?;
            self.ensure_icons(&settings).await?;
            let avatars: Vec<i32> = sqlx::query_scalar(
                "SELECT uploaded_avatar_id FROM users WHERE uploaded_avatar_id IS NOT NULL ORDER BY id",
            )
            .fetch_all(&self.state.pool)
            .await?;
            let mut conn = self.state.pool.acquire().await?;
            for upload_id in avatars {
                crate::jobs::enqueue(
                    &mut conn,
                    "create_avatar_thumbnails",
                    serde_json::json!({ "upload_id": upload_id }),
                )
                .await?;
            }
        }
        self.log
            .log(
                "Posts will be rebaked by a background job in sidekiq. You will see missing \
                 images until that has completed.",
            )
            .await;
        self.log
            .log(
                "You can expedite the process by manually running \"rake posts:rebake_uncooked_posts\"",
            )
            .await;
        sqlx::query(
            "UPDATE posts SET baked_version = NULL \
             WHERE id IN (SELECT target_id FROM upload_references WHERE target_type = 'Post')",
        )
        .execute(&self.state.pool)
        .await?;
        Ok(())
    }

    async fn ensure_icons(&self, settings: &SiteSettings) -> Result<(), AppError> {
        let store = crate::file_store::FileStore::for_site(&self.state.config, settings)?;
        let mut conn = self.state.pool.acquire().await?;
        crate::site_icons::create_optimized(&mut conn, settings, &store).await
    }

    /// UploadsRestorer#remap_uploads: the backup's base URL, CDN and
    /// database name in the restored rows made this site's. Its errors
    /// are logged, not raised.
    async fn remap_uploads(&mut self, previous_db_name: &str) {
        self.log.log("Remapping uploads...").await;
        if let Err(e) = self.remaps(previous_db_name).await {
            self.log
                .log("Something went wrong while remapping uploads.")
                .await;
            tracing::error!("{e}");
        }
    }

    async fn remaps(&mut self, previous_db_name: &str) -> Result<(), AppError> {
        let pool = &self.state.pool;
        let settings = self.settings().await?;
        let urls = crate::url::Urls {
            config: &self.state.config,
            settings: &settings,
        };
        let base_url = urls.base_url()?;
        let asset_host = urls.asset_host().map(str::to_string);
        let upload_path = format!("/uploads/{}/", self.state.config.site.db);
        if let Some(old) = metadata_value(pool, "base_url").await?
            && old != base_url
        {
            self.remap(&old, &base_url, &settings).await?;
        }
        if metadata_value(pool, "s3_base_url").await?.is_some()
            || metadata_value(pool, "s3_cdn_url").await?.is_some()
        {
            return Err(Unsupported("remapping a backup's S3 upload URLs").into());
        }
        if let Some(old_cdn) = metadata_value(pool, "cdn_url").await?
            && Some(&old_cdn) != asset_host.as_ref()
        {
            let base = asset_host.clone().unwrap_or_else(|| base_url.clone());
            self.remap(
                &format!("{old_cdn}/"),
                &schemaless(&format!("{base}/")),
                &settings,
            )
            .await?;
            let (old_host, new_host) = (url_host(&old_cdn), url_host(&base));
            if let (Some(old_host), Some(new_host)) = (old_host, new_host)
                && old_host != new_host
            {
                self.remap(&old_host, &new_host, &settings).await?;
            }
        }
        if previous_db_name != self.state.config.site.db {
            self.remap(
                &format!("/uploads/{previous_db_name}/"),
                &upload_path,
                &settings,
            )
            .await?;
        }
        Ok(())
    }

    async fn remap(
        &mut self,
        from: &str,
        to: &str,
        settings: &SiteSettings,
    ) -> Result<(), AppError> {
        self.log.log(&format!("Remapping '{from}' to '{to}'")).await;
        remap(&self.state.pool, from, to, &["backup_metadata"]).await?;
        // DbHelper.finish!: the site icons made again.
        self.ensure_icons(settings).await
    }

    /// Restorer#clear_theme_cache: `ThemeField.force_recompilation!`
    /// compiles every theme field again, which the port can't (themes are
    /// Rails'); with none there is nothing to compile.
    async fn clear_theme_cache(&mut self) {
        self.log.log("Clear theme cache").await;
        let fields: Result<i64, sqlx::Error> =
            sqlx::query_scalar("SELECT COUNT(*) FROM theme_fields")
                .fetch_one(&self.state.pool)
                .await;
        match fields {
            Ok(0) => {}
            Ok(_) => {
                tracing::warn!("recompiling theme fields after a restore is not ported");
                self.log
                    .log("Something went wrong while clearing theme cache.")
                    .await;
            }
            Err(e) => {
                tracing::error!("{e}");
                self.log
                    .log("Something went wrong while clearing theme cache.")
                    .await;
            }
        }
    }

    /// DatabaseRestorer#rollback
    async fn rollback(&mut self) {
        self.log.log("Trying to rollback...").await;
        let can = async {
            let mut conn = self.state.pool.acquire().await?;
            super::can_rollback(&mut conn).await
        };
        match can.await {
            Ok(true) if self.db_was_changed => {
                self.log.log("Rolling back...").await;
                if let Err(e) = move_tables_between_schemas(self.state, "backup", "public").await {
                    self.log.log(&format!("EXCEPTION: {e}")).await;
                }
            }
            Ok(_) => self.log.log("There was no need to rollback").await,
            Err(e) => self.log.log(&format!("EXCEPTION: {e}")).await,
        }
    }

    /// Restorer#clean_up
    async fn clean_up(&mut self) {
        self.log.log("Cleaning stuff up...").await;
        // DatabaseRestorer#clean_up
        if !self.created_functions.is_empty() {
            self.log
                .log("Dropping functions from the discourse_functions schema...")
                .await;
            for function in std::mem::take(&mut self.created_functions) {
                let dropped = sqlx::query(&format!(
                    "DROP FUNCTION IF EXISTS discourse_functions.{function}() CASCADE"
                ))
                .execute(&self.state.pool)
                .await;
                if let Err(e) = dropped {
                    self.log
                        .log(
                            "Something went wrong while dropping functions from the \
                             discourse_functions schema",
                        )
                        .await;
                    tracing::error!("{e}");
                    break;
                }
            }
        }
        // BackupFileHandler#clean_up
        if let Some(dir) = self.tmp_directory.clone() {
            self.log
                .log(&format!("Removing tmp '{}' directory...", dir.display()))
                .await;
            if dir.exists()
                && let Err(e) = std::fs::remove_dir_all(&dir)
            {
                self.log
                    .log(&format!(
                        "Something went wrong while removing the following tmp directory: {}",
                        dir.display()
                    ))
                    .await;
                tracing::error!("{e}");
            }
        }
        self.unpause_jobs().await;
        let readonly = async {
            let mut conn = self.state.pool.acquire().await?;
            crate::readonly::mode(&mut conn).await
        };
        if readonly.await.is_ok_and(|m| m.site_readonly()) {
            self.disable_readonly_mode().await;
        }
        if let Some(alive) = self.readonly_alive.take() {
            let _ = alive.send(true);
        }
        self.log.log("Marking restore as finished...").await;
        let marked = async {
            let mut conn = self.state.pool.acquire().await?;
            super::mark_as_not_running(&mut conn).await
        };
        if let Err(e) = marked.await {
            self.log
                .log("Something went wrong while marking restore as finished.")
                .await;
            tracing::error!("{e}");
        }
    }

    /// Restorer#notify_user: the admin, as the restored database knows
    /// them by email, gets the logs.
    async fn notify_user(&mut self, success: bool) {
        if success && self.user_id == SYSTEM_USER_ID {
            return;
        }
        let notified = async {
            let Some((user_id, username)) = self.restored_user().await? else {
                return Ok::<_, AppError>(false);
            };
            self.log
                .log(&format!(
                    "Notifying '{username}' of the end of the restore..."
                ))
                .await;
            let settings = self.settings().await?;
            let logs = logs_markdown(self.state, &settings, &self.log.lines, user_id).await?;
            let host = crate::pretty_text::Host::from_state(self.state);
            let ctx = crate::posting::Ctx {
                host: &host,
                settings: &settings,
                config: &self.state.config,
                i18n: &self.state.i18n,
                bus: &self.state.bus,
            };
            crate::system_message::create_from_system_user(
                &self.state.pool,
                &ctx,
                user_id,
                if success {
                    "restore_succeeded"
                } else {
                    "restore_failed"
                },
                &[("logs".to_string(), logs)],
            )
            .await?;
            Ok(true)
        };
        match notified.await {
            Ok(true) => {}
            Ok(false) => {
                let message = format!(
                    "Could not send notification to '{}' ({}), because the user does not exist.",
                    self.user.username, self.user.email
                );
                self.log.log(&message).await;
            }
            Err(e) => {
                self.log
                    .log("Something went wrong while notifying user.")
                    .await;
                tracing::error!("{e}");
            }
        }
    }
}

/// An EXPIRE every half TTL on `key` in `discourse_rs.<table>` until told
/// to stop (Discourse.keep_readonly_mode, SidekiqPauser's lease).
fn keep_alive(
    state: &AppState,
    table: &'static str,
    key: &'static str,
    ttl: i64,
) -> tokio::sync::watch::Sender<bool> {
    let (tx, mut rx) = tokio::sync::watch::channel(false);
    let pool = state.pool.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = rx.changed() => return,
                () = tokio::time::sleep(std::time::Duration::from_secs((ttl / 2) as u64)) => {}
            }
            let extended = sqlx::query(&format!(
                "UPDATE discourse_rs.{table} \
                 SET expires_at = clock_timestamp() + make_interval(secs => $2) \
                 WHERE key = $1 AND expires_at > clock_timestamp()"
            ))
            .bind(key)
            .bind(ttl as f64)
            .execute(&pool)
            .await;
            match extended {
                Ok(done) if done.rows_affected() == 0 => return,
                Ok(_) => {}
                Err(e) => tracing::error!("keeping {key} alive: {e}"),
            }
        }
    });
    tx
}

/// Compression::Gzip#decompress of the dump: written next to it without
/// `.gz`, refused past `max_bytes`.
async fn gunzip_capped(from: &Path, to: &Path, max_bytes: u64) -> Result<(), AppError> {
    let from = from.to_path_buf();
    let to = to.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        use std::io::{Read, Write};
        let input = std::fs::File::open(&from)?;
        let mut decoder = flate2::read::MultiGzDecoder::new(std::io::BufReader::new(input));
        let mut output = std::io::BufWriter::new(std::fs::File::create(&to)?);
        let mut buf = vec![0u8; 1 << 16];
        let mut written: u64 = 0;
        loop {
            let n = decoder.read(&mut buf)?;
            if n == 0 {
                break;
            }
            written += n as u64;
            if written > max_bytes {
                drop(output);
                let _ = std::fs::remove_file(&to);
                return Err(ExtractFailed.into());
            }
            output.write_all(&buf[..n])?;
        }
        output.flush()?;
        Ok(())
    })
    .await
    .map_err(std::io::Error::other)?
}

/// DatabaseRestorer#sed_command, line by line: statements this server
/// must not run, the discourse_functions function bodies, and foreign
/// servers, mappings and tables.
#[derive(Default)]
struct DumpFilter {
    in_function: bool,
    in_foreign: bool,
}

impl DumpFilter {
    fn keep(&mut self, line: &[u8]) -> bool {
        const UNWANTED: [&[u8]; 9] = [
            b"DROP SCHEMA",
            b"CREATE SCHEMA",
            b"COMMENT ON SCHEMA",
            b"SET default_table_access_method",
            b"SET transaction_timeout",
            b"CREATE EXTENSION",
            b"COMMENT ON EXTENSION",
            b"\\restrict",
            b"\\unrestrict",
        ];
        if self.in_function {
            if line == b"$$;" {
                self.in_function = false;
            }
            return false;
        }
        if self.in_foreign {
            let trimmed = line.iter().position(|b| *b != b' ').map(|i| &line[i..]);
            if trimmed.is_some_and(|t| t.starts_with(b");")) {
                self.in_foreign = false;
            }
            return false;
        }
        if UNWANTED.iter().any(|u| line.starts_with(u)) {
            return false;
        }
        if line.starts_with(b"CREATE FUNCTION discourse_functions") {
            self.in_function = true;
            return false;
        }
        if line.starts_with(b"CREATE SERVER")
            || line.starts_with(b"CREATE USER MAPPING")
            || line.starts_with(b"CREATE FOREIGN TABLE")
        {
            self.in_foreign = true;
            return false;
        }
        true
    }
}

/// `BackupRestore.move_tables_between_schemas`: the tables, views,
/// materialized views and enums the database user owns, from one schema
/// to the other (replacing what has their name there).
pub async fn move_tables_between_schemas(
    state: &AppState,
    source: &str,
    destination: &str,
) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let owner: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&mut *tx)
        .await?;
    let owner = owner.replace('\'', "''");
    let sql = format!(
        "DO $$DECLARE row record;
BEGIN
  CREATE SCHEMA IF NOT EXISTS {destination};
  FOR row IN SELECT tablename FROM pg_tables WHERE schemaname = '{source}'  AND tableowner = '{owner}'
  LOOP
    EXECUTE 'DROP TABLE IF EXISTS {destination}.' || quote_ident(row.tablename) || ' CASCADE;';
    EXECUTE 'ALTER TABLE {source}.' || quote_ident(row.tablename) || ' SET SCHEMA {destination};';
  END LOOP;
  FOR row IN SELECT viewname FROM pg_views WHERE schemaname = '{source}' AND viewowner = '{owner}'
  LOOP
    EXECUTE 'DROP VIEW IF EXISTS {destination}.' || quote_ident(row.viewname) || ' CASCADE;';
    EXECUTE 'ALTER VIEW {source}.' || quote_ident(row.viewname) || ' SET SCHEMA {destination};';
  END LOOP;
  FOR row IN SELECT matviewname FROM pg_matviews WHERE schemaname = '{source}' AND matviewowner = '{owner}'
  LOOP
    EXECUTE 'DROP MATERIALIZED VIEW IF EXISTS {destination}.' || quote_ident(row.matviewname) || ' CASCADE;';
    EXECUTE 'ALTER MATERIALIZED VIEW {source}.' || quote_ident(row.matviewname) || ' SET SCHEMA {destination};';
  END LOOP;
  FOR row IN (
    SELECT typname FROM pg_type t
    LEFT JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace
    WHERE typcategory = 'E' AND n.nspname = '{source}' AND pg_catalog.pg_get_userbyid(typowner) = '{owner}'
  ) LOOP
    EXECUTE 'DROP TYPE IF EXISTS {destination}.' || quote_ident(row.typname) || ' CASCADE;';
    EXECUTE 'ALTER TYPE {source}.' || quote_ident(row.typname) || ' SET SCHEMA {destination};';
  END LOOP;
END$$;"
    );
    sqlx::query(&sql).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

/// `BackupMetadata.update_last_restore_date`
pub async fn update_last_restore_date(pool: &sqlx::PgPool) -> Result<(), AppError> {
    let now = crate::clock::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM backup_metadata WHERE name = $1")
        .bind(LAST_RESTORE_DATE)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO backup_metadata (name, value) VALUES ($1, $2)")
        .bind(LAST_RESTORE_DATE)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// `BackupMetadata.value_for(name)`: blank is none.
async fn metadata_value(pool: &sqlx::PgPool, name: &str) -> Result<Option<String>, AppError> {
    let value: Option<Option<String>> =
        sqlx::query_scalar("SELECT value FROM backup_metadata WHERE name = $1 LIMIT 1")
            .bind(name)
            .fetch_optional(pool)
            .await?;
    Ok(value.flatten().filter(|v| !v.trim().is_empty()))
}

/// `Jobs::DropBackupSchema` (DatabaseRestorer.drop_backup_schema): the
/// `backup` schema a restore left dropped a week after it; with no
/// restore date recorded, today's is, and the schema stays.
pub async fn drop_backup_schema(state: &AppState) -> Result<(), AppError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.schemata WHERE schema_name = 'backup')",
    )
    .fetch_one(&state.pool)
    .await?;
    if !exists {
        return Ok(());
    }
    let dropable = match metadata_value(&state.pool, LAST_RESTORE_DATE).await? {
        Some(value) => match chrono::DateTime::parse_from_rfc3339(&value) {
            Ok(at) => {
                at.with_timezone(&chrono::Utc)
                    + chrono::Duration::days(DROP_BACKUP_SCHEMA_AFTER_DAYS)
                    < crate::clock::now()
            }
            Err(_) => {
                return Err(Unsupported("a last_restore_date that is not ISO 8601").into());
            }
        },
        None => {
            update_last_restore_date(&state.pool).await?;
            false
        }
    };
    if dropable {
        sqlx::query("DROP SCHEMA backup CASCADE")
            .execute(&state.pool)
            .await?;
    }
    Ok(())
}

/// `DbHelper.remap(from, to, excluded_tables:, skip_max_length_violations:
/// true)`: every updatable text column of the public tables (but those a
/// read-only trigger guards) with `from` replaced by `to`, a value that
/// would outgrow its column left as it is.
pub async fn remap(
    pool: &sqlx::PgPool,
    from: &str,
    to: &str,
    excluded_tables: &[&str],
) -> Result<(), AppError> {
    let triggers: Vec<String> = sqlx::query_scalar(
        "SELECT trigger_name::text FROM information_schema.triggers WHERE trigger_name LIKE '%_readonly'",
    )
    .fetch_all(pool)
    .await?;
    let columns: Vec<(String, String, Option<i32>)> = sqlx::query_as(
        "SELECT table_name::text, column_name::text, character_maximum_length \
           FROM information_schema.columns \
          WHERE table_schema = 'public' \
            AND is_updatable = 'YES' \
            AND (data_type LIKE 'char%' OR data_type LIKE 'text%') \
       ORDER BY table_name, column_name",
    )
    .fetch_all(pool)
    .await?;
    // Each table with its text columns and their length limits.
    type Columns = Vec<(String, Option<i32>)>;
    let mut by_table: Vec<(String, Columns)> = Vec::new();
    for (table, column, max) in columns {
        if excluded_tables.contains(&table.as_str())
            || triggers.contains(&format!("{table}_{column}_readonly"))
            || triggers.contains(&format!("{table}_readonly"))
        {
            continue;
        }
        match by_table.last_mut() {
            Some((t, cols)) if *t == table => cols.push((column, max)),
            _ => by_table.push((table, vec![(column, max)])),
        }
    }
    const TRUNCATABLE: [&str; 2] = ["bookmarks.name", "topic_links.url"];
    for (table, columns) in by_table {
        let mut updates = Vec::new();
        let mut conditions = Vec::new();
        for (column, max) in &columns {
            let mut replace = format!("REPLACE(\"{column}\", $1, $2)");
            if let Some(max) = max
                && TRUNCATABLE.contains(&format!("{table}.{column}").as_str())
            {
                replace = format!("LEFT({replace}, {max})");
            }
            let basic = format!("\"{column}\" IS NOT NULL AND \"{column}\" LIKE $3");
            match max {
                Some(max) => {
                    updates.push(format!(
                        "\"{column}\" = CASE WHEN LENGTH({replace}) <= {max} THEN {replace} ELSE \"{column}\" END"
                    ));
                    conditions.push(format!("({basic} AND LENGTH({replace}) <= {max})"));
                }
                None => {
                    updates.push(format!("\"{column}\" = {replace}"));
                    conditions.push(format!("({basic})"));
                }
            }
        }
        sqlx::query(&format!(
            "UPDATE \"{table}\" SET {} WHERE {}",
            updates.join(", "),
            conditions.join(" OR ")
        ))
        .bind(from)
        .bind(to)
        .bind(format!("%{from}%"))
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// `UrlHelper.schemaless`
fn schemaless(url: &str) -> String {
    url.strip_prefix("http:")
        .or_else(|| url.strip_prefix("https:"))
        .unwrap_or(url)
        .to_string()
}

/// `URI.parse(url).host`
fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("//")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host.split(':').next()?;
    (!host.is_empty()).then(|| host.to_string())
}

/// FileStore::LocalStore#copy_from (`rsync -a --safe-links`): the
/// backup's uploads merged into the site's; links are not followed.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filtered(dump: &str) -> String {
        let mut filter = DumpFilter::default();
        dump.lines()
            .filter(|l| filter.keep(l.as_bytes()))
            .map(|l| format!("{l}\n"))
            .collect()
    }

    #[test]
    fn the_dump_is_filtered_as_sed_filters_it() {
        let dump = "\\restrict abc\nSET statement_timeout = 0;\nSET transaction_timeout = 0;\n\
CREATE SCHEMA public;\nCOMMENT ON SCHEMA public IS 'x';\nCREATE EXTENSION IF NOT EXISTS hstore;\n\
CREATE FUNCTION discourse_functions.raise_x_readonly() RETURNS trigger\n    LANGUAGE plpgsql\n    AS $$\n\
  BEGIN\n    RAISE EXCEPTION 'x';\n  END\n$$;\nCREATE TABLE public.posts (\n    id integer\n);\n\
CREATE FOREIGN TABLE public.f (\n    id integer\n);\nCREATE TRIGGER t BEFORE INSERT ON public.posts;\n\\unrestrict abc\n";
        assert_eq!(
            filtered(dump),
            "SET statement_timeout = 0;\nCREATE TABLE public.posts (\n    id integer\n);\n\
CREATE TRIGGER t BEFORE INSERT ON public.posts;\n"
        );
    }

    #[test]
    fn urls_lose_their_scheme_and_give_their_host() {
        assert_eq!(schemaless("https://cdn.example.com/"), "//cdn.example.com/");
        assert_eq!(
            url_host("http://localhost:3042/x").as_deref(),
            Some("localhost")
        );
        assert_eq!(url_host("/relative"), None);
    }
}
