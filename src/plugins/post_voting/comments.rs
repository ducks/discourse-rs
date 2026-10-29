//! discourse-post-voting's comments (CommentsController, mounted at
//! /post_voting/comments): PostVotingComment's validations and cooking,
//! CommentCreator with its notification to the post's author, editing,
//! trashing and the comments past the preloaded ones. The caller holds
//! the transaction.
//!
//! The post_voting_comment_created and post_voting_comment_edited events
//! have no listener among the ported plugins.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::votes::{can_see_post, find_post};
use super::{COMMENT_SELECT, Comment, Flagging, USER_COMMENTED, post_in_post_voting_topic};
use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::posting::text::{Sentinel, sanitized_length};
use crate::{AppError, Unsupported};

/// `PostVotingComment::COOKED_VERSION`
pub const COOKED_VERSION: i32 = 1;

pub enum Outcome {
    /// The JSON to answer with.
    Done(Value),
    /// `Discourse::NotFound`
    NotFound,
    /// `Discourse::InvalidAccess`
    Forbidden,
    /// `ensure_logged_in`
    NotLoggedIn,
    /// `render_json_error(comment.errors.full_messages, status: 403)`
    Errors(Vec<String>),
    /// `Discourse::InvalidParameters`, raised without a name.
    InvalidParameters,
    /// `Discourse::InvalidParameters.new(name)`
    InvalidParameter(&'static str),
}

/// `PostVotingComment.cook`: the raw with its newlines run together into
/// spaces (`raw.gsub!` changes the record's raw too), and its cooked.
pub async fn cook(ctx: &Ctx<'_>, raw: &str) -> Result<(String, String), AppError> {
    let raw = collapse_newlines(raw);
    let cooked = crate::pretty_text::cook_post_voting_comment(ctx.host, &raw).await?;
    Ok((raw, cooked))
}

/// `raw.gsub!(/(\n)+/, " ")`
fn collapse_newlines(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_run = false;
    for c in raw.chars() {
        if c == '\n' {
            if !in_run {
                out.push(' ');
            }
            in_run = true;
        } else {
            out.push(c);
            in_run = false;
        }
    }
    out
}

/// The post a comment goes on, as the validations read it.
#[derive(sqlx::FromRow)]
struct CommentPost {
    id: i32,
    topic_id: i32,
    post_number: i32,
    user_id: Option<i32>,
    reply_to_post_number: Option<i32>,
}

async fn comment_post(
    conn: &mut PgConnection,
    post_id: i32,
) -> Result<Option<CommentPost>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, topic_id, post_number, user_id, reply_to_post_number FROM posts \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(post_id)
    .fetch_optional(conn)
    .await
}

/// `errors.full_messages` for a raw attribute message.
fn raw_message(message: &str) -> String {
    format!("Raw {message}")
}

