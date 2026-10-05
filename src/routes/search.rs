//! Port of app/controllers/search_controller.rb#show and #query for
//! anonymous users. Rate limiting (redis) is not ported.

use std::net::SocketAddr;

use askama::Template;
use axum::Json;
use axum::extract::{ConnectInfo, FromRequestParts, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::guardian::Guardian;
use crate::html::Crawler;
use crate::search::{BLURB_LENGTH, Search, SearchArgs, TypeFilter};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// `SearchController::PAGE_LIMIT`
const PAGE_LIMIT: i64 = 10;

#[derive(Deserialize, Default)]
pub struct ShowParams {
    q: Option<String>,
    page: Option<String>,
    search_context: Option<String>,
    context: Option<String>,
    context_id: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct QueryParams {
    term: Option<String>,
    type_filter: Option<String>,
    search_for_id: Option<String>,
    restrict_to_archetype: Option<String>,
    search_context: Option<String>,
    context: Option<String>,
    context_id: Option<String>,
}

/// `Discourse::InvalidParameters` as JSON.
pub(super) fn invalid_parameters(state: &AppState, message: &str) -> Response {
    let text = state
        .i18n
        .t_with("invalid_params", &[("message", message)])
        .unwrap_or_else(|| format!("You supplied invalid parameters to the request: {message}"));
    (
        StatusCode::BAD_REQUEST,
        [(header::CACHE_CONTROL, "no-cache, no-store")],
        Json(json!({"errors": [text], "error_type": "invalid_parameters"})),
    )
        .into_response()
}

/// `Discourse::InvalidAccess` as JSON.
pub(crate) fn invalid_access(state: &AppState) -> Response {
    invalid_access_with(state, "invalid_access")
}

/// `Discourse::InvalidAccess` with a `custom_message`.
pub(crate) fn invalid_access_with(state: &AppState, message_key: &str) -> Response {
    let text = state
        .i18n
        .t(message_key)
        .unwrap_or("You are not permitted to view the requested resource.");
    (
        StatusCode::FORBIDDEN,
        [(header::CACHE_CONTROL, "no-cache, no-store")],
        Json(json!({"errors": [text], "error_type": "invalid_access"})),
    )
        .into_response()
}

/// The peer address when the server was started with connect info.
pub struct Peer(pub Option<SocketAddr>);

impl<S: Send + Sync> FromRequestParts<S> for Peer {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Peer(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0),
        ))
    }
}

/// `request.remote_ip`: the first forwarded address, else the peer.
pub(super) fn remote_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| peer.map(|p| p.ip().to_string()))
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

fn request_context(
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
) -> (String, Option<String>, Option<String>) {
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let session_id = headers
        .get("discourse-pageview-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    (remote_ip(headers, peer), user_agent, session_id)
}

async fn run(state: &AppState, guardian: &Guardian, args: &SearchArgs) -> Result<Value, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let base_path = state.config.globals.relative_url_root();
    let mut search = Search {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian,
        urls: &urls,
        base_path,
        log_cache: &state.search_log_cache,
    };
    // Nil result -> {"grouped_search_result": null}
    Ok(match search.execute(args).await? {
        Some(doc) => doc,
        None => json!({"grouped_search_result": null}),
    })
}

