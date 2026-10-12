//! `PresenceChannel` (lib/presence_channel.rb): who is present in a named
//! channel right now (`/chat/online`, `/chat-reply/:channel_id`). Browsers
//! mark a client present, again at least every timeout while they stay,
//! or leave; a member is present while any of their clients is. Each time
//! a member enters or leaves, `/presence<name>` on the bus says so to
//! whoever may view the channel.
//!
//! Rails keeps the clients in Redis sorted sets driven by Lua scripts,
//! with a mutex to publish in order. Here they are rows in
//! `discourse_rs.presence_clients`, changed under a per-channel advisory
//! lock, with the bus message written in the same transaction, so
//! messages go out in the order the changes were made.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::AppError;
use crate::guardian::auto_groups;
use crate::site_settings::SiteSettings;
use crate::url::Urls;

/// `PresenceChannel::DEFAULT_TIMEOUT`: seconds a client stays present
/// after it last said so.
pub const DEFAULT_TIMEOUT: i64 = 60;

/// `PresenceChannel::Config`. No allowed users or groups and not public:
/// nobody may view or enter.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Config {
    pub public: bool,
    pub allowed_user_ids: Option<Vec<i32>>,
    pub allowed_group_ids: Option<Vec<i32>>,
    pub count_only: bool,
    pub timeout: Option<i64>,
}

impl Config {
    /// `can_view?(user_id:, group_ids:)`
    pub fn can_view(&self, user_id: Option<i32>, group_ids: &[i32]) -> bool {
        if self.public {
            return true;
        }
        let Some(user_id) = user_id else {
            return false;
        };
        if self
            .allowed_user_ids
            .as_ref()
            .is_some_and(|ids| ids.contains(&user_id))
        {
            return true;
        }
        if let Some(allowed) = self.allowed_group_ids.as_ref().filter(|g| !g.is_empty()) {
            // everyone and logged_in_users have no group_users rows; either
            // means any logged-in user.
            if allowed.contains(&(auto_groups::EVERYONE as i32))
                || allowed.contains(&(auto_groups::LOGGED_IN_USERS as i32))
            {
                return true;
            }
            return group_ids.iter().any(|g| allowed.contains(g));
        }
        false
    }

    /// The bus audience: none for a public channel, the allowed users' and
    /// groups' tags otherwise, and nothing published when nobody may view.
    fn audience(&self) -> Option<Option<Vec<String>>> {
        if self.public {
            return Some(None);
        }
        let users = self.allowed_user_ids.as_deref().unwrap_or_default();
        let groups = self.allowed_group_ids.as_deref().unwrap_or_default();
        if users.is_empty() && groups.is_empty() {
            return None;
        }
        let mut tags: Vec<String> = users.iter().map(|id| crate::bus::user_tag(*id)).collect();
        tags.extend(groups.iter().map(|id| crate::bus::group_tag(*id)));
        Some(Some(tags))
    }
}

/// What a channel's name resolves to: the registered prefix's config, or
/// None when no such channel exists (`PresenceChannel::NotFound`).
pub async fn config(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    name: &str,
) -> Result<Option<Config>, AppError> {
    // name[%r{/([a-zA-Z0-9_-]+)/.*}, 1]
    let Some(prefix) = prefix(name) else {
        return Ok(None);
    };
    match prefix {
        "chat" | "chat-reply" if crate::plugins::chat::enabled(settings)? => {
            crate::plugins::chat::presence::config(conn, settings, prefix, name).await
        }
        "discourse-presence" if crate::plugins::presence::enabled(settings)? => {
            crate::plugins::presence::config(conn, settings, name).await
        }
        _ => Ok(None),
    }
}

/// The first `/segment/` of a channel name, as Rails' prefix regexp finds it.
fn prefix(name: &str) -> Option<&str> {
    let bytes = name.as_bytes();
    let mut start = 0;
    while let Some(slash) = name[start..].find('/') {
        let from = start + slash + 1;
        let len = name[from..]
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
            .count();
        if len > 0 && bytes.get(from + len) == Some(&b'/') {
            return Some(&name[from..from + len]);
        }
        start = from;
    }
    None
}

/// `PresenceChannel::State`: the present members (or their count) and the
/// bus position the state was read at, from which its messages follow.
pub struct State {
    pub position: String,
    pub user_ids: Vec<i32>,
}

