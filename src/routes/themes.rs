//! Admin::ThemesController (index, show, create, update, destroy,
//! bulk_destroy, update_single_setting, get_translations, preview,
//! import, export, upload_asset, generate_key_pair, update_source), the
//! /admin/config customize, color palettes and welcome banner reads, and
//! the theme site setting write (Themes::ThemeSiteSettingManager). JSON
//! only. The saves themselves are crate::theme_writes, bundles
//! crate::theme_import.
//!
//! Refused: what clones a git repository (remote imports, update checks
//! and updates of git themes, source changes: network fetches), other
//! locales' translations.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{FromRequest, Multipart, Path, Request, State};
use axum::http::{HeaderMap, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::color_scheme::ColorScheme;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::theme_writes;
use crate::themes::{self, Theme};
use crate::{AppError, AppState, Unsupported};

fn json_request(headers: &HeaderMap, uri: &Uri) -> bool {
    uri.path().ends_with(".json")
        || headers
            .get("x-requested-with")
            .is_some_and(|v| v == "XMLHttpRequest")
        || super::wants_json(uri.query(), headers)
}

fn refuse_non_admin(state: &AppState, guardian: &crate::guardian::Guardian) -> Option<Response> {
    (!guardian.is_admin()).then(|| super::topics::not_found_response(state))
}

fn id_of(raw: &str) -> i32 {
    crate::ruby::to_i(raw.strip_suffix(".json").unwrap_or(raw)) as i32
}

/// Runs `f` with the serializers' context.
macro_rules! with_ctx {
    ($state:expr, $conn:expr, |$cx:ident| $body:expr) => {{
        let settings = SiteSettings::load(
            &mut *$conn,
            &$state.site_setting_defs,
            &$state.config.globals,
        )
        .await?;
        let urls = crate::url::Urls {
            config: &$state.config,
            settings: &settings,
        };
        let $cx = themes::Ctx {
            settings: &settings,
            i18n: &$state.i18n,
            urls: &urls,
            defs: &$state.site_setting_defs,
        };
        $body
    }};
}

/// GET /admin/themes(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    if !json_request(&headers, &uri) {
        return Err(Unsupported("the admin themes page").into());
    }
    let mut conn = state.pool.acquire().await?;
    let conn = &mut *conn;
    with_ctx!(state, conn, |cx| {
        let mut list = Vec::new();
        for t in Theme::where_ordered(conn, "TRUE").await? {
            list.push(themes::serialize(conn, &cx, &t, false).await?);
        }
        // ColorScheme.all, in the table's order.
        let ids: Vec<i32> =
            sqlx::query_scalar("SELECT id FROM color_schemes WHERE NOT remote_copy")
                .fetch_all(&mut *conn)
                .await?;
        let mut schemes = Vec::new();
        for id in ids {
            if let Some(s) = ColorScheme::find(conn, id).await? {
                schemes.push(s.serialize()?);
            }
        }
        // current_user.effective_locale
        let locale = cx.settings.get("default_locale")?.to_s();
        Ok(Json(json!({
            "themes": list,
            "extras": {"color_schemes": schemes, "locale": locale},
        }))
        .into_response())
    })
}

/// GET /admin/themes/:id(.json)
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    let mut conn = state.pool.acquire().await?;
    let conn = &mut *conn;
    let Some(t) = Theme::find(conn, id_of(&raw)).await? else {
        return Ok(super::search::invalid_parameters(&state, "id"));
    };
    with_ctx!(state, conn, |cx| {
        let body = themes::serialize(conn, &cx, &t, false).await?;
        Ok(Json(json!({ "theme": body })).into_response())
    })
}

/// GET /admin/config/customize/themes(.json): ThemeIndexSerializer.
pub async fn config_themes(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    if !json_request(&headers, &uri) {
        return Err(Unsupported("the admin themes page").into());
    }
    let mut conn = state.pool.acquire().await?;
    let conn = &mut *conn;
    with_ctx!(state, conn, |cx| {
        let mut list = Vec::new();
        for t in Theme::where_ordered(conn, "NOT component").await? {
            list.push(themes::index_json(conn, &cx, &t).await?);
        }
        Ok(Json(json!({ "themes": list })).into_response())
    })
}

/// GET /admin/config/customize/components(.json): twenty a page, by name,
/// `name` and `status` filters.
pub async fn config_components(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    const PAGE_SIZE: i64 = 20;
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    if !json_request(&headers, &uri) {
        return Err(Unsupported("the admin components page").into());
    }
    let query = crate::params::parse_query(uri.query().unwrap_or(""));
    let page = crate::params::string(&query, "page").map(|p| crate::ruby::to_i(&p));
    let mut condition = "component".to_string();
    if let Some(name) = crate::params::string(&query, "name")
        .map(|n| crate::ruby::strip(&n).to_string())
        .filter(|n| !n.is_empty())
    {
        // `ILIKE '%name%'` with the term quoted as a literal.
        condition.push_str(&format!(
            " AND name ILIKE '%' || {} || '%'",
            sql_literal(&name)
        ));
    }
    match crate::params::string(&query, "status")
        .filter(|s| !s.is_empty())
        .as_deref()
    {
        None | Some("all") => {}
        Some("used") => {
            condition.push_str(" AND id IN (SELECT child_theme_id FROM child_themes)");
        }
        Some("unused") => {
            condition.push_str(" AND id NOT IN (SELECT child_theme_id FROM child_themes)");
        }
        Some("updates_available") => {
            condition.push_str(
                " AND remote_theme_id IN (SELECT id FROM remote_themes WHERE commits_behind >= 1)",
            );
        }
        Some(_) => {
            return Ok(super::search::invalid_parameters(
                &state,
                "Discourse::InvalidParameters",
            ));
        }
    }
    let offset = page.filter(|p| *p > 0).map_or(0, |p| p * PAGE_SIZE);
    let mut conn = state.pool.acquire().await?;
    let conn = &mut *conn;
    with_ctx!(state, conn, |cx| {
        let mut list = Theme::select(
            conn,
            &format!(
                "WHERE {condition} ORDER BY name LIMIT {} OFFSET {offset}",
                PAGE_SIZE + 1
            ),
        )
        .await?;
        let has_more = list.len() as i64 > PAGE_SIZE;
        list.truncate(PAGE_SIZE as usize);
        let mut out = Vec::new();
        for t in &list {
            out.push(themes::component_json(conn, &cx, t).await?);
        }
        Ok(Json(json!({ "has_more": has_more, "components": out })).into_response())
    })
}

/// A string as a SQL literal.
fn sql_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''").replace('\\', "\\\\"))
}

/// GET /admin/config/customize/theme-site-settings(.json)
pub async fn config_theme_site_settings(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    if !json_request(&headers, &uri) {
        return Err(Unsupported("the admin theme site settings page").into());
    }
    let mut conn = state.pool.acquire().await?;
    let conn = &mut *conn;
    with_ctx!(state, conn, |cx| {
        let mut by_setting = Map::new();
        for name in themes::THEMEABLE_SITE_SETTINGS {
            let mut e = themes::site_setting_metadata(&cx, name)?;
            e.insert("themes".into(), json!([]));
            by_setting.insert(name.to_string(), Value::Object(e));
        }
        // ThemeSiteSetting.themes_with_overridden_settings
        let rows = overridden_settings(conn, &cx).await?;
        for (name, theme_id, theme_name, value) in rows {
            if let Some(Value::Array(list)) =
                by_setting.get_mut(&name).and_then(|e| e.get_mut("themes"))
            {
                list.push(json!({"theme_id": theme_id, "theme_name": theme_name, "value": value}));
            }
        }
        Ok(Json(json!({
            "themeable_site_settings": themes::THEMEABLE_SITE_SETTINGS,
            "themes_with_site_setting_overrides": by_setting,
        }))
        .into_response())
    })
}

