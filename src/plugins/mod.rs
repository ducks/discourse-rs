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

pub mod apple;
pub mod auth_providers;
pub mod cakeday;
pub mod chat;
pub mod checklist;
pub mod footnote;
pub mod lazy_videos;
pub mod local_dates;
pub mod lti;
pub mod oauth2_basic;
pub mod openid_connect;
pub mod poll;
pub mod presence;
pub mod reactions;
pub mod solved;
pub mod topic_voting;
pub mod user_notes;

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
/// Serialized for third-party plugins (src/plugin_wasm.rs) as their
/// before serialize payload.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
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

/// After load: each enabled plugin's data for the page. `replaced` are
/// the bundled plugins a third-party one (src/plugin_wasm.rs) took over.
pub async fn topic_list_after_load(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    topic_ids: &[i32],
    user_id: Option<i32>,
    replaced: &[String],
) -> Result<TopicListData, PluginError> {
    let mut data = TopicListData::default();
    let solved = solved::enabled(settings)? && !replaced.iter().any(|p| p == "discourse-solved");
    if !topic_ids.is_empty() && solved {
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

/// DiscourseEvent `:post_edited` from a revision whose topic diff has
/// `category_id`.
pub async fn topic_category_revised(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    topic_id: i32,
) -> Result<(), PluginError> {
    if topic_voting::enabled(settings)? {
        topic_voting::lifecycle::topic_category_changed(conn, topic_id).await?;
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

/// UserWarning's after_commit on create (the plugins' model callbacks):
/// discourse-user-notes' note of the warning. Nothing in the port creates
/// official warnings yet (src/flags.rs refuses them); this is where that
/// write calls once it does.
pub async fn user_warning_created(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    urls: &crate::url::Urls<'_>,
    i18n: &crate::i18n::I18n,
    warning: &user_notes::UserWarning,
) -> Result<(), crate::AppError> {
    user_notes::user_warning_created(conn, settings, urls, i18n, warning).await
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

/// The bundled plugins (their plugin.rb names) the port has started, in
/// src/plugins or the markdown crate. Started is not done: a plugin is
/// done only when it has all of the reference plugin's features (ROADMAP
/// item 210). Enabling a plugin not listed here is refused
/// (crate::site_setting_update), unless the reference has it on by
/// default.
pub const STARTED: &[&str] = &[
    "chat",
    "checklist",
    "discourse-apple-auth",
    "discourse-cakeday",
    "discourse-details",
    "discourse-lazy-videos",
    "discourse-local-dates",
    "discourse-lti",
    "discourse-oauth2-basic",
    "discourse-openid-connect",
    "discourse-presence",
    "discourse-reactions",
    "discourse-solved",
    "discourse-topic-voting",
    "discourse-user-notes",
    "footnote",
    "poll",
    "spoiler-alert",
];

/// DiscourseEvent `:site_setting_changed`, after a setting's new value is
/// saved: the handlers of the plugins the port implements, each while
/// its plugin is enabled (`plugin.on`). Of these, only chat's retention
/// reminders handle a type the settings update takes (integers); its
/// chat_allowed_groups job and discourse-reactions' like sync are on
/// list settings, which the update refuses.
pub async fn site_setting_changed(
    conn: &mut sqlx::PgConnection,
    defs: &crate::site_settings::Definitions,
    settings: &SiteSettings,
    name: &str,
    old: &crate::site_settings::Value,
    new: &crate::site_settings::Value,
) -> Result<(), crate::AppError> {
    if defs.plugin_enabled("chat", settings) {
        // Chat::RETENTION_SETTINGS_TO_USER_OPTION_FIELDS: a new non-zero
        // retention re-shows the dismissed retention reminders.
        let field = match name {
            "chat_channel_retention_days" => Some("dismissed_channel_retention_reminder"),
            "chat_dm_retention_days" => Some("dismissed_dm_retention_reminder"),
            _ => None,
        };
        if let Some(field) = field
            && old != new
            && new.to_s() != "0"
        {
            sqlx::query(&format!(
                "UPDATE user_options SET {field} = FALSE WHERE {field} = TRUE"
            ))
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

/// DiscourseEvent `:user_added_to_group` (GroupManager#add, each user
/// added): chat's AutoJoinChannels for the user.
pub async fn user_added_to_group(
    conn: &mut sqlx::PgConnection,
    state: &crate::AppState,
    settings: &SiteSettings,
    user_id: i32,
) -> Result<(), crate::AppError> {
    if chat::enabled(settings)? {
        chat::auto_join::user(conn, &state.bus, state, settings, user_id).await?;
    }
    Ok(())
}

/// DiscourseEvent `:user_removed_from_group` (GroupManager#remove, each
/// user removed): chat's AutoLeaveChannels for the user.
pub async fn user_removed_from_group(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    user_id: i32,
) -> Result<(), crate::AppError> {
    if chat::enabled(settings)? {
        chat::scheduled::auto_leave_channels(
            conn,
            settings,
            "user_removed_from_group",
            Some(user_id),
        )
        .await?;
    }
    Ok(())
}

/// DiscourseEvent `:group_destroyed`: chat's AutoLeaveChannels for
/// everyone (its group_id narrows nothing).
pub async fn group_destroyed(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
) -> Result<(), crate::AppError> {
    if chat::enabled(settings)? {
        chat::scheduled::auto_leave_channels(conn, settings, "group_destroyed", None).await?;
    }
    Ok(())
}

/// The admin dashboard's plugin sections (`register_admin_dashboard_section`,
/// a filtered register: only enabled plugins'), in registration order:
/// each one whose `enabled` holds. Only discourse-solved's "support"
/// section among the bundled plugins the port runs.
pub async fn admin_dashboard_section_ids(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
) -> Result<Vec<&'static str>, crate::AppError> {
    let mut ids = Vec::new();
    if solved::dashboard::available(conn, settings).await? {
        ids.push(solved::dashboard::SECTION_ID);
    }
    Ok(ids)
}

/// A plugin section's loader (`section[:loader].call(start_date:,
/// end_date:, current_user:)`); None for an id no plugin registers.
pub async fn admin_dashboard_section(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    id: &str,
    start: Option<&str>,
    end: Option<&str>,
) -> Result<Option<serde_json::Value>, crate::AppError> {
    if id != solved::dashboard::SECTION_ID || !solved::enabled(settings)? {
        return Ok(None);
    }
    let ids = solved::dashboard::stored_category_ids(conn).await?;
    solved::dashboard::build(conn, settings, guardian, start, end, &ids)
        .await
        .map(Some)
}

/// A plugin section's setting (`plugin_setting_definition`, then its
/// class's `validate` over the permitted params): None when the section
/// is unavailable or has no such setting, else the value to store or the
/// InvalidParameters key.
pub async fn admin_dashboard_section_setting(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    section_id: &str,
    key: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<Option<Result<serde_json::Value, &'static str>>, crate::AppError> {
    if section_id != solved::dashboard::SECTION_ID
        || !admin_dashboard_section_ids(conn, settings)
            .await?
            .contains(&section_id)
        || key != "categories"
    {
        return Ok(None);
    }
    solved::dashboard::validate_categories(conn, params)
        .await
        .map(Some)
}

/// A plugin section's component (`registerAdminDashboardSection`), drawn
/// from its data: None when no plugin registers one for the id (the
/// dashboard then draws nothing for it).
pub fn admin_dashboard_section_html(
    dashboard: &crate::admin_dashboard_view::Dashboard<'_>,
    id: &str,
    data: &serde_json::Value,
    error: bool,
) -> Result<Option<String>, crate::AppError> {
    // The initializer registers it while solved_enabled; the server sends
    // the section only then.
    if id != solved::dashboard::SECTION_ID {
        return Ok(None);
    }
    solved::dashboard_view::section(dashboard, data, error).map(Some)
}

/// The related items component of a report's plugin renderer
/// (`registerAdminReportRelatedItemsRenderer`), drawn after the related
/// users: discourse-solved's solved topics for accepted_solutions.
pub fn admin_report_related_items_html(
    cx: &crate::admin_view::AdminCx,
    list: &crate::topic_list_view::ListContext,
    report_type: &str,
    report: &serde_json::Value,
    start: &str,
    end: &str,
) -> String {
    if report_type == solved::report_view::REPORT {
        return solved::report_view::related_items(cx, list, report, start, end);
    }
    String::new()
}