async fn lock(conn: &mut PgConnection, name: &str) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('presence:' || $1))")
        .bind(name)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn user_clients(
    conn: &mut PgConnection,
    name: &str,
    user_id: i32,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT count(*) FROM discourse_rs.presence_clients WHERE channel = $1 AND user_id = $2",
    )
    .bind(name)
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await
}

/// `present(user_id:, client_id:)`: the client marked present until the
/// timeout, the member announced when it is their first. Call within a
/// transaction; the caller has checked `can_enter?`.
pub async fn present(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    urls: &Urls<'_>,
    name: &str,
    config: &Config,
    user_id: i32,
    client_id: &str,
) -> Result<(), AppError> {
    lock(conn, name).await?;
    let before = user_clients(conn, name, user_id).await?;
    sqlx::query(
        "INSERT INTO discourse_rs.presence_clients (channel, user_id, client_id, expires_at) \
         VALUES ($1, $2, $3, now() + make_interval(secs => $4)) \
         ON CONFLICT (channel, user_id, client_id) DO UPDATE SET expires_at = EXCLUDED.expires_at",
    )
    .bind(name)
    .bind(user_id)
    .bind(client_id)
    .bind(config.timeout.unwrap_or(DEFAULT_TIMEOUT) as f64)
    .execute(&mut *conn)
    .await?;
    if before == 0 {
        publish(conn, bus, urls, name, config, &[user_id], &[]).await?;
    }
    Ok(())
}

/// `leave(user_id:, client_id:)`: the client gone at once, the member
/// announced when it was their last.
pub async fn leave(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    urls: &Urls<'_>,
    name: &str,
    config: &Config,
    user_id: i32,
    client_id: &str,
) -> Result<(), AppError> {
    lock(conn, name).await?;
    let removed = sqlx::query(
        "DELETE FROM discourse_rs.presence_clients WHERE channel = $1 AND user_id = $2 AND client_id = $3",
    )
    .bind(name)
    .bind(user_id)
    .bind(client_id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if removed > 0 && user_clients(conn, name, user_id).await? == 0 {
        publish(conn, bus, urls, name, config, &[], &[user_id]).await?;
    }
    Ok(())
}

/// `state`: the members with a client in the channel, expired ones too
/// until the auto-leave sweep takes them, read after the position so a
/// subscriber replaying from it misses nothing.
pub async fn state(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    name: &str,
) -> Result<State, AppError> {
    let position = bus.now().await?.to_string();
    let user_ids = sqlx::query_scalar(
        "SELECT user_id FROM discourse_rs.presence_clients WHERE channel = $1 \
         GROUP BY user_id ORDER BY min(entered_at), user_id",
    )
    .bind(name)
    .fetch_all(&mut *conn)
    .await?;
    Ok(State { position, user_ids })
}

/// `PresenceChannelStateSerializer`: the count, the position (Rails' last
/// MessageBus id) and, unless count only, the members.
pub async fn state_json(
    conn: &mut PgConnection,
    urls: &Urls<'_>,
    config: &Config,
    state: &State,
) -> Result<Value, AppError> {
    let mut out = Map::new();
    out.insert("count".into(), json!(state.user_ids.len()));
    out.insert("last_message_id".into(), json!(state.position));
    if !config.count_only {
        out.insert(
            "users".into(),
            json!(basic_users(conn, urls, &state.user_ids).await?),
        );
    }
    Ok(Value::Object(out))
}

/// `PresenceChannel.auto_leave_all` (Jobs::PresenceChannelAutoLeave): the
/// clients past their timeout go, the members who had no other announced
/// as leaving, a channel at a time.
pub async fn auto_leave_all(state: &crate::AppState) -> Result<(), AppError> {
    let pool = &state.pool;
    let bus = &state.bus;
    let channels: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT channel FROM discourse_rs.presence_clients WHERE expires_at <= now()",
    )
    .fetch_all(pool)
    .await?;
    for name in channels {
        let mut tx = pool.begin().await?;
        let settings =
            SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
        let urls = Urls {
            config: &state.config,
            settings: &settings,
        };
        // A channel that no longer exists still sheds its clients, with
        // nobody to tell (raise_not_found: false).
        let config = config(&mut tx, &settings, &name).await?.unwrap_or_default();
        lock(&mut tx, &name).await?;
        let expired: Vec<i32> = sqlx::query_scalar(
            "DELETE FROM discourse_rs.presence_clients WHERE channel = $1 AND expires_at <= now() \
             RETURNING user_id",
        )
        .bind(&name)
        .fetch_all(&mut *tx)
        .await?;
        let mut left = Vec::new();
        for user_id in expired {
            if !left.contains(&user_id) && user_clients(&mut tx, &name, user_id).await? == 0 {
                left.push(user_id);
            }
        }
        if !left.is_empty() {
            publish(&mut tx, bus, &urls, &name, &config, &[], &left).await?;
        }
        tx.commit().await?;
    }
    Ok(())
}

