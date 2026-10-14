//! `Oneboxer.external_onebox`: another site's onebox. Where the URL leads
//! (FinalDestination#resolve, under each onebox strategy in turn), the
//! engine's preview there, or the error preview when it can't be reached;
//! cached a day (a minute when there is no onebox) under `onebox__<url>`.

use std::time::Duration;

use serde_json::json;
use sqlx::PgConnection;

use super::Site;
use serde_json::Value;

use super::engines::{self, Ctx, Fail};
use super::fetch::{Fetch, Live};
use super::matcher::{Engine, all_iframe_origins, oneboxed, origins_to_regexes};
use crate::AppError;
use crate::final_destination::{Fetcher, ResolveOptions, Status};

const PREVIEW_ERROR: &str =
    include_str!("../../vendor/discourse/lib/onebox/templates/preview_error_onebox.mustache");
const PREVIEW_ERROR_FRAGMENT: &str = include_str!(
    "../../vendor/discourse/lib/onebox/templates/preview_error_fragment_onebox.mustache"
);

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
    settings_list(site.settings, name)
}

fn settings_list(
    settings: &crate::site_settings::SiteSettings,
    name: &str,
) -> Result<Vec<String>, AppError> {
    Ok(settings
        .get(name)?
        .to_s()
        .split('|')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

/// `Oneboxer.allowed_iframe_origins`
pub fn allowed_iframe_origins(
    settings: &crate::site_settings::SiteSettings,
) -> Result<Vec<String>, AppError> {
    let mut allowed = settings_list(settings, "allowed_onebox_iframes")?;
    if allowed.iter().any(|o| o == "*") {
        allowed = all_iframe_origins();
    }
    allowed.extend(settings_list(settings, "allowed_iframes")?);
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

/// What the error previews need of the site: its messages and emoji.
pub struct Errors<'a> {
    pub i18n: &'a crate::i18n::I18n,
    pub emojis: std::sync::Arc<crate::emojis::Emojis>,
    pub base_path: &'a str,
}

impl<'a> Errors<'a> {
    pub fn of(site: &Site<'a>) -> Result<Errors<'a>, AppError> {
        Ok(Errors {
            i18n: site.i18n,
            emojis: site.settings.emojis().clone(),
            base_path: site.base_path,
        })
    }

    /// `Oneboxer.preview_error_onebox(args, is_fragment)`: the args' title
    /// (the link when it has none) and the error's message with its emoji.
    pub fn onebox(&self, mut args: serde_json::Map<String, Value>, fragment: bool) -> String {
        if let Some(link) = args.get("link").filter(|l| !l.is_null()).cloned()
            && args.get("title").is_none_or(Value::is_null)
        {
            args.insert("title".into(), link);
        }
        if let Some(Value::String(m)) = args.get("error_message") {
            let m = crate::topic_list_view::unescape_emoji(m, &self.emojis, self.base_path);
            args.insert("error_message".into(), json!(m));
        }
        crate::mustache::render(
            if fragment {
                PREVIEW_ERROR_FRAGMENT
            } else {
                PREVIEW_ERROR
            },
            &Value::Object(args),
        )
    }
}

/// The engines' fetches on the network, FinalDestination taking
/// `Oneboxer.get_final_destination_options` under the default strategy.
pub(super) fn live<'a>(
    conn: &'a mut PgConnection,
    site: &Site<'a>,
    locale: &str,
) -> Result<Live<'a>, AppError> {
    let s = site.settings;
    Ok(Live {
        conn,
        fetcher: Fetcher {
            settings: s,
            urls: crate::url::Urls {
                config: site.urls.config,
                settings: s,
            },
        },
        user_agent: onebox_user_agent(site)?,
        accept_language: accept_language(locale),
        resolve_options: ResolveOptions {
            timeout: Duration::from_secs(5),
            max_redirects: 5,
            https_redirect_ignore_limit: false,
            stop_at_blocked_pages: false,
            ignore_redirects: ignore_redirects(site)?,
            force_get_hosts: force_get_hosts(site)?,
            force_custom_user_agent_hosts: setting_list(site, "force_custom_user_agent_hosts")?,
            preserve_fragment_url_hosts: vec!["http://github.com".to_string()],
            custom_user_agent: onebox_user_agent(site)?,
            headers: vec![("Accept-Language".into(), accept_language(locale))],
        },
    })
}

/// `Oneboxer.inline_data_for(url)`: the title an engine gives an inline
/// onebox (Google Meet's, GitHub's with an access token), if it does.
pub async fn inline_data(
    conn: &mut PgConnection,
    site: &Site<'_>,
    url: &str,
    locale: &str,
) -> Result<Option<String>, AppError> {
    let s = site.settings;
    let regexes = origins_to_regexes(&allowed_iframe_origins(s)?);
    let Some(engine) = oneboxed(url, None, &regexes) else {
        return Ok(None);
    };
    match engine.name {
        "GoogleMeetOnebox" => Ok(Some(
            site.i18n
                .t("onebox.google_meet.title")
                .unwrap_or("")
                .to_string(),
        )),
        "GithubIssueOnebox"
        | "GithubCommitOnebox"
        | "GithubRepoOnebox"
        | "GithubPullRequestOnebox" => {
            let hostname = site.urls.current_hostname()?;
            let ctx = Ctx {
                iframes: &regexes,
                cookie: None,
                hostname: &hostname,
                disable_media_download_controls: false,
                i18n: site.i18n,
                settings: engines::EngineSettings::of(s, site.base_path)?,
            };
            let mut fetch = live(conn, site, locale)?;
            super::github::inline_data(engine, url, &ctx, &mut fetch).await
        }
        _ => Ok(None),
    }
}

/// `Oneboxer.preview_error_onebox(link:, error_message:)`
fn preview_error(site: &Site, link: &str, message: Option<String>) -> Result<String, AppError> {
    let mut args = serde_json::Map::new();
    args.insert("link".into(), json!(link));
    args.insert("error_message".into(), json!(message));
    Ok(Errors::of(site)?.onebox(args, false))
}

/// `Onebox.preview(url, options)` as compute_external_onebox takes it: the
/// preview and its placeholder, and when the engine found data missing,
/// the error preview (or the placeholder with the error added to it).
pub async fn engine_onebox<F: Fetch>(
    engine: &Engine,
    url: &str,
    ctx: &Ctx<'_>,
    fetch: &mut F,
    errors: &Errors<'_>,
) -> Result<Onebox, Fail> {
    let rendered = engines::preview(engine, url, ctx, fetch).await?;
    if let Some(m) = rendered.html_raised.or(rendered.placeholder_raised) {
        return Err(Fail::Raised(m));
    }
    let mut result = Onebox {
        onebox: super::preview::finish(rendered.html.as_deref().unwrap_or(""), ctx.iframes)?,
        preview: super::preview::finish(
            rendered.placeholder.as_deref().unwrap_or(""),
            ctx.iframes,
        )?,
    };
    let keys = &rendered.errors;
    if !keys.is_empty() && !(keys.len() == 1 && keys[0] == "image") {
        let mut sorted = keys.clone();
        sorted.sort();
        let comma = errors.i18n.t("word_connector.comma").unwrap_or(", ");
        let message = errors
            .i18n
            .t_count(
                "errors.onebox.missing_data",
                sorted.len() as i64,
                &[("missing_attributes", &sorted.join(comma))],
            )
            .unwrap_or_default();
        let mut args = match rendered.verified_data {
            Some(Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        };
        args.insert("error_message".into(), json!(message));
        if crate::ruby::is_blank(&result.preview) {
            result.preview = errors.onebox(args, false);
        } else {
            use crate::pretty_text::cleanup::{append, fragment_root, parse, to_html};
            let dom = parse(&result.preview);
            if let Some(aside) =
                crate::pretty_text::select::select_first(&fragment_root(&dom), "aside")
            {
                let fragment = parse(&errors.onebox(args, true));
                let children: Vec<_> = fragment_root(&fragment).children.borrow().clone();
                for child in children {
                    append(&aside, child);
                }
                result.preview = to_html(&dom);
            }
        }
    }
    Ok(result)
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
        // A private repository 404s without the token: GitHub isn't asked.
        let mut ignored = ignore_redirects(site)?;
        if !crate::ruby::is_blank(&s.get("github_onebox_access_tokens")?.to_s())
            && start.host == "github.com"
        {
            ignored.push("https://github.com".into());
        }
        let opts = ResolveOptions {
            timeout: Duration::from_secs(5),
            max_redirects: if block_redirects { 0 } else { 5 },
            https_redirect_ignore_limit: block_redirects,
            stop_at_blocked_pages: true,
            ignore_redirects: ignored,
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
        let regexes = origins_to_regexes(&allowed_iframe_origins(s)?);
        let mut result = match oneboxed(&final_url, resolution.content_type.as_deref(), &regexes) {
            None => Onebox::default(),
            Some(engine) => {
                let hostname = site.urls.current_hostname()?;
                let ctx = Ctx {
                    iframes: &regexes,
                    cookie: resolution.cookie.as_deref(),
                    hostname: &hostname,
                    disable_media_download_controls: s
                        .get("disable_onebox_media_download_controls")?
                        .truthy(),
                    i18n: site.i18n,
                    settings: engines::EngineSettings::of(s, site.base_path)?,
                };
                let mut live = live(&mut *conn, site, &locale)?;
                let errors = Errors::of(site)?;
                match engine_onebox(engine, &final_url, &ctx, &mut live, &errors).await {
                    Ok(result) => result,
                    Err(Fail::App(e)) => return Err(e),
                    Err(Fail::Web(_)) => Onebox::default(),
                    Err(Fail::Raised(_)) => return Ok(Computed::Raised),
                }
            }
        };
        // WordWatcher.censor
        {
            use crate::word_watcher as ww;
            let censor = ww::compiled_for_action(&mut *conn, s, ww::CENSOR).await?;
            result.onebox = ww::censor_html(&result.onebox, &censor)?;
            result.preview = ww::censor_html(&result.preview, &censor)?;
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
