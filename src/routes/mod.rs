mod accounts;
mod admin_email;
mod admin_site_settings;
mod admin_users;
mod bookmark_writes;
mod bookmarks;
mod bus;
mod drafts;
mod list;
mod live;
mod login_required;
mod messages;
mod notifications;
mod post_actions;
mod post_destroy;
mod posts;
mod read_tracking;
mod review;
mod robots;
mod search;
mod uploads;
pub(crate) use search::invalid_access_with;
pub(crate) use topics::not_found_response;
mod session;
mod site;
mod sitemap;
mod srv;
mod stylesheets;
mod tags;
mod topic_status;
mod topics;
mod user_avatars;
mod user_menu;
mod user_update;
mod users;

use axum::Router;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use tower_http::services::ServeDir;

use crate::AppState;

const SITE_CSS: &str = include_str!("../../static/site.css");

/// The ported stylesheets (static/css), in the order Discourse's common
/// stylesheet imports their sources, after normalize.css.
const DISCOURSE_CSS: &str = concat!(
    include_str!("../../static/vendor/normalize.css"),
    include_str!("../../static/css/foundation.css"),
    include_str!("../../static/css/buttons.css"),
    include_str!("../../static/css/header.css"),
    include_str!("../../static/css/navs.css"),
    include_str!("../../static/css/topic-list.css"),
    include_str!("../../static/css/sidebar.css"),
    include_str!("../../static/css/topic.css"),
    include_str!("../../static/css/composer.css"),
);

async fn discourse_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        DISCOURSE_CSS,
    )
}

/// The Font Awesome icons the pages use, as a sprite of <symbol>s
/// (scripts/build-icon-sprite), which the layout includes inline.
pub const ICONS_SVG: &str = include_str!("../../static/vendor/icons.svg");

async fn site_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        SITE_CSS,
    )
}

// htmx and its SSE extension, vendored from npm (static/vendor/README).
const HTMX: &str = include_str!("../../static/vendor/htmx.min.js");
const HTMX_SSE: &str = include_str!("../../static/vendor/htmx-ext-sse.js");
const SIDEBAR_JS: &str = include_str!("../../static/js/sidebar.js");
const TOPIC_JS: &str = include_str!("../../static/js/topic.js");
const COMPOSER_JS: &str = include_str!("../../static/js/composer.js");

async fn htmx() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        HTMX,
    )
}

async fn sidebar_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        SIDEBAR_JS,
    )
}

async fn topic_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        TOPIC_JS,
    )
}

async fn composer_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        COMPOSER_JS,
    )
}

async fn htmx_sse() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        HTMX_SSE,
    )
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
        .route("/", get(list::latest))
        .route("/latest", get(list::latest))
        .route("/latest.json", get(list::latest_json))
        .route("/top", get(list::top))
        .route("/top.json", get(list::top_json))
        .route("/top/{period}", get(list::top_period_redirect))
        .route("/hot", get(list::hot))
        .route("/hot.json", get(list::hot_json))
        .route("/unread", get(list::user_list))
        .route("/unread.json", get(list::user_list))
        .route("/new", get(list::user_list))
        .route("/new.json", get(list::user_list))
        .route("/unseen", get(list::user_list))
        .route("/unseen.json", get(list::user_list))
        .route("/read", get(list::user_list))
        .route("/read.json", get(list::user_list))
        .route("/posted", get(list::user_list))
        .route("/posted.json", get(list::user_list))
        .route(
            "/bookmarks",
            get(list::user_list).post(bookmark_writes::create),
        )
        .route(
            "/bookmarks.json",
            get(list::user_list).post(bookmark_writes::create),
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
        .route("/drafts", post(drafts::create))
        .route("/drafts.json", post(drafts::create))
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
        .route("/t/{slug}/timings", post(read_tracking::topic_timings))
        .route("/topics/{kind}/{username}", get(messages::personal))
        .route(
            "/topics/private-messages-group/{username}/{*rest}",
            get(messages::group),
        )
        .route(
            "/t/{id}",
            get(topics::show_by_id).delete(post_destroy::destroy_topic),
        )
        .route("/t/{slug}/{id}", get(topics::show_with_slug))
        .route("/t/{slug}/{id}/{post_number}", get(topics::show_post))
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
        .route("/c/{*path}", get(list::category))
        .route("/robots.txt", get(robots::index))
        .route("/robots-builder.json", get(robots::builder))
        .route("/sitemap.xml", get(sitemap::index))
        .route("/sitemap_{page}", get(sitemap::page))
        .route("/news.xml", get(sitemap::news))
        .route(
            "/admin/site_settings/{id}",
            put(admin_site_settings::update),
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
        .route("/search", get(search::show))
        .route("/search.json", get(search::show_json))
        .route("/search/query", get(search::query))
        .route("/search/query.json", get(search::query))
        .route("/tag/{*path}", get(tags::show))
        .route("/tags", get(tags::index))
        .route("/tags.json", get(tags::index_json))
        .route("/tags/c/{*path}", get(tags::show_in_category))
        .route("/categories", get(list::categories))
        .route("/categories.json", get(list::categories_json))
        .route("/site", get(site::site))
        .route("/site.json", get(site::site))
        .route("/site/basic-info", get(site::basic_info))
        .route("/site/basic-info.json", get(site::basic_info))
        .route("/assets/site.css", get(site_css))
        .route("/assets/discourse.css", get(discourse_css))
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
        .route("/assets/htmx.min.js", get(htmx))
        .route("/assets/htmx-ext-sse.js", get(htmx_sse))
        .route("/assets/sidebar.js", get(sidebar_js))
        .route("/assets/topic.js", get(topic_js))
        .route("/assets/composer.js", get(composer_js))
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
