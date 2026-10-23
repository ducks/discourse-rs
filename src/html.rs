//! Server-rendered pages for anonymous readers, built from the same
//! documents the JSON endpoints return. The structure follows Discourse's
//! crawler views (app/views/list/list.erb, topics/show.html.erb); the
//! styling is ours.

use std::collections::HashMap;

use askama::Template;
use chrono::NaiveDateTime;
use serde_json::Value;
use sqlx::PgConnection;

use crate::i18n::I18n;
use crate::site_settings::{SettingError, SiteSettings};

#[derive(Debug)]
pub enum HtmlError {
    Db(sqlx::Error),
    Setting(SettingError),
    Template(askama::Error),
    MessageFormat(crate::message_format::ParseError),
    Unsupported(crate::Unsupported),
}

impl std::fmt::Display for HtmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HtmlError::Db(e) => write!(f, "rendering page: {e}"),
            HtmlError::Setting(e) => e.fmt(f),
            HtmlError::Template(e) => write!(f, "rendering template: {e}"),
            HtmlError::MessageFormat(e) => e.fmt(f),
            HtmlError::Unsupported(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for HtmlError {}

impl From<crate::message_format::ParseError> for HtmlError {
    fn from(e: crate::message_format::ParseError) -> Self {
        HtmlError::MessageFormat(e)
    }
}

impl From<sqlx::Error> for HtmlError {
    fn from(e: sqlx::Error) -> Self {
        HtmlError::Db(e)
    }
}

impl From<crate::guardian::GuardianError> for HtmlError {
    fn from(e: crate::guardian::GuardianError) -> Self {
        match e {
            crate::guardian::GuardianError::Db(e) => HtmlError::Db(e),
            crate::guardian::GuardianError::Setting(e) => HtmlError::Setting(e),
            crate::guardian::GuardianError::Unsupported(e) => HtmlError::Unsupported(e),
        }
    }
}

impl From<SettingError> for HtmlError {
    fn from(e: SettingError) -> Self {
        HtmlError::Setting(e)
    }
}

impl From<crate::plugins::PluginError> for HtmlError {
    fn from(e: crate::plugins::PluginError) -> Self {
        match e {
            crate::plugins::PluginError::Db(e) => HtmlError::Db(e),
            crate::plugins::PluginError::Setting(e) => HtmlError::Setting(e),
            crate::plugins::PluginError::Unsupported(e) => HtmlError::Unsupported(e),
        }
    }
}

impl From<askama::Error> for HtmlError {
    fn from(e: askama::Error) -> Self {
        HtmlError::Template(e)
    }
}

/// What the shell shows a logged-in viewer: their name, and the CSRF
/// token the logout form and the client need (anonymous pages carry
/// none, as they may be cached).
#[derive(Clone, Default)]
pub struct Viewer {
    pub username: String,
    pub csrf_token: String,
    /// The header's avatar (48px).
    pub avatar_url: String,
    /// For the topic list's new-topic dot (`trust_level > 0`).
    pub trust_level: i32,
    pub name: Option<String>,
    /// No previous visit: the welcome banner greets a new member.
    pub first_visit: bool,
    pub staff: bool,
    /// `currentUser.can_send_private_messages`
    pub can_send_private_messages: bool,
    /// `currentUser.previous_visit_at`, for the lists' last visit marker.
    pub previous_visit_at: Option<chrono::DateTime<chrono::Utc>>,
    /// The `text_size` cookie (`<size>|<seq>`), one device's text size.
    pub text_size_cookie: Option<String>,
}

impl Viewer {
    /// `User#canManageTopic`: staff or a leader (trust level 4).
    pub fn can_manage_topic(&self) -> bool {
        self.staff || self.trust_level >= 4
    }
}

/// The viewer block for a page, plus the `_forum_session` cookie to set
/// when minting the CSRF token created the session.
#[derive(Clone, Default)]
pub struct ViewerState {
    pub viewer: Option<Viewer>,
    pub set_cookie: Option<String>,
}

/// The headers a logged-in page carries: the session cookie when it was
/// just created, `X-Discourse-Username`, and no caching (anonymous pages
/// may be cached, these never are).
pub fn with_viewer_headers(
    mut response: axum::response::Response,
    viewer: &ViewerState,
) -> axum::response::Response {
    use axum::http::{HeaderValue, header};
    if let Some(cookie) = &viewer.set_cookie
        && let Ok(value) = HeaderValue::from_str(cookie)
    {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    if let Some(v) = &viewer.viewer {
        if let Ok(value) = HeaderValue::from_str(&v.username) {
            response.headers_mut().insert("x-discourse-username", value);
        }
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache, no-store"),
        );
    }
    response
}
/// What every page's layout needs.
pub struct Site {
    pub site_title: String,
    pub site_description: String,
    pub lang: String,
    pub base_path: String,
    pub viewer: Option<Viewer>,
    /// Where the page's live updates start (pg-bus position as text), taken
    /// before its data was read; empty for no live updates.
    pub bus_position: String,
    /// What the layout's header and body need beyond the settings.
    pub chrome: Chrome,
    /// The list categories as the chrome's composer read them, for the
    /// page to take (list_categories).
    categories: Option<HashMap<i64, crate::topic_list_view::ListCategory>>,
}

impl Site {
    pub fn from_settings(settings: &SiteSettings, base_path: &str) -> Result<Site, SettingError> {
        Ok(Site {
            site_title: settings.get("title")?.to_s(),
            site_description: settings.get("site_description")?.to_s(),
            lang: settings.get("default_locale")?.to_s().replace('_', "-"),
            base_path: base_path.to_string(),
            viewer: None,
            bus_position: String::new(),
            chrome: Chrome::default(),
            categories: None,
        })
    }

    /// topic_list_view::categories: the chrome's when it read them, else
    /// read now.
    pub async fn list_categories(
        &mut self,
        conn: &mut PgConnection,
    ) -> Result<HashMap<i64, crate::topic_list_view::ListCategory>, sqlx::Error> {
        match self.categories.take() {
            Some(categories) => Ok(categories),
            None => crate::topic_list_view::categories(conn).await,
        }
    }
}

/// The page chrome around every page: the header and the body's classes.
#[derive(Clone, Default)]
pub struct Chrome {
    /// The header's logo (`SiteSetting.site_logo_url`), empty for the site
    /// title as text.
    pub logo_url: String,
    /// `SiteSetting.site_mobile_logo_url` in the mobile view, empty
    /// otherwise or for none (HomeLogo#showMobileLogo).
    pub mobile_logo_url: String,
    /// `SiteSetting.site_logo_small_url`, the minimized logo the docked
    /// header shows over a topic (HomeLogo with `@minimized`); empty for
    /// none.
    pub logo_small_url: String,
    /// `site.can_search`: the mobile header's search button.
    pub can_search: bool,
    /// `SiteSetting.site_favicon_url` and its type (MiniMime by filename),
    /// empty for no icon link (layouts/_head).
    pub favicon_url: String,
    pub favicon_type: &'static str,
    /// `SiteSetting.site_apple_touch_icon_url`, absolute.
    pub apple_touch_icon_url: String,
    /// The body's classes: `uc-<name>` for each enabled upcoming change
    /// with CSS (ApplicationController#upcomingChangeBodyClasses).
    pub body_classes: String,
    /// `canSignUp`: the header shows a Sign Up button.
    pub can_signup: bool,
    /// GlobalNotice above the page's content: the read-only notice while
    /// the site is read only, empty otherwise.
    pub global_notice: String,
    /// The sidebar's markup, empty when the page has none
    /// (ApplicationController#sidebarEnabled).
    pub sidebar: String,
    /// The mobile view's header menu with the sidebar's sections
    /// (sidebar::hamburger), which the hamburger button opens; empty on
    /// the desktop view.
    pub hamburger: String,
    /// A member's composer (composer_view), empty for a visitor.
    pub composer: String,
    /// `canCreateTopic`: the lists' New Topic button.
    pub can_create_topic: bool,
    /// TopicDraftsDropdown's menu trigger beside New Topic, while the
    /// member has drafts (`draft_count`); empty otherwise.
    pub drafts_menu_trigger: String,
    /// A member's topic tracking state, for the nav pills' counts.
    pub tracking: Option<crate::topic_tracking_report::Tracking>,
    /// What the page's live updates keep counted besides the stream's own
    /// parameters (`&sidebar=<Active key>`, `&nav=<active pill>`), encoded.
    pub live_params: String,
    /// The footer service's showFooter: the pages that hide the application
    /// footer (a list with more to load, a topic not loaded to its end, the
    /// admin) go without PoweredByDiscourse and the theme's footer.
    pub show_footer: bool,
    /// PoweredByDiscourse below the content, where the footer shows:
    /// enable_powered_by_discourse.
    pub powered_by: bool,
    /// The theme's html fields (crate::theme_html).
    pub theme_html: crate::theme_html::Fields,
    /// Chat's header icon (plugins::chat::view), empty when the viewer
    /// can't chat.
    pub chat_header_icon: String,
    /// Classes a route adds to `<html>` (htmlClass), each with a leading
    /// space.
    pub html_classes: String,
    /// The hashtag-css-generator initializer's rules for the viewer's site
    /// categories, and their ids (the ones CategoryHashtagType counts as
    /// loaded).
    pub hashtag_css: String,
    pub hashtag_category_ids: String,
    /// What hashtags.js makes emoji and the tag icon from: Emoji.url_for's
    /// location for the set, the images' version, `hashtag_icons.tag`.
    pub emoji_base: String,
    pub emoji_version: String,
    /// The custom emoji by lowercase name and the denied ones, as JSON
    /// (`registerEmoji`, `site.denied_emojis`).
    pub custom_emoji: String,
    pub denied_emoji: String,
    pub hashtag_tag_icon: String,
    /// What lightbox.js needs besides PhotoSwipe's urls, as data
    /// attributes for its script tag (`lightbox_attrs`).
    pub lightbox_attrs: String,
    /// What highlight-syntax.js reads: `session.highlightJsPath` with the
    /// base path, and `autohighlight_all_code`.
    pub highlight_js_path: String,
    pub autohighlight_all_code: bool,
    /// The site's sprite script with the base path (crate::svg_sprite::path);
    /// empty inlines the static sprite.
    pub svg_sprite_path: String,
    /// What the cooked decorators read (cooked.js): the site settings
    /// that gate them and their strings, as JSON for a script element.
    pub cooked_config: String,
    /// The user card opens (static/js/user-card.js): not for an anonymous
    /// visitor under hide_user_profiles_from_public (CardContentsBase#_show).
    pub user_cards: bool,
    /// enable_badges, the card's showBadges.
    pub enable_badges: bool,
    /// discourse-footnote's inline footnotes (static/js/inline-footnotes.js):
    /// the plugin is on and display_footnotes_inline.
    pub inline_footnotes: bool,
    /// What static/js/local-dates.js reads, as data attributes for its
    /// script tag; empty while discourse_local_dates_enabled is off.
    pub local_dates_attrs: String,
    /// What static/js/dates.js reads, as data attributes for its script
    /// tag: the dates.* strings and relative_date_duration.
    pub dates_attrs: String,
    /// avatar_sizes, for static/js/avatars.js's script tag: the sizes it
    /// picks an avatar's source from at the screen's pixel density
    /// (getRawAvatarSize).
    pub avatar_sizes: String,
    /// User.resolveCurrent's `_saveTimezone`: a member without a timezone
    /// saves the browser's guess as the app boots.
    pub save_timezone: bool,
    /// ApplicationHelper#text_size_class's size: this device's cookie when
    /// it is as new as the member's option, else the option, else
    /// default_text_size.
    pub text_size: String,
    /// The search service's searchContext as the mobile search button
    /// passes it to the full-page search (Header#toggleSearchMenu): a
    /// query string, empty on pages without one.
    pub search_context: String,
    /// The header's sidebar toggle (sidebarEnabled), shown in the narrow
    /// desktop view too, where it opens the hamburger slide-in.
    pub sidebar_toggle: bool,
    /// The same context for the search menu (search-menu.js): its type
    /// and id, empty without one.
    pub search_context_type: String,
    pub search_context_id: String,
    /// The search menu's strings, as JSON for its script's data attribute.
    pub search_menu_strings: String,
    /// The page shows the welcome banner, whose search hides the header's
    /// button while it is in view (Icons#showSearchButton).
    pub welcome_banner: bool,
    /// A route of ALL_PAGES_EXCLUDED_ROUTES (the full-page search, login,
    /// signup, ...): no header search button.
    pub hide_search_button: bool,
    /// hideApplicationHeaderButtons("login", "signup") (the invite page):
    /// no log in or sign up buttons.
    pub hide_auth_buttons: bool,
    /// No header icons for an anonymous reader while login is required
    /// (Header: login_required without a current user).
    pub hide_header_icons: bool,
    /// The mobile view's full-page chat: the header's logo gives way to
    /// chat's (ChatHeader: back to the forum, the Chat heading).
    pub chat_header: String,
    /// The chat channel route's back button indicator in the mobile view
    /// (plugins::chat::view).
    pub chat_back_indicator: String,
    /// The chat plugin is on: its below-footer connector (ChatDrawerOutlet)
    /// draws the drawer's outlet on every page.
    pub chat_drawer_outlet: bool,
    /// chat-drawer.js's attributes, empty when the viewer can't chat (no
    /// drawer opens).
    pub chat_drawer: String,
    /// The translations the templates' client strings come from (site
    /// texts included); the vendored files when the page loaded no chrome.
    pub i18n: Option<std::sync::Arc<I18n>>,
    /// Rails' admin stylesheet, which staff are served on every page.
    pub admin_stylesheet: bool,
    /// discourse-solved's admin stylesheet (its `:admin` assets), which
    /// staff are served while the plugin is enabled.
    pub solved_admin_stylesheet: bool,
    /// The color scheme meta tags and stylesheets (crate::color_mode).
    pub color_head: crate::color_mode::Head,
    /// The theme stylesheets' link tags (crate::stylesheet::manager).
    pub theme_stylesheets: String,
}

impl Chrome {
    /// The bodyClass helper: `class` on the body after the others, unless
    /// the body has it already (classList.add).
    pub fn add_body_class(&mut self, class: &str) {
        if !self.body_classes.split(' ').any(|c| c == class) {
            if !self.body_classes.is_empty() {
                self.body_classes.push(' ');
            }
            self.body_classes.push_str(class);
        }
    }

    /// A discovery list's body classes after the chrome's: WelcomeBanner's
    /// `welcome-banner--enabled`, the list's own (Navigation's
    /// `navigation-*`, AddCategoryTagClasses'), then
    /// `welcome-banner--visible`, which WelcomeBanner adds once its search
    /// is found below the header: drawn on a desktop page that can search,
    /// welcome-banner.js keeps it current as the page scrolls or narrows.
    pub fn discovery_body_classes(&mut self, classes: &[&str]) {
        if self.welcome_banner {
            self.add_body_class("welcome-banner--enabled");
        }
        for class in classes {
            self.add_body_class(class);
        }
        if self.welcome_banner && self.can_search && !crate::mobile::mobile_view() {
            self.add_body_class("welcome-banner--visible");
        }
    }

    /// An admin page: the admin panel in place of the forum's sidebar (or
    /// in the mobile view's header menu), no live sidebar counts for links
    /// that are not on the page, `admin-area` on `<html>`, `admin-interface`
    /// on the body, and no footer (hideApplicationFooter).
    pub fn use_admin_panel(&mut self, sidebar: String, hamburger: String) {
        let had_sidebar = !self.sidebar.is_empty();
        self.sidebar = sidebar;
        self.hamburger = hamburger;
        self.live_params = remove_param(&self.live_params, "sidebar");
        if !self.sidebar.is_empty() && !had_sidebar {
            self.body_classes.push_str(" has-sidebar-page");
        }
        self.body_classes.push_str(" admin-interface");
        self.body_classes = self.body_classes.trim().to_string();
        self.html_classes.push_str(" admin-area");
        self.show_footer = false;
    }
}

/// `params` (`&a=1&b=2`) less the `name` parameter.
fn remove_param(params: &str, name: &str) -> String {
    params
        .split('&')
        .filter(|p| !p.is_empty() && !p.starts_with(&format!("{name}=")))
        .map(|p| format!("&{p}"))
        .collect()
}

/// The vendored strings, for chrome that has no translations of its own.
fn vendored_i18n() -> &'static I18n {
    static I18N: std::sync::OnceLock<I18n> = std::sync::OnceLock::new();
    I18N.get_or_init(|| I18n::vendored().expect("the vendored locales parse"))
}

impl Chrome {
    fn i18n(&self) -> &I18n {
        match &self.i18n {
            Some(i18n) => i18n,
            None => vendored_i18n(),
        }
    }

    /// The Ember client's `i18n(key)`: the `js.` string, an admin's site
    /// text first; I18n.js's `[en.key]` for a missing one.
    pub fn t(&self, key: &str) -> String {
        self.i18n()
            .t(&format!("js.{key}"))
            .map(str::to_string)
            .unwrap_or_else(|| format!("[en.{key}]"))
    }

