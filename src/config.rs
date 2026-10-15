use std::collections::BTreeMap;
use std::fmt;
use std::net::SocketAddr;

use crate::ruby;

#[derive(Clone, Debug)]
pub struct Config {
    /// What is the site's own: under multisite.yml each site has its own;
    /// everything else is the process's.
    pub site: SiteConfig,
    pub bind: SocketAddr,
    pub rails_env: RailsEnv,
    /// `ENV["UNICORN_PORT"]`, appended to the base URL in development.
    pub unicorn_port: String,
    /// Directory served at /images and /uploads: a Discourse `public/` (or a
    /// restored backup's), from PUBLIC_DIR.
    pub public_dir: std::path::PathBuf,
    /// A Discourse checkout (DISCOURSE_SRC), whose public/images and bundled
    /// discourse-emojis gem serve the stock images and emoji a backup
    /// doesn't carry. Optional.
    pub discourse_src: Option<std::path::PathBuf>,
    /// Rails' `tmp/` (TMP_DIR): caches like the letter avatar proxy's.
    pub tmp_dir: std::path::PathBuf,
    /// Where `/letter_avatar_proxy` fetches from (LETTER_AVATAR_CDN), so
    /// tests can stand in for the CDN.
    pub letter_avatar_cdn: String,
    /// The NimbusSans-Regular OpenType font local letter avatars are drawn
    /// with (LETTER_AVATAR_FONT, default the path Debian's fonts-urw-base35
    /// installs it at).
    pub letter_avatar_font: std::path::PathBuf,
    pub globals: GlobalSettings,
    /// RateLimiter on (not `RateLimiter.disabled?`): on unless
    /// DISCOURSE_RS_RATE_LIMITS is "off"; the writes replay turns it off
    /// unless a case asks for it, as Rails' recorder does.
    pub rate_limits: bool,
    /// Middleware::RequestTracker's logging and beacons, which Rails loads
    /// outside development or with TRACK_REQUESTS.
    pub track_requests: bool,
}

/// A site's part of the Config (RailsMultisite's connection spec).
#[derive(Clone, Debug)]
pub struct SiteConfig {
    /// `current_db`: `default` for the environment's site, else its
    /// multisite.yml key.
    pub db: String,
    /// DATABASE_URL, or the multisite.yml entry's connection.
    pub database_url: String,
    /// The site's host names, the first `current_hostname`. Empty for the
    /// environment's site, whose one host name is DISCOURSE_HOSTNAME's
    /// (`GlobalSettings::hostname`).
    pub host_names: Vec<String>,
    /// GlobalSetting db_pool's role: DISCOURSE_DB_POOL, else 10; a
    /// multisite.yml entry's `pool`.
    pub pool: u32,
}

impl Default for SiteConfig {
    fn default() -> Self {
        SiteConfig {
            db: crate::multisite::DEFAULT.into(),
            database_url: String::new(),
            host_names: Vec::new(),
            pool: 10,
        }
    }
}

/// The CDN `UserAvatarsController#show_proxy_letter` proxies.
pub const LETTER_AVATAR_CDN: &str = "https://avatars.discourse-cdn.com";

/// `Rails.env`. Changes URL generation the same way it does in Discourse, so
/// discourse-rs can be diffed against a development Rails (e.g. dv).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailsEnv {
    Development,
    Test,
    Production,
}

/// Port of app/models/global_setting.rb with its EnvProvider: every
/// `DISCOURSE_<KEY>` env var becomes `key`, falling back to the defaults in
/// config/discourse_defaults.conf. Any key that names a site setting shadows
/// that setting (see site_settings).
#[derive(Clone, Debug, Default)]
pub struct GlobalSettings {
    vars: BTreeMap<String, String>,
}

impl GlobalSettings {
    pub fn from_vars<I, K, V>(vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        GlobalSettings {
            vars: vars
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    /// `GlobalSetting.<key>` for keys whose discourse_defaults.conf default
    /// is blank: the env value if present, else nil.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars
            .get(key)
            .map(String::as_str)
            .filter(|v| !ruby::is_blank(v))
    }

    /// Every provided key, for site setting shadowing.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.vars.keys().map(String::as_str)
    }

    pub fn cluster_name(&self) -> Option<&str> {
        self.get("cluster_name")
    }

    pub fn cdn_url(&self) -> Option<&str> {
        self.get("cdn_url")
    }

    pub fn s3_cdn_url(&self) -> Option<&str> {
        self.get("s3_cdn_url")
    }

    /// `GlobalSetting.use_s3?`: a bucket and region, and an IAM profile or
    /// keys.
    pub fn use_s3(&self) -> bool {
        let truthy = |name: &str| self.get(name).is_some_and(|v| v == "true");
        self.get("s3_bucket").is_some()
            && self.get("s3_region").is_some()
            && (truthy("s3_use_iam_profile")
                || (self.get("s3_access_key_id").is_some()
                    && self.get("s3_secret_access_key").is_some()))
    }

