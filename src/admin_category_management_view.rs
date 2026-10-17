//! The category management pages (90g6), as Ember draws them:
//! templates/admin-config/category-management.gjs (DPageHeader with
//! Create category, the Settings, All categories and each category
//! type's tabs) over AdminCategoryManagementList (DPageSubheader, the
//! filter controls, the table with DLoadMore) or the settings tab. The
//! list's script is static/js/admin-category-management.js.

use serde_json::Value;

use crate::admin_ui::{self, Action, Button};
use crate::admin_view::{AdminCx, Crumb, PageHeader, t, t_with};
use crate::topic_list_view::{emoji_unescape, escape, icon};

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

fn k(cx: &AdminCx, key: &str) -> String {
    t(cx.i18n, &format!("admin.config.category_management.{key}"))
}

/// The page: the header with its tabs (`active` the tab's id) over the
/// outlet.
pub fn page(cx: &AdminCx, types: &Value, active: &str, outlet: &str) -> String {
    let title = k(cx, "title");
    let mut items: Vec<(String, String, String, bool)> = vec![
        (
            "/admin/config/category-management/settings".into(),
            "admin.config.category_management.tabs.settings".into(),
            String::new(),
            active == "settings",
        ),
        (
            "/admin/config/category-management/all".into(),
            "admin.config.category_management.types.all.title".into(),
            String::new(),
            active == "all",
        ),
    ];
    for ty in types.as_array().into_iter().flatten() {
        let id = s(&ty["id"]);
        items.push((
            format!("/admin/config/category-management/{id}"),
            String::new(),
            s(&ty["name"]).to_string(),
            active == id,
        ));
    }
    // DNavItem: the label translated, or `@i18nLabel` as given.
    let tabs: String = items
        .iter()
        .map(|(path, key, label, is_active)| {
            let label = if key.is_empty() {
                label.clone()
            } else {
                t(cx.i18n, key)
            };
            format!(
                "<li{}><a{} href=\"{}{}\">{}</a></li>",
                if *is_active { " class=\"active\"" } else { "" },
                if *is_active { " class=\"active\"" } else { "" },
                cx.base_path,
                escape(path),
                escape(&label)
            )
        })
        .collect();
    let header = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path: "/admin/config/category-management",
            },
        ],
        title,
        actions: admin_ui::actions(
            cx,
            "d-page-header",
            &[Action::Primary(Button {
                label: Some(k(cx, "create_category")),
                href: Some("/new-category/setup"),
                ..Default::default()
            })],
        ),
        description: Some(escape(&k(cx, "header_description"))),
        learn_more_url: None,
        tabs: Some(tabs),
    }
    .render(cx);
    format!("{header}<div class=\"admin-config-page__main-area\">{outlet}</div>")
}

/// `dCategoryBadge(badge, link=false, hideParent=true)`
fn badge(cx: &AdminCx, b: &Value) -> String {
    let mut inner = String::new();
    match (s(&b["style_type"]), b["icon"].as_str(), b["emoji"].as_str()) {
        ("icon", Some(name), _) => inner.push_str(&icon(name, None)),
        ("emoji", _, Some(emoji)) => inner.push_str(&emoji_unescape(
            &format!(":{emoji}:"),
            cx.settings.emojis(),
            cx.base_path,
        )),
        _ => {}
    }
    let restricted = b["read_restricted"] == true;
    if restricted {
        inner.push_str(&icon("lock", None));
    }
    format!(
        "<span class=\"badge-category__wrapper\" style=\"--category-badge-color: #{};\"><span data-category-id=\"{}\" data-drop-close=\"true\" class=\"badge-category{} --style-{}\">{inner}<span class=\"badge-category__name\">{}</span></span></span>",
        escape(s(&b["color"])),
        b["id"],
        if restricted { " restricted" } else { "" },
        escape(s(&b["style_type"])),
        escape(s(&b["name"]))
    )
}