/// `Search.min_length_bypass?`
fn min_length_bypass(term: &str) -> bool {
    let lower = term.to_lowercase();
    if lower == "l" || lower == "r" {
        return true;
    }
    [
        "order:",
        "category:",
        "categories:",
        "tag:",
        "tags:",
        "before:",
        "after:",
        "status:",
        "user:",
        "group:",
        "badge:",
        "in:",
        "with:",
        "#",
        "@",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

/// GET /search(.json)?q=&page=
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ShowParams>,
    headers: HeaderMap,
    Peer(peer): Peer,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    show_response(state, guardian, params, headers, peer, false, Some(uri)).await
}

pub async fn show_json(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ShowParams>,
    headers: HeaderMap,
    Peer(peer): Peer,
) -> Result<Response, AppError> {
    show_response(state, guardian, params, headers, peer, true, None).await
}

async fn show_response(
    state: AppState,
    guardian: Guardian,
    params: ShowParams,
    headers: HeaderMap,
    peer: Option<SocketAddr>,
    json: bool,
    uri: Option<axum::http::Uri>,
) -> Result<Response, AppError> {
    // Before the page's data is read (crate::bus::page_position).
    let bus_position = if json {
        String::new()
    } else {
        crate::bus::page_position(&state.bus).await?
    };
    if params.search_context.is_some() || params.context.is_some() || params.context_id.is_some() {
        return Err(Unsupported("search contexts (user, topic, category, tag)").into());
    }
    let term = params.q.clone().unwrap_or_default();
    {
        let mut conn = state.pool.acquire().await?;
        let settings =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
        let min = settings.get("min_search_term_length")?.to_i().max(0) as usize;
        if !term.is_empty() && term.chars().count() < min && !min_length_bypass(&term) {
            return Ok(invalid_parameters(&state, "q"));
        }
    }
    if term.contains('\0') {
        return Ok(invalid_parameters(&state, "string contains null byte"));
    }
    let mut page = 1;
    if let Some(p) = &params.page {
        let parsed: Option<i64> = p.parse().ok().filter(|n: &i64| n.to_string() == *p);
        let Some(n) = parsed else {
            return Ok(invalid_parameters(&state, "Discourse::InvalidParameters"));
        };
        if n > PAGE_LIMIT {
            return Ok(invalid_parameters(
                &state,
                "page parameter must not be greater than 10",
            ));
        }
        page = n.max(1);
    }
    let (ip_address, user_agent, session_id) = request_context(&headers, peer);
    let args = SearchArgs {
        term: term.clone(),
        type_filter: Some(TypeFilter::Topic),
        full_page: true,
        page,
        blurb_length: 300,
        ip_address,
        user_agent,
        session_id,
    };
    let doc = run(&state, &guardian, &args).await?;
    let noindex = [("x-robots-tag", "noindex")];
    if json {
        return Ok((noindex, Json(doc)).into_response());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let base_path = state.config.globals.relative_url_root();
    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site = crate::html::Site::from_settings(&settings, base_path)?;
    site.viewer = vs.viewer.clone();
    site.bus_position = bus_position;
    let mut page = search_page(&state, site, &term, &doc, page, &mut conn).await?;
    // No crawlable_meta_data on search; the description meta and the
    // default canonical (only the page param survives) remain.
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    page.crawler = crate::html::Crawler::for_request(&urls, &uri.unwrap_or_default(), None)?;
    page.crawler.description = settings.get("site_description")?.to_s();
    let response = (
        noindex,
        Html(page.render().map_err(crate::html::HtmlError::from)?),
    )
        .into_response();
    Ok(crate::html::with_viewer_headers(response, &vs))
}

/// GET /search/query(.json)?term=
pub async fn query(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<QueryParams>,
    headers: HeaderMap,
    Peer(peer): Peer,
) -> Result<Response, AppError> {
    let Some(term) = params.term.clone().filter(|t| !t.trim().is_empty()) else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"errors": ["param is missing or the value is empty or invalid: term"]})),
        )
            .into_response());
    };
    if term.contains('\0') {
        return Ok(invalid_parameters(&state, "string contains null byte"));
    }
    if params.search_context.is_some() || params.context.is_some() || params.context_id.is_some() {
        return Err(Unsupported("search contexts (user, topic, category, tag)").into());
    }
    if params
        .search_for_id
        .as_deref()
        .is_some_and(|v| !v.is_empty())
    {
        return Err(Unsupported("search_for_id (topic id and URL lookup)").into());
    }
    if params
        .restrict_to_archetype
        .as_deref()
        .is_some_and(|v| !v.is_empty())
    {
        return Err(Unsupported("restrict_to_archetype").into());
    }
    let type_filter = match params.type_filter.as_deref().filter(|t| !t.is_empty()) {
        None => None,
        Some("topic") => Some(TypeFilter::Topic),
        Some("category") => Some(TypeFilter::Category),
        Some("user") => Some(TypeFilter::User),
        Some("tags") => Some(TypeFilter::Tags),
        Some("exclude_topics") => Some(TypeFilter::ExcludeTopics),
        Some("private_messages") => return Ok(invalid_access(&state)),
        Some("all_topics") => return Err(Unsupported("type_filter=all_topics").into()),
        Some(_) => return Ok(invalid_access(&state)),
    };
    let (ip_address, user_agent, session_id) = request_context(&headers, peer);
    let args = SearchArgs {
        term,
        type_filter,
        full_page: false,
        page: 1,
        blurb_length: BLURB_LENGTH,
        ip_address,
        user_agent,
        session_id,
    };
    let doc = run(&state, &guardian, &args).await?;
    Ok(([("x-robots-tag", "noindex")], Json(doc)).into_response())
}

#[derive(Template)]
#[template(path = "search.html")]
pub struct SearchPage {
    pub site_title: String,
    pub site_description: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: Crawler,
    pub viewer: Option<crate::html::Viewer>,
    pub bus_position: String,
    pub term: String,
    pub results: Vec<SearchResult>,
    pub searched: bool,
    pub next_url: Option<String>,
}

pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub blurb: String,
    pub username: Option<String>,
    pub created_at: String,
    pub category: Option<crate::html::CategoryBadge>,
}

/// The server-rendered results page from the /search.json document.
async fn search_page(
    state: &AppState,
    site: crate::html::Site,
    term: &str,
    doc: &Value,
    page: i64,
    conn: &mut sqlx::PgConnection,
) -> Result<SearchPage, AppError> {
    let base = site.base_path.clone();
    let cats = crate::html::categories(conn).await?;
    let topics = doc["topics"].as_array().cloned().unwrap_or_default();
    let mut results = Vec::new();
    for p in doc["posts"].as_array().into_iter().flatten() {
        let topic = topics
            .iter()
            .find(|t| t["id"] == p["topic_id"])
            .cloned()
            .unwrap_or(Value::Null);
        let slug = topic["slug"].as_str().unwrap_or("topic");
        let post_number = p["post_number"].as_i64().unwrap_or(1);
        let mut url = format!("{base}/t/{slug}/{}", p["topic_id"]);
        if post_number > 1 {
            url.push_str(&format!("/{post_number}"));
        }
        results.push(SearchResult {
            title: topic["title"].as_str().unwrap_or("").to_string(),
            url,
            blurb: p["blurb"].as_str().unwrap_or("").to_string(),
            username: p["username"].as_str().map(str::to_string),
            created_at: p["created_at"].as_str().unwrap_or("").to_string(),
            category: crate::html::badge(&base, &cats, topic["category_id"].as_i64()),
        });
    }
    let more = doc["grouped_search_result"]["more_full_page_results"] == json!(true);
    let next_url = more.then(|| {
        format!(
            "{base}/search?q={}&page={}",
            form_urlencoded::byte_serialize(term.as_bytes()).collect::<String>(),
            page + 1
        )
    });
    let _ = state;
    Ok(SearchPage {
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        site_description: site.site_description,
        lang: site.lang,
        base_path: site.base_path,
        crawler: Crawler::default(),
        term: term.to_string(),
        results,
        searched: !term.is_empty(),
        next_url,
    })
}
