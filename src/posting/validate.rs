//! What decides whether a post may be written: NewPostManager's review
//! queue, PostValidator (with its limits on new users), Post's
//! has_host_spam?, the Topic model's title and category validations, and
//! PostAnalyzer's counts over the cooked HTML they read.

use std::sync::LazyLock;

use markup5ever_rcdom::{Handle, NodeData};
use regex::Regex;
use sqlx::PgConnection;

use super::Ctx;
use super::text::{Sentinel, sanitized_length};
use crate::guardian::Guardian;
use crate::pretty_text::cleanup::{attr, element_name, fragment_root, has_class, parse, text};
use crate::{AppError, Unsupported};

/// Why `NewPostManager.post_needs_approval?` sends a post to the review
/// queue: the needs_approval score's reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    PostCount,
    Group,
    NewTopicsUnlessAllowedGroups,
    WatchedWord,
    FastTyper,
    AutoSilenceRegex,
    Category,
    ContainsMedia,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::PostCount => "post_count",
            Reason::Group => "group",
            Reason::NewTopicsUnlessAllowedGroups => "new_topics_unless_allowed_groups",
            Reason::WatchedWord => "watched_word",
            Reason::FastTyper => "fast_typer",
            Reason::AutoSilenceRegex => "auto_silence_regex",
            Reason::Category => "category",
            Reason::ContainsMedia => "contains_media",
        }
    }
}

/// What the review queue reads of a new post (NewPostManager's args).
pub struct ApprovalArgs<'a> {
    pub raw: &'a str,
    /// `args[:title]`, as sent.
    pub title: Option<&'a str>,
    /// `args[:topic_id]`: a reply's topic.
    pub topic_id: Option<i32>,
    /// `args[:category]`: a new topic's category, when given.
    pub category_id: Option<i32>,
    pub typing_duration_msecs: i64,
    pub first_post_checks: bool,
}

/// `NewPostManager.post_needs_approval?` for a post that isn't a message
/// (messages are never queued), with what `default_handler` silences the
/// author for once it is queued.
#[derive(Debug, Default)]
pub struct Approval {
    pub reason: Option<Reason>,
    /// `is_fast_typer?`
    pub fast_typer: bool,
    /// `auto_silence?` (watched silence words) or `matches_auto_silence_regex?`.
    pub auto_silence: bool,
}

/// Refuses what isn't ported: staged authors.
pub async fn post_needs_approval(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    args: &ApprovalArgs<'_>,
) -> Result<Approval, AppError> {
    let s = ctx.settings;
    let user = guardian.user().ok_or(Unsupported("posting anonymously"))?;
    // exempt_user?
    if guardian.is_staff() {
        return Ok(Approval::default());
    }
    refuse_unported_author(guardian)?;
    let (post_count, topic_count): (i32, i32) =
        sqlx::query_as("SELECT post_count, topic_count FROM user_stats WHERE user_id = $1")
            .bind(user.id)
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or((0, 0));
    // is_first_post?
    let first_post = args.first_post_checks && post_count == 0 && topic_count == 0;
    let fast_typer = first_post && {
        let threshold = match s.get("fast_typing_threshold")?.to_s().as_str() {
            "disabled" => 0,
            "low" => 1000,
            "standard" => 3000,
            "high" => 5000,
            _ => return Err(Unsupported("unknown fast_typing_threshold").into()),
        };
        args.typing_duration_msecs < threshold
            && s.get("auto_silence_fast_typers_on_first_post")?.truthy()
            && i64::from(user.trust_level)
                <= s.get("auto_silence_fast_typers_max_trust_level")?.to_i()
    };
    // WordWatcher over "#{title} #{raw}".
    let text = format!("{} {}", args.title.unwrap_or(""), args.raw);
    use crate::word_watcher as ww;
    let approval_words = ww::compiled_for_action(&mut *conn, s, ww::REQUIRE_APPROVAL).await?;
    let requires_approval = !ww::matches(&text, &approval_words, false)?.is_empty();
    // auto_silence? or matches_auto_silence_regex?
    let auto_silence = first_post && {
        let silence_words = ww::compiled_for_action(&mut *conn, s, ww::SILENCE).await?;
        !ww::matches(&text, &silence_words, false)?.is_empty()
            || matches_auto_silence_regex(s, args)?
    };
    let reason = if user.trust_level <= 1
        && i64::from(post_count + topic_count) < s.get("approve_post_count")?.to_i()
    {
        Some(Reason::PostCount)
    } else if !guardian.in_setting_groups(s, "approve_unless_allowed_groups")? {
        Some(Reason::Group)
    } else if args.title.is_some_and(|t| !t.trim().is_empty())
        && !guardian.in_setting_groups(s, "approve_new_topics_unless_allowed_groups")?
    {
        Some(Reason::NewTopicsUnlessAllowedGroups)
    } else if requires_approval {
        Some(Reason::WatchedWord)
    } else if fast_typer {
        Some(Reason::FastTyper)
    } else if auto_silence {
        Some(Reason::AutoSilenceRegex)
    } else if needs_approval_in_its_category(&mut *conn, s, guardian, args).await? {
        Some(Reason::Category)
    } else if !guardian.in_setting_groups(s, "skip_review_media_groups")?
        && contains_embedded_media(ctx, args).await?
    {
        Some(Reason::ContainsMedia)
    } else {
        None
    };
    Ok(Approval {
        reason,
        fast_typer,
        auto_silence,
    })
}

/// Authors whose posts aren't ported: staged users.
pub fn refuse_unported_author(guardian: &Guardian) -> Result<(), AppError> {
    let user = guardian.user().ok_or(Unsupported("posting anonymously"))?;
    if guardian.is_staff() {
        return Ok(());
    }
    if user.staged {
        return Err(Unsupported("posting as a staged user").into());
    }
    Ok(())
}

