//! The setup wizard's JSON (WizardController#index, StepsController#update)
//! and the admin design wizard (Admin::Config::DesignWizardController).
//! The wizards' pages are Ember's (90g).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

fn json_request(headers: &HeaderMap, uri: &Uri) -> bool {
    uri.path().ends_with(".json")
        || headers
            .get("x-requested-with")
            .is_some_and(|v| v == "XMLHttpRequest")
        || super::wants_json(uri.query(), headers)
}

/// requires_login, ensure_admin and ensure_wizard_enabled, in the order
/// the controller runs them.
async fn wizard_gate(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    settings: &SiteSettings,
    enabled_first: bool,
) -> Result<Option<Response>, AppError> {
    if !guardian.is_authenticated() {
        return Ok(Some(super::login_required::not_logged_in(state)));
    }
    let enabled = settings.get("wizard_enabled")?.truthy();
    if enabled_first && !enabled {
        return Ok(Some(super::search::invalid_access(state)));
    }
    if !guardian.is_admin() {
        return Ok(Some(super::search::invalid_access(state)));
    }
    if !enabled {
        return Ok(Some(super::search::invalid_access(state)));
    }
    Ok(None)
}

/// GET /wizard(.json), /wizard/steps/:id(.json): the wizard for the admin.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if let Some(r) = wizard_gate(&state, &guardian, &settings, false).await? {
        return Ok(r);
    }
    if !json_request(&headers, &uri) {
        return Err(Unsupported("the wizard page (Ember)").into());
    }
    let cx = crate::wizard::Cx {
        settings: &settings,
        defs: &state.site_setting_defs,
        i18n: &state.i18n,
    };
    let body = crate::wizard::wizard_json(&mut conn, &cx).await?;
    Ok(Json(body).into_response())
}

/// GET /wizard/steps/:id(.json)
pub async fn step_page(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path(_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    index(state, guardian, headers, uri).await
}

/// PUT /wizard/steps/:id(.json): StepsController#update.
pub async fn update_step(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    let pairs: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !super::session::csrf_ok(&state, &headers, &pairs, uri.path(), "PUT") {
        return Ok(super::session::bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    if let Some(r) = wizard_gate(&state, &guardian, &settings, true).await? {
        return Ok(r);
    }
    let actor = guardian
        .user_id()
        .ok_or(Unsupported("an admin without a user"))?;
    let id = raw.strip_suffix(".json").unwrap_or(&raw);
    let fields = match p.get("fields") {
        Some(Value::Object(m)) => m.clone(),
        _ => {
            return Err(std::io::Error::other("NoMethodError: [] on nil fields").into());
        }
    };
    let cx = crate::wizard::Cx {
        settings: &settings,
        defs: &state.site_setting_defs,
        i18n: &state.i18n,
    };
    let outcome = crate::wizard::update_step(&mut tx, &state.bus, &cx, actor, id, &fields).await?;
    tx.commit().await?;
    Ok(match outcome {
        crate::wizard::StepOutcome::Done => Json(json!({"success": "OK"})).into_response(),
        crate::wizard::StepOutcome::Invalid(errors) => {
            let errors: Vec<Value> = errors
                .into_iter()
                .map(|(field, description)| json!({"field": field, "description": description}))
                .collect();
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "errors": errors })),
            )
                .into_response()
        }
        crate::wizard::StepOutcome::UnknownStep => {
            super::search::invalid_parameters(&state, "step")
        }
    })
}

/// The config namespace's StaffConstraint (a 404), then the controller's
/// ensure_admin (a 403).
fn admin_only(state: &AppState, guardian: &crate::guardian::Guardian) -> Option<Response> {
    if !guardian.is_staff() {
        return Some(super::topics::not_found_response(state));
    }
    (!guardian.is_admin()).then(|| super::search::invalid_access(state))
}

/// GET /admin/config/design-wizard(.json): DesignWizardSerializer.
pub async fn design_wizard(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    if !json_request(&headers, &uri) {
        return Err(Unsupported("the admin design wizard page (Ember)").into());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let themes_cx = crate::themes::Ctx {
        settings: &settings,
        i18n: &state.i18n,
        urls: &urls,
        defs: &state.site_setting_defs,
    };
    let cx = crate::design_wizard::Cx {
        themes: &themes_cx,
        defs: &state.site_setting_defs,
    };
    let body = crate::design_wizard::index(&mut conn, &cx).await?;
    Ok(Json(body).into_response())
}

/// PUT /admin/config/design-wizard(.json): DesignWizard::Apply.
pub async fn update_design_wizard(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    let p: Map<String, Value> = crate::params::parse(uri.query(), &headers, &body);
    let pairs: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !super::session::csrf_ok(&state, &headers, &pairs, uri.path(), "PUT") {
        return Ok(super::session::bad_csrf());
    }
    let actor = guardian
        .user_id()
        .ok_or(Unsupported("an admin without a user"))?;
    let params = crate::design_wizard::Params::from(&p);
    let failed = |status: StatusCode, errors: Vec<String>| {
        (status, Json(json!({"failed": "FAILED", "errors": errors}))).into_response()
    };
    let mut tx = state.pool.begin().await?;
    let theme = {
        let settings =
            SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
        let urls = crate::url::Urls {
            config: &state.config,
            settings: &settings,
        };
        let themes_cx = crate::themes::Ctx {
            settings: &settings,
            i18n: &state.i18n,
            urls: &urls,
            defs: &state.site_setting_defs,
        };
        let cx = crate::design_wizard::Cx {
            themes: &themes_cx,
            defs: &state.site_setting_defs,
        };
        match crate::design_wizard::apply_palettes(&mut tx, &cx, &params).await? {
            Ok(theme) => theme,
            Err(crate::design_wizard::Outcome::FailedContract(errors)) => {
                return Ok(failed(StatusCode::BAD_REQUEST, errors));
            }
            Err(crate::design_wizard::Outcome::ThemeNotFound) => {
                return Ok(super::topics::not_found_response(&state));
            }
            Err(crate::design_wizard::Outcome::PaletteUnavailable) => {
                let message = state
                    .i18n
                    .t("design_wizard.errors.palette_unavailable")
                    .unwrap_or_default()
                    .to_string();
                return Ok(failed(StatusCode::UNPROCESSABLE_ENTITY, vec![message]));
            }
            Err(crate::design_wizard::Outcome::Done) => {
                return Err(Unsupported("a design wizard that ended early").into());
            }
        }
    };
    tx.commit().await?;
    // The site settings, then the theme's themeable ones, after the
    // palettes' transaction.
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let themes_cx = crate::themes::Ctx {
        settings: &settings,
        i18n: &state.i18n,
        urls: &urls,
        defs: &state.site_setting_defs,
    };
    let cx = crate::design_wizard::Cx {
        themes: &themes_cx,
        defs: &state.site_setting_defs,
    };
    crate::design_wizard::apply_settings(&mut tx, &state.bus, &cx, &params, &theme, actor).await?;
    crate::design_wizard::apply_themeable_settings(&mut tx, &cx, &params, &theme, actor).await?;
    tx.commit().await?;
    Ok(Json(json!({"success": "OK"})).into_response())
}
