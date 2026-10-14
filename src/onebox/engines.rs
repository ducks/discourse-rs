//! The onebox engines that render from the URL alone (lib/onebox/engine/):
//! media files, Twitch, Youku, Typeform, Google Meet and Google Calendar.
//! Each gives its `to_html` and `placeholder_html`, before
//! Onebox::Preview processes and sanitizes them. Engines that fetch the
//! page or an API are refused.

use std::sync::LazyLock;

use serde_json::json;

use super::fetch::Fetch;
use super::matcher::{Engine, ruby_regex};
use super::normalize_url_for_output;
use crate::{AppError, Unsupported};

/// What an engine needs besides the URL.
pub struct Ctx<'a> {
    /// `allowed_iframe_regexes`
    pub iframes: &'a [regex::Regex],
    /// The cookie FinalDestination was given (`options[:cookie]`).
    pub cookie: Option<&'a str>,
    /// `Discourse.current_hostname`
    pub hostname: &'a str,
    /// `disable_onebox_media_download_controls`
    pub disable_media_download_controls: bool,
    pub i18n: &'a crate::i18n::I18n,
    pub settings: EngineSettings,
}

/// The site settings the engines read.
#[derive(Debug, Clone, Default)]
pub struct EngineSettings {
    /// `github_onebox_access_tokens`
    pub github_tokens: String,
    /// `github_pr_status_enabled` (discourse-github)
    pub github_pr_status: bool,
    /// `facebook_app_access_token`
    pub facebook_token: String,
    /// TwitterApi has its consumer key and secret.
    pub twitter_credentials: bool,
    /// The emoji settings and custom emoji (`Emoji.codes_to_img`).
    pub emojis: std::sync::Arc<crate::emojis::Emojis>,
    /// `Discourse.base_path`
    pub base_path: String,
    /// discourse-lazy-videos for YouTube, Vimeo and TikTok.
    pub lazy_youtube: bool,
    pub lazy_vimeo: bool,
    pub lazy_tiktok: bool,
}

impl EngineSettings {
    pub fn of(
        settings: &crate::site_settings::SiteSettings,
        base_path: &str,
    ) -> Result<EngineSettings, AppError> {
        let s = |k: &str| -> Result<String, AppError> { Ok(settings.get(k)?.to_s().to_string()) };
        let on = |k: &str| settings.get(k).map(|v| v.truthy()).unwrap_or(false);
        Ok(EngineSettings {
            github_tokens: s("github_onebox_access_tokens")?,
            github_pr_status: settings
                .get("github_pr_status_enabled")
                .map(|v| v.truthy())
                .unwrap_or(false),
            facebook_token: s("facebook_app_access_token")?,
            twitter_credentials: !crate::ruby::is_blank(&s("twitter_consumer_key")?)
                && !crate::ruby::is_blank(&s("twitter_consumer_secret")?),
            emojis: settings.emojis().clone(),
            base_path: base_path.to_string(),
            lazy_youtube: on("lazy_videos_enabled") && on("lazy_youtube_enabled"),
            lazy_vimeo: on("lazy_videos_enabled") && on("lazy_vimeo_enabled"),
            lazy_tiktok: on("lazy_videos_enabled") && on("lazy_tiktok_enabled"),
        })
    }

    /// `Onebox::GithubAccess.token(org)`
    pub fn github_token(&self, org: Option<&str>) -> Option<String> {
        let map: Vec<(&str, &str)> = self
            .github_tokens
            .split('\n')
            .filter_map(|l| l.split_once('|'))
            .collect();
        let find = |k: &str| map.iter().rev().find(|(o, _)| *o == k).map(|(_, t)| *t);
        org.and_then(find)
            .or_else(|| find("default"))
            .filter(|t| !crate::ruby::is_blank(t))
            .map(str::to_string)
    }
}

/// An engine's HTML and its placeholder.
pub struct Html {
    pub html: String,
    pub placeholder: String,
}

const LAYOUT: &str = include_str!("../../vendor/discourse/lib/onebox/templates/_layout.mustache");
const GOOGLE_MEET: &str =
    include_str!("../../vendor/discourse/lib/onebox/templates/googlemeet.mustache");

pub(super) fn placeholder(kind: &str) -> String {
    format!(
        "<div class='onebox-placeholder-container'><span class='placeholder-icon {kind}'></span></div>"
    )
}

