//! BookmarkManager for posts and topics: creating a bookmark (the
//! bookmarkable's visibility, Bookmark's validations, the topic user's
//! bookmarked flag), updating, pinning and destroying one.
//!
//! Refused: bookmarkables other than posts and topics (chat messages),
//! reminder times in formats other than ISO 8601. The bookmark rate limit
//! (RateLimiter, Redis) is not ported.

use chrono::NaiveDateTime;
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::posting::{Ctx, TopicUserAttr, change_topic_user};
use crate::{AppError, Unsupported};

/// `Bookmark.auto_delete_preferences[:clear_reminder]`
const CLEAR_REMINDER: i32 = 3;

/// How a bookmark request ends when it isn't a server error.
pub enum Outcome {
    /// Created: the new bookmark's id.
    Created(i64),
    /// Updated or pinned.
    Done,
    /// Destroyed: whether the user still has a bookmark in the topic.
    Destroyed { topic_bookmarked: bool },
    /// Validation errors: a 400 with `failed_json` and the messages.
    Invalid(Vec<String>),
    /// `Discourse::NotFound`
    NotFound,
    /// `Discourse::InvalidAccess`
    Forbidden,
}

/// What a create or update carries.
pub struct Fields<'a> {
    pub name: Option<&'a str>,
    pub reminder_at: Option<&'a str>,
    pub auto_delete_preference: Option<&'a str>,
}

/// `bookmark_model_options_with_defaults`: the preference given, else the
/// user's, else clearing the reminder.
async fn auto_delete_preference(
    conn: &mut PgConnection,
    user_id: i32,
    given: Option<&str>,
) -> Result<i32, sqlx::Error> {
    if let Some(v) = given.map(str::trim).filter(|v| !v.is_empty()) {
        return Ok(crate::ruby::to_i(v) as i32);
    }
    let preference: Option<i32> = sqlx::query_scalar(
        "SELECT bookmark_auto_delete_preference FROM user_options WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(conn)
    .await?;
    Ok(preference.filter(|p| *p != 0).unwrap_or(CLEAR_REMINDER))
}

/// `Bookmark.for_user_in_topic(user_id, topic_id).exists?`
async fn bookmarked_in_topic(
    conn: &mut PgConnection,
    user_id: i32,
    topic_id: i32,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM bookmarks b \
           LEFT JOIN posts p ON p.id = b.bookmarkable_id AND b.bookmarkable_type = 'Post' \
           LEFT JOIN topics t ON (t.id = b.bookmarkable_id AND b.bookmarkable_type = 'Topic') \
                              OR (t.id = p.topic_id) \
           WHERE b.user_id = $1 AND (t.id = $2 OR p.topic_id = $2) \
             AND p.deleted_at IS NULL AND t.deleted_at IS NULL)",
    )
    .bind(user_id)
    .bind(topic_id)
    .fetch_one(conn)
    .await
}

/// `sync_topic_user_bookmarked`
async fn sync_topic_user(
    conn: &mut PgConnection,
    user_id: i32,
    topic_id: i32,
) -> Result<bool, AppError> {
    let bookmarked = bookmarked_in_topic(&mut *conn, user_id, topic_id).await?;
    change_topic_user(
        &mut *conn,
        user_id,
        topic_id,
        &[TopicUserAttr::Bookmarked(bookmarked)],
    )
    .await?;
    Ok(bookmarked)
}

/// `ensure_sane_reminder_at_time`
fn reminder_errors(ctx: &Ctx<'_>, reminder_at: Option<NaiveDateTime>, errors: &mut Vec<String>) {
    if let Some(at) = reminder_at {
        let now = crate::clock::now_naive();
        if at < now {
            errors.push(ctx.t("bookmarks.errors.cannot_set_past_reminder"));
        }
        if at > now + chrono::Duration::days(3653) {
            errors.push(ctx.t("bookmarks.errors.cannot_set_reminder_in_distant_future"));
        }
    }
}

/// `validates :name, length: { maximum: 100 }`
fn name_errors(name: Option<&str>, errors: &mut Vec<String>) {
    if name.is_some_and(|n| n.chars().count() > 100) {
        errors.push("Name is too long (maximum is 100 characters)".into());
    }
}

