//! The site texts pages (adminSiteText, 90g7) as Ember draws them, from
//! Admin::SiteTextsController's documents.

use serde_json::Value;

use crate::admin_view::{AdminCx, Crumb, PageHeader, t, t_count, t_with};
use crate::topic_list_view::{escape, icon};

/// The index's query params, as the controller resolves them.
pub struct Filters {
    pub q: String,
    pub locale: String,
    pub theme_id: Option<i64>,
    pub overridden: bool,
    pub outdated: bool,
    pub untranslated: bool,
    pub only_selected_locale: bool,
}

impl Filters {
    /// activeFilterCount
    fn active(&self) -> usize {
        [
            self.theme_id.is_some(),
            self.overridden,
            self.outdated,
            self.only_selected_locale,
            self.untranslated,
        ]
        .iter()
        .filter(|v| **v)
        .count()
    }
}

fn crumbs(cx: &AdminCx) -> Vec<Crumb<'static>> {
    vec![
        Crumb {
            label: t(cx.i18n, "admin_title"),
            path: "/admin",
        },
        Crumb {
            label: t(cx.i18n, "admin.config.site_texts.title"),
            path: "/admin/customize/site_texts",
        },
    ]
}

/// A ComboBox closed on its choice (the language, the theme): the name,
/// its value, or the none label with no value.
fn combo(cx: &AdminCx, class: &str, name: &str, value: Option<&str>, attrs: &str) -> String {
    let label = escape(&t_with(cx.i18n, "select_kit.filter_by", &[("name", name)]));
    let value_attr = value
        .map(|v| format!(" data-value=\"{}\"", escape(v)))
        .unwrap_or_default();
    format!(
        "<details class=\"select-kit single-select combobox combo-box{} {class}\"{attrs}><summary aria-label=\"{label}\" class=\"select-kit-header single-select-header combo-box-header\" data-name=\"{n}\"{value_attr} name=\"{label}\" tabindex=\"0\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"{n}\"{value_attr} title=\"{n}\"><span class=\"name\">{n}</span></div>{}</div></summary><div class=\"select-kit-body\"></div></details>",
        if value.is_some() {
            " has-selection"
        } else {
            ""
        },
        icon("angle-down", Some("angle-icon")),
        n = escape(name),
    )
}

/// templates/admin-site-text/index.gjs: the header, DFilterControls
/// (its drawer open while a filter is active) and the results' page.
pub fn index(
    cx: &AdminCx,
    doc: &Value,
    filters: &Filters,
    locales: &[(String, String)],
    show_untranslated: bool,
) -> String {
    let tr = |k: &str| escape(&t(cx.i18n, k));
    let actions = crate::admin_ui::actions(
        cx,
        "d-page-header",
        &[crate::admin_ui::Action::Default(crate::admin_ui::Button {
            class: "reseed",
            icon: Some("arrows-rotate"),
            label: crate::admin_ui::tr(cx, "admin.reseed.action.label"),
            title: crate::admin_ui::tr(cx, "admin.reseed.action.title"),
            ..Default::default()
        })],
    );
    let header = PageHeader {
        crumbs: crumbs(cx),
        title: t(cx.i18n, "admin.config.site_texts.title"),
        actions,
        description: Some(tr("admin.config.site_texts.header_description")),
        learn_more_url: None,
        tabs: None,
    }
    .render(cx);
    let locale_name = locales
        .iter()
        .find(|(v, _)| *v == filters.locale)
        .map(|(_, n)| n.as_str())
        .unwrap_or(filters.locale.as_str());
    let active = filters.active();
    let toggle = crate::admin_ui::Button {
        class: "btn-default d-filter-controls__toggle-filters",
        icon: Some(if active > 0 {
            "filter-circle-xmark"
        } else {
            "filter"
        }),
        label: Some(if active > 0 {
            t_count(
                cx.i18n,
                "admin.site_text.filters_active",
                active as i64,
                &[],
            )
        } else {
            t(cx.i18n, "admin.site_text.filters")
        }),
        title: crate::admin_ui::tr(cx, "filter_controls.toggle"),
        attrs: format!(" aria-expanded=\"{}\"", active > 0),
        ..Default::default()
    }
    .render();
    let mut out = format!(
        "{header}<div class=\"search-area\"><div class=\"d-filter-controls --dropdowns-in-filter-drawer\"><div class=\"d-filter-controls__inputs\"><div class=\"site-texts__search\"><label for=\"site-text-search\">{}</label><div class=\"filter-input-container\">{}<input autocapitalize=\"none\" autocomplete=\"off\" autocorrect=\"off\" class=\"filter-input no-blur site-text-search\" id=\"site-text-search\" placeholder=\"{}\" type=\"text\" value=\"{}\"></div></div><div class=\"site-texts__language\"><label id=\"site-text-language-label\">{}</label>{}</div>{toggle}</div>",
        tr("admin.site_text.search_label"),
        icon("magnifying-glass", Some("-left")),
        tr("admin.site_text.search"),
        escape(&filters.q),
        tr("admin.site_text.locale"),
        combo(
            cx,
            "locale-search",
            locale_name,
            Some(&filters.locale),
            " data-site-text-locale=\"\""
        ),
    );
    if active > 0 {
        out.push_str(&dropdowns(cx, doc, filters, show_untranslated));
    }
    out.push_str("</div></div>");
    if doc["extras"]["recommended"].as_bool() == Some(true) {
        out.push_str(&format!(
            "<p class=\"site-texts__recommended\">{}</p>",
            tr("admin.site_text.recommended")
        ));
    }
    out.push_str(&results(cx, doc, filters));
    // The filters' drawer, kept for the script to open.
    if active == 0 {
        out.push_str(&format!(
            "<template class=\"site-texts-dropdowns\">{}</template>",
            dropdowns(cx, doc, filters, show_untranslated)
        ));
    }
    format!("<div class=\"site-texts\">{out}</div>")
}

