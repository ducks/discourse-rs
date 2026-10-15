//! Middleware::RequestTracker's logging: every request counted in
//! application_requests (CachedCounting), crawler page views per user
//! agent in web_crawler_requests, the beacon's browser page views
//! (BrowserPageviewEvent), the engagement beacon
//! (BrowserPageviewSessionEngagement), and a browser page view of a topic
//! recorded as a topic view (TopicsController.defer_topic_view).
//!
//! Rails loads the tracker outside development only (or with
//! TRACK_REQUESTS), so none of this runs against the dev reference:
//! tests/request_tracking.rs pins it to lib/middleware/request_tracker.rb.
//! Counts wait in the process and reach the database once a minute, as
//! CachedCounting's thread flushes Redis; the work Rails hands to
//! Scheduler::Defer runs in a spawned task.
//!
//! There is no MaxMind database here, so DiscourseIpInfo has nothing for
//! an address: events carry no country or ASN, and no address is a
//! crawler's (`CrawlerDetection.crawler_ip?`).

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use axum::http::{HeaderMap, Method, header};
use chrono::{NaiveDate, NaiveDateTime};
use serde_json::Value;
use sqlx::PgConnection;

use crate::browser_pageviews as bpv;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `Middleware::RequestTracker::MAX_URL_LENGTH`
const MAX_URL_LENGTH: usize = 2000;
/// `MAX_SESSION_ID_LENGTH`
const MAX_SESSION_ID_LENGTH: usize = 32;
/// `MAX_USER_AGENT_LENGTH`
const MAX_USER_AGENT_LENGTH: usize = 1000;
/// `MAX_IP_ADDRESS_LENGTH`
const MAX_IP_ADDRESS_LENGTH: usize = 45;
/// `BrowserPageviewEvent::MAX_LANGUAGE_LENGTH`
const MAX_LANGUAGE_LENGTH: usize = 255;
/// `BrowserPageviewEvent::MAX_NORMALIZED_LANGUAGE_LENGTH`
const MAX_NORMALIZED_LANGUAGE_LENGTH: usize = 13;
/// `BrowserPageviewEvent::REDIS_FLUSH_BATCH_SIZE`
pub const FLUSH_BATCH_SIZE: usize = 1000;
/// `BrowserPageviewEvent::REDIS_QUEUE_MAX_SIZE`
const QUEUE_MAX_SIZE: usize = 1_000_000;
/// `BrowserPageviewEvent::REDIS_QUEUE_TTL`, in seconds.
const QUEUE_TTL_SECONDS: i64 = 24 * 3600;
/// `BrowserPageviewSessionEngagement::MAX_COLUMN_VALUE`
const MAX_COLUMN_VALUE: i64 = i32::MAX as i64;
/// `CachedCounting::FLUSH_DB_ITERATIONS` of one second sleeps.
const FLUSH_SECONDS: u64 = 60;
/// `UserAuthToken`'s token size (`TOKEN_SIZE`), a v0 cookie's length.
const TOKEN_SIZE: usize = 32;

/// `str.slice(0, n)`, by characters.
fn slice(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
}

/// `Discourse.beacon_pv_tracking_path`
pub fn beacon_path(base: &str) -> String {
    format!("{base}/srv/pv")
}

/// `Discourse.engagement_tracking_path`
pub fn engagement_path(base: &str) -> String {
    format!("{base}/srv/se")
}

/// The pageview tracking request (`is_pageview_tracking_request?`).
pub fn pageview_path(base: &str) -> String {
    format!("{base}/pageview")
}

/// What `extract_view_tracking_data` reads of a request.
#[derive(Clone, Debug, Default)]
pub struct ViewTracking {
    pub track_view: bool,
    pub browser_page_view: bool,
    pub is_beacon: bool,
    pub is_embed: bool,
    pub topic_id: Option<i64>,
    pub tracking_url: Option<String>,
    pub tracking_referrer: Option<String>,
    pub tracking_session_id: Option<String>,
    pub tracking_language: Option<String>,
    pub user_agent: Option<String>,
}

/// The request as the tracker reads it.
pub struct RequestParts<'a> {
    pub method: &'a Method,
    pub path: &'a str,
    pub query: Option<&'a str>,
    pub headers: &'a HeaderMap,
    pub base_path: &'a str,
}

impl RequestParts<'_> {
    fn post_to(&self, path: &str) -> bool {
        self.method == Method::POST && self.path == path
    }

    pub fn is_beacon(&self) -> bool {
        self.post_to(&beacon_path(self.base_path))
    }

    pub fn is_pageview(&self) -> bool {
        self.post_to(&pageview_path(self.base_path))
    }

    pub fn is_engagement(&self) -> bool {
        self.post_to(&engagement_path(self.base_path))
    }
}

fn ruby_to_i(v: &Value) -> Result<i64, Unsupported> {
    match v {
        Value::Number(n) => Ok(n
            .as_i64()
            .or_else(|| n.as_u64().map(|_| i64::MAX))
            .unwrap_or_else(|| n.as_f64().unwrap_or(0.0).trunc() as i64)),
        Value::String(s) => Ok(crate::ruby::to_i(s)),
        Value::Null => Ok(0),
        _ => Err(Unsupported(
            "a tracking field Ruby cannot turn into an integer",
        )),
    }
}

/// `value&.slice(0, n)` for a string or nothing; Ruby slices arrays too,
/// and fails on the rest, neither of which the port follows.
fn json_slice(v: Option<&Value>, n: usize) -> Result<Option<String>, Unsupported> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(slice(s, n))),
        Some(_) => Err(Unsupported("a beacon field that is not a string")),
    }
}

/// `read_json_body`: the parsed body, None when blank or not JSON.
fn read_json_body(body: &[u8]) -> Option<Value> {
    if crate::ruby::is_blank(&String::from_utf8_lossy(body)) {
        return None;
    }
    serde_json::from_slice(body).ok()
}