/// `ThemeSiteSetting.themes_with_overridden_settings`: the rows whose value
/// differs from the setting's default, by theme name.
async fn overridden_settings(
    conn: &mut sqlx::PgConnection,
    cx: &themes::Ctx<'_>,
) -> Result<Vec<(String, i32, String, Value)>, AppError> {
    let rows: Vec<(String, i32, String, i32, Option<String>)> = sqlx::query_as(
        "SELECT tss.name, t.id, t.name, tss.data_type, tss.value FROM theme_site_settings tss \
         JOIN themes t ON t.id = tss.theme_id WHERE NOT t.component ORDER BY tss.name, t.name",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for (name, theme_id, theme_name, data_type, value) in rows {
        if !themes::THEMEABLE_SITE_SETTINGS.contains(&name.as_str()) {
            continue;
        }
        let default = themes::site_setting_metadata(cx, &name)?["default"].clone();
        let value = match crate::site_settings::DataType::from_id(data_type) {
            Some(crate::site_settings::DataType::Bool) => {
                json!(matches!(value.as_deref(), Some("t") | Some("true")))
            }
            _ => json!(value),
        };
        if value != default {
            out.push((name, theme_id, theme_name, value));
        }
    }
    Ok(out)
}

/// GET /admin/config/welcome-banner/themes-with-setting(.json)
pub async fn welcome_banner_themes(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    // Admin::Config behind StaffConstraint: InvalidAccess for a moderator.
    if let Some(r) = super::admin_config_branding::admin_only(&state, &guardian) {
        return Ok(r);
    }
    let mut conn = state.pool.acquire().await?;
    let out = welcome_banner_theme_list(&state, &mut conn).await?;
    Ok(Json(json!({ "themes": out })).into_response())
}

/// The default theme and the user selectable ones, each with its
/// enable_welcome_banner.
pub(super) async fn welcome_banner_theme_list(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
) -> Result<Vec<serde_json::Value>, AppError> {
    with_ctx!(state, conn, |cx| {
        let default_id = cx.settings.get("default_theme_id")?.to_i();
        // Theme.not_components.where(...): the table's order.
        let ids: Vec<(i32, String)> = sqlx::query_as(
            "SELECT id, name FROM themes WHERE NOT component AND (id = $1 OR user_selectable)",
        )
        .bind(default_id as i32)
        .fetch_all(&mut *conn)
        .await?;
        let mut out = Vec::new();
        for (id, name) in ids {
            let value = themes::welcome_banner_value(conn, &cx, id).await?;
            out.push(json!({"id": id, "name": name, "enable_welcome_banner": value}));
        }
        Ok(out)
    })
}

/// GET /admin/config/colors(.json): every palette with the base one,
/// sorted active first, then the other built-in palettes before them.
pub async fn config_colors(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    if !json_request(&headers, &uri) {
        return Err(Unsupported("the admin color palettes page").into());
    }
    let mut conn = state.pool.acquire().await?;
    let conn = &mut *conn;
    with_ctx!(state, conn, |cx| {
        let default_id = cx.settings.get("default_theme_id")?.to_i() as i32;
        let default_theme = Theme::find(conn, default_id).await?;
        let light = default_theme.as_ref().and_then(|t| t.color_scheme_id);
        let dark = default_theme.as_ref().and_then(|t| t.dark_color_scheme_id);
        let default_theme_id = default_theme.as_ref().map(|t| t.id);

        let ids: Vec<i32> =
            sqlx::query_scalar("SELECT id FROM color_schemes WHERE NOT remote_copy")
                .fetch_all(&mut *conn)
                .await?;
        let mut palettes = Vec::new();
        // ColorScheme.base: the Light palette, named from the admin locale.
        let bases = ColorScheme::base_color_schemes(&state.i18n);
        let mut base = bases
            .iter()
            .find(|s| s.is_builtin_default == Some(true))
            .cloned()
            .ok_or(Unsupported("the base color scheme"))?;
        let key = "admin_js.admin.customize.theme.default_light_scheme";
        base.name = state
            .i18n
            .override_of(key)
            .or_else(|| crate::site_texts::tree().get(key))
            .unwrap_or("Light (default)")
            .to_string();
        // `ColorScheme.base` is made without a base scheme id.
        base.base_scheme_id = None;
        palettes.push(base);
        for id in ids {
            if let Some(s) = ColorScheme::find(conn, id).await? {
                palettes.push(s);
            }
        }
        let builtin = |s: &ColorScheme| s.is_builtin_default == Some(true);
        crate::ruby::sort_by_block(&mut palettes, &mut |a, b| {
            use std::cmp::Ordering::{Greater, Less};
            if light.is_none() && (builtin(a) || builtin(b)) {
                return if builtin(a) { Less } else { Greater };
            }
            let is = |id: Option<i32>, s: &ColorScheme| id == Some(s.id) && !builtin(s);
            if is(light, a) || is(light, b) {
                return if light == Some(a.id) { Less } else { Greater };
            }
            if dark.is_none() && (builtin(a) || builtin(b)) {
                return if builtin(a) { Less } else { Greater };
            }
            if is(dark, a) || is(dark, b) {
                return if dark == Some(a.id) { Less } else { Greater };
            }
            if a.user_selectable != b.user_selectable {
                return if a.user_selectable { Less } else { Greater };
            }
            let custom = |s: &ColorScheme| s.theme_id.is_none() && !builtin(s);
            if custom(a) != custom(b) {
                return if custom(a) { Less } else { Greater };
            }
            let from_default =
                |s: &ColorScheme| s.theme_id.is_some() && s.theme_id == default_theme_id;
            if from_default(a) != from_default(b) {
                return if from_default(a) { Less } else { Greater };
            }
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        });
        let mut out: Vec<Value> = Vec::new();
        for s in bases.iter().filter(|s| s.is_builtin_default != Some(true)) {
            out.push(s.serialize()?);
        }
        for s in &palettes {
            out.push(s.serialize()?);
        }
        let default_theme_json = match &default_theme {
            Some(t) => themes::serialize(conn, &cx, t, false).await?,
            None => Value::Null,
        };
        Ok(Json(json!({
            "palettes": out,
            "extras": {"default_theme": default_theme_json},
        }))
        .into_response())
    })
}

/// PUT /admin/themes/:id/site-setting(.json): Themes::ThemeSiteSettingManager.
pub async fn update_site_setting(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    let pairs: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !super::session::csrf_ok(&state, &headers, &pairs, uri.path(), "PUT") {
        return Ok(super::session::bad_csrf());
    }
    let acting = guardian
        .user_id()
        .ok_or(Unsupported("an admin without a user"))?;
    let name = p
        .get("name")
        .and_then(crate::params::scalar)
        .filter(|n| !crate::ruby::is_blank(n));
    let Some(name) = name else {
        return Err(Unsupported("a theme site setting without a name (contract)").into());
    };
    if !themes::THEMEABLE_SITE_SETTINGS.contains(&name.as_str()) {
        let message = state
            .i18n
            .t_with("themes.setting_not_themeable", &[("name", &name)])
            .unwrap_or_default();
        return Ok((
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({ "errors": [message] })),
        )
            .into_response());
    }
    let mut tx = state.pool.begin().await?;
    let Some(theme) = Theme::find(&mut tx, id_of(&raw)).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let raw_value = p.get("value").and_then(crate::params::scalar);
    let stored = match theme_writes::set_theme_site_setting(
        &mut tx,
        &state.site_setting_defs,
        &theme,
        &name,
        raw_value,
        acting,
    )
    .await?
    {
        Ok(stored) => stored,
        Err(message) => return Ok(super::search::invalid_parameters(&state, &message)),
    };
    tx.commit().await?;
    Ok(Json(json!({
        "success": "OK",
        "name": name,
        "value": stored,
        "theme_id": theme.id,
    }))
    .into_response())
}

