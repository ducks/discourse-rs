//! The admin pages as HTML: the admin shell (crate::admin_view) around
//! each page's content, rendered from the documents the admin JSON routes
//! serve. GET /admin is the dashboard (crate::admin_dashboard_view).

use askama::Template;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::admin_view::{AdminCx, AdminViewer};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// An admin page: the layout with the admin shell's content.
#[derive(Template)]
#[template(path = "admin.html")]
pub struct AdminHtml {
    pub site_title: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: crate::html::Crawler,
    pub viewer: Option<crate::html::Viewer>,
    pub bus_position: String,
    pub chrome: crate::html::Chrome,
    /// The route's title tokens before the site's title
    /// (`Dashboard - Admin`).
    pub page_title: String,
    pub main: String,
    /// The page's own script (assets name), empty for none.
    pub script: &'static str,
}

/// What every admin page loads: the settings, the shell's chrome with the
/// admin panel in place of the forum's sidebar, and the viewer as the
/// shell reads them.
pub struct Shell {
    pub settings: SiteSettings,
    pub site: crate::html::Site,
    pub viewer_state: crate::html::ViewerState,
    pub admin: AdminViewer,
}

impl Shell {
    pub async fn load(
        conn: &mut sqlx::PgConnection,
        state: &AppState,
        guardian: &crate::guardian::Guardian,
        headers: &HeaderMap,
        active: &str,
    ) -> Result<Shell, AppError> {
        let bus_position = crate::bus::page_position(&state.bus).await?;
        let settings =
            SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
        let base_path = state.config.globals.relative_url_root();
        let viewer_state = super::session::viewer_state(state, headers, &settings, guardian)?;
        let mut site = crate::html::Site::from_settings(&settings, base_path)?;
        site.viewer = viewer_state.viewer.clone();
        site.load_chrome(
            &mut *conn,
            state,
            &settings,
            guardian,
            crate::sidebar::Active::None,
        )
        .await?;
        site.bus_position = bus_position;
        let admin = match guardian.user() {
            Some(user) => AdminViewer::from_current_user(
                &crate::current_user::serialize(&mut *conn, state, &settings, user, false).await?,
            ),
            None => AdminViewer::default(),
        };
        let cx = AdminCx {
            i18n: &state.i18n,
            settings: &settings,
            base_path,
            viewer: &admin,
        };
        // The admin route forces the admin panel (maybeForceAdminSidebar),
        // whatever navigation_menu says; the mobile view's header menu
        // shows it too, as does the narrow desktop view's toggle, whose
        // sidebar is enabled but not shown.
        let (sidebar, hamburger) =
            if crate::mobile::mobile_view() || crate::mobile::narrow_desktop_view() {
                (String::new(), crate::admin_view::hamburger(&cx, active)?)
            } else {
                (crate::admin_view::sidebar(&cx, active)?, String::new())
            };
        site.chrome.use_admin_panel(sidebar, hamburger);
        Ok(Shell {
            settings,
            site,
            viewer_state,
            admin,
        })
    }

    pub fn cx<'a>(&'a self, state: &'a AppState) -> AdminCx<'a> {
        AdminCx {
            i18n: &state.i18n,
            settings: &self.settings,
            base_path: &self.site.base_path,
            viewer: &self.admin,
        }
    }

    /// The page with `main` as the admin route's content.
    pub fn render(
        self,
        state: &AppState,
        uri: &Uri,
        title: &str,
        main: String,
        script: &'static str,
        status: StatusCode,
    ) -> Result<Response, AppError> {
        let urls = crate::url::Urls {
            config: &state.config,
            settings: &self.settings,
        };
        let mut crawler = crate::html::Crawler::for_request(&urls, uri, None)?;
        crawler.description = String::new();
        let page = AdminHtml {
            site_title: self.site.site_title,
            lang: self.site.lang,
            base_path: self.site.base_path,
            crawler,
            viewer: self.site.viewer,
            bus_position: self.site.bus_position,
            chrome: self.site.chrome,
            page_title: format!(
                "{title} - {}",
                crate::admin_view::t(&state.i18n, "admin_title")
            ),
            main,
            script,
        };
        let body = page.render().map_err(crate::html::HtmlError::from)?;
        let mut response =
            crate::html::crawler_response(body, &page.crawler, &self.settings, &self.viewer_state)?;
        *response.status_mut() = status;
        Ok(response)
    }
}

