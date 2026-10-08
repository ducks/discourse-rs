//! Port of app/controllers/list_controller.rb (latest) with
//! lib/topic_query_params.rb.

use askama::Template;
use axum::Json;
use axum::extract::{Path, Query, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::category::Category;
use crate::guardian::Guardian;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_list::TopicListSerializer;
use crate::topic_query::{Filter, Options, TopicQuery};
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// The TopicQuery.public_valid_options ported so far, and the rest by
/// name (only topic_ids is checked among them).
#[derive(Deserialize, Default)]
pub struct ListParams {
    pub(super) page: Option<String>,
    pub(super) per_page: Option<String>,
    pub(super) order: Option<String>,
    pub(super) ascending: Option<String>,
    /// top lists only; not a TopicQuery option, so never echoed unless given
    pub(super) period: Option<String>,
    /// Every other param, for the ones checked by name only.
    #[serde(flatten)]
    pub(super) rest: std::collections::HashMap<String, String>,
}

/// Discourse::InvalidParameters from a list's params: JSON as Rails
/// renders it; the HTML error page is not ported, so plain text.
pub(super) fn invalid_list_params(state: &AppState, message: &str, json: bool) -> Response {
    if json {
        super::search::invalid_parameters(state, message)
    } else {
        (StatusCode::BAD_REQUEST, message.to_string()).into_response()
    }
}

/// `build_topic_list_options` + `TopicQuery.validate?`: a bad value is
/// Discourse::InvalidParameters (its message the inner error).
pub(super) fn build_options(
    params: &ListParams,
    settings: &SiteSettings,
) -> Result<Result<Options, String>, Unsupported> {
    // topic_ids must be a string or an array: topic_ids[key]= is a hash.
    for key in params.rest.keys() {
        match key.as_str() {
            "topic_ids" | "topic_ids[]" => return Err(Unsupported("the topic_ids list filter")),
            k if k.starts_with("topic_ids[") => return Ok(Err("topic_ids".into())),
            _ => {}
        }
    }
    Ok(validated_options(params, settings))
}

fn validated_options(params: &ListParams, settings: &SiteSettings) -> Result<Options, String> {
    let invalid = |name: &str| name.to_string();
    let mut options = Options {
        no_definitions: !settings
            .get("show_category_definitions_in_topic_lists")
            .map(|v| v.truthy())
            .unwrap_or(false),
        ..Options::default()
    };
    if let Some(page) = params.page.as_deref().filter(|p| !p.is_empty()) {
        let max = settings
            .get("max_topic_query_page_param")
            .map(|v| v.to_i())
            .unwrap_or(2000);
        let digits = page.strip_prefix('-').unwrap_or(page);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid("page"));
        }
        let n: i64 = page.parse().map_err(|_| invalid("page"))?;
        if !(0..=max).contains(&n) {
            return Err(invalid("page"));
        }
        options.page = n;
    }
    if let Some(per_page) = params.per_page.as_deref().filter(|p| !p.is_empty()) {
        let n: i64 = per_page.parse().map_err(|_| invalid("per_page"))?;
        if !(1..=100).contains(&n) {
            return Err(invalid("per_page"));
        }
        options.per_page = Some(n);
    }
    if let Some(order) = params.order.as_deref().filter(|o| !o.is_empty()) {
        options.order = Some(order.to_string());
    }
    if let Some(ascending) = params.ascending.as_deref().filter(|a| !a.is_empty()) {
        options.ascending = match ascending {
            "true" => true,
            "false" => false,
            _ => return Err(invalid("ascending")),
        };
    }
    Ok(options)
}