/// The serializers' context over freshly loaded settings (a theme write
/// may have just changed `default_theme_id`).
macro_rules! fresh_ctx {
    ($state:expr, $conn:expr, $settings:ident, $urls:ident, $cx:ident) => {
        let $settings = SiteSettings::load(
            &mut *$conn,
            &$state.site_setting_defs,
            &$state.config.globals,
        )
        .await?;
        let $urls = crate::url::Urls {
            config: &$state.config,
            settings: &$settings,
        };
        let $cx = themes::Ctx {
            settings: &$settings,
            i18n: &$state.i18n,
            urls: &$urls,
            defs: &$state.site_setting_defs,
        };
    };
}

/// ActiveModel's boolean cast: blank is nil, then "false", "f", "0",
/// "off" (any case) are false and everything else true.
fn cast_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Null => None,
        Value::Number(n) => Some(n.as_f64() != Some(0.0)),
        other => {
            let s = crate::params::scalar(other)?;
            if s.is_empty() {
                return None;
            }
            Some(!matches!(
                s.to_lowercase().as_str(),
                "false" | "f" | "0" | "off"
            ))
        }
    }
}

/// ActiveModel's integer cast: blank is nil, else `to_i`.
fn cast_int(v: &Value) -> Option<i32> {
    match v {
        Value::Number(n) => n.as_i64().map(|i| i as i32),
        other => crate::params::scalar(other)
            .filter(|s| !crate::ruby::is_blank(s))
            .map(|s| crate::ruby::to_i(&s) as i32),
    }
}

/// Parses the request's params and checks its CSRF token.
fn write_params(
    state: &AppState,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
    method: &str,
) -> Result<Map<String, Value>, Box<Response>> {
    let p = crate::params::parse(uri.query(), headers, body);
    let pairs: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !super::session::csrf_ok(state, headers, &pairs, uri.path(), method) {
        return Err(Box::new(super::session::bad_csrf()));
    }
    Ok(p)
}

/// The service contract's 400: `{failed, errors}`.
fn failed_contract(errors: Vec<String>) -> Response {
    (
        axum::http::StatusCode::BAD_REQUEST,
        Json(json!({"failed": "FAILED", "errors": errors})),
    )
        .into_response()
}

/// `render_json_error(message)`: 422 `{errors: [message]}`.
fn json_error(message: &str) -> Response {
    (
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "errors": [message] })),
    )
        .into_response()
}

fn tr(state: &AppState, key: &str) -> String {
    state
        .i18n
        .t(key)
        .map(str::to_string)
        .unwrap_or_else(|| format!("Translation missing: en.{key}"))
}

/// `Theme#component_validations`
fn component_errors(state: &AppState, t: &Theme, default: bool) -> Vec<String> {
    let mut errors = Vec::new();
    if t.component {
        if t.color_scheme_id.is_some() {
            errors.push(tr(state, "themes.errors.component_no_color_scheme"));
        }
        if t.user_selectable {
            errors.push(tr(state, "themes.errors.component_no_user_selectable"));
        }
        if default {
            errors.push(tr(state, "themes.errors.component_no_default"));
        }
    }
    errors
}

/// `SiteSetting.default_theme_id = id` (Theme#set_default!,
/// Theme.clear_default!).
async fn set_default_theme(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    id: i32,
) -> Result<(), AppError> {
    let settings =
        SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
    crate::site_setting_update::set(
        conn,
        &state.bus,
        &settings,
        &state.site_setting_defs,
        &state.i18n,
        "default_theme_id",
        &json!(id),
    )
    .await?
    .map_err(|_| Unsupported("an invalid default_theme_id"))?;
    Ok(())
}

async fn is_default(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    id: i32,
) -> Result<bool, AppError> {
    let settings =
        SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
    Ok(settings.get("default_theme_id")?.to_i() == i64::from(id))
}

/// The theme's fields from a `theme_fields` param, set on the theme.
async fn set_fields(
    conn: &mut sqlx::PgConnection,
    theme_id: i32,
    fields: Option<&Value>,
    changes: &mut Vec<theme_writes::FieldChange>,
) -> Result<(), AppError> {
    let list: Vec<Value> = match fields {
        None | Some(Value::Null) => return Ok(()),
        Some(Value::Array(items)) => items.clone(),
        // A form's `theme_fields[0][name]` arrives as a hash by index.
        Some(Value::Object(m)) => m.values().cloned().collect(),
        Some(_) => return Err(Unsupported("theme fields that are not a list").into()),
    };
    for item in &list {
        let input = theme_writes::FieldInput::from_param(item);
        theme_writes::set_field(conn, theme_id, &input, changes).await?;
    }
    Ok(())
}

/// Writes the theme row's columns that changed (updated_at with them).
async fn update_theme_row(
    conn: &mut sqlx::PgConnection,
    before: &Theme,
    after: &Theme,
) -> Result<(), sqlx::Error> {
    if before.name == after.name
        && before.color_scheme_id == after.color_scheme_id
        && before.dark_color_scheme_id == after.dark_color_scheme_id
        && before.user_selectable == after.user_selectable
        && before.enabled == after.enabled
        && before.auto_update == after.auto_update
        && before.component == after.component
    {
        return Ok(());
    }
    sqlx::query(
        "UPDATE themes SET name = $2, color_scheme_id = $3, dark_color_scheme_id = $4, \
         user_selectable = $5, enabled = $6, auto_update = $7, component = $8, updated_at = now() \
         WHERE id = $1",
    )
    .bind(after.id)
    .bind(&after.name)
    .bind(after.color_scheme_id)
    .bind(after.dark_color_scheme_id)
    .bind(after.user_selectable)
    .bind(after.enabled)
    .bind(after.auto_update)
    .bind(after.component)
    .execute(conn)
    .await?;
    Ok(())
}

