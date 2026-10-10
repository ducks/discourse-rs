//! What chat's mentions notify: Jobs::Chat::NotifyMentioned, a chat_mention
//! notification (linked to its mention by chat_mention_notifications) for
//! each mentioned member, with its alert.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `Notification.types[:chat_mention]`
const CHAT_MENTION: i32 = 29;
/// `Chat::UserChatChannelMembership.notification_levels[:never]`
const NOTIFICATION_NEVER: i32 = 0;

pub const NOTIFY_MENTIONED_JOB: &str = "Jobs::Chat::NotifyMentioned";

#[derive(sqlx::FromRow)]
struct MessageRow {
    chat_channel_id: i64,
    user_id: Option<i32>,
    message: Option<String>,
    thread_id: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct ChannelRow {
    chatable_type: String,
    name: Option<String>,
    slug: Option<String>,
    threading_enabled: bool,
    category_name: Option<String>,
}

#[derive(sqlx::FromRow)]
struct Membership {
    user_id: i32,
    notification_level: i32,
    muted: bool,
    last_read_message_id: Option<i64>,
}

/// Jobs::Chat::NotifyMentioned
pub async fn notify_mentioned(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    args: &Value,
) -> Result<(), AppError> {
    let Some(message_id) = args["chat_message_id"].as_i64() else {
        return Ok(());
    };
    let message: Option<MessageRow> = sqlx::query_as(
        "SELECT chat_channel_id, user_id, message, thread_id FROM chat_messages WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(message_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(message) = message else {
        return Ok(());
    };
    // An edit since the job was enqueued notifies on its own.
    let timestamp = args["timestamp"].as_str().unwrap_or_default();
    let edited_since: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM chat_message_revisions WHERE chat_message_id = $1 AND created_at > $2::timestamptz)",
    )
    .bind(message_id)
    .bind(timestamp)
    .fetch_one(&mut *conn)
    .await?;
    if edited_since {
        return Ok(());
    }
    let creator_id = message
        .user_id
        .ok_or(Unsupported("chat messages of deleted users"))?;
    let creator: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(creator_id)
        .fetch_one(&mut *conn)
        .await?;
    let channel: ChannelRow = sqlx::query_as(
        "SELECT c.chatable_type, c.name, c.slug, c.threading_enabled, categories.name AS category_name \
         FROM chat_channels c LEFT JOIN categories ON c.chatable_type = 'Category' AND categories.id = c.chatable_id \
         WHERE c.id = $1",
    )
    .bind(message.chat_channel_id)
    .fetch_one(&mut *conn)
    .await?;
    if channel.chatable_type != "Category" {
        return Err(Unsupported("chat direct messages").into());
    }
    // Chat::Message#in_thread?: a thread that shows.
    let in_thread = match message.thread_id {
        Some(thread_id) => {
            let force: bool = sqlx::query_scalar("SELECT force FROM chat_threads WHERE id = $1")
                .bind(thread_id)
                .fetch_one(&mut *conn)
                .await?;
            channel.threading_enabled || force
        }
        None => false,
    };
    if in_thread {
        return Err(Unsupported("chat threads").into());
    }
    let already: Vec<i64> = args["already_notified_user_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_i64)
        .collect();
    let Some(map) = args["to_notify_ids_map"].as_object() else {
        return Ok(());
    };
    let job = Job {
        state,
        settings,
        message_id,
        channel_id: message.chat_channel_id,
        creator: &creator,
        title: channel
            .name
            .clone()
            .filter(|n| !n.is_empty())
            .or(channel.category_name.clone())
            .unwrap_or_default(),
        slug: channel.slug.clone(),
        excerpt: crate::emoji::gsub_emoji_to_unicode(
            message.message.as_deref().unwrap_or_default(),
        ),
    };
    for (mention_type, ids) in map {
        let ids: Vec<i64> = ids
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_i64)
            .filter(|id| !already.contains(id))
            .collect();
        job.process(conn, mention_type, &ids).await?;
    }
    Ok(())
}

struct Job<'a> {
    state: &'a AppState,
    settings: &'a SiteSettings,
    message_id: i64,
    channel_id: i64,
    creator: &'a str,
    title: String,
    slug: Option<String>,
    excerpt: String,
}

