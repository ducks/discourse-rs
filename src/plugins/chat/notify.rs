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
    // @creator: nil for a deleted author, which Rails only fails on once
    // a mention is to be notified.
    let creator: Option<String> = match message.user_id {
        Some(id) => {
            sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    let channel: ChannelRow = sqlx::query_as(
        "SELECT c.chatable_type, c.name, c.slug, c.threading_enabled, categories.name AS category_name \
         FROM chat_channels c LEFT JOIN categories ON c.chatable_type = 'Category' AND categories.id = c.chatable_id \
         WHERE c.id = $1",
    )
    .bind(message.chat_channel_id)
    .fetch_one(&mut *conn)
    .await?;
    let direct_message = channel.chatable_type == "DirectMessage";
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
        direct_message,
        channel_id: message.chat_channel_id,
        thread_id: message.thread_id,
        in_thread,
        creator: creator.as_deref(),
        title: channel
            .name
            .clone()
            .filter(|n| !n.is_empty())
            .or(channel.category_name.clone())
            .unwrap_or_default(),
        slug: channel.slug.clone(),
        excerpt: settings
            .emojis()
            .gsub_to_unicode(message.message.as_deref().unwrap_or_default()),
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
    direct_message: bool,
    channel_id: i64,
    thread_id: Option<i64>,
    /// `Chat::Message#in_thread?`
    in_thread: bool,
    /// The author's username, None once they are deleted.
    creator: Option<&'a str>,
    title: String,
    slug: Option<String>,
    excerpt: String,
}

impl Job<'_> {
    /// `@creator.username`, which fails on a deleted author.
    fn creator(&self) -> Result<&str, Unsupported> {
        self.creator
            .ok_or(Unsupported("notifying of a chat message of a deleted user"))
    }

    /// process_mentions: each member (following, the channel being public;
    /// any member of a DM) whose mention the message holds.
    async fn process(
        &self,
        conn: &mut PgConnection,
        mention_type: &str,
        user_ids: &[i64],
    ) -> Result<(), AppError> {
        let memberships: Vec<Membership> = sqlx::query_as(
            "SELECT user_id, notification_level, muted, last_read_message_id FROM user_chat_channel_memberships \
             WHERE user_id = ANY($1) AND chat_channel_id = $2 AND (following OR $3) ORDER BY id",
        )
        .bind(user_ids.iter().map(|id| *id as i32).collect::<Vec<i32>>())
        .bind(self.channel_id)
        .bind(self.direct_message)
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
        let creator = self.creator()?;
        let mut data = Map::new();
        data.insert("chat_message_id".into(), json!(self.message_id));
        data.insert("chat_channel_id".into(), json!(self.channel_id));
        data.insert("mentioned_by_username".into(), json!(creator));
        data.insert(
            "is_direct_message_channel".into(),
            json!(self.direct_message),
        );
        if self.in_thread {
            data.insert("chat_thread_id".into(), json!(self.thread_id));
        }
        if !self.direct_message {
            data.insert("chat_channel_title".into(), json!(self.title));
            data.insert("chat_channel_slug".into(), json!(self.slug));
        }
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
        // Notification.create!, its after_commit work (no chat_mention
        // email: chat's summary emails are their own), then
        // `mention.notifications << notification`.
        let notification_id = crate::notification_create::create(
            conn,
            &self.state.bus,
            self.settings,
            &crate::notification_create::NewNotification::new(
                CHAT_MENTION,
                membership.user_id,
                Value::Object(data),
            )
            .read(read)
            .high_priority(true),
        )
        .await?;
        sqlx::query(
            "INSERT INTO chat_mention_notifications (chat_mention_id, notification_id) VALUES ($1, $2)",
        )
        .bind(mention)
        .bind(notification_id)
        .execute(&mut *conn)
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
        let creator = self.creator()?;
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
        let prefix = if self.direct_message {
            "discourse_push_notifications.popup.direct_message_chat_mention"
        } else {
            "discourse_push_notifications.popup.chat_mention"
        };
        let key = if mention_type == "direct_mentions" {
            format!("{prefix}.direct")
        } else {
            format!("{prefix}.other_type")
        };
        // @chat_channel.title(membership.user): a DM's is per member.
        let channel_title = if self.direct_message {
            self.dm_title_for(conn, membership.user_id).await?
        } else {
            self.title.clone()
        };
        let title = self
            .state
            .i18n
            .t_with(
                &key,
                &[
                    ("username", creator),
                    ("identifier", &identifier),
                    ("channel", &channel_title),
                ],
            )
            .unwrap_or_default();
        let payload = json!({
            "notification_type": CHAT_MENTION,
            "username": creator,
            "chat_message_id": self.message_id,
            "tag": format!("{hostname}-chat-mention-{}", self.channel_id),
            "excerpt": truncate(&self.excerpt, 400),
            // A thread's relative_url, else the channel's and the message.
            "post_url": match (self.in_thread, self.thread_id) {
                (true, Some(thread_id)) => format!(
                    "{base_path}/chat/c/{}/{}/t/{thread_id}",
                    self.slug.as_deref().unwrap_or("-"),
                    self.channel_id,
                ),
                _ => format!(
                    "{base_path}/chat/c/{}/{}/{}",
                    self.slug.as_deref().unwrap_or("-"),
                    self.channel_id,
                    self.message_id
                ),
            },
            "channel_id": self.channel_id,
            "is_direct_message_channel": self.direct_message,
            "actions": [{
                "action": "chat-reply",
                "title": self.state.i18n.t("discourse_push_notifications.actions.chat_reply.title").unwrap_or_default(),
                "placeholder": self.state.i18n.t("discourse_push_notifications.actions.chat_reply.placeholder").unwrap_or_default(),
                "type": "text",
                "icon": format!("{}/assets/push-notifications/inline_reply.png", urls.base_url()?),
            }],
            "action_data": action_data(self.channel_id, self.message_id, self.thread_id),
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
        crate::jobs::post_alert::push_notification(conn, self.settings, membership.user_id).await?;
        Ok(())
    }
}

impl Job<'_> {
    /// `DirectMessageChannel#title(user)` for a member.
    async fn dm_title_for(
        &self,
        conn: &mut PgConnection,
        user_id: i32,
    ) -> Result<String, AppError> {
        dm_title_for(self.state, self.settings, conn, self.channel_id, user_id).await
    }
}

/// `DirectMessageChannel#title(user)` as text for `user_id`.
async fn dm_title_for(
    state: &AppState,
    settings: &SiteSettings,
    conn: &mut PgConnection,
    channel_id: i64,
    user_id: i32,
) -> Result<String, AppError> {
    let channel: super::channels::ChannelRow = sqlx::query_as(&format!(
        "SELECT {} FROM chat_channels WHERE id = $1",
        super::channels::CHANNEL_COLUMNS
    ))
    .bind(channel_id)
    .fetch_one(&mut *conn)
    .await?;
    if let Some(name) = channel.name.clone().filter(|n| !n.is_empty()) {
        let group: bool =
            sqlx::query_scalar("SELECT \"group\" FROM direct_message_channels WHERE id = $1")
                .bind(channel.chatable_id)
                .fetch_one(&mut *conn)
                .await?;
        if group {
            return Ok(name);
        }
    }
    let user = crate::session::current::SessionUser::load(&mut *conn, user_id)
        .await?
        .ok_or(Unsupported(
            "a direct message title for a user that is gone",
        ))?;
    let guardian = crate::guardian::Guardian::for_user(&mut *conn, &user).await?;
    let mut cx = super::channels::Context {
        conn: &mut *conn,
        settings,
        i18n: &state.i18n,
        guardian: &guardian,
        base_path: state.config.globals.relative_url_root(),
        config: &state.config,
    };
    let dm = cx.direct_message(channel.chatable_id).await?;
    Ok(match cx.dm_title(&channel, &dm)? {
        Value::String(s) => s,
        other => other.to_string(),
    })
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

/// `push_notification_reply_action`'s action_data: the thread's id only
/// when the message has one (`compact`).
fn action_data(channel_id: i64, message_id: i64, thread_id: Option<i64>) -> Value {
    let mut data = json!({ "channel_id": channel_id, "message_id": message_id });
    if let Some(thread_id) = thread_id {
        data["thread_id"] = json!(thread_id);
    }
    data
}

pub const NOTIFY_WATCHING_JOB: &str = "Jobs::Chat::NotifyWatching";

/// `Notification.types[:chat_watched_thread]`
const CHAT_WATCHED_THREAD: i32 = 40;

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
    let direct_message = channel.chatable_type == "DirectMessage";
    // Chat::Message#in_thread?: the thread (title, original message) when
    // it shows.
    let thread: Option<(Option<String>, i64, bool)> = match message.thread_id {
        Some(thread_id) => {
            sqlx::query_as(
                "SELECT title, original_message_id, force FROM chat_threads WHERE id = $1",
            )
            .bind(thread_id)
            .fetch_optional(&mut *conn)
            .await?
        }
        None => None,
    };
    let thread = thread.filter(|(_, _, force)| channel.threading_enabled || *force);
    let except: Vec<i32> = args["except_user_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_i64().map(|id| id as i32))
        .collect();
    // The members who want every message, and those watching the
    // message's thread (any thread, hidden or not).
    let members: Vec<Membership> = sqlx::query_as(
        "SELECT m.user_id, m.notification_level, m.muted, m.last_read_message_id \
         FROM user_chat_channel_memberships m \
         JOIN users ON users.id = m.user_id \
         JOIN user_options ON user_options.user_id = users.id AND user_options.chat_enabled \
         WHERE m.chat_channel_id = $1 AND m.following AND NOT (m.user_id = ANY($2)) \
           AND (m.notification_level = $3 OR users.id IN (SELECT user_id FROM user_chat_thread_memberships \
                  WHERE thread_id = $4 AND notification_level = $5)) \
           AND (users.suspended_till IS NULL OR users.suspended_till <= now()) \
         ORDER BY m.id",
    )
    .bind(channel.id)
    .bind(&except)
    .bind(NOTIFICATION_ALWAYS)
    .bind(message.thread_id)
    .bind(super::threads::WATCHING)
    .fetch_all(&mut *conn)
    .await?;
    if members.is_empty() {
        return Ok(());
    }
    // @creator, which a deleted author fails on with someone to notify.
    let creator_id = message
        .user_id
        .ok_or(Unsupported("notifying of a chat message of a deleted user"))?;
    let creator: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(creator_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(Unsupported("notifying of a chat message of a deleted user"))?;
    // CategoryChannel#title: its name, else its category's.
    let category_title = match channel.name.clone().filter(|n| !n.is_empty()) {
        Some(name) => name,
        None if direct_message => String::new(),
        None => {
            sqlx::query_scalar("SELECT name FROM categories WHERE id = $1")
                .bind(channel.chatable_id as i32)
                .fetch_one(&mut *conn)
                .await?
        }
    };
    // A DM's: those who take communication from the creator, in the
    // screener's order.
    let members: Vec<Membership> = if direct_message {
        let ids: Vec<i32> = members.iter().map(|m| m.user_id).collect();
        let staff: bool = sqlx::query_scalar(
            "SELECT COALESCE((SELECT admin OR moderator FROM users WHERE id = $1), FALSE)",
        )
        .bind(creator_id)
        .fetch_one(&mut *conn)
        .await?;
        let screener = crate::user_comm_screener::UserCommScreener::new(
            &mut *conn,
            crate::user_comm_screener::Actor {
                id: creator_id,
                staff,
            },
            &ids,
        )
        .await?;
        let mut by_id: Vec<Option<Membership>> = members.into_iter().map(Some).collect();
        let mut ordered = Vec::new();
        for id in screener.allowing_actor_communication() {
            if let Some(slot) = by_id
                .iter_mut()
                .find(|m| m.as_ref().is_some_and(|m| m.user_id == id))
                && let Some(m) = slot.take()
            {
                ordered.push(m);
            }
        }
        ordered
    } else {
        members
    };
    let group_dm: bool = if direct_message {
        sqlx::query_scalar("SELECT \"group\" FROM direct_message_channels WHERE id = $1")
            .bind(channel.chatable_id)
            .fetch_one(&mut *conn)
            .await?
    } else {
        false
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
        // A 1:1 DM's title names no channel.
        let translated_title = if direct_message && !group_dm {
            state
                .i18n
                .t_with(
                    "discourse_push_notifications.popup.new_direct_chat_message",
                    &[("username", creator.as_str())],
                )
                .unwrap_or_default()
        } else {
            let channel_title = if direct_message {
                dm_title_for(state, settings, conn, channel.id, membership.user_id).await?
            } else {
                category_title.clone()
            };
            state
                .i18n
                .t_with(
                    "discourse_push_notifications.popup.new_chat_message",
                    &[
                        ("username", creator.as_str()),
                        ("channel", channel_title.as_str()),
                    ],
                )
                .unwrap_or_default()
        };
        let payload = json!({
            "username": creator,
            "notification_type": CHAT_MESSAGE,
            "chat_message_id": message_id,
            // Chat::Message#url
            "post_url": match (&thread, message.thread_id) {
                (Some(_), Some(thread_id)) => {
                    format!("{base_path}/chat/c/-/{}/t/{thread_id}/{message_id}", channel.id)
                }
                _ => format!("{base_path}/chat/c/-/{}/{message_id}", channel.id),
            },
            "translated_title": translated_title,
            "tag": format!("{}-chat-message-{}", urls.current_hostname()?, channel.id),
            "excerpt": truncate(
                &settings
                    .emojis()
                    .gsub_to_unicode(message.message.as_deref().unwrap_or_default()),
                400
            ),
            "channel_id": channel.id,
            "is_direct_message_channel": direct_message,
            "actions": [{
                "action": "chat-reply",
                "title": state.i18n.t("discourse_push_notifications.actions.chat_reply.title").unwrap_or_default(),
                "placeholder": state.i18n.t("discourse_push_notifications.actions.chat_reply.placeholder").unwrap_or_default(),
                "type": "text",
                "icon": format!("{}/assets/push-notifications/inline_reply.png", urls.base_url()?),
            }],
            "action_data": action_data(channel.id, message_id, message.thread_id),
        });
        // create_watched_thread_notification, for a member watching the
        // thread the message shows in.
        if let (Some((title, original_message_id, _)), Some(thread_id)) =
            (&thread, message.thread_id)
        {
            let watching: Option<Option<i64>> = sqlx::query_scalar(
                "SELECT last_read_message_id FROM user_chat_thread_memberships \
                 WHERE user_id = $1 AND thread_id = $2 AND notification_level = $3",
            )
            .bind(i64::from(membership.user_id))
            .bind(thread_id)
            .bind(super::threads::WATCHING)
            .fetch_optional(&mut *conn)
            .await?;
            if let Some(last_read_message_id) = watching {
                let description = match title.as_deref().filter(|t| !t.trim().is_empty()) {
                    Some(title) => Value::String(title.to_string()),
                    None => json!(
                        sqlx::query_scalar::<_, Option<String>>(
                            "SELECT message FROM chat_messages WHERE id = $1"
                        )
                        .bind(original_message_id)
                        .fetch_optional(&mut *conn)
                        .await?
                        .flatten()
                    ),
                };
                let data = json!({
                    "username": creator,
                    "chat_message_id": message_id,
                    "chat_channel_id": channel.id,
                    "chat_thread_id": thread_id,
                    "last_read_message_id": last_read_message_id,
                    "description": description,
                    "user_ids": [creator_id],
                });
                consolidate_watched_thread(conn, &state.bus, settings, membership.user_id, data)
                    .await?;
            }
        }
        if membership.notification_level != NOTIFICATION_ALWAYS {
            continue;
        }
        state
            .bus
            .publish(
                &mut *conn,
                &format!("/chat/notification-alert/{}", membership.user_id),
                &payload,
                Some(&[crate::bus::user_tag(membership.user_id)]),
            )
            .await?;
        crate::jobs::post_alert::push_notification(conn, settings, membership.user_id).await?;
    }
    Ok(())
}

/// `Notification.consolidate_or_create!` with chat's
/// watched_thread_message_plan (Chat::NotificationConsolidationExtension:
/// chat_watched_thread into itself, threshold 1, by thread): the data
/// merged with the thread's last such notification (its message kept
/// while the member hasn't read past it, a second username and the user
/// ids gathered), then the consolidated notification updated, or the
/// unconsolidated ones replaced by one counting them, or a new one saved.
async fn consolidate_watched_thread(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    user_id: i32,
    data: Value,
) -> Result<(), AppError> {
    let Value::Object(mut data) = data else {
        return Ok(());
    };
    let thread_id = match &data["chat_thread_id"] {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let as_i = |v: Option<&Value>| match v {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
        Some(Value::String(s)) => crate::ruby::to_i(s),
        _ => 0,
    };
    // set_data_blk
    let last: Option<String> = sqlx::query_scalar(
        "SELECT data FROM notifications WHERE user_id = $1 AND data::json ->> 'chat_thread_id' = $2 \
           AND notification_type = $3 ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(&thread_id)
    .bind(CHAT_WATCHED_THREAD)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(last) = last {
        let consolidated: Map<String, Value> = serde_json::from_str(&last).unwrap_or_default();
        if as_i(data.get("last_read_message_id")) <= as_i(consolidated.get("chat_message_id")) {
            data.insert(
                "chat_message_id".into(),
                consolidated
                    .get("chat_message_id")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
        }
        let mut user_ids: Vec<Value> = consolidated
            .get("user_ids")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        user_ids.extend(
            data.get("user_ids")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        );
        let username2 = consolidated.get("username2").filter(|v| !v.is_null());
        if username2.is_none() && data.get("username") != consolidated.get("username") {
            data.insert(
                "username2".into(),
                consolidated.get("username").cloned().unwrap_or(Value::Null),
            );
            data.insert("user_ids".into(), Value::Array(user_ids));
        } else {
            let mut unique: Vec<Value> = Vec::new();
            for id in user_ids {
                if !unique.contains(&id) {
                    unique.push(id);
                }
            }
            data.insert(
                "username".into(),
                consolidated.get("username").cloned().unwrap_or(Value::Null),
            );
            data.insert(
                "username2".into(),
                consolidated
                    .get("username2")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            data.insert("user_ids".into(), Value::Array(unique));
        }
    }
    // update_consolidated_notification!
    let consolidated: Option<(i64, String)> = sqlx::query_as(
        "SELECT id, data FROM notifications WHERE user_id = $1 AND notification_type = $2 \
           AND (data::json ->> 'consolidated')::bool AND data::json ->> 'chat_thread_id' = $3 \
         ORDER BY id LIMIT 1",
    )
    .bind(user_id)
    .bind(CHAT_WATCHED_THREAD)
    .bind(&thread_id)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((id, existing)) = consolidated {
        let mut merged: Map<String, Value> = serde_json::from_str(&existing).unwrap_or_default();
        for (key, value) in &data {
            merged.insert(key.clone(), value.clone());
        }
        if let Some(count) = merged.get("count").filter(|c| !c.is_null()) {
            let next = count.as_i64().unwrap_or(0) + 1;
            merged.insert("count".into(), json!(next));
        }
        sqlx::query(
            "UPDATE notifications SET data = $2, read = FALSE, updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(id)
        .bind(Value::Object(merged).to_string())
        .execute(&mut *conn)
        .await?;
        crate::bus::publish_notifications_state(bus, &mut *conn, settings, user_id).await?;
        return Ok(());
    }
    // create_consolidated_notification!
    let unconsolidated: Vec<(i64, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT id, created_at FROM notifications WHERE user_id = $1 AND notification_type = $2 \
           AND data::json ->> 'consolidated' IS NULL AND data::json ->> 'chat_thread_id' = $3 \
         ORDER BY id",
    )
    .bind(user_id)
    .bind(CHAT_WATCHED_THREAD)
    .bind(&thread_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut n = crate::notification_create::NewNotification::new(
        CHAT_WATCHED_THREAD,
        user_id,
        Value::Object(data.clone()),
    );
    if let Some((_, timestamp)) = unconsolidated.last() {
        data.insert("count".into(), json!(unconsolidated.len() + 1));
        let ids: Vec<i64> = unconsolidated.iter().map(|(id, _)| *id).collect();
        sqlx::query("DELETE FROM notifications WHERE id = ANY($1)")
            .bind(&ids)
            .execute(&mut *conn)
            .await?;
        n.data = data;
        n.created_at = Some(*timestamp);
    }
    crate::notification_create::create(conn, bus, settings, &n).await?;
    Ok(())
}
