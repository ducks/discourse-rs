//! Port of app/controllers/topics_controller.rb#show for anonymous users,
//! JSON only for now.

use askama::Template;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::guardian::Guardian;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_view::{CHUNK_SIZE, Options, TopicView, TopicViewError};
use crate::url::Urls;
use crate::{AppError, AppState};

#[derive(Deserialize, Default)]
pub struct ShowParams {
    page: Option<String>,
    /// `params[:show_deleted]`: present at all, deleted replies stay in.
    show_deleted: Option<String>,
    /// `?post_number=`: params[:post_number] when the path has none.
    post_number: Option<String>,
    /// `?slug=`: params[:slug] when the path has none (path params win).
    slug: Option<String>,
}

/// `/t/:id` and `/t/:slug` (the id route also catches a bare slug).
#[allow(clippy::too_many_arguments)]
pub async fn show_by_id(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    Query(params): Query<ShowParams>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    requested: Option<axum::Extension<super::RequestedUri>>,
    super::search::Peer(peer): super::search::Peer,
) -> Result<Response, AppError> {
    let (id, json) = split_format(&id);
    show(
        Incoming {
            headers: &headers,
            uri: &uri,
            requested: requested.as_ref().map(|r| &r.0),
            guardian: &guardian,
            peer,
        },
        &state,
        None,
        None,
        &id,
        None,
        &params,
        json,
    )
    .await
}

/// The post number Rails' `t/.../last` routes pass (`post_number:
/// 99_999_999`), else the segment as given.
fn route_post_number(segment: &str) -> &str {
    if segment == "last" {
        "99999999"
    } else {
        segment
    }
}

fn all_digits(segment: &str) -> bool {
    !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit())
}

/// `/t/:slug/:topic_id`
#[allow(clippy::too_many_arguments)]
pub async fn show_with_slug(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((slug, id)): Path<(String, String)>,
    Query(params): Query<ShowParams>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    requested: Option<axum::Extension<super::RequestedUri>>,
    super::search::Peer(peer): super::search::Peer,
) -> Result<Response, AppError> {
    let (id, json) = split_format(&id);
    let incoming = Incoming {
        headers: &headers,
        uri: &uri,
        requested: requested.as_ref().map(|r| &r.0),
        guardian: &guardian,
        peer,
    };
    // Rails declares `t/:topic_id/:post_number` and `t/:topic_id/last`
    // before `t/:slug/:topic_id`, so two numbers (or a number and `last`)
    // are a topic and a post number, not a slug and a topic.
    if all_digits(&slug) && (all_digits(&id) || id == "last") {
        return show(
            incoming,
            &state,
            None,
            Some(&slug),
            &slug,
            Some(route_post_number(&id)),
            &params,
            json,
        )
        .await;
    }
    show(
        incoming,
        &state,
        Some(&slug),
        Some(&id),
        &id,
        None,
        &params,
        json,
    )
    .await
}

/// `/t/:slug/:topic_id/:post_number`
#[allow(clippy::too_many_arguments)]
pub async fn show_post(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((slug, id, post_number)): Path<(String, String, String)>,
    Query(params): Query<ShowParams>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    requested: Option<axum::Extension<super::RequestedUri>>,
    super::search::Peer(peer): super::search::Peer,
) -> Result<Response, AppError> {
    let (post_number, json) = split_format(&post_number);
    show(
        Incoming {
            headers: &headers,
            uri: &uri,
            requested: requested.as_ref().map(|r| &r.0),
            guardian: &guardian,
            peer,
        },
        &state,
        Some(&slug),
        Some(&id),
        &id,
        Some(route_post_number(&post_number)),
        &params,
        json,
    )
    .await
}

/// A query param as Rack 2's parse_nested_query leaves it.
enum QueryParam {
    Missing,
    /// `key` without `=`: nil.
    Nil,
    Str(String),
    /// `key[]` or `key[child]`: an array or a hash.
    Nested,
}

