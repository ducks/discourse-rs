//! What decides whether a post may be written: NewPostManager's review
//! queue (refused, not ported), PostValidator, the Topic model's title
//! and category validations, and PostAnalyzer's counts over the cooked
//! HTML they read.

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

/// Refuses what isn't ported: staged and trust level 0 authors.
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

/// Authors whose posts aren't ported: staged users, and new users (the
/// link/media/mention limits, host spam and sockpuppet checks).
pub fn refuse_unported_author(guardian: &Guardian) -> Result<(), AppError> {
    let user = guardian.user().ok_or(Unsupported("posting anonymously"))?;
    if guardian.is_staff() {
        return Ok(());
    }
    if user.staged {
        return Err(Unsupported("posting as a staged user").into());
    }
    if user.trust_level < 1 {
        return Err(Unsupported("posting as a new user (trust level 0)").into());
    }
    Ok(())
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
    pub attachments: usize,
    pub links: Vec<String>,
    pub has_oneboxes: bool,
    /// The links cooking left for oneboxing: each href, and whether the
    /// onebox is an inline one (`.inline-onebox-loading`).
    pub onebox_links: Vec<(String, bool)>,
    /// A `div.poll`: one of discourse-poll's polls.
    pub has_polls: bool,
    /// An upload shown as video or audio, which the post processor would
    /// optimize (post_process_videos is not ported).
    pub has_upload_media: bool,
    pub has_quotes: bool,
}

/// `PostAnalyzer` counts, refusing what cooking leaves for oneboxing
/// that the post processor can't onebox (see `onebox::refuse_unported`).
pub fn analyze(cooked: &str, base_path: &str) -> Result<Analysis, Unsupported> {
    let dom = parse(cooked);
    let mut a = Analysis::default();
    let mut mentions = Vec::new();
    walk(
        &fragment_root(&dom),
        base_path,
        false,
        &mut a,
        &mut mentions,
    );
    mentions.sort();
    mentions.dedup();
    a.mentions = mentions.len();
    a.mention_names = mentions;
    Ok(a)
}

/// PostStripper removes mentions in code and quotes, quote titles,
/// oneboxes and elided parts before anything is counted.
fn walk(node: &Handle, bp: &str, stripped: bool, a: &mut Analysis, mentions: &mut Vec<String>) {
    let name = element_name(node);
    if let Some(name) = name {
        let inline = has_class(node, "inline-onebox-loading");
        if has_class(node, "onebox") || inline {
            a.has_oneboxes = true;
            if name == "a"
                && let Some(href) = attr(node, "href").filter(|h| !h.is_empty())
            {
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
                walk(child, bp, true, a, mentions);
            }
            return;
        }
        let mention = has_class(node, "mention") || has_class(node, "mention-group");
        if mention && stripped {
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
                    a.attachments += 1;
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
                walk(child, bp, true, a, mentions);
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
    let cooked_src = attr(node, "src").unwrap_or_default();
    if (cooked_src.contains("/uploads/") || attr(node, "data-orig-src").is_some())
        && element_name(node) != Some("img")
    {
        a.has_upload_media = true;
    }
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

    // unique_post_validator: Rails keeps the key in Redis for
    // unique_posts_mins; a recent identical post by the user stands in.
    let unique_mins = s.get("unique_posts_mins")?.to_i();
    if unique_mins > 0 && !acting.is_staff() && !post.raw.trim().is_empty() {
        let dupe: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM posts p JOIN topics t ON t.id = p.topic_id \
             WHERE p.user_id = $1 AND p.raw = $2 AND p.id IS DISTINCT FROM $3 \
               AND p.created_at > now() - make_interval(mins => $4) \
               AND (t.archetype = 'private_message') = $5)",
        )
        .bind(post.user_id)
        .bind(post.raw)
        .bind(post.post_id)
        .bind(unique_mins as i32)
        .bind(post.private_message)
        .fetch_one(&mut *conn)
        .await?;
        if dupe {
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

    // max_mention_validator (trusted or PM; new users are refused).
    if !acting.is_staff() {
        let max_mentions = s.get("max_mentions_per_post")?.to_i();
        if analysis.mentions as i64 > max_mentions {
            errors.push(if max_mentions == 0 {
                ctx.t("no_mentions_allowed")
            } else {
                ctx.t_count("too_many_mentions", max_mentions)
            });
        }
        // max_embedded_media_validator
        if !acting.in_setting_groups(s, "embedded_media_post_allowed_groups")?
            && analysis.embedded_media > 0
        {
            errors.push(ctx.t("no_embedded_media_allowed_group"));
        }
    }
    // max_links_validator: links need post_links_allowed_groups (or an
    // allowlisted host, not ported).
    if !analysis.links.is_empty()
        && !post.private_message
        && !acting.is_staff()
        && !acting.in_setting_groups(s, "post_links_allowed_groups")?
    {
        return Err(Unsupported("links from users outside post_links_allowed_groups").into());
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
            let (op_user, count, last_post): (Option<i32>, i64, Option<i32>) = sqlx::query_as(
                    "SELECT (SELECT user_id FROM posts WHERE topic_id = $1 ORDER BY post_number LIMIT 1), \
                       (SELECT COUNT(*) FROM (SELECT user_id FROM posts WHERE deleted_at IS NULL AND NOT hidden \
                          AND topic_id = $1 ORDER BY post_number DESC LIMIT $3) c WHERE c.user_id = $2), \
                       (SELECT id FROM posts WHERE topic_id = $1 ORDER BY post_number DESC LIMIT 1)",
                )
                .bind(topic_id)
                .bind(acting_user.id)
                .bind(max_replies)
                .fetch_one(&mut *conn)
                .await?;
            if op_user != Some(post.user_id) && count >= max_replies && last_post.is_some() {
                // guardian.can_edit?(topic.ordered_posts.last): the
                // user's own recent post; checking it is not ported.
                return Err(Unsupported("max_consecutive_replies reached").into());
            }
        }
    }
    Ok(errors)
}

/// The topic attributes the Topic model validates on create.
pub struct TopicInput<'a> {
    pub title: &'a str,
    pub category_id: Option<i32>,
    pub private_message: bool,
}

/// Topic's `validates :title` and `validates :category_id`, after the
/// before_validation cleanup; `errors.full_messages`.
pub async fn validate_topic(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    acting: &Guardian,
    topic: &TopicInput<'_>,
) -> Result<Vec<String>, AppError> {
    let s = ctx.settings;
    let mut errors = Vec::new();
    let title_attr = "activerecord.attributes.topic.title";
    let title = topic.title;
    if title.trim().is_empty() {
        errors.push(ctx.full_message(title_attr, &ctx.t("errors.messages.blank")));
    } else {
        let (min, max) = if acting.is_admin() {
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
        // max_emojis: titles with emoji are refused by the slug anyway.
        if crate::emoji::has_emoji_code(title) || title.chars().any(|c| c as u32 > 0xffff) {
            return Err(Unsupported("emoji in topic titles").into());
        }
        // unique_among
        let rule = s.get("duplicate_topic_titles")?.to_s();
        if rule != "allowed" && !topic.private_message {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM topics WHERE lower(title) = $1 AND deleted_at IS NULL)",
            )
            .bind(title.to_lowercase())
            .fetch_one(&mut *conn)
            .await?;
            if exists {
                return Err(Unsupported("duplicate topic titles").into());
            }
        }
    }
    // category_id: present, and not uncategorized, unless allowed.
    if !s.get("allow_uncategorized_topics")?.truthy() && !topic.private_message {
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
