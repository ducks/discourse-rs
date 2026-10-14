//! Port of app/controllers/search_controller.rb#show and #query for
//! anonymous users, with rate_limit_search.

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
use crate::html::Chrome;
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
    /// The page's query params (FullPageSearchController#queryParams).
    search_type: Option<String>,
    expanded: Option<String>,
    skip_context: Option<String>,
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

/// `invalid_access_with` raised inside a controller, which for topics#show
/// adds the not-found page (403, no current user) as extras.
pub(crate) async fn invalid_access_at(
    state: &AppState,
    message_key: &str,
    method: &axum::http::Method,
    path: &str,
) -> Result<Response, crate::AppError> {
    let slug = match crate::not_found_page::topic_show_slug(path) {
        Some(slug) if method == axum::http::Method::GET => slug,
        _ => return Ok(invalid_access_with(state, message_key)),
    };
    let anonymous = crate::guardian::Guardian::anonymous();
    // The API key refusal names its message (custom_message).
    let custom = (message_key == "invalid_api_credentials").then_some(message_key);
    let extras = super::topics::not_found_extras(state, &anonymous, &slug, true, custom).await?;
    let text = state
        .i18n
        .t(message_key)
        .unwrap_or("You are not permitted to view the requested resource.");
    Ok((
        StatusCode::FORBIDDEN,
        [(header::CACHE_CONTROL, "no-cache, no-store")],
        Json(json!({"errors": [text], "error_type": "invalid_access", "extras": extras})),
    )
        .into_response())
}

/// `rate_limit_search`: per user a minute, or for anonymous searches per
/// IP and site-wide, a second and a minute, in that order. A refusal is
/// a 429 with failed_json and slow_down.
async fn rate_limit_search(
    state: &AppState,
    guardian: &Guardian,
    ip: &str,
) -> Result<Option<Response>, crate::AppError> {
    use crate::rate_limiter::{LimitUser, RateLimiter};
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let limit = |name: &str| s.get(name).map(|v| v.to_i());
    let limiters = match guardian.user() {
        Some(user) => vec![RateLimiter::new(
            Some(LimitUser {
                id: user.id,
                staff: guardian.is_staff(),
            }),
            "search-min",
            limit("rate_limit_search_user")?,
            60,
        )],
        None => vec![
            RateLimiter::new(
                None,
                &format!("search-min-{ip}-per-sec"),
                limit("rate_limit_search_anon_user_per_second")?,
                1,
            ),
            RateLimiter::new(
                None,
                &format!("search-min-{ip}-per-min"),
                limit("rate_limit_search_anon_user_per_minute")?,
                60,
            ),
            RateLimiter::new(
                None,
                "search-min-anon-global-per-sec",
                limit("rate_limit_search_anon_global_per_second")?,
                1,
            ),
            RateLimiter::new(
                None,
                "search-min-anon-global-per-min",
                limit("rate_limit_search_anon_global_per_minute")?,
                60,
            ),
        ],
    };
    if crate::rate_limiter::perform_each(&state.pool, state.config.rate_limits, limiters)
        .await?
        .is_ok()
    {
        return Ok(None);
    }
    Ok(Some(
        (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "failed": "FAILED",
                "message": state.i18n.t("rate_limiter.slow_down").unwrap_or_default(),
            })),
        )
            .into_response(),
    ))
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

fn request_context(
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
) -> Result<(String, Option<String>, Option<String>), crate::remote_ip::IpSpoofAttack> {
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let session_id = headers
        .get("discourse-pageview-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    Ok((
        crate::remote_ip::remote_ip(headers, peer)?,
        user_agent,
        session_id,
    ))
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

