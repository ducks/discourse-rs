mod accounts;
mod admin_email;
mod admin_logs;
mod admin_site_settings;
mod admin_users;
mod bookmark_writes;
mod bookmarks;
mod bus;
pub(crate) mod composer;
mod drafts;
mod list;
mod live;
mod login_required;
mod messages;
mod not_found;
mod notifications;
mod post_actions;
mod post_destroy;
mod posts;
mod read_tracking;
mod review;
mod robots;
mod search;
mod uploads;
pub(crate) use search::invalid_access_at;
pub(crate) use sitemap::regenerate_sitemaps;
pub(crate) use topics::not_found_response;
mod category_notifications;
mod chat;
mod chat_pages;
mod reactions;
mod session;
mod site;
mod sitemap;
mod solved;
mod srv;
mod stylesheets;
mod tags;
mod topic_notifications;
mod topic_status;
mod topic_voting;
mod topics;
mod user_avatars;
mod user_menu;
mod user_update;
mod users;

use axum::Router;
use axum::routing::{delete, get, post, put};
use tower_http::services::ServeDir;

use crate::AppState;

/// The Font Awesome icons the pages use, as a sprite of <symbol>s
/// (scripts/build-icon-sprite), which the layout includes inline.
pub const ICONS_SVG: &str = include_str!("../../static/vendor/icons.svg");

/// GET /assets/<name>: the embedded stylesheets and scripts (crate::assets).
async fn asset(uri: axum::http::Uri, headers: axum::http::HeaderMap) -> axum::response::Response {
    let name = uri
        .path()
        .trim_start_matches('/')
        .trim_start_matches("assets/");
    crate::assets::serve(name, uri.query(), &headers)
}

/// A route Rails caches for anonymous visitors (`discourse_expires_in
/// 1.minute`): the response carries the duration for crate::anonymous_cache.
macro_rules! cached {
    ($route:expr) => {
        $route.layer(axum::middleware::from_fn(
            crate::anonymous_cache::expires_in_one_minute,
        ))
    };
}