/// PostVotingComment's validations, in declaration order: the presence
/// checks, `ensure_can_comment` on create, then PostVotingCommentValidator
/// (stripped length, TextSentinel, watched words).
async fn validate(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    post: &CommentPost,
    raw: Option<&str>,
    cooked: Option<&str>,
    create: bool,
) -> Result<Vec<String>, AppError> {
    let s = ctx.settings;
    let blank = |v: Option<&str>| v.is_none_or(|v| v.trim().is_empty());
    let mut errors = Vec::new();
    let cant_be_blank = ctx.t("errors.messages.blank");
    if blank(raw) {
        errors.push(raw_message(&cant_be_blank));
    }
    if blank(cooked) {
        errors.push(format!("Cooked {cant_be_blank}"));
    }
    if cooked.is_none() {
        errors.push(format!("Cooked version {cant_be_blank}"));
    }
    if create {
        // ensure_can_comment
        let limit = s.get("post_voting_comment_limit_per_post")?.to_i();
        if !s.get("post_voting_comment_enabled")?.truthy() {
            errors.push(ctx.t("post_voting.comment.errors.disabled"));
        } else if !post_in_post_voting_topic(conn, s, post.id).await? {
            errors.push(ctx.t("post_voting.comment.errors.post_voting_not_enabled"));
        } else if post.reply_to_post_number.is_some() {
            errors.push(ctx.t("post_voting.comment.errors.not_permitted"));
        } else if comments_count(conn, post.id).await? >= limit {
            errors.push(
                ctx.i18n
                    .t_with(
                        "post_voting.comment.errors.limit_exceeded",
                        &[("limit", &limit.to_string())],
                    )
                    .unwrap_or_default(),
            );
        }
    }
    // StrippedLengthValidator over min_post_length..comment max length.
    let raw = raw.unwrap_or("");
    let min = s.get("min_post_length")?.to_i();
    let max = s.get("post_voting_comment_max_raw_length")?.to_i();
    let length = raw.chars().count() as i64;
    if raw.trim().is_empty() && min > 0 {
        errors.push(raw_message(&cant_be_blank));
    } else if length > max {
        let form = if max == 1 { "one" } else { "other" };
        let message = ctx
            .i18n
            .t_with(
                &format!("errors.messages.too_long_validation.{form}"),
                &[("count", &max.to_string()), ("length", &length.to_string())],
            )
            .unwrap_or_default();
        errors.push(raw_message(&message));
    } else if (sanitized_length(raw, false) as i64) < min {
        errors.push(raw_message(&ctx.t_count("errors.messages.too_short", min)));
    }
    // TextSentinel.body_sentinel
    let mut entropy = s.get("body_min_entropy")?.to_i();
    if entropy > min {
        entropy = (min as f64 * 0.7) as i64;
    }
    let sentinel = Sentinel {
        text: raw,
        min_entropy: Some(entropy),
        max_word_length: None,
        allow_uppercase_posts: s.get("allow_uppercase_posts")?.truthy(),
        skip_word_length: false,
    };
    if !sentinel.valid() {
        errors.push(raw_message(&ctx.t("is_invalid")));
    }
    // WatchedWordsValidator: on base.
    if let Some(message) = crate::word_watcher::block_error(&mut *conn, s, ctx.i18n, raw).await? {
        errors.push(message);
    }
    Ok(errors)
}

/// `PostVotingComment.where(post_id:).count`: the comments not trashed.
async fn comments_count(conn: &mut PgConnection, post_id: i32) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT count(*) FROM post_voting_comments WHERE post_id = $1 AND deleted_at IS NULL",
    )
    .bind(post_id)
    .fetch_one(conn)
    .await
}

/// A comment by id with its author, trashed ones only when asked for.
pub async fn load(
    conn: &mut PgConnection,
    id: i64,
    with_deleted: bool,
) -> Result<Option<Comment>, sqlx::Error> {
    let sql = format!("{COMMENT_SELECT} WHERE c.id = $1 AND ($2 OR c.deleted_at IS NULL)");
    sqlx::query_as(&sql)
        .bind(id)
        .bind(with_deleted)
        .fetch_optional(conn)
        .await
}

/// PostVotingCommentSerializer outside a topic view: `user_voted` is
/// whether the scope's user (none for an anonymous one) voted on it.
pub async fn serialize(
    conn: &mut PgConnection,
    flagging: &Flagging,
    guardian: &Guardian,
    comment: &Comment,
) -> Result<Value, sqlx::Error> {
    let voted: bool = match guardian.user_id() {
        Some(uid) => {
            sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM post_voting_votes WHERE votable_type = 'PostVotingComment' \
                 AND votable_id = $1 AND user_id = $2)",
            )
            .bind(comment.id)
            .bind(uid)
            .fetch_one(conn)
            .await?
        }
        None => false,
    };
    Ok(flagging.comment_json(comment, Value::Bool(voted)))
}

