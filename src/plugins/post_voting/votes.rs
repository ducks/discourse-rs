//! discourse-post-voting's VoteManager and VotesController (mounted at
//! /post_voting): up and down votes on a post, upvotes on a comment,
//! undoing them within post_voting_undo_vote_action_window, and the
//! voters of a post. The caller holds the transaction.

use chrono::{Duration, NaiveDateTime};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::post_in_post_voting_topic;
use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::topic_guardian::{PostCtx, TopicCtx};
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// `PostVotingVote.directions`
pub const UP: &str = "up";
pub const DOWN: &str = "down";

/// `VotesController::VOTERS_LIMIT`
pub const VOTERS_LIMIT: i64 = 20;

/// What a votable is (`votable_type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Votable {
    Post(i32),
    Comment(i64),
}

impl Votable {
    fn kind(self) -> &'static str {
        match self {
            Votable::Post(_) => "Post",
            Votable::Comment(_) => "PostVotingComment",
        }
    }

    fn id(self) -> i64 {
        match self {
            Votable::Post(id) => i64::from(id),
            Votable::Comment(id) => id,
        }
    }

    fn table(self) -> &'static str {
        match self {
            Votable::Post(_) => "posts",
            Votable::Comment(_) => "post_voting_comments",
        }
    }
}

pub enum Outcome {
    /// `success_json`
    Done,
    /// `Discourse::NotFound`
    NotFound,
    /// `Discourse::InvalidAccess`, with its custom message translated.
    Forbidden(Option<String>),
    /// `render_json_error(message, status: 403)`
    Refused(String),
    /// `ActiveRecord::RecordInvalid` out of `create!`: 422.
    Invalid(Vec<String>),
}

/// A message the votes controller raises (`raise_error`), translated.
fn t(i18n: &crate::i18n::I18n, key: &str) -> String {
    i18n.t(key).unwrap_or(key).to_string()
}

/// `vote.error.undo_vote_action_window` for the setting.
fn undo_window_message(
    i18n: &crate::i18n::I18n,
    settings: &SiteSettings,
) -> Result<String, AppError> {
    let count = settings.get("post_voting_undo_vote_action_window")?.to_i();
    Ok(i18n
        .t_count("vote.error.undo_vote_action_window", count, &[])
        .unwrap_or_default())
}

/// `PostVotingVote.reverse_direction`, which raises on anything else.
fn reverse_direction(direction: &str) -> Result<&'static str, AppError> {
    match direction {
        UP => Ok(DOWN),
        DOWN => Ok(UP),
        other => Err(std::io::Error::other(format!("Invalid direction: {other}")).into()),
    }
}

/// `post_voting_last_voted(user_id)`: the user's latest vote on it.
async fn last_voted(
    conn: &mut PgConnection,
    votable: Votable,
    user_id: i32,
) -> Result<Option<NaiveDateTime>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT created_at FROM post_voting_votes WHERE votable_type = $1 AND votable_id = $2 \
         AND user_id = $3 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(votable.kind())
    .bind(votable.id())
    .bind(user_id)
    .fetch_optional(conn)
    .await
}

/// `VoteManager.can_undo`: no vote yet, no window, or the last vote in it.
pub async fn can_undo(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    votable: Votable,
    user_id: i32,
) -> Result<bool, AppError> {
    let Some(last) = last_voted(conn, votable, user_id).await? else {
        return Ok(true);
    };
    let window = settings.get("post_voting_undo_vote_action_window")?.to_i();
    let since = crate::clock::now_naive() - Duration::minutes(window);
    Ok(window == 0 || last.and_utc().timestamp() > since.and_utc().timestamp())
}

/// The user voted on it, in that direction when given.
async fn voted(
    conn: &mut PgConnection,
    votable: Votable,
    user_id: i32,
    direction: Option<&str>,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM post_voting_votes WHERE votable_type = $1 AND votable_id = $2 \
         AND user_id = $3 AND ($4::text IS NULL OR direction = $4))",
    )
    .bind(votable.kind())
    .bind(votable.id())
    .bind(user_id)
    .bind(direction)
    .fetch_one(conn)
    .await
}

/// `Post.find_by(id:)`: a post not deleted, with its topic.
pub async fn find_post(
    conn: &mut PgConnection,
    post_id: i64,
) -> Result<Option<(PostCtx, i32, Option<i32>)>, sqlx::Error> {
    let Ok(post_id) = i32::try_from(post_id) else {
        return Ok(None);
    };
    let row: Option<(i32, Option<i32>)> = sqlx::query_as(
        "SELECT topic_id, reply_to_post_number FROM posts WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((topic_id, reply_to)) = row else {
        return Ok(None);
    };
    let post = PostCtx::load(conn, post_id).await?;
    Ok(post.map(|p| (p, topic_id, reply_to)))
}

