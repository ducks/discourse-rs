//! Port of app/controllers/robots_txt_controller.rb: /robots.txt and
//! /robots-builder.json, built from the crawler settings.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `DISALLOWED_PATHS` (order matters).
const DISALLOWED_PATHS: &[&str] = &[
    "/admin/",
    "/auth/",
    "/assets/js/browser-update*.js",
    "/email/",
    "/session",
    "/user-api-key",
    "/*?api_key*",
    "/*?*api_key*",
];

/// `DISALLOWED_WITH_HEADER_PATHS`
const DISALLOWED_WITH_HEADER_PATHS: &[&str] = &[
    "/badges",
    "/my",
    "/search",
    "/tag/*/l",
    "/g",
    "/t/*/*.rss",
    "/c/*.rss",
];

const HEADER: &str = "# See https://datatracker.ietf.org/doc/rfc9309 for documentation on how to use the robots.txt file\n\
# Google uses the same format as the standard above. More info at https://developers.google.com/search/docs/crawling-indexing/robots/robots_txt\n";

struct Agent {
    name: String,
    disallow: Vec<String>,
}

/// `fetch_default_robots_info`
fn default_robots_info(settings: &SiteSettings, base_path: &str) -> Result<Vec<Agent>, AppError> {
    let with_base = |paths: &[&str]| -> Vec<String> {
        paths.iter().map(|p| format!("{base_path}{p}")).collect()
    };
    let deny_paths_googlebot = with_base(DISALLOWED_PATHS);
    let mut deny_paths = deny_paths_googlebot.clone();
    deny_paths.extend(with_base(DISALLOWED_WITH_HEADER_PATHS));
    let deny_all = vec![format!("{base_path}/")];
    let list = |name: &str| -> Result<Vec<String>, AppError> {
        Ok(settings
            .get(name)?
            .to_s()
            .split('|')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect())
    };
    let mut agents = Vec::new();
    let allowed = list("allowed_crawler_user_agents")?;
    if !allowed.is_empty() {
        for agent in allowed {
            let paths = if agent == "Googlebot" {
                deny_paths_googlebot.clone()
            } else {
                deny_paths.clone()
            };
            agents.push(Agent {
                name: agent,
                disallow: paths,
            });
        }
        agents.push(Agent {
            name: "*".into(),
            disallow: deny_all,
        });
    } else {
        for agent in list("blocked_crawler_user_agents")? {
            agents.push(Agent {
                name: agent,
                disallow: deny_all.clone(),
            });
        }
        agents.push(Agent {
            name: "*".into(),
            disallow: deny_paths,
        });
        agents.push(Agent {
            name: "Googlebot".into(),
            disallow: deny_paths_googlebot,
        });
    }
    Ok(agents)
}

/// GET /robots.txt
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let plain = [(header::CONTENT_TYPE, "text/plain; charset=utf-8")];
    if let Some(overridden) = settings.get("overridden_robots_txt")?.presence() {
        // OVERRIDDEN_HEADER, for admins in the browser (not over the API).
        let is_api = headers.contains_key(crate::session::api_key::HEADER_API_KEY);
        if guardian.is_admin() && !is_api {
            return Ok((
                plain,
                format!(
                    "# This robots.txt file has been customized at /admin/customize/robots\n{overridden}"
                ),
            )
                .into_response());
        }
        return Ok((plain, overridden).into_response());
    }
    let body = original(
        &settings,
        state.config.globals.relative_url_root(),
        &headers,
    )?;
    Ok((plain, body).into_response())
}

/// The robots.txt the settings give: robots_txt/no_index.erb, or
/// index.erb (whitespace and all) with the sitemap at the request's host.
fn original(
    settings: &SiteSettings,
    base_path: &str,
    headers: &HeaderMap,
) -> Result<String, AppError> {
    if !settings.get("allow_index_in_robots_txt")?.truthy() {
        return Ok(format!(
            "User-agent: googlebot\nAllow: {base_path}/\nDisallow: {base_path}/uploads/*\n\nUser-agent: *\nDisallow: {base_path}/\n\n"
        ));
    }
    let mut body = format!("{HEADER}\n");
    if !base_path.is_empty() {
        body.push_str("# This robots.txt file is not used. Please append the content below in the robots.txt file located at the root\n");
    }
    body.push_str("#\n");
    for agent in default_robots_info(settings, base_path)? {
        body.push_str(&format!("User-agent: {}\n", agent.name));
        for path in &agent.disallow {
            body.push_str(&format!("Disallow: {path}\n"));
        }
        body.push_str("\n\n");
    }
    body.push('\n');
    if settings.get("enable_sitemap")?.truthy() && !settings.get("login_required")?.truthy() {
        let host = headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("localhost");
        body.push_str(&format!("Sitemap: http://{host}/sitemap.xml\n"));
    }
    body.push_str("\n\n");
    Ok(body)
}