/// `BookmarkManager#create_for`
pub async fn create(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    bookmarkable_id: &str,
    bookmarkable_type: &str,
    fields: &Fields<'_>,
) -> Result<Outcome, AppError> {
    let s = ctx.settings;
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("bookmarking anonymously"))?;
    let id = crate::ruby::to_i(bookmarkable_id) as i32;
    // registered_bookmarkable_from_type, then validate_before_create.
    let topic_id = match bookmarkable_type {
        "Post" => {
            match crate::posting::revisions::find_post(&mut *conn, ctx, guardian, id).await? {
                Some(access) if access.can_see_topic => access.topic_id,
                _ => return Ok(Outcome::Forbidden),
            }
        }
        "Topic" => {
            let topic = crate::topic_guardian::TopicCtx::load(&mut *conn, s, guardian, id).await?;
            match topic {
                Some(t) if !t.trashed() => {
                    let secure = guardian.secure_category_ids(&mut *conn, s).await?;
                    if !guardian.can_see_topic(s, &t, true, &secure)? {
                        return Ok(Outcome::Forbidden);
                    }
                    t.id
                }
                _ => return Ok(Outcome::Forbidden),
            }
        }
        "Chat::Message" => return Err(Unsupported("bookmarking chat messages").into()),
        other => {
            let message = ctx
                .i18n
                .t_with("bookmarks.errors.invalid_bookmarkable", &[("type", other)])
                .unwrap_or_default();
            return Ok(Outcome::Invalid(vec![message]));
        }
    };
    let reminder_at = crate::user_penalties::cast_time(fields.reminder_at)?;
    let name = fields.name.filter(|n| !n.is_empty());

    // Bookmark's validations, in their order.
    let mut errors = Vec::new();
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM bookmarks WHERE user_id = $1 AND bookmarkable_id = $2 \
                        AND bookmarkable_type = $3)",
    )
    .bind(user_id)
    .bind(id)
    .bind(bookmarkable_type)
    .fetch_one(&mut *conn)
    .await?;
    if exists {
        errors.push(
            ctx.i18n
                .t_with(
                    "bookmarks.errors.already_bookmarked",
                    &[("type", bookmarkable_type)],
                )
                .unwrap_or_default(),
        );
    }
    reminder_errors(ctx, reminder_at, &mut errors);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bookmarks WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    let limit = s.get("max_bookmarks_per_user")?.to_i();
    if count >= limit {
        let urls = crate::url::Urls {
            config: ctx.config,
            settings: s,
        };
        errors.push(
            ctx.i18n
                .t_with(
                    "bookmarks.errors.too_many",
                    &[
                        (
                            "user_bookmarks_url",
                            &format!("{}/my/activity/bookmarks", urls.base_url()?),
                        ),
                        ("limit", &limit.to_string()),
                    ],
                )
                .unwrap_or_default(),
        );
    }
    name_errors(name, &mut errors);
    if !errors.is_empty() {
        return Ok(Outcome::Invalid(errors));
    }

    let preference =
        auto_delete_preference(&mut *conn, user_id, fields.auto_delete_preference).await?;
    let bookmark_id: i64 = sqlx::query_scalar(
        "INSERT INTO bookmarks (user_id, bookmarkable_id, bookmarkable_type, name, reminder_at, reminder_set_at, \
                                pinned, auto_delete_preference, created_at, updated_at) \
         SELECT $1, $2, $3, $4, $5, now, NULL, $6, clock_timestamp(), clock_timestamp() \
         FROM clock_timestamp() AS now RETURNING id",
    )
    .bind(user_id)
    .bind(id)
    .bind(bookmarkable_type)
    .bind(name)
    .bind(reminder_at)
    .bind(preference)
    .fetch_one(&mut *conn)
    .await?;
    // after_create
    sync_topic_user(&mut *conn, user_id, topic_id).await?;
    Ok(Outcome::Created(bookmark_id))
}

/// A bookmark as updating, pinning and destroying read it.
#[derive(sqlx::FromRow)]
struct Found {
    user_id: i64,
    bookmarkable_type: String,
    bookmarkable_id: i64,
    reminder_at: Option<NaiveDateTime>,
    pinned: Option<bool>,
}

