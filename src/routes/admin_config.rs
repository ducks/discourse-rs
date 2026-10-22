//! The /admin/config areas as pages (90g6). Those that are only their
//! settings: each area's settings.gjs, a DPageHeader with its breadcrumbs
//! over AdminAreaSettings for the area's `filter_area` or `categories`,
//! their JSON Admin::Config::SiteSettingsController#index as Rails routes
//! the paths there. And the pages of the flags and the permalinks, which
//! their controllers' routes hand an HTML request.

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::json;

use super::admin_pages::{Shell, wants_json};
use crate::admin_view::{Crumb, PageHeader, t};
use crate::session::current::AuthGuardian;
use crate::{AppError, AppState};

/// What AdminAreaSettings is given: `@area` or `@categories`.
#[derive(Clone, Copy)]
enum Filter {
    Area(&'static str),
    Categories(&'static str),
}

#[derive(Clone, Copy)]
struct Area {
    /// The path under /admin/config.
    path: &'static str,
    /// `admin.config.<key>`: the title and the header's description.
    key: &'static str,
    filter: Filter,
    learn_more: Option<&'static str>,
    /// The admin panel's link (its active one).
    link: &'static str,
    /// AdminController#adminContentsClassName for the route.
    classes: &'static str,
}

const fn area(
    path: &'static str,
    key: &'static str,
    filter: Filter,
    link: &'static str,
    classes: &'static str,
) -> Area {
    Area {
        path,
        key,
        filter,
        learn_more: None,
        link,
        classes,
    }
}

const AREAS: [Area; 19] = [
    area(
        "localization",
        "localization",
        Filter::Area("localization"),
        "admin_localization",
        "admin-config localization settings",
    ),
    area(
        "notifications",
        "notifications",
        Filter::Area("notifications"),
        "admin_notifications",
        "admin-config notifications settings",
    ),
    area(
        "analytics-and-seo",
        "analytics",
        Filter::Area("analytics"),
        "admin_analytics",
        "admin-config admin-analytics settings",
    ),
    area(
        "user-defaults",
        "user_defaults",
        Filter::Area("user_defaults"),
        "admin_user_defaults",
        "admin-config user-defaults settings",
    ),
    Area {
        learn_more: Some(
            "https://blog.discourse.org/2018/06/understanding-discourse-trust-levels/",
        ),
        ..area(
            "trust-levels",
            "trust_levels",
            Filter::Area("trust_levels"),
            "admin_trust_levels",
            "admin-config trust-levels settings",
        )
    },
    area(
        "group-permissions",
        "group_permissions",
        Filter::Area("group_permissions"),
        "admin_group_permissions",
        "admin-config group-permissions settings",
    ),
    area(
        "site-admin",
        "site_admin",
        Filter::Area("site_admin"),
        "admin_site_admin",
        "admin-config site-admin settings",
    ),
    area(
        "legal",
        "legal",
        Filter::Area("legal"),
        "admin_legal",
        "admin-config legal settings",
    ),
    area(
        "interface",
        "interface",
        Filter::Area("interface"),
        "admin_interface",
        "admin-config interface settings",
    ),
    area(
        "navigation",
        "navigation",
        Filter::Area("navigation"),
        "admin_navigation",
        "admin-config navigation settings",
    ),
    area(
        "security",
        "security",
        Filter::Categories("security"),
        "admin_security",
        "admin-config security settings",
    ),
    Area {
        learn_more: Some("https://meta.discourse.org/t/tips-for-preventing-spam/264020"),
        ..area(
            "spam",
            "spam",
            Filter::Categories("spam"),
            "admin_spam",
            "admin-config spam settings",
        )
    },
    area(
        "developer",
        "developer",
        Filter::Categories("developer"),
        "admin_developer",
        "admin-config developer settings",
    ),
    area(
        "rate-limits",
        "rate_limits",
        Filter::Categories("rate_limits"),
        "admin_rate_limits",
        "admin-config rate-limits settings",
    ),
    area(
        "onebox",
        "onebox",
        Filter::Categories("onebox"),
        "admin_onebox",
        "admin-config onebox settings",
    ),
    area(
        "files",
        "files",
        Filter::Categories("files"),
        "admin_files",
        "admin-config files settings",
    ),
    area(
        "search",
        "search",
        Filter::Categories("search"),
        "admin_search",
        "admin-config search settings",
    ),
    area(
        "experimental",
        "experimental",
        Filter::Area("experimental"),
        "admin_experimental",
        "admin-config experimental settings",
    ),
    area(
        "gifs",
        "gifs",
        Filter::Area("gifs"),
        "admin_gifs",
        "admin-config gifs settings",
    ),
];

/// The areas' routes, each page and its `.json`.
pub(super) fn routes(mut router: Router<AppState>) -> Router<AppState> {
    for a in AREAS {
        let handler = move |state: State<AppState>,
                            guardian: AuthGuardian,
                            headers: HeaderMap,
                            uri: Uri,
                            body: Bytes| async move {
            settings_page(state, guardian, headers, uri, body, a).await
        };
        router = router
            .route(&format!("/admin/config/{}", a.path), get(handler))
            .route(&format!("/admin/config/{}.json", a.path), get(handler));
    }
    tabbed_routes(router)
}

/// The area's settings page; its JSON (and anyone but an admin) goes to
/// the site settings controller.
async fn settings_page(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    a: Area,
) -> Result<Response, AppError> {
    if wants_json(&uri, &headers) || !guardian.is_admin() {
        return super::admin_site_settings::config_index(
            State(state),
            AuthGuardian(guardian),
            headers,
            uri,
            body,
        )
        .await;
    }
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(&mut conn, &state, &guardian, &headers, a.link).await?;
    let params = match a.filter {
        Filter::Area(v) => json!({ "filter_area": v }),
        Filter::Categories(v) => json!({ "categories": v }),
    };
    let settings =
        super::admin_advanced::area_settings(&mut conn, &state, &shell, &uri, params).await?;
    let cx = shell.cx(&state);
    let title = t(cx.i18n, &format!("admin.config.{}.title", a.key));
    let path = format!("/admin/config/{}", a.path);
    let header = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path: &path,
            },
        ],
        title: title.clone(),
        actions: String::new(),
        description: Some(t(
            cx.i18n,
            &format!("admin.config.{}.header_description", a.key),
        )),
        learn_more_url: a.learn_more.map(str::to_string),
        tabs: None,
    }
    .render(&cx);
    let content = format!("{header}<div class=\"admin-config-page__main-area\">{settings}</div>");
    let main = crate::admin_view::contents(a.classes, &content);
    shell.render(
        &state,
        &uri,
        &title,
        main,
        "site-settings.js",
        StatusCode::OK,
    )
}

