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
    // `!!value`: anything but nil and false, so a form's "false" is true.
    let truthy = |v: Option<&Value>| !matches!(v, None | Some(Value::Null | Value::Bool(false)));
    // (name, params value.to_s, backfill) for each setting.
    let updates: Vec<(String, String, bool)> = if id == "bulk_update" {
        // params[:settings].to_unsafe_h: { name => { value:, backfill: } }
        let Some(Value::Object(map)) = p.get("settings") else {
            return Err(Unsupported("a bulk site setting update without settings").into());
        };
        map.iter()
            .map(|(name, config)| {
                (
                    name.clone(),
                    config
                        .get("value")
                        .and_then(params::scalar)
                        .unwrap_or_default(),
                    truthy(config.get("backfill")),
                )
            })
            .collect()
    } else {
        vec![(
            id.clone(),
            p.get(&id).and_then(params::scalar).unwrap_or_default(),
            truthy(p.get("update_existing_user")),
        )]
    };
    // The backfill only applies to default user preferences.
    if updates.iter().any(|(name, _, backfill)| {
        *backfill
            && name.starts_with("default_")
            && !site_setting_update::SIDEBAR_DEFAULTS.contains(&name.as_str())
            && !crate::user_option_defaults::is_user_option(name)
    }) {
        return Err(Unsupported("backfilling user preferences (update_existing_user)").into());
    }
    // One transaction: a setting refused rolls back the ones before it, as
    // the service's save step does.
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    for (name, raw, backfill) in &updates {
        let outcome = site_setting_update::update(
            &mut tx,
            &state.bus,
            &state.config,
            &state.site_setting_defs,
            &settings,
            &state.i18n,
            &guardian,
            name,
            raw,
            *backfill,
            &[],
        )
        .await?;
        if let Outcome::Invalid(message) = outcome {
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "errors": [message] })),
            )
                .into_response());
        }
    }
    tx.commit().await?;
    after_update(&state, &settings).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// What the settings' change handlers do past the update's transaction:
/// a changed title or site description rewrites the welcome topic
/// (crate::seed_topics), with the settings as they now are.
pub(super) async fn after_update(state: &AppState, before: &SiteSettings) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let now =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    drop(conn);
    // Each changed setting's handler in the order the settings are saved
    // (as declared), with the site as it is at that point.
    let host = crate::pretty_text::Host::from_state(state);
    let mut step = before.clone();
    for name in ["title", "site_description"] {
        let value = now.get(name)?.clone();
        if *before.get(name)? == value {
            continue;
        }
        step = step.with(name, value);
        let ctx = crate::posting::Ctx {
            host: &host,
            settings: &step,
            config: &state.config,
            i18n: &state.i18n,
            bus: &state.bus,
        };
        crate::seed_topics::update_welcome_topic(&state.pool, &ctx).await?;
    }
    Ok(())
}

/// PUT /admin/site_settings/:id/user_count: how many users a backfill of
/// a default_* setting to `params[id]` would change.
pub async fn user_count(
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
    let Some(def) = state
        .site_setting_defs
        .get(&id)
        .filter(|_| id.starts_with("default_"))
    else {
        return Ok(super::topics::not_found_response(&state));
    };
    if site_setting_update::is_hidden(def) {
        return Ok(super::search::invalid_parameters(
            &state,
            "You are not allowed to change hidden settings",
        ));
    }
    if def.plugin.is_some() {
        return Err(Unsupported("user counts of plugin settings").into());
    }
    let new_value = p.get(&id).and_then(params::scalar).unwrap_or_default();
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let previous = settings.get(&id)?.to_s();
    let mut body = Map::new();
    if site_setting_update::SIDEBAR_DEFAULTS.contains(&id.as_str()) {
        let backfiller =
            crate::sidebar_backfill::Backfiller::new(&mut conn, &id, &previous, &new_value).await?;
        body.insert(
            "user_count".into(),
            json!(backfiller.number_of_users_to_backfill(&mut conn).await?),
        );
    } else if let Some(count) =
        crate::user_option_defaults::count(&mut conn, &id, settings.get(&id)?).await?
    {
        body.insert("user_count".into(), json!(count));
    } else if id.starts_with("default_categories_") || id.starts_with("default_tags_") {
        return Err(Unsupported("user counts of category and tag defaults").into());
    }
    Ok(Json(Value::Object(body)).into_response())
}