/// `params[key]` from the query string: the last plain `key` wins.
fn query_param(query: Option<&str>, key: &str) -> QueryParam {
    let mut found = QueryParam::Missing;
    for pair in query.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (k, Some(v)),
            None => (pair, None),
        };
        let k = decode(k);
        if k == key {
            found = match v {
                Some(v) => QueryParam::Str(decode(v)),
                None => QueryParam::Nil,
            };
        } else if k
            .strip_prefix(key)
            .is_some_and(|rest| rest.starts_with('['))
        {
            found = QueryParam::Nested;
        }
    }
    found
}

/// A query component, `+` as a space.
fn decode(s: &str) -> String {
    form_urlencoded::parse(format!("x={s}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

/// Ruby truthiness of the param: anything but nil.
fn truthy(param: &QueryParam) -> bool {
    matches!(param, QueryParam::Str(_) | QueryParam::Nested)
}

/// Rails' `(.:format)`: a trailing `.json` on the last segment.
fn split_format(segment: &str) -> (String, bool) {
    match segment.strip_suffix(".json") {
        Some(s) => (s.to_string(), true),
        None => (segment.to_string(), false),
    }
}

/// `render_json_error I18n.t(:not_found)`.
pub(crate) fn not_found_response(state: &AppState) -> Response {
    let text = state
        .i18n
        .t("not_found")
        .unwrap_or("The requested URL or resource could not be found.");
    (
        StatusCode::NOT_FOUND,
        Json(json!({"errors": [text], "error_type": "not_found"})),
    )
        .into_response()
}

/// topics#show's `extras` on an error response: the not-found page
/// (build_not_found_page), for `slug` (params[:slug] || params[:id]).
pub(crate) async fn not_found_extras(
    state: &AppState,
    guardian: &Guardian,
    slug: &str,
    forbidden: bool,
    custom_message: Option<&str>,
) -> Result<serde_json::Value, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let page = crate::not_found_page::NotFound {
        forbidden,
        custom_message,
        slug,
        guardian,
    };
    let html =
        crate::not_found_page::build(&mut conn, &settings, &state.i18n, &urls, &page).await?;
    Ok(json!({
        "title": state.i18n.t("page_not_found.page_title").unwrap_or("Page Not Found"),
        "html": html,
        "group": null,
    }))
}

/// topics#show's 404: not_found with the extras.
pub(crate) async fn topic_not_found(
    state: &AppState,
    guardian: &Guardian,
    slug: &str,
) -> Result<Response, AppError> {
    let text = state
        .i18n
        .t("not_found")
        .unwrap_or("The requested URL or resource could not be found.");
    let extras = not_found_extras(state, guardian, slug, false, None).await?;
    Ok((
        StatusCode::NOT_FOUND,
        Json(json!({"errors": [text], "error_type": "not_found", "extras": extras})),
    )
        .into_response())
}

/// topics#show's NotFound and InvalidAccess rescues with `check_permalinks`:
/// a missing topic looks up the path asked for (`/t/<slug>/<id>` when both
/// were given, else the request path), a deleted one the viewer could
/// otherwise see its own URL; then the 404.
async fn topic_missing(
    state: &AppState,
    incoming: &Incoming<'_>,
    slug: Option<&str>,
    id: &str,
    topic_id_param: Option<&str>,
    page_slug: &str,
) -> Result<Response, AppError> {
    if let Some(path) = permalink_path(state, incoming, slug, id, topic_id_param).await?
        && let Some(redirect) =
            crate::permalinks::check(state, incoming.guardian, incoming.headers, &path).await?
    {
        return Ok(redirect);
    }
    topic_not_found(state, incoming.guardian, page_slug).await
}

/// topics#show's InvalidAccess rescue with detailed_404 on: a deleted
/// topic the viewer could otherwise see is a 410 "deleted_topic" (after
/// the permalink check on its own URL), anything else the 403 itself.
async fn detailed_invalid_access(
    state: &AppState,
    incoming: &Incoming<'_>,
    topic_id: i32,
    topic_id_param: Option<&str>,
    page_slug: &str,
) -> Result<Response, AppError> {
    let guardian = incoming.guardian;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let Some(topic) =
        crate::topic_guardian::TopicCtx::load(&mut conn, &settings, guardian, topic_id).await?
    else {
        drop(conn);
        return topic_not_found(state, guardian, page_slug).await;
    };
    // Notification.remove_for(current_user.id, params[:topic_id])
    if let Some(user_id) = guardian.user_id() {
        sqlx::query(
            "DELETE FROM notifications WHERE user_id = $1 AND topic_id IS NOT DISTINCT FROM $2",
        )
        .bind(user_id)
        .bind(topic_id_param.map(|t| crate::ruby::to_i(t) as i32))
        .execute(&mut *conn)
        .await?;
    }
    let secure = guardian.secure_category_ids(&mut conn, &settings).await?;
    let can_see = guardian.can_see_topic(&settings, &topic, true, &secure)?;
    let via_group = access_topic_via_group(&mut conn, &topic).await?;
    let deleted = guardian.can_see_topic(&settings, &topic, false, &secure)?
        || (!can_see && via_group && topic.deleted_at.is_some());
    let (status, error_type, message, custom_message) = if deleted {
        let stored: Option<String> = sqlx::query_scalar("SELECT slug FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?;
        let base_path = state.config.globals.relative_url_root();
        let path = match stored.filter(|s| !s.is_empty()) {
            Some(s) => format!("{base_path}/t/{s}/{topic_id}"),
            None => format!("{base_path}/t/{topic_id}"),
        };
        drop(conn);
        if let Some(redirect) =
            crate::permalinks::check(state, guardian, incoming.headers, &path).await?
        {
            return Ok(redirect);
        }
        (
            StatusCode::GONE,
            "not_found",
            state
                .i18n
                .t("deleted_topic")
                .unwrap_or("Oops! This topic has been deleted and is no longer available."),
            Some("deleted_topic"),
        )
    } else if !can_see && via_group {
        return Err(crate::Unsupported("detailed_404 for a topic a group gives access to").into());
    } else {
        drop(conn);
        (
            StatusCode::FORBIDDEN,
            "invalid_access",
            state
                .i18n
                .t("invalid_access")
                .unwrap_or("You are not permitted to view the requested resource."),
            None,
        )
    };
    let forbidden = status == StatusCode::FORBIDDEN;
    let extras = not_found_extras(state, guardian, page_slug, forbidden, custom_message).await?;
    let mut response = (
        status,
        Json(json!({"errors": [message], "error_type": error_type, "extras": extras})),
    )
        .into_response();
    response
        .extensions_mut()
        .insert(super::not_found::ErrorPage {
            status,
            forbidden,
            custom_message,
        });
    Ok(response)
}

/// The path topics#show checks for a permalink, None when it checks none.
async fn permalink_path(
    state: &AppState,
    incoming: &Incoming<'_>,
    slug: Option<&str>,
    id: &str,
    topic_id_param: Option<&str>,
) -> Result<Option<String>, AppError> {
    let requested = match slug.filter(|s| !s.is_empty()) {
        Some(s) => format!("/t/{s}/{id}"),
        // request.path, as requested.
        None => incoming
            .requested
            .map(|r| &r.0)
            .unwrap_or(incoming.uri)
            .path()
            .to_string(),
    };
    let Ok(topic_id) = id.parse::<i32>() else {
        return Ok(Some(requested));
    };
    let guardian = incoming.guardian;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let Some(topic) =
        crate::topic_guardian::TopicCtx::load(&mut conn, &settings, guardian, topic_id).await?
    else {
        return Ok(Some(requested));
    };
    // A private message for a visitor is NotLoggedIn, rescued as a plain
    // NotFound.
    if topic.private_message() && guardian.is_anonymous() {
        return Ok(None);
    }
    let secure = guardian.secure_category_ids(&mut conn, &settings).await?;
    if guardian.can_see_topic(&settings, &topic, true, &secure)? {
        return Ok(Some(requested));
    }
    // InvalidAccess: Notification.remove_for(current_user.id,
    // params[:topic_id]). Without a topic_id param (`/t/:id`) that is
    // `topic_id IS NULL`: every notification not about a topic.
    if let Some(user_id) = guardian.user_id() {
        sqlx::query(
            "DELETE FROM notifications WHERE user_id = $1 AND topic_id IS NOT DISTINCT FROM $2",
        )
        .bind(user_id)
        .bind(topic_id_param.map(|t| crate::ruby::to_i(t) as i32))
        .execute(&mut *conn)
        .await?;
    }
    // `deleted` decides, the topic's own URL is looked up.
    let deleted = guardian.can_see_topic(&settings, &topic, false, &secure)?
        || (topic.deleted_at.is_some() && access_topic_via_group(&mut conn, &topic).await?);
    if !deleted {
        return Ok(None);
    }
    let stored: Option<String> = sqlx::query_scalar("SELECT slug FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
    let base_path = state.config.globals.relative_url_root();
    Ok(Some(match stored.filter(|s| !s.is_empty()) {
        Some(s) => format!("{base_path}/t/{s}/{topic_id}"),
        None => format!("{base_path}/t/{topic_id}"),
    }))
}

/// `Topic#access_topic_via_group`: a group of the topic's category that
/// takes members by admission or request.
async fn access_topic_via_group(
    conn: &mut sqlx::PgConnection,
    topic: &crate::topic_guardian::TopicCtx,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM groups g JOIN category_groups cg ON cg.group_id = g.id \
         WHERE cg.category_id = $1 AND (g.public_admission OR g.allow_membership_requests))",
    )
    .bind(topic.category_id)
    .fetch_one(conn)
    .await
}

/// Rails `redirect_to` a path: absolute with the request's host and scheme.
fn redirect(headers: &HeaderMap, location: String) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let location = format!("http://{host}{location}");
    (
        StatusCode::MOVED_PERMANENTLY,
        [(header::LOCATION, location)],
    )
        .into_response()
}

/// The parts of the request the topic page reads.
struct Incoming<'a> {
    headers: &'a HeaderMap,
    uri: &'a axum::http::Uri,
    /// The URI before request_format's `.json` rewrite, when it made one.
    requested: Option<&'a super::RequestedUri>,
    guardian: &'a Guardian,
    /// The peer address, for `request.remote_ip`.
    peer: Option<std::net::SocketAddr>,
}