/// `AllowlistedGenericOnebox.host_matches(uri, https_hosts)`, the media
/// engines' `always_https?`.
pub(super) fn https_host(host: &str) -> bool {
    [
        "slideshare.net",
        "dailymotion.com",
        "livestream.com",
        "imgur.com",
        "flickr.com",
    ]
    .iter()
    .any(|h| host == *h || host.ends_with(&format!(".{h}")))
}

/// `Engine#initialize` for an engine that is always https: the URL with
/// its scheme made https.
pub(super) fn https(url: &str) -> String {
    if url.starts_with("//") {
        format!("https:{url}")
    } else if let Some((_, rest)) = url.split_once("://") {
        format!("https://{rest}")
    } else {
        url.to_string()
    }
}

fn host_of(url: &str) -> String {
    let rest = url.split_once("//").map_or(url, |(_, r)| r);
    rest.split(['/', '?', '#', ':'])
        .next()
        .unwrap_or("")
        .to_lowercase()
}

/// `URI.decode_www_form_component`
fn decode_component(s: &str) -> String {
    let bytes = s.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&String::from_utf8_lossy(&bytes[i + 1..i + 3]), 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `URI.encode_www_form_component`
fn encode_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'*' | b'-' | b'.' | b'_' | b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `Onebox::Helpers.normalize_dropbox_url`: a Dropbox share as its file.
fn normalize_dropbox_url(url: &str) -> String {
    let Some(rest) = url.strip_prefix("https://www.dropbox.com/") else {
        return url.to_string();
    };
    let (path, fragment) = match rest.split_once('#') {
        Some((p, f)) => (p, Some(f)),
        None => (rest, None),
    };
    let (path, query) = match path.split_once('?') {
        Some((p, q)) => (p, Some(q.to_string())),
        None => (path, None),
    };
    let query = if url.contains("/scl/") {
        // URI.decode_www_form(query).to_h, raw=1, then encode_www_form.
        let mut params: Vec<(String, String)> = Vec::new();
        for pair in query
            .as_deref()
            .unwrap_or("")
            .split('&')
            .filter(|p| !p.is_empty())
        {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let (k, v) = (decode_component(k), decode_component(v));
            match params.iter_mut().find(|(key, _)| *key == k) {
                Some(existing) => existing.1 = v,
                None => params.push((k, v)),
            }
        }
        match params.iter_mut().find(|(k, _)| k == "raw") {
            Some(raw) => raw.1 = "1".into(),
            None => params.push(("raw".into(), "1".into())),
        }
        Some(
            params
                .iter()
                .map(|(k, v)| format!("{}={}", encode_component(k), encode_component(v)))
                .collect::<Vec<_>>()
                .join("&"),
        )
    } else {
        query
    };
    let mut out = format!("https://dl.dropboxusercontent.com/{path}");
    if let Some(q) = query {
        out.push('?');
        out.push_str(&q);
    }
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

fn controls(ctx: &Ctx) -> &'static str {
    if ctx.disable_media_download_controls {
        "controlslist=\"nodownload\""
    } else {
        ""
    }
}

fn audio(url: &str, ctx: &Ctx) -> Html {
    let escaped = normalize_url_for_output(url);
    Html {
        html: format!(
            "          <audio controls {}>\n            <source src=\"{escaped}\">\n            <a href=\"{escaped}\">{url}</a>\n          </audio>\n",
            controls(ctx)
        ),
        placeholder: placeholder("audio"),
    }
}

fn video(url: &str, ctx: &Ctx) -> Html {
    let url = normalize_dropbox_url(url);
    let escaped = normalize_url_for_output(&url);
    Html {
        html: format!(
            "          <div class=\"onebox video-onebox\">\n            <video width='100%' height='100%' controls {}>\n              <source src='{escaped}'>\n              <a href='{escaped}'>{url}</a>\n            </video>\n          </div>\n",
            controls(ctx)
        ),
        placeholder: placeholder("video"),
    }
}

fn image(url: &str) -> Html {
    let url = normalize_dropbox_url(url);
    let escaped = normalize_url_for_output(&url);
    let html = format!(
        "          <a href=\"{escaped}\" target=\"_blank\" rel=\"noopener\" class=\"onebox\">\n            <img src=\"{escaped}\">\n          </a>\n"
    );
    Html {
        placeholder: html.clone(),
        html,
    }
}

/// Onebox::Mixins::TwitchOnebox
fn twitch(url: &str, ctx: &Ctx, engine: &str) -> Html {
    let (regex, base_url, param) = match engine {
        "TwitchClipsOnebox" => (&*TWITCH_CLIPS, "clips.twitch.tv/embed?", "clip="),
        "TwitchStreamOnebox" => (&*TWITCH_STREAM, "player.twitch.tv/?", "channel="),
        _ => (&*TWITCH_VIDEO, "player.twitch.tv/?", "video=v"),
    };
    let id = regex
        .captures(url)
        .ok()
        .flatten()
        .and_then(|c| c.get(1))
        .map_or("", |m| m.as_str());
    Html {
        html: format!(
            "<iframe src=\"https://{base_url}{param}{id}&parent={}&autoplay=false\" width=\"620\" height=\"378\" frameborder=\"0\" style=\"overflow: hidden;\" scrolling=\"no\" allowfullscreen=\"allowfullscreen\"></iframe>\n",
            ctx.hostname
        ),
        placeholder: placeholder("video"),
    }
}

static TWITCH_CLIPS: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    ruby_regex(
        r"^https?://(?:clips\.twitch\.tv/embed\?clip=|www\.twitch\.tv/[a-zA-Z0-9_]+/clip/|clips\.twitch\.tv/)([a-zA-Z0-9_\-]+)",
        false,
    )
});
static TWITCH_STREAM: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    ruby_regex(
        r"^https?:\/\/(?:www\.|go\.)?twitch\.tv\/(?!directory)([a-zA-Z0-9_]{4,25})$",
        false,
    )
});
static TWITCH_VIDEO: LazyLock<fancy_regex::Regex> =
    LazyLock::new(|| ruby_regex(r"^https?://(?:www\.)?twitch\.tv/videos/([0-9]+)", false));