/// `guardian.can_see?(post)`, with the post's topic (None for a topic
/// that's gone).
pub async fn can_see_post(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    post: &PostCtx,
    topic_id: i32,
) -> Result<Option<(TopicCtx, bool)>, AppError> {
    let Some(topic) = TopicCtx::load(&mut *conn, settings, guardian, topic_id).await? else {
        return Ok(None);
    };
    let secure = guardian.secure_category_ids(&mut *conn, settings).await?;
    let can_see_topic = guardian.can_see_topic(settings, &topic, true, &secure)?;
    let can_see = guardian.can_see_post(settings, post, can_see_topic)?;
    Ok(Some((topic, can_see)))
}

/// `VoteManager.vote`: the user's vote, replacing one the other way, and
/// the votable's count; a post's change published after commit.
pub async fn vote(
    conn: &mut PgConnection,
    ctx: &crate::posting::Ctx<'_>,
    votable: Votable,
    user_id: i32,
    direction: &str,
) -> Result<Result<(), Vec<String>>, AppError> {
    let reverse = reverse_direction(direction)?;
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM post_voting_votes WHERE user_id = $1 AND votable_type = $2 \
         AND votable_id = $3 AND direction = $4 LIMIT 1",
    )
    .bind(user_id)
    .bind(votable.kind())
    .bind(votable.id())
    .bind(reverse)
    .fetch_optional(&mut *conn)
    .await?;
    let change = match (existing.is_some(), direction == UP) {
        (true, true) => 2,
        (true, false) => -2,
        (false, true) => 1,
        (false, false) => -1,
    };
    if let Some(id) = existing {
        sqlx::query("DELETE FROM post_voting_votes WHERE id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    let errors = validate(conn, ctx, votable, user_id, direction).await?;
    if !errors.is_empty() {
        return Ok(Err(errors));
    }
    sqlx::query(
        "INSERT INTO post_voting_votes (user_id, created_at, direction, votable_type, votable_id) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(user_id)
    .bind(crate::clock::now_naive())
    .bind(direction)
    .bind(votable.kind())
    .bind(votable.id())
    .execute(&mut *conn)
    .await?;
    let count = update_count(conn, votable, change).await?;
    publish(conn, ctx, votable, user_id, count, Some(direction)).await?;
    Ok(Ok(()))
}

/// `VoteManager.remove_vote`: the user's vote on it gone, and the count;
/// None when there was none (where Rails raises on a nil vote).
pub async fn remove_vote(
    conn: &mut PgConnection,
    ctx: &crate::posting::Ctx<'_>,
    votable: Votable,
    user_id: i32,
) -> Result<(), AppError> {
    let vote: Option<(i64, String)> = sqlx::query_as(
        "SELECT id, direction FROM post_voting_votes WHERE votable_type = $1 AND votable_id = $2 \
         AND user_id = $3 LIMIT 1",
    )
    .bind(votable.kind())
    .bind(votable.id())
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((id, direction)) = vote else {
        return Err(std::io::Error::other("undefined method 'direction' for nil").into());
    };
    sqlx::query("DELETE FROM post_voting_votes WHERE id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    let change = if direction == UP { -1 } else { 1 };
    let count = update_count(conn, votable, change).await?;
    publish(conn, ctx, votable, user_id, count, None).await?;
    Ok(())
}

/// `obj.update!(qa_vote_count: (obj.qa_vote_count || 0) + change)`
async fn update_count(
    conn: &mut PgConnection,
    votable: Votable,
    change: i32,
) -> Result<i32, sqlx::Error> {
    let sql = format!(
        "UPDATE {} SET qa_vote_count = COALESCE(qa_vote_count, 0) + $2, updated_at = $3 \
         WHERE id = $1 RETURNING qa_vote_count",
        votable.table()
    );
    let count: i32 = match votable {
        Votable::Post(id) => {
            sqlx::query_scalar(&sql)
                .bind(id)
                .bind(change)
                .bind(crate::clock::now_naive())
                .fetch_one(&mut *conn)
                .await?
        }
        Votable::Comment(id) => {
            sqlx::query_scalar(&sql)
                .bind(id)
                .bind(change)
                .bind(crate::clock::now_naive())
                .fetch_one(&mut *conn)
                .await?
        }
    };
    Ok(count)
}

/// PostVotingVote's validations, as `create!` raises them.
async fn validate(
    conn: &mut PgConnection,
    ctx: &crate::posting::Ctx<'_>,
    votable: Votable,
    user_id: i32,
    direction: &str,
) -> Result<Vec<String>, AppError> {
    let mut errors = Vec::new();
    let (owner, post_id, reply_to): (Option<i32>, i32, Option<i32>) = match votable {
        Votable::Post(id) => {
            sqlx::query_as("SELECT user_id, id, reply_to_post_number FROM posts WHERE id = $1")
                .bind(id)
                .fetch_one(&mut *conn)
                .await?
        }
        Votable::Comment(id) => sqlx::query_as(
            "SELECT c.user_id, c.post_id, NULL::int FROM post_voting_comments c WHERE c.id = $1",
        )
        .bind(id)
        .fetch_one(&mut *conn)
        .await?,
    };
    // ensure_valid_vote
    if owner == Some(user_id) {
        errors.push(ctx.t("post.post_voting.errors.self_voting_not_permitted"));
    }
    let post_voting = post_in_post_voting_topic(conn, ctx.settings, post_id).await?;
    match votable {
        Votable::Post(_) => {
            if !post_voting {
                errors.push(ctx.t("post.post_voting.errors.post_voting_not_enabled"));
            } else if reply_to.is_some() {
                errors.push(ctx.t("post.post_voting.errors.voting_not_permitted"));
            }
        }
        Votable::Comment(_) => {
            if direction != UP {
                errors.push(ctx.t("post.post_voting.errors.comment_cannot_be_downvoted"));
            }
            if !post_voting {
                errors.push(ctx.t("post.post_voting.errors.post_voting_not_enabled"));
            }
        }
    }
    Ok(errors)
}

/// `VoteManager.publish_changes`: a post's `post_voting_post_voted`.
async fn publish(
    conn: &mut PgConnection,
    ctx: &crate::posting::Ctx<'_>,
    votable: Votable,
    user_id: i32,
    count: i32,
    direction: Option<&str>,
) -> Result<(), AppError> {
    let Votable::Post(post_id) = votable else {
        return Ok(());
    };
    let has_votes: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM post_voting_votes WHERE votable_type = 'Post' AND votable_id = $1)",
    )
    .bind(i64::from(post_id))
    .fetch_one(&mut *conn)
    .await?;
    let mut opts = Map::new();
    opts.insert("post_voting_user_voted_id".into(), json!(user_id));
    opts.insert("post_voting_vote_count".into(), json!(count));
    opts.insert("post_voting_user_voted_direction".into(), json!(direction));
    opts.insert("post_voting_has_votes".into(), json!(has_votes));
    crate::bus::publish_post_change(ctx, conn, post_id, "post_voting_post_voted", opts, false).await
}

/// `VoteManager.bulk_remove_votes_by(user)`: every vote of the user gone,
/// the counts of what they voted on put back.
pub async fn bulk_remove_votes_by(
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<(), sqlx::Error> {
    for (table, kind) in [
        ("posts", "Post"),
        ("post_voting_comments", "PostVotingComment"),
    ] {
        sqlx::query(&format!(
            "UPDATE {table} SET qa_vote_count = qa_vote_count - ( \
               SELECT CASE WHEN direction = 'up' THEN 1 WHEN direction = 'down' THEN -1 ELSE 0 END \
               FROM post_voting_votes WHERE post_voting_votes.votable_id = {table}.id \
               AND post_voting_votes.votable_type = '{kind}' AND post_voting_votes.user_id = $1) \
             WHERE EXISTS (SELECT 1 FROM post_voting_votes WHERE post_voting_votes.votable_id = {table}.id \
               AND post_voting_votes.votable_type = '{kind}' AND post_voting_votes.user_id = $1)"
        ))
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    }
    sqlx::query("DELETE FROM post_voting_votes WHERE user_id = $1")
        .bind(user_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// A post the votes controller acts on: found, seen, and (for voting) in
/// a post voting topic.
async fn vote_post(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    post_id: i64,
    post_voting_only: bool,
) -> Result<Result<(PostCtx, TopicCtx, Option<i32>), Outcome>, AppError> {
    let Some((post, topic_id, reply_to)) = find_post(conn, post_id).await? else {
        return Ok(Err(Outcome::NotFound));
    };
    let Some((topic, can_see)) = can_see_post(conn, settings, guardian, &post, topic_id).await?
    else {
        return Ok(Err(Outcome::Forbidden(None)));
    };
    if !can_see {
        return Ok(Err(Outcome::Forbidden(None)));
    }
    if post_voting_only && !post_in_post_voting_topic(conn, settings, post.id).await? {
        return Ok(Err(Outcome::Forbidden(None)));
    }
    Ok(Ok((post, topic, reply_to)))
}

/// VotesController#create (POST /post_voting/vote): `direction` up
/// unless given.
pub async fn create(
    conn: &mut PgConnection,
    ctx: &crate::posting::Ctx<'_>,
    guardian: &Guardian,
    post_id: i64,
    direction: Option<&str>,
) -> Result<Outcome, AppError> {
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("voting anonymously"))?;
    let (post, topic, _) = match vote_post(conn, ctx.settings, guardian, post_id, true).await? {
        Ok(found) => found,
        Err(outcome) => return Ok(outcome),
    };
    // ensure_can_vote
    if post.user_id == Some(user_id) {
        return Ok(Outcome::Forbidden(Some(t(
            ctx.i18n,
            "post.post_voting.errors.self_voting_not_permitted",
        ))));
    }
    if topic.archived {
        return Ok(Outcome::Forbidden(Some(t(
            ctx.i18n,
            "post.post_voting.errors.vote_archived_topic",
        ))));
    }
    if topic.closed {
        return Ok(Outcome::Forbidden(Some(t(
            ctx.i18n,
            "post.post_voting.errors.vote_closed_topic",
        ))));
    }
    let direction = direction.unwrap_or(UP);
    let votable = Votable::Post(post.id);
    if voted(conn, votable, user_id, Some(direction)).await? {
        return Ok(Outcome::Forbidden(Some(t(
            ctx.i18n,
            "vote.error.one_vote_per_post",
        ))));
    }
    if !can_undo(conn, ctx.settings, votable, user_id).await? {
        return Ok(Outcome::Forbidden(Some(undo_window_message(
            ctx.i18n,
            ctx.settings,
        )?)));
    }
    Ok(match vote(conn, ctx, votable, user_id, direction).await? {
        Ok(()) => Outcome::Done,
        Err(errors) => Outcome::Invalid(errors),
    })
}

/// VotesController#destroy (DELETE /post_voting/vote).
pub async fn destroy(
    conn: &mut PgConnection,
    ctx: &crate::posting::Ctx<'_>,
    guardian: &Guardian,
    post_id: i64,
) -> Result<Outcome, AppError> {
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("voting anonymously"))?;
    let (post, topic, _) = match vote_post(conn, ctx.settings, guardian, post_id, true).await? {
        Ok(found) => found,
        Err(outcome) => return Ok(outcome),
    };
    // Topic.post_voting_votes(topic, user).exists?: a vote on any post of
    // the topic will do.
    let voted_in_topic: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM post_voting_votes v JOIN posts ON posts.id = v.votable_id \
         WHERE v.user_id = $1 AND v.votable_type = 'Post' AND posts.topic_id = $2)",
    )
    .bind(user_id)
    .bind(topic.id)
    .fetch_one(&mut *conn)
    .await?;
    if !voted_in_topic {
        return Ok(Outcome::Forbidden(Some(t(
            ctx.i18n,
            "vote.error.user_has_not_voted",
        ))));
    }
    let votable = Votable::Post(post.id);
    if !can_undo(conn, ctx.settings, votable, user_id).await? {
        return Ok(Outcome::Refused(undo_window_message(
            ctx.i18n,
            ctx.settings,
        )?));
    }
    remove_vote(conn, ctx, votable, user_id).await?;
    Ok(Outcome::Done)
}