/// POST /admin/themes(.json): Themes::Create.
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    let p = match write_params(&state, &headers, &uri, &body, "POST") {
        Ok(p) => p,
        Err(r) => return Ok(*r),
    };
    let acting = guardian
        .user_id()
        .ok_or(Unsupported("an admin without a user"))?;
    let theme_param = match p.get("theme") {
        Some(Value::Object(m)) => m.clone(),
        _ => {
            return Err(Unsupported("a theme create without its params (ParameterMissing)").into());
        }
    };
    let get = |k: &str| theme_param.get(k).cloned().unwrap_or(Value::Null);
    let name = crate::params::scalar(&get("name")).unwrap_or_default();
    let fields = theme_param.get("theme_fields");
    let field_count = match fields {
        Some(Value::Array(a)) => a.len(),
        Some(Value::Object(m)) => m.len(),
        _ => 0,
    };
    let mut contract = Vec::new();
    if crate::ruby::is_blank(&name) {
        contract.push("Name can't be blank".to_string());
    }
    if field_count > 100 {
        contract.push("Theme fields is too long (maximum is 100 characters)".to_string());
    }
    if !contract.is_empty() {
        return Ok(failed_contract(contract));
    }
    let now = chrono::Utc::now().naive_utc();
    let mut new = Theme {
        id: 0,
        name,
        user_id: acting,
        created_at: now,
        updated_at: now,
        component: cast_bool(&get("component")).unwrap_or(false),
        color_scheme_id: cast_int(&get("color_scheme_id")),
        dark_color_scheme_id: None,
        user_selectable: cast_bool(&get("user_selectable")).unwrap_or(false),
        auto_update: true,
        remote_theme_id: None,
        enabled: true,
    };
    let make_default = cast_bool(&get("default")).unwrap_or(false);
    // Theme.create's validations: an invalid theme is not saved, and the
    // service has no handler that renders it.
    if !component_errors(&state, &new, false).is_empty() {
        return Ok(axum::http::StatusCode::NO_CONTENT.into_response());
    }
    let mut tx = state.pool.begin().await?;
    // The fields are set on the unsaved theme (no row has its id yet).
    let mut changes = Vec::new();
    set_fields(&mut tx, i32::MIN, fields, &mut changes).await?;
    if !theme_writes::field_errors(&changes).is_empty() {
        return Ok(axum::http::StatusCode::NO_CONTENT.into_response());
    }
    let (id, created_at, updated_at): (i32, chrono::NaiveDateTime, chrono::NaiveDateTime) =
        sqlx::query_as(
            "INSERT INTO themes (name, user_id, user_selectable, color_scheme_id, component, \
             created_at, updated_at) VALUES ($1, $2, $3, $4, $5, now(), now()) \
             RETURNING id, created_at, updated_at",
        )
        .bind(&new.name)
        .bind(new.user_id)
        .bind(new.user_selectable)
        .bind(new.color_scheme_id)
        .bind(new.component)
        .fetch_one(&mut *tx)
        .await?;
    new.id = id;
    new.created_at = created_at;
    new.updated_at = updated_at;
    {
        fresh_ctx!(state, tx, settings, urls, cx);
        let env = theme_writes::Env::load(&mut tx, &cx).await?;
        theme_writes::after_save(&mut tx, &env, id, &changes, false).await?;
    }
    if make_default {
        if new.component {
            let message = tr(&state, "themes.errors.component_no_default");
            return Ok(super::search::invalid_parameters(&state, &message));
        }
        set_default_theme(&mut tx, &state, id).await?;
    }
    let theme = Theme::find(&mut tx, id)
        .await?
        .ok_or(Unsupported("a theme gone after its create"))?;
    fresh_ctx!(state, tx, settings, urls, cx);
    theme_writes::log_theme_change(&mut tx, &cx, acting, None, &theme).await?;
    let body = themes::serialize(&mut tx, &cx, &theme, false).await?;
    tx.commit().await?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({ "theme": body })),
    )
        .into_response())
}

/// `Theme::EDITABLE_SYSTEM_ATTRIBUTES`
const EDITABLE_SYSTEM_ATTRIBUTES: [&str; 8] = [
    "child_theme_ids",
    "color_scheme_id",
    "dark_color_scheme_id",
    "default",
    "locale",
    "translations",
    "user_selectable",
    "updated_at",
];

/// `theme_params`' permitted keys.
const PERMITTED: [&str; 14] = [
    "name",
    "color_scheme_id",
    "dark_color_scheme_id",
    "default",
    "user_selectable",
    "component",
    "enabled",
    "auto_update",
    "locale",
    "settings",
    "translations",
    "theme_fields",
    "child_theme_ids",
    "parent_theme_ids",
];

/// `child_theme_ids=` / `parent_theme_ids=`: the join rows replaced, the
/// new ones validated (no component parents, no theme children).
async fn replace_relations(
    conn: &mut sqlx::PgConnection,
    theme_id: i32,
    ids: &[i32],
    children: bool,
) -> Result<(), AppError> {
    let (mine, theirs) = if children {
        ("parent_theme_id", "child_theme_id")
    } else {
        ("child_theme_id", "parent_theme_id")
    };
    for id in ids {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM themes WHERE id = $1)")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
        if !exists {
            return Err(Unsupported("child themes that do not exist (RecordNotFound)").into());
        }
    }
    sqlx::query(&format!(
        "DELETE FROM child_themes WHERE {mine} = $1 AND NOT ({theirs} = ANY($2))"
    ))
    .bind(theme_id)
    .bind(ids)
    .execute(&mut *conn)
    .await?;
    let mut seen = Vec::new();
    for id in ids {
        if seen.contains(id) {
            continue;
        }
        seen.push(*id);
        let present: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS (SELECT 1 FROM child_themes WHERE {mine} = $1 AND {theirs} = $2)"
        ))
        .bind(theme_id)
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
        if present {
            continue;
        }
        let (parent, child) = if children {
            (theme_id, *id)
        } else {
            (*id, theme_id)
        };
        let invalid: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM themes WHERE (component IS TRUE AND id = $1) \
             OR (component IS FALSE AND id = $2))",
        )
        .bind(parent)
        .bind(child)
        .fetch_one(&mut *conn)
        .await?;
        if invalid {
            return Err(Unsupported("child themes that are not components (RecordInvalid)").into());
        }
        sqlx::query(
            "INSERT INTO child_themes (parent_theme_id, child_theme_id, created_at, updated_at) \
             VALUES ($1, $2, now(), now())",
        )
        .bind(parent)
        .bind(child)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

fn id_list(v: &Value) -> Vec<i32> {
    match v {
        Value::Array(items) => items.iter().filter_map(cast_int).collect(),
        Value::Object(m) => m.values().filter_map(cast_int).collect(),
        other => cast_int(other).into_iter().collect(),
    }
}

