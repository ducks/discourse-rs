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
use crate::messages_view::Route as MessagesRoute;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::user_profile_view::{ActivityFilter, Tab};
use crate::users::{PRIVATE_TYPES, PUBLIC_TYPES, User, Users};
use crate::{AppError, AppState, Unsupported};

#[derive(Deserialize, Default)]
pub struct ShowParams {
    pub(crate) skip_track_visit: Option<String>,
    pub(crate) include_post_count_for: Option<String>,
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
    /// `/u/:username/card.json`, users#show_card.
    Card,
    /// The HTML of a user stream route (`/activity`, `/activity/replies`
    /// ...): users#show, drawn as that tab.
    Activity(ActivityFilter),
    /// The HTML of a messages route (`/messages`, `/messages/sent`,
    /// `/messages/group/:name/new` ...): users#show, drawn with the list
    /// the route loads; the group name for a group's.
    Messages(MessagesRoute, Option<String>),
    /// The HTML of `/u/:username/badges`: users#badges (users#show),
    /// drawn as the badges tab.
    Badges,
}

/// `/u/:username/messages(/...)` as a page: the route and its group.
fn messages_route(rest: &str) -> Result<Option<(MessagesRoute, Option<String>)>, Unsupported> {
    let Some(tail) = rest.strip_prefix("messages") else {
        return Ok(None);
    };
    let tail = tail.trim_end_matches('/');
    let route = match tail {
        "" => MessagesRoute::Inbox,
        "/sent" => MessagesRoute::Sent,
        "/new" => MessagesRoute::New,
        "/unread" => MessagesRoute::Unread,
        "/archive" => MessagesRoute::Archive,
        "/warnings" => MessagesRoute::Warnings,
        t if t == "/tags" || t.starts_with("/tags/") => {
            return Err(Unsupported("the messages' tags pages"));
        }
        t => {
            let Some(group) = t.strip_prefix("/group/") else {
                return Ok(None);
            };
            let (name, route) = match group.split_once('/') {
                None => (group, MessagesRoute::GroupInbox),
                Some((name, "new")) => (name, MessagesRoute::GroupNew),
                Some((name, "unread")) => (name, MessagesRoute::GroupUnread),
                Some((name, "archive")) => (name, MessagesRoute::GroupArchive),
                Some(_) => return Ok(None),
            };
            if name.is_empty() {
                return Ok(None);
            }
            return Ok(Some((route, Some(name.to_string()))));
        }
    };
    Ok(Some((route, None)))
}

