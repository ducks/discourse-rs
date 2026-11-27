//! The tag groups pages (/tag_groups, /tag_groups/new, /tag_groups/:id):
//! templates/tag-groups.gjs around its index, new and edit outlets, the
//! last two TagGroupsForm. static/js/prefs-choosers.js opens the
//! TagChoosers and GroupChoosers; static/js/tag-groups.js saves and
//! deletes.

use serde_json::{Value, json};

use crate::i18n::I18n;
use crate::topic_list_view::{escape, icon};

/// PermissionType::FULL and READONLY.
const FULL: i64 = 1;
const READONLY: i64 = 3;

/// Which outlet the page draws.
pub enum Outlet<'a> {
    Index,
    New,
    /// TagGroupSerializer's JSON.
    Edit(&'a Value),
}

pub struct Page<'a> {
    pub i18n: &'a I18n,
    pub base_path: &'a str,
    pub tagging_enabled: bool,
    /// The groups, by name: (id, name).
    pub groups: &'a [(i32, String)],
    /// Site#groups less everyone: (id, name), by name.
    pub site_groups: &'a [(i32, String)],
    pub outlet: Outlet<'a>,
    /// select-kit's strings and the tag search's settings.
    pub chooser_strings: Value,
}

fn js(i18n: &I18n, key: &str) -> String {
    i18n.t(&format!("js.{key}")).unwrap_or_default().to_string()
}

pub fn render(p: &Page) -> String {
    let t = |key: &str| escape(&js(p.i18n, key));
    let mut out = format!(
        "<a class=\"tag-groups--back\" href=\"{}/tags\">{}<span>{}</span></a><div class=\"container tag-groups-container\"><h2>{}</h2>",
        p.base_path,
        icon("chevron-left", None),
        t("tagging.groups.back_btn"),
        t("tagging.groups.title"),
    );
    if !p.tagging_enabled {
        out.push_str(&format!(
            "<div class=\"tag-group-content\"><div class=\"alert info\">{}</div></div></div>",
            t("tagging.groups.disabled")
        ));
        return out;
    }
    let current = match p.outlet {
        Outlet::Edit(g) => g["id"].as_i64(),
        _ => None,
    };
    if !p.groups.is_empty() {
        out.push_str("<div class=\"tag-groups-sidebar content-list\"><ul>");
        for (id, name) in p.groups {
            let active = if current == Some(i64::from(*id)) {
                " class=\"active\" aria-current=\"page\""
            } else {
                ""
            };
            out.push_str(&format!(
                "<li><a href=\"{}/tag_groups/{id}\"{active}>{}</a></li>",
                p.base_path,
                escape(name)
            ));
        }
        out.push_str(&format!(
            "</ul><button class=\"btn btn-icon-text btn-default\" data-tag-groups-new=\"{}/tag_groups/new\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button></div>",
            p.base_path,
            icon("plus", None),
            t("tagging.groups.new")
        ));
    }
    match p.outlet {
        Outlet::Index => {
            out.push_str(&format!(
                "<div class=\"tag-group-content\"><h3>{}</h3><section class=\"tag-groups-about\"><p>{}</p></section><section>",
                t(if p.groups.is_empty() {
                    "tagging.groups.about_heading_empty"
                } else {
                    "tagging.groups.about_heading"
                }),
                t("tagging.groups.about_description")
            ));
            if p.groups.is_empty() {
                out.push_str(&format!(
                    "<a href=\"{}/tag_groups/new\" class=\"btn btn-primary\">{}{}</a>",
                    p.base_path,
                    icon("plus", None),
                    t("tagging.groups.new")
                ));
            }
            out.push_str("</section></div>");
        }
        Outlet::New => {
            let draft = json!({ "name": js(p.i18n, "tagging.groups.new_name") });
            out.push_str(&format!(
                "<div class=\"tag-group-content\"><h3>{}</h3>{}</div>",
                t("tagging.groups.new_title"),
                form(p, &draft, true)
            ));
        }
        Outlet::Edit(group) => {
            out.push_str(&format!(
                "<div class=\"tag-group-content\"><h3>{}</h3>{}</div>",
                t("tagging.groups.edit_title"),
                form(p, group, false)
            ));
        }
    }
    out.push_str("</div>");
    out
}

