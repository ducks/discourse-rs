//! GET /new-topic and /new-message (new_topic#index). Rails serves the
//! Ember app there, whose new-topic and new-message routes never draw a
//! page of their own: an anonymous visitor is sent to log in
//! (`showLogin`), and a member lands on another page with the composer
//! opened over it, prefilled from the query params.
//!
//! Here a page load draws that other page, the login page or the one the
//! route replaces itself with (the category asked for, else the
//! homepage), and hands static/js/composer.js what the route does next:
//! the URL it replaces /new-topic with and the params the composer opens
//! with (composer.js `#composer-open`). The login page drawn here takes
//! /login as its URL in static/js/login.js, after keeping this one as the
//! destination_url, as the login route's beforeModel does. Following a
//! link to either route from another page opens the composer in place
//! (composer.js), as the routes do when entered from another route.

use axum::body::Body;
use axum::extract::{Path, Query, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// Which of the two routes.
#[derive(Clone, Copy, PartialEq)]
enum Route {
    Topic,
    Message,
}

impl Route {
    fn name(self) -> &'static str {
        match self {
            Route::Topic => "new-topic",
            Route::Message => "new-message",
        }
    }
}

/// controllers/discovery/list.js `queryParams`, with the discovery
/// route's `filter`: what a replaceWith to a discovery list keeps of the
/// transition's query params (`_processActiveTransitionQueryParams`
/// scopes them to the target route; the others are dropped).
const LIST_QUERY_PARAMS: &[&str] = &[
    "order",
    "ascending",
    "status",
    "state",
    "search",
    "max_posts",
    "min_posts",
    "q",
    "before",
    "bumped_before",
    "f",
    "subset",
    "period",
    "topic_ids",
    "group_name",
    "tags",
    "match_all_tags",
    "no_subcategories",
    "no_tags",
    "exclude_tag",
    "filter",
];

pub async fn new_topic(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    show(state, guardian, headers, uri, Route::Topic).await
}

pub async fn new_message(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    show(state, guardian, headers, uri, Route::Message).await
}

async fn show(
    state: AppState,
    guardian: crate::guardian::Guardian,
    headers: HeaderMap,
    uri: Uri,
    route: Route,
) -> Result<Response, AppError> {
    // NewTopicController#index has no template for another format.
    let xhr = headers
        .get("x-requested-with")
        .is_some_and(|v| v == "XMLHttpRequest");
    if xhr || super::wants_json(uri.query(), &headers) {
        return Err(Unsupported("/new-topic or /new-message asked for as JSON").into());
    }
    let settings = {
        let mut conn = state.pool.acquire().await?;
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?
    };
    // `transition.send("showLogin")`: the login route.
    if guardian.is_anonymous() {
        return super::login_required::login_page(&state, &settings, &headers, &uri).await;
    }
    let params = query_params(uri.query())?;
    let base_path = state.config.globals.relative_url_root().to_string();

    // new-topic: a category asked for is where the route goes
    // (`replaceWith(`/c/${category.id}`)`), the composer opened only for
    // a member who may create topics.
    if route == Route::Topic {
        let category = {
            let mut conn = state.pool.acquire().await?;
            find_category(&mut conn, &state, &settings, &guardian, &base_path, &params).await?
        };
        if let Some(category) = category {
            let (slug_path, shared_draft, open) = {
                let mut conn = state.pool.acquire().await?;
                let found = crate::category::Category::find(&mut conn, category as i32)
                    .await?
                    .ok_or(Unsupported("a site category gone from the database"))?;
                (
                    found.full_slug(&mut conn).await?,
                    shared_drafts_category(&settings, &guardian)? == Some(category),
                    guardian.can_create_topic(&mut conn, &settings).await?,
                )
            };
            let page_uri: Uri = format!("{base_path}/c/{slug_path}")
                .parse()
                .map_err(|_| Unsupported("a category path that is not a URI"))?;
            let page = super::list::category(
                State(state.clone()),
                AuthGuardian(guardian),
                Path(slug_path),
                Query(super::list::ListParams::default()),
                RawQuery(None),
                headers,
                page_uri,
                None,
            )
            .await?;
            let instruction = json!({
                "route": route.name(),
                "url": format!("{base_path}/c/{category}"),
                "params": params_json(&params),
                "category": category,
                "sharedDraft": shared_draft,
                "open": open,
            });
            return with_instruction(page, &instruction).await;
        }
    }

    // `replaceWith(homepageNavigationDestination())`: discovery.<homepage>,
    // keeping the query params the discovery routes know.
    let (homepage, open) = {
        let mut conn = state.pool.acquire().await?;
        let homepage = crate::homepage::current(&mut conn, &settings, guardian.user_id()).await?;
        let open = match route {
            Route::Topic => guardian.can_create_topic(&mut conn, &settings).await?,
            Route::Message => true,
        };
        (homepage, open)
    };
    if homepage == "custom" || homepage == "blank" {
        return Err(Unsupported("/new-topic over a custom or blank homepage").into());
    }
    let known = known_query_params(&homepage, &settings)?;
    let query = carried_query(&params, &known);
    let path = format!("{base_path}/{homepage}");
    let url = if query.is_empty() {
        path.clone()
    } else {
        format!("{path}?{query}")
    };
    let page = super::list::front_page(
        State(state.clone()),
        AuthGuardian(guardian),
        headers,
        url.parse()
            .map_err(|_| Unsupported("a homepage URL that is not a URI"))?,
    )
    .await?;
    let instruction = json!({
        "route": route.name(),
        "url": url,
        "params": params_json(&params),
        "category": Value::Null,
        "sharedDraft": false,
        "open": open,
    });
    with_instruction(page, &instruction).await
}