pub fn router(state: &AppState) -> Router<AppState> {
    let config = &state.config;
    let public = &config.public_dir;
    // Site images first (a backup's or an install's public/), then the
    // stock ones from a Discourse checkout.
    let images = match &config.discourse_src {
        Some(src) => {
            ServeDir::new(public.join("images")).fallback(ServeDir::new(src.join("public/images")))
        }
        None => ServeDir::new(public.join("images")).fallback(ServeDir::new(public.join("images"))),
    };
    let mut router = Router::new()
        .route("/srv/status", get(srv::status))
        .route("/bus/events", get(bus::events))
        .route("/bus/poll", get(bus::poll))
        .route("/live", get(live::page))
        .route("/user-menu", get(user_menu::show))
        .route("/live/post/{id}", get(live::post))
        .route(
            "/live/chat/{channel_id}/{message_id}",
            get(chat_pages::live_message),
        )
        .route("/", cached!(get(list::latest)))
        .route("/latest", cached!(get(list::latest)))
        .route("/latest.json", cached!(get(list::latest_json)))
        .route("/top", cached!(get(list::top)))
        .route("/top.json", cached!(get(list::top_json)))
        .route("/top/{period}", get(list::top_period_redirect))
        .route("/hot", cached!(get(list::hot)))
        .route("/hot.json", cached!(get(list::hot_json)))
        // discourse-topic-voting's filter.
        .route("/votes", cached!(get(list::user_list)))
        .route("/votes.json", cached!(get(list::user_list)))
        .route("/unread", cached!(get(list::user_list)))
        .route("/unread.json", cached!(get(list::user_list)))
        .route("/new", cached!(get(list::user_list)))
        .route("/new.json", cached!(get(list::user_list)))
        .route("/unseen", cached!(get(list::user_list)))
        .route("/unseen.json", cached!(get(list::user_list)))
        .route("/read", cached!(get(list::user_list)))
        .route("/read.json", cached!(get(list::user_list)))
        .route("/posted", cached!(get(list::user_list)))
        .route("/posted.json", cached!(get(list::user_list)))
        .route(
            "/bookmarks",
            cached!(get(list::user_list)).post(bookmark_writes::create),
        )
        .route(
            "/bookmarks.json",
            cached!(get(list::user_list)).post(bookmark_writes::create),
        )
        .route(
            "/bookmarks/{id}",
            put(bookmark_writes::update).delete(bookmark_writes::destroy),
        )
        .route(
            "/bookmarks/{id}/toggle_pin",
            put(bookmark_writes::toggle_pin),
        )
        .route(
            "/bookmarks/{id}/toggle_pin.json",
            put(bookmark_writes::toggle_pin),
        )
        .route("/notifications", get(notifications::index))
        .route("/notifications.json", get(notifications::index))
        .route("/drafts", get(drafts::index).post(drafts::create))
        .route("/drafts.json", get(drafts::index).post(drafts::create))
        .route("/drafts/{id}", get(drafts::show).delete(drafts::destroy))
        .route("/notifications/mark-read", put(read_tracking::mark_read))
        .route(
            "/notifications/mark-read.json",
            put(read_tracking::mark_read),
        )
        .route("/notifications/read", put(read_tracking::mark_read))
        .route("/notifications/read.json", put(read_tracking::mark_read))
        .route("/topics/timings", post(read_tracking::timings))
        .route("/topics/timings.json", post(read_tracking::timings))
        .route(
            "/t/{slug}/timings",
            post(read_tracking::topic_timings).delete(read_tracking::destroy_timings),
        )
        .route(
            "/t/{slug}/timings.json",
            delete(read_tracking::destroy_timings),
        )
        // chat's Chat::Api controllers.
        // Chat::Api::ChannelMessagesController#create.
        .route("/chat/{id}", post(chat::create_message))
        // Full-page chat (Chat::ChatController#respond).
        .route("/chat", get(chat_pages::index))
        .route("/chat/channels", get(chat_pages::index))
        .route("/chat/disabled", get(chat_pages::disabled))
        .route("/chat/browse", get(chat_pages::browse_index))
        .route("/chat/browse/{tab}", get(chat_pages::browse))
        .route("/chat/browse/{tab}/page", get(chat_pages::browse_more))
        .route("/chat/c/{slug}/{id}", get(chat_pages::channel))
        .route("/chat/c/{slug}/{id}/{message_id}", get(chat_pages::channel))
        .route("/chat/api/me/channels", get(chat::me_channels))
        .route("/chat/api/me/channels.json", get(chat::me_channels))
        .route("/chat/api/channels", get(chat::index))
        .route("/chat/api/channels.json", get(chat::index))
        .route("/chat/api/channels/{id}", get(chat::show))
        .route(
            "/chat/api/channels/{id}/memberships",
            get(chat::memberships),
        )
        .route(
            "/chat/api/channels/{id}/memberships.json",
            get(chat::memberships),
        )
        .route("/chat/api/channels/read", put(chat::mark_all_read))
        .route("/chat/api/channels/read.json", put(chat::mark_all_read))
        .route("/chat/api/channels/{id}/drafts", post(chat::draft))
        .route("/chat/api/channels/{id}/drafts.json", post(chat::draft))
        .route("/chat/api/channels/{id}/read", put(chat::mark_read))
        .route("/chat/api/channels/{id}/read.json", put(chat::mark_read))
        .route(
            "/chat/api/channels/{id}/memberships/me",
            post(chat::own_membership)
                .put(chat::own_membership)
                .delete(chat::own_membership),
        )
        .route(
            "/chat/api/channels/{id}/memberships/me.json",
            post(chat::own_membership)
                .put(chat::own_membership)
                .delete(chat::own_membership),
        )
        .route(
            "/chat/api/channels/{id}/memberships/me/follows",
            delete(chat::unfollow),
        )
        .route(
            "/chat/api/channels/{id}/memberships/me/follows.json",
            delete(chat::unfollow),
        )
        .route("/chat/api/channels/{id}/messages", get(chat::messages))
        .route("/chat/api/channels/{id}/messages.json", get(chat::messages))
        .route(
            "/chat/api/channels/{id}/messages/{message_id}",
            put(chat::update_message).delete(chat::trash_message),
        )
        .route(
            "/chat/api/channels/{id}/messages/{message_id}/restore",
            put(chat::restore_message),
        )
        .route(
            "/chat/api/channels/{id}/messages/{message_id}/restore.json",
            put(chat::restore_message),
        )
        .route("/chat/{id}/react/{message_id}", put(chat::react))
        .route("/chat/{id}/{message_id}/rebake", put(chat::rebake_message))
        .route(
            "/chat/{id}/{message_id}/rebake.json",
            put(chat::rebake_message),
        )
        // discourse-solved's AnswerController.
        .route("/solution/accept", post(solved::accept))
        .route("/solution/accept.json", post(solved::accept))
        .route("/solution/unaccept", post(solved::unaccept))
        .route("/solution/unaccept.json", post(solved::unaccept))
        .route("/solution/by_user", get(solved::by_user))
        .route("/solution/shared_issue", post(solved::shared_issue))
        .route("/solution/shared_issue.json", post(solved::shared_issue))
        .route("/solution/by_user.json", get(solved::by_user))
        .route(
            "/category/{category_id}/notifications",
            post(category_notifications::set_notifications),
        )
        .route(
            "/category/{category_id}/notifications.json",
            post(category_notifications::set_notifications),
        )
        // discourse-reactions' CustomReactionsController.
        .route(
            "/discourse-reactions/posts/{post_id}/custom-reactions/{reaction}/toggle",
            put(reactions::toggle),
        )
        .route(
            "/discourse-reactions/posts/{post_id}/custom-reactions/{reaction}/toggle.json",
            put(reactions::toggle),
        )
        .route(
            "/discourse-reactions/posts/reactions",
            get(reactions::reactions_given),
        )
        .route(
            "/discourse-reactions/posts/reactions.json",
            get(reactions::reactions_given),
        )
        .route(
            "/discourse-reactions/posts/reactions-received",
            get(reactions::reactions_received),
        )
        .route(
            "/discourse-reactions/posts/reactions-received.json",
            get(reactions::reactions_received),
        )
        .route(
            "/discourse-reactions/posts/{id}/reactions-users",
            get(reactions::post_reactions_users),
        )
        .route(
            "/discourse-reactions/posts/{id}/reactions-users.json",
            get(reactions::post_reactions_users),
        )
        .route(
            "/discourse-reactions/posts/{id}/reactions-users-list",
            get(reactions::reactions_users_list),
        )
        .route(
            "/discourse-reactions/posts/{id}/reactions-users-list.json",
            get(reactions::reactions_users_list),
        )
        // discourse-topic-voting's VotesController.
        .route("/voting/vote", post(topic_voting::vote))
        .route("/voting/vote.json", post(topic_voting::vote))
        .route("/voting/unvote", post(topic_voting::unvote))
        .route("/voting/unvote.json", post(topic_voting::unvote))
        .route("/voting/who", get(topic_voting::who))
        .route("/voting/who.json", get(topic_voting::who))
        .route(
            "/topics/voted-by/{username}",
            cached!(get(topic_voting::voted_by)),
        )
        // TopicsController#set_notifications; {slug} holds the topic id.
        .route(
            "/t/{slug}/notifications",
            post(topic_notifications::set_notifications),
        )
        .route("/topics/{kind}/{username}", get(messages::personal))
        .route(
            "/topics/private-messages-group/{username}/{*rest}",
            get(messages::group),
        )
        .route(
            "/t/{id}",
            cached!(get(topics::show_by_id)).delete(post_destroy::destroy_topic),
        )
        .route("/t/{slug}/{id}", cached!(get(topics::show_with_slug)))
        .route(
            "/t/{slug}/{id}/{post_number}",
            cached!(get(topics::show_post)),
        )
        // TopicsController#status; {slug} holds the topic id here.
        .route("/t/{slug}/status", put(topic_status::status))
        .route("/t/{slug}/{id}/status", put(topic_status::status_with_slug))
        .route("/u/{username}", get(users::show).put(user_update::update))
        .route("/u/{username}/bookmarks", get(bookmarks::index))
        .route("/u/{username}/bookmarks.json", get(bookmarks::index))
        .route(
            "/u/{username}/user-menu-bookmarks",
            get(bookmarks::user_menu),
        )
        .route(
            "/u/{username}/user-menu-bookmarks.json",
            get(bookmarks::user_menu),
        )
        .route("/u/{username}/{*rest}", get(users::show_with_tail))
        .route("/users/{username}", get(users::show))
        .route("/users/{username}/bookmarks", get(bookmarks::index))
        .route("/users/{username}/bookmarks.json", get(bookmarks::index))
        .route(
            "/users/{username}/user-menu-bookmarks",
            get(bookmarks::user_menu),
        )
        .route(
            "/users/{username}/user-menu-bookmarks.json",
            get(bookmarks::user_menu),
        )
        .route("/users/{username}/{*rest}", get(users::show_with_tail))
        .route("/user_actions.json", get(users::actions))
        .route("/c/{*path}", cached!(get(list::category)))
        .route("/robots.txt", get(robots::index))
        .route("/robots-builder.json", get(robots::builder))
        .route("/sitemap.xml", get(sitemap::index))
        .route("/sitemap_{page}", get(sitemap::page))
        .route("/news.xml", get(sitemap::news))
        .route("/admin/site_settings", get(admin_site_settings::index))
        .route("/admin/site_settings.json", get(admin_site_settings::index))
        .route(
            "/admin/site_settings/{id}",
            put(admin_site_settings::update),
        )
        .route(
            "/admin/logs/screened_emails",
            get(admin_logs::screened_emails),
        )
        .route(
            "/admin/logs/screened_emails.json",
            get(admin_logs::screened_emails),
        )
        .route(
            "/admin/logs/screened_emails/{id}",
            delete(admin_logs::destroy_screened_email),
        )
        .route("/admin/email-logs/{kind}", get(admin_logs::email_logs))
        .route("/admin/logs/search_logs", get(admin_logs::search_logs))
        .route("/admin/logs/search_logs.json", get(admin_logs::search_logs))
        .route("/admin/logs/screened_urls", get(admin_logs::screened_urls))
        .route(
            "/admin/logs/screened_urls.json",
            get(admin_logs::screened_urls),
        )
        .route(
            "/admin/logs/screened_ip_addresses",
            get(admin_logs::screened_ip_addresses).post(admin_logs::create_screened_ip),
        )
        .route(
            "/admin/logs/screened_ip_addresses.json",
            get(admin_logs::screened_ip_addresses).post(admin_logs::create_screened_ip),
        )
        .route(
            "/admin/logs/screened_ip_addresses/{id}",
            put(admin_logs::update_screened_ip).delete(admin_logs::destroy_screened_ip),
        )
        .route("/admin/logs", get(admin_logs::staff_action_logs))
        .route("/admin/logs.json", get(admin_logs::staff_action_logs))
        .route(
            "/admin/logs/staff_action_logs",
            get(admin_logs::staff_action_logs),
        )
        .route(
            "/admin/logs/staff_action_logs.json",
            get(admin_logs::staff_action_logs),
        )
        .route("/admin/users/list", get(admin_users::index))
        .route("/admin/users/list.json", get(admin_users::index))
        .route("/admin/users/list/{query}", get(admin_users::index))
        .route("/admin/users/{user_id}", get(admin_users::show))
        .route(
            "/admin/users/{user_id}/grant_moderation",
            put(admin_users::grant_moderation),
        )
        .route(
            "/admin/users/{user_id}/revoke_moderation",
            put(admin_users::revoke_moderation),
        )
        .route(
            "/admin/users/{user_id}/revoke_admin",
            put(admin_users::revoke_admin),
        )
        .route(
            "/admin/users/{user_id}/grant_moderation.json",
            put(admin_users::grant_moderation),
        )
        .route(
            "/admin/users/{user_id}/revoke_moderation.json",
            put(admin_users::revoke_moderation),
        )
        .route(
            "/admin/users/{user_id}/revoke_admin.json",
            put(admin_users::revoke_admin),
        )
        .route(
            "/admin/users/{user_id}/trust_level",
            put(admin_users::trust_level),
        )
        .route(
            "/admin/users/{user_id}/trust_level.json",
            put(admin_users::trust_level),
        )
        .route(
            "/admin/users/{user_id}/trust_level_lock",
            put(admin_users::trust_level_lock),
        )
        .route(
            "/admin/users/{user_id}/trust_level_lock.json",
            put(admin_users::trust_level_lock),
        )
        .route("/admin/users/{user_id}/log_out", post(admin_users::log_out))
        .route(
            "/admin/users/{user_id}/log_out.json",
            post(admin_users::log_out),
        )
        .route(
            "/admin/users/{user_id}/activate",
            put(admin_users::activate),
        )
        .route(
            "/admin/users/{user_id}/activate.json",
            put(admin_users::activate),
        )
        .route(
            "/admin/users/{user_id}/deactivate",
            put(admin_users::deactivate),
        )
        .route(
            "/admin/users/{user_id}/deactivate.json",
            put(admin_users::deactivate),
        )
        .route(
            "/admin/users/{user_id}/groups",
            post(admin_users::add_group),
        )
        .route(
            "/admin/users/{user_id}/groups/{group_id}",
            delete(admin_users::remove_group),
        )
        .route(
            "/admin/users/{user_id}/primary_group",
            put(admin_users::primary_group),
        )
        .route(
            "/admin/users/{user_id}/groups.json",
            post(admin_users::add_group),
        )
        .route(
            "/admin/users/{user_id}/primary_group.json",
            put(admin_users::primary_group),
        )
        .route("/admin/users/{user_id}/approve", put(admin_users::approve))
        .route(
            "/admin/users/{user_id}/approve.json",
            put(admin_users::approve),
        )
        .route("/admin/users/{user_id}/suspend", put(admin_users::suspend))
        .route(
            "/admin/users/{user_id}/suspend.json",
            put(admin_users::suspend),
        )
        .route(
            "/admin/users/{user_id}/unsuspend",
            put(admin_users::unsuspend),
        )
        .route(
            "/admin/users/{user_id}/unsuspend.json",
            put(admin_users::unsuspend),
        )
        .route("/admin/users/{user_id}/silence", put(admin_users::silence))
        .route(
            "/admin/users/{user_id}/silence.json",
            put(admin_users::silence),
        )
        .route(
            "/admin/users/{user_id}/unsilence",
            put(admin_users::unsilence),
        )
        .route(
            "/admin/users/{user_id}/unsilence.json",
            put(admin_users::unsilence),
        )
        .route("/admin/email/handle_mail", post(admin_email::handle_mail))
        .route(
            "/admin/email/handle_mail.json",
            post(admin_email::handle_mail),
        )
        .route("/session/hp", get(accounts::honeypot))
        .route("/session/hp.json", get(accounts::honeypot))
        .route("/u", post(accounts::create_user))
        .route("/u.json", post(accounts::create_user))
        .route("/users", post(accounts::create_user))
        .route("/users.json", post(accounts::create_user))
        .route(
            "/u/activate-account/{token}",
            put(accounts::perform_account_activation),
        )
        .route(
            "/users/activate-account/{token}",
            put(accounts::perform_account_activation),
        )
        .route("/u/email-login", post(accounts::email_login))
        .route("/session/forgot_password", post(accounts::forgot_password))
        .route(
            "/session/forgot_password.json",
            post(accounts::forgot_password),
        )
        .route(
            "/session/password-reset-code/verify",
            post(accounts::redeem_password_reset_code),
        )
        .route(
            "/session/password-reset-code/verify.json",
            post(accounts::redeem_password_reset_code),
        )
        .route(
            "/u/password-reset/{token}",
            put(accounts::password_reset_update),
        )
        .route(
            "/users/password-reset/{token}",
            put(accounts::password_reset_update),
        )
        .route("/u/email-login.json", post(accounts::email_login))
        .route("/users/email-login", post(accounts::email_login))
        .route("/users/email-login.json", post(accounts::email_login))
        .route("/uploads.json", post(uploads::create))
        .route("/uploads/lookup-urls", post(uploads::lookup_urls))
        .route("/review", get(review::page))
        .route("/review.json", get(review::index))
        .route(
            "/review/{reviewable_id}/perform/{action_id}",
            put(review::perform),
        )
        .route("/post_actions", post(post_actions::create))
        .route("/post_actions.json", post(post_actions::create))
        .route("/post_actions/{id}", delete(post_actions::destroy))
        .route("/posts", post(posts::create))
        .route("/posts.json", post(posts::create))
        .route(
            "/posts/{id}",
            put(posts::update).delete(post_destroy::destroy_post),
        )
        .route("/raw/{topic_id}/{post_number}", get(posts::raw))
        .route("/posts/{id}/recover", put(post_destroy::recover_post))
        .route("/posts/{id}/recover.json", put(post_destroy::recover_post))
        .route("/posts/{id}/revisions/{revision}", get(posts::revision))
        .route("/session", post(session::create))
        .route("/session.json", post(session::create))
        .route("/session/csrf", get(session::csrf))
        .route("/session/csrf.json", get(session::csrf))
        .route("/session/current", get(session::current))
        .route("/session/current.json", get(session::current))
        .route("/session/{username}", delete(session::destroy))
        .route(
            "/login",
            get(login_required::show_login).post(session::enter),
        )
        .route("/search", cached!(get(search::show)))
        .route("/search.json", cached!(get(search::show_json)))
        .route("/search/query", cached!(get(search::query)))
        .route("/search/query.json", cached!(get(search::query)))
        .route("/tag/{*path}", cached!(get(tags::show)))
        .route("/tags", get(tags::index))
        .route("/tags.json", get(tags::index_json))
        .route("/tags/c/{*path}", cached!(get(tags::show_in_category)))
        .route("/categories", cached!(get(list::categories)))
        .route("/categories.json", cached!(get(list::categories_json)))
        .route("/site", get(site::site))
        .route("/site.json", get(site::site))
        .route("/site/basic-info", get(site::basic_info))
        .route("/site/basic-info.json", get(site::basic_info))
        .route("/assets/site.css", get(asset))
        .route("/assets/discourse.css", get(asset))
        .route(
            "/assets/color_definitions_light.css",
            get(stylesheets::light),
        )
        .route("/assets/color_definitions_dark.css", get(stylesheets::dark))
        .route("/fonts/{file}", get(stylesheets::font))
        .route(
            "/letter_avatar_proxy/{version}/letter/{letter}/{color}/{size}",
            get(user_avatars::show_proxy_letter),
        )
        .route("/assets/htmx.min.js", get(asset))
        .route("/assets/htmx-ext-sse.js", get(asset))
        .route("/assets/page.js", get(asset))
        .route("/assets/sidebar.js", get(asset))
        .route("/assets/topic.js", get(asset))
        .route("/assets/topic-voting.js", get(asset))
        .route("/assets/discourse-reactions.js", get(asset))
        .route("/assets/discourse-solved.js", get(asset))
        .route("/assets/chat.js", get(asset))
        .route("/assets/tracking-menu.js", get(asset))
        .route("/assets/composer.js", get(asset))
        .route("/assets/screen-track.js", get(asset))
        .route("/assets/js/onpopstate-handler.js", get(asset))
        .route("/assets/markdown.wasm", get(composer::markdown_wasm))
        .route(
            "/assets/markdown-settings.json",
            get(composer::markdown_settings),
        )
        // The local store's files (FileStore::Local): what nginx serves in a
        // Rails deployment. A remote store's urls point elsewhere.
        .nest_service("/uploads", ServeDir::new(public.join("uploads")));
    if let Some(emoji) = config.emoji_dir() {
        router = router.nest_service("/images/emoji", ServeDir::new(emoji));
    }
    router
        .nest_service("/images", images)
        // Only for routed requests, as Rails' before_action only runs once a
        // route matched: an unknown path (a browser's /favicon.ico) is a 404,
        // not a redirect to the login page that overwrites destination_url.
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            login_required::gate,
        ))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            not_found::html_errors,
        ))
}