    pub fn relative_url_root(&self) -> &str {
        self.get("relative_url_root").unwrap_or("")
    }

    /// The single-site hostname RailsMultisite reports, per environment
    /// (config/database.yml, global_setting.rb#database_config).
    pub fn hostname(&self, env: RailsEnv) -> &str {
        match env {
            RailsEnv::Production => self.get("hostname").unwrap_or("www.example.com"),
            RailsEnv::Development => self.get("hostname").unwrap_or("localhost"),
            RailsEnv::Test => "test.localhost",
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Missing(&'static str),
    Invalid { var: &'static str, value: String },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Missing(var) => write!(f, "{var} is not set"),
            ConfigError::Invalid { var, value } => write!(f, "{var} has invalid value {value:?}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_vars(std::env::vars())
    }

    /// Builds a Config from process-style `(KEY, VALUE)` pairs.
    pub fn from_vars<I>(vars: I) -> Result<Self, ConfigError>
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let vars: BTreeMap<String, String> = vars.into_iter().collect();
        let var = |k: &str| vars.get(k).map(String::as_str);

        let database_url = var("DATABASE_URL")
            .filter(|s| !s.is_empty())
            .ok_or(ConfigError::Missing("DATABASE_URL"))?
            .to_string();

        let bind_raw = var("BIND_ADDR").unwrap_or("127.0.0.1:8080");
        let bind = bind_raw.parse().map_err(|_| ConfigError::Invalid {
            var: "BIND_ADDR",
            value: bind_raw.to_string(),
        })?;

        // Rails defaults to development when RAILS_ENV is unset.
        let rails_env = match var("RAILS_ENV") {
            None | Some("") | Some("development") => RailsEnv::Development,
            Some("production") => RailsEnv::Production,
            Some("test") => RailsEnv::Test,
            Some(other) => {
                return Err(ConfigError::Invalid {
                    var: "RAILS_ENV",
                    value: other.to_string(),
                });
            }
        };

        let globals = GlobalSettings::from_vars(vars.iter().filter_map(|(k, v)| {
            k.strip_prefix("DISCOURSE_")
                .map(|key| (key.to_ascii_lowercase(), v.clone()))
        }));

        Ok(Config {
            site: SiteConfig {
                database_url,
                pool: var("DISCOURSE_DB_POOL")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(10),
                ..SiteConfig::default()
            },
            bind,
            rails_env,
            unicorn_port: var("UNICORN_PORT").unwrap_or("3000").to_string(),
            public_dir: var("PUBLIC_DIR").unwrap_or("public").into(),
            discourse_src: var("DISCOURSE_SRC")
                .filter(|s| !s.is_empty())
                .map(Into::into),
            tmp_dir: var("TMP_DIR").unwrap_or("tmp").into(),
            letter_avatar_font: var("LETTER_AVATAR_FONT")
                .filter(|s| !s.is_empty())
                .unwrap_or(crate::letter_avatar::DEFAULT_FONT)
                .into(),
            letter_avatar_cdn: var("LETTER_AVATAR_CDN")
                .unwrap_or(LETTER_AVATAR_CDN)
                .trim_end_matches('/')
                .to_string(),
            globals,
            rate_limits: var("DISCOURSE_RS_RATE_LIMITS") != Some("off"),
            track_requests: rails_env != RailsEnv::Development || var("TRACK_REQUESTS").is_some(),
        })
    }
}

impl Config {
    /// `current_db_hostnames`: the site's host names.
    pub fn hostnames(&self) -> Vec<&str> {
        if self.site.host_names.is_empty() {
            vec![self.globals.hostname(self.rails_env)]
        } else {
            self.site.host_names.iter().map(String::as_str).collect()
        }
    }

    /// `RailsMultisite::ConnectionManagement.current_hostname`: the site's
    /// first host name (`Discourse.current_hostname` is `Urls`'s, which
    /// honours force_hostname).
    pub fn hostname(&self) -> &str {
        self.site
            .host_names
            .first()
            .map_or_else(|| self.globals.hostname(self.rails_env), String::as_str)
    }
}

impl Config {
    /// The discourse-emojis gem's `dist/emoji` inside the checkout's bundle,
    /// where `/images/emoji/<set>/<name>.png` files live.
    pub fn emoji_dir(&self) -> Option<std::path::PathBuf> {
        let src = self.discourse_src.as_ref()?;
        let rubies = std::fs::read_dir(src.join(".bundle/ruby")).ok()?;
        for ruby in rubies.flatten() {
            let gems = std::fs::read_dir(ruby.path().join("gems")).ok()?;
            for gem in gems.flatten() {
                let name = gem.file_name();
                if name.to_string_lossy().starts_with("discourse-emojis-") {
                    let dir = gem.path().join("dist/emoji");
                    if dir.is_dir() {
                        return Some(dir);
                    }
                }
            }
        }
        None
    }
}
