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