/// `transition.to.queryParams`, as route-recognizer's parseQueryString
/// reads the query: pairs split on `&`, each on `=` with only its first
/// two parts kept, a pair without `=` meaning "true", the last of a
/// repeated name winning. An array param (`tags[]=`) is refused:
/// route-recognizer collects those into arrays, which the composer then
/// trips over.
fn query_params(query: Option<&str>) -> Result<Vec<(String, String)>, Unsupported> {
    let mut out: Vec<(String, String)> = Vec::new();
    let Some(query) = query.filter(|q| !q.is_empty()) else {
        return Ok(out);
    };
    for pair in query.split('&') {
        let mut parts = pair.split('=');
        let key = decode_query_param_part(parts.next().unwrap_or_default());
        let value = match parts.next() {
            None => "true".to_string(),
            Some(_) if key.len() > 2 && key.ends_with("[]") => {
                return Err(Unsupported(
                    "an array query param on /new-topic or /new-message",
                ));
            }
            Some(value) => decode_query_param_part(value),
        };
        match out.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = value,
            None => out.push((key, value)),
        }
    }
    Ok(out)
}

/// route-recognizer's decodeQueryParamPart: `+` is a space, then
/// decodeURIComponent, whose failure (a stray `%`, bytes that are not
/// UTF-8) gives "".
fn decode_query_param_part(part: &str) -> String {
    let part = part.replace('+', "%20");
    let bytes = part.as_bytes();
    let malformed = bytes.iter().enumerate().any(|(i, b)| {
        *b == b'%'
            && !(bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit))
    });
    if malformed {
        return String::new();
    }
    percent_encoding::percent_decode_str(&part)
        .decode_utf8()
        .map(|s| s.into_owned())
        .unwrap_or_default()
}

fn param<'a>(params: &'a [(String, String)], name: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

fn params_json(params: &[(String, String)]) -> Value {
    Value::Object(
        params
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect::<Map<String, Value>>(),
    )
}

/// The query params the homepage's discovery route knows: a topic list's
/// (with discourse-solved's `solved`, addDiscoveryQueryParam), or only the
/// discovery route's `filter` for the categories page.
fn known_query_params(
    homepage: &str,
    settings: &SiteSettings,
) -> Result<Vec<&'static str>, AppError> {
    if homepage == "categories" {
        return Ok(vec!["filter"]);
    }
    let mut known = LIST_QUERY_PARAMS.to_vec();
    if crate::plugins::solved::enabled(settings)? {
        known.push("solved");
    }
    Ok(known)
}

