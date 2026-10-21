//! A theme's admin page (adminCustomizeThemes.show, 90g7) and its code
//! editor (adminCustomizeThemes.edit) as Ember draws them from
//! themes#index's document: templates/admin-customize-themes/show.gjs
//! around show/index.gjs, and edit.gjs around AdminThemeEditor.
//! static/js/admin-customize.js does what their controllers do,
//! static/js/site-settings.js edits the setting rows.

use serde_json::Value;

use crate::admin_site_settings_view::{LegacyRow, legacy_row};
use crate::admin_ui::Button;
use crate::admin_view::{AdminCx, t, t_with};
use crate::topic_list_view::{escape, icon};

/// THEME_UPLOAD_VAR
const UPLOAD_VAR: i64 = 2;
/// The settings YAML's and the translations' type (SETTINGS_TYPE_ID).
const SETTINGS_TYPE: i64 = 5;
/// The theme's JS initializer, as getField maps the common `js` field.
pub const JS_TARGET: &str = "extra_js";
pub const JS_FILENAME: &str = "discourse/api-initializers/theme-initializer.gjs";
/// AdminThemeEditor's JS_DEFAULT_VALUE.
pub const JS_DEFAULT_VALUE: &str = "import { apiInitializer } from \"discourse/lib/api\";\n\nexport default apiInitializer((api) => {\n  // Your code here\n});\n";

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.0),
        _ => true,
    }
}

fn blank(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        _ => false,
    }
}

fn array(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}

/// The theme's `fields` (Theme#fieldNames): each target's editable
/// fields.
pub fn field_names(target: &str) -> Option<&'static [&'static str]> {
    const COMMON: &[&str] = &[
        "scss",
        "head_tag",
        "header",
        "after_header",
        "body_tag",
        "footer",
        "js",
        "color_definitions",
        "embedded_scss",
        "embedded_header",
    ];
    const OTHER: &[&str] = &[
        "scss",
        "head_tag",
        "header",
        "after_header",
        "body_tag",
        "footer",
    ];
    match target {
        "common" => Some(COMMON),
        "desktop" | "mobile" => Some(OTHER),
        _ => None,
    }
}

const ADVANCED_TARGETS: [&str; 2] = ["desktop", "mobile"];
const ADVANCED_FIELDS: [&str; 4] = [
    "embedded_scss",
    "embedded_header",
    "color_definitions",
    "body_tag",
];

/// Theme#themeFields' lookup: the field of a basic type (or none) with
/// the target and name.
fn theme_field<'a>(theme: &'a Value, target: &str, name: &str) -> Option<&'a Value> {
    array(&theme["theme_fields"]).iter().rev().find(|f| {
        s(&f["target"]) == target
            && s(&f["name"]) == name
            && f["type_id"]
                .as_i64()
                .is_none_or(|id| [0, 1, 5, 6].contains(&id))
    })
}

/// Theme#getField, the common `js` read from its initializer.
pub fn get_field(theme: &Value, target: &str, name: &str) -> String {
    let (target, name) = if target == "common" && name == "js" {
        (JS_TARGET, JS_FILENAME)
    } else {
        (target, name)
    };
    theme_field(theme, target, name)
        .map(|f| s(&f["value"]).to_string())
        .unwrap_or_default()
}

/// Theme#getError
fn get_error(theme: &Value, target: &str, name: &str) -> String {
    theme_field(theme, target, name)
        .map(|f| s(&f["error"]).to_string())
        .unwrap_or_default()
}

/// Theme#hasEdited for a target (any field of it with a value) or one
/// field.
fn has_edited(theme: &Value, target: &str, name: Option<&str>) -> bool {
    match name {
        Some(name) => !get_field(theme, target, name).is_empty(),
        None => array(&theme["theme_fields"])
            .iter()
            .any(|f| s(&f["target"]) == target && !s(&f["value"]).is_empty()),
    }
}

/// Theme#hasError
fn has_error(theme: &Value, target: &str, name: Option<&str>) -> bool {
    array(&theme["theme_fields"]).iter().any(|f| {
        s(&f["target"]) == target && name.is_none_or(|n| n == s(&f["name"])) && truthy(&f["error"])
    })
}

/// DUserLink around its content.
fn user_link(cx: &AdminCx, user: &Value, inner: &str) -> String {
    let username = s(&user["username"]);
    format!(
        "<a aria-label=\"{}\" data-user-card=\"{u}\" href=\"{}/u/{u}\" tabindex=\"0\">{inner}</a>",
        escape(&t_with(
            cx.i18n,
            "user.profile_possessive",
            &[("username", username)]
        )),
        cx.base_path,
        u = escape(username),
    )
}

/// What the theme's page reads besides the theme.
pub struct ShowCx<'a> {
    /// themes#index's themes, by name.
    pub all: &'a [Value],
    /// extras.color_schemes
    pub color_schemes: &'a [Value],
    /// The translations' locale (model.locale, else the user's).
    pub locale: &'a str,
    /// available_locales: (value, name).
    pub locales: &'a [(String, String)],
}

