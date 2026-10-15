//! UsersController#pick_avatar, #select_avatar and #destroy_user_image
//! (app/controllers/users_controller.rb): the avatar selector's save, a
//! selectable avatar picked, and a profile or card background removed.

use axum::Json;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::session::{bad_csrf, csrf_ok};
use crate::guardian::Guardian;
use crate::params;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::user_updater::{self, Target};
use crate::{AppError, AppState, Unsupported};

/// `AVATAR_TYPES_WITH_UPLOAD`
const AVATAR_TYPES_WITH_UPLOAD: [&str; 3] = ["uploaded", "custom", "gravatar"];

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn failed() -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "failed": "FAILED" })),
    )
        .into_response()
}

fn success() -> Response {
    Json(json!({ "success": "OK" })).into_response()
}

/// What every action starts with: the CSRF check, a session, the user
/// (fetch_user_from_params) and `guardian.ensure_can_edit!(user)`.
enum Start {
    Respond(Response),
    Go {
        params: Map<String, Value>,
        user: Target,
    },
}

/// The request an action was routed with.
struct Incoming<'a> {
    headers: &'a HeaderMap,
    uri: &'a Uri,
    body: &'a Bytes,
    method: &'a str,
}

async fn start(
    state: &AppState,
    conn: &mut PgConnection,
    guardian: &Guardian,
    username: &str,
    request: Incoming<'_>,
) -> Result<Start, AppError> {
    let Incoming {
        headers,
        uri,
        body,
        method,
    } = request;
    let p = params::parse(uri.query(), headers, body);
    if !csrf_ok(state, headers, &form_pairs(&p), uri.path(), method) {
        return Ok(Start::Respond(bad_csrf()));
    }
    if guardian.is_anonymous() {
        return Ok(Start::Respond(super::login_required::not_logged_in(state)));
    }
    let Some(user) = user_updater::find_target(&mut *conn, guardian, username).await? else {
        return Ok(Start::Respond(super::topics::not_found_response(state)));
    };
    if !user_updater::can_edit_user(guardian, &user) {
        return Ok(Start::Respond(super::search::invalid_access(state)));
    }
    Ok(Start::Go { params: p, user })
}

/// `user.in_any_groups?(group_ids)` for a user other than the viewer.
async fn user_in_any_groups(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
    group_ids: &[i64],
) -> Result<bool, AppError> {
    let granular = settings
        .get("granular_anonymous_and_logged_in_groups_permissions")?
        .truthy();
    if (!granular && group_ids.contains(&0)) || group_ids.contains(&5) {
        return Ok(true);
    }
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM group_users WHERE user_id = $1 AND group_id::bigint = ANY($2))",
    )
    .bind(user_id)
    .bind(group_ids)
    .fetch_one(conn)
    .await?)
}

/// `SiteSetting.use_site_small_logo_as_system_avatar = false`, when the
/// system user picks an avatar.
async fn stop_logo_as_system_avatar(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &SiteSettings,
) -> Result<(), AppError> {
    crate::site_setting_update::set(
        conn,
        &state.bus,
        settings,
        &state.site_setting_defs,
        &state.i18n,
        "use_site_small_logo_as_system_avatar",
        &json!(false),
    )
    .await?
    .map_err(|message| AppError::from(std::io::Error::other(message)))?;
    Ok(())
}