    /// `i18n(key, { name: value })`.
    pub fn t_with(&self, key: &str, name: &str, value: impl AsRef<str>) -> String {
        self.i18n()
            .t_with(&format!("js.{key}"), &[(name, value.as_ref())])
            .unwrap_or_else(|| format!("[en.{key}]"))
    }

    /// A server string (`I18n.t` in an ERB view), site texts included.
    pub fn server_t(&self, key: &str) -> String {
        self.i18n()
            .t(key)
            .map(str::to_string)
            .unwrap_or_else(|| format!("Translation missing: en.{key}"))
    }
}

/// static/js/dialog.js's #dialog-strings: its buttons' labels.
pub(crate) fn dialog_strings(i18n: &I18n) -> String {
    let t = |key: &str| i18n.t(&format!("js.{key}")).unwrap_or(key).to_string();
    serde_json::json!({
        "ok_value": t("ok_value"),
        "cancel": t("cancel"),
        "yes_value": t("yes_value"),
        "no_value": t("no_value"),
        "modal.close": t("modal.close"),
    })
    .to_string()
    .replace('<', "\\u003c")
}

/// The search menu's strings (search-menu.js), by the name it reads them
/// with, from their js.search.* keys.
fn search_menu_strings(i18n: &I18n) -> String {
    let strings: serde_json::Map<String, Value> = [
        "title",
        "clear_search",
        "open_advanced",
        "in_this_topic",
        "in_this_topic_tooltip",
        "in_messages",
        "in_messages_tooltip",
    ]
    .iter()
    .map(|key| {
        let full = format!("js.search.{key}");
        let text = i18n.t(&full).unwrap_or(key);
        ((*key).to_string(), Value::String(text.to_string()))
    })
    .collect();
    Value::Object(strings).to_string()
}

/// The client strings local-dates.js shows, by their js.* key.
const LOCAL_DATES_STRINGS: &[&str] = &[
    "discourse_local_dates.relative_dates.today",
    "discourse_local_dates.relative_dates.tomorrow",
    "discourse_local_dates.relative_dates.yesterday",
    "discourse_local_dates.relative_dates.countdown.passed",
    "download_calendar.add_to_calendar",
    "download_calendar.default_title",
    "download_calendar.title",
    "download_calendar.calendar",
    "download_calendar.save_ics",
    "download_calendar.save_google",
    "download_calendar.save_outlook",
    "download_calendar.save_apple",
    "download_calendar.remember",
    "download_calendar.remember_explanation",
    "cancel",
    // The insert date modal (local-dates-create.js).
    "discourse_local_dates.title",
    "discourse_local_dates.edit",
    "discourse_local_dates.create.form.insert",
    "discourse_local_dates.create.form.save",
    "discourse_local_dates.create.form.format_title",
    "discourse_local_dates.create.form.format_description",
    "discourse_local_dates.create.form.timezones_title",
    "discourse_local_dates.create.form.timezones_description",
    "discourse_local_dates.create.form.recurring_title",
    "discourse_local_dates.create.form.recurring_description",
    "discourse_local_dates.create.form.recurring_none",
    "discourse_local_dates.create.form.invalid_date",
    "discourse_local_dates.create.form.until",
    "discourse_local_dates.create.form.current_timezone",
    "discourse_local_dates.create.form.recurring.every_day",
    "discourse_local_dates.create.form.recurring.every_week",
    "discourse_local_dates.create.form.recurring.every_two_weeks",
    "discourse_local_dates.create.form.recurring.every_month",
    "discourse_local_dates.create.form.recurring.every_two_months",
    "discourse_local_dates.create.form.recurring.every_three_months",
    "discourse_local_dates.create.form.recurring.every_six_months",
    "discourse_local_dates.create.form.recurring.every_year",
    "advanced_mode_toggle.advanced_mode",
    "advanced_mode_toggle.simple_mode",
    "dates.previous_month",
    "dates.next_month",
    "modal.close",
    "select_kit.filter_placeholder",
    "select_kit.no_content",
    "select_kit.filter_by",
    "select_kit.max_content_reached.one",
    "select_kit.max_content_reached.other",
    "select_kit.default_header_text",
    "select_kit.delete_item",
    "timezone_input.ambiguous_ist",
];

/// local-dates.js's script tag attributes: the siteSettings it reads
/// (discourse_local_dates_default_timezones, title), the default title
/// (`discourse_local_dates.default_title` with the site's name), the
/// member's username and `user_option.default_calendar`, and its strings.
fn local_dates_attrs(
    settings: &SiteSettings,
    i18n: &I18n,
    username: Option<&str>,
    default_calendar: Option<i32>,
    moment_locale: &str,
    user_timezone: &str,
) -> Result<String, SettingError> {
    let esc = crate::topic_list_view::escape;
    let title = settings.get("title")?.to_s();
    let default_title = i18n
        .t("js.discourse_local_dates.default_title")
        .unwrap_or("%{site_name} Event")
        .replace("%{site_name}", &title);
    let default_calendar = default_calendar
        .and_then(|c| {
            ["none_selected", "ics", "google", "outlook", "apple"]
                .get(usize::try_from(c).ok()?)
                .copied()
        })
        .unwrap_or("");
    let strings: serde_json::Map<String, Value> = LOCAL_DATES_STRINGS
        .iter()
        .map(|key| {
            let text = i18n.t(&format!("js.{key}")).unwrap_or(key);
            ((*key).to_string(), Value::String(text.to_string()))
        })
        .collect();
    Ok(format!(
        " data-default-timezones=\"{}\" data-default-title=\"{}\" data-site-title=\"{}\" \
         data-username=\"{}\" data-default-calendar=\"{}\" data-strings=\"{}\" \
         data-moment-locale=\"{}\" data-user-timezone=\"{}\" data-default-formats=\"{}\"",
        esc(&settings
            .get("discourse_local_dates_default_timezones")?
            .to_s()),
        esc(&default_title),
        esc(&title),
        esc(username.unwrap_or("")),
        default_calendar,
        esc(&Value::Object(strings).to_string()),
        esc(moment_locale),
        esc(user_timezone),
        esc(&settings
            .get("discourse_local_dates_default_formats")?
            .to_s()),
    ))
}

/// The site settings the cooked decorators check, as the Ember client
/// reads them from siteSettings.
const COOKED_SETTINGS: &[&str] = &[
    "spoiler_enabled",
    "show_copy_button_on_codeblocks",
    "display_footnotes_inline",
    "checklist_enabled",
];

/// The client strings the cooked decorators show, by their js.* key.
const COOKED_STRINGS: &[&str] = &[
    "spoiler.label.show",
    "checklist.item",
    "copy_codeblock.copied",
    "copy_codeblock.copy",
    "copy_codeblock.fullscreen",
    "copy_codeblock.view_code",
    "fullscreen_table.expand_btn",
    "fullscreen_table.view_table",
    "modal.close",
    "expand",
    "post.collapse",
    "post.follow_quote",
    "generic_error",
    "generic_error_with_reason",
    "multiple_errors",
    "invalid_video_url",
    "cannot_render_video",
];

/// The cooked decorators' settings and strings, as JSON that is safe
/// inside a script element.
fn cooked_config(settings: &SiteSettings, i18n: &I18n) -> Result<String, SettingError> {
    let mut site_settings = serde_json::Map::new();
    for name in COOKED_SETTINGS {
        site_settings.insert(
            (*name).to_string(),
            Value::Bool(settings.get(name)?.truthy()),
        );
    }
    let strings: serde_json::Map<String, Value> = COOKED_STRINGS
        .iter()
        .map(|key| {
            let text = i18n.t(&format!("js.{key}")).unwrap_or(key);
            ((*key).to_string(), Value::String(text.to_string()))
        })
        .collect();
    let config = serde_json::json!({ "settings": site_settings, "strings": strings });
    Ok(config.to_string().replace('<', "\\u003c"))
}

/// The data attributes of static/js/dates.js's script tag: the client
/// strings discourse/lib/formatter and the bookmark reminders read (the
/// formats directly under js.dates, its tiny, medium and medium_with_ago
/// ages, js.now and js.bookmarks.reminders.*, overrides included) as JSON,
/// and the relative_date_duration site setting.
pub fn dates_attrs(settings: &SiteSettings, i18n: &I18n) -> Result<String, SettingError> {
    let mut keys: Vec<&str> = i18n
        .keys_with_prefix("js.dates.")
        .filter(|key| {
            let rest = &key["js.dates.".len()..];
            !rest.contains('.')
                || ["tiny.", "medium.", "medium_with_ago."]
                    .iter()
                    .any(|p| rest.starts_with(p))
        })
        .chain(i18n.keys_with_prefix("js.bookmarks.reminders."))
        .collect();
    keys.push("js.now");
    keys.sort_unstable();
    let strings: serde_json::Map<String, Value> = keys
        .into_iter()
        .filter_map(|key| {
            let text = i18n.t(key)?;
            Some((
                key["js.".len()..].to_string(),
                Value::String(text.to_string()),
            ))
        })
        .collect();
    Ok(format!(
        " data-strings=\"{}\" data-relative-date-duration=\"{}\"",
        crate::topic_list_view::escape(&Value::Object(strings).to_string()),
        settings.get("relative_date_duration")?.to_i()
    ))
}

impl Chrome {
    /// The route's searchContext (a topic's, a category's, a user's, a
    /// tag's, an inbox's): the mobile search button opens the full-page
    /// search with it, its checkbox off (`skip_context`).
    pub fn set_search_context(&mut self, kind: &str, id: &str) {
        let enc = |v: &str| form_urlencoded::byte_serialize(v.as_bytes()).collect::<String>();
        self.search_context = format!(
            "?context={}&context_id={}&skip_context=true",
            enc(kind),
            enc(id)
        );
        self.search_context_type = kind.to_string();
        self.search_context_id = id.to_string();
    }

    /// Adds a parameter for the page's live updates.
    pub fn live_param(&mut self, name: &str, value: &str) {
        self.live_params.push_str(&format!(
            "&{name}={}",
            form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>()
        ));
    }
}

/// lightbox.js's script tag attributes: whether the viewer may download
/// (`!prevent_anons_from_downloading_files || currentUser`) and the
/// `lightbox.*` titles its buttons and messages take.
fn lightbox_attrs(i18n: &I18n, can_download: bool) -> String {
    let mut out = format!(" data-can-download=\"{can_download}\"");
    for (attr, key) in [
        ("previous", "previous"),
        ("next", "next"),
        ("close", "close"),
        ("zoom", "zoom"),
        ("error", "error"),
        ("download", "download"),
        ("open", "open"),
        ("image-info", "image_info"),
    ] {
        let key = format!("js.lightbox.{key}");
        let title = crate::topic_list_view::escape(i18n.t(&key).unwrap_or(&key));
        out.push_str(&format!(" data-title-{attr}=\"{title}\""));
    }
    out
}

/// DMenu's trigger for the topic drafts menu, with what composer.js needs
/// to fill the menu (the count, and the labels of its items and its link
/// to every draft); empty without drafts (`@hasMenu`).
fn drafts_menu_trigger(i18n: &I18n, draft_count: i64) -> String {
    if draft_count <= 0 {
        return String::new();
    }
    let t = |key: &str| crate::topic_list_view::escape(i18n.t(&format!("js.{key}")).unwrap_or(key));
    format!(
        "<button aria-expanded=\"false\" aria-label=\"{title}\" class=\"btn no-text btn-icon fk-d-menu__trigger topic-drafts-menu-trigger d-combo-button-menu btn-primary\" title=\"{title}\" data-identifier=\"topic-drafts-menu\" data-trigger=\"\" type=\"button\" data-draft-count=\"{draft_count}\" data-label-untitled=\"{}\" data-label-view-all=\"{}\" data-label-other-drafts-one=\"{}\" data-label-other-drafts-other=\"{}\">{}</button>",
        t("drafts.dropdown.untitled"),
        t("drafts.dropdown.view_all"),
        t("drafts.dropdown.other_drafts.one"),
        t("drafts.dropdown.other_drafts.other"),
        crate::topic_list_view::icon("chevron-down", None),
        title = t("drafts.dropdown.title"),
    )
}

/// What the sidebar renders from: the Site's sidebar fields and the member
/// (with `tracking`), None for a visitor.
pub async fn sidebar_inputs(
    conn: &mut sqlx::PgConnection,
    state: &crate::AppState,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    tracking: Option<crate::topic_tracking_report::Tracking>,
) -> Result<(serde_json::Value, Option<crate::sidebar::Member>), crate::AppError> {
    let member = match (guardian.logged_in(), tracking) {
        (Some(user), Some(tracking)) => {
            let mut member =
                crate::current_user::sidebar_member(&mut *conn, settings, user, tracking).await?;
            if member.can_send_private_messages {
                member.pm_count =
                    crate::pm_tracking_state::inbox_count(&mut *conn, settings, guardian).await?;
            }
            Some(member)
        }
        _ => None,
    };
    let site = crate::site::Site {
        conn: &mut *conn,
        config: &state.config,
        settings,
        defs: &state.site_setting_defs,
        i18n: &state.i18n,
        guardian: guardian.clone(),
    }
    .sidebar_json()
    .await?;
    Ok((site, member))
}