/// A flags page: the list, its settings tab, a new flag or a flag's form.
pub(super) enum FlagsPage {
    List,
    Settings,
    New,
    Edit(String),
}

/// GET /admin/config/flags and under it, as the page (adminConfig.flags):
/// the site's flag types (Site#flag_types, by position), the header and
/// the tab's outlet; the new and edit forms go without the header.
pub(super) async fn flags_page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    page: FlagsPage,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(
        &mut conn,
        state,
        guardian,
        headers,
        "admin_moderation_flags",
    )
    .await?;
    let flags: Vec<crate::admin_flags::Flag> = sqlx::query_as(&format!(
        "SELECT {} FROM flags WHERE NOT score_type AND id <> 2 ORDER BY position",
        crate::admin_flags::COLUMNS
    ))
    .fetch_all(&mut *conn)
    .await?;
    let used: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT post_action_type_id::bigint FROM post_actions \
         UNION SELECT DISTINCT reviewable_score_type::bigint FROM reviewable_scores",
    )
    .fetch_all(&mut *conn)
    .await?;
    let base_path = shell.site.base_path.clone();
    let flag_types: Vec<serde_json::Value> = flags
        .iter()
        .map(|f| {
            crate::admin_flags::serialize(
                &state.i18n,
                &base_path,
                f,
                "post_action",
                used.contains(&f.id),
            )
        })
        .collect();
    let custom = flags.iter().filter(|f| !f.system()).count() as i64;
    let limit = shell.settings.get("custom_flags_limit")?.to_i();
    let cx = shell.cx(state);
    let t = |k: &str| crate::admin_view::t(cx.i18n, k);
    let (classes, title, content, script) = match &page {
        FlagsPage::List => (
            "admin-config flags",
            t("admin.config.flags.title"),
            format!(
                "{}<div class=\"admin-container admin-config-page__main-area\">{}</div>",
                crate::admin_config_view::flags_header(
                    &cx,
                    crate::admin_config_view::FlagsTab::Flags,
                    custom >= limit
                ),
                crate::admin_config_view::flags_list(&cx, &flag_types)
            ),
            "admin-config.js",
        ),
        FlagsPage::Settings => {
            let settings = super::admin_advanced::area_settings(
                &mut conn,
                state,
                &shell,
                uri,
                json!({ "filter_area": "flags" }),
            )
            .await?;
            (
                "admin-config flags settings",
                t("admin.config_areas.flags.settings"),
                format!(
                    "{}<div class=\"admin-container admin-config-page__main-area\">{settings}</div>",
                    crate::admin_config_view::flags_header(
                        &cx,
                        crate::admin_config_view::FlagsTab::Settings,
                        custom >= limit
                    )
                ),
                "site-settings.js",
            )
        }
        FlagsPage::New => (
            "admin-config flags new",
            String::new(),
            crate::admin_config_view::flag_form(&cx, None, &crate::admin_flags::VALID_APPLIES_TO),
            "admin-config.js",
        ),
        FlagsPage::Edit(id) => {
            // The route's model: the site's flag type, if any.
            let flag = id
                .parse::<i64>()
                .ok()
                .and_then(|id| flag_types.iter().find(|f| f["id"].as_i64() == Some(id)));
            (
                "admin-config flags edit",
                t("admin.config_areas.flags.edit_header"),
                crate::admin_config_view::flag_form(
                    &cx,
                    flag,
                    &crate::admin_flags::VALID_APPLIES_TO,
                ),
                "admin-config.js",
            )
        }
    };
    let strings = crate::admin_ui::strings(
        &cx,
        "admin-config-strings",
        &[
            "admin.config_areas.flags.delete_confirm",
            "form_kit.errors_summary_title.one",
            "form_kit.errors_summary_title.other",
            "form_kit.errors.required",
            "form_kit.errors.too_short.one",
            "form_kit.errors.too_short.other",
            "form_kit.errors.too_long.one",
            "form_kit.errors.too_long.other",
            "admin.config_areas.flags.form.invalid_applies_to",
            "select_kit.filter_placeholder",
            "select_kit.delete_item",
            "select_kit.no_content",
        ],
    );
    let main = crate::admin_view::contents(classes, &format!("{content}{strings}"));
    shell.render(state, uri, &title, main, script, StatusCode::OK)
}