/// The additional filters: the checkboxes, Reset, the theme chooser.
fn dropdowns(cx: &AdminCx, doc: &Value, filters: &Filters, show_untranslated: bool) -> String {
    let tr = |k: &str| escape(&t(cx.i18n, k));
    let checkbox = |id: &str, on: bool, key: &str| {
        format!(
            "<label class=\"checkbox-label\"><input{} id=\"{id}\" type=\"checkbox\"> {}</label>",
            if on { " checked=\"\"" } else { "" },
            tr(key)
        )
    };
    let mut boxes = checkbox(
        "toggle-overridden",
        filters.overridden,
        "admin.site_text.show_overriden",
    );
    boxes.push_str(&checkbox(
        "toggle-outdated",
        filters.outdated,
        "admin.site_text.show_outdated",
    ));
    boxes.push_str(&checkbox(
        "toggle-only-locale",
        filters.only_selected_locale,
        "admin.site_text.only_show_selected_locale",
    ));
    if show_untranslated {
        boxes.push_str(&checkbox(
            "toggle-untranslated",
            filters.untranslated,
            "admin.site_text.show_untranslated",
        ));
    }
    let themes = doc["extras"]["themes"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let theme = filters.theme_id.and_then(|id| {
        themes
            .iter()
            .find(|th| th["id"].as_i64() == Some(id))
            .map(|th| (id, th["name"].as_str().unwrap_or_default().to_string()))
    });
    let (theme_name, theme_value) = match &theme {
        Some((id, name)) => (name.clone(), Some(id.to_string())),
        None => (t(cx.i18n, "admin.site_text.all_themes"), None),
    };
    let rows: Vec<Value> = themes
        .iter()
        .map(|th| serde_json::json!({ "id": th["id"], "name": th["name"], "enabled": th["enabled"] }))
        .collect();
    format!(
        "<div class=\"d-filter-controls__dropdowns\"><div class=\"filter-options\">{boxes}</div>{}<div class=\"site-texts__theme\"><label>{}</label>{}</div></div>",
        crate::admin_ui::Button {
            class: "btn-flat site-texts__reset-filters",
            label: crate::admin_ui::tr(cx, "filter_controls.reset"),
            disabled: filters.active() == 0,
            ..Default::default()
        }
        .render(),
        tr("admin.site_text.theme"),
        combo(
            cx,
            "theme-search",
            &theme_name,
            theme_value.as_deref(),
            &format!(
                " data-site-text-themes=\"{}\" data-none=\"{}\"",
                escape(&Value::Array(rows).to_string()),
                tr("admin.site_text.all_themes")
            )
        )
    )
}

/// DLoadMore's list: SiteTextSummary for each text, or no results; the
/// next page's URL on the sentinel while there are more.
pub fn results(cx: &AdminCx, doc: &Value, filters: &Filters) -> String {
    let texts = doc["site_texts"].as_array().cloned().unwrap_or_default();
    let mut out = String::from("<div class=\"site-text-list\">");
    if texts.is_empty() {
        out.push_str(&escape(&t(cx.i18n, "admin.site_text.no_results")));
    }
    for text in &texts {
        out.push_str(&summary(cx, text));
    }
    out.push_str("<div class=\"loading-container\"></div><div aria-hidden=\"true\" class=\"load-more-sentinel\"");
    if doc["extras"]["has_more"].as_bool() == Some(true) {
        out.push_str(" data-has-more=\"\"");
    }
    let _ = filters;
    out.push_str("></div></div>");
    out
}

/// SiteTextSummary
pub fn summary(cx: &AdminCx, text: &Value) -> String {
    let overridden = text["overridden"].as_bool() == Some(true);
    let status = match text["status"].as_str() {
        Some("outdated") => Some(t(cx.i18n, "admin.site_text.status.outdated")),
        Some("invalid_interpolation_keys") => Some(t(cx.i18n, "admin.site_text.status.invalid")),
        _ if overridden => Some(t(cx.i18n, "admin.site_text.status.customized")),
        _ => None,
    };
    let id = text["id"].as_str().unwrap_or_default();
    format!(
        "<div class=\"site-text{}\" data-site-text-id=\"{}\"><div class=\"site-text__details\"><code class=\"site-text-id\">{}</code>{}</div><div class=\"site-text-value\">{}</div>{}</div>",
        if overridden { " overridden" } else { "" },
        escape(id),
        escape(id),
        status
            .map(|s| format!(
                "<span class=\"site-text__status\" data-status=\"{}\">{}</span>",
                escape(text["status"].as_str().unwrap_or_default()),
                escape(&s)
            ))
            .unwrap_or_default(),
        escape(text["value"].as_str().unwrap_or_default()),
        crate::admin_ui::Button {
            class: "btn-default site-text-edit",
            label: crate::admin_ui::tr(cx, "admin.site_text.edit"),
            ..Default::default()
        }
        .render()
    )
}

/// interpolationKeysWithStatus: the known keys (used or not) and the
/// unknown ones the content uses.
pub fn interpolation_keys(cx: &AdminCx, content: &str, keys: Option<&Vec<Value>>) -> String {
    let Some(keys) = keys else {
        return String::new();
    };
    let keys: Vec<&str> = keys.iter().filter_map(Value::as_str).collect();
    let mut used = Vec::new();
    let mut invalid: Vec<String> = Vec::new();
    let re = regex::Regex::new(r"%\{(\w+)\}").expect("a valid pattern");
    for c in re.captures_iter(content) {
        let k = &c[1];
        if keys.contains(&k) {
            used.push(k.to_string());
        } else if !invalid.iter().any(|i| i == k) {
            invalid.push(k.to_string());
        }
    }
    if keys.is_empty() && invalid.is_empty() {
        return String::new();
    }
    let mut out = String::from("<div class=\"interpolation-keys\">");
    for k in &keys {
        let is_used = used.iter().any(|u| u == k);
        out.push_str(&format!(
            "<button class=\"interpolation-keys__pill{}\" title=\"{}\" type=\"button\">{}</button>",
            if is_used { " is-used" } else { "" },
            escape(&t(
                cx.i18n,
                if is_used {
                    "admin.site_text.interpolation_key_used"
                } else {
                    "admin.site_text.interpolation_key_insert"
                }
            )),
            escape(k)
        ));
    }
    for k in &invalid {
        out.push_str(&format!(
            "<span class=\"interpolation-keys__pill is-invalid\" title=\"{}\">{}</span>",
            escape(&t(cx.i18n, "admin.site_text.interpolation_key_invalid")),
            escape(k)
        ));
    }
    out.push_str("</div>");
    out
}

/// templates/admin-site-text/edit.gjs from the site text's document.
pub fn edit(
    cx: &AdminCx,
    text: &Value,
    locale: &str,
    locale_name: &str,
    theme_id: Option<&str>,
) -> String {
    let tr = |k: &str| escape(&t(cx.i18n, k));
    let header = PageHeader {
        crumbs: crumbs(cx),
        title: t(cx.i18n, "admin.site_text.edit_title"),
        actions: String::new(),
        description: None,
        learn_more_url: None,
        tabs: None,
    }
    .render(cx);
    let mut back = format!(
        "{}/admin/customize/site_texts?locale={}",
        cx.base_path,
        escape(locale)
    );
    if let Some(theme) = theme_id {
        back.push_str(&format!("&amp;theme_id={}", escape(theme)));
    }
    let status = text["status"].as_str().unwrap_or_default();
    let value = text["value"].as_str().unwrap_or_default();
    let mut out = format!(
        "{header}<a class=\"go-back\" href=\"{back}\">{} {}</a><code class=\"edit-site-text__key\">{}</code>",
        icon("arrow-left", None),
        tr("admin.site_text.go_back"),
        escape(text["id"].as_str().unwrap_or_default())
    );
    if status == "invalid_interpolation_keys" {
        out.push_str(&format!(
            "<div class=\"outdated\" role=\"status\"><h4>{}</h4><p>{}</p></div>",
            tr("admin.site_text.invalid_title"),
            tr("admin.site_text.invalid_description")
        ));
    }
    if status == "outdated" {
        out.push_str(&format!(
            "<div class=\"outdated\"><h4>{}</h4><p>{}</p><h5>{}</h5><p>{}</p><h5>{}</h5><p>{}</p>{}</div>",
            tr("admin.site_text.outdated.title"),
            tr("admin.site_text.outdated.description"),
            tr("admin.site_text.outdated.old_default"),
            escape(text["old_default"].as_str().unwrap_or_default()),
            tr("admin.site_text.outdated.new_default"),
            escape(text["new_default"].as_str().unwrap_or_default()),
            crate::admin_ui::Button {
                class: "btn-default dismiss-outdated",
                label: crate::admin_ui::tr(cx, "admin.site_text.outdated.dismiss"),
                ..Default::default()
            }
            .render()
        ));
    } else {
        // defaultText
        let default = match &text["new_default"] {
            Value::String(s) => Some(s.as_str()),
            _ if text["overridden"].as_bool() != Some(true)
                && text["can_revert"].as_bool() != Some(true) =>
            {
                Some(value)
            }
            _ => None,
        };
        if let Some(default) = default.filter(|d| !d.is_empty()) {
            out.push_str(&format!(
                "<div class=\"edit-site-text__default\"><h2>{}</h2><p>{}</p></div>",
                tr("admin.site_text.default_text"),
                escape(default)
            ));
        }
    }
    out.push_str(&format!(
        "<label class=\"edit-site-text__value-label\" for=\"site-text-value\">{}</label><div class=\"expanding-text-area\"><textarea class=\"--expandable site-text-value\" id=\"site-text-value\" rows=\"5\">\n{}</textarea></div>",
        escape(&t_with(
            cx.i18n,
            "admin.site_text.value_label",
            &[("language", locale_name)]
        )),
        escape(value)
    ));
    out.push_str(&interpolation_keys(
        cx,
        value,
        text["interpolation_keys"].as_array(),
    ));
    let revert = if text["can_revert"].as_bool() == Some(true) {
        crate::admin_ui::Button {
            class: "revert-site-text",
            label: crate::admin_ui::tr(cx, "admin.site_text.revert"),
            ..Default::default()
        }
        .render()
    } else {
        String::new()
    };
    out.push_str(&crate::admin_customize_kit::save_controls(cx, &revert));
    format!("<div class=\"site-texts\"><div class=\"edit-site-text\">{out}</div></div>")
}
