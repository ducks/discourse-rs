//! `Oneboxer.external_onebox`: another site's onebox. Where the URL leads
//! (FinalDestination#resolve, under each onebox strategy in turn), the
//! engine's preview there, or the error preview when it can't be reached;
//! cached a day (a minute when there is no onebox) under `onebox__<url>`.

use std::time::Duration;

use serde_json::json;
use sqlx::PgConnection;

use super::Site;
use super::engines::{self, Ctx};
use super::matcher::{all_iframe_origins, oneboxed, origins_to_regexes};
use crate::AppError;
use crate::final_destination::{Fetcher, ResolveOptions, Status};

const PREVIEW_ERROR: &str =
    include_str!("../../vendor/discourse/lib/onebox/templates/preview_error_onebox.mustache");

/// An onebox and its composer preview.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Onebox {
    pub onebox: String,
    pub preview: String,
}

pub fn cache_key(url: &str) -> String {
    format!("onebox__{url}")
}

/// `Oneboxer.cached_onebox` / `cached_preview`'s read.
pub async fn cached(conn: &mut PgConnection, url: &str) -> Result<Option<Onebox>, AppError> {
    Ok(crate::owned_schema::cached_get(conn, &cache_key(url))
        .await?
        .and_then(|v| serde_json::from_str(&v).ok()))
}

/// `Oneboxer.invalidate`
pub async fn invalidate(conn: &mut PgConnection, url: &str) -> Result<(), AppError> {
    crate::owned_schema::cached_del(conn, &cache_key(url)).await?;
    crate::owned_schema::cached_del(conn, &format!("onebox_failed__{url}")).await?;
    Ok(())
}

