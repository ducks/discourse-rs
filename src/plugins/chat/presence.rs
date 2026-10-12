//! Chat's presence channels (plugin.rb's register_presence_channel_prefix):
//! `/chat/online` for everyone who may chat, `/chat-reply/:channel_id`
//! (and `/thread/:thread_id`) for who is typing, as the channel allows.

use sqlx::PgConnection;

use crate::AppError;
use crate::presence::Config;
use crate::site_settings::SiteSettings;

/// `Chat::CategoryChannel::STAFF_GROUP_IDS`: admins, moderators, staff.
const STAFF_GROUP_IDS: [i32; 3] = [1, 2, 3];

/// The config for a `chat` or `chat-reply` channel name, None for a name
/// neither knows.
pub async fn config(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    prefix: &str,
    name: &str,
) -> Result<Option<Config>, AppError> {
    if prefix == "chat" {
        if name != "/chat/online" {
            return Ok(None);
        }
        // Chat.allowed_group_ids
        let groups = settings
            .group_ids("chat_allowed_groups")?
            .into_iter()
            .map(|id| id as i32)
            .collect();
        return Ok(Some(Config {
            allowed_group_ids: Some(groups),
            ..Config::default()
        }));
    }
    let Some((channel_id, thread_id)) = reply_ids(name) else {
        return Ok(None);
    };
    // Chat::Thread.find_by(id:, channel_id:)&.channel, else the channel;
    // trashed channels are out of the default scope.
    let channel: Option<(i64, String, i64)> = match thread_id {
        Some(thread_id) => {
            sqlx::query_as(
                "SELECT c.id, c.chatable_type, c.chatable_id FROM chat_threads t \
                 JOIN chat_channels c ON c.id = t.channel_id AND c.deleted_at IS NULL \
                 WHERE t.id = $1 AND t.channel_id = $2",
            )
            .bind(thread_id)
            .bind(channel_id)
            .fetch_optional(&mut *conn)
            .await?
        }
        None => {
            sqlx::query_as(
                "SELECT id, chatable_type, chatable_id FROM chat_channels WHERE id = $1 AND deleted_at IS NULL",
            )
            .bind(channel_id)
            .fetch_optional(&mut *conn)
            .await?
        }
    };
    let Some((_, chatable_type, chatable_id)) = channel else {
        return Ok(None);
    };
    if chatable_type == "DirectMessage" {
        // DirectMessageChannel: its members, always read restricted.
        let users: Vec<i32> = sqlx::query_scalar(
            "SELECT user_id FROM direct_message_users WHERE direct_message_channel_id = $1 ORDER BY id",
        )
        .bind(chatable_id)
        .fetch_all(&mut *conn)
        .await?;
        return Ok(Some(Config {
            allowed_user_ids: Some(users),
            ..Config::default()
        }));
    }
    // CategoryChannel: public unless its category is read restricted, then
    // staff and the category's groups.
    let restricted: Option<bool> =
        sqlx::query_scalar("SELECT read_restricted FROM categories WHERE id = $1")
            .bind(chatable_id)
            .fetch_optional(&mut *conn)
            .await?;
    if restricted != Some(true) {
        return Ok(Some(Config {
            public: true,
            ..Config::default()
        }));
    }
    let mut groups = STAFF_GROUP_IDS.to_vec();
    let secure: Vec<i32> = sqlx::query_scalar(
        "SELECT group_id FROM category_groups WHERE category_id = $1 ORDER BY id",
    )
    .bind(chatable_id)
    .fetch_all(&mut *conn)
    .await?;
    for id in secure {
        if !groups.contains(&id) {
            groups.push(id);
        }
    }
    Ok(Some(Config {
        allowed_group_ids: Some(groups),
        ..Config::default()
    }))
}

/// `Chat::PRESENCE_REGEXP`: `^/chat-reply/(\d+)(?:/thread/(\d+))?$`.
fn reply_ids(name: &str) -> Option<(i64, Option<i64>)> {
    let rest = name.strip_prefix("/chat-reply/")?;
    let digits = |s: &str| {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    match rest.split_once("/thread/") {
        Some((channel, thread)) => Some((digits(channel)?, Some(digits(thread)?))),
        None => Some((digits(rest)?, None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_channels_name_a_channel_or_thread() {
        assert_eq!(reply_ids("/chat-reply/2"), Some((2, None)));
        assert_eq!(reply_ids("/chat-reply/2/thread/7"), Some((2, Some(7))));
        assert_eq!(reply_ids("/chat-reply/x"), None);
        assert_eq!(reply_ids("/chat-reply/2/"), None);
        assert_eq!(reply_ids("/chat-reply/2/thread/"), None);
    }
}
