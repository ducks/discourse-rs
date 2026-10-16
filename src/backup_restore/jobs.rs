//! The backups' regular jobs: the download link mail and the merge of an
//! uploaded backup's chunks.

use serde_json::{Value, json};

use super::store::{self, LocalBackupStore};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// Jobs::DownloadBackupEmail
pub const DOWNLOAD_BACKUP_EMAIL: &str = "download_backup_email";
/// Jobs::BackupChunksMerger
pub const BACKUP_CHUNKS_MERGER: &str = "backup_chunks_merger";

/// What `BackupRestore.spawn_process!` starts (script/spawn_backup_restore.rb):
/// the port queues it, and its worker runs the operation detached.
pub const SPAWN_BACKUP_RESTORE: &str = "spawn_backup_restore";
/// Jobs::CreateBackup
pub const CREATE_BACKUP: &str = "create_backup";
/// Jobs::ScheduleBackup
pub const SCHEDULE_BACKUP: &str = "schedule_backup";
/// Jobs::DropBackupSchema
pub const DROP_BACKUP_SCHEMA: &str = "drop_backup_schema";

/// `Discourse::SYSTEM_USER_ID`
const SYSTEM_USER_ID: i32 = -1;

/// `Group::AUTO_GROUPS[:staff]`
const STAFF_GROUP_ID: i32 = 3;

/// Discourse::InvalidParameters raised inside a job.
fn invalid_parameters(name: &str) -> AppError {
    std::io::Error::other(format!(
        "You supplied invalid parameters to the request: {name}"
    ))
    .into()
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

/// `Jobs::DownloadBackupEmail`: a fresh token on the backup's URL, mailed
/// to the admin (DownloadBackupMailer, logged as download_backup_message).
pub async fn download_backup_email(state: &AppState, args: &Value) -> Result<(), AppError> {
    let user_id = args
        .get("user_id")
        .and_then(|v| v.as_i64().or_else(|| v.as_str().map(crate::ruby::to_i)))
        .and_then(|v| i32::try_from(v).ok());
    let mut conn = state.pool.acquire().await?;
    let email: Option<String> = match user_id {
        Some(id) => {
            sqlx::query_scalar(
                "SELECT e.email FROM users u \
                 LEFT JOIN user_emails e ON e.user_id = u.id AND e.primary \
                 WHERE u.id = $1",
            )
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?
        }
        None => None,
    };
    let (Some(user_id), Some(email)) = (user_id, email) else {
        return Err(invalid_parameters("user_id"));
    };
    let Some(path) = str_arg(args, "backup_file_path").filter(|p| !p.trim().is_empty()) else {
        return Err(invalid_parameters("backup_file_path"));
    };
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let mut tx = state.pool.begin().await?;
    let token = super::token::set(&mut tx, user_id).await?;
    // URI#query= replaces whatever query the path had.
    let base = path.split(['?', '#']).next().unwrap_or(path);
    let fragment = path.split_once('#').map(|(_, f)| format!("#{f}"));
    let url = format!("{base}?token={token}{}", fragment.unwrap_or_default());
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::email::sender::Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        mailer: &state.mailer,
    };
    let built = crate::email::account::build_template(
        &ctx,
        "download_backup_mailer",
        &email,
        &[("backup_file_path", url.as_str())],
        None,
    )
    .await?;
    crate::email::sender::send(&mut tx, &ctx, built, "download_backup_message", None).await?;
    tx.commit().await?;
    Ok(())
}

/// `Jobs::BackupChunksMerger`: the parts joined into the backup, then the
/// new list of backups published to staff on `/admin/backups`.
pub async fn backup_chunks_merger(state: &AppState, args: &Value) -> Result<(), AppError> {
    let filename = str_arg(args, "filename").unwrap_or("");
    let identifier = str_arg(args, "identifier").unwrap_or("");
    let chunks = match args.get("chunks") {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
        Some(Value::String(s)) => crate::ruby::to_i(s),
        _ => 0,
    };
    if filename.trim().is_empty() {
        return Err(invalid_parameters("filename"));
    }
    if identifier.trim().is_empty() {
        return Err(invalid_parameters("identifier"));
    }
    if chunks <= 0 {
        return Err(invalid_parameters("chunks"));
    }
    // LocalBackupStore.base_directory, whatever backup_location says.
    let local = LocalBackupStore::at(&state.config.public_dir, &state.config.site.db)?;
    store::merge_chunks(&local, filename, identifier, chunks)?;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let files: Vec<Value> = LocalBackupStore::create(state, &settings)?
        .files()?
        .iter()
        .map(store::BackupFile::json)
        .collect();
    state
        .bus
        .publish(
            &mut conn,
            super::FILES_CHANNEL,
            &json!(files),
            Some(&[crate::bus::group_tag(STAFF_GROUP_ID)]),
        )
        .await?;
    Ok(())
}

