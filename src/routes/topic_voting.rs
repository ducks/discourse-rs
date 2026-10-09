//! discourse-topic-voting's VotesController, mounted at /voting: POST
//! vote and unvote (`topic_id`), answering with the voter's votes and the
//! topic's count and voters, and GET who, the topic's voters.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::plugins::topic_voting::votes::{self, Outcome, VOTER_PREVIEW_LIMIT};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

#[derive(Clone, Copy)]
enum Kind {
    Vote,
    Unvote,
}

/// POST /voting/vote
pub async fn vote(
    state: State<AppState>,
    guardian: AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    run(state, guardian, headers, uri, body, Kind::Vote).await
}

/// POST /voting/unvote
pub async fn unvote(
    state: State<AppState>,
    guardian: AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    run(state, guardian, headers, uri, body, Kind::Unvote).await
}

fn respond(state: &AppState, outcome: Outcome) -> Response {
    match outcome {
        Outcome::Done(body) => (StatusCode::OK, Json(body)).into_response(),
        Outcome::OutOfVotes(body) => (StatusCode::FORBIDDEN, Json(body)).into_response(),
        Outcome::NotFound => super::topics::not_found_response(state),
        Outcome::Forbidden => super::search::invalid_access(state),
    }
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
    // requires_plugin, then ensure_logged_in.
    if !crate::plugins::topic_voting::enabled(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    // The contract: `topic_id`, an integer, present.
    let topic_id = p
        .get("topic_id")
        .and_then(params::scalar)
        .filter(|v| !v.trim().is_empty())
        .map(|v| crate::ruby::to_i(&v) as i32);
    let Some(topic_id) = topic_id else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "failed": "FAILED", "errors": ["Topic can't be blank"] })),
        )
            .into_response());
    };
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let outcome = match kind {
        Kind::Vote => votes::cast(&mut tx, &settings, &urls, &guardian, topic_id).await?,
        Kind::Unvote => votes::remove(&mut tx, &settings, &urls, &guardian, topic_id).await?,
    };
    if matches!(outcome, Outcome::Done(_)) {
        tx.commit().await?;
    }
    Ok(respond(&state, outcome))
}

/// GET /voting/who
pub async fn who(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &Bytes::new());
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !crate::plugins::topic_voting::enabled(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    // params.require(:topic_id)
    let Some(topic_id) = p
        .get("topic_id")
        .and_then(params::scalar)
        .filter(|v| !v.is_empty())
    else {
        return Ok(super::accounts::param_missing("topic_id"));
    };
    let limit = p
        .get("limit")
        .and_then(params::scalar)
        .filter(|v| !v.trim().is_empty())
        .map(|v| crate::ruby::to_i(&v))
        .filter(|l| *l > 0)
        .map_or(VOTER_PREVIEW_LIMIT, |l| l.min(VOTER_PREVIEW_LIMIT));
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let outcome = votes::who(
        &mut conn,
        &settings,
        &urls,
        &guardian,
        crate::ruby::to_i(&topic_id) as i32,
        limit,
    )
    .await?;
    Ok(respond(&state, outcome))
}

/// GET /topics/voted-by/:username(.json): ListController#voted_by, the
/// topics the user holds an active vote on. The HTML page is the profile's
/// votes tab, not ported yet.
pub async fn voted_by(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    axum::extract::Path(username): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<super::list::ListParams>,
) -> Result<Response, AppError> {
    let Some(username) = username.strip_suffix(".json") else {
        return Err(crate::Unsupported("the voted_by HTML page").into());
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // ensure_discourse_topic_voting
    if !crate::plugins::topic_voting::enabled(&settings)?
        || !settings.get("topic_voting_show_votes_on_profile")?.truthy()
    {
        return Ok(super::topics::not_found_response(&state));
    }
    // fetch_user_from_params(include_inactive: staff)
    let target: Option<(i32, bool)> =
        sqlx::query_as("SELECT id, active FROM users WHERE username_lower = $1 LIMIT 1")
            .bind(username.to_lowercase())
            .fetch_optional(&mut *conn)
            .await?;
    let user_id = match target {
        Some((id, active)) if active || guardian.is_staff() || guardian.is_me(id) => id,
        _ => return Ok(super::topics::not_found_response(&state)),
    };
    let mut options = match super::list::build_options(&params, &settings)? {
        Ok(o) => o,
        Err(message) => return Ok(super::list::invalid_list_params(&state, &message, true)),
    };
    options.no_definitions = false;
    if params.period.is_some() {
        return Err(crate::Unsupported("period on the voted_by list").into());
    }
    let list_path = format!(
        "{}/topics/voted-by/{username}",
        state.config.globals.relative_url_root()
    );
    let more = super::list::next_url(&list_path, &params, &options, false);
    let json = voted_by_list(
        &state, &mut conn, &settings, &guardian, user_id, options, more,
    )
    .await?;
    Ok(Json(json).into_response())
}

/// TopicQuery#list_voted_by(user), serialized as a topic list.
pub(crate) async fn voted_by_list(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    user_id: i32,
    options: crate::topic_query::Options,
    more_topics_url: String,
) -> Result<Value, AppError> {
    let topics = crate::topic_query::TopicQuery {
        conn: &mut *conn,
        settings,
        guardian,
        options,
        category: Default::default(),
        tags: Default::default(),
        filter: Default::default(),
        user: Default::default(),
    }
    .list(crate::topic_query::Filter::VotedBy(user_id))
    .await?;
    let urls = Urls {
        config: &state.config,
        settings,
    };
    Ok(crate::topic_list::TopicListSerializer {
        conn,
        settings,
        i18n: &state.i18n,
        guardian,
        urls: &urls,
        more_topics_url: Some(more_topics_url),
        category_id: None,
        group_id: None,
        prefetched: Default::default(),
    }
    .serialize(&topics)
    .await?)
}
