//! `UserNotifications#digest` (app/mailers/user_notifications.rb): the
//! summary of what a user missed while away, its topics from
//! `Topic.for_digest`, its popular replies, its counts, and the views
//! user_notifications/digest.{html,text}.erb with their partials, written
//! out by hand with Erubi's trimming (a line holding only a `<% %>`
//! statement leaves nothing).

use chrono::NaiveDateTime;
use sqlx::PgConnection;

use super::notification::{Built, html_escape, layout_view};
use super::sender::Ctx;
use crate::session::current::SessionUser;
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// `ScoreCalculator.default_score_weights[:like_score] * 5.0`
const POPULAR_POST_SCORE: f64 = 75.0;
/// `Notification.types[:liked]`
const LIKED: i32 = 5;
/// `User::MAX_UNREAD_NOTIFICATIONS`
const MAX_UNREAD_NOTIFICATIONS: i64 = 99;

/// What the caller asks of the mailer: the admin's preview gives a date
/// (as the time it means and the text it was given in, which the digest
/// shows and keys its new users count by) and leaves the unsubscribe links
/// out.
#[derive(Default)]
pub struct Options {
    pub since: Option<(NaiveDateTime, String)>,
    pub skip_unsubscribe_links: bool,
}

/// The recipient as the digest reads them.
#[derive(sqlx::FromRow)]
struct Recipient {
    last_seen_at: Option<NaiveDateTime>,
    seen_notification_id: i64,
    digest_attempted_at: Option<NaiveDateTime>,
    include_tl0_in_digests: Option<bool>,
    digest_after_minutes: Option<i32>,
}

/// `Topic.for_digest(user, since, opts)`'s options.
struct ForDigest {
    since: NaiveDateTime,
    limit: Option<i64>,
    top_order: bool,
    include_tl0: bool,
    /// The fallback's `topics.created_at < 24.hours.ago`.
    created_before: Option<NaiveDateTime>,
}

/// What `Topic.for_digest` filters by, read once.
struct Filters {
    user_id: i32,
    staff: bool,
    include_tl0_in_digests: bool,
    secure_category_ids: Vec<i32>,
    grace_seconds: i64,
    suppress_categories: Vec<i32>,
    suppress_tag_ids: Vec<i32>,
    remove_category_ids: Vec<i32>,
    ignored_user_ids: Vec<i32>,
    muted_tag_ids: Vec<i32>,
    period: String,
}

fn ids(list: &[i32]) -> String {
    list.iter()
        .map(i32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

impl Filters {
    /// The relation as SQL, selecting `select` (`topics.id` in order, or a
    /// count, or the first posts' ids).
    fn sql(&self, opts: &ForDigest, select: &str, join_first_posts: bool) -> String {
        let uid = self.user_id;
        let mut sql = format!(
            "SELECT {select} FROM topics \
             LEFT OUTER JOIN topic_users ON topic_users.topic_id = topics.id AND topic_users.user_id = {uid} \
             LEFT OUTER JOIN category_users ON category_users.category_id = topics.category_id AND category_users.user_id = {uid} \
             LEFT OUTER JOIN users ON users.id = topics.user_id"
        );
        if opts.top_order {
            sql.push_str(" LEFT OUTER JOIN top_topics ON top_topics.topic_id = topics.id");
        }
        if join_first_posts {
            sql.push_str(
                " INNER JOIN posts ON posts.topic_id = topics.id AND posts.deleted_at IS NULL",
            );
        }
        let secured = if self.secure_category_ids.is_empty() {
            "NOT read_restricted".to_string()
        } else {
            format!(
                "NOT read_restricted OR id IN ({})",
                ids(&self.secure_category_ids)
            )
        };
        sql.push_str(&format!(
            " WHERE topics.deleted_at IS NULL AND topics.visible = TRUE \
             AND (topics.category_id IS NULL OR topics.category_id IN (SELECT id FROM categories WHERE {secured})) \
             AND topics.closed = FALSE AND topics.archived = FALSE \
             AND COALESCE(topic_users.notification_level, 1) <> 0 \
             AND topics.created_at > $1 \
             AND topics.created_at < $2 - make_interval(secs => {}) \
             AND topics.archetype <> 'private_message'",
            self.grace_seconds
        ));
        if !(opts.include_tl0 || self.include_tl0_in_digests) {
            sql.push_str(" AND COALESCE(users.trust_level, 0) > 0");
        }
        if join_first_posts {
            sql.push_str(" AND posts.post_number = 1");
        }
        sql.push_str(
            " AND topics.id NOT IN (SELECT categories.topic_id FROM categories WHERE categories.topic_id IS NOT NULL)",
        );
        if !self.suppress_categories.is_empty() {
            sql.push_str(&format!(
                " AND topics.category_id NOT IN ({})",
                ids(&self.suppress_categories)
            ));
        }
        if !self.suppress_tag_ids.is_empty() {
            sql.push_str(&format!(
                " AND topics.id NOT IN (SELECT topic_tags.topic_id FROM topic_tags WHERE topic_tags.tag_id IN ({}))",
                ids(&self.suppress_tag_ids)
            ));
        }
        if !self.remove_category_ids.is_empty() {
            sql.push_str(&format!(
                " AND (topic_users.notification_level != 0 OR topics.category_id NOT IN ({}))",
                ids(&self.remove_category_ids)
            ));
        }
        if !self.ignored_user_ids.is_empty() {
            sql.push_str(&format!(
                " AND topics.user_id NOT IN ({})",
                ids(&self.ignored_user_ids)
            ));
        }
        if !self.muted_tag_ids.is_empty() {
            sql.push_str(&format!(
                " AND (EXISTS (SELECT 1 FROM topic_tags WHERE topic_tags.topic_id = topics.id AND tag_id NOT IN ({})) \
                 OR NOT EXISTS (SELECT 1 FROM topic_tags WHERE topic_tags.topic_id = topics.id))",
                ids(&self.muted_tag_ids)
            ));
        }
        if opts.created_before.is_some() {
            sql.push_str(" AND topics.created_at < $3");
        }
        if opts.top_order {
            sql.push_str(&format!(
                " ORDER BY COALESCE(topic_users.notification_level, 1) DESC, \
                 COALESCE(category_users.notification_level, 1) DESC, \
                 COALESCE(top_topics.{}_score, 0) DESC, topics.bumped_at DESC",
                self.period
            ));
        }
        if let Some(limit) = opts.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }
        sql
    }

    async fn topic_ids(
        &self,
        conn: &mut PgConnection,
        opts: &ForDigest,
        now: NaiveDateTime,
    ) -> Result<Vec<i32>, AppError> {
        let sql = self.sql(opts, "topics.id", false);
        Ok(sqlx::query_scalar(&sql)
            .bind(opts.since)
            .bind(now)
            .bind(opts.created_before)
            .fetch_all(&mut *conn)
            .await?)
    }

    async fn count(
        &self,
        conn: &mut PgConnection,
        opts: &ForDigest,
        now: NaiveDateTime,
    ) -> Result<i64, AppError> {
        let sql = self.sql(opts, "COUNT(*)", false);
        Ok(sqlx::query_scalar(&sql)
            .bind(opts.since)
            .bind(now)
            .bind(opts.created_before)
            .fetch_one(&mut *conn)
            .await?)
    }

    /// `topics_for_digest.joins(:posts).where(posts: { post_number: 1 })
    /// .pluck("posts.id")`
    async fn first_post_ids(
        &self,
        conn: &mut PgConnection,
        opts: &ForDigest,
        now: NaiveDateTime,
    ) -> Result<Vec<i32>, AppError> {
        let sql = self.sql(opts, "posts.id", true);
        Ok(sqlx::query_scalar(&sql)
            .bind(opts.since)
            .bind(now)
            .bind(opts.created_before)
            .fetch_all(&mut *conn)
            .await?)
    }
}

/// Ruby's `Time#advance(months: -n)`: the same day n months back, the
/// month's last day when it has fewer.
fn months_ago(now: NaiveDateTime, months: u32) -> NaiveDateTime {
    now.checked_sub_months(chrono::Months::new(months))
        .unwrap_or(now)
}

/// `ListController.best_period_for(since)`: the first period, from the
/// shortest that still covers `since`, with a full page of scored topics;
/// else top_page_default_timeframe.
async fn best_period_for(
    conn: &mut PgConnection,
    s: &crate::site_settings::SiteSettings,
    since: NaiveDateTime,
    now: NaiveDateTime,
) -> Result<String, AppError> {
    let default = s.get("top_page_default_timeframe")?.to_s();
    let days = chrono::Duration::days;
    let mut periods = Vec::new();
    if since > now - days(8) {
        periods.push("daily");
    }
    if since > months_ago(now, 1) - days(7) {
        periods.push("weekly");
    }
    if since > months_ago(now, 3) - days(21) {
        periods.push("monthly");
    }
    if since > months_ago(now, 13) {
        periods.push("quarterly");
    }
    if since > months_ago(now, 36) {
        periods.push("yearly");
    }
    periods.push("all");
    let per_page = s.get("topics_per_period_in_top_page")?.to_i();
    for period in periods {
        let count: i64 = sqlx::query_scalar(&format!(
            "SELECT count(*) FROM (SELECT 1 FROM top_topics WHERE {period}_score > 0 LIMIT $1) t"
        ))
        .bind(per_page)
        .fetch_one(&mut *conn)
        .await?;
        if count == per_page {
            return Ok(period.to_string());
        }
    }
    if !crate::topic_query::PERIODS.contains(&default.as_str()) {
        return Err(Unsupported("an unknown top_page_default_timeframe").into());
    }
    Ok(default)
}

/// A topic the digest shows.
#[derive(sqlx::FromRow)]
struct DigestTopic {
    id: i32,
    title: String,
    slug: Option<String>,
    category_id: Option<i32>,
    created_at: NaiveDateTime,
    like_count: i32,
    posts_count: i32,
    user_id: Option<i32>,
    image_upload_id: Option<i64>,
    featured_link: Option<String>,
}

impl DigestTopic {
    fn slug(&self) -> Result<&str, AppError> {
        self.slug
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Unsupported("digest topics without a slug").into())
    }
}

