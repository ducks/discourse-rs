//! Port of Admin::SiteSettingsController#update: PUT
//! /admin/site_settings/:id(.json) with the new value under the setting's
//! name. The route is for admins only (AdminConstraint).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_setting_update::{self, Outcome};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// PUT /admin/site_settings/:id
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let id = id.strip_suffix(".json").unwrap_or(&id).to_string();
    if id == "bulk_update" {
        return Err(Unsupported("bulk site setting updates").into());
    }
    if p.contains_key("update_existing_user") {
        return Err(Unsupported("backfilling user preferences (update_existing_user)").into());
    }
    // params[id].to_s
    let raw = p.get(&id).and_then(params::scalar).unwrap_or_default();
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let outcome = site_setting_update::update(
        &mut tx,
        &state.site_setting_defs,
        &settings,
        &state.i18n,
        &guardian,
        &id,
        &raw,
    )
    .await?;
    Ok(match outcome {
        Outcome::Done => {
            tx.commit().await?;
            StatusCode::NO_CONTENT.into_response()
        }
        Outcome::Invalid(message) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "errors": [message] })),
        )
            .into_response(),
    })
}

/// GET /admin/site_settings: Admin::SiteSettingsController#index, the
/// settings an admin can see (SiteSetting.all_settings by `categories`,
/// `plugin` and `names`) and the default theme.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    uri: Uri,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let query: Vec<(String, String)> = form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
        .into_owned()
        .collect();
    let list = |key: &str| -> Option<Vec<String>> {
        let values: Vec<String> = query
            .iter()
            .filter(|(k, _)| k == key || *k == format!("{key}[]"))
            .map(|(_, v)| v.clone())
            .collect();
        (!values.is_empty()).then_some(values)
    };
    let filters = crate::admin_site_settings::Filters {
        categories: list("categories"),
        plugin: query
            .iter()
            .find(|(k, _)| k == "plugin")
            .map(|(_, v)| v.clone()),
        names: list("names"),
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let cx = crate::admin_site_settings::Context {
        defs: &state.site_setting_defs,
        settings: &settings,
        i18n: &state.i18n,
        globals: &state.config.globals,
        base_path: state.config.globals.relative_url_root(),
        urls: &urls,
    };
    let site_settings = crate::admin_site_settings::all_settings(&mut conn, &cx, &filters).await?;
    let default_theme = default_theme(&mut conn, &settings).await?;
    Ok(
        Json(json!({ "site_settings": site_settings, "default_theme": default_theme }))
            .into_response(),
    )
}

/// BasicThemeSerializer for Theme.find_default: its description is the
/// theme's own `theme_metadata.description` translation.
async fn default_theme(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
) -> Result<Value, AppError> {
    #[derive(sqlx::FromRow)]
    struct ThemeRow {
        id: i32,
        name: String,
        created_at: chrono::NaiveDateTime,
        updated_at: chrono::NaiveDateTime,
        component: bool,
    }
    let theme: Option<ThemeRow> = sqlx::query_as(
        "SELECT id, name, created_at, updated_at, component FROM themes WHERE id = $1",
    )
    .bind(settings.get("default_theme_id")?.to_i() as i32)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(ThemeRow {
        id,
        name,
        created_at,
        updated_at,
        component,
    }) = theme
    else {
        return Ok(Value::Null);
    };
    // Theme.targets[:translations], the English field (the fallback chain
    // of `en`).
    let translations: Option<String> = sqlx::query_scalar(
        "SELECT value FROM theme_fields WHERE theme_id = $1 AND target_id = 4 AND name = 'en' LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let description = translations
        .and_then(|y| serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&y).ok())
        .and_then(|y| {
            y.get("en")?
                .get("theme_metadata")?
                .get("description")?
                .as_str()
                .map(str::to_string)
        });
    Ok(json!({
        "id": id,
        "name": name,
        "description": description,
        "created_at": crate::topic_list::time_json(created_at),
        "updated_at": crate::topic_list::time_json(updated_at),
        "default": true,
        "component": component,
    }))
}
