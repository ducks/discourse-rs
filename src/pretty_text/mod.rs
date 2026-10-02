//! The pieces of lib/pretty_text.rb's markdown path that do not depend on
//! how the markdown itself is rendered: the options `PrettyText.markdown`
//! hands the renderer (`options`), and `PrettyText::Helpers`, the lookups
//! the rules make while cooking (`helpers`).
//!
//! The renderer is not here yet. Rails runs Discourse's JavaScript bundle;
//! discourse-rs will cook with Rust rules instead of vendoring that
//! bundle, measured against what Rails recorded (parity/pretty_text,
//! scripts/record-pretty-text).

pub mod cleanup;
pub mod helpers;
pub mod render;
pub mod sanitizer;

use std::sync::Arc;

use serde_json::{Map, Value, json};
use sqlx::{PgConnection, PgPool};

use crate::Unsupported;
use crate::avatar::AvatarError;
use crate::config::Config;
use crate::guardian::GuardianError;
use crate::i18n::I18n;
use crate::site_settings::{Definitions, SettingError, SiteSettings};
use crate::topic_list::TopicListError;
use crate::url::{UrlError, Urls};

#[derive(Debug)]
pub enum CookError {
    Db(sqlx::Error),
    Setting(SettingError),
    Url(UrlError),
    Unsupported(Unsupported),
}

impl std::fmt::Display for CookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CookError::Db(e) => write!(f, "cooking: {e}"),
            CookError::Setting(e) => e.fmt(f),
            CookError::Url(e) => e.fmt(f),
            CookError::Unsupported(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for CookError {}

impl From<sqlx::Error> for CookError {
    fn from(e: sqlx::Error) -> Self {
        CookError::Db(e)
    }
}

impl From<SettingError> for CookError {
    fn from(e: SettingError) -> Self {
        CookError::Setting(e)
    }
}

impl From<UrlError> for CookError {
    fn from(e: UrlError) -> Self {
        CookError::Url(e)
    }
}

impl From<Unsupported> for CookError {
    fn from(e: Unsupported) -> Self {
        CookError::Unsupported(e)
    }
}

impl From<GuardianError> for CookError {
    fn from(e: GuardianError) -> Self {
        match e {
            GuardianError::Db(e) => CookError::Db(e),
            GuardianError::Setting(e) => CookError::Setting(e),
            GuardianError::Unsupported(e) => CookError::Unsupported(e),
        }
    }
}

impl From<TopicListError> for CookError {
    fn from(e: TopicListError) -> Self {
        match e {
            TopicListError::Db(e) => CookError::Db(e),
            TopicListError::Setting(e) => CookError::Setting(e),
            TopicListError::Url(e) => CookError::Url(e),
            TopicListError::Unsupported(e) => CookError::Unsupported(e),
        }
    }
}

impl From<AvatarError> for CookError {
    fn from(e: AvatarError) -> Self {
        match e {
            AvatarError::Setting(e) => CookError::Setting(e),
            AvatarError::Url(e) => CookError::Url(e),
            AvatarError::Unsupported(e) => CookError::Unsupported(e),
        }
    }
}

/// What cooking reads from the application.
#[derive(Clone)]
pub struct Host {
    pub pool: PgPool,
    pub config: Config,
    pub site_setting_defs: Arc<Definitions>,
    pub i18n: Arc<I18n>,
}

/// The options of `PrettyText.markdown` that are ported.
#[derive(Debug, Clone, Default)]
pub struct MarkdownOptions {
    /// Topic id for the post being cooked.
    pub topic_id: Option<i64>,
    /// Post id for the post being cooked.
    pub post_id: Option<i64>,
    /// User id for the post being cooked.
    pub user_id: Option<i64>,
    /// Always link `[quote]`s to their topic, also inside that topic.
    pub force_quote_link: bool,
}

/// `opt_input` of `PrettyText.markdown`, without the per-call ids.
///
/// What plugins contribute on a Rails site is not produced: chat's
/// `additionalOptions` and its `channel` hashtag type, and discobot's
/// certificate among the allowed iframes. Settings whose values would
/// need more than is ported are refused.
pub async fn options(
    host: &Host,
    conn: &mut PgConnection,
    settings: &SiteSettings,
) -> Result<Map<String, Value>, CookError> {
    let urls = Urls {
        config: &host.config,
        settings,
    };
    if settings.get("enable_s3_uploads")?.truthy() {
        return Err(Unsupported("S3 upload paths in cooking").into());
    }
    let custom_emoji: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM custom_emojis)")
        .fetch_one(&mut *conn)
        .await?;
    if custom_emoji {
        return Err(Unsupported("custom emoji in cooking").into());
    }
    if !settings.get("emoji_deny_list")?.is_blank() {
        return Err(Unsupported("emoji_deny_list in cooking").into());
    }
    // WatchedWord.actions: censor 2, replace 5, link 8.
    let watched_words: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM watched_words WHERE action IN (2, 5, 8))")
            .fetch_one(&mut *conn)
            .await?;
    if watched_words {
        return Err(Unsupported("watched words in cooking").into());
    }
    let tagging = settings.get("tagging_enabled")?.truthy();

    let mut out = Map::new();
    out.insert(
        "siteSettings".into(),
        Value::Object(
            settings
                .client_settings_hash(&mut *conn, &host.site_setting_defs)
                .await?,
        ),
    );
    let iframes: Vec<String> = settings
        .get("allowed_iframes")?
        .to_s()
        .split('|')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    out.insert("allowedIframes".into(), json!(iframes));
    out.insert(
        "paths".into(),
        json!({
            "baseUri": host.config.globals.relative_url_root(),
            "CDN": urls.asset_host(),
        }),
    );
    out.insert("customEmoji".into(), json!({}));
    out.insert("customEmojiTranslation".into(), json!({}));
    out.insert("emojiDenyList".into(), Value::Null);
    out.insert("censoredRegexp".into(), json!([]));
    out.insert("watchedWordsReplace".into(), Value::Null);
    out.insert("watchedWordsLink".into(), Value::Null);
    out.insert("additionalOptions".into(), json!({}));
    out.insert(
        "avatar_sizes".into(),
        json!(settings.get("avatar_sizes")?.to_s()),
    );
    // HashtagAutocompleteService: core's data sources, tags while enabled.
    let mut types = vec!["category"];
    let mut icons = Map::new();
    icons.insert("category".into(), json!("folder"));
    if tagging {
        types.push("tag");
        icons.insert("tag".into(), json!("tag"));
    }
    out.insert("hashtagTypesInPriorityOrder".into(), json!(types));
    out.insert("hashtagIcons".into(), Value::Object(icons));
    Ok(out)
}