/// GET /admin/site_settings: Admin::SiteSettingsController#index, the
/// settings an admin can see (SiteSetting.all_settings by `categories`,
/// `plugin` and `names`) and the default theme.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let p = params::parse(uri.query(), &headers, &body);
    // all_settings calls `filter_categories.any?`: a string raises.
    if p.get("categories")
        .is_some_and(|v| !v.is_array() && !v.is_null())
    {
        return Err(std::io::Error::other(
            "NoMethodError: undefined method 'any?' for a String (categories)",
        )
        .into());
    }
    let filters = crate::admin_site_settings::Filters {
        categories: param_list(&p, "categories"),
        plugin: params::string(&p, "plugin"),
        names: param_list(&p, "names"),
        ..Default::default()
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

/// The settings for the filters and the default theme: with no filters
/// every setting, as the admin site settings page loads them
/// (SiteSetting.findAll); a config area's for its own.
pub async fn document(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    filters: &crate::admin_site_settings::Filters,
) -> Result<(Vec<Value>, Value), AppError> {
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let cx = crate::admin_site_settings::Context {
        defs: &state.site_setting_defs,
        settings,
        i18n: &state.i18n,
        globals: &state.config.globals,
        base_path: state.config.globals.relative_url_root(),
        urls: &urls,
    };
    let site_settings = crate::admin_site_settings::all_settings(&mut *conn, &cx, filters).await?;
    let default_theme = default_theme(&mut *conn, settings).await?;
    Ok((site_settings, default_theme))
}

/// Admin::Config::SiteSettingsController's hidden settings a config area
/// may show.
const CONFIG_ALLOWED_HIDDEN: [&str; 3] = [
    "extended_site_description",
    "about_banner_image",
    "community_owner",
];

/// `Array.wrap(params[key])` of strings, None when blank.
fn param_list(p: &Map<String, Value>, key: &str) -> Option<Vec<String>> {
    let values: Vec<String> = match p.get(key) {
        Some(Value::Array(items)) => items.iter().filter_map(params::scalar).collect(),
        Some(v) => params::scalar(v).into_iter().collect(),
        None => Vec::new(),
    };
    (!values.is_empty()).then_some(values)
}

/// The config areas' filters from their params (`filter_names`,
/// `filter_area`, `plugin`, `categories`), or None for the
/// InvalidParameters of a request naming none of them.
pub fn config_filters(
    state: &AppState,
    p: &Map<String, Value>,
) -> Option<crate::admin_site_settings::Filters> {
    let present = |key: &str| {
        p.get(key).is_some_and(|v| match v {
            Value::Null => false,
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            _ => true,
        })
    };
    let area = params::string(p, "filter_area").filter(|a| !a.is_empty());
    let valid_area = area
        .as_deref()
        .is_some_and(|a| crate::admin_site_settings::valid_area(&state.site_setting_defs, a));
    if !present("plugin") && !present("categories") && !present("filter_names") && !valid_area {
        return None;
    }
    Some(crate::admin_site_settings::Filters {
        categories: param_list(p, "categories"),
        plugin: params::string(p, "plugin").filter(|v| !v.is_empty()),
        names: param_list(p, "filter_names"),
        include_locale_setting: area.as_deref() == Some("localization"),
        area,
        allowed_hidden: Some(
            CONFIG_ALLOWED_HIDDEN
                .iter()
                .map(|s| s.to_string())
                .collect(),
        ),
    })
}

/// GET /admin/config/site_settings: Admin::Config::SiteSettingsController
/// #index, a config area's settings.
pub async fn config_index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let p = params::parse(uri.query(), &headers, &body);
    let Some(filters) = config_filters(&state, &p) else {
        return Ok(super::search::invalid_parameters(
            &state,
            "Discourse::InvalidParameters",
        ));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let (site_settings, _) = document(&mut conn, &state, &settings, &filters).await?;
    Ok(Json(json!({ "site_settings": site_settings })).into_response())
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
