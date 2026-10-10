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
pub mod cooked_post_processor;
pub mod helpers;
pub use discourse_markdown::{render, sanitizer};

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
    /// Reading or writing a stored file (thumbnails).
    Io(std::io::Error),
}

impl std::fmt::Display for CookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CookError::Db(e) => write!(f, "cooking: {e}"),
            CookError::Setting(e) => e.fmt(f),
            CookError::Url(e) => e.fmt(f),
            CookError::Unsupported(e) => e.fmt(f),
            CookError::Io(e) => write!(f, "cooking: {e}"),
        }
    }
}
impl std::error::Error for CookError {}

impl From<std::io::Error> for CookError {
    fn from(e: std::io::Error) -> Self {
        CookError::Io(e)
    }
}

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

impl From<crate::file_store::StoreError> for CookError {
    fn from(e: crate::file_store::StoreError) -> Self {
        use crate::file_store::StoreError;
        match e {
            StoreError::Setting(e) => CookError::Setting(e),
            StoreError::Url(e) => CookError::Url(e),
            StoreError::Unsupported(e) => CookError::Unsupported(e),
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

impl Host {
    pub fn from_state(state: &crate::AppState) -> Host {
        Host {
            pool: state.pool.clone(),
            config: state.config.clone(),
            site_setting_defs: state.site_setting_defs.clone(),
            i18n: state.i18n.clone(),
        }
    }
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
    /// No `nofollow` on off-site links: the post's author is staff or
    /// trusted (`Post#omit_nofollow?`).
    pub omit_nofollow: bool,
    /// Chat::Message.markdown_options: chat's features and rules, and
    /// hashtags in the chat composer's context (channels first).
    pub chat: bool,
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
    // Upload urls in cooking are the store's; only the local one is ported.
    crate::file_store::FileStore::for_site(&host.config, settings)?;
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
    // Chat's channel data source, last in the topic composer.
    if settings.get("chat_enabled")?.truthy() && settings.get("enable_public_channels")?.truthy() {
        types.push("channel");
        icons.insert("channel".into(), json!("comment"));
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
    let mut hashtag_types = options["hashtagTypesInPriorityOrder"].clone();
    // The chat composer's order (register_hashtag_type_priority_for_context):
    // channel, category, tag.
    if opts.chat
        && let Some(types) = hashtag_types.as_array_mut()
    {
        types.sort_by_key(|t| match t.as_str() {
            Some("channel") => 0,
            Some("category") => 1,
            _ => 2,
        });
    }

    let mut render_settings = render_settings(&settings, &host.i18n, &host.config)?;
    render_settings.topic_id = opts.topic_id;
    render_settings.post_id = opts.post_id;
    render_settings.force_quote_link = opts.force_quote_link;
    if opts.chat {
        chat_render_settings(&mut render_settings);
    }

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
        opts.omit_nofollow,
    )
    .await
}

/// `RenderSettings` from the site settings: what PrettyText.buildOptions
/// reads of them.
pub fn render_settings(
    settings: &SiteSettings,
    i18n: &crate::i18n::I18n,
    config: &crate::config::Config,
) -> Result<render::RenderSettings, CookError> {
    let base_path = config.globals.relative_url_root();
    // The typographer's quotes are fixed in the crate's rule.
    if settings.get("markdown_typographer_quotation_marks")?.to_s() != "“|”|‘|’" {
        return Err(
            Unsupported("markdown_typographer_quotation_marks other than the default").into(),
        );
    }
    if settings.get("unicode_usernames")?.truthy() {
        return Err(Unsupported("unicode usernames in mentions").into());
    }
    // getURL on the emoji path: the base path in front, or the
    // external emoji host in its place.
    if config.globals.cdn_url().is_some() {
        return Err(Unsupported("emoji images behind a CDN").into());
    }
    let emoji_base_path = match settings.get("external_emoji_url")?.presence() {
        Some(external) => external,
        None => format!("{base_path}/images/emoji"),
    };
    let split = |name: &str| -> Result<Vec<String>, CookError> {
        Ok(settings
            .get(name)?
            .to_s()
            .split('|')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect())
    };
    let linkify_tlds = if settings.get("enable_markdown_linkify")?.truthy() {
        Some(split("markdown_linkify_tlds")?)
    } else {
        None
    };
    let mut avatar_sizes: Vec<i64> = split("avatar_sizes")?
        .iter()
        .map(|s| crate::ruby::to_i(s))
        .collect();
    avatar_sizes.sort_unstable();
    Ok(render::RenderSettings {
        breaks: !settings.get("traditional_markdown_linebreaks")?.truthy(),
        linkify_tlds,
        linkify: None,
        typographer: settings.get("enable_markdown_typographer")?.truthy(),
        mentions: settings.get("enable_mentions")?.truthy(),
        emoji: settings.get("enable_emoji")?.truthy(),
        emoji_shortcuts: settings.get("enable_emoji_shortcuts")?.truthy(),
        inline_emoji: settings.get("enable_inline_emoji_translation")?.truthy(),
        emoji_set: settings.get("emoji_set")?.to_s(),
        emoji_base_path,
        default_code_lang: settings.get("default_code_lang")?.to_s(),
        heading_anchor_label: i18n
            .t("js.post.heading_anchor")
            .unwrap_or("Heading link")
            .to_string(),
        // buildOptions keeps the iframe prefixes with a host and a path.
        allowed_iframes: split("allowed_iframes")?
            .into_iter()
            .filter(|s| s.matches('/').count() >= 3)
            .collect(),
        allowed_href_schemes: split("allowed_href_schemes")?,
        checklist: settings.get("checklist_enabled")?.truthy(),
        footnotes: settings.get("enable_markdown_footnotes")?.truthy(),
        poll: settings.get("poll_enabled")?.truthy(),
        poll_maximum_options: settings.get("poll_maximum_options")?.to_i(),
        poll_voters_label: i18n
            .t("js.poll.voters.other")
            .unwrap_or("voters")
            .to_string(),
        local_dates: settings.get("discourse_local_dates_enabled")?.truthy(),
        local_dates_email_format: settings
            .get("discourse_local_dates_email_format")?
            .to_s()
            .to_string(),
        local_dates_email_timezone: settings
            .get("discourse_local_dates_email_timezone")?
            .to_s()
            .to_string(),
        spoiler: settings.get("spoiler_enabled")?.truthy(),
        policy: settings.get("policy_enabled")?.truthy(),
        avatar_sizes,
        base_path: base_path.to_string(),
        secure_uploads: settings.get("secure_uploads")?.truthy(),
        topic_id: None,
        force_quote_link: false,
        post_id: None,
        chat: false,
    }
    .compiled()?)
}

/// `PrettyText.extract_mentions`: the usernames of the cooked HTML's
/// `.mention` and `.mention-group` elements whose text starts with "@",
/// normalized, without repeats.
pub fn extract_mentions(cooked: &str) -> Result<Vec<String>, crate::Unsupported> {
    let dom = cleanup::parse(cooked);
    let mut out: Vec<String> = Vec::new();
    for e in cleanup::all_elements(&dom) {
        if !(cleanup::has_class(&e, "mention") || cleanup::has_class(&e, "mention-group")) {
            continue;
        }
        let text = cleanup::text(&e);
        if let Some(name) = text.strip_prefix('@') {
            let name = crate::accounts::normalize_username(name)?;
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    Ok(out)
}

/// Chat::Message.markdown_options' features: chat's rules, and
/// MARKDOWN_FEATURES leaving out polls, checklists, footnotes and policies
/// while always including inlineEmoji.
pub fn chat_render_settings(render_settings: &mut render::RenderSettings) {
    render_settings.chat = true;
    render_settings.poll = false;
    render_settings.checklist = false;
    render_settings.footnotes = false;
    render_settings.policy = false;
    render_settings.inline_emoji = true;
}

#[cfg(test)]
mod mention_tests {
    use super::extract_mentions;

    #[test]
    fn mentions_come_from_mention_elements() {
        let cooked = r#"<p>Hi <a class="mention" href="/u/User1">@User1</a>, <a class="mention-group" href="/g/staff">@staff</a> and <span class="mention">@user1</span>; not <span class="mention">bob</span> or @carol</p>"#;
        assert_eq!(extract_mentions(cooked).unwrap(), vec!["user1", "staff"]);
    }
}
