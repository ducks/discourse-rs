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
fn next_url(list_path: &str, params: &ListParams, options: &Options) -> String {
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
    if let Some(p) = options.per_page {
        pairs.push(("per_page", p.to_string()));
    }
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    let query: Vec<String> = pairs.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{list_path}?{}", query.join("&"))
}

/// The /latest document, shared by the JSON and HTML responses.
async fn latest_document(
    state: &AppState,
    params: &ListParams,
    category: Option<&Category>,
    no_subcategories: bool,
    list_path: &str,
) -> Result<Result<(serde_json::Value, SiteSettings), (StatusCode, String)>, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let mut options = match build_options(params, &settings) {
        Ok(o) => o,
        Err(e) => return Ok(Err(e)),
    };
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

    let list = TopicQuery {
        conn: &mut conn,
        settings: &settings,
        guardian: &guardian,
        options: options.clone(),
        category: Default::default(),
    }
    .list_latest()
    .await?;

    let more = next_url(list_path, params, &options);
    let json = TopicListSerializer {
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
    Ok(Ok((json, settings)))
}

/// GET /latest.json
pub async fn latest_json(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Response, AppError> {
    let list_path = format!("{}/latest", state.config.globals.relative_url_root());
    match latest_document(&state, &params, None, false, &list_path).await? {
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
    let (json, settings) = match latest_document(&state, &params, None, false, &list_path).await? {
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
            next_url("/latest", &p, &o),
            "/latest?no_definitions=true&page=3&per_page=10"
        );
        let o = Options {
            no_definitions: true,
            ..Options::default()
        };
        assert_eq!(
            next_url("/forum/latest", &ListParams::default(), &o),
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
    let (slug_path, action, none) = if let Some(p) = path.strip_suffix("/none/l/latest") {
        (p.to_string(), "/none/l/latest", true)
    } else if let Some(p) = path.strip_suffix("/none") {
        (p.to_string(), "/none", true)
    } else if let Some(p) = path.strip_suffix("/l/latest") {
        (p.to_string(), "/l/latest", false)
    } else {
        (path.clone(), "", false)
    };
    if slug_path.contains("/l/") || slug_path.ends_with("/all") {
        return Err(crate::Unsupported("category list filters top and hot").into());
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

    // category_default_view: the category's default_view if anonymous users
    // may list it, else latest. Only latest is ported.
    if action.is_empty() {
        if let Some(view) = category
            .default_view
            .as_deref()
            .filter(|v| !v.is_empty() && *v != "latest")
        {
            if ["top", "hot"].contains(&view) {
                return Err(crate::Unsupported("category default_view top/hot").into());
            }
        }
    }

    let list_path = format!("{base_path}/c/{real_slug}{action}");
    let (doc, settings) =
        match latest_document(&state, &params, Some(&category), none, &list_path).await? {
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