/// A permalinks page: the list, its settings tab, a new permalink or a
/// permalink's form.
pub(super) enum PermalinksPage {
    List,
    Settings,
    New,
    Edit(i32),
}

/// GET /admin/config/permalinks and under it, as the page
/// (adminPermalinks): the header and the tab's outlet; the forms without
/// the header, as the reference draws them. The list's `filter` (what
/// admin-config.js asks for as the field changes) draws only the results.
pub(super) async fn permalinks_page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    page: PermalinksPage,
) -> Result<Response, AppError> {
    use crate::admin_config_view as view;
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(&mut conn, state, guardian, headers, "admin_permalinks").await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &shell.settings,
    };
    let base_path = shell.site.base_path.clone();
    let site_origin = urls.base_url_no_prefix()?;
    let categories = crate::topic_list_view::categories(&mut conn).await?;
    let list = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: &base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        hide_category: false,
        member_trust_level: None,
        settings: crate::topic_list_view::ListSettings::load(&shell.settings)?,
    };
    let cx = shell.cx(state);
    let query = crate::params::parse_query(uri.query().unwrap_or(""));
    let filter = crate::params::string(&query, "filter").filter(|f| !f.is_empty());
    let mut serialized = Vec::new();
    let (classes, outlet, header, script) = match page {
        PermalinksPage::List => {
            for p in crate::permalinks::filter_by(&mut conn, filter.as_deref()).await? {
                serialized
                    .push(crate::permalinks::serialize(&mut conn, &urls, &base_path, &p).await?);
            }
            if filter.is_some() {
                // The results a filter leaves, for the script to swap in.
                let html = view::permalinks_results(&cx, &list, &site_origin, &serialized);
                return Ok(axum::response::Html(html).into_response());
            }
            (
                "admin-config admin-permalinks",
                view::permalinks_list(&cx, &list, &site_origin, &serialized),
                view::permalinks_header(&cx, view::PermalinksTab::Permalinks),
                "admin-config.js",
            )
        }
        PermalinksPage::Settings => (
            "admin-config admin-permalinks settings",
            super::admin_advanced::area_settings(
                &mut conn,
                state,
                &shell,
                uri,
                json!({ "filter_area": "permalinks" }),
            )
            .await?,
            view::permalinks_header(&cx, view::PermalinksTab::Settings),
            "site-settings.js",
        ),
        PermalinksPage::New => (
            "admin-config admin-permalinks new",
            view::permalink_form(&cx, None),
            String::new(),
            "admin-config.js",
        ),
        PermalinksPage::Edit(id) => {
            // The route's model: store.find, its 404 the page's.
            let Some(p) = crate::permalinks::find(&mut conn, id).await? else {
                return Ok(super::topics::not_found_response(state));
            };
            let pl = crate::permalinks::serialize(&mut conn, &urls, &base_path, &p).await?;
            (
                "admin-config admin-permalinks edit",
                view::permalink_form(&cx, Some(&pl)),
                String::new(),
                "admin-config.js",
            )
        }
    };
    let strings = crate::admin_ui::strings(
        &cx,
        "admin-config-strings",
        &[
            "admin.permalink.delete_confirm",
            "admin.permalink.copy_success",
            "generic_error",
            "form_kit.errors_summary_title.one",
            "form_kit.errors_summary_title.other",
            "form_kit.errors.required",
        ],
    );
    let content = format!("{}{strings}", view::permalinks_page(&header, &outlet));
    let main = crate::admin_view::contents(classes, &content);
    let title = crate::admin_view::t(cx.i18n, "admin.config.permalinks.title");
    shell.render(state, uri, &title, main, script, StatusCode::OK)
}