/// `User#posted_too_much_in_topic?(post.topic_id)` for the acting user: a
/// trust level 0 user's posts in someone else's topic since the last
/// reply, mention or quote of theirs there, against
/// newuser_max_replies_per_topic.
async fn posted_too_much_in_topic(
    conn: &mut PgConnection,
    s: &crate::site_settings::SiteSettings,
    acting: &Guardian,
    post: &PostInput<'_>,
) -> Result<bool, AppError> {
    let Some(user) = acting.user() else {
        return Ok(false);
    };
    if acting.is_staff() || user.trust_level != 0 {
        return Ok(false);
    }
    // A new topic's first post, saved: the topic is the user's own.
    if post.first_post && post.topic_id.is_none() {
        return Ok(false);
    }
    if let Some(topic_id) = post.topic_id {
        let topic: Option<(String, Option<i32>)> = sqlx::query_as(
            "SELECT archetype, user_id FROM topics WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(topic_id)
        .fetch_optional(&mut *conn)
        .await?;
        if topic.is_some_and(|(archetype, owner)| {
            archetype == "private_message" || owner == Some(user.id)
        }) {
            return Ok(false);
        }
    }
    // UserAction.last_action_in_topic: RESPONSE, MENTION or QUOTE.
    let last_action: Option<Option<i32>> = sqlx::query_scalar(
        "SELECT target_post_id FROM user_actions \
         WHERE user_id = $1 AND target_topic_id IS NOT DISTINCT FROM $2 AND action_type IN (6, 7, 9) \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(user.id)
    .bind(post.topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let since_reply: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM posts \
         WHERE user_id = $1 AND topic_id IS NOT DISTINCT FROM $2 AND deleted_at IS NULL \
           AND ($3::int IS NULL OR id > $3)",
    )
    .bind(user.id)
    .bind(post.topic_id)
    .bind(last_action.flatten())
    .fetch_one(&mut *conn)
    .await?;
    Ok(since_reply >= s.get("newuser_max_replies_per_topic")?.to_i())
}

/// `Post#has_host_spam?`: a new (trust level 0) author linking to hosts
/// outside allowed_spam_host_domains and the site's own that their earlier
/// posts link to, newuser_spam_host_threshold times with this one. The
/// post's `linked_hosts` when it is spam, for the moderators' message.
pub async fn host_spam(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    acting: &Guardian,
    reply_in_message: bool,
    analysis: &Analysis,
    max_oneboxes: i64,
) -> Result<Option<Vec<String>>, AppError> {
    let s = ctx.settings;
    let Some(user) = acting.user() else {
        return Ok(None);
    };
    // staged?, mature_staged? (staged authors are refused) or
    // has_trust_level?(1).
    if acting.has_trust_level(1) || reply_in_message {
        return Ok(None);
    }
    let hosts = linked_hosts(analysis, max_oneboxes);
    // allowed_spam_hosts: the setting's entries with a dot,
    // GlobalSetting.hostname and the multisite's current hostname.
    let mut allowed: Vec<String> = s
        .get("allowed_spam_host_domains")?
        .to_s()
        .split('|')
        .map(|h| h.trim().to_string())
        .filter(|h| h.contains('.'))
        .collect();
    allowed.push(
        ctx.config
            .globals
            .hostname(ctx.config.rails_env)
            .to_string(),
    );
    allowed.push(ctx.config.hostname().to_string());
    let counted: Vec<String> = hosts
        .iter()
        .filter(|h| {
            !allowed
                .iter()
                .any(|w| *h == w || h.ends_with(&format!(".{w}")))
        })
        .cloned()
        .collect();
    if counted.is_empty() {
        return Ok(None);
    }
    // total_hosts_usage: one for this post, and one per earlier post of
    // the user's linking to the host.
    let usage: Vec<(String, i64)> = sqlx::query_as(
        "SELECT domain, COUNT(DISTINCT post_id) FROM topic_links \
         WHERE domain = ANY($1) AND user_id = $2 GROUP BY domain",
    )
    .bind(&counted)
    .bind(user.id)
    .fetch_all(&mut *conn)
    .await?;
    let threshold = s.get("newuser_spam_host_threshold")?.to_i();
    let spam = counted.iter().any(|h| {
        let earlier = usage.iter().find(|(d, _)| d == h).map_or(0, |(_, n)| *n);
        1 + earlier >= threshold
    });
    Ok(spam.then_some(hosts))
}

/// `PostAnalyzer#link_count`: `raw_links` and the oneboxes collected.
pub fn link_count(analysis: &Analysis, max_oneboxes: i64) -> usize {
    analysis.links.len() + onebox_urls(analysis, max_oneboxes).len()
}

/// `@onebox_urls`: the first max_oneboxes_per_post of them.
fn onebox_urls(analysis: &Analysis, max_oneboxes: i64) -> &[String] {
    let n = usize::try_from(max_oneboxes.max(0)).unwrap_or(usize::MAX);
    &analysis.onebox_urls[..analysis.onebox_urls.len().min(n)]
}

/// `PostAnalyzer#linked_hosts`: the hosts of `raw_links` and the oneboxes,
/// in order, each once, as `URI::RFC2396_Parser` parses them (links it
/// can't parse, and those without a host, left out).
pub fn linked_hosts(analysis: &Analysis, max_oneboxes: i64) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for url in analysis
        .links
        .iter()
        .chain(onebox_urls(analysis, max_oneboxes))
    {
        if let Some(host) = rfc2396_host(url)
            && !hosts.contains(&host)
        {
            hosts.push(host);
        }
    }
    hosts
}

/// `URI::RFC2396_Parser.new.parse(url).host`: None when the url doesn't
/// parse (characters RFC 2396 leaves out, a bad escape, brackets outside
/// an IPv6 host) or has no server-based authority (a relative or opaque
/// url, a registry name such as a host with an underscore).
pub fn rfc2396_host(url: &str) -> Option<String> {
    static HOSTNAME: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"\A(?:(?:[a-zA-Z0-9](?:[-a-zA-Z0-9]*[a-zA-Z0-9])?)\.)*[a-zA-Z](?:[-a-zA-Z0-9]*[a-zA-Z0-9])?\.?\z",
        )
        .unwrap()
    });
    static IPV4: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\A\d+\.\d+\.\d+\.\d+\z").unwrap());
    static IPV6: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\A\[[0-9a-fA-F:.]+\]\z").unwrap());
    // URI_REF: reserved, unreserved and escaped characters, one fragment;
    // RFC 2732's brackets only around an IPv6 host (checked below).
    let bytes = url.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'%' {
            if !(bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit))
            {
                return None;
            }
            i += 3;
            continue;
        }
        if !(c.is_ascii_alphanumeric() || b";/?:@&=+$,-_.!~*'()#[]".contains(&c)) {
            return None;
        }
        i += 1;
    }
    let (rest, fragment) = url.split_once('#').unwrap_or((url, ""));
    if fragment.contains('#') {
        return None;
    }
    // An absolute url's scheme, else a relative one (no host either way
    // without "//").
    let rest = match rest.find(':') {
        Some(i)
            if rest[..i]
                .bytes()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
                && rest[..i]
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"+-.".contains(&c)) =>
        {
            &rest[i + 1..]
        }
        _ => rest,
    };
    let authority = rest.strip_prefix("//")?;
    let authority = &authority[..authority.find(['/', '?']).unwrap_or(authority.len())];
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = match host_port.rsplit_once(':') {
        Some((host, port))
            if port.bytes().all(|c| c.is_ascii_digit()) && !host_port.ends_with(']') =>
        {
            host
        }
        _ => host_port,
    };
    let brackets = url.bytes().filter(|c| *c == b'[' || *c == b']').count();
    if brackets > 0 && !(brackets == 2 && IPV6.is_match(host)) {
        return None;
    }
    (HOSTNAME.is_match(host) || IPV4.is_match(host) || IPV6.is_match(host))
        .then(|| host.to_string())
}

