//! The category editor (Ember's edit-category route on FormKit, at
//! `/c/*slug/edit(/:tab)` and `/new-category`): the General, Security
//! and Settings tabs over the category as CategorySerializer gives it,
//! saved by static/js/category-edit.js as Ember's Category#save sends it
//! (PUT or POST /categories with the same JSON), and the delete section.
//! The category types are the General tab's checkboxes, Ideas among
//! them: topic voting's toggle.
//!
//! Not drawn: the Images, Topic template, Tags and Localizations tabs,
//! the emoji and icon pickers (plain inputs here) and the colour
//! palette.

use serde_json::{Value, json};

use crate::i18n::I18n;
use crate::topic_list_view::{escape, icon};

/// What the editor draws.
pub struct Editor<'a> {
    pub i18n: &'a I18n,
    pub base_path: &'a str,
    /// The category as CategorySerializer shows it to the editor, or the
    /// new category's defaults.
    pub category: &'a Value,
    /// The category's URL (`/c/slug/id`), none for a new one.
    pub url: Option<String>,
    /// The editor's own path (`/c/slug/path/edit`), none for a new one.
    pub edit_path: Option<String>,
    pub tab: &'a str,
    /// Top-level categories a parent may be: (id, name), the category
    /// itself left out.
    pub parents: Vec<(i64, String)>,
    /// Every group name, for the permissions' group picker.
    pub groups: Vec<String>,
    /// `Categories::TypeRegistry.list` for the viewer.
    pub types: &'a Value,
}

fn t(i18n: &I18n, key: &str) -> String {
    escape(i18n.t(&format!("js.{key}")).unwrap_or(key))
}