fn path_of(url: &str) -> String {
    crate::final_destination::Uri::parse(url)
        .map(|u| u.path().to_string())
        .unwrap_or_default()
}

fn youku(url: &str) -> Html {
    static ID: LazyLock<fancy_regex::Regex> =
        LazyLock::new(|| ruby_regex(r"/v_show/id_([a-zA-Z0-9_=\-]+)(\.html)?.*", false));
    let path = path_of(url);
    let id = ID
        .captures(&path)
        .ok()
        .flatten()
        .and_then(|c| c.get(1))
        .map_or(String::new(), |m| m.as_str().to_string());
    let html = format!(
        "<iframe\n  src=\"https://player.youku.com/embed/{id}\"\n  width=\"640\"\n  height=\"430\"\n  frameborder='0'\n  allowfullscreen\n></iframe>\n"
    );
    Html {
        placeholder: html.clone(),
        html,
    }
}

fn typeform(url: &str) -> Html {
    let mut src = normalize_url_for_output(url);
    // CGI.parse(URI.parse(src).query || "")
    let query = src
        .split_once('?')
        .map(|(_, q)| q.split('#').next().unwrap_or("").to_string())
        .unwrap_or_default();
    let keys: Vec<String> = query
        .split(['&', ';'])
        .filter(|p| !p.is_empty())
        .map(|p| decode_component(p.split('=').next().unwrap_or("")))
        .collect();
    if !keys.iter().any(|k| k == "typeform-embed") {
        if keys.is_empty() {
            if !src.ends_with('?') {
                src.push('?');
            }
        } else {
            src.push('&');
        }
        src.push_str("typeform-embed=embed-widget");
    }
    Html {
        html: format!(
            "<iframe\n  src=\"{src}\"\n  width=\"100%\"\n  height=\"600px\"\n  scrolling=\"no\"\n  frameborder=\"0\"\n></iframe>\n"
        ),
        placeholder: placeholder("generic"),
    }
}

fn google_calendar(url: &str) -> Html {
    let url = url.split('&').next().unwrap_or("");
    let src = normalize_url_for_output(url);
    let placeholder = "          <div placeholder>\n            <div class='gdocs-onebox gdocs-onebox-splash' style='display:table-cell;vertical-align:middle;width:800px;height:600px'>\n              <div style='text-align:center;'>\n                <div class='gdocs-onebox-logo g-calendar-logo'></div>\n                <p>Google Calendar</p>\n              </div>\n            </div>\n          </div>\n".to_string();
    Html {
        html: format!(
            "<iframe src='{src}&rm=minimal' style='border: 0' width='800' height='600' frameborder='0' scrolling='no'>{placeholder}</iframe>"
        ),
        placeholder,
    }
}