/// `truncatedDescription`: the first 497 characters and an ellipsis past
/// 500.
fn truncated(description: &str) -> String {
    if description.chars().count() <= 500 {
        return description.to_string();
    }
    let cut: String = description.chars().take(497).collect();
    format!("{}...", cut.trim_end())
}

/// A category's row.
pub fn row(cx: &AdminCx, c: &Value, type_badges: bool) -> String {
    let href = format!("{}/c/{}/{}", cx.base_path, s(&c["slug"]), c["id"]);
    let badges: String = c["badge_chain"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| format!("<a href=\"{}\">{}</a>", escape(&href), badge(cx, b)))
        .collect();
    let types = if type_badges {
        let items: String = c["category_types"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|ty| {
                format!(
                    "<span class=\"d-table-badge admin-category-management-list__type-badge\" data-category-type=\"{}\"><span class=\"d-table-badge__content\">{}</span></span>",
                    escape(s(&ty["id"])),
                    escape(s(&ty["name"]))
                )
            })
            .collect();
        format!(
            "<div class=\"d-table__badges admin-category-management-list__type-badges\">{items}</div>"
        )
    } else {
        String::new()
    };
    let label = |key: &str| {
        format!(
            "<div class=\"d-table__mobile-label\">{}</div>",
            escape(&k(cx, key))
        )
    };
    let visibility = if c["read_restricted"] == true {
        format!(
            "{}{}",
            icon("lock", None),
            escape(&k(cx, "visibility.restricted"))
        )
    } else {
        escape(&k(cx, "visibility.public"))
    };
    // DButton with @href: the label at md widths, the gear below.
    let open = k(cx, "open_settings");
    let button = if crate::mobile::mobile_view() {
        format!(
            "<a class=\"btn no-text btn-icon btn-default btn-small admin-category-management-list__open-settings\" href=\"{}{}\" title=\"{o}\">{}<span aria-hidden=\"true\">\u{200b}</span></a>",
            cx.base_path,
            escape(s(&c["edit_url"])),
            icon("gear", None),
            o = escape(&open)
        )
    } else {
        format!(
            "<a class=\"btn btn-default btn-small admin-category-management-list__open-settings\" href=\"{}{}\" title=\"{o}\"><span class=\"d-button-label\">{o}</span></a>",
            cx.base_path,
            escape(s(&c["edit_url"])),
            o = escape(&open)
        )
    };
    format!(
        "<tr class=\"d-table__row\" data-category-id=\"{}\"><td class=\"d-table__cell --overview admin-category-management-list__category\"><div class=\"admin-category-management-list__category-badges\">{badges}</div><div class=\"d-table__overview-about admin-category-management-list__description\">{}</div>{types}</td><td class=\"d-table__cell --detail admin-category-management-list__visibility-cell\">{}<span class=\"admin-category-management-list__visibility\">{visibility}</span></td><td class=\"d-table__cell --detail admin-category-management-list__topic-count\">{}{}</td><td class=\"d-table__cell --controls admin-category-management-list__controls-cell\"><div class=\"d-table__cell-actions\">{button}</div></td></tr>",
        c["id"],
        // The serializer's escaped text, shown as text.
        escape(&truncated(s(&c["description_text"]))),
        label("table.visibility"),
        label("table.topics"),
        c["topic_count"],
    )
}

