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
use crate::search::{BLURB_LENGTH, Search, SearchArgs, SearchContext, TypeFilter};
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
    #[serde(rename = "search_context[type]")]
    context_type: Option<String>,
    #[serde(rename = "search_context[id]")]
    context_id_nested: Option<String>,
}

/// The params lookup_search_context reads, from either action.
#[derive(Default)]
struct ContextParams {
    search_context: Option<String>,
    context_type: Option<String>,
    context_id_nested: Option<String>,
    context: Option<String>,
    context_id: Option<String>,
    skip_context: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct QueryParams {
    pub term: Option<String>,
    pub type_filter: Option<String>,
    pub search_for_id: Option<String>,
    pub restrict_to_archetype: Option<String>,
    pub search_context: Option<String>,
    /// `search_context[type]` and `search_context[id]`, as the client's
    /// searchForTerm sends them.
    #[serde(rename = "search_context[type]")]
    pub context_type: Option<String>,
    #[serde(rename = "search_context[id]")]
    pub context_id_nested: Option<String>,
    pub context: Option<String>,
    pub context_id: Option<String>,
    pub skip_context: Option<String>,
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

/// The search, or the `Discourse::InvalidAccess` it raises (a personal
/// message search the viewer may not run).
async fn run(
    state: &AppState,
    guardian: &Guardian,
    args: &SearchArgs,
) -> Result<Result<Value, Response>, AppError> {
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
    Ok(match search.execute(args).await {
        Ok(Some(doc)) => Ok(doc),
        Ok(None) => Ok(json!({"grouped_search_result": null})),
        Err(crate::search::SearchError::InvalidAccess) => Err(invalid_access(state)),
        Err(e) => return Err(e.into()),
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
    let context = match lookup_search_context(
        &state,
        &guardian,
        &ContextParams {
            search_context: params.search_context.clone(),
            context_type: params.context_type.clone(),
            context_id_nested: params.context_id_nested.clone(),
            context: params.context.clone(),
            context_id: params.context_id.clone(),
            skip_context: params.skip_context.clone(),
        },
    )
    .await?
    {
        Ok(context) => context,
        Err(refused) if json => return Ok(refused),
        Err(_) => {
            return Err(Unsupported(
                "the full-page search's error page for a search context it refuses",
            )
            .into());
        }
    };
    let (ip_address, user_agent, session_id) = request_context(&headers, peer)?;
    if let Some(refused) = rate_limit_search(&state, &guardian, &ip_address).await? {
        return Ok(refused);
    }
    let mut type_filter = Some(TypeFilter::Topic);
    let context = match context {
        Some((context, forced)) => {
            if forced.is_some() {
                type_filter = forced;
            }
            Some(context)
        }
        None => None,
    };
    let args = SearchArgs {
        term: term.clone(),
        type_filter,
        full_page: true,
        page,
        blurb_length: 300,
        ip_address,
        user_agent,
        session_id,
        log: true,
        context,
        search_for_id: false,
        restrict_to_archetype: None,
    };
    let doc = match run(&state, &guardian, &args).await? {
        Ok(doc) => doc,
        Err(refused) if json => return Ok(refused),
        Err(_) => {
            return Err(Unsupported(
                "the full-page search's 403 page (a personal message search the viewer may not run)",
            )
            .into());
        }
    };
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
    Ok(
        match run_query(&state, &guardian, &params, &headers, peer).await? {
            Ok(doc) => ([("x-robots-tag", "noindex")], Json(doc)).into_response(),
            Err(refused) => refused,
        },
    )
}

/// SearchController#query up to its serialized result, or the response
/// that refuses it: what /search/query answers and the header's search
/// menu renders (crate::search_menu_view).
pub(crate) async fn run_query(
    state: &AppState,
    guardian: &Guardian,
    params: &QueryParams,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
) -> Result<Result<Value, Response>, AppError> {
    if let Some(refused) = ensure_can_search(state, guardian, headers, None).await? {
        return Ok(Err(refused));
    }
    let Some(term) = params.term.clone().filter(|t| !t.trim().is_empty()) else {
        return Ok(Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"errors": ["param is missing or the value is empty or invalid: term"]})),
        )
            .into_response()));
    };
    if term.contains('\0') {
        return Ok(Err(invalid_parameters(state, "string contains null byte")));
    }
    let looked_up = match lookup_search_context(
        state,
        guardian,
        &ContextParams {
            search_context: params.search_context.clone(),
            context_type: params.context_type.clone(),
            context_id_nested: params.context_id_nested.clone(),
            context: params.context.clone(),
            context_id: params.context_id.clone(),
            skip_context: params.skip_context.clone(),
        },
    )
    .await?
    {
        Ok(context) => context,
        Err(refused) => return Ok(Err(refused)),
    };
    let mut type_filter = match params.type_filter.as_deref().filter(|t| !t.is_empty()) {
        None => None,
        Some("topic") => Some(TypeFilter::Topic),
        Some("category") => Some(TypeFilter::Category),
        Some("user") => Some(TypeFilter::User),
        Some("tags") => Some(TypeFilter::Tags),
        Some("exclude_topics") => Some(TypeFilter::ExcludeTopics),
        Some("private_messages") => Some(TypeFilter::PrivateMessages),
        Some("all_topics") => Some(TypeFilter::AllTopics),
        Some(_) => return Ok(Err(invalid_access(state))),
    };
    // A private messages context brings its type filter.
    let context = looked_up.map(|(context, forced)| {
        if forced.is_some() {
            type_filter = forced;
        }
        context
    });
    let (ip_address, user_agent, session_id) = request_context(headers, peer)?;
    if let Some(refused) = rate_limit_search(state, guardian, &ip_address).await? {
        return Ok(Err(refused));
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
        log: true,
        context,
        search_for_id: params
            .search_for_id
            .as_deref()
            .is_some_and(|v| !v.is_empty()),
        restrict_to_archetype: params
            .restrict_to_archetype
            .clone()
            .filter(|v| !v.is_empty()),
    };
    run(state, guardian, &args).await
}