impl Job<'_> {
    /// process_mentions: each member (following, the channel being public)
    /// whose mention the message holds.
    async fn process(
        &self,
        conn: &mut PgConnection,
        mention_type: &str,
        user_ids: &[i64],
    ) -> Result<(), AppError> {
        let memberships: Vec<Membership> = sqlx::query_as(
            "SELECT user_id, notification_level, muted, last_read_message_id FROM user_chat_channel_memberships \
             WHERE user_id = ANY($1) AND chat_channel_id = $2 AND following ORDER BY id",
        )
        .bind(user_ids.iter().map(|id| *id as i32).collect::<Vec<i32>>())
        .bind(self.channel_id)
        .fetch_all(&mut *conn)
        .await?;
        for membership in memberships {
            let mention = self
                .find_mention(conn, mention_type, membership.user_id)
                .await?;
            let Some(mention) = mention else {
                continue;
            };
            self.create_notification(conn, &membership, mention, mention_type)
                .await?;
            self.send_notifications(conn, &membership, mention_type)
                .await?;
        }
        Ok(())
    }

    /// find_mention: the message's mention of this type, a group's by its
    /// name, the member's own for a direct one.
    async fn find_mention(
        &self,
        conn: &mut PgConnection,
        mention_type: &str,
        user_id: i32,
    ) -> Result<Option<i64>, AppError> {
        let (kind, target): (&str, Option<i32>) = match mention_type {
            "direct_mentions" => ("Chat::UserMention", Some(user_id)),
            "here_mentions" => ("Chat::HereMention", None),
            "global_mentions" => ("Chat::AllMention", None),
            group => {
                let id: Option<i32> =
                    sqlx::query_scalar("SELECT id FROM groups WHERE LOWER(name) = $1")
                        .bind(group)
                        .fetch_optional(&mut *conn)
                        .await?;
                ("Chat::GroupMention", id)
            }
        };
        Ok(sqlx::query_scalar(
            "SELECT id FROM chat_mentions WHERE chat_message_id = $1 AND type = $2 \
               AND target_id IS NOT DISTINCT FROM $3 ORDER BY id LIMIT 1",
        )
        .bind(self.message_id)
        .bind(kind)
        .bind(target)
        .fetch_optional(&mut *conn)
        .await?)
    }

    /// create_notification!: read when the member has read past it.
    async fn create_notification(
        &self,
        conn: &mut PgConnection,
        membership: &Membership,
        mention: i64,
        mention_type: &str,
    ) -> Result<(), AppError> {
        let mut data = Map::new();
        data.insert("chat_message_id".into(), json!(self.message_id));
        data.insert("chat_channel_id".into(), json!(self.channel_id));
        data.insert("mentioned_by_username".into(), json!(self.creator));
        data.insert("is_direct_message_channel".into(), json!(false));
        data.insert("chat_channel_title".into(), json!(self.title));
        data.insert("chat_channel_slug".into(), json!(self.slug));
        match mention_type {
            "direct_mentions" => {}
            "here_mentions" => {
                data.insert("identifier".into(), json!("here"));
            }
            "global_mentions" => {
                data.insert("identifier".into(), json!("all"));
            }
            group => {
                data.insert("identifier".into(), json!(group));
                data.insert("is_group_mention".into(), json!(true));
            }
        }
        // Chat::Notifier.user_has_seen_message?
        let read = membership.last_read_message_id.unwrap_or(0) >= self.message_id;
        let notification_id: i64 = sqlx::query_scalar(
            "INSERT INTO notifications (notification_type, user_id, data, read, high_priority, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, TRUE, now(), now()) RETURNING id",
        )
        .bind(CHAT_MENTION)
        .bind(membership.user_id)
        .bind(Value::Object(data).to_string())
        .bind(read)
        .fetch_one(&mut *conn)
        .await?;
        sqlx::query(
            "INSERT INTO chat_mention_notifications (chat_mention_id, notification_id) VALUES ($1, $2)",
        )
        .bind(mention)
        .bind(notification_id)
        .execute(&mut *conn)
        .await?;
        // after_commit refresh_notification_count; send_email has no
        // chat_mention email (chat's summary emails are their own).
        crate::bus::publish_notifications_state(
            &self.state.bus,
            &mut *conn,
            self.settings,
            membership.user_id,
        )
        .await?;
        Ok(())
    }

    /// send_notifications: the alert, unless the member mutes the channel
    /// or never wants its notifications; push notifications refused.
    async fn send_notifications(
        &self,
        conn: &mut PgConnection,
        membership: &Membership,
        mention_type: &str,
    ) -> Result<(), AppError> {
        if membership.notification_level == NOTIFICATION_NEVER || membership.muted {
            return Ok(());
        }
        let urls = crate::url::Urls {
            config: &self.state.config,
            settings: self.settings,
        };
        let base_path = self.state.config.globals.relative_url_root();
        let hostname = urls.current_hostname()?;
        let identifier = match mention_type {
            "direct_mentions" => String::new(),
            "here_mentions" => "@here".to_string(),
            "global_mentions" => "@all".to_string(),
            group => format!("@{group}"),
        };
        let key = if mention_type == "direct_mentions" {
            "discourse_push_notifications.popup.chat_mention.direct"
        } else {
            "discourse_push_notifications.popup.chat_mention.other_type"
        };
        let title = self
            .state
            .i18n
            .t_with(
                key,
                &[
                    ("username", self.creator),
                    ("identifier", &identifier),
                    ("channel", &self.title),
                ],
            )
            .unwrap_or_default();
        let payload = json!({
            "notification_type": CHAT_MENTION,
            "username": self.creator,
            "chat_message_id": self.message_id,
            "tag": format!("{hostname}-chat-mention-{}", self.channel_id),
            "excerpt": truncate(&self.excerpt, 400),
            "post_url": format!(
                "{base_path}/chat/c/{}/{}/{}",
                self.slug.as_deref().unwrap_or("-"),
                self.channel_id,
                self.message_id
            ),
            "channel_id": self.channel_id,
            "is_direct_message_channel": false,
            "actions": [{
                "action": "chat-reply",
                "title": self.state.i18n.t("discourse_push_notifications.actions.chat_reply.title").unwrap_or_default(),
                "placeholder": self.state.i18n.t("discourse_push_notifications.actions.chat_reply.placeholder").unwrap_or_default(),
                "type": "text",
                "icon": format!("{}/assets/push-notifications/inline_reply.png", urls.base_url()?),
            }],
            "action_data": { "channel_id": self.channel_id, "message_id": self.message_id },
            "translated_title": title,
        });
        self.state
            .bus
            .publish(
                &mut *conn,
                &format!("/chat/notification-alert/{}", membership.user_id),
                &payload,
                Some(&[crate::bus::user_tag(membership.user_id)]),
            )
            .await?;
        // PostAlerter.push_notification
        let push: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM push_subscriptions WHERE user_id = $1) \
             OR EXISTS (SELECT 1 FROM user_api_keys k \
                        WHERE k.user_id = $1 AND k.revoked_at IS NULL AND k.push_url IS NOT NULL)",
        )
        .bind(membership.user_id)
        .fetch_one(&mut *conn)
        .await?;
        if push {
            return Err(Unsupported("push notifications").into());
        }
        Ok(())
    }
}

