//! The activity tabs users#show answers for the Ember app that the stream
//! doesn't draw: `/u/:username/activity/{topics,read,drafts,pending,
//! bookmarks}` and the bookmarks-with-reminders redirect. With them the
//! JSON they act through: posts#pending, reviewables#destroy (a queued
//! post's author deleting it), drafts#bulk_destroy (in drafts.rs) and
//! bookmarks#bulk.

use askama::Template;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::list::ListParams;
use super::session::{bad_csrf, csrf_ok};
use crate::html::Crawler;
use crate::params;
use crate::session::current::{AuthGuardian, Incoming};
use crate::site_settings::SiteSettings;
use crate::topic_list_view::{icon, t, t_with};
use crate::url::Urls;
use crate::user_activity_view::{self as view, Bookmarks, Draft};
use crate::user_profile_view::Tab;
use crate::users::{User, Users};
use crate::{AppError, AppState, Unsupported};

/// The `userActivity.*` routes this module draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Topics,
    Read,
    Drafts,
    Pending,
    Bookmarks,
    /// `userActivity.bookmarksWithReminders`: redirects to the bookmarks.
    BookmarksWithReminders,
}

impl Page {
    pub fn from_path(rest: &str) -> Option<Page> {
        let tail = rest.strip_prefix("activity/")?.trim_end_matches('/');
        Some(match tail {
            "topics" => Page::Topics,
            "read" => Page::Read,
            "drafts" => Page::Drafts,
            "pending" => Page::Pending,
            "bookmarks" => Page::Bookmarks,
            "bookmarks-with-reminders" => Page::BookmarksWithReminders,
            _ => return None,
        })
    }

    fn path(self) -> &'static str {
        match self {
            Page::Topics => "topics",
            Page::Read => "read",
            Page::Drafts => "drafts",
            Page::Pending => "pending",
            Page::Bookmarks | Page::BookmarksWithReminders => "bookmarks",
        }
    }

    /// The route's titleToken.
    fn title_key(self) -> &'static str {
        match self {
            Page::Topics => "user_action_groups.4",
            Page::Read => "user.read",
            Page::Drafts => "user_action_groups.15",
            Page::Pending => "user.activity_stream",
            Page::Bookmarks | Page::BookmarksWithReminders => "user_action_groups.3",
        }
    }
}

