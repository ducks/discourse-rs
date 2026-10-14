//! Read-only mode (Discourse.readonly_mode? and ReadOnlyMixin): while one
//! of the keys is set, every write request is refused with a 503, reads
//! carry `Discourse-Readonly: true`, and the writes Rails skips on reads
//! (a fancy title saved back, a search logged, the scheduled jobs) are
//! skipped. The keys live in `discourse_rs.expiring_keys`, as Rails keeps
//! them in Redis: TTL ones expire, the others are far in the future.

use askama::Template;
use axum::extract::{MatchedPath, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sqlx::PgConnection;

use crate::html::{Chrome, Crawler};
use crate::session::current::Incoming;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// Discourse::READONLY_MODE_KEY: a backup restore's, 60 s kept alive.
pub const READONLY_MODE_KEY: &str = "readonly_mode";
/// Discourse::PG_READONLY_MODE_KEY: Postgres failover, 300 s.
pub const PG_READONLY_MODE_KEY: &str = "readonly_mode:postgres";
/// Discourse::USER_READONLY_MODE_KEY: the admin toggle, no TTL.
pub const USER_READONLY_MODE_KEY: &str = "readonly_mode:user";
/// Discourse::PG_FORCE_READONLY_MODE_KEY: no TTL.
pub const PG_FORCE_READONLY_MODE_KEY: &str = "readonly_mode:postgres_force";
/// Discourse::STAFF_WRITES_ONLY_MODE_KEY: staff can still write.
pub const STAFF_WRITES_ONLY_MODE_KEY: &str = "readonly_mode:staff_writes_only";

/// Discourse::READONLY_KEYS: any of them makes the site read only.
const READONLY_KEYS: [&str; 4] = [
    READONLY_MODE_KEY,
    PG_READONLY_MODE_KEY,
    USER_READONLY_MODE_KEY,
    PG_FORCE_READONLY_MODE_KEY,
];

/// `Discourse.readonly_channel`
pub const CHANNEL: &str = "/site/read-only";

/// What `check_readonly_mode` sets: `@readonly_mode` (also true in
/// staff-writes-only mode) and `@staff_writes_only_mode`, plus
/// `pg_readonly_mode?` for the writes only Postgres failover skips.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mode {
    pub readonly: bool,
    pub staff_writes_only: bool,
    pub postgres: bool,
}

impl Mode {
    /// `Discourse.readonly_mode?`: one of the READONLY_KEYS, not the
    /// staff-writes-only pseudo mode.
    pub fn site_readonly(&self) -> bool {
        self.readonly && !self.staff_writes_only
    }
}

