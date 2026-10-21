//! The badge pages: BadgesController#index and #show (GET /badges,
//! /badges/:id(/:slug)) as JSON, as the pages Ember draws from it
//! (crate::badges_page) and as the badge's RSS feed,
//! UserBadgesController#index (GET /user_badges) and #username (GET
//! /user-badges/:username), whose page in a browser is Ember's unknown
//! route (refused).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::badges_view::{
    BadgeRow, BadgesView, GROUPED_COLUMNS, Side, USER_BADGE_COLUMNS, UserBadgeRow, badge_json,
    badge_type_json, detailed_document,
};
use crate::guardian::Guardian;
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::users::User;
use crate::{AppError, AppState, Unsupported};

/// `UserBadgesController::MAX_BADGES`
const MAX_BADGES: i64 = 96;

/// The format a path segment ends with: the segment and its format.
fn strip_format(segment: &str) -> (&str, Format) {
    if let Some(s) = segment.strip_suffix(".json") {
        (s, Format::Json)
    } else if let Some(s) = segment.strip_suffix(".rss") {
        (s, Format::Rss)
    } else if let Some(s) = segment.strip_suffix(".html") {
        (s, Format::Html)
    } else {
        (segment, Format::Html)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Html,
    Json,
    Rss,
}

/// after_action :add_noindex_header
fn noindex(doc: Value) -> Response {
    ([("x-robots-tag", "noindex")], Json(doc)).into_response()
}

/// BadgesController#index's badges document: every badge, or the
/// enabled and listable ones, matching the search; `has_badge` for a
/// current user.
async fn index_doc(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    search: Option<&str>,
    listable_only: bool,
) -> Result<Value, AppError> {
    let listable = if listable_only {
        "AND b.enabled AND b.listable"
    } else {
        ""
    };
    // name ILIKE "%#{search}%": the search's own % and _ stay wildcards.
    // Badge.all: in the table's own order (sorted client side), read
    // without the joins that would reorder it.
    let ids: Vec<i32> = sqlx::query_scalar(&format!(
        "SELECT b.id FROM badges b WHERE ($1::text IS NULL OR b.name ILIKE '%' || $1 || '%') {listable}"
    ))
    .bind(search)
    .fetch_all(&mut *conn)
    .await?;
    let mut loaded: Vec<BadgeRow> = sqlx::query_as(&format!(
        "{} WHERE b.id = ANY($1)",
        crate::badges_view::BADGE_ROW_SELECT,
    ))
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut rows = Vec::with_capacity(ids.len());
    for id in &ids {
        if let Some(i) = loaded.iter().position(|b| b.id == *id) {
            rows.push(loaded.swap_remove(i));
        }
    }
    // current_user.user_badges (enabled badges), distinct badge ids
    let has: Option<Vec<i32>> = match guardian.user_id() {
        Some(uid) => Some(
            sqlx::query_scalar(
                "SELECT DISTINCT badge_id FROM user_badges WHERE user_id = $1 \
                 AND badge_id IN (SELECT id FROM badges WHERE enabled)",
            )
            .bind(uid)
            .fetch_all(&mut *conn)
            .await?,
        ),
        None => None,
    };
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let base_path = state.config.globals.relative_url_root();
    let mut badges = Vec::new();
    let mut types: Vec<Value> = Vec::new();
    let mut type_ids: Vec<i32> = Vec::new();
    let mut groupings: Vec<Value> = Vec::new();
    let mut grouping_ids: Vec<i32> = Vec::new();
    for b in &rows {
        // BadgeIndexSerializer#has_badge, when there is a current user.
        let has_badge = has.as_ref().map(|h| h.contains(&b.id));
        badges.push(badge_json(
            &state.i18n,
            settings,
            &urls,
            base_path,
            b,
            true,
            has_badge,
        )?);
        if !type_ids.contains(&b.badge_type_id) {
            type_ids.push(b.badge_type_id);
            types.push(badge_type_json(b.badge_type_id, &b.badge_type_name));
        }
        if !grouping_ids.contains(&b.badge_grouping_id) {
            grouping_ids.push(b.badge_grouping_id);
            if let Some(g) =
                crate::badges_view::badge_grouping_json(&mut *conn, b.badge_grouping_id).await?
            {
                groupings.push(g);
            }
        }
    }
    Ok(json!({
        "badge_types": types,
        "badge_groupings": groupings,
        "badges": badges,
    }))
}

fn list_context<'a>(
    state: &'a AppState,
    frame: &'a super::groups::Frame,
    settings: &SiteSettings,
) -> Result<crate::topic_list_view::ListContext<'a>, AppError> {
    Ok(crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: state.config.globals.relative_url_root(),
        now: crate::clock::now(),
        categories: &frame.categories,
        expand_all_pinned: false,
        member_trust_level: frame.site.viewer.as_ref().map(|v| v.trust_level),
        settings: crate::topic_list_view::ListSettings::load(settings)?,
    })
}

