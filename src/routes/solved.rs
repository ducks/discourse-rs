//! discourse-solved's AnswerController: POST /solution/accept and
//! /solution/unaccept (`id`, the post), answering with the topic's
//! accepted answers (AcceptedAnswersHelper.serialize) and publishing them
//! on the topic's channel.
//!
//! An anonymous request crashes Rails (`limit_accepts` calls `staff?` on
//! a nil user: a 500); here the permission check refuses it with 403.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::plugins::solved::answers::{self, Outcome};
use crate::rate_limiter::RateLimiter;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_view::{Options, TopicView};
use crate::url::Urls;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

#[derive(Clone, Copy)]
enum Kind {
    Accept,
    Unaccept,
}

/// POST /solution/accept
pub async fn accept(
    state: State<AppState>,
    guardian: AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    run(state, guardian, headers, uri, body, Kind::Accept).await
}

/// POST /solution/unaccept
pub async fn unaccept(
    state: State<AppState>,
    guardian: AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    run(state, guardian, headers, uri, body, Kind::Unaccept).await
}

async fn run(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    kind: Kind,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // requires_plugin
    if !crate::plugins::solved::enabled(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    // limit_accepts, for accept and unaccept: not for staff (nor when a
    // plugin's solved_answers_controller_run_rate_limiter says so).
    if let Some(user) = guardian.user()
        && !guardian.is_staff()
    {
        for limiter in [
            RateLimiter::new(None, &format!("accept-hr-{}", user.id), 20, 3600),
            RateLimiter::new(None, &format!("accept-min-{}", user.id), 4, 30),
        ] {
            if let Err(refused) = limiter
                .performed(&state.pool, state.config.rate_limits)
                .await?
            {
                return Ok(refused.into_response(&state.i18n));
            }
        }
    }
    // The contract: `post_id`, an integer, present.
    let post_id = p
        .get("id")
        .and_then(params::scalar)
        .filter(|v| !v.trim().is_empty() && v.trim().parse::<i64>().is_ok())
        .map(|v| crate::ruby::to_i(&v) as i32);
    let Some(post_id) = post_id else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "failed": "FAILED", "errors": ["Post can't be blank"] })),
        )
            .into_response());
    };
    let outcome = match kind {
        Kind::Accept => {
            let host = crate::pretty_text::Host::from_state(&state);
            let ctx = crate::posting::Ctx {
                host: &host,
                settings: &settings,
                config: &state.config,
                i18n: &state.i18n,
                bus: &state.bus,
            };
            answers::accept(&mut tx, &ctx, &guardian, post_id).await?
        }
        Kind::Unaccept => {
            answers::unaccept(
                &mut tx,
                &state.bus,
                &crate::web_hooks::Payloads {
                    settings: &settings,
                    i18n: &state.i18n,
                    config: &state.config,
                },
                &guardian,
                post_id,
            )
            .await?
        }
    };
    let topic_id = match outcome {
        Outcome::Done(topic_id) => topic_id,
        Outcome::NotFound => return Ok(super::topics::not_found_response(&state)),
        Outcome::Forbidden => return Ok(super::search::invalid_access(&state)),
        Outcome::Failed => {
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "failed": "FAILED" })),
            )
                .into_response());
        }
    };
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let accepted_answers = TopicView {
        conn: &mut tx,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        options: Options {
            page: 0,
            post_number: None,
            ..Default::default()
        },
        post_types: Vec::new(),
        logo_small_url: None,
    }
    .solved_accepted_answers(topic_id)
    .await?;
    // publish_solution / publish_unaccepted
    if matches!(outcome, Outcome::Done(_)) {
        let message = json!({
            "type": match kind {
                Kind::Accept => "accepted_solution",
                Kind::Unaccept => "unaccepted_solution",
            },
            "accepted_answers": accepted_answers,
        });
        crate::bus::publish_to_topic(&state.bus, &mut tx, topic_id, &message).await?;
    }
    tx.commit().await?;
    Ok((StatusCode::OK, Json(accepted_answers)).into_response())
}

