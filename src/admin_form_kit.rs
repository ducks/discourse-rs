//! FormKit (frontend/discourse/app/form-kit) as the admin pages' forms
//! render it: the form, FKField's container with its title, optional
//! label, tooltip, description and help text, the input, select,
//! checkbox, textarea and toggle controls, and the actions row. The pages'
//! scripts read the controls by their `name`.

use crate::topic_list_view::{escape, icon};

/// `<form.Field>`: its control's markup in the container.
#[derive(Default)]
pub struct Field<'a> {
    pub name: &'a str,
    /// The control type (`@type`): input, select, checkbox, textarea, ...
    pub control: &'a str,
    pub title: Option<String>,
    /// `@format`: large, full, small, max.
    pub format: Option<&'a str>,
    /// `showOptional` on a field not required: the (optional) label, its
    /// text given.
    pub optional: Option<String>,
    pub tooltip: Option<String>,
    pub description: Option<String>,
    pub help_text: Option<String>,
    pub disabled: bool,
    pub inner: String,
}

impl Field<'_> {
    pub fn render(&self) -> String {
        let mut class = format!(
            "form-kit__container form-kit__field form-kit__field-{}",
            self.control
        );
        if self.disabled {
            class.push_str(" is-disabled");
        }
        if let Some(format) = self.format {
            class.push_str(&format!(" --{format}"));
        }
        let mut out = format!(
            "<div class=\"{class}\"{} data-name=\"{n}\" id=\"control-{n}\" data-control-type=\"{}\">",
            if self.disabled {
                " data-disabled=\"\""
            } else {
                ""
            },
            self.control,
            n = escape(self.name)
        );
        if let Some(title) = self.title.as_ref().filter(|_| self.control != "checkbox") {
            out.push_str(&format!(
                "<label class=\"form-kit__container-title\" for=\"{}\"><span>{}</span>",
                escape(&format!("control-{}-input", self.name)),
                escape(title)
            ));
            if let Some(optional) = &self.optional {
                out.push_str(&format!(
                    "<span class=\"form-kit__container-optional\">({})</span>",
                    escape(optional)
                ));
            }
            if let Some(tooltip) = &self.tooltip {
                out.push_str(&tooltip_trigger(tooltip, "form-kit__tooltip"));
            }
            out.push_str("</label>");
        }
        if let Some(description) = &self.description {
            out.push_str(&format!(
                "<p class=\"form-kit-text form-kit__container-description\">{}</p>",
                escape(description)
            ));
        }
        out.push_str(&format!(
            "<div class=\"form-kit__container-content\">{}",
            self.inner
        ));
        if let Some(help) = &self.help_text {
            out.push_str(&format!(
                "<p class=\"form-kit-text form-kit__container-help-text\">{}</p>",
                escape(help)
            ));
        }
        out.push_str("</div></div>");
        out
    }
}

/// DTooltip's trigger with the circle-question icon; its content in a
/// template for the page's script (admin-ui.js shows it).
pub fn tooltip_trigger(content: &str, class: &str) -> String {
    format!(
        "<span class=\"fk-d-tooltip__trigger {class}\" aria-expanded=\"false\" data-trigger=\"\" role=\"button\"><span class=\"fk-d-tooltip__trigger-container\"><span class=\"fk-d-tooltip__icon\">{}</span></span><template class=\"d-tooltip-content\">{}</template></span>",
        icon("circle-question", None),
        escape(content)
    )
}

/// FKControlInput
pub fn input(
    name: &str,
    value: &str,
    kind: &str,
    placeholder: Option<&str>,
    disabled: bool,
) -> String {
    format!(
        "<div class=\"form-kit__control-input-wrapper\"><input class=\"form-kit__control-input\"{} id=\"control-{n}-input\" name=\"{n}\"{} type=\"{kind}\" value=\"{}\"></div>",
        if disabled { " disabled=\"\"" } else { "" },
        placeholder
            .map(|p| format!(" placeholder=\"{}\"", escape(p)))
            .unwrap_or_default(),
        escape(value),
        n = escape(name)
    )
}