/// The `noindex` header of every badges response on a page.
fn with_noindex(mut response: Response) -> Response {
    response.headers_mut().insert(
        "x-robots-tag",
        axum::http::HeaderValue::from_static("noindex"),
    );
    response
}

/// GET /badges(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("enable_badges")?.truthy() {
        return Ok(super::topics::not_found_response(&state));
    }
    let json = uri.path().ends_with(".json") || super::wants_json(uri.query(), &headers);
    let p = params::parse(uri.query(), &headers, b"");
    let xhr = headers
        .get("x-requested-with")
        .is_some_and(|v| v == "XMLHttpRequest");
    let only_listable = p
        .get("only_listable")
        .and_then(params::scalar)
        .is_some_and(|v| v == "true");
    let search = p.get("search").and_then(params::scalar);
    let listable_only = !guardian.is_staff() || only_listable || !xhr;
    let doc = index_doc(
        &state,
        &mut conn,
        &settings,
        &guardian,
        search.as_deref(),
        listable_only,
    )
    .await?;
    if json {
        return Ok(noindex(doc));
    }
    // The page: badges/index drawn from the preloaded badges.
    let mut frame = super::groups::frame(
        &state,
        &mut conn,
        &settings,
        &guardian,
        &headers,
        &uri,
        crate::sidebar::Active::Badges,
    )
    .await?;
    let cx = list_context(&state, &frame, &settings)?;
    let main = crate::badges_page::render_index(&cx, &doc)?;
    let page_title = crate::topic_list_view::t(&cx, "badges.title");
    drop(cx);
    frame.site.chrome.body_classes.push_str(" badges-page");
    let response = super::groups::finish(frame, &settings, page_title, main)?;
    Ok(with_noindex(response))
}

/// GET /badges/:id
pub async fn show(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let (id, format) = strip_format(&id);
    show_badge(state, guardian, id.to_string(), format, headers, uri).await
}

