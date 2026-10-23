//! `PostRevisor#revise!` on a topic's first post for the topic's tracked
//! fields alone (TopicsController#update, TopicConverter): the title, the
//! archetype and the featured link, saved through Topic's validations and
//! callbacks, with the new version and revision on the first post and
//! what an edit does after it (the excerpt, the draft sequence, the
//! post-processing and alert jobs, web hooks, the reload publish).
//!
//! Topic-only changes always make a new version (the grace period applies
//! to the post's own fields), so nothing here leans on Redis.

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::{PgConnection, PgPool};

use super::search_index::{self, BaseUrls};
use super::small_action;
use super::text::{EmojiEscape, TitleOptions, clean_title, fancy_title, slug_for};
use super::validate::{self, Persisted, TopicInput};
use super::{Ctx, next_draft_sequence};
use crate::guardian::Guardian;
use crate::onebox::uri::RubyUri;
use crate::topic_guardian::TopicCtx;
use crate::url::Urls;
use crate::{AppError, Unsupported, modifications};

/// The tracked topic fields being changed, as the controller passes them.
#[derive(Debug, Default, Clone)]
pub struct TopicFields {
    pub title: Option<String>,
    pub archetype: Option<String>,
    /// `category_id`, 0 for none.
    pub category_id: Option<i32>,
    /// `featured_link`; a blank one clears it.
    pub featured_link: Option<String>,
    /// `tags`, as names; empty removes them all.
    pub tags: Option<Vec<String>>,
}

impl TopicFields {
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.archetype.is_none()
            && self.category_id.is_none()
            && self.featured_link.is_none()
            && self.tags.is_none()
    }
}

/// `revise!`'s options.
#[derive(Debug, Default, Clone, Copy)]
pub struct Opts {
    pub keep_existing_draft: bool,
    pub bypass_bump: bool,
    /// `silent:`: no notifications about the revision.
    pub silent: bool,
    /// `hidden:`: the revision is hidden from those who can't see hidden
    /// revisions (and leaves `public_version` alone).
    pub hidden: bool,
    /// `bypass_rate_limiter:` no EditRateLimiter.
    pub bypass_rate_limiter: bool,
    /// posts#update sending the post's own edit reason again: kept, where
    /// a topic-only revision otherwise clears it.
    pub keep_edit_reason: bool,
}

#[derive(Debug)]
pub enum Outcome {
    Revised,
    /// The topic didn't save: its `errors.full_messages`, empty when a
    /// field callback refused without saying why.
    Invalid(Vec<String>),
    /// EditRateLimiter refused it (the edit rolled back).
    RateLimited(crate::rate_limiter::LimitExceeded),
}

#[derive(sqlx::FromRow)]
struct PostRow {
    id: i32,
    user_id: Option<i32>,
    post_number: i32,
    cooked: String,
    edit_reason: Option<String>,
    last_version_at: NaiveDateTime,
    hidden: bool,
    hidden_reason_id: Option<i32>,
    wiki: bool,
    reply_to_post_number: Option<i32>,
}

#[derive(sqlx::FromRow, Clone)]
struct TopicRow {
    title: String,
    slug: Option<String>,
    fancy_title: Option<String>,
    category_id: Option<i32>,
    archetype: String,
    featured_link: Option<String>,
    user_id: Option<i32>,
    slow_mode_seconds: i32,
    og_image_upload_id: Option<i64>,
}

/// `Post.hidden_reasons[:flag_threshold_reached]`
const FLAG_THRESHOLD_REACHED: i32 = 1;

/// `PostRevisor.new(topic.ordered_posts.first, topic).revise!(editor,
/// fields, opts)` for topic fields, in its own transaction: the jobs
/// Rails defers to the commit go out with it, the rest of revise! runs
/// after.
pub async fn revise(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    editor: &Guardian,
    topic_id: i32,
    fields: &TopicFields,
    opts: Opts,
) -> Result<Outcome, AppError> {
    let mut conn = pool.acquire().await?;
    let Some(prepared) = prepare(&mut conn, ctx, editor, topic_id, fields).await? else {
        return Ok(Outcome::Invalid(Vec::new()));
    };
    drop(conn);
    let mut tx = pool.begin().await?;
    let mut deferred = Deferred::default();
    let revised = match apply(
        &mut tx,
        pool,
        ctx,
        editor,
        &prepared,
        fields,
        opts,
        &mut deferred,
    )
    .await?
    {
        Applied::Revised(revised) => revised,
        Applied::RateLimited(refused) => {
            tx.rollback().await?;
            return Ok(Outcome::RateLimited(refused));
        }
        Applied::Invalid(errors) => {
            tx.rollback().await?;
            // advance_draft_sequence ran in the rolled back
            // transaction: its upsert has taken a draft_sequences id
            // all the same.
            if !opts.keep_existing_draft && prepared.editor_id > 0 {
                sqlx::query("SELECT nextval('draft_sequences_id_seq')")
                    .execute(pool)
                    .await?;
            }
            return Ok(Outcome::Invalid(errors));
        }
    };
    deferred.enqueue(&mut tx, ctx).await?;
    tx.commit().await?;
    let mut conn = pool.acquire().await?;
    finish(&mut conn, ctx, editor, &prepared, &revised, opts).await?;
    Ok(Outcome::Revised)
}

/// What Rails runs from `DB.after_commit`, in the order it was asked for:
/// it goes out when the outermost transaction commits, after the jobs
/// enqueued in it.
#[derive(Default)]
pub struct Deferred(Vec<AfterCommit>);

enum AfterCommit {
    Job(&'static str, Value),
    /// New tags' tag_created web hooks (Tag's after_commit).
    TagsCreated(Vec<crate::tagging::CreatedTag>),
    /// TopicChanges#apply_tag_changes' block: NotifyTagChange and the tag
    /// change whisper for what the topic's tags became.
    TagsChanged {
        topic_id: i32,
        editor_id: i32,
        previous: Vec<String>,
    },
}

impl Deferred {
    pub fn push(&mut self, name: &'static str, args: Value) {
        self.0.push(AfterCommit::Job(name, args));
    }

    pub fn push_tags_created(&mut self, created: Vec<crate::tagging::CreatedTag>) {
        if !created.is_empty() {
            self.0.push(AfterCommit::TagsCreated(created));
        }
    }

    pub fn push_tags_changed(&mut self, topic_id: i32, editor_id: i32, previous: Vec<String>) {
        self.0.push(AfterCommit::TagsChanged {
            topic_id,
            editor_id,
            previous,
        });
    }