/// `publish_message`: the entering members (BasicUserSerializer) or the
/// leaving ids, or for a count-only channel the change in count, to the
/// channel's viewers.
async fn publish(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    urls: &Urls<'_>,
    name: &str,
    config: &Config,
    entering: &[i32],
    leaving: &[i32],
) -> Result<(), AppError> {
    let mut message = Map::new();
    if config.count_only {
        let delta = entering.len() as i64 - leaving.len() as i64;
        if delta == 0 {
            return Ok(());
        }
        message.insert("count_delta".into(), json!(delta));
    } else {
        if !leaving.is_empty() {
            message.insert("leaving_user_ids".into(), json!(leaving));
        }
        if !entering.is_empty() {
            message.insert(
                "entering_users".into(),
                json!(basic_users(conn, urls, entering).await?),
            );
        }
    }
    let Some(audience) = config.audience() else {
        return Ok(());
    };
    bus.publish(
        conn,
        &format!("/presence{name}"),
        &Value::Object(message),
        audience.as_deref(),
    )
    .await?;
    Ok(())
}

/// BasicUserSerializer for each user, by id.
async fn basic_users(
    conn: &mut PgConnection,
    urls: &Urls<'_>,
    ids: &[i32],
) -> Result<Vec<Value>, AppError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(i32, String, Option<String>, Option<i32>)> = sqlx::query_as(
        "SELECT id, username, name, uploaded_avatar_id FROM users WHERE id = ANY($1) ORDER BY id",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    let names = urls.settings.get("enable_names")?.truthy();
    let logo = crate::admin_users::logo_small_url(&mut *conn, urls.settings).await?;
    let mut out = Vec::new();
    for (id, username, name, avatar) in rows {
        let mut user = Map::new();
        user.insert("id".into(), json!(id));
        user.insert("username".into(), json!(username));
        if names {
            user.insert("name".into(), json!(name));
        }
        user.insert(
            "avatar_template".into(),
            json!(crate::avatar::avatar_template(
                urls,
                id,
                &username,
                avatar,
                logo.as_deref()
            )?),
        );
        out.push(Value::Object(user));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_are_found_as_rails_matches_them() {
        assert_eq!(prefix("/chat/online"), Some("chat"));
        assert_eq!(prefix("/chat-reply/2"), Some("chat-reply"));
        assert_eq!(prefix("/chat-reply/2/thread/3"), Some("chat-reply"));
        assert_eq!(prefix("/chat"), None);
        assert_eq!(prefix("chat/online"), None);
        assert_eq!(prefix("//x/y"), Some("x"));
    }

    #[test]
    fn viewers_follow_the_config() {
        let public = Config {
            public: true,
            ..Config::default()
        };
        assert!(public.can_view(None, &[]));
        let groups = Config {
            allowed_group_ids: Some(vec![10]),
            ..Config::default()
        };
        assert!(!groups.can_view(None, &[10]));
        assert!(groups.can_view(Some(3), &[10]));
        assert!(!groups.can_view(Some(3), &[11]));
        let everyone = Config {
            allowed_group_ids: Some(vec![0]),
            ..Config::default()
        };
        assert!(everyone.can_view(Some(3), &[]));
        let users = Config {
            allowed_user_ids: Some(vec![3]),
            ..Config::default()
        };
        assert!(users.can_view(Some(3), &[]) && !users.can_view(Some(4), &[]));
        assert!(!Config::default().can_view(Some(3), &[]));
        assert_eq!(Config::default().audience(), None);
        assert_eq!(public.audience(), Some(None));
        assert_eq!(groups.audience(), Some(Some(vec!["group:10".to_string()])));
    }
}