/// `user.uploaded_avatar_id = id; user.save!`: the column, then the save's
/// callbacks that read it (refresh_avatar's rebake of the user's quotes,
/// the user's upload reference and the user_updated event when it
/// changed; the primary email's autosave; the rest of after_save).
async fn save_uploaded_avatar(
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
async fn save_user_avatar(
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
async fn ensure_user_avatar(conn: &mut PgConnection, user_id: i32) -> Result<(), AppError> {
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

/// PUT /u/:username/preferences/avatar/pick
pub async fn pick(
    state: AppState,
    guardian: Guardian,
    username: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let mut tx = state.pool.begin().await?;
    let (p, user) = match start(
        &state,
        &mut tx,
        &guardian,
        &username,
        Incoming {
            headers: &headers,
            uri: &uri,
            body: &body,
            method: "PUT",
        },
    )
    .await?
    {
        Start::Respond(r) => return Ok(r),
        Start::Go { params, user } => (params, user),
    };
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    if settings.get("discourse_connect_overrides_avatar")?.truthy()
        || settings.get("auth_overrides_avatar")?.truthy()
    {
        return Ok(failed());
    }
    let kind = params::string(&p, "type").filter(|t| !crate::ruby::is_blank(t));
    if kind.as_deref() == Some("gravatar") && !settings.get("gravatar_enabled")?.truthy() {
        return Ok(failed());
    }
    if let Some(k) = kind.as_deref()
        && !AVATAR_TYPES_WITH_UPLOAD.contains(&k)
        && k != "system"
    {
        return Ok(failed());
    }
    let is_system = user.id == crate::avatar::SYSTEM_USER_ID;
    let upload_id = match kind.as_deref() {
        None | Some("system") => {
            let has_avatar: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_avatars WHERE user_id = $1)")
                    .bind(user.id)
                    .fetch_one(&mut *tx)
                    .await?;
            if !has_avatar {
                return Err(Unsupported(
                    "picking the system avatar for a user without a user_avatars row (NoMethodError in Rails)",
                )
                .into());
            }
            None
        }
        Some(k) => {
            let allowed = settings.group_ids("uploaded_avatars_allowed_groups")?;
            if !is_system && !user_in_any_groups(&mut tx, &settings, user.id, &allowed).await? {
                return Ok(failed());
            }
            let raw = params::string(&p, "upload_id").unwrap_or_default();
            let id = i32::try_from(crate::ruby::to_i(&raw)).unwrap_or(0);
            let upload: Option<Option<i32>> =
                sqlx::query_scalar("SELECT user_id FROM uploads WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await?;
            let Some(upload_owner) = upload else {
                let message = state.i18n.t("avatar.missing").unwrap_or_default();
                return Ok((
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({ "errors": [message] })),
                )
                    .into_response());
            };
            ensure_user_avatar(&mut tx, user.id).await?;
            if !can_pick_avatar(&mut tx, &guardian, user.id, id, upload_owner).await? {
                return Ok(super::search::invalid_access(&state));
            }
            Some((k == "gravatar", id))
        }
    };
    if is_system {
        stop_logo_as_system_avatar(&mut tx, &state, &settings).await?;
    }
    save_uploaded_avatar(
        &mut tx,
        &state,
        &settings,
        &user,
        upload_id.map(|(_, id)| id),
    )
    .await?;
    match upload_id {
        Some((true, id)) => save_user_avatar(&mut tx, user.id, None, Some(Some(id))).await?,
        Some((false, id)) => save_user_avatar(&mut tx, user.id, Some(Some(id)), None).await?,
        None => {}
    }
    tx.commit().await?;
    Ok(success())
}

/// `guardian.can_pick_avatar?(user_avatar, upload)`
async fn can_pick_avatar(
    conn: &mut PgConnection,
    guardian: &Guardian,
    user_id: i32,
    upload_id: i32,
    upload_owner: Option<i32>,
) -> Result<bool, AppError> {
    if guardian.is_admin() {
        return Ok(true);
    }
    let contains: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_avatars WHERE user_id = $1 \
         AND (gravatar_upload_id = $2 OR custom_upload_id = $2))",
    )
    .bind(user_id)
    .bind(upload_id)
    .fetch_one(&mut *conn)
    .await?;
    let me = guardian.user_id();
    if contains || upload_owner == Some(user_id) || (upload_owner.is_some() && upload_owner == me) {
        return Ok(true);
    }
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_uploads WHERE upload_id = $1 AND user_id = $2)",
    )
    .bind(upload_id)
    .bind(me.unwrap_or(0))
    .fetch_one(conn)
    .await?)
}