/// `PostAnalyzer#attachment_count`: the `a.attachment` links under the
/// store's absolute base url, or (the local store) its relative one.
pub fn attachment_count(ctx: &Ctx<'_>, analysis: &Analysis) -> Result<usize, AppError> {
    if analysis.attachment_hrefs.is_empty() {
        return Ok(0);
    }
    let store = crate::file_store::FileStore::for_site(ctx.config, ctx.settings)?;
    let absolute = store.absolute_base_url();
    let relative = store.internal().then(|| store.relative_base_url());
    Ok(analysis
        .attachment_hrefs
        .iter()
        .filter(|h| h.starts_with(absolute) || relative.is_some_and(|r| h.starts_with(r)))
        .count())
}

/// `matches_auto_silence_regex?` (on a first post): the setting as a
/// case-insensitive Ruby regexp over the title and raw. A pattern the
/// port's regexp engine can't compile is refused, not guessed at.
fn matches_auto_silence_regex(
    s: &crate::site_settings::SiteSettings,
    args: &ApprovalArgs<'_>,
) -> Result<bool, AppError> {
    let pattern = s.get("auto_silence_first_post_regex")?.to_s();
    if pattern.trim().is_empty() {
        return Ok(false);
    }
    let regex = fancy_regex::Regex::new(&format!("(?i){pattern}"))
        .map_err(|_| Unsupported("an auto_silence_first_post_regex the port can't compile"))?;
    let text = format!("{} {}", args.title.unwrap_or(""), args.raw);
    regex
        .is_match(&text)
        .map_err(|_| Unsupported("auto_silence_first_post_regex backtracking limit").into())
}

/// `post_needs_approval_in_its_category?`: the reply's topic's category
/// reviews replies, or the new topic's category reviews topics, for this
/// user. Category group moderators (who skip it) are refused.
async fn needs_approval_in_its_category(
    conn: &mut PgConnection,
    s: &crate::site_settings::SiteSettings,
    guardian: &Guardian,
    args: &ApprovalArgs<'_>,
) -> Result<bool, AppError> {
    let (category_id, post_type) = if let Some(topic_id) = args.topic_id {
        let category: Option<Option<i32>> =
            sqlx::query_scalar("SELECT category_id FROM topics WHERE id = $1")
                .bind(topic_id)
                .fetch_optional(&mut *conn)
                .await?;
        match category.flatten() {
            Some(id) => (id, REVIEW_REPLY),
            None => return Ok(false),
        }
    } else if let Some(id) = args.category_id {
        (id, REVIEW_TOPIC)
    } else {
        return Ok(false);
    };
    let required = posting_review_required(&mut *conn, guardian, category_id, post_type).await?;
    if required && s.get("enable_category_group_moderation")?.truthy() {
        return Err(Unsupported("category posting review with category group moderation").into());
    }
    Ok(required)
}

/// `CategoryPostingReviewGroup.post_types`
pub const REVIEW_TOPIC: i32 = 0;
pub const REVIEW_REPLY: i32 = 1;

/// `CategoryGuardian#posting_review_required?(category, :topic | :reply)`
pub async fn posting_review_required(
    conn: &mut PgConnection,
    guardian: &Guardian,
    category_id: i32,
    post_type: i32,
) -> Result<bool, AppError> {
    let column = if post_type == REVIEW_TOPIC {
        "topic_posting_review_mode"
    } else {
        "reply_posting_review_mode"
    };
    let mode: Option<i32> = sqlx::query_scalar(&format!(
        "SELECT {column} FROM category_settings WHERE category_id = $1"
    ))
    .bind(category_id)
    .fetch_optional(&mut *conn)
    .await?;
    let mode = mode.unwrap_or(0);
    if mode != 2 && mode != 3 {
        return Ok(mode == 1);
    }
    // CategoryPostingReviewGroup.user_in_group?: everyone, or one of the
    // user's groups.
    let mut groups = vec![crate::guardian::auto_groups::EVERYONE as i32];
    groups.extend(guardian.group_ids().iter().map(|g| *g as i32));
    let in_group: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM category_posting_review_groups \
         WHERE category_id = $1 AND post_type = $2 AND group_id = ANY($3))",
    )
    .bind(category_id)
    .bind(post_type)
    .bind(&groups)
    .fetch_one(&mut *conn)
    .await?;
    Ok(if mode == 2 { !in_group } else { in_group })
}

