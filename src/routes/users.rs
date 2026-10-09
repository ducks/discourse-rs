//! Port of users_controller#show and #summary and user_actions#index for
//! anonymous readers, plus the server-rendered profile page.

use askama::Template;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use super::search::Peer;
use crate::html::Chrome;
use crate::html::Crawler;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::user_profile_view::{ActivityFilter, Tab};
use crate::users::{PRIVATE_TYPES, PUBLIC_TYPES, User, Users};
use crate::{AppError, AppState, Unsupported};

#[derive(Deserialize, Default)]
pub struct ShowParams {
    skip_track_visit: Option<String>,
    include_post_count_for: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct ActionsParams {
    username: Option<String>,
    filter: Option<String>,
    offset: Option<String>,
    limit: Option<String>,
    acting_username: Option<String>,
}

/// `Discourse::InvalidAccess` as JSON.
fn invalid_access(state: &AppState) -> Response {
    let text = state
        .i18n
        .t("invalid_access")
        .unwrap_or("You are not permitted to view the requested resource.");
    (
        StatusCode::FORBIDDEN,
        Json(json!({"errors": [text], "error_type": "invalid_access"})),
    )
        .into_response()
}

enum Action {
    Show,
    Summary,
    /// The HTML of a user stream route (`/activity`, `/activity/replies`
    /// ...): users#show, drawn as that tab.
    Activity(ActivityFilter),
}

/// Where `/u/:username(/...)` goes.
enum Routed {
    /// UsersController: the username, the action, and whether the request
    /// wants JSON.
    Profile(String, Action, bool),
    /// `/activity.json`, PostsController#user_posts_feed: the username.
    PostsFeed(String),
}

/// `/u/:username(/...)`: which action the tail selects, and whether the
/// request wants JSON.
fn route(username: &str, rest: Option<&str>) -> Result<Routed, Unsupported> {
    let (username, json) = match username.strip_suffix(".json") {
        Some(u) => (u.to_string(), true),
        None => (username.to_string(), false),
    };
    let Some(rest) = rest else {
        return Ok(Routed::Profile(username, Action::Show, json));
    };
    let (rest, json) = match rest.strip_suffix(".json") {
        Some(r) => (r, true),
        None => (rest, json),
    };
    let action = match rest {
        "summary" => Action::Summary,
        "badges" | "notifications" | "messages" | "private-messages" | "deleted-posts" => {
            Action::Show
        }
        "activity" if json => return Ok(Routed::PostsFeed(username)),
        "activity" if !json => Action::Activity(ActivityFilter::All),
        r if !json
            && let Some(filter) = r
                .strip_prefix("activity/")
                .and_then(ActivityFilter::from_path)
                .filter(|f| *f != ActivityFilter::Topics) =>
        {
            Action::Activity(filter)
        }
        "activity" => Action::Show,
        r if r.starts_with("activity/")
            || r.starts_with("notifications/")
            || r.starts_with("messages/")
            || r.starts_with("private-messages/") =>
        {
            Action::Show
        }
        "card" => return Err(Unsupported("/u/:username/card.json")),
        _ => return Err(Unsupported("unknown /u/:username route")),
    };
    Ok(Routed::Profile(username, action, json))
}

/// GET /u/{username}
pub async fn show(
    State(state): State<AppState>,
    axum::Extension(incoming): axum::Extension<crate::session::current::Incoming>,
    Path(username): Path<String>,
    Query(params): Query<ShowParams>,
    headers: HeaderMap,
    Peer(peer): Peer,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    let Routed::Profile(username, action, json) = route(&username, None)? else {
        return Err(Unsupported("a feed without a path").into());
    };
    respond(
        state,
        incoming,
        Target {
            username,
            action,
            json,
        },
        params,
        headers,
        peer,
        uri,
    )
    .await
}

/// GET /u/{username}/{*rest}
pub async fn show_with_tail(
    State(state): State<AppState>,
    axum::Extension(incoming): axum::Extension<crate::session::current::Incoming>,
    Path((username, rest)): Path<(String, String)>,
    Query(params): Query<ShowParams>,
    headers: HeaderMap,
    Peer(peer): Peer,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    let (username, action, json) = match route(&username, Some(&rest))? {
        Routed::Profile(username, action, json) => (username, action, json),
        Routed::PostsFeed(username) => {
            return posts_feed(&state, &incoming.guardian, &username).await;
        }
    };
    respond(
        state,
        incoming,
        Target {
            username,
            action,
            json,
        },
        params,
        headers,
        peer,
        uri,
    )
    .await
}

/// Which user, action and format the path selected.
struct Target {
    username: String,
    action: Action,
    json: bool,
}

async fn respond(
    state: AppState,
    incoming: crate::session::current::Incoming,
    target: Target,
    params: ShowParams,
    headers: HeaderMap,
    peer: Option<std::net::SocketAddr>,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    // Before the page's data is read (crate::bus::page_position).
    let bus_position = if target.json {
        String::new()
    } else {
        crate::bus::page_position(&state.bus).await?
    };
    let guardian = incoming.guardian.clone();
    let auth_token: Option<String> = incoming
        .session
        .as_ref()
        .map(|s| s.token.auth_token.clone());
    let Target {
        username,
        action,
        json,
    } = target;
    if params.include_post_count_for.is_some() {
        return Err(Unsupported("include_post_count_for").into());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let noindex = [("x-robots-tag", "noindex")];
    // ensure_public_can_see_profiles!
    if settings.get("hide_user_profiles_from_public")?.truthy() {
        return Ok((noindex, invalid_access(&state)).into_response());
    }
    let not_found = || (noindex, super::topics::not_found_response(&state)).into_response();
    let Some(user) = User::find_active(&mut conn, &username).await? else {
        return Ok(not_found());
    };
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let base_path = state.config.globals.relative_url_root();
    let mut users = Users {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        base_path,
        auth_token: auth_token.as_deref(),
    };
    let doc = match action {
        Action::Show | Action::Activity(_) => {
            let doc = users.show(&user).await?;
            // Viewing one's own profile isn't tracked.
            if params.skip_track_visit.is_none() && !guardian.is_me(user.id) {
                let ip = super::search::remote_ip(&headers, peer);
                users.track_view(&user, &ip).await?;
            }
            doc
        }
        Action::Summary => {
            if !user.visible_to(&settings, &guardian)? {
                return Ok(not_found());
            }
            users.summary(&user).await?
        }
    };
    if json {
        return Ok((noindex, Json(doc)).into_response());
    }
    // The HTML profile draws on the show document, the summary and the
    // public activity stream.
    let show_doc = match action {
        Action::Show | Action::Activity(_) => doc,
        Action::Summary => users.show(&user).await?,
    };
    // user/index: one's own profile opens the activity stream, others
    // view_user_route.
    let filter = match action {
        Action::Summary => None,
        Action::Activity(filter) => Some(filter),
        Action::Show if guardian.is_me(user.id) => Some(ActivityFilter::All),
        Action::Show => match settings.get("view_user_route")?.to_s().as_str() {
            "summary" => None,
            "activity" => Some(ActivityFilter::All),
            _ => return Err(Unsupported("view_user_route other than summary or activity").into()),
        },
    };
    let visible = user.visible_to(&settings, &guardian)?;
    let summary_doc = if visible && filter.is_none() {
        Some(users.summary(&user).await?)
    } else {
        None
    };
    // The stream's first page (UserStream#findItems: offset 0, the
    // default limit); routes user_actions refuses show nothing.
    let show_votes = crate::plugins::topic_voting::enabled(&settings)?
        && settings.get("topic_voting_show_votes_on_profile")?.truthy();
    // discourse-topic-voting's votes tab: ListController#voted_by's first
    // page, which 404s while the tab is off.
    let mut voted_topics = Vec::new();
    if filter == Some(ActivityFilter::Votes) {
        if !show_votes {
            return Ok(super::topics::not_found_response(&state));
        }
        let doc = super::topic_voting::voted_by_list(
            &state,
            &mut *users.conn,
            &settings,
            &guardian,
            user.id,
            crate::topic_query::Options::default(),
            String::new(),
        )
        .await?;
        voted_topics = doc["topic_list"]["topics"]
            .as_array()
            .cloned()
            .unwrap_or_default();
    }
    // discourse-reactions' reactions tab: its route's first page of
    // CustomReactionsController#reactions_given, with the same checks.
    let mut reactions = Vec::new();
    if filter == Some(ActivityFilter::Reactions) {
        if !crate::plugins::reactions::enabled(&settings)? {
            return Ok(super::topics::not_found_response(&state));
        }
        if guardian.is_anonymous() {
            return Err(Unsupported("discourse-reactions' activity tab, anonymously").into());
        }
        let sees_private = guardian.is_me(user.id) || guardian.is_admin();
        if !visible || (!sees_private && settings.get("hide_user_activity_tab")?.truthy()) {
            return Ok(super::topics::not_found_response(&state));
        }
        let urls = crate::url::Urls {
            config: &state.config,
            settings: &settings,
        };
        let mut reader = crate::plugins::reactions::users::Reader {
            conn: &mut *users.conn,
            settings: &settings,
            urls: &urls,
            guardian: &guardian,
        };
        reactions = reader
            .reactions_given(user.id, None)
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default();
    }
    // discourse-solved's Solved tab: its route's first page of
    // SolvedTopicsController#by_user (20), with the same checks.
    let mut solved = Vec::new();
    if filter == Some(ActivityFilter::Solved) {
        if !crate::plugins::solved::enabled(&settings)? {
            return Ok(super::topics::not_found_response(&state));
        }
        let public = guardian.is_authenticated()
            || !settings.get("hide_user_profiles_from_public")?.truthy();
        let sees_private =
            guardian.is_authenticated() && (guardian.is_me(user.id) || guardian.is_admin());
        if !public
            || !visible
            || (!sees_private && settings.get("hide_user_activity_tab")?.truthy())
        {
            return Ok(super::topics::not_found_response(&state));
        }
        let host = crate::pretty_text::Host::from_state(&state);
        let ctx = crate::posting::Ctx {
            host: &host,
            settings: &settings,
            config: &state.config,
            i18n: &state.i18n,
            bus: &state.bus,
        };
        let urls = crate::url::Urls {
            config: &state.config,
            settings: &settings,
        };
        solved = crate::plugins::solved::by_user::by_user(
            &mut *users.conn,
            &ctx,
            &urls,
            &guardian,
            user.id,
            0,
            20,
        )
        .await?["user_solved_posts"]
            .as_array()
            .cloned()
            .unwrap_or_default();
    }
    let stream = match filter {
        Some(ActivityFilter::Votes | ActivityFilter::Reactions | ActivityFilter::Solved) => {
            Vec::new()
        }
        Some(filter) if visible && !settings.get("hide_user_activity_tab")?.truthy() => {
            users
                .actions(&user, filter.action_types(), 0, 30, None)
                .await?
        }
        _ => Vec::new(),
    };
    let tab = match filter {
        None => Tab::Summary(summary_doc.as_ref()),
        Some(filter) => Tab::Activity {
            filter,
            stream: &stream,
            topics: &voted_topics,
            reactions: &reactions,
            solved: &solved,
        },
    };
    let active = if filter == Some(ActivityFilter::All) && guardian.is_me(user.id) {
        crate::sidebar::Active::MyPosts
    } else {
        crate::sidebar::Active::None
    };
    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site = crate::html::Site::from_settings(&settings, base_path)?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(&mut conn, &state, &settings, &guardian, active)
        .await?;
    site.bus_position = bus_position;
    let mut page = profile_page(
        &mut conn,
        site,
        &state.i18n,
        &guardian,
        &user,
        &show_doc,
        &tab,
        &settings,
    )
    .await?;
    // The tab template's bodyClass.
    if show_doc["user"]["profile_hidden"] != true {
        page.chrome.body_classes.push_str(match filter {
            None => " user-summary-page",
            Some(_) => " user-activity-page",
        });
    }
    // crawlable_meta_data(title: username, image: the 45px avatar)
    let avatar = show_doc["user"]["avatar_template"]
        .as_str()
        .map(|t| urls.absolute(&t.replace("{size}", "45")))
        .transpose()?;
    let mut crawler =
        crate::html::Crawler::for_request(&urls, &uri, None)?.with_meta(&user.username, "", avatar);
    crawler.description = settings.get("site_description")?.to_s();
    page.crawler = crawler;
    let response = (
        noindex,
        Html(page.render().map_err(crate::html::HtmlError::from)?),
    )
        .into_response();
    Ok(crate::html::with_viewer_headers(response, &vs))
}

/// GET /user_actions.json
pub async fn actions(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ActionsParams>,
) -> Result<Response, AppError> {
    let Some(username) = params.username.as_deref().filter(|u| !u.is_empty()) else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(
                json!({"errors": ["param is missing or the value is empty or invalid: username"]}),
            ),
        )
            .into_response());
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let Some(user) = User::find_active(&mut conn, username).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let offset = params
        .offset
        .as_deref()
        .map(crate::ruby::to_i)
        .unwrap_or(0)
        .max(0);
    let mut action_types: Vec<i32> = params
        .filter
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| crate::ruby::to_i(s) as i32)
        .collect();
    let limit = params
        .limit
        .as_deref()
        .map(crate::ruby::to_i)
        .unwrap_or(30)
        .min(100);
    if limit < 0 {
        return Err(Unsupported("negative user_actions limit").into());
    }
    // ensure_user_actions_visible!: hidden profiles 404, and so do private
    // types and a hidden activity tab, except to the user themself and
    // admins (can_see_user_actions?).
    let sees_private =
        guardian.is_authenticated() && (guardian.is_me(user.id) || guardian.is_admin());
    if !user.visible_to(&settings, &guardian)?
        || (!sees_private
            && (settings.get("hide_user_activity_tab")?.truthy()
                || action_types.iter().any(|t| PRIVATE_TYPES.contains(t))))
    {
        return Ok(super::topics::not_found_response(&state));
    }
    // No filter: every type for those who see the private ones, else the
    // public types.
    if action_types.is_empty() && !sees_private {
        action_types = PUBLIC_TYPES.to_vec();
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
        base_path: state.config.globals.relative_url_root(),
        auth_token: None,
    };
    let list = users
        .actions(
            &user,
            &action_types,
            offset,
            limit,
            params.acting_username.as_deref().filter(|u| !u.is_empty()),
        )
        .await?;
    Ok(Json(json!({"user_actions": list})).into_response())
}

