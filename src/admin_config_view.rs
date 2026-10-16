//! The /admin/config areas with pages of their own (90g6), as Ember draws
//! them: the flags (templates/admin-config/flags.gjs over
//! AdminConfigAreasFlags and AdminFlagItem, and AdminFlagsForm). Their
//! script is static/js/admin-config.js.

use serde_json::Value;

use crate::admin_ui::{self, Action, Button};
use crate::admin_view::{AdminCx, Crumb, PageHeader, t};
use crate::topic_list_view::{escape, icon};

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// The flags page's tab.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FlagsTab {
    Flags,
    Settings,
}

/// flags.gjs's DPageHeader: Add flag (disabled at custom_flags_limit)
/// and the Settings and Flags tabs; the settings tab's AdminAreaSettings
/// adds its breadcrumb.
pub fn flags_header(cx: &AdminCx, tab: FlagsTab, add_disabled: bool) -> String {
    let title = t(cx.i18n, "admin.config.flags.title");
    let mut crumbs = vec![
        Crumb {
            label: t(cx.i18n, "admin_title"),
            path: "/admin",
        },
        Crumb {
            label: title.clone(),
            path: "/admin/config/flags",
        },
    ];
    if tab == FlagsTab::Settings {
        crumbs.push(Crumb {
            label: t(cx.i18n, "settings"),
            path: "/admin/config/flags/settings",
        });
    }
    let add = t(cx.i18n, "admin.config_areas.flags.add");
    let actions = admin_ui::actions(
        cx,
        "d-page-header",
        &[Action::Primary(Button {
            class: "admin-flags__header-add-flag",
            label: Some(add.clone()),
            title: Some(add),
            disabled: add_disabled,
            href: Some("/admin/config/flags/new"),
            ..Default::default()
        })],
    );
    PageHeader {
        crumbs,
        title,
        actions,
        description: Some(t(cx.i18n, "admin.config.flags.header_description")),
        learn_more_url: Some("https://meta.discourse.org/t/moderation-flags/325589".into()),
        tabs: Some(admin_ui::tabs(
            cx,
            &[
                (
                    "admin-flags-tabs__settings",
                    "/admin/config/flags/settings",
                    "settings",
                    tab == FlagsTab::Settings,
                ),
                (
                    "admin-flags-tabs__flags",
                    "/admin/config/flags",
                    "admin.config_areas.flags.flags_tab",
                    tab == FlagsTab::Flags,
                ),
            ],
        )),
    }
    .render(cx)
}

/// DToggleSwitch with the classes its invocation gives the button.
fn toggle_switch(class: &str, on: bool, attrs: &str) -> String {
    format!(
        "<div class=\"d-toggle-switch\"><label class=\"d-toggle-switch__label\"><button aria-checked=\"{on}\" class=\"d-toggle-switch__checkbox {}\" role=\"switch\" type=\"button\"{attrs}></button><span class=\"d-toggle-switch__checkbox-slider\">{}</span></label></div>",
        escape(class),
        if on {
            icon("check", None)
        } else {
            String::new()
        }
    )
}

/// AdminConfigAreasFlags: the site's flag types, each an AdminFlagItem.
pub fn flags_list(cx: &AdminCx, flags: &[Value]) -> String {
    let i18n = cx.i18n;
    let enabled_label = escape(&t(i18n, "admin.config_areas.flags.enabled"));
    let mut out = format!(
        "<div class=\"container admin-flags\"><table class=\"d-table admin-flags__items\"><thead class=\"d-table__header\"><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\">{enabled_label}</th></thead><tbody class=\"d-table__body\">",
        escape(&t(i18n, "admin.config_areas.flags.description"))
    );
    let last = flags.len().saturating_sub(1);
    for (index, flag) in flags.iter().enumerate() {
        let id = flag["id"].as_i64().unwrap_or_default();
        let name_key = s(&flag["name_key"]);
        let system = flag["system"] == Value::Bool(true);
        let can_edit = !system;
        let can_delete = can_edit && flag["is_used"] != Value::Bool(true);
        // SYSTEM_FLAG_IDS.notify_user
        let can_move = id != 6;
        let overview = format!(
            "<div class=\"d-table__overview-name admin-flag-item__name\">{}</div><div class=\"d-table__overview-about\">{}</div>",
            escape(s(&flag["name"])),
            s(&flag["description"])
        );
        let overview = if can_edit {
            format!(
                "<a class=\"d-table__overview-link\" href=\"{}/admin/config/flags/{id}\">{overview}</a>",
                cx.base_path
            )
        } else {
            overview
        };
        out.push_str(&format!(
            "<tr class=\"d-table__row admin-flag-item {} saved\" data-flag-id=\"{id}\" data-flag-name=\"{}\"><td class=\"d-table__cell --overview\">{overview}</td><td class=\"d-table__cell --detail\"><div class=\"d-table__mobile-label\">{enabled_label}</div>{}</td><td class=\"d-table__cell --controls\"><div class=\"d-table__cell-actions\">",
            escape(name_key),
            escape(s(&flag["name"])),
            toggle_switch(
                &format!("admin-flag-item__toggle {name_key}"),
                flag["enabled"] == Value::Bool(true),
                ""
            )
        ));
        out.push_str(
            &Button {
                class: "btn-default btn-small admin-flag-item__edit",
                label: Some(t(i18n, "admin.config_areas.flags.edit")),
                title: Some(t(
                    i18n,
                    if can_edit {
                        "admin.config_areas.flags.form.edit_flag"
                    } else {
                        "admin.config_areas.flags.form.non_editable"
                    },
                )),
                disabled: !can_edit,
                ..Default::default()
            }
            .render(),
        );
        if can_move {
            let item = |b: Button| format!("<li class=\"dropdown-menu__item\">{}</li>", b.render());
            let mut items = String::new();
            // isFirstFlag: the second flag, the first movable one.
            if index != 1 {
                items.push_str(&item(Button {
                    class: "btn-transparent admin-flag-item__move-up",
                    icon: Some("arrow-up"),
                    label: Some(t(i18n, "admin.config_areas.flags.more_options.move_up")),
                    ..Default::default()
                }));
            }
            if index != last {
                items.push_str(&item(Button {
                    class: "btn-transparent admin-flag-item__move-down",
                    icon: Some("arrow-down"),
                    label: Some(t(i18n, "admin.config_areas.flags.more_options.move_down")),
                    ..Default::default()
                }));
            }
            items.push_str(&item(Button {
                class: "btn-transparent --danger admin-flag-item__delete",
                icon: Some("trash-can"),
                label: Some(t(i18n, "admin.config_areas.flags.delete")),
                title: Some(t(
                    i18n,
                    if can_delete {
                        "admin.config_areas.flags.form.delete_flag"
                    } else {
                        "admin.config_areas.flags.form.non_deletable"
                    },
                )),
                disabled: !can_delete,
                ..Default::default()
            }));
            out.push_str(&admin_ui::menu(
                "btn-default",
                "flag-menu",
                "ellipsis-vertical",
                &t(i18n, "admin.config_areas.flags.more_options.title"),
                &format!("<ul class=\"dropdown-menu\">{items}</ul>"),
            ));
        }
        out.push_str("</div></td></tr>");
    }
    out.push_str("</tbody></table></div>");
    out
}

