//! Port of app/controllers/tags_controller.rb for anonymous users: the
//! per-tag topic lists (`/tag/...`, `/tags/c/...`) and the tags index.

use askama::Template;
use axum::Json;
use axum::extract::{Path, Query, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use super::list::{ListKind, ListParams, ListScope, TagListRequest, list_document_for};
use crate::category::Category;
use crate::guardian::Guardian;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::tags::{Tag, visible_tags_where};
use crate::topic_query::Options;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// How the tag was named in the path, which `page_params` echoes back in
/// the next-page URL.
enum TagRef {
    /// `/tag/:tag_slug/:tag_id`
    SlugId(String, i32),
    /// `/tag/:tag_id` (JSON only)
    Id(i32),
    /// `/tag/:tag_name` (legacy)
    Name(String),
}

struct TagPath {
    tag: TagRef,
    /// `/tags/c/*category_slug_path_with_id/...`
    category_path: Option<String>,
    /// `/none/` between the category and the tag
    no_subcategories: bool,
    /// `/l/:filter`
    filter: Option<String>,
    json: bool,
}

/// Splits `/tag/...` and `/tags/c/...` paths the way the routes do.
fn parse_path(path: &str, with_category: bool) -> Result<TagPath, Unsupported> {
    let (path, json) = match path.strip_suffix(".json") {
        Some(p) => (p, true),
        None => (path, false),
    };
    let mut segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let mut filter = None;
    if segments.len() >= 2 && segments[segments.len() - 2] == "l" {
        filter = Some(segments[segments.len() - 1].to_string());
        segments.truncate(segments.len() - 2);
    }
    if segments.is_empty() {
        return Err(Unsupported("tag path without a tag"));
    }
    let numeric = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let last = segments[segments.len() - 1];
    let tag = if numeric(last) && segments.len() >= 2 && (with_category || segments.len() == 2) {
        let id: i32 = last
            .parse()
            .map_err(|_| Unsupported("tag id out of range"))?;
        let slug = segments[segments.len() - 2].to_string();
        segments.truncate(segments.len() - 2);
        TagRef::SlugId(slug, id)
    } else if numeric(last) && !with_category {
        segments.pop();
        if !json {
            return Err(Unsupported("/tag/:tag_id without .json"));
        }
        TagRef::Id(
            last.parse()
                .map_err(|_| Unsupported("tag id out of range"))?,
        )
    } else {
        segments.pop();
        TagRef::Name(last.to_string())
    };
    let mut no_subcategories = false;
    let mut category_path = None;
    if with_category {
        match segments.last().copied() {
            Some("none") => {
                no_subcategories = true;
                segments.pop();
            }
            Some("all") => {
                segments.pop();
            }
            _ => {}
        }
        if segments.is_empty() {
            return Err(Unsupported("/tags/c/ without a category"));
        }
        category_path = Some(segments.join("/"));
    } else if !segments.is_empty() {
        return Err(Unsupported("unknown /tag/ route"));
    }
    Ok(TagPath {
        tag,
        category_path,
        no_subcategories,
        filter,
        json,
    })
}

/// `tags[]` from the query string, which TagsController#tag_params
/// prefers over the tag in the path (prefixed with it only on the
/// name-based routes, by TopicQueryParams).
fn query_tags(raw_query: Option<&str>) -> Vec<String> {
    let Some(q) = raw_query else {
        return Vec::new();
    };
    form_urlencoded::parse(q.as_bytes())
        .filter(|(k, _)| k == "tags[]" || k == "tags")
        .map(|(_, v)| v.into_owned())
        .collect()
}

/// `construct_url_with(:next, list_opts)`: the request's tag route with
/// the list options as query params, sorted, `.json` dropped.
pub(super) fn next_url(
    list_path: &str,
    params: &ListParams,
    options: &Options,
    request: &TagListRequest,
) -> String {
    let mut pairs: Vec<String> = Vec::new();
    if let Some(a) = params.ascending.as_deref().filter(|a| !a.is_empty()) {
        pairs.push(format!("ascending={a}"));
    }
    if !request.no_tags {
        pairs.push("match_all_tags=true".to_string());
    } else {
        pairs.push("no_tags=true".to_string());
    }
    if options.no_subcategories {
        pairs.push("no_subcategories=true".to_string());
    }
    if let Some(o) = &options.order {
        pairs.push(format!("order={o}"));
    }
    pairs.push(format!("page={}", options.page + 1));
    if let Some(p) = options.per_page {
        pairs.push(format!("per_page={p}"));
    }
    if !request.tags.is_empty() {
        let tags: Vec<String> = request
            .tags
            .iter()
            .map(|t| {
                format!(
                    "tags%5B%5D={}",
                    form_urlencoded::byte_serialize(t.as_bytes()).collect::<String>()
                )
            })
            .collect();
        pairs.push(tags.join("&"));
    }
    pairs.sort();
    format!("{list_path}?{}", pairs.join("&"))
}

/// GET /tag/{*path}
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(path): Path<String>,
    Query(params): Query<ListParams>,
    RawQuery(raw_query): RawQuery,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    let parsed = parse_path(&path, false)?;
    show_list(state, guardian, parsed, params, raw_query, headers, uri).await
}