/// `Rack::MethodOverride`: a POST with a form `_method` (or the
/// `X-HTTP-Method-Override` header) of delete/put/patch is handled as
/// that method, which is how HTML forms log out.
pub async fn method_override(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::body::Body;
    use axum::http::{Method, header};
    if request.method() != Method::POST {
        return next.run(request).await;
    }
    let (mut parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, 1 << 20).await {
        Ok(b) => b,
        Err(_) => {
            return axum::response::IntoResponse::into_response(
                axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            );
        }
    };
    let form = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/x-www-form-urlencoded"));
    let overridden = parts
        .headers
        .get("x-http-method-override")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| {
            if !form {
                return None;
            }
            form_urlencoded::parse(&bytes)
                .find(|(k, _)| k == "_method")
                .map(|(_, v)| v.into_owned())
        });
    if let Some(m) = overridden
        && let Ok(method) = Method::from_bytes(m.to_ascii_uppercase().as_bytes())
        && matches!(method, Method::DELETE | Method::PUT | Method::PATCH)
    {
        parts.method = method;
    }
    next.run(axum::extract::Request::from_parts(parts, Body::from(bytes)))
        .await
}

/// `request.format` for a path without an extension, as ActionDispatch
/// decides it: `?format=json`, or a valid Accept header (an XHR's, or one
/// that is not browser-like: no `*/*` beside other types) whose preferred
/// type is JSON. The port tells JSON by the `.json` suffix, so such a GET
/// goes to the path's `.json` twin, for the pages whose controllers then
/// answer JSON (/search, /login and the /my redirects stay HTML; so does
/// the front page, whose JSON is the homepage list's, not ported).
pub async fn request_format(
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::Method;
    if (request.method() == Method::GET || request.method() == Method::HEAD)
        && json_twin_page(request.uri().path())
        && wants_json(request.uri().query(), request.headers())
    {
        let uri = request.uri();
        let rewritten = match uri.query() {
            Some(q) => format!("{}.json?{q}", uri.path()),
            None => format!("{}.json", uri.path()),
        };
        if let Ok(uri) = rewritten.parse() {
            *request.uri_mut() = uri;
        }
    }
    next.run(request).await
}

