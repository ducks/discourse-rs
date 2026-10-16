//! The admin email pages (templates/admin-email.gjs): the page header
//! with its tabs over the outlet, which is the email area's
//! AdminAreaSettings, the server settings with the test email form, the
//! summary preview, the advanced test, or the email templates' list.
//! static/js/admin-email.js does what the controllers do.

use serde_json::Value;

use crate::admin_ui::{Button, tr};
use crate::admin_view::{AdminCx, Crumb, PageHeader, t};
use crate::topic_list_view::{escape, icon};

/// The adminEmail route's tabs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Settings,
    ServerSettings,
    PreviewDigest,
    AdvancedTest,
    Templates,
}

impl Tab {
    /// The admin-contents classes the route gives (`admin-email` and the
    /// child route's name).
    pub fn classes(self) -> &'static str {
        match self {
            Tab::Settings => "admin-email settings",
            Tab::ServerSettings => "admin-email server-settings",
            Tab::PreviewDigest => "admin-email preview-digest",
            Tab::AdvancedTest => "admin-email advanced-test",
            Tab::Templates => "admin-email admin-email-templates",
        }
    }
}

/// The page: DPageHeader with its tabs, then the outlet in the
/// admin container.
pub fn render(cx: &AdminCx, tab: Tab, content: &str) -> String {
    let title = t(cx.i18n, "admin.config.email.title");
    let tabs = crate::admin_ui::tabs(
        cx,
        &[
            ("", "/admin/email", "settings", tab == Tab::Settings),
            (
                "",
                "/admin/email/server-settings",
                "admin.config.email.sub_pages.server_settings.title",
                tab == Tab::ServerSettings,
            ),
            (
                "",
                "/admin/email/preview-digest",
                "admin.config.email.sub_pages.preview_summary.title",
                tab == Tab::PreviewDigest,
            ),
            (
                "",
                "/admin/email/advanced-test",
                "admin.config.email.sub_pages.advanced_test.title",
                tab == Tab::AdvancedTest,
            ),
            (
                "",
                "/admin/email/templates",
                "admin.config.email.sub_pages.templates.title",
                tab == Tab::Templates,
            ),
        ],
    );
    let header = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path: "/admin/email",
            },
        ],
        title,
        actions: String::new(),
        description: Some(escape(&t(cx.i18n, "admin.config.email.header_description"))),
        learn_more_url: None,
        tabs: Some(tabs),
    }
    .render(cx);
    format!("{header}<div class=\"admin-container\">{content}</div>")
}

/// settings.gjs: the email area's AdminAreaSettings (drawn by the
/// caller) in the main area.
pub fn settings(area_settings: &str) -> String {
    format!("<div class=\"admin-config-page__main-area\">{area_settings}</div>")
}

/// DTextField with its placeholder, for the test addresses.
fn address_field(cx: &AdminCx) -> String {
    format!(
        "<input placeholder=\"{}\" type=\"text\">",
        escape(&t(cx.i18n, "admin.email.test_email_address"))
    )
}

/// server-settings.gjs: the delivery settings' table and the test email
/// form. The delivery method's cell reads the controller's own
/// `delivery_method`, which nothing sets: it stays empty, as the
/// reference draws it.
pub fn server_settings(cx: &AdminCx, doc: &Value) -> String {
    let mut out = format!(
        "<table class=\"table\"><tbody><tr><th>{}</th><td></td></tr>",
        escape(&t(cx.i18n, "admin.email.delivery_method"))
    );
    for s in doc["settings"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "<tr><th style=\"width: 25%\">{}</th><td>{}</td></tr>",
            escape(s["name"].as_str().unwrap_or_default()),
            escape(&display(&s["value"]))
        ));
    }
    out.push_str(&format!(
        "</tbody></table><form><div class=\"admin-controls\"><div class=\"controls\"><div class=\"inline-form\">{}{}</div></div></div></form>",
        address_field(cx),
        Button {
            class: "btn-primary",
            label: tr(cx, "admin.email.send_test"),
            disabled: true,
            ..Default::default()
        }
        .render()
        .replacen("type=\"button\"", "type=\"submit\"", 1)
    ));
    out
}

/// A setting's value as Ember interpolates it: strings as they are,
/// arrays joined with commas, null as nothing.
fn display(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().map(display).collect::<Vec<_>>().join(","),
        other => other.to_string(),
    }
}

/// DPageSubheader with only its description.
fn description(cx: &AdminCx, key: &str) -> String {
    crate::admin_ui::subheader("", "", Some(&escape(&t(cx.i18n, key))))
}

/// advanced-test.gjs before a run: the textarea and Run test; the
/// results are the script's.
pub fn advanced_test(cx: &AdminCx) -> String {
    format!(
        "{}<div class=\"email-advanced-test\"><label for=\"email\">{}</label><textarea class=\"email-body\" name=\"email\"></textarea>{}</div><div class=\"loading-container\"></div>",
        description(
            cx,
            "admin.config.email.sub_pages.advanced_test.header_description"
        ),
        escape(&t(cx.i18n, "admin.email.advanced_test.email")),
        Button {
            class: "btn-default",
            label: tr(cx, "admin.email.advanced_test.run"),
            ..Default::default()
        }
        .render()
    )
}

