use std::fmt;
use std::net::SocketAddr;

/// Process-level configuration, the equivalent of Discourse's GlobalSetting.
/// Discourse reads these from DISCOURSE_* env vars (config/discourse_defaults.conf);
/// where a setting has a Discourse counterpart, the same env var name is used.
#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub bind: SocketAddr,
    /// GlobalSetting.cluster_name, checked by /srv/status?cluster=.
    pub cluster_name: Option<String>,
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
        let database_url = std::env::var("DATABASE_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .ok_or(ConfigError::Missing("DATABASE_URL"))?;

        let bind_raw = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".into());
        let bind = bind_raw.parse().map_err(|_| ConfigError::Invalid {
            var: "BIND_ADDR",
            value: bind_raw.clone(),
        })?;

        let cluster_name = std::env::var("DISCOURSE_CLUSTER_NAME")
            .ok()
            .filter(|s| !s.is_empty());

        Ok(Config {
            database_url,
            bind,
            cluster_name,
        })
    }
}
