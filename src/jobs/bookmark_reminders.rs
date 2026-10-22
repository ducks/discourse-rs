//! `Jobs::BookmarkReminderNotifications` with
//! `BookmarkReminderNotificationHandler`: every five minutes, the bookmarks
//! whose reminder is due (300 at most, the earliest first) remind their
//! owner with a bookmark_reminder notification, as long as they can still
//! see what they bookmarked; then the bookmark is deleted (when its
//! owner chose that) or its reminder marked sent.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// The scheduled job's name.
pub const JOB: &str = "bookmark_reminder_notifications";

/// `max_reminder_notifications_per_run`
const MAX_PER_RUN: i64 = 300;

/// `Notification.types[:bookmark_reminder]`
const BOOKMARK_REMINDER: i32 = 24;

/// `Bookmark.auto_delete_preferences`
const WHEN_REMINDER_SENT: i32 = 1;
const CLEAR_REMINDER: i32 = 3;

#[derive(sqlx::FromRow)]
struct Bookmark {
    id: i32,
    user_id: i32,
    name: Option<String>,
    bookmarkable_type: String,
    bookmarkable_id: i32,
    auto_delete_preference: i32,
}

/// `execute`: each due bookmark in its own transaction. One that can't be
/// handled doesn't hold back the rest; the first error is the job's.
pub async fn run(state: &AppState) -> Result<(), AppError> {
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id::int FROM bookmarks WHERE reminder_at IS NOT NULL AND reminder_at <= now() \
         AND reminder_last_sent_at IS NULL ORDER BY reminder_at ASC LIMIT $1",
    )
    .bind(MAX_PER_RUN)
    .fetch_all(&state.pool)
    .await?;
    let mut first_error = None;
    for id in ids {
        if let Err(e) = send_notification(state, id).await {
            tracing::error!("bookmark reminder {id}: {e}");
            first_error.get_or_insert(e);
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// `BookmarkReminderNotificationHandler#send_notification`
async fn send_notification(state: &AppState, bookmark_id: i32) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let Some(bookmark) = sqlx::query_as::<_, Bookmark>(
        "SELECT id::int, user_id::int, name, bookmarkable_type, bookmarkable_id::int, auto_delete_preference \
         FROM bookmarks WHERE id = $1 FOR UPDATE",
    )
    .bind(bookmark_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(());
    };
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let notified = match reminder_target(&mut tx, state, &settings, &bookmark).await? {
        Some(target) => {
            notify(&mut tx, state, &settings, &bookmark, &target).await?;
            true
        }
        None => false,
    };
    if notified && bookmark.auto_delete_preference == WHEN_REMINDER_SENT {
        crate::bookmark_manager::destroy(
            &state.bus,
            &mut tx,
            &settings,
            bookmark.user_id,
            bookmark.id,
        )
        .await?;
    } else {
        clear_reminder(&mut tx, &bookmark).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// What the reminder points at: the topic and post number (none for a chat
/// message), and the notification's title and url.
struct Target {
    topic: Option<(i32, i32)>,
    title: String,
    url: String,
}

/// `reminder_conditions` and the data `reminder_handler` sends:
/// PostBookmarkable's post (and its topic) or TopicBookmarkable's topic
/// (through its first post), when the owner can see the post; chat's
/// message (Chat::MessageBookmarkable) when the owner may join its channel.
async fn reminder_target(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    bookmark: &Bookmark,
) -> Result<Option<Target>, AppError> {
    if crate::plugins::chat::bookmarks::is_type(&bookmark.bookmarkable_type) {
        // A filtered register: no bookmarkable while chat is disabled,
        // which Rails fails on.
        if !crate::plugins::chat::bookmarks::registered(settings)? {
            return Err(
                Unsupported("reminders for chat message bookmarks while chat is disabled").into(),
            );
        }
        let Some(user) =
            crate::session::current::SessionUser::load(&mut *conn, bookmark.user_id).await?
        else {
            return Ok(None);
        };
        let guardian = Guardian::for_user(&mut *conn, &user).await?;
        let mut chat = crate::plugins::chat::channels::Context {
            conn: &mut *conn,
            settings,
            i18n: &state.i18n,
            guardian: &guardian,
            base_path: state.config.globals.relative_url_root(),
            config: &state.config,
        };
        return Ok(chat
            .reminder(i64::from(bookmark.bookmarkable_id))
            .await?
            .map(|(title, url)| Target {
                topic: None,
                title,
                url,
            }));
    }
    // Trashed posts and topics are out of their default scopes.
    let post_id: Option<i32> =
        match bookmark.bookmarkable_type.as_str() {
            "Post" => {
                sqlx::query_scalar("SELECT id FROM posts WHERE id = $1 AND deleted_at IS NULL")
                    .bind(bookmark.bookmarkable_id)
                    .fetch_optional(&mut *conn)
                    .await?
            }
            "Topic" => sqlx::query_scalar(
                "SELECT p.id FROM topics t JOIN posts p ON p.topic_id = t.id AND p.post_number = 1 \
               AND p.deleted_at IS NULL \
             WHERE t.id = $1 AND t.deleted_at IS NULL",
            )
            .bind(bookmark.bookmarkable_id)
            .fetch_optional(&mut *conn)
            .await?,
            _ => return Err(Unsupported("reminders for bookmarks of other kinds").into()),
        };
    let Some(post_id) = post_id else {
        return Ok(None);
    };
    let row: Option<(i32, i32, String, Option<String>)> = sqlx::query_as(
        "SELECT p.topic_id, p.post_number, t.title, t.slug FROM posts p \
         JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL WHERE p.id = $1",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((topic_id, post_number, title, slug)) = row else {
        return Ok(None);
    };
    if !owner_can_see_post(conn, settings, bookmark.user_id, topic_id, post_id).await? {
        return Ok(None);
    }
    let slug = slug.ok_or(Unsupported("a reminder for a topic without a slug"))?;
    Ok(Some(Target {
        topic: Some((topic_id, post_number)),
        title,
        // Post#url
        url: format!("/t/{slug}/{topic_id}/{post_number}"),
    }))
}

/// `bookmark.user.guardian.can_see_post?(post)`
async fn owner_can_see_post(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
    topic_id: i32,
    post_id: i32,
) -> Result<bool, AppError> {
    let Some(user) = crate::session::current::SessionUser::load(&mut *conn, user_id).await? else {
        return Ok(false);
    };
    let guardian = Guardian::for_user(&mut *conn, &user).await?;
    let Some(topic) =
        crate::topic_guardian::TopicCtx::load(&mut *conn, settings, &guardian, topic_id).await?
    else {
        return Ok(false);
    };
    let secure = guardian.secure_category_ids(&mut *conn, settings).await?;
    let topic_visible = guardian.can_see_topic(settings, &topic, true, &secure)?;
    let post = crate::topic_guardian::PostCtx::load(&mut *conn, post_id)
        .await?
        .ok_or(Unsupported("a bookmarked post that vanished"))?;
    Ok(guardian.can_see_post(settings, &post, topic_visible)?)
}

/// `BaseBookmarkable.send_reminder_notification`: the notification, then
/// its after_commit work (the owner's notification state; shelved when
/// they are in do not disturb; no email, as EmailUser has no
/// bookmark_reminder).
async fn notify(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    bookmark: &Bookmark,
    target: &Target,
) -> Result<(), AppError> {
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(bookmark.user_id)
        .fetch_one(&mut *conn)
        .await?;
    let mut data = Map::new();
    data.insert("title".into(), json!(target.title));
    data.insert("bookmarkable_url".into(), json!(target.url));
    data.insert("display_username".into(), json!(username));
    data.insert("bookmark_name".into(), json!(bookmark.name));
    data.insert("bookmark_id".into(), json!(bookmark.id));
    data.insert(
        "bookmarkable_type".into(),
        json!(bookmark.bookmarkable_type),
    );
    data.insert("bookmarkable_id".into(), json!(bookmark.bookmarkable_id));
    let mut notification = crate::notification_create::NewNotification::new(
        BOOKMARK_REMINDER,
        bookmark.user_id,
        Value::Object(data),
    );
    if let Some((topic_id, post_number)) = target.topic {
        notification = notification.topic(topic_id, Some(post_number));
    }
    crate::notification_create::create(conn, &state.bus, settings, &notification).await?;
    Ok(())
}

/// `Bookmark#clear_reminder!`: sent now, no longer set; the reminder time
/// itself goes when the owner chose to clear it.
async fn clear_reminder(conn: &mut PgConnection, bookmark: &Bookmark) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE bookmarks SET reminder_last_sent_at = clock_timestamp(), reminder_set_at = NULL, \
           reminder_at = CASE WHEN $2 THEN NULL ELSE reminder_at END, updated_at = clock_timestamp() \
         WHERE id = $1",
    )
    .bind(bookmark.id)
    .bind(bookmark.auto_delete_preference == CLEAR_REMINDER)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
