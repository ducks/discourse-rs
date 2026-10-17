//! What the customize pages (90g7) draw with beyond crate::admin_ui and
//! crate::admin_form_kit: AdminConfigAreaCard, the select-kit choosers
//! (MultiSelects with their selection, the CategoryChooser ComboBox),
//! FKField's custom and input fields, DSaveControls.

use serde_json::Value;

use crate::admin_view::{AdminCx, t, t_with};
use crate::topic_list_view::{ListCategory, ListContext, escape, icon};

/// AdminConfigAreaCard: the heading over the content; with a toggle
/// title it is collapsable, and collapsed without content.
pub fn area_card(
    class: &str,
    heading: &str,
    content: Option<&str>,
    toggle_title: Option<&str>,
) -> String {
    let class = if class.is_empty() {
        String::new()
    } else {
        format!(" {class}")
    };
    let toggle = toggle_title
        .map(|title| {
            format!(
                "<button class=\"btn no-text admin-config-area-card__toggle-button btn-transparent\" title=\"{}\" type=\"button\">{}</button>",
                escape(title),
                icon(
                    if content.is_some() {
                        "angle-down"
                    } else {
                        "angle-right"
                    },
                    None
                )
            )
        })
        .unwrap_or_default();
    format!(
        "<section class=\"admin-config-area-card{class}\"><div class=\"admin-config-area-card__header-wrapper\"><h3 class=\"admin-config-area-card__title\">{}</h3>{toggle}</div>{}</section>",
        escape(heading),
        content
            .map(|c| format!("<div class=\"admin-config-area-card__content\">{c}</div>"))
            .unwrap_or_default()
    )
}

/// DSaveControls: the save button (disabled), what it yields after it.
pub fn save_controls(cx: &AdminCx, inner: &str) -> String {
    format!(
        "<div class=\"controls save-button\"><button class=\"btn btn-primary save-changes\" disabled=\"\" type=\"button\"><span class=\"d-button-label\">{}</span></button>{inner}</div>",
        escape(&t(cx.i18n, "save"))
    )
}

/// A select-kit MultiSelect closed with its selection (WatchedWords,
/// TagChooser, UserChooser): the header's names, the plus caret while
/// more may be chosen (`maximum`). admin-customize.js opens it: creatable
/// rows, or with data-search the tags' or users' results.
pub fn chooser(
    id: Option<&str>,
    class: &str,
    none: &str,
    chosen: &[String],
    maximum: Option<usize>,
    attrs: &str,
) -> String {
    let joined = escape(&chosen.join(","));
    let full = maximum.is_some_and(|m| chosen.len() >= m);
    let (id_attr, header_id, body_id) = match id {
        Some(id) => (
            format!(" id=\"{id}\""),
            format!(" id=\"{id}-header\""),
            format!(" id=\"{id}-body\""),
        ),
        None => Default::default(),
    };
    format!(
        "<details class=\"select-kit multi-select {class}{}\"{id_attr} data-chooser=\"\" data-chosen=\"{}\" data-none=\"{}\"{attrs}><summary class=\"select-kit-header multi-select-header\" data-name=\"{joined}\" data-value=\"{joined}\"{header_id} tabindex=\"0\"><div class=\"select-kit-header-wrapper\"><span class=\"formatted-selection\">{}</span>{}</div></summary><div class=\"select-kit-body\"{body_id}></div></details>",
        if chosen.is_empty() {
            ""
        } else {
            " has-selection"
        },
        escape(&serde_json::to_string(chosen).unwrap_or_default()),
        escape(none),
        if chosen.is_empty() {
            escape(none)
        } else {
            escape(&chosen.join(", "))
        },
        if full {
            String::new()
        } else {
            icon("plus", Some("angle-icon"))
        }
    )
}

/// The CategoryChooser ComboBox, closed: the category's badge, or blank.
/// admin-customize.js opens it with the site's categories.
pub fn category_chooser(
    cx: &AdminCx,
    list: &ListContext,
    class: &str,
    category: Option<&ListCategory>,
) -> String {
    let (name, value, badge) = match category {
        Some(c) => (
            c.name.clone(),
            c.id.to_string(),
            crate::topic_list_view::category_badge_html(list, c, false, false),
        ),
        None => (String::new(), String::new(), String::new()),
    };
    let label = if category.is_some() {
        t_with(cx.i18n, "select_kit.filter_by", &[("name", &name)])
    } else {
        t(cx.i18n, "select_kit.select_to_filter")
    };
    let choice_attrs = if category.is_some() {
        format!(" data-name=\"{}\" data-value=\"{value}\"", escape(&name))
    } else {
        String::new()
    };
    format!(
        "<details class=\"select-kit single-select combobox combo-box category-chooser{} {class}\" data-category-chooser=\"\" data-value=\"{value}\"><summary aria-label=\"{l}\" class=\"select-kit-header single-select-header combo-box-header\" data-name=\"{n}\" data-value=\"{value}\" name=\"{l}\" tabindex=\"0\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\"{choice_attrs} title=\"{n}\"><span class=\"name\">{badge}</span></div>{}</div></summary><div class=\"select-kit-body\"></div></details>",
        if category.is_some() {
            " has-selection"
        } else {
            ""
        },
        icon("angle-down", Some("angle-icon")),
        l = escape(&label),
        n = escape(&name),
    )
}

/// The site's categories for the CategoryChooser's rows: id, name and
/// badge, by name as the client sorts them.
pub fn categories_json(list: &ListContext) -> String {
    let mut all: Vec<&ListCategory> = list.categories.values().collect();
    all.sort_by_key(|c| c.name.to_lowercase());
    let rows: Vec<Value> = all
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id,
                "name": c.name,
                "badge": crate::topic_list_view::category_badge_html(list, c, false, false),
            })
        })
        .collect();
    format!(
        "<script type=\"application/json\" id=\"customize-categories\">{}</script>",
        Value::Array(rows).to_string().replace('<', "\\u003c")
    )
}

/// FKField `@type="custom"`: the control's markup in the container.
pub fn custom_field(
    name: &str,
    title: &str,
    optional: Option<&str>,
    description: Option<&str>,
    inner: &str,
) -> String {
    crate::admin_form_kit::Field {
        name,
        control: "custom",
        title: Some(title.to_string()),
        optional: optional.map(str::to_string),
        description: description.map(str::to_string),
        inner: format!("<div class=\"form-kit__control-custom\">{inner}</div>"),
        ..Default::default()
    }
    .render()
}

/// FKField `@type="input"`.
pub fn input_field(
    name: &str,
    title: &str,
    optional: Option<&str>,
    value: &str,
    placeholder: Option<&str>,
    format: Option<&str>,
) -> String {
    crate::admin_form_kit::Field {
        name,
        control: "input",
        title: Some(title.to_string()),
        format,
        optional: optional.map(str::to_string),
        inner: crate::admin_form_kit::input(name, value, "text", placeholder, false),
        ..Default::default()
    }
    .render()
}

/// FKField `@type="checkbox"` in a checkbox group.
pub fn checkbox_field(name: &str, title: &str, checked: bool) -> String {
    crate::admin_form_kit::Field {
        name,
        control: "checkbox",
        inner: crate::admin_form_kit::checkbox(name, checked, Some(title), None, false),
        ..Default::default()
    }
    .render()
}
