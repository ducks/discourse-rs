//! Admin::ColorSchemesController (/admin/color_schemes) and
//! Admin::Config::ColorPalettesController#show (/admin/config/colors/:id),
//! with ColorSchemeRevisor and ColorScheme#diverge_from_remote. JSON only.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::color_scheme::ColorScheme;
use crate::session::current::AuthGuardian;
use crate::{AppError, AppState, Unsupported};

fn json_request(headers: &HeaderMap, uri: &Uri) -> bool {
    uri.path().ends_with(".json")
        || headers
            .get("x-requested-with")
            .is_some_and(|v| v == "XMLHttpRequest")
        || super::wants_json(uri.query(), headers)
}

fn pairs(p: &Map<String, Value>) -> Vec<(String, String)> {
    p.iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn id_of(raw: &str) -> i32 {
    crate::ruby::to_i(raw.strip_suffix(".json").unwrap_or(raw)) as i32
}

fn refuse_non_admin(state: &AppState, guardian: &crate::guardian::Guardian) -> Option<Response> {
    (!guardian.is_admin()).then(|| super::topics::not_found_response(state))
}

/// ColorSchemeColor's hex format.
fn valid_hex(hex: &str) -> bool {
    (hex.len() == 3 || hex.len() == 6) && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// ActiveModel's boolean cast: None for a blank value.
fn cast_bool(v: &str) -> Option<bool> {
    if v.is_empty() {
        return None;
    }
    Some(!matches!(
        v,
        "0" | "f" | "F" | "false" | "FALSE" | "off" | "OFF"
    ))
}

/// ActiveModel's integer cast: None for a blank value.
fn cast_int(v: &str) -> Option<i32> {
    (!crate::ruby::is_blank(v)).then(|| crate::ruby::to_i(v) as i32)
}

/// `color_scheme_params`: the permitted keys of `color_scheme`.
struct SchemeParams {
    name: Option<Option<String>>,
    user_selectable: Option<Option<bool>>,
    base_scheme_id: Option<Option<i32>>,
    base_scheme_id_raw: Option<String>,
    colors: Option<Vec<(String, String)>>,
    default_light_on_theme: Option<String>,
    default_dark_on_theme: Option<String>,
}

fn scheme_params(p: &Map<String, Value>) -> SchemeParams {
    let empty = Map::new();
    let s = p
        .get("color_scheme")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let scalar = |k: &str| {
        s.get(k)
            .map(|v| crate::params::scalar(v).unwrap_or_default())
    };
    let colors = s.get("colors").map(|c| {
        let items: Vec<&Value> = match c {
            Value::Array(a) => a.iter().collect(),
            Value::Object(m) => m.values().collect(),
            _ => Vec::new(),
        };
        items
            .into_iter()
            .filter_map(Value::as_object)
            .map(|c| {
                let get = |k: &str| c.get(k).and_then(crate::params::scalar).unwrap_or_default();
                (get("name"), get("hex"))
            })
            .collect()
    });
    SchemeParams {
        name: s.get("name").map(crate::params::scalar),
        user_selectable: scalar("user_selectable").map(|v| cast_bool(&v)),
        base_scheme_id: scalar("base_scheme_id").map(|v| cast_int(&v)),
        base_scheme_id_raw: scalar("base_scheme_id"),
        colors,
        default_light_on_theme: scalar("default_light_on_theme"),
        default_dark_on_theme: scalar("default_dark_on_theme"),
    }
}

fn errors(messages: Vec<String>) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "errors": messages })),
    )
        .into_response()
}

const COLORS_INVALID: &str = "Color scheme colors is invalid";

async fn rendered(conn: &mut PgConnection, id: i32) -> Result<Response, AppError> {
    let scheme = ColorScheme::find(conn, id)
        .await?
        .ok_or(Unsupported("a color scheme gone once saved"))?;
    Ok(Json(scheme.serialize()?).into_response())
}

/// GET /admin/color_schemes(.json): the base palettes, then every scheme.
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
        return Err(Unsupported("the admin color schemes page").into());
    }
    let mut conn = state.pool.acquire().await?;
    let mut out = Vec::new();
    for scheme in ColorScheme::base_color_schemes(&state.i18n) {
        out.push(scheme.serialize()?);
    }
    for scheme in ColorScheme::all(&mut conn).await? {
        out.push(scheme.serialize()?);
    }
    Ok(Json(Value::Array(out)).into_response())
}

/// GET /admin/config/colors/:id(.json)
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
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
    match ColorScheme::find(&mut conn, id_of(&raw)).await? {
        Some(scheme) => Ok(Json(scheme.serialize()?).into_response()),
        None => Ok(super::topics::not_found_response(&state)),
    }
}

async fn write_prelude(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
    method: &str,
) -> Result<Result<Map<String, Value>, Response>, AppError> {
    if let Some(r) = refuse_non_admin(state, guardian) {
        return Ok(Err(r));
    }
    let p = crate::params::parse(uri.query(), headers, body);
    if !super::session::csrf_ok(state, headers, &pairs(&p), uri.path(), method) {
        return Ok(Err(super::session::bad_csrf()));
    }
    if !json_request(headers, uri) {
        return Ok(Err(super::session::render_empty()));
    }
    Ok(Ok(p))
}