/// `extract_beacon_view_tracking_data`: the beacon's JSON body.
fn extract_beacon(headers: &HeaderMap, body: &[u8]) -> Result<ViewTracking, Unsupported> {
    let data = match read_json_body(body) {
        None | Some(Value::Null) | Some(Value::Bool(false)) => serde_json::Map::new(),
        Some(Value::Object(map)) => map,
        Some(_) => return Err(Unsupported("a beacon body that is not a JSON object")),
    };
    let topic_id = match data.get("topic_id") {
        None | Some(Value::Null) => None,
        Some(v) => Some(ruby_to_i(v)?),
    };
    let tracking_language = match data.get("language") {
        Some(Value::String(s)) => Some(slice(s, MAX_LANGUAGE_LENGTH)),
        _ => None,
    };
    Ok(ViewTracking {
        track_view: false,
        browser_page_view: true,
        is_beacon: true,
        is_embed: data.get("embed") == Some(&Value::Bool(true)),
        topic_id,
        tracking_url: json_slice(data.get("url"), MAX_URL_LENGTH)?,
        tracking_referrer: json_slice(data.get("referrer"), MAX_URL_LENGTH)?,
        tracking_session_id: json_slice(data.get("session_id"), MAX_SESSION_ID_LENGTH)?,
        tracking_language,
        user_agent: header_str(headers, "user-agent").map(|ua| slice(&ua, MAX_USER_AGENT_LENGTH)),
    })
}

/// `extract_view_tracking_data(env, status, headers)`; `body` is the
/// beacon's, read before the app runs.
pub fn extract_view_tracking(
    req: &RequestParts<'_>,
    status: u16,
    content_type: Option<&str>,
    body: &[u8],
) -> Result<ViewTracking, Unsupported> {
    if req.is_beacon() {
        return extract_beacon(req.headers, body);
    }
    let headers = req.headers;
    let is_html = content_type.is_some_and(|c| c.contains("text/html"));
    let is_ajax = headers
        .get("x-requested-with")
        .is_some_and(|v| v.as_bytes() == b"XMLHttpRequest");
    let env_track_view = header_str(headers, "discourse-track-view");
    let truthy = |v: &Option<String>| matches!(v.as_deref(), Some("1" | "true"));
    let explicit = status == 200 && truthy(&env_track_view);
    let implicit = status == 200
        && !matches!(env_track_view.as_deref(), Some("0" | "false"))
        && req.method == Method::GET
        && !is_ajax
        && is_html;
    let deferred =
        truthy(&header_str(headers, "discourse-track-view-deferred")) && !req.is_pageview();
    let embed_mode = req.query.and_then(|q| {
        form_urlencoded::parse(q.as_bytes())
            .filter(|(k, _)| k == "embed_mode")
            .last()
            .map(|(_, v)| v.into_owned())
    });
    let is_embed = embed_mode.as_deref() == Some("true")
        || truthy(&header_str(headers, "discourse-track-view-embed"));
    Ok(ViewTracking {
        track_view: explicit || implicit,
        browser_page_view: explicit || deferred,
        is_beacon: false,
        is_embed,
        topic_id: header_str(headers, "discourse-track-view-topic-id")
            .map(|v| crate::ruby::to_i(&v)),
        tracking_url: header_str(headers, "discourse-track-view-url")
            .map(|v| slice(&v, MAX_URL_LENGTH)),
        tracking_referrer: header_str(headers, "discourse-track-view-referrer")
            .map(|v| slice(&v, MAX_URL_LENGTH)),
        tracking_session_id: header_str(headers, "discourse-track-view-session-id")
            .map(|v| slice(&v, MAX_SESSION_ID_LENGTH)),
        tracking_language: None,
        user_agent: header_str(headers, "user-agent").map(|ua| slice(&ua, MAX_USER_AGENT_LENGTH)),
    })
}

/// What the session layer learned that the tracker reads back from the
/// env: an API request (`API_KEY_ENV` / `USER_API_KEY_ENV`) and its user,
/// and a cookie whose token found no one (`BAD_TOKEN`).
#[derive(Clone, Debug, Default)]
pub struct Tracked {
    pub is_api: bool,
    pub is_user_api: bool,
    pub user_id: Option<i64>,
    pub bad_token: bool,
}

tokio::task_local! {
    static TRACKED: std::sync::Arc<Mutex<Tracked>>;
}

fn with_tracked(f: impl FnOnce(&mut Tracked)) {
    let _ = TRACKED.try_with(|t| f(&mut t.lock().unwrap_or_else(|e| e.into_inner())));
}

/// An admin (`user_api` false) or user API key let the request in.
pub fn mark_api(user_api: bool, user_id: i32) {
    with_tracked(|t| {
        if user_api {
            t.is_user_api = true;
        } else {
            t.is_api = true;
        }
        t.user_id = Some(user_id.into());
    });
}

/// `env[BAD_TOKEN] = true`
pub fn mark_bad_token() {
    with_tracked(|t| t.bad_token = true);
}

/// Runs the app with the env the session layer marks, and what it marked.
pub async fn scope<F: std::future::Future>(f: F) -> (F::Output, Tracked) {
    let tracked = std::sync::Arc::new(Mutex::new(Tracked::default()));
    let out = TRACKED.scope(tracked.clone(), f).await;
    let tracked = tracked.lock().unwrap_or_else(|e| e.into_inner()).clone();
    (out, tracked)
}

/// The auth cookie as `get_data` finds it: v0 (the raw token) or v1 (the
/// encrypted hash).
pub enum AuthCookie {
    Raw,
    Hash { user_id: Option<i64> },
}

/// `find_v0_auth_cookie(request) || find_v1_auth_cookie(env)`
pub fn find_auth_cookie(state: &AppState, headers: &HeaderMap) -> Option<AuthCookie> {
    let name = crate::session::current::TOKEN_COOKIE;
    let raw = crate::session::current::cookie(headers, name)?;
    if crate::ruby::is_blank(raw) {
        return None;
    }
    if raw.chars().count() == TOKEN_SIZE {
        return Some(AuthCookie::Raw);
    }
    let map = state.keys.codec.decrypt(name, raw).ok()?;
    if map.is_empty() {
        return None;
    }
    Some(AuthCookie::Hash {
        user_id: map.get("user_id").and_then(|v| v.as_i64()),
    })
}