/// What Admin::RobotsTxtController renders: the robots.txt (the override,
/// else the original) and whether it is overridden.
async fn admin_document(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Value, AppError> {
    let settings =
        SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
    Ok(match settings.get("overridden_robots_txt")?.presence() {
        Some(overridden) => json!({ "robots_txt": overridden, "overridden": true }),
        None => json!({
            "robots_txt": original(&settings, state.config.globals.relative_url_root(), headers)?,
            "overridden": false,
        }),
    })
}

/// GET /admin/customize/robots(.json): Admin::RobotsTxtController#show,
/// or for a page request adminCustomizeRobotsTxt.
pub async fn admin_show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let mut conn = state.pool.acquire().await?;
    let doc = admin_document(&mut conn, &state, &headers).await?;
    if !super::admin_pages::wants_json(&uri, &headers) {
        drop(conn);
        return super::admin_customize::robots_page(&state, &guardian, &headers, &uri, &doc).await;
    }
    Ok(Json(doc).into_response())
}

/// `SiteSetting.overridden_robots_txt = value`, logged by the admin as
/// StaffActionLogger#log_site_setting_change has it.
async fn set_override(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    value: &str,
) -> Result<(), AppError> {
    let actor = guardian
        .user_id()
        .ok_or(Unsupported("an admin without a user"))?;
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let previous = settings.get("overridden_robots_txt")?.to_s();
    if crate::site_setting_update::set(
        &mut tx,
        &state.bus,
        &settings,
        &state.site_setting_defs,
        &state.i18n,
        "overridden_robots_txt",
        &json!(value),
    )
    .await?
    .is_err()
    {
        return Err(Unsupported("an overridden_robots_txt its validation refuses").into());
    }
    crate::site_setting_update::log_site_setting_change(
        &mut tx,
        actor,
        "overridden_robots_txt",
        &previous,
        value,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// PUT /admin/customize/robots(.json): #update.
pub async fn admin_update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    let pairs: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !super::session::csrf_ok(&state, &headers, &pairs, uri.path(), "PUT") {
        return Ok(super::session::bad_csrf());
    }
    // params.require(:robots_txt)
    let value = match p.get("robots_txt") {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        Some(Value::Array(_) | Value::Object(_)) => {
            return Err(Unsupported("a robots_txt that is not a string").into());
        }
        Some(v) if crate::params::scalar(v).is_some_and(|s| !s.is_empty()) => {
            crate::params::scalar(v).unwrap_or_default()
        }
        _ => return Ok(super::accounts::param_missing("robots_txt")),
    };
    set_override(&state, &guardian, &value).await?;
    let mut conn = state.pool.acquire().await?;
    Ok(Json(admin_document(&mut conn, &state, &headers).await?).into_response())
}

/// DELETE /admin/customize/robots(.json): #reset.
pub async fn admin_reset(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    if !super::session::csrf_ok(&state, &headers, &[], uri.path(), "DELETE") {
        return Ok(super::session::bad_csrf());
    }
    set_override(&state, &guardian, "").await?;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let robots = original(
        &settings,
        state.config.globals.relative_url_root(),
        &headers,
    )?;
    Ok(Json(json!({ "robots_txt": robots, "overridden": false })).into_response())
}

/// GET /robots-builder.json
pub async fn builder(State(state): State<AppState>) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let agents: Vec<Value> =
        default_robots_info(&settings, state.config.globals.relative_url_root())?
            .into_iter()
            .map(|a| json!({"name": a.name, "disallow": a.disallow}))
            .collect();
    let mut result = serde_json::Map::new();
    result.insert("header".into(), json!(HEADER));
    result.insert("agents".into(), Value::Array(agents));
    if let Some(overridden) = settings.get("overridden_robots_txt")?.presence() {
        result.insert("overridden".into(), json!(overridden));
    }
    Ok(Json(Value::Object(result)).into_response())
}