/// The spawned backup or restore (script/spawn_backup_restore.rb): run
/// detached from the queue, as Rails detaches the process, so a long
/// operation holds no worker.
pub async fn spawn_backup_restore(state: &AppState, args: &Value) -> Result<(), AppError> {
    let user_id = args
        .get("user_id")
        .and_then(|v| v.as_i64().or_else(|| v.as_str().map(crate::ruby::to_i)))
        .and_then(|v| i32::try_from(v).ok())
        .unwrap_or(0);
    let opts = args.get("opts").cloned().unwrap_or(Value::Null);
    match str_arg(args, "type") {
        Some("backup") => {
            let options = super::creator::Options {
                publish_to_message_bus: opts["publish_to_message_bus"] == true,
                with_uploads: opts["with_uploads"].as_bool(),
                client_id: opts["client_id"].as_str().map(str::to_string),
                filename: opts["filename"].as_str().map(str::to_string),
            };
            let state = state.clone();
            tokio::spawn(async move {
                if let Err(e) = super::creator::run(&state, user_id, options).await {
                    tracing::error!("backup: {e}");
                }
            });
            Ok(())
        }
        Some("restore") => {
            let options = super::restorer::Options {
                filename: opts["filename"].as_str().map(str::to_string),
                client_id: opts["client_id"].as_str().map(str::to_string),
                disable_emails: opts["disable_emails"].as_bool().unwrap_or(true),
            };
            let state = state.clone();
            tokio::spawn(async move {
                if let Err(e) = super::restorer::run(&state, user_id, options).await {
                    tracing::error!("restore: {e}");
                }
            });
            Ok(())
        }
        other => Err(std::io::Error::other(format!(
            "Unknown argument: {}",
            other.unwrap_or_default()
        ))
        .into()),
    }
}

/// `Jobs::CreateBackup`: a backup by the system user, in the job,
/// with uploads as backup_with_uploads says.
pub async fn create_backup(state: &AppState) -> Result<(), AppError> {
    let settings = {
        let mut conn = state.pool.acquire().await?;
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?
    };
    super::creator::run(
        state,
        SYSTEM_USER_ID,
        super::creator::Options {
            publish_to_message_bus: false,
            with_uploads: Some(settings.get("backup_with_uploads")?.truthy()),
            client_id: None,
            filename: None,
        },
    )
    .await?;
    Ok(())
}

/// `Jobs::ScheduleBackup` (daily): old backups past remove_older_backups
/// deleted (not in development), then, when backups are on and the
/// newest is backup_frequency days old, create_backup queued for
/// backup_time_of_day plus up to ten random minutes.
pub async fn schedule_backup(state: &AppState) -> Result<(), AppError> {
    match schedule(state).await {
        Ok(()) => Ok(()),
        Err(e) if e.is_unsupported() => Err(e),
        Err(e) => {
            tracing::error!("schedule_backup: {e}");
            Err(crate::Unsupported(
                "schedule_backup's failure message to the admins (SystemMessage target_group_names)",
            )
            .into())
        }
    }
}

async fn schedule(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let store = LocalBackupStore::create(state, &settings)?;
    // Creator#delete_prior_to_n_days, its errors only logged.
    if state.config.rails_env != crate::config::RailsEnv::Development
        && let Err(e) = store.delete_prior_to_n_days(
            &settings.get("remove_older_backups")?.to_s(),
            crate::clock::now(),
        )
    {
        tracing::error!("Something went wrong while deleting backups prior to n days....: {e}");
    }
    if !settings.get("enable_backups")?.truthy() {
        return Ok(());
    }
    let frequency = settings.get("backup_frequency")?.to_i();
    if frequency == 0 {
        return Ok(());
    }
    if let Some(latest) = store.latest_file()? {
        let date = latest.last_modified.date();
        if date + chrono::Duration::days(frequency) > crate::clock::now().date_naive() {
            return Ok(());
        }
    }
    // Jobs.cancel_scheduled_job(:create_backup)
    sqlx::query(
        "DELETE FROM discourse_rs.jobs WHERE name = $1 AND failed_at IS NULL \
         AND (locked_until IS NULL OR locked_until < clock_timestamp())",
    )
    .bind(CREATE_BACKUP)
    .execute(&mut *conn)
    .await?;
    let time_of_day = settings.get("backup_time_of_day")?.to_s();
    let (hours, minutes) = time_of_day
        .split_once(':')
        .map(|(h, m)| (crate::ruby::to_i(h), crate::ruby::to_i(m)))
        .unwrap_or((0, 0));
    let seconds = hours * 3600 + minutes * 60 + (rand::random::<u64>() % 600) as i64;
    crate::jobs::enqueue_in(&mut conn, seconds, CREATE_BACKUP, json!({})).await?;
    Ok(())
}