/// `Engine#link`: `Onebox::Helpers.uri_encode(@url)`.
pub(super) fn link(url: &str) -> Result<String, Unsupported> {
    super::uri::uri_encode(url).ok_or(Unsupported("onebox links Addressable can't parse"))
}

/// The partials the onebox templates include.
fn partial(name: &str) -> Option<&'static str> {
    match name {
        "json_ld_partials/movie" => Some(include_str!(
            "../../vendor/discourse/lib/onebox/templates/json_ld_partials/movie.mustache"
        )),
        "github/github_body" => Some(include_str!(
            "../../vendor/discourse/lib/onebox/templates/github/github_body.mustache"
        )),
        _ => None,
    }
}

/// Onebox::Layout: the engine's view inside _layout.mustache.
pub(super) fn layout(name: &str, view: &str, record: &serde_json::Value) -> String {
    let link = record["link"].as_str().unwrap_or("");
    let uri = super::uri::RubyUri::parse(&normalize_url_for_output(link));
    let host = uri
        .as_ref()
        .and_then(|u| u.host.clone())
        .unwrap_or_default();
    // Fix any relative paths
    let mut record = record.clone();
    if let Some(image) = record["image"].as_str()
        && image.starts_with('/')
        && image.chars().nth(1).is_some_and(|c| c != '/')
    {
        let scheme = uri
            .as_ref()
            .and_then(|u| u.scheme.clone())
            .unwrap_or_default();
        record["image"] = json!(format!("{scheme}://{host}/{image}"));
    }
    let record = &record;
    let domain = match record["domain"].as_str() {
        Some(d) => d.to_string(),
        None => host.strip_prefix("www.").unwrap_or(&host).to_string(),
    };
    let details = json!({
        "link": record["link"],
        "title": record["title"],
        "favicon": record["favicon"],
        "domain": domain,
        "article_published_time": record["article_published_time"],
        "article_published_time_title": record["article_published_time_title"],
        "metadata_1_label": record["metadata_1_label"],
        "metadata_1_value": record["metadata_1_value"],
        "metadata_2_label": record["metadata_2_label"],
        "metadata_2_value": record["metadata_2_value"],
        "subname": name,
        "view": crate::mustache::render_with_partials(view, record, &partial),
    });
    crate::mustache::render(LAYOUT, &details)
}

fn google_meet(url: &str, ctx: &Ctx) -> Result<Html, Unsupported> {
    static CODE: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
        fancy_regex::Regex::new(r"(?i)\A/(?<code>[a-z]{3}-[a-z]{4}-[a-z]{3})/?\z").unwrap()
    });
    let path = path_of(url);
    let code = CODE
        .captures(&path)
        .ok()
        .flatten()
        .and_then(|c| c.name("code"))
        .map(|m| m.as_str().to_lowercase());
    let t = |key: &str| ctx.i18n.t(key).unwrap_or("").to_string();
    let record = json!({
        "link": link(url)?,
        "domain": "Google Meet",
        "title": t("onebox.google_meet.title"),
        "description": if code.is_none() { json!(t("onebox.google_meet.description")) } else { json!(null) },
        "meeting_code": code,
        "meeting_code_label": t("onebox.google_meet.meeting_code"),
        "join_label": t("onebox.google_meet.join"),
    });
    let html = layout("googlemeet", GOOGLE_MEET, &record);
    Ok(Html {
        placeholder: html.clone(),
        html,
    })
}