/// `get_data`'s request data, short of what needs the site settings.
pub struct RequestData {
    pub status: u16,
    pub view: ViewTracking,
    pub has_auth_cookie: bool,
    pub current_user_id: Option<i64>,
    pub is_api: bool,
    pub is_user_api: bool,
    pub is_background: bool,
    pub is_mobile: bool,
    /// The helper's user agent (`HttpUserAgentEncoder.ensure_utf8`).
    pub agent: String,
    pub via: Option<String>,
    pub discourse_render: Option<String>,
    pub request_remote_ip: String,
    pub occurred_at: NaiveDateTime,
}

impl RequestData {
    /// `get_data(env, result, ...)`.
    pub fn new(
        req: &RequestParts<'_>,
        view: ViewTracking,
        status: u16,
        cookie: Option<AuthCookie>,
        api: &Tracked,
        remote_ip: &str,
    ) -> RequestData {
        let headers = req.headers;
        let agent = header_str(headers, "user-agent").unwrap_or_default();
        let current_user_id = if view.browser_page_view {
            match &cookie {
                Some(AuthCookie::Hash { user_id }) => *user_id,
                // CurrentUser.lookup_from_env: an API key's user.
                _ => api.user_id,
            }
        } else {
            None
        };
        let base = req.base_path;
        RequestData {
            status,
            has_auth_cookie: cookie.is_some(),
            current_user_id,
            is_api: api.is_api,
            is_user_api: api.is_user_api,
            is_background: req.path.starts_with(&format!("{base}/message-bus/"))
                || req.path.starts_with(&format!("{base}/topics/timings")),
            is_mobile: crate::mobile::mobile_device(&agent),
            via: header_str(headers, "via"),
            discourse_render: header_str(headers, "discourse-render"),
            request_remote_ip: slice(remote_ip, MAX_IP_ADDRESS_LENGTH),
            occurred_at: crate::clock::now_naive(),
            agent,
            view,
        }
    }

    /// The helper's `is_crawler?`.
    pub fn is_crawler(&self, settings: &SiteSettings) -> Result<bool, AppError> {
        if self.discourse_render.as_deref() == Some("crawler")
            || crate::crawler_detection::crawler(settings, Some(&self.agent), self.via.as_deref())?
        {
            return Ok(true);
        }
        let lower = self.agent.to_lowercase();
        Ok(lower.contains("discourse") && !lower.contains("mobile"))
    }
}

/// `tracks_browser_page_view?(data)`
pub fn tracks_browser_page_view(
    data: &RequestData,
    is_crawler: bool,
    login_required: bool,
) -> bool {
    if !data.view.browser_page_view || is_crawler {
        return false;
    }
    if data.has_auth_cookie {
        return true;
    }
    // crawler_ip? needs an ASN, which only MaxMind gives.
    !login_required
}

/// A count `log_request` makes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Count {
    /// `ApplicationRequest.increment!(req_type)`
    App(&'static str),
    /// `WebCrawlerRequest.increment!(user_agent)`
    Crawler(String),
}

/// `log_request(data)`: the counts, and whether the topic view is
/// recorded (`defer_topic_view`).
pub fn log_request(
    data: &RequestData,
    is_crawler: bool,
    login_required: bool,
) -> (Vec<Count>, bool) {
    use Count::App;
    let mut counts = Vec::new();
    let view = &data.view;
    if data.is_api {
        counts.push(App("api"));
    } else if data.is_user_api {
        counts.push(App("user_api"));
    } else if view.track_view {
        if is_crawler {
            counts.push(App("page_view_crawler"));
            // request_data[:user_agent], the whole one for a crawler.
            counts.push(Count::Crawler(data.agent.clone()));
        } else if view.is_embed {
            // counted below, a beacon's only.
        } else if data.has_auth_cookie {
            counts.push(App("page_view_logged_in"));
            if data.is_mobile {
                counts.push(App("page_view_logged_in_mobile"));
            }
        } else if !login_required {
            counts.push(App("page_view_anon"));
            if data.is_mobile {
                counts.push(App("page_view_anon_mobile"));
            }
        }
    }

    let mut topic_view = false;
    if tracks_browser_page_view(data, is_crawler, login_required) {
        let (logged_in, mobile) = (data.has_auth_cookie, data.is_mobile);
        if view.is_embed {
            if view.is_beacon {
                counts.push(App("page_view_embed"));
            }
        } else if view.is_beacon {
            if logged_in {
                counts.push(App("page_view_logged_in_browser_beacon"));
                if mobile {
                    counts.push(App("page_view_logged_in_browser_mobile_beacon"));
                }
            } else {
                counts.push(App("page_view_anon_browser_beacon"));
                if mobile {
                    counts.push(App("page_view_anon_browser_mobile_beacon"));
                }
            }
        } else if logged_in {
            counts.push(App("page_view_logged_in_browser"));
            if mobile {
                counts.push(App("page_view_logged_in_browser_mobile"));
            }
        } else {
            counts.push(App("page_view_anon_browser"));
            if mobile {
                counts.push(App("page_view_anon_browser_mobile"));
            }
        }
        topic_view = view.topic_id.is_some()
            && (!data.has_auth_cookie || data.current_user_id.is_some())
            && !view.is_embed;
    }

    counts.push(App("http_total"));
    let status = data.status;
    let bucket = if status >= 500 {
        Some("http_5xx")
    } else if data.is_background {
        Some("http_background")
    } else if status >= 400 {
        Some("http_4xx")
    } else if status >= 300 {
        Some("http_3xx")
    } else if status >= 200 {
        Some("http_2xx")
    } else {
        None
    };
    counts.extend(bucket.map(App));
    (counts, topic_view)
}

/// `CachedCounting`'s queue: counts by (count, UTC day), waiting for the
/// flush to the database, and whether the site's minute flush runs.
#[derive(Default)]
pub struct Counts {
    queue: Mutex<HashMap<(Count, NaiveDate), i64>>,
    flusher: AtomicBool,
}

