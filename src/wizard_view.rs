//! The setup wizard's page as Ember draws it once Wizard.load came back:
//! templates/wizard.gjs (SiteSkeleton behind the step), wizard/step.gjs
//! and static/wizard's WizardStep, WizardField and its text, radio and
//! dropdown fields, from WizardSerializer's JSON. static/js/wizard.js
//! runs the fields and Jump in.

use serde_json::Value;

use crate::preferences_view::{Combo, combo};
use crate::topic_list_view::{ListContext, escape, icon};

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// Ember's dasherize for the ids the wizard has.
fn dasherize(v: &str) -> String {
    v.replace('_', "-")
}

/// The step `step` of the wizard (its Jump in label is the wizard
/// bundle's, wizard_js), with the locales the default_locale
/// dropdown lists (siteSettings.available_locales: value, name).
pub fn render(cx: &ListContext, step: &Value, locales: &[(String, String)]) -> String {
    let mut out = String::from("<div id=\"wizard-main\">");
    out.push_str(&site_skeleton());
    let id = s(&step["id"]);
    out.push_str(&format!(
        "<div class=\"wizard-container__step {}\"><div class=\"wizard-container\"><div class=\"wizard-container__step-contents\"><div class=\"wizard-container__step-header\"><h1 class=\"wizard-container__step-title\">{}</h1></div><div class=\"wizard-container__step-container\">",
        escape(id),
        escape(s(&step["title"]))
    ));
    let fields = step["fields"].as_array().cloned().unwrap_or_default();
    if !fields.is_empty() {
        out.push_str(
            "<div class=\"wizard-container__step-form\"><div class=\"wizard-container__fields\">",
        );
        for f in &fields {
            out.push_str(&field(cx, f, locales));
        }
        out.push_str("</div></div>");
    }
    out.push_str(&format!(
        "</div></div><div class=\"wizard-container__step-footer\"><button class=\"wizard-container__button jump-in btn btn-primary\" type=\"button\" data-step-id=\"{}\">{}</button></div></div></div></div>",
        escape(id),
        escape(cx.i18n.t("wizard_js.wizard.jump_in").unwrap_or("wizard.jump_in"))
    ));
    out
}

/// WizardField: its label, required mark and description, the type's
/// input, and the extra description.
fn field(cx: &ListContext, f: &Value, locales: &[(String, String)]) -> String {
    let id = s(&f["id"]);
    let kind = s(&f["type"]);
    let mut out = format!(
        "<div class=\"wizard-container__field {k}-field {k}-{}{}\" data-field-id=\"{}\" data-field-type=\"{k}\"{}>",
        dasherize(id),
        if f["disabled"] == true {
            " disabled"
        } else {
            ""
        },
        escape(id),
        if f["required"] == true {
            " data-required"
        } else {
            ""
        },
        k = dasherize(kind),
    );
    let label = s(&f["label"]);
    let description = s(&f["description"]);
    if !label.is_empty() || !description.is_empty() {
        out.push_str(&format!(
            "<label for=\"{}\"><span class=\"wizard-container__label\">{}</span>",
            escape(id),
            escape(label)
        ));
        if f["required"] == true {
            out.push_str("<span class=\"wizard-container__label required\">*</span>");
        }
        if !description.is_empty() {
            out.push_str(&format!(
                "<div class=\"wizard-container__description\">{description}</div>"
            ));
        }
        out.push_str("</label>");
    }
    out.push_str("<div class=\"wizard-container__input\">");
    match kind {
        "text" => out.push_str(&format!(
            "<div><input id=\"{}\" class=\"ember-text-field ember-view wizard-container__text-input\" placeholder=\"{}\" tabindex=\"9\" type=\"text\" value=\"{}\"></div>",
            escape(id),
            escape(s(&f["placeholder"])),
            escape(s(&f["value"]))
        )),
        "radio" => out.push_str(&radio(f)),
        _ => {
            // The dropdown's ComboBox: default_locale lists the site's
            // locales in place of the field's choices.
            let rows: Vec<(String, String)> = if id == "default_locale" {
                locales.to_vec()
            } else {
                f["choices"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| (s(&c["id"]).to_string(), s(&c["label"]).to_string()))
                    .collect()
            };
            out.push_str(&combo(
                cx,
                &Combo {
                    id: None,
                    component_class: "",
                    class: "wizard-container__dropdown",
                    rows,
                    value: f["value"].as_str().map(str::to_string),
                    none: None,
                    key: None,
                    filterable: false,
                },
            ));
        }
    }
    out.push_str("</div>");
    let extra = s(&f["extra_description"]);
    if !extra.is_empty() {
        out.push_str(&format!(
            "<div class=\"wizard-container__description extra\">{extra}</div>"
        ));
    }
    out.push_str("</div>");
    out
}

/// The radio field's choices, the field's value selected.
fn radio(f: &Value) -> String {
    let id = s(&f["id"]);
    let value = s(&f["value"]);
    let mut out = String::from("<div class=\"wizard-container__radio-choices\">");
    for c in f["choices"].as_array().into_iter().flatten() {
        let choice = s(&c["id"]);
        let selected = choice == value;
        out.push_str(&format!(
            "<div class=\"wizard-container__radio-choice{}\" data-choice-id=\"{c_e}\"><label class=\"wizard-container__label\"><input{} class=\"wizard-container__radio\"{} name=\"{}\" type=\"radio\" value=\"{c_e}\"><span class=\"wizard-container__radio-label\"><span>{}</span></span></label></div>",
            if selected { " --selected" } else { "" },
            if selected { " checked" } else { "" },
            if c["disabled"] == true { " disabled" } else { "" },
            escape(id),
            escape(s(&c["label"])),
            c_e = escape(choice),
        ));
    }
    out.push_str("</div>");
    out
}

