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
                trust_level, title, primary_group_id, flair_group_id \
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

/// `UserAvatar#update_gravatar!`: the attempt recorded, the gravatar
/// fetched at the largest avatar size and made an avatar upload, and the
/// user's avatar moved to it when it showed the old gravatar. A 404 (no
/// gravatar for the address) is no error; any other error status is.
pub async fn update_gravatar(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    avatar_id: i32,
) -> Result<(), AppError> {
    let (user_id, gravatar_upload_id): (i32, Option<i32>) = sqlx::query_as(
        "UPDATE user_avatars SET last_gravatar_download_attempt = clock_timestamp(), \
                updated_at = clock_timestamp() WHERE id = $1 \
         RETURNING user_id, gravatar_upload_id",
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
    let (bytes, extension) = match download(&mut *conn, &fetcher, &url, max_file_size).await? {
        Downloaded::File { bytes, extension } => (bytes, extension),
        Downloaded::Nothing | Downloaded::HttpError(404) => return Ok(()),
        Downloaded::HttpError(status) => {
            return Err(std::io::Error::other(format!("gravatar download: {status} Error")).into());
        }
    };
    save_gravatar(
        state,
        conn,
        settings,
        &user,
        gravatar_upload_id,
        &url,
        &bytes,
        &extension,
    )
    .await
}

/// The rest of `update_gravatar!` once the gravatar is downloaded: the
/// avatar upload made of it (`for_gravatar`, its url as the origin), then
/// the user's avatar and the gravatar column moved to it when it is new.
#[allow(clippy::too_many_arguments)]
pub async fn save_gravatar(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user: &Target,
    gravatar_upload_id: Option<i32>,
    url: &str,
    bytes: &[u8],
    extension: &str,
) -> Result<(), AppError> {
    let user_id = user.id;
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
    let staff: bool = user.admin || user.moderator;
    let outcome = crate::uploads::create(
        &mut *conn,
        settings,
        &state.i18n,
        &urls,
        &store,
        &NewUpload {
            user_id,
            staff,
            upload_type: "avatar",
            filename: &filename,
            bytes,
            pasted: false,
            for_private_message: false,
            for_export: false,
            for_theme: false,
            skip_validations: false,
            gravatar_origin: Some(url),
        },
    )
    .await?;
    let upload_id = match outcome {
        Outcome::Created(upload) => upload["id"].as_i64().map(|id| id as i32),
        Outcome::Invalid(_) => None,
    };
    if gravatar_upload_id == upload_id {
        return Ok(());
    }
    let Some(upload_id) = upload_id else {
        return Err(Unsupported(
            "a gravatar the upload validation refuses, replacing another (Rails raises)",
        )
        .into());
    };
    // The user's avatar follows when it was the old gravatar.
    let uploaded_avatar_id: Option<i32> =
        sqlx::query_scalar("SELECT uploaded_avatar_id FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    if gravatar_upload_id.is_some() && uploaded_avatar_id == gravatar_upload_id {
        save_uploaded_avatar(&mut *conn, state, settings, user, Some(upload_id)).await?;
    }
    save_user_avatar(&mut *conn, user_id, None, Some(Some(upload_id))).await?;
    Ok(())
}

/// `Jobs::UpdateGravatar`: for a user whose avatar row it is and who has
/// an email, the gravatar updated, and made the avatar of a user without
/// one (`update_column`, no callbacks).
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
    update_gravatar(state, conn, settings, avatar_id).await?;
    sqlx::query(
        "UPDATE users SET uploaded_avatar_id = a.gravatar_upload_id \
         FROM user_avatars a WHERE a.id = $1 AND users.id = $2 \
         AND users.uploaded_avatar_id IS NULL AND a.gravatar_upload_id IS NOT NULL",
    )
    .bind(avatar_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
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