/// templates/admin-customize-themes/show.gjs with its index outlet.
pub fn show(cx: &AdminCx, theme: &Value, page: &ShowCx) -> String {
    let i18n = cx.i18n;
    let component = truthy(&theme["component"]);
    let system = truthy(&theme["system"]);
    let remote = &theme["remote_theme"];
    let mut out = String::from(
        "<div class=\"admin-config-page\"><div class=\"show-current-style admin-customize-themes-show\">",
    );
    out.push_str(&format!(
        "<div class=\"back-to-themes-and-components\"><a href=\"{}/admin/config/customize/{}\">{}{}</a></div><span></span>",
        cx.base_path,
        if component { "components" } else { "themes" },
        icon("angle-left", None),
        escape(&t(
            i18n,
            if component {
                "admin.config_areas.themes_and_components.components.back"
            } else {
                "admin.config_areas.themes_and_components.themes.back"
            }
        ))
    ));
    let name = s(&theme["name"]);
    out.push_str(&format!(
        "<div class=\"title admin-customize-themes-show__title\"><button aria-label=\"Edit theme name: {n}\" class=\"btn no-text btn-transparent title-button\" aria-level=\"2\" role=\"heading\" type=\"button\"><span>{n}</span>{}</button></div>",
        if system {
            String::new()
        } else {
            icon("pencil", Some("inline-icon"))
        },
        n = escape(name)
    ));
    for error in array(&theme["errors"]) {
        out.push_str(&format!(
            "<div class=\"alert alert-error\">{}</div>",
            escape(s(error))
        ));
    }
    let remote_url = s(&remote["remote_url"]);
    let source_is_http = remote_url.starts_with("http://") || remote_url.starts_with("https://");
    let remote_link = if truthy(&remote["branch"]) {
        format!(
            "{}/tree/{}",
            remote_url.strip_suffix(".git").unwrap_or(remote_url),
            s(&remote["branch"])
        )
    } else {
        remote_url.to_string()
    };
    let remote_error = s(&remote["last_error_text"]);
    let remote_error_html = format!(
        "<div class=\"error-message\">{} {}</div><div class=\"raw-error\"><code>{}</code></div>",
        icon("triangle-exclamation", None),
        escape(&t(i18n, "admin.customize.theme.repo_unreachable")),
        escape(remote_error)
    );
    let finish_install = !remote_url.is_empty()
        && !truthy(&remote["local_version"])
        && !truthy(&remote["commits_behind"]);
    if finish_install {
        out.push_str("<div class=\"control-unit\">");
        if source_is_http {
            out.push_str(&format!(
                "<a class=\"remote-url\" href=\"{}\">{}{}</a>",
                escape(&remote_link),
                escape(&t(i18n, "admin.customize.theme.source_url")),
                icon("link", None)
            ));
        } else {
            out.push_str(&format!(
                "<div class=\"remote-url\"><code>{}</code>{}</div>",
                escape(remote_url),
                if truthy(&remote["branch"]) {
                    format!(" (<code>{}</code>)", escape(s(&remote["branch"])))
                } else {
                    String::new()
                }
            ));
        }
        out.push_str(&format!(
            "<p class=\"description\">{}</p>",
            escape(&t(i18n, "admin.customize.theme.placeholder_description"))
        ));
        if !remote_error.is_empty() {
            out.push_str(&remote_error_html);
        }
        out.push_str(
            &Button {
                class: "btn-primary finish-install",
                icon: Some("download"),
                label: Some(t(i18n, "admin.customize.theme.finish_install")),
                ..Default::default()
            }
            .render(),
        );
        out.push_str(
            &Button {
                class: "btn-danger",
                attrs: " data-action=\"delete\"".into(),
                icon: Some("trash-can"),
                label: Some(t(i18n, "admin.customize.delete")),
                ..Default::default()
            }
            .render(),
        );
        out.push_str(&format!(
            "<span class=\"status-message\">{} {}</span></div>",
            escape(&t(i18n, "admin.customize.theme.last_attempt")),
            crate::admin_ui::time(&remote["updated_at"])
                .map(|at| crate::admin_ui::format_date(cx, at, true))
                .unwrap_or_default()
        ));
        out.push_str("</div></div>");
        return out;
    }
    if !truthy(&theme["supported"]) {
        out.push_str("<div class=\"alert alert-error\">");
        out.push_str(&escape(&t(
            i18n,
            "admin.customize.theme.required_version.error",
        )));
        for (key, field) in [
            ("minimum", "minimum_discourse_version"),
            ("maximum", "maximum_discourse_version"),
        ] {
            if truthy(&remote[field]) {
                out.push(' ');
                out.push_str(&escape(&t_with(
                    i18n,
                    &format!("admin.customize.theme.required_version.{key}"),
                    &[("version", s(&remote[field]))],
                )));
            }
        }
        out.push_str("</div>");
    }
    if !truthy(&theme["enabled"]) {
        out.push_str("<div class=\"alert alert-error\">");
        let by = &theme["disabled_by"];
        if truthy(by) {
            out.push_str(&escape(&t(i18n, "admin.customize.theme.disabled_by")));
            out.push(' ');
            let avatar = crate::post_view::avatar_img(
                s(&by["avatar_template"]),
                20,
                &format!(" title=\"{}\"", escape(s(&by["username"]))),
            );
            out.push_str(&user_link(
                cx,
                by,
                &format!("{avatar} {}", escape(s(&by["username"]))),
            ));
            if let Some(at) = crate::admin_ui::time(&theme["disabled_at"]) {
                out.push(' ');
                out.push_str(&crate::admin_ui::format_date(cx, at, true));
            }
        } else {
            out.push_str(&escape(&t(i18n, "admin.customize.theme.disabled")));
        }
        out.push_str(
            &Button {
                class: "btn-default",
                icon: Some("check"),
                label: Some(t(i18n, "admin.customize.theme.enable")),
                attrs: " data-action=\"enable\"".into(),
                ..Default::default()
            }
            .render(),
        );
        out.push_str("</div>");
    }
    out.push_str("<div class=\"metadata control-unit remote-theme-metadata\">");
    if truthy(remote) {
        if !remote_url.is_empty() {
            let pretty = if remote_url.starts_with("http://github.com/")
                || remote_url.starts_with("https://github.com/")
            {
                let bare = remote_url.split_once("://").map_or(remote_url, |(_, r)| r);
                bare.strip_suffix(".git").unwrap_or(bare).to_string()
            } else {
                remote_url.to_string()
            };
            let branch = [&remote["local_compat_ref"], &remote["branch"]]
                .into_iter()
                .find(|v| truthy(v))
                .map(|v| s(v).to_string());
            out.push_str(&format!(
                "<div class=\"remote-url\">{}{}</div>",
                if source_is_http {
                    format!(
                        "<a href=\"{}\">{}</a>",
                        escape(&remote_link),
                        escape(&pretty)
                    )
                } else {
                    escape(&pretty)
                },
                branch
                    .map(|b| format!(" (<code>{}</code>)", escape(&b)))
                    .unwrap_or_default()
            ));
        }
        for (field, class, key) in [
            ("about_url", "about-url", "about_theme"),
            ("license_url", "license-url", "license"),
        ] {
            if truthy(&remote[field]) {
                out.push_str(&format!(
                    "<a class=\"url {class}\" href=\"{}\">{}{}</a>",
                    escape(s(&remote[field])),
                    escape(&t(i18n, &format!("admin.customize.theme.{key}"))),
                    icon("link", None)
                ));
            }
        }
        if truthy(&theme["description"]) {
            out.push_str(&format!(
                "<span class=\"theme-description\">{}</span>",
                escape(s(&theme["description"]))
            ));
        }
        for (field, class, key) in [
            ("authors", "authors", "authors"),
            ("theme_version", "version", "version"),
        ] {
            if truthy(&remote[field]) {
                out.push_str(&format!(
                    "<span class=\"{class}\"><span class=\"heading\">{}</span> {}</span>",
                    escape(&t(i18n, &format!("admin.customize.theme.{key}"))),
                    escape(&match &remote[field] {
                        Value::String(v) => v.clone(),
                        other => other.to_string(),
                    })
                ));
            }
        }
        if !remote_error.is_empty() {
            out.push_str(&remote_error_html);
        }
    }
    out.push_str("</div>");
    if system {
        out.push_str(&format!(
            "<div class=\"alert alert-info system-theme-info\">{}</div>",
            escape(&t(i18n, "admin.customize.theme.built_in_description"))
        ));
    }
    if truthy(&theme["only_theme_color_schemes"]) {
        out.push_str(&format!(
            "<div class=\"alert alert-info\">{}</div>",
            escape(&t(i18n, "admin.customize.theme.only_theme_color_schemes"))
        ));
    }
    out.push_str(&show_index(cx, theme, page));
    out.push_str("</div></div>");
    out
}

