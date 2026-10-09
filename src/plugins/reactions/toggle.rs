//! `DiscourseReactions::PostReaction::Toggle` and the ReactionManager it
//! drives: a user's reaction on a post goes on, off, or to another one,
//! with the shadow like a reaction counting as a like keeps (created
//! silently through the likes code, removed through it), the reaction
//! notification, and the `/topic/:id/reactions` and `acted` messages.
//!
//! As in Rails, the manager reads the user's like and reaction once, up
//! front, and acts on that reading: switching reactions removes the shadow
//! like twice (the second finds it gone, or for staff removes it again),
//! and the main reaction's row is created, removed and created again when
//! a reaction is switched to it. Reactions in messages are refused, as
//! likes there are.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::Reactions;
use crate::guardian::Guardian;
use crate::likes;
use crate::post_actions::{self, ActOpts, ActionTypes};
use crate::posting::Ctx;
use crate::posting::revisions::find_post;
use crate::{AppError, Unsupported};

const LIKE: i32 = super::LIKE;
/// `Notification.types[:reaction]`
const REACTION_NOTIFICATION: i32 = 25;
/// `ReactionNotification::HEART_ICON_NAME`
const HEART: &str = "heart";

pub enum Outcome {
    /// Toggled; the post's like count as it was loaded, before.
    Done(i32),
    /// `on_model_not_found(:post)`: Discourse::NotFound.
    NotFound,
    /// Discourse::InvalidAccess: the post out of sight, or the user may
    /// not like it or undo what they did.
    InvalidAccess,
    /// `on_failed_policy(:reaction_is_valid)`: `render_json_error(post)`.
    InvalidReaction,
}

/// What the manager read before acting.
struct Manager<'a> {
    ctx: &'a Ctx<'a>,
    guardian: &'a Guardian,
    reactions: &'a Reactions,
    user_id: i32,
    post_id: i32,
    value: &'a str,
    /// `@like`: the user's live like of the post.
    like: bool,
    /// `@reaction`: the row for this value on the post.
    reaction_id: i64,
    /// The like count of the service's post: as loaded, until a
    /// destroyer that removes a like reloads it.
    like_count: i32,
}

/// The user's reaction on the post (`reaction_user`): its id and reaction.
async fn reaction_user(
    conn: &mut PgConnection,
    user_id: i32,
    post_id: i32,
) -> Result<Option<(i64, i64)>, AppError> {
    Ok(sqlx::query_as(
        "SELECT id, reaction_id FROM discourse_reactions_reaction_users \
         WHERE user_id = $1 AND post_id = $2 ORDER BY id LIMIT 1",
    )
    .bind(user_id)
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?)
}

/// `reaction_scope.first_or_create`
async fn first_or_create_reaction(
    conn: &mut PgConnection,
    post_id: i32,
    value: &str,
) -> Result<i64, AppError> {
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM discourse_reactions_reactions \
         WHERE post_id = $1 AND reaction_value = $2 AND reaction_type = 0 ORDER BY id LIMIT 1",
    )
    .bind(post_id)
    .bind(value)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = existing {
        return Ok(id);
    }
    Ok(sqlx::query_scalar(
        "INSERT INTO discourse_reactions_reactions (post_id, reaction_type, reaction_value, created_at, updated_at) \
         VALUES ($1, 0, $2, clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(post_id)
    .bind(value)
    .fetch_one(&mut *conn)
    .await?)
}