/// A post the digest shows (a topic's first or best post, a popular one).
#[derive(sqlx::FromRow)]
struct DigestPost {
    topic_id: i32,
    post_number: i32,
    cooked: String,
    user_id: Option<i32>,
    user_deleted: bool,
}

/// A poster as the views show them.
#[derive(sqlx::FromRow)]
struct Poster {
    username: String,
    name: Option<String>,
    uploaded_avatar_id: Option<i32>,
}

/// Ruby's `String#truncate(n, separator: /\s/)`: at most n characters,
/// the "..." included, cut at the last whitespace that fits.
fn truncate(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        return s.to_string();
    }
    let stop = n - 3;
    // rindex(/\s/, stop): the last whitespace starting at or before stop.
    let cut = (0..=stop.min(chars.len() - 1))
        .rev()
        .find(|&i| chars[i].is_whitespace())
        .unwrap_or(stop);
    let head: String = chars[..cut].iter().collect();
    format!("{head}...")
}

/// `email_excerpt(html, post)`: first_paragraphs_from (the body's
/// paragraphs, lists and quotes and any onebox, until
/// digest_min_excerpt_length characters of text; else the first non-empty
/// paragraph or div, or a lightboxed image), else the whole html,
/// formatted for email.
pub(crate) fn email_excerpt(
    s: &crate::site_settings::SiteSettings,
    urls: &Urls<'_>,
    i18n: &crate::i18n::I18n,
    cooked: &str,
    post_url: Option<&str>,
) -> Result<String, AppError> {
    use crate::pretty_text::cleanup;
    let dom = cleanup::parse(cooked);
    let root = cleanup::fragment_root(&dom);
    let is_body_child = |node: &markup5ever_rcdom::Handle| {
        node.parent
            .take()
            .map(|weak| {
                let parent = weak.upgrade();
                node.parent.set(Some(weak));
                parent.is_some_and(|p| std::rc::Rc::ptr_eq(&p, &root))
            })
            .unwrap_or(false)
    };
    let min = s.get("digest_min_excerpt_length")?.to_i();
    let mut result = String::new();
    let mut length = 0i64;
    let mut done = false;
    for node in cleanup::all_elements(&dom) {
        let name = cleanup::element_name(&node);
        let wanted = (matches!(name, Some("p" | "ul" | "blockquote")) && is_body_child(&node))
            || (name == Some("aside") && cleanup::has_class(&node, "onebox"));
        if !wanted {
            continue;
        }
        let text = cleanup::text(&node);
        if text.trim().is_empty() {
            continue;
        }
        result.push_str(&cleanup::node_html(&node));
        length += text.chars().count() as i64;
        if length >= min {
            done = true;
            break;
        }
    }
    let html = if done || !result.is_empty() {
        result
    } else {
        // body > p:not(:empty), body > div:not(:empty),
        // body > p > div.lightbox-wrapper img: the first in document order.
        let fallback = cleanup::all_elements(&dom).into_iter().find(|node| {
            let name = cleanup::element_name(node);
            let not_empty = !node.children.borrow().is_empty();
            if matches!(name, Some("p" | "div")) && is_body_child(node) && not_empty {
                return true;
            }
            if name != Some("img") {
                return false;
            }
            // img inside div.lightbox-wrapper inside a body-level p.
            let mut ancestor = node.parent.take().and_then(|w| {
                let up = w.upgrade();
                node.parent.set(Some(w));
                up
            });
            while let Some(a) = ancestor {
                if cleanup::element_name(&a) == Some("div")
                    && cleanup::has_class(&a, "lightbox-wrapper")
                {
                    let parent = a.parent.take().and_then(|w| {
                        let up = w.upgrade();
                        a.parent.set(Some(w));
                        up
                    });
                    if let Some(p) = parent
                        && cleanup::element_name(&p) == Some("p")
                        && is_body_child(&p)
                    {
                        return true;
                    }
                }
                ancestor = a.parent.take().and_then(|w| {
                    let up = w.upgrade();
                    a.parent.set(Some(w));
                    up
                });
            }
            false
        });
        match fallback {
            Some(node) => cleanup::node_html(&node),
            None => cooked.to_string(),
        }
    };
    super::notification::format_for_email(&html, urls, i18n, post_url)
}

/// MarkdownLinker: numbered markdown links, their references listed at
/// the bottom (each list only the ones made since the last).
struct MarkdownLinker {
    base_url: String,
    links: Vec<String>,
    rendered: usize,
}

impl MarkdownLinker {
    fn create(&mut self, title: &str, url: &str) -> String {
        let link = if url.starts_with(&self.base_url) {
            url.to_string()
        } else {
            format!("{}{url}", self.base_url)
        };
        self.links.push(link);
        format!("[{title}][{}]", self.links.len())
    }

    fn references(&mut self) -> String {
        let mut out = String::new();
        for i in self.rendered..self.links.len() {
            out.push_str(&format!("[{}]: {}\n", i + 1, self.links[i]));
        }
        self.rendered = self.links.len();
        out
    }
}

/// `@counts`: (id, label key, value, href).
struct Count {
    id: &'static str,
    label: &'static str,
    value: i64,
    href: String,
}

