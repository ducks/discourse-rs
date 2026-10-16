//! Inviting a user to an existing message (`Topic#invite` and its private
//! `invite_to_private_message`) and taking one off it
//! (`Topic#remove_allowed_user`).

use serde_json::json;
use sqlx::{PgConnection, PgPool};

use crate::guardian::Guardian;
use crate::posting::small_action::{self, SmallAction};
use crate::posting::{
    Ctx, TopicUserAttr, change_topic_user, notification_levels, notification_reasons,
};
use crate::rate_limiter::{LimitExceeded, LimitUser, RateLimiter};
use crate::session::current::SessionUser;
use crate::user_comm_screener::{Actor, UserCommScreener};
use crate::{AppError, Unsupported};

/// `Notification.types[:invited_to_private_message]`
const INVITED_TO_PRIVATE_MESSAGE: i32 = 7;

pub enum Invited {
    /// The user now on the message.
    User(i32),
    /// `topic.invite` returned false: `failed_json`.
    Failed,
    /// No user has that email: `Invite.generate` to it, when the
    /// inviter may invite by email.
    ByEmail,
    /// `Topic::UserExists` or `Topic::NotAllowed`: its message.
    Refused(String),
    RateLimited(LimitExceeded),
}

/// `User.find_by_username_or_email`
pub async fn find_by_username_or_email(
    conn: &mut PgConnection,
    username_or_email: &str,
) -> Result<Option<i32>, AppError> {
    if username_or_email.contains('@') {
        Ok(sqlx::query_scalar(
            "SELECT user_id FROM user_emails WHERE lower(email) = lower($1) ORDER BY id LIMIT 1",
        )
        .bind(username_or_email.trim())
        .fetch_optional(conn)
        .await?)
    } else {
        let normalized = crate::accounts::normalize_username(username_or_email)?;
        Ok(
            sqlx::query_scalar("SELECT id FROM users WHERE username_lower = $1")
                .bind(normalized)
                .fetch_optional(conn)
                .await?,
        )
    }
}