/// `construct_url_with(:next, opts)`: `latest_path(opts + page+1)`, params
/// sorted by name, `.json` dropped.
pub(super) fn next_url(
    list_path: &str,
    params: &ListParams,
    options: &Options,
    top: bool,
) -> String {
    let mut pairs: Vec<(&str, String)> = Vec::new();
    if let Some(a) = params.ascending.as_deref().filter(|a| !a.is_empty()) {
        pairs.push(("ascending", a.to_string()));
    }
    if options.no_definitions {
        pairs.push(("no_definitions", "true".into()));
    }
    if options.no_subcategories {
        pairs.push(("no_subcategories", "true".into()));
    }
    if let Some(o) = &options.order {
        pairs.push(("order", o.clone()));
    }
    pairs.push(("page", (options.page + 1).to_string()));
    if let Some(p) = options.per_page.filter(|_| true) {
        pairs.push(("per_page", p.to_string()));
    }
    if top && let Some(period) = params.period.as_deref().filter(|p| !p.is_empty()) {
        pairs.push(("period", period.to_string()));
    }
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    let query: Vec<String> = pairs.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{list_path}?{}", query.join("&"))
}

/// What TagsController does differently from ListController when it
/// builds a list: `tags`/`no_tags`, per_page clamped to 30, the top period
/// straight from the param or top_page_default_timeframe, and the
/// next-page URL carrying `match_all_tags` and `tags[]`.
pub(super) struct TagListRequest {
    pub(super) tags: Vec<String>,
    pub(super) no_tags: bool,
}

/// The /latest document, shared by the JSON and HTML responses.
async fn list_document(
    state: &AppState,
    guardian: &Guardian,
    params: &ListParams,
    category: Option<&Category>,
    no_subcategories: bool,
    kind: ListKind,
    list_path: &str,
) -> Result<Result<(serde_json::Value, SiteSettings), String>, AppError> {
    list_document_for(
        state,
        guardian,
        params,
        ListScope {
            category,
            no_subcategories,
            kind,
            list_path,
            tag_request: None,
        },
    )
    .await
}

/// What a list is scoped to, beyond the query params.
pub(super) struct ListScope<'a> {
    pub(super) category: Option<&'a Category>,
    pub(super) no_subcategories: bool,
    pub(super) kind: ListKind,
    pub(super) list_path: &'a str,
    pub(super) tag_request: Option<&'a TagListRequest>,
}

pub(super) async fn list_document_for(
    state: &AppState,
    guardian: &Guardian,
    params: &ListParams,
    scope: ListScope<'_>,
) -> Result<Result<(serde_json::Value, SiteSettings), String>, AppError> {
    let ListScope {
        category,
        no_subcategories,
        kind,
        list_path,
        tag_request,
    } = scope;
    let t0 = std::time::Instant::now();
    let mut conn = state.pool.acquire().await?;
    let t_acquire = t0.elapsed();
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let t_settings = t0.elapsed();
    let mut options = match build_options(params, &settings)? {
        Ok(o) => o,
        Err(e) => return Ok(Err(e)),
    };
    let mut for_period: Option<String> = None;
    if let ListKind::Top = kind {
        let period = match params.period.as_deref().filter(|p| !p.is_empty()) {
            Some(p) => p.to_string(),
            None if tag_request.is_some() => settings.get("top_page_default_timeframe")?.to_s(),
            None => best_period_for(&mut conn, &settings, category.map(|c| c.id)).await?,
        };
        if !crate::topic_query::PERIODS.contains(&period.as_str()) {
            return Ok(Err(
                "Invalid period. Valid periods are all, yearly, quarterly, monthly, weekly, daily"
                    .into(),
            ));
        }
        // top_#{period}: per_page defaults to topics_per_period_in_top_page and
        // is always carried in the next-page URL. Tag lists keep the default.
        if options.per_page.is_none() && tag_request.is_none() {
            options.per_page = Some(settings.get("topics_per_period_in_top_page")?.to_i());
        }
        for_period = Some(period);
    }
    // no_definitions only when `params[:category].blank? && filter == :latest`.
    if kind != ListKind::Latest {
        options.no_definitions = false;
    }
    if let Some(c) = category {
        // set_category puts the id in params[:category], which is why
        // no_definitions is never set for category lists.
        options.category_id = Some(c.id);
        options.no_definitions = false;
        options.no_subcategories = no_subcategories;
    }
    if let Some(t) = tag_request {
        options.no_definitions = false;
        options.no_subcategories = no_subcategories;
        options.per_page = options.per_page.map(|p| p.clamp(1, 30));
        options.tags = t.tags.clone();
        options.no_tags = t.no_tags;
    }
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };

    let mut query = TopicQuery {
        conn: &mut conn,
        settings: &settings,
        guardian,
        options: options.clone(),
        category: Default::default(),
        tags: Default::default(),
        filter: Default::default(),
        user: Default::default(),
    };
    let t_query0 = t0.elapsed();
    let list = query.list(kind.filter(for_period.as_deref())).await?;

    let t_query = t0.elapsed();
    let more = match tag_request {
        Some(t) => super::tags::next_url(list_path, params, &options, t),
        None => next_url(list_path, params, &options, kind == ListKind::Top),
    };
    let mut json = TopicListSerializer {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian,
        urls: &urls,
        more_topics_url: Some(more),
        category_id: options.category_id,
        group_id: None,
        prefetched: Default::default(),
    }
    .serialize(&list)
    .await?;
    if let Some(period) = &for_period {
        // TopicList#for_period, emitted after more_topics_url.
        if let Some(list) = json["topic_list"].as_object_mut() {
            let entries: Vec<(String, serde_json::Value)> =
                list.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            list.clear();
            for (k, v) in entries {
                list.insert(k.clone(), v);
                let has_more = list.contains_key("more_topics_url");
                if (k == "filter" && !has_more) || k == "more_topics_url" {
                    list.insert("for_period".into(), serde_json::json!(period));
                }
            }
        }
    }
    // RUST_LOG=discourse_rs=debug shows where a list request spends its time.
    tracing::debug!(
        acquire = ?t_acquire,
        settings = ?(t_settings - t_acquire),
        query = ?(t_query - t_query0),
        serialize = ?(t0.elapsed() - t_query),
        total = ?t0.elapsed(),
        "list phases"
    );
    Ok(Ok((json, settings)))
}