/// PUT /admin/themes/:id(.json): Admin::ThemesController#update.
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    let p = match write_params(&state, &headers, &uri, &body, "PUT") {
        Ok(p) => p,
        Err(r) => return Ok(*r),
    };
    let acting = guardian
        .user_id()
        .ok_or(Unsupported("an admin without a user"))?;
    let mut tx = state.pool.begin().await?;
    let Some(before) = Theme::find(&mut tx, id_of(&raw)).await? else {
        return Ok(super::search::invalid_parameters(&state, "id"));
    };
    let original_json = {
        fresh_ctx!(state, tx, settings, urls, cx);
        let v = themes::serialize(&mut tx, &cx, &before, false).await?;
        crate::web_hooks::to_json(&v)
    };
    let theme_param = match p.get("theme") {
        Some(Value::Object(m)) => m.clone(),
        _ => {
            return Err(Unsupported("a theme update without its params (ParameterMissing)").into());
        }
    };
    let keys: Vec<&str> = theme_param
        .keys()
        .map(String::as_str)
        .filter(|k| PERMITTED.contains(k))
        .collect();
    let get = |k: &str| theme_param.get(k).cloned().unwrap_or(Value::Null);
    let enabled_param = theme_param.get("enabled");
    let disables = enabled_param.is_some_and(|v| *v == json!(false) || *v == json!("false"));
    let enables = enabled_param.is_some_and(|v| *v == json!(true) || *v == json!("true"));
    if before.system() && keys.iter().any(|k| !EDITABLE_SYSTEM_ATTRIBUTES.contains(k)) {
        return Ok(super::search::invalid_access(&state));
    }
    // params[:theme][:remote_check] / [:remote_update]: truthy values.
    let truthy = |k: &str| {
        theme_param
            .get(k)
            .is_some_and(|v| !matches!(v, Value::Null | Value::Bool(false)))
    };
    let (remote_check, remote_update) = (truthy("remote_check"), truthy("remote_update"));
    let mut after = before.clone();
    if keys.contains(&"name") {
        after.name = crate::params::scalar(&get("name")).unwrap_or_default();
    }
    if keys.contains(&"color_scheme_id") {
        after.color_scheme_id = cast_int(&get("color_scheme_id"));
    }
    if keys.contains(&"dark_color_scheme_id") {
        after.dark_color_scheme_id = cast_int(&get("dark_color_scheme_id"));
    }
    if keys.contains(&"user_selectable") {
        after.user_selectable = cast_bool(&get("user_selectable")).unwrap_or(false);
    }
    if keys.contains(&"enabled") {
        after.enabled = cast_bool(&get("enabled")).unwrap_or(false);
    }
    if keys.contains(&"auto_update") {
        after.auto_update = cast_bool(&get("auto_update")).unwrap_or(false);
    }
    if keys.contains(&"child_theme_ids") {
        replace_relations(&mut tx, before.id, &id_list(&get("child_theme_ids")), true).await?;
    }
    if keys.contains(&"parent_theme_ids") {
        replace_relations(
            &mut tx,
            before.id,
            &id_list(&get("parent_theme_ids")),
            false,
        )
        .await?;
    }
    // set_fields: kept for the save, refused in allowlist mode and for a
    // git theme.
    let mut changes = Vec::new();
    if keys.contains(&"theme_fields") {
        let remote = themes::RemoteTheme::find(&mut tx, before.remote_theme_id).await?;
        if allowlist_mode(&state) || remote.is_some_and(|r| r.is_git()) {
            tx.commit().await?;
            return Ok(super::search::invalid_access(&state));
        }
        set_fields(
            &mut tx,
            before.id,
            theme_param.get("theme_fields"),
            &mut changes,
        )
        .await?;
    }
    // update_settings: written at once, as ThemeSetting saves its rows.
    let mut refresh = false;
    if let Some(Value::Object(settings)) = theme_param.get("settings") {
        for (name, value) in settings {
            match update_one_setting(&mut tx, &state, before.id, name, Some(value)).await? {
                Ok(r) => refresh |= r,
                Err(theme_writes::SettingError::NotFound) => {
                    tx.commit().await?;
                    return Ok(super::topics::not_found_response(&state));
                }
                Err(theme_writes::SettingError::Invalid(message)) => {
                    tx.commit().await?;
                    return Ok(super::search::invalid_parameters(&state, &message));
                }
            }
        }
    }
    // update_translations
    if let Some(Value::Object(translations)) = theme_param.get("translations") {
        let locale = crate::params::scalar(&get("locale")).filter(|l| !crate::ruby::is_blank(l));
        if locale.as_deref().is_some_and(|l| l != "en") {
            return Err(Unsupported("theme translations in other locales").into());
        }
        for (key, value) in translations {
            let value = crate::params::scalar(value).unwrap_or_default();
            if !theme_writes::update_translation(&mut tx, before.id, key, &value).await? {
                tx.commit().await?;
                return Ok(super::topics::not_found_response(&state));
            }
        }
    }
    // handle_switch
    let component_param = crate::params::scalar(&get("component"));
    let switch_to_theme = component_param.as_deref() == Some("false") && before.component;
    let switch_to_component = component_param.as_deref() == Some("true") && !before.component;
    if (switch_to_theme || switch_to_component) && is_default(&mut tx, &state, before.id).await? {
        tx.commit().await?;
        return Ok(super::search::invalid_parameters(&state, "component"));
    }
    if switch_to_component {
        after.component = true;
        after.color_scheme_id = None;
        after.user_selectable = false;
        sqlx::query("DELETE FROM child_themes WHERE parent_theme_id = $1")
            .bind(before.id)
            .execute(&mut *tx)
            .await?;
    }
    if switch_to_theme {
        after.enabled = true;
        after.component = false;
        sqlx::query("DELETE FROM child_themes WHERE child_theme_id = $1")
            .bind(before.id)
            .execute(&mut *tx)
            .await?;
    }
    // remote_check: RemoteTheme#update_remote_version, a no-op for a theme
    // that isn't git; remote_update: update_from_remote, which fails on the
    // empty url of one that isn't (its error kept on the remote theme).
    if remote_check || remote_update {
        let Some(remote) = themes::RemoteTheme::find(&mut tx, before.remote_theme_id).await? else {
            return Err(std::io::Error::other("NoMethodError: remote_theme is nil").into());
        };
        if remote.is_git() {
            return Err(Unsupported("checking or updating a git theme (a network fetch)").into());
        }
        if remote_update {
            let error = crate::theme_import::git_import(&state.i18n, &remote.remote_url, None)?;
            sqlx::query(
                "UPDATE remote_themes SET last_error_text = $2, updated_at = now() WHERE id = $1",
            )
            .bind(remote.id)
            .bind(&error.0)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(json_error(&error.0));
        }
    }
    // @theme.save: the validations, then the row and after_save.
    let default_now = is_default(&mut tx, &state, before.id).await?;
    let mut errors = component_errors(&state, &after, default_now);
    errors.extend(theme_writes::field_errors(&changes));
    if !errors.is_empty() {
        tx.commit().await?;
        return Ok(json_error(&errors.join(", ")));
    }
    update_theme_row(&mut tx, &before, &after).await?;
    {
        fresh_ctx!(state, tx, settings, urls, cx);
        let env = theme_writes::Env::load(&mut tx, &cx).await?;
        theme_writes::after_save(
            &mut tx,
            &env,
            before.id,
            &changes,
            before.name != after.name,
        )
        .await?;
    }
    if refresh {
        return Err(Unsupported(
            "theme settings that request a refresh (Discourse.request_refresh!)",
        )
        .into());
    }
    // update_default_theme
    if keys.contains(&"default") {
        let wants = crate::params::scalar(&get("default")).as_deref() == Some("true");
        let current = is_default(&mut tx, &state, before.id).await?;
        if current && !wants {
            set_default_theme(&mut tx, &state, -1).await?;
        } else if wants {
            if after.component {
                tx.commit().await?;
                let message = tr(&state, "themes.errors.component_no_default");
                return Ok(super::search::invalid_parameters(&state, &message));
            }
            set_default_theme(&mut tx, &state, before.id).await?;
        }
    }
    let theme = Theme::find(&mut tx, before.id)
        .await?
        .ok_or(Unsupported("a theme gone after its update"))?;
    fresh_ctx!(state, tx, settings, urls, cx);
    if (!disables && !enables) || keys.len() > 1 {
        theme_writes::log_theme_change(&mut tx, &cx, acting, Some(&original_json), &theme).await?;
    }
    if disables {
        theme_writes::log_component_toggle(&mut tx, acting, &theme, false).await?;
    }
    if enables {
        theme_writes::log_component_toggle(&mut tx, acting, &theme, true).await?;
    }
    let body = themes::serialize(&mut tx, &cx, &theme, false).await?;
    tx.commit().await?;
    Ok(Json(json!({ "theme": body })).into_response())
}

/// DELETE /admin/themes/:id(.json): Themes::Destroy.
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    if let Err(r) = write_params(&state, &headers, &uri, &body, "DELETE") {
        return Ok(*r);
    }
    let acting = guardian
        .user_id()
        .ok_or(Unsupported("an admin without a user"))?;
    let id = id_of(&raw);
    if id <= 0 {
        return Ok(failed_contract(vec!["Id must be greater than 0".into()]));
    }
    let mut tx = state.pool.begin().await?;
    let Some(theme) = Theme::find(&mut tx, id).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    // The not_default policy fails without a handler: nothing rendered.
    if is_default(&mut tx, &state, id).await? {
        return Ok(axum::http::StatusCode::NO_CONTENT.into_response());
    }
    theme_writes::destroy(&mut tx, &theme).await?;
    fresh_ctx!(state, tx, settings, urls, cx);
    theme_writes::log_theme_destroy(&mut tx, &cx, acting, &theme).await?;
    theme_writes::destroy_remote_theme(&mut tx, &theme).await?;
    tx.commit().await?;
    Ok(axum::http::StatusCode::NO_CONTENT.into_response())
}

