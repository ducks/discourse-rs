//! Chat::MessageBookmarkable, chat's registered bookmarkable: its list
//! query (the bookmarks of messages in channels the viewer may reach), its
//! search, what Chat::UserMessageBookmarkSerializer draws (the channel's
//! title, the message's excerpt, its URL and author), who may see one
//! (can_join_chat_channel?), and its reminder (the notification's title
//! and URL).
//!
//! Chat registers it through `register_bookmarkable`, a filtered register:
//! only while chat_enabled. core's bookmark list and reminder job reach it
//! through crate::bookmarks and crate::jobs::bookmark_reminders.

use sqlx::PgConnection;

use super::channels::{CHANNEL_COLUMNS, ChannelRow, Context};
use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `Chat::Message.polymorphic_name`, what bookmarks store as their type.
pub const POLYMORPHIC_NAME: &str = "ChatMessage";

/// `Bookmark.registered_bookmarkable_from_type` resolving to Chat::Message:
/// the polymorphic name (Chat::BookmarkExtension's class mapping) or the
/// class name.
pub fn is_type(bookmarkable_type: &str) -> bool {
    bookmarkable_type == POLYMORPHIC_NAME || bookmarkable_type == "Chat::Message"
}

/// Whether the bookmarkable is registered: chat's register is filtered by
/// the plugin being enabled.
pub fn registered(settings: &SiteSettings) -> Result<bool, AppError> {
    Ok(super::enabled(settings)?)
}

/// `Chat::ChannelFetcher.all_secured_channel_ids(guardian)` (following:
/// true): the channels the viewer has a membership of among those
/// generate_allowed_channel_ids_sql allows.
async fn all_secured_channel_ids(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
) -> Result<Vec<i64>, AppError> {
    let Some(user_id) = guardian.user_id() else {
        return Err(Unsupported("chat message bookmarks without a viewer").into());
    };
    let allowed = super::allowed_channel_ids_sql(settings, guardian, false)?;
    Ok(sqlx::query_scalar(&format!(
        "SELECT chat_channel_id FROM user_chat_channel_memberships \
         WHERE user_chat_channel_memberships.user_id = $1 \
           AND user_chat_channel_memberships.chat_channel_id IN ({allowed})"
    ))
    .bind(user_id)
    .fetch_all(conn)
    .await?)
}

/// `list_query(user, guardian)`, with `search_query` when `search`: the
/// owner's (`$1`) bookmarks of messages not trashed, in the channels the
/// viewer may reach; None when there are none. Searching matches the
/// message or the bookmark's name against `$3` (perform_search_query's
/// ILIKE pattern). Without a search `$2` and `$3` are still referenced, so
/// the union's binds keep their types.
pub async fn list_query(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    search: bool,
) -> Result<Option<String>, AppError> {
    let ids = all_secured_channel_ids(conn, settings, guardian).await?;
    if ids.is_empty() {
        return Ok(None);
    }
    let ids = ids
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let filter = if search {
        " AND (chat_messages.message ILIKE $3 OR bookmarks.name ILIKE $3)"
    } else {
        " AND $2::text IS NOT NULL AND $3::text IS NOT NULL"
    };
    Ok(Some(format!(
        "SELECT bookmarks.* FROM bookmarks \
           INNER JOIN chat_messages ON chat_messages.id = bookmarks.bookmarkable_id \
             AND chat_messages.deleted_at IS NULL AND bookmarks.bookmarkable_type = '{POLYMORPHIC_NAME}' \
         WHERE bookmarks.user_id = $1 AND bookmarks.bookmarkable_type = '{POLYMORPHIC_NAME}' \
           AND chat_messages.chat_channel_id IN ({ids}){filter}"
    )))
}

/// A bookmarked message as the bookmark's `bookmarkable` loads it (Chat::
/// Message's Trashable default scope: trashed messages are none), with its
/// channel (None when trashed) and author (None when gone).
#[derive(sqlx::FromRow)]
struct MessageRow {
    id: i64,
    chat_channel_id: i64,
    cooked: Option<String>,
    thread_id: Option<i64>,
    thread_force: Option<bool>,
    user_id: Option<i32>,
    username: Option<String>,
    user_name: Option<String>,
    uploaded_avatar_id: Option<i32>,
}

/// A loaded bookmarkable message.
pub struct Message {
    row: MessageRow,
    channel: Option<ChannelRow>,
}

/// The author, for BasicUserSerializer.
pub struct Author {
    pub id: i32,
    pub username: String,
    pub name: Option<String>,
    pub uploaded_avatar_id: Option<i32>,
}

/// What Chat::UserMessageBookmarkSerializer draws of the message.
pub struct Serialized {
    /// `title` and `fancy_title`: the channel's title for the viewer.
    pub title: String,
    /// The excerpt of the cooked message; None without one.
    pub excerpt: Option<String>,
    pub url: String,
    pub user: Option<Author>,
}