/// `Topic#invite(invited_by, username_or_email)` for a message, from the
/// checks on the target to the notification. A target that is not a user
/// is Rails' email invite (refused) or nothing.
pub async fn invite(
    pool: &PgPool,
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    topic_id: i32,
    username_or_email: &str,
) -> Result<Invited, AppError> {
    let s = ctx.settings;
    let inviter = guardian.user().ok_or(Unsupported("inviting anonymously"))?;
    let Some(target) = find_by_username_or_email(&mut *conn, username_or_email).await? else {
        // `username_or_email =~ /\A.+@.+\z/`: the caller invites by email
        // when the guardian may.
        static EMAILISH: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| regex::Regex::new(r"(?s)\A.+@.+\z").unwrap());
        if EMAILISH.is_match(username_or_email) {
            return Ok(Invited::ByEmail);
        }
        return Ok(Invited::Failed);
    };
    let t = |key: &str, args: &[(&str, &str)]| {
        ctx.i18n
            .t_with(key, args)
            .unwrap_or_else(|| key.to_string())
    };
    let allowed: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topic_allowed_users WHERE topic_id = $1 AND user_id = $2)",
    )
    .bind(topic_id)
    .bind(target)
    .fetch_one(&mut *conn)
    .await?;
    if allowed {
        return Ok(Invited::Refused(t("topic_invite.user_exists", &[])));
    }
    let (username, allow_pms, suspended, target_staff): (String, Option<bool>, bool, bool) =
        sqlx::query_as(
            "SELECT u.username, o.allow_private_messages, COALESCE(u.suspended_till > now(), FALSE), \
                    u.admin OR u.moderator \
             FROM users u LEFT JOIN user_options o ON o.user_id = u.id WHERE u.id = $1",
        )
        .bind(target)
        .fetch_one(&mut *conn)
        .await?;
    let screener = UserCommScreener::new(
        &mut *conn,
        Actor {
            id: inviter.id,
            staff: guardian.is_staff(),
        },
        &[target],
    )
    .await?;
    if screener.ignoring_or_muting_actor(target)? {
        return Ok(Invited::Refused(t(
            "not_accepting_pms",
            &[("username", &username)],
        )));
    }
    let muted: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topic_users WHERE topic_id = $1 AND user_id = $2 AND notification_level = 0)",
    )
    .bind(topic_id)
    .bind(target)
    .fetch_one(&mut *conn)
    .await?;
    if muted {
        return Ok(Invited::Refused(t("topic_invite.muted_topic", &[])));
    }
    if screener.disallowing_pms_from_actor(target)? {
        return Ok(Invited::Refused(t(
            "topic_invite.receiver_does_not_allow_pm",
            &[],
        )));
    }
    let back = UserCommScreener::new(
        &mut *conn,
        Actor {
            id: target,
            staff: target_staff,
        },
        &[inviter.id],
    )
    .await?;
    if back.disallowing_pms_from_actor(inviter.id)? {
        return Ok(Invited::Refused(t(
            "topic_invite.sender_does_not_allow_pm",
            &[],
        )));
    }

    // invite_to_private_message: can_send_private_message?(target)
    let can_send = guardian.can_send_private_messages(s)?
        && (guardian.is_staff() || allow_pms.unwrap_or(true))
        && (guardian.is_staff() || !suspended)
        && (!guardian.is_silenced() || target_staff);
    if !can_send {
        return Ok(Invited::Refused(t(
            "activerecord.errors.models.topic.attributes.base.cant_send_pm",
            &[],
        )));
    }
    // rate_limit_topic_invitation
    let limit_user = Some(LimitUser {
        id: inviter.id,
        staff: guardian.is_staff(),
    });
    for (kind, setting, secs) in [
        (
            "topic-invitations-per-day",
            "max_topic_invitations_per_day",
            86_400,
        ),
        (
            "topic-invitations-per-minute",
            "max_topic_invitations_per_minute",
            60,
        ),
    ] {
        let limiter = RateLimiter::new(limit_user, kind, s.get(setting)?.to_i(), secs);
        if let Err(refused) = limiter.performed(pool, ctx.config.rate_limits).await? {
            return Ok(Invited::RateLimited(refused));
        }
    }

    sqlx::query(
        "INSERT INTO topic_allowed_users (topic_id, user_id, created_at, updated_at) \
         VALUES ($1, $2, clock_timestamp(), clock_timestamp())",
    )
    .bind(topic_id)
    .bind(target)
    .execute(&mut *conn)
    .await?;
    // TopicUser.change: watching, so they hear of new posts.
    change_topic_user(
        &mut *conn,
        target,
        topic_id,
        &[TopicUserAttr::NotificationLevel(
            notification_levels::WATCHING,
            notification_reasons::USER_CHANGED,
        )],
    )
    .await?;
    crate::bus::publish_notification_level_change(
        ctx.bus,
        &mut *conn,
        target,
        topic_id,
        notification_levels::WATCHING,
        Some(notification_reasons::USER_CHANGED),
    )
    .await?;
    // The small action, unless the message's author (not the inviter) is
    // in one of its groups.
    let author_in_group: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topic_allowed_groups tag JOIN group_users gu ON gu.group_id = tag.group_id \
                        JOIN topics t ON t.id = tag.topic_id AND gu.user_id = t.user_id \
                        WHERE tag.topic_id = $1)",
    )
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    if !author_in_group {
        small_action::add(
            &mut *conn,
            ctx,
            &SmallAction {
                user_id: inviter.id,
                topic_id,
                action_code: "invited_user",
                bump: false,
                who: Some(&username),
            },
        )
        .await?;
    }
    create_invite_notification(conn, ctx, guardian, topic_id, target).await?;
    Ok(Invited::User(target))
}