/// A user fields page: the list, a new field, or a field's edit.
pub(super) enum UserFieldsPage {
    List,
    New,
    Edit(i32),
}

/// GET /admin/config/user-fields, /new and /:id/edit as the page
/// (adminUserFields): the header over the list or the form.
pub(super) async fn user_fields_page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    page: UserFieldsPage,
) -> Result<Response, AppError> {
    use crate::admin_config_view as view;
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(&mut conn, state, guardian, headers, "admin_user_fields").await?;
    let cx = shell.cx(state);
    let t = |k: &str| crate::admin_view::t(cx.i18n, k);
    let section = t("admin.user_fields.title");
    let with_header = matches!(page, UserFieldsPage::List);
    let (classes, outlet, title) = match page {
        UserFieldsPage::List => {
            let mut fields = Vec::new();
            for f in crate::user_fields::all(&mut conn).await? {
                fields.push(crate::user_fields::serialize(&mut conn, &f).await?);
            }
            // sortedUserFields
            fields.sort_by_key(|f| f["position"].as_i64().unwrap_or_default());
            (
                "admin-config admin-user-fields",
                view::user_fields_list(&cx, &fields),
                section,
            )
        }
        UserFieldsPage::New => (
            "admin-config admin-user-fields new",
            view::user_field_form(&cx, None),
            format!("{} - {section}", t("admin.user_fields.new_header")),
        ),
        UserFieldsPage::Edit(id) => {
            let Some(field) = crate::user_fields::find(&mut conn, id).await? else {
                return Ok(super::topics::not_found_response(state));
            };
            let field = crate::user_fields::serialize(&mut conn, &field).await?;
            (
                "admin-config admin-user-fields edit",
                view::user_field_form(&cx, Some(&field)),
                format!("{} - {section}", t("admin.user_fields.edit_header")),
            )
        }
    };
    let strings = crate::admin_ui::strings(
        &cx,
        "admin-config-strings",
        &[
            "admin.user_fields.delete_confirm",
            "admin.user_fields.requirement.confirmation",
            "admin.config_areas.user_fields.delete_successful",
            "admin.config_areas.user_fields.save_successful",
            "form_kit.errors_summary_title.one",
            "form_kit.errors_summary_title.other",
            "form_kit.errors.required",
            "select_kit.filter_placeholder",
        ],
    );
    let content = format!(
        "{}{strings}",
        view::user_fields_page(&cx, with_header, &outlet)
    );
    let main = crate::admin_view::contents(classes, &content);
    shell.render(state, uri, &title, main, "admin-config.js", StatusCode::OK)
}

