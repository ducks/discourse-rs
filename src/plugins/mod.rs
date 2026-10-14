//! Discourse's bundled plugins, ported as Rust in the binary (PLUGINS.md,
//! decision 6), each on when its enabled setting is. Third-party plugins
//! are a separate, sandboxed runtime (the Luau spike, branch
//! spike/plugin-runtime) over the same phases.
//!
//! A plugin changes what core computes only at the lifecycle phases of
//! core's pipelines (PLUGINS.md, "Rewriting core values"), never by
//! reaching into core's internals. The topic list's, with the serializer
//! (topic_list.rs) running them in order:
//!
//! - after load (`TopicListSerializer::prefetch`): what the plugins need
//!   for the page, users to load as well among it;
//! - before serialize: each topic's poster inputs (`PosterInputs`),
//!   rewritten before the posters are built from them;
//! - after serialize: the keys plugins add to each topic.
//!
//! Core's events (DiscourseEvent) reach the plugins listening through the
//! functions here, called where Rails triggers them.

pub mod chat;
pub mod poll;
pub mod presence;
pub mod reactions;
pub mod solved;
pub mod topic_voting;

use std::collections::HashMap;

use crate::site_settings::{SettingError, SiteSettings};

#[derive(Debug)]
pub enum PluginError {
    Db(sqlx::Error),
    Setting(SettingError),
    Unsupported(crate::Unsupported),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginError::Db(e) => write!(f, "plugin: {e}"),
            PluginError::Setting(e) => e.fmt(f),
            PluginError::Unsupported(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for PluginError {}

impl From<sqlx::Error> for PluginError {
    fn from(e: sqlx::Error) -> Self {
        PluginError::Db(e)
    }
}

impl From<SettingError> for PluginError {
    fn from(e: SettingError) -> Self {
        PluginError::Setting(e)
    }
}

impl From<crate::avatar::AvatarError> for PluginError {
    fn from(e: crate::avatar::AvatarError) -> Self {
        use crate::avatar::AvatarError;
        use crate::url::UrlError;
        match e {
            AvatarError::Setting(e) | AvatarError::Url(UrlError::Setting(e)) => {
                PluginError::Setting(e)
            }
            AvatarError::Url(UrlError::S3Unsupported) => {
                PluginError::Unsupported(crate::Unsupported("S3 upload stores"))
            }
            AvatarError::Unsupported(e) => PluginError::Unsupported(e),
        }
    }
}

impl From<crate::guardian::GuardianError> for PluginError {
    fn from(e: crate::guardian::GuardianError) -> Self {
        match e {
            crate::guardian::GuardianError::Db(e) => PluginError::Db(e),
            crate::guardian::GuardianError::Setting(e) => PluginError::Setting(e),
            crate::guardian::GuardianError::Unsupported(e) => PluginError::Unsupported(e),
        }
    }
}

/// A topic's poster inputs (TopicPostersSummary's `user_ids`,
/// `descriptions_by_id` and `last_poster_is_topic_creator?`), the values
/// plugins rewrite before core builds the posters.
#[derive(Debug, Clone, PartialEq)]
pub struct PosterInputs {
    pub user_ids: Vec<Option<i32>>,
    pub descriptions: HashMap<i32, Vec<String>>,
    pub last_poster_stays: bool,
}

/// The topic list's after-load data, every plugin's, for one page.
#[derive(Debug, Default)]
pub struct TopicListData {
    pub solved: Option<solved::TopicListData>,
    pub topic_voting: Option<topic_voting::TopicListData>,
}

impl TopicListData {
    /// Users the plugins ask to be loaded with the page
    /// (`topic_list_preload_user_ids`).
    pub fn user_ids(&self) -> Vec<i32> {
        let mut ids = Vec::new();
        if let Some(solved) = &self.solved {
            ids.extend(solved.answerers.values().flatten());
        }
        ids
    }
}

/// After load: each enabled plugin's data for the page.
pub async fn topic_list_after_load(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    topic_ids: &[i32],
    user_id: Option<i32>,
) -> Result<TopicListData, PluginError> {
    let mut data = TopicListData::default();
    if !topic_ids.is_empty() && solved::enabled(settings)? {
        data.solved = Some(solved::TopicListData::load(conn, topic_ids).await?);
    }
    if !topic_ids.is_empty() && topic_voting::enabled(settings)? {
        data.topic_voting =
            Some(topic_voting::TopicListData::load(conn, topic_ids, user_id).await?);
    }
    Ok(data)
}

/// Before serialize: a topic's poster inputs, rewritten by each plugin.
pub fn topic_list_before_serialize(
    data: &TopicListData,
    i18n: &crate::i18n::I18n,
    topic_id: i32,
    last_post_user_id: i32,
    inputs: &mut PosterInputs,
) {
    if let Some(solved) = &data.solved {
        solved.rewrite_posters(i18n, topic_id, last_post_user_id, inputs);
    }
}

/// DiscourseEvent `:topic_status_updated` (TopicStatusUpdater, when the
/// status changed), to the enabled plugins listening.
pub async fn topic_status_updated(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    topic_id: i32,
    status: &str,
    enabled: bool,
) -> Result<(), PluginError> {
    if topic_voting::enabled(settings)? {
        topic_voting::lifecycle::topic_status_updated(conn, topic_id, status, enabled).await?;
    }
    Ok(())
}

/// DiscourseEvent `:topic_trashed` (Topic#trash! on a topic not yet
/// trashed).
pub async fn topic_trashed(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    topic_id: i32,
    closed: bool,
    archived: bool,
) -> Result<(), PluginError> {
    if topic_voting::enabled(settings)? {
        topic_voting::lifecycle::topic_trashed(conn, topic_id, closed, archived).await?;
    }
    Ok(())
}

/// UserDestroyer, before the user row goes: the rows the plugins'
/// associations on User take with it (`has_many ..., dependent:`). Rails
/// declares those when a plugin loads, so they apply whether or not the
/// plugin is enabled. Refused for the unported plugins that hold rows of
/// the user (their user_destroyed handlers and associations).
pub async fn user_destroying(
    conn: &mut sqlx::PgConnection,
    user_id: i32,
) -> Result<(), PluginError> {
    // (table, column, plugin)
    const UNPORTED: [(&str, &str, &str); 9] = [
        ("ai_api_audit_logs", "user_id", "discourse-ai"),
        ("ai_agents", "user_id", "discourse-ai"),
        (
            "gamification_leaderboard_scores",
            "user_id",
            "discourse-gamification",
        ),
        (
            "discourse_post_event_invitees",
            "user_id",
            "discourse-events",
        ),
        ("discourse_post_event_hosts", "user_id", "discourse-events"),
        ("voice_rooms", "creator_id", "voice"),
        ("voice_room_memberships", "user_id", "voice"),
        ("voice_co_presences", "user_id_1", "voice"),
        ("voice_co_presences", "user_id_2", "voice"),
    ];
    for (table, column, _) in UNPORTED {
        let any: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS (SELECT 1 FROM {table} WHERE {column} = $1)"
        ))
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        if any {
            return Err(PluginError::Unsupported(crate::Unsupported(
                "deleting a user with data of unported plugins (discourse-ai, gamification, events, voice)",
            )));
        }
    }
    chat::membership::user_destroying(conn, user_id).await?;
    topic_voting::lifecycle::user_destroying(conn, user_id).await?;
    poll::user_destroying(conn, user_id).await?;
    solved::shared_issue::user_destroying(conn, user_id).await?;
    Ok(())
}