/// The relatives selector's SiteSetting (relativesSelectorSettingsFor
/// Component / ForTheme) drawn by ThemeSettingRelativesSelector.
fn relatives(cx: &AdminCx, theme: &Value, page: &ShowCx) -> String {
    let i18n = cx.i18n;
    let component = truthy(&theme["component"]);
    let id = theme["id"].as_i64();
    let names = |list: &[&Value]| -> Vec<String> {
        list.iter().map(|t| s(&t["name"]).to_string()).collect()
    };
    // The relatives as afterFindAll maps them onto the loaded themes.
    let mapped = |key: &str| -> Vec<&Value> {
        array(&theme[key])
            .iter()
            .filter_map(|r| page.all.iter().find(|t| t["id"] == r["id"]))
            .collect()
    };
    let (setting, label, choices, value, defaults, button) = if component {
        let available: Vec<&Value> = page
            .all
            .iter()
            .filter(|t| t["component"] == Value::Bool(false))
            .collect();
        (
            "parent_theme_ids",
            "admin.customize.theme.component_on_themes",
            names(&available),
            names(&mapped("parent_themes")),
            names(&available),
            "admin.customize.theme.add_all_themes",
        )
    } else {
        let available: Vec<&Value> = page
            .all
            .iter()
            .filter(|t| t["id"].as_i64() != id && truthy(&t["component"]))
            .collect();
        let active: Vec<&Value> = available
            .iter()
            .copied()
            .filter(|t| !array(&t["parent_themes"]).is_empty())
            .collect();
        (
            "child_theme_ids",
            "admin.customize.theme.included_components",
            names(&available),
            names(&mapped("child_themes")),
            names(&active),
            "admin.customize.theme.add_all",
        )
    };
    let joined = value.join("|");
    let ids: serde_json::Map<String, Value> = page
        .all
        .iter()
        .map(|t| (s(&t["name"]).to_string(), t["id"].clone()))
        .collect();
    let row = legacy_row(
        cx,
        &serde_json::json!({
            "setting": setting,
            "type": "list",
            "list_type": "compact",
            "label": t(i18n, label),
            "default": joined,
            "value": joined,
        }),
        &LegacyRow {
            class: "theme-setting",
            name: None,
            staff_log: Some((setting.to_string(), "change_site_setting")),
            kind: None,
            choices: Some(choices),
            default_values: Some((defaults, t(i18n, button))),
            attrs: format!(
                " data-saver=\"theme-relatives\" data-theme-ids=\"{}\"",
                escape(&Value::Object(ids).to_string())
            ),
        },
    );
    format!(
        "<section class=\"form-horizontal theme settings control-unit relative-theme-selector {}\"><div class=\"row setting\">{row}</div></section>",
        if component {
            "parent-themes-setting"
        } else {
            "included-components-setting"
        }
    )
}