impl Site {
    /// Fills the page chrome for the request's `guardian`. Every enabled
    /// upcoming change counts as enabled for the viewer; changes enabled
    /// for some groups only are not told apart yet.
    pub async fn load_chrome(
        &mut self,
        conn: &mut PgConnection,
        state: &crate::AppState,
        settings: &SiteSettings,
        guardian: &crate::guardian::Guardian,
        active: crate::sidebar::Active,
    ) -> Result<(), crate::AppError> {
        self.chrome.i18n = Some(state.i18n.clone());
        let urls = crate::url::Urls {
            config: &state.config,
            settings,
        };
        // GlobalNotice's read-only notice (isReadOnly, isStaffWritesOnly).
        let mode = crate::readonly::mode(&mut *conn).await?;
        self.chrome.global_notice =
            crate::readonly::notice(&state.i18n, mode, guardian.user().is_some());
        self.chrome.logo_url = crate::site_icons::site_url(&mut *conn, &urls, "logo").await?;
        self.chrome.logo_small_url =
            crate::site_icons::site_url(&mut *conn, &urls, "logo_small").await?;
        // HomeLogo#showMobileLogo
        if crate::mobile::mobile_view() {
            self.chrome.mobile_logo_url =
                crate::site_icons::site_url(&mut *conn, &urls, "mobile_logo").await?;
        }
        self.chrome.can_search = guardian.can_search(settings)?;
        self.chrome.search_menu_strings = search_menu_strings(&state.i18n);
        // build-category-route sets the category's searchContext.
        if let crate::sidebar::Active::Category(id) = &active {
            self.chrome.set_search_context("category", &id.to_string());
        }
        self.chrome.favicon_url = crate::site_icons::site_url(&mut *conn, &urls, "favicon").await?;
        self.chrome.favicon_type = crate::site_icons::mime_type(&self.chrome.favicon_url);
        self.chrome.apple_touch_icon_url =
            crate::site_icons::site_url(&mut *conn, &urls, "apple_touch_icon").await?;
        let mut classes = Vec::new();
        for name in state.site_setting_defs.upcoming_changes_with_css() {
            if settings.get(name)?.truthy() {
                classes.push(format!("uc-{}", name.replace('_', "-")));
            }
        }
        // GlimmerSiteHeader's `staff` for a staff member.
        if guardian.is_staff() {
            classes.push("staff".into());
        }
        self.chrome.show_footer = true;
        self.chrome.powered_by = settings.get("enable_powered_by_discourse")?.truthy();
        // theme_lookup and customHTML: the default theme's html fields for
        // the user agent's device.
        self.chrome.theme_html = crate::theme_html::lookup(
            &mut *conn,
            &crate::themes::Ctx {
                settings,
                i18n: &state.i18n,
                urls: &urls,
                defs: &state.site_setting_defs,
            },
            Some(settings.get("default_theme_id")?.to_i()),
            crate::mobile::on_mobile_device(),
        )
        .await?;
        self.chrome.admin_stylesheet = guardian.is_staff();
        self.chrome.solved_admin_stylesheet =
            guardian.is_staff() && crate::plugins::solved::enabled(settings)?;
        self.chrome.user_cards =
            guardian.user().is_some() || !settings.get("hide_user_profiles_from_public")?.truthy();
        self.chrome.enable_badges = settings.get("enable_badges")?.truthy();
        self.chrome.inline_footnotes = settings.get("enable_markdown_footnotes")?.truthy()
            && settings.get("display_footnotes_inline")?.truthy();
        self.chrome.can_signup = !settings.get("invite_only")?.truthy()
            && settings.get("allow_new_registrations")?.truthy()
            && !settings.get("enable_discourse_connect")?.truthy();

        // sidebarEnabled: canDisplaySidebar (not for anonymous visitors
        // when login is required) and the sidebar navigation menu, never in
        // the mobile view, whose header menu shows the same sections.
        // In the narrow desktop view the sidebar is enabled but not shown
        // (calculateShowSidebar): its toggle opens the same sections as the
        // mobile view's hamburger, in a slide-in.
        let mobile = crate::mobile::mobile_view();
        let narrow = crate::mobile::narrow_desktop_view();
        let can_display_sidebar =
            guardian.user().is_some() || !settings.get("login_required")?.truthy();
        let sidebar_enabled =
            can_display_sidebar && !mobile && settings.get("navigation_menu")?.to_s() == "sidebar";
        self.chrome.sidebar_toggle = sidebar_enabled;
        self.chrome.hide_header_icons = !can_display_sidebar;
        let hamburger = can_display_sidebar && (mobile || (narrow && sidebar_enabled));
        self.chrome.tracking =
            crate::topic_tracking_report::load(&mut *conn, settings, guardian).await?;
        let inputs = if sidebar_enabled || hamburger {
            Some(
                sidebar_inputs(
                    &mut *conn,
                    state,
                    settings,
                    guardian,
                    self.chrome.tracking.clone(),
                )
                .await?,
            )
        } else {
            None
        };
        // The site's categories, as the sidebar read them or, without one,
        // as Site serializes them (none for a visitor when login is
        // required).
        let site_categories = match &inputs {
            Some((site, _)) => site["categories"].as_array().cloned().unwrap_or_default(),
            None if guardian.user().is_none() && settings.get("login_required")?.truthy() => {
                Vec::new()
            }
            None => {
                crate::categories::Categories {
                    conn: &mut *conn,
                    settings,
                    i18n: &state.i18n,
                    guardian,
                    base_path: &self.base_path,
                    topic_url_via_slug: true,
                }
                .for_site()
                .await?
            }
        };
        self.chrome.hashtag_css = crate::hashtags::generator_css(&site_categories);
        self.chrome.hashtag_category_ids = site_categories
            .iter()
            .filter_map(|c| c["id"].as_i64())
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let emoji_set = settings.get("emoji_set")?.to_s().to_string();
        self.chrome.emoji_base = match settings.get("external_emoji_url")?.presence() {
            Some(external) => format!("{external}/{emoji_set}"),
            None => format!("{}/images/emoji/{emoji_set}", self.base_path),
        };
        self.chrome.emoji_version = crate::emoji::image_version().to_string();
        self.chrome.custom_emoji = settings.emojis().custom_urls_json();
        self.chrome.denied_emoji = serde_json::json!(settings.emojis().denied).to_string();
        if settings.get("tagging_enabled")?.truthy() {
            self.chrome.hashtag_tag_icon = "tag".into();
        }
        self.chrome.lightbox_attrs = lightbox_attrs(
            &state.i18n,
            !settings
                .get("prevent_anons_from_downloading_files")?
                .truthy()
                || guardian.user().is_some(),
        );
        self.chrome.highlight_js_path = format!(
            "{}{}",
            self.base_path,
            crate::highlight_js::path(
                &state.site.highlight_js,
                &urls.current_hostname()?,
                &settings.get("highlighted_languages")?.to_s(),
                state.config.globals.get("asset_url_salt").unwrap_or(""),
            )
        );
        self.chrome.autohighlight_all_code = settings.get("autohighlight_all_code")?.truthy();
        let theme_id = Some(settings.get("default_theme_id")?.to_i());
        let sprite = crate::svg_sprite::for_site(&mut *conn, state, settings, theme_id).await?;
        self.chrome.svg_sprite_path = format!(
            "{}{}",
            self.base_path,
            crate::svg_sprite::path(&urls.current_hostname()?, theme_id, &sprite.version)
        );
        self.chrome.cooked_config = cooked_config(settings, &state.i18n)?;
        self.chrome.dates_attrs = dates_attrs(settings, &state.i18n)?;
        self.chrome.avatar_sizes = settings.get("avatar_sizes")?.to_s().to_string();
        self.chrome.text_size = settings.get("default_text_size")?.to_s();
        // The member's options the chrome reads: text size, local dates and
        // the composer's.
        let options: Option<ChromeOptions> = match guardian.user() {
            Some(user) => {
                sqlx::query_as(
                    "SELECT timezone, text_size_key, text_size_seq, default_calendar, \
                       composition_mode, hide_presence, color_scheme_id, dark_scheme_id, \
                       interface_color_mode \
                     FROM user_options WHERE user_id = $1",
                )
                .bind(user.id)
                .fetch_optional(&mut *conn)
                .await?
            }
            None => None,
        };
        let color_options = options.as_ref().map(|o| crate::color_mode::Options {
            color_scheme_id: o.color_scheme_id,
            dark_scheme_id: o.dark_scheme_id,
            interface_color_mode: o.interface_color_mode,
        });
        self.chrome.color_head = crate::color_mode::head(
            &mut *conn,
            settings,
            &state.i18n,
            &self.base_path,
            &crate::color_mode::cookies(),
            color_options.as_ref(),
        )
        .await?;
        // The theme targets' stylesheets of the default theme and its
        // components (the port draws the default theme everywhere).
        if let Some(theme_id) = theme_id.and_then(|id| i32::try_from(id).ok()) {
            // `supports_rtl`: a right-to-left locale (Rtl::LOCALES, the
            // request's I18n.locale) gets the `_rtl` targets, flipped by
            // Rtlcss.
            let mut locale = settings.get("default_locale")?.to_s();
            if let Some(user) = guardian.user()
                && settings.get("allow_user_locale")?.truthy()
            {
                let own: Option<Option<String>> =
                    sqlx::query_scalar("SELECT locale FROM users WHERE id = $1")
                        .bind(user.id)
                        .fetch_optional(&mut *conn)
                        .await?;
                if let Some(own) = own.flatten().filter(|l| !l.is_empty()) {
                    locale = own;
                }
            }
            let rtl = matches!(locale.as_str(), "ar" | "fa_IR" | "he" | "ug" | "ur");
            self.chrome.theme_stylesheets = crate::stylesheet::manager::link_tags(
                &mut *conn,
                &crate::stylesheet::manager::Cx {
                    config: &state.config,
                    settings,
                },
                theme_id,
                rtl,
            )
            .await?;
        }
        if let Some(o) = &options {
            self.chrome.save_timezone = o.timezone.as_deref().is_none_or(str::is_empty);
            // UserOption.text_sizes
            let sizes = ["normal", "larger", "largest", "smaller", "smallest"];
            if let Some(size) = usize::try_from(o.text_size_key)
                .ok()
                .and_then(|k| sizes.get(k))
            {
                self.chrome.text_size = size.to_string();
            }
            let cookie = self
                .viewer
                .as_ref()
                .and_then(|v| v.text_size_cookie.as_deref())
                .and_then(|c| c.split_once('|'));
            if let Some((size, seq)) = cookie
                && sizes.contains(&size)
                && crate::ruby::to_i(seq) >= i64::from(o.text_size_seq)
            {
                self.chrome.text_size = size.to_string();
            }
        }
        if crate::plugins::local_dates::enabled(settings)? {
            let (default_calendar, user_timezone) = match &options {
                Some(o) => (
                    Some(o.default_calendar),
                    o.timezone.clone().unwrap_or_default(),
                ),
                None => (None, String::new()),
            };
            // configureMoment's locale (crate::moment_locale), when it has
            // more than the formats: the request's I18n.locale, the
            // member's own under allow_user_locale (effective_locale).
            let mut locale = settings.get("default_locale")?.to_s();
            if let Some(user) = guardian.user()
                && settings.get("allow_user_locale")?.truthy()
            {
                let own: Option<Option<String>> =
                    sqlx::query_scalar("SELECT locale FROM users WHERE id = $1")
                        .bind(user.id)
                        .fetch_optional(&mut *conn)
                        .await?;
                if let Some(own) = own.flatten().filter(|l| !l.is_empty()) {
                    locale = own;
                }
            }
            let moment_locale = if crate::moment_locale::has_files(&locale) {
                crate::moment_locale::url(&self.base_path, &locale, &state.i18n)
            } else {
                String::new()
            };
            self.chrome.local_dates_attrs = local_dates_attrs(
                settings,
                &state.i18n,
                guardian.user().map(|u| u.username.as_str()),
                default_calendar,
                &moment_locale,
                &user_timezone,
            )?;
        }
        let color_selector = self.chrome.color_head.footer_button.clone();
        let sidebar_cx = |plugin_sections| {
            inputs.as_ref().map(|(_, member)| crate::sidebar::Context {
                i18n: &state.i18n,
                settings,
                base_path: &self.base_path,
                active: &active,
                member: member.as_ref(),
                plugin_sections,
                color_selector: &color_selector,
            })
        };
        // Chat's header icon and sidebar sections (chat-setup.js adds the
        // body's chat-enabled class).
        let chat = crate::plugins::chat::view::load(
            &mut *conn,
            state,
            settings,
            guardian,
            &self.base_path,
            &active,
            sidebar_cx("").as_ref(),
        )
        .await?;
        let chat_sections = chat
            .as_ref()
            .map(|c| c.sidebar_sections.clone())
            .unwrap_or_default();
        if let (Some((site, _)), Some(cx)) = (&inputs, sidebar_cx(&chat_sections))
            && hamburger
        {
            self.chrome.hamburger = crate::sidebar::hamburger(site, &cx)?;
        } else if let (Some((site, _)), Some(cx)) = (&inputs, sidebar_cx(&chat_sections)) {
            self.chrome.sidebar = crate::sidebar::render(site, &cx)?;
            if self.chrome.tracking.is_some() {
                self.chrome.live_param("sidebar", &active.key());
            }
            // Sidebar.gjs's bodyClass
            classes.push("has-sidebar-page".into());
        }
        if let Some(chat) = chat {
            classes.insert(0, "chat-enabled".into());
            // Kept current over the live stream.
            self.chrome.live_param("chat", &active.key());
            self.chrome.chat_header_icon = chat.header_icon;
            self.chrome.chat_back_indicator = chat.back_indicator;
            self.chrome.chat_drawer = crate::plugins::chat::drawer::script_attrs(
                &mut *conn,
                settings,
                guardian,
                &self.base_path,
                chat.starred,
            )
            .await?;
        }
        self.chrome.chat_drawer_outlet = crate::plugins::chat::enabled(settings)?;
        if let Some(user_id) = guardian.user_id() {
            self.chrome.can_create_topic = guardian.can_create_topic(&mut *conn, settings).await?;
            let draft_count: Option<i32> =
                sqlx::query_scalar("SELECT draft_count FROM user_stats WHERE user_id = $1")
                    .bind(user_id)
                    .fetch_optional(&mut *conn)
                    .await?;
            self.chrome.drafts_menu_trigger =
                drafts_menu_trigger(&state.i18n, i64::from(draft_count.unwrap_or(0)));
            self.chrome.composer = self
                .composer(&mut *conn, state, settings, guardian, options.as_ref())
                .await?;
        }
        self.chrome.body_classes = classes.join(" ");
        Ok(())
    }

    /// The member's composer, with the categories they may create topics
    /// in for its chooser.
    async fn composer(
        &mut self,
        conn: &mut PgConnection,
        state: &crate::AppState,
        settings: &SiteSettings,
        guardian: &crate::guardian::Guardian,
        options: Option<&ChromeOptions>,
    ) -> Result<String, crate::AppError> {
        let mut allowed = guardian
            .topic_create_allowed_category_ids(&mut *conn, settings)
            .await?;
        // The chooser leaves out uncategorized unless topics may go there.
        if !settings.get("allow_uncategorized_topics")?.truthy() {
            let uncategorized = settings.get("uncategorized_category_id")?.to_i() as i32;
            allowed.retain(|id| *id != uncategorized);
        }
        // Category#minimumRequiredTags: with required tag groups, the
        // larger of their counts' sum and the minimum; else the minimum.
        let rows: Vec<(i32, i32, Option<String>, i64)> = sqlx::query_as(
            "SELECT c.id, c.topic_count, c.description, \
               CASE WHEN EXISTS (SELECT 1 FROM category_required_tag_groups r WHERE r.category_id = c.id) \
                 THEN GREATEST((SELECT COALESCE(SUM(r.min_count), 0) FROM category_required_tag_groups r \
                                WHERE r.category_id = c.id), COALESCE(c.minimum_required_tags, 0)) \
                 ELSE GREATEST(COALESCE(c.minimum_required_tags, 0), 0) END::bigint \
             FROM categories c WHERE c.id = ANY($1)",
        )
        .bind(&allowed)
        .fetch_all(&mut *conn)
        .await?;
        let mut chooser = Vec::with_capacity(rows.len());
        for (id, topic_count, description, minimum_required_tags) in rows {
            chooser.push(crate::composer_view::ChooserCategory {
                id: i64::from(id),
                topic_count: i64::from(topic_count),
                // Category#description_text
                description_text: crate::categories::description_plain_text(
                    description.as_deref(),
                )?,
                minimum_required_tags,
            });
        }
        // composer.canEditTags for a new topic: site.can_tag_topics.
        let tag_chooser = if guardian.can_tag_topics(settings)? {
            Some(crate::composer_view::TagChooser {
                can_create: guardian.can_create_tag(settings)?,
                staff: guardian.is_staff(),
                max_tags_per_topic: settings.get("max_tags_per_topic")?.to_i(),
                max_tag_search_results: settings.get("max_tag_search_results")?.to_i(),
                max_tag_length: settings.get("max_tag_length")?.to_i(),
                force_lowercase: settings.get("force_lowercase_tags")?.truthy(),
                sort_alphabetically: settings.get("tags_sort_alphabetically")?.truthy(),
                prioritize_recent: settings.get("prioritize_recently_used_tags")?.truthy(),
                style: settings.get("tag_style")?.to_s(),
            })
        } else {
            None
        };
        let categories = crate::topic_list_view::categories(&mut *conn).await?;
        let list = crate::topic_list_view::ListContext {
            i18n: &state.i18n,
            base_path: &self.base_path,
            now: chrono::Utc::now(),
            categories: &categories,
            expand_all_pinned: false,
            hide_category: false,
            member_trust_level: guardian.user().map(|u| u.trust_level),
            settings: crate::topic_list_view::ListSettings::load(settings)?,
        };
        let default_category =
            Some(settings.get("default_composer_category")?.to_i()).filter(|id| *id > 0);
        let wasm = format!(
            "{}/assets/markdown.wasm?v={}",
            self.base_path,
            crate::routes::composer::MARKDOWN_WASM_VERSION
        );
        let render_settings = format!("{}/assets/markdown-settings.json", self.base_path);
        // allowPreview: not for the rich editor (composition_mode 1).
        let (allow_preview, hide_presence) = match guardian.user_id() {
            Some(_) => (
                options.map(|o| o.composition_mode) != Some(1),
                options.is_some_and(|o| o.hide_presence),
            ),
            None => (false, false),
        };
        // discourse-presence: what the composer and the topic's footer show
        // of who else is replying or editing.
        let presence = if crate::plugins::presence::enabled(settings)? {
            let js = |key: &str| {
                state
                    .i18n
                    .t(&format!("js.presence.{key}"))
                    .unwrap_or(key)
                    .to_string()
            };
            Some(serde_json::json!({
                "replying_one": js("replying.one"),
                "replying_other": js("replying.other"),
                "editing_one": js("editing.one"),
                "editing_other": js("editing.other"),
                "replying_to_topic_one": js("replying_to_topic.one"),
                "replying_to_topic_other": js("replying_to_topic.other"),
                "max_users_shown": settings.get("presence_max_users_shown")?.to_i(),
                "whisperer": guardian.is_whisperer(settings)?,
                "hide_presence": hide_presence,
                "user_id": guardian.user_id(),
                "profile_possessive": state
                    .i18n
                    .t("js.user.profile_possessive")
                    .unwrap_or("%{username}'s profile"),
            }))
        } else {
            None
        };
        // The toolbar's options menu: add-poll-ui-builder's condition is
        // staff or poll_create_allowed_groups.
        let can_create_poll = guardian.is_staff()
            || guardian.in_setting_groups(settings, "poll_create_allowed_groups")?;
        let popup_options = crate::composer_view::popup_options(settings, can_create_poll)?;
        let unported_popup = crate::composer_view::unported_popup_plugins(settings)?;
        let html = crate::composer_view::render(
            &list,
            &chooser,
            default_category,
            Some((
                &wasm,
                &render_settings,
                settings.get("max_oneboxes_per_post")?.to_i(),
            )),
            allow_preview,
            presence.as_ref(),
            crate::composer_view::UploadUi::for_user(
                &settings.get("authorized_extensions")?.to_s(),
                &settings.get("authorized_extensions_for_staff")?.to_s(),
                guardian.is_staff(),
            )
            .as_ref(),
            Some((&popup_options, &unported_popup)),
            tag_chooser.as_ref(),
            &crate::composer_view::RouteData {
                max_title_length: settings.get("max_topic_title_length")?.to_i(),
                dialog_src: crate::assets::url(&self.base_path, "dialog.js"),
                dialog_strings: dialog_strings(&state.i18n),
            },
        );
        self.categories = Some(categories);
        Ok(html)
    }
}

/// The member's user_options columns the page chrome reads.
#[derive(sqlx::FromRow)]
struct ChromeOptions {
    timezone: Option<String>,
    text_size_key: i32,
    text_size_seq: i32,
    default_calendar: i32,
    composition_mode: i32,
    hide_presence: bool,
    color_scheme_id: Option<i32>,
    dark_scheme_id: Option<i32>,
    interface_color_mode: i32,
}