/// FormKit's text field with its character counter (`length` validation).
fn counted_field(
    name: &str,
    control: &str,
    title: &str,
    value: &str,
    min: usize,
    max: usize,
) -> String {
    let count = value.chars().count();
    let control_html = match control {
        "textarea" => format!(
            "<textarea class=\"form-kit__control-textarea\" id=\"control-{name}-input\" name=\"{name}\" style=\"height: 60px\">{}</textarea>",
            escape(value)
        ),
        _ => format!(
            "<div class=\"form-kit__control-input-wrapper\"><input class=\"form-kit__control-input\" id=\"control-{name}-input\" name=\"{name}\" value=\"{}\" type=\"text\"></div>",
            escape(value)
        ),
    };
    format!(
        "<div class=\"form-kit__container form-kit__field form-kit__field-{control} --large\" data-name=\"{name}\" id=\"control-{name}\" data-control-type=\"{control}\" data-required=\"\" data-min=\"{min}\" data-max=\"{max}\" data-title=\"{t}\"><label for=\"control-{name}-input\" class=\"form-kit__container-title\"><span>{t}</span></label><div class=\"form-kit__container-content\">{control_html}<div class=\"form-kit__meta\"><span class=\"form-kit__char-counter{}\">{count}/{max}</span></div></div></div>",
        if count < min || count > max {
            " --insufficient"
        } else {
            ""
        },
        t = escape(title)
    )
}

/// A checkbox group's field: the box, its title, its description.
fn checkbox_field(name: &str, title: &str, description: &str, checked: bool) -> String {
    format!(
        "<div class=\"form-kit__container form-kit__field form-kit__field-checkbox --full\" data-name=\"{name}\" id=\"control-{name}\" data-control-type=\"checkbox\"><div class=\"form-kit__container-content\"><label class=\"form-kit__control-checkbox-label\"><input class=\"form-kit__control-checkbox\" id=\"control-{name}-input\" name=\"{name}\" type=\"checkbox\"{}><span class=\"form-kit__control-checkbox-checkmark\">{}</span><span class=\"form-kit__control-checkbox-content\"><span class=\"form-kit__control-checkbox-title\"><span>{}</span></span><span class=\"form-kit__control-checkbox-description\">{}</span></span></label></div></div>",
        if checked { " checked=\"\"" } else { "" },
        icon("check", None),
        escape(title),
        escape(description)
    )
}

/// select-kit's MultiSelect, closed: the names chosen (or Select…), the
/// choices for admin-config.js.
fn multi_select(
    cx: &AdminCx,
    class: &str,
    choices: &[(String, String)],
    chosen: &[String],
) -> String {
    let names: Vec<&str> = chosen
        .iter()
        .filter_map(|v| {
            choices
                .iter()
                .find(|(id, _)| id == v)
                .map(|(_, n)| n.as_str())
        })
        .collect();
    let options: Vec<Value> = choices
        .iter()
        .map(|(id, name)| serde_json::json!({"id": id, "name": name}))
        .collect();
    format!(
        "<details class=\"select-kit multi-select{} {}\" data-choices=\"{}\" data-none=\"{}\"><summary data-name=\"{}\" data-value=\"{}\" tabindex=\"0\" class=\"select-kit-header multi-select-header\"><div class=\"select-kit-header-wrapper\"><span class=\"formatted-selection\">{}</span>{}</div></summary><div class=\"select-kit-body\"></div></details>",
        if chosen.is_empty() {
            ""
        } else {
            " has-selection"
        },
        escape(class),
        escape(&Value::Array(options).to_string()),
        escape(&t(cx.i18n, "select_kit.default_header_text")),
        escape(&names.join(",")),
        escape(&chosen.join(",")),
        escape(&if names.is_empty() {
            t(cx.i18n, "select_kit.default_header_text")
        } else {
            names.join(", ")
        }),
        icon("plus", Some("angle-icon"))
    )
}