/// UserNotifications#digest: None when it has no popular topics (the
/// mailer never calls `mail`), though the unsubscribe key is made first
/// all the same.
pub async fn build(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
    to: &str,
    opts: &Options,
) -> Result<Option<Built>, AppError> {
    let s = ctx.settings;
    let emojis = s.emojis().clone();
    let i18n = ctx.i18n;
    let locale = s.get("default_locale")?.to_s();
    if locale != "en" {
        return Err(Unsupported("digests in locales other than English").into());
    }
    let overridden: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM translation_overrides WHERE locale = $1 \
         AND translation_key LIKE 'user_notifications.digest.%')",
    )
    .bind(&locale)
    .fetch_one(&mut *conn)
    .await?;
    if overridden {
        return Err(Unsupported("translation overrides of the digest").into());
    }
    let session = SessionUser::load(&mut *conn, user_id)
        .await?
        .ok_or(Unsupported("a user that vanished"))?;
    let guardian = crate::guardian::Guardian::for_user(&mut *conn, &session).await?;
    let user: Recipient = sqlx::query_as(
        "SELECT u.last_seen_at, u.seen_notification_id::int8 AS seen_notification_id, s.digest_attempted_at, \
                o.include_tl0_in_digests, o.digest_after_minutes \
         FROM users u LEFT JOIN user_stats s ON s.user_id = u.id \
         LEFT JOIN user_options o ON o.user_id = u.id WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    let t = |key: &str, args: &[(&str, &str)]| -> Result<String, AppError> {
        Ok(i18n
            .t_with(&format!("user_notifications.digest.{key}"), args)
            .ok_or(Unsupported("a missing digest translation"))?)
    };

    // build_summary_for
    let title = s.get("title")?.to_s();
    let site_name = s.get("email_prefix")?.presence().unwrap_or(title.clone());
    let urls = Urls {
        config: ctx.config,
        settings: s,
    };
    let base_url = urls.base_url()?;
    let base_url_no_prefix = urls.base_url_no_prefix()?;
    let base_path = ctx.config.globals.relative_url_root().to_string();
    let header_color = crate::color_scheme::hex_for_name(&mut *conn, s, "header_primary")
        .await?
        .unwrap_or_default();
    let header_bgcolor = crate::color_scheme::hex_for_name(&mut *conn, s, "header_background")
        .await?
        .unwrap_or_default();
    let mut linker = MarkdownLinker {
        base_url: base_url.clone(),
        links: Vec::new(),
        rendered: 0,
    };
    let disable_custom_styles = !s.get("apply_custom_styles_to_digest")?.truthy();

    let unsubscribe_key = if opts.skip_unsubscribe_links {
        None
    } else {
        Some(
            super::unsubscribe::create_key_for(
                &mut *conn,
                user_id,
                super::unsubscribe::DIGEST_TYPE,
                None,
            )
            .await?,
        )
    };

    let now = crate::clock::now_naive();
    let since = match &opts.since {
        Some((since, _)) => *since,
        None => [
            user.last_seen_at,
            user.digest_attempted_at,
            Some(months_ago(now, 1)),
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(now),
    };

    // Topic.for_digest's inputs.
    let mut remove_category_ids: Vec<i32> = sqlx::query_scalar(
        "SELECT category_id FROM category_users WHERE user_id = $1 AND notification_level = 0",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    // SiteSetting.shared_drafts_enabled?: a category other than
    // Uncategorized is set.
    if let Some(shared_drafts) = s.get("shared_drafts_category")?.presence() {
        let id = crate::ruby::to_i(&shared_drafts);
        if id != s.get("uncategorized_category_id")?.to_i() {
            remove_category_ids.push(id as i32);
        }
    }
    let mut seen = std::collections::HashSet::new();
    remove_category_ids.retain(|id| seen.insert(*id));
    let suppress_tags: Vec<String> = s
        .get("digest_suppress_tags")?
        .to_s()
        .split('|')
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect();
    let suppress_tag_ids: Vec<i32> = if suppress_tags.is_empty() {
        Vec::new()
    } else {
        sqlx::query_scalar("SELECT id FROM tags WHERE lower(name) = ANY($1)")
            .bind(&suppress_tags)
            .fetch_all(&mut *conn)
            .await?
    };
    let filters = Filters {
        user_id,
        staff: guardian.is_staff(),
        include_tl0_in_digests: user.include_tl0_in_digests == Some(true),
        secure_category_ids: guardian.secure_category_ids(&mut *conn, s).await?,
        grace_seconds: s.get("editing_grace_period")?.to_i(),
        suppress_categories: s
            .get("digest_suppress_categories")?
            .to_s()
            .split('|')
            .filter(|c| !c.is_empty())
            .map(|c| crate::ruby::to_i(c) as i32)
            .collect(),
        suppress_tag_ids,
        remove_category_ids,
        ignored_user_ids: sqlx::query_scalar(
            "SELECT ignored_user_id FROM ignored_users WHERE user_id = $1 AND expiring_at >= $2",
        )
        .bind(user_id)
        .bind(now)
        .fetch_all(&mut *conn)
        .await?,
        muted_tag_ids: sqlx::query_scalar(
            "SELECT tag_id FROM tag_users WHERE user_id = $1 AND notification_level = 0",
        )
        .bind(user_id)
        .fetch_all(&mut *conn)
        .await?,
        period: best_period_for(&mut *conn, s, since, now).await?,
    };

    let digest_topics = s.get("digest_topics")?.to_i();
    let mut digest_opts = ForDigest {
        since,
        limit: Some(digest_topics + s.get("digest_other_topics")?.to_i()),
        top_order: true,
        include_tl0: false,
        created_before: None,
    };
    let mut topic_ids = filters.topic_ids(&mut *conn, &digest_opts, now).await?;
    if topic_ids.is_empty() && !filters.include_tl0_in_digests {
        // Topics from new users, at least a day old.
        digest_opts.include_tl0 = true;
        digest_opts.created_before = Some(now - chrono::Duration::hours(24));
        topic_ids = filters.topic_ids(&mut *conn, &digest_opts, now).await?;
    }
    let split = (digest_topics.max(0) as usize).min(topic_ids.len());
    if split == 0 {
        return Ok(None);
    }
    let topics: Vec<DigestTopic> = {
        let rows: Vec<DigestTopic> = sqlx::query_as(
            "SELECT id, title, slug, category_id, created_at, like_count, posts_count, user_id, \
                    image_upload_id::int8 AS image_upload_id, featured_link \
             FROM topics WHERE id = ANY($1)",
        )
        .bind(&topic_ids)
        .fetch_all(&mut *conn)
        .await?;
        let mut rows: Vec<Option<DigestTopic>> = rows.into_iter().map(Some).collect();
        topic_ids
            .iter()
            .filter_map(|id| {
                rows.iter_mut()
                    .find(|r| r.as_ref().is_some_and(|r| r.id == *id))
                    .and_then(Option::take)
            })
            .collect()
    };
    let (popular, other) = topics.split_at(split);
    for topic in &topics {
        if topic.image_upload_id.is_some() {
            return Err(Unsupported("topic images in digests").into());
        }
        if topic.featured_link.is_some() {
            return Err(Unsupported("featured links in digests").into());
        }
        if topic.user_id.is_none() {
            return Err(Unsupported("digest topics of deleted users").into());
        }
    }

    // @popular_posts
    let digest_posts = s.get("digest_posts")?.to_i();
    let popular_posts: Vec<DigestPost> = if digest_posts > 0 {
        let every_topic = ForDigest {
            since: chrono::DateTime::UNIX_EPOCH.naive_utc(),
            limit: None,
            top_order: false,
            include_tl0: false,
            created_before: None,
        };
        let topics_sql = filters.sql(&every_topic, "topics.id", false);
        let whisper = if filters.staff {
            ""
        } else {
            " AND posts.post_type <> 4"
        };
        // The subquery takes $1 (epoch) and $2 (now); the posts' own window
        // $4 and $5.
        let sql = format!(
            "SELECT posts.topic_id, posts.post_number, posts.cooked, posts.user_id, posts.user_deleted \
             FROM posts INNER JOIN ({topics_sql}) AS digest_topics ON digest_topics.id = posts.topic_id \
             WHERE posts.deleted_at IS NULL AND posts.created_at > $4{whisper} \
             AND posts.post_type = 1 AND posts.hidden = false AND posts.user_deleted = false \
             AND posts.post_number > 1 AND posts.score > {POPULAR_POST_SCORE} \
             AND posts.created_at < $5 \
             ORDER BY posts.score DESC, posts.created_at ASC LIMIT {digest_posts}"
        );
        sqlx::query_as(&sql)
            .bind(every_topic.since)
            .bind(now)
            .bind(None::<NaiveDateTime>)
            .bind(since)
            .bind(now - chrono::Duration::seconds(filters.grace_seconds))
            .fetch_all(&mut *conn)
            .await?
    } else {
        Vec::new()
    };

    // The first posts' excerpts.
    let mut first_posts: Vec<Option<DigestPost>> = Vec::new();
    let mut excerpts: Vec<String> = Vec::new();
    for topic in popular {
        let post: Option<DigestPost> = sqlx::query_as(
            "SELECT topic_id, post_number, cooked, user_id, user_deleted FROM posts \
             WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL",
        )
        .bind(topic.id)
        .fetch_optional(&mut *conn)
        .await?;
        let excerpt = match &post {
            Some(p) => {
                let url = format!(
                    "{base_url}/t/{}/{}/{}",
                    topic.slug()?,
                    topic.id,
                    p.post_number
                );
                email_excerpt(s, &urls, i18n, &p.cooked, Some(&url))?
            }
            None => String::new(),
        };
        excerpts.push(excerpt);
        first_posts.push(post);
    }

    // @counts
    let mut new_topics_count = filters
        .count(
            &mut *conn,
            &ForDigest {
                since,
                limit: None,
                top_order: false,
                include_tl0: false,
                created_before: None,
            },
            now,
        )
        .await?;
    if new_topics_count == 0 {
        new_topics_count = topics.len() as i64;
    }
    let mut counts = vec![Count {
        id: "new_topics",
        label: "new_topics",
        value: new_topics_count,
        href: format!("{base_url}/new"),
    }];
    let unread: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM (SELECT 1 FROM notifications n LEFT JOIN topics t ON t.id = n.topic_id \
         WHERE t.deleted_at IS NULL AND n.high_priority = FALSE AND n.user_id = $1 \
           AND n.id > $2 AND NOT read LIMIT $3) AS x",
    )
    .bind(user_id)
    .bind(user.seen_notification_id)
    .bind(MAX_UNREAD_NOTIFICATIONS)
    .fetch_one(&mut *conn)
    .await?;
    let unread_high: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM notifications n LEFT JOIN topics t ON t.id = n.topic_id \
         WHERE t.deleted_at IS NULL AND n.high_priority = TRUE AND n.user_id = $1 AND NOT read",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if unread + unread_high > 0 {
        counts.push(Count {
            id: "unread_notifications",
            label: "unread_notifications",
            value: unread + unread_high,
            href: format!("{base_url}/my/notifications"),
        });
    }
    if counts.len() < 3 {
        let likes: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notifications n LEFT JOIN topics t ON t.id = n.topic_id \
             WHERE t.deleted_at IS NULL AND n.notification_type = $3 AND n.user_id = $1 \
               AND NOT read AND n.created_at > $2",
        )
        .bind(user_id)
        .bind(since)
        .bind(LIKED)
        .fetch_one(&mut *conn)
        .await?;
        if likes > 0 {
            counts.push(Count {
                id: "likes_received",
                label: "liked_received",
                value: likes,
                href: format!("{base_url}/my/notifications"),
            });
        }
    }
    if counts.len() < 3 && user.digest_after_minutes.unwrap_or(0) >= 1440 {
        let date = match &opts.since {
            Some((_, text)) => text.clone(),
            None => since.format("%Y-%m-%d").to_string(),
        };
        let value = summary_new_users_count(&mut *conn, &date).await?;
        if value > 0 {
            counts.push(Count {
                id: "new_users",
                label: "new_users",
                value,
                href: format!("{base_url}/about"),
            });
        }
    }

    // Ruby's Time#to_s.
    let since_text = match &opts.since {
        Some((_, text)) => text.clone(),
        None => since.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
    };
    let preheader = t("preheader", &[("since", &since_text)])?;
    let subject_key = if s.get("simple_email_subject")?.truthy() {
        "subject_template_improved"
    } else {
        "subject_template"
    };
    let subject = t(subject_key, &[("email_prefix", &site_name)])?;
    let email_site_title = s
        .get("email_site_title")?
        .presence()
        .unwrap_or(title.clone());
    let from_alias = t("from", &[("site_name", &email_site_title)])?;

    // The views' per-topic and per-post values.
    let enable_names = s.get("enable_names")?.truthy();
    let logo_small = crate::admin_users::logo_small_url(&mut *conn, s).await?;
    let mut posters: std::collections::HashMap<i32, Poster> = std::collections::HashMap::new();
    let poster_ids: Vec<i32> = topics
        .iter()
        .filter_map(|t| t.user_id)
        .chain(popular_posts.iter().filter_map(|p| p.user_id))
        .collect();
    for id in poster_ids {
        if posters.contains_key(&id) {
            continue;
        }
        let poster: Option<Poster> =
            sqlx::query_as("SELECT username, name, uploaded_avatar_id FROM users WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?;
        let poster = poster.ok_or(Unsupported("digest posts of deleted users"))?;
        posters.insert(id, poster);
    }
    let small_avatar_url = |id: i32, p: &Poster| -> Result<String, AppError> {
        let template = crate::avatar::avatar_template(
            &urls,
            id,
            &p.username,
            p.uploaded_avatar_id,
            logo_small.as_deref(),
        )?;
        let absolute = urls.absolute(&template.replace("{size}", "45"))?;
        // UrlHelper.schemaless
        Ok(absolute
            .split_once(':')
            .filter(|(scheme, _)| matches!(*scheme, "http" | "https"))
            .map(|(_, rest)| rest.to_string())
            .unwrap_or(absolute))
    };
    let topic_url = |t: &DigestTopic| -> Result<String, AppError> {
        Ok(format!("{base_url}/t/{}/{}", t.slug()?, t.id))
    };
    let mut badges: std::collections::HashMap<i32, String> = std::collections::HashMap::new();
    for topic in &topics {
        if let Some(id) = topic.category_id
            && !badges.contains_key(&id)
        {
            let badge = crate::category_badge::email_html_for(
                &mut *conn,
                s,
                &base_url_no_prefix,
                &base_path,
                id,
            )
            .await?;
            badges.insert(id, badge);
        }
    }
    let badge = |t: &DigestTopic| -> String {
        t.category_id
            .and_then(|id| badges.get(&id).cloned())
            .unwrap_or_default()
    };
    let short_date = |at: NaiveDateTime| -> String {
        if chrono::Datelike::year(&at) == chrono::Datelike::year(&now) {
            at.format("%B %-d").to_string()
        } else {
            at.format("%B %-d, %Y").to_string()
        }
    };
    let email_image_url = |name: &str| format!("{base_url}/images/emails/{name}");
    let digest_title =
        |t: &DigestTopic| html_escape(&emojis.gsub_to_unicode(&truncate(&t.title, 100)));
    let join = html_escape(&t("join_the_discussion", &[])?);

    // digest.html.erb
    let header = {
        let logo_url = {
            let digest = crate::site_icons::site_url(&mut *conn, &urls, "digest_logo").await?;
            let logo = if digest.is_empty() || digest.to_lowercase().ends_with(".svg") {
                crate::site_icons::site_url(&mut *conn, &urls, "logo").await?
            } else {
                digest
            };
            Some(logo).filter(|l| !l.is_empty() && !l.to_lowercase().ends_with(".svg"))
        };
        let mut h = format!(
            "<table width=\"100%\" class=\"digest-header text-header with-dir\" style=\"background-color:#{};\">\n  <tr>\n    <!-- Empty cells pad either side of the email content -->\n    <td></td>\n    <td width=\"650\" align=\"center\" style=\"text-align: center;padding: 20px 0; font-family:Arial,sans-serif;\">\n      <a href=\"{}\" style=\"color:#{};font-size:22px;text-decoration:none;\">\n",
            html_escape(&header_bgcolor),
            html_escape(&base_url),
            html_escape(&header_color)
        );
        match &logo_url {
            None => h.push_str(&format!("        {}\n", html_escape(&title))),
            Some(logo) => h.push_str(&format!(
                "        <img src=\"{}\" height=\"40\" style=\"clear:both;display:block;height:40px;margin:auto;max-width:100%;outline:0;text-decoration:none;\" alt=\"{}\">\n",
                html_escape(logo),
                html_escape(&title)
            )),
        }
        h.push_str("      </a>\n    </td>\n    <td></td>\n  </tr>\n</table>");
        h
    };
    let stats = {
        let mut h = format!(
            "<table align=\"center\" class=\"digest-content-header with-accent-colors with-dir\" style=\"border-spacing:0;margin:0;padding:0;vertical-align:top;width:100%\">\n  <tbody>\n    <tr>\n      <td style=\"border-collapse:collapse!important;color:#0a0a0a;line-height:1.3;margin:0;padding:0;vertical-align:top;word-wrap:normal\">\n        <br/>\n        <center class=\"header-since-last-visit with-accent-colors\" style=\"font-size:22px;font-weight:400;mso-line-height-rule:exactly;line-height:36px;\">{}</center>\n        <table class=\"header-stats with-dir\" style=\"table-layout:fixed;margin:10px 0 20px 0;padding:0;vertical-align:top;width:100%;min-width:100%;\">\n          <tbody>\n            <tr class=\"header-stat-count\">\n",
            html_escape(&t("since_last_visit", &[])?)
        );
        for c in &counts {
            h.push_str(&format!(
                "                <td id=\"{}_stat_count\" style=\"text-align:center;font-size:36px;font-weight:400;\">\n                  <a class=\"with-accent-colors\" href=\"{}\"><strong>{}</strong></a>\n                </td>\n",
                c.id,
                html_escape(&c.href),
                c.value
            ));
        }
        h.push_str("            </tr>\n            <tr class=\"header-stat-description\">\n");
        for c in &counts {
            h.push_str(&format!(
                "                <td id=\"{}_stat_description\" style=\"font-size:14px;font-weight:400;text-align:center;\">\n                  <a class=\"with-accent-colors\" href=\"{}\"><strong>{}</strong></a>\n                </td>\n",
                c.id,
                html_escape(&c.href),
                html_escape(&t(c.label, &[])?)
            ));
        }
        h.push_str(&format!(
            "            </tr>\n          </tbody>\n        </table>\n        <center class=\"header-popular-topics with-accent-colors\" style=\"font-size:22px;font-weight:400;margin-bottom: 8px; mso-line-height-rule:exactly;line-height:36px;\">{}</center>\n      </td>\n    </tr>\n  </tbody>\n</table>",
            html_escape(&t("popular_topics", &[])?)
        ));
        h
    };
    let popular_topics_html = {
        let mut h = String::from(
            "<table class=\"with-dir\" align=\"center\" style=\"border-spacing:0;background:#fefefe;margin:0;padding:0;text-align:center;vertical-align:top;width:100%\">\n  <tbody>\n    <tr>\n      <td class=\"digest-popular-topic-wrapper with-dir\" style=\"-moz-hyphens:auto;-webkit-hyphens:auto;border-collapse:collapse!important;color:#0a0a0a;hyphens:auto;line-height:1.3;margin:0;padding:0;vertical-align:top;word-wrap:normal\">\n        \n\n",
        );
        for (i, topic) in popular.iter().enumerate() {
            let poster = &posters[&topic.user_id.unwrap_or_default()];
            let url = format!(
                "{base_url_no_prefix}{base_path}/t/{}/{}",
                topic.slug()?,
                topic.id
            );
            let mut p = format!(
                "<table class=\"digest-topic popular-topic with-dir\" style=\"width:100%\">\n  <tbody>\n    <tr>\n      <td class=\"digest-topic-category with-dir\" style=\"margin:0;padding:0 0 0 16px;vertical-align:top;\">\n        <p style=\"color:#8f8f8f;line-height:1.3;margin: 20px 0 0 0;\">\n          {}\n        </p>\n      </td>\n      <td class=\"with-dir\" style=\"margin:0;padding:0 16px 0 0;text-align:right;vertical-align:top;\">\n        <p class=\"text-right digest-created-at\" style=\"color:#8f8f8f;line-height:1.3;margin:20px 0 0 0;font-weight:400;\">\n          {}\n        </p>\n      </td>\n    </tr>\n  </tbody>\n</table>\n<table class=\"digest-topic-title-wrapper with-dir\" style=\"vertical-align:top;width:100%\">\n  <tbody>\n    <tr>\n      <td class=\"with-dir\" style=\"padding:0 8px 8px 16px;width:100%;\">\n        <h2 class=\"digest-topic-title\" style=\"font-size:18px;font-weight:400;line-height:1.3;margin:0;padding:0;word-wrap:normal\">\n          <a href=\"{}\" style=\"font-weight:400;line-height:1.3;margin:0;padding:0;text-decoration:none\">\n            <strong>{}</strong>\n          </a>\n        </h2>\n      </td>\n    </tr>\n  </tbody>\n</table>\n<table class=\"digest-topic-title-wrapper with-dir\" style=\"padding:0;position:relative;vertical-align:top;width:100%\">\n  <tbody>\n    <tr>\n      <td class=\"digest-topic-op\" style=\"color:#0a0a0a;line-height:1.3;margin:0 auto;padding:0 0 0 16px;width:50px;vertical-align:top;\">\n        <img src=\"{}\" style=\"border-radius:50%;clear:both;display:block;float:none;height:50px;width:50px;margin:0;max-width:100%;outline:0;text-align:center;text-decoration:none;\" align=\"center\" alt=\"{}\">\n      </td>\n      <td style=\"color:#0a0a0a;padding:0 16px 0 8px;vertical-align:top;\">\n",
                badge(topic),
                html_escape(&short_date(topic.created_at)),
                html_escape(&url),
                digest_title(topic),
                html_escape(&small_avatar_url(
                    topic.user_id.unwrap_or_default(),
                    poster
                )?),
                html_escape(&poster.username)
            );
            if let Some(name) = poster.name.as_deref().filter(|n| {
                enable_names
                    && !n.trim().is_empty()
                    && n.to_lowercase() != poster.username.to_lowercase()
            }) {
                p.push_str(&format!(
                    "            <h6 class=\"digest-topic-username\" style=\"color:inherit;line-height:1.3;margin:0;padding:0;font-weight: normal;font-size:16px;\">{}</h6>\n",
                    html_escape(name)
                ));
            }
            p.push_str(&format!(
                "          <p class=\"digest-topic-name\" style=\"color:inherit;font-size:14px;font-weight:400;line-height:1.3;margin:0 0 8px 0;padding:0;word-wrap:normal;\">{}</p>\n      </td>\n    </tr>\n  </tbody>\n</table>\n",
                html_escape(&poster.username)
            ));
            if first_posts[i].as_ref().is_some_and(|p| !p.user_deleted) {
                p.push_str(&format!(
                    "  <table class=\"digest-topic-body with-dir\" style=\"border-bottom:1px solid #f3f3f3;mso-border-bottom-alt:none;padding:0;vertical-align:top;width:100%;\">\n    <tbody>\n      <tr>\n        <td class=\"post-excerpt\" style=\"color:#0a0a0a;font-size:14px;padding:0 16px 0 16px;width:100%;font-weight:normal;mso-padding-alt: 16px;\">\n          {}\n        </td>\n      </tr>\n    </tbody>\n  </table>\n",
                    excerpts[i]
                ));
            }
            p.push_str(&format!(
                "<table class=\"digest-topic-stats with-dir\" style=\"padding:0;vertical-align:top;width:100%; padding-top:20px;\">\n  <tbody>\n    <tr>\n      <td class=\"digest-topic-stat\" style=\"padding:0 8px 16px 16px;white-space:nowrap;vertical-align:top;width:75px\">\n        <img class=\"digest-icon\" src=\"{}\" style=\"clear:both;display:inline-block;float:left;height:20px;width:20px;margin:0;max-width:100%;outline:0;text-decoration:none;\" alt=\"likes\">\n        <span style=\"color:#8f8f8f;float:left;line-height:1.3;margin:0 5px 10px 5px;padding:0;font-weight:400;\">&nbsp;{}</span>\n      </td>\n      <td class=\"digest-topic-stat\" style=\"padding:0 8px 16px 8px;white-space:nowrap;vertical-align:top;width:75px\">\n        <img class=\"digest-icon\" src=\"{}\" style=\"clear:none;display:inline-block;float:left;height:20px;width:20px;margin:0;max-width:100%;outline:0;text-decoration:none;\" alt=\"replies\">\n        <span style=\"color:#8f8f8f;float:left;line-height:1.3;margin:0 5px 10px 5px;padding:0;font-weight:400;\">&nbsp;{}</span>\n      </td>\n      <td class=\"digest-read-more-wrap\" style=\"line-height:1.3;padding:0 16px 0 8px;text-align:right;white-space:nowrap;vertical-align:top;\">\n        <span class=\"with_accent-colors mso-accent-link\">\n          <a href=\"{}\" class=\"digest-button with-accent-colors\" style=\"width:100%;text-decoration:none;padding:8px 16px;white-space:nowrap;\">\n            {join}\n          </a>\n        </span>\n      </td>\n    </tr>\n  </tbody>\n</table>\n<div style=\"background-color:#f3f3f3;\">\n  <table class=\"spacer with-dir\" style=\"padding:0;width:100%;background-color:#f3f3f3\">\n    <tbody><tr><td height=\"20px\" style=\"border-collapse:collapse!important;line-height:20px;margin:0;mso-line-height-rule:exactly;padding:0;\">&#xA0;</td></tr></tbody>\n  </table>\n</div>",
                html_escape(&email_image_url("heart_lightdark.png")),
                topic.like_count,
                html_escape(&email_image_url("comment_lightdark.png")),
                topic.posts_count - 1,
                html_escape(&url),
            ));
            h.push_str(&format!("          {p}\n\n"));
            if i < 5 {
                // digest_custom_html("below_post_N"): empty.
                h.push_str("            \n");
            }
        }
        h.push_str("\n        \n      </td>\n    </tr>\n  </tbody>\n</table>");
        h
    };
    let popular_posts_html = if popular_posts.is_empty() {
        String::new()
    } else {
        let mut h = format!(
            "  <center class=\"header-popular-posts\" style=\"color:#0a0a0a;background:#f3f3f3;font-size:22px;font-weight:400;padding: 20px 0;font-family:Arial,sans-serif;\">\n    {}\n  </center>\n\n  <table width=\"100%\" class=\"body with-dir\" style=\"background:#f3f3f3;border-spacing:0;border-collapse:collapse!important;font-family:Arial,sans-serif;font-size:14px;font-weight:200;line-height:1.3;padding:0;vertical-align:top;\">\n    <tr>\n      <td class=\"side-spacer\" style=\"padding:0;\">&nbsp;</td>\n      <td class=\"with-dir\" width=\"650\" align=\"center\" valign=\"top\" style=\"border-collapse:collapse!important;line-height:1.3;margin:0;padding:0;vertical-align:top;\">\n",
            html_escape(&t("popular_posts", &[])?)
        );
        for post in &popular_posts {
            let author = post
                .user_id
                .ok_or(Unsupported("digest posts of deleted users"))?;
            let poster = &posters[&author];
            let topic: (String, Option<String>) =
                sqlx::query_as("SELECT title, slug FROM topics WHERE id = $1")
                    .bind(post.topic_id)
                    .fetch_one(&mut *conn)
                    .await?;
            let slug = topic
                .1
                .clone()
                .filter(|s| !s.is_empty())
                .ok_or(Unsupported("digest topics without a slug"))?;
            let full_url = format!("{base_url}/t/{slug}/{}/{}", post.topic_id, post.post_number);
            let mut p = format!(
                "<table class=\"popular-post-excerpt with-dir\" style=\"width:100%;background:#fefefe;border-spacing:0;padding:0;vertical-align:top;\">\n  <tbody>\n    <tr>\n      <td class=\"post-excerpt with-dir\" style=\"color:#0a0a0a;font-size:14px;font-weight:normal;padding:16px;width:100%;\">\n        {}\n      </td>\n    </tr>\n  </tbody>\n</table>\n\n<table class=\"popular-post-spacer with-dir\" style=\"border-spacing:0;background-color:#f3f3f3;padding:0;vertical-align:top;width:100%\">\n  <tbody>\n    <tr>\n      <td style=\"border-spacing:0;padding:0;color:#0a0a0a;line-height:1.3;padding:0 0 0 65px;\">\n        <div style=\"width: 0;height: 0;border-top: 30px solid #fefefe;border-right: 30px solid transparent;\" dm=\"triangle\"></div>\n      </td>\n    </tr>\n  </tbody>\n</table>\n\n<table class=\"popular-post-meta with-dir\" style=\"background-color:#f3f3f3;padding:0;position:relative;vertical-align:top;width:100%\">\n  <tbody>\n    <tr>\n      <td style=\"padding:0 8px 8px 8px;width:50px;vertical-align:top;\">\n        <img src=\"{}\" style=\"border-radius:50%;clear:both;display:block;height:50px;width:50px;outline:0;\" alt=\"{}\">\n      </td>\n      <td style=\"color:#0a0a0a;line-height:1.3;padding:0 8px 8px 8px;vertical-align:top;\">\n",
                email_excerpt(s, &urls, i18n, &post.cooked, Some(&full_url))?,
                html_escape(&small_avatar_url(author, poster)?),
                html_escape(&poster.username)
            );
            // `post.user.name && ...`: an empty name still counts.
            if let Some(name) = poster
                .name
                .as_deref()
                .filter(|n| enable_names && n.to_lowercase() != poster.username.to_lowercase())
            {
                p.push_str(&format!(
                    "            <h6 style=\"color:inherit;line-height:1.3;margin:0;padding:0;font-weight: normal;font-size:16px;\">{}</h6>\n",
                    html_escape(name)
                ));
            }
            p.push_str(&format!(
                "          <p style=\"color:inherit;font-size:14px;font-weight:400;line-height:1.3;margin:0 0 8px 0;padding:0;word-wrap:normal;\">{}</p>\n      </td>\n      <td style=\"color:#0a0a0a;line-height:1.3;padding:0 8px 8px 8px;text-align:right;\">\n        <p style=\"color:#8f8f8f;line-height:1.3;margin:0 0 10px 0;padding:0;\">\n          <a href=\"{}\" style=\"font-weight:400;line-height:1.3;margin:0;padding:0;text-decoration:none\">{}</a>\n        </p>\n        <a href=\"{}\" class=\"digest-button with-accent-colors\" style=\"width:100%;text-decoration:none;padding:8px 16px;white-space: nowrap;\">\n          {join}\n        </a>\n      </td>\n    </tr>\n  </tbody>\n</table>\n\n<div style=\"background-color:#f3f3f3\">\n  <table class=\"spacer with-dir\" style=\"padding:0;vertical-align:top;width:100%;background-color:#f3f3f3\">\n    <tbody><tr><td height=\"40\" style=\"-moz-hyphens:auto;-webkit-hyphens:auto;border-collapse:collapse!important;color:#0a0a0a;font-size:40px;font-weight:400;hyphens:auto;line-height:40px;margin:0;mso-line-height-rule:exactly;padding:0;vertical-align:top;word-wrap:normal\">&#xA0;</td></tr></tbody>\n  </table>\n<div>",
                html_escape(&poster.username),
                html_escape(&full_url),
                html_escape(&emojis.gsub_to_unicode(&truncate(&topic.0, 100))),
                html_escape(&full_url),
            ));
            h.push_str(&format!("          {p}\n"));
        }
        h.push_str("      </td>\n      <td class=\"side-spacer\" style=\"padding:0;\">&nbsp;</td>\n    </tr>\n  </table>\n");
        h
    };
    let new_topics_html = if other.is_empty() {
        String::new()
    } else {
        let mut h = format!(
            "  <center class=\"digest-new-header\" style=\"color:#0a0a0a;background:#f3f3f3;font-size:22px;font-weight:400;padding-bottom: 8px;font-family:Arial,sans-serif;mso-line-height-rule:exactly;line-height:36px;\">{}</center>\n\n  <table width=\"100%\" class=\"digest-new-topics body with-dir\" style=\"background:#f3f3f3;border-spacing:0;border-collapse:collapse!important;font-family:Arial,sans-serif;font-size:14px;font-weight:200;line-height:1.3;padding:0;vertical-align:top;\">\n    <tr>\n      <td class=\"side-spacer\" style=\"padding:0;\">&nbsp;</td>\n      <td width=\"650\" align=\"center\" valign=\"top\" style=\"border-collapse:collapse!important;margin:0;padding:0;\">\n        <table class=\"digest-new-topic with-dir\" style=\"padding:0;vertical-align:top;width:100%\">\n          <tbody>\n",
            html_escape(&t("more_new", &[])?)
        );
        for topic in other {
            let url = format!(
                "{base_url_no_prefix}{base_path}/t/{}/{}",
                topic.slug()?,
                topic.id
            );
            h.push_str(&format!(
                "              <tr style=\"vertical-align:top;\">\n  <td class=\"digest-new-topic-title with-dir\" style=\"padding:8px;\">\n    <a href=\"{}\" style=\"font-weight:400;line-height:1.3;margin:0;padding:0;text-decoration:none;\">\n      <strong>{}</strong>\n    </a>\n    <p class=\"digest-new-topic-category\" style=\"color:#0a0a0a;line-height:1.3;margin:0 0 10px 0;padding:0;\">\n      {}\n    </p>\n  </td>\n  <td class=\"digest-new-topic-stat with-dir\" style=\"padding:8px;\">\n    <img class=\"digest-icon\" src=\"{}\" style=\"clear:both;display:inline-block;float:left;height:20px;width:20px;margin:0;max-width:100%;outline:0;text-decoration:none;\" alt=\"likes\">\n    <p style=\"color:#8f8f8f;float:left;line-height:1.3;margin:0 5px 10px 5px;padding:0;font-weight:400;\">{}</p>\n  </td>\n  <td class=\"digest-new-topic-stat digest-replies with-dir\" style=\"padding:8px;\">\n    <img class=\"digest-icon\" src=\"{}\" style=\"clear:none;display:inline-block;float:left;height:20px;width:20px;margin:0;max-width:100%;outline:0;text-decoration:none;\" alt=\"replies\">\n    <p style=\"color:#8f8f8f;float:left;line-height:1.3;margin:0 5px 10px 5px;padding:0;font-weight:400;\">{}</p>\n  </td>\n</tr>\n\n<tr class=\"digest-new-topic-spacer\"><td colspan=\"6\" height=\"10px\" style=\"border-collapse:collapse!important;line-height:10px;margin:0;mso-line-height-rule:exactly;padding:0;\">&#xA0;</td></tr>\n",
                html_escape(&url),
                digest_title(topic),
                badge(topic),
                html_escape(&email_image_url("heart_lightdark.png")),
                topic.like_count,
                html_escape(&email_image_url("comment_lightdark.png")),
                topic.posts_count - 1,
            ));
        }
        h.push_str("          </tbody>\n        </table>\n      </td>\n      <td class=\"side-spacer\" style=\"padding:0;\">&nbsp;</td>\n    </tr>\n  </table>\n");
        h
    };
    let footer_html = {
        let mut h = String::from(
            "<table width=\"100%\" class='summary-footer with-dir'>\n  <tr>\n    <!-- Empty cells pad either side of the email content -->\n    <td></td>\n    <td width=\"650\" align=\"center\">\n",
        );
        if let Some(key) = &unsubscribe_key {
            let link_to = |text: &str, url: &str| {
                format!("<a href=\"{}\">{}</a>", html_escape(url), html_escape(text))
            };
            h.push_str(&format!(
                "        {}\n",
                t(
                    "unsubscribe",
                    &[
                        (
                            "site_link",
                            &format!("<a href='{base_url}'>{site_name}</a>")
                        ),
                        (
                            "email_preferences_link",
                            &link_to(
                                &t("your_email_settings", &[])?,
                                &format!("{base_url}/my/preferences/emails")
                            )
                        ),
                        (
                            "unsubscribe_link",
                            &link_to(
                                &t("click_here", &[])?,
                                &format!("{base_url}/email/unsubscribe/{key}")
                            )
                        ),
                    ]
                )?
            ));
        }
        h.push_str("    </td>\n    <td></td>\n  </tr>\n</table>");
        h
    };
    let view = format!(
        "<div class=\"summary-email\">\n  <span class=\"preheader\" style=\"display:none!important;color:#f3f3f3;font-size:1px;line-height:1px;max-height:0;max-width:0;mso-hide:all!important;opacity:0;overflow:hidden;visibility:hidden\">\n    {}\n  </span>\n  <table border=\"0\" cellspacing=\"0\" width=\"100%\">\n    <tr>\n      <td>\n        {header}\n        <table width=\"100%\" class=\"body digest-content with-dir\" style=\"background:#f3f3f3;padding:0;border-spacing:0;font-family:Arial,sans-serif;font-size:14px;font-weight:200;line-height:1.3;vertical-align:top;\">\n          <tr>\n            <td class=\"side-spacer\" style=\"vertical-align:top;padding:0;\">\n              <div class=\"with-accent-colors\">\n                <table class=\"spacer with-accent-colors\" style=\"border-spacing:0;padding:0;width:100%\">\n                  <tbody>\n                    <tr>\n                      <td height=\"400px\" style=\"height:400px;border-collapse:collapse!important;margin:0;mso-line-height-rule:exactly;padding:0;\">&nbsp;</td>\n                    </tr>\n                  </tbody>\n                </table>\n              </div>\n            </td>\n            <td  width=\"650\" style=\"vertical-align:top;padding:0;font-family:Arial,sans-serif;\">\n              {stats}\n              {popular_topics_html}\n            </td>\n            <td class=\"side-spacer\" style=\"vertical-align:top;padding:0;\">\n              <!-- Background that goes down part-way behind content -->\n              <div class=\"with-accent-colors\">\n                <table class=\"spacer with-dir with-accent-colors\" style=\"border-spacing:0;padding:0;width:100%\">\n                  <tbody>\n                    <tr>\n                      <td height=\"400px\" style=\"height:400px;border-collapse:collapse!important;margin:0;mso-line-height-rule:exactly;padding:0;\">&nbsp;</td>\n                    </tr>\n                  </tbody>\n                </table>\n              </div>\n            </td>\n          </tr>\n        </table>\n\n        {popular_posts_html}\n\n        {new_topics_html}\n\n        {STYLES}\n\n        \n        {footer_html}\n        \n      </td>\n      <!-- Empty cells pad either side of the email content -->\n      <td></td>\n    </tr>\n  </table>\n</div>\n",
        html_escape(&preheader)
    );
    let html_part = if disable_custom_styles {
        // email_template's bare branch.
        format!("  {view}\n  \n")
    } else {
        layout_view(&view, &locale.replacen('_', "-", 1), "")
    };

    // digest.text.erb
    let mut text = String::new();
    let site_link = linker.create(&site_name, "/");
    text.push_str(&t(
        "why",
        &[("site_link", &site_link), ("since", &since_text)],
    )?);
    text.push_str("\n\n");
    for c in &counts {
        text.push_str(&format!("{} {}\n", c.value, t(c.label, &[])?));
    }
    text.push_str("\n\n");
    // _popular_topics.text.erb
    text.push_str(&format!("  \n  ### {}\n\n", t("popular_topics", &[])?));
    for topic in popular {
        text.push_str(&format!(
            "    {}\n\n",
            linker.create(&topic.title, &topic_url(topic)?)
        ));
        // Topic#best_post: the highest scored regular post.
        let best: Option<(i32, String)> = sqlx::query_as(
            "SELECT post_number, cooked FROM posts WHERE topic_id = $1 AND deleted_at IS NULL \
             AND post_type = 1 AND user_deleted = false ORDER BY score DESC NULLS LAST LIMIT 1",
        )
        .bind(topic.id)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some((post_number, cooked)) = best {
            let post_url = format!("/t/{}/{}/{post_number}", topic.slug()?, topic.id);
            text.push_str(&format!("      {}\n", text_excerpt(&cooked, &post_url)));
        }
        text.push_str("\n    \n");
    }
    text.push_str("\n  \n");
    text.push_str("\n\n");
    // _popular_posts.text.erb
    if !popular_posts.is_empty() {
        text.push_str(&format!("  ### {}\n\n", t("popular_posts", &[])?));
        for post in &popular_posts {
            let poster = &posters[&post.user_id.unwrap_or_default()];
            let (title, slug): (String, Option<String>) =
                sqlx::query_as("SELECT title, slug FROM topics WHERE id = $1")
                    .bind(post.topic_id)
                    .fetch_one(&mut *conn)
                    .await?;
            let slug = slug
                .filter(|s| !s.is_empty())
                .ok_or(Unsupported("digest topics without a slug"))?;
            let link = linker.create(&title, &format!("{base_url}/t/{slug}/{}", post.topic_id));
            text.push_str(&format!("    {} - {link}\n\n", poster.username));
            let post_url = format!("/t/{slug}/{}/{}", post.topic_id, post.post_number);
            text.push_str(&format!("    {}\n", text_excerpt(&post.cooked, &post_url)));
        }
    }
    text.push_str("\n\n");
    // _other_new_topics.text.erb
    if !other.is_empty() {
        text.push_str(&format!("**{}**\n\n", t("more_new", &[])?));
        for topic in other {
            let link = linker.create(&topic.title, &topic_url(topic)?);
            let category = match topic.category_id {
                Some(id) => {
                    let name: Option<String> =
                        sqlx::query_scalar("SELECT name FROM categories WHERE id = $1")
                            .bind(id)
                            .fetch_optional(&mut *conn)
                            .await?;
                    name.map(|n| format!("[{n}]")).unwrap_or_default()
                }
                None => String::new(),
            };
            text.push_str(&format!("* {link} - {} - {category}\n", topic.posts_count));
        }
    }
    text.push_str("\n\n");
    // _footer.text.erb
    let site_link = linker.create(&site_name, "/");
    text.push('\n');
    text.push_str(&linker.references());
    text.push_str("\n\n\n");
    if let Some(key) = &unsubscribe_key {
        let settings_link =
            linker.create(&t("your_email_settings", &[])?, "/my/preferences/emails");
        let unsubscribe_link =
            linker.create(&t("click_here", &[])?, &format!("/email/unsubscribe/{key}"));
        text.push_str(&format!(
            "  {}\n",
            t(
                "unsubscribe",
                &[
                    ("site_link", &site_link),
                    ("email_preferences_link", &settings_link),
                    ("unsubscribe_link", &unsubscribe_link),
                ]
            )?
        ));
    }
    text.push('\n');
    text.push_str(&linker.references());
    text.push_str("\n\n\n");

    // Email::MessageBuilder's headers.
    let first_post_ids = filters
        .first_post_ids(&mut *conn, &digest_opts, now)
        .await?;
    let notification_email = s.get("notification_email")?.to_s();
    let cleanup = |name: &str| name.replace([':', '<', '>', ',', '"'], "");
    let from = format!("\"{}\" <{notification_email}>", cleanup(&from_alias));
    let mut headers = vec![
        ("To".to_string(), to.to_string()),
        ("Subject".to_string(), subject),
        ("From".to_string(), from.clone()),
    ];
    if let Some(key) = &unsubscribe_key {
        headers.push((
            "List-Unsubscribe".into(),
            format!("<{base_url}/email/unsubscribe/{key}>"),
        ));
        headers.push((
            "List-Unsubscribe-Post".into(),
            "List-Unsubscribe=One-Click".into(),
        ));
    }
    if !first_post_ids.is_empty() {
        headers.push((
            "X-Discourse-Post-Ids".into(),
            first_post_ids
                .iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(","),
        ));
    }
    headers.push((
        "X-Discourse-Topic-Ids".into(),
        topic_ids
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(","),
    ));
    headers.push(("X-Auto-Response-Suppress".into(), "All".into()));
    headers.push(("x-ms-reactions".into(), "disallow".into()));
    headers.push(("Reply-To".into(), from));
    for item in s.get("email_custom_headers")?.to_s().split('|') {
        if let Some((name, value)) = item.split_once(':') {
            let (name, value) = (name.trim(), value.trim());
            if !name.is_empty() && !value.is_empty() {
                headers.push((name.to_string(), value.to_string()));
            }
        }
    }
    Ok(Some(Built {
        message: super::Message {
            headers,
            text,
            html: None,
        },
        html_part,
        view_rendered: true,
    }))
}