async fn insert_color(
    conn: &mut PgConnection,
    scheme_id: i32,
    name: &str,
    hex: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO color_scheme_colors (name, hex, color_scheme_id, created_at, updated_at) \
         VALUES ($1, $2, $3, now(), now())",
    )
    .bind(name)
    .bind(hex)
    .bind(scheme_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// POST /admin/color_schemes(.json): `ColorScheme.create`.
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match write_prelude(&state, &guardian, &headers, &uri, &body, "POST").await? {
        Ok(p) => p,
        Err(r) => return Ok(r),
    };
    let params = scheme_params(&p);
    if params.default_light_on_theme.is_some() || params.default_dark_on_theme.is_some() {
        return Err(
            Unsupported("creating a color scheme with theme defaults (unknown attribute)").into(),
        );
    }
    let colors = params.colors.unwrap_or_default();
    // validates_associated, then the autosave validation of each new color.
    let invalid = colors.iter().filter(|(_, hex)| !valid_hex(hex)).count();
    if invalid > 0 {
        return Ok(errors(vec![COLORS_INVALID.to_string(); invalid + 1]));
    }
    let Some(Some(name)) = params.name else {
        return Err(Unsupported("a color scheme without a name (NOT NULL violation)").into());
    };
    let user_selectable = match params.user_selectable {
        Some(None) => {
            return Err(Unsupported("a blank user_selectable (NOT NULL violation)").into());
        }
        Some(Some(b)) => b,
        None => false,
    };
    let mut tx = state.pool.begin().await?;
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO color_schemes (name, base_scheme_id, user_selectable, created_at, updated_at) \
         VALUES ($1, $2, $3, now(), now()) RETURNING id",
    )
    .bind(&name)
    .bind(params.base_scheme_id.flatten())
    .bind(user_selectable)
    .fetch_one(&mut *tx)
    .await?;
    for (name, hex) in &colors {
        insert_color(&mut tx, id, name, hex).await?;
    }
    let response = rendered(&mut tx, id).await?;
    tx.commit().await?;
    Ok(response)
}

#[derive(sqlx::FromRow)]
struct SchemeRow {
    id: i32,
    name: String,
    version: i32,
    theme_id: Option<i32>,
    base_scheme_id: Option<i32>,
    user_selectable: bool,
}

async fn find_row(conn: &mut PgConnection, id: i32) -> Result<Option<SchemeRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, name, version, theme_id, base_scheme_id, user_selectable FROM color_schemes \
         WHERE id = $1 AND NOT remote_copy",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await
}

