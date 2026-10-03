mod accounts;
mod admin_email;
mod bookmarks;
mod list;
mod login_required;
mod messages;
mod notifications;
mod post_actions;
mod post_destroy;
mod posts;
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
mod tags;
mod topic_status;
mod topics;
mod users;

use axum::Router;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use tower_http::services::ServeDir;

use crate::AppState;

const SITE_CSS: &str = include_str!("../../static/site.css");

async fn site_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        SITE_CSS,
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
        .route("/bookmarks", get(list::user_list))
        .route("/bookmarks.json", get(list::user_list))
        .route("/notifications", get(notifications::index))
        .route("/notifications.json", get(notifications::index))
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
        .route("/u/{username}", get(users::show))
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
        .nest_service("/uploads", ServeDir::new(public.join("uploads")));
    if let Some(emoji) = config.emoji_dir() {
        router = router.nest_service("/images/emoji", ServeDir::new(emoji));
    }
    router
        .nest_service("/images", images)
        .layer(axum::middleware::from_fn_with_state(
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
