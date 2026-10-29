//! discourse-post-voting's engine, mounted at /post_voting:
//! VotesController (POST and DELETE vote, GET voters, POST and DELETE
//! vote/comment) and CommentsController (GET, POST, PUT and DELETE
//! comments).

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::plugins::post_voting::{comments, votes};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// ActionController::ParameterMissing
fn param_missing(key: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "errors": [format!("param is missing or the value is empty or invalid: {key}")]
        })),
    )
        .into_response()
}

/// `render_json_error(errors, status:)`
fn json_errors(status: StatusCode, errors: Vec<String>) -> Response {
    (status, Json(json!({ "errors": errors }))).into_response()
}

/// A request's params, its CSRF check passed for a write.
struct Request {
    params: Map<String, Value>,
    json_body: bool,
}

impl Request {
    fn parse(
        state: &AppState,
        method: &Method,
        headers: &HeaderMap,
        uri: &Uri,
        body: &Bytes,
    ) -> Result<Self, Box<Response>> {
        let params = params::parse(uri.query(), headers, body);
        if method != Method::GET
            && !csrf_ok(
                state,
                headers,
                &form_pairs(&params),
                uri.path(),
                method.as_str(),
            )
        {
            return Err(Box::new(bad_csrf()));
        }
        let json_body = headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("application/json"));
        Ok(Request { params, json_body })
    }

    fn get(&self, key: &str) -> Option<String> {
        self.params.get(key).and_then(params::scalar)
    }

    /// `params.require(key)`: present and not blank.
    fn require(&self, key: &str) -> Result<String, Box<Response>> {
        self.get(key)
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| Box::new(param_missing(key)))
    }
}

/// `find_by(id:)` of an id param: a non-integer finds nothing.
fn id_of(value: &str) -> i64 {
    value.trim().parse().unwrap_or(-1)
}

fn votes_response(state: &AppState, outcome: votes::Outcome) -> Response {
    match outcome {
        votes::Outcome::Done => Json(json!({ "success": "OK" })).into_response(),
        votes::Outcome::NotFound => super::topics::not_found_response(state),
        votes::Outcome::Forbidden(None) => super::search::invalid_access(state),
        votes::Outcome::Forbidden(Some(message)) => (
            StatusCode::FORBIDDEN,
            [(header::CACHE_CONTROL, "no-cache, no-store")],
            Json(json!({ "errors": [message], "error_type": "invalid_access" })),
        )
            .into_response(),
        votes::Outcome::Refused(message) => json_errors(StatusCode::FORBIDDEN, vec![message]),
        votes::Outcome::Invalid(errors) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "errors": errors, "error_type": "record_invalid" })),
        )
            .into_response(),
    }
}

#[derive(Clone, Copy)]
enum VoteAction {
    Create,
    Destroy,
    CreateComment,
    DestroyComment,
}

/// POST /post_voting/vote
pub async fn create_vote(
    state: State<AppState>,
    guardian: AuthGuardian,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    vote_action(
        state,
        guardian,
        method,
        headers,
        uri,
        body,
        VoteAction::Create,
    )
    .await
}

/// DELETE /post_voting/vote
pub async fn destroy_vote(
    state: State<AppState>,
    guardian: AuthGuardian,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    vote_action(
        state,
        guardian,
        method,
        headers,
        uri,
        body,
        VoteAction::Destroy,
    )
    .await
}

/// POST /post_voting/vote/comment
pub async fn create_comment_vote(
    state: State<AppState>,
    guardian: AuthGuardian,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let action = VoteAction::CreateComment;
    vote_action(state, guardian, method, headers, uri, body, action).await
}

/// DELETE /post_voting/vote/comment
pub async fn destroy_comment_vote(
    state: State<AppState>,
    guardian: AuthGuardian,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let action = VoteAction::DestroyComment;
    vote_action(state, guardian, method, headers, uri, body, action).await
}