/// `create_invite_notification!`, then the notification's after_commit:
/// the target's counts and the invite email.
async fn create_invite_notification(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    topic_id: i32,
    target: i32,
) -> Result<(), AppError> {
    let inviter = guardian.user().ok_or(Unsupported("inviting anonymously"))?;
    let screener = UserCommScreener::new(
        &mut *conn,
        Actor {
            id: inviter.id,
            staff: guardian.is_staff(),
        },
        &[target],
    )
    .await?;
    if screener.ignoring_or_muting_actor(target)? {
        return Err(Unsupported("a user ignoring the inviter in between (NotAllowed)").into());
    }
    let title: String = sqlx::query_scalar("SELECT title FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
    let data = json!({
        "topic_title": title,
        "display_username": inviter.username,
        "original_user_id": inviter.id,
        "original_username": inviter.username,
    });
    notify_invited(conn, ctx, topic_id, target, &data).await
}

/// An invited_to_private_message notification on the message's first
/// post, then its after_commit: the user's counts and the invite email.
async fn notify_invited(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    target: i32,
    data: &serde_json::Value,
) -> Result<(), AppError> {
    crate::notification_create::create(
        conn,
        ctx.bus,
        ctx.settings,
        &crate::notification_create::NewNotification::new(
            INVITED_TO_PRIVATE_MESSAGE,
            target,
            data.clone(),
        )
        .topic(topic_id, Some(1)),
    )
    .await?;
    Ok(())
}

/// `Topic#remove_allowed_user(removed_by, user)`: false when the user is
/// not directly on the message.
pub async fn remove_allowed_user(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    removed_by: i32,
    topic_id: i32,
    user_id: i32,
) -> Result<bool, AppError> {
    let removed =
        sqlx::query("DELETE FROM topic_allowed_users WHERE topic_id = $1 AND user_id = $2")
            .bind(topic_id)
            .bind(user_id)
            .execute(&mut *conn)
            .await?
            .rows_affected();
    if removed == 0 {
        return Ok(false);
    }
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    small_action::add(
        &mut *conn,
        ctx,
        &SmallAction {
            user_id: removed_by,
            topic_id,
            action_code: if user_id == removed_by {
                "user_left"
            } else {
                "removed_user"
            },
            bump: false,
            who: Some(&username),
        },
    )
    .await?;
    // A user who can no longer see the message loses its notifications.
    let user = SessionUser::load(&mut *conn, user_id)
        .await?
        .ok_or(Unsupported("removing a user that is gone"))?;
    let their_guardian = Guardian::for_user(&mut *conn, &user).await?;
    let topic =
        crate::topic_guardian::TopicCtx::load(&mut *conn, ctx.settings, &their_guardian, topic_id)
            .await?
            .ok_or(Unsupported("removing a user from a message that is gone"))?;
    let secure = their_guardian
        .secure_category_ids(&mut *conn, ctx.settings)
        .await?;
    if !their_guardian.can_see_topic(ctx.settings, &topic, false, &secure)? {
        let gone = sqlx::query("DELETE FROM notifications WHERE user_id = $1 AND topic_id = $2")
            .bind(user_id)
            .bind(topic_id)
            .execute(&mut *conn)
            .await?
            .rows_affected();
        if gone > 0 {
            crate::bus::publish_notifications_state(ctx.bus, &mut *conn, ctx.settings, user_id)
                .await?;
        }
    }
    ctx.bus
        .publish(
            &mut *conn,
            &format!("/topic/{topic_id}"),
            &json!({"type": "remove_allowed_user"}),
            Some(&[crate::bus::user_tag(user_id)]),
        )
        .await?;
    Ok(true)
}

/// `Topic#invite_group(user, group, should_notify:)`: the group on the
/// message, a small action on it when it has a visible post (with
/// post_alert and group_pm_alert when notifying), then the group's members
/// directly on the message (but its author) taken off by the system.
pub async fn invite_group(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
    group_id: i32,
    topic_id: i32,
    should_notify: bool,
) -> Result<(), AppError> {
    sqlx::query("INSERT INTO topic_allowed_groups (topic_id, group_id) VALUES ($1, $2)")
        .bind(topic_id)
        .bind(group_id)
        .execute(&mut *conn)
        .await?;
    // update_columns(has_messages: true): no updated_at.
    sqlx::query("UPDATE groups SET has_messages = TRUE WHERE id = $1 AND NOT has_messages")
        .bind(group_id)
        .execute(&mut *conn)
        .await?;
    let group_name: String = sqlx::query_scalar("SELECT name FROM groups WHERE id = $1")
        .bind(group_id)
        .fetch_one(&mut *conn)
        .await?;
    let last_post: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM posts WHERE topic_id = $1 AND NOT hidden AND deleted_at IS NULL \
         ORDER BY post_number DESC LIMIT 1",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(last_post) = last_post {
        small_action::add(
            &mut *conn,
            ctx,
            &SmallAction {
                user_id,
                topic_id,
                action_code: "invited_group",
                bump: false,
                who: Some(&group_name),
            },
        )
        .await?;
        if should_notify {
            crate::jobs::enqueue(&mut *conn, "post_alert", json!({ "post_id": last_post })).await?;
            crate::jobs::enqueue(
                &mut *conn,
                "group_pm_alert",
                json!({ "user_id": user_id, "group_id": group_id, "post_id": last_post }),
            )
            .await?;
        }
    }
    let members: Vec<i32> = sqlx::query_scalar(
        "SELECT users.id FROM users WHERE users.id IN ( \
           SELECT topic_allowed_users.user_id FROM topic_allowed_users \
           INNER JOIN group_users ON group_users.user_id = topic_allowed_users.user_id \
           INNER JOIN topic_allowed_groups ON topic_allowed_groups.group_id = group_users.group_id \
           WHERE topic_allowed_groups.group_id = $1 AND topic_allowed_users.topic_id = $2 \
             AND topic_allowed_users.user_id != (SELECT user_id FROM topics WHERE id = $2)) \
         ORDER BY users.id",
    )
    .bind(group_id)
    .bind(topic_id)
    .fetch_all(&mut *conn)
    .await?;
    for member in members {
        remove_allowed_user(&mut *conn, ctx, -1, topic_id, member).await?;
    }
    Ok(())
}