/// `GlobalSetting.pg_force_readonly_mode`, process-wide as GlobalSetting is.
static FORCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Reads `pg_force_readonly_mode` (DISCOURSE_PG_FORCE_READONLY_MODE) from
/// the global settings, once at startup.
pub fn force_from_globals(globals: &crate::config::GlobalSettings) {
    FORCED.store(
        globals.get("pg_force_readonly_mode") == Some("true"),
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// The keys set now, read once (check_readonly_mode).
pub async fn mode(conn: &mut PgConnection) -> Result<Mode, sqlx::Error> {
    let keys: Vec<String> = sqlx::query_scalar(
        "SELECT key FROM discourse_rs.expiring_keys \
         WHERE key LIKE 'readonly\\_mode%' AND expires_at > clock_timestamp()",
    )
    .fetch_all(&mut *conn)
    .await?;
    let has = |k: &str| keys.iter().any(|key| key == k);
    let forced = FORCED.load(std::sync::atomic::Ordering::Relaxed);
    let readonly = forced || READONLY_KEYS.iter().any(|k| has(k));
    let staff_writes_only = !readonly && has(STAFF_WRITES_ONLY_MODE_KEY);
    Ok(Mode {
        readonly: readonly || staff_writes_only,
        staff_writes_only,
        postgres: has(PG_READONLY_MODE_KEY),
    })
}

/// The key's lifetime as Discourse.enable_readonly_mode gives it: the
/// user, forced and staff-writes-only keys never expire.
fn ttl_seconds(key: &str) -> Option<i64> {
    match key {
        USER_READONLY_MODE_KEY | PG_FORCE_READONLY_MODE_KEY | STAFF_WRITES_ONLY_MODE_KEY => None,
        PG_READONLY_MODE_KEY => Some(300),
        _ => Some(60),
    }
}

/// Sets the key alone (`redis.set` / `setex`), without telling browsers.
pub async fn set_key(conn: &mut PgConnection, key: &str) -> Result<(), sqlx::Error> {
    let seconds = ttl_seconds(key).unwrap_or(100 * 365 * 24 * 3600) as f64;
    sqlx::query(
        "INSERT INTO discourse_rs.expiring_keys (key, expires_at) \
         VALUES ($1, clock_timestamp() + make_interval(secs => $2)) \
         ON CONFLICT (key) DO UPDATE SET expires_at = EXCLUDED.expires_at",
    )
    .bind(key)
    .bind(seconds)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `Discourse.enable_readonly_mode(key)`: the key, then
/// `MessageBus.publish("/site/read-only", true)` to everyone. A TTL key is
/// not kept alive (Rails' keep_readonly_mode thread): whoever sets one
/// sets it again before it lapses.
pub async fn enable(conn: &mut PgConnection, bus: &pg_bus::Bus, key: &str) -> Result<(), AppError> {
    set_key(conn, key).await?;
    bus.publish(conn, CHANNEL, &json!(true), None).await?;
    Ok(())
}

/// `Discourse.disable_readonly_mode(key)`: DEL, then publish false, set
/// or not.
pub async fn disable(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    key: &str,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM discourse_rs.expiring_keys WHERE key = $1")
        .bind(key)
        .execute(&mut *conn)
        .await?;
    bus.publish(conn, CHANNEL, &json!(false), None).await?;
    Ok(())
}

/// `raise Discourse::ReadOnly` from a handler (an allowlisted action that
/// refuses some users itself); the gate renders it as the 503.
#[derive(Debug)]
pub struct ReadOnly;

impl std::fmt::Display for ReadOnly {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the site is in read only mode")
    }
}

impl std::error::Error for ReadOnly {}

/// Marks a response as Discourse::ReadOnly raised, for the gate to render.
#[derive(Debug, Clone, Copy)]
pub struct Raised;

/// Actions `allow_in_readonly_mode` lets through, by route.
fn allowed_in_readonly_mode(method: &Method, route: &str) -> bool {
    // Admin::BackupsController: create, cancel, restore, rollback, readonly,
    // email, destroy, the chunk upload and the multipart uploads.
    match *method {
        Method::PUT => matches!(
            route,
            "/admin/backups/readonly" | "/admin/backups/readonly.json" | "/admin/backups/{id}"
        ),
        Method::DELETE => matches!(
            route,
            "/admin/backups/{id}" | "/admin/backups/cancel" | "/admin/backups/cancel.json"
        ),
        Method::POST => matches!(
            route,
            "/admin/backups"
                | "/admin/backups.json"
                | "/admin/backups/{id}/restore"
                | "/admin/backups/{id}/restore.json"
                | "/admin/backups/rollback"
                | "/admin/backups/rollback.json"
                | "/admin/backups/upload"
                | "/admin/backups/upload.json"
                | "/admin/backups/create-multipart"
                | "/admin/backups/create-multipart.json"
                | "/admin/backups/complete-multipart"
                | "/admin/backups/complete-multipart.json"
                | "/admin/backups/abort-multipart"
                | "/admin/backups/abort-multipart.json"
                | "/admin/backups/batch-presign-multipart-parts"
                | "/admin/backups/batch-presign-multipart-parts.json"
        ),
        _ => false,
    }
}

/// Actions `allow_in_staff_writes_only_mode` lets through, by route. The
/// handlers then refuse anyone but staff themselves.
fn allowed_in_staff_writes_only_mode(method: &Method, route: &str) -> bool {
    match *method {
        // SessionController#create, #forgot_password,
        // #redeem_password_reset_code; UsersController#email_login.
        Method::POST => matches!(
            route,
            "/session"
                | "/session.json"
                | "/session/forgot_password"
                | "/session/forgot_password.json"
                | "/session/password-reset-code/verify"
                | "/session/password-reset-code/verify.json"
                | "/u/email-login"
                | "/u/email-login.json"
                | "/users/email-login"
                | "/users/email-login.json"
        ),
        // UsersController#password_reset_update.
        Method::PUT => matches!(
            route,
            "/u/password-reset/{token}" | "/users/password-reset/{token}"
        ),
        _ => false,
    }
}

/// Routes under AdminConstraint (not only StaffConstraint): a moderator
/// does not reach them in Rails, so their handlers answer the routing 404.
fn admin_only(route: &str) -> bool {
    route.starts_with("/admin/site_settings")
        || route.starts_with("/admin/email/")
        || route.starts_with("/admin/backups")
        || route.starts_with("/admin/permalinks")
        // resources :api (the web hooks), AdminConstraint.
        || route.starts_with("/admin/api/")
        || matches!(
            route,
            "/admin/users/{user_id}/grant_moderation"
                | "/admin/users/{user_id}/grant_moderation.json"
                | "/admin/users/{user_id}/revoke_moderation"
                | "/admin/users/{user_id}/revoke_moderation.json"
                | "/admin/users/{user_id}/revoke_admin"
                | "/admin/users/{user_id}/revoke_admin.json"
                | "/admin/users/{user_id}/log_out"
                | "/admin/users/{user_id}/log_out.json"
                | "/admin/users/{user_id}/groups"
                | "/admin/users/{user_id}/groups.json"
                | "/admin/users/{user_id}/groups/{group_id}"
                | "/admin/users/{user_id}/penalty_history"
                | "/admin/users/{user_id}/penalty_history.json"
                | "/admin/users/{user_id}/grant_admin"
                | "/admin/users/{user_id}/grant_admin.json"
                | "/admin/impersonate"
                | "/admin/impersonate.json"
        )
}

/// Whether Rails' routing turns the request away before any filter runs:
/// the admin namespace is StaffConstraint, some of it AdminConstraint;
/// DELETE /admin/impersonate is ImpersonatorConstraint (an admin, or the
/// puppet of an impersonating one).
fn routing_refuses(method: &Method, route: &str, guardian: &crate::guardian::Guardian) -> bool {
    if !route.starts_with("/admin/") {
        return false;
    }
    if *method == Method::DELETE
        && matches!(route, "/admin/impersonate" | "/admin/impersonate.json")
    {
        return !guardian.is_admin() && !guardian.user().is_some_and(|u| u.is_impersonating());
    }
    !guardian.is_staff() || (!guardian.is_admin() && admin_only(route))
}

/// `check_readonly_mode`, `block_if_readonly_mode`, the `Discourse::ReadOnly`
/// rescue and `add_readonly_header`, around every routed request. Rails
/// runs it after the CSRF check and before the login checks: an anonymous
/// write is a 503, not a 403.
pub async fn gate(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    // Static files and the message bus are not Rails controllers.
    let path = request.uri().path();
    if path.starts_with("/assets/")
        || path.starts_with("/images/")
        || path.starts_with("/uploads/")
        || path.starts_with("/fonts/")
        || path.starts_with("/bus/")
    {
        return Ok(next.run(request).await);
    }
    let mode = {
        let mut conn = state.pool.acquire().await?;
        mode(&mut conn).await?
    };
    let guardian = request
        .extensions()
        .get::<Incoming>()
        .map(|i| i.guardian.clone())
        .unwrap_or_else(crate::guardian::Guardian::anonymous);
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| path.to_string());
    let json = wants_json_errors(
        request.uri().path(),
        request.uri().query(),
        request.headers(),
    );
    let method = request.method().clone();
    let headers = request.headers().clone();
    let uri = request.uri().clone();
    let writes = method != Method::GET && method != Method::HEAD;

    let blocked = writes
        && mode.readonly
        && !routing_refuses(&method, &route, &guardian)
        && !(mode.staff_writes_only
            && (allowed_in_staff_writes_only_mode(&method, &route) || guardian.is_staff()))
        && !allowed_in_readonly_mode(&method, &route);
    let (request, csrf_failed) = if blocked {
        csrf_check(&state, request).await
    } else {
        (request, false)
    };
    let mut request = request;
    request.extensions_mut().insert(mode);
    // The rescue's response skips the after_actions: no header on it.
    if blocked && !csrf_failed {
        return refused(&state, &guardian, &headers, &uri, json).await;
    }
    let mut response = next.run(request).await;
    if response.extensions().get::<Raised>().is_some() {
        return refused(&state, &guardian, &headers, &uri, json).await;
    }
    if mode.readonly {
        response
            .headers_mut()
            .insert("discourse-readonly", HeaderValue::from_static("true"));
    }
    Ok(response)
}

/// `verify_authenticity_token` runs before block_if_readonly_mode: a write
/// without a valid token goes on to its handler, which answers BAD CSRF.
/// Actions that skip the check (UsersController#create, StaticController
/// #enter, the one-click unsubscribe) are refused as read only.
async fn csrf_check(state: &AppState, request: Request) -> (Request, bool) {
    let path = request.uri().path().to_string();
    if matches!(
        path.as_str(),
        "/u" | "/u.json" | "/users" | "/users.json" | "/login"
    ) || path.starts_with("/email/unsubscribe/")
    {
        return (request, false);
    }
    let (parts, body) = request.into_parts();
    let form = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/x-www-form-urlencoded"));
    let (body, pairs) = if form && !parts.headers.contains_key("x-csrf-token") {
        match axum::body::to_bytes(body, 1 << 20).await {
            Ok(bytes) => {
                let pairs: Vec<(String, String)> = form_urlencoded::parse(&bytes)
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                (axum::body::Body::from(bytes), pairs)
            }
            Err(_) => (axum::body::Body::empty(), Vec::new()),
        }
    } else {
        (body, Vec::new())
    };
    let ok = crate::routes::csrf_ok(
        state,
        &parts.headers,
        &pairs,
        parts.uri.path(),
        parts.method.as_str(),
    );
    (Request::from_parts(parts, body), !ok)
}

/// `respond_to` for the ReadOnly rescue: the JSON format (the `.json`
/// path, the format param or the Accept header) or an XHR gets JSON.
fn wants_json_errors(path: &str, query: Option<&str>, headers: &HeaderMap) -> bool {
    path.ends_with(".json")
        || headers
            .get("x-requested-with")
            .is_some_and(|v| v == "XMLHttpRequest")
        || crate::routes::wants_json(query, headers)
}

/// exceptions/read_only in the no_ember layout.
#[derive(Template)]
#[template(path = "email_page.html")]
struct ReadOnlyPage {
    site_title: String,
    lang: String,
    base_path: String,
    crawler: Crawler,
    viewer: Option<crate::html::Viewer>,
    bus_position: String,
    chrome: Chrome,
    html: String,
}

/// The rescue's 503: `render_json_error(I18n.t("read_only_mode_enabled"),
/// type: :read_only)`, or the page.
async fn refused(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &axum::http::Uri,
    json: bool,
) -> Result<Response, AppError> {
    let message = state
        .i18n
        .t("read_only_mode_enabled")
        .unwrap_or("The site is in read only mode. Interactions are disabled.");
    if json {
        return Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({ "errors": [message], "error_type": "read_only" })),
        )
            .into_response());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let base_path = state.config.globals.relative_url_root().to_string();
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let vs = crate::routes::viewer_state(state, headers, &settings, guardian)?;
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
    let mut crawler = Crawler::for_request(&urls, uri, None)?;
    crawler.description = String::new();
    let body = ReadOnlyPage {
        site_title: site.site_title,
        lang: site.lang,
        base_path: site.base_path,
        crawler,
        viewer: site.viewer,
        bus_position: String::new(),
        chrome: site.chrome,
        html: format!("<p>{}</p>\n", crate::category_badge::html_escape(message)),
    }
    .render()
    .map_err(crate::html::HtmlError::from)?;
    let response = (StatusCode::SERVICE_UNAVAILABLE, axum::response::Html(body)).into_response();
    Ok(crate::html::with_viewer_headers(response, &vs))
}

