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
}

impl TopicFields {
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.archetype.is_none()
            && self.category_id.is_none()
            && self.featured_link.is_none()
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
/// fields, opts)` for topic fields.
pub async fn revise(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    editor: &Guardian,
    topic_id: i32,
    fields: &TopicFields,
    opts: Opts,
) -> Result<Outcome, AppError> {
    let s = ctx.settings;
    let editor_user = editor
        .user()
        .ok_or(Unsupported("revising a topic anonymously"))?
        .clone();
    let mut conn = pool.acquire().await?;
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
    // time counts) && slow_mode_prevents_editing: an error on the post,
    // which the controller doesn't show.
    let grace = s.get("editing_grace_period")?.to_i();
    let elapsed = (revised_at - post.last_version_at).num_milliseconds() as f64 / 1000.0;
    if topic.slow_mode_seconds > 0
        && editor_user.id > 0
        && !editor.is_staff()
        && elapsed > grace as f64
        && s.get("slow_mode_prevents_editing")?.truthy()
    {
        return Ok(Outcome::Invalid(Vec::new()));
    }
    let can_edit_featured_link = match &fields.featured_link {
        Some(link) if !link.trim().is_empty() => {
            crate::topic_guardian::can_edit_featured_link(
                &mut conn,
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
            can_change_archetype(&mut conn, ctx, editor, topic_id, archetype).await?
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
    drop(conn);

    let mut tx = pool.begin().await?;
    // update_post: a new version with no post fields changed.
    let public_step = i32::from(!opts.hidden);
    sqlx::query(
        "UPDATE posts SET version = version + 1, public_version = public_version + $2, \
                last_version_at = $3, edit_reason = NULL, last_editor_id = $4, \
                self_edits = self_edits + $5, updated_at = clock_timestamp() \
         WHERE id = $1",
    )
    .bind(post.id)
    .bind(public_step)
    .bind(revised_at)
    .bind(editor_user.id)
    .bind(i32::from(self_edit))
    .execute(&mut *tx)
    .await?;
    let urls = Urls {
        config: ctx.config,
        settings: s,
    };
    let hostname = urls.current_hostname()?;
    let store = crate::file_store::FileStore::for_site(ctx.config, s)?;
    crate::upload_references::link_post_uploads(
        &mut tx,
        s,
        &hostname,
        &store,
        post.id,
        &post.cooked,
    )
    .await?;
    save_reply_relationship(&mut tx, topic_id, post.id, post.reply_to_post_number).await?;
    if editor_user.id != -1 {
        sqlx::query(
            "UPDATE user_stats SET post_edits_count = COALESCE(post_edits_count, 0) + 1 WHERE user_id = $1",
        )
        .bind(editor_user.id)
        .execute(&mut *tx)
        .await?;
    }

    // update_topic: the field callbacks in tracking order, then the save.
    let mut diff: Vec<(&str, [Value; 2])> = Vec::new();
    let mut new = topic.clone();
    let mut errored = false;
    let mut errors: Vec<String> = Vec::new();
    if let Some(title) = &fields.title {
        record_change(&mut diff, "title", json!(topic.title), json!(title));
        new.title = title.clone();
    }
    if !errored && let Some(archetype) = &fields.archetype {
        if can_change_archetype {
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
    if !errored
        && let Some(category_id) = fields.category_id
        && Some(category_id) != topic.category_id
    {
        return Err(Unsupported("changing a topic's category (188b)").into());
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
        } else if !can_edit_featured_link {
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
    let mut title_changed = false;
    if !errored {
        errors = save_topic(
            &mut tx,
            ctx,
            editor,
            topic_id,
            &topic,
            &mut new,
            author_admin,
            author_trust_level,
        )
        .await?;
        title_changed = new.title != topic.title;
        errored = !errors.is_empty();
    }
    if errored {
        tx.rollback().await?;
        // advance_draft_sequence ran in the rolled back transaction: its
        // upsert has taken a draft_sequences id all the same.
        if !opts.keep_existing_draft && editor_user.id > 0 {
            sqlx::query("SELECT nextval('draft_sequences_id_seq')")
                .execute(pool)
                .await?;
        }
        return Ok(Outcome::Invalid(errors));
    }

    // create_revision: post_changes (only edit_reason can change here)
    // merged with the topic's diff.
    let mut modifications: Vec<(&str, [Value; 2])> = Vec::new();
    if post.edit_reason.is_some() {
        modifications.push(("edit_reason", [json!(post.edit_reason), Value::Null]));
    }
    modifications.extend(diff.iter().cloned());
    let yaml = modifications::dump(&modifications)?;
    let (revision_id, version): (i32, i32) = sqlx::query_as(
        "INSERT INTO post_revisions (user_id, post_id, number, modifications, hidden, created_at, updated_at) \
         SELECT $1, id, version, $3, $4, clock_timestamp(), clock_timestamp() FROM posts WHERE id = $2 \
         RETURNING id, number",
    )
    .bind(editor_user.id)
    .bind(post.id)
    .bind(&yaml)
    .bind(opts.hidden)
    .fetch_one(&mut *tx)
    .await?;
    let _ = version;
    notify_revision(
        &mut tx,
        ctx,
        &NotifyRevision {
            topic_id,
            post_id: post.id,
            author_id: post.user_id,
            editor_id: editor_user.id,
            first_post: true,
            wiki: post.wiki,
            private_message: new.archetype == "private_message",
            silent: opts.silent,
            revision_id,
            category_changed: modifications.iter().any(|(k, _)| *k == "category_id"),
            tags_changed: false,
        },
    )
    .await?;
    // perform_edit
    if ctx.config.rate_limits {
        let limiter = super::limits::edit_limiter(
            s,
            crate::rate_limiter::LimitUser {
                id: editor_user.id,
                staff: editor.is_staff(),
            },
            editor_user.trust_level,
        )?;
        if let Err(refused) = limiter.performed(pool, true).await? {
            tx.rollback().await?;
            return Ok(Outcome::RateLimited(refused));
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
    );
    sqlx::query("UPDATE topics SET excerpt = $2 WHERE id = $1")
        .bind(topic_id)
        .bind(excerpt)
        .execute(&mut *tx)
        .await?;
    sync_category_description(&mut tx, ctx, topic_id, &post.cooked).await?;
    if !opts.keep_existing_draft {
        next_draft_sequence(&mut tx, editor_user.id, &format!("topic_{topic_id}")).await?;
    }
    // post_process_post, alert_users and publish_changes, committed with
    // the edit.
    crate::jobs::enqueue(
        &mut tx,
        "process_post",
        json!({"bypass_bump": false, "cooking_options": null, "new_post": false, "post_id": post.id, "skip_pull_hotlinked_images": false, "invalidate_oneboxes": true}),
    )
    .await?;
    // :post_edited with topic_changed: topic_edited's web hooks for the
    // first post.
    crate::web_hooks::enqueue_topic_hooks(
        &mut tx,
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
    if editor_user.id != -1 {
        crate::jobs::enqueue(&mut tx, "post_alert", json!({"post_id": post.id})).await?;
    }
    let mut publish = Map::new();
    if !diff.is_empty() {
        publish.insert("reload_topic".into(), json!(true));
    }
    crate::bus::publish_post_change(ctx, &mut tx, post.id, "revised", publish, false).await?;
    crate::badge_granter::queue(
        &mut tx,
        s,
        crate::badge_granter::Trigger::PostRevision,
        &[post.id],
    )
    .await?;
    tx.commit().await?;

    // bump_topic: a wiki first post whose post fields changed (only the
    // edit reason can, here).
    if !opts.bypass_bump && post.wiki && post.edit_reason.is_some() {
        let mut tx = pool.begin().await?;
        sqlx::query("UPDATE topics SET bumped_at = clock_timestamp() WHERE id = $1")
            .bind(topic_id)
            .execute(&mut *tx)
            .await?;
        use crate::topic_tracking_state as tracking;
        tracking::publish_muted(ctx.bus, &mut tx, topic_id).await?;
        tracking::publish_unmuted(ctx.bus, &mut tx, topic_id).await?;
        tracking::publish_latest(ctx.bus, s, &mut tx, topic_id).await?;
        tx.commit().await?;
    }

    let mut conn = pool.acquire().await?;
    let base_path = ctx.config.globals.relative_url_root().to_string();
    if let Some(user_id) = post.user_id {
        super::links::extract_from(
            &mut conn,
            &super::links::Site {
                hostname: &hostname,
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
    crate::post_destroyer::reset_highest(&mut conn, topic_id).await?;
    // ReviewablePost.queue_for_review_if_possible
    if s.get("review_every_post")?.truthy()
        && new.archetype != "private_message"
        && editor_user.id > 0
        && !editor.is_staff()
        && !editor.has_trust_level(4)
    {
        return Err(Unsupported("review_every_post on a topic edit").into());
    }
    let _ = title_changed;
    Ok(Outcome::Revised)
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

/// `@topic.save(validate: true)`: before_validation's cleanup (the title
/// assigned again through `title=`, the featured link stripped), the
/// validations, before_save's fancy title and category, the row written
/// when anything changed, and after_save's search index. Returns the
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
        return Err(Unsupported("saving a regular topic without a category").into());
    }
    if title_changed {
        new.fancy_title = Some(fancy_title(&new.title, EmojiEscape::from_settings(s)?)?);
    }
    if (title_changed || category_changed) && old.og_image_upload_id.is_some() {
        return Err(Unsupported("regenerating a topic's og image").into());
    }
    let changed = title_changed
        || new.slug != old.slug
        || new.fancy_title != old.fancy_title
        || new.archetype != old.archetype
        || new.featured_link != old.featured_link
        || category_changed;
    if changed {
        sqlx::query(
            "UPDATE topics SET title = $2, slug = $3, fancy_title = $4, archetype = $5, \
                    featured_link = $6, category_id = $7, updated_at = clock_timestamp() \
             WHERE id = $1",
        )
        .bind(topic_id)
        .bind(&new.title)
        .bind(&new.slug)
        .bind(&new.fancy_title)
        .bind(&new.archetype)
        .bind(&new.featured_link)
        .bind(new.category_id)
        .execute(&mut *conn)
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
    Ok(Vec::new())
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
    pub post_id: i32,
    pub author_id: Option<i32>,
    pub editor_id: i32,
    pub first_post: bool,
    pub wiki: bool,
    pub private_message: bool,
    pub silent: bool,
    pub revision_id: i32,
    pub category_changed: bool,
    pub tags_changed: bool,
}

/// `PostActionNotifier.after_create_post_revision`: the author when
/// someone else edits, the topic's watchers for a wiki or freely editable
/// first post; none in messages, for silent revisions, or where the
/// settings turn edit notifications off.
pub async fn notify_revision(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    n: &NotifyRevision,
) -> Result<(), AppError> {
    let s = ctx.settings;
    if n.private_message || n.silent {
        return Ok(());
    }
    if (s.get("disable_system_edit_notifications")?.truthy() && n.editor_id == -1)
        || (s.get("disable_category_edit_notifications")?.truthy() && n.category_changed)
        || (s.get("disable_tags_edit_notifications")?.truthy() && n.tags_changed)
    {
        return Ok(());
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
    if !user_ids.is_empty() {
        crate::jobs::enqueue(
            conn,
            "notify_post_revision",
            json!({ "user_ids": user_ids, "post_revision_id": n.revision_id }),
        )
        .await?;
    }
    let _ = n.post_id;
    Ok(())
}

/// `Post#sync_category_description` from revise_topic: a category
/// definition's description follows its first paragraph. Changing it
/// publishes the category on /categories (CategorySerializer, not
/// ported), so a change is refused.
async fn sync_category_description(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    cooked: &str,
) -> Result<(), AppError> {
    let category: Option<Option<String>> = sqlx::query_scalar(
        "SELECT description FROM categories WHERE topic_id = $1 ORDER BY id LIMIT 1",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(description) = category else {
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
    Err(Unsupported(
        "a category description changing on its definition's revision (publish_category's CategorySerializer)",
    )
    .into())
}
