//! Two sites in one process (RailsMultisite): each site's AppState over its
//! own cloned database, the process-wide parts shared as main.rs shares
//! them, every request through the dispatcher by its Host. Every test
//! asserts that what one site does stays out of the other, except for
//! what Rails shares between sites (the `GLOBAL::` rate limits).

mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, other_site, state};
use discourse_rs::AppState;
use discourse_rs::config::{Config, RailsEnv, SiteConfig};
use discourse_rs::multisite::{Dispatcher, Sites};
use discourse_rs::request_tracking;
use discourse_rs::site_settings::SiteSettings;
use http_body_util::BodyExt;
use tower::ServiceExt;

const BROWSER: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

struct Site {
    db: TestDb,
    st: AppState,
    /// The Host its requests carry.
    host: &'static str,
    dispatcher: Dispatcher,
}

/// The second site's host names.
const B_HOST: &str = "b.example";
const B_OTHER_HOST: &str = "b2.example";

/// Two sites behind one dispatcher, each with its config from `make`:
/// `a` the default (test.localhost), `b` the multisite.yml entry `second`
/// (b.example and b2.example).
async fn two_sites(make: impl Fn() -> Config) -> (Site, Site) {
    let db_a = TestDb::new().await;
    let db_b = TestDb::new().await;
    let mut config_a = make();
    config_a.site.database_url = db_a.url();
    config_a.site.multisite = true;
    let mut config_b = make();
    config_b.site = SiteConfig {
        db: "second".into(),
        database_url: db_b.url(),
        host_names: vec![B_HOST.into(), B_OTHER_HOST.into()],
        pool: 4,
        multisite: true,
    };
    let a = state(db_a.pool.clone(), config_a).await;
    let b = other_site(&a, db_b.pool.clone(), config_b).await;
    let dispatcher = Sites::multisite(vec![a.clone(), b.clone()], &[])
        .unwrap()
        .into_service();
    (
        Site {
            db: db_a,
            st: a,
            host: "test.localhost",
            dispatcher: dispatcher.clone(),
        },
        Site {
            db: db_b,
            st: b,
            host: B_HOST,
            dispatcher,
        },
    )
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
        let mut request = Request::builder().method(method).uri(path);
        if !headers.iter().any(|(k, _)| *k == "host") {
            request = request.header(header::HOST, self.site.host);
        }
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
        let response = self.site.dispatcher.clone().oneshot(request).await.unwrap();
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

async fn set_title(site: &Site, title: &str) {
    common::set_setting(&site.db.pool, "title", 1, title).await;
}

fn json(reply: &Reply) -> serde_json::Value {
    serde_json::from_str(&reply.body).unwrap_or_else(|_| panic!("not JSON: {}", reply.body))
}

/// The Host picks the site; a port in it is not part of the name.
#[tokio::test]
async fn the_host_picks_the_site() {
    let (a, b) = two_sites(|| config(RailsEnv::Test, &[])).await;
    set_title(&a, "Forum A").await;
    set_title(&b, "Forum B").await;
    let reply = Client::new(&a, "127.0.0.1")
        .get("/site/basic-info.json")
        .await;
    assert_eq!(json(&reply)["title"], "Forum A");
    let reply = Client::new(&b, "127.0.0.1")
        .get("/site/basic-info.json")
        .await;
    assert_eq!(json(&reply)["title"], "Forum B");
    let reply = Client::new(&b, "127.0.0.1")
        .send(
            Method::GET,
            "/site/basic-info.json",
            &[("host", "b.example:8443")],
            "",
        )
        .await;
    assert_eq!(json(&reply)["title"], "Forum B");
}

/// RailsMultisite::Middleware: a host no site has is a bare 404 `not
/// found`, except /srv/status, which the default site answers.
#[tokio::test]
async fn an_unknown_host_is_not_found_but_the_status_check_answers() {
    let (a, _b) = two_sites(|| config(RailsEnv::Test, &[])).await;
    let mut client = Client::new(&a, "127.0.0.1");
    let reply = client
        .send(
            Method::GET,
            "/latest.json",
            &[("host", "nowhere.example")],
            "",
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.body, "not found");
    assert!(reply.headers.get(header::CONTENT_TYPE).is_none());
    let reply = client
        .send(
            Method::GET,
            "/srv/status",
            &[("host", "nowhere.example")],
            "",
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, "ok");
}

/// A request to an asset host names its site in `__ws`, and its cookies
/// are dropped (ConnectionManagement#host).
#[tokio::test]
async fn an_asset_host_request_names_its_site_in_ws() {
    let (a, b) = two_sites(|| config(RailsEnv::Test, &[])).await;
    set_title(&b, "Forum B").await;
    let mut on_b = Client::new(&b, "127.0.0.1");
    on_b.login("user1").await;
    assert_eq!(
        on_b.get("/session/current.json").await.status,
        StatusCode::OK
    );
    // The default site's host names are the asset hosts without a CDN.
    let reply = Client::new(&a, "127.0.0.1")
        .get("/site/basic-info.json?__ws=b.example")
        .await;
    assert_eq!(json(&reply)["title"], "Forum B");
    let reply = on_b
        .send(
            Method::GET,
            "/session/current.json?__ws=b.example",
            &[("host", a.host)],
            "",
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND, "cookies dropped");
    // `__ws` is only honoured on an asset host.
    let reply = Client::new(&b, "127.0.0.1")
        .get("/site/basic-info.json?__ws=test.localhost")
        .await;
    assert_eq!(json(&reply)["title"], "Forum B");
}

/// /highlight-js/:hostname/ (and /user_avatar/:hostname/, /svg-sprite/)
/// are the named site's (`with_hostname`), the default's when no site has
/// the name.
#[tokio::test]
async fn a_path_hostname_picks_the_site() {
    let (a, _b) = two_sites(|| config(RailsEnv::Test, &[])).await;
    let location = |reply: &Reply| {
        reply.headers[header::LOCATION]
            .to_str()
            .unwrap()
            .to_string()
    };
    let mut client = Client::new(&a, "127.0.0.1");
    let reply = client.get("/highlight-js/b.example/0.js").await;
    assert_eq!(reply.status, StatusCode::FOUND);
    assert!(
        location(&reply).contains("/highlight-js/b.example/"),
        "{}",
        location(&reply)
    );
    let reply = client.get("/highlight-js/nowhere.example/0.js").await;
    assert!(
        location(&reply).contains("/highlight-js/test.localhost/"),
        "{}",
        location(&reply)
    );
    let reply = client
        .get("/svg-sprite/b.example/svg-1-0000000000000000000000000000000000000000.js")
        .await;
    assert_eq!(reply.status, StatusCode::FOUND);
    assert!(
        location(&reply).contains("/svg-sprite/b.example/svg-1-"),
        "{}",
        location(&reply)
    );
    // The current version is served, cached for a year.
    let current = location(&reply);
    let path = &current[current.find("/svg-sprite/").unwrap()..];
    let reply = client.get(path).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.headers[header::CACHE_CONTROL],
        "max-age=31556952, public, immutable"
    );
    assert!(
        reply.body.starts_with("window.__svg_sprite = \"")
            && reply.body.contains("<symbol id=\\\"bars\\\""),
        "{}",
        &reply.body[..80]
    );
    // Pages load it rather than inline it.
    let page = client.get("/latest").await;
    assert!(
        page.body
            .contains("<script src=\"/svg-sprite/test.localhost/svg-"),
        "no sprite script"
    );
    assert!(!page.body.contains("<symbol id=\"bars\""), "sprite inlined");
    // The user_avatar route's shape is checked in multisite::tests.
}