/// SearchController#ensure_can_search: `Guardian#can_search?` is a user
/// or allow_anonymous_search. Anonymous HTML (`uri` given) is sent to log
/// in, anything else gets ensure_logged_in's 403.
async fn ensure_can_search(
    state: &AppState,
    guardian: &Guardian,
    headers: &HeaderMap,
    uri: Option<&axum::http::Uri>,
) -> Result<Option<Response>, AppError> {
    if guardian.user_id().is_some() {
        return Ok(None);
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if settings.get("allow_anonymous_search")?.truthy() {
        return Ok(None);
    }
    // request.format.html?: no .json, and an Accept that does not ask
    // for JSON.
    let accepts_json = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|a| a.contains("application/json"));
    Ok(Some(match uri {
        Some(uri) if !accepts_json => {
            super::login_required::redirect_to_login(state, &settings, headers, uri)?
        }
        _ => super::login_required::not_logged_in(state),
    }))
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
    if let Some(refused) = ensure_can_search(&state, &guardian, &headers, uri.as_ref()).await? {
        return Ok(refused);
    }
    // Before the page's data is read (crate::bus::page_position).
    let bus_position = if json {
        String::new()
    } else {
        crate::bus::page_position(&state.bus).await?
    };
    let term = params.q.clone().unwrap_or_default();
    // The page's route searches with the context whenever it has a
    // context_id (FullPageSearchRoute#model reads skip_context off the
    // wrong object), so only a page without a term can carry one.
    let page_context_only = !json
        && params.search_context.is_none()
        && (term.is_empty() || params.context_id.is_none());
    if (params.search_context.is_some() || params.context.is_some() || params.context_id.is_some())
        && !page_context_only
    {
        return Err(Unsupported("search contexts (user, topic, category, tag)").into());
    }
    let search_type = crate::search_view::SearchType::parse(params.search_type.as_deref());
    if !json && search_type == crate::search_view::SearchType::CategoriesTags {
        return Err(Unsupported(
            "the full-page search's categories & tags type (Category.search, tags/filter/search)",
        )
        .into());
    }
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
    let (ip_address, user_agent, session_id) = request_context(&headers, peer)?;
    if let Some(refused) = rate_limit_search(&state, &guardian, &ip_address).await? {
        return Ok(refused);
    }
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
    site.load_chrome(
        &mut conn,
        &state,
        &settings,
        &guardian,
        crate::sidebar::Active::None,
    )
    .await?;
    site.bus_position = bus_position;
    // The search button isn't in the header on the full-page search
    // (ALL_PAGES_EXCLUDED_ROUTES).
    site.chrome.can_search = false;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    // The users type's userSearch({term, limit: 20}) after the route's
    // search.
    let users = if search_type == crate::search_view::SearchType::Users
        && valid_search_term(&term, &settings)?
    {
        let options = crate::user_search::Options {
            limit: Some(20),
            ..Default::default()
        };
        let mut search = match crate::user_search::UserSearch::new(
            &mut conn,
            &settings,
            &guardian,
            crate::ruby::strip(term.strip_prefix('@').unwrap_or(&term)),
            options,
        )
        .await?
        {
            Ok(search) => search,
            Err(_) => return Err(Unsupported("a refused user search on the search page").into()),
        };
        let ids = search.search_ids().await?;
        crate::user_search::found_users(&mut conn, &urls, &guardian, &ids).await?
    } else {
        Vec::new()
    };
    let context = match (params.context.as_deref(), params.skip_context.as_deref()) {
        (Some(kind), skip) => Some((
            kind,
            params.context_id.as_deref().unwrap_or("undefined"),
            // searchContextEnabled
            skip.is_none() || skip == Some("false"),
        )),
        _ => None,
    };
    let context_category_name = match context {
        Some(("category", id, _)) => {
            sqlx::query_scalar("SELECT name FROM categories WHERE id::text = $1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
        }
        _ => None,
    };
    let view = crate::search_view::SearchView {
        q: &term,
        search_type,
        expanded: params.expanded.as_deref() == Some("true"),
        context,
        context_category_name,
        default_order: settings.get("search_default_sort_order")?.to_i(),
        member: guardian.user().is_some(),
        staff: guardian.is_staff(),
        search_active: valid_search_term(&term, &settings)?,
        login_required: settings.get("login_required")?.truthy(),
        doc: Some(&doc),
        users: &users,
        page,
        avatar_size: 0,
        site_url: format!("{}/", urls.base_url()?),
    };
    let mut page = search_page(&state, site, &settings, view, &mut conn).await?;
    // No crawlable_meta_data on search; the description meta and the
    // default canonical (only the page param survives) remain.
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
    if let Some(refused) = ensure_can_search(&state, &guardian, &headers, None).await? {
        return Ok(refused);
    }
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
    let (ip_address, user_agent, session_id) = request_context(&headers, peer)?;
    if let Some(refused) = rate_limit_search(&state, &guardian, &ip_address).await? {
        return Ok(refused);
    }
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
    pub lang: String,
    pub base_path: String,
    pub crawler: Crawler,
    pub viewer: Option<crate::html::Viewer>,
    pub bus_position: String,
    pub chrome: Chrome,
    /// The cleaned term the route's title names (FullPageSearchRoute#titleToken).
    pub term: String,
    pub body: String,
    /// What search-page.js reads: min_search_term_length and its strings.
    pub min_length: i64,
    pub too_short: String,
    pub clear_label: String,
}

/// `isValidSearchTerm`: a term the client searches for.
fn valid_search_term(term: &str, settings: &SiteSettings) -> Result<bool, AppError> {
    let trimmed = term.trim();
    if trimmed.is_empty() {
        return Ok(false);
    }
    static BYPASS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?i)^(l|r)$|order:|category:|categories:|tags?:|before:|after:|status:|user:|group:|badge:|in:|with:|#|@",
        )
        .unwrap()
    });
    if BYPASS.is_match(trimmed) {
        return Ok(true);
    }
    let min = settings.get("min_search_term_length")?.to_i();
    Ok(trimmed.chars().count() as i64 >= min)
}

/// The full-page search as the client renders it (crate::search_view).
async fn search_page(
    state: &AppState,
    mut site: crate::html::Site,
    settings: &SiteSettings,
    mut view: crate::search_view::SearchView<'_>,
    conn: &mut sqlx::PgConnection,
) -> Result<SearchPage, AppError> {
    let categories = crate::topic_list_view::categories(conn).await?;
    let cx = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: &site.base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        member_trust_level: site.viewer.as_ref().map(|v| v.trust_level),
        settings: crate::topic_list_view::ListSettings::load(settings)?,
    };
    view.avatar_size = cx.settings.avatar_size_48;
    let body = crate::search_view::render(&cx, &view);
    let term = crate::search_view::clean_term(view.q, view.default_order).0;
    let too_short = crate::topic_list_view::t(&cx, "search.too_short");
    let clear_label = crate::topic_list_view::t(&cx, "search.clear_search");
    drop(cx);
    // The template's bodyClass.
    site.chrome.body_classes.push_str(" search-page");
    Ok(SearchPage {
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        lang: site.lang,
        base_path: site.base_path,
        crawler: Crawler::default(),
        term,
        body,
        min_length: settings.get("min_search_term_length")?.to_i(),
        too_short,
        clear_label,
    })
}