pub struct CategoryBadge {
    pub name: String,
    pub color: String,
    pub url: String,
}

#[derive(Template)]
#[template(path = "latest.html")]
pub struct LatestPage {
    pub site_title: String,
    /// The `<title>` (document_title).
    pub title: String,
    pub site_description: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: Crawler,
    pub viewer: Option<Viewer>,
    pub bus_position: String,
    pub chrome: Chrome,
    /// The topic list's rows, rendered (topic_list_view).
    pub rows: Vec<String>,
    pub more_url: Option<String>,
    /// Set on category pages.
    pub heading: Option<CategoryHeading>,
    /// Set on tag pages (list.erb's tag breadcrumb).
    pub tag: Option<TagHeading>,
    /// A tag's page: the TagInfo panel its info button shows, hidden.
    pub tag_info: String,
    /// The list kept live (`latest`, `new`, `unread`), empty for none.
    pub live_filter: String,
    /// When the page was rendered, in milliseconds (live updates count the
    /// topics bumped after it).
    pub live_since: String,
    /// The navigation pills (`top_menu`); empty where the page has none.
    pub nav: Vec<NavItem>,
    /// The welcome banner (welcome_banner), empty where it is off.
    pub banner: String,
    /// The category and tag drops (breadcrumbs), empty where not drawn.
    pub breadcrumbs: String,
    /// Staff controls beside the new topic button (the categories page's
    /// new category button).
    pub admin_controls: String,
    /// After the new topic button on a category's list: the member's
    /// notification level in it (category_notifications); it also marks
    /// the navigation as a category's, in place of the heading.
    pub category_controls: String,
    /// A category's list: the category navigation instead of the heading.
    pub category_nav: bool,
    /// A top list (the list JSON's `for_period`): the PeriodChooser above
    /// the topics, empty on other lists.
    pub top_lists: String,
    /// DiscoveryTopics' `footer.topic-list-bottom`, drawn (list_bottom).
    pub list_bottom: String,
    /// TopicDismissButtons in the navigation controls (dismiss_buttons).
    pub dismiss_top: String,
    /// topic-dismiss.js' strings (dismiss_strings), empty without the
    /// buttons.
    pub dismiss_strings: String,
}

/// What DiscoveryTopics' footer reads besides the list itself: the list's
/// filter type (filterTypeForMode: `latest`, `new`, `unread`, `top`...),
/// /new's `subset` query param, and the category and tag the member's
/// tracking counts are taken in (newTopicsCount, newRepliesCount).
#[derive(Default)]
pub struct ListFooterScope<'a> {
    pub filter: &'a str,
    pub subset: Option<&'a str>,
    pub category_id: Option<i32>,
    pub no_subcategories: bool,
    pub tag_id: Option<i32>,
}

/// SvgDocumentsCheckmark, EmptyTopicFilter's image.
const DOCUMENTS_CHECKMARK: &str = include_str!("../templates/empty_states/documents_checkmark.svg");

/// DiscoveryTopics' `footer.topic-list-bottom`: DConditionalLoadingSpinner's
/// container (empty: the list is not loading more when drawn), then, once
/// the list is all loaded, TopicDismissButtons at the bottom (desktop, more
/// than five topics) and EmptyTopicFilter's education for a member whose
/// list is empty (showEmptyFilterEducationInFooter).
fn list_bottom(
    i18n: &I18n,
    base_path: &str,
    tracking: Option<&crate::topic_tracking_report::Tracking>,
    topics: usize,
    all_loaded: bool,
    scope: &ListFooterScope,
) -> String {
    use crate::topic_list_view::{escape, icon};
    use crate::topic_tracking_report::Kind;
    let t = |key: &str| {
        i18n.t(&format!("js.{key}"))
            .map(str::to_string)
            .unwrap_or_else(|| format!("[en.{key}]"))
    };
    let mut out =
        String::from("<footer class=\"topic-list-bottom\"><div class=\"loading-container\"></div>");
    if all_loaded {
        // showBottomDismissButtons, which TopicDismissButtons draws past
        // five topics.
        if !crate::mobile::mobile_view() {
            out.push_str(&dismiss_buttons(i18n, "bottom", tracking, topics, scope));
        }
        // The member check is `currentUser`; the tracking report is there
        // for every member.
        if let Some(tracking) = tracking.filter(|_| topics == 0) {
            let unified_new = tracking.unified_new;
            let new_filter = scope.filter == "new";
            let unread_filter = scope.filter == "unread";
            let title = if unread_filter {
                t("topics.none.education.unread")
            } else if new_filter && unified_new {
                t("topics.none.education.unified_new")
            } else if new_filter {
                t("topics.none.education.new")
            } else {
                t("topics.none.education.generic")
            };
            // ctaLabelWithAction: under unified new a subset with nothing
            // left offers the other one while it has some.
            let count = |kind| {
                tracking.count_scoped(
                    kind,
                    scope.category_id,
                    scope.no_subcategories,
                    scope.tag_id,
                )
            };
            let other_subset = match scope.subset.filter(|_| unified_new && new_filter) {
                Some("topics") if count(Kind::Unread) > 0 => {
                    Some(("replies", "topic.browse_new_replies"))
                }
                Some("replies") if count(Kind::New) > 0 => {
                    Some(("topics", "topic.browse_new_topics"))
                }
                _ => None,
            };
            let (cta_label, cta_data) = match other_subset {
                Some((subset, label)) => (t(label), format!(" data-subset=\"{subset}\"")),
                None => (t("topic.browse_latest_topics"), String::new()),
            };
            out.push_str(&format!(
                "<div class=\"empty-state__container --empty-topic-filter --with-image\"><div class=\"empty-state\"><div class=\"empty-state__image\">{DOCUMENTS_CHECKMARK}</div><div class=\"empty-state__title\" data-test-title=\"\">{}</div><div class=\"empty-state__cta\"><button class=\"btn btn-primary\" type=\"button\"{cta_data}><span class=\"d-button-label\">{}</span></button></div>",
                escape(&title),
                escape(&cta_label),
            ));
            if new_filter || unread_filter {
                let tip = i18n
                    .t_with(
                        "js.topics.none.education.topic_tracking_preferences",
                        &[("basePath", &escape(base_path))],
                    )
                    .unwrap_or_default();
                out.push_str(&format!(
                    "<div class=\"empty-state__tip\">{} {tip}</div>",
                    icon("circle-info", None)
                ));
            }
            out.push_str("</div></div>");
        }
    }
    out.push_str("</footer>");
    out
}

/// TopicDismissButtons at `position`: `top` in the navigation controls
/// (always drawn, empty unless there is something to dismiss), `bottom` in
/// the list's footer past five topics. The discovery controller's
/// showResetNew and showDismissRead: the new or unread list with topics.
/// On new, under unified new, the DComboButton (Dismiss all, or the
/// subset's label) whose menu offers "Dismiss and stop tracking" unless
/// the subset is topics; otherwise the classic "Dismiss New". On unread,
/// "Dismiss…", which opens DismissRead (topic-dismiss.js). The row
/// carries the list's category for the requests the buttons send.
/// Selecting topics (BulkSelectHelper) is not ported, so the
/// `_with_selected` labels are never drawn.
pub fn dismiss_buttons(
    i18n: &I18n,
    position: &str,
    tracking: Option<&crate::topic_tracking_report::Tracking>,
    topics: usize,
    scope: &ListFooterScope,
) -> String {
    use crate::topic_list_view::{escape, icon};
    if position == "bottom" && topics <= 5 {
        return String::new();
    }
    let t = |key: &str| {
        escape(
            i18n.t(&format!("js.{key}"))
                .unwrap_or_else(|| key.rsplit('.').next().unwrap_or(key)),
        )
    };
    // The lists' own checks read no member; only members reach new and
    // unread, and unified new is a member's.
    let tracking = tracking.filter(|_| topics > 0);
    let show_reset_new = scope.filter == "new" && tracking.is_some();
    let show_dismiss_read = scope.filter == "unread" && tracking.is_some();
    let mut out = format!("<div class=\"row dismiss-container-{position}\"");
    if let Some(id) = scope
        .category_id
        .filter(|_| show_reset_new || show_dismiss_read)
    {
        out.push_str(&format!(" data-category-id=\"{id}\""));
        if scope.no_subcategories {
            out.push_str(" data-no-subcategories=\"\"");
        }
    }
    out.push('>');
    if show_dismiss_read {
        out.push_str(&format!(
            "<button class=\"btn btn-default dismiss-read\" id=\"dismiss-topics-{position}\" title=\"{}\" type=\"button\"><span class=\"d-button-label\">{}</span></button>",
            t("topics.bulk.dismiss_tooltip"),
            t("topics.bulk.dismiss_button"),
        ));
    }
    if let Some(tracking) = tracking.filter(|_| show_reset_new) {
        if tracking.unified_new {
            let (label, has_menu) = match scope.subset {
                Some("topics") => (t("topics.bulk.dismiss_new_topics"), false),
                Some("replies") => (t("topics.bulk.dismiss_new_replies"), true),
                _ => (t("topics.bulk.dismiss_all"), true),
            };
            out.push_str(&format!(
                "<div class=\"d-combo-button{} topic-dismiss-buttons__combo\" role=\"group\"><button class=\"btn d-combo-button-button btn-default dismiss-read topic-dismiss-buttons__button\" id=\"dismiss-new-{position}\" type=\"button\"><span class=\"d-button-label\">{label}</span></button>",
                if has_menu { " --has-menu" } else { "" },
            ));
            if has_menu {
                out.push_str(&format!(
                    "<button aria-expanded=\"false\" aria-label=\"{}\" class=\"btn no-text btn-icon fk-d-menu__trigger dismiss-new-menu-trigger d-combo-button-menu btn-default dismiss-read topic-dismiss-buttons__menu\" id=\"dismiss-new-menu-{position}\" data-identifier=\"dismiss-new-menu\" data-trigger=\"\" type=\"button\">{}</button>",
                    t("topics.bulk.dismiss_new_menu"),
                    icon("chevron-down", None),
                ));
            }
            out.push_str("</div>");
        } else {
            out.push_str(&format!(
                "<button class=\"btn btn-icon-text btn-default dismiss-read\" id=\"dismiss-new-{position}\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
                icon("check", None),
                t("topics.bulk.dismiss_new"),
            ));
        }
    }
    out.push_str("</div>");
    out
}

/// The strings topic-dismiss.js shows (the stop tracking menu item and
/// DismissRead), by their js.* key.
pub fn dismiss_strings(i18n: &I18n) -> String {
    let strings: serde_json::Map<String, Value> = [
        "topics.bulk.dismiss_and_stop_tracking",
        "topics.bulk.dismiss_read",
        "topics.bulk.also_dismiss_topics",
        "topics.bulk.dismiss",
    ]
    .iter()
    .map(|key| {
        let text = i18n.t(&format!("js.{key}")).unwrap_or(key);
        ((*key).to_string(), Value::String(text.to_string()))
    })
    .collect();
    Value::Object(strings).to_string()
}

/// A navigation pill, as NavItem renders it.
pub struct NavItem {
    /// The filter: `latest`, `new`, `hot`, `categories`...
    pub name: String,
    pub label: String,
    /// `js.filters.<name>.help`
    pub title: String,
    pub href: String,
    pub active: bool,
    /// `hasIcon`: the unread pill.
    pub has_icon: bool,
}

/// BreadCrumbs for a list with no category or tag: the category drop and,
/// with tagging, the tag drop, closed (their select-kit headers).
pub fn breadcrumbs(i18n: &I18n, settings: &SiteSettings) -> Result<String, SettingError> {
    use crate::topic_list_view::{escape, icon};
    let t = |key: &str| i18n.t(&format!("js.{key}")).unwrap_or_default().to_string();
    let drop = |kind: &str, extra_class: &str, label: &str| {
        let label = escape(label);
        let aria = escape(
            &i18n
                .t_with("js.select_kit.filter_by", &[("name", &label)])
                .unwrap_or_default(),
        );
        format!(
            "<li><details class=\"select-kit single-select combobox combo-box {kind}-drop{extra_class}\">\
             <summary aria-label=\"{aria}\" name=\"{aria}\" data-name=\"{label}\" data-value=\"\" tabindex=\"0\" \
             class=\"select-kit-header single-select-header combo-box-header {kind}-drop-header\">\
             <div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"{label}\" title=\"{label}\">\
             <span class=\"name\">{label}</span></div>{}</div></summary><div class=\"select-kit-body\"></div></details></li>",
            icon("angle-right", Some("angle-icon"))
        )
    };
    // BreadCrumbs#hidden: in the mobile view, outside a category.
    let mut out = String::from(if crate::mobile::mobile_view() {
        "<ol class=\"category-breadcrumb hidden\">"
    } else {
        "<ol class=\"category-breadcrumb\">"
    });
    out.push_str(&drop(
        "category",
        " category-breadcrumb__category-selector",
        &t("categories.categories_label"),
    ));
    if settings.get("tagging_enabled")?.truthy() {
        out.push_str(&drop("tag", " tag_all", &t("tagging.selector_tags")));
    }
    out.push_str("</ol>");
    Ok(out)
}

/// A select-kit drop in the breadcrumbs: its header, showing `label` (a
/// name, or a category badge), with the selection's `value`.
fn breadcrumb_drop(
    i18n: &I18n,
    kind: &str,
    classes: &str,
    name: &str,
    label: &str,
    value: Option<i64>,
    li_style: Option<&str>,
) -> String {
    use crate::topic_list_view::{escape, icon};
    let name = escape(name);
    let aria = escape(
        &i18n
            .t_with("js.select_kit.filter_by", &[("name", &name)])
            .unwrap_or_default(),
    );
    let (summary_value, choice_value) = match value {
        Some(v) => (v.to_string(), format!(" data-value=\"{v}\"")),
        None => (String::new(), String::new()),
    };
    let li = match li_style {
        Some(style) => format!("<li style=\"{style}\">"),
        None => "<li>".to_string(),
    };
    format!(
        "{li}<details class=\"select-kit single-select combobox combo-box {kind}-drop{classes}\">\
         <summary aria-label=\"{aria}\" name=\"{aria}\" data-name=\"{name}\" data-value=\"{summary_value}\" tabindex=\"0\" \
         class=\"select-kit-header single-select-header combo-box-header {kind}-drop-header\">\
         <div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"{name}\"{choice_value} title=\"{name}\">\
         <span class=\"name\">{label}</span></div>{}</div></summary><div class=\"select-kit-body\"></div></details></li>",
        icon("angle-right", Some("angle-icon"))
    )
}

/// BreadCrumbs on a tag's list: the category drop, closed, then the tag
/// drop with the tag selected.
pub fn tag_breadcrumbs(i18n: &I18n, tag_name: &str, tag_id: i32) -> String {
    use crate::topic_list_view::escape;
    let t = |key: &str| i18n.t(&format!("js.{key}")).unwrap_or_default().to_string();
    let categories = t("categories.categories_label");
    let mut out = String::from(if crate::mobile::mobile_view() {
        "<ol class=\"category-breadcrumb hidden\">"
    } else {
        "<ol class=\"category-breadcrumb\">"
    });
    out.push_str(&breadcrumb_drop(
        i18n,
        "category",
        " category-breadcrumb__category-selector",
        &categories,
        &escape(&categories),
        None,
        None,
    ));
    out.push_str(&breadcrumb_drop(
        i18n,
        "tag",
        &format!(" has-selection tag-{}", escape(&tag_name.to_lowercase())),
        tag_name,
        &escape(tag_name),
        Some(i64::from(tag_id)),
        None,
    ));
    out.push_str("</ol>");
    out
}

/// The navigation pills on a tag's list: the top menu's filters under
/// the tag (`/tag/:slug/:id/l/:filter`), categories to /categories.
pub fn tag_nav_items(
    i18n: &I18n,
    settings: &SiteSettings,
    base_path: &str,
    tag_url: &str,
    tag_id: i32,
    active: &str,
    tracking: Option<&crate::topic_tracking_report::Tracking>,
) -> Result<Vec<NavItem>, SettingError> {
    Ok(nav_items(i18n, settings, base_path, active, tracking)?
        .into_iter()
        .map(|mut item| {
            if item.name != "categories" {
                item.href = format!("{tag_url}/l/{}", item.name);
                // lookupCount within the tag.
                let count = match tracking {
                    Some(_) if item.name == "latest" && !crate::mobile::mobile_view() => 0,
                    Some(t) => t.lookup_tag(&item.name, tag_id),
                    None => 0,
                };
                if count > 0 {
                    item.label = i18n
                        .t_count(
                            &format!("js.filters.{}.title_with_count", item.name),
                            count,
                            &[],
                        )
                        .unwrap_or(item.label);
                } else {
                    item.label = i18n
                        .t(&format!("js.filters.{}.title", item.name))
                        .map(str::to_string)
                        .unwrap_or(item.label);
                }
            }
            item
        })
        .collect())
}

