//! The HTML of `/u/:username/notifications(/<tab>)`: users#show answers
//! the Ember app, which draws the profile with the notifications
//! navigation over the route's list (UserNotifications' notifications for
//! the user or an admin, the stream routes' user actions, discourse-reactions'
//! received reactions).

use askama::Template;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};
use serde_json::Value;

use crate::html::Crawler;
use crate::session::current::Incoming;
use crate::site_settings::SiteSettings;
use crate::topic_list_view::escape;
use crate::url::Urls;
use crate::user_notifications_view::{self as view, Route};
use crate::user_profile_view::Tab;
use crate::users::{User, Users};
use crate::{AppError, AppState, Unsupported};

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

/// The tab path after `notifications`, or None for a path no route has.
pub fn route(rest: &str) -> Option<Route> {
    let tail = rest.strip_prefix("notifications")?;
    let tail = tail.trim_end_matches('/');
    match tail {
        "" => Some(Route::Index),
        t => Route::from_path(t.strip_prefix('/')?),
    }
}

pub async fn page(
    state: AppState,
    incoming: Incoming,
    username: String,
    route: Route,
    headers: HeaderMap,
    peer: Option<std::net::SocketAddr>,
    uri: Uri,
) -> Result<Response, AppError> {
    let guardian = incoming.guardian.clone();
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
    let base_path = state.config.globals.relative_url_root().to_string();
    // UserNotifications#model loads nothing for others; the index then
    // goes to the responses, whose private stream Ember cannot load: its
    // /404.
    if !guardian.is_me(user.id) && !guardian.is_admin() {
        return Ok(found(&headers, format!("{base_path}/404")));
    }
    if settings.get("show_user_menu_avatars")?.truthy() {
        return Err(Unsupported("show_user_menu_avatars (the items' avatars, 194)").into());
    }
    let reactions_ui = crate::plugins::reactions::view::ReactionsUi::load(&settings)?;
    if route == Route::ReactionsReceived && reactions_ui.is_none() {
        return Ok((noindex, super::topics::not_found_response(&state)).into_response());
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
    if !guardian.is_me(user.id) {
        let ip = crate::remote_ip::remote_ip(&headers, peer)?;
        users.track_view(&user, &ip).await?;
    }
    // The stream routes' user actions.
    let stream = match route.action_type() {
        Some(kind) => users.actions(&user, &[kind], 0, 30, None).await?,
        None => Vec::new(),
    };

    // The parent route's model: the user's notifications, filtered on the
    // index by ?filter.
    let filter = crate::params::parse(uri.query(), &headers, &axum::body::Bytes::new())
        .get("filter")
        .and_then(crate::params::scalar)
        .filter(|f| f == "read" || f == "unread");
    let query = crate::notifications::Query {
        recent: false,
        silent: false,
        limit: 60,
        offset: 0,
        filter: if route == Route::Index {
            filter.clone()
        } else {
            None
        },
        types: Vec::new(),
    };
    let (doc, bumped) = crate::notifications::Notifications {
        conn: &mut conn,
        settings: &settings,
        guardian: &guardian,
        urls: &urls,
    }
    .index(user.id, &user.username, &query)
    .await?;
    if bumped {
        crate::bus::publish_notifications_state(&state.bus, &mut conn, &settings, user.id).await?;
    }
    let notifications: Vec<Value> = doc["notifications"].as_array().cloned().unwrap_or_default();

    let mut reactions = Vec::new();
    if route == Route::ReactionsReceived {
        let mut reader = crate::plugins::reactions::users::Reader {
            conn: &mut conn,
            settings: &settings,
            urls: &urls,
            guardian: &guardian,
        };
        reactions = reader
            .reactions_received(
                user.id,
                &crate::plugins::reactions::users::ReceivedParams {
                    before_reaction_user_id: None,
                    before_like_id: None,
                    acting_username: None,
                    include_likes: false,
                },
            )
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default();
    }

    let session_user = incoming
        .session
        .as_ref()
        .map(|s| &s.user)
        .ok_or(Unsupported("a logged-in viewer without a session"))?;
    let current_user = crate::current_user::serialize(
        &mut conn,
        &state,
        &settings,
        session_user,
        incoming
            .session
            .as_ref()
            .and_then(|s| s.token.authenticated_with_oauth)
            .unwrap_or(false),
    )
    .await?;
    let can_chat = crate::plugins::chat::enabled(&settings)?
        && crate::plugins::chat::can_chat(&mut conn, &settings, &guardian).await?;
    let categories = crate::topic_list_view::categories(&mut conn).await?;
    let cx = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: &base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        member_trust_level: None,
        settings: crate::topic_list_view::ListSettings::load(&settings)?,
    };
    let mcx = crate::user_menu_view::MenuContext {
        i18n: &state.i18n,
        base_path: &base_path,
        mobile: crate::mobile::mobile_view() || crate::mobile::narrow_desktop_view(),
        user: &current_user,
        notification_types: crate::site::NOTIFICATION_TYPES,
        prioritize_full_name: settings.get("prioritize_full_name_in_ux")?.truthy(),
        vote_limits: settings.get("topic_voting_enable_vote_limits")?.truthy(),
        enable_user_status: settings.get("enable_user_status")?.truthy(),
        can_chat,
        emojis: settings.emojis(),
        now: crate::clock::now(),
        relative_date_duration: settings.get("relative_date_duration")?.to_i(),
    };
    let user_path = format!("{base_path}/u/{}", escape(&user.username.to_lowercase()));
    let dismiss =
        (!notifications.is_empty()).then(|| notifications.iter().all(|n| n["read"] == true));
    let mut main = view::nav(
        &cx,
        &user_path,
        route,
        settings.get("enable_mentions")?.truthy(),
        reactions_ui.is_some(),
        dismiss,
    );
    main.push_str(&match route {
        Route::Index => view::index(
            &cx,
            &mcx,
            &notifications,
            filter.as_deref().unwrap_or("all"),
        ),
        Route::ReactionsReceived => match &reactions_ui {
            Some(ui) => view::reactions(&cx, ui, &reactions),
            None => String::new(),
        },
        stream_route => view::stream(&cx, stream_route.action_type().unwrap_or(0), &stream),
    });
    main.push_str(&format!(
        "<script type=\"application/json\" id=\"notifications-page-strings\">{}</script>",
        serde_json::json!({
            "high_priority": current_user["unread_high_priority_notifications"],
            "confirm": crate::topic_list_view::t_count(
                &cx,
                "notifications.dismiss_confirmation.body.default",
                current_user["unread_high_priority_notifications"].as_i64().unwrap_or(0),
                &[]
            ),
            "dismiss": crate::topic_list_view::t(&cx, "notifications.dismiss_confirmation.dismiss"),
            "cancel": crate::topic_list_view::t(&cx, "notifications.dismiss_confirmation.cancel"),
        })
        .to_string()
        .replace('<', "\\u003c")
    ));

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
    site.chrome
        .set_search_context("user", &user.username.to_lowercase());
    let mut page = super::users::profile_page(
        &mut conn,
        site,
        &state.i18n,
        &guardian,
        &user,
        &show,
        &Tab::Notifications(main),
        &settings,
    )
    .await?;
    page.chrome
        .body_classes
        .push_str(" user-notifications-page");
    // The route's titleToken, UserNotifications', the user's.
    page.page_title = format!(
        "{} - {} - {}",
        crate::topic_list_view::t(&cx, route.title_key()),
        crate::topic_list_view::t(&cx, "user.notifications"),
        user.username
    );
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
    page.crawler = crawler;
    let response = (
        noindex,
        Html(page.render().map_err(crate::html::HtmlError::from)?),
    )
        .into_response();
    Ok(crate::html::with_viewer_headers(response, &vs))
}
