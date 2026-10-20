//! PostOwnerChanger (app/services/post_owner_changer.rb): posts of one
//! topic handed to a new owner. Each goes through `Post#set_owner`, which
//! is PostRevisor#revise! with the post's own raw, the new user id and the
//! "Ownership transferred" edit reason (a new version, or with
//! `skip_revision` none, as UserMerger asks). Then the new owner's like
//! on it goes, they watch (first post) or track the topic, and the topic's
//! owner, last poster, statistics and reply_to_user_ids follow.
//!
//! Refused: a first post without its author (the topic and post
//! recovered), a raw TextCleaner would change (the post recooked), a
//! category's definition post, a hidden post its new owner edits
//! (flags removed and the post unhidden), staff_edit_locks_post,
//! review_every_post for editors it reviews, slow mode for editors it
//! holds, and the og image regeneration on a like count change.

use chrono::NaiveDateTime;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};

use crate::guardian::Guardian;
use crate::posting::text::{normalize_whitespaces, word_count};
use crate::posting::{
    Ctx, TopicUserAttr, change_topic_user, next_draft_sequence, notification_levels,
    notification_reasons, post_types,
};
use crate::rate_limiter::LimitExceeded;
use crate::session::current::SessionUser;
use crate::topic_guardian::{PostCtx, TopicCtx};
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// `Discourse::SYSTEM_USER_ID`
const SYSTEM_USER_ID: i32 = -1;
/// `UserAction` types
const NEW_TOPIC: i32 = 4;
const REPLY: i32 = 5;
const RESPONSE: i32 = 6;
const WAS_LIKED: i32 = 2;
/// `UserHistory.actions[:post_edit]`
const POST_EDIT: i32 = 53;
/// `Post.hidden_reasons[:flag_threshold_reached]`
const FLAG_THRESHOLD_REACHED: i32 = 1;

/// What `PostOwnerChanger.new` takes, the acting user aside.
pub struct Change {
    pub topic_id: i32,
    pub post_ids: Vec<i32>,
    pub new_owner: i32,
    pub skip_revision: bool,
}

#[derive(Debug)]
pub enum Outcome {
    Done,
    /// `ArgumentError`: no topic (or no posts) to change.
    ArgumentError,
    /// `Discourse::InvalidAccess`: the topic or a post out of sight. The
    /// posts changed before it stay changed.
    InvalidAccess,
    /// `RateLimiter::LimitExceeded` out of the edit or like limiter.
    RateLimited(LimitExceeded),
}

/// The topic as PostOwnerChanger holds it: the columns `save!` may write
/// against the values it last read or wrote.
struct TopicState {
    id: i32,
    archetype: String,
    category_id: Option<i32>,
    slow_mode_seconds: i32,
    user_id: Option<i32>,
    last_post_user_id: Option<i32>,
    saved_user_id: Option<i32>,
    saved_last_post_user_id: Option<i32>,
    saved_category_id: Option<i32>,
}

#[derive(sqlx::FromRow)]
struct PostRow {
    id: i32,
    user_id: Option<i32>,
    post_number: i32,
    post_type: i32,
    raw: String,
    cooked: String,
    edit_reason: Option<String>,
    version: i32,
    public_version: i32,
    hidden: bool,
    hidden_reason_id: Option<i32>,
    wiki: bool,
    reply_to_post_number: Option<i32>,
    reply_quoted: bool,
    created_at: NaiveDateTime,
    deleted_at: Option<NaiveDateTime>,
}

