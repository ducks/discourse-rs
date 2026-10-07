//! Port of app/controllers/users_controller.rb#bookmarks (JSON) and
//! #user_menu_bookmarks. The `.ics` feed is not ported.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::bookmarks::{Bookmarks, ListQuery, PER_PAGE};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

#[derive(Deserialize, Default)]
pub struct Params {
    q: Option<String>,
    page: Option<String>,
    limit: Option<String>,
}

fn no_store(body: serde_json::Value) -> Response {
    (
        StatusCode::OK,
        [(header::CACHE_CONTROL, "no-cache, no-store")],
        Json(body),
    )
        .into_response()
}

/// GET /u/:username/bookmarks(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(username): Path<String>,
    Query(params): Query<Params>,
) -> Result<Response, AppError> {
    let Some(viewer) = guardian.user() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;

    // fetch_user_from_params: oneself, else an active user (staff also
    // find inactive ones).
    let username_lower = username.to_lowercase();
    let user_id: Option<i32> = if viewer.username.to_lowercase() == username_lower {
        Some(viewer.id)
    } else {
        sqlx::query_scalar("SELECT id FROM users WHERE username_lower = $1 AND (active OR $2)")
            .bind(&username_lower)
            .bind(guardian.is_staff())
            .fetch_optional(&mut *conn)
            .await?
    };
    let Some(user_id) = user_id else {
        return Ok(super::topics::not_found_response(&state));
    };
    // can_see_bookmarks?
    if !guardian.is_me(user_id) && !guardian.is_admin() {
        return Ok(super::search::invalid_access(&state));
    }

    // fetch_limit_from_params(default: nil, max: BOOKMARKS_LIMIT)
    let per_page = match params.limit.as_deref() {
        None => None,
        Some(raw) => match raw.trim().parse::<i64>() {
            Ok(n) if (0..=PER_PAGE).contains(&n) => Some(n),
            _ => return Ok(super::search::invalid_parameters(&state, "limit")),
        },
    };
    let page = params.page.as_deref().map(crate::ruby::to_i).unwrap_or(0);

    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let mut bookmarks = Bookmarks {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
    };
    let list = bookmarks
        .load(
            user_id,
            &ListQuery {
                search_term: params.q.as_deref(),
                page,
                per_page,
                exclude_ids: &[],
            },
        )
        .await?;
    if list.is_empty() {
        return Ok(no_store(json!({"bookmarks": []})));
    }
    let items = bookmarks.serialize(&list, false).await?;
    let mut doc = serde_json::Map::new();
    if list.has_more {
        // The username as the request spelled it.
        doc.insert(
            "more_bookmarks_url".into(),
            json!(format!(
                "{}/u/{username}/bookmarks.json?page={}",
                state.config.globals.relative_url_root(),
                page + 1
            )),
        );
    }
    doc.insert("bookmarks".into(), json!(items));
    Ok(no_store(json!({"user_bookmark_list": doc})))
}

/// GET /u/:username/user-menu-bookmarks(.json)
pub async fn user_menu(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(username): Path<String>,
) -> Result<Response, AppError> {
    let Some(viewer) = guardian.user() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    // username_equals_to?
    if viewer.username.to_lowercase() != username.to_lowercase() {
        return Ok(super::search::invalid_access(&state));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let doc = Bookmarks {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
    }
    .user_menu(viewer.id)
    .await?;
    Ok(no_store(doc))
}
