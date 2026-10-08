//! Server-rendered pages for anonymous readers, built from the same
//! documents the JSON endpoints return. The structure follows Discourse's
//! crawler views (app/views/list/list.erb, topics/show.html.erb); the
//! styling is ours.

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
}

impl std::fmt::Display for HtmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HtmlError::Db(e) => write!(f, "rendering page: {e}"),
            HtmlError::Setting(e) => e.fmt(f),
            HtmlError::Template(e) => write!(f, "rendering template: {e}"),
            HtmlError::MessageFormat(e) => e.fmt(f),
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

impl From<SettingError> for HtmlError {
    fn from(e: SettingError) -> Self {
        HtmlError::Setting(e)
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
        })
    }
}

/// The page chrome around every page: the header and the body's classes.
#[derive(Clone, Default)]
pub struct Chrome {
    /// The header's logo (`SiteSetting.site_logo_url`), empty for the site
    /// title as text.
    pub logo_url: String,
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
    /// The sidebar's markup, empty when the page has none
    /// (ApplicationController#sidebarEnabled).
    pub sidebar: String,
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
    /// PoweredByDiscourse below the content: enable_powered_by_discourse,
    /// less the pages that hide the application footer (a list with more
    /// to load, a topic not loaded to its end).
    pub powered_by: bool,
}

impl Chrome {
    /// Adds a parameter for the page's live updates.
    pub fn live_param(&mut self, name: &str, value: &str) {
        self.live_params.push_str(&format!(
            "&{name}={}",
            form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>()
        ));
    }
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
            Some(crate::current_user::sidebar_member(&mut *conn, settings, user, tracking).await?)
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
    /// for some groups only are not told apart yet, and read-only mode is
    /// not ported.
    pub async fn load_chrome(
        &mut self,
        state: &crate::AppState,
        settings: &SiteSettings,
        guardian: &crate::guardian::Guardian,
        active: crate::sidebar::Active,
    ) -> Result<(), crate::AppError> {
        let mut conn = state.pool.acquire().await?;
        let urls = crate::url::Urls {
            config: &state.config,
            settings,
        };
        self.chrome.logo_url = crate::site_icons::site_url(&mut conn, &urls, "logo").await?;
        self.chrome.favicon_url = crate::site_icons::site_url(&mut conn, &urls, "favicon").await?;
        self.chrome.favicon_type = crate::site_icons::mime_type(&self.chrome.favicon_url);
        self.chrome.apple_touch_icon_url =
            crate::site_icons::site_url(&mut conn, &urls, "apple_touch_icon").await?;
        let mut classes = Vec::new();
        for name in state.site_setting_defs.upcoming_changes_with_css() {
            if settings.get(name)?.truthy() {
                classes.push(format!("uc-{}", name.replace('_', "-")));
            }
        }
        self.chrome.powered_by = settings.get("enable_powered_by_discourse")?.truthy();
        self.chrome.can_signup = !settings.get("invite_only")?.truthy()
            && settings.get("allow_new_registrations")?.truthy()
            && !settings.get("enable_discourse_connect")?.truthy();

        // sidebarEnabled: canDisplaySidebar (not for anonymous visitors
        // when login is required) and the sidebar navigation menu.
        let sidebar_enabled = (guardian.user().is_some()
            || !settings.get("login_required")?.truthy())
            && settings.get("navigation_menu")?.to_s() == "sidebar";
        self.chrome.tracking =
            crate::topic_tracking_report::load(&mut conn, settings, guardian).await?;
        if sidebar_enabled {
            let (site, member) = sidebar_inputs(
                &mut conn,
                state,
                settings,
                guardian,
                self.chrome.tracking.clone(),
            )
            .await?;
            let emoji_set = settings.get("emoji_set")?.to_s().to_string();
            self.chrome.sidebar = crate::sidebar::render(
                &site,
                &crate::sidebar::Context {
                    i18n: &state.i18n,
                    settings,
                    base_path: &self.base_path,
                    active: &active,
                    member: member.as_ref(),
                    emoji_set: &emoji_set,
                },
            )?;
            if self.chrome.tracking.is_some() {
                self.chrome.live_param("sidebar", &active.key());
            }
            // Sidebar.gjs's bodyClass
            classes.push("has-sidebar-page".into());
        }
        if let Some(user_id) = guardian.user_id() {
            self.chrome.can_create_topic = guardian.can_create_topic(&mut conn, settings).await?;
            let draft_count: Option<i32> =
                sqlx::query_scalar("SELECT draft_count FROM user_stats WHERE user_id = $1")
                    .bind(user_id)
                    .fetch_optional(&mut *conn)
                    .await?;
            self.chrome.drafts_menu_trigger =
                drafts_menu_trigger(&state.i18n, i64::from(draft_count.unwrap_or(0)));
            self.chrome.composer = self.composer(&mut conn, state, settings, guardian).await?;
        }
        self.chrome.body_classes = classes.join(" ");
        Ok(())
    }