/// DELETE /admin/themes/bulk_destroy(.json): Themes::BulkDestroy.
pub async fn bulk_destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    let p = match write_params(&state, &headers, &uri, &body, "DELETE") {
        Ok(p) => p,
        Err(r) => return Ok(*r),
    };
    let acting = guardian
        .user_id()
        .ok_or(Unsupported("an admin without a user"))?;
    let raw_ids = p.get("theme_ids").cloned().unwrap_or(Value::Null);
    let present = match &raw_ids {
        Value::Array(a) => !a.is_empty(),
        Value::Object(m) => !m.is_empty(),
        Value::Null => false,
        other => crate::params::scalar(other).is_some_and(|s| !crate::ruby::is_blank(&s)),
    };
    if !present {
        return Ok(failed_contract(vec![
            "Theme ids can't be blank".into(),
            "Theme ids is too short (minimum is 1 character)".into(),
        ]));
    }
    let mut ids: Vec<i32> = Vec::new();
    for id in id_list(&raw_ids) {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.len() > 50 {
        return Ok(failed_contract(vec![
            "Theme ids is too long (maximum is 50 characters)".into(),
        ]));
    }
    if ids.iter().any(|id| *id <= 0) {
        return Ok(failed_contract(vec![
            "Theme ids must all be positive".into(),
        ]));
    }
    let mut tx = state.pool.begin().await?;
    let list: Vec<Theme> = {
        let mut out = Vec::new();
        let all: Vec<i32> =
            sqlx::query_scalar("SELECT id FROM themes WHERE id = ANY($1) ORDER BY id")
                .bind(&ids)
                .fetch_all(&mut *tx)
                .await?;
        for id in all {
            if let Some(t) = Theme::find(&mut tx, id).await? {
                out.push(t);
            }
        }
        out
    };
    if list.is_empty() {
        return Ok(super::topics::not_found_response(&state));
    }
    {
        fresh_ctx!(state, tx, settings, urls, cx);
        for theme in &list {
            theme_writes::log_theme_destroy(&mut tx, &cx, acting, theme).await?;
        }
    }
    for theme in &list {
        let default = is_default(&mut tx, &state, theme.id).await?;
        theme_writes::destroy(&mut tx, theme).await?;
        theme_writes::destroy_remote_theme(&mut tx, theme).await?;
        if default {
            set_default_theme(&mut tx, &state, -1).await?;
        }
    }
    tx.commit().await?;
    Ok(axum::http::StatusCode::NO_CONTENT.into_response())
}

/// `theme.cached_settings[name]`, None for no such setting.
async fn cached_setting(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    theme_id: i32,
    name: &str,
) -> Result<Option<Value>, AppError> {
    let settings =
        SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
    let store = crate::file_store::FileStore::for_site(&state.config, &settings)?;
    let vcx = themes::ValueCx::new(&store, &settings)?;
    theme_writes::cached_setting(conn, &vcx, theme_id, name).await
}

/// `theme.update_setting(name, value)` with the store and settings its
/// managers read.
async fn update_one_setting(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    theme_id: i32,
    name: &str,
    value: Option<&Value>,
) -> Result<Result<bool, theme_writes::SettingError>, AppError> {
    let settings =
        SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
    let store = crate::file_store::FileStore::for_site(&state.config, &settings)?;
    let vcx = themes::ValueCx::new(&store, &settings)?;
    theme_writes::update_setting(conn, &vcx, theme_id, name, value, &state.i18n).await
}

/// PUT /admin/themes/:id/setting(.json): update_single_setting.
pub async fn update_setting(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    let p = match write_params(&state, &headers, &uri, &body, "PUT") {
        Ok(p) => p,
        Err(r) => return Ok(*r),
    };
    let acting = guardian
        .user_id()
        .ok_or(Unsupported("an admin without a user"))?;
    let name = p
        .get("name")
        .and_then(crate::params::scalar)
        .filter(|n| !n.is_empty());
    let Some(name) = name else {
        return Ok((
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({"errors": ["param is missing or the value is empty or invalid: name"]})),
        )
            .into_response());
    };
    let mut tx = state.pool.begin().await?;
    let Some(theme) = Theme::find(&mut tx, id_of(&raw)).await? else {
        return Ok(super::search::invalid_parameters(&state, "id"));
    };
    let new_value = p.get("value").cloned().filter(|v| !v.is_null());
    let previous = cached_setting(&mut tx, &state, theme.id, &name).await?;
    match update_one_setting(&mut tx, &state, theme.id, &name, new_value.as_ref()).await? {
        Ok(false) => {}
        Ok(true) => {
            return Err(Unsupported(
                "theme settings that request a refresh (Discourse.request_refresh!)",
            )
            .into());
        }
        Err(theme_writes::SettingError::NotFound) => {
            return Ok(super::topics::not_found_response(&state));
        }
        Err(theme_writes::SettingError::Invalid(message)) => {
            return Ok(json_error(&message));
        }
    }
    {
        fresh_ctx!(state, tx, settings, urls, cx);
        let env = theme_writes::Env::load(&mut tx, &cx).await?;
        theme_writes::after_save(&mut tx, &env, theme.id, &[], false).await?;
    }
    let new_log = new_value.as_ref().and_then(theme_writes::log_string);
    theme_writes::log_setting_change(
        &mut tx,
        acting,
        &theme,
        &name,
        previous.as_ref().and_then(theme_writes::log_string),
        new_log,
    )
    .await?;
    let value = cached_setting(&mut tx, &state, theme.id, &name).await?;
    tx.commit().await?;
    let mut out = Map::new();
    out.insert(name, value.unwrap_or(Value::Null));
    Ok(Json(Value::Object(out)).into_response())
}

/// GET /admin/themes/:id/translations/:locale(.json): Themes::GetTranslations.
pub async fn translations(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((raw, locale)): Path<(String, String)>,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    let locale = locale.strip_suffix(".json").unwrap_or(&locale).to_string();
    if !themes::locale_available(&locale) {
        let message = state
            .i18n
            .t_with(
                "errors.messages.invalid_locale",
                &[("invalid_locale", &locale)],
            )
            .unwrap_or_default();
        return Ok(failed_contract(vec![message]));
    }
    let mut conn = state.pool.acquire().await?;
    let Some(theme) = Theme::find(&mut conn, id_of(&raw)).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if locale != "en" {
        return Err(Unsupported("theme translations in other locales").into());
    }
    let list: Vec<Value> = crate::site_texts::theme_translations(&mut conn, theme.id)
        .await?
        .into_iter()
        .map(|e| json!({"key": e.key, "value": e.value(), "default": e.default}))
        .collect();
    // model :translations: an empty list fails the step, which nothing
    // renders.
    if list.is_empty() {
        return Ok(axum::http::StatusCode::NO_CONTENT.into_response());
    }
    Ok(Json(json!({"success": "OK", "translations": list})).into_response())
}

/// GET /admin/themes/:id/preview: to the home page with the theme.
pub async fn preview(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    let mut conn = state.pool.acquire().await?;
    let Some(theme) = Theme::find(&mut conn, id_of(&raw)).await? else {
        return Ok(super::search::invalid_parameters(&state, "id"));
    };
    let location = format!(
        "{}/?preview_theme_id={}",
        state.config.globals.relative_url_root(),
        theme.id
    );
    Ok((
        axum::http::StatusCode::FOUND,
        [(axum::http::header::LOCATION, location)],
    )
        .into_response())
}