/// `find_post` and `ensure_post_voting_enabled`, the before actions of
/// load_more_comments and create.
async fn post_for_comments(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    post_id: i64,
) -> Result<Result<CommentPost, Outcome>, AppError> {
    let Ok(post_id) = i32::try_from(post_id) else {
        return Ok(Err(Outcome::NotFound));
    };
    let Some(post) = comment_post(conn, post_id).await? else {
        return Ok(Err(Outcome::NotFound));
    };
    if !post_in_post_voting_topic(conn, ctx.settings, post.id).await? {
        return Ok(Err(Outcome::Forbidden));
    }
    Ok(Ok(post))
}

/// `@guardian.ensure_can_see!(post)`, with the post's topic.
async fn see_post(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i32,
) -> Result<Option<crate::topic_guardian::TopicCtx>, AppError> {
    let Some((post, topic_id, _)) = find_post(conn, i64::from(post_id)).await? else {
        return Ok(None);
    };
    Ok(
        match can_see_post(conn, ctx.settings, guardian, &post, topic_id).await? {
            Some((topic, true)) => Some(topic),
            _ => None,
        },
    )
}

/// CommentsController#load_more_comments (GET /post_voting/comments):
/// the post's comments after `last_comment_id`. None for a missing
/// `last_comment_id` (ParameterMissing).
pub async fn load_more(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i64,
    last_comment_id: Option<&str>,
) -> Result<Option<Outcome>, AppError> {
    let post = match post_for_comments(conn, ctx, post_id).await? {
        Ok(post) => post,
        Err(outcome) => return Ok(Some(outcome)),
    };
    if see_post(conn, ctx, guardian, post.id).await?.is_none() {
        return Ok(Some(Outcome::Forbidden));
    }
    let Some(last) = last_comment_id.filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if post.reply_to_post_number.is_some() && post.post_number != 1 {
        return Ok(Some(Outcome::InvalidParameters));
    }
    // `id > ?` with what was sent: Postgres refuses a non-integer.
    let last: i64 = last.trim().parse().map_err(|_| {
        std::io::Error::other(format!("invalid input syntax for type bigint: {last}"))
    })?;
    let sql = format!(
        "{COMMENT_SELECT} WHERE c.id > $1 AND c.post_id = $2 AND c.deleted_at IS NULL ORDER BY c.id"
    );
    let comments: Vec<Comment> = sqlx::query_as(&sql)
        .bind(last)
        .bind(post.id)
        .fetch_all(&mut *conn)
        .await?;
    let flagging = Flagging::load(conn, ctx.settings, guardian).await?;
    let mut out = Vec::with_capacity(comments.len());
    for c in &comments {
        out.push(serialize(conn, &flagging, guardian, c).await?);
    }
    Ok(Some(Outcome::Done(json!({ "comments": out }))))
}

