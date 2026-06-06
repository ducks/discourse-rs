//! Like / unlike business logic. Pure functions over a `&mut PgConnection`
//! so they can be called from route handlers, background jobs, or tests
//! without an actix runtime in scope.

use diesel::prelude::*;
use diesel::result::Error as DieselError;
use serde_json::json;

use crate::models::{NewNotification, NewPostLike, PostLike};
use crate::schema::{notifications, post_likes, posts, users};

#[derive(Debug)]
pub enum LikeError {
    PostNotFound,
    SelfLike,
    Db(DieselError),
}

impl From<DieselError> for LikeError {
    fn from(e: DieselError) -> Self {
        match e {
            DieselError::NotFound => LikeError::PostNotFound,
            other => LikeError::Db(other),
        }
    }
}

#[derive(Debug)]
pub enum LikeOutcome {
    /// A new like row was inserted; counters were incremented.
    Created(PostLike),
    /// The user had already liked this post; nothing changed.
    AlreadyLiked(PostLike),
}

impl LikeOutcome {
    pub fn into_like(self) -> PostLike {
        match self {
            LikeOutcome::Created(l) | LikeOutcome::AlreadyLiked(l) => l,
        }
    }
}

#[derive(Debug)]
pub enum UnlikeOutcome {
    /// A like row was deleted; counters were decremented.
    Removed,
    /// No like existed (post missing or user never liked). No-op.
    NothingToRemove,
}

/// Like a post. Wraps everything in a single transaction so counters and
/// the like row stay consistent on failure. Idempotent: re-liking a post
/// returns `AlreadyLiked` rather than erroring.
pub fn like_post(
    conn: &mut PgConnection,
    user_id: i32,
    post_id: i32,
) -> Result<LikeOutcome, LikeError> {
    conn.transaction::<LikeOutcome, LikeError, _>(|conn| {
        // Find the post and its author. Filter out hidden posts here so we
        // never create likes on invisible content.
        let (author_id, deleted): (i32, Option<chrono::NaiveDateTime>) = posts::table
            .filter(posts::id.eq(post_id))
            .filter(posts::hidden.eq(false))
            .select((posts::user_id, posts::deleted_at))
            .first(conn)
            .map_err(LikeError::from)?;

        if deleted.is_some() {
            return Err(LikeError::PostNotFound);
        }

        if author_id == user_id {
            return Err(LikeError::SelfLike);
        }

        let new_like = NewPostLike { user_id, post_id };
        let inserted: Option<PostLike> = diesel::insert_into(post_likes::table)
            .values(&new_like)
            .on_conflict((post_likes::user_id, post_likes::post_id))
            .do_nothing()
            .returning(PostLike::as_returning())
            .get_result(conn)
            .optional()
            .map_err(LikeError::from)?;

        let Some(like) = inserted else {
            // Already liked. Fetch the existing row for the response.
            let existing: PostLike = post_likes::table
                .filter(post_likes::user_id.eq(user_id))
                .filter(post_likes::post_id.eq(post_id))
                .first(conn)
                .map_err(LikeError::from)?;
            return Ok(LikeOutcome::AlreadyLiked(existing));
        };

        // Bump denormalized counters in the same tx as the insert so they
        // can't drift on partial failure.
        diesel::update(posts::table.filter(posts::id.eq(post_id)))
            .set(posts::like_count.eq(posts::like_count + 1))
            .execute(conn)
            .map_err(LikeError::from)?;

        diesel::update(users::table.filter(users::id.eq(user_id)))
            .set(users::likes_given.eq(users::likes_given + 1))
            .execute(conn)
            .map_err(LikeError::from)?;

        diesel::update(users::table.filter(users::id.eq(author_id)))
            .set(users::likes_received.eq(users::likes_received + 1))
            .execute(conn)
            .map_err(LikeError::from)?;

        // Notify the post author. No consolidation in v1 — N likes produce
        // N notifications. Consolidation can come later.
        let notif = NewNotification {
            user_id: author_id,
            notification_type: "post_liked".to_string(),
            data: json!({}),
            topic_id: None,
            post_id: Some(post_id),
            acting_user_id: Some(user_id),
        };
        diesel::insert_into(notifications::table)
            .values(&notif)
            .execute(conn)
            .map_err(LikeError::from)?;

        Ok(LikeOutcome::Created(like))
    })
}

/// Unlike a post. Idempotent: removing a like that doesn't exist returns
/// `NothingToRemove` rather than erroring. Counters are floored at zero
/// so any pre-existing drift can't push them negative.
pub fn unlike_post(
    conn: &mut PgConnection,
    user_id: i32,
    post_id: i32,
) -> Result<UnlikeOutcome, LikeError> {
    conn.transaction::<UnlikeOutcome, LikeError, _>(|conn| {
        // We allow unliking on hidden/deleted posts so users can clean up
        // old likes even after a post is moderated away.
        let author_id: Option<i32> = posts::table
            .filter(posts::id.eq(post_id))
            .select(posts::user_id)
            .first(conn)
            .optional()
            .map_err(LikeError::from)?;

        let Some(author_id) = author_id else {
            return Ok(UnlikeOutcome::NothingToRemove);
        };

        let deleted = diesel::delete(
            post_likes::table
                .filter(post_likes::user_id.eq(user_id))
                .filter(post_likes::post_id.eq(post_id)),
        )
        .execute(conn)
        .map_err(LikeError::from)?;

        if deleted == 0 {
            return Ok(UnlikeOutcome::NothingToRemove);
        }

        diesel::sql_query("UPDATE posts SET like_count = GREATEST(like_count - 1, 0) WHERE id = $1")
            .bind::<diesel::sql_types::Integer, _>(post_id)
            .execute(conn)
            .map_err(LikeError::from)?;

        diesel::sql_query(
            "UPDATE users SET likes_given = GREATEST(likes_given - 1, 0) WHERE id = $1",
        )
        .bind::<diesel::sql_types::Integer, _>(user_id)
        .execute(conn)
        .map_err(LikeError::from)?;

        diesel::sql_query(
            "UPDATE users SET likes_received = GREATEST(likes_received - 1, 0) WHERE id = $1",
        )
        .bind::<diesel::sql_types::Integer, _>(author_id)
        .execute(conn)
        .map_err(LikeError::from)?;

        Ok(UnlikeOutcome::Removed)
    })
}