/// `contains_embedded_media?(args)`: the raw, cooked as `Post.new(raw:,
/// topic_id:)` would, embeds media (PostAnalyzer#embedded_media_count).
async fn contains_embedded_media(ctx: &Ctx<'_>, args: &ApprovalArgs<'_>) -> Result<bool, AppError> {
    let cooked = crate::pretty_text::cook(
        ctx.host,
        args.raw,
        &crate::pretty_text::MarkdownOptions {
            topic_id: args.topic_id.map(i64::from),
            post_id: None,
            user_id: None,
            force_quote_link: false,
            omit_nofollow: false,
            chat: false,
        },
    )
    .await?;
    let base_path = ctx.config.globals.relative_url_root().to_string();
    Ok(analyze(&cooked, &base_path)?.embedded_media > 0)
}

/// PostAnalyzer over a cooked post, after PostStripper.
#[derive(Debug, Default)]
pub struct Analysis {
    pub mentions: usize,
    /// `raw_mentions`: the mentioned names, lowercased, deduplicated.
    pub mention_names: Vec<String>,
    pub embedded_media: usize,
    /// The hrefs of `a.attachment` links; `attachment_count` keeps those
    /// under the store's base urls.
    pub attachment_hrefs: Vec<String>,
    /// `raw_links`: every link but mentions, anchors and hashtags.
    pub links: Vec<String>,
    /// The hrefs of the `a.onebox` links (class exactly "onebox") that
    /// PostAnalyzer#cook collects as `@onebox_urls`, before
    /// max_oneboxes_per_post is applied.
    pub onebox_urls: Vec<String>,
    pub has_oneboxes: bool,
    /// The links cooking left for oneboxing: each href, and whether the
    /// onebox is an inline one (`.inline-onebox-loading`).
    pub onebox_links: Vec<(String, bool)>,
    /// A `div.poll`: one of discourse-poll's polls.
    pub has_polls: bool,
    pub has_quotes: bool,
}

/// `PostAnalyzer` counts, refusing what cooking leaves for oneboxing
/// that the post processor can't onebox (see `onebox::refuse_unported`).
pub fn analyze(cooked: &str, base_path: &str) -> Result<Analysis, Unsupported> {
    analyze_stripped(cooked, base_path, false)
}

/// `analyze` with the enabled plugins' post strippers: chat's
/// (`.chat-transcript .mention` removed) when `chat` is on.
pub fn analyze_stripped(
    cooked: &str,
    base_path: &str,
    chat: bool,
) -> Result<Analysis, Unsupported> {
    let dom = parse(cooked);
    let mut a = Analysis::default();
    let mut mentions = Vec::new();
    walk(
        &fragment_root(&dom),
        base_path,
        Strip {
            quoted: false,
            chat,
            in_transcript: false,
        },
        &mut a,
        &mut mentions,
    );
    mentions.sort();
    mentions.dedup();
    a.mentions = mentions.len();
    a.mention_names = mentions;
    Ok(a)
}

/// Where the walk is, for PostStripper: under a quote or `pre` (its
/// mentions removed), and under a chat transcript while chat's stripper
/// runs.
#[derive(Clone, Copy)]
struct Strip {
    quoted: bool,
    chat: bool,
    in_transcript: bool,
}

impl Strip {
    fn quote(self) -> Strip {
        Strip {
            quoted: true,
            ..self
        }
    }
}

/// PostStripper removes mentions in code and quotes, quote titles,
/// oneboxes and elided parts before anything is counted.
fn walk(node: &Handle, bp: &str, stripped: Strip, a: &mut Analysis, mentions: &mut Vec<String>) {
    let name = element_name(node);
    let stripped = Strip {
        in_transcript: stripped.in_transcript
            || (stripped.chat && name.is_some() && has_class(node, "chat-transcript")),
        ..stripped
    };
    if let Some(name) = name {
        let inline = has_class(node, "inline-onebox-loading");
        if has_class(node, "onebox") || inline {
            a.has_oneboxes = true;
            if name == "a"
                && let Some(href) = attr(node, "href").filter(|h| !h.is_empty())
            {
                if attr(node, "class").as_deref() == Some("onebox") {
                    a.onebox_urls.push(href.clone());
                }
                a.onebox_links.push((href, inline));
            }
        }
        if name == "div" && has_class(node, "poll") {
            a.has_polls = true;
        }
        if has_class(node, "elided") || has_class(node, "onebox") {
            return;
        }
        if name == "aside" && has_class(node, "quote") {
            a.has_quotes = true;
            for child in node.children.borrow().iter() {
                if has_class(child, "title") {
                    continue;
                }
                walk(child, bp, stripped.quote(), a, mentions);
            }
            return;
        }
        let mention = has_class(node, "mention") || has_class(node, "mention-group");
        if (mention && stripped.quoted) || (stripped.in_transcript && has_class(node, "mention")) {
            return;
        }
        if mention {
            let t = text(node);
            if let Some(name) = t.strip_prefix('@') {
                mentions.push(name.to_lowercase());
            }
        }
        match name {
            "img" | "video" | "audio" => {
                let allowed = [
                    "avatar",
                    "favicon",
                    "thumbnail",
                    "emoji",
                    "ytp-thumbnail-image",
                ];
                let class = attr(node, "class").unwrap_or_default();
                if !class.split_whitespace().any(|c| allowed.contains(&c)) {
                    a.embedded_media += 1;
                }
            }
            "a" => {
                let href = attr(node, "href").unwrap_or_default();
                if has_class(node, "attachment") {
                    a.attachment_hrefs.push(href.clone());
                }
                let class = attr(node, "class").unwrap_or_default();
                let is_mention = class.contains("mention")
                    && (href.starts_with(&format!("{bp}/u/"))
                        || href.starts_with(&format!("{bp}/users/")));
                let is_anchor = class.contains("anchor") && href.starts_with('#');
                let is_hashtag = class.contains("hashtag")
                    && (href.starts_with(&format!("{bp}/c/"))
                        || href.starts_with(&format!("{bp}/tag/")));
                if !is_mention && !is_anchor && !is_hashtag {
                    a.links.push(href);
                }
            }
            _ => {}
        }
        if name == "pre" {
            for child in node.children.borrow().iter() {
                walk(child, bp, stripped.quote(), a, mentions);
            }
            return;
        }
    }
    let html = &node.data;
    if let NodeData::Element { .. } | NodeData::Document = html {
        for child in node.children.borrow().iter() {
            walk(child, bp, stripped, a, mentions);
        }
    }
}