/// An area whose tabs are each AdminAreaSettings (content, login): the
/// tab's path under the area, its DNavItem label, its route's class, the
/// AdminAreaSettings area, and whether it shows its breadcrumb.
struct Tab {
    path: &'static str,
    label: &'static str,
    class: &'static str,
    area: &'static str,
    crumb: bool,
}

struct TabbedArea {
    path: &'static str,
    key: &'static str,
    link: &'static str,
    /// Each tab's title is the area's (true) or the tab's label (false).
    area_title: bool,
    tabs: &'static [Tab],
}

const CONTENT: TabbedArea = TabbedArea {
    path: "content",
    key: "content",
    link: "admin_content",
    area_title: false,
    tabs: &[
        Tab {
            path: "",
            label: "admin.config.content.sub_pages.categories_and_tags.title",
            class: "categories-and-tags",
            area: "categories_and_tags",
            crumb: false,
        },
        Tab {
            path: "sharing",
            label: "admin.config.content.sub_pages.sharing.title",
            class: "sharing",
            area: "sharing",
            crumb: false,
        },
        Tab {
            path: "posts-and-topics",
            label: "admin.config.content.sub_pages.posts_and_topics.title",
            class: "posts-and-topics",
            area: "posts_and_topics",
            crumb: false,
        },
        Tab {
            path: "stats-and-thresholds",
            label: "admin.config.content.sub_pages.stats_and_thresholds.title",
            class: "stats-and-thresholds",
            area: "stats_and_thresholds",
            crumb: false,
        },
    ],
};

/// The plugins' login tabs (site.admin_config_login_routes) are not
/// here: no plugin registers one until the plugins run (crate::site).
const LOGIN: TabbedArea = TabbedArea {
    path: "login-and-authentication",
    key: "login",
    link: "admin_login",
    area_title: true,
    tabs: &[
        Tab {
            path: "",
            label: "admin.config.login.sub_pages.common_settings.title",
            class: "settings",
            area: "login",
            crumb: true,
        },
        Tab {
            path: "discourse-id",
            label: "admin.config.login.sub_pages.discourse_id.title",
            class: "discourse-id",
            area: "",
            crumb: false,
        },
        Tab {
            path: "authenticators",
            label: "admin.config.login.sub_pages.authenticators.title",
            class: "authenticators",
            area: "authenticators",
            crumb: false,
        },
        Tab {
            path: "discourseconnect",
            label: "admin.config.login.sub_pages.discourseconnect.title",
            class: "discourseconnect",
            area: "discourseconnect",
            crumb: false,
        },
    ],
};