/// AdminFlagsForm under its BackButton: the flag's (or a new one's)
/// fields; admin-config.js validates and saves them.
pub fn flag_form(cx: &AdminCx, flag: Option<&Value>, applies_to_types: &[&str]) -> String {
    let i18n = cx.i18n;
    let k = |key: &str| t(i18n, &format!("admin.config_areas.flags.form.{key}"));
    let name = flag.map(|f| s(&f["name"])).unwrap_or_default();
    let description = flag.map(|f| s(&f["description"])).unwrap_or_default();
    let applies_to: Vec<String> = flag
        .and_then(|f| f["applies_to"].as_array())
        .map(|a| a.iter().map(|v| s(v).to_string()).collect())
        .unwrap_or_default();
    let flag_bool =
        |key: &str, default: bool| flag.map(|f| f[key] == Value::Bool(true)).unwrap_or(default);
    let choices: Vec<(String, String)> = applies_to_types
        .iter()
        .map(|ty| (ty.to_string(), k(&ty.to_lowercase().replace("::", "_"))))
        .collect();
    let mut form = format!(
        "<div class=\"form-kit__alert alert alert-warning\">{} <span class=\"form-kit__alert-message\">{}</span></div>",
        icon("circle-info", None),
        escape(&k(if flag.is_some() {
            "edit_warning"
        } else {
            "create_warning"
        }))
    );
    form.push_str(&counted_field("name", "input", &k("name"), name, 3, 200));
    form.push_str(&counted_field(
        "description",
        "textarea",
        &k("description"),
        description,
        3,
        1000,
    ));
    let applies_title = k("applies_to");
    form.push_str(&format!(
        "<div class=\"form-kit__container form-kit__field form-kit__field-custom --large\" data-name=\"appliesTo\" id=\"control-appliesTo\" data-control-type=\"custom\" data-required=\"\" data-title=\"{t}\"><label for=\"control-appliesTo-input\" class=\"form-kit__container-title\"><span>{t}</span></label><div class=\"form-kit__container-content\"><div class=\"form-kit__control-custom\">{}</div></div></div>",
        multi_select(cx, "admin-flag-form__applies-to", &choices, &applies_to),
        t = escape(&applies_title)
    ));
    form.push_str("<fieldset class=\"form-kit__fieldset form-kit__checkbox-group\">");
    form.push_str(&checkbox_field(
        "requireMessage",
        &k("require_message"),
        &k("require_message_description"),
        flag_bool("require_message", false),
    ));
    form.push_str(&checkbox_field(
        "enabled",
        &k("enabled"),
        "",
        flag_bool("enabled", true),
    ));
    form.push_str(&checkbox_field(
        "autoActionType",
        &k("auto_action_type"),
        "",
        flag_bool("auto_action_type", false),
    ));
    form.push_str("</fieldset>");
    form.push_str(&format!(
        "<button class=\"btn btn-primary form-kit__button\" type=\"submit\"><span class=\"d-button-label\">{}</span></button>",
        escape(&k("save"))
    ));
    format!(
        "<div class=\"admin-container admin-config-page__main-area\">{}<div class=\"admin-config-area\"><div class=\"admin-config-area__primary-content admin-flag-form\"><section class=\"admin-config-area-card\"><div class=\"admin-config-area-card__header-wrapper\"><h3 class=\"admin-config-area-card__title\">{}</h3></div><div class=\"admin-config-area-card__content\"><form class=\"form-kit\" novalidate=\"\" data-flag-form=\"\" data-flag-id=\"{}\">{form}</form></div></section></div></div></div>",
        back_button(cx, "/admin/config/flags", "admin.config_areas.flags.back"),
        escape(&k(if flag.is_some() {
            "edit_header"
        } else {
            "add_header"
        })),
        flag.and_then(|f| f["id"].as_i64())
            .map(|id| id.to_string())
            .unwrap_or_default()
    )
}

/// BackButton for a route under the current one (LinkTo's active class).
fn back_button(cx: &AdminCx, path: &str, label_key: &str) -> String {
    format!(
        "<a class=\"active btn btn-transparent back-button\" href=\"{}{}\">{}{}</a>",
        cx.base_path,
        escape(path),
        icon("chevron-left", None),
        escape(&t(cx.i18n, label_key))
    )
}

/// The permalinks page's tab.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PermalinksTab {
    Permalinks,
    Settings,
}

/// admin-permalinks.gjs's DPageHeader: Add permalink, the Settings and
/// Permalinks tabs; the settings tab's AdminAreaSettings adds its crumb.
pub fn permalinks_header(cx: &AdminCx, tab: PermalinksTab) -> String {
    let title = t(cx.i18n, "admin.config.permalinks.title");
    let mut crumbs = vec![
        Crumb {
            label: t(cx.i18n, "admin_title"),
            path: "/admin",
        },
        Crumb {
            label: title.clone(),
            path: "/admin/config/permalinks",
        },
    ];
    if tab == PermalinksTab::Settings {
        crumbs.push(Crumb {
            label: t(cx.i18n, "settings"),
            path: "/admin/config/permalinks/settings",
        });
    }
    let add = t(cx.i18n, "admin.permalink.add");
    PageHeader {
        crumbs,
        title,
        actions: admin_ui::actions(
            cx,
            "d-page-header",
            &[Action::Primary(Button {
                class: "admin-permalinks__header-add-permalink",
                label: Some(add.clone()),
                title: Some(add),
                href: Some("/admin/config/permalinks/new"),
                ..Default::default()
            })],
        ),
        description: Some(t(cx.i18n, "admin.config.permalinks.header_description")),
        learn_more_url: Some("https://meta.discourse.org/t/20930".into()),
        tabs: Some(admin_ui::tabs(
            cx,
            &[
                (
                    "admin-permalinks-tabs__settings",
                    "/admin/config/permalinks/settings",
                    "admin.permalink.nav.settings",
                    tab == PermalinksTab::Settings,
                ),
                (
                    "admin-permalins-permalinks",
                    "/admin/config/permalinks",
                    "admin.permalink.nav.permalinks",
                    tab == PermalinksTab::Permalinks,
                ),
            ],
        )),
    }
    .render(cx)
}

