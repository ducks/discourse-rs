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