/// ColorPalettePicker for one of the theme's palettes, with its
/// description and the controls admin-customize.js shows while it is
/// changed.
fn palette_picker(cx: &AdminCx, theme: &Value, page: &ShowCx, dark: bool) -> String {
    let i18n = cx.i18n;
    let only_theme = truthy(&theme["only_theme_color_schemes"]);
    let schemes = filtered_schemes(theme, page);
    let rows: Vec<(String, String)> = schemes
        .iter()
        .map(|c| (c["id"].to_string(), s(&c["name"]).to_string()))
        .collect();
    let field = if dark {
        "dark_color_scheme_id"
    } else {
        "color_scheme_id"
    };
    let selected = theme[field].as_i64().map(|id| id.to_string());
    let none = (!only_theme).then(|| t(i18n, "admin.customize.theme.default_light_scheme"));
    let picker = crate::admin_ui::combo_box(
        cx,
        "color-palette-picker",
        &rows,
        selected.as_deref(),
        none.as_deref(),
    );
    let edit = selected
        .as_ref()
        .map(|id| {
            format!(
                "<a href=\"{}/admin/config/colors/{id}\">{}</a>",
                cx.base_path,
                escape(&t(i18n, "admin.customize.theme.edit_colors"))
            )
        })
        .unwrap_or_default();
    let (class, label, desc) = if dark {
        (
            "theme-settings__dark-color-scheme",
            "admin.customize.theme.dark_color_scheme",
            "admin.customize.theme.dark_color_scheme_select",
        )
    } else {
        (
            "theme-settings__light-color-scheme",
            "admin.customize.theme.color_scheme",
            "admin.customize.theme.color_scheme_select",
        )
    };
    format!(
        "<section class=\"form-horizontal theme settings control-unit {class}\" data-field=\"{field}\" data-value=\"{}\"><div class=\"row setting\"><div class=\"setting-label\">{}</div><div class=\"setting-value\"><div class=\"color-palette-input-group\">{picker}</div><div class=\"desc\">{} {edit}</div></div><div class=\"setting-controls\"></div></div></section>",
        selected.as_deref().unwrap_or_default(),
        escape(&t(i18n, label)),
        escape(&t(i18n, desc)),
    )
}

/// filteredColorSchemes: the theme's own palettes when it keeps to them
/// and has any.
fn filtered_schemes<'a>(theme: &Value, page: &ShowCx<'a>) -> Vec<&'a Value> {
    let all: Vec<&Value> = page.color_schemes.iter().collect();
    if !truthy(&theme["only_theme_color_schemes"]) {
        return all;
    }
    let own: Vec<&Value> = all
        .iter()
        .copied()
        .filter(|c| c["theme_id"] == theme["id"])
        .collect();
    if own.is_empty() { all } else { own }
}