/// `PostVotingComment.find_by(id:)` (not deleted): its post and author.
async fn find_comment(
    conn: &mut PgConnection,
    comment_id: i64,
) -> Result<Option<(i32, i32)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT post_id, user_id FROM post_voting_comments WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(comment_id)
    .fetch_optional(conn)
    .await
}

/// `ensure_can_see!(comment.post)`: the comment's post, deleted posts
/// not found.
async fn can_see_comment_post(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    post_id: i32,
) -> Result<bool, AppError> {
    let Some((post, topic_id, _)) = find_post(conn, i64::from(post_id)).await? else {
        // comment.post is nil: can_see?(nil) is false.
        return Ok(false);
    };
    Ok(can_see_post(conn, settings, guardian, &post, topic_id)
        .await?
        .is_some_and(|(_, can_see)| can_see))
}

/// VotesController#create_comment_vote (POST /post_voting/vote/comment).
pub async fn create_comment_vote(
    conn: &mut PgConnection,
    ctx: &crate::posting::Ctx<'_>,
    guardian: &Guardian,
    comment_id: i64,
) -> Result<Outcome, AppError> {
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("voting anonymously"))?;
    let Some((post_id, author)) = find_comment(conn, comment_id).await? else {
        return Ok(Outcome::NotFound);
    };
    if !can_see_comment_post(conn, ctx.settings, guardian, post_id).await? {
        return Ok(Outcome::Forbidden(None));
    }
    if author == user_id {
        return Ok(Outcome::Forbidden(Some(t(
            ctx.i18n,
            "post.post_voting.errors.self_voting_not_permitted",
        ))));
    }
    // ensure_can_vote checks nothing else for a comment: a second upvote
    // fails the votes' unique index, a server error.
    Ok(
        match vote(conn, ctx, Votable::Comment(comment_id), user_id, UP).await? {
            Ok(()) => Outcome::Done,
            Err(errors) => Outcome::Invalid(errors),
        },
    )
}

