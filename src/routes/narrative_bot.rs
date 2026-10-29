//! discourse-narrative-bot's engine, mounted at /discobot:
//! CertificatesController#generate (GET /discobot/certificate.svg), and
//! the plugin's public files under /plugins/discourse-narrative-bot.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

use crate::params;
use crate::plugins::narrative_bot::certificate::{self, Certificate};
use crate::rate_limiter::{LimitUser, RateLimiter};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

/// GET /discobot/certificate(.svg): requires_plugin, requires_login, the
/// date and user_id params, the certificate rate limits (but for staff),
/// then the user's certificate, cached for a day.
pub async fn certificate(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !crate::plugins::narrative_bot::enabled(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    let Some(viewer) = guardian.user() else {
        return Ok(super::topics::not_found_response(&state));
    };
    let p = params::parse(uri.query(), &headers, b"");
    let param = |key: &str| {
        p.get(key)
            .and_then(params::scalar)
            .filter(|v| !v.trim().is_empty())
    };
    for key in ["date", "user_id"] {
        if param(key).is_none() {
            return Ok(super::search::invalid_parameters(
                &state,
                &format!("{key} must be present"),
            ));
        }
    }
    let date = param("date").unwrap_or_default();
    let user_id_param = param("user_id").unwrap_or_default();
    let user_id = crate::ruby::to_i(&user_id_param);
    let limiter = if user_id != i64::from(viewer.id) {
        RateLimiter::new(
            Some(LimitUser {
                id: viewer.id,
                staff: guardian.is_staff(),
            }),
            "svg_certificate",
            3,
            60,
        )
    } else {
        RateLimiter::new(
            Some(LimitUser {
                id: viewer.id,
                staff: guardian.is_staff(),
            }),
            "svg_certificate_self",
            30,
            600,
        )
    };
    if !guardian.is_staff()
        && let Err(refused) = limiter
            .performed(&state.pool, state.config.rate_limits)
            .await?
    {
        return Ok(refused.into_response(&state.i18n));
    }
    // User.find_by(id: params[:user_id])
    let user: Option<(i32, String)> = if user_id_param.chars().all(|c| c.is_ascii_digit()) {
        sqlx::query_as("SELECT id, username FROM users WHERE id = $1")
            .bind(i32::try_from(user_id).unwrap_or(0))
            .fetch_optional(&mut *conn)
            .await?
    } else {
        None
    };
    let Some((user_id, username)) = user else {
        return Ok(super::topics::not_found_response(&state));
    };
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    // avatar_url(user): the template at 250 pixels, absolute.
    let template = crate::avatar::template_for(&mut conn, &urls, user_id)
        .await?
        .unwrap_or_default();
    let avatar_url = urls.absolute(&format!(
        "{}{}",
        state.config.globals.relative_url_root(),
        template.replace("{size}", "250")
    ))?;
    let discobot: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(crate::plugins::narrative_bot::BOT_USER_ID)
        .fetch_one(&mut *conn)
        .await?;
    let logo = match crate::admin_users::logo_small_url(&mut conn, &settings).await? {
        Some(url) => urls.full_cdn_url(&url)?,
        None => String::new(),
    };
    let locale = match guardian.user_id() {
        Some(id) => crate::plugins::narrative_bot::bot::User::find(&mut conn, id)
            .await?
            .map(|u| u.effective_locale(&settings, &state.i18n))
            .transpose()?,
        None => None,
    }
    .unwrap_or_else(|| {
        settings
            .get("default_locale")
            .map(|v| v.to_s())
            .unwrap_or_default()
    });
    let i18n = state.i18n.locale(&locale);
    let today = crate::clock::now().date_naive();
    let cert = Certificate {
        name: certificate::titleize(&username),
        date: certificate::localize_date(&i18n, certificate::parse_date(&date, today)),
        avatar_url,
        discobot_username: discobot,
        logo_url: certificate::logo_uri(&urls, &logo)?,
        font_url: urls
            .full_cdn_url("/plugins/discourse-narrative-bot/fonts/GreatVibes-Regular.ttf")?,
        i18n,
    };
    let svg = certificate::render(param("type").as_deref() == Some("advanced"), &cert);
    Ok((
        [
            (header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
            (header::CACHE_CONTROL, "max-age=86400, public, immutable"),
        ],
        svg,
    )
        .into_response())
}

/// GET /plugins/discourse-narrative-bot/*file: the plugin's public files.
pub async fn public_file(Path(file): Path<String>) -> Response {
    match certificate::public_file(&file) {
        Some((bytes, kind)) => (
            [
                (header::CONTENT_TYPE, kind),
                (header::CACHE_CONTROL, "max-age=31556952, public, immutable"),
            ],
            bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