    pub async fn enqueue(self, conn: &mut PgConnection, ctx: &Ctx<'_>) -> Result<(), AppError> {
        for item in self.0 {
            match item {
                AfterCommit::Job(name, args) => {
                    crate::jobs::enqueue(&mut *conn, name, args).await?
                }
                AfterCommit::TagsCreated(created) => {
                    crate::tagging::enqueue_tag_created(&mut *conn, &created).await?
                }
                AfterCommit::TagsChanged {
                    topic_id,
                    editor_id,
                    previous,
                } => tags_changed(&mut *conn, ctx, topic_id, editor_id, &previous).await?,
            }
        }
        Ok(())
    }
}

/// apply_tag_changes' after_commit block: the first post's author and
/// last editor told of the tags added and removed (notify_tag_change),
/// and the whisper saying so.
async fn tags_changed(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    editor_id: i32,
    previous: &[String],
) -> Result<(), AppError> {
    let Some((post_id, user_id, last_editor_id)): Option<(i32, Option<i32>, Option<i32>)> =
        sqlx::query_as(
            "SELECT id, user_id, last_editor_id FROM posts WHERE topic_id = $1 \
               AND deleted_at IS NULL ORDER BY post_number LIMIT 1",
        )
        .bind(topic_id)
        .fetch_optional(&mut *conn)
        .await?
    else {
        return Ok(());
    };
    let mut notified = vec![user_id];
    if last_editor_id != user_id {
        notified.push(last_editor_id);
    }
    let persisted: Vec<String> = sqlx::query_scalar(
        "SELECT tags.name FROM topic_tags JOIN tags ON tags.id = topic_tags.tag_id \
         WHERE topic_tags.topic_id = $1 ORDER BY topic_tags.id",
    )
    .bind(topic_id)
    .fetch_all(&mut *conn)
    .await?;
    let added: Vec<String> = persisted
        .iter()
        .filter(|t| !previous.contains(t))
        .cloned()
        .collect();
    let removed: Vec<String> = previous
        .iter()
        .filter(|t| !persisted.contains(t))
        .cloned()
        .collect();
    let mut diff_tags = added.clone();
    for t in &removed {
        if !diff_tags.contains(t) {
            diff_tags.push(t.clone());
        }
    }
    if diff_tags.is_empty() {
        return Ok(());
    }
    crate::jobs::enqueue(
        &mut *conn,
        "notify_tag_change",
        json!({"post_id": post_id, "notified_user_ids": notified, "diff_tags": diff_tags}),
    )
    .await?;
    // create_small_action_for_tag_changes
    let s = ctx.settings;
    if !s.get("create_post_for_category_and_tag_changes")?.truthy()
        || s.get("whispers_allowed_groups")?.to_s().is_empty()
    {
        return Ok(());
    }
    let added_raw = tag_list_to_raw(&mut *conn, &added).await?;
    let removed_raw = tag_list_to_raw(&mut *conn, &removed).await?;
    let raw = match (added.is_empty(), removed.is_empty()) {
        (false, false) => ctx.i18n.t_with(
            "topic_tag_changed.added_and_removed",
            &[("added", &added_raw), ("removed", &removed_raw)],
        ),
        (false, true) => ctx
            .i18n
            .t_with("topic_tag_changed.added", &[("added", &added_raw)]),
        _ => ctx
            .i18n
            .t_with("topic_tag_changed.removed", &[("removed", &removed_raw)]),
    }
    .ok_or(Unsupported("topic_tag_changed"))?;
    let whisper = small_action::add_whisper(
        &mut *conn,
        ctx,
        &small_action::SmallAction {
            user_id: editor_id,
            topic_id,
            action_code: "tags_changed",
            bump: false,
            who: None,
        },
        &raw,
    )
    .await?;
    // custom_fields: an array is a row per value.
    for (name, values) in [("tags_added", &added), ("tags_removed", &removed)] {
        for value in values.iter() {
            sqlx::query(
                "INSERT INTO post_custom_fields (post_id, name, value, created_at, updated_at) \
                 VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp())",
            )
            .bind(whisper)
            .bind(name)
            .bind(value)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

/// `PostRevisor.tag_list_to_raw`: the tags sorted, each as the hashtag the
/// composer would write (`#name`, or `#name::tag` where a top-level
/// category has that slug), or the bare name where no hashtag can say it.
async fn tag_list_to_raw(conn: &mut PgConnection, tags: &[String]) -> Result<String, AppError> {
    let mut sorted = tags.to_vec();
    sorted.sort();
    if sorted.iter().any(|t| t.contains(':')) {
        return Err(Unsupported("tag hashtags in the parent:child form").into());
    }
    let lower: Vec<String> = sorted.iter().map(|t| t.to_lowercase()).collect();
    let conflicting: Vec<String> = sqlx::query_scalar(
        "SELECT lower(slug) FROM categories WHERE parent_category_id IS NULL \
           AND lower(slug) = ANY($1)",
    )
    .bind(&lower)
    .fetch_all(&mut *conn)
    .await?;
    Ok(sorted
        .iter()
        .map(|slug| {
            let reference = if conflicting.contains(&slug.to_lowercase()) {
                format!("{slug}::tag")
            } else {
                slug.clone()
            };
            if usable_ref(&reference) {
                format!("#{reference}")
            } else {
                slug.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", "))
}

/// `HashtagRewriter.usable_ref?`: the ref scans back as one whole hashtag
/// (hashtag-autocomplete's MATCHER characters, no dot at either end).
fn usable_ref(reference: &str) -> bool {
    let ok = |c: char, dot: bool| {
        c.is_ascii_alphanumeric()
            || matches!(c, '_' | ':' | '-')
            || (dot && c == '.')
            || ('\u{C0}'..='\u{1FFF}').contains(&c)
            || ('\u{2C00}'..='\u{D7FF}').contains(&c)
    };
    let chars: Vec<char> = reference.chars().collect();
    match chars.len() {
        0 => false,
        1 => ok(chars[0], false),
        n if n > 101 => false,
        n => {
            ok(chars[0], false)
                && ok(chars[n - 1], false)
                && chars[1..n - 1].iter().all(|c| ok(*c, true))
        }
    }
}

/// What revise! reads before its transaction.
pub struct Prepared {
    topic_id: i32,
    pub(super) editor_id: i32,
    editor_trust_level: i32,
    post: PostRow,
    topic: TopicRow,
    revised_at: NaiveDateTime,
    can_edit_featured_link: bool,
    can_change_archetype: bool,
    author_admin: bool,
    author_trust_level: Option<i32>,
}

/// The first post and topic as revise! finds them; None when slow mode
/// keeps the editor out (an error on the post, which no caller shows).
pub async fn prepare(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    editor: &Guardian,
    topic_id: i32,
    fields: &TopicFields,
) -> Result<Option<Prepared>, AppError> {
    let s = ctx.settings;
    let editor_user = editor
        .user()
        .ok_or(Unsupported("revising a topic anonymously"))?
        .clone();
    let post: Option<PostRow> = sqlx::query_as(
        "SELECT id, user_id, post_number, cooked, edit_reason, last_version_at, hidden, \
                hidden_reason_id, wiki, reply_to_post_number \
         FROM posts WHERE topic_id = $1 AND deleted_at IS NULL ORDER BY post_number LIMIT 1",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let post = post.ok_or(Unsupported("revising a topic without posts"))?;
    if post.post_number != 1 {
        return Err(Unsupported("revising a topic whose first post is deleted").into());
    }
    let topic: TopicRow = sqlx::query_as(
        "SELECT title, slug, fancy_title, category_id, archetype, featured_link, user_id, \
                slow_mode_seconds, og_image_upload_id \
         FROM topics WHERE id = $1",
    )
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    let self_edit = post.user_id == Some(editor_user.id);
    // remove_flags_and_unhide_post
    if self_edit && post.hidden && post.hidden_reason_id == Some(FLAG_THRESHOLD_REACHED) {
        return Err(Unsupported("editing a post hidden by flags").into());
    }
    let (revised_at,): (NaiveDateTime,) = sqlx::query_as("SELECT clock_timestamp()::timestamp")
        .fetch_one(&mut *conn)
        .await?;
    // affected_by_slow_mode? && !grace_period_edit? (no raw: only the
    // time counts) && slow_mode_prevents_editing.
    let grace = s.get("editing_grace_period")?.to_i();
    let elapsed = (revised_at - post.last_version_at).num_milliseconds() as f64 / 1000.0;
    if topic.slow_mode_seconds > 0
        && editor_user.id > 0
        && !editor.is_staff()
        && elapsed > grace as f64
        && s.get("slow_mode_prevents_editing")?.truthy()
    {
        return Ok(None);
    }
    let can_edit_featured_link = match &fields.featured_link {
        Some(link) if !link.trim().is_empty() => {
            crate::topic_guardian::can_edit_featured_link(
                &mut *conn,
                s,
                Some(editor_user.trust_level),
                topic.category_id,
            )
            .await?
        }
        _ => true,
    };
    let can_change_archetype = match &fields.archetype {
        Some(archetype) => {
            can_change_archetype(&mut *conn, ctx, editor, topic_id, archetype).await?
        }
        None => true,
    };
    let author_admin: bool = match topic.user_id {
        Some(id) => {
            sqlx::query_scalar("SELECT COALESCE((SELECT admin FROM users WHERE id = $1), FALSE)")
                .bind(id)
                .fetch_one(&mut *conn)
                .await?
        }
        None => false,
    };
    let author_trust_level: Option<i32> = match topic.user_id {
        Some(id) => {
            sqlx::query_scalar("SELECT trust_level FROM users WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    Ok(Some(Prepared {
        topic_id,
        editor_id: editor_user.id,
        editor_trust_level: editor_user.trust_level,
        post,
        topic,
        revised_at,
        can_edit_featured_link,
        can_change_archetype,
        author_admin,
        author_trust_level,
    }))
}

/// How revise!'s transaction ends.
pub enum Applied {
    Revised(Revised),
    /// The topic's errors (none when a callback refused silently); the
    /// caller rolls back.
    Invalid(Vec<String>),
    RateLimited(crate::rate_limiter::LimitExceeded),
}

/// The topic fields' changes, `[before, after]` by field in tracking
/// order.
pub type TopicDiff = Vec<(&'static str, [Value; 2])>;

/// What the transaction changed, for what comes after it.
pub struct Revised {
    diff: TopicDiff,
    archetype: String,
}

/// revise!'s transaction: the post's new version, the topic's fields
/// and save, the revision, the edit rate limit, the excerpt and the
/// draft sequence.
#[allow(clippy::too_many_arguments)]
pub async fn apply(
    conn: &mut PgConnection,
    pool: &PgPool,
    ctx: &Ctx<'_>,
    editor: &Guardian,
    p: &Prepared,
    fields: &TopicFields,
    opts: Opts,
    deferred: &mut Deferred,
) -> Result<Applied, AppError> {
    let s = ctx.settings;
    let (post, topic_id) = (&p.post, p.topic_id);
    let self_edit = post.user_id == Some(p.editor_id);
    // update_post: a new version with no post fields changed.
    let public_step = i32::from(!opts.hidden);
    sqlx::query(
        "UPDATE posts SET version = version + 1, public_version = public_version + $2, \
                last_version_at = $3, edit_reason = CASE WHEN $6 THEN edit_reason END, \
                last_editor_id = $4, self_edits = self_edits + $5, updated_at = clock_timestamp() \
         WHERE id = $1",
    )
    .bind(post.id)
    .bind(public_step)
    .bind(p.revised_at)
    .bind(p.editor_id)
    .bind(i32::from(self_edit))
    .bind(opts.keep_edit_reason)
    .execute(&mut *conn)
    .await?;
    let urls = Urls {
        config: ctx.config,
        settings: s,
    };
    let hostname = urls.current_hostname()?;
    let store = crate::file_store::FileStore::for_site(ctx.config, s)?;
    crate::upload_references::link_post_uploads(
        &mut *conn,
        s,
        &hostname,
        &store,
        post.id,
        &post.cooked,
    )
    .await?;
    save_reply_relationship(&mut *conn, topic_id, post.id, post.reply_to_post_number).await?;
    if p.editor_id != -1 {
        sqlx::query(
            "UPDATE user_stats SET post_edits_count = COALESCE(post_edits_count, 0) + 1 WHERE user_id = $1",
        )
        .bind(p.editor_id)
        .execute(&mut *conn)
        .await?;
    }

    let (diff, archetype) =
        match update_topic(&mut *conn, ctx, editor, p, fields, opts.silent, deferred).await? {
            Ok(changed) => changed,
            Err(errors) => return Ok(Applied::Invalid(errors)),
        };

    // create_revision: post_changes (only edit_reason can change here)
    // merged with the topic's diff.
    let mut modifications: Vec<(&str, [Value; 2])> = Vec::new();
    if post.edit_reason.is_some() && !opts.keep_edit_reason {
        modifications.push(("edit_reason", [json!(post.edit_reason), Value::Null]));
    }
    modifications.extend(diff.iter().cloned());
    let yaml = modifications::dump_column(&modifications)?;
    let revision_id: i32 = sqlx::query_scalar(
        "INSERT INTO post_revisions (user_id, post_id, number, modifications, hidden, created_at, updated_at) \
         SELECT $1, id, version, $3, $4, clock_timestamp(), clock_timestamp() FROM posts WHERE id = $2 \
         RETURNING id",
    )
    .bind(p.editor_id)
    .bind(post.id)
    .bind(&yaml)
    .bind(opts.hidden)
    .fetch_one(&mut *conn)
    .await?;
    if let Some(user_ids) = revision_recipients(
        &mut *conn,
        ctx,
        &NotifyRevision {
            topic_id,
            author_id: post.user_id,
            editor_id: p.editor_id,
            first_post: true,
            wiki: post.wiki,
            private_message: archetype == "private_message",
            silent: opts.silent,
            category_changed: modifications.iter().any(|(k, _)| *k == "category_id"),
            tags_changed: modifications.iter().any(|(k, _)| *k == "tags"),
        },
    )
    .await?
    {
        deferred.push(
            "notify_post_revision",
            json!({ "user_ids": user_ids, "post_revision_id": revision_id }),
        );
    }
    // perform_edit
    if ctx.config.rate_limits && !opts.bypass_rate_limiter {
        let limiter = super::limits::edit_limiter(
            s,
            crate::rate_limiter::LimitUser {
                id: p.editor_id,
                staff: editor.is_staff(),
            },
            p.editor_trust_level,
        )?;
        if let Err(refused) = limiter.performed(pool, true).await? {
            return Ok(Applied::RateLimited(refused));
        }
    }
    // revise_topic: the excerpt and a category's description.
    let excerpt = crate::excerpt::excerpt(
        &post.cooked,
        s.get("topic_excerpt_maxlength")?.to_i().max(0) as usize,
        &crate::excerpt::Options {
            strip_links: true,
            image_mode: crate::excerpt::ImageMode::Strip,
            ..Default::default()
        },
        crate::excerpt::Plugins::from_settings(s)?,
    );
    sqlx::query("UPDATE topics SET excerpt = $2 WHERE id = $1")
        .bind(topic_id)
        .bind(excerpt)
        .execute(&mut *conn)
        .await?;
    sync_category_description(&mut *conn, ctx, topic_id, &post.cooked).await?;
    if !opts.keep_existing_draft {
        next_draft_sequence(&mut *conn, p.editor_id, &format!("topic_{topic_id}")).await?;
    }
    Ok(Applied::Revised(Revised { diff, archetype }))
}

/// `update_topic`: the tracked fields' callbacks in tracking order, then
/// the topic's save. The diff and the archetype it ends with, or the
/// topic's errors (none when a callback refused without saying why).
#[allow(clippy::type_complexity)]
pub async fn update_topic(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    editor: &Guardian,
    p: &Prepared,
    fields: &TopicFields,
    silent: bool,
    deferred: &mut Deferred,
) -> Result<Result<(TopicDiff, String), Vec<String>>, AppError> {
    let s = ctx.settings;
    let (post, topic, topic_id) = (&p.post, &p.topic, p.topic_id);
    let mut diff: Vec<(&'static str, [Value; 2])> = Vec::new();
    let mut new = topic.clone();
    let mut errored = false;
    let mut errors: Vec<String> = Vec::new();
    if let Some(title) = &fields.title {
        record_change(&mut diff, "title", json!(topic.title), json!(title));
        new.title = title.clone();
    }
    if !errored && let Some(archetype) = &fields.archetype {
        if p.can_change_archetype {
            record_change(
                &mut diff,
                "archetype",
                json!(topic.archetype),
                json!(archetype),
            );
            new.archetype = archetype.clone();
        } else {
            errored = true;
        }
    }
    // What the topic row holds as the callbacks go: the category callback
    // saves the topic before the final save does.
    let mut saved = topic.clone();
    let saver = Saver {
        ctx,
        editor,
        topic_id,
        author_admin: p.author_admin,
        author_trust_level: p.author_trust_level,
    };
    if !errored && let Some(category_id) = fields.category_id {
        let new_category = (category_id != 0).then_some(category_id);
        if new_category.is_none() && new.archetype == "private_message" {
            record_change(
                &mut diff,
                "category_id",
                json!(new.category_id),
                Value::Null,
            );
            new.category_id = None;
        } else if new_category.is_none()
            || can_move_topic_to_category(&mut *conn, s, editor, category_id).await?
        {
            let tag_errors = match new_category {
                Some(target) => {
                    category_tags_errors(&mut *conn, ctx, editor, topic_id, target, fields).await?
                }
                None => Vec::new(),
            };
            if !tag_errors.is_empty() {
                errors = tag_errors;
                errored = true;
            } else {
                record_change(
                    &mut diff,
                    "category_id",
                    json!(new.category_id),
                    json!(new_category),
                );
                let old_category = new.category_id;
                let moved = change_category_to_id(
                    &mut *conn,
                    &saver,
                    &mut saved,
                    &mut new,
                    category_id,
                    silent,
                    post.id,
                    deferred,
                )
                .await?;
                if let Err(save_errors) = moved {
                    errors = save_errors;
                    errored = true;
                }
                // create_small_action_for_category_change: a whisper
                // naming both categories.
                if !errored
                    && let (Some(from), Some(to)) = (old_category, new_category)
                    && s.get("create_post_for_category_and_tag_changes")?.truthy()
                    && !s.get("whispers_allowed_groups")?.to_s().is_empty()
                {
                    let from = category_slug_ref(&mut *conn, from).await?;
                    let to = category_slug_ref(&mut *conn, to).await?;
                    let raw = ctx
                        .i18n
                        .t_with(
                            "topic_category_changed",
                            &[("from", &format!("#{from}")), ("to", &format!("#{to}"))],
                        )
                        .ok_or(Unsupported("topic_category_changed"))?;
                    small_action::add_whisper(
                        &mut *conn,
                        ctx,
                        &small_action::SmallAction {
                            user_id: p.editor_id,
                            topic_id,
                            action_code: "category_changed",
                            bump: false,
                            who: None,
                        },
                        &raw,
                    )
                    .await?;
                }
            }
        }
    }
    // TopicChanges#apply_tag_changes
    let mut tags_saved = false;
    if !errored
        && let Some(tags) = &fields.tags
        && editor.can_tag_topics(s)?
    {
        let mut previous: Vec<String> = sqlx::query_scalar(
            "SELECT tags.name FROM topic_tags JOIN tags ON tags.id = topic_tags.tag_id \
             WHERE topic_tags.topic_id = $1 ORDER BY topic_tags.id",
        )
        .bind(topic_id)
        .fetch_all(&mut *conn)
        .await?;
        previous.sort();
        if !(tags.is_empty() && previous.is_empty()) {
            let tagged = crate::tagging::TaggedTopic {
                id: Some(topic_id),
                private_message: new.archetype == "private_message",
                category_id: new.category_id,
            };
            match crate::tagging::tag_topic_by_names(
                &mut *conn, s, ctx.i18n, editor, &tagged, tags, false,
            )
            .await?
            {
                Err(rejected) => {
                    deferred.push_tags_created(rejected.created);
                    errors = rejected.errors;
                    errored = true;
                }
                Ok(tagging) => {
                    if tagging.has_invalid_tag() {
                        return Err(Unsupported(
                            "a topic's tags with one Tag's validations refuse (RecordInvalid)",
                        )
                        .into());
                    }
                    deferred.push_tags_created(tagging.created.clone());
                    // topic.tags = tags, written at once for a saved topic.
                    crate::tagging::apply(
                        &mut *conn,
                        &crate::tagging::SavedTopic {
                            id: topic_id,
                            private_message: tagged.private_message,
                            category_id: tagged.category_id,
                        },
                        &tagging,
                    )
                    .await?;
                    tags_saved = true;
                    let mut next: Vec<String> =
                        tagging.tags.iter().map(|t| t.name.clone()).collect();
                    next.sort();
                    if next != previous {
                        record_change(&mut diff, "tags", json!(previous), json!(next));
                        if !silent {
                            deferred.push_tags_changed(topic_id, p.editor_id, previous);
                        }
                    }
                }
            }
        }
    }
    if !errored && let Some(link) = &fields.featured_link {
        if link.trim().is_empty() {
            record_change(
                &mut diff,
                "featured_link",
                json!(topic.featured_link),
                Value::Null,
            );
            new.featured_link = None;
        } else if !p.can_edit_featured_link {
            errored = true;
        } else {
            record_change(
                &mut diff,
                "featured_link",
                json!(topic.featured_link),
                json!(link),
            );
            new.featured_link = Some(link.clone());
        }
    }
    if !errored {
        // after_save with tags_changed: the reindex queued and TagUser's
        // auto watching and tracking, ahead of the save's own index of a
        // new title (a failed save rolls it all back).
        if tags_saved {
            crate::tagging::after_tags_saved(&mut *conn, topic_id).await?;
        }
        errors = saver.save(&mut *conn, &mut saved, &mut new).await?;
        errored = !errors.is_empty();
    }
    if errored {
        return Ok(Err(errors));
    }
    Ok(Ok((diff, new.archetype)))
}

/// revise! after its transaction: the wiki bump, links, reset_highest,
/// post_process_post (process_post, :post_edited's web hooks and
/// plugins), alert_users, publish_changes and grant_badge.
pub async fn finish(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    editor: &Guardian,
    p: &Prepared,
    revised: &Revised,
    opts: Opts,
) -> Result<(), AppError> {
    let s = ctx.settings;
    let (post, topic_id) = (&p.post, p.topic_id);
    // bump_topic: a wiki first post whose post fields changed (only the
    // edit reason can, here).
    if !opts.bypass_bump && post.wiki && post.edit_reason.is_some() && !opts.keep_edit_reason {
        sqlx::query("UPDATE topics SET bumped_at = clock_timestamp() WHERE id = $1")
            .bind(topic_id)
            .execute(&mut *conn)
            .await?;
        use crate::topic_tracking_state as tracking;
        tracking::publish_muted(ctx.bus, &mut *conn, topic_id).await?;
        tracking::publish_unmuted(ctx.bus, &mut *conn, topic_id).await?;
        tracking::publish_latest(ctx.bus, s, &mut *conn, topic_id).await?;
    }
    let urls = Urls {
        config: ctx.config,
        settings: s,
    };
    let base_path = ctx.config.globals.relative_url_root().to_string();
    if let Some(user_id) = post.user_id {
        super::links::extract_from(
            &mut *conn,
            &super::links::Site {
                hostname: &urls.current_hostname()?,
                base_path: &base_path,
                base_url_no_prefix: &urls.base_url_no_prefix()?,
                settings: s,
            },
            &super::links::LinkPost {
                id: post.id,
                user_id,
                topic_id,
                cooked: &post.cooked,
            },
        )
        .await?;
    }
    crate::post_destroyer::reset_highest(&mut *conn, topic_id).await?;
    crate::jobs::enqueue(
        &mut *conn,
        "process_post",
        json!({"bypass_bump": false, "cooking_options": null, "new_post": false, "post_id": post.id, "skip_pull_hotlinked_images": false, "invalidate_oneboxes": true}),
    )
    .await?;
    // :post_edited with topic_changed: topic_edited's web hooks for the
    // first post.
    crate::web_hooks::enqueue_topic_hooks(
        &mut *conn,
        &crate::web_hooks::Payloads {
            settings: s,
            i18n: ctx.i18n,
            config: ctx.config,
        },
        "topic_edited",
        topic_id,
        None,
    )
    .await?;
    if revised.diff.iter().any(|(k, _)| *k == "category_id") {
        crate::plugins::topic_category_revised(&mut *conn, s, topic_id).await?;
    }
    if p.editor_id != -1 {
        crate::jobs::enqueue(&mut *conn, "post_alert", json!({"post_id": post.id})).await?;
    }
    let mut publish = Map::new();
    if !revised.diff.is_empty() {
        publish.insert("reload_topic".into(), json!(true));
    }
    crate::bus::publish_post_change(ctx, &mut *conn, post.id, "revised", publish, false).await?;
    crate::badge_granter::queue(
        &mut *conn,
        s,
        crate::badge_granter::Trigger::PostRevision,
        &[post.id],
    )
    .await?;
    // ReviewablePost.queue_for_review_if_possible
    if s.get("review_every_post")?.truthy()
        && revised.archetype != "private_message"
        && p.editor_id > 0
        && !editor.is_staff()
        && !editor.has_trust_level(4)
    {
        return Err(Unsupported("review_every_post on a topic edit").into());
    }
    Ok(())
}

fn record_change(
    diff: &mut Vec<(&str, [Value; 2])>,
    field: &'static str,
    before: Value,
    after: Value,
) {
    if before != after {
        diff.push((field, [before, after]));
    }
}

/// `can_change_archetype?(topic, archetype)`
async fn can_change_archetype(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    topic_id: i32,
    archetype: &str,
) -> Result<bool, AppError> {
    let s = ctx.settings;
    let topic = TopicCtx::load(&mut *conn, s, guardian, topic_id)
        .await?
        .ok_or(Unsupported("changing the archetype of a missing topic"))?;
    if archetype == topic.archetype {
        return Ok(true);
    }
    if archetype == "banner" || topic.archetype == "banner" {
        return Ok(guardian.can_banner_topic(&topic));
    }
    if archetype == "private_message" || topic.private_message() {
        let can_post_anywhere = guardian.can_create_post_anywhere(&mut *conn, s).await?;
        let can_create_post = guardian.can_create_post(s, &topic, can_post_anywhere)?;
        return Ok(guardian.can_convert_topic(s, &topic, can_create_post)?);
    }
    Ok(true)
}

/// What saving the topic reads besides its attributes.
struct Saver<'a, 'b> {
    ctx: &'a Ctx<'b>,
    editor: &'a Guardian,
    topic_id: i32,
    /// `user.admin?` of the topic's author (its title length).
    author_admin: bool,
    author_trust_level: Option<i32>,
}

impl Saver<'_, '_> {
    /// `topic.save(validate: true)` from `saved` (the row) to `new`:
    /// returns the validation errors, and on success `saved` is `new`.
    async fn save(
        &self,
        conn: &mut PgConnection,
        saved: &mut TopicRow,
        new: &mut TopicRow,
    ) -> Result<Vec<String>, AppError> {
        let errors = save_topic(
            conn,
            self.ctx,
            self.editor,
            self.topic_id,
            saved,
            new,
            self.author_admin,
            self.author_trust_level,
        )
        .await?;
        if errors.is_empty() {
            *saved = new.clone();
        }
        Ok(errors)
    }
}

/// `@topic.save(validate: true)`: before_validation's cleanup (the title
/// assigned again through `title=`, the featured link stripped), the
/// validations, before_save's fancy title and what a new category brings
/// (its slow mode and auto close), the row written when anything changed,
/// after_save's search index and after_update's tag stats. Returns the
/// validation errors (nothing written when there are some).
#[allow(clippy::too_many_arguments)]
async fn save_topic(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    editor: &Guardian,
    topic_id: i32,
    old: &TopicRow,
    new: &mut TopicRow,
    author_admin: bool,
    author_trust_level: Option<i32>,
) -> Result<Vec<String>, AppError> {
    let s = ctx.settings;
    // before_validation
    new.title = clean_title(
        &new.title,
        &TitleOptions {
            prettify: s.get("title_prettify")?.truthy(),
            allow_uppercase_posts: s.get("allow_uppercase_posts")?.truthy(),
            remove_extraneous_space: s.get("title_remove_extraneous_space")?.truthy(),
        },
    );
    if s.get("slug_generation_method")?.to_s() != "ascii" {
        return Err(Unsupported("slug_generation_method other than ascii").into());
    }
    // `title=`: the slug from the new title, the fancy title cleared.
    new.slug = Some(if new.title.trim().is_empty() {
        String::new()
    } else {
        slug_for(&new.title, &s.get("default_locale")?.to_s())?
    });
    new.fancy_title = None;
    new.featured_link = new
        .featured_link
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string);

    let title_changed = new.title != old.title;
    let category_changed = new.category_id != old.category_id;
    let featured_link_changed = new.featured_link != old.featured_link;
    let private_message = new.archetype == "private_message";
    let mut errors = validate::validate_topic_with(
        &mut *conn,
        ctx,
        editor,
        &TopicInput {
            title: &new.title,
            category_id: new.category_id,
            private_message,
        },
        Some(&Persisted {
            topic_id,
            author_admin,
            check_title: title_changed || category_changed,
            check_category: category_changed && new.archetype == "regular",
        }),
    )
    .await?;
    // validates :featured_link, url: true, then the guardian check and
    // normalize_featured_link.
    if let Some(link) = new.featured_link.clone() {
        let attribute = "activerecord.attributes.topic.featured_link";
        let mut link_errors = false;
        if !valid_url(&link)? {
            errors.push(ctx.full_message(attribute, &ctx.t("errors.messages.invalid")));
            link_errors = true;
        }
        if featured_link_changed
            && !crate::topic_guardian::can_edit_featured_link(
                &mut *conn,
                s,
                author_trust_level,
                new.category_id,
            )
            .await?
        {
            errors.push(ctx.full_message(
                attribute,
                &ctx.t("activerecord.errors.models.topic.attributes.featured_link.invalid"),
            ));
            link_errors = true;
        }
        if featured_link_changed && !link_errors {
            match normalized_encode(&link)? {
                Some(normalized) => new.featured_link = Some(normalized),
                None => errors.push(ctx.full_message(
                    attribute,
                    &ctx.t("activerecord.errors.models.topic.attributes.featured_link.invalid"),
                )),
            }
        }
    }
    if !errors.is_empty() {
        return Ok(errors);
    }

    // before_save
    if new.category_id.is_none() && new.archetype == "regular" {
        // ensure_topic_has_a_category
        new.category_id = Some(s.get("uncategorized_category_id")?.to_i() as i32);
    }
    let category_changed = new.category_id != old.category_id;
    if title_changed {
        new.fancy_title = Some(fancy_title(&new.title, EmojiEscape::from_settings(s)?));
    }
    if category_changed {
        // inherit_slow_mode_from_category
        let slow_mode: Option<i32> =
            sqlx::query_scalar("SELECT default_slow_mode_seconds FROM categories WHERE id = $1")
                .bind(new.category_id)
                .fetch_optional(&mut *conn)
                .await?
                .flatten();
        if let Some(seconds) = slow_mode {
            new.slow_mode_seconds = seconds;
        }
    }
    if (title_changed || category_changed) && old.og_image_upload_id.is_some() {
        return Err(Unsupported("regenerating a topic's og image").into());
    }
    let changed = title_changed
        || new.slug != old.slug
        || new.fancy_title != old.fancy_title
        || new.archetype != old.archetype
        || new.featured_link != old.featured_link
        || new.slow_mode_seconds != old.slow_mode_seconds
        || category_changed;
    if changed {
        sqlx::query(
            "UPDATE topics SET title = $2, slug = $3, fancy_title = $4, archetype = $5, \
                    featured_link = $6, category_id = $7, slow_mode_seconds = $8, \
                    updated_at = clock_timestamp() \
             WHERE id = $1",
        )
        .bind(topic_id)
        .bind(&new.title)
        .bind(&new.slug)
        .bind(&new.fancy_title)
        .bind(&new.archetype)
        .bind(&new.featured_link)
        .bind(new.category_id)
        .bind(new.slow_mode_seconds)
        .execute(&mut *conn)
        .await?;
    }
    if category_changed {
        // before_save's inherit_auto_close_from_category, its timer
        // written with the topic.
        crate::topic_timer::inherit_auto_close_from_category(
            &mut *conn,
            ctx,
            topic_id,
            crate::topic_timer::types::CLOSE,
        )
        .await?;
    }
    // after_save: SearchIndexer.queue_post_reindex, then the topic's own
    // index (its first post's and its row) when the title changed.
    if title_changed || category_changed {
        sqlx::query(
            "UPDATE post_search_data SET version = 0 FROM posts \
             WHERE post_search_data.post_id = posts.id AND posts.topic_id = $1",
        )
        .bind(topic_id)
        .execute(&mut *conn)
        .await?;
    }
    if title_changed {
        let first_post: Option<i32> = sqlx::query_scalar(
            "SELECT id FROM posts WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL",
        )
        .bind(topic_id)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some(post_id) = first_post {
            let urls = Urls {
                config: ctx.config,
                settings: s,
            };
            search_index::reindex_post(
                &mut *conn,
                s,
                &BaseUrls {
                    base_path: ctx.config.globals.relative_url_root(),
                    base_url_no_prefix: urls.base_url_no_prefix()?,
                    store: crate::file_store::FileStore::for_site(ctx.config, s)?,
                },
                post_id,
            )
            .await?;
        }
    }
    if category_changed {
        // after_update: CategoryTagStat.topic_moved for a tagged topic,
        // else the topic leaves profiles when its category is restricted.
        let tag_ids: Vec<i32> =
            sqlx::query_scalar("SELECT tag_id FROM topic_tags WHERE topic_id = $1 ORDER BY id")
                .bind(topic_id)
                .fetch_all(&mut *conn)
                .await?;
        if !tag_ids.is_empty() {
            category_tag_stats_moved(&mut *conn, &tag_ids, old.category_id, new.category_id)
                .await?;
        } else {
            let restricted: bool = sqlx::query_scalar(
                "SELECT COALESCE((SELECT read_restricted FROM categories WHERE id = $1), FALSE)",
            )
            .bind(new.category_id)
            .fetch_one(&mut *conn)
            .await?;
            if restricted {
                sqlx::query(
                    "UPDATE user_profiles SET featured_topic_id = NULL WHERE featured_topic_id = $1",
                )
                .bind(topic_id)
                .execute(&mut *conn)
                .await?;
            }
        }
    }
    Ok(Vec::new())
}

/// `CategoryTagStat.topic_moved(topic, from, to)`
async fn category_tag_stats_moved(
    conn: &mut PgConnection,
    tag_ids: &[i32],
    from: Option<i32>,
    to: Option<i32>,
) -> Result<(), sqlx::Error> {
    if let Some(from) = from {
        sqlx::query(
            "UPDATE category_tag_stats SET topic_count = topic_count - 1 \
             WHERE tag_id = ANY($1) AND category_id = $2 AND topic_count > 0",
        )
        .bind(tag_ids)
        .bind(from)
        .execute(&mut *conn)
        .await?;
    }
    if let Some(to) = to {
        let updated: Vec<i64> = sqlx::query_scalar(
            "UPDATE category_tag_stats SET topic_count = topic_count + 1 \
             WHERE tag_id = ANY($1) AND category_id = $2 RETURNING tag_id",
        )
        .bind(tag_ids)
        .bind(to)
        .fetch_all(&mut *conn)
        .await?;
        for tag_id in tag_ids
            .iter()
            .filter(|t| !updated.contains(&i64::from(**t)))
        {
            sqlx::query(
                "INSERT INTO category_tag_stats (tag_id, category_id, topic_count) VALUES ($1, $2, 1)",
            )
            .bind(tag_id)
            .bind(to)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

/// `can_move_topic_to_category?(category)`: staff, or those who may
/// create topics there without review.
pub(crate) async fn can_move_topic_to_category(
    conn: &mut PgConnection,
    s: &crate::site_settings::SiteSettings,
    guardian: &Guardian,
    category_id: i32,
) -> Result<bool, AppError> {
    if guardian.is_staff() {
        return Ok(true);
    }
    let category_id = if category_id == 0 {
        s.get("uncategorized_category_id")?.to_i() as i32
    } else {
        category_id
    };
    Ok(
        can_create_topic_on_category(&mut *conn, s, guardian, category_id).await?
            && !validate::posting_review_required(
                &mut *conn,
                guardian,
                category_id,
                validate::REVIEW_TOPIC,
            )
            .await?,
    )
}

/// `can_create_topic_on_category?(category)`
pub async fn can_create_topic_on_category(
    conn: &mut PgConnection,
    s: &crate::site_settings::SiteSettings,
    guardian: &Guardian,
    category_id: i32,
) -> Result<bool, AppError> {
    Ok(guardian.can_create_topic(&mut *conn, s).await?
        && guardian
            .topic_create_allowed_category_ids(&mut *conn, s)
            .await?
            .contains(&category_id))
}

/// The category callback's `DiscourseTagging.validate_category_tags` for
/// the tags the topic will have (those sent, else its own): the topic's
/// errors, empty when they may go there.
async fn category_tags_errors(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    topic_id: i32,
    category_id: i32,
    fields: &TopicFields,
) -> Result<Vec<String>, AppError> {
    let names: Vec<String> = match &fields.tags {
        Some(tags) => tags.clone(),
        None => {
            sqlx::query_scalar(
                "SELECT tags.name FROM topic_tags JOIN tags ON tags.id = topic_tags.tag_id \
                 WHERE topic_tags.topic_id = $1 ORDER BY topic_tags.id",
            )
            .bind(topic_id)
            .fetch_all(&mut *conn)
            .await?
        }
    };
    let Some(category) = crate::tagging::TagCategory::load(&mut *conn, category_id).await? else {
        return Ok(Vec::new());
    };
    crate::tagging::validate_category_tags(
        conn,
        ctx.settings,
        ctx.i18n,
        guardian,
        &category,
        &names,
    )
    .await
}

/// `Topic#change_category_to_id(category_id, silent:)` from the category
/// callback: Ok(true) when the topic moved, Err with the topic's errors
/// when its save failed.
#[allow(clippy::too_many_arguments)]
async fn change_category_to_id(
    conn: &mut PgConnection,
    saver: &Saver<'_, '_>,
    saved: &mut TopicRow,
    new: &mut TopicRow,
    category_id: i32,
    silent: bool,
    first_post_id: i32,
    deferred: &mut Deferred,
) -> Result<Result<bool, Vec<String>>, AppError> {
    let ctx = saver.ctx;
    let s = ctx.settings;
    let topic_id = saver.topic_id;
    // check_result(false): an error-less failure.
    let refused = Err(Vec::new());
    if new.archetype == "private_message" {
        return Ok(refused);
    }
    let uncategorized = s.get("uncategorized_category_id")?.to_i() as i32;
    let category_id = if category_id == 0 {
        uncategorized
    } else {
        category_id
    };
    if new.category_id == Some(category_id) {
        return Ok(Ok(false));
    }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE id = $1)")
        .bind(category_id)
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        return Ok(refused);
    }
    sqlx::query("UPDATE reviewables SET category_id = $2 WHERE topic_id = $1")
        .bind(topic_id)
        .bind(category_id)
        .execute(&mut *conn)
        .await?;
    // changed_to_category: a category's definition stays where it is.
    let definition: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE topic_id = $1)")
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?;
    if definition {
        return Ok(Ok(false));
    }
    if category_id == uncategorized && !s.get("allow_uncategorized_topics")?.truthy() {
        return Ok(refused);
    }
    let old_category = new.category_id;
    // update(category_id:): the save's own result goes unchecked; the
    // final save fails the same way.
    new.category_id = Some(category_id);
    let errors = saver.save(&mut *conn, saved, new).await?;
    if !errors.is_empty() {
        return Ok(Err(errors));
    }
    if let Some(old) = old_category {
        sqlx::query("UPDATE categories SET topic_count = topic_count - 1 WHERE id = $1")
            .bind(old)
            .execute(&mut *conn)
            .await?;
        let (old_restricted, new_restricted): (bool, bool) = sqlx::query_as(
            "SELECT COALESCE((SELECT read_restricted FROM categories WHERE id = $1), FALSE), \
                    COALESCE((SELECT read_restricted FROM categories WHERE id = $2), FALSE)",
        )
        .bind(old)
        .bind(category_id)
        .fetch_one(&mut *conn)
        .await?;
        let count = match (old_restricted, new_restricted) {
            (true, false) => Some(1),
            (false, true) => Some(-1),
            _ => None,
        };
        if let Some(count) = count {
            sqlx::query(
                "UPDATE tags SET public_topic_count = COALESCE(public_topic_count, 0) + $2 \
                 WHERE id IN (SELECT tag_id FROM topic_tags WHERE topic_id = $1)",
            )
            .bind(topic_id)
            .bind(count)
            .execute(&mut *conn)
            .await?;
        }
    }
    category_auto_watch(&mut *conn, category_id, topic_id).await?;
    category_auto_track(&mut *conn, category_id, topic_id).await?;
    if !silent && !s.get("disable_category_edit_notifications")?.truthy() {
        let (user_id, last_editor_id): (Option<i32>, Option<i32>) =
            sqlx::query_as("SELECT user_id, last_editor_id FROM posts WHERE id = $1")
                .bind(first_post_id)
                .fetch_one(&mut *conn)
                .await?;
        let mut notified: Vec<Option<i32>> = vec![user_id];
        if last_editor_id != user_id {
            notified.push(last_editor_id);
        }
        deferred.push(
            "notify_category_change",
            json!({"post_id": first_post_id, "notified_user_ids": notified}),
        );
    }
    deferred.push(
        "update_topic_upload_security",
        json!({"topic_id": topic_id}),
    );
    if s.get("experimental_topic_category_change_notification")?
        .truthy()
    {
        return Err(Unsupported(
            "experimental_topic_category_change_notification (publish_category_change)",
        )
        .into());
    }
    sqlx::query("UPDATE categories SET topic_count = topic_count + 1 WHERE id = $1")
        .bind(category_id)
        .execute(&mut *conn)
        .await?;
    if let Some(old) = old_category {
        super::create::feature_topics_for(&mut *conn, s, old, None).await?;
    }
    if old_category != Some(category_id) {
        super::create::feature_topics_for(&mut *conn, s, category_id, None).await?;
    }
    Ok(Ok(true))
}

/// `CategoryUser.auto_watch(category_id:, topic_id:)`: the topic's
/// readers watching the category start watching it, and those it had
/// watching for a category no longer theirs go back to tracking.
async fn category_auto_watch(
    conn: &mut PgConnection,
    category_id: i32,
    topic_id: i32,
) -> Result<(), sqlx::Error> {
    use super::notification_levels::{REGULAR, TRACKING, WATCHING};
    use super::notification_reasons::{AUTO_WATCH_CATEGORY, USER_CHANGED};
    sqlx::query(
        "UPDATE topic_users tu \
         SET notification_level = CASE WHEN should_track THEN $3 WHEN should_watch THEN $4 \
                                       ELSE notification_level END, \
             notifications_reason_id = CASE WHEN should_track THEN NULL \
                                            WHEN should_watch THEN $6 \
                                            ELSE notifications_reason_id END \
         FROM ( \
           SELECT tu1.topic_id, tu1.user_id, \
                  CASE WHEN cu.user_id IS NULL AND tu1.notification_level = $4 \
                            AND tu1.notifications_reason_id = $6 THEN true ELSE false END should_track, \
                  CASE WHEN cu.user_id IS NOT NULL AND tu1.notification_level IN ($5, $3) \
                       THEN true ELSE false END should_watch \
           FROM topic_users tu1 \
           JOIN topics t ON t.id = tu1.topic_id \
           LEFT JOIN category_users cu ON cu.category_id = t.category_id \
                AND cu.user_id = tu1.user_id AND cu.notification_level = $4 \
           WHERE COALESCE(tu1.notifications_reason_id, 0) <> $7 AND t.category_id = $1 \
             AND tu1.topic_id = $2 \
         ) AS X \
         WHERE X.topic_id = tu.topic_id AND X.user_id = tu.user_id \
           AND (should_watch OR should_track) AND tu.topic_id = $2",
    )
    .bind(category_id)
    .bind(topic_id)
    .bind(TRACKING)
    .bind(WATCHING)
    .bind(REGULAR)
    .bind(AUTO_WATCH_CATEGORY)
    .bind(USER_CHANGED)
    .execute(conn)
    .await?;
    Ok(())
}

/// `CategoryUser.auto_track(category_id:, topic_id:)`: regular readers
/// tracking the category start tracking the topic.
async fn category_auto_track(
    conn: &mut PgConnection,
    category_id: i32,
    topic_id: i32,
) -> Result<(), sqlx::Error> {
    use super::notification_levels::{REGULAR, TRACKING};
    use super::notification_reasons::{AUTO_TRACK_CATEGORY, USER_CHANGED};
    sqlx::query(
        "UPDATE topic_users tu SET notification_level = $3, notifications_reason_id = $4 \
         FROM topics t, category_users cu \
         WHERE tu.topic_id = t.id AND cu.category_id = t.category_id AND cu.user_id = tu.user_id \
           AND cu.notification_level = $3 AND tu.notification_level = $5 \
           AND COALESCE(tu.notifications_reason_id, 0) <> $6 \
           AND t.category_id = $1 AND tu.topic_id = $2",
    )
    .bind(category_id)
    .bind(topic_id)
    .bind(TRACKING)
    .bind(AUTO_TRACK_CATEGORY)
    .bind(REGULAR)
    .bind(USER_CHANGED)
    .execute(conn)
    .await?;
    Ok(())
}

/// `UrlValidator`: an http(s) URL whose host has a dot. Non-ASCII URLs,
/// which it retries through Addressable's encoding, aren't ported.
fn valid_url(url: &str) -> Result<bool, Unsupported> {
    match RubyUri::parse(url) {
        Some(uri) => Ok(matches!(uri.scheme.as_deref(), Some("http" | "https"))
            && uri.host.as_deref().is_some_and(|h| h.contains('.'))),
        None if !url.is_ascii() => Err(Unsupported("non-ASCII featured links")),
        None => Ok(false),
    }
}

/// `UrlHelper.normalized_encode` for what `URI.parse` reads: the scheme
/// and host lowercased. None for one too long (`ArgumentError`).
fn normalized_encode(url: &str) -> Result<Option<String>, Unsupported> {
    if url.chars().count() > 2_000 {
        return Ok(None);
    }
    let mut uri = RubyUri::parse(url).ok_or(Unsupported(
        "featured links URI can't parse (Addressable's normalized_encode)",
    ))?;
    uri.host = uri.host.map(|h| h.to_lowercase());
    Ok(Some(uri.to_s()))
}

/// `Post#save_reply_relationships` for a post keeping its reply_to.
async fn save_reply_relationship(
    conn: &mut PgConnection,
    topic_id: i32,
    post_id: i32,
    reply_to_post_number: Option<i32>,
) -> Result<(), AppError> {
    let Some(n) = reply_to_post_number else {
        return Ok(());
    };
    let parent: Option<i32> =
        sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = $1 AND post_number = $2")
            .bind(topic_id)
            .bind(n)
            .fetch_optional(&mut *conn)
            .await?;
    if let Some(parent) = parent {
        let inserted = sqlx::query(
            "INSERT INTO post_replies (post_id, reply_post_id, created_at, updated_at) \
             VALUES ($1, $2, clock_timestamp(), clock_timestamp()) ON CONFLICT DO NOTHING",
        )
        .bind(parent)
        .bind(post_id)
        .execute(&mut *conn)
        .await?
        .rows_affected();
        if inserted > 0 {
            sqlx::query("UPDATE posts SET reply_count = reply_count + 1 WHERE id = $1")
                .bind(parent)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}

/// What PostActionNotifier.after_create_post_revision reads.
pub struct NotifyRevision {
    pub topic_id: i32,
    pub author_id: Option<i32>,
    pub editor_id: i32,
    pub first_post: bool,
    pub wiki: bool,
    pub private_message: bool,
    pub silent: bool,
    pub category_changed: bool,
    pub tags_changed: bool,
}

/// `PostActionNotifier.after_create_post_revision`'s notify_post_revision
/// recipients: the author when someone else edits, the topic's watchers
/// for a wiki or freely editable first post; None in messages, for silent
/// revisions, where the settings turn edit notifications off, or with
/// nobody to tell.
pub async fn revision_recipients(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    n: &NotifyRevision,
) -> Result<Option<Vec<i32>>, AppError> {
    let s = ctx.settings;
    if n.private_message || n.silent {
        return Ok(None);
    }
    if (s.get("disable_system_edit_notifications")?.truthy() && n.editor_id == -1)
        || (s.get("disable_category_edit_notifications")?.truthy() && n.category_changed)
        || (s.get("disable_tags_edit_notifications")?.truthy() && n.tags_changed)
    {
        return Ok(None);
    }
    let mut user_ids: Vec<i32> = Vec::new();
    if let Some(author) = n.author_id
        && author != n.editor_id
    {
        user_ids.push(author);
    }
    if n.first_post {
        let unlimited: bool = sqlx::query_scalar(
            "SELECT COALESCE(c.allow_unlimited_owner_edits_on_first_post, FALSE) \
             FROM topics t LEFT JOIN categories c ON c.id = t.category_id WHERE t.id = $1",
        )
        .bind(n.topic_id)
        .fetch_one(&mut *conn)
        .await?;
        if n.wiki || unlimited {
            let watchers: Vec<i32> = sqlx::query_scalar(
                "SELECT user_id FROM topic_users WHERE topic_id = $1 AND notification_level = 3 \
                   AND user_id <> $2 ORDER BY id",
            )
            .bind(n.topic_id)
            .bind(n.editor_id)
            .fetch_all(&mut *conn)
            .await?;
            user_ids.extend(watchers);
        }
    }
    Ok((!user_ids.is_empty()).then_some(user_ids))
}

/// `Post#sync_category_description` from revise_topic: a category
/// definition's description follows its first paragraph, and a change is
/// published on /categories.
pub(crate) async fn sync_category_description(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    cooked: &str,
) -> Result<(), AppError> {
    let category: Option<(i32, Option<String>)> = sqlx::query_as(
        "SELECT id, description FROM categories WHERE topic_id = $1 ORDER BY id LIMIT 1",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((category_id, description)) = category else {
        return Ok(());
    };
    let Some(html) = crate::username_changer::first_paragraph_description(cooked) else {
        return Ok(());
    };
    let template = ctx
        .i18n
        .t_with(
            "category.post_template",
            &[(
                "replace_paragraph",
                ctx.i18n.t("category.replace_paragraph").unwrap_or(""),
            )],
        )
        .ok_or(Unsupported("category.post_template"))?;
    let prefix: String = template.chars().take(51).collect();
    let new_description = (!html.starts_with(&prefix)).then_some(html);
    if description == new_description {
        return Ok(());
    }
    // update_column (no updated_at), then publish_category.
    sqlx::query("UPDATE categories SET description = $2 WHERE id = $1")
        .bind(category_id)
        .bind(&new_description)
        .execute(&mut *conn)
        .await?;
    let cx = crate::category_serializer::Cx {
        config: ctx.config,
        i18n: ctx.i18n,
        defs: &ctx.host.site_setting_defs,
    };
    crate::category_serializer::publish_category(
        &mut *conn,
        &cx,
        ctx.bus,
        ctx.settings,
        category_id,
        &Default::default(),
    )
    .await
}

/// `topic.save!` on a topic as it is (TopicConverter after adding allowed
/// users): before_validation's title reassignment still clears the fancy
/// title, the validations run, and the row is written when that changed
/// anything.
pub async fn save_as_is(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    editor: &Guardian,
    topic_id: i32,
) -> Result<(), AppError> {
    let mut saved: TopicRow = sqlx::query_as(
        "SELECT title, slug, fancy_title, category_id, archetype, featured_link, user_id, \
                slow_mode_seconds, og_image_upload_id \
         FROM topics WHERE id = $1",
    )
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    let author_admin: bool =
        sqlx::query_scalar("SELECT COALESCE((SELECT admin FROM users WHERE id = $1), FALSE)")
            .bind(saved.user_id)
            .fetch_one(&mut *conn)
            .await?;
    let author_trust_level: Option<i32> =
        sqlx::query_scalar("SELECT trust_level FROM users WHERE id = $1")
            .bind(saved.user_id)
            .fetch_optional(&mut *conn)
            .await?;
    let saver = Saver {
        ctx,
        editor,
        topic_id,
        author_admin,
        author_trust_level,
    };
    let mut new = saved.clone();
    let errors = saver.save(&mut *conn, &mut saved, &mut new).await?;
    if !errors.is_empty() {
        return Err(Unsupported("a topic save! that raises RecordInvalid").into());
    }
    Ok(())
}

/// `Category#slug_ref`: `parent:slug` for a subcategory, else its slug.
async fn category_slug_ref(conn: &mut PgConnection, category_id: i32) -> Result<String, AppError> {
    let (slug, parent_slug): (String, Option<String>) = sqlx::query_as(
        "SELECT c.slug, p.slug FROM categories c LEFT JOIN categories p ON p.id = c.parent_category_id \
         WHERE c.id = $1",
    )
    .bind(category_id)
    .fetch_one(conn)
    .await?;
    Ok(match parent_slug {
        Some(parent) => format!("{parent}:{slug}"),
        None => slug,
    })
}

/// A tags param as a list: an array, or a hash's values (Rack's indexed
/// form `tags[0][id]`).
pub fn tag_items(v: &Value) -> Result<Vec<Value>, AppError> {
    Ok(match v {
        Value::Null => Vec::new(),
        Value::String(s) if s.is_empty() => Vec::new(),
        Value::Array(items) => items.clone(),
        Value::Object(map) => map.values().cloned().collect(),
        _ => return Err(Unsupported("tags of another shape").into()),
    })
}

/// `PostRevisor.tag_change_noop?(topic, incoming)`: the incoming tags (names,
/// or objects with ids) are the topic's own, synonyms resolved to their
/// target; blank ones match a topic without tags.
pub async fn tag_change_noop(
    conn: &mut PgConnection,
    topic_id: i32,
    incoming: &Value,
) -> Result<bool, AppError> {
    let current = current_tag_ids(&mut *conn, topic_id).await?;
    let items = tag_items(incoming)?;
    if items.is_empty() {
        return Ok(current.is_empty());
    }
    let ids: Vec<i32> = if items[0].is_string() {
        let mut names: Vec<String> = Vec::new();
        for item in &items {
            let name = item
                .as_str()
                .ok_or(Unsupported("tags of mixed shapes"))?
                .to_lowercase();
            if !names.contains(&name) {
                names.push(name);
            }
        }
        let found: Vec<i32> = sqlx::query_scalar("SELECT id FROM tags WHERE lower(name) = ANY($1)")
            .bind(&names)
            .fetch_all(&mut *conn)
            .await?;
        if found.len() != names.len() {
            return Ok(false);
        }
        found
    } else {
        let mut ids = Vec::new();
        for item in &items {
            match item
                .get("id")
                .and_then(crate::params::scalar)
                .filter(|i| !i.is_empty())
            {
                Some(id) => ids.push(crate::ruby::to_i(&id) as i32),
                None => return Ok(false),
            }
        }
        ids
    };
    if ids.is_empty() {
        return Ok(false);
    }
    let canonical: Vec<i32> = sqlx::query_scalar(
        "SELECT DISTINCT COALESCE(target_tag_id, id) AS id FROM tags WHERE id = ANY($1) ORDER BY 1",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    Ok(canonical == current)
}

/// The topic's tag ids, sorted.
async fn current_tag_ids(conn: &mut PgConnection, topic_id: i32) -> Result<Vec<i32>, sqlx::Error> {
    sqlx::query_scalar("SELECT tag_id FROM topic_tags WHERE topic_id = $1 ORDER BY tag_id")
        .bind(topic_id)
        .fetch_all(conn)
        .await
}