impl Counts {
    fn queue(&self) -> std::sync::MutexGuard<'_, HashMap<(Count, NaiveDate), i64>> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// `perform_increment!`, and `ensure_thread!` for the minute's flush.
pub fn increment(state: &AppState, counts: Vec<Count>) {
    let date = crate::clock::now().date_naive();
    {
        let mut queue = state.site.counts.queue();
        for count in counts {
            *queue.entry((count, date)).or_insert(0) += 1;
        }
    }
    if !state.site.counts.flusher.swap(true, Ordering::SeqCst) {
        let state = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(FLUSH_SECONDS)).await;
                if let Err(e) = flush(&state).await {
                    tracing::warn!("Unexpected error while processing cached counts: {e}");
                }
            }
        });
    }
}

/// `CachedCounting.flush`: every waiting count written (`write_cache!`).
pub async fn flush(state: &AppState) -> Result<(), AppError> {
    let counts = std::mem::take(&mut *state.site.counts.queue());
    if counts.is_empty() {
        return Ok(());
    }
    let mut counts = counts.into_iter();
    let written = async {
        let mut conn = state.pool.acquire().await?;
        // Each count leaves the queue as it is written (HGET then HDEL):
        // one that fails is lost, the rest wait for the next flush.
        for ((count, date), n) in counts.by_ref() {
            match count {
                Count::App(req_type) => {
                    let id = crate::reports::req_type(req_type).expect("a known request type");
                    write_application_request(&mut conn, id, n, date).await?
                }
                Count::Crawler(user_agent) => {
                    write_web_crawler_request(&mut conn, &user_agent, n, date).await?
                }
            }
        }
        Ok::<(), AppError>(())
    }
    .await;
    if written.is_err() {
        let mut queue = state.site.counts.queue();
        for (key, n) in counts {
            *queue.entry(key).or_insert(0) += n;
        }
    }
    written
}

/// `ApplicationRequest.write_cache!`
async fn write_application_request(
    conn: &mut PgConnection,
    req_type: i32,
    count: i64,
    date: NaiveDate,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO application_requests (date, req_type, count) VALUES ($1, $2, $3) \
         ON CONFLICT (date, req_type) DO UPDATE SET count = application_requests.count + excluded.count",
    )
    .bind(date)
    .bind(req_type)
    .bind(count as i32)
    .execute(conn)
    .await?;
    Ok(())
}

/// `WebCrawlerRequest.write_cache!`: the day's row for the agent, made
/// with no count when missing, then counted.
async fn write_web_crawler_request(
    conn: &mut PgConnection,
    user_agent: &str,
    count: i64,
    date: NaiveDate,
) -> Result<(), sqlx::Error> {
    let found: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM web_crawler_requests WHERE date = $1 AND user_agent = $2 LIMIT 1",
    )
    .bind(date)
    .bind(user_agent)
    .fetch_optional(&mut *conn)
    .await?;
    let id = match found {
        Some(id) => id,
        None => {
            sqlx::query_scalar(
                "INSERT INTO web_crawler_requests (date, user_agent, count) VALUES ($1, $2, 0) RETURNING id",
            )
            .bind(date)
            .bind(user_agent)
            .fetch_one(&mut *conn)
            .await?
        }
    };
    sqlx::query("UPDATE web_crawler_requests SET count = count + $2 WHERE id = $1")
        .bind(id)
        .bind(count as i32)
        .execute(conn)
        .await?;
    Ok(())
}

/// `Scheduler::Defer`'s work still running.
static PENDING: AtomicUsize = AtomicUsize::new(0);
static DONE: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// `Scheduler::Defer.later(name) { ... }`: after the response, its
/// failure logged.
pub fn later<F>(name: &'static str, work: F)
where
    F: std::future::Future<Output = Result<(), AppError>> + Send + 'static,
{
    PENDING.fetch_add(1, Ordering::SeqCst);
    tokio::spawn(async move {
        if let Err(e) = work.await {
            tracing::warn!("{name}: {e}");
        }
        PENDING.fetch_sub(1, Ordering::SeqCst);
        DONE.notify_waiters();
    });
}

/// Waits for the deferred work to finish (`Scheduler::Defer.do_all_work`).
pub async fn drain() {
    loop {
        let done = DONE.notified();
        tokio::pin!(done);
        done.as_mut().enable();
        if PENDING.load(Ordering::SeqCst) == 0 {
            return;
        }
        done.await;
    }
}

/// `log_later(data, env, request)`: the counts and the beacon's event,
/// only the event (queued) while Postgres is read only.
pub fn log_later(state: &AppState, data: RequestData) {
    let state = state.clone();
    later("Track view", async move {
        let mut conn = state.pool.acquire().await?;
        let settings =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
        let pg_readonly = crate::readonly::mode(&mut conn).await?.postgres;
        drop(conn);
        let is_crawler = data.is_crawler(&settings)?;
        let login_required = settings.get("login_required")?.truthy();
        if !pg_readonly {
            let (counts, topic_view) = log_request(&data, is_crawler, login_required);
            increment(&state, counts);
            if topic_view && let Some(topic_id) = data.view.topic_id {
                let ip = data.request_remote_ip.clone();
                let user_id = data.current_user_id;
                let state = state.clone();
                later("Topic View", async move {
                    defer_topic_view(&state, topic_id, &ip, user_id).await
                });
            }
        }
        track_browser_pageview(&state, &data, is_crawler, login_required);
        Ok(())
    });
}

/// `TopicsController.defer_topic_view`: a view of a topic the viewer may
/// see, not a shared draft's.
async fn defer_topic_view(
    state: &AppState,
    topic_id: i64,
    ip: &str,
    user_id: Option<i64>,
) -> Result<(), AppError> {
    let Ok(topic_id) = i32::try_from(topic_id) else {
        return Ok(());
    };
    let mut conn = state.pool.acquire().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1)")
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        return Ok(());
    }
    let shared_draft: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM shared_drafts WHERE topic_id = $1)")
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?;
    if shared_draft {
        return Ok(());
    }
    let user_id = match user_id {
        Some(id) => match i32::try_from(id) {
            Ok(id) => Some(id),
            Err(_) => return Ok(()),
        },
        None => None,
    };
    if let Some(id) = user_id {
        let found: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
        if !found {
            return Ok(());
        }
    }
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let guardian = crate::onebox::guardian_for(&mut conn, user_id).await?;
    if !crate::onebox::can_see_topic(&mut conn, &settings, &guardian, topic_id).await? {
        return Ok(());
    }
    let hours = settings.get("topic_view_duration_hours")?.to_i();
    add_topic_view(&mut conn, topic_id, ip, user_id, hours).await
}

