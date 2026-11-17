//! The chat channel homepage (plugin.rb's `register_homepage("chat")`,
//! Chat::HomepageChannelSiteSetting): default_homepage "chat" lands people
//! on the channel chat_homepage_channel names, at `/`. Those who can't
//! access it, and crawlers, get the top menu's homepage instead.

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::AppError;
use crate::guardian::{Guardian, auto_groups};
use crate::plugin_api::Homepage;
use crate::site_settings::SiteSettings;

/// `register_homepage("chat", ...)`. The path is the channel's
/// ([`path`]); this one is the lambda's fallback.
pub const HOMEPAGE: Homepage = Homepage {
    id: "chat",
    name: "chat.homepage_option",
    path: "/chat",
    anonymous: true,
    server_side: false,
};

/// `Chat::Channel::ARCHIVED`
const ARCHIVED: i32 = 3;

/// The homepage channel: `Chat::Channel.find_by(id:
/// SiteSetting.chat_homepage_channel.presence)`, not trashed.
#[derive(sqlx::FromRow)]
struct Channel {
    id: i64,
    slug: Option<String>,
    status: i32,
    chatable_type: String,
    /// The category's read_restricted, None when the category is gone.
    read_restricted: Option<bool>,
    /// The chatable's id, a category's for a public channel.
    category_id: i64,
}

async fn channel(
    conn: &mut PgConnection,
    settings: &SiteSettings,
) -> Result<Option<Channel>, AppError> {
    let value = settings.get("chat_homepage_channel")?.to_s();
    let Ok(id) = value.trim().parse::<i64>() else {
        return Ok(None);
    };
    Ok(sqlx::query_as(
        "SELECT chat_channels.id, chat_channels.slug, chat_channels.status, \
                chat_channels.chatable_type, chat_channels.chatable_id AS category_id, \
                categories.read_restricted \
         FROM chat_channels \
         LEFT JOIN categories ON categories.id = chat_channels.chatable_id \
           AND chat_channels.chatable_type = 'Category' \
         WHERE chat_channels.id = $1 AND chat_channels.deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?)
}

/// The `path` lambda: the channel's relative_url without the subfolder,
/// or /chat without a channel.
pub async fn path(conn: &mut PgConnection, settings: &SiteSettings) -> Result<String, AppError> {
    Ok(match channel(conn, settings).await? {
        Some(c) => format!("/chat/c/{}/{}", c.slug.as_deref().unwrap_or("-"), c.id),
        None => HOMEPAGE.path.to_string(),
    })
}

/// The `enabled` lambda: offered while public channels are on.
pub fn offered(settings: &SiteSettings) -> Result<bool, AppError> {
    Ok(settings.get("enable_public_channels")?.truthy())
}

/// The `available` lambda: an open (not archived) public channel the
/// visitor may see, for a member who can chat and has it on, or for an
/// anonymous visitor while anonymous chat access is allowed. Never for a
/// crawler.
pub async fn available(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    crawler: bool,
) -> Result<bool, AppError> {
    let Some(channel) = channel(conn, settings).await? else {
        return Ok(false);
    };
    // archived? / public_channel?
    if channel.status == ARCHIVED || channel.chatable_type != "Category" {
        return Ok(false);
    }
    if crawler {
        return Ok(false);
    }
    // can_preview_chat_channel?: the chatable exists and is visible.
    let Some(read_restricted) = channel.read_restricted else {
        return Ok(false);
    };
    let Some(user_id) = guardian.user_id() else {
        // can_preview_anonymous_public_chat_channel?
        let allowed = settings.get("enable_public_channels")?.truthy()
            && settings
                .group_ids("chat_allowed_groups")?
                .contains(&auto_groups::ANONYMOUS_USERS);
        return Ok(allowed && !read_restricted);
    };
    if !super::can_chat(&mut *conn, settings, guardian).await? {
        return Ok(false);
    }
    let chat_enabled = super::options(&mut *conn, user_id)
        .await?
        .is_some_and(|o| o.chat_enabled);
    if !chat_enabled {
        return Ok(false);
    }
    if !read_restricted {
        return Ok(true);
    }
    let secure = guardian.secure_category_ids(&mut *conn, settings).await?;
    Ok(secure.contains(&(channel.category_id as i32)))
}

/// `Chat::HomepageChannelSiteSetting.values`: the public channels that
/// aren't archived, by id, named by their title.
pub async fn setting_values(conn: &mut PgConnection) -> Result<Value, AppError> {
    let rows: Vec<(i64, String)> = sqlx::query_as(&format!(
        "SELECT chat_channels.id, COALESCE(NULLIF(BTRIM(chat_channels.name), ''), categories.name) \
         {SELECTABLE} ORDER BY chat_channels.id"
    ))
    .fetch_all(&mut *conn)
    .await?;
    Ok(json!(
        rows.into_iter()
            .map(|(id, name)| json!({ "name": name, "value": id }))
            .collect::<Vec<_>>()
    ))
}

/// `Chat::HomepageChannelSiteSetting.valid_value?`: blank, or a selectable
/// channel's id.
pub async fn valid_setting_value(conn: &mut PgConnection, value: &str) -> Result<bool, AppError> {
    if crate::ruby::is_blank(value) {
        return Ok(true);
    }
    let Ok(id) = value.trim().parse::<i64>() else {
        return Ok(false);
    };
    Ok(sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 {SELECTABLE} AND chat_channels.id = $1)"
    ))
    .bind(id)
    .fetch_one(&mut *conn)
    .await?)
}

/// `selectable_channels`: Chat::Channel.public_channels (category
/// channels whose category exists, not trashed) not archived.
const SELECTABLE: &str = "FROM chat_channels \
     INNER JOIN categories ON categories.id = chat_channels.chatable_id \
     WHERE chat_channels.chatable_type = 'Category' AND chat_channels.deleted_at IS NULL \
       AND chat_channels.status <> 3";

/// `Chat::Channel#clear_homepage_anon_cache`: the anonymous site data
/// holds the homepage path, which follows the homepage channel's slug and
/// status.
pub async fn clear_anon_cache(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    channel_id: i64,
) -> Result<(), AppError> {
    if settings.get("chat_homepage_channel")?.to_s() == channel_id.to_string() {
        crate::bus::clear_anon_cache(bus, conn).await?;
    }
    Ok(())
}