/// `Post#unique_post_key`: the author, the topic's kind and `raw_hash`.
fn unique_post_key(user_id: i32, private_message: bool, raw: &str) -> String {
    use sha1::{Digest, Sha1};
    format!(
        "unique{}-post-{user_id}:{:x}",
        if private_message { "-pm" } else { "" },
        Sha1::digest(raw.as_bytes())
    )
}

/// `Post#store_unique_post_key`, as PostCreator calls it once a post is
/// created (Redis's SETEX for unique_posts_mins).
pub async fn store_unique_post_key(
    conn: &mut PgConnection,
    settings: &crate::site_settings::SiteSettings,
    user_id: i32,
    private_message: bool,
    raw: &str,
    post_id: i32,
) -> Result<(), AppError> {
    let mins = settings.get("unique_posts_mins")?.to_i();
    if mins > 0 {
        let key = unique_post_key(user_id, private_message, raw);
        crate::owned_schema::cached_setex(conn, &key, mins * 60, &post_id.to_string()).await?;
    }
    Ok(())
}

/// The post a validator looks at.
pub struct PostInput<'a> {
    pub raw: &'a str,
    pub topic_id: Option<i32>,
    /// The first post of its topic (a new topic, or post 1 being edited).
    pub first_post: bool,
    pub private_message: bool,
    pub new_record: bool,
    pub post_id: Option<i32>,
    pub user_id: i32,
}

/// `PostValidator#validate` for a non-staged acting user, returning
/// `errors.full_messages`.
pub async fn validate_post(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    acting: &Guardian,
    post: &PostInput<'_>,
    analysis: &Analysis,
) -> Result<Vec<String>, AppError> {
    let body = body_errors(&mut *conn, ctx, acting, post).await?;
    if !body.is_empty() {
        return Ok(body
            .iter()
            .map(|e| match e {
                BodyError::Raw(m) => ctx.full_message("activerecord.attributes.post.raw", m),
                BodyError::Base(m) => m.clone(),
            })
            .collect());
    }
    base_errors(conn, ctx, acting, post, analysis).await
}

/// An error of PostValidator's first validators: on `raw` (a message
/// without the attribute) or on base (watched words that block).
pub enum BodyError {
    Raw(String),
    Base(String),
}

/// PostValidator's `errors[:raw]`, without the attribute.
pub async fn raw_errors(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    acting: &Guardian,
    post: &PostInput<'_>,
) -> Result<Vec<String>, AppError> {
    Ok(body_errors(conn, ctx, acting, post)
        .await?
        .into_iter()
        .filter_map(|e| match e {
            BodyError::Raw(m) => Some(m),
            BodyError::Base(_) => None,
        })
        .collect())
}

/// PostValidator's first validators: post_body_validator (stripped_length,
/// raw_quality, WatchedWordsValidator) and unique_post_validator. The
/// other validators only run once these pass.
pub async fn body_errors(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    acting: &Guardian,
    post: &PostInput<'_>,
) -> Result<Vec<BodyError>, AppError> {
    let s = ctx.settings;
    let mut errors = Vec::new();

    // post_body_validator: stripped_length then raw_quality.
    let max = s.get("max_post_length")?.to_i();
    let min = if post.private_message {
        s.get("min_personal_message_post_length")?.to_i()
    } else if post.first_post {
        s.get("min_first_post_length")?.to_i()
    } else {
        s.get("min_post_length")?.to_i()
    };
    let raw_chars = post.raw.chars().count() as i64;
    if post.raw.trim().is_empty() && min > 0 {
        errors.push(BodyError::Raw(ctx.t("errors.messages.blank")));
    } else if raw_chars > max {
        let form = if max == 1 { "one" } else { "other" };
        let message = ctx
            .i18n
            .t_with(
                &format!("errors.messages.too_long_validation.{form}"),
                &[
                    ("count", &max.to_string()),
                    ("length", &raw_chars.to_string()),
                ],
            )
            .unwrap_or_default();
        errors.push(BodyError::Raw(message));
    } else {
        let strip_uploads = s.get("prevent_uploads_only_posts")?.truthy();
        if (sanitized_length(post.raw, strip_uploads) as i64) < min {
            let message = ctx.t_count("errors.messages.too_short", min);
            errors.push(BodyError::Raw(message));
        }
    }
    // TextSentinel.body_sentinel
    let mut entropy = s.get("body_min_entropy")?.to_i();
    if post.private_message {
        let pm_min = s.get("min_personal_message_post_length")?.to_i();
        let post_min = s.get("min_post_length")?.to_i().max(1);
        entropy = (entropy as f64 * (pm_min as f64 / post_min as f64)) as i64;
        if entropy > pm_min {
            entropy = (pm_min as f64 * 0.7) as i64;
        }
    } else {
        let post_min = s.get("min_post_length")?.to_i();
        if entropy > post_min {
            entropy = (post_min as f64 * 0.7) as i64;
        }
    }
    let sentinel = Sentinel {
        text: post.raw,
        min_entropy: Some(entropy),
        max_word_length: None,
        allow_uppercase_posts: s.get("allow_uppercase_posts")?.truthy(),
        skip_word_length: false,
    };
    if !sentinel.valid() {
        errors.push(BodyError::Raw(ctx.t("is_invalid")));
    }
    // WatchedWordsValidator (staged authors are refused).
    if let Some(message) =
        crate::word_watcher::block_error(&mut *conn, s, ctx.i18n, post.raw).await?
    {
        errors.push(BodyError::Base(message));
    }

    // max_posts_validator
    if post.new_record && posted_too_much_in_topic(&mut *conn, s, acting, post).await? {
        let max = s.get("newuser_max_replies_per_topic")?.to_i();
        errors.push(BodyError::Base(ctx.t_count("too_many_replies", max)));
    }

    // unique_post_validator: `matches_recent_post?`, the key PostCreator
    // stored for a post created with this raw (an edit's new raw included)
    // naming another post.
    let unique_mins = s.get("unique_posts_mins")?.to_i();
    if unique_mins > 0 && !acting.is_staff() && !post.raw.trim().is_empty() {
        let key = unique_post_key(post.user_id, post.private_message, post.raw);
        let recent = crate::owned_schema::cached_get(&mut *conn, &key).await?;
        if recent.is_some_and(|id| Some(crate::ruby::to_i(&id) as i32) != post.post_id) {
            errors.push(BodyError::Raw(ctx.t("just_posted_that")));
        }
    }
    Ok(errors)
}

