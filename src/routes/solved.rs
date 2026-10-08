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
        Kind::Accept => answers::accept(&mut tx, &state.bus, &settings, &guardian, post_id).await?,
        Kind::Unaccept => {
            answers::unaccept(&mut tx, &state.bus, &settings, &guardian, post_id).await?
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
        },
        post_types: Vec::new(),
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
