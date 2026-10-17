//! Port of Admin::EmailStylesController: the HTML template and CSS every
//! outgoing email is rendered with (EmailStyle, EmailStyleUpdater). The
//! settings are written as `SiteSetting.x = value` writes them
//! (add_override! and remove_override!): no staff log, no publish, all
//! three being hidden settings.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::session::{bad_csrf, csrf_ok};
use crate::email::notification::DEFAULT_TEMPLATE;
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `EmailStyle.default_css`
const DEFAULT_CSS: &str = "";

/// EmailStyleSerializer, rooted `email_style`.
fn serialized(s: &SiteSettings) -> Result<Value, AppError> {
    Ok(json!({
        "email_style": {
            "id": "email-style",
            "html": crate::email::notification::email_template(s)?,
            "css": s.get("email_custom_css")?.to_s(),
            "default_html": DEFAULT_TEMPLATE,
            "default_css": DEFAULT_CSS,
        }
    }))
}

async fn load_settings(
    state: &AppState,
    conn: &mut PgConnection,
) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// GET /admin/customize/email_style(.json), and /email_style/html and
/// /email_style/css (the editor's tabs).
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    show_page(state, guardian, None, headers, uri).await
}

/// The JSON, or for a page request the editor of the field (the index
/// going on to html, as its route's replaceWith does).
async fn show_page(
    state: AppState,
    guardian: crate::guardian::Guardian,
    field: Option<&str>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let mut conn = state.pool.acquire().await?;
    let s = load_settings(&state, &mut conn).await?;
    let doc = serialized(&s)?;
    drop(conn);
    if !(super::wants_json(uri.query(), &headers) || uri.path().ends_with(".json")) {
        let Some(field) = field else {
            return Ok(axum::response::Redirect::to(&format!(
                "{}/admin/customize/email_style/html",
                state.config.globals.relative_url_root()
            ))
            .into_response());
        };
        return super::admin_customize::email_style_page(
            &state, &guardian, &headers, &uri, &doc, field,
        )
        .await;
    }
    Ok(axum::Json(doc).into_response())
}

/// GET /admin/customize/email_style/:field, `field` html or css (the
/// route's constraint; anything else has no route).
pub async fn show_field(
    state: State<AppState>,
    guardian: AuthGuardian,
    axum::extract::Path(field): axum::extract::Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let field = field.strip_suffix(".json").unwrap_or(&field);
    if !matches!(field, "html" | "css") {
        return Ok(super::topics::not_found_response(&state.0));
    }
    show_page(state.0, guardian.0, Some(field), headers, uri).await
}

/// `provider.save(name, value, type)`: the row updated, or created.
async fn save(conn: &mut PgConnection, name: &str, value: &str) -> Result<(), AppError> {
    let updated = sqlx::query(
        "UPDATE site_settings SET value = $2, data_type = 1, updated_at = clock_timestamp() WHERE name = $1",
    )
    .bind(name)
    .bind(value)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if updated == 0 {
        sqlx::query(
            "INSERT INTO site_settings (name, data_type, value, created_at, updated_at) \
             VALUES ($1, 1, $2, clock_timestamp(), clock_timestamp())",
        )
        .bind(name)
        .bind(value)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `remove_override!`: the row deleted.
async fn remove(conn: &mut PgConnection, name: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM site_settings WHERE name = $1")
        .bind(name)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// PUT /admin/customize/email_style(.json): EmailStyleUpdater#update with
/// `email_style[html]` and `email_style[css]`, either one optional.
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let pairs: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !csrf_ok(&state, &headers, &pairs, uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    // params.require(:email_style).permit(:html, :css)
    let attrs: Map<String, Value> = match p.get("email_style") {
        Some(Value::Object(m)) if !m.is_empty() => m.clone(),
        Some(Value::String(s)) if !crate::ruby::is_blank(s) => {
            return Err(Unsupported("email_style given as a string").into());
        }
        _ => return Ok(super::accounts::param_missing("email_style")),
    };
    let permitted = |key: &str| -> Result<Option<String>, AppError> {
        match attrs.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(Value::Null) => Err(Unsupported("a null email style field").into()),
            // permit drops what is not a scalar.
            Some(Value::Array(_) | Value::Object(_)) => Ok(None),
            Some(other) => Ok(params::scalar(other)),
        }
    };
    let html = permitted("html")?;
    let css = permitted("css")?;

    let mut errors: Vec<String> = Vec::new();
    if let Some(html) = &html
        && !html.contains("%{email_content}")
    {
        errors.push(
            state
                .i18n
                .t_with(
                    "email_style.html_missing_placeholder",
                    &[("placeholder", "%{email_content}")],
                )
                .unwrap_or_default(),
        );
    }
    let mut compiled = None;
    if let Some(css) = &css {
        match crate::email::custom_css::compile(css)? {
            Ok(out) => compiled = Some(out),
            Err(message) => errors.push(message),
        }
    }
    if !errors.is_empty() {
        return Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            axum::Json(json!({ "errors": errors })),
        )
            .into_response());
    }

    let mut tx = state.pool.begin().await?;
    if let Some(html) = &html {
        if html == DEFAULT_TEMPLATE {
            remove(&mut tx, "email_custom_template").await?;
        } else {
            save(&mut tx, "email_custom_template", html).await?;
        }
    }
    if let Some(css) = &css {
        if css == DEFAULT_CSS {
            remove(&mut tx, "email_custom_css").await?;
            remove(&mut tx, "email_custom_css_compiled").await?;
        } else {
            save(&mut tx, "email_custom_css", css).await?;
            save(
                &mut tx,
                "email_custom_css_compiled",
                compiled.as_deref().unwrap_or_default(),
            )
            .await?;
        }
    }
    let s = load_settings(&state, &mut tx).await?;
    let out = serialized(&s)?;
    tx.commit().await?;
    Ok(axum::Json(out).into_response())
}