/// PUT /u/:username/preferences/avatar/select: one of selectable_avatars.
pub async fn select(
    state: AppState,
    guardian: Guardian,
    username: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let mut tx = state.pool.begin().await?;
    let (p, user) = match start(
        &state,
        &mut tx,
        &guardian,
        &username,
        Incoming {
            headers: &headers,
            uri: &uri,
            body: &body,
            method: "PUT",
        },
    )
    .await?
    {
        Start::Respond(r) => return Ok(r),
        Start::Go { params, user } => (params, user),
    };
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let Some(url) = params::string(&p, "url").filter(|u| !crate::ruby::is_blank(u)) else {
        return Ok(failed());
    };
    if settings.get("selectable_avatars_mode")?.to_s() == "disabled" {
        return Ok(failed());
    }
    let selectable: Vec<i64> = settings
        .get("selectable_avatars")?
        .to_s()
        .split('|')
        .map(crate::ruby::to_i)
        .filter(|id| *id != 0)
        .collect();
    if selectable.is_empty() {
        return Ok(failed());
    }
    let Some((upload_id, _)) = crate::upload_references::get_from_url(&mut tx, &url).await? else {
        return Ok(failed());
    };
    // SiteSetting.selectable_avatars: the uploads that still exist.
    if !selectable.contains(&i64::from(upload_id)) {
        return Ok(failed());
    }
    let is_system = user.id == crate::avatar::SYSTEM_USER_ID;
    if is_system {
        stop_logo_as_system_avatar(&mut tx, &state, &settings).await?;
    }
    save_uploaded_avatar(&mut tx, &state, &settings, &user, Some(upload_id)).await?;
    ensure_user_avatar(&mut tx, user.id).await?;
    save_user_avatar(&mut tx, user.id, Some(Some(upload_id)), None).await?;
    // user.avatar_template, read after the setting changed.
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let template = crate::avatar::template_for(&mut tx, &urls, user.id)
        .await?
        .unwrap_or_default();
    tx.commit().await?;
    Ok(Json(json!({
        "avatar_template": template,
        "custom_avatar_template": template,
        "uploaded_avatar_id": upload_id,
    }))
    .into_response())
}

/// DELETE /u/:username/preferences/user_image: a profile or card
/// background removed (UserProfile#clear_profile_background and
/// #clear_card_background).
pub async fn destroy_user_image(
    state: AppState,
    guardian: Guardian,
    username: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let mut tx = state.pool.begin().await?;
    let (p, user) = match start(
        &state,
        &mut tx,
        &guardian,
        &username,
        Incoming {
            headers: &headers,
            uri: &uri,
            body: &body,
            method: "DELETE",
        },
    )
    .await?
    {
        Start::Respond(r) => return Ok(r),
        Start::Go { params, user } => (params, user),
    };
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let Some(kind) = params::string(&p, "type").filter(|t| !crate::ruby::is_blank(t)) else {
        return Ok(super::accounts::param_missing("type"));
    };
    let column = match kind.as_str() {
        "profile_background" => "profile_background_upload_id",
        "card_background" => "card_background_upload_id",
        _ => return Ok(super::search::invalid_parameters(&state, "type")),
    };
    let (profile, card): (Option<i32>, Option<i32>) = sqlx::query_as(&format!(
        "UPDATE user_profiles SET {column} = NULL WHERE user_id = $1 \
         RETURNING profile_background_upload_id, card_background_upload_id"
    ))
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    // after_save :trigger_badges, then the backgrounds' references.
    crate::badge_granter::queue_user_change(&mut tx, &settings, user.id).await?;
    let ids: Vec<i64> = [profile, card]
        .into_iter()
        .flatten()
        .map(i64::from)
        .collect();
    crate::upload_references::ensure_exist_many(&mut tx, "UserProfile", user.id.into(), &ids)
        .await?;
    tx.commit().await?;
    Ok(success())
}
