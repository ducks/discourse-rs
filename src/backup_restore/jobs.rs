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
        &mut tx,
        &ctx,
        "download_backup_mailer",
        &email,
        &[("backup_file_path", &url)],
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
    let local = LocalBackupStore::at(&state.config.public_dir)?;
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
