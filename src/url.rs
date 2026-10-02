//! Base URL and absolute URL helpers: lib/discourse.rb (base_url_no_prefix,
//! current_hostname_with_port), lib/url_helper.rb (absolute) and
//! lib/global_path.rb (cdn_path, upload_cdn_path, full_cdn_url).

use std::fmt;

use crate::config::{Config, RailsEnv};
use crate::site_settings::{SettingError, SiteSettings};

#[derive(Debug)]
pub enum UrlError {
    Setting(SettingError),
    /// S3 upload stores aren't ported; producing a local URL would be wrong.
    S3Unsupported,
}

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UrlError::Setting(e) => e.fmt(f),
            UrlError::S3Unsupported => write!(f, "S3 upload CDN (s3_cdn_url) is not supported yet"),
        }
    }
}

impl std::error::Error for UrlError {}

impl From<SettingError> for UrlError {
    fn from(e: SettingError) -> Self {
        UrlError::Setting(e)
    }
}

/// Everything URL generation reads, bundled per request.
pub struct Urls<'a> {
    pub config: &'a Config,
    pub settings: &'a SiteSettings,
}

impl Urls<'_> {
    /// `SiteSetting.scheme`
    pub fn scheme(&self) -> Result<&'static str, UrlError> {
        Ok(if self.settings.get("force_https")?.truthy() {
            "https"
        } else {
            "http"
        })
    }

    /// `Discourse.current_hostname`: force_hostname, else the site's host.
    pub fn current_hostname(&self) -> Result<String, UrlError> {
        Ok(match self.settings.get("force_hostname")?.presence() {
            Some(host) => host,
            None => self
                .config
                .globals
                .hostname(self.config.rails_env)
                .to_string(),
        })
    }

    /// `Discourse.current_hostname_with_port`
    pub fn current_hostname_with_port(&self) -> Result<String, UrlError> {
        let default_port = if self.settings.get("force_https")?.truthy() {
            443
        } else {
            80
        };
        let port = self.settings.get("port")?;
        let mut result = self.current_hostname()?;
        // Compared as an integer, appended as written.
        if port.to_i() > 0 && port.to_i() != default_port {
            result.push_str(&format!(":{}", port.presence().unwrap_or_default()));
        }
        if self.config.rails_env == RailsEnv::Development && port.is_blank() {
            result.push_str(&format!(":{}", self.config.unicorn_port));
        }
        Ok(result)
    }

    /// `Discourse.base_url`: base_url_no_prefix plus the base path.
    pub fn base_url(&self) -> Result<String, UrlError> {
        Ok(format!(
            "{}{}",
            self.base_url_no_prefix()?,
            self.config.globals.relative_url_root()
        ))
    }

    /// `Discourse.base_url_no_prefix`
    pub fn base_url_no_prefix(&self) -> Result<String, UrlError> {
        Ok(format!(
            "{}://{}",
            self.scheme()?,
            self.current_hostname_with_port()?
        ))
    }

    /// `Discourse.asset_host`: the CDN in production and development only.
    pub(crate) fn asset_host(&self) -> Option<&str> {
        match self.config.rails_env {
            RailsEnv::Production | RailsEnv::Development => self.config.globals.cdn_url(),
            RailsEnv::Test => None,
        }
    }

    /// `UrlHelper.absolute(url)`: prefixes a root-relative path with the CDN
    /// or base URL; anything else (absolute, protocol-relative, empty) is
    /// returned unchanged.
    pub fn absolute(&self, url: &str) -> Result<String, UrlError> {
        let root_relative = url.starts_with('/') && !url.starts_with("//") && url.len() > 1;
        if !root_relative {
            return Ok(url.to_string());
        }
        let base = match self.asset_host() {
            Some(cdn) if cdn.starts_with("//") => format!("https:{cdn}"),
            Some(cdn) => cdn.to_string(),
            None => self.base_url_no_prefix()?,
        };
        Ok(format!("{base}{url}"))
    }

    /// `GlobalPath#cdn_path`
    fn cdn_path(&self, path: &str) -> String {
        match self.config.globals.cdn_url() {
            None => path.to_string(),
            Some(cdn) => format!("{cdn}{}{path}", self.config.globals.relative_url_root()),
        }
    }

    /// `SiteSetting::Upload.s3_cdn_url`
    fn s3_cdn_url(&self) -> Result<Option<String>, UrlError> {
        Ok(if self.settings.get("enable_s3_uploads")?.truthy() {
            self.settings.get("s3_cdn_url")?.presence()
        } else {
            self.config.globals.s3_cdn_url().map(str::to_string)
        })
    }

    /// `GlobalPath#upload_cdn_path`
    fn upload_cdn_path(&self, path: &str) -> Result<String, UrlError> {
        if self.s3_cdn_url()?.is_some() {
            return Err(UrlError::S3Unsupported);
        }
        Ok(if path.starts_with("http") || path.starts_with("//") {
            path.to_string()
        } else {
            self.cdn_path(path)
        })
    }

    /// `GlobalPath#full_cdn_url`: absolute, and always with a scheme.
    pub fn full_cdn_url(&self, url: &str) -> Result<String, UrlError> {
        let absolute = self.absolute(&self.upload_cdn_path(url)?)?;
        Ok(if has_scheme(&absolute) {
            absolute
        } else {
            format!("{}:{absolute}", self.scheme()?)
        })
    }
}

