//! Port of app/models/user_avatar.rb and the user saves that change an
//! avatar: `user.uploaded_avatar_id = ...; user.save!`, the UserAvatar
//! row's own save, and UserAvatar#update_gravatar! with its job.

use serde_json::json;
use sqlx::PgConnection;

use crate::final_destination::{Fetcher, Options, Read};
use crate::site_settings::SiteSettings;
use crate::uploads::{NewUpload, Outcome};
use crate::url::Urls;
use crate::user_updater::{self, Target};
use crate::{AppError, AppState, Unsupported};

/// `UserAvatar.@@custom_user_gravatar_email_hash`: the system user's
/// gravatar is Discourse's.
const SYSTEM_GRAVATAR_EMAIL: &str = "info@discourse.org";

/// `User.email_hash(email)`
pub fn email_hash(email: &str) -> String {
    format!("{:x}", md5::compute(email.trim().to_lowercase().as_bytes()))
}

/// The user as the saves read them, by id.
pub async fn load_target(
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<Option<Target>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, username, username_lower, name, locale, admin, moderator, staged, active, \
                trust_level, title, primary_group_id, flair_group_id, date_of_birth \
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(conn)
    .await
}

/// What `FileHelper.download` gave.
enum Downloaded {
    File {
        bytes: Vec<u8>,
        extension: String,
    },
    /// Nothing to read, or a file over max_file_size.
    Nothing,
    /// The response's error status (OpenURI::HTTPError).
    HttpError(u16),
}

/// `FileHelper.download(url, max_file_size:, follow_redirect: true)`
/// through FinalDestination (read_timeout 5), the file's extension from
/// its content type, else from the url's path.
async fn download(
    conn: &mut PgConnection,
    fetcher: &Fetcher<'_>,
    url: &str,
    max_file_size: usize,
) -> Result<Downloaded, AppError> {
    let opts = Options {
        timeout: std::time::Duration::from_secs(5),
        max_redirects: 5,
        https_redirect_ignore_limit: false,
        stop_at_blocked_pages: false,
        headers: Vec::new(),
    };
    match fetcher
        .get(conn, url, &opts, |_| max_file_size, |_| false)
        .await?
    {
        Read::Page {
            body,
            content_type,
            uri,
        } => {
            if body.len() > max_file_size {
                return Ok(Downloaded::Nothing);
            }
            // MiniMime.lookup_by_content_type(...).extension
            let from_type = content_type.as_deref().and_then(|t| {
                match t
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_lowercase()
                    .as_str()
                {
                    "image/png" => Some(".png"),
                    "image/jpeg" => Some(".jpg"),
                    "image/gif" => Some(".gif"),
                    "image/webp" => Some(".webp"),
                    _ => None,
                }
            });
            let extension = match from_type {
                Some(e) => e.to_string(),
                None => std::path::Path::new(uri.path())
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default(),
            };
            Ok(Downloaded::File {
                bytes: body,
                extension,
            })
        }
        Read::Failed { status } if status >= 400 => Ok(Downloaded::HttpError(status)),
        Read::Failed { .. } | Read::Nothing => Ok(Downloaded::Nothing),
    }
}

