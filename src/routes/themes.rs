//! Admin::ThemesController's reads (index, show), the /admin/config
//! customize, color palettes and welcome banner reads, and the theme site
//! setting write (Themes::ThemeSiteSettingManager). JSON only.
//!
//! Refused for now: creating, updating, destroying, importing and
//! exporting themes, their fields and settings (see ROADMAP 90d).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::color_scheme::ColorScheme;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
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
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    let mut conn = state.pool.acquire().await?;
    let conn = &mut *conn;
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
        Ok(Json(json!({ "themes": out })).into_response())
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
    let def = state.site_setting_defs.get(&name).ok_or(Unsupported(
        "a themeable setting missing from site_settings.yml",
    ))?;
    // to_db_value / to_rb_value for the setting's type, the default when
    // nil or equal to it.
    let raw_value = p.get("value").and_then(crate::params::scalar);
    let (data_type, db_value, rb_value): (i32, String, Value) = match def.data_type {
        crate::site_settings::DataType::Bool => {
            let default = def.default.truthy();
            let value = match raw_value.as_deref() {
                None => default,
                Some(v) => matches!(v, "t" | "true"),
            };
            (5, if value { "t" } else { "f" }.into(), json!(value))
        }
        crate::site_settings::DataType::Enum => {
            let default = def.default.to_s();
            let value = raw_value.unwrap_or(default);
            let valid = crate::setting_enums::recorded(&name)
                .and_then(|r| r.get("valid_values"))
                .and_then(Value::as_array)
                .is_some_and(|vs| vs.iter().any(|v| v["value"] == json!(value)));
            if !valid {
                return Ok(super::search::invalid_parameters(
                    &state,
                    &format!("Invalid value `{value}` for `{name}`"),
                ));
            }
            (7, value.clone(), json!(value))
        }
        _ => return Err(Unsupported("themeable settings of other types").into()),
    };
    let existing: Option<(i32, i32, Option<String>)> = sqlx::query_as(
        "SELECT id::int, data_type, value FROM theme_site_settings WHERE theme_id = $1 AND name = $2 LIMIT 1",
    )
    .bind(theme.id)
    .bind(&name)
    .fetch_optional(&mut *tx)
    .await?;
    let rb = |t: i32, v: Option<&str>| match crate::site_settings::DataType::from_id(t) {
        Some(crate::site_settings::DataType::Bool) => json!(matches!(v, Some("t") | Some("true"))),
        _ => json!(v),
    };
    let previous = existing.as_ref().map(|(_, t, v)| rb(*t, v.as_deref()));
    let row_id: i32 = match &existing {
        Some((id, _, current)) => {
            if current.as_deref() != Some(db_value.as_str()) {
                sqlx::query("UPDATE theme_site_settings SET value = $2, updated_at = now() WHERE id = $1")
                    .bind(id)
                    .bind(&db_value)
                    .execute(&mut *tx)
                    .await?;
            }
            *id
        }
        None => {
            sqlx::query_scalar(
                "INSERT INTO theme_site_settings (theme_id, name, data_type, value, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, now(), now()) RETURNING id::int",
            )
            .bind(theme.id)
            .bind(&name)
            .bind(data_type)
            .bind(&db_value)
            .fetch_one(&mut *tx)
            .await?
        }
    };
    let action = crate::staff_action_logs::action_id("change_theme_site_setting")
        .ok_or(Unsupported("a staff action missing from enums.json"))?;
    // A string column casts booleans to "t" and "f".
    let as_log = |v: &Value| match v {
        Value::Null => None,
        Value::Bool(b) => Some(if *b { "t" } else { "f" }.to_string()),
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    };
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, subject, previous_value, new_value, \
         admin_only, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, TRUE, now(), now())",
    )
    .bind(action)
    .bind(acting)
    .bind(format!("{}: {name}", theme.name))
    .bind(previous.as_ref().and_then(as_log))
    .bind(as_log(&rb_value))
    .execute(&mut *tx)
    .await?;
    let stored: (Option<String>,) =
        sqlx::query_as("SELECT value FROM theme_site_settings WHERE id = $1")
            .bind(row_id)
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(Json(json!({
        "success": "OK",
        "name": name,
        "value": stored.0,
        "theme_id": theme.id,
    }))
    .into_response())
}

/// PUT /admin/themes/:id/setting: a theme setting's change invalidates the
/// theme's scss fields, whose rebake is a stylesheet compile
/// (ThemeField#ensure_baked! writes the compiled CSS's SHA1).
pub async fn update_setting(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    Err(Unsupported("theme setting changes (scss fields rebaked: stylesheet compile)").into())
}

/// POST /admin/themes, PUT and DELETE /admin/themes/:id, bulk_destroy,
/// import: not ported yet (their staff logs carry the full ThemeSerializer
/// JSON, and saves rebake fields).
pub async fn write_refused(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_admin(&state, &guardian) {
        return Ok(r);
    }
    Err(Unsupported("creating, updating and destroying themes").into())
}