/// What the summary preview shows as the route loads it.
pub struct Preview<'a> {
    pub username: &'a str,
    /// `oneWeekAgo()`, YYYY-MM-DD.
    pub last_seen: &'a str,
    pub html_content: &'a str,
    pub text_content: &'a str,
    pub mobile: bool,
}

/// preview-digest.gjs: the date, the user and Refresh, the html and text
/// toggle, the send form and the preview (html first).
pub fn preview_digest(cx: &AdminCx, p: &Preview) -> String {
    let mut out = description(
        cx,
        "admin.config.email.sub_pages.preview_summary.header_description",
    );
    // DatePickerPast: Pikaday on the desktop (admin-email.js sets the
    // date), the browser's date input in the mobile view.
    out.push_str(&format!(
        "<div class=\"admin-controls email-preview\"><div class=\"controls\"><div class=\"inline-form\"><label for=\"last-seen\">{}</label><div class=\"date-picker-wrapper\"><input autocomplete=\"off\" class=\"date-picker\" id=\"last-seen\" placeholder=\"{}\" type=\"{}\" data-value=\"{}\" data-pikaday=\"{}\"></div><label>{}:</label>",
        escape(&t(cx.i18n, "admin.email.last_seen_user")),
        escape(&t(cx.i18n, "dates.placeholder")),
        if p.mobile { "date" } else { "text" },
        escape(p.last_seen),
        crate::assets::url(cx.base_path, "pikaday.js"),
        escape(&t(cx.i18n, "admin.email.user"))
    ));
    // EmailGroupUserChooser, maximum one, the viewer chosen.
    out.push_str(&format!(
        "<details class=\"select-kit multi-select user-chooser email-group-user-chooser has-selection\" data-value=\"{u}\" data-current=\"{u}\" data-placeholder=\"{}\" data-caret=\"angle-down\"{}><summary class=\"select-kit-header multi-select-header\" data-name=\"{u}\" tabindex=\"0\"><div class=\"select-kit-header-wrapper\"><span class=\"formatted-selection\">{u}</span>{}</div></summary><div class=\"select-kit-body\"></div></details>",
        escape(&t(cx.i18n, "select_kit.filter_placeholder")),
        crate::admin_ui::chooser_labels(cx),
        icon("angle-down", Some("angle-icon")),
        u = escape(p.username)
    ));
    out.push_str(
        &Button {
            class: "btn-primary digest-refresh-button",
            label: tr(cx, "admin.email.refresh"),
            ..Default::default()
        }
        .render(),
    );
    out.push_str(&format!(
        "<div class=\"toggle\"><label>{}</label>{}</div></div></div></div>",
        escape(&t(cx.i18n, "admin.email.format")),
        toggle(cx, true)
    ));
    out.push_str("<div class=\"loading-container\"><div class=\"email-preview-digest\">");
    if !p.html_content.is_empty() {
        out.push_str(&send_form(cx));
    }
    out.push_str(&format!(
        "<div class=\"preview-output\">{}</div></div></div>",
        output(cx, true, p.html_content, p.text_content)
    ));
    out.push_str(&format!(
        "<template class=\"email-preview-text\"><pre>{}</pre></template>",
        escape(p.text_content)
    ));
    out
}

/// The format toggle: the shown format as text, the other as its link.
fn toggle(cx: &AdminCx, show_html: bool) -> String {
    let html = escape(&t(cx.i18n, "admin.email.html"));
    let text = escape(&t(cx.i18n, "admin.email.text"));
    if show_html {
        format!("<span>{html}</span> | <a class=\"show-text-link\" href>{text}</a>")
    } else {
        format!("<a class=\"show-html-link\" href>{html}</a> | <span>{text}</span>")
    }
}

/// The send form, shown while there is html to send.
fn send_form(cx: &AdminCx) -> String {
    format!(
        "<div class=\"controls\"><div class=\"inline-form\"><label>{}</label>{}{}</div></div>",
        escape(&t(cx.i18n, "admin.email.send_digest_label")),
        address_field(cx),
        Button {
            class: "btn-default",
            label: tr(cx, "admin.email.send_digest"),
            disabled: true,
            ..Default::default()
        }
        .render()
    )
}

/// The preview: the html in an iframe (or the no result note), or the
/// text.
fn output(cx: &AdminCx, show_html: bool, html: &str, text: &str) -> String {
    if !show_html {
        return format!("<pre>{}</pre>", escape(text));
    }
    if html.is_empty() {
        return format!("<p>{}</p>", escape(&t(cx.i18n, "admin.email.no_result")));
    }
    format!(
        "<iframe srcdoc=\"{}\" title=\"{}\"></iframe>",
        escape(html),
        escape(&t(cx.i18n, "admin.email.html_preview"))
    )
}