/// The admin routes' StaffConstraint: anyone else gets not found.
fn refuse_non_staff(state: &AppState, guardian: &crate::guardian::Guardian) -> Option<Response> {
    (!guardian.is_staff()).then(|| super::topics::not_found_response(state))
}

#[derive(Deserialize, Default)]
pub struct DashboardParams {
    range: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
}

/// GET /admin (admin#index as HTML): the dashboard, from the document
/// /admin/dashboard.json serves for the controller's range. AdminController
/// is admin only; a moderator's request is rescued as InvalidAccess with
/// the Ember layout (403), whose client then routes to the dashboard all
/// the same.
pub async fn dashboard(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    Query(params): Query<DashboardParams>,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_staff(&state, &guardian) {
        return Ok(r);
    }
    if super::wants_json(uri.query(), &headers)
        || headers
            .get("x-requested-with")
            .is_some_and(|v| v == "XMLHttpRequest")
    {
        // `render body: nil`
        return Ok(StatusCode::OK.into_response());
    }
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(&mut conn, &state, &guardian, &headers, "admin_home").await?;
    let improvements = guardian
        .upcoming_change_enabled(&mut conn, &shell.settings, "dashboard_improvements")
        .await?;
    if !improvements {
        return Err(crate::Unsupported(
            "the legacy admin dashboard page (dashboard_improvements off)",
        )
        .into());
    }
    let today = crate::clock::now().date_naive();
    let range = crate::admin_dashboard_view::Range::from_params(
        params.range.as_deref(),
        params.start_date.as_deref(),
        params.end_date.as_deref(),
        today,
    );
    let doc = super::admin_dashboard_sections::document(
        &mut conn,
        &state,
        &guardian,
        &shell.settings,
        &range.start.format("%Y-%m-%d").to_string(),
        &range.end.format("%Y-%m-%d").to_string(),
    )
    .await?;
    let categories = crate::topic_list_view::categories(&mut conn).await?;
    let list = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: &shell.site.base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        member_trust_level: None,
        settings: crate::topic_list_view::ListSettings::load(&shell.settings)?,
    };
    let cx = shell.cx(&state);
    let content = crate::admin_dashboard_view::Dashboard {
        admin: &cx,
        list: &list,
        range: &range,
        doc: &doc,
        use_legacy_pageviews: shell.settings.get("use_legacy_pageviews")?.truthy(),
    }
    .render()?;
    let main = crate::admin_view::contents("dashboard general dashboard-next", &content);
    let title = crate::admin_view::t(&state.i18n, "admin.config.dashboard.title");
    let status = if guardian.is_admin() {
        StatusCode::OK
    } else {
        StatusCode::FORBIDDEN
    };
    shell.render(&state, &uri, &title, main, "admin-dashboard.js", status)
}

