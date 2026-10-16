//! RailsMultisite: several forums from one process, by hostname. Each site
//! is an `AppState` and a `Router` built by the same startup (`boot`); the
//! `Dispatcher` picks the site per request as RailsMultisite::Middleware
//! does and hands the request to that site's router. Without a
//! multisite.yml there is one site, served whatever the host.
//!
//! multisite.yml (`DISCOURSE_MULTISITE_CONFIG_PATH`) has Rails' shape: a
//! db key per site, each with `database`, `host`, `port`, `username`,
//! `password`, `pool` and `host_names`. The key `default` is refused: the
//! default site is the environment's (`DATABASE_URL`,
//! `DISCOURSE_HOSTNAME`). See docs/multisite.md.

use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::error::Error;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use futures_util::future::{Either, Ready, ready};
use serde_yaml_ng::Value as Yaml;
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tower::util::Oneshot;
use tower::{Service, ServiceExt};

use crate::AppState;
use crate::config::{Config, SiteConfig};
use crate::email::Mailer;
use crate::i18n::I18n;
use crate::rate_limiter::MemoryStore;
use crate::session::current::Keys;
use crate::site_settings::{Definitions, SiteSettings};

/// `RailsMultisite::ConnectionManagement::DEFAULT`: the environment's site.
pub const DEFAULT: &str = "default";

/// One entry of multisite.yml.
#[derive(Debug, Clone, PartialEq)]
pub struct SiteSpec {
    pub db: String,
    pub database: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub pool: Option<u32>,
    pub host_names: Vec<String>,
}

/// multisite.yml's sites, in file order. An empty file is multisite with
/// the default site alone.
pub fn parse(src: &str) -> Result<Vec<SiteSpec>, String> {
    let root: Yaml = serde_yaml_ng::from_str(src).map_err(|e| format!("multisite.yml: {e}"))?;
    if root.is_null() {
        return Ok(Vec::new());
    }
    let sites = root
        .as_mapping()
        .ok_or("multisite.yml: expected a mapping of db keys")?;
    let mut specs: Vec<SiteSpec> = Vec::new();
    for (key, entry) in sites {
        let db = scalar(key).ok_or("multisite.yml: a db key that is not a string")?;
        if db == DEFAULT {
            return Err("multisite.yml: Please do not name any db default!".into());
        }
        let entry = entry
            .as_mapping()
            .ok_or_else(|| format!("multisite.yml: {db}: expected a mapping"))?;
        let field = |name: &str| -> Result<Option<String>, String> {
            match entry.get(Yaml::String(name.into())) {
                None | Some(Yaml::Null) => Ok(None),
                Some(v) => scalar(v)
                    .map(Some)
                    .ok_or_else(|| format!("multisite.yml: {db}: {name} is not a scalar")),
            }
        };
        if let Some(adapter) = field("adapter")?
            && !matches!(adapter.as_str(), "postgresql" | "postgis")
        {
            return Err(format!(
                "multisite.yml: {db}: adapter {adapter} is not postgresql"
            ));
        }
        let database = field("database")?
            .ok_or_else(|| format!("multisite.yml: {db}: database is missing"))?;
        let number = |name: &str| -> Result<Option<u64>, String> {
            field(name)?
                .map(|v| {
                    v.parse()
                        .map_err(|_| format!("multisite.yml: {db}: {name} {v:?} is not a number"))
                })
                .transpose()
        };
        let port = number("port")?
            .map(|p| u16::try_from(p).map_err(|_| format!("multisite.yml: {db}: port {p}")))
            .transpose()?;
        let pool = number("pool")?
            .map(|p| u32::try_from(p).map_err(|_| format!("multisite.yml: {db}: pool {p}")))
            .transpose()?;
        let host_names: Vec<String> = match entry.get(Yaml::String("host_names".into())) {
            Some(Yaml::Sequence(names)) => names
                .iter()
                .map(|n| {
                    scalar(n)
                        .ok_or_else(|| format!("multisite.yml: {db}: a host name not a string"))
                })
                .collect::<Result<_, _>>()?,
            _ => Vec::new(),
        };
        if host_names.is_empty() {
            return Err(format!("multisite.yml: {db}: host_names is missing"));
        }
        let (host, username, password) = (field("host")?, field("username")?, field("password")?);
        specs.push(SiteSpec {
            db,
            database,
            host,
            port,
            username,
            password,
            pool,
            host_names,
        });
    }
    Ok(specs)
}