fn s(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A FormKit field: its title, then the control.
fn field(name: &str, control: &str, title: &str, inner: &str) -> String {
    format!(
        "<div class=\"form-kit__container form-kit__field form-kit__field-{control} --full\" \
         data-name=\"{name}\" id=\"control-{name}\" data-control-type=\"{control}\">\
         <div class=\"form-kit__container-title\"><span class=\"form-kit__container-primary-title\">{title}</span></div>\
         <div class=\"form-kit__container-content\">{inner}</div></div>"
    )
}

fn input(name: &str, value: &str, kind: &str, placeholder: &str) -> String {
    let placeholder = if placeholder.is_empty() {
        String::new()
    } else {
        format!(" placeholder=\"{placeholder}\"")
    };
    format!(
        "<div class=\"form-kit__control-input-wrapper\"><input class=\"form-kit__control-input\" \
         name=\"{name}\" value=\"{}\" type=\"{kind}\"{placeholder}></div>",
        escape(value)
    )
}

fn checkbox(name: &str, checked: bool, label: &str, disabled: bool) -> String {
    format!(
        "<label class=\"form-kit__control-checkbox-label\"><input class=\"form-kit__control-checkbox\" \
         name=\"{name}\" type=\"checkbox\"{}{}><span class=\"form-kit__control-checkbox-checkmark\">{}</span>\
         <span class=\"form-kit__control-checkbox-content\"><span class=\"form-kit__control-checkbox-title\">{label}</span></span></label>",
        if checked { " checked" } else { "" },
        if disabled { " disabled" } else { "" },
        icon("check", None)
    )
}

fn select(name: &str, value: &str, options: &[(String, String)]) -> String {
    let mut out = format!("<select class=\"form-kit__control-select\" name=\"{name}\">");
    for (v, label) in options {
        out.push_str(&format!(
            "<option value=\"{}\"{}>{}</option>",
            escape(v),
            if v == value { " selected" } else { "" },
            escape(label)
        ));
    }
    out.push_str("</select>");
    out
}

impl Editor<'_> {
    fn general(&self) -> String {
        let i18n = self.i18n;
        let c = self.category;
        let mut out = String::new();
        out.push_str(&field(
            "name",
            "input",
            &t(i18n, "category.name"),
            &input(
                "name",
                &s(&c["name"]),
                "text",
                &t(i18n, "category.name_placeholder"),
            ),
        ));
        out.push_str(&field(
            "slug",
            "input",
            &t(i18n, "category.slug"),
            &input(
                "slug",
                &s(&c["slug"]),
                "text",
                &t(i18n, "category.slug_placeholder"),
            ),
        ));
        let mut parents = vec![(
            String::new(),
            self.i18n
                .t("js.category.none_subcategory_text")
                .unwrap_or("None")
                .to_string(),
        )];
        parents.extend(
            self.parents
                .iter()
                .map(|(id, name)| (id.to_string(), name.clone())),
        );
        out.push_str(&field(
            "parent_category_id",
            "custom",
            &t(i18n, "category.parent"),
            &select("parent_category_id", &s(&c["parent_category_id"]), &parents),
        ));
        // The category types: Discussion always, the others toggled (Ideas
        // is topic voting).
        let current: Vec<String> = c["category_types"]
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        let mut types = String::new();
        for ty in self.types.as_array().into_iter().flatten() {
            let id = s(&ty["id"]);
            let discussion = id == "discussion";
            let checked = discussion || current.contains(&id);
            let unavailable = ty["can_enable_plugin"] == json!(false);
            types.push_str(&format!(
                "<div class=\"category-type-option\" data-category-type=\"{}\">{}</div>",
                escape(&id),
                checkbox(
                    "category_types",
                    checked,
                    &escape(&s(&ty["name"])),
                    discussion || (unavailable && !checked),
                )
            ));
        }
        out.push_str(&field(
            "category_types",
            "custom",
            &t(i18n, "category.category_types"),
            &types,
        ));
        let style = s(&c["style_type"]);
        let styles = [
            ("square", "category.styles.options.square"),
            ("icon", "category.styles.options.icon"),
            ("emoji", "category.styles.options.emoji"),
        ];
        let options: Vec<(String, String)> = styles
            .iter()
            .map(|(v, key)| {
                (
                    v.to_string(),
                    i18n.t(&format!("js.{key}")).unwrap_or(v).to_string(),
                )
            })
            .collect();
        out.push_str(&field(
            "style_type",
            "custom",
            &t(i18n, "category.style"),
            &select(
                "style_type",
                if style.is_empty() { "square" } else { &style },
                &options,
            ),
        ));
        out.push_str(&field(
            "emoji",
            "input",
            &t(i18n, "category.emoji"),
            &input("emoji", &s(&c["emoji"]), "text", ""),
        ));
        out.push_str(&field(
            "icon",
            "input",
            &t(i18n, "category.icon"),
            &input("icon", &s(&c["icon"]), "text", ""),
        ));
        for (name, key) in [
            ("color", "category.background_color"),
            ("text_color", "category.foreground_color"),
        ] {
            let value = s(&c[name]);
            out.push_str(&field(
                name,
                "custom",
                &t(i18n, key),
                &format!(
                    "<div class=\"category-color-editor\"><span class=\"color-swatch\" style=\"background-color: #{}\"></span>{}</div>",
                    escape(&value),
                    input(name, &value, "text", &t(i18n, "category.color_placeholder"))
                ),
            ));
        }
        out
    }

    fn security(&self) -> String {
        let i18n = self.i18n;
        let levels = [
            (1, "category.permissions.create"),
            (2, "category.permissions.reply"),
            (3, "category.permissions.see"),
        ];
        let mut rows = String::new();
        let perms = self.category["group_permissions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for p in &perms {
            let name = s(&p["group_name"]);
            let level = p["permission_type"].as_i64().unwrap_or(1);
            let options: Vec<(String, String)> = levels
                .iter()
                .map(|(v, key)| {
                    (
                        v.to_string(),
                        i18n.t(&format!("js.{key}")).unwrap_or(key).to_string(),
                    )
                })
                .collect();
            rows.push_str(&format!(
                "<div class=\"permission-row row-body\" data-group-name=\"{}\"><span class=\"group-name\">{}</span>{}\
                 <button class=\"btn btn-flat btn-icon no-text remove-permission\" type=\"button\">{}</button></div>",
                escape(&name),
                escape(&name),
                select("permission_type", &level.to_string(), &options),
                icon("trash-can", None)
            ));
        }
        let taken: Vec<String> = perms.iter().map(|p| s(&p["group_name"])).collect();
        let mut add = vec![(
            String::new(),
            self.i18n
                .t("js.category.security_add_group")
                .unwrap_or("Add a group")
                .to_string(),
        )];
        add.extend(
            self.groups
                .iter()
                .filter(|g| !taken.contains(g))
                .map(|g| (g.clone(), g.clone())),
        );
        format!(
            "<div class=\"category-permissions-table\" data-group-names=\"{}\">{rows}</div>\
             <div class=\"add-group\">{}</div>",
            escape(&serde_json::to_string(&self.groups).unwrap_or_default()),
            select("add_group", "", &add)
        )
    }

    fn settings(&self) -> String {
        let i18n = self.i18n;
        let c = self.category;
        let mut out = String::new();
        for (name, key) in [
            ("num_featured_topics", "category.num_featured_topics"),
            ("auto_close_hours", "topic.auto_close.label"),
            ("default_slow_mode_seconds", "topic.slow_mode_update.title"),
            ("minimum_required_tags", "category.minimum_required_tags"),
        ] {
            out.push_str(&field(
                name,
                "input-number",
                &t(i18n, key),
                &input(name, &s(&c[name]), "number", ""),
            ));
        }
        for (name, key) in [
            ("read_only_banner", "category.read_only_banner"),
            (
                "topic_title_placeholder",
                "category.topic_title_placeholder",
            ),
            ("sort_order", "category.sort_order"),
            ("default_view", "category.default_view"),
            ("default_top_period", "category.default_top_period"),
            ("default_list_filter", "category.default_list_filter"),
            ("subcategory_list_style", "category.subcategory_list_style"),
        ] {
            out.push_str(&field(
                name,
                "input",
                &t(i18n, key),
                &input(name, &s(&c[name]), "text", ""),
            ));
        }
        for (name, key) in [
            ("show_subcategory_list", "category.show_subcategory_list"),
            ("allow_badges", "category.allow_badges_label"),
            (
                "topic_featured_link_allowed",
                "category.topic_featured_link_allowed",
            ),
            ("all_topics_wiki", "category.all_topics_wiki"),
            (
                "allow_unlimited_owner_edits_on_first_post",
                "category.allow_unlimited_owner_edits_on_first_post",
            ),
            (
                "navigate_to_first_post_after_read",
                "category.navigate_to_first_post_after_read",
            ),
        ] {
            out.push_str(&field(
                name,
                "checkbox",
                "",
                &checkbox(name, c[name] == json!(true), &t(i18n, key), false),
            ));
        }
        // The category-custom-settings outlet: discourse-post-voting's
        // section, while it is on (its keys on the category say so).
        if c.get(crate::plugins::post_voting::CREATE_AS_POST_VOTING_DEFAULT)
            .is_some()
        {
            let mut fields = String::new();
            for key in [
                crate::plugins::post_voting::CREATE_AS_POST_VOTING_DEFAULT,
                crate::plugins::post_voting::ONLY_POST_VOTING_IN_THIS_CATEGORY,
            ] {
                let name = format!("custom_fields.{key}");
                fields.push_str(&field(
                    &name,
                    "checkbox",
                    &t(i18n, &format!("category.{key}")),
                    &checkbox(
                        &name,
                        c["custom_fields"][key] == json!(true),
                        &t(i18n, &format!("category.{key}")),
                        false,
                    ),
                ));
            }
            out.push_str(&format!(
                "<div class=\"form-kit__section category-custom-settings-outlet post-voting-category-settings\"><div class=\"form-kit__section-header\"><div class=\"form-kit__section-heading\"><h2 class=\"form-kit__section-title\">{}</h2></div></div>{fields}</div>",
                t(i18n, "category.post_voting_settings_heading")
            ));
        }
        out
    }

    /// The editor's `<div class="edit-category-page">`.
    pub fn html(&self) -> String {
        let i18n = self.i18n;
        let c = self.category;
        let new = self.url.is_none();
        let title = if new {
            t(i18n, "category.create")
        } else {
            escape(
                &i18n
                    .t_with(
                        "js.category.edit_dialog_title",
                        &[("categoryName", &s(&c["name"]))],
                    )
                    .unwrap_or_default(),
            )
        };
        let tab_base = match &self.edit_path {
            Some(path) => path.clone(),
            None => format!("{}/new-category", self.base_path),
        };
        let mut nav = String::from("<ul class=\"nav-pills edit-category-nav\">");
        for (tab, key) in [
            ("general", "category.general"),
            ("security", "category.security"),
            ("settings", "category.settings"),
        ] {
            nav.push_str(&format!(
                "<li class=\"edit-category-{tab}\"><a href=\"{tab_base}/{tab}\"{}>{}</a></li>",
                if tab == self.tab {
                    " class=\"active\""
                } else {
                    ""
                },
                t(i18n, key)
            ));
        }
        nav.push_str("</ul>");
        let mut sections = String::new();
        for (tab, body) in [
            ("general", self.general()),
            ("security", self.security()),
            ("settings", self.settings()),
        ] {
            sections.push_str(&format!(
                "<div class=\"edit-category-tab edit-category-tab-{tab}\"{}>{body}</div>",
                if tab == self.tab { "" } else { " hidden" }
            ));
        }
        let save = if new {
            t(i18n, "category.create_category")
        } else {
            t(i18n, "category.save")
        };
        let delete = match (&self.url, c["can_delete"] == json!(true)) {
            (Some(_), true) => format!(
                "<div class=\"edit-category-page__delete\"><div><h3 class=\"edit-category-page__delete-title\">{}</h3></div>\
                 <div class=\"edit-category-page__delete-actions\"><button class=\"btn btn-danger\" id=\"delete-category\" type=\"button\">{}{}</button></div></div>",
                t(i18n, "category.delete"),
                icon("trash-can", None),
                t(i18n, "category.delete")
            ),
            (Some(_), false) => match c["cannot_delete_reason"].as_str() {
                Some(reason) => format!(
                    "<div class=\"edit-category-page__delete --forbidden\"><p class=\"edit-category-page__delete-description\">{reason}</p></div>"
                ),
                None => String::new(),
            },
            (None, _) => String::new(),
        };
        let data = json!({
            "id": c["id"],
            "url": self.url,
            "position": c["position"],
            "strings": {
                "delete_confirm": i18n.t("js.category.delete_confirm"),
                "save_error": i18n.t("js.category.save_error"),
                "creation_error": i18n.t("js.category.creation_error"),
                "saved": i18n.t("js.saved"),
            },
        });
        format!(
            "<div class=\"edit-category-page\" data-category=\"{}\">\
             <div class=\"d-page-header edit-category-page-header\"><div class=\"d-page-header__title-row\">\
             <h1 class=\"d-page-header__title\">{title}</h1></div></div>{nav}\
             <form class=\"form-kit edit-category-form\" novalidate=\"\">{sections}\
             <div class=\"edit-category-errors alert alert-error\" hidden></div>\
             <div class=\"edit-category-footer\"><button class=\"btn btn-primary\" id=\"save-category\" type=\"submit\">{save}</button></div>\
             </form><div class=\"edit-category-page__footer\">{delete}</div></div>",
            escape(&data.to_string())
        )
    }
}

/// A new category as the creator starts it.
pub fn new_category() -> Value {
    json!({
        "id": null,
        "name": "",
        "slug": "",
        "color": "0088CC",
        "text_color": "FFFFFF",
        "style_type": "square",
        "emoji": null,
        "icon": null,
        "parent_category_id": null,
        "category_types": {"discussion": {}},
        "group_permissions": [{"permission_type": 1, "group_name": "everyone", "group_id": 0}],
        "num_featured_topics": 3,
        "subcategory_list_style": "rows_with_featured_topics",
        "default_top_period": "all",
        "default_list_filter": "all",
        "minimum_required_tags": 0,
        "allow_badges": true,
        "topic_featured_link_allowed": true,
        "search_priority": 0,
    })
}
