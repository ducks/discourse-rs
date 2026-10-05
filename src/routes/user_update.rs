//! Port of UsersController#update: PUT /u/:username (also with .json).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::session::current::Incoming;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::user_updater;
use crate::users::{User, Users};
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// PUT /u/{username}
pub async fn update(
    State(state): State<AppState>,
    axum::Extension(incoming): axum::Extension<Incoming>,
    Path(username): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let guardian = incoming.guardian.clone();
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    // requires_login
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state, uri.path()));
    }
    let mut tx = state.pool.begin().await?;
    let Some(target) = user_updater::find_target(&mut tx, &guardian, &username).await? else {
        return Ok(super::topics::not_found_response(&state, false));
    };
    // guardian.ensure_can_edit!(user)
    if !user_updater::can_edit_user(&guardian, &target) {
        return Ok(super::search::invalid_access(&state));
    }
    // user_params.except(:username, :email, :password)
    let mut attrs = p;
    for key in ["username", "email", "password"] {
        attrs.remove(key);
    }
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    user_updater::update(&mut tx, &ctx, &guardian, &target, &attrs).await?;

    // json_result(user, serializer: UserSerializer)
    let Some(user) = User::find_by_id(&mut tx, target.id).await? else {
        return Ok(super::topics::not_found_response(&state, false));
    };
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let auth_token: Option<String> = incoming
        .session
        .as_ref()
        .map(|s| s.token.auth_token.clone());
    let mut users = Users {
        conn: &mut tx,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        base_path: state.config.globals.relative_url_root(),
        auth_token: auth_token.as_deref(),
    };
    let doc = users.show(&user).await?;
    tx.commit().await?;
    Ok((
        StatusCode::OK,
        Json(json!({ "success": "OK", "user": doc["user"] })),
    )
        .into_response())
}