    /// The member's composer, with the categories they may create topics
    /// in for its chooser.
    async fn composer(
        &self,
        conn: &mut PgConnection,
        state: &crate::AppState,
        settings: &SiteSettings,
        guardian: &crate::guardian::Guardian,
    ) -> Result<String, crate::AppError> {
        let mut allowed = guardian
            .topic_create_allowed_category_ids(&mut *conn, settings)
            .await?;
        // The chooser leaves out uncategorized unless topics may go there.
        if !settings.get("allow_uncategorized_topics")?.truthy() {
            let uncategorized = settings.get("uncategorized_category_id")?.to_i() as i32;
            allowed.retain(|id| *id != uncategorized);
        }
        let rows: Vec<(i32, i32, Option<String>)> = sqlx::query_as(
            "SELECT id, topic_count, description FROM categories WHERE id = ANY($1)",
        )
        .bind(&allowed)
        .fetch_all(&mut *conn)
        .await?;
        let mut chooser = Vec::with_capacity(rows.len());
        for (id, topic_count, description) in rows {
            chooser.push(crate::composer_view::ChooserCategory {
                id: i64::from(id),
                topic_count: i64::from(topic_count),
                // Category#description_text
                description_text: crate::categories::description_plain_text(
                    description.as_deref(),
                )?,
            });
        }
        let categories = crate::topic_list_view::categories(&mut *conn).await?;
        let list = crate::topic_list_view::ListContext {
            i18n: &state.i18n,
            base_path: &self.base_path,
            now: chrono::Utc::now(),
            categories: &categories,
            expand_all_pinned: false,
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
        let allow_preview = match guardian.user_id() {
            Some(uid) => {
                let mode: Option<i32> = sqlx::query_scalar(
                    "SELECT composition_mode FROM user_options WHERE user_id = $1",
                )
                .bind(uid)
                .fetch_optional(&mut *conn)
                .await?;
                mode != Some(1)
            }
            None => false,
        };
        Ok(crate::composer_view::render(
            &list,
            &chooser,
            default_category,
            Some((&wasm, &render_settings)),
            allow_preview,
            crate::composer_view::UploadUi::for_user(
                &settings.get("authorized_extensions")?.to_s(),
                &settings.get("authorized_extensions_for_staff")?.to_s(),
                guardian.is_staff(),
            )
            .as_ref(),
        ))
    }
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
    let mut out = String::from("<ol class=\"category-breadcrumb\">");
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
/// `tracking`), unread folded into new with unified new, the active list
/// added when the menu lacks it, and the member's counts in the labels.
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
    let mut names: Vec<&str> = top_menu
        .split('|')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .filter(|name| !(unified_new && *name == "unread"))
        .collect();
    if !active.is_empty() && !names.contains(&active) {
        names.push(active);
    }
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

/// NavItem#displayName: `title_with_count` while the member has topics in
/// the list (never for latest on desktop), else `title`.
pub fn nav_label(
    i18n: &I18n,
    name: &str,
    tracking: Option<&crate::topic_tracking_report::Tracking>,
) -> String {
    let count = tracking.map(|t| t.lookup(name)).unwrap_or(0);
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
    /// `#topic-title`'s title wrapper (post_view::topic_title).
    pub title_html: String,
    /// The posts as Ember renders them (post_view::stream).
    pub posts: Vec<String>,
    /// Around the posts (post_view): the bottom topic map, the timeline,
    /// the footer buttons and the suggested topics.
    pub bottom_map: String,
    pub timeline: String,
    pub footer_buttons: String,
    pub more_topics: String,
    pub prev_url: Option<String>,
    pub next_url: Option<String>,
    pub topic_id: i64,
    /// The last page: live updates append new posts here.
    pub live: bool,
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

/// The latest page from the /latest.json document. `expand_all_pinned` is
/// a category's or a tag's list, which shows every pinned topic's excerpt.
pub async fn latest_page(
    conn: &mut PgConnection,
    site: Site,
    list: &Value,
    i18n: &I18n,
    settings: &SiteSettings,
    expand_all_pinned: bool,
) -> Result<LatestPage, HtmlError> {
    let categories = crate::topic_list_view::categories(conn).await?;
    let cx = crate::topic_list_view::ListContext {
        i18n,
        base_path: &site.base_path,
        now: chrono::Utc::now(),
        categories: &categories,
        expand_all_pinned,
        member_trust_level: site.viewer.as_ref().map(|v| v.trust_level),
        settings: crate::topic_list_view::ListSettings::load(settings)?,
    };
    let users = list["users"].as_array().cloned().unwrap_or_default();
    let rows = list["topic_list"]["topics"]
        .as_array()
        .map(|topics| {
            topics
                .iter()
                .map(|t| crate::topic_list_view::row(&cx, t, &users))
                .collect()
        })
        .unwrap_or_default();
    // Discovery topics hides the footer while the list can load more.
    let mut site = site;
    if list["topic_list"]["more_topics_url"].is_string() {
        site.chrome.powered_by = false;
    }
    Ok(LatestPage {
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
        live_filter: String::new(),
        live_since: String::new(),
        nav: Vec::new(),
        banner: String::new(),
        breadcrumbs: String::new(),
        admin_controls: String::new(),
        more_url: list["topic_list"]["more_topics_url"]
            .as_str()
            .map(str::to_string),
    })
}

/// User#pmPath for Mark unread: a message the viewer is on goes to their
/// inbox, one they see through a group to that group's (the first of the
/// message's groups they are in); anything else to the home page.
async fn defer_to(
    conn: &mut PgConnection,
    view: &Value,
    base: &str,
    viewer: Option<&str>,
) -> Result<String, HtmlError> {
    let home = format!("{base}/");
    let Some(username) = viewer.filter(|_| view["archetype"] == "private_message") else {
        return Ok(home);
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
        return Ok(format!("{base}/u/{lower}/messages"));
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
        .map(|name| format!("{base}/u/{lower}/messages/group/{name}"))
        .unwrap_or(home))
}

/// The topic page from the /t/:id.json document.
pub async fn topic_page(
    conn: &mut PgConnection,
    i18n: &I18n,
    settings: &SiteSettings,
    site: Site,
    view: &Value,
    page: i64,
) -> Result<TopicPage, HtmlError> {
    let base = site.base_path.clone();
    let slug = s(&view["slug"]);
    let id = view["id"].as_i64().unwrap_or(0);
    let topic_url = format!("{base}/t/{slug}/{id}");

    let categories = crate::topic_list_view::categories(conn).await?;
    let list = crate::topic_list_view::ListContext {
        i18n,
        base_path: &base,
        now: chrono::Utc::now(),
        categories: &categories,
        expand_all_pinned: false,
        member_trust_level: site.viewer.as_ref().map(|v| v.trust_level),
        settings: crate::topic_list_view::ListSettings::load(settings)?,
    };
    let title_html = crate::post_view::topic_title(&list, view, &topic_url);
    let post_settings = crate::post_view::PostSettings::load(settings)?;
    let viewer = site.viewer.as_ref().map(|v| v.username.as_str());
    let staff = site.viewer.as_ref().is_some_and(|v| v.staff);
    let can_send_pms = site
        .viewer
        .as_ref()
        .is_some_and(|v| v.can_send_private_messages);
    let mut topic = crate::post_view::TopicInfo::from_view(view);
    topic.defer_to = defer_to(conn, view, &base, viewer).await?;
    // The first post's topic map, rendered before the posts that carry it.
    if crate::post_view::shows_op_map(view, &post_settings) {
        let cx = crate::post_view::PostContext {
            list: &list,
            settings: &post_settings,
            topic: &topic,
            viewer,
            staff,
            can_send_pms,
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
    };
    let posts = view["post_stream"]["posts"]
        .as_array()
        .map(|posts| crate::post_view::stream(&cx, posts))
        .unwrap_or_default();
    let bottom_map = if crate::post_view::shows_bottom_map(view, &post_settings) {
        crate::post_view::topic_map(&cx, view, "--bottom")
    } else {
        String::new()
    };
    let timeline = crate::post_view::timeline(&cx, view);
    let footer_buttons = crate::post_view::footer_buttons(&cx, view);
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
    let more_topics = crate::post_view::more_topics(&cx, view, tracking.as_ref())?;

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

    // topic.gjs hides the footer until the post stream is loaded to its end.
    let mut site = site;
    if page < last_page {
        site.chrome.powered_by = false;
    }
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
        title_html,
        posts,
        bottom_map,
        timeline,
        footer_buttons,
        more_topics,
        topic_id: id,
        // New posts land on the last page; earlier pages stay as they are.
        live: page >= last_page,
        prev_url: (page > 1).then(|| page_url(page - 1)),
        next_url: (page < last_page).then(|| page_url(page + 1)),
    })
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
}

/// The categories index from the /categories.json document
/// (categories/index.html.erb's table, plus featured topics).
pub async fn categories_page(
    conn: &mut PgConnection,
    _i18n: &I18n,
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
                                        title: crate::emoji::gsub_emoji_to_unicode(&s(&t["title"])),
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
    pub fn with_meta(mut self, title: &str, description: &str, image: Option<String>) -> Crawler {
        self.title = crate::emoji::gsub_emoji_to_unicode(title);
        self.og_description = crate::emoji::gsub_emoji_to_unicode(description);
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
}

impl CategoriesPage {
    /// No category preselected on the categories page.
    pub fn heading_id(&self) -> Option<i32> {
        None
    }
}