/// Each site keeps its own sprite: a badge icon on one site is in its
/// sprite only, and the other's version stays.
#[tokio::test]
async fn each_site_has_its_own_sprite() {
    let (a, b) = two_sites(|| config(RailsEnv::Test, &[])).await;
    let mut client_a = Client::new(&a, "127.0.0.1");
    let mut client_b = Client::new(&b, "127.0.0.1");
    let path = |page: &str| {
        let start = page.find("/svg-sprite/").expect("sprite script");
        page[start..start + page[start..].find('"').unwrap()].to_string()
    };
    let a_before = path(&client_a.get("/latest?a=1").await.body);
    let b_before = path(&client_b.get("/latest?b=1").await.body);
    sqlx::query("UPDATE badges SET icon = 'otter', updated_at = now() WHERE id = 1")
        .execute(&b.db.pool)
        .await
        .unwrap();
    let a_after = path(&client_a.get("/latest?a=2").await.body);
    let b_after = path(&client_b.get("/latest?b=2").await.body);
    assert_eq!(a_before, a_after);
    assert_ne!(b_before, b_after);
    let sprite = client_b.get(&b_after).await;
    assert!(sprite.body.contains("<symbol id=\\\"otter\\\""));
    let sprite = client_a.get(&a_after).await;
    assert!(!sprite.body.contains("<symbol id=\\\"otter\\\""));
}