/// The pages Rails answers as JSON when asked to: the topic lists, the
/// categories and tags, topics, user pages, drafts, notifications and the
/// review queue.
fn json_twin_page(path: &str) -> bool {
    let Some(rest) = path.strip_prefix('/') else {
        return false;
    };
    let first = rest.split('/').next().unwrap_or("");
    let last = rest.rsplit('/').next().unwrap_or("");
    !rest.is_empty()
        && !last.contains('.')
        && matches!(
            first,
            "latest"
                | "top"
                | "hot"
                | "new"
                | "unread"
                | "unseen"
                | "read"
                | "posted"
                | "bookmarks"
                | "categories"
                | "c"
                | "tags"
                | "tag"
                | "t"
                | "u"
                | "drafts"
                | "notifications"
                | "review"
        )
}

/// ActionDispatch's `formats`: the format param first, then the Accept
/// header when valid (`valid_accept_header`), its types by q, the first
/// kept on ties.
pub(super) fn wants_json(query: Option<&str>, headers: &axum::http::HeaderMap) -> bool {
    if let Some(format) = query
        .into_iter()
        .flat_map(|q| q.split('&'))
        .find_map(|p| p.strip_prefix("format="))
    {
        return format == "json";
    }
    let Some(accept) = headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .filter(|a| !a.trim().is_empty())
    else {
        return false;
    };
    let xhr = headers
        .get("x-requested-with")
        .is_some_and(|v| v == "XMLHttpRequest");
    // BROWSER_LIKE_ACCEPTS: `*/*` alongside other types.
    static BROWSER_LIKE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let browser_like = BROWSER_LIKE
        .get_or_init(|| regex::Regex::new(r",\s*\*/\*|\*/\*\s*,").unwrap())
        .is_match(accept);
    if !xhr && browser_like {
        return false;
    }
    let mut types: Vec<(f32, &str)> = accept
        .split(',')
        .map(|entry| {
            let mut parts = entry.split(';');
            let mime = parts.next().unwrap_or("").trim();
            let q = parts
                .filter_map(|p| p.trim().strip_prefix("q="))
                .find_map(|q| q.parse::<f32>().ok())
                .unwrap_or(1.0);
            (q, mime)
        })
        .collect();
    types.sort_by(|a, b| b.0.total_cmp(&a.0));
    types
        .first()
        .is_some_and(|(_, mime)| *mime == "application/json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue, header};

    fn headers(accept: &str, xhr: bool) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::ACCEPT, HeaderValue::from_str(accept).unwrap());
        if xhr {
            h.insert(
                "x-requested-with",
                HeaderValue::from_static("XMLHttpRequest"),
            );
        }
        h
    }

    #[test]
    fn json_from_accept_as_actiondispatch_decides() {
        // As the reference answered /latest for each.
        let ember = "application/json, text/javascript, */*; q=0.01";
        assert!(wants_json(None, &headers(ember, true)));
        assert!(wants_json(None, &headers("application/json", false)));
        // Browser-like (*/* beside other types) and not an XHR: HTML.
        assert!(!wants_json(None, &headers("application/json, */*", false)));
        assert!(!wants_json(
            None,
            &headers("text/html,application/xhtml+xml,*/*;q=0.8", false)
        ));
        // q orders the types.
        assert!(!wants_json(
            None,
            &headers("application/json;q=0.5, text/html", false)
        ));
        // The format param wins over the header.
        assert!(wants_json(
            Some("page=2&format=json"),
            &headers("text/html", false)
        ));
        assert!(!wants_json(
            Some("format=html"),
            &headers("application/json", false)
        ));
        assert!(!wants_json(None, &HeaderMap::new()));
    }

    #[test]
    fn json_twins_are_the_pages_rails_answers_as_json() {
        for path in [
            "/latest",
            "/c/general/4/l/top",
            "/t/welcome/5",
            "/u/user1/summary",
            "/tag/guide",
        ] {
            assert!(json_twin_page(path), "{path}");
        }
        for path in [
            "/",
            "/search",
            "/login",
            "/my/messages",
            "/latest.json",
            "/uploads/x.png",
        ] {
            assert!(!json_twin_page(path), "{path}");
        }
    }
}
