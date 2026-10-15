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