/// CommentsController#create (POST /post_voting/comments), through
/// CommentCreator.
pub async fn create(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i64,
    raw: Option<&str>,
) -> Result<Outcome, AppError> {
    let post = match post_for_comments(conn, ctx, post_id).await? {
        Ok(post) => post,
        Err(outcome) => return Ok(outcome),
    };
    let Some(user) = guardian.user() else {
        return Ok(Outcome::NotLoggedIn);
    };
    let Some(topic) = see_post(conn, ctx, guardian, post.id).await? else {
        return Ok(Outcome::Forbidden);
    };
    let can_post_anywhere = guardian
        .can_create_post_anywhere(&mut *conn, ctx.settings)
        .await?;
    if !guardian.can_create_post_on_topic(ctx.settings, &topic, can_post_anywhere)? {
        return Ok(Outcome::Forbidden);
    }
    // before_validation :cook_raw, if the raw is set.
    let (raw, cooked) = match raw {
        Some(raw) => {
            let (raw, cooked) = cook(ctx, raw).await?;
            (Some(raw), Some(cooked))
        }
        None => (None, None),
    };
    let errors = validate(conn, ctx, &post, raw.as_deref(), cooked.as_deref(), true).await?;
    if !errors.is_empty() {
        return Ok(Outcome::Errors(errors));
    }
    let now = crate::clock::now_naive();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO post_voting_comments (post_id, user_id, raw, cooked, cooked_version, \
           created_at, updated_at, last_editor_id) VALUES ($1, $2, $3, $4, $5, $6, $6, $2) RETURNING id",
    )
    .bind(post.id)
    .bind(user.id)
    .bind(&raw)
    .bind(&cooked)
    .bind(COOKED_VERSION)
    .bind(now)
    .fetch_one(&mut *conn)
    .await?;
    notify_commented(conn, ctx, &post, id, user.id, &user.username).await?;
    let comment = load(conn, id, false)
        .await?
        .ok_or(Unsupported("a comment gone as it was created"))?;
    publish_created(conn, ctx, &post, &comment).await?;
    let flagging = Flagging::load(conn, ctx.settings, guardian).await?;
    Ok(Outcome::Done(
        serialize(conn, &flagging, guardian, &comment).await?,
    ))
}

/// `CommentCreator.create_commented_notification`: the post's author is
/// told, unless they wrote the comment.
async fn notify_commented(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    post: &CommentPost,
    comment_id: i64,
    commenter_id: i32,
    commenter_username: &str,
) -> Result<(), AppError> {
    if post.user_id == Some(commenter_id) {
        return Ok(());
    }
    let Some(author) = post.user_id else {
        return Err(Unsupported("a comment on a post whose author is gone").into());
    };
    let mut data = Map::new();
    data.insert("post_voting_comment_id".into(), json!(comment_id));
    data.insert("display_username".into(), json!(commenter_username));
    crate::notification_create::create(
        conn,
        ctx.bus,
        ctx.settings,
        &crate::notification_create::NewNotification::new(
            USER_COMMENTED,
            author,
            Value::Object(data),
        )
        .topic(post.topic_id, Some(post.post_number)),
    )
    .await?;
    notification_alert(conn, ctx, post, author, commenter_username).await
}

/// `PostAlerter.create_notification_alert` for the comment: the live
/// alert (to a user seen in the last 30 days) and push notifications,
/// none for a suspended user.
async fn notification_alert(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    post: &CommentPost,
    user_id: i32,
    username: &str,
) -> Result<(), AppError> {
    let suspended: bool =
        sqlx::query_scalar("SELECT COALESCE(suspended_till > $2, FALSE) FROM users WHERE id = $1")
            .bind(user_id)
            .bind(crate::clock::now_naive())
            .fetch_one(&mut *conn)
            .await?;
    if suspended {
        return Ok(());
    }
    let (cooked, title, slug): (String, String, Option<String>) = sqlx::query_as(
        "SELECT p.cooked, t.title, t.slug FROM posts p JOIN topics t ON t.id = p.topic_id WHERE p.id = $1",
    )
    .bind(post.id)
    .fetch_one(&mut *conn)
    .await?;
    // Post#url: no base path.
    let post_url = format!(
        "/t/{}/{}/{}",
        slug.unwrap_or_default(),
        post.topic_id,
        post.post_number
    );
    let excerpt = crate::excerpt::excerpt(
        &cooked,
        400,
        &crate::excerpt::Options {
            text_entities: true,
            strip_links: true,
            remap_emoji: true,
            denied_emojis: ctx.settings.emojis().denied.clone(),
            plain_hashtags: true,
            post_url: Some(post_url.clone()),
            ..Default::default()
        },
        crate::excerpt::Plugins::from_settings(ctx.settings)?,
    );
    let payload = json!({
        "notification_type": USER_COMMENTED,
        "post_number": post.post_number,
        "topic_title": title,
        "topic_id": post.topic_id,
        "post_id": post.id,
        "excerpt": excerpt,
        "username": username,
        "post_url": post_url,
    });
    crate::bus::publish_notification_alert(ctx.bus, &mut *conn, user_id, &payload).await?;
    crate::jobs::post_alert::push_notification(conn, ctx.settings, user_id).await?;
    Ok(())
}