/// FKControlSelect over DNativeSelect: the options (value, label), the
/// selected value, and the none option's label when it is included.
pub fn select(
    name: &str,
    options: &[(String, String)],
    selected: &str,
    none: Option<&str>,
    disabled: bool,
) -> String {
    let mut out = format!(
        "<select class=\"d-native-select form-kit__control-select\"{} id=\"control-{n}-input\" name=\"{n}\">",
        if disabled { " disabled=\"\"" } else { "" },
        n = escape(name)
    );
    if let Some(none) = none {
        let sel = selected.is_empty();
        out.push_str(&format!(
            "<option class=\"d-native-select__option{}\"{} value=\"__NONE__\">{}</option>",
            if sel { " --selected" } else { "" },
            if sel { " selected=\"\"" } else { "" },
            escape(none)
        ));
    }
    for (value, label) in options {
        let sel = value == selected;
        out.push_str(&format!(
            "<option class=\"d-native-select__option{} form-kit__control-option\"{} value=\"{}\">{}</option>",
            if sel { " --selected" } else { "" },
            if sel { " selected=\"\"" } else { "" },
            escape(value),
            escape(label)
        ));
    }
    out.push_str("</select>");
    out
}

/// FKControlCheckbox: the box with the field's title (and its tooltip)
/// beside it, and the description block.
pub fn checkbox(
    name: &str,
    checked: bool,
    title: Option<&str>,
    tooltip: Option<&str>,
    disabled: bool,
) -> String {
    let mut out = format!(
        "<label class=\"form-kit__control-checkbox-label\" for=\"control-{n}-input\"><input{} class=\"form-kit__control-checkbox\"{} id=\"control-{n}-input\" name=\"{n}\" type=\"checkbox\"><span class=\"form-kit__control-checkbox-checkmark\">{}</span><span class=\"form-kit__control-checkbox-content\">",
        if checked { " checked=\"\"" } else { "" },
        if disabled { " disabled=\"\"" } else { "" },
        icon("check", None),
        n = escape(name)
    );
    if let Some(title) = title {
        out.push_str(&format!(
            "<span class=\"form-kit__control-checkbox-title\"><span>{}</span>",
            escape(title)
        ));
        if let Some(tooltip) = tooltip {
            out.push_str(&tooltip_trigger(tooltip, "form-kit__tooltip"));
        }
        out.push_str("</span>");
    }
    out.push_str("<span class=\"form-kit__control-checkbox-description\"></span></span></label>");
    out
}

/// FKControlTextarea
pub fn textarea(name: &str, value: &str, disabled: bool) -> String {
    format!(
        "<textarea class=\"form-kit__control-textarea\"{} id=\"control-{n}-input\" name=\"{n}\">{}</textarea>",
        if disabled { " disabled=\"\"" } else { "" },
        escape(value),
        n = escape(name)
    )
}

/// FKControlToggle: DToggleSwitch.
pub fn toggle(name: &str, on: bool, disabled: bool) -> String {
    format!(
        "<div class=\"d-toggle-switch form-kit__control-toggle\"><label class=\"d-toggle-switch__label\"><button aria-checked=\"{on}\" class=\"d-toggle-switch__checkbox\"{} data-name=\"{}\" role=\"switch\" type=\"button\"></button><span class=\"d-toggle-switch__checkbox-slider\">{}</span></label></div>",
        if disabled { " disabled=\"\"" } else { "" },
        escape(name),
        if on {
            icon("check", None)
        } else {
            String::new()
        }
    )
}

/// `<form.Actions>`
pub fn actions(inner: &str) -> String {
    format!("<div class=\"form-kit__actions form-kit__section\">{inner}</div>")
}

/// `<form.Submit>`
pub fn submit(class: &str, label: &str) -> String {
    let class = if class.is_empty() {
        String::new()
    } else {
        format!(" {}", escape(class))
    };
    format!(
        "<button class=\"btn btn-primary form-kit__button{class}\" type=\"submit\"><span class=\"d-button-label\">{}</span></button>",
        escape(label)
    )
}

/// `<form.Button>`: a button, or with `href` a route button the script
/// follows.
pub fn button(class: &str, label: &str, href: Option<&str>) -> String {
    format!(
        "<button class=\"btn {} form-kit__button\" type=\"button\"{}><span class=\"d-button-label\">{}</span></button>",
        escape(class),
        href.map(|h| format!(" data-href=\"{}\"", escape(h)))
            .unwrap_or_default(),
        escape(label)
    )
}

/// `<Form>`
pub fn form(class: &str, inner: &str) -> String {
    let class = if class.is_empty() {
        "form-kit".to_string()
    } else {
        format!("form-kit {class}")
    };
    format!(
        "<form class=\"{}\" novalidate=\"\">{inner}</form>",
        escape(&class)
    )
}