/// BreadCrumbs on a category's list: a drop per level of the category's
/// ancestry (its own selected), then one for its subcategories when it
/// has any (`no subcategories` on its /none list), then the tag drop.
pub fn category_breadcrumbs(
    list: &crate::topic_list_view::ListContext,
    settings: &SiteSettings,
    category_id: i64,
    no_subcategories: bool,
) -> Result<String, SettingError> {
    use crate::topic_list_view::{category_badge_html, escape};
    let i18n = list.i18n;
    let t = |key: &str| i18n.t(&format!("js.{key}")).unwrap_or_default().to_string();
    // category.ancestors: from the top level down to the category.
    let mut ancestors = Vec::new();
    let mut at = list.categories.get(&category_id);
    while let Some(c) = at {
        ancestors.insert(0, c);
        at = c.parent_id.and_then(|id| list.categories.get(&id));
    }
    let has_children = |id: i64| list.categories.values().any(|c| c.parent_id == Some(id));
    let mut out = String::from("<ol class=\"category-breadcrumb\">");
    let parents = std::iter::once(None).chain(ancestors.iter().map(|c| Some(*c)));
    let selected = ancestors
        .iter()
        .map(|c| Some(*c))
        .chain(std::iter::once(None));
    for (parent, category) in parents.zip(selected) {
        if parent.is_some_and(|p| !has_children(p.id)) {
            continue;
        }
        let side = if parent.is_some() {
            " category-breadcrumb__subcategory-selector"
        } else {
            " category-breadcrumb__category-selector"
        };
        out.push_str(&match category {
            Some(c) => {
                let badge = category_badge_html(list, c, false, true);
                // categoryVariables
                let mut style = format!(
                    "--category-badge-color: #{};--category-badge-text-color: #{};",
                    c.color, c.text_color
                );
                if let Some(p) = c.parent_id.and_then(|id| list.categories.get(&id)) {
                    style.push_str(&format!(
                        "--parent-category-badge-color: #{};--parent-category-badge-text-color: #{};",
                        p.color, p.text_color
                    ));
                }
                breadcrumb_drop(
                    i18n,
                    "category",
                    &format!(" has-selection{side}"),
                    &c.name,
                    &badge,
                    Some(c.id),
                    Some(&style),
                )
            }
            None if no_subcategories => {
                let label = t("categories.no_subcategories");
                breadcrumb_drop(
                    i18n,
                    "category",
                    &format!(" has-selection{side}"),
                    &label,
                    &escape(&label),
                    None,
                    None,
                )
            }
            None => {
                let label = if parent.is_some() {
                    t("categories.subcategories_label")
                } else {
                    t("categories.categories_label")
                };
                breadcrumb_drop(i18n, "category", side, &label, &escape(&label), None, None)
            }
        });
    }
    if settings.get("tagging_enabled")?.truthy() {
        let label = t("tagging.selector_tags");
        out.push_str(&breadcrumb_drop(
            i18n,
            "tag",
            " tag_all",
            &label,
            &escape(&label),
            None,
            None,
        ));
    }
    out.push_str("</ol>");
    Ok(out)
}

/// NavItem.buildList for a category's list: the top menu's filters but
/// `categories`, each at the category's path (`/none` kept), counted in
/// the category; then the plugins' items (discourse-topic-voting's Votes
/// and My Votes on a voting category, before Top).
#[allow(clippy::too_many_arguments)]
pub fn category_nav_items(
    i18n: &I18n,
    settings: &SiteSettings,
    category_path: &str,
    category_id: i32,
    no_subcategories: bool,
    active: &str,
    tracking: Option<&crate::topic_tracking_report::Tracking>,
    can_vote: bool,
) -> Result<Vec<NavItem>, SettingError> {
    let mut items: Vec<NavItem> = nav_items(i18n, settings, "", active, tracking)?
        .into_iter()
        .filter(|item| item.name != "categories")
        .map(|mut item| {
            item.href = format!("{category_path}/l/{}", item.name);
            item.label =
                category_nav_label(i18n, &item.name, tracking, category_id, no_subcategories);
            item
        })
        .collect();
    if can_vote && !no_subcategories {
        if let Some(hot) = items.iter_mut().find(|i| i.name == "hot") {
            hot.title = i18n
                .t("js.topic_voting.hot_nav_help")
                .unwrap_or_default()
                .to_string();
        }
        let mut voting = vec![("votes", "order=votes")];
        if tracking.is_some() {
            voting.push(("my_votes", "state=my_votes"));
        }
        for (name, param) in voting {
            let item = NavItem {
                name: name.to_string(),
                label: i18n
                    .t(&format!("js.filters.{name}.title"))
                    .unwrap_or(name)
                    .to_string(),
                title: i18n
                    .t(&format!("js.filters.{name}.help"))
                    .unwrap_or_default()
                    .to_string(),
                href: format!("{category_path}/l/latest?{param}"),
                active: false,
                has_icon: false,
            };
            match items.iter().position(|i| i.name == "top") {
                Some(at) => items.insert(at, item),
                None => items.push(item),
            }
        }
    }
    Ok(items)
}

/// NavItem#displayName within a category (`lookupCount` with the
/// category, and noSubcategories on its /none list).
fn category_nav_label(
    i18n: &I18n,
    name: &str,
    tracking: Option<&crate::topic_tracking_report::Tracking>,
    category_id: i32,
    no_subcategories: bool,
) -> String {
    let count = match tracking {
        Some(_) if name == "latest" && !crate::mobile::mobile_view() => 0,
        Some(t) => t.lookup_in(name, category_id, no_subcategories),
        None => 0,
    };
    let title = || {
        i18n.t(&format!("js.filters.{name}.title"))
            .unwrap_or(name)
            .to_string()
    };
    if count > 0 {
        i18n.t_count(&format!("js.filters.{name}.title_with_count"), count, &[])
            .unwrap_or_else(title)
    } else {
        title()
    }
}

/// CategoryNotificationsTracking for a member: the trigger showing their
/// level in the category, and its menu (hidden until opened), each level
/// saved to /category/:id/notifications.
pub fn category_notifications(
    i18n: &I18n,
    base_path: &str,
    category_id: i64,
    level: i64,
) -> String {
    notifications_tracking(
        i18n,
        "category",
        &format!("hx-post=\"{base_path}/category/{category_id}/notifications\""),
        "notification_level",
        level,
    )
}

/// TagNotificationsTracking for a member: as the category's, each level
/// saved to PUT /tag/:id/notifications.
pub fn tag_notifications(i18n: &I18n, base_path: &str, tag_id: i32, level: i64) -> String {
    notifications_tracking(
        i18n,
        "tag",
        &format!("hx-put=\"{base_path}/tag/{tag_id}/notifications\""),
        "tag_notification[notification_level]",
        level,
    )
}

/// The notifications tracking DMenu: `kind` is the trigger's class prefix
/// (`category`, `tag`), `request` the levels' htmx request attribute and
/// `param` the level's param name.
fn notifications_tracking(
    i18n: &I18n,
    kind: &str,
    request: &str,
    param: &str,
    level: i64,
) -> String {
    use crate::post_view::d_icon;
    use crate::topic_list_view::escape;
    const LEVELS: [(i64, &str, &str); 5] = [
        (3, "watching", "d-watching"),
        (2, "tracking", "d-tracking"),
        (4, "watching_first_post", "d-watching-first"),
        (1, "regular", "d-regular"),
        (0, "muted", "d-muted"),
    ];
    let t = |key: &str| i18n.t(&format!("js.{key}")).unwrap_or_default().to_string();
    let scope = if kind == "tag" { "tagging" } else { "category" };
    let title = |key: &str| t(&format!("{scope}.notifications.{key}.title"));
    let (_, key, icon_name) = LEVELS
        .iter()
        .find(|(l, _, _)| *l == level)
        .copied()
        .unwrap_or(LEVELS[3]);
    let tooltip = i18n
        .t_with(
            "js.notifications_tracking.tooltip",
            &[("level", &title(key))],
        )
        .unwrap_or_default();
    let items: String = LEVELS
        .iter()
        .map(|(l, key, icon_name)| {
            format!(
                "<li class=\"dropdown-menu__item\"><button class=\"btn no-text notifications-tracking-btn{}\" data-level-id=\"{l}\" \
                 data-level-name=\"{key}\" data-title=\"{}\" data-tooltip=\"{}\" {request} hx-vals='{{\"{param}\": {l}}}' \
                 hx-swap=\"none\" type=\"button\"><div class=\"notifications-tracking-btn__icons\">{}</div><div class=\"notifications-tracking-btn__texts\">\
                 <span class=\"notifications-tracking-btn__label\">{}</span><span class=\"notifications-tracking-btn__description\">{}</span></div></button></li>",
                if *l == level { " -selected" } else { "" },
                escape(&title(key)),
                escape(&i18n
                    .t_with("js.notifications_tracking.tooltip", &[("level", &title(key))])
                    .unwrap_or_default()),
                d_icon(icon_name, None),
                escape(&title(key)),
                escape(&t(&format!("{scope}.notifications.{key}.description"))),
            )
        })
        .collect();
    format!(
        "<button class=\"btn btn-default no-text fk-d-menu__trigger notifications-tracking-trigger btn-default btn-icon \
         notifications-tracking-trigger-btn {kind}-notifications-tracking\" title=\"{}\" aria-expanded=\"false\" \
         data-identifier=\"notifications-tracking\" data-trigger=\"\" data-level-id=\"{level}\" data-level-name=\"{key}\">{}</button>\
         <div class=\"fk-d-menu notifications-tracking-content -animated\" data-content=\"\" data-identifier=\"notifications-tracking\" \
         role=\"dialog\" data-strategy=\"absolute\" data-placement=\"bottom-end\" hidden><div class=\"fk-d-menu__inner-content\">\
         <ul class=\"dropdown-menu\">{items}</ul></div></div>",
        escape(&tooltip),
        d_icon(icon_name, None)
    )
}

/// The WelcomeBanner component for a discovery list (`filter`: latest,
/// categories...), above the topic content: empty where the settings keep
/// it off this page.
pub fn welcome_banner(
    i18n: &I18n,
    settings: &SiteSettings,
    viewer: Option<&Viewer>,
    base_path: &str,
    filter: &str,
) -> Result<String, crate::AppError> {
    use crate::topic_list_view::escape;
    if !settings.get("enable_welcome_banner")?.truthy() {
        return Ok(String::new());
    }
    if settings.get("welcome_banner_location")?.to_s() != "above_topic_content" {
        return Err(crate::Unsupported("the welcome banner below the site header").into());
    }
    let top_menu = settings.get("top_menu")?.to_s();
    let shown = match settings
        .get("welcome_banner_page_visibility")?
        .to_s()
        .as_ref()
    {
        "top_menu_pages" => top_menu.split('|').any(|item| item == filter),
        "homepage" => top_menu.split('|').next() == Some(filter),
        "discovery" | "all_pages" => true,
        _ => false,
    };
    if !shown {
        return Ok(String::new());
    }
    let t = |key: &str| i18n.t(&format!("js.{key}")).unwrap_or_default().to_string();
    let site_name = settings.get("title")?.to_s();
    let (header, member) = match viewer {
        None => (
            i18n.t_with(
                "js.welcome_banner.header.anonymous_members",
                &[("site_name", &escape(&site_name))],
            ),
            "anonymous_members",
        ),
        Some(v) => {
            // prioritizeNameFallback
            let prioritize_name = settings.get("enable_names")?.truthy()
                && !settings.get("prioritize_username_in_ux")?.truthy();
            let display = match v
                .name
                .as_deref()
                .filter(|n| prioritize_name && !n.trim().is_empty())
            {
                Some(name) => name,
                None => v.username.as_str(),
            };
            let key = if v.first_visit {
                "js.welcome_banner.header.new_members"
            } else {
                "js.welcome_banner.header.logged_in_members"
            };
            (
                i18n.t_with(
                    key,
                    &[
                        ("site_name", &escape(&site_name)),
                        ("preferred_display_name", &escape(display)),
                    ],
                ),
                "logged_in_members",
            )
        }
    };
    let subheader = t(&format!("welcome_banner.subheader.{member}"));
    let image = settings.get("welcome_banner_image")?.to_s();
    let text_color = settings.get("welcome_banner_text_color")?.to_s();
    let (bg_class, bg_style, color_style) = if image.is_empty() {
        (String::new(), String::new(), String::new())
    } else {
        (
            " --with-bg-img".to_string(),
            format!(" style=\"background-image:url({});\"", escape(&image)),
            if text_color.is_empty() {
                String::new()
            } else {
                format!(" style=\"color:{};\"", escape(&text_color))
            },
        )
    };
    let advanced = escape(&t("search.open_advanced"));
    let placeholder = escape(&t("welcome_banner.search_placeholder"));
    let icon = |name: &str| crate::topic_list_view::icon(name, None);
    let mut out = format!(
        "<div class=\"welcome-banner --location-above-topic-content{bg_class}\">\
         <div class=\"custom-search-banner-wrap welcome-banner__wrap\"{bg_style}>\
         <div class=\"welcome-banner__title\"{color_style}>{}",
        header.unwrap_or_default()
    );
    if !subheader.is_empty() {
        out.push_str(&format!(
            "<p class=\"welcome-banner__subheader\">{subheader}</p>"
        ));
    }
    out.push_str(&format!(
        "</div><div class=\"search-menu welcome-banner__search-menu\">\
         <a class=\"btn no-text btn-icon search-icon\" href=\"{base_path}/search?expanded=true\" title=\"{advanced}\">{}<span aria-hidden=\"true\">\u{200b}</span></a>\
         <div class=\"search-menu-container menu-panel-results\"><div class=\"search-input-wrapper\">\
         <div class=\"search-input search-input--welcome-banner\">\
         <input aria-label=\"{}\" autocomplete=\"off\" class=\"search-term__input\" enterkeyhint=\"search\" id=\"welcome-banner-search-input\" placeholder=\"{placeholder}\" value=\"\" type=\"search\">\
         <div class=\"searching\"><button class=\"btn no-text btn-icon show-advanced-search btn-transparent\" title=\"{advanced}\" type=\"button\">{}<span aria-hidden=\"true\">\u{200b}</span></button></div>\
         </div></div></div></div></div></div>",
        icon("magnifying-glass"),
        escape(&t("search.title")),
        icon("sliders")
    ));
    Ok(out)
}

/// The pills of the top-level lists (NavItem.buildList): `top_menu` in
/// order, the ones that need an account only for a member (who has
/// `tracking`), unread folded into new with unified new, and the member's
/// counts in the labels. A list the menu lacks gets no pill: buildList
/// would add it from `args.filterMode`, but DNavigation passes
/// `filterType`, so it adds nothing.
pub fn nav_items(
    i18n: &I18n,
    settings: &SiteSettings,
    base_path: &str,
    active: &str,
    tracking: Option<&crate::topic_tracking_report::Tracking>,
) -> Result<Vec<NavItem>, SettingError> {
    const MEMBERS_ONLY: [&str; 5] = ["new", "unread", "read", "posted", "bookmarks"];
    let unified_new = tracking.is_some_and(|t| t.unified_new);
    let top_menu = settings.get("top_menu")?.to_s();
    let names: Vec<&str> = top_menu
        .split('|')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .filter(|name| !(unified_new && *name == "unread"))
        .collect();
    Ok(names
        .into_iter()
        .filter(|name| tracking.is_some() || !MEMBERS_ONLY.contains(name))
        .map(|name| NavItem {
            name: name.to_string(),
            label: nav_label(i18n, name, tracking),
            title: i18n
                .t(&format!(
                    "js.filters.{}.help",
                    if name == "new" && unified_new {
                        "unified_new"
                    } else {
                        name
                    }
                ))
                .unwrap_or_default()
                .to_string(),
            href: format!("{base_path}/{name}"),
            active: name == active,
            has_icon: name == "unread",
        })
        .collect())
}

/// NavigationBar#selectedNavItem's label (the mobile menu's trigger): the
/// active pill, else the first.
pub fn selected_nav_label(nav: &[NavItem]) -> &str {
    nav.iter()
        .find(|i| i.active)
        .or(nav.first())
        .map(|i| i.label.as_str())
        .unwrap_or_default()
}

/// NavItem#displayName: `title_with_count` while the member has topics in
/// the list (never for latest on desktop), else `title`.
pub fn nav_label(
    i18n: &I18n,
    name: &str,
    tracking: Option<&crate::topic_tracking_report::Tracking>,
) -> String {
    // Latest counts only in the mobile view.
    let count = match tracking {
        Some(_) if name == "latest" && !crate::mobile::mobile_view() => 0,
        Some(t) => t.lookup(name),
        None => 0,
    };
    let title = || {
        i18n.t(&format!("js.filters.{name}.title"))
            .unwrap_or(name)
            .to_string()
    };
    if count > 0 {
        i18n.t_count(&format!("js.filters.{name}.title_with_count"), count, &[])
            .unwrap_or_else(title)
    } else {
        title()
    }
}