/// The ResetButton of DFilterControls: the icon alone, or labelled in
/// the no results note.
fn reset_button(cx: &AdminCx, disabled: bool, label: bool) -> String {
    Button {
        class: "btn-default d-filter-controls__reset",
        icon: Some("arrow-rotate-left"),
        label: if label {
            tr(cx, "filter_controls.reset")
        } else {
            None
        },
        title: tr(cx, "filter_controls.reset"),
        disabled,
        attrs: format!(
            " aria-label=\"{}\"",
            escape(&t(cx.i18n, "filter_controls.reset"))
        ),
        ..Default::default()
    }
    .render()
}

/// admin-email-templates/index.gjs: DFilterControls over the templates
/// sorted by title, the Only show overridden toggle, and the table; the
/// URL's `filter` and `overridden` applied as Ember applies them.
pub fn templates(cx: &AdminCx, doc: &Value, filter: &str, overridden_only: bool) -> String {
    let mut templates: Vec<&Value> = doc["email_templates"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    templates.sort_by(|a, b| {
        crate::preferences_view::locale_compare(
            a["title"].as_str().unwrap_or_default(),
            b["title"].as_str().unwrap_or_default(),
        )
    });
    if overridden_only {
        templates.retain(|t| t["can_revert"].as_bool().unwrap_or(false));
    }
    let terms: Vec<String> = filter
        .split(',')
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    let shown: Vec<&Value> = templates
        .iter()
        .copied()
        .filter(|tpl| {
            terms.is_empty()
                || terms.iter().any(|term| {
                    ["title", "id"].iter().any(|k| {
                        tpl[*k]
                            .as_str()
                            .is_some_and(|v| v.to_lowercase().contains(term))
                    })
                })
        })
        .collect();
    let mut out = format!(
        "<div class=\"d-filter-controls\"><div class=\"d-filter-controls__inputs\"><div class=\"filter-input-container\">{}<input autocapitalize=\"none\" autocomplete=\"off\" autocorrect=\"off\" class=\"filter-input d-filter-controls__input\" placeholder=\"{}\" type=\"text\" value=\"{}\"></div></div>{}</div>",
        icon("magnifying-glass", Some("-left")),
        escape(&t(
            cx.i18n,
            "admin.customize.email_templates.search_templates"
        )),
        escape(filter),
        reset_button(cx, terms.is_empty() && filter.is_empty(), false)
    );
    out.push_str(&format!(
        "<label class=\"checkbox-label\"><input{} id=\"toggle-overridden\" type=\"checkbox\"> {}</label>",
        if overridden_only { " checked" } else { "" },
        escape(&t(cx.i18n, "admin.site_text.show_overriden"))
    ));
    if shown.is_empty() {
        if !filter.is_empty() {
            out.push_str(&format!(
                "<div class=\"d-filter-controls__no-results\"><p>{}</p>{}</div>",
                escape(&t(
                    cx.i18n,
                    "admin.customize.email_templates.no_templates_found"
                )),
                reset_button(cx, false, true)
            ));
        }
        return out;
    }
    out.push_str(&format!(
        "<table class=\"d-table email-templates-list\"><thead class=\"d-table__header\"><tr class=\"d-table__row\"><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\"></th></tr></thead><tbody class=\"d-table__body\">",
        escape(&t(cx.i18n, "admin.customize.email_templates.title"))
    ));
    let edit = tr(cx, "admin.customize.email_templates.edit");
    for tpl in shown {
        let id = tpl["id"].as_str().unwrap_or_default();
        let path = format!("/admin/email/templates/{id}");
        out.push_str(&format!(
            "<tr class=\"d-table__row email-templates-list__row{}\" data-template-id=\"{}\"><td class=\"d-table__cell --overview\"><a class=\"d-table__overview-name admin-email-templates__name\" href=\"{}{}\">{}</a></td><td class=\"d-table__cell --controls\"><div class=\"d-table__cell-actions\">{}</div></td></tr>",
            if tpl["can_revert"].as_bool().unwrap_or(false) {
                " overridden"
            } else {
                ""
            },
            escape(id),
            cx.base_path,
            escape(&path),
            escape(tpl["title"].as_str().unwrap_or_default()),
            Button {
                class: "btn-default admin-email-templates__edit-button",
                label: edit.clone(),
                href: Some(&path),
                ..Default::default()
            }
            .render()
        ));
    }
    out.push_str("</tbody></table>");
    out
}

/// The strings admin-email.js shows.
pub fn strings(cx: &AdminCx) -> String {
    crate::admin_ui::strings(
        cx,
        "admin-email-strings",
        &[
            "admin.email.sending_test",
            "admin.email.error",
            "admin.email.test_error",
            "admin.email.advanced_test.text",
            "admin.email.advanced_test.elided",
            "admin.email.html",
            "admin.email.text",
            "admin.email.no_result",
            "admin.email.html_preview",
            "admin.email.sent_test",
            "admin.customize.email_templates.no_templates_found",
            "filter_controls.reset",
            "dates.previous_month",
            "dates.next_month",
            "generic_error",
        ],
    )
}