/// GET /latest.json
pub async fn latest_json(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ListParams>,
) -> Result<Response, AppError> {
    let list_path = format!("{}/latest", state.config.globals.relative_url_root());
    match list_document(
        &state,
        &guardian,
        &params,
        None,
        false,
        ListKind::Latest,
        &list_path,
    )
    .await?
    {
        Ok((json, _)) => Ok(Json(json).into_response()),
        Err(message) => Ok(invalid_list_params(&state, &message, true)),
    }
}

/// GET / and GET /latest: the server-rendered list. The "more" link
/// points at the HTML page.
pub async fn latest(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    // Before the page's data is read (crate::bus::page_position).
    let bus_position = crate::bus::page_position(&state.bus).await?;
    let list_path = format!("{}/latest", state.config.globals.relative_url_root());
    // Also before: topics bumped after it are news to this page.
    let live_since = crate::clock::now().timestamp_millis().to_string();
    let (json, settings) = match list_document(
        &state,
        &guardian,
        &params,
        None,
        false,
        ListKind::Latest,
        &list_path,
    )
    .await?
    {
        Ok(doc) => doc,
        Err(message) => return Ok(invalid_list_params(&state, &message, false)),
    };
    let mut conn = state.pool.acquire().await?;
    let base_path = state.config.globals.relative_url_root();
    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site = crate::html::Site::from_settings(&settings, base_path)?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(
        &state,
        &settings,
        &guardian,
        crate::sidebar::Active::Discovery,
    )
    .await?;
    site.bus_position = bus_position;
    let mut page =
        crate::html::latest_page(&mut conn, site, &json, &state.i18n, &settings, false).await?;
    page.banner = crate::html::welcome_banner(
        &state.i18n,
        &settings,
        page.viewer.as_ref(),
        base_path,
        "latest",
    )?;
    page.breadcrumbs = crate::html::breadcrumbs(&state.i18n, &settings)?;
    page.nav = crate::html::nav_items(
        &state.i18n,
        &settings,
        base_path,
        "latest",
        page.chrome.tracking.as_ref(),
    )?;
    if page.chrome.tracking.is_some() {
        page.chrome.live_param("nav", "latest");
    }
    page.live_filter = "latest".to_string();
    page.live_since = live_since;
    // Strip `no_definitions`, which is what the JSON list carries around
    // but the HTML list applies on its own.
    page.more_url = page.more_url.map(|u| {
        u.replace("no_definitions=true&", "")
            .replace("?no_definitions=true", "")
    });
    page.crawler = list_crawler(&mut conn, &state, &settings, &uri, None, None).await?;
    let body = page.render().map_err(crate::html::HtmlError::from)?;
    Ok(crate::html::crawler_response(
        body,
        &page.crawler,
        &settings,
        &vs,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_url_sorts_params_and_increments_page() {
        let p = ListParams {
            per_page: Some("10".into()),
            ..ListParams::default()
        };
        let o = Options {
            page: 2,
            per_page: Some(10),
            no_definitions: true,
            ..Options::default()
        };
        assert_eq!(
            next_url("/latest", &p, &o, false),
            "/latest?no_definitions=true&page=3&per_page=10"
        );
        let o = Options {
            no_definitions: true,
            ..Options::default()
        };
        assert_eq!(
            next_url("/forum/latest", &ListParams::default(), &o, false),
            "/forum/latest?no_definitions=true&page=1"
        );
    }
}

/// `/c/*category_slug_path_with_id(.json)` -> list#category_default,
/// `.../none` -> list#category_none_default (both in the category's
/// default view), and `.../l/<filter>` with or without `/none` for the
/// ported filters.
pub async fn category(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(path): Path<String>,
    Query(params): Query<ListParams>,
    RawQuery(raw_query): RawQuery,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    let (path, json) = match path.strip_suffix(".json") {
        Some(p) => (p.to_string(), true),
        None => (path.clone(), false),
    };
    // Before the page's data is read (crate::bus::page_position).
    let bus_position = if json {
        String::new()
    } else {
        crate::bus::page_position(&state.bus).await?
    };
    // `.../none` (no_subcategories), `.../l/{latest,top,hot}`, or both.
    let mut rest = path.as_str();
    let mut kind = ListKind::Latest;
    let mut action = String::new();
    let mut filtered = false;
    for (name, k) in ListKind::ALL {
        let suffix = format!("/l/{name}");
        if let Some(p) = rest.strip_suffix(suffix.as_str()) {
            rest = p;
            kind = k;
            action = suffix;
            filtered = true;
            break;
        }
    }
    let none = match rest.strip_suffix("/none") {
        Some(p) => {
            rest = p;
            action = format!("/none{action}");
            true
        }
        None => false,
    };
    let slug_path = rest.to_string();
    if slug_path.contains("/l/") || slug_path.ends_with("/all") {
        return Err(crate::Unsupported("unknown category list filter").into());
    }
    if let Some(response) = ensure_logged_in(&state, &guardian, kind) {
        return Ok(response);
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let max_nesting = settings.get("max_category_nesting")?.to_i();
    let base_path = state.config.globals.relative_url_root().to_string();

    // set_category: missing or unreadable -> 404, before any redirect.
    let category = Category::find_by_slug_path_with_id(&mut conn, &slug_path, max_nesting).await?;
    let Some(category) = category else {
        return Ok(super::topics::not_found_response(&state));
    };
    if category.read_restricted
        && !guardian
            .secure_category_ids(&mut conn, &settings)
            .await?
            .contains(&category.id)
    {
        return Ok(super::topics::not_found_response(&state));
    }
    // slugs_do_not_match: a wrong slug path redirects, query string kept.
    let real_slug = category.full_slug(&mut conn).await?;
    if slug_path.trim_matches('/') != real_slug {
        let query = raw_query.map(|q| format!("?{q}")).unwrap_or_default();
        let format = if json { ".json" } else { "" };
        let host = headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("localhost");
        let location = format!("http://{host}{base_path}/c/{real_slug}{action}{format}{query}");
        return Ok((
            StatusCode::MOVED_PERMANENTLY,
            [(header::LOCATION, location)],
        )
            .into_response());
    }
    drop(conn);

    // category_default_view: the category's default_view when anonymous users
    // may list it (latest, top, hot), else latest. The URL stays the
    // category_default one.
    if !filtered {
        // Only a filter the user may list; anonymous users get latest,
        // top, hot (and categories, which isn't a topic list).
        kind = match category
            .default_view
            .as_deref()
            .and_then(ListKind::from_name)
        {
            Some(k) if guardian.is_authenticated() || !k.requires_login() => k,
            _ => ListKind::Latest,
        };
    }

    let list_path = format!("{base_path}/c/{real_slug}{action}");
    let (doc, settings) = match list_document(
        &state,
        &guardian,
        &params,
        Some(&category),
        none,
        kind,
        &list_path,
    )
    .await?
    {
        Ok(doc) => doc,
        Err(message) => return Ok(invalid_list_params(&state, &message, json)),
    };
    if json {
        return Ok(Json(doc).into_response());
    }
    let mut conn = state.pool.acquire().await?;
    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site = crate::html::Site::from_settings(&settings, &base_path)?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(
        &state,
        &settings,
        &guardian,
        crate::sidebar::Active::Category(category.id),
    )
    .await?;
    site.bus_position = bus_position;
    let mut page =
        crate::html::latest_page(&mut conn, site, &doc, &state.i18n, &settings, true).await?;
    page.heading = Some(
        crate::html::category_heading(&mut conn, &base_path, &category, params.page.is_none())
            .await?,
    );
    // canonical_url "#{Discourse.base_url_no_prefix}#{.url}"
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let canonical = format!(
        "{}{}",
        urls.base_url_no_prefix()?,
        category.url(&mut conn, &base_path).await?
    );
    let description = crate::categories::description_plain_text(category.description.as_deref())?
        .map(|d| html_escape::decode_html_entities(&d).into_owned())
        .filter(|d| !d.is_empty());
    let meta = Some((category.name.clone(), description.unwrap_or_default()));
    page.crawler = list_crawler(&mut conn, &state, &settings, &uri, Some(canonical), meta).await?;
    let body = page.render().map_err(crate::html::HtmlError::from)?;
    Ok(crate::html::crawler_response(
        body,
        &page.crawler,
        &settings,
        &vs,
    )?)
}

/// GET /categories(.json) -> categories#index for anonymous users.
pub async fn categories(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<CategoriesParams>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    categories_response(state, guardian, params, false, headers, Some(uri)).await
}

pub async fn categories_json(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<CategoriesParams>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    categories_response(state, guardian, params, true, headers, None).await
}

async fn categories_response(
    state: AppState,
    guardian: Guardian,
    params: CategoriesParams,
    json: bool,
    headers: HeaderMap,
    uri: Option<axum::http::Uri>,
) -> Result<Response, AppError> {
    // Before the page's data is read (crate::bus::page_position).
    let bus_position = if json {
        String::new()
    } else {
        crate::bus::page_position(&state.bus).await?
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let has_parent = params.parent_category_id.is_some();
    let doc = crate::category_list::CategoryList {
        secure_ids: Vec::new(),
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        include_topics_param: params.include_topics.is_some(),
        include_subcategories_param: params.include_subcategories.as_deref() == Some("true"),
        parent_category_param: params.parent_category_id.filter(|p| !p.trim().is_empty()),
        page: params
            .page
            .as_deref()
            .map(crate::ruby::to_i)
            .filter(|p| *p > 0)
            .unwrap_or(1),
    }
    .json()
    .await?;
    if json {
        return Ok(Json(doc).into_response());
    }
    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site =
        crate::html::Site::from_settings(&settings, state.config.globals.relative_url_root())?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(
        &state,
        &settings,
        &guardian,
        crate::sidebar::Active::Categories,
    )
    .await?;
    site.bus_position = bus_position;
    let mut page = crate::html::categories_page(&mut conn, &state.i18n, site, &doc).await?;
    let base_path = state.config.globals.relative_url_root();
    let style = settings.get("desktop_category_page_style")?.to_s();
    let created_order = style == "categories_and_latest_topics_created_date";
    if (style == "categories_and_latest_topics" || created_order) && !has_parent {
        // CategoriesController#fetch_topic_list: latest, topics_per_page.
        let latest_params = ListParams {
            per_page: Some(
                topics_per_page(&mut conn, &settings, &guardian)
                    .await?
                    .to_string(),
            ),
            order: created_order.then(|| "created".to_string()),
            ..Default::default()
        };
        let latest = match list_document(
            &state,
            &guardian,
            &latest_params,
            None,
            false,
            ListKind::Latest,
            &format!("{base_path}/latest"),
        )
        .await?
        {
            Ok((latest, _)) => latest,
            Err(message) => return Ok(invalid_list_params(&state, &message, false)),
        };
        let categories = crate::topic_list_view::categories(&mut conn).await?;
        let cx = crate::topic_list_view::ListContext {
            i18n: &state.i18n,
            base_path,
            now: crate::clock::now(),
            categories: &categories,
            expand_all_pinned: false,
            member_trust_level: page.viewer.as_ref().map(|v| v.trust_level),
            settings: crate::topic_list_view::ListSettings::load(&settings)?,
        };
        // The viewer's muted categories (Category#notification_level 0).
        let muted: Vec<i64> = match guardian.user_id() {
            Some(uid) => sqlx::query_scalar(
                "SELECT category_id::int8 FROM category_users WHERE user_id = $1 AND notification_level = 0",
            )
            .bind(uid)
            .fetch_all(&mut *conn)
            .await?,
            None => Vec::new(),
        };
        page.main = crate::categories_view::render(&cx, &doc, &latest, &muted)?;
        page.nav = crate::html::nav_items(
            &state.i18n,
            &settings,
            base_path,
            "categories",
            page.chrome.tracking.as_ref(),
        )?;
        page.breadcrumbs = crate::html::breadcrumbs(&state.i18n, &settings)?;
        // DNavigation's showCategoryAdmin: the new category button (with
        // fixed_category_positions, Rails' admin dropdown, not drawn).
        if doc["category_list"]["can_create_category"] == true
            && !settings.get("fixed_category_positions")?.truthy()
        {
            page.admin_controls = format!(
                "<button class=\"btn btn-icon-text btn-default\" id=\"create-category\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
                crate::topic_list_view::icon("plus", None),
                state.i18n.t("js.category.create").unwrap_or_default()
            );
        }
        // discovery/categories.gjs: bodyClass "categories-list"
        page.chrome.body_classes.push_str(" categories-list");
    }
    page.banner = crate::html::welcome_banner(
        &state.i18n,
        &settings,
        page.viewer.as_ref(),
        state.config.globals.relative_url_root(),
        "categories",
    )?;
    let uri = uri.unwrap_or_default();
    page.crawler = list_crawler(&mut conn, &state, &settings, &uri, None, None).await?;
    let body = page.render().map_err(crate::html::HtmlError::from)?;
    Ok(crate::html::crawler_response(
        body,
        &page.crawler,
        &settings,
        &vs,
    )?)
}

#[derive(Deserialize, Default)]
pub struct CategoriesParams {
    include_topics: Option<String>,
    include_subcategories: Option<String>,
    parent_category_id: Option<String>,
    page: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Latest,
    Top,
    Hot,
    Unread,
    New,
    Unseen,
    Read,
    Posted,
    Bookmarks,
}

impl ListKind {
    /// `Discourse.filters` (`anonymous_filters` first), as route segments.
    pub const ALL: [(&'static str, ListKind); 9] = [
        ("latest", ListKind::Latest),
        ("top", ListKind::Top),
        ("hot", ListKind::Hot),
        ("unread", ListKind::Unread),
        ("new", ListKind::New),
        ("unseen", ListKind::Unseen),
        ("read", ListKind::Read),
        ("posted", ListKind::Posted),
        ("bookmarks", ListKind::Bookmarks),
    ];

    pub fn name(self) -> &'static str {
        ListKind::ALL
            .iter()
            .find(|(_, k)| *k == self)
            .map(|(n, _)| *n)
            .expect("every kind is listed")
    }

    pub fn from_name(name: &str) -> Option<ListKind> {
        ListKind::ALL
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, k)| *k)
    }

    /// The TopicQuery filter; top needs its resolved period.
    fn filter(self, period: Option<&str>) -> Filter {
        match self {
            ListKind::Latest => Filter::Latest,
            ListKind::Top => Filter::Top(period.unwrap_or("all").to_string()),
            ListKind::Hot => Filter::Hot,
            ListKind::Unread => Filter::Unread,
            ListKind::New => Filter::New,
            ListKind::Unseen => Filter::Unseen,
            ListKind::Read => Filter::Read,
            ListKind::Posted => Filter::Posted,
            ListKind::Bookmarks => Filter::Bookmarks,
        }
    }

    /// ListController's `ensure_logged_in` filters.
    pub fn requires_login(self) -> bool {
        self.filter(None).requires_login()
    }
}

/// `ListController.best_period_for(nil, category_id)`: the category's
/// default_top_period (else top_page_default_timeframe), unless a period
/// among [default, all] has a full page of scored topics.
async fn best_period_for(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    category_id: Option<i32>,
) -> Result<String, AppError> {
    let site_default = settings.get("top_page_default_timeframe")?.to_s();
    let mut default = site_default.clone();
    if let Some(id) = category_id {
        let period: Option<Option<String>> =
            sqlx::query_scalar("SELECT default_top_period FROM categories WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?;
        if let Some(Some(p)) = period {
            default = p;
        }
    }
    if !crate::topic_query::PERIODS.contains(&default.as_str()) {
        default = site_default;
    }
    let per_page = settings.get("topics_per_period_in_top_page")?.to_i();
    let mut candidates = vec![default.clone()];
    if default != "all" {
        candidates.push("all".to_string());
    }
    for period in candidates {
        if !crate::topic_query::PERIODS.contains(&period.as_str()) {
            continue;
        }
        let sql = match category_id {
            Some(_) => format!(
                "SELECT count(*) FROM (SELECT 1 FROM top_topics JOIN topics ON topics.id = top_topics.topic_id \
                 WHERE top_topics.{period}_score > 0 AND topics.category_id = $1 LIMIT $2) t"
            ),
            None => format!(
                "SELECT count(*) FROM (SELECT 1 FROM top_topics WHERE {period}_score > 0 AND $1::int IS NULL LIMIT $2) t"
            ),
        };
        let count: i64 = sqlx::query_scalar(&sql)
            .bind(category_id)
            .bind(per_page)
            .fetch_one(&mut *conn)
            .await?;
        if count == per_page {
            return Ok(period);
        }
    }
    Ok(default)
}

/// GET /top(.json) and /hot(.json).
pub async fn top(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    front_list(
        state,
        guardian,
        params,
        ListKind::Top,
        false,
        headers,
        Some(uri),
    )
    .await
}

pub async fn top_json(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    front_list(state, guardian, params, ListKind::Top, true, headers, None).await
}

pub async fn hot(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    front_list(
        state,
        guardian,
        params,
        ListKind::Hot,
        false,
        headers,
        Some(uri),
    )
    .await
}

pub async fn hot_json(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    front_list(state, guardian, params, ListKind::Hot, true, headers, None).await
}

/// `/top/:period(.json)` -> 301 to `/top?period=`.
pub async fn top_period_redirect(
    State(state): State<AppState>,
    Path(period): Path<String>,
    headers: HeaderMap,
) -> Response {
    let (period, format) = match period.strip_suffix(".json") {
        Some(p) => (p.to_string(), ".json"),
        None => (period, ""),
    };
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let location = format!(
        "http://{host}{}/top{format}?period={period}",
        state.config.globals.relative_url_root()
    );
    (
        StatusCode::MOVED_PERMANENTLY,
        [(header::LOCATION, location)],
    )
        .into_response()
}

async fn front_list(
    state: AppState,
    guardian: Guardian,
    params: ListParams,
    kind: ListKind,
    json: bool,
    headers: HeaderMap,
    uri: Option<axum::http::Uri>,
) -> Result<Response, AppError> {
    // Before the page's data is read (crate::bus::page_position).
    let bus_position = if json {
        String::new()
    } else {
        crate::bus::page_position(&state.bus).await?
    };
    let live_since = crate::clock::now().timestamp_millis().to_string();
    if let Some(response) = ensure_logged_in(&state, &guardian, kind) {
        return Ok(response);
    }
    let list_path = format!(
        "{}/{}",
        state.config.globals.relative_url_root(),
        kind.name()
    );
    let (doc, settings) =
        match list_document(&state, &guardian, &params, None, false, kind, &list_path).await? {
            Ok(doc) => doc,
            Err(message) => return Ok(invalid_list_params(&state, &message, json)),
        };
    if json {
        return Ok(Json(doc).into_response());
    }
    let mut conn = state.pool.acquire().await?;
    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site =
        crate::html::Site::from_settings(&settings, state.config.globals.relative_url_root())?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(
        &state,
        &settings,
        &guardian,
        crate::sidebar::Active::Discovery,
    )
    .await?;
    site.bus_position = bus_position;
    let mut page =
        crate::html::latest_page(&mut conn, site, &doc, &state.i18n, &settings, false).await?;
    page.banner = crate::html::welcome_banner(
        &state.i18n,
        &settings,
        page.viewer.as_ref(),
        state.config.globals.relative_url_root(),
        kind.name(),
    )?;
    page.breadcrumbs = crate::html::breadcrumbs(&state.i18n, &settings)?;
    if matches!(kind, ListKind::Latest | ListKind::New | ListKind::Unread) {
        page.live_filter = kind.name().to_string();
        page.live_since = live_since;
    }
    page.nav = crate::html::nav_items(
        &state.i18n,
        &settings,
        state.config.globals.relative_url_root(),
        kind.name(),
        page.chrome.tracking.as_ref(),
    )?;
    if page.chrome.tracking.is_some() {
        page.chrome.live_param("nav", kind.name());
    }
    let uri = uri.unwrap_or_default();
    page.crawler = list_crawler(&mut conn, &state, &settings, &uri, None, None).await?;
    let body = page.render().map_err(crate::html::HtmlError::from)?;
    Ok(crate::html::crawler_response(
        body,
        &page.crawler,
        &settings,
        &vs,
    )?)
}

/// The crawler block for a list page: the site's title, description and
/// OpenGraph image unless the page supplies its own.
async fn list_crawler(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    uri: &axum::http::Uri,
    canonical: Option<String>,
    meta: Option<(String, String)>,
) -> Result<crate::html::Crawler, AppError> {
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let site_description = settings.get("site_description")?.to_s();
    let (title, description) = meta.unwrap_or_else(|| {
        (
            settings.get("title").map(|t| t.to_s()).unwrap_or_default(),
            site_description.clone(),
        )
    });
    let image = crate::html::site_opengraph_image(conn, &urls).await?;
    let mut crawler = crate::html::Crawler::for_request(&urls, uri, canonical)?.with_meta(
        &title,
        &description,
        image,
    );
    crawler.description = if description.is_empty() {
        site_description
    } else {
        description
    };
    Ok(crawler)
}

/// ListController's `ensure_logged_in` for the login-only filters: the 403
/// `not_logged_in` body, which a page request gets as the not-found page
/// (not_found::html_errors).
fn ensure_logged_in(state: &AppState, guardian: &Guardian, kind: ListKind) -> Option<Response> {
    if !kind.requires_login() || guardian.is_authenticated() {
        return None;
    }
    Some(super::login_required::not_logged_in(state))
}

/// GET /unread, /new, /unseen, /read, /posted, /bookmarks (.json): the
/// login-only front lists, one handler keyed by the path.
pub async fn user_list(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    let path = uri.path().trim_start_matches('/');
    let (name, json) = match path.strip_suffix(".json") {
        Some(n) => (n, true),
        None => (path, false),
    };
    let kind = ListKind::from_name(name).ok_or(crate::Unsupported("unknown list filter"))?;
    front_list(
        state,
        guardian,
        params,
        kind,
        json,
        headers,
        (!json).then_some(uri),
    )
    .await
}

/// `CategoriesController#topics_per_page`: categories_topics when set,
/// else one and a half times the visible top-level categories, 5 to 100.
async fn topics_per_page(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
) -> Result<i64, AppError> {
    let set = settings.get("categories_topics")?.to_i();
    if set > 0 {
        return Ok(set);
    }
    let allowed = guardian.allowed_category_ids(&mut *conn, settings).await?;
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM categories WHERE parent_category_id IS NULL AND id = ANY($1)",
    )
    .bind(&allowed)
    .fetch_one(&mut *conn)
    .await?;
    Ok(((count as f64 * 1.5) as i64).clamp(5, 100))
}
