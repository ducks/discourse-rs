//! Port of app/controllers/list_controller.rb (latest) with
//! lib/topic_query_params.rb.

use askama::Template;
use axum::Json;
use axum::extract::{Path, Query, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Html;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::category::Category;
use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::topic_list::TopicListSerializer;
use crate::topic_query::{Options, TopicQuery};
use crate::url::Urls;
use crate::{AppError, AppState};

/// The TopicQuery.public_valid_options ported so far; the rest are
/// rejected until they are.
#[derive(Deserialize, Default)]
pub struct ListParams {
    page: Option<String>,
    per_page: Option<String>,
    order: Option<String>,
    ascending: Option<String>,
    /// top lists only; not a TopicQuery option, so never echoed unless given
    period: Option<String>,
}

/// `build_topic_list_options` + `TopicQuery.validate?`: a bad value is
/// Discourse::InvalidParameters, a 400.
fn build_options(
    params: &ListParams,
    settings: &SiteSettings,
) -> Result<Options, (StatusCode, String)> {
    let invalid = |name: &str| (StatusCode::BAD_REQUEST, format!("{name} is invalid"));
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
fn next_url(list_path: &str, params: &ListParams, options: &Options, top: bool) -> String {
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
    if top {
        if let Some(period) = params.period.as_deref().filter(|p| !p.is_empty()) {
            pairs.push(("period", period.to_string()));
        }
    }
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    let query: Vec<String> = pairs.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{list_path}?{}", query.join("&"))
}

/// The /latest document, shared by the JSON and HTML responses.
async fn list_document(
    state: &AppState,
    params: &ListParams,
    category: Option<&Category>,
    no_subcategories: bool,
    kind: ListKind,
    list_path: &str,
) -> Result<Result<(serde_json::Value, SiteSettings), (StatusCode, String)>, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let mut options = match build_options(params, &settings) {
        Ok(o) => o,
        Err(e) => return Ok(Err(e)),
    };
    let mut for_period: Option<String> = None;
    if let ListKind::Top = kind {
        // top_#{period}: per_page defaults to topics_per_period_in_top_page and
        // is always carried in the next-page URL.
        if options.per_page.is_none() {
            options.per_page = Some(settings.get("topics_per_period_in_top_page")?.to_i());
        }
        let period = match params.period.as_deref().filter(|p| !p.is_empty()) {
            Some(p) => p.to_string(),
            None => best_period_for(&mut conn, &settings, category.map(|c| c.id)).await?,
        };
        if !crate::topic_query::PERIODS.contains(&period.as_str()) {
            return Ok(Err((
                StatusCode::BAD_REQUEST,
                "Invalid period. Valid periods are all, yearly, quarterly, monthly, weekly, daily"
                    .into(),
            )));
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
    let guardian = Guardian::anonymous();
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };

    let mut query = TopicQuery {
        conn: &mut conn,
        settings: &settings,
        guardian: &guardian,
        options: options.clone(),
        category: Default::default(),
        filter: Default::default(),
    };
    let list = match (&kind, &for_period) {
        (ListKind::Latest, _) => query.list_latest().await?,
        (ListKind::Top, Some(period)) => query.list_top_for(period).await?,
        (ListKind::Top, None) => unreachable!("top lists always resolve a period"),
        (ListKind::Hot, _) => query.list_hot().await?,
    };

    let more = next_url(list_path, params, &options, kind == ListKind::Top);
    let mut json = TopicListSerializer {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        more_topics_url: Some(more),
        category_id: options.category_id,
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
    Ok(Ok((json, settings)))
}

/// GET /latest.json
pub async fn latest_json(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Response, AppError> {
    let list_path = format!("{}/latest", state.config.globals.relative_url_root());
    match list_document(&state, &params, None, false, ListKind::Latest, &list_path).await? {
        Ok((json, _)) => Ok(Json(json).into_response()),
        Err((status, message)) => Ok((status, message).into_response()),
    }
}

/// GET / and GET /latest: the server-rendered list. The "more" link
/// points at the HTML page.
pub async fn latest(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Response, AppError> {
    let list_path = format!("{}/latest", state.config.globals.relative_url_root());
    let (json, settings) =
        match list_document(&state, &params, None, false, ListKind::Latest, &list_path).await? {
            Ok(doc) => doc,
            Err((status, message)) => return Ok((status, message).into_response()),
        };
    let mut conn = state.pool.acquire().await?;
    let base_path = state.config.globals.relative_url_root();
    let site = crate::html::Site::from_settings(&settings, base_path)?;
    let mut page = crate::html::latest_page(&mut conn, site, &json).await?;
    // Strip `no_definitions`, which is what the JSON list carries around
    // but the HTML list applies on its own.
    page.more_url = page.more_url.map(|u| {
        u.replace("no_definitions=true&", "")
            .replace("?no_definitions=true", "")
    });
    Ok(Html(page.render().map_err(crate::html::HtmlError::from)?).into_response())
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

/// `/c/*category_slug_path_with_id(.json)` -> list#category_default, and
/// `.../l/latest` -> list#category_latest. The other filters and `/none`
/// aren't ported.
pub async fn category(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(params): Query<ListParams>,
    RawQuery(raw_query): RawQuery,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let (path, json) = match path.strip_suffix(".json") {
        Some(p) => (p.to_string(), true),
        None => (path.clone(), false),
    };
    // `.../none` (no_subcategories) and `.../none/l/latest`, else `.../l/latest`.
    // `.../none` (no_subcategories), `.../l/{latest,top,hot}`, or both.
    let mut rest = path.as_str();
    let mut kind = ListKind::Latest;
    let mut action = String::new();
    for (suffix, k) in [
        ("/l/latest", ListKind::Latest),
        ("/l/top", ListKind::Top),
        ("/l/hot", ListKind::Hot),
    ] {
        if let Some(p) = rest.strip_suffix(suffix) {
            rest = p;
            kind = k;
            action = suffix.to_string();
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
        return Err(
            crate::Unsupported("category list filters other than latest, top and hot").into(),
        );
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let max_nesting = settings.get("max_category_nesting")?.to_i();
    let base_path = state.config.globals.relative_url_root().to_string();

    // set_category: missing or unreadable -> 404, before any redirect.
    let category = Category::find_by_slug_path_with_id(&mut conn, &slug_path, max_nesting).await?;
    let Some(category) = category else {
        return Ok(super::topics::not_found_response(&state, false));
    };
    if category.read_restricted {
        return Ok(super::topics::not_found_response(&state, false));
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
    if action.is_empty() {
        kind = match category.default_view.as_deref() {
            Some("top") => ListKind::Top,
            Some("hot") => ListKind::Hot,
            _ => ListKind::Latest,
        };
    }

    let list_path = format!("{base_path}/c/{real_slug}{action}");
    let (doc, settings) =
        match list_document(&state, &params, Some(&category), none, kind, &list_path).await? {
            Ok(doc) => doc,
            Err((status, message)) => return Ok((status, message).into_response()),
        };
    if json {
        return Ok(Json(doc).into_response());
    }
    let mut conn = state.pool.acquire().await?;
    let site = crate::html::Site::from_settings(&settings, &base_path)?;
    let mut page = crate::html::latest_page(&mut conn, site, &doc).await?;
    page.heading = Some(
        crate::html::category_heading(&mut conn, &base_path, &category, params.page.is_none())
            .await?,
    );
    Ok(Html(page.render().map_err(crate::html::HtmlError::from)?).into_response())
}

/// GET /categories(.json) -> categories#index for anonymous users.
pub async fn categories(
    State(state): State<AppState>,
    Query(params): Query<CategoriesParams>,
) -> Result<Response, AppError> {
    categories_response(state, params, false).await
}

pub async fn categories_json(
    State(state): State<AppState>,
    Query(params): Query<CategoriesParams>,
) -> Result<Response, AppError> {
    categories_response(state, params, true).await
}

async fn categories_response(
    state: AppState,
    params: CategoriesParams,
    json: bool,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let guardian = Guardian::anonymous();
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let doc = crate::category_list::CategoryList {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        include_topics_param: params.include_topics.is_some(),
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
    let site =
        crate::html::Site::from_settings(&settings, state.config.globals.relative_url_root())?;
    let page = crate::html::categories_page(&mut conn, &state.i18n, site, &doc).await?;
    Ok(Html(page.render().map_err(crate::html::HtmlError::from)?).into_response())
}

#[derive(Deserialize, Default)]
pub struct CategoriesParams {
    include_topics: Option<String>,
    page: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Latest,
    Top,
    Hot,
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
    Query(params): Query<ListParams>,
) -> Result<Response, AppError> {
    front_list(state, params, ListKind::Top, false).await
}

pub async fn top_json(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Response, AppError> {
    front_list(state, params, ListKind::Top, true).await
}

pub async fn hot(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Response, AppError> {
    front_list(state, params, ListKind::Hot, false).await
}

pub async fn hot_json(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Response, AppError> {
    front_list(state, params, ListKind::Hot, true).await
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
    params: ListParams,
    kind: ListKind,
    json: bool,
) -> Result<Response, AppError> {
    let name = match kind {
        ListKind::Latest => "latest",
        ListKind::Top => "top",
        ListKind::Hot => "hot",
    };
    let list_path = format!("{}/{name}", state.config.globals.relative_url_root());
    let (doc, settings) =
        match list_document(&state, &params, None, false, kind, &list_path).await? {
            Ok(doc) => doc,
            Err((status, message)) => return Ok((status, message).into_response()),
        };
    if json {
        return Ok(Json(doc).into_response());
    }
    let mut conn = state.pool.acquire().await?;
    let site =
        crate::html::Site::from_settings(&settings, state.config.globals.relative_url_root())?;
    let page = crate::html::latest_page(&mut conn, site, &doc).await?;
    Ok(Html(page.render().map_err(crate::html::HtmlError::from)?).into_response())
}