/// SiteSkeleton: the inert placeholder site behind the wizard.
fn site_skeleton() -> String {
    let bar = |m: &str| format!("<span class=\"site-skeleton__bar {m}\"></span>");
    let nav = |name: &str, m: &str, active: bool| {
        format!(
            "<div class=\"site-skeleton__nav-item{}\"><span class=\"site-skeleton__nav-icon\">{}</span>{}</div>",
            if active { " --active" } else { "" },
            icon(name, None),
            bar(m)
        )
    };
    let category = |lead: String, m: &str| {
        format!(
            "<div class=\"site-skeleton__category\">{lead}{}</div>",
            bar(m)
        )
    };
    let dot = |m: &str| format!("<span class=\"site-skeleton__category-dot {m}\"></span>");
    let channel = |m: &str| format!("<span class=\"site-skeleton__channel {m}\"></span>");
    let nav_icon = |name: &str| {
        format!(
            "<span class=\"site-skeleton__nav-icon\">{}</span>",
            icon(name, None)
        )
    };
    let title = format!(
        "<div class=\"site-skeleton__sidebar-title\">{}</div>",
        bar("--title")
    );
    let mut out = format!(
        "<div aria-hidden=\"true\" class=\"site-skeleton\"><div class=\"site-skeleton__header\"><div class=\"site-skeleton__brand\"><span class=\"site-skeleton__menu\">{}</span>{}</div><span class=\"site-skeleton__chat\">{}</span><span class=\"site-skeleton__avatar\"></span></div>",
        icon("bars", None),
        bar("--brand"),
        icon("comment", None)
    );
    out.push_str("<div class=\"site-skeleton__body\"><aside class=\"site-skeleton__sidebar\"><div class=\"site-skeleton__new-topic\"></div>");
    out.push_str(&nav("layer-group", "--accent --md", true));
    out.push_str(&nav("user", "--lg", false));
    out.push_str(&nav("inbox", "--md", false));
    out.push_str(&nav("flag", "--sm", false));
    out.push_str(&nav("wrench", "--md", false));
    out.push_str(&nav("paper-plane", "--sm", false));
    out.push_str(&format!(
        "<div class=\"site-skeleton__sidebar-section\">{title}{}{}{}{}</div>",
        category(dot("--blue"), "--md"),
        category(dot("--light-blue"), "--lg"),
        category(dot("--staff"), "--sm"),
        category(nav_icon("list"), "--md"),
    ));
    out.push_str(&format!(
        "<div class=\"site-skeleton__sidebar-section\">{}</div>",
        category(nav_icon("magnifying-glass"), "--md")
    ));
    out.push_str(&format!(
        "<div class=\"site-skeleton__sidebar-section\">{title}{}{}</div></aside>",
        category(channel("--blue"), "--md"),
        category(channel("--red"), "--sm"),
    ));
    out.push_str(&format!(
        "<main class=\"site-skeleton__content\"><section class=\"site-skeleton__card\"><div class=\"site-skeleton__welcome-row\"><div class=\"site-skeleton__welcome-greeting\">{}{}</div><div class=\"site-skeleton__search\"><span class=\"site-skeleton__search-icon\">{}</span>{}<span class=\"site-skeleton__search-icon --filters\">{}</span></div></div>",
        bar("--heading --lg"),
        bar("--heading --md"),
        icon("magnifying-glass", None),
        bar("--sm"),
        icon("sliders", None),
    ));
    out.push_str(&format!(
        "<div class=\"site-skeleton__tabs\"><span class=\"--active\">{}</span><span>{}</span><span>{}</span><button tabindex=\"-1\" type=\"button\"></button></div>",
        bar("--accent --sm"),
        bar("--sm"),
        bar("--sm"),
    ));
    let topic = |title: &str, excerpt: &[&str], tag: &str, pin: bool| {
        let mut t = format!(
            "<div class=\"site-skeleton__topic\"><span class=\"site-skeleton__topic-avatar --system\"></span><div class=\"site-skeleton__topic-main\">{}{}<div class=\"site-skeleton__topic-excerpt\">",
            bar("--sm"),
            bar(&format!("--strong {title}")),
        );
        for m in excerpt {
            t.push_str(&bar(m));
        }
        t.push_str(&format!(
            "</div><span class=\"site-skeleton__topic-tag{tag}\"></span></div>"
        ));
        if pin {
            t.push_str("<span class=\"site-skeleton__topic-pin\"></span>");
        }
        t.push_str("</div>");
        t
    };
    out.push_str("<div class=\"site-skeleton__topic-list\">");
    out.push_str(&topic("--lg", &["--full", "--full", "--lg"], "", true));
    out.push_str(&topic("--md", &["--full", "--md"], " --staff", false));
    out.push_str(&topic("--lg", &["--full", "--lg"], " --staff", false));
    out.push_str("</div></section></main></div></div>");
    out
}