/// `PostOwnerChanger#change_owner!`. `pool` is for the rate limiters,
/// which keep their own books as Redis does.
pub async fn change_owner(
    pool: &PgPool,
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    acting: &Guardian,
    change: &Change,
) -> Result<Outcome, AppError> {
    let s = ctx.settings;
    type TopicRow = (String, Option<i32>, i32, Option<i32>, Option<i32>);
    let topic: Option<TopicRow> = sqlx::query_as(
        "SELECT archetype, category_id, slow_mode_seconds, user_id, last_post_user_id \
         FROM topics WHERE id = $1",
    )
    .bind(change.topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((archetype, category_id, slow_mode_seconds, user_id, last_post_user_id)) = topic
    else {
        return Ok(Outcome::ArgumentError);
    };
    if change.post_ids.is_empty() {
        return Ok(Outcome::ArgumentError);
    }
    let mut topic = TopicState {
        id: change.topic_id,
        archetype,
        category_id,
        slow_mode_seconds,
        user_id,
        last_post_user_id,
        saved_user_id: user_id,
        saved_last_post_user_id: last_post_user_id,
        saved_category_id: category_id,
    };

    let topic_ctx = TopicCtx::load(&mut *conn, s, acting, topic.id)
        .await?
        .ok_or(Unsupported("a topic that vanished"))?;
    let secure = acting.secure_category_ids(&mut *conn, s).await?;
    if !acting.can_see_topic(s, &topic_ctx, true, &secure)? {
        return Ok(Outcome::InvalidAccess);
    }
    let new_owner = SessionUser::load(&mut *conn, change.new_owner)
        .await?
        .ok_or(Unsupported("a new owner that vanished"))?;
    let owner_guardian = Guardian::for_user(&mut *conn, &new_owner).await?;

    for &post_id in &change.post_ids {
        let post: Option<PostRow> = sqlx::query_as(
            "SELECT id, user_id, post_number, post_type, raw, cooked, edit_reason, version, \
                    public_version, hidden, hidden_reason_id, wiki, reply_to_post_number, reply_quoted, \
                    created_at, deleted_at \
             FROM posts WHERE id = $1 AND topic_id = $2",
        )
        .bind(post_id)
        .bind(topic.id)
        .fetch_optional(&mut *conn)
        .await?;
        let Some(post) = post else {
            continue;
        };
        let post_ctx = PostCtx::load(&mut *conn, post.id)
            .await?
            .ok_or(Unsupported("a post that vanished"))?;
        if !acting.can_see_post(s, &post_ctx, true)? {
            return Ok(Outcome::InvalidAccess);
        }
        let first_post = post.post_number == 1;
        if first_post {
            topic.user_id = Some(new_owner.id);
            if post.user_id.is_none() {
                return Err(Unsupported(
                    "changing the owner of a first post without its author (topic recovered)",
                )
                .into());
            }
        }

        // The revision's transaction: a refused edit limiter rolls it back.
        let mut revision = sqlx::Connection::begin(&mut *conn).await?;
        if let Some(refused) = set_owner(
            pool,
            &mut revision,
            ctx,
            acting,
            &mut topic,
            &post,
            &new_owner,
            change,
        )
        .await?
        {
            revision.rollback().await?;
            return Ok(Outcome::RateLimited(refused));
        }
        revision.commit().await?;

        if let crate::likes::Outcome::RateLimited(refused) =
            crate::likes::destroy_with(pool, conn, ctx, &owner_guardian, post.id, true).await?
        {
            return Ok(Outcome::RateLimited(refused));
        }

        let level = if first_post {
            notification_levels::WATCHING
        } else {
            notification_levels::TRACKING
        };
        change_topic_user(
            ctx.bus,
            &mut *conn,
            new_owner.id,
            topic.id,
            &[
                TopicUserAttr::NotificationLevel(level, notification_reasons::USER_CHANGED),
                TopicUserAttr::Posted(true),
            ],
        )
        .await?;

        let last: Option<i32> = sqlx::query_scalar(
            "SELECT id FROM posts WHERE topic_id = $1 AND NOT hidden AND deleted_at IS NULL \
             ORDER BY post_number DESC LIMIT 1",
        )
        .bind(topic.id)
        .fetch_optional(&mut *conn)
        .await?;
        if last == Some(post.id) {
            topic.last_post_user_id = Some(new_owner.id);
        }

        update_statistics(conn, s, topic.id).await?;

        sqlx::query(
            "UPDATE user_stats SET first_post_created_at = \
               (SELECT created_at FROM posts WHERE user_id = $1 AND deleted_at IS NULL \
                ORDER BY created_at LIMIT 1) \
             WHERE user_id = $1",
        )
        .bind(new_owner.id)
        .execute(&mut *conn)
        .await?;

        sqlx::query(
            "UPDATE posts SET reply_to_user_id = $3 WHERE topic_id = $1 AND reply_to_post_number = $2",
        )
        .bind(topic.id)
        .bind(post.post_number)
        .bind(new_owner.id)
        .execute(&mut *conn)
        .await?;

        save_topic(conn, ctx, &mut topic).await?;
    }
    Ok(Outcome::Done)
}

/// `@topic.save!(validate: false)`: ensure_topic_has_a_category, then the
/// changed columns with updated_at. A topic without a category lands in
/// uncategorized: its slow mode and auto close come from there, its
/// posts are queued for reindexing, and it leaves profiles when that
/// category is restricted.
async fn save_topic(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic: &mut TopicState,
) -> Result<(), AppError> {
    let s = ctx.settings;
    // ensure_topic_has_a_category: regular topics only (the schema's
    // has_category_id check keeps them from having none).
    if topic.category_id.is_none() && topic.archetype == "regular" {
        topic.category_id = Some(s.get("uncategorized_category_id")?.to_i() as i32);
    }
    if topic.user_id == topic.saved_user_id
        && topic.last_post_user_id == topic.saved_last_post_user_id
        && topic.category_id == topic.saved_category_id
    {
        return Ok(());
    }
    let category_changed = topic.category_id != topic.saved_category_id;
    if category_changed {
        let tagged: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM topic_tags WHERE topic_id = $1)")
                .bind(topic.id)
                .fetch_one(&mut *conn)
                .await?;
        if tagged {
            return Err(Unsupported(
                "a tagged topic without a category saved into uncategorized (CategoryTagStat.topic_moved)",
            )
            .into());
        }
        // before_save: inherit_slow_mode_from_category
        sqlx::query(
            "UPDATE topics SET category_id = $2, \
               slow_mode_seconds = COALESCE((SELECT default_slow_mode_seconds FROM categories WHERE id = $2), \
                                            slow_mode_seconds) \
             WHERE id = $1",
        )
        .bind(topic.id)
        .bind(topic.category_id)
        .execute(&mut *conn)
        .await?;
        // and inherit_auto_close_from_category
        crate::topic_timer::inherit_auto_close_from_category(
            &mut *conn,
            ctx,
            topic.id,
            crate::topic_timer::types::CLOSE,
        )
        .await?;
    }
    sqlx::query(
        "UPDATE topics SET user_id = $2, last_post_user_id = $3, updated_at = clock_timestamp() \
         WHERE id = $1",
    )
    .bind(topic.id)
    .bind(topic.user_id)
    .bind(topic.last_post_user_id)
    .execute(&mut *conn)
    .await?;
    if category_changed {
        // after_save: SearchIndexer.queue_post_reindex
        sqlx::query(
            "UPDATE post_search_data SET version = 0 FROM posts \
             WHERE post_search_data.post_id = posts.id AND posts.topic_id = $1",
        )
        .bind(topic.id)
        .execute(&mut *conn)
        .await?;
        // after_update: out of the profiles for a restricted category.
        let restricted: bool = sqlx::query_scalar(
            "SELECT COALESCE((SELECT read_restricted FROM categories WHERE id = $1), FALSE)",
        )
        .bind(topic.category_id)
        .fetch_one(&mut *conn)
        .await?;
        if restricted {
            sqlx::query(
                "UPDATE user_profiles SET featured_topic_id = NULL WHERE featured_topic_id = $1",
            )
            .bind(topic.id)
            .execute(&mut *conn)
            .await?;
        }
        topic.saved_category_id = topic.category_id;
    }
    topic.saved_user_id = topic.user_id;
    topic.saved_last_post_user_id = topic.last_post_user_id;
    Ok(())
}