/// The tabbed areas' routes: each tab and its `.json`, and content's
/// settings route, which goes to the area's first tab.
pub(super) fn tabbed_routes(mut router: Router<AppState>) -> Router<AppState> {
    for area in [&CONTENT, &LOGIN] {
        for (index, tab) in area.tabs.iter().enumerate() {
            let path = if tab.path.is_empty() {
                format!("/admin/config/{}", area.path)
            } else {
                format!("/admin/config/{}/{}", area.path, tab.path)
            };
            let handler = move |state: State<AppState>,
                                guardian: AuthGuardian,
                                headers: HeaderMap,
                                uri: Uri,
                                body: Bytes| async move {
                tabbed_page(state, guardian, headers, uri, body, area, index).await
            };
            router = router
                .route(&path, get(handler))
                .route(&format!("{path}.json"), get(handler));
        }
    }
    router.route(
        "/admin/config/content/settings",
        get(
            |State(state): State<AppState>,
             AuthGuardian(guardian): AuthGuardian,
             headers: HeaderMap,
             uri: Uri,
             body: Bytes| async move {
                if wants_json(&uri, &headers) || !guardian.is_admin() {
                    return super::admin_site_settings::config_index(
                        State(state),
                        AuthGuardian(guardian),
                        headers,
                        uri,
                        body,
                    )
                    .await;
                }
                let to = format!(
                    "{}/admin/config/content",
                    state.config.globals.relative_url_root()
                );
                Ok::<Response, AppError>(axum::response::Redirect::to(&to).into_response())
            },
        ),
    )
}

/// A tab of a tabbed area: the area's header with its tabs over the tab's
/// AdminAreaSettings. Discourse ID's tab, whose controller is not ported,
/// is refused.
async fn tabbed_page(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    area: &'static TabbedArea,
    index: usize,
) -> Result<Response, AppError> {
    let tab = &area.tabs[index];
    if tab.area.is_empty() {
        // Admin::Config::DiscourseIdController
        if !guardian.is_staff() {
            return Ok(super::topics::not_found_response(&state));
        }
        return Err(crate::Unsupported("the Discourse ID admin page and its JSON").into());
    }
    if wants_json(&uri, &headers) || !guardian.is_admin() {
        return super::admin_site_settings::config_index(
            State(state),
            AuthGuardian(guardian),
            headers,
            uri,
            body,
        )
        .await;
    }
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(&mut conn, &state, &guardian, &headers, area.link).await?;
    let settings = super::admin_advanced::area_settings(
        &mut conn,
        &state,
        &shell,
        &uri,
        json!({ "filter_area": tab.area }),
    )
    .await?;
    let cx = shell.cx(&state);
    let area_path = format!("/admin/config/{}", area.path);
    let tab_path = |t: &Tab| {
        if t.path.is_empty() {
            area_path.clone()
        } else {
            format!("{area_path}/{}", t.path)
        }
    };
    let title = t(cx.i18n, &format!("admin.config.{}.title", area.key));
    let mut crumbs = vec![
        Crumb {
            label: t(cx.i18n, "admin_title"),
            path: "/admin",
        },
        Crumb {
            label: title.clone(),
            path: &area_path,
        },
    ];
    let own_path = tab_path(tab);
    if tab.crumb {
        crumbs.push(Crumb {
            label: t(cx.i18n, "settings"),
            path: &own_path,
        });
    }
    let paths: Vec<String> = area.tabs.iter().map(tab_path).collect();
    let items: Vec<(&str, &str, &str, bool)> = area
        .tabs
        .iter()
        .zip(paths.iter())
        .enumerate()
        .map(|(i, (t, p))| ("", p.as_str(), t.label, i == index))
        .collect();
    let header = PageHeader {
        crumbs,
        title: title.clone(),
        actions: String::new(),
        description: Some(t(
            cx.i18n,
            &format!("admin.config.{}.header_description", area.key),
        )),
        learn_more_url: None,
        tabs: Some(crate::admin_ui::tabs(&cx, &items)),
    }
    .render(&cx);
    let content = format!("{header}<div class=\"admin-config-page__main-area\">{settings}</div>");
    let classes = format!("admin-config {} {}", area.key, tab.class);
    let main = crate::admin_view::contents(&classes, &content);
    let page_title = if area.area_title {
        title
    } else {
        t(cx.i18n, tab.label)
    };
    shell.render(
        &state,
        &uri,
        &page_title,
        main,
        "site-settings.js",
        StatusCode::OK,
    )
}
