//! Admin::Config::DiscourseIdController: the Discourse ID tab of login and
//! authentication (templates/admin-config/login/discourse-id.gjs), its
//! JSON, the enabled switch's settings update, and regenerating the
//! credentials. Regenerating with credentials there asks Discourse ID for
//! a challenge (DiscourseId::Concerns::ChallengeFlow), which is not ported.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use super::config_branding::admin_only;
use super::pages::{Shell, wants_json};
use crate::admin::ui::Button;
use crate::admin::view::{AdminCx, t};
use crate::routes::session::{bad_csrf, csrf_ok};
use crate::session::current::AuthGuardian;
use crate::site_setting_update::{self, Outcome, discourse_id_configured};
use crate::site_settings::SiteSettings;
use crate::topic_list_view::{escape, icon};
use crate::{AppError, AppState, Unsupported, params};

/// `show`'s JSON: whether it is enabled and configured, and the Discourse
/// ID accounts, those made and those used in the last 30 days.
async fn model(conn: &mut sqlx::PgConnection, settings: &SiteSettings) -> Result<Value, AppError> {
    let (total, signups, logins): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*), \
                count(*) FILTER (WHERE created_at > now() - interval '30 days'), \
                count(*) FILTER (WHERE last_used > now() - interval '30 days') \
         FROM user_associated_accounts WHERE provider_name = 'discourse_id'",
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(json!({
        "enabled": settings.get("enable_discourse_id")?.truthy(),
        "configured": discourse_id_configured(settings)?,
        "stats": {
            "total_users": total,
            "signups_30_days": signups,
            "logins_30_days": logins,
        },
    }))
}

/// GET /admin/config/login-and-authentication/discourse-id(.json): the
/// JSON, or the tab with the login area's header over it.
pub(crate) async fn show(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    header: impl FnOnce(&AdminCx) -> String,
    title: impl FnOnce(&AdminCx) -> String,
    classes: &str,
) -> Result<Response, AppError> {
    if let Some(refused) = admin_only(state, guardian) {
        return Ok(refused);
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let model = model(&mut conn, &settings).await?;
    if wants_json(uri, headers) {
        return Ok(Json(model).into_response());
    }
    let shell = Shell::load(&mut conn, state, guardian, headers, "admin_login").await?;
    let cx = shell.cx(state);
    let header = header(&cx);
    let title = title(&cx);
    let content = format!(
        "{header}<div class=\"admin-config-page__main-area\">{}{}</div>",
        crate::admin::ui::strings(
            &cx,
            "discourse-id-strings",
            &[
                "admin.config.discourse_id.regenerate_confirm",
                "admin.config.discourse_id.regenerate_success",
            ],
        ),
        render(&cx, &model)
    );
    let main = crate::admin::view::contents(classes, &content);
    shell.render(
        state,
        uri,
        &title,
        main,
        "admin-discourse-id.js",
        StatusCode::OK,
    )
}

/// DiscourseIdAdmin
fn render(cx: &AdminCx, model: &Value) -> String {
    let enabled = model["enabled"].as_bool().unwrap_or(false);
    let configured = model["configured"].as_bool().unwrap_or(false);
    // DToggleSwitch, in the discourse-id-toggle outlet.
    let toggle = format!(
        "<div class=\"d-toggle-switch\"><label class=\"d-toggle-switch__label\"><button aria-checked=\"{enabled}\" class=\"d-toggle-switch__checkbox discourse-id-toggle\" role=\"switch\" type=\"button\"></button><span class=\"d-toggle-switch__checkbox-slider\">{}</span></label></div>",
        if enabled {
            icon("check", None)
        } else {
            String::new()
        }
    );
    let stat = |key: &str| {
        format!(
            "<div class=\"discourse-id-stats__item\"><span class=\"discourse-id-stats__value\">{}</span><span class=\"discourse-id-stats__label\">{}</span></div>",
            model["stats"][key],
            escape(&t(
                cx.i18n,
                &format!("admin.config.discourse_id.stats.{key}")
            ))
        )
    };
    let regenerate = Button {
        class: "btn-default btn-small discourse-id-regenerate",
        icon: None,
        label: Some(t(
            cx.i18n,
            "admin.config.discourse_id.regenerate_credentials",
        )),
        title: None,
        disabled: !configured,
        attrs: String::new(),
        href: None,
    }
    .render();
    format!(
        "<div class=\"admin-config-area discourse-id-admin\"><div class=\"admin-config-area__primary-content\"><section class=\"admin-config-area-card\"><div class=\"discourse-id-header\"><h3 class=\"admin-config-area-card__title\">{}</h3>{toggle}</div><p class=\"admin-config-area-card__description\">{} <a href=\"https://id.discourse.com\" rel=\"noopener noreferrer\" target=\"_blank\">{}</a></p><div class=\"admin-config-area-card__content\"><div class=\"discourse-id-stats\">{}{}{}</div><div class=\"discourse-id-footer\">{regenerate}</div></div></section></div></div>",
        escape(&t(cx.i18n, "admin.config.discourse_id.title")),
        escape(&t(cx.i18n, "admin.config.discourse_id.description")),
        escape(&t(cx.i18n, "admin.config.discourse_id.learn_more")),
        stat("total_users"),
        stat("signups_30_days"),
        stat("logins_30_days"),
    )
}

/// PUT .../discourse-id/settings(.json): `enabled`, when given, through
/// SiteSetting.set_and_log (EnableDiscourseIdValidator's InvalidParameters
/// as a 400).
pub async fn update_settings(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let form: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !csrf_ok(&state, &headers, &form, uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    if let Some(refused) = admin_only(&state, &guardian) {
        return Ok(refused);
    }
    if let Some(enabled) = p.get("enabled") {
        let raw = params::scalar(enabled).unwrap_or_default();
        let mut tx = state.pool.begin().await?;
        let settings =
            SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
        let outcome = site_setting_update::update_as_given(
            &mut tx,
            &state.bus,
            &state.config,
            &state.site_setting_defs,
            &settings,
            &state.i18n,
            &guardian,
            "enable_discourse_id",
            &raw,
        )
        .await?;
        if let Outcome::Invalid(message) = outcome {
            return Ok(crate::routes::search::invalid_parameters(&state, &message));
        }
        tx.commit().await?;
        super::site_settings::after_update(&state, &settings).await?;
    }
    Ok(Json(json!({ "success": "OK" })).into_response())
}

/// POST .../discourse-id/regenerate(.json): DiscourseId::RegenerateCredentials.
/// Without credentials its policy fails, a 422; with them it asks Discourse
/// ID for a challenge, which is not ported.
pub async fn regenerate(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let form: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !csrf_ok(&state, &headers, &form, uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    if let Some(refused) = admin_only(&state, &guardian) {
        return Ok(refused);
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !discourse_id_configured(&settings)? {
        return Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "failed": "FAILED",
                "error": state.i18n.t("discourse_id.errors.not_configured").unwrap_or_default(),
            })),
        )
            .into_response());
    }
    Err(Unsupported("regenerating Discourse ID credentials (its challenge flow)").into())
}