/// GET /badges/:id/:slug
pub async fn show_with_slug(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path((id, slug)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let (_, format) = strip_format(&slug);
    show_badge(state, guardian, id, format, headers, uri).await
}

/// BadgesController#show: the enabled badge, the viewer's grant
/// notification marked read; as JSON, the page or the RSS feed.
async fn show_badge(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    id: String,
    format: Format,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let format = if format == Format::Html && super::wants_json(uri.query(), &headers) {
        Format::Json
    } else {
        format
    };
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("enable_badges")?.truthy() {
        return Ok(super::topics::not_found_response(&state));
    }
    // Badge.enabled.find(params[:id])
    let id = i32::try_from(crate::ruby::to_i(&id)).unwrap_or(0);
    let enabled: Option<bool> = sqlx::query_scalar("SELECT enabled FROM badges WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    if enabled != Some(true) {
        return Ok(super::topics::not_found_response(&state));
    }
    let mut has_badge = false;
    if let Some(uid) = guardian.user_id() {
        // UserBadge.find_by(user_id:, badge_id:): the first grant.
        let grant: Option<Option<i64>> = sqlx::query_scalar(
            "SELECT notification_id FROM user_badges WHERE user_id = $1 AND badge_id = $2 \
             ORDER BY id LIMIT 1",
        )
        .bind(uid)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(notification_id) = grant {
            has_badge = true;
            if let Some(nid) = notification_id {
                // notification.update(read: true): a change writes and
                // refreshes the count.
                let owner: Option<i32> = sqlx::query_scalar(
                    "UPDATE notifications SET read = TRUE, updated_at = clock_timestamp() \
                     WHERE id = $1 AND NOT read RETURNING user_id",
                )
                .bind(nid)
                .fetch_optional(&mut *tx)
                .await?;
                if let Some(owner) = owner {
                    crate::bus::publish_notifications_state(&state.bus, &mut tx, &settings, owner)
                        .await?;
                }
            }
        }
    }
    let badge = BadgeRow::load(&mut tx, id).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let body = badge_json(
        &state.i18n,
        &settings,
        &urls,
        state.config.globals.relative_url_root(),
        &badge,
        true,
        has_badge.then_some(true),
    )?;
    tx.commit().await?;
    let doc = json!({
        "badge_types": [badge_type_json(badge.badge_type_id, &badge.badge_type_name)],
        "badge": body,
    });
    let display_name = badge.display_name(&state.i18n);
    let site_title = settings.get("title")?.to_s().to_string();
    let rss_title = state
        .i18n
        .t_with(
            "rss_description.badge",
            &[("display_name", &display_name), ("site_title", &site_title)],
        )
        .unwrap_or_else(|| format!("{display_name} badge on {site_title}"));
    let rss_link = format!(
        "{}/badges/{}/{}",
        urls.base_url()?,
        badge.id,
        body["slug"].as_str().unwrap_or_default()
    );
    match format {
        Format::Json => Ok(noindex(doc)),
        Format::Rss => {
            let mut conn = state.pool.acquire().await?;
            let feed = rss(&mut conn, &settings, &doc, &rss_title, &rss_link).await?;
            Ok((
                [
                    (header::CONTENT_TYPE, "application/rss+xml; charset=utf-8"),
                    (header::HeaderName::from_static("x-robots-tag"), "noindex"),
                ],
                feed,
            )
                .into_response())
        }
        Format::Html => {
            show_page(
                &state,
                &guardian,
                &settings,
                &doc,
                &headers,
                &uri,
                (&rss_title, &rss_link),
            )
            .await
        }
    }
}

/// ERB's `<%= %>`: HTML escaped.
fn erb_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// badges/show.rss.erb: the badge, and its last 50 grants of existing
/// users, newest first.
async fn rss(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    doc: &Value,
    title: &str,
    link: &str,
) -> Result<String, AppError> {
    let badge_id = doc["badge"]["id"].as_i64().unwrap_or(0) as i32;
    let description = doc["badge"]["long_description"]
        .as_str()
        .unwrap_or_default();
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<rss version=\"2.0\" xmlns:discourse=\"http://www.discourse.org/\" xmlns:atom=\"http://www.w3.org/2005/Atom\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n  <channel>\n    <title>{}</title>\n    <link>{}</link>\n    <description><![CDATA[{}]]></description>\n",
        erb_escape(title),
        erb_escape(link),
        erb_escape(description)
    );
    // @badge.user_badges.first.present?
    let any: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_badges WHERE badge_id = $1)")
            .bind(badge_id)
            .fetch_one(&mut *conn)
            .await?;
    if any {
        out.push_str(&format!(
            "      <atom:link href=\"{}\" rel=\"self\" type=\"application/rss+xml\" />\n",
            erb_escape(&format!("{link}.rss"))
        ));
        let enable_names = settings.get("enable_names")?.truthy();
        #[derive(sqlx::FromRow)]
        struct Grant {
            username: String,
            name: Option<String>,
            granted_at: chrono::NaiveDateTime,
            by_username: Option<String>,
            by_name: Option<String>,
        }
        let rows: Vec<Grant> = sqlx::query_as(
            "SELECT u.username, u.name, ub.granted_at, g.username AS by_username, g.name AS by_name \
                 FROM user_badges ub \
                 JOIN users u ON u.id = ub.user_id LEFT JOIN users g ON g.id = ub.granted_by_id \
                 WHERE ub.badge_id = $1 ORDER BY ub.id DESC LIMIT 50",
        )
        .bind(badge_id)
        .fetch_all(&mut *conn)
        .await?;
        let who = |username: &str, name: Option<&str>| match name.filter(|n| !n.trim().is_empty()) {
            Some(n) if enable_names => format!("@{username} {n}"),
            _ => format!("@{username}"),
        };
        for Grant {
            username,
            name,
            granted_at,
            by_username,
            by_name,
        } in rows
        {
            let by_username =
                by_username.ok_or(Unsupported("a badge feed's grant by a missing user"))?;
            out.push_str(&format!(
                "        <item>\n          <title><![CDATA[{}]]></title>\n          <discourse:grantedAt>{}</discourse:grantedAt>\n          <discourse:grantedBy><![CDATA[{}]]></discourse:grantedBy>\n          <guid>{}</guid>\n        </item>\n",
                erb_escape(&who(&username, name.as_deref())),
                granted_at.and_utc().format("%a, %d %b %Y %H:%M:%S +0000"),
                erb_escape(&who(&by_username, by_name.as_deref())),
                erb_escape(&format!("{link}?username={username}"))
            ));
        }
    }
    out.push_str("  </channel>\n</rss>\n");
    Ok(out)
}

