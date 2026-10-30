//! `Chat::CreateDirectMessageChannel` (POST
//! /chat/api/direct-message-channels): the direct message between the
//! actor and the target users and groups' members, found or created, its
//! channel, and the members' memberships.

use serde_json::{Map, Value, json};

use super::channels::{CHANNEL_COLUMNS, ChannelRow, Context};
use crate::AppError;
use crate::user_comm_screener::{Actor, UserCommScreener};

/// How the service ends, as Chat::Api::DirectMessagesController renders
/// it.
pub enum Outcome {
    /// `{channel: ChannelSerializer}`
    Created(Value),
    /// The actor is not among the target users (chat disabled):
    /// ActiveRecord::RecordNotFound.
    NotFound,
    /// A 400 `{error: ...}`.
    Refused(String),
    /// render_json_error: a 422 `{errors: [...]}`.
    Unprocessable(String),
    /// The channel's ActiveRecord::RecordInvalid, raised by
    /// set_optional_params after the direct message and its channel were
    /// saved (the service has no transaction): a 422 record_invalid, the
    /// rows kept.
    RecordInvalid(Vec<String>),
}

/// The service's params.
pub struct Params {
    pub name: Option<String>,
    pub target_usernames: Vec<String>,
    pub target_groups: Vec<String>,
    pub upsert: bool,
}

/// `ActiveSupportTypeExtensions::Array#cast_value` for a param: a string
/// splits on commas (or pipes), an array stays, each item an integer
/// when it reads as one (which the queries turn back into text).
pub fn cast_array(value: Option<&Value>) -> Vec<String> {
    let items: Vec<String> = match value {
        None | Some(Value::Null) => return Vec::new(),
        Some(Value::String(s)) => split_list(s),
        Some(Value::Array(items)) => items.iter().filter_map(crate::params::scalar).collect(),
        Some(Value::Object(map)) => map.values().filter_map(crate::params::scalar).collect(),
        Some(other) => crate::params::scalar(other).into_iter().collect(),
    };
    items
        .into_iter()
        .map(|item| match item.trim().parse::<i64>() {
            Ok(n) if item.trim() == item => n.to_string(),
            _ => item,
        })
        .collect()
}