/// TagGroup#permissionName.
fn permission_name(permissions: &Value) -> &'static str {
    match permissions.get("0").and_then(Value::as_i64) {
        _ if !permissions.is_object() => "public",
        Some(FULL) => "public",
        Some(READONLY) => "visible",
        _ => "private",
    }
}

/// A TagChooser's or GroupChooser's closed header over `items`
/// ([value, name]).
fn chooser(class: &str, kind: &str, items: &[(String, String)], attrs: &str, none: &str) -> String {
    let names: Vec<&str> = items.iter().map(|(_, n)| n.as_str()).collect();
    let values: Vec<&str> = items.iter().map(|(v, _)| v.as_str()).collect();
    let shown = if names.is_empty() {
        none.to_string()
    } else {
        names.join(", ")
    };
    format!(
        "<details class=\"select-kit multi-select {class}{}\" data-chooser=\"{kind}\" data-items=\"{}\"{attrs}><summary data-name=\"{}\" data-value=\"{}\" tabindex=\"0\" class=\"select-kit-header multi-select-header\"><div class=\"select-kit-header-wrapper\"><span class=\"formatted-selection\">{}</span>{}</div></summary><div class=\"select-kit-body\"></div></details>",
        if items.is_empty() {
            ""
        } else {
            " has-selection"
        },
        escape(&Value::Array(items.iter().map(|(v, n)| json!([v, n])).collect()).to_string()),
        escape(&names.join(",")),
        escape(&values.join(",")),
        escape(&shown),
        icon("plus", Some("angle-icon")),
    )
}

fn tag_items(tags: &Value) -> Vec<(String, String)> {
    tags.as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let name = t["name"].as_str()?.to_string();
            let value = t["id"].as_i64().map_or(name.clone(), |id| id.to_string());
            Some((value, name))
        })
        .collect()
}