/// A session is its site's: user 3's cookie from one site is anonymous on
/// the other (its token is in the other site's database only).
#[tokio::test]
async fn a_session_stays_on_its_site() {
    let (a, b) = two_sites(|| config(RailsEnv::Test, &[])).await;
    let mut on_a = Client::new(&a, "127.0.0.1");
    on_a.login("user1").await;
    assert_eq!(
        on_a.get("/session/current.json").await.status,
        StatusCode::OK
    );
    let mut on_b = Client::new(&b, "127.0.0.1");
    on_b.cookies = on_a.cookies.clone();
    assert_eq!(
        on_b.get("/session/current.json").await.status,
        StatusCode::NOT_FOUND
    );
}

/// The anonymous cache is the site's: what one site stored is not served
/// to the other.
#[tokio::test]
async fn the_anonymous_cache_is_per_site() {
    let (a, b) = two_sites(|| config(RailsEnv::Test, &[])).await;
    let cached = |reply: &Reply| {
        reply
            .headers
            .get("x-discourse-cached")
            .map(|v| v.to_str().unwrap().to_string())
    };
    for expected in ["skip", "store", "true"] {
        let reply = Client::new(&a, "127.0.0.1").get("/latest.json").await;
        assert_eq!(cached(&reply).as_deref(), Some(expected));
    }
    let reply = Client::new(&b, "127.0.0.1").get("/latest.json").await;
    assert_eq!(cached(&reply).as_deref(), Some("skip"));
}

/// Live updates are the site's: a message published on one site's bus is
/// not in the other's backlog.
#[tokio::test]
async fn bus_messages_stay_on_their_site() {
    let (a, b) = two_sites(|| config(RailsEnv::Test, &[])).await;
    let from_a = a.st.bus.now().await.unwrap();
    let from_b = b.st.bus.now().await.unwrap();
    let mut tx = a.db.pool.begin().await.unwrap();
    a.st.bus
        .publish(&mut tx, "/latest", &serde_json::json!({"site": "a"}), None)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        common::bus_messages(&a.st, from_a, &["/latest"])
            .await
            .len(),
        1
    );
    assert!(
        common::bus_messages(&b.st, from_b, &["/latest"])
            .await
            .is_empty()
    );
}

/// A job is its site's: queued on one site, the other's worker leaves it.
#[tokio::test]
async fn jobs_run_on_their_own_site() {
    let (a, b) = two_sites(|| config(RailsEnv::Test, &[])).await;
    let mut conn = a.db.pool.acquire().await.unwrap();
    discourse_rs::jobs::enqueue(
        &mut conn,
        "update_gravatar",
        serde_json::json!({"user_id": 3}),
    )
    .await
    .unwrap();
    drop(conn);
    assert_eq!(discourse_rs::jobs::drain(&b.st).await.unwrap(), 0);
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM discourse_rs.jobs")
        .fetch_one(&a.db.pool)
        .await
        .unwrap();
    assert_eq!(queued, 1);
}

