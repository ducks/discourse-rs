//! Two sites in one process (RailsMultisite): each site's AppState over its
//! own cloned database, the process-wide parts shared as main.rs shares
//! them. Every test asserts that what one site does stays out of the
//! other, except for what Rails shares between sites (the `GLOBAL::`
//! rate limits).

mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, other_site, state};
use discourse_rs::AppState;
use discourse_rs::config::{Config, RailsEnv};
use discourse_rs::request_tracking;
use discourse_rs::site_settings::SiteSettings;
use http_body_util::BodyExt;
use tower::ServiceExt;

const BROWSER: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

struct Site {
    db: TestDb,
    st: AppState,
}

/// Two sites, `a` and `b`, each with its config from `make`.
async fn two_sites(make: impl Fn() -> Config) -> (Site, Site) {
    let db_a = TestDb::new().await;
    let db_b = TestDb::new().await;
    let a = state(db_a.pool.clone(), make()).await;
    let b = other_site(&a, db_b.pool.clone(), make()).await;
    (Site { db: db_a, st: a }, Site { db: db_b, st: b })
}

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: String,
}

impl Reply {
    fn cookies(&self) -> Vec<(String, String)> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok()?.split(';').next()?.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }
}

/// A browser of one site: its cookies kept between requests.
struct Client<'a> {
    site: &'a Site,
    from: String,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl<'a> Client<'a> {
    fn new(site: &'a Site, from: &str) -> Client<'a> {
        Client {
            site,
            from: from.into(),
            cookies: Vec::new(),
            csrf: None,
        }
    }

    async fn send(
        &mut self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Reply {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost");
        if !self.cookies.is_empty() {
            let cookie: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            request = request.header(header::COOKIE, cookie.join("; "));
        }
        if let Some(token) = &self.csrf {
            request = request.header("x-csrf-token", token.as_str());
        }
        for (k, v) in headers {
            request = request.header(*k, *v);
        }
        if !body.is_empty() && !headers.iter().any(|(k, _)| *k == "content-type") {
            request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        let mut request = request.body(Body::from(body.to_string())).unwrap();
        let peer: std::net::SocketAddr = format!("{}:443", self.from).parse().unwrap();
        request.extensions_mut().insert(ConnectInfo(peer));
        let response = discourse_rs::app(self.site.st.clone())
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let reply = Reply {
            status,
            headers,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        };
        for (name, value) in reply.cookies() {
            self.cookies.retain(|(n, _)| *n != name);
            if !value.is_empty() {
                self.cookies.push((name, value));
            }
        }
        reply
    }

    async fn get(&mut self, path: &str) -> Reply {
        self.send(Method::GET, path, &[], "").await
    }

    async fn login(&mut self, login: &str) {
        let csrf = self.get("/session/csrf.json").await;
        let token: serde_json::Value = serde_json::from_str(&csrf.body).unwrap();
        self.csrf = token["csrf"].as_str().map(str::to_string);
        let reply = self
            .send(
                Method::POST,
                "/session",
                &[("x-requested-with", "XMLHttpRequest")],
                &format!("login={login}&password=password"),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    }
}

async fn settings(site: &Site) -> SiteSettings {
    let mut conn = site.db.pool.acquire().await.unwrap();
    SiteSettings::load(
        &mut conn,
        &site.st.site_setting_defs,
        &site.st.config.globals,
    )
    .await
    .unwrap()
}

/// Two sites whose site_settings tables have the same fingerprint (the
/// same row count and newest updated_at) still resolve their own values.
#[tokio::test]
async fn settings_resolve_per_site_even_when_their_tables_look_alike() {
    let (a, b) = two_sites(|| config(RailsEnv::Test, &[])).await;
    for (site, title) in [(&a, "Forum A"), (&b, "Forum B")] {
        sqlx::query(
            "INSERT INTO site_settings (name, data_type, value, created_at, updated_at) \
             VALUES ('title', 1, $1, '2030-01-01', '2030-01-01')",
        )
        .bind(title)
        .execute(&site.db.pool)
        .await
        .unwrap();
    }
    for _ in 0..2 {
        assert_eq!(settings(&a).await.get("title").unwrap().to_s(), "Forum A");
        assert_eq!(settings(&b).await.get("title").unwrap().to_s(), "Forum B");
    }
}

/// A site's translation overrides are read into its own strings: a
/// request to the other site neither sees nor drops them.
#[tokio::test]
async fn translation_overrides_are_per_site() {
    let (a, b) = two_sites(|| config(RailsEnv::Test, &[])).await;
    sqlx::query(
        "INSERT INTO translation_overrides (locale, translation_key, value, \
         original_translation, status, created_at, updated_at) \
         VALUES ('en', 'js.log_in', 'Enter', '', 0, now(), now())",
    )
    .execute(&a.db.pool)
    .await
    .unwrap();
    let page_a = Client::new(&a, "127.0.0.1").get("/latest").await;
    let page_b = Client::new(&b, "127.0.0.1").get("/latest").await;
    assert!(
        page_a
            .body
            .contains(r#"<span class="d-button-label">Enter</span>"#)
    );
    assert!(!page_b.body.contains(">Enter<"));
    assert!(
        page_b
            .body
            .contains(r#"<span class="d-button-label">Log In</span>"#)
    );
    assert_eq!(a.st.i18n.override_of("js.log_in"), Some("Enter"));
    assert_eq!(b.st.i18n.override_of("js.log_in"), None);
}

fn tracking() -> Config {
    let mut cfg = config(RailsEnv::Test, &[]);
    cfg.track_requests = true;
    cfg
}

async fn http_total(site: &Site) -> i64 {
    sqlx::query_scalar(
        "SELECT COALESCE(sum(count), 0)::bigint FROM application_requests WHERE req_type = $1",
    )
    .bind(discourse_rs::reports::req_type("http_total").unwrap())
    .fetch_one(&site.db.pool)
    .await
    .unwrap()
}

/// CachedCounting's counts wait per site and are written into the site's
/// own database, whichever site flushes first.
#[tokio::test]
async fn request_counts_land_in_their_own_sites_database() {
    let (a, b) = two_sites(tracking).await;
    let before = (http_total(&a).await, http_total(&b).await);
    for _ in 0..3 {
        Client::new(&a, "203.0.113.7").get("/about.json").await;
    }
    Client::new(&b, "203.0.113.7").get("/about.json").await;
    request_tracking::drain().await;
    request_tracking::flush(&b.st).await.unwrap();
    assert_eq!(http_total(&b).await, before.1 + 1);
    assert_eq!(
        http_total(&a).await,
        before.0,
        "a's counts wait for a's flush"
    );
    request_tracking::flush(&a.st).await.unwrap();
    assert_eq!(http_total(&a).await, before.0 + 3);
    assert_eq!(http_total(&b).await, before.1 + 1);
}

/// The beacon's events queued while a site's Postgres is read only wait
/// in that site's queue, and the other site's flush job leaves them.
#[tokio::test]
async fn queued_pageviews_wait_for_their_own_site() {
    let (a, b) = two_sites(tracking).await;
    {
        let mut conn = a.db.pool.acquire().await.unwrap();
        discourse_rs::readonly::set_key(&mut conn, discourse_rs::readonly::PG_READONLY_MODE_KEY)
            .await
            .unwrap();
    }
    let reply = Client::new(&a, "203.0.113.7")
        .send(
            Method::POST,
            "/srv/pv",
            &[
                ("user-agent", BROWSER),
                ("origin", "http://test.localhost"),
                ("content-type", "application/json"),
            ],
            r#"{"url":"http://test.localhost/latest","session_id":"s1"}"#,
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    request_tracking::drain().await;
    assert_eq!(request_tracking::queued_count(&a.st.site.pageview_queue), 1);
    assert_eq!(request_tracking::queued_count(&b.st.site.pageview_queue), 0);
    discourse_rs::jobs::run_scheduled(
        &b.st,
        discourse_rs::jobs::rollups::FLUSH_BROWSER_PAGEVIEW_EVENTS,
    )
    .await
    .unwrap();
    assert_eq!(request_tracking::queued_count(&a.st.site.pageview_queue), 1);
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM browser_pageview_events")
        .fetch_one(&b.db.pool)
        .await
        .unwrap();
    assert_eq!(events, 0);
}

async fn last_seen(site: &Site) -> Option<chrono::NaiveDateTime> {
    sqlx::query_scalar("SELECT last_seen_at FROM users WHERE id = 3")
        .fetch_one(&site.db.pool)
        .await
        .unwrap()
}

/// `should_update_last_seen?`'s gate is the site's: user 3 seen on one
/// site is still recorded on the other within the minute.
#[tokio::test]
async fn the_last_seen_gate_is_per_site() {
    let (a, b) = two_sites(|| config(RailsEnv::Test, &[])).await;
    let mut on_a = Client::new(&a, "127.0.0.1");
    let mut on_b = Client::new(&b, "127.0.0.1");
    on_a.login("user1").await;
    on_b.login("user1").await;
    let before = (last_seen(&a).await, last_seen(&b).await);
    on_a.get("/latest").await;
    assert!(last_seen(&a).await > before.0);
    on_b.get("/latest").await;
    assert!(last_seen(&b).await > before.1);
}

fn limited() -> Config {
    let mut cfg = config(
        RailsEnv::Test,
        &[
            ("max_reqs_per_ip_per_10_seconds", "3"),
            ("max_reqs_per_ip_per_minute", "1000"),
        ],
    );
    cfg.rate_limits = true;
    cfg
}

/// The per-address limits are `GLOBAL::` (one budget for every site, as
/// Rails keeps them outside the site's namespace).
#[tokio::test]
async fn an_addresss_budget_spans_the_sites() {
    let (a, b) = two_sites(limited).await;
    for site in [&a, &b, &a] {
        let reply = Client::new(site, "203.0.113.9")
            .get("/session/csrf.json")
            .await;
        assert_eq!(reply.status, StatusCode::OK);
    }
    let reply = Client::new(&b, "203.0.113.9")
        .get("/session/csrf.json")
        .await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        reply.headers["discourse-rate-limit-error-code"],
        "ip_10_secs_limit"
    );
}

/// A signed-in user's limits, and a crawler's, are the site's: user 3
/// and GPTBot on one site do not count on the other.
#[tokio::test]
async fn user_and_crawler_limits_are_per_site() {
    let (a, b) = two_sites(limited).await;
    let mut on_a = Client::new(&a, "127.0.0.1");
    let mut on_b = Client::new(&b, "127.0.0.1");
    on_a.login("user1").await;
    on_b.login("user1").await;
    for (i, on) in ["a", "a", "a", "b", "b"].into_iter().enumerate() {
        let client = if on == "a" { &mut on_a } else { &mut on_b };
        client.from = format!("203.0.113.{}", 20 + i);
        let reply = client.get("/session/current.json").await;
        assert_eq!(reply.status, StatusCode::OK, "request {i}: {}", reply.body);
    }
    on_a.from = "203.0.113.30".into();
    let refused = on_a.get("/session/current.json").await;
    assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        refused.headers["discourse-rate-limit-error-code"],
        "user_10_secs_limit"
    );

    let crawl = |site| async move {
        Client::new(site, "127.0.0.1")
            .send(
                Method::GET,
                "/session/csrf.json",
                &[("user-agent", "GPTBot/1.1")],
                "",
            )
            .await
            .status
    };
    assert_eq!(crawl(&a).await, StatusCode::OK);
    assert_eq!(crawl(&b).await, StatusCode::OK);
    assert_eq!(crawl(&a).await, StatusCode::TOO_MANY_REQUESTS);
}