/// GET /admin/themes/:id/objects_setting_metadata/:setting_name(.json):
/// ThemeObjectsSettingMetadataSerializer, the categories an objects
/// setting's value names and its properties' descriptions.
pub async fn objects_setting_metadata(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((raw, setting_name)): Path<(String, String)>,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    let setting_name = setting_name
        .strip_suffix(".json")
        .unwrap_or(&setting_name)
        .to_string();
    let mut conn = state.pool.acquire().await?;
    let conn = &mut *conn;
    let Some(theme) = Theme::find(conn, id_of(&raw)).await? else {
        return Ok(super::search::invalid_parameters(&state, "id"));
    };
    let settings = themes::settings_of(conn, theme.id).await?;
    let Some(setting) = settings.into_iter().find(|s| s.name == setting_name) else {
        return Ok(super::search::invalid_parameters(&state, "setting_name"));
    };
    if setting.type_name != "objects" {
        return Err(
            std::io::Error::other("NoMethodError: categories of a non-objects setting").into(),
        );
    }
    let site_settings =
        SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
    let store = crate::file_store::FileStore::for_site(&state.config, &site_settings)?;
    let vcx = themes::ValueCx::new(&store, &site_settings)?;
    let stored = themes::stored_settings(conn, theme.id).await?;
    let value = themes::setting_value(
        conn,
        &vcx,
        theme.id,
        &setting,
        setting.row(&stored),
        themes::Reading::Value,
    )
    .await?;
    let objects = value.as_array().cloned().unwrap_or_default();
    let mut ids: Vec<i32> =
        crate::schema_objects::property_values_of_type(&setting.schema(), &objects, "categories")
            .iter()
            .filter_map(Value::as_i64)
            .filter_map(|id| i32::try_from(id).ok())
            .collect();
    ids.sort_unstable();
    let mut categories = Map::new();
    if !ids.is_empty() {
        let cx = crate::web_hooks::admin::Cx {
            settings: &site_settings,
            i18n: &state.i18n,
            guardian: &guardian,
            base_path: state.config.globals.relative_url_root(),
        };
        let list = crate::web_hooks::admin::categories(conn, &cx, &ids).await?;
        for category in list.as_array().into_iter().flatten() {
            if let Some(id) = category.get("id").and_then(Value::as_i64) {
                categories.insert(id.to_string(), category.clone());
            }
        }
    }
    let prefix = format!(
        "theme_metadata.settings.{}.schema.properties.",
        setting.name
    );
    let mut descriptions = Map::new();
    for t in crate::site_texts::theme_translations_of(conn, theme.id, true).await? {
        if let Some(rest) = t.key.strip_prefix(&prefix) {
            descriptions.insert(rest.replace(".schema.properties.", "."), json!(t.value()));
        }
    }
    Ok(Json(json!({
        "categories": categories,
        "property_descriptions": descriptions,
    }))
    .into_response())
}

/// `Theme.allowed_remote_theme_ids` is set: `allowed_theme_repos`
/// restricts the themes to its repositories (ban_in_allowlist_mode!).
fn allowlist_mode(state: &AppState) -> bool {
    state.config.globals.get("allowed_theme_repos").is_some()
}

/// `guardian.allowed_theme_repo_import?(repo)` for an admin.
fn allowed_theme_repo(state: &AppState, repo: &str) -> bool {
    match state.config.globals.get("allowed_theme_repos") {
        Some(list) => list.split(',').map(crate::ruby::strip).any(|u| u == repo),
        None => true,
    }
}

/// A file part of a multipart request.
struct FilePart {
    name: String,
    filename: String,
    content_type: Option<String>,
    bytes: Vec<u8>,
}

/// A request's params: the query and a JSON, form or multipart body,
/// with a multipart body's files apart.
struct FormParams {
    params: Map<String, Value>,
    files: Vec<FilePart>,
}

impl FormParams {
    fn text(&self, key: &str) -> Option<String> {
        self.params.get(key).and_then(crate::params::scalar)
    }

    fn file(&self, key: &str) -> Option<&FilePart> {
        self.files.iter().find(|f| f.name == key)
    }

    fn pairs(&self) -> Vec<(String, String)> {
        self.params
            .iter()
            .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
            .collect()
    }
}

async fn read_params(state: &AppState, request: Request) -> Result<FormParams, AppError> {
    let headers = request.headers().clone();
    let query = request.uri().query().map(str::to_string);
    let multipart = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("multipart/form-data"));
    if !multipart {
        let body = Bytes::from_request(request, state)
            .await
            .map_err(|e| std::io::Error::other(e.body_text()))?;
        return Ok(FormParams {
            params: crate::params::parse(query.as_deref(), &headers, &body),
            files: Vec::new(),
        });
    }
    let mut out = FormParams {
        params: crate::params::parse_query(query.as_deref().unwrap_or("")),
        files: Vec::new(),
    };
    let mut parts = Multipart::from_request(request, state)
        .await
        .map_err(|e| std::io::Error::other(e.body_text()))?;
    while let Some(field) = parts
        .next_field()
        .await
        .map_err(|e| std::io::Error::other(e.body_text()))?
    {
        let name = field.name().unwrap_or_default().to_string();
        match field.file_name().map(str::to_string) {
            Some(filename) => {
                let content_type = field.content_type().map(str::to_string);
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| std::io::Error::other(e.body_text()))?;
                out.params.remove(&name);
                out.files.push(FilePart {
                    name,
                    filename,
                    content_type,
                    bytes: bytes.to_vec(),
                });
            }
            None => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| std::io::Error::other(e.body_text()))?;
                out.files.retain(|f| f.name != name);
                out.params.insert(name, Value::String(text));
            }
        }
    }
    Ok(out)
}

/// `Admin::ThemesController::THEME_CONTENT_TYPES`
const THEME_CONTENT_TYPES: [&str; 4] = [
    "application/gzip",
    "application/x-gzip",
    "application/x-zip-compressed",
    "application/zip",
];

/// The largest request body the import and asset uploads read.
pub const BUNDLE_BODY_LIMIT: usize = 256 * 1024 * 1024;

/// POST /admin/themes/import(.json): a remote theme (refused past its
/// checks: cloning is a network fetch), its placeholder, or a bundle.
pub async fn import(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    request: Request,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    let headers = request.headers().clone();
    let path = request.uri().path().to_string();
    let form = read_params(&state, request).await?;
    if !super::session::csrf_ok(&state, &headers, &form.pairs(), &path, "POST") {
        return Ok(super::session::bad_csrf());
    }
    let acting = guardian
        .user_id()
        .ok_or(Unsupported("an admin without a user"))?;
    if let Some(remote) = form.text("remote") {
        return import_remote(&state, &form, &remote, acting).await;
    }
    let theme_file = match (form.file("theme"), form.params.get("theme")) {
        (Some(file), _) => Some(file),
        // String#content_type
        (None, Some(_)) => {
            return Err(
                std::io::Error::other("NoMethodError: content_type of a theme param").into(),
            );
        }
        (None, None) => None,
    };
    let bundle = match (form.file("bundle"), form.params.get("bundle")) {
        (Some(file), _) => Some(file),
        (None, Some(_)) => {
            return Err(std::io::Error::other("NoMethodError: path of a bundle param").into());
        }
        (None, None) => theme_file.filter(|f| {
            f.content_type
                .as_deref()
                .is_some_and(|c| THEME_CONTENT_TYPES.contains(&c))
        }),
    };
    let Some(bundle) = bundle else {
        return Ok(json_error(&tr(
            &state,
            "themes.import_error.unknown_file_type",
        )));
    };
    if allowlist_mode(&state) {
        return Ok(super::search::invalid_access(&state));
    }
    let mut tx = state.pool.begin().await?;
    fresh_ctx!(state, tx, settings, urls, cx);
    let uploader = crate::theme_import::Uploader {
        cx: &cx,
        staff: true,
    };
    let imported = crate::theme_import::update_zipped_theme(
        &mut tx,
        &uploader,
        &crate::theme_import::ZipImport {
            bytes: &bundle.bytes,
            original_filename: &bundle.filename,
            user_id: acting,
            theme_id: form.text("theme_id"),
            update_components: form.text("components"),
        },
    )
    .await?;
    let theme = match imported {
        Ok(theme) => theme,
        Err(error) => return Ok(json_error(&error.0)),
    };
    theme_writes::log_theme_change(&mut tx, &cx, acting, None, &theme).await?;
    let body = themes::serialize(&mut tx, &cx, &theme, false).await?;
    tx.commit().await?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({ "theme": body })),
    )
        .into_response())
}

