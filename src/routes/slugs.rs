//! SlugsController#generate: POST /slugs(.json) with `name` answers the
//! slug Slug.for(name, "") would make, for trust level 4 and staff. The
//! chat channel modals fill their slug field from it as the name is
//! typed.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::rate_limiter::{LimitUser, RateLimiter};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

const MAX_SLUG_GENERATIONS_PER_MINUTE: i64 = 20;

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// `Slug.for(string, "")` under slug_generation_method.
pub(crate) fn slug_for(settings: &SiteSettings, string: &str) -> Result<String, AppError> {
    Ok(
        match settings.get("slug_generation_method")?.to_s().as_str() {
            "ascii" => crate::posting::text::slug_with_default(
                string,
                &settings.get("default_locale")?.to_s(),
                "",
            )?,
            "encoded" => crate::category_writes::encoded_slug(string),
            _ => String::new(),
        },
    )
}

/// POST /slugs
pub async fn generate(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    // requires_login
    let Some(user) = guardian.user() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let Some(name) = p
        .get("name")
        .and_then(params::scalar)
        .filter(|n| !n.is_empty())
    else {
        return Ok(super::accounts::param_missing("name"));
    };
    if !guardian.has_trust_level(4) {
        return Ok(super::search::invalid_access(&state));
    }
    let limiter = RateLimiter::new(
        Some(LimitUser {
            id: user.id,
            staff: guardian.is_staff(),
        }),
        "max-slug-generations-per-minute",
        MAX_SLUG_GENERATIONS_PER_MINUTE,
        60,
    );
    if let Err(refused) = limiter
        .performed(&state.pool, state.config.rate_limits)
        .await?
    {
        return Ok(refused.into_response(&state.i18n));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let slug = slug_for(&settings, &name)?;
    Ok(Json(json!({ "success": "OK", "slug": slug })).into_response())
}
