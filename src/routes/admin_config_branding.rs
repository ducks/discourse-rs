//! Admin::Config::LogoController and Admin::Config::FontsController: the
//! logo and fonts pages (90g6) and their updates, SiteSetting::Update over
//! the pages' settings in one transaction.
//!
//! Refused: the topic Open Graph image preview (TopicOgImageGenerator is
//! not ported).

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use super::admin_pages::{Shell, wants_json};
use super::session::{bad_csrf, csrf_ok};
use crate::session::current::AuthGuardian;
use crate::site_setting_update::{self, Outcome};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported, params};

/// SiteSetting::Update.call with `(name, value, backfill)` for each
/// setting, sorted as the settings are declared (the type supervisor's
/// dependency order), in one transaction. Err(message) for what the
/// controllers raise as Discourse::InvalidParameters.
async fn update_settings(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    mut updates: Vec<(String, String, bool)>,
) -> Result<Result<(), String>, AppError> {
    let defs = &state.site_setting_defs;
    updates.sort_by_key(|(name, ..)| defs.position(name).unwrap_or(usize::MAX));
    let mut tx = state.pool.begin().await?;
    let settings = SiteSettings::load(&mut tx, defs, &state.config.globals).await?;
    for (name, raw, backfill) in &updates {
        let outcome = site_setting_update::update(
            &mut tx,
            &state.bus,
            &state.config,
            defs,
            &settings,
            &state.i18n,
            guardian,
            name,
            raw,
            *backfill,
        )
        .await?;
        if let Outcome::Invalid(message) = outcome {
            return Ok(Err(message));
        }
    }
    tx.commit().await?;
    Ok(Ok(()))
}

/// Admin::AdminController behind StaffConstraint: not found for anyone
/// but staff, InvalidAccess for a moderator.
pub(super) fn admin_only(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
) -> Option<Response> {
    if guardian.is_admin() {
        None
    } else if guardian.is_staff() {
        Some(super::search::invalid_access(state))
    } else {
        Some(super::topics::not_found_response(state))
    }
}

/// The update's answer: success_json, or the InvalidParameters raised.
fn answer(state: &AppState, outcome: Result<(), String>) -> Response {
    match outcome {
        Ok(()) => Json(json!({ "success": "OK" })).into_response(),
        Err(message) => super::search::invalid_parameters(state, &message),
    }
}

/// The request's params, None when its CSRF check fails.
fn checked_params(
    state: &AppState,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let p = params::parse(uri.query(), headers, body);
    let form: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    csrf_ok(state, headers, &form, uri.path(), "PUT").then_some(p)
}

/// `params[name].to_s`
fn param(p: &serde_json::Map<String, serde_json::Value>, name: &str) -> String {
    p.get(name).and_then(params::scalar).unwrap_or_default()
}

/// PUT /admin/config/logo(.json)
pub async fn logo_update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Some(p) = checked_params(&state, &headers, &uri, &body) else {
        return Ok(bad_csrf());
    };
    if let Some(refused) = admin_only(&state, &guardian) {
        return Ok(refused);
    }
    // The hidden ones left out.
    let updates = crate::admin_branding_view::LOGO_SETTINGS
        .iter()
        .filter(|name| {
            state
                .site_setting_defs
                .get(name)
                .is_some_and(|def| !site_setting_update::is_hidden(def))
        })
        .map(|name| (name.to_string(), param(&p, name), false))
        .collect();
    let outcome = update_settings(&state, &guardian, updates).await?;
    Ok(answer(&state, outcome))
}

/// PUT /admin/config/fonts(.json): the fonts and the default text size,
/// whose backfill `update_existing_users` asks for.
pub async fn fonts_update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Some(p) = checked_params(&state, &headers, &uri, &body) else {
        return Ok(bad_csrf());
    };
    if let Some(refused) = admin_only(&state, &guardian) {
        return Ok(refused);
    }
    let backfill = param(&p, "update_existing_users") == "true";
    let updates = vec![
        ("base_font".to_string(), param(&p, "base_font"), false),
        ("heading_font".to_string(), param(&p, "heading_font"), false),
        (
            "default_text_size".to_string(),
            param(&p, "default_text_size"),
            backfill,
        ),
    ];
    let outcome = update_settings(&state, &guardian, updates).await?;
    Ok(answer(&state, outcome))
}

/// GET /admin/config/logo/og-image-preview(.json)
pub async fn og_image_preview(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    if let Some(refused) = admin_only(&state, &guardian) {
        return Ok(refused);
    }
    Err(Unsupported("the topic Open Graph image preview (TopicOgImageGenerator)").into())
}

/// Which page.
#[derive(Clone, Copy)]
pub enum Page {
    Logo,
    Fonts,
    WelcomeBanner,
}