/// PostValidator's other validators, once `raw_errors` found none:
/// `errors.full_messages` (all on base).
pub async fn base_errors(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    acting: &Guardian,
    post: &PostInput<'_>,
    analysis: &Analysis,
) -> Result<Vec<String>, AppError> {
    let s = ctx.settings;
    let mut errors = Vec::new();
    let acting_user = acting
        .user()
        .ok_or(Unsupported("validating without a user"))?;
    // acting_user_is_trusted?: has_trust_level?(1), staff included.
    let trusted = acting.has_trust_level(1);
    // add_error_if_count_exceeded
    let exceeded =
        |errors: &mut Vec<String>, none_key: &str, limit_key: &str, count: usize, max: i64| {
            if count as i64 > max {
                errors.push(if max == 0 {
                    ctx.t(none_key)
                } else {
                    ctx.t_count(limit_key, max)
                });
            }
        };

    if !acting.is_staff() {
        // max_mention_validator
        if trusted || post.private_message {
            let max = s.get("max_mentions_per_post")?.to_i();
            exceeded(
                &mut errors,
                "no_mentions_allowed",
                "too_many_mentions",
                analysis.mentions,
                max,
            );
        } else {
            let max = s.get("newuser_max_mentions_per_post")?.to_i();
            exceeded(
                &mut errors,
                "no_mentions_allowed_newuser",
                "too_many_mentions_newuser",
                analysis.mentions,
                max,
            );
        }
        // max_embedded_media_validator
        if !acting.in_setting_groups(s, "embedded_media_post_allowed_groups")? {
            exceeded(
                &mut errors,
                "no_embedded_media_allowed_group",
                "no_embedded_media_allowed_group",
                analysis.embedded_media,
                0,
            );
        } else if acting_user.trust_level == 0 {
            let max = s.get("newuser_max_embedded_media")?.to_i();
            exceeded(
                &mut errors,
                "no_embedded_media_allowed",
                "too_many_embedded_media",
                analysis.embedded_media,
                max,
            );
        }
    }
    // max_attachments_validator
    if !trusted && !post.private_message {
        let max = s.get("newuser_max_attachments")?.to_i();
        let count = attachment_count(ctx, analysis)?;
        exceeded(
            &mut errors,
            "no_attachments_allowed",
            "too_many_attachments",
            count,
            max,
        );
    }
    // max_links_validator: every linked host one the user may post
    // (post_links_allowed_groups, or allowed_link_domains), then
    // newuser_links_validator. has_oneboxes? only finds the oneboxes the
    // link count holds.
    let max_oneboxes = s.get("max_oneboxes_per_post")?.to_i();
    let links = link_count(analysis, max_oneboxes);
    let hosts_allowed = links == 0 || post.private_message || {
        let unrestricted = acting.is_authenticated()
            && acting.in_setting_groups(s, "post_links_allowed_groups")?;
        let domains = s.get("allowed_link_domains")?.to_s();
        linked_hosts(analysis, max_oneboxes)
            .iter()
            .all(|h| !h.is_empty() && (unrestricted || domains.split('|').any(|d| d == h)))
    };
    if !hosts_allowed {
        errors.push(ctx.t("links_require_trust"));
    } else if !trusted && !post.private_message {
        let max = s.get("newuser_max_links")?.to_i();
        exceeded(
            &mut errors,
            "no_links_allowed",
            "too_many_links",
            links,
            max,
        );
    }
    // max_quotes_validator
    let max_quotes = s.get("max_quotes_per_post")?.to_i();
    if max_quotes > 0 {
        static QUOTES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\[quote[=\]]").unwrap());
        let count = QUOTES.find_iter(post.raw).count() as i64;
        if count > max_quotes {
            errors.push(ctx.t_count("too_many_quotes", max_quotes));
        }
    }
    // force_edit_last_validator
    if post.new_record && !post.private_message && !acting.is_staff() {
        let max_replies = s.get("max_consecutive_replies")?.to_i();
        if max_replies > 0
            && let Some(topic_id) = post.topic_id
        {
            // topic.ordered_posts (Trashable's default scope leaves the
            // deleted out): the first's author and the last.
            let (op_user, category_id, count, last_post): (
                Option<i32>,
                Option<i32>,
                i64,
                Option<i32>,
            ) = sqlx::query_as(
                "SELECT (SELECT user_id FROM posts WHERE topic_id = $1 AND deleted_at IS NULL \
                           ORDER BY post_number LIMIT 1), \
                        (SELECT category_id FROM topics WHERE id = $1), \
                   (SELECT COUNT(*) FROM (SELECT user_id FROM posts WHERE deleted_at IS NULL AND NOT hidden \
                      AND topic_id = $1 ORDER BY post_number DESC LIMIT $3) c WHERE c.user_id = $2), \
                   (SELECT id FROM posts WHERE topic_id = $1 AND deleted_at IS NULL \
                      ORDER BY post_number DESC LIMIT 1)",
            )
            .bind(topic_id)
            .bind(acting_user.id)
            .bind(max_replies)
            .fetch_one(&mut *conn)
            .await?;
            // is_category_group_moderator?(topic.category)
            let group_moderator = match category_id {
                Some(cid) if acting.category_group_moderation_allowed(s)? => {
                    match acting.category_group_moderator_sql() {
                        Some(moderated) => {
                            sqlx::query_scalar::<_, bool>(&format!("SELECT $1 IN ({moderated})"))
                                .bind(cid)
                                .fetch_one(&mut *conn)
                                .await?
                        }
                        None => false,
                    }
                }
                _ => false,
            };
            if op_user != Some(post.user_id) && !group_moderator && count >= max_replies {
                // guardian.can_edit?(topic.ordered_posts.last)
                let can_edit = match last_post {
                    Some(last) => {
                        match crate::posting::revisions::find_post(&mut *conn, ctx, acting, last)
                            .await?
                        {
                            Some(access) => access.can_edit(ctx, acting)?,
                            None => false,
                        }
                    }
                    None => false,
                };
                if can_edit {
                    errors.push(ctx.t_count("max_consecutive_replies", max_replies));
                }
            }
        }
    }
    Ok(errors)
}