/// GlobalNotice's read-only and staff-writes-only notices, as the page
/// shows them above its content: the member's text or the visitor's.
pub fn notice(i18n: &crate::i18n::I18n, mode: Mode, logged_in: bool) -> String {
    let (id, key) = if mode.staff_writes_only {
        ("alert-staff-writes-only", "staff_writes_only_mode")
    } else if mode.readonly {
        ("alert-read-only", "read_only_mode")
    } else {
        return String::new();
    };
    let which = if logged_in {
        "enabled"
    } else {
        "enabled_anonymous"
    };
    let text = i18n
        .t(&format!("js.{key}.{which}"))
        .unwrap_or_default()
        .to_string();
    format!(
        "<div class=\"global-notice\"><div class=\"row\"><div class=\"alert alert-info {id}\" id=\"global-notice-{id}\"><span class=\"text\">{}</span></div></div></div>",
        crate::category_badge::html_escape(&text)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlists_follow_the_controllers() {
        assert!(allowed_in_readonly_mode(
            &Method::PUT,
            "/admin/backups/readonly.json"
        ));
        assert!(!allowed_in_readonly_mode(&Method::POST, "/posts.json"));
        assert!(allowed_in_readonly_mode(
            &Method::DELETE,
            "/admin/backups/{id}"
        ));
        assert!(allowed_in_readonly_mode(
            &Method::POST,
            "/admin/backups/upload"
        ));
        assert!(allowed_in_staff_writes_only_mode(
            &Method::POST,
            "/session.json"
        ));
        assert!(allowed_in_staff_writes_only_mode(
            &Method::PUT,
            "/u/password-reset/{token}"
        ));
        assert!(!allowed_in_staff_writes_only_mode(
            &Method::DELETE,
            "/session/{username}"
        ));
    }

    #[test]
    fn admin_routes_refuse_by_constraint() {
        let anonymous = crate::guardian::Guardian::anonymous();
        assert!(routing_refuses(
            &Method::PUT,
            "/admin/users/{user_id}/silence",
            &anonymous
        ));
        assert!(!routing_refuses(&Method::POST, "/posts.json", &anonymous));
    }
}