impl Context<'_> {
    /// The bookmark's message, when it is not trashed.
    pub async fn bookmarked_message(&mut self, id: i64) -> Result<Option<Message>, AppError> {
        let row: Option<MessageRow> = sqlx::query_as(
            "SELECT m.id, m.chat_channel_id, m.cooked, m.thread_id, t.force AS thread_force, \
                    u.id AS user_id, u.username, u.name AS user_name, u.uploaded_avatar_id \
             FROM chat_messages m \
             LEFT JOIN chat_threads t ON t.id = m.thread_id \
             LEFT JOIN users u ON u.id = m.user_id \
             WHERE m.id = $1 AND m.deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let channel: Option<ChannelRow> = sqlx::query_as(&format!(
            "SELECT {CHANNEL_COLUMNS} FROM chat_channels \
             WHERE chat_channels.id = $1 AND chat_channels.deleted_at IS NULL"
        ))
        .bind(row.chat_channel_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        Ok(Some(Message { row, channel }))
    }

    /// `Chat::Channel#title(user)` for the viewer: a category channel's
    /// name, else its category's; a direct message's participants.
    async fn channel_title(&mut self, channel: &ChannelRow) -> Result<String, AppError> {
        match channel.chatable_type.as_str() {
            "Category" => {
                if let Some(name) = channel
                    .name
                    .as_deref()
                    .filter(|n| !crate::ruby::is_blank(n))
                {
                    return Ok(name.to_string());
                }
                let name: Option<String> =
                    sqlx::query_scalar("SELECT name FROM categories WHERE id = $1")
                        .bind(channel.chatable_id)
                        .fetch_optional(&mut *self.conn)
                        .await?;
                Ok(name.ok_or(Unsupported("chat channels of deleted categories"))?)
            }
            "DirectMessage" => {
                let dm = self.direct_message(channel.chatable_id).await?;
                self.dm_title(channel, &dm)
            }
            _ => Err(Unsupported("chat channels of other chatables").into()),
        }
    }

    /// `Chat::Message#url`: in a thread's when the message is in one
    /// (in_thread?: threading on in its channel, or the thread forced).
    fn message_url(&self, m: &Message) -> String {
        let row = &m.row;
        let threading = m.channel.as_ref().is_some_and(|c| c.threading_enabled);
        match row.thread_id {
            Some(thread_id) if threading || row.thread_force == Some(true) => format!(
                "{}/chat/c/-/{}/t/{thread_id}/{}",
                self.base_path, row.chat_channel_id, row.id
            ),
            _ => format!(
                "{}/chat/c/-/{}/{}",
                self.base_path, row.chat_channel_id, row.id
            ),
        }
    }

    /// Chat::UserMessageBookmarkSerializer's values for the viewer.
    pub async fn serialize_bookmarked(&mut self, m: &Message) -> Result<Serialized, AppError> {
        // `chat_message.chat_channel.title`: a trashed channel is nil
        // there, which Rails fails on.
        let Some(channel) = m.channel.as_ref() else {
            return Err(Unsupported("bookmarks of chat messages in deleted channels").into());
        };
        let title = self.channel_title(channel).await?;
        let plugins = crate::excerpt::Plugins::from_settings(self.settings)?;
        let excerpt = m.row.cooked.as_deref().map(|cooked| {
            crate::excerpt::excerpt(
                cooked,
                300,
                &crate::excerpt::Options {
                    keep_emoji_images: true,
                    ..Default::default()
                },
                plugins,
            )
        });
        let user = match (m.row.user_id, m.row.username.clone()) {
            (Some(id), Some(username)) => Some(Author {
                id,
                username,
                name: m.row.user_name.clone(),
                uploaded_avatar_id: m.row.uploaded_avatar_id,
            }),
            _ => None,
        };
        Ok(Serialized {
            title,
            excerpt,
            url: self.message_url(m),
            user,
        })
    }

    /// `can_see_bookmarkable?`: can_join_chat_channel? on the message's
    /// channel (a trashed one is nil, which no one may join).
    pub async fn can_see_bookmarked(&mut self, m: &Message) -> Result<bool, AppError> {
        match m.channel.clone() {
            Some(channel) => self.can_join(&channel, None).await,
            None => Ok(false),
        }
    }

    /// `reminder_conditions` and `reminder_handler`'s data, for the
    /// bookmark's owner as the context's guardian: the notification's
    /// title and the message's URL, when the message and its channel are
    /// there and the owner may join the channel.
    pub async fn reminder(
        &mut self,
        message_id: i64,
    ) -> Result<Option<(String, String)>, AppError> {
        let Some(m) = self.bookmarked_message(message_id).await? else {
            return Ok(None);
        };
        let Some(channel) = m.channel.clone() else {
            return Ok(None);
        };
        if !self.can_see_bookmarked(&m).await? {
            return Ok(None);
        }
        // direct_message_channel? && !direct_message_group?
        let one_to_one = channel.chatable_type == "DirectMessage"
            && !self.direct_message(channel.chatable_id).await?.group;
        let kind = if one_to_one {
            "direct_message"
        } else {
            "channel"
        };
        let channel_name = self.channel_title(&channel).await?;
        let title = self
            .i18n
            .t_with(
                &format!("chat.bookmarkable.notification_title_{kind}"),
                &[("channel_name", &channel_name)],
            )
            .ok_or(Unsupported(
                "a chat bookmark reminder title without its translation",
            ))?;
        Ok(Some((title, self.message_url(&m))))
    }
}