/// RailsMultisite::CookieSalt: the cookie key's salt is
/// `"authenticated encrypted cookie <host>"`, so a session cookie works
/// only on the host name it was set for, even another name of its site.
#[tokio::test]
async fn a_cookie_is_keyed_to_its_host() {
    use discourse_rs::session::cookie::{Codec, SALT, derive_key, derive_key_with_salt};
    let (_a, b) = two_sites(|| config(RailsEnv::Test, &[])).await;
    let mut on_b = Client::new(&b, "127.0.0.1");
    on_b.login("user1").await;
    assert_eq!(
        on_b.get("/session/current.json").await.status,
        StatusCode::OK
    );
    let token = on_b
        .cookies
        .iter()
        .find(|(n, _)| n == "_t")
        .map(|(_, v)| v.clone())
        .unwrap();
    let secret = &b.st.keys.secret_key_base;
    assert_eq!(
        derive_key_with_salt(secret, &format!("{SALT} b.example")),
        derive_key_with_salt(secret, "authenticated encrypted cookie b.example")
    );
    assert_ne!(
        derive_key_with_salt(secret, &format!("{SALT} b.example")),
        derive_key(secret)
    );
    assert!(
        Codec::for_host(secret, Some(B_HOST))
            .decrypt("_t", &token)
            .is_ok()
    );
    assert!(Codec::new(secret).decrypt("_t", &token).is_err());
    assert!(
        Codec::for_host(secret, Some(B_OTHER_HOST))
            .decrypt("_t", &token)
            .is_err()
    );
    let reply = on_b
        .send(
            Method::GET,
            "/session/current.json",
            &[("host", B_OTHER_HOST)],
            "",
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}

/// Scratch directories for a test's PUBLIC_DIR and TMP_DIR, shared by
/// both sites as multisite shares them.
fn scratch(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let base = std::env::temp_dir().join(format!(
        "discourse-rs-multisite-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    (base.join("public"), base.join("tmp"))
}

/// A site's uploads live at `uploads/<db>` (FileStore::BaseStore's
/// upload_path), its URLs say so, and only those count as its own.
#[tokio::test]
async fn uploads_live_under_the_sites_db_key() {
    let (a, b) = two_sites(|| config(RailsEnv::Test, &[])).await;
    for (site, db) in [(&a, "default"), (&b, "second")] {
        let store =
            discourse_rs::file_store::FileStore::for_site(&site.st.config, &settings(site).await)
                .unwrap();
        assert_eq!(store.db(), db);
        assert_eq!(store.relative_base_url(), format!("/uploads/{db}"));
        assert_eq!(
            store.path_for_kind("original", 7, &"a".repeat(40), ".png"),
            format!("/uploads/{db}/original/1X/{}.png", "a".repeat(40))
        );
    }
}

/// A site's backup is its own: `backups/<db>`, its uploads from
/// `uploads/<db>`, `db_name` and `multisite` in its metadata; restored
/// on another site, its upload URLs are remapped to that site's db key.
#[tokio::test]
async fn backups_are_per_site_and_restore_across_sites() {
    use discourse_rs::backup_restore::{creator, restorer};
    let (public, tmp) = scratch("backups");
    let make = || {
        let mut cfg = common::recorded_config();
        cfg.public_dir = public.clone();
        cfg.tmp_dir = tmp.clone();
        cfg
    };
    let (a, b) = two_sites(make).await;
    for db in ["default", "second"] {
        let dir = public.join("uploads").join(db).join("original/1X");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{db}.png")), db).unwrap();
    }
    let sha1 = "b".repeat(40);
    sqlx::query(
        "INSERT INTO uploads (user_id, original_filename, filesize, width, height, url, sha1, \
         extension, created_at, updated_at) \
         VALUES (-1, 'second.png', 6, 1, 1, '/uploads/second/original/1X/second.png', $1, 'png', now(), now())",
    )
    .bind(&sha1)
    .execute(&b.db.pool)
    .await
    .unwrap();

    let backup = creator::run(
        &b.st,
        1,
        creator::Options {
            with_uploads: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(backup.success, "{:#?}", backup.logs);
    let archive = public.join("backups/second").join(&backup.filename);
    let listing = std::process::Command::new("tar")
        .arg("-tzf")
        .arg(&archive)
        .output()
        .unwrap();
    let entries = String::from_utf8_lossy(&listing.stdout).into_owned();
    assert!(entries.contains("uploads/second/original/1X/second.png"));
    assert!(!entries.contains("uploads/default"));
    let dump = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "tar -xzOf '{}' dump.sql.gz | gzip -dc",
            archive.display()
        ))
        .output()
        .unwrap();
    let sql = String::from_utf8_lossy(&dump.stdout);
    assert!(sql.contains("\tdb_name\tsecond\n"));
    assert!(sql.contains("\tmultisite\tt\n"));
    assert!(
        !public
            .join("backups/default")
            .join(&backup.filename)
            .exists()
    );

    // The default site restores the second's backup.
    std::fs::create_dir_all(public.join("backups/default")).unwrap();
    std::fs::copy(
        &archive,
        public.join("backups/default").join(&backup.filename),
    )
    .unwrap();
    let outcome = restorer::run(
        &a.st,
        1,
        restorer::Options {
            filename: Some(backup.filename.clone()),
            client_id: None,
            disable_emails: true,
        },
    )
    .await
    .unwrap();
    assert!(outcome.success, "{:#?}", outcome.logs);
    let url: String = sqlx::query_scalar("SELECT url FROM uploads WHERE sha1 = $1")
        .bind(&sha1)
        .fetch_one(&a.db.pool)
        .await
        .unwrap();
    assert_eq!(url, "/uploads/default/original/1X/second.png");
    assert!(
        public
            .join("uploads/default/original/1X/second.png")
            .exists()
    );
    assert!(
        public
            .join("uploads/second/original/1X/second.png")
            .exists()
    );
    let _ = std::fs::remove_dir_all(public.parent().unwrap());
}

async fn last_job_performed(site: &Site) -> Option<String> {
    let mut conn = site.db.pool.acquire().await.unwrap();
    discourse_rs::owned_schema::cached_get(
        &mut conn,
        discourse_rs::problem_checks::LAST_JOB_PERFORMED_KEY,
    )
    .await
    .unwrap()
}

/// Each site's worker keeps its own scheduled state: the first site's
/// runs its scheduled jobs at once, the next waits out its stagger, and
/// what one runs is recorded in its own database only.
#[tokio::test]
async fn scheduled_jobs_run_per_site_worker() {
    let (a, b) = two_sites(|| config(RailsEnv::Test, &[])).await;
    let (stop_a, stopped_a) = tokio::sync::oneshot::channel::<()>();
    let (stop_b, stopped_b) = tokio::sync::oneshot::channel::<()>();
    let worker_a = tokio::spawn(discourse_rs::jobs::work(
        a.st.clone(),
        async {
            stopped_a.await.ok();
        },
        std::time::Duration::ZERO,
    ));
    let worker_b = tokio::spawn(discourse_rs::jobs::work(
        b.st.clone(),
        async {
            stopped_b.await.ok();
        },
        std::time::Duration::from_secs(3600),
    ));
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    while last_job_performed(&a).await.is_none() {
        assert!(tokio::time::Instant::now() < deadline, "a ran nothing");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(last_job_performed(&b).await, None);
    stop_a.send(()).ok();
    stop_b.send(()).ok();
    worker_a.await.unwrap();
    worker_b.await.unwrap();
    assert_eq!(last_job_performed(&b).await, None);
}

/// A request straight to a dispatcher: (status, body).
async fn dispatch(dispatcher: &Dispatcher, host: &str, path: &str) -> (StatusCode, String) {
    let request = Request::builder()
        .uri(path)
        .header(header::HOST, host)
        .body(Body::empty())
        .unwrap();
    let response = dispatcher.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Startup as main.rs runs it: a site that fails to start (here its
/// database does not exist) is logged and skipped, its hosts answer 503,
/// and the default site serves.
#[tokio::test]
async fn a_site_that_fails_to_start_answers_503_and_the_others_serve() {
    use discourse_rs::multisite::{Shared, SiteSpec, boot_all};
    use sqlx::postgres::PgConnectOptions;
    let db = TestDb::new().await;
    let (public, tmp) = scratch("boot");
    let mut process = config(RailsEnv::Test, &[]);
    process.site.database_url = db.url();
    process.site.pool = 2;
    process.public_dir = public.clone();
    process.tmp_dir = tmp;
    let shared = Shared {
        site_setting_defs: std::sync::Arc::new(
            discourse_rs::site_settings::Definitions::vendored().unwrap(),
        ),
        i18n: std::sync::Arc::new(discourse_rs::i18n::I18n::vendored().unwrap()),
        keys: std::sync::Arc::new(discourse_rs::session::current::Keys::ephemeral()),
        mailer: discourse_rs::email::Mailer::memory(),
        global_rate_limits: Default::default(),
    };
    let server: PgConnectOptions = db.url().parse().unwrap();
    let missing = SiteSpec {
        db: "gone".into(),
        database: "discourse_rs_208d_no_such_database".into(),
        host: server
            .get_socket()
            .map(|s| s.display().to_string())
            .or_else(|| Some(server.get_host().to_string())),
        port: Some(server.get_port()),
        username: None,
        password: None,
        pool: Some(1),
        host_names: vec!["gone.example".into()],
    };
    let sites = boot_all(&shared, process, Some(vec![missing]))
        .await
        .unwrap();
    assert_eq!(sites.states().count(), 1);
    let dispatcher = sites.into_service();
    assert_eq!(
        dispatch(&dispatcher, "gone.example", "/latest.json").await,
        (StatusCode::SERVICE_UNAVAILABLE, "site unavailable".into())
    );
    let (status, _) = dispatch(&dispatcher, "test.localhost", "/about.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        dispatch(&dispatcher, "nowhere.example", "/srv/status").await,
        (StatusCode::OK, "ok".into())
    );
    let _ = std::fs::remove_dir_all(public.parent().unwrap());
}

/// The startup check: each site may hold its pool, the rate limiter's 4
/// and pg-bus's listener; more than the server's max_connections warns.
#[tokio::test]
async fn the_connection_budget_warns_past_max_connections() {
    let (a, b) = two_sites(|| config(RailsEnv::Test, &[])).await;
    assert!(
        discourse_rs::multisite::connection_budget(&[a.st.clone(), b.st.clone()])
            .await
            .is_empty()
    );
    let max: String = sqlx::query_scalar("SHOW max_connections")
        .fetch_one(&a.db.pool)
        .await
        .unwrap();
    let max: u32 = max.parse().unwrap();
    let with_pool = |site: &Site| {
        let mut st = site.st.clone();
        let mut config = (*st.config).clone();
        config.site.pool = max / 2;
        st.config = std::sync::Arc::new(config);
        st
    };
    let warnings =
        discourse_rs::multisite::connection_budget(&[with_pool(&a), with_pool(&b)]).await;
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains(&format!("(default, second) may hold {}", 2 * (max / 2 + 5))),
        "{}",
        warnings[0]
    );
}

/// A site's pool keeps ActiveRecord's 1000 prepared statements per
/// connection: more than a topic page runs, which sqlx's default of 100
/// made re-plan on every request.
#[tokio::test]
async fn site_pools_keep_a_thousand_prepared_statements() {
    let db = TestDb::new().await;
    let pool = discourse_rs::multisite::connect_pool(&db.url(), 1)
        .await
        .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    for i in 0..150 {
        let _: i32 = sqlx::query_scalar(&format!("SELECT {i}::int"))
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    }
    let prepared: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_prepared_statements")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert!(prepared >= 150, "{prepared} prepared statements kept");
    drop(conn);
    pool.close().await;
}