/// `TopicViewItem.add(topic_id, ip, user_id)`: once a day per topic and
/// user (or address), the view counted on the topic and its daily stats.
pub async fn add_topic_view(
    conn: &mut PgConnection,
    topic_id: i32,
    ip: &str,
    user_id: Option<i32>,
    duration_hours: i64,
) -> Result<(), AppError> {
    let at = crate::clock::now().date_naive();
    let key = match user_id {
        Some(id) => format!("view:{topic_id}:{at}:user-{id}"),
        None => format!("view:{topic_id}:{at}:ip-{ip}"),
    };
    if !crate::owned_schema::set_once(&mut *conn, &key, duration_hours * 3600).await? {
        return Ok(());
    }
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    let inserted = match user_id {
        None => sqlx::query(
            "INSERT INTO topic_views (topic_id, ip_address, viewed_at, user_id) \
             SELECT $1, $2::inet, $3, NULL \
             WHERE NOT EXISTS (SELECT 1 FROM topic_views \
               WHERE ip_address = $2::inet AND topic_id = $1 AND user_id IS NULL)",
        )
        .bind(topic_id)
        .bind(ip)
        .bind(at)
        .execute(&mut *tx)
        .await?
        .rows_affected(),
        // The address of a signed in viewer is not kept.
        Some(user_id) => sqlx::query(
            "INSERT INTO topic_views (topic_id, ip_address, viewed_at, user_id) \
             SELECT $1, NULL, $2, $3 \
             WHERE NOT EXISTS (SELECT 1 FROM topic_views WHERE user_id = $3 AND topic_id = $1)",
        )
        .bind(topic_id)
        .bind(at)
        .bind(user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected(),
    };
    if inserted > 0
        && let Some(user_id) = user_id
    {
        sqlx::query("UPDATE user_stats SET topics_entered = topics_entered + 1 WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE topics SET views = views + 1 WHERE id = $1")
        .bind(topic_id)
        .execute(&mut *tx)
        .await?;
    // TopicViewStat.add
    let (anon, logged_in) = if user_id.is_some() { (0, 1) } else { (1, 0) };
    sqlx::query(
        "INSERT INTO topic_view_stats (topic_id, viewed_at, anonymous_views, logged_in_views) \
         VALUES ($1, $2, $3, $4) ON CONFLICT (topic_id, viewed_at) DO UPDATE SET \
         anonymous_views = topic_view_stats.anonymous_views + $3, \
         logged_in_views = topic_view_stats.logged_in_views + $4",
    )
    .bind(topic_id)
    .bind(at)
    .bind(anon)
    .bind(logged_in)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// A beacon's event (`build_browser_pageview_event_payload`).
#[derive(Clone, Debug)]
pub struct EventPayload {
    pub user_id: Option<i64>,
    pub url: Option<String>,
    pub ip_address: String,
    pub user_agent: Option<String>,
    pub referrer: Option<String>,
    pub language: Option<String>,
    pub session_id: Option<String>,
    pub topic_id: Option<i64>,
    pub occurred_at: NaiveDateTime,
}

fn present(s: &Option<String>) -> bool {
    s.as_deref().is_some_and(|s| !crate::ruby::is_blank(s))
}

/// `track_browser_pageview(data)`: a beacon's event, when its page view
/// counts and it carries what an event needs.
fn track_browser_pageview(
    state: &AppState,
    data: &RequestData,
    is_crawler: bool,
    login_required: bool,
) {
    if !data.view.is_beacon || !tracks_browser_page_view(data, is_crawler, login_required) {
        return;
    }
    let payload = EventPayload {
        user_id: data.current_user_id,
        url: data.view.tracking_url.clone(),
        ip_address: data.request_remote_ip.clone(),
        user_agent: data.view.user_agent.clone(),
        referrer: data.view.tracking_referrer.clone(),
        language: data.view.tracking_language.clone(),
        session_id: data.view.tracking_session_id.clone(),
        topic_id: data.view.topic_id,
        occurred_at: data.occurred_at,
    };
    // REQUIRED_BROWSER_PAGEVIEW_EVENT_FIELDS
    if !present(&payload.url)
        || crate::ruby::is_blank(&payload.ip_address)
        || !present(&payload.user_agent)
        || !present(&payload.session_id)
    {
        tracing::debug!("Discarding BrowserPageviewEvent: incomplete payload");
        return;
    }
    let state = state.clone();
    later("Create BrowserPageviewEvent", async move {
        let mut conn = state.pool.acquire().await?;
        if crate::readonly::mode(&mut conn).await?.postgres {
            enqueue_for_later(&state.site.pageview_queue, payload);
            return Ok(());
        }
        match create_event(&mut conn, &payload, true).await {
            Ok(()) => Ok(()),
            Err(e) if e.sql_state().as_deref() == Some("25006") => {
                enqueue_for_later(&state.site.pageview_queue, payload);
                Ok(())
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to create BrowserPageviewEvent with payload {payload:?}: {e}"
                );
                Ok(())
            }
        }
    });
}

/// `IPAddr.new(ip)` succeeds: an address, or one with a prefix.
fn valid_ip(ip: &str) -> bool {
    let addr = ip.split_once('/').map_or(ip, |(a, _)| a);
    addr.parse::<std::net::IpAddr>().is_ok()
}

/// `BrowserPageviewEvent.create_from_payload!` (`callbacks`, with
/// truncate_fields' cut of the normalized language) or one row of
/// `insert_all` (without).
async fn create_event(
    conn: &mut PgConnection,
    payload: &EventPayload,
    callbacks: bool,
) -> Result<(), AppError> {
    // The inet column casts a bad address to nil: NotNullViolation, which
    // the tracker discards.
    if !valid_ip(&payload.ip_address) {
        tracing::debug!(
            "Discarding BrowserPageviewEvent: invalid IP {}",
            payload.ip_address
        );
        return Ok(());
    }
    let topic_id = payload
        .topic_id
        .map(i32::try_from)
        .transpose()
        .map_err(|_| AppError::from(Unsupported("a pageview topic id out of the integer range")))?;
    let user_id = payload
        .user_id
        .map(i32::try_from)
        .transpose()
        .map_err(|_| AppError::from(Unsupported("a pageview user id out of the integer range")))?;
    let url = payload.url.as_deref().map(|u| slice(u, MAX_URL_LENGTH));
    let referrer = payload
        .referrer
        .as_deref()
        .map(|r| slice(r, MAX_URL_LENGTH));
    let normalized_referrer = bpv::normalize_referrer(payload.referrer.as_deref())?;
    let normalized_url = bpv::normalize_site_path(payload.url.as_deref());
    let user_agent = payload
        .user_agent
        .as_deref()
        .map(|u| slice(u, MAX_USER_AGENT_LENGTH));
    let language = payload
        .language
        .as_deref()
        .map(|l| slice(l, MAX_LANGUAGE_LENGTH));
    let mut normalized_language = bpv::normalize_language(language.as_deref());
    if callbacks {
        normalized_language =
            normalized_language.map(|l| slice(&l, MAX_NORMALIZED_LANGUAGE_LENGTH));
    }
    let browser = bpv::browser_id(user_agent.as_deref().unwrap_or_default());
    sqlx::query(
        "INSERT INTO browser_pageview_events (url, normalized_url, normalized_url_version, \
           ip_address, country_code, asn, referrer, normalized_referrer, normalized_referrer_version, \
           user_agent, language, normalized_language, browser, session_id, user_id, topic_id, created_at) \
         VALUES ($1, $2, $3, $4::inet, NULL, NULL, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
    )
    .bind(url)
    .bind(normalized_url)
    .bind(bpv::SITE_PATH_VERSION)
    .bind(&payload.ip_address)
    .bind(referrer)
    .bind(normalized_referrer)
    .bind(bpv::REFERRER_VERSION)
    .bind(user_agent)
    .bind(language)
    .bind(normalized_language)
    .bind(browser)
    .bind(payload.session_id.as_deref().map(|s| slice(s, MAX_SESSION_ID_LENGTH)))
    .bind(user_id)
    .bind(topic_id)
    .bind(payload.occurred_at)
    .execute(conn)
    .await?;
    Ok(())
}

/// `BrowserPageviewEvent::REDIS_QUEUE_KEY`'s list, kept in the process
/// (Redis's role): the events Postgres refused while read only, and when
/// the list was last pushed to (its expiry).
#[derive(Default)]
pub struct PageviewQueue(Mutex<(VecDeque<EventPayload>, Option<NaiveDateTime>)>);

impl PageviewQueue {
    fn live(&self) -> std::sync::MutexGuard<'_, (VecDeque<EventPayload>, Option<NaiveDateTime>)> {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let now = crate::clock::now_naive();
        if guard
            .1
            .is_some_and(|pushed| (now - pushed).num_seconds() >= QUEUE_TTL_SECONDS)
        {
            guard.0.clear();
            guard.1 = None;
        }
        guard
    }
}

/// `BrowserPageviewEvent.enqueue_for_later(payload)`
pub fn enqueue_for_later(queue: &PageviewQueue, payload: EventPayload) {
    if !present(&payload.url)
        || !valid_ip(&payload.ip_address)
        || !present(&payload.user_agent)
        || !present(&payload.session_id)
    {
        return;
    }
    let mut queue = queue.live();
    if queue.0.len() >= QUEUE_MAX_SIZE {
        return;
    }
    queue.0.push_back(payload);
    queue.1 = Some(crate::clock::now_naive());
}

/// `BrowserPageviewEvent.queued_count`
pub fn queued_count(queue: &PageviewQueue) -> usize {
    queue.live().0.len()
}

/// `BrowserPageviewEvent.flush_queued!`: up to a batch of queued events
/// inserted, the number taken off the queue.
pub async fn flush_queued(
    queue: &PageviewQueue,
    conn: &mut PgConnection,
) -> Result<usize, AppError> {
    if crate::readonly::mode(&mut *conn).await?.postgres {
        return Ok(0);
    }
    let batch: Vec<EventPayload> = queue
        .live()
        .0
        .iter()
        .take(FLUSH_BATCH_SIZE)
        .cloned()
        .collect();
    if batch.is_empty() {
        return Ok(0);
    }
    // insert_all in a savepoint, one by one when the batch fails.
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    let mut failed = None;
    for payload in &batch {
        if let Err(e) = create_event(&mut tx, payload, false).await {
            failed = Some(e);
            break;
        }
    }
    match failed {
        None => {
            tx.commit().await?;
            drop_front(queue, batch.len());
        }
        Some(e) => {
            tx.rollback().await?;
            if e.sql_state().as_deref() == Some("25006") {
                return Ok(0);
            }
            tracing::error!(
                "Failed to insert queued BrowserPageviewEvents in bulk; retrying individually: {e}"
            );
            for payload in &batch {
                if let Err(e) = create_event(&mut *conn, payload, false).await {
                    if e.sql_state().as_deref() == Some("25006") {
                        return Err(e);
                    }
                    tracing::error!(
                        "Discarding queued BrowserPageviewEvent after insert failure: {e}"
                    );
                }
                drop_front(queue, 1);
            }
        }
    }
    Ok(batch.len())
}

fn drop_front(queue: &PageviewQueue, n: usize) {
    let mut queue = queue.live();
    let n = n.min(queue.0.len());
    queue.0.drain(..n);
}

/// `track_session_engagement(env)`: the engagement beacon's counts, kept
/// at their highest per session (`upsert_from_payload`).
pub fn track_session_engagement(state: &AppState, body: &[u8]) {
    let Some(Value::Object(payload)) = read_json_body(body) else {
        return;
    };
    let state = state.clone();
    later("Track session engagement", async move {
        let mut conn = state.pool.acquire().await?;
        if crate::readonly::mode(&mut conn).await?.postgres {
            return Ok(());
        }
        let settings =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
        let max_engaged = settings.get("browser_pageview_max_engaged_seconds")?.to_i();
        match engagement(&payload, max_engaged) {
            Ok(Some(row)) => upsert_engagement(&mut conn, &row).await,
            Ok(None) => Ok(()),
            Err(e) => {
                tracing::warn!("Discarding session engagement: {e}");
                Ok(())
            }
        }
    });
}

/// A session's engagement as `upsert_from_payload` writes it.
#[derive(Debug, PartialEq)]
pub struct Engagement {
    pub session_id: String,
    /// mouse move, click, key, scroll, touch and back/forward events.
    pub events: [i64; 6],
    pub engaged_seconds: i64,
    pub time_to_first_interaction_ms: Option<i64>,
}

/// `BrowserPageviewSessionEngagement::INTERACTION_COLUMNS`
const INTERACTION_FIELDS: [&str; 6] = [
    "mouse_move_events",
    "click_events",
    "key_events",
    "scroll_events",
    "touch_events",
    "back_forward_events",
];

/// Ruby's `blank?` for a JSON value.
fn json_blank(v: &Value) -> bool {
    match v {
        Value::Null | Value::Bool(false) => true,
        Value::String(s) => crate::ruby::is_blank(s),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// The payload's values, None for a blank session; Err where Ruby
/// raises (and the engagement is discarded).
pub fn engagement(
    payload: &serde_json::Map<String, Value>,
    max_engaged_seconds: i64,
) -> Result<Option<Engagement>, Unsupported> {
    let field = |k: &str| ruby_to_i(payload.get(k).unwrap_or(&Value::Null));
    let mut events = [0i64; 6];
    for (slot, name) in events.iter_mut().zip(INTERACTION_FIELDS) {
        *slot = field(name)?.clamp(0, MAX_COLUMN_VALUE);
    }
    let engaged_seconds = field("engaged_seconds")?
        .clamp(0, max_engaged_seconds.max(0))
        .clamp(0, MAX_COLUMN_VALUE);
    let time_to_first_interaction_ms = match payload.get("time_to_first_interaction_ms") {
        Some(v) if !json_blank(v) => Some(ruby_to_i(v)?),
        _ => None,
    }
    .filter(|ms| *ms <= MAX_COLUMN_VALUE);
    let session_id = match payload.get("session_id") {
        None => return Ok(None),
        Some(v) if json_blank(v) => return Ok(None),
        Some(Value::String(s)) => slice(s, MAX_SESSION_ID_LENGTH),
        Some(_) => return Err(Unsupported("a session id that is not a string")),
    };
    Ok(Some(Engagement {
        session_id,
        events,
        engaged_seconds,
        time_to_first_interaction_ms,
    }))
}

async fn upsert_engagement(conn: &mut PgConnection, row: &Engagement) -> Result<(), AppError> {
    let ttfi = match row.time_to_first_interaction_ms {
        Some(ms) if ms < i64::from(i32::MIN) => {
            return Err(Unsupported("a time to first interaction out of the integer range").into());
        }
        other => other.map(|ms| ms as i32),
    };
    let [mouse, click, key, scroll, touch, back] = row.events.map(|v| v as i32);
    sqlx::query(
        "INSERT INTO browser_pageview_session_engagements (session_id, mouse_move_events, \
           click_events, key_events, scroll_events, touch_events, back_forward_events, \
           engaged_seconds, time_to_first_interaction_ms, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now(), now()) \
         ON CONFLICT (session_id) DO UPDATE SET \
           mouse_move_events = GREATEST(browser_pageview_session_engagements.mouse_move_events, EXCLUDED.mouse_move_events), \
           click_events = GREATEST(browser_pageview_session_engagements.click_events, EXCLUDED.click_events), \
           key_events = GREATEST(browser_pageview_session_engagements.key_events, EXCLUDED.key_events), \
           scroll_events = GREATEST(browser_pageview_session_engagements.scroll_events, EXCLUDED.scroll_events), \
           touch_events = GREATEST(browser_pageview_session_engagements.touch_events, EXCLUDED.touch_events), \
           back_forward_events = GREATEST(browser_pageview_session_engagements.back_forward_events, EXCLUDED.back_forward_events), \
           engaged_seconds = GREATEST(browser_pageview_session_engagements.engaged_seconds, EXCLUDED.engaged_seconds), \
           time_to_first_interaction_ms = GREATEST(browser_pageview_session_engagements.time_to_first_interaction_ms, EXCLUDED.time_to_first_interaction_ms), \
           updated_at = EXCLUDED.updated_at",
    )
    .bind(&row.session_id)
    .bind(mouse)
    .bind(click)
    .bind(key)
    .bind(scroll)
    .bind(touch)
    .bind(back)
    .bind(row.engaged_seconds as i32)
    .bind(ttfi)
    .execute(conn)
    .await?;
    Ok(())
}

/// `same_origin_request?(request)`: the Origin (else the Referer) on the
/// site's scheme, host and port.
pub fn same_origin(headers: &HeaderMap, base_url_no_prefix: &str) -> bool {
    let origin = header_str(headers, "origin")
        .filter(|o| !crate::ruby::is_blank(o))
        .or_else(|| {
            header_str(headers, header::REFERER.as_str()).filter(|r| !crate::ruby::is_blank(r))
        });
    let Some(origin) = origin else {
        return false;
    };
    match (origin_parts(base_url_no_prefix), origin_parts(&origin)) {
        (Some(site), Some(origin)) => site == origin,
        _ => false,
    }
}

/// `URI.parse(s)`'s scheme (downcased), host and port (the scheme's
/// default for http and https); None where URI raises.
fn origin_parts(s: &str) -> Option<(Option<String>, Option<String>, Option<u16>)> {
    if s.chars()
        .any(|c| c.is_whitespace() || c.is_control() || !c.is_ascii())
    {
        return None;
    }
    let Some((scheme, rest)) = s.split_once("://") else {
        // No authority: no host, and a scheme only before a colon.
        let scheme = s
            .split_once(':')
            .map(|(sc, _)| sc.to_ascii_lowercase())
            .filter(|sc| {
                sc.starts_with(|c: char| c.is_ascii_alphabetic())
                    && sc
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
            });
        let port = match scheme.as_deref() {
            Some("http") => Some(80),
            Some("https") => Some(443),
            _ => None,
        };
        return Some((scheme, None, port));
    };
    let scheme = scheme.to_ascii_lowercase();
    if !scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = if host_port.starts_with('[') {
        let end = host_port.find(']')?;
        let (h, p) = host_port.split_at(end + 1);
        (h, p.strip_prefix(':'))
    } else {
        match host_port.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (host_port, None),
        }
    };
    let default = match scheme.as_str() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    };
    let port = match port {
        None | Some("") => default,
        Some(p) if p.chars().all(|c| c.is_ascii_digit()) => Some(p.parse().ok()?),
        Some(_) => return None,
    };
    let host = (!host.is_empty()).then(|| host.to_string());
    Some((Some(scheme), host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    #[test]
    fn same_origin_compares_scheme_host_and_port() {
        let site = "http://test.localhost";
        assert!(same_origin(
            &headers(&[("origin", "http://test.localhost")]),
            site
        ));
        assert!(same_origin(
            &headers(&[("origin", "http://test.localhost:80/x")]),
            site
        ));
        assert!(!same_origin(
            &headers(&[("origin", "https://test.localhost")]),
            site
        ));
        assert!(!same_origin(
            &headers(&[("origin", "http://test.localhost:3000")]),
            site
        ));
        assert!(!same_origin(
            &headers(&[("origin", "http://evil.com")]),
            site
        ));
        assert!(same_origin(
            &headers(&[("referer", "http://test.localhost/t/1")]),
            site
        ));
        assert!(!same_origin(&headers(&[]), site));
        assert!(!same_origin(&headers(&[("origin", "null")]), site));
    }

    fn data(view: ViewTracking) -> RequestData {
        RequestData {
            status: 200,
            view,
            has_auth_cookie: false,
            current_user_id: None,
            is_api: false,
            is_user_api: false,
            is_background: false,
            is_mobile: false,
            agent: "Mozilla".into(),
            via: None,
            discourse_render: None,
            request_remote_ip: "1.2.3.4".into(),
            occurred_at: crate::clock::now_naive(),
        }
    }

    #[test]
    fn counts_follow_log_request() {
        use Count::App;
        let view = ViewTracking {
            track_view: true,
            ..Default::default()
        };
        let (counts, _) = log_request(&data(view.clone()), false, false);
        assert_eq!(
            counts,
            vec![App("page_view_anon"), App("http_total"), App("http_2xx")]
        );
        let (counts, _) = log_request(&data(view.clone()), false, true);
        assert_eq!(counts, vec![App("http_total"), App("http_2xx")]);
        let (counts, _) = log_request(&data(view), true, false);
        assert_eq!(
            counts,
            vec![
                App("page_view_crawler"),
                Count::Crawler("Mozilla".into()),
                App("http_total"),
                App("http_2xx")
            ]
        );

        let beacon = ViewTracking {
            browser_page_view: true,
            is_beacon: true,
            topic_id: Some(3),
            ..Default::default()
        };
        let mut d = data(beacon);
        d.status = 204;
        d.is_mobile = true;
        let (counts, topic_view) = log_request(&d, false, false);
        assert_eq!(
            counts,
            vec![
                App("page_view_anon_browser_beacon"),
                App("page_view_anon_browser_mobile_beacon"),
                App("http_total"),
                App("http_2xx")
            ]
        );
        assert!(topic_view);
        // A cookie without a user to view as: no topic view.
        d.has_auth_cookie = true;
        let (_, topic_view) = log_request(&d, false, false);
        assert!(!topic_view);

        let mut d = data(ViewTracking::default());
        d.is_background = true;
        d.status = 404;
        assert_eq!(
            log_request(&d, false, false).0,
            vec![App("http_total"), App("http_background")]
        );
        d.status = 502;
        assert_eq!(
            log_request(&d, false, false).0,
            vec![App("http_total"), App("http_5xx")]
        );
        d.status = 101;
        d.is_background = false;
        assert_eq!(log_request(&d, false, false).0, vec![App("http_total")]);
    }

    #[test]
    fn engagement_follows_upsert_from_payload() {
        let payload = |v: Value| v.as_object().unwrap().clone();
        let row = engagement(
            &payload(serde_json::json!({
                "session_id": "abcdefghijklmnopqrstuvwxyz0123456789",
                "click_events": "4",
                "scroll_events": -3,
                "key_events": 2.9,
                "engaged_seconds": 99999,
                "time_to_first_interaction_ms": 0
            })),
            1800,
        )
        .unwrap()
        .unwrap();
        assert_eq!(row.session_id, "abcdefghijklmnopqrstuvwxyz012345");
        assert_eq!(row.events, [0, 4, 2, 0, 0, 0]);
        assert_eq!(row.engaged_seconds, 1800);
        assert_eq!(row.time_to_first_interaction_ms, Some(0));
        let blank = engagement(&payload(serde_json::json!({"session_id": " "})), 1800).unwrap();
        assert_eq!(blank, None);
        assert!(
            engagement(
                &payload(serde_json::json!({"session_id": "a", "click_events": true})),
                1800
            )
            .is_err()
        );
        let big = engagement(
            &payload(serde_json::json!({"session_id": "a", "time_to_first_interaction_ms": 3_000_000_000i64})),
            1800,
        )
        .unwrap()
        .unwrap();
        assert_eq!(big.time_to_first_interaction_ms, None);
    }
}