/// templates/admin-customize-themes/show/index.gjs
fn show_index(cx: &AdminCx, theme: &Value, page: &ShowCx) -> String {
    let i18n = cx.i18n;
    let tr = |k: &str| escape(&t(i18n, k));
    let component = truthy(&theme["component"]);
    let system = truthy(&theme["system"]);
    let remote = &theme["remote_theme"];
    let is_git = truthy(&remote["is_git"]);
    let id = theme["id"].as_i64().unwrap_or_default();
    let mut out = String::from("<div class=\"metadata control-unit\">");
    if truthy(remote) {
        out.push_str("<div class=\"control-unit\">");
        if is_git {
            let behind = remote["commits_behind"].as_i64().unwrap_or_default();
            if behind != 0 {
                out.push_str(
                    &Button {
                        class: "btn-primary",
                        attrs: " data-action=\"update\"".into(),
                        icon: Some("download"),
                        label: Some(t(i18n, "admin.customize.theme.update_to_latest")),
                        ..Default::default()
                    }
                    .render(),
                );
            } else {
                out.push_str(
                    &Button {
                        class: "btn-default",
                        attrs: " data-action=\"check\"".into(),
                        icon: Some("arrows-rotate"),
                        label: Some(t(i18n, "admin.customize.theme.check_for_updates")),
                        ..Default::default()
                    }
                    .render(),
                );
            }
            out.push_str(
                &Button {
                    class: "btn-default",
                    attrs: " data-action=\"change-source\"".into(),
                    icon: Some("code-branch"),
                    label: Some(t(i18n, "admin.customize.theme.change_source.button")),
                    ..Default::default()
                }
                .render(),
            );
            out.push_str("<span class=\"status-message\">");
            let branch = [&remote["remote_compat_ref"], &remote["branch"]]
                .into_iter()
                .find(|v| truthy(v))
                .map(|v| s(v).to_string());
            let code = |b: &str| format!("<code>{}</code>", escape(b));
            if behind != 0 {
                let overwritten =
                    remote["local_version"] != remote["remote_version"] && behind == -1;
                if overwritten {
                    out.push_str(&tr("admin.customize.theme.has_overwritten_history"));
                } else if let Some(b) = &branch {
                    out.push_str(&crate::admin_view::t_count(
                        i18n,
                        "admin.customize.theme.commits_behind_branch",
                        behind,
                        &[("branch", &code(b))],
                    ));
                } else {
                    out.push_str(&escape(&crate::admin_view::t_count(
                        i18n,
                        "admin.customize.theme.commits_behind",
                        behind,
                        &[],
                    )));
                }
                if truthy(&remote["github_diff_link"]) {
                    out.push_str(&format!(
                        " <a href=\"{}\">{}</a>",
                        escape(s(&remote["github_diff_link"])),
                        tr("admin.customize.theme.compare_commits")
                    ));
                }
            } else if s(&remote["last_error_text"]).is_empty() {
                match &branch {
                    Some(b) => out.push_str(&t_with(
                        i18n,
                        "admin.customize.theme.up_to_date_branch",
                        &[("branch", &code(b))],
                    )),
                    None => out.push_str(&tr("admin.customize.theme.up_to_date")),
                }
                if let Some(at) = crate::admin_ui::time(&remote["updated_at"]) {
                    out.push(' ');
                    out.push_str(&crate::admin_ui::format_date(cx, at, true));
                }
            }
            out.push_str("</span>");
        } else {
            out.push_str(&format!(
                "<span class=\"status-message\">{} {}</span>",
                icon("circle-info", None),
                tr("admin.customize.theme.imported_from_archive")
            ));
        }
        out.push_str("</div>");
    } else if !system {
        let user = &theme["user"];
        out.push_str(&format!(
            "<span class=\"heading created-by\">{}</span><span>{}</span>",
            tr("admin.customize.theme.creator"),
            user_link(cx, user, &escape(s(&user["username"])))
        ));
    }
    out.push_str("</div>");
    // showCheckboxes
    if !component || truthy(remote) {
        out.push_str("<div class=\"control-unit\">");
        let checkbox = |field: &str, key: &str, checked: bool| {
            format!(
                "<div class=\"inline-edit\" data-field=\"{field}\" data-checked=\"{checked}\"><label class=\"checkbox-label\"><input{} type=\"checkbox\"> {}</label></div>",
                if checked { " checked" } else { "" },
                tr(key)
            )
        };
        if !component {
            out.push_str(&checkbox(
                "default",
                "admin.customize.theme.is_default",
                truthy(&theme["default"]),
            ));
            out.push_str(&checkbox(
                "user_selectable",
                "admin.customize.theme.user_selectable",
                truthy(&theme["user_selectable"]),
            ));
        }
        if truthy(remote) {
            out.push_str(&checkbox(
                "auto_update",
                "admin.customize.theme.auto_update",
                truthy(&theme["auto_update"]),
            ));
        }
        out.push_str("</div>");
    }
    if !component {
        // showColorSchemePickers
        let schemes = filtered_schemes(theme, page);
        let show = !truthy(&theme["only_theme_color_schemes"]) || {
            let dark = schemes.iter().filter(|c| truthy(&c["is_dark"])).count();
            schemes.len() - dark > 1 || dark > 1
        };
        if show {
            out.push_str(&palette_picker(cx, theme, page, false));
            out.push_str(&palette_picker(cx, theme, page, true));
        }
    }
    out.push_str(&relatives(cx, theme, page));
    if !(system || is_git) {
        out.push_str(&format!(
            "<div class=\"control-unit\"><div class=\"mini-title\">{}</div>",
            tr("admin.customize.theme.css_html")
        ));
        let edited: Vec<&Value> = array(&theme["theme_fields"])
            .iter()
            .filter(|f| !blank(&f["value"]) && f["type_id"].as_i64() != Some(SETTINGS_TYPE))
            .collect();
        if edited.is_empty() {
            out.push_str(&format!(
                "<div class=\"description\">{}</div>",
                tr("admin.customize.theme.edit_css_html_help")
            ));
        } else {
            out.push_str(&format!(
                "<div class=\"description\">{}</div><ul>",
                tr("admin.customize.theme.custom_sections")
            ));
            for target in ["common", "desktop", "mobile"] {
                let mut fields: Vec<&str> = edited
                    .iter()
                    .filter(|f| s(&f["target"]) == target)
                    .map(|f| s(&f["name"]))
                    .collect();
                if target == "common" && !get_field(theme, "common", "js").is_empty() {
                    fields.push("js");
                }
                if fields.is_empty() {
                    continue;
                }
                let names: Vec<String> = fields
                    .iter()
                    .map(|n| t(i18n, &format!("admin.customize.theme.{n}.text")))
                    .collect();
                out.push_str(&format!(
                    "<li>{}: {}</li>",
                    tr(&format!("admin.customize.theme.{target}")),
                    escape(&names.join(", "))
                ));
            }
            out.push_str("</ul>");
        }
        out.push_str(
            &Button {
                class: "btn-default edit edit-code",
                label: Some(t(i18n, "admin.customize.theme.edit_css_html")),
                attrs: if is_git {
                    " data-confirm=\"true\"".into()
                } else {
                    String::new()
                },
                ..Default::default()
            }
            .render(),
        );
        out.push_str("</div>");
        out.push_str(&format!(
            "<div class=\"control-unit\"><div class=\"mini-title\">{}</div>",
            tr("admin.customize.theme.uploads")
        ));
        let uploads: Vec<&Value> = array(&theme["theme_fields"])
            .iter()
            .filter(|f| s(&f["target"]) == "common" && f["type_id"].as_i64() == Some(UPLOAD_VAR))
            .collect();
        if uploads.is_empty() {
            out.push_str(&format!(
                "<div class=\"description\">{}</div>",
                tr("admin.customize.theme.no_uploads")
            ));
        } else {
            out.push_str("<ul class=\"removable-list\">");
            for upload in uploads {
                out.push_str(&format!(
                    "<li data-name=\"{n}\"><span class=\"col\">${n}: <a href=\"{}\" rel=\"noopener noreferrer\" target=\"_blank\">{}</a></span><span class=\"col\">{}</span></li>",
                    escape(s(&upload["url"])),
                    escape(s(&upload["filename"])),
                    Button {
                        class: "second btn-default btn-default cancel-edit",
                        icon: Some("xmark"),
                        ..Default::default()
                    }
                    .render(),
                    n = escape(s(&upload["name"])),
                ));
            }
            out.push_str("</ul>");
        }
        out.push_str(
            &Button {
                class: "btn-default upload",
                icon: Some("plus"),
                label: Some(t(i18n, "admin.customize.theme.add")),
                ..Default::default()
            }
            .render(),
        );
        out.push_str("</div>");
    }
    let mut extra: Vec<&Value> = array(&theme["theme_fields"])
        .iter()
        .filter(|f| {
            let type_id = f["type_id"].as_i64();
            (s(&f["target"]) == "extra_js" && type_id == Some(6))
                || (s(&f["target"]) == "extra_scss" && type_id == Some(1))
        })
        .collect();
    if !extra.is_empty() && !system {
        extra.sort_by(|a, b| s(&a["file_path"]).cmp(s(&b["file_path"])));
        out.push_str(&format!(
            "<div class=\"control-unit extra-files\"><div class=\"mini-title\">{}</div><details><summary>{}</summary><ul>",
            tr("admin.customize.theme.extra_files"),
            tr(if truthy(remote) {
                "admin.customize.theme.extra_files_remote"
            } else {
                "admin.customize.theme.extra_files_upload"
            })
        ));
        for f in extra {
            out.push_str(&format!("<li>{}</li>", escape(s(&f["file_path"]))));
        }
        out.push_str("</ul></details></div>");
    }
    let name = s(&theme["name"]);
    let site_settings = array(&theme["themeable_site_settings"]);
    if !site_settings.is_empty() {
        out.push_str(&format!(
            "<div class=\"control-unit\"><div class=\"mini-title\">{}</div><p><i>{}</i></p><section class=\"form-horizontal theme settings theme-site-settings control-unit\">",
            tr("admin.customize.theme.theme_site_settings"),
            t_with(
                i18n,
                "admin.customize.theme.overriden_site_settings_explanation",
                &[(
                    "themeSiteSettingsConfigUrl",
                    &format!("{}/admin/config/customize/theme-site-settings", cx.base_path)
                )]
            )
        ));
        for setting in site_settings {
            out.push_str(&legacy_row(
                cx,
                setting,
                &LegacyRow {
                    class: "theme-site-setting control-unit",
                    name: None,
                    staff_log: Some((
                        format!("{name}: {}", s(&setting["setting"])),
                        "change_theme_site_setting",
                    )),
                    kind: None,
                    choices: None,
                    default_values: None,
                    attrs: " data-saver=\"theme-site-setting\"".into(),
                },
            ));
        }
        out.push_str("</section></div>");
    }
    let settings = array(&theme["settings"]);
    if !settings.is_empty() {
        out.push_str(&format!(
            "<div class=\"control-unit theme-settings\"><div class=\"mini-title\">{}</div><p><i>{}</i></p><section class=\"form-horizontal theme settings control-unit\">",
            tr("admin.customize.theme.theme_settings"),
            tr("admin.customize.theme.overriden_settings_explanation")
        ));
        for setting in settings {
            out.push_str(&legacy_row(
                cx,
                setting,
                &LegacyRow {
                    class: "theme-setting control-unit",
                    name: None,
                    staff_log: Some((s(&setting["setting"]).to_string(), "change_site_setting")),
                    kind: None,
                    choices: None,
                    default_values: None,
                    attrs: " data-saver=\"theme-setting\"".into(),
                },
            ));
        }
        out.push_str("</section></div>");
    }
    let translations = array(&theme["translations"]);
    if !translations.is_empty() {
        let picker = crate::admin_ui::combo_box(
            cx,
            "translation-selector",
            page.locales,
            Some(page.locale),
            None,
        );
        out.push_str(&format!(
            "<div class=\"control-unit\"><div class=\"translation-selector-container\"><span class=\"mini-title\">{}</span><div class=\"translation-selector-controls\">{picker}</div></div><div class=\"loading-container\"><section class=\"form-horizontal theme settings translations control-unit\">",
            tr("admin.customize.theme.theme_translations")
        ));
        out.push_str(&translation_rows(cx, translations, page.locale));
        out.push_str("</section></div></div>");
    }
    out.push_str("<div class=\"theme-controls\">");
    out.push_str(&format!(
        "<a class=\"btn btn-default\" href=\"{}/admin/themes/{id}/preview\" rel=\"noopener noreferrer\" target=\"_blank\" title=\"{}\">{}{}</a>",
        cx.base_path,
        tr("admin.customize.explain_preview"),
        icon("desktop", None),
        tr("admin.customize.theme.preview")
    ));
    if !system {
        // DButton with @href: its link.
        out.push_str(&format!(
            "<a class=\"btn btn-icon-text btn-default export\" href=\"{}/admin/customize/themes/{id}/export\">{}<span class=\"d-button-label\">{}</span></a>",
            cx.base_path,
            icon("download", None),
            tr("admin.export_json.button_text")
        ));
    }
    let is_default = truthy(&theme["default"]);
    if theme["user"]["id"].as_i64().unwrap_or_default() > 0 && !is_default {
        out.push_str(
            &Button {
                class: "btn-default btn-normal",
                attrs: " data-action=\"convert\"".into(),
                icon: Some("rotate"),
                label: Some(t(i18n, "admin.customize.theme.convert")),
                title: Some(t(
                    i18n,
                    if component {
                        "admin.customize.theme.convert_component_tooltip"
                    } else {
                        "admin.customize.theme.convert_theme_tooltip"
                    },
                )),
                ..Default::default()
            }
            .render(),
        );
    }
    if component {
        let enabled = truthy(&theme["enabled"]);
        out.push_str(
            &Button {
                class: "btn-default",
                attrs: format!(
                    " data-action=\"{}\"",
                    if enabled { "disable" } else { "enable" }
                ),
                icon: Some(if enabled { "ban" } else { "check" }),
                label: Some(t(
                    i18n,
                    if enabled {
                        "admin.customize.theme.disable"
                    } else {
                        "admin.customize.theme.enable"
                    },
                )),
                ..Default::default()
            }
            .render(),
        );
    }
    if !settings.is_empty() && !system {
        out.push_str(
            &Button {
                class: "btn-default btn-normal",
                attrs: " data-action=\"settings-editor\"".into(),
                icon: Some("pencil"),
                label: Some(t(i18n, "admin.customize.theme.settings_editor")),
                title: Some(t(i18n, "admin.customize.theme.settings_editor")),
                ..Default::default()
            }
            .render(),
        );
    }
    if !(system || is_default) {
        out.push_str(
            &Button {
                class: "btn-danger delete",
                attrs: " data-action=\"delete\"".into(),
                icon: Some("trash-can"),
                label: Some(t(i18n, "admin.customize.delete")),
                ..Default::default()
            }
            .render(),
        );
    }
    out.push_str("</div>");
    out
}