/// The page's wrapper around the header (if any) and the outlet.
pub fn permalinks_page(header: &str, outlet: &str) -> String {
    format!(
        "<div class=\"admin-permalinks admin-config-page\">{header}<div class=\"admin-container admin-config-page__main-area\">{outlet}</div></div>"
    )
}

/// `DiscourseURL.isInternal`: a path, or an absolute URL on this site.
fn internal(url: &str, site_origin: &str) -> bool {
    (url.starts_with('/') && !url.starts_with("//"))
        || url
            .strip_prefix(site_origin)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// A permalink's row (admin-permalinks/index.gjs).
fn permalink_row(
    cx: &AdminCx,
    list: &crate::topic_list_view::ListContext,
    site_origin: &str,
    pl: &Value,
) -> String {
    let i18n = cx.i18n;
    let id = pl["id"].as_i64().unwrap_or_default();
    let url = s(&pl["url"]);
    let mut destination = String::new();
    if !pl["topic_id"].is_null() {
        destination.push_str(&format!(
            "<a href=\"{}\">{}</a>",
            escape(s(&pl["topic_url"])),
            escape(s(&pl["topic_title"]))
        ));
    }
    if !pl["post_id"].is_null() {
        destination.push_str(&format!(
            "<a href=\"{}\">{} #{}</a>",
            escape(s(&pl["post_url"])),
            escape(s(&pl["post_topic_title"])),
            pl["post_number"]
        ));
    }
    if let Some(category) = pl["category_id"]
        .as_i64()
        .and_then(|cid| list.categories.get(&cid))
    {
        destination.push_str(&crate::topic_list_view::category_badge(list, category));
    }
    if !pl["tag_id"].is_null() {
        destination.push_str(&format!(
            "<a href=\"{}\">{}</a>",
            escape(s(&pl["tag_url"])),
            escape(s(&pl["tag_name"]))
        ));
    }
    if let Some(external) = pl["external_url"].as_str().filter(|u| !u.is_empty()) {
        if !internal(external, site_origin) {
            destination.push_str(&icon("up-right-from-square", None));
        }
        destination.push_str(&format!("<a href=\"{e}\">{e}</a>", e = escape(external)));
    }
    if !pl["user_id"].is_null() {
        destination.push_str(&format!(
            "<a href=\"{}\">{}</a>",
            escape(s(&pl["user_url"])),
            escape(s(&pl["username"]))
        ));
    }
    let copy = Button {
        class: "btn-flat",
        icon: Some("far-clipboard"),
        title: Some(t(i18n, "admin.permalink.copy_to_clipboard")),
        ..Default::default()
    }
    .render();
    let edit = Button {
        class: "btn-default btn-small admin-permalink-item__edit",
        label: Some(t(i18n, "admin.config_areas.permalinks.edit")),
        ..Default::default()
    }
    .render();
    let menu = admin_ui::menu(
        "btn-default",
        "permalink-menu",
        "ellipsis-vertical",
        &t(i18n, "admin.permalink.more_options"),
        &format!(
            "<ul class=\"dropdown-menu\"><li class=\"dropdown-menu__item\">{}</li></ul>",
            Button {
                class: "btn-transparent --danger admin-permalink-item__delete",
                icon: Some("trash-can"),
                label: Some(t(i18n, "admin.config_areas.permalinks.delete")),
                ..Default::default()
            }
            .render()
        ),
    );
    format!(
        "<tr class=\"d-table__row admin-permalink-item {}\" data-permalink-id=\"{id}\"><td class=\"d-table__cell --overview\">{copy}<span class=\"admin-permalink-item__url\" id=\"admin-permalink-{id}\" title=\"{u}\">{u}</span></td><td class=\"d-table__cell --detail destination\">{destination}</td><td class=\"d-table__cell --controls\"><div class=\"d-table__cell-actions\">{edit}{menu}</div></td></tr>",
        escape(&url.replacen('/', "_", 1)),
        u = escape(url)
    )
}

/// admin-permalinks/index.gjs: DFilterControls over the table, or the
/// empty list when the site has none.
pub fn permalinks_list(
    cx: &AdminCx,
    list: &crate::topic_list_view::ListContext,
    site_origin: &str,
    permalinks: &[Value],
) -> String {
    let i18n = cx.i18n;
    if permalinks.is_empty() {
        return admin_ui::empty_list(
            cx,
            "admin-permalinks__add-permalink",
            "admin.permalink.add",
            "/admin/config/permalinks/new",
            "admin.permalink.no_permalinks",
        );
    }
    let reset = escape(&t(i18n, "filter_controls.reset"));
    format!(
        "<div class=\"d-filter-controls\"><div class=\"d-filter-controls__inputs\"><div class=\"filter-input-container\">{}<input autocapitalize=\"none\" autocomplete=\"off\" autocorrect=\"off\" class=\"filter-input d-filter-controls__input\" value=\"\" placeholder=\"{}\" type=\"text\"></div></div><button aria-label=\"{reset}\" class=\"btn no-text btn-icon btn-default d-filter-controls__reset\" disabled=\"\" title=\"{reset}\" type=\"button\">{}<span aria-hidden=\"true\">\u{200b}</span></button></div><div class=\"loading-container\"></div>{}",
        icon("magnifying-glass", Some("-left")),
        escape(&t(i18n, "admin.permalink.form.filter")),
        icon("arrow-rotate-left", None),
        permalinks_results(cx, list, site_origin, permalinks)
    )
}

/// The table of permalinks (DFilterControls' content), or its no results
/// message.
pub fn permalinks_results(
    cx: &AdminCx,
    list: &crate::topic_list_view::ListContext,
    site_origin: &str,
    permalinks: &[Value],
) -> String {
    let i18n = cx.i18n;
    if permalinks.is_empty() {
        let reset = escape(&t(i18n, "filter_controls.reset"));
        return format!(
            "<div class=\"d-filter-controls__no-results\"><p>{}</p><button aria-label=\"{reset}\" class=\"btn btn-icon-text btn-default d-filter-controls__reset\" title=\"{reset}\" type=\"button\">{}<span class=\"d-button-label\">{reset}</span></button></div>",
            escape(&t(i18n, "search.no_results")),
            icon("arrow-rotate-left", None)
        );
    }
    let rows: String = permalinks
        .iter()
        .map(|pl| permalink_row(cx, list, site_origin, pl))
        .collect();
    format!(
        "<div class=\"permalink-results\"><table class=\"d-table permalinks\"><thead class=\"d-table__header\"><tr class=\"d-table__row\"><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\">{}</th></tr></thead><tbody class=\"d-table__body\">{rows}</tbody></table></div>",
        escape(&t(i18n, "admin.permalink.url")),
        escape(&t(i18n, "admin.permalink.destination"))
    )
}

/// The permalink types the form offers: the type, its option's key, the
/// field it fills, the column it reads, the field's format.
const PERMALINK_TYPES: [(&str, &str, &str, &str, &str); 6] = [
    ("topic", "topic_title", "topicId", "topic_id", "small"),
    ("post", "post_title", "postId", "post_id", "small"),
    (
        "category",
        "category_title",
        "categoryId",
        "category_id",
        "small",
    ),
    ("tag", "tag_title", "tagName", "tag_name", "small"),
    (
        "external_url",
        "external_url",
        "externalUrl",
        "external_url",
        "large",
    ),
    ("user", "user_title", "userId", "user_id", "small"),
];

/// FormKit's input field, required.
fn required_input(name: &str, title: &str, format: &str, value: Option<&str>) -> String {
    format!(
        "<div class=\"form-kit__container form-kit__field form-kit__field-input --{format}\" data-name=\"{name}\" id=\"control-{name}\" data-control-type=\"input\" data-required=\"\" data-title=\"{t}\"><label for=\"control-{name}-input\" class=\"form-kit__container-title\"><span>{t}</span></label><div class=\"form-kit__container-content\"><div class=\"form-kit__control-input-wrapper\"><input class=\"form-kit__control-input\" id=\"control-{name}-input\" name=\"{name}\"{} type=\"text\"></div></div></div>",
        value
            .map(|v| format!(" value=\"{}\"", escape(v)))
            .unwrap_or_default(),
        t = escape(title)
    )
}

/// AdminPermalinkForm under its BackButton: the URL, the type and the
/// type's own field, which admin-config.js swaps when the type changes.
pub fn permalink_form(cx: &AdminCx, permalink: Option<&Value>) -> String {
    let i18n = cx.i18n;
    let k = |key: &str| t(i18n, &format!("admin.permalink.{key}"));
    // formData's permalinkType: the first column the permalink sets.
    let (kind, value) = match permalink {
        Some(pl) => PERMALINK_TYPES
            .iter()
            .find_map(|(kind, _, _, column, _)| {
                let column = if *kind == "tag" { "tag_id" } else { column };
                let present = match &pl[column] {
                    Value::Null => false,
                    Value::String(x) => !x.trim().is_empty(),
                    _ => true,
                };
                if !present {
                    return None;
                }
                let v = match (*kind, &pl[column]) {
                    ("tag", _) => s(&pl["tag_name"]).to_string(),
                    (_, Value::String(x)) => x.clone(),
                    (_, other) => other.to_string(),
                };
                Some((*kind, Some(v)))
            })
            .unwrap_or(("", None)),
        None => ("topic", None),
    };
    let new = permalink.is_none();
    let empty = new.then_some("");
    let mut form = required_input(
        "url",
        &k("form.url"),
        "large",
        permalink.map(|p| s(&p["url"])).or(empty),
    );
    let options: String = PERMALINK_TYPES
        .iter()
        .map(|(v, label, ..)| {
            let sel = *v == kind;
            format!(
                "<option class=\"d-native-select__option{} form-kit__control-option\"{} value=\"{v}\">{}</option>",
                if sel { " --selected" } else { "" },
                if sel { " selected=\"\"" } else { "" },
                escape(&k(label))
            )
        })
        .collect();
    form.push_str(&format!(
        "<div class=\"form-kit__container form-kit__field form-kit__field-select\" data-name=\"permalinkType\" id=\"control-permalinkType\" data-control-type=\"select\" data-required=\"\" data-title=\"{t}\"><label for=\"control-permalinkType-input\" class=\"form-kit__container-title\"><span>{t}</span></label><div class=\"form-kit__container-content\"><select class=\"form-kit__control-select d-native-select\" id=\"control-permalinkType-input\" name=\"permalinkType\">{options}</select></div></div>",
        t = escape(&k("form.permalink_type"))
    ));
    // Every type's field (empty), for the script; the chosen one drawn.
    let fields: serde_json::Map<String, Value> = PERMALINK_TYPES
        .iter()
        .map(|(v, _, name, column, format)| {
            let title = if *v == "external_url" {
                k("external_url")
            } else {
                k(column)
            };
            (
                v.to_string(),
                Value::String(required_input(name, &title, format, Some(""))),
            )
        })
        .collect();
    if let Some((_, _, name, column, format)) = PERMALINK_TYPES.iter().find(|(v, ..)| *v == kind) {
        let title = if kind == "external_url" {
            k("external_url")
        } else {
            k(column)
        };
        form.push_str(&required_input(
            name,
            &title,
            format,
            value.as_deref().or(empty),
        ));
    }
    form.push_str(&format!(
        "<button class=\"btn btn-primary form-kit__button\" type=\"submit\"><span class=\"d-button-label\">{}</span></button>",
        escape(&k("form.save"))
    ));
    format!(
        "{}<div class=\"admin-config-area\"><div class=\"admin-config-area__primary-content admin-permalink-form\"><section class=\"admin-config-area-card\"><div class=\"admin-config-area-card__header-wrapper\"><h3 class=\"admin-config-area-card__title\">{}</h3></div><div class=\"admin-config-area-card__content\"><form class=\"form-kit\" novalidate=\"\" data-permalink-form=\"\" data-permalink-id=\"{}\" data-type-fields=\"{}\">{form}</form></div></section></div></div>",
        back_button(cx, "/admin/config/permalinks", "admin.permalink.back"),
        escape(&k(if new {
            "form.add_header"
        } else {
            "form.edit_header"
        })),
        permalink
            .and_then(|p| p["id"].as_i64())
            .map(|id| id.to_string())
            .unwrap_or_default(),
        escape(&Value::Object(fields).to_string())
    )
}

/// admin-user-fields.gjs: the header (Add user field) over the outlet.
/// DPageHeader goes headless on the new and edit routes.
pub fn user_fields_page(cx: &AdminCx, with_header: bool, outlet: &str) -> String {
    let title = t(cx.i18n, "admin.config.user_fields.title");
    let header = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path: "/admin/config/user-fields",
            },
        ],
        title,
        actions: admin_ui::actions(
            cx,
            "d-page-header",
            &[Action::Primary(Button {
                label: Some(t(cx.i18n, "admin.user_fields.add")),
                href: Some("/admin/config/user-fields/new"),
                ..Default::default()
            })],
        ),
        description: Some(t(cx.i18n, "admin.config.user_fields.header_description")),
        learn_more_url: Some(
            "https://meta.discourse.org/t/creating-and-configuring-custom-user-fields/113192"
                .into(),
        ),
        tabs: None,
    }
    .render(cx);
    let header = if with_header { header } else { String::new() };
    format!(
        "<div class=\"admin-user_fields admin-config-page\">{header}<div class=\"admin-config-page__main-area\"><div class=\"user-fields\">{outlet}</div></div></div>"
    )
}