/// The document title Ember sets once the route loads
/// (ApplicationRoute#_collectTitleTokens): the route's titleToken, if it
/// has one, the site title, and on the front page the short site
/// description when there is one, joined with " - ".
pub fn document_title(token: Option<&str>, site_title: &str, short_description: &str) -> String {
    let mut tokens: Vec<&str> = token.into_iter().collect();
    tokens.push(site_title);
    if !short_description.is_empty() {
        tokens.push(short_description);
    }
    tokens.join(" - ")
}

impl Chrome {
    /// `i18n("filters.<filter>.title")`, a `/` in the filter read as `.`.
    fn filter_title(&self, filter: &str) -> String {
        self.t(&format!("filters.{}.title", filter.replacen('/', ".", 1)))
    }

    fn t_args(&self, key: &str, args: &[(&str, &str)]) -> String {
        self.i18n()
            .t_with(&format!("js.{key}"), args)
            .unwrap_or_else(|| format!("[en.{key}]"))
    }

    /// BuildTopicRoute#titleToken: none for the homepage's own list.
    pub fn topics_title_token(&self, filter: &str, homepage: &str) -> Option<String> {
        (filter != homepage)
            .then(|| self.t_with("filters.with_topics", "filter", self.filter_title(filter)))
    }

    /// BuildCategoryRoute#titleToken: a subcategory's name after its
    /// parent's, `parent/child`.
    pub fn category_title_token(&self, filter: &str, name: &str, parent: Option<&str>) -> String {
        let category = match parent {
            Some(p) => format!("{p}/{name}"),
            None => name.to_string(),
        };
        self.t_args(
            "filters.with_category",
            &[
                ("filter", &self.filter_title(filter)),
                ("category", &category),
            ],
        )
    }

    /// TagShowRoute#titleToken: `tag` None for the untagged (`none`) list.
    pub fn tag_title_token(
        &self,
        filter: &str,
        tag: Option<&str>,
        category: Option<&str>,
    ) -> String {
        let filter = self.filter_title(filter);
        match (tag, category) {
            (Some(tag), Some(category)) => self.t_args(
                "tagging.filters.with_category",
                &[("filter", &filter), ("tag", tag), ("category", category)],
            ),
            (Some(tag), None) => self.t_args(
                "tagging.filters.without_category",
                &[("filter", &filter), ("tag", tag)],
            ),
            (None, Some(category)) => self.t_args(
                "tagging.filters.untagged_with_category",
                &[("filter", &filter), ("category", category)],
            ),
            (None, None) => self.t_args(
                "tagging.filters.untagged_without_category",
                &[("filter", &filter)],
            ),
        }
    }
}

pub struct TagHeading {
    pub name: String,
    pub url: String,
}

#[derive(Template)]
#[template(path = "topic.html")]
pub struct TopicPage {
    pub site_title: String,
    pub site_description: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: Crawler,
    pub viewer: Option<Viewer>,
    pub bus_position: String,
    pub chrome: Chrome,
    pub title: String,
    pub canonical_url: String,
    /// The topic's archetype, the class of DiscourseTopic's element.
    pub archetype: String,
    /// `#topic-title`'s title wrapper (post_view::topic_title).
    pub title_html: String,
    /// The topic in the composer's reply-where dialog, for a member
    /// (post_view::reply_where_topic); empty for a visitor.
    pub reply_where: String,
    /// The docked header's minimized logo and topic info, as a template
    /// static/js/header-topic-info.js docks (header_topic_view).
    pub header_topic_info: String,
    /// The posts as Ember renders them (post_view::stream).
    pub posts: Vec<String>,
    /// Around the posts (post_view): the bottom topic map, the timeline,
    /// the footer buttons and the suggested topics.
    pub bottom_map: String,
    pub timeline: String,
    pub footer_buttons: String,
    pub more_topics: String,
    /// The topic admin menu's content for its triggers and the add
    /// participant modal's config (each empty without one).
    pub topic_menus: String,
    pub prev_url: Option<String>,
    pub next_url: Option<String>,
    pub topic_id: i64,
    /// The last page: live updates append new posts here.
    pub live: bool,
    /// discourse-reactions is on: its script runs the reaction controls.
    pub reactions: bool,
    /// discourse-solved is on: its script runs the Solved button and the
    /// accepted answers.
    pub solved: bool,
    /// `currentUser.ignored_users`, comma separated, whose quotes
    /// quote-controls.js hides.
    pub ignored_users: String,
    /// quote-button.js's config (`quote_button_config`).
    pub quote_config: String,
    /// dialog.js's strings, for a message whose participants the viewer
    /// may remove (removing oneself asks first) and for a member's polls
    /// (their errors); empty otherwise.
    pub dialog_strings: String,
    /// discourse-poll is on: its widget's script and strings.
    pub poll_scripts: String,
}

/// `categories` rows the pages link to.
#[derive(sqlx::FromRow)]
pub(crate) struct CategoryRow {
    id: i32,
    name: String,
    color: String,
    slug: String,
    parent_category_id: Option<i32>,
}

pub(crate) async fn categories(conn: &mut PgConnection) -> Result<Vec<CategoryRow>, sqlx::Error> {
    sqlx::query_as("SELECT id, name, color, slug, parent_category_id FROM categories")
        .fetch_all(conn)
        .await
}

/// `Category#url`: parent slug first for subcategories.
fn category_url(base_path: &str, cats: &[CategoryRow], c: &CategoryRow) -> String {
    match c
        .parent_category_id
        .and_then(|p| cats.iter().find(|x| x.id == p))
    {
        Some(parent) => format!("{base_path}/c/{}/{}/{}", parent.slug, c.slug, c.id),
        None => format!("{base_path}/c/{}/{}", c.slug, c.id),
    }
}

pub(crate) fn badge(
    base_path: &str,
    cats: &[CategoryRow],
    id: Option<i64>,
) -> Option<CategoryBadge> {
    let c = cats.iter().find(|c| Some(i64::from(c.id)) == id)?;
    Some(CategoryBadge {
        name: c.name.clone(),
        color: c.color.clone(),
        url: category_url(base_path, cats, c),
    })
}