/// `value.split(/,(?!.*\|)|\|(?!.*,)/)`: on commas with no pipe after
/// them, and on pipes with no comma after them; trailing empty pieces
/// dropped, as Ruby's split does.
fn split_list(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in s.char_indices() {
        let rest = &s[i + c.len_utf8()..];
        let split = (c == ',' && !rest.contains('|')) || (c == '|' && !rest.contains(','));
        if split {
            out.push(s[start..i].to_string());
            start = i + c.len_utf8();
        }
    }
    out.push(s[start..].to_string());
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

/// A target user as the policies read one.
#[derive(sqlx::FromRow)]
struct Target {
    id: i32,
    username: String,
}

impl Context<'_> {
    /// `Chat::CreateDirectMessageChannel.call`
    pub async fn create_direct_message_channel(
        &mut self,
        params: &Params,
    ) -> Result<Outcome, AppError> {
        let s = self.settings;
        let g = self.guardian;
        let Some(actor) = g.user() else {
            return Ok(Outcome::NotFound);
        };
        let t = |key: &str, args: &[(&str, &str)]| {
            self.i18n
                .t_with(key, args)
                .unwrap_or_else(|| key.to_string())
        };

        // fetch_target_users: the visible groups whose members are
        // visible, then the users with chat enabled.
        let groups: Vec<String> = if params.target_groups.is_empty() {
            Vec::new()
        } else {
            sqlx::query_scalar(&format!(
                "SELECT groups.name FROM groups WHERE groups.name = ANY($1) AND {} AND {} ORDER BY groups.id",
                crate::groups::visible_groups_where(g, "groups"),
                crate::groups::members_visible_groups_where(g, "groups"),
            ))
            .bind(&params.target_groups)
            .fetch_all(&mut *self.conn)
            .await?
        };
        let mut usernames: Vec<String> = Vec::new();
        for username in params
            .target_usernames
            .iter()
            .chain(std::iter::once(&actor.username))
        {
            usernames.push(crate::accounts::normalize_username(username)?);
        }
        let targets: Vec<Target> = sqlx::query_as(
            "SELECT DISTINCT users.id, users.username FROM users \
             INNER JOIN user_options ON user_options.user_id = users.id \
             LEFT OUTER JOIN group_users ON group_users.user_id = users.id \
             LEFT OUTER JOIN groups ON groups.id = group_users.group_id \
             WHERE user_options.chat_enabled = TRUE \
               AND (users.username_lower = ANY($1) OR (groups.name = ANY($2) AND group_users.user_id IS NOT NULL)) \
             ORDER BY users.id",
        )
        .bind(&usernames)
        .bind(&groups)
        .fetch_all(&mut *self.conn)
        .await?;
        if !targets.iter().any(|u| u.id == actor.id) {
            return Ok(Outcome::NotFound);
        }
        let others: Vec<&Target> = targets.iter().filter(|u| u.id != actor.id).collect();

        // can_create_direct_message: staff or the DM groups.
        if !(g.is_staff() || super::can_direct_message(s, g)?) {
            return Ok(Outcome::Refused(t(
                "chat.errors.invalid_direct_message",
                &[],
            )));
        }
        // MaxUsersExcess
        let max = s.get("chat_max_direct_message_users")?.to_i();
        if !g.is_staff() && others.len() as i64 > max {
            return Ok(Outcome::Refused(if max == 0 {
                t(
                    "chat.errors.over_chat_max_direct_message_users_allow_self",
                    &[],
                )
            } else {
                let key = if max == 1 { "one" } else { "other" };
                t(
                    &format!("chat.errors.over_chat_max_direct_message_users.{key}"),
                    &[("count", &max.to_string())],
                )
            }));
        }
        let ids: Vec<i32> = targets.iter().map(|u| u.id).collect();
        let screener = UserCommScreener::new(
            &mut *self.conn,
            Actor {
                id: actor.id,
                staff: g.is_staff(),
            },
            &ids,
        )
        .await?;
        // actor_allows_dms
        if screener.actor_disallowing_all_pms() {
            return Ok(Outcome::Unprocessable(t(
                "chat.errors.actor_disallowed_dms",
                &[],
            )));
        }
        // CanCommunicateAllParties
        let preventing = screener.preventing_actor_communication();
        if let Some(first) = preventing.first() {
            let username = others
                .iter()
                .find(|u| u.id == *first)
                .map(|u| u.username.clone())
                .unwrap_or_default();
            return Ok(Outcome::Unprocessable(t(
                "chat.errors.not_accepting_dms",
                &[("username", &username)],
            )));
        }
        for (check, key) in [
            (0, "chat.errors.actor_preventing_target_user_from_dm"),
            (1, "chat.errors.actor_ignoring_target_user"),
            (2, "chat.errors.actor_muting_target_user"),
        ] {
            let mut found = None;
            for user in &others {
                let hit = match check {
                    0 => screener.actor_disallowing_pms(user.id)?,
                    1 => screener.actor_ignoring(user.id)?,
                    _ => screener.actor_muting(user.id)?,
                };
                if hit {
                    found = Some(user.username.clone());
                    break;
                }
            }
            if let Some(username) = found {
                return Ok(Outcome::Unprocessable(t(key, &[("username", &username)])));
            }
        }

        // fetch_or_create_direct_message
        let name = params.name.clone().filter(|n| !n.is_empty());
        let is_group = ids.len() > 2 || name.is_some();
        let existing: Option<i64> = if params.upsert || !is_group {
            sqlx::query_scalar(
                "SELECT direct_message_channels.id FROM direct_message_channels \
                 INNER JOIN direct_message_users ON direct_message_users.direct_message_channel_id = direct_message_channels.id \
                 INNER JOIN users ON users.id = direct_message_users.user_id \
                 WHERE direct_message_channels.\"group\" = $2 \
                 GROUP BY direct_message_channels.id \
                 HAVING $1::int[] = ARRAY_AGG(users.id ORDER BY users.id) \
                 ORDER BY direct_message_channels.id LIMIT 1",
            )
            .bind(&ids)
            .bind(is_group)
            .fetch_optional(&mut *self.conn)
            .await?
        } else {
            None
        };
        let dm_id = match existing {
            Some(id) => id,
            None => {
                let now = crate::clock::now_naive();
                let id: i64 = sqlx::query_scalar(
                    "INSERT INTO direct_message_channels (\"group\", created_at, updated_at) \
                     VALUES ($1, $2, $2) RETURNING id",
                )
                .bind(is_group)
                .bind(now)
                .fetch_one(&mut *self.conn)
                .await?;
                for user_id in &ids {
                    sqlx::query(
                        "INSERT INTO direct_message_users (direct_message_channel_id, user_id, created_at, updated_at) \
                         VALUES ($1, $2, $3, $3)",
                    )
                    .bind(id)
                    .bind(user_id)
                    .bind(crate::clock::now_naive())
                    .execute(&mut *self.conn)
                    .await?;
                }
                id
            }
        };
        // DirectMessageChannel.find_or_create_by(chatable:)
        let existing_channel: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM chat_channels WHERE chatable_type = 'DirectMessage' AND chatable_id = $1 \
               AND type = 'DirectMessageChannel' AND deleted_at IS NULL ORDER BY id LIMIT 1",
        )
        .bind(dm_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let channel_id = match existing_channel {
            Some(id) => id,
            None => {
                sqlx::query_scalar(
                    "INSERT INTO chat_channels (chatable_id, chatable_type, type, created_at, updated_at) \
                     VALUES ($1, 'DirectMessage', 'DirectMessageChannel', $2, $2) RETURNING id",
                )
                .bind(dm_id)
                .bind(crate::clock::now_naive())
                .fetch_one(&mut *self.conn)
                .await?
            }
        };
        // set_optional_params
        if let Some(name) = &name {
            let max_title = s.get("max_topic_title_length")?.to_i();
            if name.trim().is_empty() {
                return Ok(Outcome::RecordInvalid(vec!["Name can't be blank".into()]));
            }
            if name.chars().count() as i64 > max_title {
                return Ok(Outcome::RecordInvalid(vec![format!(
                    "Name is too long (maximum is {max_title} characters)"
                )]));
            }
            sqlx::query(
                "UPDATE chat_channels SET name = $2, updated_at = $3 WHERE id = $1 AND name IS DISTINCT FROM $2",
            )
            .bind(channel_id)
            .bind(name)
            .bind(crate::clock::now_naive())
            .execute(&mut *self.conn)
            .await?;
        }
        // create_memberships: insert_all, unique by user and channel.
        let now = crate::clock::now_naive();
        for user_id in &ids {
            sqlx::query(
                "INSERT INTO user_chat_channel_memberships \
                   (user_id, chat_channel_id, muted, following, notification_level, created_at, updated_at) \
                 VALUES ($1, $2, FALSE, $3, 2, $4, $4) ON CONFLICT (user_id, chat_channel_id) DO NOTHING",
            )
            .bind(user_id)
            .bind(channel_id)
            .bind(*user_id == actor.id)
            .bind(now)
            .execute(&mut *self.conn)
            .await?;
        }
        // recompute_users_count: ChannelMembershipsQuery.count (every
        // member of a DM).
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM user_chat_channel_memberships m JOIN users ON users.id = m.user_id \
             WHERE m.chat_channel_id = $1 AND users.id > 0 \
               AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = users.id) \
               AND users.active AND NOT users.staged \
               AND (users.suspended_till IS NULL OR users.suspended_till <= $2) \
               AND (users.silenced_till IS NULL OR users.silenced_till <= $2)",
        )
        .bind(channel_id)
        .bind(crate::clock::now_naive())
        .fetch_one(&mut *self.conn)
        .await?;
        sqlx::query(
            "UPDATE chat_channels SET user_count = $2, user_count_stale = FALSE, updated_at = $3 \
             WHERE id = $1 AND (user_count <> $2 OR user_count_stale)",
        )
        .bind(channel_id)
        .bind(count as i32)
        .bind(crate::clock::now_naive())
        .execute(&mut *self.conn)
        .await?;

        let channel: ChannelRow = sqlx::query_as(&format!(
            "SELECT {CHANNEL_COLUMNS} FROM chat_channels WHERE chat_channels.id = $1"
        ))
        .bind(channel_id)
        .fetch_one(&mut *self.conn)
        .await?;
        let body = self.channel(&channel, None, None, None).await?;
        let mut out = Map::new();
        out.insert("channel".into(), body);
        Ok(Outcome::Created(json!(out)))
    }
}

#[cfg(test)]
mod tests {
    use super::split_list;

    #[test]
    fn strings_split_as_the_array_type_does() {
        assert_eq!(split_list("a,b"), vec!["a", "b"]);
        assert_eq!(split_list("a|b"), vec!["a", "b"]);
        assert_eq!(split_list("a"), vec!["a"]);
        assert_eq!(split_list("a,b,"), vec!["a", "b"]);
        assert!(split_list("").is_empty());
    }
}
