//! Admin::DashboardController#new_features and #toggle_feature:
//! /admin/whats-new and /admin/dashboard/whats-new (the merged feed for
//! staff, `force_refresh` fetching it again first; the newest entry's
//! date bumped for an admin and marked seen), the page as HTML, and
//! /admin/toggle-feature (UpcomingChanges::Toggle). StaffConstraint:
//! everyone else gets not found.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::admin_pages::{Shell, wants_json};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, new_features, params};

/// What #new_features does before rendering: the newest entry's date
/// bumped for an admin, then the features with the unseen flag, and all
/// marked seen.
async fn load(
    conn: &mut PgConnection,
    state: &AppState,
    guardian: &crate::guardian::Guardian,
) -> Result<(Vec<Map<String, Value>>, bool), AppError> {
    let features = new_features::merged(&mut *conn, state).await?;
    let user_id = guardian.user_id().unwrap_or_default();
    if guardian.is_admin()
        && let Some(created_at) = features.first().and_then(|f| f.get("created_at"))
    {
        new_features::bump_last_viewed(&mut *conn, user_id, created_at).await?;
    }
    let unseen = new_features::has_unseen(&mut *conn, user_id).await?;
    new_features::mark_seen(&mut *conn, state, user_id).await?;
    Ok((features, unseen))
}

/// GET /admin/whats-new(.json) and /admin/dashboard/whats-new(.json)
pub async fn new_features(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    let mut conn = state.pool.acquire().await?;
    if !wants_json(&uri, &headers) {
        // The route's page: DashboardNewFeatures loads the JSON (without a
        // refresh), which the page draws as it does; the dot on its link
        // goes (the route's activate).
        let (features, _) = load(&mut conn, &state, &guardian).await?;
        let shell = Shell::load(&mut conn, &state, &guardian, &headers, "admin_whats_new").await?;
        let cx = shell.cx(&state);
        let host = crate::pretty_text::Host::from_state(&state);
        let opts = crate::pretty_text::MarkdownOptions {
            omit_nofollow: true,
            ..Default::default()
        };
        let mut cooked = Vec::new();
        for item in &features {
            let raw = item
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default();
            cooked.push(crate::pretty_text::cook(&host, raw, &opts).await?);
        }
        let list: Vec<crate::admin_whats_new_view::Feature> = features
            .iter()
            .zip(cooked)
            .map(|(item, cooked)| crate::admin_whats_new_view::Feature { item, cooked })
            .collect();
        let strings = crate::admin_ui::strings(
            &cx,
            "admin-whats-new-strings",
            &["admin.dashboard.new_features.no_new_features_error"],
        );
        let main = crate::admin_view::contents(
            "whats-new",
            &format!("{}{strings}", crate::admin_whats_new_view::page(&cx, &list)),
        );
        let title = crate::admin_view::t(cx.i18n, "admin.dashboard.new_features.title");
        return shell.render(
            &state,
            &uri,
            &title,
            main,
            "admin-whats-new.js",
            StatusCode::OK,
        );
    }
    let p = params::parse(uri.query(), &headers, &body);
    if params::string(&p, "force_refresh").as_deref() == Some("true") {
        let user_id = guardian.user_id().unwrap_or_default();
        let limiter = crate::rate_limiter::RateLimiter::new(
            Some(crate::rate_limiter::LimitUser {
                id: user_id,
                staff: true,
            }),
            "force-refresh-new-features",
            5,
            60,
        )
        .apply_limit_to_staff();
        if let Err(refused) = limiter
            .performed(&state.pool, state.config.rate_limits)
            .await?
        {
            return Ok(refused.into_response(&state.i18n));
        }
        let settings =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
        let body = new_features::fetch(&state, &settings).await?;
        new_features::update(&mut conn, &state, &body).await?;
    }
    let (features, unseen) = load(&mut conn, &state, &guardian).await?;
    let general = crate::admin_dashboard::general_data(&mut conn).await?;
    Ok(Json(json!({
        "new_features": features,
        "has_unseen_features": unseen,
        "release_notes_link": general["release_notes_link"],
    }))
    .into_response())
}

/// POST /admin/toggle-feature(.json): UpcomingChanges::Toggle, a setting
/// that isn't an upcoming change refused as InvalidAccess.
pub async fn toggle_feature(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Some(p) = super::admin_config_about::params_of(&state, &headers, &uri, &body, "POST")
    else {
        return Ok(super::session::bad_csrf());
    };
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    let unavailable = super::search::invalid_access(&state);
    super::admin_config_upcoming_changes::toggle_change(&state, &guardian, &p, unavailable).await
}

pub(super) fn routes(mut router: axum::Router<AppState>) -> axum::Router<AppState> {
    use axum::routing::{get, post};
    for suffix in ["", ".json"] {
        router = router
            .route(&format!("/admin/whats-new{suffix}"), get(new_features))
            .route(
                &format!("/admin/dashboard/whats-new{suffix}"),
                get(new_features),
            )
            .route(
                &format!("/admin/toggle-feature{suffix}"),
                post(toggle_feature),
            );
    }
    router
}
