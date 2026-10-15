//! The admin groups pages (admin-groups.gjs): the page header with its
//! Settings and Groups tabs over the groups list (GroupList, as /groups
//! draws it) or the groups settings (AdminAreaSettings for the groups
//! category).

use crate::admin_users_view::nav_item;
use crate::admin_view::{AdminCx, Crumb, PageHeader, t};
use crate::topic_list_view::escape;

/// Which tab is open.
#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Index,
    Settings,
}

/// The page: the header, then the tab's content in the config page's main
/// area.
pub fn render(cx: &AdminCx, tab: Tab, content: &str) -> String {
    let base = cx.base_path;
    let title = t(cx.i18n, "admin.config.groups.title");
    let settings = t(cx.i18n, "settings");
    let mut tabs = nav_item(
        "admin-groups-tabs__settings",
        &format!("{base}/admin/groups/settings"),
        &settings,
        tab == Tab::Settings,
    );
    tabs.push_str(&nav_item(
        "admin-groups-tabs__index",
        &format!("{base}/admin/groups"),
        &title,
        tab == Tab::Index,
    ));
    let mut crumbs = vec![
        Crumb {
            label: t(cx.i18n, "admin_title"),
            path: "/admin",
        },
        Crumb {
            label: title.clone(),
            path: "/admin/groups",
        },
    ];
    // AdminAreaSettings adds its own crumb.
    if tab == Tab::Settings {
        crumbs.push(Crumb {
            label: settings,
            path: "/admin/groups/settings",
        });
    }
    let mut out = PageHeader {
        crumbs,
        title,
        actions: String::new(),
        description: Some(escape(&t(
            cx.i18n,
            "admin.config.groups.header_description",
        ))),
        learn_more_url: None,
        tabs: Some(tabs),
    }
    .render(cx);
    out.push_str("<div class=\"admin-container admin-config-page__main-area\">");
    out.push_str(content);
    out.push_str("</div>");
    out
}