/// `CommentCreator.publish_changes`: the comment as an anonymous viewer
/// sees it, and the post's comment count.
async fn publish_created(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    post: &CommentPost,
    comment: &Comment,
) -> Result<(), AppError> {
    let anonymous = Guardian::anonymous();
    let flagging = Flagging::load(conn, ctx.settings, &anonymous).await?;
    let serialized = serialize(conn, &flagging, &anonymous, comment).await?;
    let mut opts = Map::new();
    opts.insert("comment".into(), serialized);
    opts.insert(
        "comments_count".into(),
        json!(comments_count(conn, post.id).await?),
    );
    crate::bus::publish_post_change(
        ctx,
        conn,
        post.id,
        "post_voting_post_commented",
        opts,
        false,
    )
    .await
}

/// `can_edit_comment?` (and `can_delete_comment?`): its author or staff.
fn can_edit(guardian: &Guardian, comment: &Comment) -> bool {
    match guardian.user_id() {
        Some(uid) => comment.user_id == uid || guardian.is_admin() || guardian.is_moderator(),
        None => false,
    }
}

/// CommentsController#update (PUT /post_voting/comments).
pub async fn update(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    comment_id: i64,
    raw: &str,
) -> Result<Outcome, AppError> {
    let Some(comment) = load(conn, comment_id, false).await? else {
        return Ok(Outcome::NotFound);
    };
    if see_post(conn, ctx, guardian, comment.post_id)
        .await?
        .is_none()
    {
        return Ok(Outcome::Forbidden);
    }
    if !can_edit(guardian, &comment) {
        return Ok(Outcome::Forbidden);
    }
    // The raw cooks again only when it changes.
    let (raw, cooked) = if raw == comment.raw {
        (comment.raw.clone(), comment.cooked.clone())
    } else {
        cook(ctx, raw).await?
    };
    let Some(post) = comment_post(conn, comment.post_id).await? else {
        return Err(Unsupported("editing a comment of a deleted post").into());
    };
    let errors = validate(conn, ctx, &post, Some(&raw), Some(&cooked), false).await?;
    if !errors.is_empty() {
        return Ok(Outcome::Errors(errors));
    }
    if raw != comment.raw || cooked != comment.cooked {
        sqlx::query(
            "UPDATE post_voting_comments SET raw = $2, cooked = $3, cooked_version = $4, updated_at = $5 \
             WHERE id = $1",
        )
        .bind(comment.id)
        .bind(&raw)
        .bind(&cooked)
        .bind(COOKED_VERSION)
        .bind(crate::clock::now_naive())
        .execute(&mut *conn)
        .await?;
    }
    let comment = load(conn, comment.id, false)
        .await?
        .ok_or(Unsupported("a comment gone as it was edited"))?;
    let mut opts = Map::new();
    opts.insert("comment_id".into(), json!(comment.id));
    opts.insert("comment_raw".into(), json!(comment.raw));
    opts.insert("comment_cooked".into(), json!(comment.cooked));
    crate::bus::publish_post_change(
        ctx,
        conn,
        comment.post_id,
        "post_voting_post_comment_edited",
        opts,
        false,
    )
    .await?;
    let flagging = Flagging::load(conn, ctx.settings, guardian).await?;
    Ok(Outcome::Done(
        serialize(conn, &flagging, guardian, &comment).await?,
    ))
}