#[allow(clippy::too_many_arguments)]
async fn show(
    incoming: Incoming<'_>,
    state: &AppState,
    slug: Option<&str>,
    // params[:topic_id]: every route but `/t/:id` has one.
    topic_id_param: Option<&str>,
    id: &str,
    post_number: Option<&str>,
    params: &ShowParams,
    json: bool,
) -> Result<Response, AppError> {
    // Before the page's data is read (crate::bus::page_position).
    let bus_position = if json {
        String::new()
    } else {
        crate::bus::page_position(&state.bus).await?
    };
    let (headers, uri, guardian) = (incoming.headers, incoming.uri, incoming.guardian);
    let slug = slug.or(params.slug.as_deref());
    // build_not_found_page searches for params[:slug] || params[:id].
    let page_slug = slug.filter(|s| !s.is_empty()).unwrap_or(id).to_string();
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let base_path = state.config.globals.relative_url_root().to_string();
    let format = if json { ".json" } else { "" };

    // Discourse::InvalidParameters for non-scalar ids can't happen here.
    let page = crate::ruby::to_i(params.page.as_deref().unwrap_or(""));
    // topic_not_found takes its own connection: this one goes first.
    if page < 0 {
        drop(conn);
        return topic_not_found(state, guardian, &page_slug).await;
    }

    // A slug in the id position (`/t/my-topic`, or `123abc`) redirects to
    // the canonical URL.
    let topic_id: i32 = match id.parse() {
        Ok(n) => n,
        Err(_) => {
            let found: Option<(i32, String)> = sqlx::query_as(
                "SELECT id, slug FROM topics WHERE slug = $1 AND deleted_at IS NULL ORDER BY id LIMIT 1",
            )
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
            drop(conn);
            return Ok(match found {
                Some((id, slug)) => redirect(
                    headers,
                    canonical(&base_path, &slug, id, None, page, format),
                ),
                None => {
                    topic_missing(state, &incoming, slug, id, topic_id_param, &page_slug).await?
                }
            });
        }
    };
    let post_number: Option<i64> = match post_number {
        Some(n) => Some(
            n.parse()
                .map_err(|_| crate::Unsupported("non-numeric post_number"))?,
        ),
        // A query post_number, as TopicView reads it (`to_i`).
        None => params
            .post_number
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(crate::ruby::to_i),
    };

    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let mut view = TopicView {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian,
        urls: &urls,
        options: Options {
            page,
            post_number,
            show_deleted: params.show_deleted.is_some(),
            post_ids: None,
        },
        post_types: Vec::new(),
        logo_small_url: None,
    };
    let mut rendered = match view.render(topic_id).await {
        Ok(r) => r,
        Err(TopicViewError::NotFound) => {
            drop(view);
            drop(conn);
            return topic_missing(state, &incoming, slug, id, topic_id_param, &page_slug).await;
        }
        Err(TopicViewError::InvalidAccess) => {
            drop(view);
            drop(conn);
            return detailed_invalid_access(state, &incoming, topic_id, topic_id_param, &page_slug)
                .await;
        }
        Err(TopicViewError::NotLoggedIn) => {
            // detailed_404 re-raises NotLoggedIn: a 403 with the
            // not-found page as extras (a page request gets the 404).
            drop(view);
            drop(conn);
            let extras = not_found_extras(state, guardian, &page_slug, true, None).await?;
            let mut response = super::login_required::not_logged_in(state);
            let body = json!({
                "errors": [state.i18n.t("not_logged_in").unwrap_or("You need to be logged in to do that.")],
                "error_type": "not_logged_in",
                "extras": extras,
            });
            *response.body_mut() = axum::body::Body::from(body.to_string());
            return Ok(response);
        }
        Err(e) => return Err(e.into()),
    };

    // Pages past the end redirect to the last one.
    if page > 1 {
        let count = rendered.json["post_stream"]["stream"]
            .as_array()
            .map(Vec::len)
            .unwrap_or(0) as i64;
        if (page - 1) * CHUNK_SIZE >= count {
            let last_page = ((count - 1).max(0) / CHUNK_SIZE) + 1;
            let mut location = format!("{base_path}/t/{}/{topic_id}{format}", rendered.slug);
            if last_page > 1 {
                location.push_str(&format!("?page={last_page}"));
            }
            return Ok(redirect(headers, location));
        }
    }

    // slugs_do_not_match, or a non-JSON request without a slug.
    let slug_mismatch = slug.is_some_and(|s| s != rendered.slug) || (slug.is_none() && !json);
    if slug_mismatch {
        return Ok(redirect(
            headers,
            canonical(
                &base_path,
                &rendered.slug,
                topic_id,
                post_number,
                page,
                format,
            ),
        ));
    }

    // track_visit_to_topic: the incoming link of a page request and the
    // member's visit, both deferred past the response; then the member's
    // draft.
    let current_user_id = guardian.user_id();
    let track_visit =
        current_user_id.is_some() && (!json || truthy(&query_param(uri.query(), "track_visit")));
    if !json {
        // TopicView#current_post_number
        let current_post_number = rendered.json["highest_post_number"]
            .as_i64()
            .map(|highest| post_number.unwrap_or(0).max(1).min(highest));
        let visit = crate::incoming_link::Visit {
            referer: headers
                .get(header::REFERER)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string),
            host: crate::incoming_link::request_host(headers, uri)?,
            current_user_id,
            topic_id,
            post_number: current_post_number,
            username: match query_param(uri.query(), "u") {
                QueryParam::Str(u) => Some(u),
                _ => None,
            },
            ip_address: crate::remote_ip::remote_ip(headers, incoming.peer)?,
        };
        let pool = state.pool.clone();
        crate::request_tracking::later("Track Link", async move {
            let mut conn = pool.acquire().await?;
            crate::incoming_link::add(&mut conn, visit).await
        });
    }
    if let Some(user_id) = current_user_id.filter(|_| track_visit) {
        let task_state = state.clone();
        crate::request_tracking::later("Track Visit", async move {
            let mut conn = task_state.pool.acquire().await?;
            crate::read_tracking::track_visit(&task_state.bus, &mut conn, topic_id, user_id).await
        });
        // Draft.get at the view's draft sequence (the current one).
        let (draft, _) =
            crate::drafts::show(&mut conn, user_id, &format!("topic_{topic_id}")).await?;
        rendered.json["draft"] = json!(draft);
    }

    if json {
        return Ok(Json(rendered.json).into_response());
    }
    let vs = super::session::viewer_state(state, headers, &settings, guardian)?;
    let mut site = crate::html::Site::from_settings(&settings, &base_path)?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(
        &mut conn,
        state,
        &settings,
        guardian,
        crate::sidebar::Active::None,
    )
    .await?;
    site.bus_position = bus_position;
    // TopicRoute#setupController: the topic's searchContext.
    site.chrome
        .set_search_context("topic", &rendered.json["id"].to_string());
    // A message's browse more line: the viewer's pmTopicTrackingState
    // counts for the inbox it reads, with this message read through the
    // posts the page shows.
    let highest_seen = rendered.json["post_stream"]["posts"]
        .as_array()
        .and_then(|p| p.iter().filter_map(|p| p["post_number"].as_i64()).max())
        .and_then(|n| i32::try_from(n).ok())
        .unwrap_or(0);
    let pm_counts = if rendered.json["archetype"] == "private_message" {
        Some(
            crate::pm_tracking_state::browse_more_counts(
                &mut conn,
                &settings,
                guardian,
                rendered.json["suggested_group_name"].as_str(),
                (topic_id, highest_seen),
            )
            .await?,
        )
    } else {
        None
    };
    let mut page = crate::html::topic_page(
        &mut conn,
        &state.i18n,
        &settings,
        site,
        &rendered.json,
        page,
        pm_counts,
    )
    .await?;
    page.crawler = topic_crawler(
        &mut conn,
        state,
        &settings,
        uri,
        &page.canonical_url,
        &rendered.json,
        rendered.page_image.map_err(crate::Unsupported)?,
    )
    .await?;
    let body = page.render().map_err(crate::html::HtmlError::from)?;
    Ok(crate::html::crawler_response(
        body,
        &page.crawler,
        &settings,
        &vs,
    )?)
}