/// GET /admin/users/:id/:username: the user's admin page (adminUser
/// index), from Admin::UsersController#show's document, under the users
/// pages' header.
pub async fn user_show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    axum::extract::Path((id, _username)): axum::extract::Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_staff(&state, &guardian) {
        return Ok(r);
    }
    // User.find_by(id:): a non-numeric id finds nobody.
    let Ok(id) = id.parse::<i32>() else {
        return Ok(super::topics::not_found_response(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(&mut conn, &state, &guardian, &headers, "admin_users").await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &shell.settings,
    };
    let show_cx = crate::admin_user_show::Context {
        settings: &shell.settings,
        defs: &state.site_setting_defs,
        i18n: &state.i18n,
        urls: &urls,
        globals: &state.config.globals,
        development: state.config.rails_env == crate::config::RailsEnv::Development,
    };
    let Some(user) = crate::admin_user_show::show(
        &mut conn,
        &show_cx,
        &guardian,
        id,
        crate::admin_user_show::ShowOptions {
            show: true,
            detailed: true,
        },
    )
    .await?
    else {
        return Ok(super::topics::not_found_response(&state));
    };
    let user_fields: Vec<(i64, String)> = sqlx::query_as::<_, (i32, String)>(
        "SELECT id, name FROM user_fields ORDER BY position, id",
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|(id, name)| (i64::from(id), name))
    .collect();
    let categories = crate::topic_list_view::categories(&mut conn).await?;
    let list = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: &shell.site.base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        member_trust_level: None,
        settings: crate::topic_list_view::ListSettings::load(&shell.settings)?,
    };
    let cx = shell.cx(&state);
    let header_viewer = crate::admin_users_view::HeaderViewer {
        admin: shell.admin.admin,
        can_invite_to_forum: shell.admin.can_invite_to_forum,
    };
    let page = crate::admin_user_view::UserPage {
        user_fields: &user_fields,
        now: crate::clock::now(),
    };
    let content = format!(
        "<div class=\"admin-users admin-config-page\">{}<div class=\"admin-container admin-config-page__main-area\"></div></div><div class=\"admin-container admin-config-page__main-area\">{}</div>",
        crate::admin_users_view::users_header(&cx, &header_viewer, ""),
        crate::admin_user_view::render(&cx, &list, &page, &user)
    );
    let main = crate::admin_view::contents("admin-users admin-user", &content);
    let title = format!(
        "{} - {}",
        user["username"].as_str().unwrap_or_default(),
        crate::admin_view::t(&state.i18n, "admin.config.users.title")
    );
    let main = main.replacen(
        "<div class=\"admin-contents admin-users admin-user\">",
        &format!(
            "<div class=\"admin-contents admin-users admin-user\" data-user-id=\"{id}\" data-username=\"{}\">",
            crate::topic_list_view::escape(user["username"].as_str().unwrap_or_default())
        ),
        1,
    );
    shell.render(&state, &uri, &title, main, "admin-user.js", StatusCode::OK)
}

/// GET /admin/users/settings: the users pages' settings tab, an
/// AdminAreaSettings for the users area.
pub async fn users_settings(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_staff(&state, &guardian) {
        return Ok(r);
    }
    let filters = settings_filters(&uri);
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(&mut conn, &state, &guardian, &headers, "admin_users").await?;
    // /admin/config/site_settings.json for `filter_area: "users"`.
    let area = super::admin_site_settings::config_filters(
        &state,
        serde_json::json!({ "filter_area": "users" })
            .as_object()
            .ok_or_else(|| std::io::Error::other("params"))?,
    )
    .ok_or_else(|| std::io::Error::other("the users settings' filters"))?;
    let (site_settings, default_theme) =
        super::admin_site_settings::document(&mut conn, &state, &shell.settings, &area).await?;
    let (groups, site_categories) = site_lists(&mut conn).await?;
    let cx = shell.cx(&state);
    let header_viewer = crate::admin_users_view::HeaderViewer {
        admin: shell.admin.admin,
        can_invite_to_forum: shell.admin.can_invite_to_forum,
    };
    let page = crate::admin_site_settings_view::Page {
        settings: &site_settings,
        default_theme: &default_theme,
        groups: &groups,
        categories: &site_categories,
        filters: &filters,
        category: "all_results",
    };
    let content = format!(
        "<div class=\"admin-users admin-config-page\">{}<div class=\"admin-container admin-config-page__main-area\"></div></div><div class=\"admin-container admin-config-page__main-area\">{}</div>",
        crate::admin_users_view::users_header(&cx, &header_viewer, "settings"),
        crate::admin_site_settings_view::area_settings(&cx, &page)
    );
    let main = crate::admin_view::contents("admin-users settings", &content);
    let title = crate::admin_view::t(&state.i18n, "settings");
    shell.render(
        &state,
        &uri,
        &title,
        main,
        "site-settings.js",
        StatusCode::OK,
    )
}

/// GET /admin/users/:id/:username/badges and /tl3_requirements: the user
/// page's tabs, not ported yet.
pub async fn user_subpage(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_staff(&state, &guardian) {
        return Ok(r);
    }
    Err(crate::Unsupported("the admin user's badges and TL3 requirements pages").into())
}

/// GET /admin/site_settings: the index's JSON, else the first category's
/// page (adminSiteSettings index replaces itself with `required`).
pub async fn site_settings_root(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    if wants_json(&uri, &headers) || !guardian.is_admin() {
        return super::admin_site_settings::index(
            State(state),
            AuthGuardian(guardian),
            headers,
            uri,
            body,
        )
        .await;
    }
    let base = state.config.globals.relative_url_root();
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    Ok(axum::response::Redirect::to(&format!(
        "{base}/admin/site_settings/category/required{query}"
    ))
    .into_response())
}