/// The badge's page: badges/show drawn from the badge and its grants
/// (`/user_badges.json?badge_id=` with the page's username).
async fn show_page(
    state: &AppState,
    guardian: &Guardian,
    settings: &SiteSettings,
    doc: &Value,
    headers: &HeaderMap,
    uri: &Uri,
    (rss_title, rss_link): (&str, &str),
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let p = params::parse(uri.query(), headers, b"");
    let username = p.get("username").and_then(params::scalar);
    let mut grants_params = Map::new();
    grants_params.insert("badge_id".into(), doc["badge"]["id"].clone());
    if let Some(u) = &username {
        grants_params.insert("username".into(), json!(u));
    }
    let grants = match user_badges_doc(state, &mut conn, settings, guardian, &grants_params).await?
    {
        Ok(doc) => doc,
        Err(response) => return Ok(response),
    };
    let mut frame = super::groups::frame(
        state,
        &mut conn,
        settings,
        guardian,
        headers,
        uri,
        crate::sidebar::Active::Badges,
    )
    .await?;
    let cx = list_context(state, &frame, settings)?;
    let (main, more) = crate::badges_page::render_show(
        &cx,
        &crate::badges_page::Show {
            badge: doc,
            grants: &grants,
            username: username.as_deref(),
            enable_badges: true,
        },
    )?;
    let name = doc["badge"]["name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let page_title = name.clone();
    drop(cx);
    if more {
        frame.site.chrome.show_footer = false;
    }
    let site_title = settings.get("title")?.to_s().to_string();
    let meta_title = state
        .i18n
        .t_with(
            "badges.badge_title_metadata",
            &[("display_name", &name), ("site_title", &site_title)],
        )
        .unwrap_or_else(|| format!("{name} badge on {site_title}"));
    frame.crawler = frame.crawler.clone().with_meta(
        settings.emojis(),
        &meta_title,
        doc["badge"]["long_description"]
            .as_str()
            .unwrap_or_default(),
        None,
    );
    frame.crawler.rss = Some((format!("{rss_link}.rss"), rss_title.to_string()));
    let response = super::groups::finish(frame, settings, page_title, main)?;
    Ok(with_noindex(response))
}

/// `fetch_badge_from_params`: by badge_name, else the required badge_id;
/// enabled badges only. Err is the response.
async fn fetch_badge(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    p: &Map<String, Value>,
) -> Result<Result<i32, Response>, AppError> {
    let id: Option<i32> = match p.get("badge_name").and_then(params::scalar) {
        Some(name) => {
            sqlx::query_scalar(
                "SELECT id FROM badges WHERE name = $1 AND enabled ORDER BY id LIMIT 1",
            )
            .bind(name)
            .fetch_optional(&mut *conn)
            .await?
        }
        None => {
            let Some(raw) = p
                .get("badge_id")
                .and_then(params::scalar)
                .filter(|v| !v.trim().is_empty())
            else {
                return Ok(Err(super::accounts::param_missing("badge_id")));
            };
            let raw = i32::try_from(crate::ruby::to_i(&raw)).unwrap_or(0);
            sqlx::query_scalar("SELECT id FROM badges WHERE id = $1 AND enabled")
                .bind(raw)
                .fetch_optional(&mut *conn)
                .await?
        }
    };
    Ok(id.ok_or_else(|| super::topics::not_found_response(state)))
}

/// UserBadgesController#index's document; Err is the response.
async fn user_badges_doc(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    p: &Map<String, Value>,
) -> Result<Result<Value, Response>, AppError> {
    let badge_id = match fetch_badge(state, &mut *conn, p).await? {
        Ok(id) => id,
        Err(response) => return Ok(Err(response)),
    };
    let username = p.get("username").and_then(params::scalar);
    let mut user_id = None;
    let mut grant_count = None;
    if let Some(name) = &username {
        // fetch_user_from_params(include_inactive: true), can_see_profile?
        let name = name.strip_suffix(".json").unwrap_or(name);
        let Some(user) = User::find_any(&mut *conn, name).await? else {
            return Ok(Err(super::topics::not_found_response(state)));
        };
        if !user.visible_to(settings, guardian)? {
            return Ok(Err(super::topics::not_found_response(state)));
        }
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM user_badges WHERE badge_id = $1 AND user_id = $2",
        )
        .bind(badge_id)
        .bind(user.id)
        .fetch_one(&mut *conn)
        .await?;
        user_id = Some(user.id);
        grant_count = Some(count);
    }
    // fetch_int_from_params(:offset, default: 0)
    let offset = match p.get("offset").and_then(params::scalar) {
        None => 0,
        Some(raw) => match raw.trim().parse::<i64>() {
            Ok(n) => n.max(0),
            Err(_) => return Ok(Err(super::search::invalid_parameters(state, "offset"))),
        },
    };
    // granted_before is permitted but never read.
    let rows: Vec<UserBadgeRow> = sqlx::query_as(&format!(
        "SELECT {USER_BADGE_COLUMNS} FROM user_badges \
         WHERE badge_id = $1 AND ($2::int IS NULL OR user_id = $2) \
         ORDER BY granted_at DESC, id DESC LIMIT $3 OFFSET $4"
    ))
    .bind(badge_id)
    .bind(user_id)
    .bind(MAX_BADGES)
    .bind(offset)
    .fetch_all(&mut *conn)
    .await?;
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let mut view = BadgesView {
        conn: &mut *conn,
        settings,
        i18n: &state.i18n,
        guardian,
        urls: &urls,
        base_path: state.config.globals.relative_url_root(),
    };
    let allowed = view.allowed_topic_ids(&rows).await?;
    let mut side = Side::default();
    let mut items = Vec::new();
    for row in &rows {
        items.push(view.user_badge_json(&mut side, row, &allowed).await?);
    }
    let mut doc = Map::new();
    doc.insert("badges".into(), Value::Array(side.badges));
    doc.insert("badge_types".into(), Value::Array(side.badge_types));
    doc.insert("users".into(), Value::Array(side.users));
    if !side.topics.is_empty() {
        doc.insert("topics".into(), Value::Array(side.topics));
    }
    doc.insert(
        "user_badge_info".into(),
        json!({"grant_count": grant_count, "username": username, "user_badges": items}),
    );
    Ok(Ok(Value::Object(doc)))
}