/// PUT /admin/color_schemes/:id(.json): ColorSchemeRevisor, then the
/// default theme's light or dark scheme.
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match write_prelude(&state, &guardian, &headers, &uri, &body, "PUT").await? {
        Ok(p) => p,
        Err(r) => return Ok(r),
    };
    let mut tx = state.pool.begin().await?;
    let Some(mut row) = find_row(&mut tx, id_of(&raw)).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let params = scheme_params(&p);
    if row.theme_id.is_some() {
        // A param's String never equals the Integer column.
        let renamed = matches!(&params.name, Some(n) if n.as_deref() != Some(row.name.as_str()));
        if renamed || params.base_scheme_id_raw.is_some() {
            return Ok(super::search::invalid_access(&state));
        }
    }

    // ColorSchemeRevisor#revise
    let mut name = row.name.clone();
    let mut user_selectable = row.user_selectable;
    let mut base_scheme_id = row.base_scheme_id;
    if let Some(n) = &params.name {
        name = n.clone().ok_or(Unsupported(
            "a color scheme without a name (NOT NULL violation)",
        ))?;
    }
    if let Some(u) = params.user_selectable {
        user_selectable = u.ok_or(Unsupported("a blank user_selectable (NOT NULL violation)"))?;
    }
    if let Some(b) = params.base_scheme_id {
        base_scheme_id = b;
    }
    let colors_present = params.colors.as_ref().is_some_and(|c| !c.is_empty());
    if colors_present && row.theme_id.is_some() && base_scheme_id.is_none() {
        // diverge_from_remote: reload (the assignments above are lost), a
        // remote copy with the current colors, the scheme based on it.
        let copy: i32 = sqlx::query_scalar(
            "INSERT INTO color_schemes (name, version, theme_id, via_wizard, remote_copy, base_scheme_id, \
             user_selectable, created_at, updated_at) \
             SELECT name, version, theme_id, FALSE, TRUE, NULL, FALSE, now(), now() FROM color_schemes \
             WHERE id = $1 RETURNING id",
        )
        .bind(row.id)
        .fetch_one(&mut *tx)
        .await?;
        let current: Vec<(String, String)> = sqlx::query_as(
            "SELECT name, hex FROM color_scheme_colors WHERE color_scheme_id = $1 ORDER BY id",
        )
        .bind(row.id)
        .fetch_all(&mut *tx)
        .await?;
        for (n, h) in &current {
            insert_color(&mut tx, copy, n, h).await?;
        }
        sqlx::query(
            "UPDATE color_schemes SET base_scheme_id = $2, version = version + 1, updated_at = now() \
             WHERE id = $1",
        )
        .bind(row.id)
        .bind(copy)
        .execute(&mut *tx)
        .await?;
        row = find_row(&mut tx, row.id)
            .await?
            .ok_or(Unsupported("a color scheme gone while diverging"))?;
        name = row.name.clone();
        user_selectable = row.user_selectable;
        base_scheme_id = row.base_scheme_id;
    }
    let mut invalid_existing = false;
    let mut invalid_new = 0;
    if let Some(colors) = &params.colors {
        // colors_by_name, read once: the last color of a name, by id.
        let rows: Vec<(i32, String, String)> = sqlx::query_as(
            "SELECT id, name, hex FROM color_scheme_colors WHERE color_scheme_id = $1 ORDER BY id",
        )
        .bind(row.id)
        .fetch_all(&mut *tx)
        .await?;
        let mut by_name: std::collections::HashMap<String, (i32, String)> = rows
            .into_iter()
            .map(|(id, name, hex)| (name, (id, hex)))
            .collect();
        for (cname, hex) in colors {
            match by_name.get_mut(cname) {
                Some((cid, current)) => {
                    if !valid_hex(hex) {
                        invalid_existing = true;
                    } else if current != hex {
                        sqlx::query(
                            "UPDATE color_scheme_colors SET hex = $2, updated_at = now() WHERE id = $1",
                        )
                        .bind(*cid)
                        .bind(hex)
                        .execute(&mut *tx)
                        .await?;
                        *current = hex.clone();
                    }
                }
                None if valid_hex(hex) => insert_color(&mut tx, row.id, cname, hex).await?,
                None => invalid_new += 1,
            }
        }
    }
    // validates_associated once, then each new color the autosave check
    // finds invalid.
    let invalid: Vec<String> = if invalid_existing || invalid_new > 0 {
        vec![COLORS_INVALID.to_string(); 1 + invalid_new]
    } else {
        Vec::new()
    };
    let changed = name != row.name
        || user_selectable != row.user_selectable
        || base_scheme_id != row.base_scheme_id;
    if (params.colors.is_some() || changed) && invalid.is_empty() {
        sqlx::query(
            "UPDATE color_schemes SET name = $2, user_selectable = $3, base_scheme_id = $4, \
             version = $5, updated_at = now() WHERE id = $1",
        )
        .bind(row.id)
        .bind(&name)
        .bind(user_selectable)
        .bind(base_scheme_id)
        .bind(row.version + 1)
        .execute(&mut *tx)
        .await?;
    }

    // update_theme_default_scheme!
    let on_theme = |v: &Option<String>| v.as_ref().map(|v| (v != "false").then_some(row.id));
    let light = on_theme(&params.default_light_on_theme);
    let dark = on_theme(&params.default_dark_on_theme);
    if light.is_some() || dark.is_some() {
        let settings = crate::site_settings::SiteSettings::load(
            &mut tx,
            &state.site_setting_defs,
            &state.config.globals,
        )
        .await?;
        let default_id = settings.get("default_theme_id")?.to_i() as i32;
        let theme: Option<(Option<i32>, Option<i32>)> = sqlx::query_as(
            "SELECT color_scheme_id, dark_color_scheme_id FROM themes WHERE id = $1",
        )
        .bind(default_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((current_light, current_dark)) = theme else {
            return Err(Unsupported("no default theme to set a color scheme on").into());
        };
        let new_light = light.unwrap_or(current_light);
        let new_dark = dark.unwrap_or(current_dark);
        if new_light != current_light || new_dark != current_dark {
            sqlx::query(
                "UPDATE themes SET color_scheme_id = $2, dark_color_scheme_id = $3, updated_at = now() \
                 WHERE id = $1",
            )
            .bind(default_id)
            .bind(new_light)
            .bind(new_dark)
            .execute(&mut *tx)
            .await?;
        }
    }
    if !invalid.is_empty() {
        tx.commit().await?;
        return Ok(errors(invalid));
    }
    let response = rendered(&mut tx, row.id).await?;
    tx.commit().await?;
    Ok(response)
}

/// DELETE /admin/color_schemes/:id(.json)
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Err(r) = write_prelude(&state, &guardian, &headers, &uri, &body, "DELETE").await? {
        return Ok(r);
    }
    let mut tx = state.pool.begin().await?;
    let Some(row) = find_row(&mut tx, id_of(&raw)).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if row.theme_id.is_some() {
        return Ok(super::search::invalid_access(&state));
    }
    sqlx::query("DELETE FROM color_scheme_colors WHERE color_scheme_id = $1")
        .bind(row.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM color_schemes WHERE id = $1")
        .bind(row.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({"success": "OK"})).into_response())
}
