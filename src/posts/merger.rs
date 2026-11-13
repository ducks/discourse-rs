//! PostMerger: several posts of one author in one topic become the last
//! of them, revised to hold every raw; the others are destroyed in the
//! revision's transaction.
//!
//! Refused: a merged post that fails validation (Rails runs the destroys,
//! rolls them back and answers 200, their live updates already sent), and
//! whatever PostRevisor and PostDestroyer refuse.

use sqlx::PgPool;

use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::posting::revise::{self, Changes};
use crate::posting::text::sanitized_length;
use crate::topic_guardian::TopicCtx;
use crate::{AppError, Unsupported, post_destroyer};

pub enum Outcome {
    Done,
    /// `PostMerger::CannotMergeError`, with its message.
    CannotMerge(String),
    InvalidAccess,
    /// EditRateLimiter refused the revision, before anything was destroyed.
    RateLimited(crate::rate_limiter::LimitExceeded),
}

#[derive(sqlx::FromRow)]
struct Post {
    id: i32,
    topic_id: i32,
    user_id: Option<i32>,
    post_number: i32,
    raw: String,
}

/// `PostMerger.new(user, posts).merge`, `posts` being
/// `Post.where(id: post_ids).order(:id)`.
pub async fn merge(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_ids: &[i32],
) -> Result<Outcome, AppError> {
    let s = ctx.settings;
    let user = guardian.user().ok_or(Unsupported("merging anonymously"))?;
    let mut conn = pool.acquire().await?;
    let mut posts: Vec<Post> = sqlx::query_as(
        "SELECT id, topic_id, user_id, post_number, raw FROM posts \
         WHERE id = ANY($1) AND deleted_at IS NULL ORDER BY id",
    )
    .bind(post_ids)
    .fetch_all(&mut *conn)
    .await?;
    if posts.len() < 2 {
        return Ok(Outcome::Done);
    }
    // ensure_same_topic!, ensure_same_user!
    if posts.iter().any(|p| p.topic_id != posts[0].topic_id) {
        return Ok(Outcome::CannotMerge(
            ctx.t("merge_posts.errors.different_topics"),
        ));
    }
    if posts.iter().any(|p| p.user_id != posts[0].user_id) {
        return Ok(Outcome::CannotMerge(
            ctx.t("merge_posts.errors.different_users"),
        ));
    }
    // ensure_can_merge!: can_moderate_topic?, a trashed topic being none.
    let topic = TopicCtx::load(&mut conn, s, guardian, posts[0].topic_id)
        .await?
        .filter(|t| !t.trashed());
    let Some(topic) = topic else {
        return Ok(Outcome::InvalidAccess);
    };
    let secure = guardian.secure_category_ids(&mut conn, s).await?;
    let can_see_topic = guardian.can_see_topic(s, &topic, true, &secure)?;
    if !guardian.is_staff()
        && !guardian.can_perform_action_available_to_group_moderators(s, can_see_topic)?
    {
        return Ok(Outcome::InvalidAccess);
    }
    // guardian.ensure_can_delete!(post), each as the sort reaches it.
    for post in &posts {
        if post_destroyer::can_delete(&mut conn, ctx, guardian, post.id, post.topic_id, false)
            .await?
            .is_none()
        {
            return Ok(Outcome::InvalidAccess);
        }
    }
    posts.sort_by_key(|p| p.post_number);
    let raw = posts
        .iter()
        .map(|p| p.raw.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let Some(kept) = posts.pop() else {
        return Ok(Outcome::Done);
    };
    let edit_reason = ctx
        .i18n
        .t_count(
            "merge_posts.edit_reason",
            posts.len() as i64,
            &[("username", &user.username)],
        )
        .ok_or(Unsupported("merge_posts.edit_reason without a translation"))?;
    // ensure_max_post_length!
    if sanitized_length(&raw, false) as i64 > s.get("max_post_length")?.to_i() {
        return Ok(Outcome::CannotMerge(
            ctx.t("merge_posts.errors.max_post_length"),
        ));
    }
    // Each PostDestroyer holds its post and topic from before the revision.
    let mut merged = Vec::with_capacity(posts.len());
    for post in &posts {
        merged
            .push(post_destroyer::prepare(&mut conn, ctx, guardian, post.id, can_see_topic).await?);
    }
    drop(conn);
    let changes = Changes {
        raw: Some(raw),
        edit_reason: Some(edit_reason),
        merged,
        ..Default::default()
    };
    Ok(
        match revise::revise(pool, ctx, guardian, kept.id, changes).await? {
            revise::Outcome::Revised => Outcome::Done,
            revise::Outcome::RateLimited(refused) => Outcome::RateLimited(refused),
            revise::Outcome::Invalid(_) => {
                return Err(Unsupported("merging posts into one that fails validation").into());
            }
            revise::Outcome::EditConflict => {
                return Err(Unsupported("an edit conflict merging posts").into());
            }
        },
    )
}