/// UserDestroyer with `delete_posts`: the plugins'
/// `user_destroyer_on_content_deletion_callbacks`. Chat's deletes the
/// user's messages in a job; post-voting's and discourse-ai's are refused
/// when they would act.
pub async fn user_content_deleting(
    conn: &mut sqlx::PgConnection,
    user_id: i32,
) -> Result<(), PluginError> {
    let comments: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM post_voting_comments WHERE user_id = $1)")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    let ai_logs: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM ai_api_audit_logs l WHERE \
           l.post_id IN (SELECT id FROM posts WHERE user_id = $1) OR \
           l.topic_id IN (SELECT id FROM topics WHERE user_id = $1))",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if comments || ai_logs {
        return Err(PluginError::Unsupported(crate::Unsupported(
            "deleting a user's post-voting comments or discourse-ai logs",
        )));
    }
    crate::jobs::enqueue(
        conn,
        chat::membership::DELETE_USER_MESSAGES_JOB,
        serde_json::json!({ "user_id": user_id }),
    )
    .await?;
    Ok(())
}

/// `WebHook.active_web_hooks(event).exists?`: an active hook for the
/// event, or a wildcard one.
pub(crate) async fn web_hooks_active(
    conn: &mut sqlx::PgConnection,
    event: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM web_hooks w \
         LEFT JOIN web_hook_event_types_hooks h ON h.web_hook_id = w.id \
         LEFT JOIN web_hook_event_types t ON t.id = h.web_hook_event_type_id \
         WHERE w.active AND (w.wildcard_web_hook OR t.name = $1))",
    )
    .bind(event)
    .fetch_one(&mut *conn)
    .await
}