#[derive(Template)]
#[template(path = "user.html")]
pub struct ProfilePage {
    pub site_title: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: Crawler,
    pub viewer: Option<crate::html::Viewer>,
    pub bus_position: String,
    pub chrome: Chrome,
    pub username: String,
    /// The `.user-main` container (crate::user_profile_view).
    pub main: String,
}

#[allow(clippy::too_many_arguments)]
async fn profile_page(
    conn: &mut sqlx::PgConnection,
    site: crate::html::Site,
    i18n: &crate::i18n::I18n,
    guardian: &crate::guardian::Guardian,
    user: &User,
    show: &Value,
    tab: &Tab<'_>,
    settings: &SiteSettings,
) -> Result<ProfilePage, AppError> {
    let categories = crate::topic_list_view::categories(conn).await?;
    let cx = crate::topic_list_view::ListContext {
        i18n,
        base_path: &site.base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        member_trust_level: site.viewer.as_ref().map(|v| v.trust_level),
        settings: crate::topic_list_view::ListSettings::load(settings)?,
    };
    let draft_count: i64 = match guardian.user_id() {
        Some(id) => {
            sqlx::query_scalar::<_, i32>("SELECT draft_count FROM user_stats WHERE user_id = $1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
                .map(i64::from)
                .unwrap_or(0)
        }
        None => 0,
    };
    let viewer = crate::user_profile_view::Viewer {
        id: guardian.user_id(),
        admin: guardian.is_admin(),
        staff: guardian.is_staff(),
        can_send_private_messages: guardian.is_authenticated()
            && guardian.can_send_private_messages(settings)?,
        draft_count,
        can_direct_message: crate::plugins::chat::user_can_direct_message(
            &mut *conn, settings, guardian,
        )
        .await?,
    };
    let profile_settings = crate::user_profile_view::ProfileSettings {
        enable_badges: settings.get("enable_badges")?.truthy(),
        hide_user_activity_tab: settings.get("hide_user_activity_tab")?.truthy(),
        show_votes: crate::plugins::topic_voting::enabled(settings)?
            && settings.get("topic_voting_show_votes_on_profile")?.truthy(),
        reactions: crate::plugins::reactions::view::ReactionsUi::load(settings)?,
        solved: crate::plugins::solved::enabled(settings)?,
    };
    let main = crate::user_profile_view::render(&cx, &profile_settings, &viewer, show, tab);
    Ok(ProfilePage {
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        lang: site.lang,
        base_path: site.base_path,
        crawler: Crawler::default(),
        username: user.username.clone(),
        main,
    })
}

/// GET /u/:username/activity.json: PostsController#user_posts_feed. The
/// user's latest public posts (Post.public_posts.visible, regular ones,
/// newest first, 50 at most), less those the viewer cannot see, each
/// PostSerializer with add_excerpt. A user the viewer may not see the
/// profile of is a 404.
async fn posts_feed(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    username: &str,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let not_found = || super::topics::not_found_response(state);
    let Some(user) = User::find_active(&mut conn, username).await? else {
        return Ok(not_found());
    };
    if !user.visible_to(&settings, guardian)? {
        return Ok(not_found());
    }
    if settings.get("content_localization_enabled")?.truthy() {
        return Err(Unsupported("translated excerpts in the posts feed").into());
    }
    // ignored_user_like_counts takes likes by users the viewer ignores off
    // the counts.
    if let Some(viewer) = guardian.user_id() {
        let ignores: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM ignored_users WHERE user_id = $1)")
                .bind(viewer)
                .fetch_one(&mut *conn)
                .await?;
        if ignores {
            return Err(Unsupported("posts feed for a viewer who ignores users").into());
        }
    }
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT p.id FROM posts p JOIN topics t ON t.id = p.topic_id \
         WHERE p.user_id = $1 AND p.post_type = 1 AND p.deleted_at IS NULL AND NOT p.hidden \
           AND t.visible AND t.deleted_at IS NULL AND t.archetype <> 'private_message' \
         ORDER BY p.created_at DESC LIMIT 50",
    )
    .bind(user.id)
    .fetch_all(&mut *conn)
    .await?;
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let max_length = settings.get("post_excerpt_maxlength")?.to_i().max(0) as usize;
    let mut posts = Vec::with_capacity(ids.len());
    for id in ids {
        // posts.reject { !guardian.can_see?(post) }
        let visible = crate::posting::revisions::find_post(&mut conn, &ctx, guardian, id)
            .await?
            .is_some_and(|a| a.can_see_post);
        if !visible {
            continue;
        }
        let mut view = crate::topic_view::TopicView {
            conn: &mut conn,
            settings: &settings,
            i18n: &state.i18n,
            guardian,
            urls: &urls,
            options: crate::topic_view::Options {
                page: 0,
                post_number: None,
            },
            post_types: Vec::new(),
        };
        let Value::Object(mut post) = view.serialize_single_post(id, false, false, false).await?
        else {
            continue;
        };
        // add_excerpt: excerpt and truncated, which PostSerializer lists
        // before post_url.
        let post_url = post.shift_remove("post_url");
        let cooked = post["cooked"].as_str().unwrap_or_default().to_string();
        post.insert(
            "excerpt".into(),
            json!(crate::excerpt::excerpt(
                &cooked,
                max_length,
                &crate::excerpt::Options {
                    post_url: post_url
                        .as_ref()
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    ..Default::default()
                }
            )),
        );
        post.insert("truncated".into(), json!(true));
        if let Some(url) = post_url {
            post.insert("post_url".into(), url);
        }
        posts.push(Value::Object(post));
    }
    Ok(Json(Value::Array(posts)).into_response())
}