/// `post.excerpt(1000, strip_links: true, text_entities: true,
/// markdown_images: true)`
fn text_excerpt(cooked: &str, post_url: &str) -> String {
    crate::excerpt::excerpt(
        cooked,
        1000,
        &crate::excerpt::Options {
            strip_links: true,
            text_entities: true,
            image_mode: crate::excerpt::ImageMode::Markdown,
            post_url: Some(post_url.to_string()),
            ..Default::default()
        },
    )
}

/// `summary_new_users_count(since)`: real, activated, unstaged,
/// unsuspended users created after `date` (since's day, or the text the
/// admin gave), cached a day per date as Redis keeps it.
async fn summary_new_users_count(conn: &mut PgConnection, date: &str) -> Result<i64, AppError> {
    let key = format!("summary-new-users:{date}");
    if let Some(value) = crate::owned_schema::cached_get(&mut *conn, &key).await? {
        return Ok(crate::ruby::to_i(&value));
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users u WHERE u.id > 0 \
         AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = u.id) \
         AND u.active AND NOT u.staged \
         AND (u.suspended_till IS NULL OR u.suspended_till <= now()) \
         AND u.created_at > $1::timestamp",
    )
    .bind(date)
    .fetch_one(&mut *conn)
    .await?;
    crate::owned_schema::cached_setex(&mut *conn, &key, 86_400, &count.to_string()).await?;
    Ok(count)
}