/// The kept params as route-recognizer writes them: sorted by name, each
/// key and value through encodeURIComponent.
fn carried_query(params: &[(String, String)], known: &[&str]) -> String {
    let mut kept: Vec<&(String, String)> = params
        .iter()
        .filter(|(k, _)| known.contains(&k.as_str()))
        .collect();
    kept.sort_by(|a, b| a.0.cmp(&b.0));
    kept.iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                crate::directory_view::encode_uri_component(k),
                crate::directory_view::encode_uri_component(v)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// `#loadCategoryFromTransition`: `category_id` by id, else `category`
/// by name then by slug (`parent/child` under the parent), among the
/// site's categories. Lazy-loaded categories (Category.asyncFindById and
/// asyncFindBySlugPath) are refused.
async fn find_category(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    base_path: &str,
    params: &[(String, String)],
) -> Result<Option<i64>, AppError> {
    let by_id = param(params, "category_id").filter(|v| !v.is_empty());
    let by_name = param(params, "category").filter(|v| !v.is_empty());
    if by_id.is_none() && by_name.is_none() {
        return Ok(None);
    }
    if guardian.can_lazy_load_categories(settings)? {
        return Err(Unsupported("/new-topic's category with lazy-loaded categories").into());
    }
    let categories = crate::categories::Categories {
        conn,
        settings,
        i18n: &state.i18n,
        guardian,
        base_path,
        topic_url_via_slug: true,
    }
    .for_site()
    .await?;
    Ok(match by_id {
        // Category.findById: the id map's key, as the param spells it.
        Some(id) => categories
            .iter()
            .filter_map(|c| c["id"].as_i64())
            .find(|c| c.to_string() == id),
        None => category_by_path(&categories, by_name.unwrap_or_default()),
    })
}

/// `#getCategory` by "nameLower", then by "slug". Only the first two
/// segments of a path count.
fn category_by_path(categories: &[Value], path: &str) -> Option<i64> {
    let mut segments = path.split('/');
    let main = segments.next().unwrap_or_default().to_lowercase();
    let sub = segments.next().map(str::to_lowercase);
    let field = |c: &Value, by_name: bool| -> String {
        if by_name {
            c["name"].as_str().unwrap_or_default().to_lowercase()
        } else {
            c["slug"].as_str().unwrap_or_default().to_string()
        }
    };
    [true, false].into_iter().find_map(|by_name| {
        let parent = categories.iter().find(|c| field(c, by_name) == main)?;
        match &sub {
            None => parent["id"].as_i64(),
            Some(sub) => categories
                .iter()
                .find(|c| field(c, by_name) == *sub && c["parent_category_id"] == parent["id"])
                .and_then(|c| c["id"].as_i64()),
        }
    })
}

/// `site.shared_drafts_category_id`: sent only to who may see shared drafts.
fn shared_drafts_category(
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
) -> Result<Option<i64>, AppError> {
    if !guardian.can_see_shared_draft(settings)? {
        return Ok(None);
    }
    Ok(settings
        .get("shared_drafts_category")?
        .presence()
        .map(|id| crate::ruby::to_i(&id)))
}

/// The page with what composer.js does next (`#composer-open`), put
/// before the composer it opens. A page that is not drawn (a redirect, a
/// not found) goes out as it is.
async fn with_instruction(page: Response, instruction: &Value) -> Result<Response, AppError> {
    if page.status() != StatusCode::OK {
        return Ok(page);
    }
    let (mut parts, body) = page.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_err(|_| Unsupported("reading the page under /new-topic"))?;
    let html = String::from_utf8(bytes.to_vec())
        .map_err(|_| Unsupported("a page under /new-topic that is not UTF-8"))?;
    const COMPOSER: &str = "<div id=\"reply-control\"";
    let Some(at) = html.find(COMPOSER) else {
        return Err(Unsupported("/new-topic over a page without the composer").into());
    };
    // JSON in a script element: `<` escaped so no `</script>` closes it.
    let script = format!(
        "<script type=\"application/json\" id=\"composer-open\">{}</script>",
        instruction.to_string().replace('<', "\\u003c")
    );
    let html = format!("{}{script}{}", &html[..at], &html[at..]);
    parts.headers.remove(header::CONTENT_LENGTH);
    // The page carries this request's params: never a cached copy.
    parts.headers.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-cache, no-store"),
    );
    Ok(Response::from_parts(parts, Body::from(html)).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(q: &str) -> Vec<(String, String)> {
        query_params(Some(q)).unwrap()
    }

    #[test]
    fn last_value_wins_and_plus_is_a_space() {
        assert_eq!(
            params("f=1&q=x+y&f=2"),
            [("f".into(), "2".into()), ("q".into(), "x y".into())]
        );
        // Only the first two parts of a pair; no `=` is "true"; a bad
        // escape is "".
        assert_eq!(
            params("tags=a,b=c&flag&x=%zz&y=%E2%9C%93"),
            [
                ("tags".into(), "a,b".into()),
                ("flag".into(), "true".into()),
                ("x".into(), String::new()),
                ("y".into(), "\u{2713}".into()),
            ]
        );
        assert!(query_params(Some("tags[]=a")).is_err());
        assert_eq!(params("tags[]"), [("tags[]".into(), "true".into())]);
        assert!(params("").is_empty());
    }

    #[test]
    fn carries_the_list_params_sorted_and_encoded() {
        let known = LIST_QUERY_PARAMS.to_vec();
        assert_eq!(
            carried_query(
                &params(
                    "title=T&tags=howto,guide&order=views&page=2&category=nope&status=a%20b&order="
                ),
                &known
            ),
            "order=&status=a%20b&tags=howto%2Cguide"
        );
        assert_eq!(
            carried_query(&params("ascending=false&filter=tracked"), &["filter"]),
            "filter=tracked"
        );
    }

    #[test]
    fn finds_a_category_by_name_then_slug() {
        let categories = vec![
            json!({"id": 1, "name": "Uncategorized", "slug": "uncategorized", "parent_category_id": null}),
            json!({"id": 4, "name": "General", "slug": "general", "parent_category_id": null}),
            json!({"id": 34, "name": "Sub General", "slug": "sub-general", "parent_category_id": 4}),
            json!({"id": 2, "name": "Site Feedback", "slug": "site-feedback", "parent_category_id": null}),
        ];
        assert_eq!(category_by_path(&categories, "general"), Some(4));
        assert_eq!(category_by_path(&categories, "Site Feedback"), Some(2));
        assert_eq!(category_by_path(&categories, "SUB-GENERAL"), Some(34));
        assert_eq!(category_by_path(&categories, "sub general"), Some(34));
        assert_eq!(
            category_by_path(&categories, "general/sub-general"),
            Some(34)
        );
        assert_eq!(
            category_by_path(&categories, "General/Sub General/x"),
            Some(34)
        );
        assert_eq!(
            category_by_path(&categories, "site-feedback/sub-general"),
            None
        );
        assert_eq!(category_by_path(&categories, "nope"), None);
    }
}