/// What AdminWelcomeBannerForm loads: the themes with the banner's
/// theme setting, the settings, the site texts in the user's locale.
async fn welcome_banner(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    shell: &Shell,
) -> Result<String, AppError> {
    let themes: Vec<(i64, String, bool)> = super::themes::welcome_banner_theme_list(state, conn)
        .await?
        .iter()
        .map(|t| {
            (
                t["id"].as_i64().unwrap_or_default(),
                t["name"].as_str().unwrap_or_default().to_string(),
                t["enable_welcome_banner"].as_bool().unwrap_or_default(),
            )
        })
        .collect();
    let get = |name: &str| {
        shell
            .settings
            .get(name)
            .map(|v| v.to_s())
            .unwrap_or_default()
    };
    // The upload setting's URL (getURL'd by the client).
    let image_id = crate::ruby::to_i(&get("welcome_banner_image"));
    let image: Option<String> = if image_id == 0 {
        None
    } else {
        sqlx::query_scalar("SELECT url FROM uploads WHERE id = $1")
            .bind(image_id as i32)
            .fetch_optional(&mut *conn)
            .await?
    };
    let image = image
        .map(|u| format!("{}{u}", shell.site.base_path))
        .unwrap_or_default();
    // I18n.currentLocale(): the site's locale here.
    let locale = shell.site.lang.clone();
    let locales: Vec<(String, String)> = crate::setting_enums::recorded("default_locale")
        .and_then(|r| r.get("valid_values"))
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(|v| {
                    // translate_names: the name is a client key.
                    let name = v["name"].as_str().unwrap_or_default();
                    (
                        v["value"].as_str().unwrap_or_default().to_string(),
                        crate::admin_view::t_opt(&state.i18n, name)
                            .unwrap_or_else(|| name.to_string()),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let overrides = crate::site_texts::Overrides::load(&mut *conn, &locale).await?;
    let mut texts = Vec::new();
    for (name, key, ..) in crate::admin_branding_view::WELCOME_BANNER_TEXTS {
        let record = crate::site_texts::record_for(&overrides, key, None, &locale)?;
        texts.push((
            name.to_string(),
            record.value.as_str().unwrap_or_default().to_string(),
        ));
    }
    let (color, visibility, location) = (
        get("welcome_banner_text_color"),
        get("welcome_banner_page_visibility"),
        get("welcome_banner_location"),
    );
    Ok(crate::admin_branding_view::welcome_banner(
        &shell.cx(state),
        &crate::admin_branding_view::WelcomeBanner {
            themes: &themes,
            image: &image,
            text_color: &color,
            page_visibility: &visibility,
            location: &location,
            locale: &locale,
            locales: &locales,
            texts: &texts,
        },
    ))
}

/// GET /admin/config/welcome-banner(.json)
pub async fn welcome_banner_page(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    page(&state, &guardian, &headers, &uri, Page::WelcomeBanner).await
}

/// GET /admin/config/logo and /admin/config/fonts: the page, or for JSON
/// the controllers' empty index (204).
pub async fn page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    page: Page,
) -> Result<Response, AppError> {
    if let Some(refused) = admin_only(state, guardian) {
        return Ok(refused);
    }
    if wants_json(uri, headers) {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let mut conn = state.pool.acquire().await?;
    let (link, key) = match page {
        Page::Logo => ("admin_logo", "logo"),
        Page::Fonts => ("admin_fonts", "fonts"),
        Page::WelcomeBanner => ("admin_welcome_banner", "admin-welcome-banner"),
    };
    let shell = Shell::load(&mut conn, state, guardian, headers, link).await?;
    let content = match page {
        Page::Logo => {
            // What AdminLogoForm reads: the site's settings and the
            // branding settings' placeholders, which it loads from
            // /admin/config/site_settings.json (measuring their uploads).
            let filters = super::admin_site_settings::config_filters(
                state,
                json!({ "categories": ["branding"] })
                    .as_object()
                    .ok_or_else(|| std::io::Error::other("the branding filter"))?,
            )
            .ok_or_else(|| std::io::Error::other("the branding filter"))?;
            let (settings, _) =
                super::admin_site_settings::document(&mut conn, state, &shell.settings, &filters)
                    .await?;
            crate::admin_branding_view::logo(&shell.cx(state), &settings)
        }
        Page::Fonts => crate::admin_branding_view::fonts(&shell.cx(state)),
        Page::WelcomeBanner => welcome_banner(state, &mut conn, &shell).await?,
    };
    let cx = shell.cx(state);
    let strings = crate::admin_ui::strings(
        &cx,
        "admin-branding-strings",
        crate::admin_branding_view::STRINGS,
    );
    let main = crate::admin_view::contents(
        &format!("admin-config {key}"),
        &format!("{content}{strings}"),
    );
    // The welcome banner's route has no title token.
    let title = match page {
        Page::WelcomeBanner => String::new(),
        _ => crate::admin_view::t(cx.i18n, &format!("admin.config.{key}.title")),
    };
    shell.render(
        state,
        uri,
        &title,
        main,
        "admin-branding.js",
        StatusCode::OK,
    )
}

/// GET /admin/config/logo(.json)
pub async fn logo_page(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    page(&state, &guardian, &headers, &uri, Page::Logo).await
}

/// GET /admin/config/fonts(.json)
pub async fn fonts_page(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    page(&state, &guardian, &headers, &uri, Page::Fonts).await
}

/// The logo and fonts routes, each with its `.json`.
pub(super) fn routes(mut router: axum::Router<AppState>) -> axum::Router<AppState> {
    use axum::routing::get;
    for suffix in ["", ".json"] {
        router = router
            .route(
                &format!("/admin/config/logo{suffix}"),
                get(logo_page).put(logo_update),
            )
            .route(
                &format!("/admin/config/fonts{suffix}"),
                get(fonts_page).put(fonts_update),
            )
            .route(
                &format!("/admin/config/logo/og-image-preview{suffix}"),
                get(og_image_preview),
            )
            .route(
                &format!("/admin/config/welcome-banner{suffix}"),
                get(welcome_banner_page),
            );
    }
    router
}