/// `Topic#update_statistics!`: reset_highest, feature_topic_users and
/// update_action_counts.
async fn update_statistics(
    conn: &mut PgConnection,
    s: &crate::site_settings::SiteSettings,
    topic_id: i32,
) -> Result<(), AppError> {
    crate::post_destroyer::reset_highest(&mut *conn, topic_id).await?;
    crate::jobs::choose_featured_users(&mut *conn, topic_id).await?;
    let (old, new, og_image): (i32, i64, bool) = sqlx::query_as(
        "SELECT like_count, (SELECT COALESCE(SUM(like_count), 0) FROM posts \
           WHERE topic_id = $1 AND post_type <> $2 AND deleted_at IS NULL), \
           og_image_upload_id IS NOT NULL \
         FROM topics WHERE id = $1",
    )
    .bind(topic_id)
    .bind(post_types::WHISPER)
    .fetch_one(&mut *conn)
    .await?;
    if og_image && i64::from(old) != new && s.get("generate_topic_og_image")?.truthy() {
        return Err(Unsupported("the topic og image regenerated on a like count change").into());
    }
    sqlx::query("UPDATE topics SET like_count = $2 WHERE id = $1")
        .bind(topic_id)
        .bind(new)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `Post#set_owner(new_user, actor, skip_revision)`: the revision, then
/// the topic's last poster when the post is its highest. A refused edit
/// limiter comes back.
#[allow(clippy::too_many_arguments)]
async fn set_owner(
    pool: &PgPool,
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    editor: &Guardian,
    topic: &mut TopicState,
    post: &PostRow,
    new_owner: &SessionUser,
    change: &Change,
) -> Result<Option<LimitExceeded>, AppError> {
    if post.user_id == Some(new_owner.id) {
        return Ok(None);
    }
    if let Some(refused) =
        revise_owner(pool, conn, ctx, editor, topic, post, new_owner, change).await?
    {
        return Ok(Some(refused));
    }
    let highest: i32 = sqlx::query_scalar("SELECT highest_post_number FROM topics WHERE id = $1")
        .bind(topic.id)
        .fetch_one(&mut *conn)
        .await?;
    if post.post_number == highest {
        // update_columns: written, and no longer a change for save!.
        sqlx::query("UPDATE topics SET last_post_user_id = $2 WHERE id = $1")
            .bind(topic.id)
            .bind(new_owner.id)
            .execute(&mut *conn)
            .await?;
        topic.last_post_user_id = Some(new_owner.id);
        topic.saved_last_post_user_id = Some(new_owner.id);
    }
    Ok(None)
}

/// `PostRevisor#revise!(editor, {raw:, user_id:, edit_reason:},
/// bypass_bump: true, skip_revision:, skip_validations: true)` for an owner
/// change.
#[allow(clippy::too_many_arguments)]
async fn revise_owner(
    pool: &PgPool,
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    editor: &Guardian,
    topic: &TopicState,
    post: &PostRow,
    new_owner: &SessionUser,
    change: &Change,
) -> Result<Option<LimitExceeded>, AppError> {
    let s = ctx.settings;
    let editor_user = editor
        .user()
        .ok_or(Unsupported("changing a post's owner anonymously"))?
        .clone();
    let raw = if post.raw.is_empty() {
        String::new()
    } else {
        normalize_whitespaces(&post.raw).trim_end().to_string()
    };
    if raw != post.raw {
        return Err(Unsupported("an owner change TextCleaner changes the raw of").into());
    }
    let category_definition: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE topic_id = $1)")
            .bind(topic.id)
            .fetch_one(&mut *conn)
            .await?;
    if category_definition && post.post_number == 1 {
        return Err(Unsupported("changing the owner of a category's definition post").into());
    }
    if topic.slow_mode_seconds > 0 && !editor.is_staff() {
        return Err(Unsupported("changing a post's owner in slow mode").into());
    }
    let edit_reason = ctx.t("change_owner.post_revision_text");
    let new_version = !change.skip_revision;
    let self_edit = editor_user.id == new_owner.id;
    let private_message = topic.archetype == "private_message";
    if self_edit && post.hidden && post.hidden_reason_id == Some(FLAG_THRESHOLD_REACHED) {
        return Err(Unsupported("a hidden post's new owner unhiding it").into());
    }

    // update_post: the previous owner's reply and topic actions.
    let prev_owner = match post.user_id {
        Some(id) => sqlx::query_scalar::<_, i32>("SELECT id FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?
            .map(Some)
            .ok_or(Unsupported("a post whose author row is gone"))?,
        None => None,
    };
    if let Some(prev) = prev_owner {
        sqlx::query(
            "UPDATE user_actions SET user_id = $3 WHERE target_post_id = $1 AND user_id = $2 \
             AND action_type IN ($4, $5)",
        )
        .bind(post.id)
        .bind(prev)
        .bind(new_owner.id)
        .bind(REPLY)
        .bind(RESPONSE)
        .execute(&mut *conn)
        .await?;
        if post.post_number == 1 {
            sqlx::query(
                "UPDATE user_actions SET user_id = $3 WHERE target_topic_id = $1 AND user_id = $2 \
                 AND action_type = $4",
            )
            .bind(topic.id)
            .bind(prev)
            .bind(new_owner.id)
            .bind(NEW_TOPIC)
            .execute(&mut *conn)
            .await?;
        }
    }
    let quoted = crate::posting::quotes::quoted_post_numbers(&raw, topic.id, post.post_number);
    let version = post.version + i32::from(new_version);
    let public_version = post.public_version + i32::from(new_version);
    let edit_reason_saved: String = edit_reason.chars().take(1000).collect();
    sqlx::query(
        "UPDATE posts SET user_id = $2, edit_reason = $3, last_editor_id = $4, word_count = $5, \
                self_edits = self_edits + $6, quote_count = $7, version = $8, public_version = $9, \
                last_version_at = CASE WHEN $10 THEN clock_timestamp() ELSE last_version_at END, \
                updated_at = clock_timestamp() \
         WHERE id = $1",
    )
    .bind(post.id)
    .bind(new_owner.id)
    .bind(&edit_reason_saved)
    .bind(editor_user.id)
    .bind(word_count(&raw))
    .bind(i32::from(self_edit))
    .bind(quoted.len() as i32)
    .bind(version)
    .bind(public_version)
    .bind(new_version)
    .execute(&mut *conn)
    .await?;
    // @post.link_post_uploads
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
    // save_reply_relationships: the reply_to_post_number and quoted posts.
    if post.deleted_at.is_none() {
        let mut numbers: Vec<i32> = quoted.iter().flatten().copied().collect();
        numbers.extend(post.reply_to_post_number);
        for n in numbers {
            let parent: Option<i32> =
                sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = $1 AND post_number = $2")
                    .bind(topic.id)
                    .bind(n)
                    .fetch_optional(&mut *conn)
                    .await?;
            let Some(parent) = parent else {
                continue;
            };
            let inserted = sqlx::query(
                "INSERT INTO post_replies (post_id, reply_post_id, created_at, updated_at) \
                 VALUES ($1, $2, clock_timestamp(), clock_timestamp()) ON CONFLICT DO NOTHING",
            )
            .bind(parent)
            .bind(post.id)
            .execute(&mut *conn)
            .await?
            .rows_affected();
            if inserted > 0 && post.post_type != post_types::WHISPER {
                sqlx::query("UPDATE posts SET reply_count = reply_count + 1 WHERE id = $1")
                    .bind(parent)
                    .execute(&mut *conn)
                    .await?;
            }
        }
    }
    if editor_user.id != SYSTEM_USER_ID {
        sqlx::query(
            "UPDATE user_stats SET post_edits_count = COALESCE(post_edits_count, 0) + 1 WHERE user_id = $1",
        )
        .bind(editor_user.id)
        .execute(&mut *conn)
        .await?;
    }
    // The owners' stats: likes received, the topic or post count, the
    // first post date.
    if let Some(prev) = prev_owner
        && prev != new_owner.id
    {
        let likes = sqlx::query(
            "UPDATE user_actions SET user_id = $3 WHERE target_post_id = $1 AND user_id = $2 \
             AND action_type = $4",
        )
        .bind(post.id)
        .bind(prev)
        .bind(new_owner.id)
        .bind(WAS_LIKED)
        .execute(&mut *conn)
        .await?
        .rows_affected() as i32;
        let column = if post.post_number == 1 {
            Some("topic_count")
        } else if post.post_type == post_types::REGULAR {
            Some("post_count")
        } else {
            None
        };
        let prev_stat: Option<(i32, i32, i32, Option<NaiveDateTime>)> = sqlx::query_as(
            "SELECT topic_count, post_count, likes_received, first_post_created_at \
             FROM user_stats WHERE user_id = $1",
        )
        .bind(prev)
        .fetch_optional(&mut *conn)
        .await?;
        let (prev_topics, prev_posts, prev_likes, prev_first) =
            prev_stat.ok_or(Unsupported("a previous owner without user stats"))?;
        let mut prev_likes_received = prev_likes;
        if !private_message {
            if post.deleted_at.is_none()
                && let Some(column) = column
            {
                let count = if column == "topic_count" {
                    prev_topics
                } else {
                    prev_posts
                };
                if count >= 1 {
                    sqlx::query(&format!(
                        "UPDATE user_stats SET {column} = COALESCE({column}, 0) - 1 WHERE user_id = $1"
                    ))
                    .bind(prev)
                    .execute(&mut *conn)
                    .await?;
                }
            }
            prev_likes_received -= likes;
        }
        // The previous owner's in-memory likes_received is only saved with
        // the first post date.
        if prev_first == Some(post.created_at) {
            sqlx::query(
                "UPDATE user_stats SET likes_received = $2, first_post_created_at = \
                   (SELECT created_at FROM posts WHERE user_id = $1 AND deleted_at IS NULL \
                    ORDER BY created_at LIMIT 1) \
                 WHERE user_id = $1",
            )
            .bind(prev)
            .bind(prev_likes_received)
            .execute(&mut *conn)
            .await?;
        }
        if !private_message {
            if post.deleted_at.is_none()
                && let Some(column) = column
            {
                sqlx::query(&format!(
                    "UPDATE user_stats SET {column} = COALESCE({column}, 0) + 1 WHERE user_id = $1"
                ))
                .bind(new_owner.id)
                .execute(&mut *conn)
                .await?;
            }
            if likes != 0 {
                sqlx::query(
                    "UPDATE user_stats SET likes_received = likes_received + $2 WHERE user_id = $1",
                )
                .bind(new_owner.id)
                .bind(likes)
                .execute(&mut *conn)
                .await?;
            }
        }
    }
    // create_or_update_revision: none with skip_revision.
    if new_version {
        let mut fields: Vec<(&str, [Value; 2])> = Vec::new();
        if post.edit_reason.as_deref() != Some(edit_reason_saved.as_str()) {
            fields.push((
                "edit_reason",
                [json!(post.edit_reason), json!(edit_reason_saved)],
            ));
        }
        fields.push(("user_id", [json!(post.user_id), json!(new_owner.id)]));
        let yaml = crate::modifications::dump(&fields)?;
        let revision_id: i32 = sqlx::query_scalar(
            "INSERT INTO post_revisions (user_id, post_id, number, modifications, hidden, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, FALSE, clock_timestamp(), clock_timestamp()) RETURNING id",
        )
        .bind(editor_user.id)
        .bind(post.id)
        .bind(version)
        .bind(&yaml)
        .fetch_one(&mut *conn)
        .await?;
        // PostActionNotifier.after_create_post_revision: `post_revision.post`
        // finds no deleted post.
        let notify_disabled = private_message
            || post.deleted_at.is_some()
            || (s.get("disable_system_edit_notifications")?.truthy()
                && editor_user.id == SYSTEM_USER_ID);
        if !notify_disabled {
            let mut user_ids: Vec<i32> = Vec::new();
            if editor_user.id != new_owner.id {
                user_ids.push(new_owner.id);
            }
            if post.post_number == 1 {
                let unlimited: bool = sqlx::query_scalar(
                    "SELECT COALESCE(c.allow_unlimited_owner_edits_on_first_post, FALSE) \
                     FROM topics t LEFT JOIN categories c ON c.id = t.category_id WHERE t.id = $1",
                )
                .bind(topic.id)
                .fetch_one(&mut *conn)
                .await?;
                if post.wiki || unlimited {
                    let watchers: Vec<i32> = sqlx::query_scalar(
                        "SELECT user_id FROM topic_users WHERE topic_id = $1 AND notification_level = 3 \
                           AND user_id <> $2 ORDER BY id",
                    )
                    .bind(topic.id)
                    .bind(editor_user.id)
                    .fetch_all(&mut *conn)
                    .await?;
                    user_ids.extend(watchers);
                }
            }
            if !user_ids.is_empty() {
                crate::jobs::enqueue(
                    &mut *conn,
                    "notify_post_revision",
                    json!({ "user_ids": user_ids, "post_revision_id": revision_id }),
                )
                .await?;
            }
        }
        // perform_edit
        if ctx.config.rate_limits {
            let limiter = crate::posting::limits::edit_limiter(
                s,
                crate::rate_limiter::LimitUser {
                    id: editor_user.id,
                    staff: editor.is_staff(),
                },
                editor_user.trust_level,
            )?;
            if let Err(refused) = limiter.performed(pool, true).await? {
                return Ok(Some(refused));
            }
        }
    }
    // revise_topic: the first post's excerpt.
    if post.post_number == 1 {
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
            .bind(topic.id)
            .bind(excerpt)
            .execute(&mut *conn)
            .await?;
    }
    // advance_draft_sequence for the last editor.
    next_draft_sequence(&mut *conn, editor_user.id, &format!("topic_{}", topic.id)).await?;

    // After the transaction.
    if s.get("staff_edit_locks_post")?.truthy()
        && !post.wiki
        && editor.is_staff()
        && editor_user.id != SYSTEM_USER_ID
        && !(new_owner.admin || new_owner.moderator)
    {
        return Err(Unsupported("staff_edit_locks_post").into());
    }
    // StaffActionLogger#log_post_edit: the raw was among the fields.
    if editor.is_staff() && editor_user.id != new_owner.id {
        let truncate = |s: &str| {
            if s.chars().count() > 50_000 {
                format!("{}...", s.chars().take(50_001).collect::<String>())
            } else {
                s.to_string()
            }
        };
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, post_id, details, admin_only, created_at, updated_at) \
             VALUES ($4, $1, $2, $3, FALSE, clock_timestamp(), clock_timestamp())",
        )
        .bind(editor_user.id)
        .bind(post.id)
        .bind(format!("{}\n\n---\n\n{}", truncate(&raw), truncate(&raw)))
        .bind(POST_EDIT)
        .execute(&mut *conn)
        .await?;
    }
    // QuotedPost.extract_from and TopicLink.extract_from
    crate::posting::rebake::extract_quoted_posts(
        &mut *conn,
        post.id,
        topic.id,
        post.post_number,
        post.reply_to_post_number,
        post.reply_quoted,
        &post.cooked,
    )
    .await?;
    let base_path = ctx.config.globals.relative_url_root().to_string();
    crate::posting::links::extract_from(
        &mut *conn,
        &crate::posting::links::Site {
            hostname: &hostname,
            base_path: &base_path,
            base_url_no_prefix: &urls.base_url_no_prefix()?,
            settings: s,
        },
        &crate::posting::links::LinkPost {
            id: post.id,
            user_id: new_owner.id,
            topic_id: topic.id,
            cooked: &post.cooked,
        },
    )
    .await?;
    // Only the owner changed: no Topic.reset_highest. post_process_post,
    // alert_users, publish_changes, grant_badge.
    crate::jobs::enqueue(
        &mut *conn,
        "process_post",
        json!({"bypass_bump": false, "cooking_options": null, "new_post": false, "post_id": post.id, "skip_pull_hotlinked_images": false, "invalidate_oneboxes": true}),
    )
    .await?;
    if editor_user.id != SYSTEM_USER_ID {
        crate::jobs::enqueue(&mut *conn, "post_alert", json!({"post_id": post.id})).await?;
    }
    crate::bus::publish_post_change(
        ctx,
        &mut *conn,
        post.id,
        "revised",
        Default::default(),
        false,
    )
    .await?;
    crate::badge_granter::queue(
        &mut *conn,
        s,
        crate::badge_granter::Trigger::PostRevision,
        &[post.id],
    )
    .await?;
    // ReviewablePost.queue_for_review_if_possible, for editors it reviews.
    if s.get("review_every_post")?.truthy()
        && !change.skip_revision
        && editor_user.id > 0
        && !editor.is_staff()
        && editor_user.trust_level < 4
    {
        return Err(Unsupported("review_every_post on an owner change").into());
    }
    Ok(None)
}