/// The bookmark, if it exists, after `find_bookmark_and_check_access`.
async fn find_own(
    conn: &mut PgConnection,
    user_id: i32,
    bookmark_id: i32,
) -> Result<Result<Found, Outcome>, sqlx::Error> {
    let row: Option<Found> = sqlx::query_as(
        "SELECT user_id, bookmarkable_type, bookmarkable_id, reminder_at, pinned FROM bookmarks WHERE id = $1",
    )
    .bind(bookmark_id)
    .fetch_optional(conn)
    .await?;
    Ok(match row {
        None => Err(Outcome::NotFound),
        Some(b) if b.user_id != i64::from(user_id) => Err(Outcome::Forbidden),
        Some(b) => Ok(b),
    })
}

/// `BookmarkManager#update`
pub async fn update(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
    bookmark_id: i32,
    fields: &Fields<'_>,
) -> Result<Outcome, AppError> {
    let Found {
        reminder_at: current_reminder,
        ..
    } = match find_own(&mut *conn, user_id, bookmark_id).await? {
        Ok(found) => found,
        Err(outcome) => return Ok(outcome),
    };
    let reminder_at = crate::user_penalties::cast_time(fields.reminder_at)?;
    let reminder_changed = reminder_at != current_reminder;
    let mut errors = Vec::new();
    reminder_errors(ctx, reminder_at.filter(|_| reminder_changed), &mut errors);
    name_errors(fields.name, &mut errors);
    if !errors.is_empty() {
        return Ok(Outcome::Invalid(errors));
    }
    let preference =
        auto_delete_preference(&mut *conn, user_id, fields.auto_delete_preference).await?;
    // The reminder resets its last sending when it changes; the pinned
    // option, not given, saves as nil.
    sqlx::query(
        "UPDATE bookmarks SET name = $2, reminder_set_at = clock_timestamp(), pinned = NULL, \
                auto_delete_preference = $3, updated_at = clock_timestamp(), \
                reminder_at = CASE WHEN $4 THEN $5 ELSE reminder_at END, \
                reminder_last_sent_at = CASE WHEN $4 THEN NULL ELSE reminder_last_sent_at END \
         WHERE id = $1",
    )
    .bind(bookmark_id)
    .bind(fields.name)
    .bind(preference)
    .bind(reminder_changed)
    .bind(reminder_at)
    .execute(&mut *conn)
    .await?;
    Ok(Outcome::Done)
}

/// `BookmarkManager#toggle_pin`
pub async fn toggle_pin(
    conn: &mut PgConnection,
    user_id: i32,
    bookmark_id: i32,
) -> Result<Outcome, AppError> {
    let Found { pinned, .. } = match find_own(&mut *conn, user_id, bookmark_id).await? {
        Ok(found) => found,
        Err(outcome) => return Ok(outcome),
    };
    sqlx::query("UPDATE bookmarks SET pinned = $2, updated_at = clock_timestamp() WHERE id = $1")
        .bind(bookmark_id)
        .bind(!pinned.unwrap_or(false))
        .execute(&mut *conn)
        .await?;
    Ok(Outcome::Done)
}

/// `BookmarkManager#destroy`, then `bookmark_metadata`.
pub async fn destroy(
    conn: &mut PgConnection,
    user_id: i32,
    bookmark_id: i32,
) -> Result<Outcome, AppError> {
    let Found {
        bookmarkable_type: kind,
        bookmarkable_id: id,
        ..
    } = match find_own(&mut *conn, user_id, bookmark_id).await? {
        Ok(found) => found,
        Err(outcome) => return Ok(outcome),
    };
    let topic_id: i32 = match kind.as_str() {
        "Post" => sqlx::query_scalar::<_, i32>(
            "SELECT topic_id FROM posts WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(Unsupported("destroying a bookmark of a deleted post"))?,
        "Topic" => i32::try_from(id).map_err(|_| Unsupported("topic ids beyond 32 bits"))?,
        _ => return Err(Unsupported("bookmarkables other than posts and topics").into()),
    };
    sqlx::query("DELETE FROM bookmarks WHERE id = $1")
        .bind(bookmark_id)
        .execute(&mut *conn)
        .await?;
    // after_destroy, then the metadata.
    let topic_bookmarked = sync_topic_user(&mut *conn, user_id, topic_id).await?;
    Ok(Outcome::Destroyed { topic_bookmarked })
}