/// GET /admin/site_settings/category/:category: the site settings page for
/// the category, filtered by the route's query params; a category with no
/// matching setting goes to All.
pub async fn site_settings_category(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    axum::extract::Path(category): axum::extract::Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    // The site settings are under AdminConstraint.
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let filters = settings_filters(&uri);
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(
        &mut conn,
        &state,
        &guardian,
        &headers,
        "admin_all_site_settings",
    )
    .await?;
    let (site_settings, default_theme) = super::admin_site_settings::document(
        &mut conn,
        &state,
        &shell.settings,
        &Default::default(),
    )
    .await?;
    let categories = crate::admin_site_settings_view::filter(&site_settings, &filters);
    let found = categories
        .iter()
        .any(|c| c.key == category && !c.settings.is_empty());
    if !found && category != "all_results" {
        let base = state.config.globals.relative_url_root();
        let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
        return Ok(axum::response::Redirect::to(&format!(
            "{base}/admin/site_settings/category/all_results{query}"
        ))
        .into_response());
    }
    let (groups, site_categories) = site_lists(&mut conn).await?;
    let cx = shell.cx(&state);
    let page = crate::admin_site_settings_view::Page {
        settings: &site_settings,
        default_theme: &default_theme,
        groups: &groups,
        categories: &site_categories,
        filters: &filters,
        category: &category,
    };
    let content = crate::admin_site_settings_view::render(&cx, &page, &categories);
    let main =
        crate::admin_view::contents("admin-site-settings admin-site-settings-category", &content);
    let title = crate::admin_view::t(&state.i18n, "admin.config.site_settings.title");
    shell.render(
        &state,
        &uri,
        &title,
        main,
        "site-settings.js",
        StatusCode::OK,
    )
}

/// The site settings filters in the route's query params.
fn settings_filters(uri: &Uri) -> crate::admin_site_settings_view::Filters {
    let mut filters = crate::admin_site_settings_view::Filters::default();
    for (k, v) in form_urlencoded::parse(uri.query().unwrap_or("").as_bytes()) {
        match k.as_ref() {
            "filter" => filters.filter = v.into_owned(),
            "onlyOverridden" => filters.only_overridden = v == "true",
            "dependsOn" if !v.is_empty() => filters.depends_on = Some(v.into_owned()),
            _ => {}
        }
    }
    filters
}

/// Site#groups and Site#categories (id, name) for the group and category
/// settings' choosers.
async fn site_lists(
    conn: &mut sqlx::PgConnection,
) -> Result<(Vec<(i64, String)>, Vec<(i64, String)>), AppError> {
    let list = |rows: Vec<(i32, String)>| -> Vec<(i64, String)> {
        rows.into_iter()
            .map(|(id, name)| (i64::from(id), name))
            .collect()
    };
    let groups = sqlx::query_as::<_, (i32, String)>("SELECT id, name FROM groups ORDER BY name")
        .fetch_all(&mut *conn)
        .await?;
    let categories =
        sqlx::query_as::<_, (i32, String)>("SELECT id, name FROM categories ORDER BY id")
            .fetch_all(&mut *conn)
            .await?;
    Ok((list(groups), list(categories)))
}