/// VotesController#destroy_comment_vote (DELETE /post_voting/vote/comment).
pub async fn destroy_comment_vote(
    conn: &mut PgConnection,
    ctx: &crate::posting::Ctx<'_>,
    guardian: &Guardian,
    comment_id: i64,
) -> Result<Outcome, AppError> {
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("voting anonymously"))?;
    let Some((post_id, _)) = find_comment(conn, comment_id).await? else {
        return Ok(Outcome::NotFound);
    };
    if !can_see_comment_post(conn, ctx.settings, guardian, post_id).await? {
        return Ok(Outcome::Forbidden(None));
    }
    let votable = Votable::Comment(comment_id);
    if !voted(conn, votable, user_id, None).await? {
        return Ok(Outcome::Forbidden(Some(t(
            ctx.i18n,
            "vote.error.user_has_not_voted",
        ))));
    }
    remove_vote(conn, ctx, votable, user_id).await?;
    Ok(Outcome::Done)
}

/// VotesController#voters (GET /post_voting/voters): the post's latest
/// voters as BasicVoters, and how many there are.
pub async fn voters(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    urls: &Urls<'_>,
    guardian: &Guardian,
    post_id: i64,
) -> Result<Result<Value, Outcome>, AppError> {
    let (post, _, _) = match vote_post(conn, settings, guardian, post_id, false).await? {
        Ok(found) => found,
        Err(outcome) => return Ok(Err(outcome)),
    };
    // id, username, name, uploaded_avatar_id, direction
    type VoterRow = (i32, String, Option<String>, Option<i32>, String);
    let rows: Vec<VoterRow> = sqlx::query_as(
        "SELECT u.id, u.username, u.name, u.uploaded_avatar_id, v.direction FROM users u \
         JOIN post_voting_votes v ON v.user_id = u.id \
         WHERE v.votable_id = $1 AND v.votable_type = 'Post' \
         ORDER BY v.created_at DESC LIMIT $2",
    )
    .bind(i64::from(post.id))
    .bind(VOTERS_LIMIT)
    .fetch_all(&mut *conn)
    .await?;
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM post_voting_votes WHERE votable_id = $1 AND votable_type = 'Post'",
    )
    .bind(i64::from(post.id))
    .fetch_one(&mut *conn)
    .await?;
    let names = settings.get("enable_names")?.truthy();
    let mut voters = Vec::with_capacity(rows.len());
    for (id, username, name, avatar, direction) in rows {
        let mut u = Map::new();
        u.insert("id".into(), json!(id));
        u.insert("username".into(), json!(username));
        if names {
            u.insert("name".into(), json!(name));
        }
        u.insert(
            "avatar_template".into(),
            json!(crate::avatar::avatar_template(
                urls, id, &username, avatar, None
            )?),
        );
        u.insert("direction".into(), json!(direction));
        voters.push(Value::Object(u));
    }
    Ok(Ok(json!({ "voters": voters, "total_voters_count": total })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_up_and_down_reverse() {
        assert_eq!(reverse_direction("up").unwrap(), "down");
        assert_eq!(reverse_direction("down").unwrap(), "up");
        assert!(reverse_direction("sideways").is_err());
    }
}