/// USER_FIELD_FLAGS
const USER_FIELD_FLAGS: [&str; 5] = [
    "editable",
    "show_on_profile",
    "show_on_signup",
    "show_on_user_card",
    "searchable",
];

/// AdminConfigAreasUserFieldsList: the fields by position, each an
/// AdminUserFieldItem, or the empty list.
pub fn user_fields_list(cx: &AdminCx, fields: &[Value]) -> String {
    let i18n = cx.i18n;
    if fields.is_empty() {
        return format!(
            "<div class=\"container admin-user_fields\">{}</div>",
            admin_ui::empty_list(
                cx,
                "admin-user_fields__add-emoji",
                "admin.user_fields.add",
                "/admin/config/user-fields/new",
                "admin.user_fields.no_user_fields",
            )
        );
    }
    let last = fields.len() - 1;
    let rows: String = fields
        .iter()
        .enumerate()
        .map(|(index, f)| {
            let id = f["id"].as_i64().unwrap_or_default();
            let flags: Vec<String> = USER_FIELD_FLAGS
                .iter()
                .filter(|flag| f[**flag] == Value::Bool(true))
                .map(|flag| t(i18n, &format!("admin.user_fields.{flag}.enabled")))
                .collect();
            let item = |b: Button| format!("<li class=\"dropdown-menu__item\">{}</li>", b.render());
            let mut items = String::new();
            if index != 0 {
                items.push_str(&item(Button {
                    class: "btn-transparent admin-user_field-item__move-up",
                    icon: Some("arrow-up"),
                    label: Some(t(i18n, "admin.config_areas.user_fields.more_options.move_up")),
                    ..Default::default()
                }));
            }
            if index != last {
                items.push_str(&item(Button {
                    class: "btn-transparent admin-user_field-item__move-down",
                    icon: Some("arrow-down"),
                    label: Some(t(
                        i18n,
                        "admin.config_areas.user_fields.more_options.move_down",
                    )),
                    ..Default::default()
                }));
            }
            items.push_str(&item(Button {
                class: "btn-transparent --danger admin-user_field-item__delete",
                icon: Some("trash-can"),
                label: Some(t(i18n, "admin.config_areas.user_fields.delete")),
                ..Default::default()
            }));
            format!(
                "<tr class=\"d-table__row admin-user_field-item\" data-user-field-id=\"{id}\" data-position=\"{}\"><td class=\"d-table__cell --overview\"><a class=\"d-table__overview-link\" href=\"{}/admin/config/user-fields/{id}/edit\"><div class=\"d-table__overview-name admin-user_field-item__name\">{}</div><div class=\"d-table__overview-about\">{}</div></a><div class=\"d-table__overview-flags\">{}</div></td><td class=\"d-table__cell --detail\">{}</td><td class=\"d-table__cell --controls\"><div class=\"d-table__cell-actions\">{}{}</div></td></tr>",
                f["position"].as_i64().unwrap_or_default(),
                cx.base_path,
                escape(s(&f["name"])),
                s(&f["description"]),
                escape(&flags.join(", ")),
                escape(&t(
                    i18n,
                    &format!("admin.user_fields.field_types.{}", s(&f["field_type"]))
                )),
                Button {
                    class: "btn-default btn-small admin-user_field-item__edit",
                    label: Some(t(i18n, "admin.user_fields.edit")),
                    ..Default::default()
                }
                .render(),
                admin_ui::menu(
                    "btn-default",
                    "user_field-menu",
                    "ellipsis-vertical",
                    &t(i18n, "admin.config_areas.user_fields.more_options.title"),
                    &format!("<ul class=\"dropdown-menu\">{items}</ul>"),
                )
            )
        })
        .collect();
    format!(
        "<div class=\"container admin-user_fields\"><table class=\"d-table user-fields\"><thead class=\"d-table__header\"><tr class=\"d-table__row\"><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\">{}</th></tr></thead><tbody class=\"d-table__body\">{rows}</tbody></table></div>",
        escape(&t(i18n, "admin.config_areas.user_fields.field")),
        escape(&t(i18n, "admin.config_areas.user_fields.type"))
    )
}