/// GET /tags/c/{*path}
pub async fn show_in_category(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(path): Path<String>,
    Query(params): Query<ListParams>,
    RawQuery(raw_query): RawQuery,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    let parsed = parse_path(&path, true)?;
    show_list(state, guardian, parsed, params, raw_query, headers, uri).await
}

fn redirect(headers: &HeaderMap, location: String) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    (
        StatusCode::MOVED_PERMANENTLY,
        [(header::LOCATION, format!("http://{host}{location}"))],
    )
        .into_response()
}

/// `show_#{filter}`.
async fn show_list(
    state: AppState,
    guardian: Guardian,
    path: TagPath,
    params: ListParams,
    raw_query: Option<String>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    // Before the page's data is read (crate::bus::page_position).
    let bus_position = if path.json {
        String::new()
    } else {
        crate::bus::page_position(&state.bus).await?
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let not_found = || super::topics::not_found_response(&state, false);
    // ensure_tags_enabled
    if !settings.get("tagging_enabled")?.truthy() {
        return Ok(not_found());
    }
    let kind = match path.filter.as_deref() {
        None | Some("latest") => ListKind::Latest,
        Some("top") => ListKind::Top,
        Some("hot") => ListKind::Hot,
        Some(_) => {
            return Err(Unsupported("tag list filters other than latest, top and hot").into());
        }
    };
    let base_path = state.config.globals.relative_url_root().to_string();

    // set_category: missing or unreadable -> 404; no slug check.
    let mut category = None;
    if let Some(slug_path) = &path.category_path {
        let max_nesting = settings.get("max_category_nesting")?.to_i();
        let found = Category::find_by_slug_path_with_id(&mut conn, slug_path, max_nesting).await?;
        match found {
            Some(c) if !c.read_restricted => category = Some(c),
            _ => return Ok(not_found()),
        }
    }

    // fetch_tag(raise_not_found: false)
    let (tag, found_by_name) = match &path.tag {
        TagRef::SlugId(_, id) | TagRef::Id(id) => match Tag::find(&mut conn, *id).await? {
            Some(t) => (Some(t), false),
            None => (Tag::find_by_name(&mut conn, &id.to_string()).await?, true),
        },
        TagRef::Name(name) => (Tag::find_by_name(&mut conn, name).await?, false),
    };
    if let Some(t) = &tag
        && !t.visible_to_anonymous(&mut conn).await?
    {
        return Ok(not_found());
    }
    let has_id = matches!(path.tag, TagRef::SlugId(..) | TagRef::Id(_));
    if has_id && tag.is_none() {
        return Ok(not_found());
    }

    if let Some(t) = &tag {
        let target = match t.target_tag_id {
            Some(target_id) if target_id != t.id => Tag::find(&mut conn, target_id).await?,
            _ => None,
        };
        // should_redirect_tag?: never for JSON; canonical URLs only when
        // the slug is off; legacy ones always.
        // A numeric-named tag reached through /tag/:tag_id keeps its URL.
        let should_redirect = !path.json
            && match &path.tag {
                TagRef::SlugId(slug, _) => *slug != t.slug_for_url(),
                TagRef::Id(_) => !found_by_name,
                TagRef::Name(_) => true,
            };
        if target.is_some() || should_redirect {
            let t = target.as_ref().unwrap_or(t);
            let filter = path.filter.as_deref().filter(|f| *f != "latest");
            let mut url = match &category {
                Some(c) => {
                    let category_path = c.full_slug(&mut conn).await?;
                    let mode = if path.no_subcategories { "none/" } else { "" };
                    format!(
                        "{base_path}/tags/c/{category_path}/{mode}{}/{}",
                        t.slug_for_url(),
                        t.id
                    )
                }
                None => t.url(&base_path),
            };
            if let Some(f) = filter {
                url.push_str(&format!("/l/{f}"));
            }
            if path.json {
                url.push_str(".json");
            }
            if let Some(q) = raw_query.as_deref().filter(|q| !q.is_empty()) {
                url.push_str(&format!("?{q}"));
            }
            return Ok(redirect(&headers, url));
        }
    }

    let tag_name = match (&tag, &path.tag) {
        (Some(t), _) => t.name.clone(),
        (None, TagRef::Name(name)) => name.clone(),
        (None, _) => unreachable!("id routes 404 without a tag"),
    };
    // tag_params: query tags[] win over the path tag, except that the
    // name-based routes prepend it (TopicQueryParams).
    let mut tags = query_tags(raw_query.as_deref());
    if tags.is_empty() {
        tags.push(tag_name.clone());
    } else if matches!(path.tag, TagRef::Name(_)) {
        tags.insert(0, tag_name.clone());
        tags.dedup();
    }
    let request = if tag_name == "none" {
        TagListRequest {
            tags: Vec::new(),
            no_tags: true,
        }
    } else {
        TagListRequest {
            tags,
            no_tags: false,
        }
    };
    let action = path
        .filter
        .as_deref()
        .map(|f| format!("/l/{f}"))
        .unwrap_or_default();
    let list_path = match &category {
        Some(c) => {
            let category_path = c.full_slug(&mut conn).await?;
            match &path.tag {
                TagRef::SlugId(slug, id) => {
                    format!("{base_path}/tags/c/{category_path}/{slug}/{id}{action}")
                }
                TagRef::Id(_) => unreachable!("parsed as slug/id under a category"),
                TagRef::Name(name) => format!("{base_path}/tags/c/{category_path}/{name}{action}"),
            }
        }
        None => match &path.tag {
            TagRef::SlugId(slug, id) => format!("{base_path}/tag/{slug}/{id}{action}"),
            TagRef::Id(id) => format!("{base_path}/tag/{id}{action}"),
            TagRef::Name(name) => format!("{base_path}/tag/{name}{action}"),
        },
    };
    drop(conn);

    let (doc, settings) = match list_document_for(
        &state,
        &guardian,
        &params,
        ListScope {
            category: category.as_ref(),
            no_subcategories: path.no_subcategories,
            kind,
            list_path: &list_path,
            tag_request: Some(&request),
        },
    )
    .await?
    {
        Ok(doc) => doc,
        Err((status, message)) => return Ok((status, message).into_response()),
    };
    let mut conn = state.pool.acquire().await?;
    let empty = doc["topic_list"]["topics"]
        .as_array()
        .is_some_and(|t| t.is_empty());
    if empty && tag_name != "none" {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM tags WHERE lower(name) = $1)")
                .bind(tag_name.to_lowercase())
                .fetch_one(&mut *conn)
                .await?;
        if !exists {
            return Ok(not_found());
        }
    }
    if path.json {
        return Ok(Json(doc).into_response());
    }
    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site = crate::html::Site::from_settings(&settings, &base_path)?;
    site.load_chrome(&state, &settings).await?;
    site.viewer = vs.viewer.clone();
    site.bus_position = bus_position;
    let mut page =
        crate::html::latest_page(&mut conn, site, &doc, &state.i18n, &settings, true).await?;
    if let Some(c) = &category {
        page.heading = Some(
            crate::html::category_heading(&mut conn, &base_path, c, params.page.is_none()).await?,
        );
    }
    page.tag = Some(crate::html::TagHeading {
        name: tag_name.clone(),
        url: match &tag {
            Some(t) => t.url(&base_path),
            None => format!("{base_path}/tag/{tag_name}"),
        },
    });
    // canonical_url: the canonical tag route for this action; the title
    // and description meta from rss_by_tag and the tag's description.
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let canonical = format!("{}{list_path}", urls.base_url_no_prefix()?);
    let title = state
        .i18n
        .t_with("rss_by_tag", &[("tag", &request.tags.join(" & "))])
        .unwrap_or_else(|| format!("Topics tagged {}", request.tags.join(" & ")));
    let description = tag
        .as_ref()
        .and_then(|t| t.description.clone())
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| title.clone());
    let image = crate::html::site_opengraph_image(&mut conn, &urls).await?;
    let mut crawler = crate::html::Crawler::for_request(&urls, &uri, Some(canonical))?.with_meta(
        &title,
        &description,
        image,
    );
    crawler.description = description;
    page.crawler = crawler;
    let body = page.render().map_err(crate::html::HtmlError::from)?;
    Ok(crate::html::crawler_response(
        body,
        &page.crawler,
        &settings,
        &vs,
    )?)
}

