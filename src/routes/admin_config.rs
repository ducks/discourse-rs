//! The /admin/config areas whose page is only their settings (90g6):
//! each area's settings.gjs, a DPageHeader with its breadcrumbs over
//! AdminAreaSettings for the area's `filter_area` or `categories`. Their
//! JSON is Admin::Config::SiteSettingsController#index, as Rails routes
//! the paths there.

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::Response;
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
    router
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