/// GET /solution/by_user(.json): SolvedTopicsController#by_user.
pub async fn by_user(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // requires_plugin
    if !crate::plugins::solved::enabled(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    let scalar = |key: &str| p.get(key).and_then(params::scalar);
    let Some(username) = scalar("username").filter(|u| !u.is_empty()) else {
        return Ok(super::accounts::param_missing("username"));
    };
    // fetch_user_from_params(include_inactive: staff, or show_inactive_accounts
    // for a member): the viewer themself, else an active user unless
    // inactive ones are included.
    let username_lower = username.to_lowercase();
    let username_lower = username_lower
        .strip_suffix(".json")
        .unwrap_or(&username_lower)
        .to_string();
    let include_inactive = guardian.is_staff()
        || (guardian.is_authenticated() && settings.get("show_inactive_accounts")?.truthy());
    let user_id: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM users WHERE username_lower = $1 AND (active OR $2 OR id = $3) LIMIT 1",
    )
    .bind(&username_lower)
    .bind(include_inactive)
    .bind(guardian.user_id().unwrap_or(0))
    .fetch_optional(&mut *conn)
    .await?;
    let Some(user) = (match user_id {
        Some(id) => crate::users::User::find_by_id(&mut conn, id).await?,
        None => None,
    }) else {
        return Ok(super::topics::not_found_response(&state));
    };
    // public_can_see_profiles?, can_see_profile?, can_see_user_actions?
    let public =
        guardian.is_authenticated() || !settings.get("hide_user_profiles_from_public")?.truthy();
    let sees_private =
        guardian.is_authenticated() && (guardian.is_me(user.id) || guardian.is_admin());
    if !public
        || !user.visible_to(&settings, &guardian)?
        || (!sees_private && settings.get("hide_user_activity_tab")?.truthy())
    {
        return Ok(super::topics::not_found_response(&state));
    }
    let offset = scalar("offset")
        .map(|v| crate::ruby::to_i(&v))
        .unwrap_or(0)
        .max(0);
    let limit = scalar("limit").map(|v| crate::ruby::to_i(&v)).unwrap_or(30);
    if limit < 0 {
        return Err(crate::Unsupported("a negative by_user limit").into());
    }
    let host = crate::pretty_text::Host::from_state(&state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let body = crate::plugins::solved::by_user::by_user(
        &mut conn, &ctx, &urls, &guardian, user.id, offset, limit,
    )
    .await?;
    Ok(Json(body).into_response())
}

/// POST /solution/shared_issue(.json): SharedIssueController#create.
pub async fn shared_issue(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    use crate::plugins::solved::shared_issue::{Outcome, toggle};
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // requires_plugin, requires_login
    if !crate::plugins::solved::enabled(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    // rate_limit_shared_issue: not for staff, nor without a topic_id
    // (keyed by the param as sent).
    if let Some(user) = guardian.user()
        && !guardian.is_staff()
        && let Some(raw) = p
            .get("topic_id")
            .and_then(params::scalar)
            .filter(|v| !v.trim().is_empty())
    {
        let limiter = RateLimiter::new(None, &format!("shared-issue-{}-{raw}", user.id), 5, 3600);
        if let Err(refused) = limiter
            .performed(&state.pool, state.config.rate_limits)
            .await?
        {
            return Ok(refused.into_response(&state.i18n));
        }
    }
    // The contract: `topic_id`, an integer, present.
    let topic_id = p
        .get("topic_id")
        .and_then(params::scalar)
        .filter(|v| !v.trim().is_empty() && v.trim().parse::<i64>().is_ok())
        .map(|v| crate::ruby::to_i(&v) as i32);
    let Some(topic_id) = topic_id else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "failed": "FAILED", "errors": ["Topic can't be blank"] })),
        )
            .into_response());
    };
    match toggle(&mut tx, &state.bus, &settings, &guardian, topic_id).await? {
        Outcome::Done { count, created } => {
            tx.commit().await?;
            Ok(
                Json(json!({ "count": count, "user_created_shared_issue": created }))
                    .into_response(),
            )
        }
        Outcome::NotFound => Ok(super::topics::not_found_response(&state)),
        Outcome::Forbidden => Ok(super::search::invalid_access(&state)),
    }
}

/// GET /admin/plugins/solved/dashboard-support(.json)
/// (AdminDashboardSupportController#index, under StaffConstraint): the
/// dashboard's support section for the period and categories asked for,
/// as the section refetches it when its category filter changes.
pub async fn dashboard_support(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    let p = params::parse(uri.query(), &headers, &body);
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // requires_plugin
    if !crate::plugins::solved::enabled(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    let mut dates = [None, None];
    for (slot, key) in dates.iter_mut().zip(["start_date", "end_date"]) {
        *slot = match p.get(key) {
            None | Some(Value::Null) => None,
            Some(v @ (Value::String(_) | Value::Number(_) | Value::Bool(_))) => params::scalar(v),
            Some(_) => {
                return Err(crate::Unsupported("support section dates that aren't strings").into());
            }
        };
    }
    let ids = p.get("category_ids").cloned().unwrap_or(Value::Null);
    let data = crate::plugins::solved::dashboard::build(
        &mut conn,
        &settings,
        &guardian,
        dates[0].as_deref(),
        dates[1].as_deref(),
        &ids,
    )
    .await?;
    Ok(Json(data).into_response())
}
