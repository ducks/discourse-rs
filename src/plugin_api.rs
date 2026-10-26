//! Plugins built outside this crate (spike). A plugin implements
//! [`Plugin`] and a binary of its own runs the server with it:
//! `discourse_rs::cli::run(vec![Arc::new(MyPlugin)])`. The bundled
//! plugins (crate::plugins) are wired into core by name and are not on
//! this API yet.
//!
//! The plugins register once, before anything reads the settings or the
//! translations, and stay for the life of the process. Each hook runs
//! only while the plugin's enabled setting is on, as Rails' plugin
//! hooks check `enabled?`.
//!
//! What a plugin can do, as Discourse's plugin API names it:
//! - `settings.yml` and its locales: [`Plugin::settings_yml`],
//!   [`Plugin::locale_ymls`];
//! - routes (`Engine.routes`, `requires_plugin`): [`Plugin::routes`],
//!   404 while the plugin is disabled;
//! - jobs (`Jobs::Base`, `Jobs::Scheduled`): [`Plugin::jobs`],
//!   [`Plugin::scheduled`], [`Plugin::run_job`];
//! - `add_to_serializer(:current_user)` and `(:topic_view)`:
//!   [`Plugin::current_user_fields`], [`Plugin::topic_view_fields`];
//! - plugin outlets in the server-rendered pages: [`Plugin::outlet_html`].

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use serde_json::{Map, Value};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// What a hook may read: the request's or job's connection, the site's
/// settings and translations.
pub struct Ctx<'a> {
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
    pub i18n: &'a I18n,
}

/// A plugin outlet in a server-rendered page.
pub enum Outlet<'a> {
    /// `topic-above-posts`: above a topic's post stream. `view` is the
    /// topic view's JSON, with the plugins' `topic_view_fields` in it.
    TopicAbovePosts { view: &'a Value },
}

#[async_trait]
pub trait Plugin: Send + Sync {
    /// The plugin's name, as `# name:` in plugin.rb.
    fn name(&self) -> &'static str;

    /// `enabled_site_setting`: the setting that turns the plugin on.
    /// None: always on.
    fn enabled_setting(&self) -> Option<&'static str> {
        None
    }

    /// The plugin's settings, one document of plugin_settings.yml's shape:
    /// `plugin`, `enabled_site_setting` and `settings` (config/settings.yml).
    fn settings_yml(&self) -> Option<&'static str> {
        None
    }

    /// Locale files, each with its `en:` root: server.en.yml as it is,
    /// client.en.yml with its keys under `js.` (`en: js: ...`).
    fn locale_ymls(&self) -> &[&'static str] {
        &[]
    }

    /// Routes mounted beside core's. They answer 404 while the plugin is
    /// disabled.
    fn routes(&self) -> Router<AppState> {
        Router::new()
    }

    /// Names of the regular jobs this plugin runs (`Jobs.enqueue`).
    fn jobs(&self) -> &[&'static str] {
        &[]
    }

    /// Scheduled jobs: name and interval (`every`). They run on each
    /// site's worker beside core's.
    fn scheduled(&self) -> Vec<(&'static str, Duration)> {
        Vec::new()
    }

    /// Runs one of `jobs` or `scheduled` (args are null for a scheduled
    /// one).
    async fn run_job(&self, _state: &AppState, _name: &str, _args: &Value) -> Result<(), AppError> {
        Ok(())
    }

    /// Keys added to `/session/current.json`'s `current_user`.
    async fn current_user_fields(
        &self,
        _ctx: &mut Ctx<'_>,
        _guardian: &Guardian,
    ) -> Result<Map<String, Value>, AppError> {
        Ok(Map::new())
    }

    /// Keys added to the topic view (`/t/:id.json`, and the page's view).
    async fn topic_view_fields(
        &self,
        _ctx: &mut Ctx<'_>,
        _guardian: &Guardian,
        _topic_id: i32,
    ) -> Result<Map<String, Value>, AppError> {
        Ok(Map::new())
    }

    /// HTML for an outlet, inserted as it is: escape what you render.
    async fn outlet_html(
        &self,
        _ctx: &mut Ctx<'_>,
        _outlet: &Outlet<'_>,
    ) -> Result<Option<String>, AppError> {
        Ok(None)
    }
}

static REGISTERED: OnceLock<Vec<Arc<dyn Plugin>>> = OnceLock::new();

/// Registers the process's plugins. Once, before the settings and the
/// translations are first read; a second call is refused.
pub fn install(plugins: Vec<Arc<dyn Plugin>>) -> Result<(), String> {
    REGISTERED
        .set(plugins)
        .map_err(|_| "plugins are already installed".to_string())
}

/// The registered plugins, enabled or not.
pub fn registered() -> &'static [Arc<dyn Plugin>] {
    REGISTERED.get().map(Vec::as_slice).unwrap_or(&[])
}

/// Whether a plugin's enabled setting is on.
pub fn is_enabled(plugin: &dyn Plugin, settings: &SiteSettings) -> Result<bool, AppError> {
    match plugin.enabled_setting() {
        Some(name) => Ok(settings.get(name)?.truthy()),
        None => Ok(true),
    }
}

/// The enabled plugins.
pub fn enabled(settings: &SiteSettings) -> Result<Vec<&'static Arc<dyn Plugin>>, AppError> {
    let mut on = Vec::new();
    for plugin in registered() {
        if is_enabled(plugin.as_ref(), settings)? {
            on.push(plugin);
        }
    }
    Ok(on)
}

/// The enabled plugins' `current_user_fields`, merged into `out`.
pub async fn current_user_fields(
    ctx: &mut Ctx<'_>,
    guardian: &Guardian,
    out: &mut Map<String, Value>,
) -> Result<(), AppError> {
    for plugin in enabled(ctx.settings)? {
        out.extend(plugin.current_user_fields(ctx, guardian).await?);
    }
    Ok(())
}

/// The enabled plugins' `topic_view_fields`, merged into `out`.
pub async fn topic_view_fields(
    ctx: &mut Ctx<'_>,
    guardian: &Guardian,
    topic_id: i32,
    out: &mut Map<String, Value>,
) -> Result<(), AppError> {
    for plugin in enabled(ctx.settings)? {
        out.extend(plugin.topic_view_fields(ctx, guardian, topic_id).await?);
    }
    Ok(())
}

/// The enabled plugins' HTML for an outlet, in registration order.
pub async fn outlet_html(ctx: &mut Ctx<'_>, outlet: &Outlet<'_>) -> Result<String, AppError> {
    let mut html = String::new();
    for plugin in enabled(ctx.settings)? {
        if let Some(part) = plugin.outlet_html(ctx, outlet).await? {
            html.push_str(&part);
        }
    }
    Ok(html)
}

/// The plugin that runs a regular job by this name, enabled or not.
pub fn job_owner(name: &str) -> Option<&'static Arc<dyn Plugin>> {
    registered().iter().find(|p| p.jobs().contains(&name))
}