/// GET /user_badges(.json)
pub async fn user_badges_index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("enable_badges")?.truthy() {
        return Ok(super::topics::not_found_response(&state));
    }
    let p = params::parse(uri.query(), &headers, b"");
    Ok(
        match user_badges_doc(&state, &mut conn, &settings, &guardian, &p).await? {
            Ok(doc) => Json(doc).into_response(),
            Err(response) => response,
        },
    )
}

/// UserBadgesController#username's document for a user the viewer may
/// see: the user's grants of enabled badges, grouped per badge when
/// asked.
pub(super) async fn username_doc(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    user_id: i32,
    grouped: bool,
) -> Result<Value, AppError> {
    // user.user_badges: the enabled badges', in the database's order.
    let sql = if grouped {
        format!(
            "SELECT {GROUPED_COLUMNS} FROM user_badges WHERE user_badges.user_id = $1 \
             AND (user_badges.badge_id IN (SELECT id FROM badges WHERE enabled)) \
             GROUP BY user_badges.badge_id"
        )
    } else {
        format!(
            "SELECT {USER_BADGE_COLUMNS} FROM user_badges WHERE user_badges.user_id = $1 \
             AND (user_badges.badge_id IN (SELECT id FROM badges WHERE enabled))"
        )
    };
    let rows: Vec<UserBadgeRow> = sqlx::query_as(&sql)
        .bind(user_id)
        .fetch_all(&mut *conn)
        .await?;
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let mut view = BadgesView {
        conn: &mut *conn,
        settings,
        i18n: &state.i18n,
        guardian,
        urls: &urls,
        base_path: state.config.globals.relative_url_root(),
    };
    let allowed = view.allowed_topic_ids(&rows).await?;
    let mut side = Side::default();
    let mut items = Vec::new();
    for row in &rows {
        items.push(
            view.detailed_user_badge(&mut side, row, None, Some(&allowed))
                .await?,
        );
    }
    Ok(detailed_document(side, "user_badges", Value::Array(items)))
}