fn scalar(v: &Yaml) -> Option<String> {
    match v {
        Yaml::String(s) => Some(s.clone()),
        Yaml::Number(n) => Some(n.to_string()),
        Yaml::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Percent-encodes all but RFC 3986's unreserved characters.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

impl SiteSpec {
    /// The entry as a connection URL; a `host` that is a directory is a
    /// Unix socket's, as libpq takes it. What the entry leaves out falls to
    /// libpq's environment defaults (PGHOST, PGUSER, ...).
    pub fn database_url(&self) -> String {
        let mut url = String::from("postgresql://");
        if let Some(user) = &self.username {
            url.push_str(&encode(user));
            if let Some(password) = &self.password {
                url.push(':');
                url.push_str(&encode(password));
            }
            url.push('@');
        }
        let socket = self.host.as_deref().filter(|h| h.starts_with('/'));
        match self.host.as_deref() {
            Some(host) if socket.is_none() && host.contains(':') => {
                url.push_str(&format!("[{host}]"))
            }
            Some(host) if socket.is_none() => url.push_str(host),
            _ => url.push_str("localhost"),
        }
        if let Some(port) = self.port {
            url.push_str(&format!(":{port}"));
        }
        url.push('/');
        url.push_str(&encode(&self.database));
        if let Some(socket) = socket {
            url.push_str("?host=");
            url.push_str(&encode(socket));
        }
        url
    }

    /// The site's Config: the process's with this site's database, host
    /// names, pool and db key.
    pub fn config(&self, process: &Config) -> Config {
        Config {
            site: SiteConfig {
                db: self.db.clone(),
                database_url: self.database_url(),
                host_names: self.host_names.clone(),
                pool: self.pool.unwrap_or(process.site.pool),
                multisite: true,
            },
            ..process.clone()
        }
    }
}

/// Where each site's data lives, for the rule that no two sites share a
/// database: host, socket, port and database name.
fn database_identity(url: &str) -> Result<String, String> {
    let options: PgConnectOptions = url
        .parse()
        .map_err(|e| format!("multisite: a database URL that does not parse: {e}"))?;
    Ok(format!(
        "{}|{:?}|{}|{}",
        options.get_host(),
        options.get_socket(),
        options.get_port(),
        options.get_database().unwrap_or_default()
    ))
}

/// Refuses two sites on one database, or one host name on two sites.
pub fn check_distinct(configs: &[Config]) -> Result<(), String> {
    let mut databases: HashMap<String, &str> = HashMap::new();
    let mut hosts: HashMap<&str, &str> = HashMap::new();
    for config in configs {
        let db = config.site.db.as_str();
        if let Some(other) = databases.insert(database_identity(&config.site.database_url)?, db) {
            return Err(format!(
                "multisite: sites {other} and {db} share a database"
            ));
        }
        for host in config.hostnames() {
            if let Some(other) = hosts.insert(host, db) {
                return Err(format!(
                    "multisite: sites {other} and {db} both name the host {host}"
                ));
            }
        }
    }
    Ok(())
}

/// What every site of the process shares: the parsed site setting
/// definitions and strings (each site caches its own resolution and
/// overrides over them), secret_key_base, the mailer and the `GLOBAL::`
/// rate limit lists.
#[derive(Clone)]
pub struct Shared {
    pub site_setting_defs: Arc<Definitions>,
    pub i18n: Arc<I18n>,
    pub keys: Arc<Keys>,
    pub mailer: Mailer,
    pub global_rate_limits: MemoryStore,
}

impl Shared {
    /// What `state`'s process shares, for another site beside it.
    pub fn of(state: &AppState) -> Shared {
        Shared {
            site_setting_defs: state.site_setting_defs.clone(),
            i18n: state.i18n.clone(),
            keys: state.keys.clone(),
            mailer: state.mailer.clone(),
            global_rate_limits: state.global_rate_limits.clone(),
        }
    }

    /// A site's state over its own pool and bus: its own caches, lists and
    /// in-process state.
    pub fn site(
        &self,
        pool: PgPool,
        bus: pg_bus::Bus,
        config: Config,
        rate_limits: MemoryStore,
    ) -> AppState {
        AppState {
            anonymous_cache: Arc::new(crate::anonymous_cache::Cache::new(&config)),
            config: Arc::new(config),
            site_setting_defs: Arc::new(self.site_setting_defs.for_site()),
            i18n: Arc::new(self.i18n.for_site()),
            search_log_cache: Default::default(),
            keys: self.keys.clone(),
            mailer: self.mailer.clone(),
            bus,
            rate_limits,
            global_rate_limits: self.global_rate_limits.clone(),
            site: Default::default(),
            pool,
        }
    }
}

/// A site's startup: its pool, the schema verified, the port's own schema,
/// pg-bus, the request tracker's lists, the site icons' optimized copies
/// (SiteIconManager.ensure_optimized! at boot). The default site also
/// reads back the process's `GLOBAL::` lists, which are saved to its
/// database.
pub async fn boot(shared: &Shared, config: Config) -> Result<AppState, Box<dyn Error>> {
    let db = config.site.db.clone();
    // A request waits up to 5 s for a connection, ActiveRecord's
    // checkout_timeout, rather than sqlx's 30.
    let pool = PgPoolOptions::new()
        .max_connections(config.site.pool)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .idle_timeout(IDLE_TIMEOUT)
        .connect(&config.site.database_url)
        .await?;

    let report = crate::schema::verify(&mut *pool.acquire().await?).await?;
    tracing::info!(
        site = db,
        discourse = crate::schema::discourse_commit(),
        versions = report.expected,
        "schema verified"
    );
    if !report.extra.is_empty() {
        tracing::warn!(
            site = db,
            count = report.extra.len(),
            latest = report.extra.last().map(String::as_str),
            "database has schema versions newer than the vendored structure.sql"
        );
    }

    crate::owned_schema::migrate(&pool).await?;
    let bus = pg_bus::Bus::start(pool.clone(), crate::bus::config()).await?;
    let rate_limits = MemoryStore::load(&pool).await?;
    if db == DEFAULT {
        shared
            .global_rate_limits
            .extend(MemoryStore::load_global(&pool).await?);
    }
    let state = shared.site(pool, bus, config, rate_limits);

    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    match crate::site_icons::ensure_optimized(&mut conn, &settings, &state.config).await {
        Ok(0) => {}
        Ok(n) => tracing::info!(
            site = db,
            files = n,
            "made the site icons' optimized copies"
        ),
        Err(e) => tracing::error!(site = db, "site icons: {e}"),
    }
    drop(conn);
    Ok(state)
}

/// A site's shutdown: the tracker's lists kept for the next start (the
/// `GLOBAL::` ones in the default site's database) and the counts written.
/// Deferred work (`request_tracking::drain`) is drained once for the
/// process before.
pub async fn shut_down(state: &AppState) -> Result<(), Box<dyn Error>> {
    state.rate_limits.save(&state.pool).await?;
    if state.config.site.db == DEFAULT {
        state.global_rate_limits.save(&state.pool).await?;
    }
    if let Err(e) = crate::request_tracking::flush(state).await {
        tracing::warn!(
            site = state.config.site.db,
            "Unexpected error while processing cached counts: {e:?}"
        );
    }
    Ok(())
}

/// How long a pool keeps a connection nobody uses, so an idle site holds
/// few.
pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Every site started: the default (whose failure stops the process),
/// then each multisite.yml entry, one that fails logged and served as 503
/// (as Rails' safe_each_connection keeps going).
pub async fn boot_all(
    shared: &Shared,
    mut process: Config,
    specs: Option<Vec<SiteSpec>>,
) -> Result<Sites, Box<dyn Error>> {
    process.site.multisite = specs.is_some();
    let Some(specs) = specs else {
        return Ok(Sites::single(boot(shared, process).await?));
    };
    let mut configs = vec![process.clone()];
    configs.extend(specs.iter().map(|spec| spec.config(&process)));
    check_distinct(&configs)?;
    let mut configs = configs.into_iter();
    let default = configs.next().expect("the default site's config");
    let mut states = vec![boot(shared, default).await?];
    let mut down = Vec::new();
    for config in configs {
        let db = config.site.db.clone();
        match boot(shared, config.clone()).await {
            Ok(state) => states.push(state),
            Err(e) => {
                tracing::error!(
                    site = db,
                    "URGENT: Failed to initialize site {db}: {e}; its hosts answer 503"
                );
                down.push(config);
            }
        }
    }
    for warning in connection_budget(&states).await {
        tracing::warn!("{warning}");
    }
    Ok(Sites::multisite(states, &down)?)
}

/// What a site may hold at once: its pool, the rate limiter's own pool
/// and pg-bus's listener (a backup's pg_dump adds one while it runs).
pub fn site_connections(config: &Config) -> u32 {
    config.site.pool + crate::rate_limiter::OWN_POOL_SIZE + 1
}

/// The sites' connections against each Postgres server's
/// max_connections: a warning per server they could exhaust.
pub async fn connection_budget(states: &[AppState]) -> Vec<String> {
    let mut servers: Vec<(String, u32, Vec<&str>, &AppState)> = Vec::new();
    for state in states {
        let options = state.pool.connect_options();
        let server = match options.get_socket() {
            Some(socket) => format!("{}:{}", socket.display(), options.get_port()),
            None => format!("{}:{}", options.get_host(), options.get_port()),
        };
        let need = site_connections(&state.config);
        let db = state.config.site.db.as_str();
        match servers.iter_mut().find(|s| s.0 == server) {
            Some(s) => {
                s.1 += need;
                s.2.push(db);
            }
            None => servers.push((server, need, vec![db], state)),
        }
    }
    let mut warnings = Vec::new();
    for (server, need, sites, state) in servers {
        let max: Result<String, sqlx::Error> = sqlx::query_scalar("SHOW max_connections")
            .fetch_one(&state.pool)
            .await;
        match max.map(|m| m.parse::<u32>()) {
            Ok(Ok(max)) if need > max => warnings.push(format!(
                "multisite: the sites on {server} ({}) may hold {need} connections \
                 (each its pool, the rate limiter's {} and pg-bus's listener), more than \
                 its max_connections of {max}",
                sites.join(", "),
                crate::rate_limiter::OWN_POOL_SIZE,
            )),
            Ok(_) => {}
            Err(e) => warnings.push(format!(
                "multisite: max_connections of {server} not read: {e}"
            )),
        }
    }
    warnings
}

/// The process's sites and how a request finds its own.
pub struct Sites {
    /// Every site's state, the default first.
    states: Vec<AppState>,
    /// A router per host name: under multisite each host has its own
    /// cookie key (RailsMultisite::CookieSalt), so its own AppState.
    routers: Vec<Router>,
    /// host name -> router
    by_host: HashMap<String, usize>,
    /// The default site's router for its first host name.
    default_router: usize,
    /// The default site's router for /srv/status on an unknown host,
    /// whose cookie salt names no host.
    no_host_router: usize,
    /// Under multisite.yml: the Host picks the site, an unknown one is not
    /// found. Otherwise every request is the one site's.
    multisite: bool,
    /// `RailsMultisite::ConnectionManagement.asset_hostnames`: hosts whose
    /// requests name their site in `__ws`.
    asset_hostnames: HashSet<String>,
}

impl Sites {
    /// One site, whatever the host (no multisite.yml).
    pub fn single(state: AppState) -> Sites {
        Sites {
            routers: vec![crate::app(state.clone())],
            states: vec![state],
            by_host: HashMap::new(),
            default_router: 0,
            no_host_router: 0,
            multisite: false,
            asset_hostnames: HashSet::new(),
        }
    }

    /// Under multisite.yml: the default site first, then the others. The
    /// asset hostnames are `GlobalSetting.cdn_hostnames`, else the default
    /// site's host names (200-first_middlewares.rb). The sites in `down`
    /// failed to start: their hosts answer 503 until a restart brings them
    /// up.
    pub fn multisite(states: Vec<AppState>, down: &[Config]) -> Result<Sites, String> {
        let Some(default) = states.first() else {
            return Err("multisite: no default site".into());
        };
        if default.config.site.db != DEFAULT {
            return Err("multisite: the first site must be the default".into());
        }
        let configs: Vec<Config> = states
            .iter()
            .map(|s| (*s.config).clone())
            .chain(down.iter().cloned())
            .collect();
        check_distinct(&configs)?;
        let mut asset_hostnames = cdn_hostnames(&default.config);
        if asset_hostnames.is_empty() {
            asset_hostnames = default
                .config
                .hostnames()
                .into_iter()
                .map(str::to_string)
                .collect();
        }
        let for_host = |state: &AppState, host: Option<&str>| {
            let mut state = state.clone();
            state.keys = Arc::new(state.keys.for_host(host));
            crate::app(state)
        };
        let mut routers = vec![for_host(default, None)];
        let mut by_host = HashMap::new();
        for state in &states {
            for host in state.config.hostnames() {
                by_host.insert(host.to_string(), routers.len());
                routers.push(for_host(state, Some(host)));
            }
        }
        let unavailable = Router::new()
            .fallback(|| async { (StatusCode::SERVICE_UNAVAILABLE, "site unavailable") });
        for config in down {
            for host in config.hostnames() {
                by_host.insert(host.to_string(), routers.len());
                routers.push(unavailable.clone());
            }
        }
        Ok(Sites {
            default_router: by_host[default.config.hostname()],
            no_host_router: 0,
            routers,
            states,
            by_host,
            multisite: true,
            asset_hostnames,
        })
    }

    /// Every site's state, the default first.
    pub fn states(&self) -> impl Iterator<Item = &AppState> {
        self.states.iter()
    }

    pub fn into_service(self) -> Dispatcher {
        Dispatcher(Arc::new(self))
    }

    /// The router for a request, None for a host no site has.
    fn pick(&self, request: &mut Request<Body>) -> Option<usize> {
        if !self.multisite {
            return Some(self.default_router);
        }
        let mut host = request_host(request)?;
        // ConnectionManagement#host: a request to an asset host names its
        // site in `__ws`, and its cookies are not the site's.
        if self.asset_hostnames.contains(&host)
            && let Some(ws) = request.uri().query().and_then(|q| {
                form_urlencoded::parse(q.as_bytes())
                    .find(|(k, _)| k == "__ws")
                    .map(|(_, v)| v.into_owned())
            })
        {
            request.headers_mut().remove(header::COOKIE);
            host = ws;
        }
        let path = request.uri().path();
        match self.by_host.get(&host) {
            // `with_hostname(params[:hostname])`: the path's hostname picks
            // the site, the default when no site has it.
            Some(&i) => Some(match path_hostname(path) {
                Some(named) => self
                    .by_host
                    .get(named)
                    .copied()
                    .unwrap_or(self.default_router),
                None => i,
            }),
            // DiscoursePatches' db_lookup.
            None if path == "/srv/status" => Some(self.no_host_router),
            None => None,
        }
    }
}

/// `GlobalSetting.cdn_hostnames`: the CDN's host and the origin's.
fn cdn_hostnames(config: &Config) -> HashSet<String> {
    let globals = &config.globals;
    let cdn = globals.cdn_url().and_then(|url| {
        let rest = url.split_once("://").map_or(url, |(_, r)| r);
        let authority = rest.split(['/', '?', '#']).next()?;
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        Some(strip_port(host).to_string()).filter(|h| !h.is_empty())
    });
    cdn.into_iter()
        .chain(globals.get("cdn_origin_hostname").map(str::to_string))
        .collect()
}

/// `request.host`: the Host header (HTTP/2's authority) without the port.
fn request_host(request: &Request<Body>) -> Option<String> {
    let host = match request.headers().get(header::HOST) {
        Some(h) => h.to_str().ok()?,
        None => request.uri().authority()?.as_str(),
    };
    Some(strip_port(host).to_string())
}

fn strip_port(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    host.rsplit_once(':').map_or(host, |(h, _)| h)
}

/// The site a path names: `/highlight-js/:hostname/:version.js`,
/// `/svg-sprite/:hostname/...` and
/// `/user_avatar/:hostname/:username/:size/:version.png`.
fn path_hostname(path: &str) -> Option<&str> {
    let (prefix, rest) = path.strip_prefix('/')?.split_once('/')?;
    let segments = rest.split('/').count();
    let named = match prefix {
        "highlight-js" => segments == 2,
        "svg-sprite" => segments >= 2,
        "user_avatar" => segments == 4,
        _ => false,
    };
    named.then(|| rest.split('/').next()).flatten()
}

/// RailsMultisite::Middleware's answer for a host no site has.
fn not_found() -> Response {
    let mut response = Response::new(Body::from("not found"));
    *response.status_mut() = StatusCode::NOT_FOUND;
    response
}

/// The process's service: each request to its site's router.
#[derive(Clone)]
pub struct Dispatcher(Arc<Sites>);

impl Dispatcher {
    pub fn sites(&self) -> &Sites {
        &self.0
    }
}

impl Service<Request<Body>> for Dispatcher {
    type Response = Response;
    type Error = Infallible;
    type Future = Either<Oneshot<Router, Request<Body>>, Ready<Result<Response, Infallible>>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: Request<Body>) -> Self::Future {
        match self.0.pick(&mut request) {
            Some(i) => Either::Left(self.0.routers[i].clone().oneshot(request)),
            None => Either::Right(ready(Ok(not_found()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rails_shape() {
        let specs = parse(
            "mlp:\n  adapter: postgresql\n  database: discourse_mlp\n  username: discourse_mlp\n  \
             password: \"apple jack\"\n  host: dbhost\n  port: 5433\n  pool: 5\n  timeout: 5000\n  \
             host_names:\n    - discourse.equestria.com\n    - discourse.equestria.internal\n\
             other:\n  database: other\n  host: /run/postgresql\n  host_names: [other.example]\n",
        )
        .unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].db, "mlp");
        assert_eq!(specs[0].pool, Some(5));
        assert_eq!(
            specs[0].host_names,
            ["discourse.equestria.com", "discourse.equestria.internal"]
        );
        assert_eq!(
            specs[0].database_url(),
            "postgresql://discourse_mlp:apple%20jack@dbhost:5433/discourse_mlp"
        );
        assert_eq!(
            specs[1].database_url(),
            "postgresql://localhost/other?host=%2Frun%2Fpostgresql"
        );
        assert_eq!(parse("").unwrap(), vec![]);
    }

    #[test]
    fn refuses_what_rails_refuses() {
        assert!(
            parse("default:\n  database: x\n  host_names: [a]\n")
                .unwrap_err()
                .contains("Please do not name any db default!")
        );
        assert!(
            parse("a:\n  host_names: [a]\n")
                .unwrap_err()
                .contains("database")
        );
        assert!(
            parse("a:\n  database: a\n")
                .unwrap_err()
                .contains("host_names")
        );
        assert!(
            parse("a:\n  adapter: mysql2\n  database: a\n  host_names: [a]\n")
                .unwrap_err()
                .contains("adapter")
        );
    }

    #[test]
    fn finds_the_hostname_a_path_names() {
        assert_eq!(
            path_hostname("/highlight-js/a.example/abc.js"),
            Some("a.example")
        );
        assert_eq!(
            path_hostname("/user_avatar/a.example/sam/48/1_2.png"),
            Some("a.example")
        );
        assert_eq!(path_hostname("/user_avatar/sam/refresh_gravatar"), None);
        assert_eq!(
            path_hostname("/svg-sprite/a.example/svg-1-abc.js"),
            Some("a.example")
        );
        assert_eq!(path_hostname("/latest"), None);
        assert_eq!(strip_port("a.example:8080"), "a.example");
        assert_eq!(strip_port("[::1]:80"), "::1");
    }
}