/// The crawler block of topics/show: canonical_url from the topic view,
/// the description from the topic's stored excerpt else the first shown
/// post's summary, TopicView#image_url else the site's image.
async fn topic_crawler(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    uri: &axum::http::Uri,
    canonical_path: &str,
    view: &serde_json::Value,
    page_image: crate::topic_view::PageImage,
) -> Result<crate::html::Crawler, AppError> {
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let canonical = format!("{}{canonical_path}", urls.base_url_no_prefix()?);
    let title = view["title"].as_str().unwrap_or("").to_string();
    // TopicView#summary: 500 characters of the first post, links stripped,
    // entities as text, newlines as spaces.
    let first_cooked = view["post_stream"]["posts"][0]["cooked"]
        .as_str()
        .unwrap_or("");
    let summary = crate::excerpt::excerpt(
        first_cooked,
        500,
        &crate::excerpt::Options {
            strip_links: true,
            text_entities: true,
            image_mode: crate::excerpt::ImageMode::Strip,
            ..Default::default()
        },
        crate::excerpt::Plugins::from_settings(settings)?,
    )
    .replace('\n', " ")
    .trim()
    .to_string();
    let topic_id = view["id"].as_i64().unwrap_or(0) as i32;
    let stored: Option<Option<String>> =
        sqlx::query_scalar("SELECT excerpt FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_optional(&mut *conn)
            .await?;
    // plain_text_excerpt: the stored excerpt without tags, entities decoded.
    let plain = stored
        .flatten()
        .map(|e| html_escape::decode_html_entities(&strip_tags(&e)).into_owned())
        .filter(|e| !e.is_empty());
    let description = plain.unwrap_or_else(|| summary.clone());
    let image = match page_image.url.as_deref().filter(|u| !u.is_empty()) {
        Some(u) => Some(absolute_image_url(&urls, u)?),
        None => crate::html::site_opengraph_image(&mut *conn, &urls).await?,
    };
    let mut crawler = crate::html::Crawler::for_request(&urls, uri, Some(canonical))?.with_meta(
        urls.settings.emojis(),
        &title,
        &summary,
        image,
    );
    crawler.description = description;
    // image_width, image_height and image_type: TopicView#image_upload's.
    if let Some(upload_id) = page_image.upload_id {
        let upload: Option<(Option<i32>, Option<i32>, Option<String>)> =
            sqlx::query_as("SELECT width, height, extension FROM uploads WHERE id = $1")
                .bind(upload_id)
                .fetch_optional(&mut *conn)
                .await?;
        if let Some((width, height, extension)) = upload {
            crawler.image_size = width.zip(height);
            crawler.image_type = extension
                .filter(|e| !e.is_empty())
                .and_then(|e| crate::file_store::mime_type(&format!("x.{e}")));
        }
    }
    Ok(crawler)
}

/// `ApplicationHelper#get_absolute_image_url`: a protocol-relative link
/// takes the base URL's scheme, an upload, image or avatar path the base
/// URL, a path under the subfolder the base URL without it.
fn absolute_image_url(urls: &crate::url::Urls<'_>, link: &str) -> Result<String, AppError> {
    if link.starts_with("//") {
        return Ok(format!("{}:{link}", urls.scheme()?));
    }
    if ["/uploads/", "/images/", "/user_avatar/"]
        .iter()
        .any(|p| link.starts_with(p))
    {
        return Ok(format!("{}{link}", urls.base_url()?));
    }
    let root = urls.config.globals.relative_url_root();
    if !root.is_empty() && link.starts_with(root) {
        return Ok(format!("{}{link}", urls.base_url_no_prefix()?));
    }
    Ok(link.to_string())
}

/// `gsub(/<[^>]*>/, "")`
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// `redirect_to_correct_topic`'s target.
fn canonical(
    base_path: &str,
    slug: &str,
    id: i32,
    post_number: Option<i64>,
    page: i64,
    format: &str,
) -> String {
    let mut url = format!("{base_path}/t/{slug}/{id}");
    if let Some(n) = post_number.filter(|n| *n > 0) {
        url.push_str(&format!("/{n}"));
    }
    url.push_str(format);
    if page > 0 {
        url.push_str(&format!("?page={page}"));
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_urls() {
        assert_eq!(canonical("", "a-b", 5, None, 0, ".json"), "/t/a-b/5.json");
        assert_eq!(
            canonical("/f", "a-b", 5, Some(3), 2, ""),
            "/f/t/a-b/5/3?page=2"
        );
        assert_eq!(split_format("35.json"), ("35".to_string(), true));
        assert_eq!(split_format("35"), ("35".to_string(), false));
    }
}
