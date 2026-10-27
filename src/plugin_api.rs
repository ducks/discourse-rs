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
//! - plugin outlets in the server-rendered pages: [`Plugin::outlet_html`];
//! - `register_homepage`: [`Plugin::homepage`],
//!   [`Plugin::homepage_available`];
//! - a page of its own in core's head: [`PageHead`].
//!
//! A plugin's crate repeats this crate's `[patch.crates-io]` for
//! grass_compiler: Cargo applies patches from the top-level crate only,
//! and the unpatched compiler fails on core's own stylesheets.

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
    /// `Discourse.base_path`.
    pub base_path: &'a str,
}

/// A plugin outlet in a server-rendered page.
pub enum Outlet<'a> {
    /// `topic-above-posts`: above a topic's post stream. `view` is the
    /// topic view's JSON, with the plugins' `topic_view_fields` in it.
    TopicAbovePosts { view: &'a Value },
}

/// `register_homepage`: an option for the default_homepage setting. While
/// it is the visitor's homepage, `/` answers as the plugin's own route at
/// `path` does (Rails mounts `route` at `/`; here the route is the one
/// behind `path`), and `path` stays the page's canonical URL. Offered
/// while the plugin is enabled, which is the `enabled` condition a
/// plugin passes in practice.
#[derive(Debug, Clone, Copy)]
pub struct Homepage {
    /// The stored setting value.
    pub id: &'static str,
    /// The client translation key the admin setting shows.
    pub name: &'static str,
    /// The application path, without the subfolder.
    pub path: &'static str,
    /// Whether logged-out visitors may have it.
    pub anonymous: bool,
    /// Whether navigating to it takes a full page request.
    pub server_side: bool,
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

    /// `register_asset` for a stylesheet: where the plugin serves its CSS
    /// (from its own routes). Core's pages link it after the theme's while
    /// the plugin is enabled, as Rails links each plugin's target.
    fn stylesheet_url(&self, _base_path: &str) -> Option<String> {
        None
    }