/// RFC 3986 scheme: ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ":"
fn has_scheme(url: &str) -> bool {
    let Some((scheme, _)) = url.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GlobalSettings;
    use crate::site_settings::Definitions;

    fn config(env: RailsEnv, globals: &[(&str, &str)]) -> Config {
        Config {
            database_url: String::new(),
            bind: "127.0.0.1:0".parse().unwrap(),
            rails_env: env,
            unicorn_port: "3000".into(),
            public_dir: "public".into(),
            discourse_src: None,
            globals: GlobalSettings::from_vars(globals.iter().copied()),
        }
    }

    fn settings(rows: &[(&str, i32, &str)], config: &Config) -> SiteSettings {
        let defs = Definitions::vendored().unwrap();
        let rows = rows
            .iter()
            .map(|(n, t, v)| (n.to_string(), *t, Some(v.to_string())))
            .collect();
        SiteSettings::resolve(&defs, rows, &config.globals).unwrap()
    }

    fn with<T>(
        env: RailsEnv,
        globals: &[(&str, &str)],
        rows: &[(&str, i32, &str)],
        f: impl FnOnce(&Urls) -> T,
    ) -> T {
        let config = config(env, globals);
        let settings = settings(rows, &config);
        f(&Urls {
            config: &config,
            settings: &settings,
        })
    }

    #[test]
    fn base_url_per_environment() {
        let base = |env| with(env, &[], &[], |u| u.base_url_no_prefix().unwrap());
        assert_eq!(base(RailsEnv::Production), "http://www.example.com");
        assert_eq!(base(RailsEnv::Development), "http://localhost:3000");
        assert_eq!(base(RailsEnv::Test), "http://test.localhost");

        let host = with(
            RailsEnv::Production,
            &[("hostname", "forum.example.org")],
            &[],
            |u| u.base_url_no_prefix().unwrap(),
        );
        assert_eq!(host, "http://forum.example.org");
    }

    #[test]
    fn port_and_https_rules() {
        let base = |rows: &[(&str, i32, &str)]| {
            with(RailsEnv::Production, &[], rows, |u| {
                u.base_url_no_prefix().unwrap()
            })
        };
        assert_eq!(base(&[("port", 1, "8080")]), "http://www.example.com:8080");
        assert_eq!(base(&[("port", 1, "80")]), "http://www.example.com");
        assert_eq!(
            base(&[("port", 1, "443"), ("force_https", 5, "t")]),
            "https://www.example.com"
        );
        assert_eq!(base(&[("port", 1, "abc")]), "http://www.example.com");
        assert_eq!(
            base(&[("force_hostname", 1, "forced.test")]),
            "http://forced.test"
        );

        // Development appends the unicorn port only when port is blank.
        let dev = with(RailsEnv::Development, &[], &[("port", 1, "4200")], |u| {
            u.base_url_no_prefix().unwrap()
        });
        assert_eq!(dev, "http://localhost:4200");
    }

    #[test]
    fn absolute_only_touches_root_relative_paths() {
        with(RailsEnv::Test, &[], &[], |u| {
            assert_eq!(
                u.absolute("/uploads/a.png").unwrap(),
                "http://test.localhost/uploads/a.png"
            );
            assert_eq!(u.absolute("//cdn.test/a.png").unwrap(), "//cdn.test/a.png");
            assert_eq!(u.absolute("https://x.test/a").unwrap(), "https://x.test/a");
            assert_eq!(u.absolute("").unwrap(), "");
            assert_eq!(u.absolute("/").unwrap(), "/");
        });
    }

    #[test]
    fn cdn_prefixes_uploads_and_protocol_relative_cdn_gets_https() {
        with(
            RailsEnv::Production,
            &[("cdn_url", "https://cdn.test")],
            &[],
            |u| {
                assert_eq!(
                    u.full_cdn_url("/images/a.png").unwrap(),
                    "https://cdn.test/images/a.png"
                );
            },
        );
        with(
            RailsEnv::Production,
            &[("cdn_url", "//cdn.test"), ("relative_url_root", "/forum")],
            &[],
            |u| {
                // cdn_path yields //cdn.test/..., which absolute leaves alone;
                // full_cdn_url then adds SiteSetting.scheme.
                assert_eq!(
                    u.full_cdn_url("/images/a.png").unwrap(),
                    "http://cdn.test/forum/images/a.png"
                );
                // absolute itself upgrades a protocol-relative CDN to https.
                assert_eq!(
                    u.absolute("/images/a.png").unwrap(),
                    "https://cdn.test/images/a.png"
                );
            },
        );
    }

    #[test]
    fn full_cdn_url_adds_a_scheme_to_protocol_relative_urls() {
        with(RailsEnv::Production, &[("force_https", "true")], &[], |u| {
            assert_eq!(
                u.full_cdn_url("//files.test/a.png").unwrap(),
                "https://files.test/a.png"
            );
            assert_eq!(
                u.full_cdn_url("/images/a.png").unwrap(),
                "https://www.example.com/images/a.png"
            );
        });
    }

    #[test]
    fn s3_cdn_is_an_explicit_error() {
        with(
            RailsEnv::Production,
            &[("s3_cdn_url", "https://s3cdn.test")],
            &[],
            |u| {
                assert!(matches!(
                    u.full_cdn_url("/images/a.png"),
                    Err(UrlError::S3Unsupported)
                ));
            },
        );
    }
}