fn found(headers: &HeaderMap, location: String) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    (
        StatusCode::FOUND,
        [(header::LOCATION, format!("http://{host}{location}"))],
    )
        .into_response()
}

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// `user.pending_posts.order(created_at: :desc)` as PendingPostSerializer
/// writes them.
async fn pending_posts(
    conn: &mut sqlx::PgConnection,
    urls: &Urls<'_>,
    user_id: i32,
) -> Result<Vec<Value>, AppError> {
    type Row = (
        i64,
        Option<i32>,
        chrono::NaiveDateTime,
        Option<i32>,
        Value,
        Option<i32>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i32>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT r.id, r.category_id, r.created_at, r.target_created_by_id, r.payload, \
                t.id, t.title, t.slug, u.username, u.name, u.uploaded_avatar_id \
         FROM reviewables r \
         LEFT JOIN topics t ON t.id = r.topic_id AND t.deleted_at IS NULL \
         LEFT JOIN users u ON u.id = r.target_created_by_id \
         WHERE r.type = 'ReviewableQueuedPost' AND r.status = 0 AND r.target_created_by_id = $1 \
         ORDER BY r.created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (
        id,
        category_id,
        created_at,
        by_id,
        payload,
        topic_id,
        title,
        slug,
        username,
        name,
        upload,
    ) in rows
    {
        let avatar_template = match (&username, by_id) {
            (Some(username), Some(by_id)) => Some(crate::avatar::avatar_template(
                urls, by_id, username, upload, None,
            )?),
            _ => None,
        };
        // Topic#url: on the base url.
        let topic_url = match topic_id {
            Some(tid) => Some(format!(
                "{}/t/{}/{tid}",
                urls.base_url()?,
                slug.as_deref().unwrap_or_default()
            )),
            None => None,
        };
        out.push(json!({
            "id": id,
            "avatar_template": avatar_template,
            "category_id": category_id,
            "created_at": crate::topic_list::time_json(created_at),
            "created_by_id": by_id,
            "name": name,
            "raw_text": payload["raw"],
            "title": payload.get("title").filter(|t| !t.is_null()).cloned().unwrap_or(json!(title)),
            "topic_id": topic_id,
            "topic_url": topic_url,
            "username": username,
        }));
    }
    Ok(out)
}

/// Whether the request wants JSON (check_xhr): a .json path or an XHR.
fn wants_json(headers: &HeaderMap, json_suffix: bool) -> bool {
    json_suffix
        || headers
            .get("x-requested-with")
            .and_then(|h| h.to_str().ok())
            .is_some_and(|h| h == "XMLHttpRequest")
}

/// GET /posts/:username/pending(.json): PostsController#pending.
pub async fn pending(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(username): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let suffix = uri.path().ends_with(".json");
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !wants_json(&headers, suffix) {
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
    let Some(viewer) = guardian.user() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let username_lower = username.to_lowercase();
    let user_id: Option<i32> = if viewer.username.to_lowercase() == username_lower {
        Some(viewer.id)
    } else {
        sqlx::query_scalar("SELECT id FROM users WHERE username_lower = $1 AND (active OR $2)")
            .bind(&username_lower)
            .bind(guardian.is_staff())
            .fetch_optional(&mut *conn)
            .await?
    };
    // can_edit_user?
    let Some(user_id) = user_id.filter(|id| guardian.is_me(*id) || guardian.is_staff()) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let posts = pending_posts(&mut conn, &urls, user_id).await?;
    Ok(Json(json!({ "pending_posts": posts })).into_response())
}

/// DELETE /review/:reviewable_id: ReviewablesController#destroy, the
/// `delete` action on a queued post the user created.
pub async fn destroy_reviewable(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(reviewable_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "DELETE") {
        return Ok(bad_csrf());
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    // :reviewable_id is constrained to digits; the format may follow.
    let digits = reviewable_id
        .strip_suffix(".json")
        .unwrap_or(&reviewable_id);
    let Some(reviewable_id) = digits
        .parse::<i64>()
        .ok()
        .filter(|_| digits.bytes().all(|b| b.is_ascii_digit()))
    else {
        return Ok(super::topics::not_found_response(&state));
    };
    // fetch_target_user
    if p.contains_key("username") || p.contains_key("external_id") {
        return Err(Unsupported("deleting another user's reviewable over the API").into());
    }
    let mut tx = state.pool.begin().await?;
    // Reviewable.find_by_flagger_or_queued_post_creator
    let kind: Option<String> = sqlx::query_scalar(
        "SELECT type FROM reviewables WHERE id = $1 AND (created_by_id = $2 \
         OR (target_created_by_id = $2 AND type = 'ReviewableQueuedPost'))",
    )
    .bind(reviewable_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(kind) = kind else {
        return Ok(super::topics::not_found_response(&state));
    };
    if kind != "ReviewableQueuedPost" {
        return Err(Unsupported("deleting a reviewable one flagged (only queued posts)").into());
    }
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let host = crate::pretty_text::Host::from_state(&state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let performed = crate::review_posts::perform(
        &mut tx,
        &ctx,
        &guardian,
        reviewable_id,
        "delete",
        None,
        &crate::review_posts::Args::default(),
        Some(&state.mailer),
        None,
    )
    .await?;
    match performed {
        crate::review_posts::Performed::Done { .. } => {
            tx.commit().await?;
            Ok(Json(json!({ "success": "OK" })).into_response())
        }
        // Reviewable::InvalidAction and UpdateConflict are not rescued.
        crate::review_posts::Performed::InvalidAction => Err(Unsupported(
            "deleting a reviewable without a delete action (a server error in Rails)",
        )
        .into()),
        crate::review_posts::Performed::Conflict => Err(Unsupported(
            "a reviewable version conflict on delete (a server error in Rails)",
        )
        .into()),
    }
}

/// PUT /bookmarks/bulk: BookmarksController#bulk, BookmarksBulkAction.
pub async fn bulk_bookmarks(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let ids: Vec<i64> = match p.get("bookmark_ids") {
        Some(Value::Array(ids)) if !ids.is_empty() => ids
            .iter()
            .map(|v| {
                params::scalar(v)
                    .map(|s| crate::ruby::to_i(&s))
                    .unwrap_or(0)
            })
            .collect(),
        Some(v) if params::scalar(v).is_some_and(|s| !crate::ruby::is_blank(&s)) => {
            return Ok(super::search::invalid_parameters(
                &state,
                "Expecting bookmark_ids to contain a list of bookmark ids",
            ));
        }
        Some(Value::Object(m)) if !m.is_empty() => {
            return Ok(super::search::invalid_parameters(
                &state,
                "Expecting bookmark_ids to contain a list of bookmark ids",
            ));
        }
        _ => return Ok(super::accounts::param_missing("bookmark_ids")),
    };
    let operation = match p.get("operation") {
        Some(Value::Object(op)) if !op.is_empty() => op,
        _ => return Ok(super::accounts::param_missing("operation")),
    };
    let Some(kind) = operation
        .get("type")
        .and_then(params::scalar)
        .filter(|t| !crate::ruby::is_blank(t))
    else {
        return Ok(super::accounts::param_missing("operation_type"));
    };
    if kind != "clear_reminder" && kind != "delete" {
        return Ok(super::search::invalid_parameters(&state, "operation"));
    }
    let mut tx = state.pool.begin().await?;
    let found: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT id::bigint, user_id::bigint FROM bookmarks WHERE id = ANY($1) ORDER BY id",
    )
    .bind(&ids)
    .fetch_all(&mut *tx)
    .await?;
    let mut changed = Vec::new();
    let mut reminders_deleted = false;
    let mut forbidden = false;
    for (id, owner) in found {
        // can_delete? and can_edit? on a bookmark: its owner. The ones
        // already done stay done, as Rails has no transaction around them.
        if owner != i64::from(user_id) {
            forbidden = true;
            break;
        }
        if kind == "delete" {
            match crate::bookmark_manager::destroy(&mut tx, user_id, id as i32).await? {
                crate::bookmark_manager::Outcome::Destroyed {
                    reminders_deleted: deleted,
                    ..
                } => reminders_deleted |= deleted,
                _ => return Err(Unsupported("a bulk bookmark delete the manager refused").into()),
            }
        } else {
            // clear_reminder!(force_clear_reminder_at: true)
            sqlx::query(
                "UPDATE bookmarks SET reminder_last_sent_at = clock_timestamp(), reminder_set_at = NULL, \
                 reminder_at = NULL, updated_at = clock_timestamp() WHERE id = $1",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
        changed.push(id);
    }
    tx.commit().await?;
    // The reminder notifications' after_commit refresh_notification_count.
    if reminders_deleted {
        let mut conn = state.pool.acquire().await?;
        let settings =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
        crate::bus::publish_notifications_state(&state.bus, &mut conn, &settings, user_id).await?;
    }
    if forbidden {
        return Ok(super::search::invalid_access(&state));
    }
    changed.sort();
    Ok(Json(json!({ "bookmark_ids": changed })).into_response())
}

/// The HTML of `/u/:username/activity/<page>`.
#[allow(clippy::too_many_arguments)]
pub async fn page(
    state: AppState,
    incoming: Incoming,
    username: String,
    page: Page,
    mut params: ListParams,
    headers: HeaderMap,
    peer: Option<std::net::SocketAddr>,
    uri: Uri,
) -> Result<Response, AppError> {
    let guardian = incoming.guardian.clone();
    let base_path = state.config.globals.relative_url_root().to_string();
    // The topic lists' routes have two query params, order and ascending;
    // only DLoadMore's next page (user-activity.js, an XHR) asks for a page.
    let next_page_request = wants_json(&headers, false);
    if !next_page_request {
        params = ListParams {
            order: params.order.take(),
            ascending: params.ascending.take(),
            ..ListParams::default()
        };
    }
    let current_page = params
        .page
        .as_deref()
        .map(crate::ruby::to_i)
        .unwrap_or(0)
        .max(0);
    if page == Page::BookmarksWithReminders {
        let mut location = format!("{base_path}/u/{username}/activity/bookmarks");
        if let Some(q) = uri.query().filter(|q| !q.is_empty()) {
            location.push('?');
            location.push_str(q);
        }
        return Ok(found(&headers, location));
    }
    let bus_position = crate::bus::page_position(&state.bus).await?;
    let auth_token: Option<String> = incoming
        .session
        .as_ref()
        .map(|s| s.token.auth_token.clone());
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let noindex = [("x-robots-tag", "noindex")];
    if settings.get("hide_user_profiles_from_public")?.truthy() && guardian.is_anonymous() {
        return Ok((noindex, super::search::invalid_access(&state)).into_response());
    }
    let Some(user) = User::find_active(&mut conn, &username).await? else {
        return Ok((noindex, super::topics::not_found_response(&state)).into_response());
    };
    if page == Page::Drafts {
        // UserActivityDrafts#beforeModel
        match guardian.user() {
            None => return Ok(found(&headers, format!("{base_path}/latest"))),
            Some(viewer) if viewer.id != user.id => {
                return Ok(found(
                    &headers,
                    format!(
                        "{base_path}/u/{}/activity/drafts",
                        viewer.username.to_lowercase()
                    ),
                ));
            }
            Some(_) => {}
        }
    }
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let mut users = Users {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        base_path: &base_path,
        auth_token: auth_token.as_deref(),
    };
    let show = users.show(&user).await?;
    if !guardian.is_me(user.id) && !next_page_request {
        let ip = crate::remote_ip::remote_ip(&headers, peer)?;
        users.track_view(&user, &ip).await?;
    }
    let categories = crate::topic_list_view::categories(&mut conn).await?;
    let cx = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: &base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        member_trust_level: guardian.user().map(|u| u.trust_level),
        settings: crate::topic_list_view::ListSettings::load(&settings)?,
    };
    let viewing_self = guardian.is_me(user.id);
    let username_lower = user.username.to_lowercase();
    // A model request that fails: a 403 leaves Ember's exception page,
    // naming the JSON, in the profile's place; posts#pending's 404 (another
    // member's) is the /404 route.
    let exception = match page {
        Page::Read | Page::Pending if guardian.is_anonymous() => Some(if page == Page::Read {
            format!("{base_path}/read.json")
        } else {
            format!("{base_path}/posts/{username_lower}/pending.json")
        }),
        Page::Pending if !(viewing_self || guardian.is_staff()) => {
            return Ok(found(&headers, format!("{base_path}/404")));
        }
        _ => None,
    };
    let mut content = match page {
        _ if exception.is_some() => String::new(),
        Page::Topics | Page::Read => {
            let doc = if page == Page::Topics {
                let mut options = match super::list::build_options(&params, &settings)? {
                    Ok(o) => o,
                    Err(message) => {
                        return Ok(super::list::invalid_list_params(&state, &message, false));
                    }
                };
                options.no_definitions = false;
                let list_path = format!(
                    "{base_path}/topics/created-by/{}",
                    user.username.to_lowercase()
                );
                let more = super::list::next_url(&list_path, &params, &options, false);
                super::list::topics_by_list(
                    &state, &mut conn, &settings, &guardian, user.id, options, more,
                )
                .await?
            } else {
                match super::list::list_document(
                    &state,
                    &guardian,
                    &params,
                    None,
                    false,
                    super::list::ListKind::Read,
                    &format!("{base_path}/read"),
                )
                .await?
                {
                    Ok((doc, _)) => doc,
                    Err(message) => {
                        return Ok(super::list::invalid_list_params(&state, &message, false));
                    }
                }
            };
            let (title, body) = match page {
                Page::Topics if viewing_self => (
                    t(&cx, "user_activity.no_topics_title"),
                    t_with(
                        &cx,
                        "user_activity.no_topics_body",
                        &[("searchUrl", &format!("{base_path}/search"))],
                    ),
                ),
                Page::Topics => (
                    t_with(
                        &cx,
                        "user_activity.no_topics_title_others",
                        &[("username", &user.username)],
                    ),
                    String::new(),
                ),
                _ => (
                    t(&cx, "user_activity.no_read_topics_title"),
                    t_with(
                        &cx,
                        "user_activity.no_read_topics_body",
                        &[
                            ("topUrl", &format!("{base_path}/top")),
                            ("categoriesUrl", &format!("{base_path}/categories")),
                            ("searchIcon", &icon("magnifying-glass", None)),
                        ],
                    ),
                ),
            };
            // UserTopicsListController#canBulkSelect: canManageTopic (staff
            // or a trust level 4 member); the dismiss buttons never show here.
            let can_manage =
                guardian.is_staff() || guardian.user().is_some_and(|u| u.trust_level >= 4);
            let next_page = doc["topic_list"]["more_topics_url"]
                .is_string()
                .then_some(current_page + 1);
            let sort = view::Sort {
                order: params.order.as_deref().filter(|o| !o.is_empty()),
                ascending: params.ascending.as_deref().is_some_and(|a| !a.is_empty()),
            };
            view::topic_list(&cx, &doc, &title, &body, can_manage, &sort, next_page)
        }
        Page::Drafts => {
            let rows = crate::drafts::stream(&mut conn, &urls, &guardian, user.id, 0, 30).await?;
            let host = crate::pretty_text::Host::from_state(&state);
            let mut drafts = Vec::with_capacity(rows.len());
            for row in &rows {
                let data: Value = row["data"]
                    .as_str()
                    .and_then(|d| serde_json::from_str(d).ok())
                    .unwrap_or(Value::Null);
                let reply = data["reply"].as_str().unwrap_or_default();
                let cooked =
                    crate::pretty_text::markdown(&host, reply, &Default::default()).await?;
                drafts.push((row, data, view::client_excerpt(&cooked, 300)));
            }
            let drafts: Vec<Draft> = drafts
                .into_iter()
                .map(|(row, data, excerpt)| Draft { row, data, excerpt })
                .collect();
            view::drafts(&cx, &drafts, &user.username)
        }
        Page::Pending => {
            let host = crate::pretty_text::Host::from_state(&state);
            let mut posts = Vec::new();
            for post in pending_posts(&mut conn, &urls, user.id).await? {
                let raw = post["raw_text"].as_str().unwrap_or_default();
                let cooked = crate::pretty_text::markdown(&host, raw, &Default::default()).await?;
                posts.push((post, cooked));
            }
            view::pending(&cx, &posts, viewing_self)
        }
        Page::Bookmarks | Page::BookmarksWithReminders => {
            let search = params::parse_query(uri.query().unwrap_or_default())
                .get("q")
                .and_then(params::scalar)
                .unwrap_or_default();
            let mut loaded = Vec::new();
            let mut next_page = None;
            let denied = !(viewing_self || guardian.is_admin());
            if !denied {
                let mut bookmarks = crate::bookmarks::Bookmarks {
                    conn: &mut conn,
                    settings: &settings,
                    i18n: &state.i18n,
                    guardian: &guardian,
                    urls: &urls,
                };
                let list = bookmarks
                    .load(
                        user.id,
                        &crate::bookmarks::ListQuery {
                            search_term: Some(search.as_str()),
                            page: current_page,
                            per_page: None,
                            exclude_ids: &[],
                        },
                    )
                    .await?;
                next_page = list.has_more.then_some(current_page + 1);
                if !list.is_empty() {
                    loaded = bookmarks.serialize(&list, false).await?;
                }
            }
            // Bookmark#timezone: currentUser.user_option.timezone, else the
            // browser's guess (UTC here).
            let zone: Option<String> = match guardian.user_id() {
                Some(id) => sqlx::query_scalar::<_, Option<String>>(
                    "SELECT timezone FROM user_options WHERE user_id = $1",
                )
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
                .flatten(),
                None => None,
            };
            let zone: chrono_tz::Tz = zone.and_then(|z| z.parse().ok()).unwrap_or(chrono_tz::UTC);
            let shown = if denied {
                Bookmarks::Denied
            } else if loaded.is_empty() && crate::ruby::is_blank(&search) {
                Bookmarks::None
            } else if loaded.is_empty() {
                Bookmarks::NothingFound
            } else {
                Bookmarks::List(&loaded)
            };
            view::bookmarks(
                &cx,
                &shown,
                &search,
                zone,
                crate::mobile::mobile_view(),
                next_page,
            )
        }
    };

    if exception.is_none() {
        content.push_str(&view::strings(&cx));
    }
    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site = crate::html::Site::from_settings(&settings, &base_path)?;
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
    site.chrome.set_search_context("user", &username_lower);
    if let Some(request_url) = exception {
        let page_html = super::users::ProfilePage {
            site_title: site.site_title,
            viewer: site.viewer,
            bus_position: site.bus_position,
            chrome: site.chrome,
            lang: site.lang,
            base_path: site.base_path,
            crawler: Crawler::default(),
            page_title: format!("{} - {}", t(&cx, "user.profile"), user.username),
            main: crate::messages_view::exception(&cx, 403, &request_url),
        };
        let response = (
            noindex,
            Html(page_html.render().map_err(crate::html::HtmlError::from)?),
        )
            .into_response();
        return Ok(crate::html::with_viewer_headers(response, &vs));
    }
    let mut page_html = super::users::profile_page(
        &mut conn,
        site,
        &state.i18n,
        &guardian,
        &user,
        &show,
        &Tab::ActivityPage(page.path(), content),
        &settings,
    )
    .await?;
    page_html
        .chrome
        .body_classes
        .push_str(" user-activity-page");
    if matches!(page, Page::Bookmarks) {
        page_html
            .chrome
            .body_classes
            .push_str(" user-activity-bookmarks-page");
    }
    // The route's titleToken, UserActivity's, the user's.
    page_html.page_title = if page == Page::Pending {
        format!("{} - {}", t(&cx, "user.activity_stream"), user.username)
    } else {
        format!(
            "{} - {} - {}",
            t(&cx, page.title_key()),
            t(&cx, "user.activity_stream"),
            user.username
        )
    };
    let avatar = show["user"]["avatar_template"]
        .as_str()
        .map(|t| urls.absolute(&t.replace("{size}", "45")))
        .transpose()?;
    let mut crawler = Crawler::for_request(&urls, &uri, None)?.with_meta(
        urls.settings.emojis(),
        &user.username,
        "",
        avatar,
    );
    crawler.description = settings.get("site_description")?.to_s();
    page_html.crawler = crawler;
    let response = (
        noindex,
        Html(page_html.render().map_err(crate::html::HtmlError::from)?),
    )
        .into_response();
    Ok(crate::html::with_viewer_headers(response, &vs))
}

/// The query of a page request.
pub fn list_params(uri: &Uri) -> ListParams {
    Query::<ListParams>::try_from_uri(uri)
        .map(|Query(p)| p)
        .unwrap_or_default()
}

/// DELETE /bookmarks/bulk: bookmarks#destroy with that id.
pub async fn destroy_bookmark_bulk(
    state: State<AppState>,
    guardian: AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    super::bookmark_writes::destroy(
        state,
        guardian,
        Path("bulk".to_string()),
        headers,
        uri,
        body,
    )
    .await
}