/// The topic attributes the Topic model validates.
pub struct TopicInput<'a> {
    pub title: &'a str,
    pub category_id: Option<i32>,
    pub private_message: bool,
}

/// What differs when validating a saved topic rather than a new one.
pub struct Persisted {
    pub topic_id: i32,
    /// `record.user.try(:admin?)`: the topic's author, not the editor.
    pub author_admin: bool,
    /// `title_changed? || category_id_changed?`
    pub check_title: bool,
    /// `category_id_changed?`
    pub check_category: bool,
}

/// Topic's `validates :title` and `validates :category_id`, after the
/// before_validation cleanup; `errors.full_messages`.
pub async fn validate_topic(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    acting: &Guardian,
    topic: &TopicInput<'_>,
) -> Result<Vec<String>, AppError> {
    validate_topic_with(conn, ctx, acting, topic, None).await
}

/// [`validate_topic`] for a new topic (`persisted` None) or a saved one,
/// whose title is validated only when it or the category changed and
/// whose duplicates leave the topic itself out.
pub async fn validate_topic_with(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    acting: &Guardian,
    topic: &TopicInput<'_>,
    persisted: Option<&Persisted>,
) -> Result<Vec<String>, AppError> {
    let s = ctx.settings;
    let mut errors = Vec::new();
    let title_attr = "activerecord.attributes.topic.title";
    let title = topic.title;
    let author_admin = persisted.map_or(acting.is_admin(), |p| p.author_admin);
    if persisted.is_some_and(|p| !p.check_title) {
        // An unchanged title in an unchanged category isn't validated.
    } else if title.trim().is_empty() {
        errors.push(ctx.full_message(title_attr, &ctx.t("errors.messages.blank")));
    } else {
        let (min, max) = if author_admin {
            (1, s.get("max_topic_title_length")?.to_i())
        } else if topic.private_message {
            (
                s.get("min_personal_message_title_length")?.to_i(),
                s.get("max_topic_title_length")?.to_i(),
            )
        } else {
            (
                s.get("min_topic_title_length")?.to_i(),
                s.get("max_topic_title_length")?.to_i(),
            )
        };
        let len = title.chars().count() as i64;
        if len > max {
            errors
                .push(ctx.full_message(title_attr, &ctx.t_count("errors.messages.too_long", max)));
        } else if len < min {
            errors
                .push(ctx.full_message(title_attr, &ctx.t_count("errors.messages.too_short", min)));
        }
        // censored_words, then watched_words (on base).
        let censor =
            crate::word_watcher::compiled_for_action(&mut *conn, s, crate::word_watcher::CENSOR)
                .await?;
        if let Some(words) = crate::word_watcher::censored_words(title, &censor)? {
            let message = ctx
                .i18n
                .t_with(
                    "errors.messages.contains_censored_words",
                    &[("censored_words", &words)],
                )
                .unwrap_or_default();
            errors.push(ctx.full_message(title_attr, &message));
        }
        if let Some(message) =
            crate::word_watcher::block_error(&mut *conn, s, ctx.i18n, title).await?
        {
            errors.push(message);
        }
        // quality_title
        if !topic.private_message {
            let min_length = s.get("min_topic_title_length")?.to_i();
            let entropy = s.get("title_min_entropy")?.to_i();
            let locale = s.get("default_locale")?.to_s();
            let sentinel = Sentinel {
                text: title,
                min_entropy: Some(if min_length > entropy {
                    entropy
                } else {
                    (min_length as f64 * 0.7) as i64
                }),
                max_word_length: Some(s.get("title_max_word_length")?.to_i()),
                allow_uppercase_posts: s.get("allow_uppercase_posts")?.truthy(),
                skip_word_length: matches!(locale.as_str(), "ja" | "ko" | "zh_CN" | "zh_TW"),
            };
            if !sentinel.valid() {
                let key = if !sentinel.seems_meaningful() {
                    "errors.messages.is_invalid_meaningful"
                } else if !sentinel.seems_unpretentious() {
                    "errors.messages.is_invalid_unpretentious"
                } else if !sentinel.seems_quiet() {
                    "errors.messages.is_invalid_quiet"
                } else {
                    "errors.messages.is_invalid"
                };
                errors.push(ctx.full_message(title_attr, &ctx.t(key)));
            }
        }
        // MaxEmojisValidator: the emoji images
        // PrettyText.unescape_emoji(Emoji.unicode_unescape(CGI.escapeHTML(title)))
        // holds, against max_emojis_in_title.
        let escaped = crate::posting::text::escape_emoji(
            &crate::discourse_diff::escape_html(title),
            crate::posting::text::EmojiEscape::from_settings(s)?,
        );
        if crate::emoji::has_emoji_code(&escaped) {
            static EMOJI_IMG: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                regex::Regex::new(r"<img.+?class\s*=\s*'(emoji|emoji emoji-custom)'")
                    .expect("the emoji image pattern")
            });
            let base_path = ctx.config.globals.relative_url_root();
            let unescaped = crate::topic_list_view::unescape_emoji(&escaped, s.emojis(), base_path);
            let count = EMOJI_IMG.find_iter(&unescaped).count() as i64;
            let max = s.get("max_emojis_in_title")?.to_i();
            if count > max {
                let message = if max > 0 {
                    ctx.i18n.t_with(
                        "errors.messages.max_emojis",
                        &[("max_emojis_count", &max.to_string())],
                    )
                } else {
                    ctx.i18n
                        .t("errors.messages.emojis_disabled")
                        .map(str::to_string)
                };
                errors.push(ctx.full_message(title_attr, &message.unwrap_or_default()));
            }
        }
        // unique_among
        let rule = s.get("duplicate_topic_titles")?.to_s();
        if rule != "allowed" && !topic.private_message {
            let own_id = persisted.map_or(0, |p| p.topic_id);
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM topics WHERE lower(title) = $1 AND deleted_at IS NULL \
                   AND id <> $2)",
            )
            .bind(title.to_lowercase())
            .bind(own_id)
            .fetch_one(&mut *conn)
            .await?;
            if exists {
                if persisted.is_none() {
                    return Err(Unsupported("duplicate topic titles").into());
                }
                if let Some(url) =
                    duplicate_title_url(conn, ctx, acting, topic, own_id, &rule).await?
                {
                    errors.push(
                        ctx.i18n
                            .t_with("errors.messages.topic_title_already_used", &[("url", &url)])
                            .unwrap_or_default(),
                    );
                }
            }
        }
    }
    // category_id: present, and not uncategorized, unless allowed.
    if persisted.is_none_or(|p| p.check_category)
        && !s.get("allow_uncategorized_topics")?.truthy()
        && !topic.private_message
    {
        let uncategorized = s.get("uncategorized_category_id")?.to_i() as i32;
        match topic.category_id {
            None => errors.push(ctx.full_message(
                "activerecord.attributes.topic.category_id",
                &ctx.t("errors.messages.blank"),
            )),
            Some(id) if id == uncategorized => errors.push(ctx.full_message(
                "activerecord.attributes.topic.category_id",
                &ctx.t("errors.messages.exclusion"),
            )),
            _ => {}
        }
    }
    Ok(errors)
}