/// UserField.fieldTypes()
const USER_FIELD_TYPES: [&str; 6] = [
    "text",
    "textarea",
    "date",
    "confirm",
    "dropdown",
    "multiselect",
];

/// ValueList (inputType array): the values, each removable and (with
/// more than one) shifted, over the ComboBox that adds one.
pub fn value_list(cx: &AdminCx, values: &[String]) -> String {
    let small = |class: &str, name: &str| {
        Button {
            class,
            icon: Some(name),
            ..Default::default()
        }
        .render()
    };
    let mut out = String::from("<div class=\"value-list\">");
    if !values.is_empty() {
        out.push_str("<div class=\"values\">");
        for (i, v) in values.iter().enumerate() {
            out.push_str(&format!(
                "<div class=\"value\" data-index=\"{i}\">{}<input class=\"value-input\" title=\"{v}\" value=\"{v}\" type=\"text\">",
                small("btn-default remove-value-btn btn-small", "xmark"),
                v = escape(v)
            ));
            if values.len() > 1 {
                out.push_str(&small(
                    "btn-default shift-up-value-btn btn-small",
                    "arrow-up",
                ));
                out.push_str(&small(
                    "btn-default shift-down-value-btn btn-small",
                    "arrow-down",
                ));
            }
            out.push_str("</div>");
        }
        out.push_str("</div>");
    }
    let aria = escape(&t(cx.i18n, "select_kit.select_to_filter"));
    out.push_str(&format!(
        "<details class=\"select-kit single-select combobox combo-box value-list-adder\"><summary aria-label=\"{aria}\" name=\"{aria}\" data-name=\"\" data-value=\"\" tabindex=\"0\" class=\"select-kit-header single-select-header combo-box-header\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" title=\"\"><span class=\"name\"></span></div>{}</div></summary><div class=\"select-kit-body\"></div></details></div>",
        icon("angle-down", Some("angle-icon"))
    ));
    out
}