/// Where `/u/:username(/...)` goes.
enum Routed {
    /// UsersController: the username, the action, and whether the request
    /// wants JSON.
    Profile(String, Action, bool),
    /// users#preferences and users_email#index: the username, the route,
    /// and whether the request wants JSON.
    Preferences(String, super::user_preferences::Route, bool),
    /// `/u/:username/emails`: users#check_emails.
    CheckEmails(String),
    /// The HTML of `/u/:username/notifications(/<tab>)`.
    Notifications(String, crate::user_notifications_view::Route),
    /// A path users#show answers that the Ember app has no route for: its
    /// not-found page.
    EmberUnknown,
    /// The HTML of the `userActivity.*` routes the stream doesn't draw.
    ActivityPage(String, super::user_activity::Page),
    /// `/activity.json`, PostsController#user_posts_feed: the username.
    PostsFeed(String),
    /// No route matches (`/u/:username/card` has only a .json route).
    NotFound,
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
    if rest.starts_with("preferences") {
        return Ok(match super::user_preferences::route(rest) {
            Some(route) => Routed::Preferences(username, route, json),
            None => Routed::NotFound,
        });
    }
    if rest == "emails" {
        return Ok(Routed::CheckEmails(username));
    }
    if !json && (rest == "notifications" || rest.starts_with("notifications/")) {
        return Ok(match super::user_notifications::route(rest) {
            Some(route) => Routed::Notifications(username, route),
            None => Routed::EmberUnknown,
        });
    }
    if !json && let Some(page) = super::user_activity::Page::from_path(rest) {
        return Ok(Routed::ActivityPage(username, page));
    }
    if !json && let Some((route, group)) = messages_route(rest)? {
        return Ok(Routed::Profile(
            username,
            Action::Messages(route, group),
            json,
        ));
    }
    let action = match rest {
        "summary" => Action::Summary,
        "badges" if !json => Action::Badges,
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
        "card" if json => Action::Card,
        "card" => return Ok(Routed::NotFound),
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
        Routed::Profile(username, Action::Messages(route, group), _) => {
            return messages_page(
                state,
                incoming,
                MessagesTarget {
                    username,
                    route,
                    group,
                },
                headers,
                peer,
                uri,
            )
            .await;
        }
        Routed::Profile(username, action, json) => (username, action, json),
        Routed::PostsFeed(username) => {
            return posts_feed(&state, &incoming.guardian, &username).await;
        }
        Routed::Preferences(_, route, true) => {
            return Ok(super::user_preferences::json(
                &state,
                &incoming.guardian,
                &route,
            ));
        }
        Routed::Preferences(username, route, false) => {
            return super::user_preferences::page(
                state, incoming, username, route, headers, peer, uri,
            )
            .await;
        }
        Routed::CheckEmails(username) => {
            let context = crate::params::parse(uri.query(), &headers, &axum::body::Bytes::new())
                .get("context")
                .and_then(crate::params::scalar);
            return super::user_preferences::check_emails(
                &state,
                &incoming.guardian,
                &username,
                context,
            )
            .await;
        }
        Routed::ActivityPage(username, page) => {
            let params = super::user_activity::list_params(&uri);
            return super::user_activity::page(
                state, incoming, username, page, params, headers, peer, uri,
            )
            .await;
        }
        Routed::Notifications(username, route) => {
            return super::user_notifications::page(
                state, incoming, username, route, headers, peer, uri,
            )
            .await;
        }
        Routed::EmberUnknown => {
            let bus_position = crate::bus::page_position(&state.bus).await?;
            let mut conn = state.pool.acquire().await?;
            let settings =
                SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals)
                    .await?;
            return super::user_preferences::ember_not_found(
                &state,
                &mut conn,
                &settings,
                &incoming.guardian,
                &headers,
                bus_position,
            )
            .await;
        }
        Routed::NotFound => return Ok(super::topics::not_found_response(&state)),
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

/// users#show_card's document for `username`: UserCardSerializer, or
/// HiddenProfileSerializer, or InactiveUserSerializer for a viewer who
/// isn't staff, with `topic_post_count`; the visit is tracked unless
/// `skip_track_visit` or one's own. Err is the refusal to send instead
/// (invalid access, not found). The card's HTML (super::user_card) draws
/// on it too.
pub(crate) async fn card_doc(
    state: &AppState,
    incoming: &crate::session::current::Incoming,
    username: &str,
    params: &ShowParams,
    headers: &HeaderMap,
    peer: Option<std::net::SocketAddr>,
) -> Result<Result<Value, Response>, AppError> {
    let guardian = &incoming.guardian;
    let auth_token: Option<String> = incoming
        .session
        .as_ref()
        .map(|s| s.token.auth_token.clone());
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let noindex = [("x-robots-tag", "noindex")];
    // ensure_public_can_see_profiles!
    if settings.get("hide_user_profiles_from_public")?.truthy() && guardian.is_anonymous() {
        return Ok(Err((noindex, invalid_access(state)).into_response()));
    }
    // fetch_user_from_params(include_inactive: true)
    let Some(user) = User::find_any(&mut conn, username).await? else {
        return Ok(Err(
            (noindex, super::topics::not_found_response(state)).into_response()
        ));
    };
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let mut users = Users {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian,
        urls: &urls,
        base_path: state.config.globals.relative_url_root(),
        auth_token: auth_token.as_deref(),
    };
    // assign_topic_post_count: `include_post_count_for.to_i`.
    let topic_id = params
        .include_post_count_for
        .as_deref()
        .map(crate::ruby::to_i)
        .unwrap_or(0) as i32;
    let count = users.topic_post_count(&user, topic_id).await?;
    let doc = if !guardian.is_staff() && !user.active {
        users.inactive_card(&user, count).await?
    } else {
        users.card(&user, count).await?
    };
    if params.skip_track_visit.is_none() && !guardian.is_me(user.id) {
        let ip = crate::remote_ip::remote_ip(headers, peer)?;
        users.track_view(&user, &ip).await?;
    }
    Ok(Ok(doc))
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
    if matches!(target.action, Action::Card) {
        let noindex = [("x-robots-tag", "noindex")];
        return Ok(
            match card_doc(&state, &incoming, &target.username, &params, &headers, peer).await? {
                Ok(doc) => (noindex, Json(doc)).into_response(),
                Err(refused) => refused,
            },
        );
    }
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
    // assign_topic_post_count: `include_post_count_for.to_i`.
    let post_count_for = params
        .include_post_count_for
        .as_deref()
        .map(crate::ruby::to_i)
        .unwrap_or(0) as i32;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let noindex = [("x-robots-tag", "noindex")];
    // ensure_public_can_see_profiles!
    if settings.get("hide_user_profiles_from_public")?.truthy() && guardian.is_anonymous() {
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
        Action::Show | Action::Activity(_) | Action::Card | Action::Badges => {
            if matches!(action, Action::Badges) && !settings.get("enable_badges")?.truthy() {
                return Ok(not_found());
            }
            let mut doc = users.show(&user).await?;
            // UserSerializer gets the post count too, HiddenProfileSerializer
            // only on the card.
            if doc["user"]["profile_hidden"] != true
                && let Some(count) = users.topic_post_count(&user, post_count_for).await?
            {
                doc["user"]["topic_post_count"] = count;
            }
            // Viewing one's own profile isn't tracked.
            if params.skip_track_visit.is_none() && !guardian.is_me(user.id) {
                let ip = crate::remote_ip::remote_ip(&headers, peer)?;
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
        // show_with_tail sends these to messages_page.
        Action::Messages(..) => return Err(Unsupported("a messages page without its route").into()),
    };
    if json {
        return Ok((noindex, Json(doc)).into_response());
    }
    // The HTML profile draws on the show document, the summary and the
    // public activity stream.
    let show_doc = match action {
        Action::Show
        | Action::Activity(_)
        | Action::Card
        | Action::Messages(..)
        | Action::Badges => doc,
        Action::Summary => users.show(&user).await?,
    };
    // user/index: one's own profile opens the activity stream, others
    // view_user_route.
    let filter = match action {
        Action::Summary | Action::Card | Action::Messages(..) | Action::Badges => None,
        Action::Activity(filter) => Some(filter),
        Action::Show if guardian.is_me(user.id) => Some(ActivityFilter::All),
        Action::Show => match settings.get("view_user_route")?.to_s().as_str() {
            "summary" => None,
            "activity" => Some(ActivityFilter::All),
            _ => return Err(Unsupported("view_user_route other than summary or activity").into()),
        },
    };
    let visible = user.visible_to(&settings, &guardian)?;
    // user/badges: the user's grants grouped per badge
    // (`/user-badges/:username.json?grouped=true`).
    let badges_tab = if matches!(action, Action::Badges) {
        if !visible {
            return Err(Unsupported("a hidden user's badges tab").into());
        }
        let doc = super::badges::username_doc(
            &state,
            &mut *users.conn,
            &settings,
            &guardian,
            user.id,
            true,
        )
        .await?;
        Some(doc)
    } else {
        None
    };
    let summary_doc = if visible && filter.is_none() && badges_tab.is_none() {
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
    let badges_content = match &badges_tab {
        Some(doc) => {
            let cx = crate::topic_list_view::ListContext {
                i18n: &state.i18n,
                base_path,
                now: crate::clock::now(),
                categories: &std::collections::HashMap::new(),
                expand_all_pinned: false,
                member_trust_level: None,
                settings: crate::topic_list_view::ListSettings::load(&settings)?,
            };
            Some(crate::badges_page::render_user_badges(
                &cx,
                doc,
                &user.username.to_lowercase(),
                settings.get("max_favorite_badges")?.to_i(),
            )?)
        }
        None => None,
    };
    let tab = match (&badges_content, filter) {
        (Some(content), _) => Tab::Badges(content.clone()),
        (None, filter) => match filter {
            None => Tab::Summary(summary_doc.as_ref()),
            Some(filter) => Tab::Activity {
                filter,
                stream: &stream,
                topics: &voted_topics,
                reactions: &reactions,
                solved: &solved,
            },
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
    // UserRoute#setupController: the user's searchContext.
    site.chrome
        .set_search_context("user", &user.username.to_lowercase());
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
        page.chrome
            .body_classes
            .push_str(match (&badges_content, filter) {
                (Some(_), _) => " user-badges-page",
                (None, None) => " user-summary-page",
                (None, Some(_)) => " user-activity-page",
            });
    }
    // crawlable_meta_data(title: username, image: the 45px avatar)
    let avatar = show_doc["user"]["avatar_template"]
        .as_str()
        .map(|t| urls.absolute(&t.replace("{size}", "45")))
        .transpose()?;
    let mut crawler = crate::html::Crawler::for_request(&urls, &uri, None)?.with_meta(
        urls.settings.emojis(),
        &user.username,
        "",
        avatar,
    );
    crawler.description = settings.get("site_description")?.to_s();
    page.crawler = crawler;
    let response = (
        noindex,
        Html(page.render().map_err(crate::html::HtmlError::from)?),
    )
        .into_response();
    Ok(crate::html::with_viewer_headers(response, &vs))
}

/// A messages page's user, route and group.
struct MessagesTarget {
    username: String,
    route: MessagesRoute,
    group: Option<String>,
}

/// The HTML of `/u/:username/messages(/...)`: the profile (users#show) with
/// the route's list (ListController#private_messages*, the JSON route's
/// own document) under the messages navigation, the counts from the
/// viewer's PrivateMessageTopicTrackingState. A list the viewer may not
/// read leaves Ember's exception page (403) or not-found page (404) in
/// the profile's place, as does a group the user is not in.
async fn messages_page(
    state: AppState,
    incoming: crate::session::current::Incoming,
    target: MessagesTarget,
    headers: HeaderMap,
    peer: Option<std::net::SocketAddr>,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    use crate::messages_view::{InboxRow, Messages};
    use crate::pm_tracking_state::{Inbox, Kind, lookup_count};

    let bus_position = crate::bus::page_position(&state.bus).await?;
    let guardian = incoming.guardian.clone();
    let auth_token: Option<String> = incoming
        .session
        .as_ref()
        .map(|s| s.token.auth_token.clone());
    let MessagesTarget {
        username,
        route,
        group,
    } = target;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let noindex = [("x-robots-tag", "noindex")];
    if settings.get("hide_user_profiles_from_public")?.truthy() {
        return Ok((noindex, invalid_access(&state)).into_response());
    }
    let Some(user) = User::find_active(&mut conn, &username).await? else {
        return Ok((noindex, super::topics::not_found_response(&state)).into_response());
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
    let show_doc = users.show(&user).await?;
    if !guardian.is_me(user.id) {
        let ip = crate::remote_ip::remote_ip(&headers, peer)?;
        users.track_view(&user, &ip).await?;
    }
    let viewing_self = guardian.is_me(user.id);
    let username_lower = user.username.to_lowercase();
    let user_path = format!(
        "{base_path}/u/{}",
        crate::topic_list_view::escape(&username_lower)
    );
    // The profile user's visible groups (users#show), and those with
    // messages (groupsWithMessages).
    let profile_groups: Vec<Value> = show_doc["user"]["groups"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    // userPrivateMessages.group's model: the group among them by name.
    let group = match &group {
        Some(name) => {
            let found = profile_groups
                .iter()
                .find(|g| g["name"].as_str().map(str::to_lowercase) == Some(name.to_lowercase()));
            match found {
                Some(g) => Some(g.clone()),
                None if viewing_self => None,
                None => {
                    return Err(Unsupported(
                        "another user's inbox of a group they are not in (Ember fails to load it)",
                    )
                    .into());
                }
            }
        }
        None => None,
    };
    // An unknown group: exception-unknown, the not-found page.
    let missing_group = route.is_group() && group.is_none();
    let group_name = group
        .as_ref()
        .and_then(|g| g["name"].as_str())
        .map(str::to_string);
    let list_doc = if missing_group {
        None
    } else {
        use crate::pm_lists::{GroupView, PmList};
        let list = match route {
            MessagesRoute::Inbox => PmList::Inbox,
            MessagesRoute::Sent => PmList::Sent,
            MessagesRoute::New => PmList::New,
            MessagesRoute::Unread => PmList::Unread,
            MessagesRoute::Archive => PmList::Archive,
            MessagesRoute::Warnings => PmList::Warnings,
            group_route => PmList::Group {
                id: group
                    .as_ref()
                    .and_then(|g| g["id"].as_i64())
                    .ok_or(Unsupported("a group without an id"))? as i32,
                name: group_name.clone().unwrap_or_default(),
                view: match group_route {
                    MessagesRoute::GroupNew => GroupView::New,
                    MessagesRoute::GroupUnread => GroupView::Unread,
                    MessagesRoute::GroupArchive => GroupView::Archive,
                    _ => GroupView::Inbox,
                },
            },
        };
        Some(
            super::messages::list_doc(
                &state,
                &guardian,
                &username_lower,
                list,
                &super::list::ListParams::default(),
            )
            .await?,
        )
    };
    // The JSON the route asked for, as the exception page names it.
    let request_url = {
        let filter = match route {
            MessagesRoute::Inbox => "private-messages".to_string(),
            MessagesRoute::Sent => "private-messages-sent".to_string(),
            MessagesRoute::New => "private-messages-new".to_string(),
            MessagesRoute::Unread => "private-messages-unread".to_string(),
            MessagesRoute::Archive => "private-messages-archive".to_string(),
            MessagesRoute::Warnings => "private-messages-warnings".to_string(),
            _ => "private-messages-group".to_string(),
        };
        let mut url = format!("{base_path}/topics/{filter}/{username_lower}");
        if let Some(name) = &group_name {
            url.push('/');
            url.push_str(name);
            if let Some(f) = route.filter().filter(|f| *f != "inbox") {
                url.push('/');
                url.push_str(f);
            }
        }
        url.push_str(".json");
        url
    };

    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site = crate::html::Site::from_settings(&settings, base_path)?;
    site.viewer = vs.viewer.clone();
    // My messages is current on one's own messages routes (its model is
    // the current user), not on an error page.
    let active = if viewing_self && matches!(list_doc, Some(Ok(_))) {
        crate::sidebar::Active::MyMessages
    } else {
        crate::sidebar::Active::None
    };
    site.load_chrome(&mut conn, &state, &settings, &guardian, active)
        .await?;
    site.bus_position = bus_position;
    // build-private-messages-route: the user's searchContext as an inbox.
    site.chrome
        .set_search_context("private_messages", &username_lower);
    let categories = crate::topic_list_view::categories(&mut conn).await?;
    let cx = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        member_trust_level: site.viewer.as_ref().map(|v| v.trust_level),
        settings: crate::topic_list_view::ListSettings::load(&settings)?,
    };
    let render = |page: ProfilePage| -> Result<Response, AppError> {
        let response = (
            noindex,
            Html(page.render().map_err(crate::html::HtmlError::from)?),
        )
            .into_response();
        Ok(crate::html::with_viewer_headers(response, &vs))
    };
    // The page without the profile: Ember's exception or not-found page.
    let bare = |site: crate::html::Site, page_title: String, main: String| ProfilePage {
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        lang: site.lang,
        base_path: site.base_path,
        crawler: Crawler::default(),
        page_title,
        main,
    };
    let list = match list_doc {
        None => {
            let page = crate::not_found_page::NotFound {
                forbidden: false,
                custom_message: None,
                slug: "",
                guardian: &guardian,
            };
            let html =
                crate::not_found_page::build(&mut conn, &settings, &state.i18n, &urls, &page)
                    .await?;
            return render(bare(
                site,
                String::new(),
                format!("<div class=\"container\">{html}</div>"),
            ));
        }
        Some(Ok(doc)) => doc,
        Some(Err(response)) => {
            let status = response.status().as_u16();
            if status == 404 {
                let page = crate::not_found_page::NotFound {
                    forbidden: false,
                    custom_message: None,
                    slug: "",
                    guardian: &guardian,
                };
                let html =
                    crate::not_found_page::build(&mut conn, &settings, &state.i18n, &urls, &page)
                        .await?;
                return render(bare(
                    site,
                    String::new(),
                    format!("<div class=\"container\">{html}</div>"),
                ));
            }
            let main = crate::messages_view::exception(&cx, status, &request_url);
            return render(bare(site, format!("Profile - {}", user.username), main));
        }
    };
    // tagsForUser: a tag on a message links to the user's tagged messages.
    let tagged = list["topic_list"]["topics"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|t| t["tags"].as_array().is_some_and(|tags| !tags.is_empty()));
    if tagged {
        return Err(
            Unsupported("tags on a messages list (links to the messages' tags pages)").into(),
        );
    }

    // The viewer's pmTopicTrackingState, their visible groups, and whether
    // counts or dots show.
    let states = crate::pm_tracking_state::report(&mut conn, &settings, &guardian).await?;
    let viewer_groups: Vec<(i32, String)> =
        crate::pm_tracking_state::viewer_groups(&mut conn, &guardian)
            .await?
            .into_iter()
            .map(|(id, name, _)| (id, name))
            .collect();
    let show_count = match guardian.user_id() {
        Some(id) => sqlx::query_scalar::<_, bool>(
            "SELECT COALESCE(sidebar_show_count_of_new_items, false) FROM user_options WHERE user_id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .unwrap_or(false),
        None => false,
    };
    let count = |kind: Kind, inbox: Inbox| lookup_count(&states, &viewer_groups, kind, inbox);
    let inbox_total =
        |inbox: fn() -> Inbox<'static>| count(Kind::New, inbox()) + count(Kind::Unread, inbox());
    let row_name = |name: &str, total: i64| {
        let name = crate::topic_list_view::escape(name);
        if total > 0 && show_count {
            format!("{name}&nbsp;({total})")
        } else {
            name
        }
    };
    let mut dropdown = Vec::new();
    let user_total = inbox_total(|| Inbox::User);
    dropdown.push(InboxRow {
        href: format!("{user_path}/messages"),
        name_html: row_name(
            &crate::topic_list_view::t(&cx, "user.messages.inbox"),
            user_total,
        ),
        icon: None,
        show_unread_icon: user_total > 0 && !show_count,
    });
    for g in &profile_groups {
        if g["has_messages"] != true {
            continue;
        }
        let name = g["name"].as_str().unwrap_or_default();
        let total = count(Kind::New, Inbox::Group(name)) + count(Kind::Unread, Inbox::Group(name));
        dropdown.push(InboxRow {
            href: format!(
                "{user_path}/messages/group/{}",
                crate::topic_list_view::escape(name)
            ),
            name_html: row_name(name, total),
            icon: Some("inbox"),
            show_unread_icon: total > 0 && !show_count,
        });
    }
    if guardian.can_tag_pms(&settings)? {
        dropdown.push(InboxRow {
            href: format!("{user_path}/messages/tags"),
            name_html: crate::topic_list_view::escape(&crate::topic_list_view::t(
                &cx,
                "user.messages.tags",
            )),
            icon: Some("tags"),
            show_unread_icon: false,
        });
    }
    let inbox = match &group_name {
        Some(name) => Inbox::Group(name),
        None => Inbox::User,
    };
    let (new_count, unread_count) = match inbox {
        Inbox::User => (
            count(Kind::New, Inbox::User),
            count(Kind::Unread, Inbox::User),
        ),
        Inbox::Group(name) => (
            count(Kind::New, Inbox::Group(name)),
            count(Kind::Unread, Inbox::Group(name)),
        ),
    };
    // group.group_user: the profile user's membership (users#show
    // group_users).
    let group_level = group.as_ref().and_then(|g| {
        show_doc["user"]["group_users"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|gu| gu["group_id"] == g["id"])
            .and_then(|gu| gu["notification_level"].as_i64())
    });
    let can_send_private_messages =
        guardian.is_authenticated() && guardian.can_send_private_messages(&settings)?;
    let unified_new = guardian
        .upcoming_change_enabled(&mut conn, &settings, "enable_unified_new")
        .await?;
    let messages = Messages {
        user_path: &user_path,
        route,
        group: group_name.as_deref().map(|name| (name, group_level)),
        viewing_self,
        viewer_admin: guardian.is_admin(),
        show_new_pm: viewing_self && can_send_private_messages,
        can_send_private_messages,
        can_manage_topic: site
            .viewer
            .as_ref()
            .is_some_and(crate::html::Viewer::can_manage_topic),
        unified_new,
        new_count,
        unread_count,
        dropdown,
        current_url: uri.path(),
        list: &list,
    };
    let content = crate::messages_view::render(&cx, &messages);
    let tab = Tab::Messages(content);
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
    page.chrome.body_classes.push_str(" user-messages-page");
    // titleToken: the list's, then the messages', then the user's.
    let t = |key: &str| crate::topic_list_view::t(&cx, key);
    let list_title = match (&group_name, route.filter()) {
        (Some(name), filter) => {
            let mut chars = name.chars();
            let mut title: String = chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default();
            if let Some(f) = filter.filter(|f| *f != "inbox") {
                title.push(' ');
                title.push_str(&t(&format!("user.messages.{f}")));
            }
            title
        }
        (None, Some(f)) => t(&format!("user.messages.{f}")),
        // I18n's missing translation for `user.messages.null`.
        (None, None) => "[en.user.messages.null]".to_string(),
    };
    page.page_title = format!(
        "{list_title} - {} - {}",
        t("user.private_messages"),
        user.username
    );
    let avatar = show_doc["user"]["avatar_template"]
        .as_str()
        .map(|t| urls.absolute(&t.replace("{size}", "45")))
        .transpose()?;
    let mut crawler = crate::html::Crawler::for_request(&urls, &uri, None)?.with_meta(
        urls.settings.emojis(),
        &user.username,
        "",
        avatar,
    );
    crawler.description = settings.get("site_description")?.to_s();
    page.crawler = crawler;
    render(page)
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
    /// The title before the site's ("Profile - <username>"), empty for none.
    pub page_title: String,
    /// The `.user-main` container (crate::user_profile_view).
    pub main: String,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn profile_page(
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
        can_invite_to_forum: guardian.can_invite_to_forum(settings)?,
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
        page_title: format!("Profile - {}", user.username),
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