/// TagGroupsForm over `group` (its buffered fields).
fn form(p: &Page, group: &Value, is_new: bool) -> String {
    let t = |key: &str| escape(&js(p.i18n, key));
    let none = js(p.i18n, "select_kit.default_header_text");
    let permissions = &group["permissions"];
    let permission = permission_name(permissions);
    // selectedGroupIds: the groups but everyone with FULL.
    let selected: Vec<(String, String)> = permissions
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(id, perm)| id.as_str() != "0" && perm.as_i64() == Some(FULL))
        .filter_map(|(id, _)| {
            let id: i32 = id.parse().ok()?;
            let name = p.site_groups.iter().find(|(g, _)| *g == id)?.1.clone();
            Some((id.to_string(), name))
        })
        .collect();
    let options = escape(
        &Value::Array(
            p.site_groups
                .iter()
                .map(|(id, name)| json!([id.to_string(), name]))
                .collect(),
        )
        .to_string(),
    );
    let group_chooser = || {
        chooser(
            "group-chooser",
            "options",
            &selected,
            &format!(
                " data-group-ids=\"\" data-options=\"{options}\" data-placeholder=\"{}\"",
                t("tagging.groups.select_groups_placeholder")
            ),
            &none,
        )
    };
    let radio = |value: &str, label: &str| {
        format!(
            "<input class=\"tag-permissions-choice\" id=\"{value}-permission\" name=\"tag-permissions-choice\" type=\"radio\" value=\"{value}\"{}><label class=\"radio\" for=\"{value}-permission\">{}</label>",
            if permission == value { " checked" } else { "" },
            t(label)
        )
    };
    let tag_attrs = |placeholder: &str, max: Option<usize>| {
        format!(
            " data-own-scope=\"\" data-allow-any=\"\" data-exclude-synonyms=\"\" data-placeholder=\"{}\"{}",
            t(placeholder),
            max.map(|m| format!(" data-maximum=\"{m}\""))
                .unwrap_or_default()
        )
    };
    format!(
        "<form class=\"tag-group-form\" data-tag-group-id=\"{id}\" data-strings=\"{strings}\">\
<section class=\"group-name\"><label>{name_label}</label><div><input class=\"ember-text-field\" name=\"name\" type=\"text\" value=\"{name}\"></div></section>\
<section class=\"group-tags-list\"><label>{tags_label}</label><br>{tags}</section>\
<section class=\"parent-tag-section\"><label>{parent_label}</label><div>{parent}</div><div class=\"description\">{parent_description}</div></section>\
<section class=\"group-one-per-topic\"><label><input class=\"ember-checkbox\" name=\"onepertopic\" type=\"checkbox\"{one}> {one_label}</label></section>\
<section class=\"group-visibility\">\
<div class=\"group-visibility-option\">{public}</div>\
<div class=\"group-visibility-option\">{visible}<div class=\"group-access-control\">{chooser_a}</div></div>\
<div class=\"group-visibility-option\">{private}<div class=\"group-access-control\">{chooser_b}</div></div>\
</section>\
<div class=\"tag-group-controls\"><button class=\"btn btn-text btn-primary\" data-tag-group-save=\"\" type=\"button\"><span class=\"d-button-label\">{save}</span></button><button class=\"btn btn-icon-text btn-danger\" data-tag-group-delete=\"\" type=\"button\"{disabled}>{trash}<span class=\"d-button-label\">{delete}</span></button></div>\
</form>",
        id = group["id"]
            .as_i64()
            .map(|i| i.to_string())
            .unwrap_or_default(),
        strings = escape(
            &json!({
                "empty_name": js(p.i18n, "tagging.groups.cannot_save.empty_name"),
                "no_tags": js(p.i18n, "tagging.groups.cannot_save.no_tags"),
                "no_groups": js(p.i18n, "tagging.groups.cannot_save.no_groups"),
                "confirm_delete": js(p.i18n, "tagging.groups.confirm_delete"),
            })
            .to_string()
        ),
        name_label = t("tagging.groups.name_placeholder"),
        name = escape(group["name"].as_str().unwrap_or_default()),
        tags_label = t("tagging.groups.tags_label"),
        tags = chooser(
            "tag-chooser",
            "tag",
            &tag_items(&group["tags"]),
            &format!(
                " data-field=\"tags\"{}",
                tag_attrs("tagging.groups.tags_placeholder", None)
            ),
            &none
        ),
        parent_label = t("tagging.groups.parent_tag_label"),
        parent = chooser(
            "tag-chooser",
            "tag",
            &tag_items(&group["parent_tag"]),
            &format!(
                " data-field=\"parent_tag\"{}",
                tag_attrs("tagging.groups.parent_tag_placeholder", Some(1))
            ),
            &none
        ),
        parent_description = t("tagging.groups.parent_tag_description"),
        one = if group["one_per_topic"] == true {
            " checked"
        } else {
            ""
        },
        one_label = t("tagging.groups.one_per_topic_label"),
        public = radio("public", "tagging.groups.everyone_can_use"),
        visible = radio("visible", "tagging.groups.usable_only_by_groups"),
        private = radio("private", "tagging.groups.visible_only_to_groups"),
        chooser_a = group_chooser(),
        chooser_b = group_chooser(),
        save = t("tagging.groups.save"),
        disabled = if is_new { " disabled" } else { "" },
        trash = icon("trash-can", None),
        delete = t("tagging.groups.delete"),
    ) + &format!(
        "<script type=\"application/json\" id=\"prefs-choosers-strings\">{}</script>",
        p.chooser_strings.to_string().replace('<', "\\u003c")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_names() {
        assert_eq!(permission_name(&Value::Null), "public");
        assert_eq!(permission_name(&json!({"0": 1})), "public");
        assert_eq!(permission_name(&json!({"0": 3, "10": 1})), "visible");
        assert_eq!(permission_name(&json!({"10": 1})), "private");
    }
}