/// CommentsController#destroy (DELETE /post_voting/comments): the
/// comment trashed (`trash!` without a user).
pub async fn destroy(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    comment_id: i64,
) -> Result<Outcome, AppError> {
    let Some(comment) = load(conn, comment_id, false).await? else {
        return Ok(Outcome::NotFound);
    };
    if see_post(conn, ctx, guardian, comment.post_id)
        .await?
        .is_none()
    {
        return Ok(Outcome::Forbidden);
    }
    if !can_edit(guardian, &comment) {
        return Ok(Outcome::Forbidden);
    }
    trash(conn, comment.id, None).await?;
    let mut opts = Map::new();
    opts.insert("comment_id".into(), json!(comment.id));
    opts.insert(
        "comments_count".into(),
        json!(comments_count(conn, comment.post_id).await?),
    );
    crate::bus::publish_post_change(
        ctx,
        conn,
        comment.post_id,
        "post_voting_post_comment_trashed",
        opts,
        false,
    )
    .await?;
    Ok(Outcome::Done(json!({ "success": "OK" })))
}

/// What a flag on a comment asks for (`params.permit(...)`).
pub struct FlagParams {
    pub comment_id: i64,
    pub flag_type_id: i64,
    pub is_warning: bool,
    pub take_action: bool,
    pub queue_for_review: bool,
}

/// CommentsController#flag (PUT /post_voting/comments/flag) up to
/// CommentReviewQueue's reviewable: the comment, the flag type and the
/// guardian's checks. The reviewable itself is refused: the review queue
/// serializes no plugin reviewable yet.
pub async fn flag(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    params: &FlagParams,
) -> Result<Outcome, AppError> {
    // PostVotingComment.find: RecordNotFound.
    let Some(comment) = load(conn, params.comment_id, false).await? else {
        return Ok(Outcome::NotFound);
    };
    // ReviewableScore.types: the flags but like, and the score types.
    let valid: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM flags WHERE id = $1 AND id <> 2)")
            .bind(params.flag_type_id)
            .fetch_one(&mut *conn)
            .await?;
    if !valid {
        return Ok(Outcome::InvalidParameter("flag_type_id"));
    }
    // ensure_can_flag_post_voting_comment!
    let flagging = Flagging::load(conn, ctx.settings, guardian).await?;
    let can_see = see_post(conn, ctx, guardian, comment.post_id)
        .await?
        .is_some();
    if !can_see || !flagging.can_flag(&comment) {
        return Ok(Outcome::Forbidden);
    }
    // ensure_can_flag_post_voting_comment_as!
    let staff = guardian.is_staff();
    if !staff && (params.take_action || params.queue_for_review) {
        return Ok(Outcome::Forbidden);
    }
    let notify_user: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM flags WHERE id = $1 AND name_key = 'notify_user')",
    )
    .bind(params.flag_type_id)
    .fetch_one(&mut *conn)
    .await?;
    if notify_user && params.is_warning && !staff {
        return Ok(Outcome::Forbidden);
    }
    Err(
        Unsupported("flagging post voting comments (plugin reviewables in the review queue)")
            .into(),
    )
}

/// `Trashable#trash!(by)`: deleted_at and deleted_by_id, no updated_at.
pub async fn trash(
    conn: &mut PgConnection,
    comment_id: i64,
    by: Option<i32>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE post_voting_comments SET deleted_at = $2, deleted_by_id = $3 WHERE id = $1",
    )
    .bind(comment_id)
    .bind(crate::clock::now_naive())
    .bind(by)
    .execute(conn)
    .await?;
    Ok(())
}

/// `Trashable#recover!`
pub async fn recover(conn: &mut PgConnection, comment_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE post_voting_comments SET deleted_at = NULL, deleted_by_id = NULL WHERE id = $1",
    )
    .bind(comment_id)
    .execute(conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newlines_run_together_into_one_space() {
        assert_eq!(collapse_newlines("line 1\n\nline 2\n"), "line 1 line 2 ");
        assert_eq!(collapse_newlines("a\r\nb"), "a\r b");
        assert_eq!(collapse_newlines("plain"), "plain");
    }
}