fn setting_list(site: &Site, name: &str) -> Result<Vec<String>, AppError> {
    Ok(site
        .settings
        .get(name)?
        .to_s()
        .split('|')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

/// `Oneboxer.allowed_iframe_origins`
pub fn allowed_iframe_origins(site: &Site) -> Result<Vec<String>, AppError> {
    let mut allowed = setting_list(site, "allowed_onebox_iframes")?;
    if allowed.iter().any(|o| o == "*") {
        allowed = all_iframe_origins();
    }
    allowed.extend(setting_list(site, "allowed_iframes")?);
    Ok(allowed)
}

/// `Oneboxer.ignore_redirects`
fn ignore_redirects(site: &Site) -> Result<Vec<String>, AppError> {
    let mut list: Vec<String> = [
        "http://www.dropbox.com",
        "http://store.steampowered.com",
        "http://vimeo.com",
        "https://reddit.com",
        "https://www.reddit.com",
        "https://old.reddit.com",
        "https://np.reddit.com",
        "https://new.reddit.com",
        "https://www.youtube.com",
        "https://youtu.be",
        "https://meet.google.com",
        "https://twitter.com",
        "https://x.com",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    list.push(site.urls.base_url()?);
    Ok(list)
}

/// `Oneboxer.force_get_hosts`: the setting's domains and Amazon's.
fn force_get_hosts(site: &Site) -> Result<Vec<String>, AppError> {
    let mut hosts: Vec<String> = setting_list(site, "force_get_hosts")?
        .into_iter()
        .map(|d| format!("https://{d}"))
        .collect();
    for suffix in [
        "com", "com.br", "ca", "cn", "fr", "de", "in", "it", "co.jp", "com.mx", "nl", "pl", "sa",
        "sg", "es", "se", "com.tr", "ae", "co.uk",
    ] {
        hosts.push(format!("https://www.amazon.{suffix}"));
    }
    let mut unique: Vec<String> = Vec::new();
    for h in hosts {
        if !unique.contains(&h) {
            unique.push(h);
        }
    }
    Ok(unique)
}

/// `Onebox::Helpers.user_agent`
fn onebox_user_agent(site: &Site) -> Result<String, AppError> {
    let custom = site.settings.get("onebox_user_agent")?.to_s().to_string();
    Ok(if custom.trim().is_empty() {
        crate::schema::user_agent()
    } else {
        format!("{custom} v{}", crate::schema::discourse_version())
    })
}

/// `Oneboxer.accept_language`
pub(super) fn accept_language(locale: &str) -> String {
    if locale == "en" {
        "en;q=0.9, *;q=0.5".to_string()
    } else {
        format!("{};q=0.9, en;q=0.8, *;q=0.5", locale.replace('_', "-"))
    }
}

fn strategy_key(hostname: &str) -> String {
    format!("ONEBOXER_STRATEGY_{hostname}")
}

/// `Oneboxer.preview_error_onebox(args)`: the link, its title (the link
/// itself) and the error's message with its emoji.
fn preview_error(site: &Site, link: &str, message: Option<String>) -> Result<String, AppError> {
    let s = site.settings;
    let message = match message {
        Some(m) if s.get("enable_emoji")?.truthy() => {
            Some(crate::topic_list_view::emoji_unescape_with(
                &m,
                s.get("enable_inline_emoji_translation")?.truthy(),
                &s.get("emoji_set")?.to_s(),
                site.base_path,
            ))
        }
        m => m,
    };
    Ok(crate::mustache::render(
        PREVIEW_ERROR,
        &json!({ "link": link, "title": link, "error_message": message }),
    ))
}

/// What computing the onebox came to: an onebox to cache, or an error
/// Oneboxer.onebox_raw rescues into a blank onebox, uncached.
enum Computed {
    Onebox(Onebox),
    Raised,
}

/// `Oneboxer.compute_external_onebox`
async fn compute(
    conn: &mut PgConnection,
    site: &Site<'_>,
    url: &str,
) -> Result<Computed, AppError> {
    let Some(start) = crate::final_destination::Uri::parse(url) else {
        return Ok(Computed::Raised);
    };
    let s = site.settings;
    let mut strategies = vec!["default", "force_get_and_ua"];
    if let Some(preferred) =
        crate::owned_schema::cached_get(conn, &strategy_key(&start.host)).await?
        && let Some(i) = strategies.iter().position(|s| *s == preferred)
    {
        let p = strategies.remove(i);
        strategies.insert(0, p);
    }
    let block_redirects = s.get("block_onebox_on_redirect")?.truthy();
    let locale = super::onebox_locale(s)?;
    let fetcher = Fetcher {
        settings: s,
        urls: crate::url::Urls {
            config: site.urls.config,
            settings: s,
        },
    };
    let mut remaining = strategies.into_iter();
    loop {
        let strategy = remaining.next().unwrap_or("default");
        let forced = strategy == "force_get_and_ua";
        let host_url = vec![format!("https://{}", start.host)];
        if s.get("github_onebox_access_tokens")?.to_s().trim() != "" && start.host == "github.com" {
            return Err(crate::Unsupported("oneboxes with GitHub access tokens").into());
        }
        let opts = ResolveOptions {
            timeout: Duration::from_secs(5),
            max_redirects: if block_redirects { 0 } else { 5 },
            https_redirect_ignore_limit: block_redirects,
            stop_at_blocked_pages: true,
            ignore_redirects: ignore_redirects(site)?,
            force_get_hosts: if forced {
                host_url.clone()
            } else {
                force_get_hosts(site)?
            },
            force_custom_user_agent_hosts: if forced {
                host_url
            } else {
                setting_list(site, "force_custom_user_agent_hosts")?
            },
            preserve_fragment_url_hosts: vec!["http://github.com".to_string()],
            custom_user_agent: onebox_user_agent(site)?,
            headers: vec![("Accept-Language".into(), accept_language(&locale))],
        };
        let resolution = match fetcher.resolve(conn, url, &opts).await? {
            Ok(r) => r,
            Err(_) => return Ok(Computed::Raised),
        };
        let message = match &resolution.status {
            Status::BlockedPage => return Ok(Computed::Onebox(Onebox::default())),
            Status::Resolved => None,
            Status::InvalidAddress => Some(site.i18n.t_with(
                "errors.onebox.invalid_address",
                &[("hostname", &resolution.hostname)],
            )),
            // Another strategy, while there is one.
            _ if remaining.len() > 0 => continue,
            Status::Failure {
                bot_challenge: true,
                ..
            } => Some(
                site.i18n
                    .t("errors.onebox.bot_challenge")
                    .map(str::to_string),
            ),
            Status::Failure { status_code, .. } => Some(site.i18n.t_with(
                "errors.onebox.error_response",
                &[("status_code", &status_code.to_string())],
            )),
            Status::TooManyRedirects | Status::Timeout => Some(None),
        };
        if let Some(message) = message {
            return Ok(Computed::Onebox(Onebox {
                onebox: String::new(),
                preview: preview_error(site, url, message)?,
            }));
        }
        let Some(uri) = resolution.uri else {
            return Ok(Computed::Onebox(Onebox::default()));
        };
        let final_url = uri.to_s();
        let regexes = origins_to_regexes(&allowed_iframe_origins(site)?);
        let result = match oneboxed(&final_url, resolution.content_type.as_deref(), &regexes) {
            None => Onebox::default(),
            Some(engine) => {
                let hostname = site.urls.current_hostname()?;
                let ctx = Ctx {
                    hostname: &hostname,
                    disable_media_download_controls: s
                        .get("disable_onebox_media_download_controls")?
                        .truthy(),
                    i18n: site.i18n,
                };
                let html = engines::render(engine, &final_url, &ctx)?;
                Onebox {
                    onebox: super::preview::finish(&html.html, &regexes)?,
                    preview: super::preview::finish(&html.placeholder, &regexes)?,
                }
            }
        };
        // WordWatcher.censor
        if !result.onebox.is_empty() || !result.preview.is_empty() {
            super::refuse_censored(conn).await?;
        }
        if strategy != "default" {
            crate::owned_schema::cached_setex(
                conn,
                &strategy_key(&uri.host),
                14 * 86_400,
                strategy,
            )
            .await?;
        }
        return Ok(Computed::Onebox(result));
    }
}

/// `Oneboxer.external_onebox`: the cached onebox, else computed and
/// cached; blank when computing raised.
pub async fn external_onebox(
    conn: &mut PgConnection,
    site: &Site<'_>,
    url: &str,
) -> Result<Onebox, AppError> {
    if let Some(hit) = cached(conn, url).await? {
        return Ok(hit);
    }
    match compute(conn, site, url).await? {
        Computed::Raised => Ok(Onebox::default()),
        Computed::Onebox(result) => {
            let ttl = if result.onebox.trim().is_empty() {
                60
            } else {
                86_400
            };
            crate::owned_schema::cached_setex(
                conn,
                &cache_key(url),
                ttl,
                &serde_json::to_string(&result)?,
            )
            .await?;
            Ok(result)
        }
    }
}