/// ActiveSupport's String#truncate: at most `max` characters, the last
/// three "..." when cut.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max - 3).collect();
    format!("{kept}...")
}

pub const NOTIFY_WATCHING_JOB: &str = "Jobs::Chat::NotifyWatching";

/// `Chat::UserChatChannelMembership::NOTIFICATION_LEVELS[:always]`
const NOTIFICATION_ALWAYS: i32 = 2;
/// `Notification.types[:chat_message]`
const CHAT_MESSAGE: i32 = 30;

/// Jobs::Chat::NotifyWatching: the alert of a new message for the
/// channel's members who want every one (notification_level always, not
/// muted), who can chat, aren't suspended, can still join it and haven't
/// read past it; push notifications refused.
pub async fn notify_watching(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    args: &Value,
) -> Result<(), AppError> {
    let Some(message_id) = args["chat_message_id"].as_i64() else {
        return Ok(());
    };
    let message: Option<MessageRow> = sqlx::query_as(
        "SELECT chat_channel_id, user_id, message, thread_id FROM chat_messages WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(message_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(message) = message else {
        return Ok(());
    };
    let sql = format!(
        "SELECT {} FROM chat_channels WHERE id = $1",
        super::channels::CHANNEL_COLUMNS
    );
    let channel: super::channels::ChannelRow = sqlx::query_as(&sql)
        .bind(message.chat_channel_id)
        .fetch_one(&mut *conn)
        .await?;
    if channel.chatable_type != "Category" {
        return Err(Unsupported("chat direct messages").into());
    }
    if message.thread_id.is_some() {
        let force: bool = sqlx::query_scalar("SELECT force FROM chat_threads WHERE id = $1")
            .bind(message.thread_id)
            .fetch_one(&mut *conn)
            .await?;
        if channel.threading_enabled || force {
            return Err(Unsupported("chat threads").into());
        }
    }
    let creator_id = message
        .user_id
        .ok_or(Unsupported("chat messages of deleted users"))?;
    let creator: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(creator_id)
        .fetch_one(&mut *conn)
        .await?;
    let except: Vec<i32> = args["except_user_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_i64().map(|id| id as i32))
        .collect();
    // A thread's watchers are its members watching it; the message is in
    // no shown thread here.
    let members: Vec<Membership> = sqlx::query_as(
        "SELECT m.user_id, m.notification_level, m.muted, m.last_read_message_id \
         FROM user_chat_channel_memberships m \
         JOIN users ON users.id = m.user_id \
         JOIN user_options ON user_options.user_id = users.id AND user_options.chat_enabled \
         WHERE m.chat_channel_id = $1 AND m.following AND NOT (m.user_id = ANY($2)) \
           AND m.notification_level = $3 \
           AND (users.suspended_till IS NULL OR users.suspended_till <= now()) \
         ORDER BY m.id",
    )
    .bind(channel.id)
    .bind(&except)
    .bind(NOTIFICATION_ALWAYS)
    .fetch_all(&mut *conn)
    .await?;
    // CategoryChannel#title: its name, else its category's.
    let channel_title = match channel.name.clone().filter(|n| !n.is_empty()) {
        Some(name) => name,
        None => {
            sqlx::query_scalar("SELECT name FROM categories WHERE id = $1")
                .bind(channel.chatable_id as i32)
                .fetch_one(&mut *conn)
                .await?
        }
    };
    for membership in members {
        let Some(user) =
            crate::session::current::SessionUser::load(&mut *conn, membership.user_id).await?
        else {
            continue;
        };
        let guardian = crate::guardian::Guardian::for_user(&mut *conn, &user).await?;
        let can_join = super::channels::Context {
            conn: &mut *conn,
            settings,
            i18n: &state.i18n,
            guardian: &guardian,
            base_path: state.config.globals.relative_url_root(),
            config: &state.config,
        }
        .can_join(&channel, None)
        .await?;
        if !can_join || membership.last_read_message_id.unwrap_or(0) >= message_id {
            continue;
        }
        if membership.muted {
            continue;
        }
        let urls = crate::url::Urls {
            config: &state.config,
            settings,
        };
        let base_path = state.config.globals.relative_url_root();
        let payload = json!({
            "username": creator,
            "notification_type": CHAT_MESSAGE,
            "chat_message_id": message_id,
            "post_url": format!("{base_path}/chat/c/-/{}/{message_id}", channel.id),
            "translated_title": state
                .i18n
                .t_with(
                    "discourse_push_notifications.popup.new_chat_message",
                    &[("username", creator.as_str()), ("channel", channel_title.as_str())],
                )
                .unwrap_or_default(),
            "tag": format!("{}-chat-message-{}", urls.current_hostname()?, channel.id),
            "excerpt": truncate(
                &crate::emoji::gsub_emoji_to_unicode(message.message.as_deref().unwrap_or_default()),
                400
            ),
            "channel_id": channel.id,
            "is_direct_message_channel": false,
            "actions": [{
                "action": "chat-reply",
                "title": state.i18n.t("discourse_push_notifications.actions.chat_reply.title").unwrap_or_default(),
                "placeholder": state.i18n.t("discourse_push_notifications.actions.chat_reply.placeholder").unwrap_or_default(),
                "type": "text",
                "icon": format!("{}/assets/push-notifications/inline_reply.png", urls.base_url()?),
            }],
            "action_data": { "channel_id": channel.id, "message_id": message_id },
        });
        state
            .bus
            .publish(
                &mut *conn,
                &format!("/chat/notification-alert/{}", membership.user_id),
                &payload,
                Some(&[crate::bus::user_tag(membership.user_id)]),
            )
            .await?;
        // PostAlerter.push_notification
        let push: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM push_subscriptions WHERE user_id = $1) \
             OR EXISTS (SELECT 1 FROM user_api_keys k \
                        WHERE k.user_id = $1 AND k.revoked_at IS NULL AND k.push_url IS NOT NULL)",
        )
        .bind(membership.user_id)
        .fetch_one(&mut *conn)
        .await?;
        if push {
            return Err(Unsupported("push notifications").into());
        }
    }
    Ok(())
}