async fn vote_action(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    action: VoteAction,
) -> Result<Response, AppError> {
    let request = match Request::parse(&state, &method, &headers, &uri, &body) {
        Ok(r) => r,
        Err(response) => return Ok(*response),
    };
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // requires_plugin, then ensure_logged_in.
    if !crate::plugins::post_voting::enabled(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let host = crate::pretty_text::Host::from_state(&state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let outcome = match action {
        VoteAction::Create | VoteAction::Destroy => {
            // find_vote_post: JSON params are wrapped under `vote`, which
            // skips the require.
            let post_id = match request.get("post_id") {
                Some(id) if !id.trim().is_empty() => id_of(&id),
                _ if request.json_body => -1,
                _ => return Ok(param_missing("post_id")),
            };
            match action {
                VoteAction::Create => {
                    let direction = request.get("direction").filter(|d| !d.is_empty());
                    votes::create(&mut tx, &ctx, &guardian, post_id, direction.as_deref()).await?
                }
                _ => votes::destroy(&mut tx, &ctx, &guardian, post_id).await?,
            }
        }
        VoteAction::CreateComment | VoteAction::DestroyComment => {
            let comment_id = request.get("comment_id").map_or(-1, |id| id_of(&id));
            match action {
                VoteAction::CreateComment => {
                    votes::create_comment_vote(&mut tx, &ctx, &guardian, comment_id).await?
                }
                _ => votes::destroy_comment_vote(&mut tx, &ctx, &guardian, comment_id).await?,
            }
        }
    };
    if matches!(outcome, votes::Outcome::Done) {
        tx.commit().await?;
    }
    Ok(votes_response(&state, outcome))
}

/// GET /post_voting/voters
pub async fn voters(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let request = match Request::parse(&state, &method, &headers, &uri, &Bytes::new()) {
        Ok(r) => r,
        Err(response) => return Ok(*response),
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !crate::plugins::post_voting::enabled(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let post_id = match request.require("post_id") {
        Ok(id) => id_of(&id),
        Err(response) => return Ok(*response),
    };
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    Ok(
        match votes::voters(&mut conn, &settings, &urls, &guardian, post_id).await? {
            Ok(body) => Json(body).into_response(),
            Err(outcome) => votes_response(&state, outcome),
        },
    )
}

fn comments_response(state: &AppState, outcome: comments::Outcome) -> Response {
    match outcome {
        comments::Outcome::Done(body) => Json(body).into_response(),
        comments::Outcome::NotFound => super::topics::not_found_response(state),
        comments::Outcome::Forbidden => super::search::invalid_access(state),
        comments::Outcome::NotLoggedIn => super::login_required::not_logged_in(state),
        comments::Outcome::Errors(errors) => json_errors(StatusCode::FORBIDDEN, errors),
        // Raised without a message: the class name.
        comments::Outcome::InvalidParameters => {
            super::search::invalid_parameters(state, "Discourse::InvalidParameters")
        }
        comments::Outcome::InvalidParameter(name) => super::search::invalid_parameters(state, name),
    }
}

/// PUT /post_voting/comments/flag: CommentsController#flag.
pub async fn flag_comment(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let request = match Request::parse(&state, &method, &headers, &uri, &body) {
        Ok(r) => r,
        Err(response) => return Ok(*response),
    };
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    if !crate::plugins::post_voting::enabled(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    let Some(user) = guardian.user() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let limiter = crate::rate_limiter::RateLimiter::new(
        Some(crate::rate_limiter::LimitUser {
            id: user.id,
            staff: guardian.is_staff(),
        }),
        "flag_post_voting_comment",
        4,
        60,
    );
    if let Err(refused) = limiter
        .performed(&state.pool, state.config.rate_limits)
        .await?
    {
        return Ok(refused.into_response(&state.i18n));
    }
    // ActiveRecord::Type::Boolean: blank is nil, the false values false.
    let boolean =
        |key: &str| crate::ruby::cast_boolean(request.get(key).as_deref()).unwrap_or(false);
    let params = comments::FlagParams {
        comment_id: request.get("comment_id").map_or(-1, |id| id_of(&id)),
        flag_type_id: request
            .get("flag_type_id")
            .map_or(0, |id| crate::ruby::to_i(&id)),
        is_warning: boolean("is_warning"),
        // Present at all is truthy to the guardian's check, "false" too.
        take_action: request.get("take_action").is_some(),
        queue_for_review: request.get("queue_for_review").is_some(),
    };
    let host = crate::pretty_text::Host::from_state(&state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let outcome = comments::flag(&mut tx, &ctx, &guardian, &params).await?;
    if matches!(outcome, comments::Outcome::Done(_)) {
        tx.commit().await?;
    }
    Ok(comments_response(&state, outcome))
}

/// GET, POST, PUT and DELETE /post_voting/comments
pub async fn comments(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let request = match Request::parse(&state, &method, &headers, &uri, &body) {
        Ok(r) => r,
        Err(response) => return Ok(*response),
    };
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // requires_plugin
    if !crate::plugins::post_voting::enabled(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    let host = crate::pretty_text::Host::from_state(&state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let outcome = if method == Method::GET || method == Method::POST {
        // find_post: `post_id` required.
        let post_id = match request.require("post_id") {
            Ok(id) => id_of(&id),
            Err(response) => return Ok(*response),
        };
        if method == Method::GET {
            let last = request.get("last_comment_id");
            match comments::load_more(&mut tx, &ctx, &guardian, post_id, last.as_deref()).await? {
                Some(outcome) => outcome,
                None => return Ok(param_missing("last_comment_id")),
            }
        } else {
            let raw = request.get("raw");
            comments::create(&mut tx, &ctx, &guardian, post_id, raw.as_deref()).await?
        }
    } else {
        // ensure_logged_in, then the required params.
        if guardian.is_anonymous() {
            return Ok(super::login_required::not_logged_in(&state));
        }
        let comment_id = match request.require("comment_id") {
            Ok(id) => id_of(&id),
            Err(response) => return Ok(*response),
        };
        if method == Method::PUT {
            let raw = match request.require("raw") {
                Ok(raw) => raw,
                Err(response) => return Ok(*response),
            };
            comments::update(&mut tx, &ctx, &guardian, comment_id, &raw).await?
        } else {
            comments::destroy(&mut tx, &ctx, &guardian, comment_id).await?
        }
    };
    if matches!(outcome, comments::Outcome::Done(_)) && method != Method::GET {
        tx.commit().await?;
    }
    Ok(comments_response(&state, outcome))
}