/// GET /admin/groups: the groups list under the admin groups header, as
/// /groups lists them; its JSON is the empty index's 204.
pub async fn groups_index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = refuse_non_staff(&state, &guardian) {
        return Ok(r);
    }
    if wants_json(&uri, &headers) {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let p = crate::params::parse(uri.query(), &headers, &[]);
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(&mut conn, &state, &guardian, &headers, "admin_groups").await?;
    // The route's model: store.findAll("group", params), groups#index.
    let doc = match super::groups::index_doc(
        &state,
        &mut conn,
        &shell.settings,
        &guardian,
        &p,
        crate::mobile::mobile_view(),
    )
    .await?
    {
        Ok(doc) => doc,
        Err(refused) => return Ok(refused),
    };
    let categories = std::collections::HashMap::new();
    let list_cx = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: &shell.site.base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        member_trust_level: None,
        settings: crate::topic_list_view::ListSettings::load(&shell.settings)?,
    };
    let filter = crate::params::string(&p, "filter").unwrap_or_default();
    let kind = crate::params::string(&p, "type");
    let can_create_group = guardian.is_admin()
        || (shell.settings.get("moderators_manage_groups")?.truthy() && guardian.is_moderator());
    let list = crate::groups_view::render_index(
        &list_cx,
        &crate::groups_view::Index {
            doc: &doc,
            filter: &filter,
            kind: kind.as_deref(),
            can_create_group,
        },
    );
    let cx = shell.cx(&state);
    let content =
        crate::admin_groups_view::render(&cx, crate::admin_groups_view::Tab::Index, &list);
    let main = crate::admin_view::contents("admin-groups", &content);
    let title = crate::admin_view::t(&state.i18n, "admin.config.groups.title");
    shell.render(&state, &uri, &title, main, "groups.js", StatusCode::OK)
}

/// GET /admin/groups/settings: AdminAreaSettings for the groups category;
/// its JSON is Admin::SiteSettingsController#index's.
pub async fn groups_settings(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    if wants_json(&uri, &headers) || !guardian.is_admin() {
        return super::admin_site_settings::index(
            State(state),
            AuthGuardian(guardian),
            headers,
            uri,
            body,
        )
        .await;
    }
    let filters = settings_filters(&uri);
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(&mut conn, &state, &guardian, &headers, "admin_groups").await?;
    // /admin/config/site_settings.json for `categories: "groups"`.
    let area = super::admin_site_settings::config_filters(
        &state,
        serde_json::json!({ "categories": "groups" })
            .as_object()
            .ok_or_else(|| std::io::Error::other("params"))?,
    )
    .ok_or_else(|| std::io::Error::other("the groups settings' filters"))?;
    let (site_settings, default_theme) =
        super::admin_site_settings::document(&mut conn, &state, &shell.settings, &area).await?;
    let (groups, site_categories) = site_lists(&mut conn).await?;
    let cx = shell.cx(&state);
    let page = crate::admin_site_settings_view::Page {
        settings: &site_settings,
        default_theme: &default_theme,
        groups: &groups,
        categories: &site_categories,
        filters: &filters,
        category: "all_results",
    };
    let content = crate::admin_groups_view::render(
        &cx,
        crate::admin_groups_view::Tab::Settings,
        &crate::admin_site_settings_view::area_settings(&cx, &page),
    );
    let main = crate::admin_view::contents("admin-groups settings", &content);
    let title = crate::admin_view::t(&state.i18n, "settings");
    shell.render(
        &state,
        &uri,
        &title,
        main,
        "site-settings.js",
        StatusCode::OK,
    )
}

/// A request for JSON rather than a page: `.json`, XHR, or an Accept the
/// format negotiation reads as JSON.
fn wants_json(uri: &Uri, headers: &HeaderMap) -> bool {
    uri.path().ends_with(".json")
        || headers
            .get("x-requested-with")
            .is_some_and(|v| v == "XMLHttpRequest")
        || super::wants_json(uri.query(), headers)
}

/// GET /admin/users and /admin/users/list: the index's JSON, else for a
/// page the active users' list, where the client's adminUsers and
/// adminUsersList index routes send it.
pub async fn users_root(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if wants_json(&uri, &headers) || !guardian.is_staff() {
        return super::admin_users::index(State(state), AuthGuardian(guardian), None, uri).await;
    }
    let base = state.config.globals.relative_url_root();
    Ok(axum::response::Redirect::to(&format!("{base}/admin/users/list/active")).into_response())
}

/// The users list's query params.
#[derive(Default)]
struct UsersListParams {
    filter: Option<String>,
    username: Option<String>,
    order: Option<String>,
    asc: Option<String>,
    show_emails: Option<String>,
    account_type: Option<String>,
    activation: Option<String>,
    /// DLoadMore's next page, which admin-users.js loads.
    page: Option<String>,
}