/// The ssh key the import's `public_key` names (`ssh_key_<public key>`).
async fn private_key_for(
    conn: &mut sqlx::PgConnection,
    public_key: &str,
) -> Result<Option<String>, AppError> {
    Ok(
        crate::owned_schema::cached_get(conn, &format!("ssh_key_{public_key}"))
            .await?
            .filter(|k| !crate::ruby::is_blank(k)),
    )
}

/// The remote branch of `import`.
async fn import_remote(
    state: &AppState,
    form: &FormParams,
    remote: &str,
    acting: i32,
) -> Result<Response, AppError> {
    if remote.chars().count() > crate::theme_import::MAX_REMOTE_LENGTH {
        let repo: String = remote
            .chars()
            .take(crate::theme_import::MAX_REMOTE_LENGTH + 1)
            .collect();
        let message = state
            .i18n
            .t_with("themes.import_error.not_allowed_theme", &[("repo", &repo)])
            .unwrap_or_default();
        return Ok(json_error(&message));
    }
    let stripped = crate::ruby::strip(remote);
    if !allowed_theme_repo(state, stripped) {
        let message = state
            .i18n
            .t_with(
                "themes.import_error.not_allowed_theme",
                &[("repo", stripped)],
            )
            .unwrap_or_default();
        return Ok((
            axum::http::StatusCode::FORBIDDEN,
            Json(json!({ "errors": [message] })),
        )
            .into_response());
    }
    let branch = form.text("branch");
    let public_key = form.text("public_key");
    let mut tx = state.pool.begin().await?;
    let private_key = match &public_key {
        Some(key) => private_key_for(&mut tx, key).await?,
        None => None,
    };
    if public_key
        .as_deref()
        .is_some_and(|k| !crate::ruby::is_blank(k))
        && private_key.is_none()
    {
        return Ok(json_error(&tr(state, "themes.import_error.ssh_key_gone")));
    }
    if form.params.contains_key("placeholder") {
        if private_key.is_none() {
            return Ok(json_error(&tr(
                state,
                "themes.import_error.git_authentication",
            )));
        }
    } else {
        let error = crate::theme_import::git_import(&state.i18n, stripped, private_key.as_deref())?;
        if !form.params.contains_key("force") {
            return Ok(json_error(&error.0));
        }
    }
    let theme = crate::theme_import::create_placeholder(
        &mut tx,
        remote,
        branch.as_deref(),
        private_key.as_deref(),
        acting,
    )
    .await?;
    fresh_ctx!(state, tx, settings, urls, cx);
    let body = themes::serialize(&mut tx, &cx, &theme, false).await?;
    tx.commit().await?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({ "theme": body })),
    )
        .into_response())
}

/// GET /admin/customize/themes/:id/export: the theme as a zip.
pub async fn export(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    let mut conn = state.pool.acquire().await?;
    let Some(theme) = Theme::find(&mut conn, id_of(&raw)).await? else {
        return Ok(super::search::invalid_parameters(&state, "id"));
    };
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let store = crate::file_store::FileStore::for_site(&state.config, &settings)?;
    let (filename, bytes) = crate::theme_import::export(&mut conn, &store, &theme).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\"; filename*=UTF-8''{filename}"),
            ),
        ],
        bytes,
    )
        .into_response())
}

/// POST /admin/themes/upload_asset(.json): a file uploaded for themes.
pub async fn upload_asset(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    request: Request,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    let headers = request.headers().clone();
    let path = request.uri().path().to_string();
    let form = read_params(&state, request).await?;
    if !super::session::csrf_ok(&state, &headers, &form.pairs(), &path, "POST") {
        return Ok(super::session::bad_csrf());
    }
    let acting = guardian
        .user_id()
        .ok_or(Unsupported("an admin without a user"))?;
    if allowlist_mode(&state) {
        return Ok(super::search::invalid_access(&state));
    }
    let Some(file) = form.file("file") else {
        return Err(std::io::Error::other("NoMethodError: path of a missing file").into());
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let outcome = crate::uploads::create(
        &mut conn,
        &settings,
        &state.i18n,
        &urls,
        &crate::file_store::FileStore::for_site(&state.config, &settings)?,
        &crate::uploads::NewUpload {
            user_id: acting,
            staff: true,
            upload_type: "theme",
            filename: &file.filename,
            bytes: &file.bytes,
            pasted: false,
            for_private_message: false,
            for_export: false,
            for_theme: true,
            gravatar_origin: None,
            skip_validations: false,
        },
    )
    .await?;
    match outcome {
        crate::uploads::Outcome::Created(upload) => Ok((
            axum::http::StatusCode::CREATED,
            Json(json!({ "upload_id": upload["id"] })),
        )
            .into_response()),
        crate::uploads::Outcome::Invalid(errors) => Ok((
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "errors": errors })),
        )
            .into_response()),
    }
}

/// POST /admin/themes/generate_key_pair(.json): an RSA key pair, the
/// private key kept an hour under its public key for the import.
pub async fn generate_key_pair(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    if let Err(r) = write_params(&state, &headers, &uri, &body, "POST") {
        return Ok(*r);
    }
    let (public_key, private_key) =
        tokio::task::spawn_blocking(crate::theme_import::generate_ssh_key)
            .await
            .map_err(std::io::Error::other)??;
    let mut conn = state.pool.acquire().await?;
    crate::owned_schema::cached_setex(
        &mut conn,
        &format!("ssh_key_{public_key}"),
        3600,
        &private_key,
    )
    .await?;
    Ok(Json(json!({ "public_key": public_key })).into_response())
}

/// PUT /admin/themes/:id/source(.json): a git theme's remote url changed,
/// refused once its checks pass (the update clones the repository).
pub async fn update_source(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    let p = match write_params(&state, &headers, &uri, &body, "PUT") {
        Ok(p) => p,
        Err(r) => return Ok(*r),
    };
    let mut conn = state.pool.acquire().await?;
    let Some(theme) = Theme::find(&mut conn, id_of(&raw)).await? else {
        return Ok(super::search::invalid_parameters(&state, "id"));
    };
    let remote = themes::RemoteTheme::find(&mut conn, theme.remote_theme_id).await?;
    if !remote.as_ref().is_some_and(themes::RemoteTheme::is_git) {
        return Ok(super::search::invalid_parameters(&state, "remote_theme"));
    }
    let remote_url = p
        .get("remote_url")
        .and_then(crate::params::scalar)
        .map(|u| crate::ruby::strip(&u).to_string())
        .filter(|u| !u.is_empty());
    let Some(remote_url) = remote_url else {
        return Ok(super::search::invalid_parameters(&state, "remote_url"));
    };
    if remote_url.chars().count() > crate::theme_import::MAX_REMOTE_LENGTH {
        let repo: String = remote_url.chars().take(101).collect();
        let message = state
            .i18n
            .t_with("themes.import_error.not_allowed_theme", &[("repo", &repo)])
            .unwrap_or_default();
        return Ok(json_error(&message));
    }
    if !allowed_theme_repo(&state, &remote_url) {
        let message = state
            .i18n
            .t_with(
                "themes.import_error.not_allowed_theme",
                &[("repo", &remote_url)],
            )
            .unwrap_or_default();
        return Ok((
            axum::http::StatusCode::FORBIDDEN,
            Json(json!({ "errors": [message] })),
        )
            .into_response());
    }
    if let Some(public_key) = p
        .get("public_key")
        .and_then(crate::params::scalar)
        .filter(|k| !crate::ruby::is_blank(k))
        && private_key_for(&mut conn, &public_key).await?.is_none()
    {
        return Ok(json_error(&tr(&state, "themes.import_error.ssh_key_gone")));
    }
    Err(Unsupported("changing a git theme's source (update_from_remote clones it)").into())
}