/// The translations' rows (ThemeTranslation each), alone for the
/// locale picker to swap in.
pub fn translation_rows(cx: &AdminCx, translations: &[Value], locale: &str) -> String {
    translations
        .iter()
        .map(|tr| {
            let setting = serde_json::json!({
                "value": tr["value"],
                "default": tr["default"],
                "textarea": true,
            });
            legacy_row(
                cx,
                &setting,
                &LegacyRow {
                    class: "theme-translation",
                    name: Some(s(&tr["key"])),
                    staff_log: None,
                    kind: Some("string"),
                    choices: None,
                    default_values: None,
                    attrs: format!(
                        " data-saver=\"theme-translation\" data-translation-key=\"{}\" data-locale=\"{}\"",
                        escape(s(&tr["key"])),
                        escape(locale)
                    ),
                },
            )
        })
        .collect()
}

/// templates/admin-customize-themes/edit.gjs with AdminThemeEditor for
/// the target's field; `type_path` is the route's `:type`.
pub fn edit(cx: &AdminCx, theme: &Value, type_path: &str, target: &str, field: &str) -> String {
    let i18n = cx.i18n;
    let tr = |k: &str| escape(&t(i18n, k));
    let id = theme["id"].as_i64().unwrap_or_default();
    let base = format!("{}/admin/customize/{type_path}/{id}", cx.base_path);
    let mut out = String::from(
        "<div class=\"admin-config-page\"><div class=\"current-style\"><div class=\"wrapper\"><div class=\"editor-container\">",
    );
    out.push_str(&format!(
        "<div class=\"editor-information\"><div class=\"editor-information__title\">{}<span class=\"editor-theme-name-wrapper\">{} <a class=\"editor-theme-name\" href=\"{base}\">{}</a></span></div><div class=\"editor-information__admin-actions\"><div class=\"d-toggle-switch\"><label class=\"d-toggle-switch__label\"><button aria-checked=\"false\" aria-labelledby=\"theme-editor-show-advanced-label\" class=\"d-toggle-switch__checkbox\" role=\"switch\" type=\"button\"></button><span class=\"d-toggle-switch__checkbox-slider\"></span></label><span class=\"d-toggle-switch__checkbox-label\" id=\"theme-editor-show-advanced-label\">{}</span></div>{}</div></div>",
        Button {
            class: "btn-default btn-small editor-back-button",
            icon: Some("chevron-left"),
            title: Some(t(i18n, "go_back")),
            ..Default::default()
        }
        .render(),
        tr("admin.customize.theme.edit_css_html"),
        escape(s(&theme["name"])),
        tr("admin.customize.theme.show_advanced"),
        Button {
            class: "btn-transparent theme-editor-maximize",
            icon: Some("discourse-expand"),
            title: Some(t(i18n, "admin.customize.theme.maximize_editor")),
            ..Default::default()
        }
        .render(),
    ));
    out.push_str(&editor_navs(cx, theme, type_path, target, field, false));
    let error = get_error(theme, target, field);
    if !error.is_empty() {
        out.push_str(&format!(
            "<pre class=\"field-error\">{}</pre>",
            escape(&error)
        ));
    }
    let title = t(i18n, &format!("admin.customize.theme.{field}.title"));
    out.push_str(&format!(
        "<div class=\"field-info\">{}</div>",
        escape(&title)
    ));
    let mut content = get_field(theme, target, field);
    if content.is_empty() && field == "js" {
        content = JS_DEFAULT_VALUE.to_string();
    }
    let mode = if field == "color_definitions" {
        "scss"
    } else if field == "js" {
        "javascript"
    } else if field.contains("scss") {
        "scss"
    } else {
        "html"
    };
    out.push_str(&crate::admin_code_view::ace_editor(cx, &content, mode, ""));
    out.push_str(&format!(
        "</div><div class=\"admin-footer\"><div class=\"status-actions\"><a class=\"preview-link\" href=\"{}/admin/themes/{id}/preview\" rel=\"noopener noreferrer\" target=\"_blank\" title=\"{}\">{}</a></div><div class=\"buttons\">{}</div></div></div></div></div>",
        cx.base_path,
        tr("admin.customize.explain_preview"),
        tr("admin.customize.preview"),
        Button {
            class: "btn-primary save-theme",
            label: Some(t(i18n, "admin.customize.save")),
            disabled: true,
            ..Default::default()
        }
        .render()
    ));
    out
}