/// `UserAvatar#update_gravatar!(selection:)`: the attempt recorded, the
/// gravatar fetched at the largest avatar size and made an avatar upload,
/// and the user's avatar moved to it when it showed the old gravatar (or,
/// `if_missing`, when the user has none), unless a connected account's
/// avatar is selected. A 404 (no gravatar for the address) is no error;
/// any other error status is.
pub async fn update_gravatar(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    avatar_id: i32,
    if_missing: bool,
) -> Result<(), AppError> {
    let user_id: i32 = sqlx::query_scalar(
        "UPDATE user_avatars SET last_gravatar_download_attempt = clock_timestamp(), \
                updated_at = clock_timestamp() WHERE id = $1 \
         RETURNING user_id",
    )
    .bind(avatar_id)
    .fetch_one(&mut *conn)
    .await?;
    let Some(user) = load_target(&mut *conn, user_id).await? else {
        return Ok(());
    };
    let email: Option<String> =
        sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(email) = email.filter(|e| !crate::ruby::is_blank(e)) else {
        return Ok(());
    };
    let max = crate::avatar::avatar_sizes(settings)?
        .into_iter()
        .max()
        .unwrap_or(0);
    let hash = if user_id == crate::avatar::SYSTEM_USER_ID {
        email_hash(SYSTEM_GRAVATAR_EMAIL)
    } else {
        email_hash(&email)
    };
    // SecureRandom.urlsafe_base64(5)
    let reset_cache = {
        use base64::Engine;
        let bytes: [u8; 5] = rand::random();
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    };
    let url = format!(
        "https://{}/avatar/{hash}.png?s={max}&d=404&reset_cache={reset_cache}",
        settings.get("gravatar_base_url")?.to_s()
    );
    let fetcher = Fetcher {
        settings,
        urls: Urls {
            config: &state.config,
            settings,
        },
    };
    let max_file_size = (settings.get("max_image_size_kb")?.to_i().max(0) * 1024) as usize;
    let downloaded = match download(&mut *conn, &fetcher, &url, max_file_size).await? {
        Downloaded::File { bytes, extension } => Some((bytes, extension)),
        Downloaded::Nothing | Downloaded::HttpError(404) => None,
        Downloaded::HttpError(status) => {
            return Err(std::io::Error::other(format!("gravatar download: {status} Error")).into());
        }
    };
    let gravatar = match downloaded {
        Some((bytes, extension)) => {
            Some(gravatar_upload(state, conn, settings, &user, &url, &bytes, &extension).await?)
        }
        None => None,
    };
    save_gravatar(state, conn, settings, &user, gravatar, if_missing).await
}

/// `download_gravatar!` past the download: the avatar upload made of it
/// (`for_gravatar`, its url as the origin). None when it is refused.
pub async fn gravatar_upload(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user: &Target,
    url: &str,
    bytes: &[u8],
    extension: &str,
) -> Result<Option<i32>, AppError> {
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let extension = if extension.is_empty() {
        ".png"
    } else {
        extension
    };
    let filename = format!("gravatar{extension}");
    let store = crate::file_store::FileStore::for_site(&state.config, settings)?;
    let outcome = crate::uploads::create(
        &mut *conn,
        settings,
        &state.i18n,
        &urls,
        &store,
        &NewUpload {
            user_id: user.id,
            staff: user.admin || user.moderator,
            upload_type: "avatar",
            filename: &filename,
            bytes,
            pasted: false,
            for_private_message: false,
            for_export: false,
            for_theme: false,
            skip_validations: false,
            origin: Some(url),
            for_gravatar: true,
        },
    )
    .await?;
    Ok(match outcome {
        Outcome::Created(upload) => upload["id"].as_i64().map(|id| id as i32),
        Outcome::Invalid(_) => None,
    })
}

/// The rest of `update_gravatar!` once the gravatar is downloaded
/// (`Some(upload)`, None for a refused one): under the user's lock, the
/// gravatar column moved to a new upload, and the user's avatar with it
/// when it showed the old gravatar or (`if_missing`) none, unless a
/// connected account's avatar is selected.
pub async fn save_gravatar(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user: &Target,
    downloaded: Option<Option<i32>>,
    if_missing: bool,
) -> Result<(), AppError> {
    let user_id = user.id;
    let uploaded_avatar_id: Option<i32> =
        sqlx::query_scalar("SELECT uploaded_avatar_id FROM users WHERE id = $1 FOR UPDATE")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    let (old_gravatar, selected): (Option<i32>, Option<i64>) = sqlx::query_as(
        "SELECT gravatar_upload_id, selected_user_associated_account_id \
         FROM user_avatars WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    let selected_gravatar = old_gravatar.is_some() && uploaded_avatar_id == old_gravatar;
    let select_if_missing = if_missing && uploaded_avatar_id.is_none();
    let mut gravatar = old_gravatar;
    match downloaded {
        Some(Some(upload_id)) if old_gravatar != Some(upload_id) => {
            save_user_avatar(&mut *conn, user_id, None, Some(Some(upload_id))).await?;
            gravatar = Some(upload_id);
        }
        Some(None) => {
            return Err(Unsupported(
                "a gravatar the upload validation refuses (Rails saves it with the avatar)",
            )
            .into());
        }
        _ => {}
    }
    let Some(gravatar) = gravatar else {
        return Ok(());
    };
    if selected.is_some()
        || !(selected_gravatar || select_if_missing)
        || uploaded_avatar_id == Some(gravatar)
    {
        return Ok(());
    }
    save_uploaded_avatar(&mut *conn, state, settings, user, Some(gravatar)).await
}

/// `Jobs::UpdateGravatar`: for a user whose avatar row it is and who has
/// an email, the gravatar updated and made the avatar of a user without
/// one.
pub async fn update_gravatar_job(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
    avatar_id: i32,
) -> Result<(), AppError> {
    let ok: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_avatars a JOIN users u ON u.id = a.user_id \
         JOIN user_emails e ON e.user_id = u.id AND e.\"primary\" AND e.email <> '' \
         WHERE a.id = $1 AND u.id = $2)",
    )
    .bind(avatar_id)
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if !ok {
        return Ok(());
    }
    update_gravatar(state, conn, settings, avatar_id, true).await
}

