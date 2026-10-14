//! Port of TopicsController#timer: POST /t/:topic_id/timer, setting or
//! removing a topic's timer.

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
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_timer::{self, SetOptions, SetOutcome, When};
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// POST /t/:topic_id/timer
pub async fn timer(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(topic_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    // :topic_id is constrained to digits.
    if topic_id.is_empty() || !topic_id.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(super::topics::not_found_response(&state));
    }
    let param = |key: &str| p.get(key).and_then(params::scalar);
    let present = |key: &str| param(key).filter(|v| !v.trim().is_empty());

    let Some(status_name) = present("status_type") else {
        return Ok(super::accounts::param_missing("status_type"));
    };
    let Some(status_type) = topic_timer::public_type_by_name(&status_name) else {
        return Ok(super::search::invalid_parameters(&state, "status_type"));
    };
    // Any value, "false" and "" too, asks for a duration.
    let based_on_last_post = param("based_on_last_post");
    if based_on_last_post.is_some() && present("duration_minutes").is_none() {
        return Ok(super::accounts::param_missing("duration_minutes"));
    }

    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let s = &settings;
    let topic_id = crate::ruby::to_i(&topic_id) as i32;
    let topic: Option<bool> =
        sqlx::query_scalar("SELECT closed FROM topics WHERE id = $1 AND deleted_at IS NULL")
            .bind(topic_id)
            .fetch_optional(&mut *tx)
            .await?;
    let topic_ctx = match topic {
        Some(_) => crate::topic_guardian::TopicCtx::load(&mut tx, s, &guardian, topic_id).await?,
        None => None,
    };
    // ensure_can_set_topic_timer!(topic): a missing topic skips the
    // visibility check.
    if !topic_timer::can_set_topic_timer_for(&mut tx, s, &guardian, topic.map(|_| topic_id)).await?
    {
        return Ok(super::search::invalid_access(&state));
    }
    if topic_timer::is_destructive(status_type) {
        let can_delete = match &topic_ctx {
            Some(t) => guardian.can_delete_topic(s, t)?,
            None => false,
        };
        if !can_delete {
            return Ok(super::search::invalid_access(&state));
        }
    }
    let creating = present("time").is_some() || present("duration_minutes").is_some();
    if status_type == topic_timer::types::PUBLISH_TO_CATEGORY && creating {
        let category: Option<i32> =
            match present("category_id").and_then(|c| crate::ruby::integer(&c)) {
                Some(id) => {
                    sqlx::query_scalar("SELECT id FROM categories WHERE id = $1")
                        .bind(id)
                        .fetch_optional(&mut *tx)
                        .await?
                }
                None => None,
            };
        let Some(category) = category else {
            return Ok(super::topics::not_found_response(&state));
        };
        // can_create_topic_on_category?
        let allowed = guardian.can_create_topic(&mut tx, s).await?
            && guardian
                .topic_create_allowed_category_ids(&mut tx, s)
                .await?
                .contains(&category);
        if !allowed {
            return Ok(super::search::invalid_access(&state));
        }
        if let Some(t) = &topic_ctx
            && t.private_message()
        {
            let anywhere = guardian.can_create_post_anywhere(&mut tx, s).await?;
            let can_create_post = guardian.can_create_post(s, t, anywhere)?;
            if !guardian.can_convert_topic(s, t, can_create_post)? {
                return Ok(super::search::invalid_access(&state));
            }
        }
    }
    let Some(closed_before) = topic else {
        // nil.set_or_create_timer
        return Err(std::io::Error::other("NoMethodError: timer on a missing topic").into());
    };

    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let time = param("time");
    let opts = SetOptions {
        by_user: guardian.user_id(),
        based_on_last_post: based_on_last_post.is_some_and(|v| !v.trim().is_empty()),
        category_id: present("category_id").map(|c| crate::ruby::to_i(&c)),
        duration_minutes: present("duration_minutes").map(|d| crate::ruby::to_i(&d)),
    };
    let when = match &time {
        Some(t) => When::Text(t),
        None => When::Blank,
    };
    let outcome =
        topic_timer::set_or_create_timer(&mut tx, &ctx, topic_id, status_type, when, &opts).await?;
    let timer = match outcome {
        SetOutcome::Invalid(message) => {
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "errors": [message] })),
            )
                .into_response());
        }
        SetOutcome::InvalidParameters => {
            return Ok(super::search::invalid_parameters(
                &state,
                "Discourse::InvalidParameters",
            ));
        }
        SetOutcome::Nothing => None,
        SetOutcome::Saved { timer, created } => Some((timer, created)),
    };
    // topic.save: a validated save rewrites the slug and clears
    // fancy_title.
    let slug = crate::posting::topic_save::reassigned_slug(&mut tx, s, topic_id).await?;
    sqlx::query(
        "UPDATE topics SET slug = $2, fancy_title = NULL, updated_at = clock_timestamp() \
         WHERE id = $1 AND (slug IS DISTINCT FROM $2 OR fancy_title IS NOT NULL)",
    )
    .bind(topic_id)
    .bind(&slug)
    .execute(&mut *tx)
    .await?;
    // `closed` is the controller's topic: a new timer was built on it, so
    // it sees the status the timer changed; a timer found in the database
    // changed its own copy, leaving this one as it was.
    let closed = match &timer {
        Some((_, true)) => {
            sqlx::query_scalar::<_, bool>("SELECT closed FROM topics WHERE id = $1")
                .bind(topic_id)
                .fetch_one(&mut *tx)
                .await?
        }
        _ => closed_before,
    };
    tx.commit().await?;
    let timer = timer.map(|(t, _)| t);
    Ok(Json(json!({
        "success": "OK",
        "execute_at": timer.as_ref().map(|t| crate::topic_list::time_json(t.execute_at)),
        "duration_minutes": timer.as_ref().and_then(|t| t.duration_minutes),
        "based_on_last_post": timer.as_ref().map(|t| t.based_on_last_post),
        "closed": closed,
        "category_id": timer.as_ref().and_then(|t| t.category_id),
    }))
    .into_response())
}