/// The editor's target and field navs (DHorizontalOverflowNav), the
/// advanced ones shown with `advanced` or once edited.
pub fn editor_navs(
    cx: &AdminCx,
    theme: &Value,
    type_path: &str,
    target: &str,
    field: &str,
    advanced: bool,
) -> String {
    let i18n = cx.i18n;
    let id = theme["id"].as_i64().unwrap_or_default();
    let base = format!("{}/admin/customize/{type_path}/{id}", cx.base_path);
    let nav = |class: &str, items: &str| {
        format!(
            "<nav class=\"horizontal-overflow-nav\"><div class=\"d-overflow-controls --owned-scroller horizontal-overflow-nav__controls\"><ul class=\"nav-pills action-list {class}\" data-d-scroll-axis=\"horizontal\">{items}</ul></div></nav>"
        )
    };
    let link = |class: &str, title: Option<&str>, href: &str, inner: &str| {
        format!(
            "<li><a class=\"{class}\"{} href=\"{href}\">{inner}</a></li>",
            title
                .map(|t| format!(" title=\"{}\"", escape(t)))
                .unwrap_or_default()
        )
    };
    let mut out = String::new();
    let targets: Vec<&str> = ["common", "desktop", "mobile"]
        .into_iter()
        .filter(|t| advanced || has_edited(theme, t, None) || !ADVANCED_TARGETS.contains(t))
        .collect();
    if targets.len() > 1 {
        let mut items = String::new();
        for t_name in &targets {
            let mut class = if has_edited(theme, t_name, None) {
                "edited".to_string()
            } else {
                "blank".to_string()
            };
            if *t_name == target {
                class.insert_str(0, "active ");
            }
            let mut inner = String::new();
            if has_error(theme, t_name, None) {
                inner.push_str(&icon("triangle-exclamation", None));
            }
            match *t_name {
                "desktop" => inner.push_str(&icon("desktop", None)),
                "mobile" => inner.push_str(&icon("mobile-screen-button", None)),
                _ => {}
            }
            inner.push_str(&escape(&t(
                i18n,
                &format!("admin.customize.theme.{t_name}"),
            )));
            items.push_str(&link(
                &class,
                None,
                &format!("{base}/{t_name}/{field}/edit"),
                &inner,
            ));
        }
        out.push_str(&format!(
            "<div class=\"edit-main-nav admin-controls\">{}</div>",
            nav("target", &items)
        ));
    }
    let mut items = String::new();
    for name in field_names(target).unwrap_or_default() {
        let edited = has_edited(theme, target, Some(name));
        if !(advanced || edited || !ADVANCED_FIELDS.contains(name)) {
            continue;
        }
        let mut class = if edited {
            "edited".to_string()
        } else {
            "blank".to_string()
        };
        if *name == field {
            class.insert_str(0, "active ");
        }
        let mut inner = String::new();
        if has_error(theme, target, Some(name)) {
            inner.push_str(&icon("triangle-exclamation", None));
        }
        if name.contains("_tag") {
            inner.push_str(&icon("far-file-lines", None));
        }
        inner.push_str(&escape(&t(
            i18n,
            &format!("admin.customize.theme.{name}.text"),
        )));
        let title = t(i18n, &format!("admin.customize.theme.{name}.title"));
        items.push_str(&link(
            &class,
            Some(&title),
            &format!("{base}/{target}/{name}/edit"),
            &inner,
        ));
    }
    out.push_str(&format!(
        "<div class=\"admin-controls\">{}</div>",
        nav("fields", &items)
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_common_js_field_reads_its_initializer() {
        let theme = json!({"theme_fields": [
            {"target": "extra_js", "name": JS_FILENAME, "value": "x", "type_id": 6},
            {"target": "common", "name": "scss", "value": ".a{}", "type_id": 1, "error": "bad"},
        ]});
        assert_eq!(get_field(&theme, "common", "js"), "x");
        assert_eq!(get_error(&theme, "common", "scss"), "bad");
        assert!(has_edited(&theme, "common", None));
        assert!(!has_edited(&theme, "desktop", None));
        assert!(has_error(&theme, "common", Some("scss")));
        assert!(!has_error(&theme, "common", Some("js")));
    }
}