/// GET /user-badges/:username(.json)
pub async fn username(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(username): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let (username, format) = strip_format(&username);
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("enable_badges")?.truthy() {
        return Ok(super::topics::not_found_response(&state));
    }
    // The Ember app has no route for the HTML page: its not-found page.
    if format != Format::Json && !super::wants_json(uri.query(), &headers) {
        let bus_position = crate::bus::page_position(&state.bus).await?;
        return super::user_preferences::ember_not_found(
            &state,
            &mut conn,
            &settings,
            &guardian,
            &headers,
            bus_position,
        )
        .await;
    }
    // fetch_user_from_params(include_inactive: staff, or a user with
    // show_inactive_accounts)
    let include_inactive = guardian.is_staff()
        || (guardian.is_authenticated() && settings.get("show_inactive_accounts")?.truthy());
    let user = if include_inactive
        || guardian
            .user()
            .is_some_and(|u| u.username.eq_ignore_ascii_case(username))
    {
        User::find_any(&mut conn, username).await?
    } else {
        User::find_active(&mut conn, username).await?
    };
    let Some(user) = user else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !user.visible_to(&settings, &guardian)? {
        return Ok(super::topics::not_found_response(&state));
    }
    let p = params::parse(uri.query(), &headers, b"");
    let grouped = p
        .get("grouped")
        .and_then(params::scalar)
        .is_some_and(|v| !v.is_empty());
    let doc = username_doc(&state, &mut conn, &settings, &guardian, user.id, grouped).await?;
    Ok(Json(doc).into_response())
}