/// `PrettyText.markdown(text, opts)`: raw post text to HTML, before
/// `PrettyText.cleanup`.
///
/// The renderer cannot wait on the database, so it runs twice when the
/// text refers to anything outside itself: once to learn what it would
/// look up (quoted users and topics, hashtags, uploads), then, with those
/// resolved, for the result.
pub async fn markdown(host: &Host, raw: &str, opts: &MarkdownOptions) -> Result<String, CookError> {
    use render::context::{Hashtag, Lookups, TopicInfo, Upload};

    let mut conn = host.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &host.site_setting_defs, &host.config.globals).await?;
    // What `options` refuses (watched words, custom emoji...) the renderer
    // does not handle either.
    let options = options(host, &mut conn, &settings).await?;
    let hashtag_types = options["hashtagTypesInPriorityOrder"].clone();

    let mut render_settings =
        render::RenderSettings::from_site_settings(&settings, &host.i18n, &host.config)?;
    render_settings.topic_id = opts.topic_id;
    render_settings.post_id = opts.post_id;
    render_settings.force_quote_link = opts.force_quote_link;

    let (html, needs) = render::render(raw, &render_settings, Lookups::default())?;
    if needs.is_empty() {
        return Ok(html);
    }

    let text = |value: Value| value.as_str().unwrap_or_default().to_string();
    let mut lookups = Lookups::default();
    let mut helpers = helpers::Helpers {
        host,
        conn: &mut conn,
        settings: &settings,
    };
    for username in &needs.usernames {
        let avatar = helpers.avatar_template(Some(username)).await?;
        lookups.avatars.insert(username.clone(), text(avatar));
        let group = helpers.primary_user_group(Some(username)).await?;
        lookups.primary_groups.insert(username.clone(), text(group));
    }
    for id in &needs.topics {
        let info = helpers.topic_info(&json!(id)).await?;
        let info = info.as_object().map(|info| TopicInfo {
            title: text(info["title"].clone()),
            href: text(info["href"].clone()),
        });
        lookups.topics.insert(*id, info);
    }
    for slug in &needs.hashtags {
        let found = helpers
            .hashtag_lookup(&json!(slug), &json!(opts.user_id), &hashtag_types)
            .await?;
        let optional = |value: &Value| value.as_str().map(str::to_string);
        let found = found.as_object().map(|h| Hashtag {
            relative_url: text(h["relative_url"].clone()),
            text: text(h["text"].clone()),
            kind: text(h["type"].clone()),
            slug: text(h["slug"].clone()),
            reference: text(h["ref"].clone()),
            id: h["id"].as_i64().unwrap_or_default(),
            style_type: optional(&h["style_type"]),
            emoji: optional(&h["emoji"]),
            icon: optional(&h["icon"]),
        });
        lookups.hashtags.insert(slug.clone(), found);
    }
    if !needs.uploads.is_empty() {
        let found = helpers.upload_urls(&json!(needs.uploads)).await?;
        for (short_url, upload) in found.as_object().into_iter().flatten() {
            lookups.uploads.insert(
                short_url.clone(),
                Upload {
                    url: text(upload["url"].clone()),
                    short_path: text(upload["short_path"].clone()),
                    base62_sha1: text(upload["base62_sha1"].clone()),
                },
            );
        }
    }
    Ok(render::render(raw, &render_settings, lookups)?.0)
}

/// `PrettyText.cook(raw, opts)`: the markdown, then `PrettyText.cleanup`.
/// What the post processor adds after (oneboxes, image sizes) is not part
/// of it.
pub async fn cook(host: &Host, raw: &str, opts: &MarkdownOptions) -> Result<String, CookError> {
    let html = markdown(host, raw, opts).await?;
    let mut conn = host.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &host.site_setting_defs, &host.config.globals).await?;
    cleanup::cleanup(
        &mut conn,
        &settings,
        &host.config,
        &host.i18n,
        &html,
        opts.user_id,
    )
    .await
}