/// SearchController#lookup_search_context: `search_context[type]` and
/// `[id]`, or `context` and `context_id`, unless `skip_context=true`. A
/// topic is the one context ported; a missing or unseen one is invalid
/// access, an unknown type or a blank id invalid parameters.
async fn lookup_search_context(
    state: &AppState,
    guardian: &Guardian,
    params: &ContextParams,
) -> Result<Result<Option<(SearchContext, Option<TypeFilter>)>, Response>, AppError> {
    if params.skip_context.as_deref() == Some("true") {
        return Ok(Ok(None));
    }
    let (kind, id) = if params.context_type.is_some() || params.context_id_nested.is_some() {
        (
            params.context_type.clone(),
            params.context_id_nested.clone(),
        )
    } else if params.search_context.is_some() {
        // A plain string where Rails reads a hash (its TypeError).
        return Err(Unsupported("search_context given as a string").into());
    } else if let (Some(kind), Some(id)) = (&params.context, &params.context_id) {
        (Some(kind.clone()), Some(id.clone()))
    } else {
        return Ok(Ok(None));
    };
    let kind = kind.unwrap_or_default();
    if !["user", "topic", "category", "private_messages", "tag"].contains(&kind.as_str()) {
        return Ok(Err(invalid_parameters(state, "search_context")));
    }
    let id = id.unwrap_or_default();
    if id.trim().is_empty() {
        return Ok(Err(invalid_parameters(state, "search_context")));
    }
    let mut conn = state.pool.acquire().await?;
    if kind == "user" || kind == "private_messages" {
        // A user by username; any user is visible (ensure_can_see!).
        let user_id: Option<i32> =
            sqlx::query_scalar("SELECT id FROM users WHERE username_lower = $1 LIMIT 1")
                .bind(id.to_lowercase())
                .fetch_optional(&mut *conn)
                .await?;
        let Some(user_id) = user_id else {
            return Ok(Err(invalid_access(state)));
        };
        let type_filter = (kind == "private_messages").then_some(TypeFilter::PrivateMessages);
        return Ok(Ok(Some((SearchContext::User(user_id), type_filter))));
    }
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if kind == "category" {
        // Category.find_by(id: id.to_i), then can_see_category?.
        let row: Option<(i32, bool, bool)> = sqlx::query_as(
            "SELECT id, read_restricted, \
                    btrim(COALESCE(email_in, '')) <> '' AND email_in_allow_strangers \
             FROM categories WHERE id = $1",
        )
        .bind(crate::ruby::to_i(&id))
        .fetch_optional(&mut *conn)
        .await?;
        let Some((category_id, read_restricted, strangers_mail_in)) = row else {
            return Ok(Err(invalid_access(state)));
        };
        let admin_sees_all = guardian.is_admin()
            && !settings
                .get("suppress_secured_categories_from_admin")?
                .truthy();
        let staged_mail_in = guardian.user().is_some_and(|u| u.staged) && strangers_mail_in;
        if read_restricted
            && !admin_sees_all
            && !staged_mail_in
            && !guardian
                .secure_category_ids(&mut conn, &settings)
                .await?
                .contains(&category_id)
        {
            return Ok(Err(invalid_access(state)));
        }
        return Ok(Ok(Some((SearchContext::Category(category_id), None))));
    }
    if kind == "tag" {
        // A tag hidden from the viewer is no context; Tag.where_name(id).first,
        // then can_see_tag?.
        let hidden = crate::tagging::hidden_tag_names(&mut conn, guardian).await?;
        if hidden.contains(&id) {
            return Ok(Err(invalid_access(state)));
        }
        let tag: Option<(i32, String)> = sqlx::query_as(
            "SELECT id, name FROM tags WHERE lower(tags.name) IN ($1) ORDER BY tags.id LIMIT 1",
        )
        .bind(id.to_lowercase())
        .fetch_optional(&mut *conn)
        .await?;
        let Some((tag_id, name)) = tag else {
            return Ok(Err(invalid_access(state)));
        };
        if !settings.get("tagging_enabled")?.truthy() || hidden.contains(&name) {
            return Ok(Err(invalid_access(state)));
        }
        return Ok(Ok(Some((SearchContext::Tag(tag_id), None))));
    }
    // `to_i`: the leading digits, 0 without any.
    let digits: String = id
        .trim_start()
        .chars()
        .enumerate()
        .take_while(|(i, c)| c.is_ascii_digit() || (*i == 0 && (*c == '-' || *c == '+')))
        .map(|(_, c)| c)
        .collect();
    let topic_id: i32 = digits.parse().unwrap_or(0);
    let archetype: Option<String> =
        sqlx::query_scalar("SELECT archetype FROM topics WHERE id = $1 AND deleted_at IS NULL")
            .bind(topic_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(archetype) = archetype else {
        return Ok(Err(invalid_access(state)));
    };
    if guardian
        .can_see_topic_ids(&mut conn, &settings, &[topic_id])
        .await?
        .is_empty()
    {
        return Ok(Err(invalid_access(state)));
    }
    Ok(Ok(Some((
        SearchContext::Topic(crate::search::TopicContext {
            id: topic_id,
            private_message: archetype == "private_message",
        }),
        None,
    ))))
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
    // ALL_PAGES_EXCLUDED_ROUTES: full-page-search.
    site.chrome.hide_search_button = true;
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

/// `to_i`: the leading integer, 0 without one.
fn ruby_to_i(s: &str) -> i64 {
    let s = s.trim_start();
    let (sign, rest) = match s.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, s.strip_prefix('+').unwrap_or(s)),
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse::<i64>().map(|n| sign * n).unwrap_or(0)
}

/// POST /search/click: SearchController#click, the result a search log's
/// searcher followed (the member's own log, or an anonymous one from
/// their IP).
pub async fn click(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Peer(peer): Peer,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    let form: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !super::session::csrf_ok(&state, &headers, &form, uri.path(), "POST") {
        return Ok(super::session::bad_csrf());
    }
    let get = |name: &str| {
        form.iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
    };
    let mut values = Vec::new();
    for name in ["search_log_id", "search_result_type", "search_result_id"] {
        match get(name) {
            Some(v) => values.push(v),
            None => {
                return Ok((
                    StatusCode::BAD_REQUEST,
                    Json(json!({"errors": [format!(
                        "param is missing or the value is empty or invalid: {name}"
                    )]})),
                )
                    .into_response());
            }
        }
    }
    // SearchLog.search_result_types
    let result_type = match values[1].to_lowercase().as_str() {
        "topic" => Some(1),
        "user" => Some(2),
        "category" => Some(3),
        "tag" => Some(4),
        _ => None,
    };
    if let Some(result_type) = result_type {
        let mut conn = state.pool.acquire().await?;
        let result_id: Option<i64> = if result_type == 4 {
            sqlx::query_scalar::<_, i32>(
                "SELECT id FROM tags WHERE lower(name) = lower($1) LIMIT 1",
            )
            .bind(&values[2])
            .fetch_optional(&mut *conn)
            .await?
            .map(i64::from)
        } else {
            Some(ruby_to_i(&values[2]))
        };
        let log_id = ruby_to_i(&values[0]);
        match guardian.user_id() {
            Some(user_id) => {
                sqlx::query(
                    "UPDATE search_logs SET search_result_type = $3, search_result_id = $4 \
                     WHERE id = $1 AND user_id = $2",
                )
                .bind(log_id)
                .bind(user_id)
                .bind(result_type)
                .bind(result_id)
                .execute(&mut *conn)
                .await?;
            }
            None => {
                let ip = crate::remote_ip::remote_ip(&headers, peer)?;
                sqlx::query(
                    "UPDATE search_logs SET search_result_type = $3, search_result_id = $4 \
                     WHERE id = $1 AND ip_address = $2::inet",
                )
                .bind(log_id)
                .bind(ip)
                .bind(result_type)
                .bind(result_id)
                .execute(&mut *conn)
                .await?;
            }
        }
    }
    Ok(Json(json!({"success": "OK"})).into_response())
}
