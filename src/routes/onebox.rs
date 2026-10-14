//! OneboxController and InlineOneboxController: GET /onebox, a link's
//! onebox preview for the composer, and GET /inline-onebox, the titles of
//! inline links. Both for members only, one request at a time per member
//! (the `onebox:preview:<id>` and `inline-onebox:preview:<id>` locks).
//! What a preview can't make (another site's) is refused, as post
//! processing refuses it.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::guardian::Guardian;
use crate::onebox::{Opts, Site};
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

/// `InlineOneboxController::MAX_URLS_LIMIT`
const MAX_URLS_LIMIT: usize = 10;

fn int(p: &serde_json::Map<String, Value>, key: &str) -> Option<i32> {
    // params[key].to_i: 0 finds nothing.
    let n = p
        .get(key)
        .and_then(params::scalar)
        .map_or(0, |v| crate::ruby::to_i(&v));
    i32::try_from(n).ok().filter(|n| *n != 0)
}

async fn released(conn: &mut sqlx::PgConnection, key: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM discourse_rs.expiring_keys WHERE key = $1")
        .bind(key)
        .execute(conn)
        .await?;
    Ok(())
}

/// GET /onebox
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let p = params::parse(uri.query(), &headers, &body);
    let url = p.get("url").and_then(params::scalar).unwrap_or_default();
    let refresh = p.get("refresh").and_then(params::scalar).as_deref() == Some("true");
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // Oneboxer.cached_preview: only other sites' previews are cached.
    if !refresh
        && let Some(cached) = crate::onebox::external::cached(&mut conn, &url).await?
        && !cached.preview.trim().is_empty()
    {
        return Ok(plain(cached.preview.trim()));
    }
    // Only one preview at a time per member.
    let lock = format!("onebox:preview:{user_id}");
    if !crate::owned_schema::set_once(&mut conn, &lock, 60).await? {
        return Ok(StatusCode::TOO_MANY_REQUESTS.into_response());
    }
    if crate::owned_schema::cached_get(&mut conn, &format!("onebox_failed__{url}"))
        .await?
        .is_some()
    {
        released(&mut conn, &lock).await?;
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let host = crate::pretty_text::Host::from_state(&state);
    let site = Site {
        settings: &settings,
        i18n: &state.i18n,
        urls: Urls {
            config: &state.config,
            settings: &settings,
        },
        base_path: state.config.globals.relative_url_root(),
        host: Some(&host),
    };
    let opts = Opts {
        editor_id: Some(user_id),
        author_id: Some(user_id),
        category_id: int(&p, "category_id"),
        topic_id: int(&p, "topic_id"),
    };
    // invalidate_oneboxes
    if refresh {
        crate::onebox::external::invalidate(&mut conn, &url).await?;
    }
    let preview = crate::onebox::preview(&mut conn, &site, &url, &opts).await;
    released(&mut conn, &lock).await?;
    let preview = preview?;
    let preview = preview.trim();
    if preview.is_empty() {
        crate::owned_schema::cached_setex(&mut conn, &format!("onebox_failed__{url}"), 3600, "t")
            .await?;
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    Ok(plain(preview))
}

fn plain(body: &str) -> Response {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        body.to_string(),
    )
        .into_response()
}

/// GET /inline-onebox
pub async fn inline(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let p = params::parse(uri.query(), &headers, &body);
    let urls: Vec<String> = match p.get("urls") {
        Some(Value::Array(a)) => a.iter().filter_map(params::scalar).collect(),
        Some(v) => params::scalar(v).into_iter().collect(),
        None => Vec::new(),
    };
    if urls.len() > MAX_URLS_LIMIT {
        return Ok((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({
                "failed": "FAILED",
                "errors": [state.i18n.t("inline_oneboxer.too_many_urls").unwrap_or_default()],
            })),
        )
            .into_response());
    }
    let mut conn = state.pool.acquire().await?;
    let lock = format!("inline-onebox:preview:{user_id}");
    if !crate::owned_schema::set_once(&mut conn, &lock, 60).await? {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "60")],
            Json(json!({
                "failed": "FAILED",
                "errors": [state.i18n.t("inline_oneboxer.concurrency_not_allowed").unwrap_or_default()],
            })),
        )
            .into_response());
    }
    let result = lookup_all(&state, &mut conn, &guardian, &p, &urls).await;
    released(&mut conn, &lock).await?;
    Ok(Json(json!({ "inline-oneboxes": result? })).into_response())
}

async fn lookup_all(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    guardian: &Guardian,
    p: &serde_json::Map<String, Value>,
    urls: &[String],
) -> Result<Vec<Value>, AppError> {
    let settings =
        SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
    let site = Site {
        settings: &settings,
        i18n: &state.i18n,
        urls: Urls {
            config: &state.config,
            settings: &settings,
        },
        base_path: state.config.globals.relative_url_root(),
        host: None,
    };
    let opts = Opts {
        editor_id: guardian.user_id(),
        author_id: guardian.user_id(),
        category_id: int(p, "category_id"),
        topic_id: int(p, "topic_id"),
    };
    let mut out = Vec::new();
    for url in urls {
        if let Some(onebox) = crate::onebox::inline_lookup(conn, &site, url, &opts).await? {
            out.push(onebox);
        }
    }
    Ok(out)
}