impl UsersListParams {
    fn parse(query: Option<&str>) -> UsersListParams {
        let mut p = UsersListParams::default();
        for (k, v) in form_urlencoded::parse(query.unwrap_or("").as_bytes()) {
            let v = Some(v.into_owned());
            match k.as_ref() {
                "filter" => p.filter = v,
                "username" => p.username = v,
                "order" => p.order = v,
                "asc" => p.asc = v,
                "show_emails" => p.show_emails = v,
                "account_type" => p.account_type = v,
                "activation" => p.activation = v,
                "page" => p.page = v,
                _ => {}
            }
        }
        p
    }
}

/// GET /admin/users/list/:query: the index's JSON, else the list's page
/// (adminUsersList.show) with the users of its first page, for the
/// controller's filter, order and dropdowns as the URL carries them.
pub async fn users_list(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    axum::extract::Path(query): axum::extract::Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if wants_json(&uri, &headers) || !guardian.is_staff() {
        return super::admin_users::index(
            State(state),
            AuthGuardian(guardian),
            Some(axum::extract::Path(query)),
            uri,
        )
        .await;
    }
    let params = UsersListParams::parse(uri.query());
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(&mut conn, &state, &guardian, &headers, "admin_users").await?;
    let filter = params.filter.or(params.username).unwrap_or_default();
    let asc = params.asc.as_deref().is_some_and(|a| a == "true");
    let account_type = match params.account_type.as_deref() {
        Some(t @ ("human" | "bot" | "all")) if query == "staff" => t.to_string(),
        _ => "human".to_string(),
    };
    let activation = match params.activation.as_deref() {
        Some(a @ ("activated" | "not_activated")) if query == "new" => a.to_string(),
        _ => "all".to_string(),
    };
    let can_check_emails = shell.admin.admin
        || (shell.settings.get("moderators_view_emails")?.truthy() && shell.admin.staff);
    let show_emails = can_check_emails && params.show_emails.as_deref() == Some("true");
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &shell.settings,
    };
    let p = crate::admin_users::ListParams {
        query: Some(query.clone()),
        order: params.order.clone(),
        asc,
        page: params
            .page
            .as_deref()
            .map(crate::ruby::to_i)
            .unwrap_or(1)
            .max(1),
        show_emails,
        filter: Some(filter.clone()).filter(|f| !f.is_empty()),
        account_type: (query == "staff").then(|| account_type.clone()),
        activation: (query == "new" && activation != "all").then(|| activation.clone()),
        ..Default::default()
    };
    let users = match crate::admin_users::list(
        &mut conn,
        &shell.settings,
        &urls,
        &guardian,
        &p,
        &format!("/admin/users/list/{query}.json"),
    )
    .await?
    {
        crate::admin_users::Listed::Users(users) => users,
        crate::admin_users::Listed::InvalidFilter => {
            return Ok(super::search::invalid_parameters(&state, "filter"));
        }
    };
    let categories = crate::topic_list_view::categories(&mut conn).await?;
    let list = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: &shell.site.base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        member_trust_level: None,
        settings: crate::topic_list_view::ListSettings::load(&shell.settings)?,
    };
    let cx = shell.cx(&state);
    let header_viewer = crate::admin_users_view::HeaderViewer {
        admin: shell.admin.admin,
        can_invite_to_forum: shell.admin.can_invite_to_forum,
    };
    let s = crate::admin_users_view::ListState {
        query: &query,
        filter: &filter,
        order: params.order.as_deref(),
        asc,
        show_emails,
        can_check_emails,
        account_type: &account_type,
        activation: &activation,
        must_approve_users: shell.settings.get("must_approve_users")?.truthy(),
        more: users.len() >= 100,
    };
    // admin-users.gjs draws its header and an empty main area, then the
    // outlet's main area with the list.
    let content = format!(
        "<div class=\"admin-users admin-config-page\">{}<div class=\"admin-container admin-config-page__main-area\"></div></div><div class=\"admin-container admin-config-page__main-area\">{}</div>",
        crate::admin_users_view::users_header(&cx, &header_viewer, &query),
        crate::admin_users_view::list(&cx, &list, &s, &users)
    );
    let main = crate::admin_view::contents("admin-users admin-users-list", &content);
    let title = crate::admin_view::t(&state.i18n, "admin.config.users.title");
    shell.render(&state, &uri, &title, main, "admin-users.js", StatusCode::OK)
}