/// The table, or with no categories nothing (the empty message is above).
pub fn table(cx: &AdminCx, categories: &[Value], type_badges: bool) -> String {
    if categories.is_empty() {
        return String::new();
    }
    let rows: String = categories.iter().map(|c| row(cx, c, type_badges)).collect();
    let th = |class: &str, key: Option<&str>| {
        format!(
            "<th class=\"d-table__header-cell admin-category-management-list__{class}\">{}</th>",
            key.map(|k2| escape(&k(cx, k2))).unwrap_or_default()
        )
    };
    format!(
        "<div><table class=\"d-table admin-category-management-list__table\"><colgroup><col class=\"admin-category-management-list__overview-column\"><col class=\"admin-category-management-list__visibility-column\"><col class=\"admin-category-management-list__topics-column\"><col class=\"admin-category-management-list__controls-column\"></colgroup><thead class=\"d-table__header\"><tr class=\"d-table__row\">{}{}{}{}</tr></thead><tbody class=\"d-table__body\">{rows}</tbody></table><div aria-hidden=\"true\" class=\"load-more-sentinel\"></div></div>",
        th("category-header", Some("table.category")),
        th("visibility-header", Some("table.visibility")),
        th("topics-header", Some("table.topics")),
        th("controls-header", None),
    )
}

/// AdminCategoryManagementList for the type with its first page.
pub fn list(cx: &AdminCx, model: &Value, first: &Value) -> String {
    let name = s(&model["name"]);
    let subheader = admin_ui::subheader(name, "", Some(&escape(s(&model["description"]))));
    if model["available"] == false {
        return format!(
            "<div class=\"admin-category-management-list admin-detail\">{subheader}<div class=\"admin-category-management-list__unavailable\">{}</div></div>",
            escape(&t_with(
                cx.i18n,
                "admin.config.category_management.types.unavailable",
                &[("categoryType", name)]
            ))
        );
    }
    let categories = first["categories"].as_array().cloned().unwrap_or_default();
    let no_results = t_with(
        cx.i18n,
        "admin.config.category_management.no_results",
        &[("type", name)],
    );
    let options: String = ["all", "public", "restricted"]
        .iter()
        .map(|v| {
            format!(
                "<option class=\"d-native-select__option{}\" value=\"{v}\"{}>{}</option>",
                if *v == "all" { " --selected" } else { "" },
                if *v == "all" { " selected" } else { "" },
                escape(&k(cx, &format!("visibility_filter.{v}")))
            )
        })
        .collect();
    let reset = escape(&t(cx.i18n, "filter_controls.reset"));
    let empty = if categories.is_empty() {
        format!(
            "<p class=\"admin-category-management-list__empty\">{}</p>",
            escape(&no_results)
        )
    } else {
        String::new()
    };
    format!(
        "<div class=\"admin-category-management-list admin-detail\" data-type=\"{}\" data-has-more=\"{}\" data-no-results=\"{}\">{subheader}<div class=\"d-filter-controls\"><div class=\"d-filter-controls__inputs\"><div class=\"filter-input-container\">{}<input autocapitalize=\"none\" autocomplete=\"off\" autocorrect=\"off\" class=\"filter-input d-filter-controls__input\" placeholder=\"{}\" type=\"text\" value=\"\"></div></div><div class=\"d-filter-controls__dropdowns\"><select aria-label=\"{}\" class=\"d-filter-controls__dropdown d-native-select\">{options}</select></div><button aria-label=\"{reset}\" class=\"btn no-text btn-icon btn-default d-filter-controls__reset\" disabled=\"\" title=\"{reset}\" type=\"button\">{}<span aria-hidden=\"true\">\u{200b}</span></button></div><div class=\"loading-container\"></div>{empty}{}</div>",
        escape(s(&model["id"])),
        first["has_more"] == true,
        escape(&no_results),
        icon("magnifying-glass", Some("-left")),
        escape(&k(cx, "filter_placeholder")),
        escape(&k(cx, "visibility_filter.all")),
        icon("arrow-rotate-left", None),
        table(cx, &categories, s(&model["id"]) == "all"),
    )
}

/// The strings admin-category-management.js shows.
pub const STRINGS: &[&str] = &[
    "admin.config.category_management.table.category",
    "admin.config.category_management.table.visibility",
    "admin.config.category_management.table.topics",
    "admin.config.category_management.visibility.public",
    "admin.config.category_management.visibility.restricted",
    "admin.config.category_management.open_settings",
    "filter_controls.reset",
    "search.no_results",
];