    /// `register_svg_icon`: icons added to the site's sprite.
    fn svg_icons(&self) -> &[&'static str] {
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

    /// `register_homepage`.
    fn homepage(&self) -> Option<Homepage> {
        None
    }

    /// `register_homepage(available:)`: false gives this visitor the top
    /// menu's homepage instead. It runs whenever the homepage is
    /// resolved, so keep it cheap.
    async fn homepage_available(
        &self,
        _conn: &mut PgConnection,
        _settings: &SiteSettings,
        _guardian: &Guardian,
    ) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// What a plugin's own page (a layout of its own, as Rails plugins render
/// with `layout "..."`) takes from core's head: layouts/_head's icons,
/// the color schemes' stylesheets, core's stylesheet, the default theme's
/// stylesheets and its `head_tag` and `body_tag` (theme_lookup).
pub struct PageHead {
    /// `html_lang`: default_locale with a hyphen.
    pub lang: String,
    /// Goes in `<head>`, as it is: the icon links, the color scheme meta
    /// and stylesheets, core's stylesheet, the theme's stylesheets, then
    /// the theme's head_tag.
    pub head: String,
    /// `theme_lookup("body_tag")`, first in `<body>`.
    pub body_tag: String,
}

impl PageHead {
    pub async fn load(
        conn: &mut PgConnection,
        state: &AppState,
        settings: &SiteSettings,
        guardian: &Guardian,
    ) -> Result<PageHead, AppError> {
        let base = state.config.globals.relative_url_root().to_string();
        let urls = crate::url::Urls {
            config: &state.config,
            settings,
        };
        let theme_state = crate::theme_cache::read(&mut *conn).await?;
        let mut head = String::new();
        let favicon = crate::site_icons::site_url(&mut *conn, &urls, "favicon").await?;
        if !favicon.is_empty() {
            head.push_str(&format!(
                "<link rel=\"icon\" type=\"{}\" href=\"{}\">\n",
                crate::site_icons::mime_type(&favicon),
                escape(&favicon)
            ));
        }
        let touch = crate::site_icons::site_url(&mut *conn, &urls, "apple_touch_icon").await?;
        if !touch.is_empty() {
            head.push_str(&format!(
                "<link rel=\"apple-touch-icon\" type=\"image/png\" href=\"{}\">\n",
                escape(&touch)
            ));
        }
        let options: Option<(Option<i32>, Option<i32>, i32)> = match guardian.user() {
            Some(user) => {
                sqlx::query_as(
                    "SELECT color_scheme_id, dark_scheme_id, interface_color_mode \
                     FROM user_options WHERE user_id = $1",
                )
                .bind(user.id)
                .fetch_optional(&mut *conn)
                .await?
            }
            None => None,
        };
        let options = options.map(|(light, dark, mode)| crate::color_mode::Options {
            color_scheme_id: light,
            dark_scheme_id: dark,
            interface_color_mode: mode,
        });
        let colors = crate::color_mode::head_at(
            &mut *conn,
            settings,
            &state.i18n,
            &base,
            &crate::color_mode::cookies(),
            options.as_ref(),
            &theme_state,
        )
        .await?;
        head.push_str(&colors.meta);
        head.push_str(&colors.links);
        head.push_str(&format!(
            "\n<link rel=\"stylesheet\" href=\"{}\">\n",
            crate::assets::url(&base, "discourse.css")
        ));
        let theme_id = settings.get("default_theme_id")?.to_i();
        if let Ok(theme_id) = i32::try_from(theme_id) {
            head.push_str(
                &crate::stylesheet::manager::link_tags_at(
                    &mut *conn,
                    &crate::stylesheet::manager::Cx {
                        config: &state.config,
                        settings,
                    },
                    theme_id,
                    false,
                    &theme_state,
                )
                .await?,
            );
        }
        let html = crate::theme_html::lookup_at(
            &mut *conn,
            &crate::themes::Ctx {
                settings,
                i18n: &state.i18n,
                urls: &urls,
                defs: &state.site_setting_defs,
            },
            Some(theme_id),
            crate::mobile::on_mobile_device(),
            &theme_state,
        )
        .await?;
        head.push_str(&stylesheet_links(settings, &base)?);
        head.push('\n');
        head.push_str(&html.head_tag);
        Ok(PageHead {
            lang: settings.get("default_locale")?.to_s().replace('_', "-"),
            head,
            body_tag: html.body_tag,
        })
    }
}

fn escape(value: &str) -> String {
    crate::admin_site_settings::escape_html(value)
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

/// A plugin's homepage option, with the plugin that registered it.
pub type HomepageOption = (&'static Arc<dyn Plugin>, Homepage);

/// `DiscoursePluginRegistry.homepage_options`: the enabled plugins'.
pub fn homepages(settings: &SiteSettings) -> Result<Vec<HomepageOption>, AppError> {
    Ok(enabled(settings)?
        .into_iter()
        .filter_map(|plugin| plugin.homepage().map(|h| (plugin, h)))
        .collect())
}

/// `HomepageHelper.option_available?`: true for a homepage no plugin
/// registered. A failing check is logged and counts as unavailable, so
/// it cannot take the front page down.
pub async fn homepage_available(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    id: &str,
) -> Result<bool, AppError> {
    let Some((plugin, _)) = homepages(settings)?.into_iter().find(|(_, h)| h.id == id) else {
        return Ok(true);
    };
    match plugin.homepage_available(conn, settings, guardian).await {
        Ok(available) => Ok(available),
        Err(e) => {
            tracing::warn!("Homepage availability check failed for '{id}': {e}");
            Ok(false)
        }
    }
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

/// The enabled plugins' stylesheet links, for a page's head.
pub fn stylesheet_links(settings: &SiteSettings, base_path: &str) -> Result<String, AppError> {
    let mut out = String::new();
    for plugin in enabled(settings)? {
        if let Some(url) = plugin.stylesheet_url(base_path) {
            out.push_str(&format!(
                "<link href=\"{}\" media=\"all\" rel=\"stylesheet\" data-target=\"{}\"/>\n",
                escape(&url),
                escape(plugin.name())
            ));
        }
    }
    Ok(out)
}

/// Every installed plugin's `svg_icons` (DiscoursePluginRegistry.svg_icons).
pub fn svg_icons() -> impl Iterator<Item = &'static str> {
    registered()
        .iter()
        .flat_map(|p| p.svg_icons().iter().copied())
}

/// `form_authenticity_token` for a plugin's HTML form: the session's token,
/// masked, and the session cookie to set when the token was just made.
pub fn form_authenticity_token(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    settings: &SiteSettings,
) -> Result<(String, Option<String>), AppError> {
    let mut session = crate::session::forum::load_forum_session(state, headers);
    let stored = session.csrf_token();
    let masked = crate::session::csrf::mask(&stored)
        .ok_or(crate::Unsupported("session csrf token not decodable"))?;
    Ok((masked, session.set_cookie(state, settings)?))
}

/// `verify_authenticity_token` for a plugin's write: the session's token
/// against `X-CSRF-Token` or the form's `authenticity_token`. API keys pass.
pub fn verify_authenticity_token(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    form: &[(String, String)],
    path: &str,
    method: &str,
) -> bool {
    crate::routes::csrf_ok(state, headers, form, path, method)
}

/// The plugin that runs a regular job by this name, enabled or not.
pub fn job_owner(name: &str) -> Option<&'static Arc<dyn Plugin>> {
    registered().iter().find(|p| p.jobs().contains(&name))
}