/// PUT /discourse-reactions/posts/:post_id/custom-reactions/:reaction/toggle,
/// in the caller's transaction.
pub async fn toggle(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i32,
    value: &str,
) -> Result<Outcome, AppError> {
    let user = guardian
        .user()
        .ok_or(Unsupported("reacting anonymously"))?
        .clone();
    // model :post (Post.find_by: a live post)
    let like_count: Option<i32> =
        sqlx::query_scalar("SELECT like_count FROM posts WHERE id = $1 AND deleted_at IS NULL")
            .bind(post_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(like_count) = like_count else {
        return Ok(Outcome::NotFound);
    };
    // policy :can_see_post
    let Some(access) = find_post(&mut *conn, ctx, guardian, post_id).await? else {
        return Ok(Outcome::InvalidAccess);
    };
    if access.topic.private_message() {
        return Err(Unsupported("reactions in messages").into());
    }
    // policy :reaction_is_valid
    let reactions = Reactions::load(ctx.settings)?;
    if !reactions.is_valid(&mut *conn, value).await? {
        return Ok(Outcome::InvalidReaction);
    }

    // ReactionManager.new
    let like: Option<(i32, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT user_id, created_at FROM post_actions WHERE post_id = $1 AND user_id = $2 \
         AND post_action_type_id = $3 AND deleted_at IS NULL ORDER BY id LIMIT 1",
    )
    .bind(post_id)
    .bind(user.id)
    .bind(LIKE)
    .fetch_optional(&mut *conn)
    .await?;
    let existing = reaction_user(&mut *conn, user.id, post_id).await?;
    let previous: Option<String> = match existing {
        None => like.map(|_| reactions.main.clone()),
        Some((_, reaction_id)) => sqlx::query_scalar(
            "SELECT reaction_value FROM discourse_reactions_reactions WHERE id = $1",
        )
        .bind(reaction_id)
        .fetch_optional(&mut *conn)
        .await?
        .flatten(),
    };

    // toggle!: can_use_reactions? (post_can_act? for a like, with no taken
    // actions), then may the user undo what they did.
    let types = ActionTypes::load(&mut *conn).await?;
    let author_missing = match access.post.user_id {
        Some(id) => {
            !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
                .bind(id)
                .fetch_one(&mut *conn)
                .await?
        }
        None => true,
    };
    let can_use = guardian.post_can_act(
        ctx.settings,
        &types,
        ("like", post_actions::LIKE),
        &ActOpts {
            topic: &access.topic,
            post: &access.post,
            taken: None,
            can_see_post: access.can_see_post,
            author_missing,
        },
    )?;
    if !can_use {
        return Ok(Outcome::InvalidAccess);
    }
    if let Some((like_user, like_created_at)) = like
        && !guardian.can_delete_post_action(
            ctx.settings,
            &access.topic,
            like_user,
            like_created_at,
        )?
    {
        return Ok(Outcome::InvalidAccess);
    }
    if let Some((reaction_user_id, _)) = existing {
        // ReactionUser#can_undo?
        let can_undo: bool = sqlx::query_scalar(
            "SELECT created_at > now() - make_interval(mins => $2) \
             FROM discourse_reactions_reaction_users WHERE id = $1",
        )
        .bind(reaction_user_id)
        .bind(reactions.undo_window_mins as i32)
        .fetch_one(&mut *conn)
        .await?;
        if !can_undo {
            return Ok(Outcome::InvalidAccess);
        }
    }

    let reaction_id = first_or_create_reaction(&mut *conn, post_id, value).await?;
    let mut manager = Manager {
        ctx,
        guardian,
        reactions: &reactions,
        user_id: user.id,
        post_id,
        value,
        like: like.is_some(),
        reaction_id,
        like_count,
    };
    if value == reactions.main {
        manager.toggle_like(&mut *conn, existing).await?;
    } else {
        manager
            .toggle_reaction(&mut *conn, existing, previous.as_deref())
            .await?;
    }

    // publish_post_acted
    crate::bus::publish_post_change(ctx, &mut *conn, post_id, "acted", Map::new(), false).await?;
    // publish_reaction_change, to the topic's audience.
    let mut changed = vec![value.to_string()];
    if let Some(previous) = previous
        && previous != value
    {
        changed.push(previous);
    }
    crate::bus::publish_to_topic_channel(
        ctx.bus,
        &mut *conn,
        access.topic_id,
        &format!("/topic/{}/reactions", access.topic_id),
        &json!({"post_id": post_id, "reactions": changed}),
    )
    .await?;
    Ok(Outcome::Done(manager.like_count))
}

impl Manager<'_> {
    async fn toggle_like(
        &mut self,
        conn: &mut PgConnection,
        existing: Option<(i64, i64)>,
    ) -> Result<(), AppError> {
        if let Some(existing) = existing {
            self.remove_reaction(&mut *conn, existing).await?;
            self.reaction_id =
                first_or_create_reaction(&mut *conn, self.post_id, self.value).await?;
            self.add_reaction(&mut *conn).await
        } else if self.like {
            self.remove_shadow_like(&mut *conn).await
        } else {
            self.add_shadow_like(&mut *conn, true).await
        }
    }

    async fn toggle_reaction(
        &mut self,
        conn: &mut PgConnection,
        existing: Option<(i64, i64)>,
        previous: Option<&str>,
    ) -> Result<(), AppError> {
        if let Some(existing) = existing {
            self.remove_reaction(&mut *conn, existing).await?;
            if previous == Some(self.value) {
                return Ok(());
            }
        }
        if self.like {
            self.remove_shadow_like(&mut *conn).await?;
        }
        if reaction_user(&mut *conn, self.user_id, self.post_id)
            .await?
            .is_none()
        {
            self.add_reaction(&mut *conn).await?;
        }
        Ok(())
    }

    /// `add_shadow_like`: PostActionCreator.like, silent; its failures
    /// (already liked, not allowed) are ignored.
    async fn add_shadow_like(
        &mut self,
        conn: &mut PgConnection,
        notify: bool,
    ) -> Result<(), AppError> {
        likes::create(&mut *conn, self.ctx, self.guardian, self.post_id, true).await?;
        if notify {
            self.add_notification(&mut *conn).await?;
        }
        Ok(())
    }

    /// `remove_shadow_like`: PostActionDestroyer whatever it finds, then
    /// the main reaction's rows go and the notification is reconsidered.
    async fn remove_shadow_like(&mut self, conn: &mut PgConnection) -> Result<(), AppError> {
        // `result.post = @post.reload` once a like is removed.
        if let likes::Outcome::Done =
            likes::destroy(&mut *conn, self.ctx, self.guardian, self.post_id).await?
        {
            self.like_count = sqlx::query_scalar("SELECT like_count FROM posts WHERE id = $1")
                .bind(self.post_id)
                .fetch_one(&mut *conn)
                .await?;
        }
        // delete_like_reaction
        sqlx::query(
            "DELETE FROM discourse_reactions_reactions WHERE reaction_value = $1 AND post_id = $2",
        )
        .bind(&self.reactions.main)
        .bind(self.post_id)
        .execute(&mut *conn)
        .await?;
        self.remove_notification(&mut *conn).await
    }

    /// `add_reaction`: the user's ReactionUser on the current reaction
    /// (its counter cache counts it), its shadow like unless excluded, the
    /// notification.
    async fn add_reaction(&mut self, conn: &mut PgConnection) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO discourse_reactions_reaction_users (reaction_id, user_id, post_id, created_at, updated_at) \
             VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp())",
        )
        .bind(self.reaction_id)
        .bind(self.user_id)
        .bind(self.post_id)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "UPDATE discourse_reactions_reactions \
             SET reaction_users_count = COALESCE(reaction_users_count, 0) + 1 WHERE id = $1",
        )
        .bind(self.reaction_id)
        .execute(&mut *conn)
        .await?;
        if !self.reactions.excluded_from_like(self.value) {
            self.add_shadow_like(&mut *conn, false).await?;
        }
        self.add_notification(&mut *conn).await
    }

    /// `remove_reaction`: the user's ReactionUser (uncounted from its
    /// reaction), the shadow like, and reactions left without users.
    async fn remove_reaction(
        &mut self,
        conn: &mut PgConnection,
        (reaction_user_id, reaction_id): (i64, i64),
    ) -> Result<(), AppError> {
        sqlx::query("DELETE FROM discourse_reactions_reaction_users WHERE id = $1")
            .bind(reaction_user_id)
            .execute(&mut *conn)
            .await?;
        sqlx::query(
            "UPDATE discourse_reactions_reactions \
             SET reaction_users_count = COALESCE(reaction_users_count, 0) - 1 WHERE id = $1",
        )
        .bind(reaction_id)
        .execute(&mut *conn)
        .await?;
        self.remove_shadow_like(&mut *conn).await?;
        // delete_reaction_with_no_users
        sqlx::query("DELETE FROM discourse_reactions_reactions WHERE reaction_users_count = 0 AND post_id = $1")
            .bind(self.post_id)
            .execute(&mut *conn)
            .await?;
        Ok(())
    }

    /// `ReactionNotification#create`
    async fn add_notification(&self, conn: &mut PgConnection) -> Result<(), AppError> {
        let (username, name): (String, Option<String>) =
            sqlx::query_as("SELECT username, name FROM users WHERE id = $1")
                .bind(self.user_id)
                .fetch_one(&mut *conn)
                .await?;
        let mut custom_data = Map::new();
        if self.value == HEART {
            custom_data.insert("reaction_icon".into(), json!(self.value));
        }
        crate::jobs::post_alert::notify_reaction(
            self.ctx,
            &mut *conn,
            self.post_id,
            self.user_id,
            &username,
            name.as_deref(),
            custom_data,
        )
        .await
    }

    /// `ReactionNotification#delete`: once the user has no reaction left
    /// on the post, the author's reaction notifications on it go and one
    /// is rebuilt from the reactions of the last day.
    async fn remove_notification(&self, conn: &mut PgConnection) -> Result<(), AppError> {
        let remaining: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM discourse_reactions_reactions r \
             JOIN discourse_reactions_reaction_users ru ON ru.reaction_id = r.id \
             WHERE r.post_id = $1 AND ru.user_id = $2",
        )
        .bind(self.post_id)
        .bind(self.user_id)
        .fetch_one(&mut *conn)
        .await?;
        if remaining != 0 {
            return Ok(());
        }
        let post: (Option<i32>, i32, i32, Option<String>) = sqlx::query_as(
            "SELECT p.user_id, p.topic_id, p.post_number, t.title FROM posts p \
             LEFT JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL WHERE p.id = $1",
        )
        .bind(self.post_id)
        .fetch_one(&mut *conn)
        .await?;
        let (Some(author), topic_id, post_number, title) = post else {
            return Ok(());
        };
        let reads: Vec<bool> = sqlx::query_scalar(
            "DELETE FROM notifications WHERE topic_id = $1 AND user_id = $2 AND post_number = $3 \
             AND notification_type = $4 RETURNING read",
        )
        .bind(topic_id)
        .bind(author)
        .bind(post_number)
        .bind(REACTION_NOTIFICATION)
        .fetch_all(&mut *conn)
        .await?;
        let read = reads.iter().all(|r| *r);
        // refresh_notification: a live topic, and reactions of the last day.
        let mut created = false;
        if let Some(title) = title {
            let remaining: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
                "SELECT u.username, u.name, r.reaction_value FROM discourse_reactions_reactions r \
                 JOIN discourse_reactions_reaction_users ru ON ru.reaction_id = r.id \
                 JOIN users u ON u.id = ru.user_id \
                 WHERE r.post_id = $1 AND r.created_at > now() - interval '1 day' \
                 ORDER BY r.created_at DESC",
            )
            .bind(self.post_id)
            .fetch_all(&mut *conn)
            .await?;
            if let Some((username, name, _)) = remaining.first() {
                let mut data = Map::new();
                data.insert("topic_title".into(), json!(title));
                data.insert("count".into(), json!(remaining.len()));
                data.insert("username".into(), json!(username));
                data.insert("display_username".into(), json!(username));
                data.insert("display_name".into(), json!(name));
                if let Some((username2, name2, _)) = remaining.get(1) {
                    data.insert("username2".into(), json!(username2));
                    data.insert("name2".into(), json!(name2));
                }
                if remaining
                    .iter()
                    .all(|(_, _, value)| value.as_deref() == Some(HEART))
                {
                    data.insert("reaction_icon".into(), json!(HEART));
                }
                sqlx::query(
                    "INSERT INTO notifications (notification_type, user_id, topic_id, post_number, data, read, \
                                                high_priority, created_at, updated_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, FALSE, clock_timestamp(), clock_timestamp())",
                )
                .bind(REACTION_NOTIFICATION)
                .bind(author)
                .bind(topic_id)
                .bind(post_number)
                .bind(Value::Object(data).to_string())
                .bind(read)
                .execute(&mut *conn)
                .await?;
                created = true;
            }
        }
        // after_commit refresh_notification_count
        if created || !reads.is_empty() {
            crate::bus::publish_notifications_state(
                self.ctx.bus,
                &mut *conn,
                self.ctx.settings,
                author,
            )
            .await?;
        }
        Ok(())
    }
}