/// `user.uploaded_avatar_id = id; user.save!`: the column, then the save's
/// callbacks that read it (refresh_avatar's rebake of the user's quotes,
/// the user's upload reference and the user_updated event when it
/// changed; the primary email's autosave; the rest of after_save).
pub(crate) async fn save_uploaded_avatar(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    user: &Target,
    upload_id: Option<i32>,
) -> Result<(), AppError> {
    let before: Option<i32> =
        sqlx::query_scalar("SELECT uploaded_avatar_id FROM users WHERE id = $1")
            .bind(user.id)
            .fetch_one(&mut *conn)
            .await?;
    let changed = before != upload_id;
    if changed {
        sqlx::query(
            "UPDATE users SET uploaded_avatar_id = $2, updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(user.id)
        .bind(upload_id)
        .execute(&mut *conn)
        .await?;
    }
    user_updater::autosave_primary_email(&mut *conn, user.id).await?;
    user_updater::after_save(
        &mut *conn,
        settings,
        user.id,
        user.admin && user.active,
        &user.username_lower,
        user.name.as_deref(),
    )
    .await?;
    if !changed {
        return Ok(());
    }
    crate::jobs::enqueue(
        &mut *conn,
        "rebake_quoted_posts_for_user",
        json!({ "user_id": user.id }),
    )
    .await?;
    crate::upload_references::ensure_exist(
        &mut *conn,
        "User",
        user.id.into(),
        upload_id.map(i64::from),
    )
    .await?;
    // after_update :trigger_user_updated_event, if human?
    if user.id > 0 {
        crate::web_hooks::enqueue_user_hooks(
            &mut *conn,
            &crate::web_hooks::Payloads {
                settings,
                i18n: &state.i18n,
                config: &state.config,
            },
            "user_updated",
            user.id,
        )
        .await?;
    }
    Ok(())
}

/// `user_avatar.save!` with its upload columns set: saved (and its upload
/// references made again) only when one changed.
pub(crate) async fn save_user_avatar(
    conn: &mut PgConnection,
    user_id: i32,
    custom: Option<Option<i32>>,
    gravatar: Option<Option<i32>>,
) -> Result<(), AppError> {
    let (id, old_custom, old_gravatar): (i32, Option<i32>, Option<i32>) = sqlx::query_as(
        "SELECT id, custom_upload_id, gravatar_upload_id FROM user_avatars WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    let custom = custom.unwrap_or(old_custom);
    let gravatar = gravatar.unwrap_or(old_gravatar);
    if custom == old_custom && gravatar == old_gravatar {
        return Ok(());
    }
    sqlx::query(
        "UPDATE user_avatars SET custom_upload_id = $2, gravatar_upload_id = $3, \
                updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(id)
    .bind(custom)
    .bind(gravatar)
    .execute(&mut *conn)
    .await?;
    let ids: Vec<i64> = [custom, gravatar]
        .into_iter()
        .flatten()
        .map(i64::from)
        .collect();
    crate::upload_references::ensure_exist_many(&mut *conn, "UserAvatar", id.into(), &ids).await?;
    Ok(())
}

/// `user_avatar.update!(selected_user_associated_account_id: id)`: saved
/// only when it changes. The row must exist.
async fn select_associated_account(
    conn: &mut PgConnection,
    user_id: i32,
    account_id: Option<i64>,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE user_avatars SET selected_user_associated_account_id = $2, \
                updated_at = clock_timestamp() \
         WHERE user_id = $1 AND selected_user_associated_account_id IS DISTINCT FROM $2",
    )
    .bind(user_id)
    .bind(account_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// `UserAvatar.clear_associated_account_selection(account_ids,
/// except_user_id:)`: avatars showing one of the accounts stop (an
/// update_all, no timestamps).
pub async fn clear_associated_account_selection(
    conn: &mut PgConnection,
    account_ids: &[i64],
    except_user_id: Option<i32>,
) -> Result<(), AppError> {
    if account_ids.is_empty() {
        return Ok(());
    }
    // where.not(user_id: nil) is `user_id IS NOT NULL`, every row.
    sqlx::query(
        "UPDATE user_avatars SET selected_user_associated_account_id = NULL \
         WHERE selected_user_associated_account_id = ANY($1) \
           AND ($2::int IS NULL OR user_id <> $2)",
    )
    .bind(account_ids)
    .bind(except_user_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// `UserAvatar.remove_upload(upload_id)` (Upload's after_destroy): the
/// upload stops being anyone's avatar, gravatar, custom avatar or kept
/// connected account avatar, and those accounts stop being selected.
pub async fn remove_upload(conn: &mut PgConnection, upload_id: i32) -> Result<(), AppError> {
    for (table, column) in [
        ("users", "uploaded_avatar_id"),
        ("user_avatars", "gravatar_upload_id"),
        ("user_avatars", "custom_upload_id"),
    ] {
        sqlx::query(&format!(
            "UPDATE {table} SET {column} = NULL WHERE {column} = $1"
        ))
        .bind(upload_id)
        .execute(&mut *conn)
        .await?;
    }
    let accounts: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM user_associated_accounts WHERE avatar_upload_id = $1")
            .bind(upload_id)
            .fetch_all(&mut *conn)
            .await?;
    clear_associated_account_selection(&mut *conn, &accounts, None).await?;
    sqlx::query(
        "UPDATE user_associated_accounts SET avatar_upload_id = NULL WHERE avatar_upload_id = $1",
    )
    .bind(upload_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// UserAssociatedAccount's after_save when its user changed.
pub async fn associated_account_user_changed(
    conn: &mut PgConnection,
    account_id: i64,
    user_id: Option<i32>,
) -> Result<(), AppError> {
    clear_associated_account_selection(conn, &[account_id], user_id).await
}

/// What `destroy` adds to deleting user_associated_accounts rows: their
/// upload references (dependent: :destroy) and the after_destroy's
/// cleared selection. `ids` are the rows deleted.
pub async fn associated_accounts_destroyed(
    conn: &mut PgConnection,
    ids: &[i64],
) -> Result<(), AppError> {
    if ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "DELETE FROM upload_references WHERE target_type = 'UserAssociatedAccount' \
         AND target_id = ANY($1)",
    )
    .bind(ids)
    .execute(&mut *conn)
    .await?;
    clear_associated_account_selection(conn, ids, None).await
}

/// `UserAvatar.retrieve_for_associated_account(account, selection:)`: the
/// user's avatar row made, the account selected as the avatar when
/// auth_overrides_avatar (or, on `initial`, for a user with no avatar
/// yet), and its image downloaded in the background into the account.
pub async fn retrieve_for_associated_account(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    account_id: i64,
    initial: bool,
) -> Result<(), AppError> {
    let row: Option<(Option<i32>, Option<serde_json::Value>)> = sqlx::query_as(
        "SELECT u.id, a.info -> 'image' FROM user_associated_accounts a \
         LEFT JOIN users u ON u.id = a.user_id WHERE a.id = $1",
    )
    .bind(account_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((Some(user_id), image)) = row else {
        return Ok(());
    };
    let url = match image {
        None | Some(serde_json::Value::Null) => return Ok(()),
        Some(serde_json::Value::String(url)) if crate::ruby::is_blank(&url) => return Ok(()),
        Some(serde_json::Value::String(url)) => url,
        Some(_) => {
            return Err(Unsupported("a provider's image that is not a string").into());
        }
    };
    sqlx::query("SELECT 1 FROM users WHERE id = $1 FOR UPDATE")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    ensure_user_avatar(&mut *conn, user_id).await?;
    let select = if settings.get("auth_overrides_avatar")?.truthy() {
        true
    } else if initial {
        sqlx::query_scalar(
            "SELECT u.uploaded_avatar_id IS NULL AND a.custom_upload_id IS NULL \
             FROM users u JOIN user_avatars a ON a.user_id = u.id WHERE u.id = $1",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?
    } else {
        false
    };
    if select {
        select_associated_account(&mut *conn, user_id, Some(account_id)).await?;
    }
    crate::jobs::enqueue(
        &mut *conn,
        "download_avatar_from_url",
        json!({"url": url, "user_id": user_id, "associated_account_id": account_id}),
    )
    .await?;
    Ok(())
}

/// `UserAvatar.download_for_user(url, user)`: the image fetched
/// (FileHelper.download) and made an avatar upload named for its type
/// (FastImage.type). None when it could not be read or was refused.
async fn download_for_user(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user: &Target,
    url: &str,
) -> Result<Option<i32>, AppError> {
    let fetcher = Fetcher {
        settings,
        urls: Urls {
            config: &state.config,
            settings,
        },
    };
    let max_file_size = (settings.get("max_image_size_kb")?.to_i().max(0) * 1024) as usize;
    // Net::ReadTimeout, OpenURI::HTTPError and SSRFError are skipped.
    let Downloaded::File { bytes, .. } = download(&mut *conn, &fetcher, url, max_file_size).await?
    else {
        return Ok(None);
    };
    let ext = crate::images::info(&bytes)
        .map(|i| i.format.name())
        .unwrap_or("");
    let filename = format!("external-avatar.{ext}");
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let store = crate::file_store::FileStore::for_site(&state.config, settings)?;
    let outcome = crate::uploads::create(
        &mut *conn,
        settings,
        &state.i18n,
        &urls,
        &store,
        &NewUpload {
            user_id: user.id,
            staff: user.admin || user.moderator,
            upload_type: "avatar",
            filename: &filename,
            bytes: &bytes,
            pasted: false,
            for_private_message: false,
            for_export: false,
            for_theme: false,
            skip_validations: false,
            origin: Some(url),
            for_gravatar: false,
        },
    )
    .await?;
    Ok(match outcome {
        Outcome::Created(upload) => upload["id"].as_i64().map(|id| id as i32),
        Outcome::Invalid(_) => None,
    })
}

/// `Jobs::DownloadAvatarFromUrl`: UserAvatar.import_url_for_user for a
/// user that is still there.
pub async fn download_avatar_from_url_job(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    args: &serde_json::Value,
) -> Result<(), AppError> {
    let present = |v: &serde_json::Value| match v {
        serde_json::Value::Null => false,
        serde_json::Value::String(s) => !crate::ruby::is_blank(s),
        _ => true,
    };
    if !present(&args["url"]) {
        return Err(std::io::Error::other("Discourse::InvalidParameters: url").into());
    }
    if !present(&args["user_id"]) {
        return Err(std::io::Error::other("Discourse::InvalidParameters: user_id").into());
    }
    let Some(url) = args["url"].as_str() else {
        return Err(Unsupported("download_avatar_from_url with a url that is not a string").into());
    };
    let user_id = match &args["user_id"] {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => Some(crate::ruby::to_i(s)),
        _ => None,
    };
    let Some(user) = (match user_id.and_then(|id| i32::try_from(id).ok()) {
        Some(id) => load_target(&mut *conn, id).await?,
        None => None,
    }) else {
        return Ok(());
    };
    let account_id = match &args["associated_account_id"] {
        serde_json::Value::Null => None,
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => Some(crate::ruby::to_i(s)),
        _ => return Err(Unsupported("an associated_account_id that is not an id").into()),
    };
    if let Some(account_id) = account_id {
        return import_associated_account_avatar(state, conn, settings, &user, url, account_id)
            .await;
    }
    let override_gravatar = !matches!(
        &args["override_gravatar"],
        serde_json::Value::Null | serde_json::Value::Bool(false)
    );
    import_url_for_user(state, conn, settings, &user, url, override_gravatar).await
}

/// `UserAvatar.import_url_for_user(url, user, override_gravatar:)`: the
/// image kept as the custom avatar, and shown unless the user shows a
/// gravatar or upload of their own (or override_gravatar says to anyway).
async fn import_url_for_user(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user: &Target,
    url: &str,
    override_gravatar: bool,
) -> Result<(), AppError> {
    let Some(upload_id) = download_for_user(state, conn, settings, user, url).await? else {
        return Ok(());
    };
    let uploaded_avatar_id: Option<i32> =
        sqlx::query_scalar("SELECT uploaded_avatar_id FROM users WHERE id = $1 FOR UPDATE")
            .bind(user.id)
            .fetch_one(&mut *conn)
            .await?;
    let avatar: Option<(Option<i32>, Option<i32>)> = sqlx::query_as(
        "SELECT custom_upload_id, gravatar_upload_id FROM user_avatars WHERE user_id = $1",
    )
    .bind(user.id)
    .fetch_optional(&mut *conn)
    .await?;
    let (old_custom, gravatar) = avatar.unwrap_or((None, None));
    let cached = old_custom == Some(upload_id) || gravatar == Some(upload_id);
    if cached && !override_gravatar {
        return Ok(());
    }
    let custom = if cached { old_custom } else { Some(upload_id) };
    let contains = |id: Option<i32>| id.is_some() && (id == custom || id == gravatar);
    let replace_current =
        uploaded_avatar_id.is_none() || !contains(uploaded_avatar_id) || override_gravatar;
    // avatar.save!: one save of the custom upload and the selection.
    ensure_user_avatar(&mut *conn, user.id).await?;
    save_user_avatar(&mut *conn, user.id, Some(custom), None).await?;
    if replace_current {
        select_associated_account(&mut *conn, user.id, None).await?;
        save_uploaded_avatar(&mut *conn, state, settings, user, Some(upload_id)).await?;
    }
    Ok(())
}

/// The account's image url, when the account is still the user's.
async fn associated_account_image(
    conn: &mut PgConnection,
    user_id: i32,
    account_id: i64,
    lock: bool,
) -> Result<Option<(String, Option<String>)>, AppError> {
    let sql = if lock {
        "SELECT provider_name, info ->> 'image' FROM user_associated_accounts \
         WHERE user_id = $1 AND id = $2 FOR UPDATE"
    } else {
        "SELECT provider_name, info ->> 'image' FROM user_associated_accounts \
         WHERE user_id = $1 AND id = $2"
    };
    Ok(sqlx::query_as(sql)
        .bind(user_id)
        .bind(account_id)
        .fetch_optional(conn)
        .await?)
}

/// `UserAvatar.import_associated_account_avatar(url, user, account_id)`:
/// the image kept on the account whose image it still is, and shown when
/// the user's avatar is that account's (of an enabled authenticator).
async fn import_associated_account_avatar(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user: &Target,
    url: &str,
    account_id: i64,
) -> Result<(), AppError> {
    let current = associated_account_image(&mut *conn, user.id, account_id, false).await?;
    if !current.is_some_and(|(_, image)| image.as_deref() == Some(url)) {
        return Ok(());
    }
    let Some(upload_id) = download_for_user(state, conn, settings, user, url).await? else {
        return Ok(());
    };
    sqlx::query("SELECT 1 FROM users WHERE id = $1 FOR UPDATE")
        .bind(user.id)
        .execute(&mut *conn)
        .await?;
    let Some((provider, image)) =
        associated_account_image(&mut *conn, user.id, account_id, true).await?
    else {
        return Ok(());
    };
    if image.as_deref() != Some(url) {
        return Ok(());
    }
    // account.update!(avatar_upload: upload), its after_save's reference.
    let changed = sqlx::query(
        "UPDATE user_associated_accounts SET avatar_upload_id = $2, updated_at = clock_timestamp() \
         WHERE id = $1 AND avatar_upload_id IS DISTINCT FROM $2",
    )
    .bind(account_id)
    .bind(upload_id)
    .execute(&mut *conn)
    .await?
    .rows_affected()
        > 0;
    if changed {
        crate::upload_references::ensure_exist(
            &mut *conn,
            "UserAssociatedAccount",
            account_id,
            Some(i64::from(upload_id)),
        )
        .await?;
    }
    let selected: Option<Option<i64>> = sqlx::query_scalar(
        "SELECT selected_user_associated_account_id FROM user_avatars WHERE user_id = $1",
    )
    .bind(user.id)
    .fetch_optional(&mut *conn)
    .await?;
    if selected.flatten() == Some(account_id) && authenticator_enabled(settings, &provider)? {
        save_uploaded_avatar(&mut *conn, state, settings, user, Some(upload_id)).await?;
    }
    Ok(())
}

/// `account.authenticator`: an enabled authenticator by the account's
/// provider name.
fn authenticator_enabled(settings: &SiteSettings, provider: &str) -> Result<bool, AppError> {
    Ok(crate::auth::enabled(settings)?
        .iter()
        .any(|p| p.name == provider))
}

/// `UserAvatar#pick_associated_account(account_id)`: the user's account
/// of an enabled authenticator, with its avatar kept, made the avatar.
/// False when there is none.
pub async fn pick_associated_account(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    user: &Target,
    account_id: i64,
) -> Result<bool, AppError> {
    sqlx::query("SELECT 1 FROM users WHERE id = $1 FOR UPDATE")
        .bind(user.id)
        .execute(&mut *conn)
        .await?;
    let account: Option<(String, Option<i32>)> = sqlx::query_as(
        "SELECT a.provider_name, up.id FROM user_associated_accounts a \
         LEFT JOIN uploads up ON up.id = a.avatar_upload_id \
         WHERE a.user_id = $1 AND a.id = $2 FOR UPDATE OF a",
    )
    .bind(user.id)
    .bind(account_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((provider, Some(upload_id))) = account else {
        return Ok(false);
    };
    if !authenticator_enabled(settings, &provider)? {
        return Ok(false);
    }
    select_associated_account(&mut *conn, user.id, Some(account_id)).await?;
    save_uploaded_avatar(&mut *conn, state, settings, user, Some(upload_id)).await?;
    Ok(true)
}

/// `UserAvatar.pick_for_user!(user, upload_id, type:)`: the avatar row
/// (built when missing) keeps the upload as its gravatar or custom one,
/// stops showing a connected account, and the user shows the upload.
pub async fn pick_for_user(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    user: &Target,
    upload_id: Option<i32>,
    kind: &str,
) -> Result<(), AppError> {
    sqlx::query("SELECT 1 FROM users WHERE id = $1 FOR UPDATE")
        .bind(user.id)
        .execute(&mut *conn)
        .await?;
    ensure_user_avatar(&mut *conn, user.id).await?;
    match kind {
        "gravatar" => save_user_avatar(&mut *conn, user.id, None, Some(upload_id)).await?,
        "uploaded" | "custom" => {
            save_user_avatar(&mut *conn, user.id, Some(upload_id), None).await?
        }
        _ => {}
    }
    select_associated_account(&mut *conn, user.id, None).await?;
    save_uploaded_avatar(&mut *conn, state, settings, user, upload_id).await
}

/// `user.create_user_avatar unless user.user_avatar`
pub(crate) async fn ensure_user_avatar(
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO user_avatars (user_id, created_at, updated_at) \
         SELECT $1, clock_timestamp(), clock_timestamp() \
         WHERE NOT EXISTS (SELECT 1 FROM user_avatars WHERE user_id = $1)",
    )
    .bind(user_id)
    .execute(conn)
    .await?;
    Ok(())
}