/// digest/_styles.html.erb
const STYLES: &str = "<style>\n  @media only screen {\n    html {\n      min-height: 100%;\n      background: #f3f3f3\n    }\n  }\n\n  @media only screen and (max-width:900px) {\n    table.body img {\n      width: auto;\n      height: auto\n    }\n    table.body center {\n      min-width: 0!important;\n    }\n    table.body .side-spacer {\n      width: 2.5%!important;\n    }\n    table.body .column,\n    table.body .columns {\n      height: auto!important;\n      -moz-box-sizing: border-box;\n      -webkit-box-sizing: border-box;\n      box-sizing: border-box;\n      padding-left: 16px!important;\n      padding-right: 16px!important\n    }\n    table.body .column .column,\n    table.body .column .columns,\n    table.body .columns .column,\n    table.body .columns .columns {\n      padding-left: 0!important;\n      padding-right: 0!important\n    }\n  }\n</style>";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_cuts_at_whitespace_like_ruby() {
        assert_eq!(truncate("short", 100), "short");
        let long = "word ".repeat(30);
        let cut = truncate(long.trim(), 100);
        assert!(cut.ends_with("..."));
        assert!(cut.chars().count() <= 100);
        assert_eq!(cut, format!("{}...", "word ".repeat(19).trim_end()));
    }
}
