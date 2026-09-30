//! Port of app/controllers/robots_txt_controller.rb: /robots.txt and
//! /robots-builder.json, built from the crawler settings.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

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
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let plain = [(header::CONTENT_TYPE, "text/plain; charset=utf-8")];
    if let Some(overridden) = settings.get("overridden_robots_txt")?.presence() {
        return Ok((plain, overridden).into_response());
    }
    let base_path = state.config.globals.relative_url_root();
    if !settings.get("allow_index_in_robots_txt")?.truthy() {
        // robots_txt/no_index.erb
        let body = format!(
            "User-agent: googlebot\nAllow: {base_path}/\nDisallow: {base_path}/uploads/*\n\nUser-agent: *\nDisallow: {base_path}/\n"
        );
        return Ok((plain, body).into_response());
    }
    // robots_txt/index.erb, whitespace and all.
    let mut body = format!("{HEADER}\n");
    if !base_path.is_empty() {
        body.push_str("# This robots.txt file is not used. Please append the content below in the robots.txt file located at the root\n");
    }
    body.push_str("#\n");
    for agent in default_robots_info(&settings, base_path)? {
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
    Ok((plain, body).into_response())
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