/// GET /tags(.json) -> tags#index with tags_listed_by_group off.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    index_response(state, guardian, false, headers, Some(uri)).await
}

pub async fn index_json(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    index_response(state, guardian, true, headers, None).await
}

#[derive(sqlx::FromRow)]
struct CountRow {
    id: i32,
    name: String,
    slug: Option<String>,
    description: Option<String>,
    public_topic_count: i32,
    staff_topic_count: i32,
    pm_topic_count: i32,
}

impl CountRow {
    /// `tag_counts_json` entry; browsable tags have no target_tag.
    fn json(&self, staff_counts: bool, pm_count: bool) -> Value {
        let count = if staff_counts {
            self.staff_topic_count
        } else {
            self.public_topic_count
        };
        let mut out = json!({
            "id": self.id,
            "text": self.name,
            "name": self.name,
            "slug": self.slug.as_deref().filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| format!("{}-tag", self.id)),
            "description": self.description,
            "count": count,
            "pm_only": count == 0 && self.pm_topic_count > 0,
            "target_tag": null,
        });
        if pm_count {
            out["pm_count"] = json!(self.pm_topic_count);
        }
        out
    }
}

const COUNT_COLUMNS: &str = "tags.id, tags.name, tags.slug, tags.description, \
    tags.public_topic_count, tags.staff_topic_count, tags.pm_topic_count";