/// The engines over StandardEmbed (embed_engines.rs).
const EMBED_ENGINES: &[&str] = &[
    "AnimatedImageOnebox",
    "AsciinemaOnebox",
    "AudioboomOnebox",
    "AudioComOnebox",
    "BandCampOnebox",
    "CloudAppOnebox",
    "CoubOnebox",
    "DiscourseTopicOnebox",
    "FacebookMediaOnebox",
    "FiveHundredPxOnebox",
    "FlickrOnebox",
    "FlickrShortenedOnebox",
    "GithubFolderOnebox",
    "GoogleDocsOnebox",
    "GoogleDriveOnebox",
    "GooglePhotosOnebox",
    "ImgurOnebox",
    "InstagramOnebox",
    "KalturaOnebox",
    "LoomOnebox",
    "MixcloudOnebox",
    "MotokoOnebox",
    "RedditMediaOnebox",
    "ReplitOnebox",
    "RestreamOnebox",
    "SimplecastOnebox",
    "SketchFabOnebox",
    "SlidesOnebox",
    "SoundCloudOnebox",
    "SpotifyOnebox",
    "SteamStoreOnebox",
    "TiktokOnebox",
    "TrelloOnebox",
    "VimeoOnebox",
    "WistiaOnebox",
    "YoutubeOnebox",
];

/// The GitHub and GitLab engines (github.rs).
const GITHUB_ENGINES: &[&str] = &[
    "GithubActionsOnebox",
    "GithubBlobOnebox",
    "GithubCommitOnebox",
    "GithubGistOnebox",
    "GithubIssueOnebox",
    "GithubPullRequestOnebox",
    "GithubRepoOnebox",
    "GitlabBlobOnebox",
];

/// The JSON and HTML engines (api_engines.rs).
const API_ENGINES: &[&str] = &[
    "AmazonOnebox",
    "GfycatOnebox",
    "GoogleMapsOnebox",
    "GooglePlayAppOnebox",
    "HackernewsOnebox",
    "PastebinOnebox",
    "PdfOnebox",
    "PubmedOnebox",
    "StackExchangeOnebox",
    "ThreadsStatusOnebox",
    "TwitterStatusOnebox",
    "WikimediaOnebox",
    "WikipediaOnebox",
    "XkcdOnebox",
];

/// The engines ported: every engine but the ones UNPORTED names.
pub const PORTED: &[&str] = &[
    "AudioOnebox",
    "VideoOnebox",
    "ImageOnebox",
    "TwitchClipsOnebox",
    "TwitchStreamOnebox",
    "TwitchVideoOnebox",
    "YoukuOnebox",
    "TypeformOnebox",
    "GoogleCalendarOnebox",
    "GoogleMeetOnebox",
    "AllowlistedGenericOnebox",
    "AnimatedImageOnebox",
    "AsciinemaOnebox",
    "AudioboomOnebox",
    "AudioComOnebox",
    "BandCampOnebox",
    "CloudAppOnebox",
    "CoubOnebox",
    "DiscourseTopicOnebox",
    "FacebookMediaOnebox",
    "FiveHundredPxOnebox",
    "FlickrOnebox",
    "FlickrShortenedOnebox",
    "GithubFolderOnebox",
    "GoogleDocsOnebox",
    "GoogleDriveOnebox",
    "GooglePhotosOnebox",
    "ImgurOnebox",
    "InstagramOnebox",
    "KalturaOnebox",
    "LoomOnebox",
    "MixcloudOnebox",
    "MotokoOnebox",
    "RedditMediaOnebox",
    "ReplitOnebox",
    "RestreamOnebox",
    "SimplecastOnebox",
    "SketchFabOnebox",
    "SlidesOnebox",
    "SoundCloudOnebox",
    "SpotifyOnebox",
    "SteamStoreOnebox",
    "TiktokOnebox",
    "TrelloOnebox",
    "VimeoOnebox",
    "WistiaOnebox",
    "YoutubeOnebox",
    "GithubActionsOnebox",
    "GithubBlobOnebox",
    "GithubCommitOnebox",
    "GithubGistOnebox",
    "GithubIssueOnebox",
    "GithubPullRequestOnebox",
    "GithubRepoOnebox",
    "GitlabBlobOnebox",
    "AmazonOnebox",
    "GfycatOnebox",
    "GoogleMapsOnebox",
    "GooglePlayAppOnebox",
    "HackernewsOnebox",
    "PastebinOnebox",
    "PdfOnebox",
    "PubmedOnebox",
    "StackExchangeOnebox",
    "ThreadsStatusOnebox",
    "TwitterStatusOnebox",
    "WikimediaOnebox",
    "WikipediaOnebox",
    "XkcdOnebox",
];

