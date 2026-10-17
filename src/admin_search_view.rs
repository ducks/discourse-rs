//! The admin search (admin/components/admin-search.gjs) as the page
//! /admin/search (admin-search/index) draws it and the modal
//! (modal/admin-search.gjs) holds it: the input, the status and the
//! results, which admin-search.js fills from the data source's items.

use crate::admin_view::{AdminCx, Crumb, PageHeader, t};
use crate::topic_list_view::escape;

/// The strings admin-search.js formats with.
pub const STRINGS: &[&str] = &[
    "admin.search.result_count.one",
    "admin.search.result_count.other",
    "admin.search.no_results",
    "admin.search.modal_title",
    "admin.config.search_everything.header_shortcut",
];

/// AdminSearch before the data source is built: the spinner while there
/// is a filter to search for. An empty filter is the input's value
/// attribute as Glimmer first sets it; admin-search.js sets any other.
pub fn component(cx: &AdminCx, filter: &str) -> String {
    let loading = if filter.is_empty() {
        "<div class=\"loading-container\"></div>".to_string()
    } else {
        "<div class=\"loading-container visible\" data-loading=\"true\"><div class=\"spinner \"></div></div>"
            .to_string()
    };
    format!(
        "<div class=\"admin-search__input-container\"><div class=\"admin-search__input-group\">{}<input autofocus=\"\" class=\"admin-search__input-field\" placeholder=\"{}\" type=\"text\"{}></div></div><div aria-live=\"polite\" class=\"sr-only\" role=\"status\"></div><div class=\"admin-search__results\">{loading}</div>",
        crate::post_view::d_icon("magnifying-glass", Some("admin-search__input-icon")),
        escape(&t(cx.i18n, "admin.search.instructions")),
        if filter.is_empty() { " value=\"\"" } else { "" },
    )
}

/// The search page: DPageHeader and the component with the `filter`.
pub fn page(cx: &AdminCx, filter: &str) -> String {
    let title = t(cx.i18n, "admin.config.search_everything.title");
    let header = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path: "/admin/search",
            },
        ],
        title,
        actions: String::new(),
        // The shortcut's sentence admin-search.js adds where there is a
        // keyboard (capabilities.hasKeyboard).
        description: Some(escape(&t(
            cx.i18n,
            "admin.config.search_everything.header_description",
        ))),
        learn_more_url: None,
        tabs: Some(String::new()),
    }
    .render(cx);
    format!(
        "{header}<div class=\"admin-container admin-config-page__main-area\"><div class=\"admin-config-area__full-width\">{}</div></div>",
        component(cx, filter)
    )
}

/// What every admin page gives an admin for the search: the strings and
/// the modal's component, which admin-search.js clones into the modal the
/// sidebar's search box and `mod+/` open.
pub fn head(cx: &AdminCx) -> String {
    let strings: serde_json::Map<String, serde_json::Value> = STRINGS
        .iter()
        .map(|k| (k.to_string(), serde_json::Value::String(t(cx.i18n, k))))
        .collect();
    format!(
        "<script type=\"application/json\" id=\"admin-search-strings\">{}</script><template id=\"admin-search-modal-template\">{}</template>",
        serde_json::Value::Object(strings)
            .to_string()
            .replace('<', "\\u003c"),
        component(cx, "")
    )
}