/// AdminUserFieldsForm (in its card, under the BackButton): the type,
/// name and description, the options for a dropdown or multiselect, the
/// requirement, the preferences, Save and Cancel. A new field takes the
/// route's defaults.
pub fn user_field_form(cx: &AdminCx, field: Option<&Value>) -> String {
    let i18n = cx.i18n;
    let k = |key: &str| t(i18n, &format!("admin.user_fields.{key}"));
    let get = |key: &str| field.map(|f| &f[key]);
    let field_type = get("field_type").and_then(Value::as_str).unwrap_or("text");
    let requirement = get("requirement")
        .and_then(Value::as_str)
        .unwrap_or("optional");
    let flag = |key: &str| match field {
        Some(f) => f[key] == Value::Bool(true),
        None => key == "show_on_signup",
    };
    // `editable` false, not just unset, disables Show on signup.
    let editable_false = field.is_some_and(|f| f["editable"] == Value::Bool(false));
    let options: Vec<String> = get("options")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|v| s(v).to_string()).collect())
        .unwrap_or_default();
    let field_div = |name: &str, control: &str, format: Option<&str>, title: &str, inner: &str| {
        format!(
            "<div class=\"form-kit__container form-kit__field form-kit__field-{control}{}\" data-name=\"{name}\" id=\"control-{name}\" data-control-type=\"{control}\" data-required=\"\" data-title=\"{t}\"><label for=\"control-{name}-input\" class=\"form-kit__container-title\"><span>{t}</span></label><div class=\"form-kit__container-content\">{inner}</div></div>",
            format.map(|f| format!(" --{f}")).unwrap_or_default(),
            t = escape(title)
        )
    };
    let type_options: Vec<(String, String)> = USER_FIELD_TYPES
        .iter()
        .map(|ty| (ty.to_string(), k(&format!("field_types.{ty}"))))
        .collect();
    let mut form = field_div(
        "field_type",
        "select",
        Some("large"),
        &k("type"),
        &crate::admin_form_kit::select("field_type", &type_options, field_type, None, false),
    );
    let input = |name: &str, class: &str, max: u32, value: &str| {
        format!(
            "<div class=\"form-kit__control-input-wrapper\"><input class=\"form-kit__control-input {class}\" id=\"control-{name}-input\" name=\"{name}\" value=\"{}\" maxlength=\"{max}\" type=\"text\"></div>",
            escape(value)
        )
    };
    form.push_str(&field_div(
        "name",
        "input",
        Some("large"),
        &k("name"),
        &input(
            "name",
            "user-field-name",
            255,
            get("name").map(s).unwrap_or_default(),
        ),
    ));
    form.push_str(&field_div(
        "description",
        "input",
        Some("large"),
        &k("description"),
        &input(
            "description",
            "user-field-desc",
            1000,
            get("description").map(s).unwrap_or_default(),
        ),
    ));
    let options_field = field_div(
        "options",
        "custom",
        Some("large"),
        &k("options"),
        &format!(
            "<div class=\"form-kit__control-custom\">{}</div>",
            value_list(cx, &options)
        ),
    );
    if field_type == "dropdown" || field_type == "multiselect" {
        form.push_str(&options_field);
    }
    let radio = |value: &str, title: &str, description: Option<String>| {
        format!(
            "<div class=\"form-kit__field form-kit__field-radio\"><label for=\"control-requirement-{value}\" class=\"form-kit__control-radio-label\"><input class=\"form-kit__control-radio\" id=\"control-requirement-{value}\" name=\"requirement\" type=\"radio\" value=\"{value}\"{}><span class=\"form-kit__control-radio-content\">{}{}</span></label></div>",
            if value == requirement {
                " checked=\"\""
            } else {
                ""
            },
            escape(title),
            description
                .map(|d| format!(
                    " <span class=\"form-kit__control-radio-description\">{}</span>",
                    escape(&d)
                ))
                .unwrap_or_default()
        )
    };
    form.push_str(&field_div(
        "requirement",
        "radio-group",
        Some("full"),
        &k("requirement.title"),
        &format!(
            "<fieldset class=\"form-kit__fieldset form-kit__control-radio-group\" name=\"requirement\" id=\"control-requirement-input\">{}{}{}</fieldset>",
            radio("optional", &k("requirement.optional.title"), None),
            radio(
                "for_all_users",
                &k("requirement.for_all_users.title"),
                Some(k("requirement.for_all_users.description"))
            ),
            radio(
                "on_signup",
                &k("requirement.on_signup.title"),
                Some(k("requirement.on_signup.description"))
            ),
        ),
    ));
    let for_all = requirement == "for_all_users";
    let checkbox = |name: &str, checked: bool, disabled: bool, tooltip: Option<String>| {
        format!(
            "<div class=\"form-kit__container form-kit__field form-kit__field-checkbox\" data-name=\"{name}\" id=\"control-{name}\" data-control-type=\"checkbox\"><div class=\"form-kit__container-content\">{}</div></div>",
            crate::admin_form_kit::checkbox(
                name,
                checked,
                Some(&k(&format!("{name}.title"))),
                tooltip.as_deref(),
                disabled
            )
        )
    };
    form.push_str(&format!(
        "<div class=\"user-field-preferences\"><fieldset class=\"form-kit__fieldset form-kit__checkbox-group\"><legend class=\"form-kit__fieldset-title\">{}</legend>{}{}{}{}{}</fieldset></div>",
        escape(&k("preferences")),
        checkbox("editable", flag("editable") || for_all, for_all, None),
        checkbox("show_on_profile", flag("show_on_profile"), false, None),
        checkbox("show_on_user_card", flag("show_on_user_card"), false, None),
        checkbox("searchable", flag("searchable"), false, Some(k("searchable.tooltip"))),
        checkbox(
            "show_on_signup",
            flag("show_on_signup") || requirement != "optional",
            requirement != "optional" || editable_false,
            None
        ),
    ));
    form.push_str(&format!(
        // form.Submit draws no icon for its @icon.
        "<div class=\"form-kit__section form-kit__actions\"><button class=\"btn btn-primary form-kit__button save\" type=\"submit\"><span class=\"d-button-label\">{}</span></button><button class=\"form-kit__button btn btn-default\" type=\"button\" data-user-field-cancel=\"\"><span class=\"d-button-label\">{}</span></button></div>",
        escape(&k("save")),
        escape(&k("cancel"))
    ));
    format!(
        "<a class=\"btn btn-transparent back-button\" href=\"{}/admin/config/user-fields\">{}{}</a><div class=\"admin-config-area user-field\"><div class=\"admin-config-area__primary-content\"><div class=\"admin-config-area-card\"><form class=\"form-kit\" novalidate=\"\" data-user-field-form=\"\" data-user-field-id=\"{}\" data-requirement=\"{}\"{} data-options-field=\"{}\">{form}</form></div></div></div>",
        cx.base_path,
        icon("chevron-left", None),
        escape(&k("back")),
        field
            .and_then(|f| f["id"].as_i64())
            .map(|id| id.to_string())
            .unwrap_or_default(),
        escape(requirement),
        if editable_false {
            " data-editable-false=\"\""
        } else {
            ""
        },
        escape(&field_div(
            "options",
            "custom",
            Some("large"),
            &k("options"),
            &format!(
                "<div class=\"form-kit__control-custom\">{}</div>",
                value_list(cx, &[])
            ),
        ))
    )
}