/// `Onebox::Engine#to_html` and `#placeholder_html` for the engine that
/// handles the URL; refused for an engine that fetches.
pub fn render(engine: &Engine, url: &str, ctx: &Ctx) -> Result<Html, Unsupported> {
    let https_url = |always: bool| if always { https(url) } else { url.to_string() };
    match engine.name {
        "AudioOnebox" => Ok(audio(&https_url(https_host(&host_of(url))), ctx)),
        "VideoOnebox" => Ok(video(&https_url(https_host(&host_of(url))), ctx)),
        "ImageOnebox" => Ok(image(&https_url(https_host(&host_of(url))))),
        "TwitchClipsOnebox" | "TwitchStreamOnebox" | "TwitchVideoOnebox" => {
            Ok(twitch(&https_url(engine.always_https), ctx, engine.name))
        }
        "YoukuOnebox" => Ok(youku(&https_url(engine.always_https))),
        "TypeformOnebox" => Ok(typeform(&https_url(engine.always_https))),
        "GoogleCalendarOnebox" => Ok(google_calendar(&https_url(engine.always_https))),
        "GoogleMeetOnebox" => google_meet(&https_url(engine.always_https), ctx),
        _ => Err(Unsupported("onebox engines that fetch the page")),
    }
}

/// What running an engine raised past its own rescues.
#[derive(Debug)]
pub enum Fail {
    /// One of Onebox::Preview's WEB_EXCEPTIONS: the HTML is blank.
    Web(String),
    /// Anything else Ruby raises: Oneboxer.onebox_raw gives up on the
    /// onebox (a blank one, uncached).
    Raised(String),
    /// A refusal or an error of this port's own.
    App(AppError),
}

impl From<AppError> for Fail {
    fn from(e: AppError) -> Fail {
        Fail::App(e)
    }
}

impl From<Unsupported> for Fail {
    fn from(e: Unsupported) -> Fail {
        Fail::App(e.into())
    }
}

/// What Onebox::Preview gets from an engine.
#[derive(Debug, Default)]
pub struct Rendered {
    /// `to_html`, None for nil (or a web error: both preview as "").
    pub html: Option<String>,
    /// What `to_html` raised past Preview's rescue, if it did.
    pub html_raised: Option<String>,
    /// `placeholder_html`
    pub placeholder: Option<String>,
    /// What `placeholder_html` raised past Preview's rescue.
    pub placeholder_raised: Option<String>,
    /// `errors.keys`
    pub errors: Vec<String>,
    /// `verified_data`, for the engines that define it.
    pub verified_data: Option<serde_json::Value>,
}

/// An engine's preview: `to_html`, `placeholder_html` and the errors they
/// left, fetching what the engine fetches.
pub async fn preview<F: Fetch>(
    engine: &Engine,
    url: &str,
    ctx: &Ctx<'_>,
    fetch: &mut F,
) -> Result<Rendered, Fail> {
    let result = match engine.name {
        "AllowlistedGenericOnebox" => super::generic::preview(url, ctx, fetch).await,
        name if EMBED_ENGINES.contains(&name) => {
            super::embed_engines::preview(engine, url, ctx, fetch).await
        }
        name if GITHUB_ENGINES.contains(&name) => {
            super::github::preview(engine, url, ctx, fetch).await
        }
        name if API_ENGINES.contains(&name) => {
            super::api_engines::preview(engine, url, ctx, fetch).await
        }
        _ => render(engine, url, ctx)
            .map(|h| Rendered {
                html: Some(h.html),
                placeholder: Some(h.placeholder),
                ..Default::default()
            })
            .map_err(Fail::from),
    };
    match result {
        Err(Fail::Web(_)) => Ok(Rendered::default()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropbox_shares_as_their_files() {
        assert_eq!(
            normalize_dropbox_url("https://www.dropbox.com/s/abc/movie.mp4"),
            "https://dl.dropboxusercontent.com/s/abc/movie.mp4"
        );
        assert_eq!(
            normalize_dropbox_url("https://www.dropbox.com/scl/fi/abc/movie.mp4?rlkey=x&dl=0"),
            "https://dl.dropboxusercontent.com/scl/fi/abc/movie.mp4?rlkey=x&dl=0&raw=1"
        );
        assert_eq!(
            normalize_dropbox_url("https://example.com/a.mp4"),
            "https://example.com/a.mp4"
        );
    }
}