/// UniqueAmongValidator's first duplicate (by id) among
/// `duplicate_title_candidates`: listable topics in the same category, and
/// with `disallowed` those the acting user can see too (secured, listed
/// unless they may see unlisted ones). Its `url`; None when no duplicate
/// is a candidate.
async fn duplicate_title_url(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    acting: &Guardian,
    topic: &TopicInput<'_>,
    own_id: i32,
    rule: &str,
) -> Result<Option<String>, AppError> {
    let secure = acting.secure_category_ids(&mut *conn, ctx.settings).await?;
    let found: Option<(i32, Option<String>)> = sqlx::query_as(
        "SELECT id, slug FROM topics \
         WHERE deleted_at IS NULL AND lower(title) = $1 AND id <> $2 \
           AND archetype <> 'private_message' \
           AND (category_id IS NOT DISTINCT FROM $3 \
                OR ($4 AND (category_id IS NULL OR category_id IN \
                      (SELECT id FROM categories WHERE NOT read_restricted OR id = ANY($5))) \
                    AND ($6 OR visible))) \
         ORDER BY id LIMIT 1",
    )
    .bind(topic.title.to_lowercase())
    .bind(own_id)
    .bind(topic.category_id)
    .bind(rule == "disallowed")
    .bind(&secure)
    .bind(acting.can_see_unlisted_topics())
    .fetch_optional(&mut *conn)
    .await?;
    let Some((id, slug)) = found else {
        return Ok(None);
    };
    let slug = slug.ok_or(Unsupported("a duplicate topic without its slug"))?;
    let base = crate::url::Urls {
        config: ctx.config,
        settings: ctx.settings,
    }
    .base_url()?;
    Ok(Some(format!("{base}/t/{slug}/{id}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc2396_hosts() {
        let host = |u: &str| rfc2396_host(u);
        assert_eq!(
            host("https://example.com/one").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            host("http://Example.COM:8080/x?y=1#z").as_deref(),
            Some("Example.COM")
        );
        assert_eq!(
            host("//cdn.example.com/a.png").as_deref(),
            Some("cdn.example.com")
        );
        assert_eq!(
            host("http://user:pw@example.com/").as_deref(),
            Some("example.com")
        );
        assert_eq!(host("http://1.2.3.4/").as_deref(), Some("1.2.3.4"));
        assert_eq!(host("http://[::1]:3000/").as_deref(), Some("[::1]"));
        // Relative, opaque, registry-based or unparsable: no host.
        assert_eq!(host("/t/a-topic/5"), None);
        assert_eq!(host("#heading"), None);
        assert_eq!(host("mailto:someone@example.com"), None);
        assert_eq!(host("http://my_host.example.com/"), None);
        assert_eq!(host("https://example.com/a b"), None);
        assert_eq!(host("https://example.com/a|b"), None);
        assert_eq!(host("https://example.com/%zz"), None);
        assert_eq!(host("https://example.com/[x]"), None);
    }

    #[test]
    fn link_counts_and_hosts() {
        let analysis = Analysis {
            links: vec![
                "https://example.com/a".into(),
                "https://example.com/b".into(),
                "/t/local/1".into(),
            ],
            onebox_urls: vec![
                "https://example.org/x".into(),
                "https://example.net/y".into(),
            ],
            ..Default::default()
        };
        assert_eq!(link_count(&analysis, 50), 5);
        assert_eq!(link_count(&analysis, 1), 4);
        assert_eq!(
            linked_hosts(&analysis, 50),
            vec!["example.com", "example.org", "example.net"]
        );
        assert_eq!(linked_hosts(&analysis, 0), vec!["example.com"]);
    }
}