/// "Sep 30, 2026" from a Rails ISO timestamp; the raw string if unparsable.
fn date(iso: &str) -> String {
    NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%S%.3fZ")
        .map(|t| t.format("%b %-d, %Y").to_string())
        .unwrap_or_else(|_| iso.to_string())
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

/// How latest_page draws a list's rows.
#[derive(Clone, Copy, Default)]
pub struct ListRows {
    /// A category's or a tag's list, which shows every pinned topic's
    /// excerpt.
    pub expand_all_pinned: bool,
    /// TopicList#hideCategory: the rows without their category badge.
    pub hide_category: bool,
    /// A list in the default order, which marks where the member's last
    /// visit was (TopicList#lastVisitedTopic).
    pub highlight_last_visited: bool,
}

/// The latest page from the /latest.json document, its rows drawn as
/// `shape` says; `footer` the list's filter and scope, for its footer
/// (list_bottom).
pub async fn latest_page(
    conn: &mut PgConnection,
    mut site: Site,
    list: &Value,
    i18n: &I18n,
    settings: &SiteSettings,
    shape: ListRows,
    footer: &ListFooterScope<'_>,
) -> Result<LatestPage, HtmlError> {
    let categories = site.list_categories(conn).await?;
    let cx = crate::topic_list_view::ListContext {
        i18n,
        base_path: &site.base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: shape.expand_all_pinned,
        hide_category: shape.hide_category,
        member_trust_level: site.viewer.as_ref().map(|v| v.trust_level),
        settings: crate::topic_list_view::ListSettings::load(settings)?,
    };
    let users = list["users"].as_array().cloned().unwrap_or_default();
    let previous_visit_at = site
        .viewer
        .as_ref()
        .and_then(|v| v.previous_visit_at)
        .filter(|_| shape.highlight_last_visited);
    let rows: Vec<String> = list["topic_list"]["topics"]
        .as_array()
        .map(|topics| {
            let last_visited = previous_visit_at
                .and_then(|at| crate::topic_list_view::last_visited_topic(topics, at));
            topics
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    if last_visited == Some(i) {
                        crate::topic_list_view::last_visited_row(&cx, t, &users)
                    } else {
                        crate::topic_list_view::row(&cx, t, &users)
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    // DiscoveryTopics on a top list: `@period` is the list's for_period.
    let top_lists = list["topic_list"]["for_period"]
        .as_str()
        .map(|period| crate::directory_view::period_chooser(&cx, period))
        .unwrap_or_default();
    // Discovery topics hides the footer while the list can load more.
    if list["topic_list"]["more_topics_url"].is_string() {
        site.chrome.show_footer = false;
    }
    let list_bottom = list_bottom(
        i18n,
        &site.base_path,
        site.chrome.tracking.as_ref(),
        rows.len(),
        !list["topic_list"]["more_topics_url"].is_string(),
        footer,
    );
    let dismiss_top = dismiss_buttons(
        i18n,
        "top",
        site.chrome.tracking.as_ref(),
        rows.len(),
        footer,
    );
    let dismiss_strings = if dismiss_top.contains("<button") {
        dismiss_strings(i18n)
    } else {
        String::new()
    };
    Ok(LatestPage {
        title: site.site_title.clone(),
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        site_description: site.site_description,
        lang: site.lang,
        base_path: site.base_path,
        crawler: Crawler::default(),
        rows,
        heading: None,
        tag: None,
        tag_info: String::new(),
        live_filter: String::new(),
        live_since: String::new(),
        nav: Vec::new(),
        banner: String::new(),
        breadcrumbs: String::new(),
        admin_controls: String::new(),
        category_controls: String::new(),
        category_nav: false,
        top_lists,
        list_bottom,
        dismiss_top,
        dismiss_strings,
        more_url: list["topic_list"]["more_topics_url"]
            .as_str()
            .map(str::to_string),
    })
}

/// User#pmPath: a message the viewer is on goes to their inbox, one they
/// see through a group to that group's (the first of the message's groups
/// they are in); None for a topic, an anonymous reader, or a message to
/// none of their groups.
pub(crate) async fn pm_path(
    conn: &mut PgConnection,
    view: &Value,
    base: &str,
    viewer: Option<&str>,
) -> Result<Option<String>, sqlx::Error> {
    let Some(username) = viewer.filter(|_| view["archetype"] == "private_message") else {
        return Ok(None);
    };
    let lower = username.to_lowercase();
    let details = &view["details"];
    let groups = details["allowed_groups"].as_array();
    let direct = details["allowed_users"].as_array().is_some_and(|users| {
        users.iter().any(|u| {
            u["username"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(username))
        })
    });
    let Some(groups) = groups.filter(|_| !direct) else {
        return Ok(Some(format!("{base}/u/{lower}/messages")));
    };
    let member_of: Vec<i32> = sqlx::query_scalar(
        "SELECT gu.group_id FROM group_users gu JOIN users u ON u.id = gu.user_id \
         WHERE u.username_lower = $1",
    )
    .bind(&lower)
    .fetch_all(&mut *conn)
    .await?;
    Ok(groups
        .iter()
        .find(|g| {
            g["id"]
                .as_i64()
                .is_some_and(|id| member_of.contains(&(id as i32)))
        })
        .and_then(|g| g["name"].as_str())
        .map(|name| format!("{base}/u/{lower}/messages/group/{name}")))
}

/// The topic's title block (`#topic-title`'s contents): a message's glyph
/// for `member` (the viewer's username and whether they can send
/// messages), its title, statuses, category and tags, and
/// discourse-topic-voting's outlet for `voter` (the viewer's id and trust
/// level, when they may vote).
#[allow(clippy::too_many_arguments)]
pub async fn topic_title_html(
    conn: &mut sqlx::PgConnection,
    i18n: &I18n,
    settings: &SiteSettings,
    list: &crate::topic_list_view::ListContext<'_>,
    view: &Value,
    topic_url: &str,
    member: Option<(&str, bool)>,
    voter: Option<(i32, i32)>,
) -> Result<String, HtmlError> {
    let can_send_pms = member.is_some_and(|(_, can)| can);
    let inbox = if can_send_pms {
        pm_path(conn, view, list.base_path, member.map(|(u, _)| u)).await?
    } else {
        None
    };
    let glyph = crate::post_view::private_message_glyph(list, view, can_send_pms, inbox.as_deref());
    let mut title_html = crate::post_view::topic_title(list, view, topic_url, &glyph);
    // discourse-topic-voting's topic-title outlet.
    if view["can_vote"] == serde_json::Value::Bool(true) {
        let voter = match voter {
            Some((id, trust_level)) => {
                let votes =
                    crate::plugins::topic_voting::UserVotes::load(conn, settings, id, trust_level)
                        .await?;
                Some(crate::plugins::topic_voting::view::Voter { votes })
            }
            None => None,
        };
        title_html.push_str(&crate::plugins::topic_voting::view::title_voting(
            i18n,
            list.base_path,
            settings.get("topic_voting_show_who_voted")?.truthy(),
            view,
            voter.as_ref(),
        ));
    }
    Ok(title_html)
}

/// CurrentUserSerializer#ignored_users: the usernames a member ignores.
pub async fn ignored_usernames(
    conn: &mut PgConnection,
    username: &str,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT u.username FROM ignored_users ig \
         JOIN users me ON me.id = ig.user_id \
         JOIN users u ON u.id = ig.ignored_user_id \
         WHERE me.username = $1 ORDER BY ig.id",
    )
    .bind(username)
    .fetch_all(conn)
    .await
}

/// What quote-button.js (PostTextSelection and its toolbar, and the topic
/// controller's selectText) reads from the topic and the viewer, as JSON
/// safe in a script element: whether the toolbar quotes
/// (embedQuoteButton), copies quotes (`enable_quoting`), shares and
/// offers the fast edit of a post the viewer can edit, the settings
/// buildQuote and the share sources read, and the strings.
async fn quote_button_config(
    conn: &mut PgConnection,
    i18n: &I18n,
    settings: &SiteSettings,
    view: &Value,
    viewer: Option<&str>,
) -> Result<String, HtmlError> {
    let enable_quoting: bool = match viewer {
        Some(username) => {
            sqlx::query_scalar(
                "SELECT COALESCE((SELECT uo.enable_quoting FROM user_options uo \
                 JOIN users u ON u.id = uo.user_id WHERE u.username = $1), TRUE)",
            )
            .bind(username)
            .fetch_one(&mut *conn)
            .await?
        }
        None => true,
    };
    let read_restricted: bool = match view["category_id"].as_i64() {
        Some(id) => {
            sqlx::query_scalar("SELECT read_restricted FROM categories WHERE id = $1::integer")
                .bind(id as i32)
                .fetch_optional(&mut *conn)
                .await?
                .unwrap_or(false)
        }
        None => false,
    };
    let editable: Vec<i64> = view["post_stream"]["posts"]
        .as_array()
        .map(|posts| {
            posts
                .iter()
                .filter(|p| p["can_edit"] == true)
                .filter_map(|p| p["id"].as_i64())
                .collect()
        })
        .unwrap_or_default();
    let setting = |name: &str| settings.get(name).map(|v| v.to_s());
    let flag = |name: &str| settings.get(name).map(|v| v.truthy());
    let strings: serde_json::Map<String, Value> = [
        "post.quote_reply",
        "post.quote_reply_shortcut",
        "post.quote_edit",
        "post.quote_edit_shortcut",
        "post.quote_copy",
        "post.quote_copied_to_clipboard",
        "share.twitter",
        "share.facebook",
        "share.email",
        "composer.save_edit",
        "modal.close",
    ]
    .iter()
    .map(|key| {
        let text = i18n.t(&format!("js.{key}")).unwrap_or(key);
        ((*key).to_string(), Value::String(text.to_string()))
    })
    .collect();
    let config = serde_json::json!({
        "topic": {
            "id": view["id"],
            "slug": view["slug"],
            "title": view["title"],
            "canCreatePost": view["details"]["can_create_post"] == true,
            "canReplyAsNewTopic": view["details"]["can_reply_as_new_topic"] == true,
            "privateMessage": view["archetype"] == "private_message",
            "invisible": view["visible"] == false,
            "readRestricted": read_restricted,
            "draftKey": format!("topic_{}", view["id"]),
            "draftSequence": view["draft_sequence"],
        },
        "user": viewer.map(|_| serde_json::json!({ "enableQuoting": enable_quoting })),
        "editablePosts": editable,
        "settings": {
            "enable_fast_edit": flag("enable_fast_edit")?,
            "share_quote_buttons": setting("share_quote_buttons")?,
            "share_quote_visibility": setting("share_quote_visibility")?,
            "login_required": flag("login_required")?,
            "display_name_on_posts": flag("display_name_on_posts")?,
            "prioritize_username_in_ux": flag("prioritize_username_in_ux")?,
            "title": setting("title")?,
            "facebook_app_id": setting("facebook_app_id")?,
        },
        "strings": strings,
    });
    Ok(config.to_string().replace('<', "\\u003c"))
}

/// The topic page from the /t/:id.json document; `pm_counts` for a
/// message's browse more line (pm_tracking_state::browse_more_counts).
pub async fn topic_page(
    conn: &mut PgConnection,
    i18n: &I18n,
    settings: &SiteSettings,
    mut site: Site,
    view: &Value,
    page: i64,
    pm_counts: Option<crate::post_view::PmCounts>,
) -> Result<TopicPage, HtmlError> {
    let base = site.base_path.clone();
    let slug = s(&view["slug"]);
    let id = view["id"].as_i64().unwrap_or(0);
    let topic_url = format!("{base}/t/{slug}/{id}");

    let categories = site.list_categories(conn).await?;
    let list = crate::topic_list_view::ListContext {
        i18n,
        base_path: &base,
        now: chrono::Utc::now(),
        categories: &categories,
        expand_all_pinned: false,
        hide_category: false,
        member_trust_level: site.viewer.as_ref().map(|v| v.trust_level),
        settings: crate::topic_list_view::ListSettings::load(settings)?,
    };
    let voter = match site.viewer.as_ref() {
        Some(v) if view["can_vote"] == serde_json::Value::Bool(true) => {
            let id: i32 = sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
                .bind(&v.username)
                .fetch_one(&mut *conn)
                .await?;
            Some((id, v.trust_level))
        }
        _ => None,
    };
    let post_settings = crate::post_view::PostSettings::load(settings)?;
    let viewer = site.viewer.as_ref().map(|v| v.username.as_str());
    let staff = site.viewer.as_ref().is_some_and(|v| v.staff);
    let can_send_pms = site
        .viewer
        .as_ref()
        .is_some_and(|v| v.can_send_private_messages);
    let title_html = topic_title_html(
        conn,
        i18n,
        settings,
        &list,
        view,
        &topic_url,
        viewer.map(|v| (v, can_send_pms)),
        voter,
    )
    .await?;
    let reply_where = if viewer.is_some() {
        crate::post_view::reply_where_topic(&list, view, staff || view["closed"] != true)
    } else {
        String::new()
    };
    // currentUser.ignored_users
    let ignored_users = match viewer {
        Some(username) => ignored_usernames(conn, username).await?,
        None => Vec::new(),
    };
    let mut topic = crate::post_view::TopicInfo::from_view(view);
    topic.pm_path = pm_path(conn, view, &base, viewer).await?;
    // What the staff menus read of the viewer.
    if let Some(username) = viewer {
        let user_id: Option<i32> =
            sqlx::query_scalar("SELECT id FROM users WHERE username_lower = $1")
                .bind(username.to_lowercase())
                .fetch_optional(&mut *conn)
                .await?;
        if let Some(user) = match user_id {
            Some(id) => crate::session::current::SessionUser::load(&mut *conn, id).await?,
            None => None,
        } {
            let guardian = crate::guardian::Guardian::for_user(&mut *conn, &user).await?;
            topic.menus = crate::post_view::StaffMenus::load(&guardian, settings, view)?;
        }
    }
    // Mark unread's destination: the message's inbox, else home.
    topic.defer_to = topic.pm_path.clone().unwrap_or_else(|| format!("{base}/"));
    if post_settings.poll.is_some()
        && let Some(username) = viewer
    {
        let user_id: Option<i32> =
            sqlx::query_scalar("SELECT id FROM users WHERE username_lower = $1")
                .bind(username.to_lowercase())
                .fetch_optional(&mut *conn)
                .await?;
        let posts = view["post_stream"]["posts"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        topic.poll_viewer =
            crate::plugins::poll::view::PollViewer::for_posts(conn, user_id, posts).await?;
    }
    // The first post's topic map, rendered before the posts that carry it.
    if crate::post_view::shows_op_map(view, &post_settings) {
        let cx = crate::post_view::PostContext {
            list: &list,
            settings: &post_settings,
            topic: &topic,
            viewer,
            staff,
            can_send_pms,
            ignored_users: &ignored_users,
        };
        topic.op_map = crate::post_view::topic_map(&cx, view, "--op");
    }
    let cx = crate::post_view::PostContext {
        list: &list,
        settings: &post_settings,
        topic: &topic,
        viewer,
        staff,
        can_send_pms,
        ignored_users: &ignored_users,
    };
    let posts = view["post_stream"]["posts"]
        .as_array()
        .map(|posts| crate::post_view::stream(&cx, posts, &view["post_stream"]["gaps"]))
        .unwrap_or_default();
    let bottom_map = if crate::post_view::shows_bottom_map(view, &post_settings) {
        crate::post_view::topic_map(&cx, view, "--bottom")
    } else {
        String::new()
    };
    let timeline = crate::post_view::timeline(&cx, view);
    let footer_buttons = crate::post_view::footer_buttons(&cx, view);
    let topic_admin_menu = crate::post_view::topic_admin_menu(&cx, view);
    let topic_menus = format!(
        "{topic_admin_menu}{}",
        crate::post_view::add_pm_participants_config(&cx, view)
    );
    let header_topic_info = crate::header_topic_view::template(
        &cx,
        view,
        &crate::header_topic_view::MinimizedLogo {
            small_url: site.chrome.logo_small_url.clone(),
            title: site.site_title.clone(),
        },
    );
    // The counts after screen-track marks this topic read through the
    // posts the page shows (TopicTrackingState#updateSeen).
    let tracking = site.chrome.tracking.as_ref().map(|t| {
        let mut t = t.clone();
        let highest_seen = view["post_stream"]["posts"]
            .as_array()
            .and_then(|p| p.iter().filter_map(|p| p["post_number"].as_i64()).max())
            .unwrap_or(0);
        t.update_seen(id as i32, highest_seen as i32);
        t
    });
    let more_topics = crate::post_view::more_topics(&cx, view, tracking.as_ref(), pm_counts)?;
    let quote_config = quote_button_config(conn, i18n, settings, view, viewer).await?;
    // A message's participant removal asks first; a poll reports its
    // errors in a dialog.
    let dialog_strings = if viewer.is_some()
        && (view["archetype"] == "private_message"
            || post_settings.poll.is_some()
            || !topic_admin_menu.is_empty())
    {
        dialog_strings(i18n)
    } else {
        String::new()
    };

    let stream_len = view["post_stream"]["stream"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0) as i64;
    let chunk = view["chunk_size"].as_i64().unwrap_or(20);
    let last_page = ((stream_len - 1).max(0) / chunk) + 1;
    let page = page.max(1);
    let page_url = |n: i64| {
        if n <= 1 {
            topic_url.clone()
        } else {
            format!("{topic_url}?page={n}")
        }
    };

    // The page's posts, which the live updates redraw when a change spans
    // the topic (discourse-solved's accepted answers).
    let numbers: Vec<i64> = view["post_stream"]["posts"]
        .as_array()
        .map(|posts| {
            posts
                .iter()
                .filter_map(|p| p["post_number"].as_i64())
                .collect()
        })
        .unwrap_or_default();
    if let (Some(first), Some(last)) = (numbers.iter().min(), numbers.iter().max()) {
        site.chrome.live_param("posts", &format!("{first}-{last}"));
    }
    // PostStream#loadedAllPosts: the stream's last post is drawn, on the
    // last page or in the window around a post near the end.
    let last_id = view["post_stream"]["stream"]
        .as_array()
        .and_then(|s| s.last())
        .and_then(Value::as_i64);
    let loaded_all = page >= last_page
        || last_id.is_some_and(|id| {
            view["post_stream"]["posts"]
                .as_array()
                .is_some_and(|posts| posts.iter().any(|p| p["id"].as_i64() == Some(id)))
        });
    // topic.gjs hides the footer until the post stream is loaded to its end.
    if !loaded_all {
        site.chrome.show_footer = false;
    }
    // topic.gjs's body classes: AddCategoryTagClasses, AddTopicStatusClasses
    // and the archetype's.
    let archetype = s(&view["archetype"]);
    site.chrome
        .body_classes
        .push_str(&topic_body_classes(&list, view, &archetype));
    Ok(TopicPage {
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        site_description: site.site_description,
        lang: site.lang,
        base_path: site.base_path,
        crawler: Crawler::default(),
        title: s(&view["title"]),
        canonical_url: page_url(page),
        archetype,
        reply_where,
        title_html,
        header_topic_info,
        posts,
        bottom_map,
        timeline,
        footer_buttons,
        more_topics,
        topic_menus,
        topic_id: id,
        // New posts land at the end; pages before it stay as they are.
        live: loaded_all,
        reactions: post_settings.reactions.is_some(),
        solved: post_settings.solved.is_some(),
        ignored_users: ignored_users.join(","),
        quote_config,
        dialog_strings,
        poll_scripts: if post_settings.poll.is_some() {
            crate::plugins::poll::view::page_scripts(i18n, &base)
        } else {
            String::new()
        },
        prev_url: (page > 1).then(|| page_url(page - 1)),
        next_url: (page < last_page).then(|| page_url(page + 1)),
    })
}

/// The body classes topic.gjs adds, each after a space:
/// AddCategoryTagClasses' `category` and `category-<fullSlug>` and a
/// `tag-<name>` per tag, AddTopicStatusClasses' unlisted, pinned and
/// unpinned, then `archetype-<archetype>`.
fn topic_body_classes(
    list: &crate::topic_list_view::ListContext,
    view: &Value,
    archetype: &str,
) -> String {
    let mut out = String::new();
    if let Some(category) = view["category_id"]
        .as_i64()
        .and_then(|id| list.categories.get(&id))
    {
        out.push_str(&format!(
            " category category-{}",
            crate::topic_list_view::category_full_slug(list, category)
        ));
    }
    for tag in view["tags"].as_array().into_iter().flatten() {
        // A tag is its name, or an object with one (tags_descriptions on).
        if let Some(name) = tag.as_str().or_else(|| tag["name"].as_str()) {
            out.push_str(&format!(" tag-{name}"));
        }
    }
    // Topic#invisible, #pinned and #unpinned (pinned once, now not).
    if view["visible"] == false {
        out.push_str(" topic-status-unlisted");
    }
    if view["pinned"] == true {
        out.push_str(" topic-status-pinned");
    }
    if view["unpinned"] == true {
        out.push_str(" topic-status-unpinned");
    }
    out.push_str(&format!(" archetype-{archetype}"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_urls() {
        assert_eq!(date("2026-09-30T07:28:29.580Z"), "Sep 30, 2026");
        assert_eq!(date("garbage"), "garbage");
        let cats = vec![
            CategoryRow {
                id: 1,
                name: "General".into(),
                color: "0088CC".into(),
                slug: "general".into(),
                parent_category_id: None,
            },
            CategoryRow {
                id: 2,
                name: "Sub".into(),
                color: "AB9364".into(),
                slug: "sub".into(),
                parent_category_id: Some(1),
            },
        ];
        assert_eq!(category_url("", &cats, &cats[0]), "/c/general/1");
        assert_eq!(category_url("/f", &cats, &cats[1]), "/f/c/general/sub/2");
        assert!(badge("", &cats, Some(9)).is_none());
    }

    #[test]
    fn lightbox_attrs_carry_the_titles_and_download() {
        let i18n = I18n::vendored().unwrap();
        let attrs = lightbox_attrs(&i18n, false);
        assert!(attrs.starts_with(" data-can-download=\"false\""));
        assert!(attrs.contains(" data-title-previous=\"Previous (Left arrow key)\""));
        assert!(attrs.contains(" data-title-image-info=\"Image information\""));
        assert!(lightbox_attrs(&i18n, true).contains("data-can-download=\"true\""));
    }
}

/// A subcategory entry on a category page's first page.
pub struct SubcategoryItem {
    pub name: String,
    pub url: String,
    pub description: Option<String>,
}

/// The category page header: the category (and its parent) as links,
/// plus its visible subcategories on the first page (list.erb 13-37).
pub struct CategoryHeading {
    /// For the New Topic button: a topic created here starts in it.
    pub id: i32,
    pub name: String,
    pub url: String,
    pub parent: Option<CategoryBadge>,
    pub subcategories: Vec<SubcategoryItem>,
}

pub async fn category_heading(
    conn: &mut PgConnection,
    base_path: &str,
    category: &crate::category::Category,
    first_page: bool,
) -> Result<CategoryHeading, HtmlError> {
    let parent = match category.parent_category_id {
        Some(id) => match crate::category::Category::find(conn, id).await? {
            Some(p) => Some(CategoryBadge {
                name: p.name.clone(),
                color: p.color.clone(),
                url: p.url(conn, base_path).await?,
            }),
            None => None,
        },
        None => None,
    };
    let mut subcategories = Vec::new();
    if first_page {
        for sub in category.visible_subcategories(conn).await? {
            subcategories.push(SubcategoryItem {
                name: sub.name.clone(),
                url: sub.url(conn, base_path).await?,
                description: sub
                    .description
                    .as_deref()
                    .map(|d| crate::categories::description_plain_text(Some(d)))
                    .transpose()
                    .ok()
                    .flatten()
                    .flatten(),
            });
        }
    }
    Ok(CategoryHeading {
        id: category.id,
        name: category.name.clone(),
        url: category.url(conn, base_path).await?,
        parent,
        subcategories,
    })
}

pub struct CategoryIndexItem {
    pub name: String,
    pub url: String,
    pub color: String,
    pub description: Option<String>,
    pub topic_count: i64,
    pub subcategories: Vec<CategoryBadge>,
    pub topics: Vec<FeaturedTopic>,
}

pub struct FeaturedTopic {
    pub title: String,
    pub url: String,
    pub bumped_at: String,
    pub bumped_at_iso: String,
}

#[derive(Template)]
#[template(path = "categories.html")]
pub struct CategoriesPage {
    pub site_title: String,
    /// The `<title>` (document_title).
    pub title: String,
    pub site_description: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: Crawler,
    pub viewer: Option<Viewer>,
    pub bus_position: String,
    pub chrome: Chrome,
    pub categories: Vec<CategoryIndexItem>,
    pub banner: String,
    /// The categories-and-latest view (categories_view), empty for the page
    /// styles not ported, which keep the plain table.
    pub main: String,
    pub nav: Vec<NavItem>,
    pub breadcrumbs: String,
    pub admin_controls: String,
    /// After the new topic button on a category's list: the member's
    /// notification level in it (category_notifications); it also marks
    /// the navigation as a category's, in place of the heading.
    pub category_controls: String,
    /// A category's list: the category navigation instead of the heading.
    pub category_nav: bool,
}

/// The categories index from the /categories.json document
/// (categories/index.html.erb's table, plus featured topics).
pub async fn categories_page(
    conn: &mut PgConnection,
    _i18n: &I18n,
    emojis: &crate::emojis::Emojis,
    site: Site,
    doc: &Value,
) -> Result<CategoriesPage, HtmlError> {
    let cats = categories(conn).await?;
    let base = site.base_path.clone();
    let items = doc["category_list"]["categories"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|c| {
                    let subcategories = c["subcategory_ids"]
                        .as_array()
                        .map(|ids| {
                            ids.iter()
                                .filter_map(|id| badge(&base, &cats, id.as_i64()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let row = cats
                        .iter()
                        .find(|x| Some(i64::from(x.id)) == c["id"].as_i64());
                    CategoryIndexItem {
                        name: s(&c["name"]),
                        url: row
                            .map(|r| category_url(&base, &cats, r))
                            .unwrap_or_default(),
                        color: s(&c["color"]),
                        description: c["description"].as_str().map(str::to_string),
                        topic_count: c["topic_count"].as_i64().unwrap_or(0),
                        subcategories,
                        topics: c["topics"]
                            .as_array()
                            .map(|ts| {
                                ts.iter()
                                    .map(|t| FeaturedTopic {
                                        title: emojis.gsub_to_unicode(&s(&t["title"])),
                                        url: format!("{base}/t/{}/{}", s(&t["slug"]), t["id"]),
                                        bumped_at: date(&s(&t["bumped_at"])),
                                        bumped_at_iso: s(&t["bumped_at"]),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(CategoriesPage {
        title: site.site_title.clone(),
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        site_description: site.site_description,
        lang: site.lang,
        base_path: site.base_path,
        crawler: Crawler::default(),
        categories: items,
        banner: String::new(),
        main: String::new(),
        nav: Vec::new(),
        breadcrumbs: String::new(),
        admin_controls: String::new(),
        category_controls: String::new(),
        category_nav: false,
    })
}

#[derive(Template)]
#[template(path = "tags.html")]
pub struct TagsPage {
    pub site_title: String,
    pub site_description: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: Crawler,
    pub viewer: Option<Viewer>,
    pub bus_position: String,
    pub chrome: Chrome,
    pub groups: Vec<TagGroupItem>,
}

pub struct TagGroupItem {
    pub name: Option<String>,
    pub tags: Vec<TagBoxItem>,
}

pub struct TagBoxItem {
    pub name: String,
    pub url: String,
    pub count: i64,
}

/// The tags index from the /tags.json document (tags/index.html.erb):
/// each category's tags, then the rest under "Other Tags".
pub fn tags_page(
    i18n: &I18n,
    site: Site,
    doc: &Value,
    category_names: &[(i64, String)],
) -> TagsPage {
    let base = site.base_path.clone();
    let boxes = |tags: &Value| -> Vec<TagBoxItem> {
        tags.as_array()
            .map(|list| {
                list.iter()
                    .map(|t| TagBoxItem {
                        name: s(&t["text"]),
                        url: format!("{base}/tag/{}", t["id"]),
                        count: t["count"].as_i64().unwrap_or(0),
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut groups: Vec<TagGroupItem> = doc["extras"]["categories"]
        .as_array()
        .map(|cats| {
            cats.iter()
                .map(|c| TagGroupItem {
                    name: category_names
                        .iter()
                        .find(|(id, _)| Some(*id) == c["id"].as_i64())
                        .map(|(_, name)| name.clone()),
                    tags: boxes(&c["tags"]),
                })
                .collect()
        })
        .unwrap_or_default();
    let other = boxes(&doc["tags"]);
    if !other.is_empty() {
        groups.push(TagGroupItem {
            name: Some(
                i18n.t("js.tagging.other_tags")
                    .unwrap_or("Other Tags")
                    .to_string(),
            ),
            tags: other,
        });
    }
    TagsPage {
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        site_description: site.site_description,
        lang: site.lang,
        base_path: site.base_path,
        crawler: Crawler::default(),
        groups,
    }
}

/// What the layout's `<head>` says to crawlers: the canonical link, the
/// description meta and `crawlable_meta_data`'s OpenGraph/Twitter tags.
#[derive(Debug, Clone, Default)]
pub struct Crawler {
    /// Absolute; empty when the page sets none.
    pub canonical: String,
    /// `<meta name="description">` (`description_content`)
    pub description: String,
    /// og:title; empty when the page emits no crawlable_meta_data.
    pub title: String,
    /// og:description
    pub og_description: String,
    /// og:image, absolute
    pub image: Option<String>,
    /// og:url: the request's own URL
    pub url: String,
    /// `add_noindex_header_to_non_canonical`: the canonical differs from
    /// the request URL.
    pub noindex: bool,
    /// `auto_discovery_link_tag(:rss, href, title:)`: the feed's URL and
    /// title.
    pub rss: Option<(String, String)>,
}

impl Crawler {
    /// `default_canonical`: the request path plus its `page` param.
    pub fn default_canonical(base_url_no_prefix: &str, path: &str, query: Option<&str>) -> String {
        let mut canonical = format!("{base_url_no_prefix}{path}");
        if let Some(q) = query {
            let page: Vec<&str> = q.split('&').filter(|p| p.starts_with("page=")).collect();
            if !page.is_empty() {
                canonical.push('?');
                canonical.push_str(&page.join("&"));
            }
        }
        canonical
    }

    pub fn new(
        base_url_no_prefix: &str,
        path: &str,
        query: Option<&str>,
        canonical: String,
    ) -> Crawler {
        let mut url = format!("{base_url_no_prefix}{path}");
        if let Some(q) = query.filter(|q| !q.is_empty()) {
            url.push('?');
            url.push_str(q);
        }
        Crawler {
            noindex: !canonical.is_empty() && canonical != url,
            canonical,
            url,
            ..Crawler::default()
        }
    }

    /// The crawler block for a request: the given canonical, else the
    /// default one.
    pub fn for_request(
        urls: &crate::url::Urls<'_>,
        uri: &axum::http::Uri,
        canonical: Option<String>,
    ) -> Result<Crawler, crate::url::UrlError> {
        let base = urls.base_url_no_prefix()?;
        let canonical =
            canonical.unwrap_or_else(|| Crawler::default_canonical(&base, uri.path(), uri.query()));
        Ok(Crawler::new(&base, uri.path(), uri.query(), canonical))
    }

    /// `crawlable_meta_data(title:, description:, image:)`: the site's
    /// OpenGraph image when the page has none.
    pub fn with_meta(
        mut self,
        emojis: &crate::emojis::Emojis,
        title: &str,
        description: &str,
        image: Option<String>,
    ) -> Crawler {
        self.title = emojis.gsub_to_unicode(title);
        self.og_description = emojis.gsub_to_unicode(description);
        self.image = image.filter(|i| !i.is_empty());
        self
    }
}

/// The site's OpenGraph image (`SiteSetting.site_opengraph_image_url`),
/// None when nothing resolves.
pub async fn site_opengraph_image(
    conn: &mut PgConnection,
    urls: &crate::url::Urls<'_>,
) -> Result<Option<String>, crate::site_icons::IconError> {
    let url = crate::site_icons::site_url(conn, urls, "opengraph_image").await?;
    Ok((!url.is_empty()).then_some(url))
}

/// An HTML response with the non-canonical noindex header when the
/// setting asks for it.
pub fn crawler_response(
    body: String,
    crawler: &Crawler,
    settings: &SiteSettings,
    viewer: &ViewerState,
) -> Result<axum::response::Response, SettingError> {
    use axum::response::IntoResponse;
    let mut response = axum::response::Html(body).into_response();
    if crawler.noindex && !settings.get("allow_indexing_non_canonical_urls")?.truthy() {
        response.headers_mut().insert(
            "x-robots-tag",
            axum::http::HeaderValue::from_static("noindex"),
        );
    }
    Ok(with_viewer_headers(response, viewer))
}

/// A live update of one post for the topic page: appended to the posts
/// when new, else replacing the post in place (htmx out-of-band swaps).
#[derive(Template)]
#[template(path = "post_fragment.html")]
pub struct PostFragment {
    /// The post as post_view renders it; when replacing, its wrapper
    /// carries the out-of-band swap onto the post shown.
    pub html: String,
    pub append: bool,
}

/// What a notification says its actor did, by `Notification.types`, for
/// the header's alert and the user menu.
pub fn notification_verb(notification_type: i64) -> &'static str {
    match notification_type {
        1 => "mentioned you in",
        2 => "replied in",
        3 => "quoted you in",
        4 => "edited your post in",
        5 | 19 => "liked your post in",
        6 => "sent you a message,",
        7 => "invited you to a message,",
        9 | 36 => "posted in",
        11 => "linked to your post in",
        12 => "earned a badge,",
        13 => "invited you to",
        17 => "posted a new topic,",
        24 => "Reminder:",
        // discourse-reactions' reaction.
        25 => "reacted to your post in",
        _ => "in",
    }
}

impl LatestPage {
    /// The discovery controller's `canBulkSelect`: topic managers, or the
    /// new and unread lists while they have topics to dismiss.
    pub fn bulk_select(&self) -> bool {
        self.viewer.as_ref().is_some_and(Viewer::can_manage_topic)
            || (matches!(self.live_filter.as_str(), "new" | "unread") && !self.rows.is_empty())
    }

    /// The category the new topic button starts in, on a category's list.
    pub fn heading_id(&self) -> Option<i32> {
        self.heading.as_ref().map(|h| h.id)
    }

    /// The footer draws EmptyTopicFilter, whose button needs its script.
    pub fn empty_filter(&self) -> bool {
        self.list_bottom.contains("--empty-topic-filter")
    }
}

impl CategoriesPage {
    /// No category preselected on the categories page.
    pub fn heading_id(&self) -> Option<i32> {
        None
    }

    /// TopicDismissButtons' top row: the categories page has no list to
    /// dismiss.
    pub fn dismiss_top(&self) -> &'static str {
        "<div class=\"row dismiss-container-top\"></div>"
    }
}

#[cfg(test)]
mod list_bottom_tests {
    use super::*;
    use crate::topic_tracking_report::{State, Tracking};
    use std::collections::{HashMap, HashSet};

    fn tracking(states: Vec<State>) -> Tracking {
        Tracking {
            states,
            muted_category_ids: HashSet::new(),
            parents: HashMap::new(),
            unified_new: true,
        }
    }

    /// A topic new to the member (no read posts) or unread (some read).
    fn state(topic_id: i32, category_id: i32, last_read: Option<i32>) -> State {
        State {
            topic_id,
            highest_post_number: 3,
            last_read_post_number: last_read,
            category_id,
            is_category_topic: false,
            notification_level: last_read.map(|_| 2),
            created_in_new_period: true,
            tags: Some(vec![]),
        }
    }

    fn scope(filter: &str) -> ListFooterScope<'_> {
        ListFooterScope {
            filter,
            ..Default::default()
        }
    }

    async fn draw(
        mobile: bool,
        tracking: Option<&Tracking>,
        topics: usize,
        all_loaded: bool,
        scope: &ListFooterScope<'_>,
    ) -> String {
        let i18n = I18n::vendored().unwrap();
        crate::mobile::scope(mobile, async {
            list_bottom(&i18n, "/forum", tracking, topics, all_loaded, scope)
        })
        .await
    }

    const EMPTY: &str =
        r#"<footer class="topic-list-bottom"><div class="loading-container"></div></footer>"#;

    /// The footer's spinner container is always there; a page that can
    /// load more draws nothing after it, a loaded one the dismiss buttons'
    /// row past five topics on desktop.
    #[tokio::test]
    async fn the_footer_follows_the_lists_length_and_layout() {
        let member = tracking(vec![]);
        for (mobile, viewer, topics, all_loaded, dismiss) in [
            (false, None, 5, true, false),
            (false, None, 6, true, true),
            (false, Some(&member), 30, true, true),
            (false, None, 30, false, false),
            (true, None, 6, true, false),
            (true, Some(&member), 30, true, false),
        ] {
            let html = draw(mobile, viewer, topics, all_loaded, &scope("latest")).await;
            if dismiss {
                assert_eq!(
                    html,
                    r#"<footer class="topic-list-bottom"><div class="loading-container"></div><div class="row dismiss-container-bottom"></div></footer>"#
                );
            } else {
                assert_eq!(html, EMPTY, "{mobile} {topics} {all_loaded}");
            }
        }
        // An anonymous visitor's empty list has no education.
        assert_eq!(draw(false, None, 0, true, &scope("latest")).await, EMPTY);
        // Nor does a member's list that can still load more.
        assert_eq!(
            draw(false, Some(&member), 0, false, &scope("unread")).await,
            EMPTY
        );
    }

    const CHEVRON: &str = r##"<svg class="fa d-icon d-icon-chevron-down svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#chevron-down"></use></svg>"##;

    /// TopicDismissButtons on new, as Ember draws them (rs-ref-a): under
    /// unified new the combo, its label and menu by the subset; the
    /// classic "Dismiss New" otherwise; the bottom row only past five
    /// topics.
    #[test]
    fn new_draws_the_dismiss_combo_by_subset() {
        let i18n = I18n::vendored().unwrap();
        let member = tracking(vec![]);
        let combo = |label: &str, menu: bool| {
            format!(
                r#"<div class="row dismiss-container-top"><div class="d-combo-button{} topic-dismiss-buttons__combo" role="group"><button class="btn d-combo-button-button btn-default dismiss-read topic-dismiss-buttons__button" id="dismiss-new-top" type="button"><span class="d-button-label">{label}</span></button>{}</div></div>"#,
                if menu { " --has-menu" } else { "" },
                if menu {
                    format!(
                        r#"<button aria-expanded="false" aria-label="Dismiss options" class="btn no-text btn-icon fk-d-menu__trigger dismiss-new-menu-trigger d-combo-button-menu btn-default dismiss-read topic-dismiss-buttons__menu" id="dismiss-new-menu-top" data-identifier="dismiss-new-menu" data-trigger="" type="button">{CHEVRON}</button>"#
                    )
                } else {
                    String::new()
                }
            )
        };
        for (subset, label, menu) in [
            (None, "Dismiss all", true),
            (Some("bogus"), "Dismiss all", true),
            (Some("topics"), "Dismiss new topics", false),
            (Some("replies"), "Dismiss new replies", true),
        ] {
            let new = ListFooterScope {
                filter: "new",
                subset,
                ..Default::default()
            };
            assert_eq!(
                dismiss_buttons(&i18n, "top", Some(&member), 2, &new),
                combo(label, menu),
                "{subset:?}"
            );
        }
        let classic = Tracking {
            unified_new: false,
            ..tracking(vec![])
        };
        assert_eq!(
            dismiss_buttons(&i18n, "top", Some(&classic), 2, &scope("new")),
            r##"<div class="row dismiss-container-top"><button class="btn btn-icon-text btn-default dismiss-read" id="dismiss-new-top" type="button"><svg class="fa d-icon d-icon-check svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#check"></use></svg><span class="d-button-label">Dismiss New</span></button></div>"##
        );
        // The bottom row is there past five topics only.
        assert_eq!(
            dismiss_buttons(&i18n, "bottom", Some(&member), 5, &scope("new")),
            ""
        );
        assert!(
            dismiss_buttons(&i18n, "bottom", Some(&member), 6, &scope("new"))
                .contains(r#"id="dismiss-new-bottom""#)
        );
    }

    /// Unread's "Dismiss…"; a category's list carries the category for
    /// the requests; nothing to dismiss leaves the row empty.
    #[test]
    fn unread_draws_dismiss_and_empty_lists_draw_none() {
        let i18n = I18n::vendored().unwrap();
        let member = tracking(vec![]);
        let unread = ListFooterScope {
            filter: "unread",
            category_id: Some(4),
            no_subcategories: true,
            ..Default::default()
        };
        assert_eq!(
            dismiss_buttons(&i18n, "top", Some(&member), 1, &unread),
            r#"<div class="row dismiss-container-top" data-category-id="4" data-no-subcategories=""><button class="btn btn-default dismiss-read" id="dismiss-topics-top" title="Dismiss just new posts or stop tracking topics" type="button"><span class="d-button-label">Dismiss…</span></button></div>"#
        );
        let empty = r#"<div class="row dismiss-container-top"></div>"#;
        assert_eq!(
            dismiss_buttons(&i18n, "top", Some(&member), 0, &unread),
            empty
        );
        assert_eq!(dismiss_buttons(&i18n, "top", None, 3, &unread), empty);
        assert_eq!(
            dismiss_buttons(&i18n, "top", Some(&member), 3, &scope("latest")),
            empty
        );
    }

    /// EmptyTopicFilter in a member's empty list footer: the filter's
    /// education, "Browse latest topics", and on new and unread the
    /// tracking preferences tip under the base path.
    #[tokio::test]
    async fn a_members_empty_list_draws_the_education() {
        let member = tracking(vec![]);
        let image = format!(
            r#"<footer class="topic-list-bottom"><div class="loading-container"></div><div class="empty-state__container --empty-topic-filter --with-image"><div class="empty-state"><div class="empty-state__image">{DOCUMENTS_CHECKMARK}</div>"#
        );
        let cta = r#"<div class="empty-state__cta"><button class="btn btn-primary" type="button"><span class="d-button-label">Browse latest topics</span></button></div>"#;
        let tip = r##"<div class="empty-state__tip"><svg class="fa d-icon d-icon-circle-info svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#circle-info"></use></svg> You can view and change your new topic tracking settings in <a href="/forum/my/preferences/tracking">your preferences</a>.</div>"##;
        for (mobile, filter, title, with_tip) in [
            (false, "unread", "Nothing left unread... impressive!", true),
            (true, "unread", "Nothing left unread... impressive!", true),
            (
                false,
                "new",
                "Nothing new at the moment... check back soon!",
                true,
            ),
            (
                false,
                "latest",
                "Nothing to see here...check back soon!",
                false,
            ),
            (
                false,
                "top",
                "Nothing to see here...check back soon!",
                false,
            ),
        ] {
            let html = draw(mobile, Some(&member), 0, true, &scope(filter)).await;
            let title =
                format!(r#"<div class="empty-state__title" data-test-title="">{title}</div>"#);
            let tip = if with_tip { tip } else { "" };
            assert_eq!(
                html,
                format!("{image}{title}{cta}{tip}</div></div></footer>"),
                "{filter}"
            );
        }
    }

    /// Under unified new, an empty subset offers the other one while the
    /// member has some of it in the list's scope (ctaLabelWithAction),
    /// else latest topics.
    #[tokio::test]
    async fn an_empty_new_subset_offers_the_other() {
        // Topic 1 new in category 10, topic 2 unread in category 11.
        let member = tracking(vec![state(1, 10, None), state(2, 11, Some(1))]);
        let button = |subset: &str, label: &str| {
            format!(
                r#"<button class="btn btn-primary" type="button" data-subset="{subset}"><span class="d-button-label">{label}</span></button>"#
            )
        };
        let latest = r#"<button class="btn btn-primary" type="button"><span class="d-button-label">Browse latest topics</span></button>"#;
        for (subset, category_id, expected) in [
            (
                Some("topics"),
                None,
                button("replies", "Browse new replies"),
            ),
            (Some("replies"), None, button("topics", "Browse new topics")),
            (Some("topics"), Some(10), latest.to_string()),
            (
                Some("replies"),
                Some(10),
                button("topics", "Browse new topics"),
            ),
            (Some("replies"), Some(11), latest.to_string()),
            (None, None, latest.to_string()),
        ] {
            let html = draw(
                false,
                Some(&member),
                0,
                true,
                &ListFooterScope {
                    filter: "new",
                    subset,
                    category_id,
                    ..Default::default()
                },
            )
            .await;
            assert!(
                html.contains(&expected),
                "{subset:?} {category_id:?}: {html}"
            );
        }
        // Without unified new the subset is not read.
        let legacy = Tracking {
            unified_new: false,
            ..member
        };
        let html = draw(
            false,
            Some(&legacy),
            0,
            true,
            &ListFooterScope {
                filter: "new",
                subset: Some("topics"),
                ..Default::default()
            },
        )
        .await;
        assert!(html.contains(latest), "{html}");
    }
}