/// `Topic#remove_allowed_group(removed_by, name)`: false when no group of
/// that name is on the message.
pub async fn remove_allowed_group(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    removed_by: i32,
    topic_id: i32,
    name: &str,
) -> Result<bool, AppError> {
    let group: Option<(i32, String)> =
        sqlx::query_as("SELECT id, name FROM groups WHERE name = $1")
            .bind(name)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((group_id, group_name)) = group else {
        return Ok(false);
    };
    let removed =
        sqlx::query("DELETE FROM topic_allowed_groups WHERE topic_id = $1 AND group_id = $2")
            .bind(topic_id)
            .bind(group_id)
            .execute(&mut *conn)
            .await?
            .rows_affected();
    if removed == 0 {
        return Ok(false);
    }
    small_action::add(
        &mut *conn,
        ctx,
        &SmallAction {
            user_id: removed_by,
            topic_id,
            action_code: "removed_group",
            bump: false,
            who: Some(&group_name),
        },
    )
    .await?;
    crate::jobs::enqueue(
        &mut *conn,
        "delete_inaccessible_notifications",
        json!({ "topic_id": topic_id }),
    )
    .await?;
    Ok(true)
}

/// `Jobs::GroupPmAlert`: the group's default levels on the message (for
/// members without one), its tracking members' summaries, and an
/// invitation for its watching members but the inviter.
pub async fn group_pm_alert(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
    group_id: i32,
    post_id: i32,
) -> Result<(), AppError> {
    let inviter: Option<String> = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&mut *conn)
        .await?;
    let group: Option<(i32, i32)> =
        sqlx::query_as("SELECT user_count, id FROM groups WHERE id = $1")
            .bind(group_id)
            .fetch_optional(&mut *conn)
            .await?;
    let topic: Option<(i32, Option<i32>, String)> = sqlx::query_as(
        "SELECT t.id, t.user_id, t.title FROM posts p JOIN topics t ON t.id = p.topic_id \
           AND t.deleted_at IS NULL WHERE p.id = $1 AND p.deleted_at IS NULL",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    let (Some(inviter), Some((user_count, _)), Some((topic_id, author, title))) =
        (inviter, group, topic)
    else {
        return Ok(());
    };
    let limit = ctx.settings.get("group_pm_user_limit")?.to_i();
    if i64::from(user_count) > limit {
        return Err(
            Unsupported("group_pm_alert for a group over group_pm_user_limit (raises)").into(),
        );
    }
    crate::posting::group_messages::apply_default_levels(
        &mut *conn, group_id, topic_id, author, true,
    )
    .await?;
    let tracking: Vec<i32> = sqlx::query_scalar(
        "SELECT users.id FROM users INNER JOIN group_users ON users.id = group_users.user_id \
         WHERE group_users.group_id = $1 AND group_users.notification_level = 2 ORDER BY users.id",
    )
    .bind(group_id)
    .fetch_all(&mut *conn)
    .await?;
    let stats = crate::posting::group_messages::group_stats(&mut *conn, topic_id).await?;
    for member in tracking {
        crate::posting::group_messages::notify_group_summary(
            ctx, &mut *conn, &stats, member, topic_id, None,
        )
        .await?;
    }
    let watching: Vec<i32> = sqlx::query_scalar(
        "SELECT users.id FROM users INNER JOIN group_users ON users.id = group_users.user_id \
         WHERE group_users.group_id = $1 AND group_users.notification_level IN (3, 4) \
           AND group_users.user_id != $2 ORDER BY users.id",
    )
    .bind(group_id)
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    let data = json!({
        "topic_title": title,
        "display_username": inviter,
        "group_id": group_id,
    });
    for member in watching {
        notify_invited(&mut *conn, ctx, topic_id, member, &data).await?;
    }
    Ok(())
}