async fn index_response(
    state: AppState,
    guardian: Guardian,
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
    if !settings.get("tagging_enabled")?.truthy() {
        return Ok(super::topics::not_found_response(&state, false));
    }
    if settings.get("tags_listed_by_group")?.truthy() {
        return Err(Unsupported("tags_listed_by_group").into());
    }
    let count_column = guardian.tag_count_column(&settings)?;
    let staff_counts = count_column == "staff_topic_count";
    // show_pm_tags && display_personal_messages_tag_counts
    let pm_count = guardian.can_tag_pms(&settings)?
        && settings
            .get("display_personal_messages_tag_counts")?
            .truthy();
    // show_all_tags?: admins also get unused and PM-only tags.
    let used = if guardian.is_admin() {
        String::new()
    } else {
        format!(" AND tags.{count_column} > 0")
    };
    let visible = visible_tags_where(&guardian, &settings)?;
    // Tag.browsable(guardian).used_tags_in_regular_topics(guardian).order(:id)
    let tags: Vec<CountRow> = sqlx::query_as(&format!(
        "SELECT {COUNT_COLUMNS} FROM tags WHERE tags.target_tag_id IS NULL AND {visible} \
         {used} ORDER BY tags.id"
    ))
    .fetch_all(&mut *conn)
    .await?;
    let allowed = guardian.allowed_category_ids(&mut conn, &settings).await?;
    // Categories with category_tags among the allowed ones, by id; each
    // with its visible base tags minus the PM-only ones.
    let category_ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM categories WHERE id IN (SELECT category_id FROM category_tags WHERE category_id = ANY($1)) ORDER BY id",
    )
    .bind(&allowed)
    .fetch_all(&mut *conn)
    .await?;
    // without_pm_only_tags, skipped for admins.
    let pm_only = if guardian.is_admin() {
        String::new()
    } else {
        format!(" AND NOT (tags.pm_topic_count > 0 AND tags.{count_column} = 0)")
    };
    let mut category_names = Vec::new();
    let mut categories = Vec::new();
    for category_id in category_ids {
        let rows: Vec<CountRow> = sqlx::query_as(&format!(
            "SELECT {COUNT_COLUMNS} FROM tags JOIN category_tags ct ON ct.tag_id = tags.id \
             WHERE ct.category_id = $1 AND tags.target_tag_id IS NULL AND {visible} \
             {pm_only} ORDER BY ct.id"
        ))
        .bind(category_id)
        .fetch_all(&mut *conn)
        .await?;
        if rows.is_empty() {
            continue;
        }
        let name: String = sqlx::query_scalar("SELECT name FROM categories WHERE id = $1")
            .bind(category_id)
            .fetch_one(&mut *conn)
            .await?;
        category_names.push((i64::from(category_id), name));
        categories.push(json!({
            "id": category_id,
            "tags": rows.iter().map(|r| r.json(staff_counts, pm_count)).collect::<Vec<_>>(),
        }));
    }
    let doc = json!({
        "tags": tags.iter().map(|r| r.json(staff_counts, pm_count)).collect::<Vec<_>>(),
        "extras": { "categories": categories },
    });
    if json {
        return Ok(Json(doc).into_response());
    }
    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site =
        crate::html::Site::from_settings(&settings, state.config.globals.relative_url_root())?;
    site.load_chrome(&state, &settings).await?;
    site.viewer = vs.viewer.clone();
    site.bus_position = bus_position;
    let mut page = crate::html::tags_page(&state.i18n, site, &doc, &category_names);
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let title = state.i18n.t("tags.title").unwrap_or("Tags").to_string();
    let image = crate::html::site_opengraph_image(&mut conn, &urls).await?;
    let mut crawler = crate::html::Crawler::for_request(&urls, &uri.unwrap_or_default(), None)?
        .with_meta(&title, &title, image);
    crawler.description = title;
    page.crawler = crawler;
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
    fn parses_tag_routes() {
        let p = parse_path("howto.json", false).unwrap();
        assert!(matches!(p.tag, TagRef::Name(ref n) if n == "howto"));
        assert!(p.json && p.filter.is_none());

        let p = parse_path("howto/3/l/top", false).unwrap();
        assert!(matches!(p.tag, TagRef::SlugId(ref s, 3) if s == "howto"));
        assert_eq!(p.filter.as_deref(), Some("top"));

        let p = parse_path("3.json", false).unwrap();
        assert!(matches!(p.tag, TagRef::Id(3)));
        assert!(parse_path("3", false).is_err());

        let p = parse_path("general/4/none/howto/3.json", true).unwrap();
        assert_eq!(p.category_path.as_deref(), Some("general/4"));
        assert!(p.no_subcategories);
        assert!(matches!(p.tag, TagRef::SlugId(ref s, 3) if s == "howto"));

        let p = parse_path("general/sub-general/34/howto", true).unwrap();
        assert_eq!(p.category_path.as_deref(), Some("general/sub-general/34"));
        assert!(matches!(p.tag, TagRef::Name(ref n) if n == "howto"));
    }

    #[test]
    fn next_url_carries_the_tag_params() {
        let request = TagListRequest {
            tags: vec!["howto".into()],
            no_tags: false,
        };
        let params = ListParams {
            per_page: Some("1".into()),
            ..ListParams::default()
        };
        let options = Options {
            per_page: Some(1),
            ..Options::default()
        };
        assert_eq!(
            next_url("/tag/howto", &params, &options, &request),
            "/tag/howto?match_all_tags=true&page=1&per_page=1&tags%5B%5D=howto"
        );
    }
}
