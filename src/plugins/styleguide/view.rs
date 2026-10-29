//! The styleguide's pages: templates/styleguide.gjs with the section
//! navigation, StyleguideSection's page header (breadcrumbs and title),
//! and each section's StyleguideExample cards. The atoms the port draws
//! itself (typography, the font scale, buttons, colors, icons, form
//! inputs, spinners) are shown with the port's markup; the other sections
//! showcase Ember components the server-rendered pages do not have, and
//! say so.

use super::{Section, all_categories};
use crate::admin_form_kit as fk;
use crate::i18n::I18n;
use crate::topic_list_view::{escape, icon};

/// What the pages read.
pub struct Cx<'a> {
    pub i18n: &'a I18n,
    pub base_path: &'a str,
    /// The site sprite's symbol ids, sorted (the Icons section).
    pub icons: &'a [String],
}

impl Cx<'_> {
    fn t(&self, key: &str) -> String {
        let full = format!("js.{key}");
        self.i18n.t(&full).unwrap_or(key).to_string()
    }
}

/// `section-title`: `styleguide.sections.<id with underscores>.title`.
pub fn section_title(cx: &Cx, id: &str) -> String {
    cx.t(&format!(
        "styleguide.sections.{}.title",
        id.replace('-', "_")
    ))
}

/// The page: the navigation and the section, or the index's welcome.
pub fn page(cx: &Cx, section: Option<&Section>) -> String {
    let (class, title, contents) = match section {
        Some(section) => (
            format!(" {}-examples", section.id),
            section_title(cx, section.id),
            contents(cx, section),
        ),
        None => (
            String::new(),
            cx.t("styleguide.title"),
            format!(
                "<div class=\"description\">{}</div>",
                escape(&cx.t("styleguide.welcome"))
            ),
        ),
    };
    let mut crumbs = breadcrumb(
        &cx.t("styleguide.title"),
        &format!("{}/styleguide", cx.base_path),
    );
    if let Some(section) = section {
        crumbs.push_str(&breadcrumb(
            &section_title(cx, section.id),
            &format!(
                "{}/styleguide/{}/{}",
                cx.base_path, section.category, section.id
            ),
        ));
    }
    format!(
        "<link rel=\"stylesheet\" href=\"{}\" data-target=\"styleguide\">\
         <section class=\"styleguide\">{}<section class=\"styleguide-contents\">\
         <section class=\"styleguide-section{class}\"><div class=\"d-page-header\">\
         <div class=\"d-page-header__breadcrumbs\"><ul class=\"d-breadcrumbs\">{crumbs}</ul></div>\
         <div class=\"d-page-header__title-row\"><h1 class=\"d-page-header__title\">{}</h1></div></div>\
         <div class=\"styleguide-section-contents\">{contents}</div></section></section></section>",
        crate::assets::url(cx.base_path, "styleguide.css"),
        nav(cx, section),
        escape(&title),
    )
}

fn breadcrumb(label: &str, path: &str) -> String {
    format!(
        "<li class=\"d-breadcrumbs__item\"><a class=\"d-breadcrumbs__link\" href=\"{}\">{}</a></li>",
        escape(path),
        escape(label)
    )
}

/// The styleguide sidebar panel's sections and links, drawn beside the
/// section (the port's sidebar has no panels to switch to).
fn nav(cx: &Cx, current: Option<&Section>) -> String {
    let mut out = String::from("<nav class=\"styleguide-nav sidebar-sections\">");
    for (category, sections) in all_categories() {
        out.push_str(&format!(
            "<div class=\"sidebar-section\" data-section-name=\"styleguide-category-{category}\">\
             <div class=\"sidebar-section-header-wrapper\"><span class=\"sidebar-section-header-text\">{}</span></div>\
             <ul class=\"sidebar-section-content\">",
            escape(&cx.t(&format!("styleguide.categories.{category}")))
        ));
        for section in sections {
            let active = if current == Some(section) {
                " active"
            } else {
                ""
            };
            let title = section_title(cx, section.id);
            out.push_str(&format!(
                "<li class=\"sidebar-section-link-wrapper\" data-list-item-name=\"styleguide-section-{id}\">\
                 <a class=\"sidebar-section-link sidebar-row{active}\" href=\"{}/styleguide/{}/{id}\" title=\"{t}\">\
                 <span class=\"sidebar-section-link-content-text\">{t}</span></a></li>",
                escape(cx.base_path),
                section.category,
                id = section.id,
                t = escape(&title),
            ));
        }
        out.push_str("</ul></div>");
    }
    out.push_str("</nav>");
    out
}

/// StyleguideExample: a card with its title and the demo.
fn example(title: &str, body: &str) -> String {
    format!(
        "<section class=\"styleguide-example\"><div class=\"styleguide-example__header\">\
         <h2 class=\"styleguide-example__title\" data-test-example-title>{}</h2></div>\
         <div class=\"styleguide-example__body\">{body}</div></section>",
        escape(title)
    )
}

fn contents(cx: &Cx, section: &Section) -> String {
    match section.id {
        "typography" => typography(cx),
        "font-scale" => font_scale(cx),
        "buttons" => buttons(),
        "colors" => colors(),
        "icons" => icons(cx),
        "forms" => forms(),
        "spinners" => [
            example("spinner - small", "<div class=\"spinner small\"></div>"),
            example("spinner - regular", "<div class=\"spinner\"></div>"),
        ]
        .concat(),
        _ => "<div class=\"styleguide-note\">This section shows Ember components the \
              server-rendered pages do not have.</div>"
            .to_string(),
    }
}

fn typography(cx: &Cx) -> String {
    let text = escape(&cx.t("styleguide.sections.typography.example"));
    let mut out: String = ["h1", "h2", "h3", "h4", "h5", "h6"]
        .iter()
        .map(|h| example(h, &format!("<{h}>{text}</{h}>")))
        .collect();
    out.push_str(&example(
        "p",
        &format!(
            "<p>{}</p>",
            escape(&cx.t("styleguide.sections.typography.paragraph"))
        ),
    ));
    out
}

/// DHighlightedCode
fn code(lang: &str, source: &str) -> String {
    format!(
        "<pre class=\"highlighted-code\"><code class=\"lang-{lang}\">{}</code></pre>",
        escape(source)
    )
}

fn font_scale(cx: &Cx) -> String {
    let mut out = format!(
        "<div class=\"section-description\"><p>Discourse users can select from 4 different text sizes in their user settings, by default these are:<ul><li>Smaller: 14px</li><li>Normal (default): 15px</li><li>Larger: 17px</li><li>Largest: 19px</li></ul></p>\
         <p>If you'd like to increase the font size of your entire Discourse community, you can override the font-size of the HTML element. You can also provide different font sizes for the user text size settings defined above. The example below increases all text size options by 1px.</p>{}\
         <p>If you want to scale the fonts of a specific element, you can use Discourse's font scaling variables. Using the variable system ensures you're using a consistent set of font-sizes throughout your community.<p>Changing the font-size of a parent element will proportionately scale the font sizes of all its children.</p>{}</p></div>",
        code(
            "scss",
            "html {\n  font-size: 16px; // default font-size\n\n  &.text-size-smaller {\n    font-size: 15px;\n  }\n  &.text-size-larger {\n    font-size: 18px;\n  }\n  &.text-size-largest {\n    font-size: 20px;\n  }\n}"
        ),
        code(
            "scss",
            ".parent {\n  font-size: var(--font-up-3);\n  // Increases the relative font-size of this element and its children by 3 steps in the scale\n  .child {\n    // If this is set to var(--font-down-3) in Discourse's default CSS,\n    // the parent font-size increase above would make this equivalent to\n    // var(--font-0) (var(--font-down-3) + var(--font-up-3) = var(--font-0))\n  }\n}"
        ),
    );
    let text = escape(&cx.t("styleguide.sections.typography.example"));
    for step in [
        "up-6", "up-5", "up-4", "up-3", "up-2", "up-1", "0", "down-1", "down-2", "down-3",
        "down-4", "down-5", "down-6",
    ] {
        let title = if step == "0" {
            "var(--font-0), 1em \u{2014} base font".to_string()
        } else {
            format!("var(--font-{step})")
        };
        out.push_str(&example(
            &title,
            &format!("<p class=\"font-{step}\">{text}</p>"),
        ));
    }
    out
}

/// DButton: `btn` (or `btn-link` displayed as a link) with the class its
/// content gives it (icon only, text only, both), its icon and label.
fn button(
    class: &str,
    icon_name: Option<&str>,
    label: Option<&str>,
    title: Option<&str>,
    disabled: bool,
) -> String {
    let content_class = match (icon_name, label) {
        (Some(_), Some(_)) => "btn-icon-text",
        (Some(_), None) => "no-text btn-icon",
        _ => "btn-text",
    };
    let base = if class.contains("btn-link") {
        ""
    } else {
        "btn "
    };
    format!(
        "<button class=\"{base}{content_class} {class}\"{}{} type=\"button\">{}{}</button>",
        title
            .map(|t| format!(" title=\"{}\"", escape(t)))
            .unwrap_or_default(),
        if disabled { " disabled=\"\"" } else { "" },
        icon_name.map(|i| icon(i, None)).unwrap_or_default(),
        label
            .map(|l| format!("<span class=\"d-button-label\">{}</span>", escape(l)))
            .unwrap_or_default(),
    )
}

/// The three states (normal, hover, disabled) of a class with an icon and
/// a label.
fn states(class: &str, icon_name: Option<&str>) -> String {
    [
        button(class, icon_name, Some("normal"), None, false),
        button(
            &format!("{class} btn-hover"),
            icon_name,
            Some("hover"),
            None,
            false,
        ),
        button(class, icon_name, Some("disabled"), None, true),
    ]
    .concat()
}

/// The three sizes (large, default, small); `titled` for icon-only
/// buttons, whose size is their title.
fn sizes(class: &str, icon_name: Option<&str>, titled: bool) -> String {
    [
        ("large", " btn-large"),
        ("default", ""),
        ("small", " btn-small"),
    ]
    .iter()
    .map(|(name, size)| {
        let class = format!("{class}{size}");
        if titled {
            button(&class, icon_name, None, Some(name), false)
        } else {
            button(&class, icon_name, Some(name), None, false)
        }
    })
    .collect()
}

fn buttons() -> String {
    let icon_states = [
        button("btn-default", Some("xmark"), None, Some("normal"), false),
        button(
            "btn-default btn-hover",
            Some("xmark"),
            None,
            Some("hover"),
            false,
        ),
        button("btn-default", Some("xmark"), None, Some("disabled"), true),
    ]
    .concat();
    let toggle = [
        fk::toggle("styleguide-toggle", true, false),
        fk::toggle("styleguide-toggle-on", true, true),
        fk::toggle("styleguide-toggle-off", false, true),
    ]
    .concat()
    .replace(" form-kit__control-toggle", "");
    [
        example(
            "DButton - icon only - sizes (large, default, small)",
            &sizes("btn-default", Some("xmark"), true),
        ),
        example("DButton - icon only - states", &icon_states),
        example(
            "DButton - text only - sizes (large, default, small)",
            &sizes("btn-default", None, false),
        ),
        example("DButton - text only - states", &states("btn-default", None)),
        example(
            "DButton - icon and text - sizes (large, default, small)",
            &sizes("btn-default", Some("plus"), false),
        ),
        example(
            "DButton - icon and text - states",
            &states("btn-default", Some("plus")),
        ),
        example(
            "DButton - icon and text - sizes",
            &sizes("btn-primary", Some("plus"), false),
        ),
        example(
            "DButton - icon and text - btn-primary - states",
            &states("btn-primary", Some("plus")),
        ),
        example(
            "DButton - icon and text - btn-danger - sizes",
            &sizes("btn-danger", Some("trash-can"), false),
        ),
        example(
            "DButton - icon and text - btn-danger - states",
            &states("btn-danger", Some("trash-can")),
        ),
        example(
            "DButton - btn-flat - icon only - sizes",
            &sizes("btn-flat", Some("trash-can"), true),
        ),
        example(
            "DButton - btn-flat - states",
            &states("btn-flat", Some("trash-can")),
        ),
        example(
            "DButton - btn-transparent - states",
            &states("btn-transparent", Some("trash-can")),
        ),
        example("DButton - link", &states("btn-link", Some("trash-can"))),
        example("DToggleSwitch", &toggle),
    ]
    .concat()
}

/// The Colors section's rows, by card.
const COLORS: &[(&str, &[&[&str]])] = &[
    (
        "primary",
        &[
            &["primary-very-low", "primary-low", "primary-low-mid"],
            &["primary-medium", "primary-high", "primary"],
        ],
    ),
    (
        "primary cont'd",
        &[
            &[
                "primary-50",
                "primary-100",
                "primary-200",
                "primary-300",
                "primary-400",
                "primary-500",
            ],
            &[
                "primary-600",
                "primary-700",
                "primary-800",
                "primary-900",
                "primary",
            ],
        ],
    ),
    (
        "secondary",
        &[&[
            "secondary-low",
            "secondary-medium",
            "secondary-high",
            "secondary",
        ]],
    ),
    (
        "tertiary",
        &[&[
            "tertiary-low",
            "tertiary-medium",
            "tertiary-high",
            "tertiary",
        ]],
    ),
    (
        "tertiary cont'd",
        &[
            &[
                "tertiary-25",
                "tertiary-50",
                "tertiary-100",
                "tertiary-200",
                "tertiary-300",
                "tertiary-400",
            ],
            &[
                "tertiary-500",
                "tertiary-600",
                "tertiary-700",
                "tertiary-800",
                "tertiary-900",
                "tertiary",
            ],
        ],
    ),
    ("quaternary", &[&["quaternary-low", "quaternary"]]),
    ("highlight", &[&["highlight-bg", "highlight"]]),
    (
        "danger",
        &[&["danger-low", "danger-low-mid", "danger-medium", "danger"]],
    ),
    ("success", &[&["success-low", "success-medium", "success"]]),
    ("love", &[&["love-low", "love"]]),
    (
        "header",
        &[
            &["header_background"],
            &[
                "header_primary",
                "header_primary-very-high",
                "header_primary-high",
            ],
            &[
                "header_primary-medium",
                "header_primary-low-mid",
                "header_primary-low",
            ],
        ],
    ),
];

fn colors() -> String {
    COLORS
        .iter()
        .map(|(title, rows)| {
            let body: String = rows
                .iter()
                .map(|row| {
                    let swatches: String = row
                        .iter()
                        .map(|color| {
                            format!(
                                "<section class=\"color-example\"><div class=\"color-bg {color}\"></div>\
                                 <div class=\"color-name\">var(--{color})</div></section>"
                            )
                        })
                        .collect();
                    format!("<section class=\"color-row\">{swatches}</section>")
                })
                .collect();
            example(title, &body)
        })
        .collect()
}

fn icons(cx: &Cx) -> String {
    let gallery: String = cx
        .icons
        .iter()
        .map(|id| {
            format!(
                "<div class=\"styleguide-icon\">{}<span>{}</span></div>",
                icon(id, None),
                escape(id)
            )
        })
        .collect();
    format!(
        "<div class=\"section-description\"><p>Discourse uses a free set of SVG icons from Font Awesome (<a href=\"https://fontawesome.com/icons?d=gallery&amp;m=free\">{}</a>).</p>\
         <p>Plugins and themes can add SVG icons to the SVG spritesheet, or replace existing icons entirely.</p>\
         <p><ul><li><a href=\"https://meta.discourse.org/t/introducing-font-awesome-5-and-svg-icons/101643\">How to use SVG icons in your plugin or theme</a></li>\
         <li><a href=\"https://meta.discourse.org/t/replace-discourses-default-svg-icons-with-custom-icons-in-a-theme/115736/1\">How to replace Discourse's default icons in a theme</a></li></ul></p>\
         <p>By default, all icons have the <pre class=\"pre-inline\">.d-icon</pre> class applied along with a class containing the name of the icon (e.g., <pre class=\"pre-inline\">.d-icon-link</pre>)</p></div>{}",
        escape(&cx.t("styleguide.sections.icons.full_list")),
        example(
            "Available icons",
            &format!("<section class=\"styleguide-icons\">{gallery}</section>")
        ),
    )
}

/// A FormKit field around its control.
fn field(
    name: &str,
    control: &str,
    title: &str,
    format: Option<&'static str>,
    description: Option<&str>,
    inner: String,
) -> String {
    fk::Field {
        name,
        control,
        title: Some(title.to_string()),
        format,
        description: description.map(str::to_string),
        inner,
        ..Default::default()
    }
    .render()
}

/// The Forms section's examples the port's FormKit markup covers.
fn forms() -> String {
    let input = [
        field(
            "username",
            "input",
            "Username",
            None,
            None,
            fk::input("username", "", "text", Some("Username"), false),
        ),
        field(
            "age",
            "input-number",
            "Age",
            Some("small"),
            None,
            fk::input("age", "", "number", Some("Age"), false),
        ),
        field(
            "secret",
            "password",
            "Secret",
            None,
            Some("An important password"),
            fk::input("secret", "", "password", None, false),
        ),
    ]
    .concat();
    let yes_no = [
        ("true".to_string(), "Yes".to_string()),
        ("false".to_string(), "No".to_string()),
    ];
    let checkboxes = [
        field(
            "contract",
            "checkbox",
            "Use my email for any purpose.",
            None,
            None,
            fk::checkbox(
                "contract",
                false,
                Some("Use my email for any purpose."),
                None,
                false,
            ),
        ),
        field(
            "contract2",
            "checkbox",
            "Sign my soul away.",
            None,
            None,
            fk::checkbox("contract2", false, Some("Sign my soul away."), None, false),
        ),
    ]
    .concat();
    [
        example("Input", &fk::form("", &input)),
        example(
            "Toggle",
            &fk::form(
                "",
                &field("enabled", "toggle", "Enabled", None, None, fk::toggle("enabled", false, false)),
            ),
        ),
        example(
            "Textarea",
            &fk::form(
                "",
                &field("query", "textarea", "Query", None, None, fk::textarea("query", "", false)),
            ),
        ),
        example(
            "Select",
            &fk::form(
                "",
                &field(
                    "enabled",
                    "select",
                    "Enabled",
                    None,
                    None,
                    fk::select("enabled", &yes_no, "", Some("None"), false),
                ),
            ),
        ),
        example(
            "CheckboxGroup",
            &fk::form(
                "",
                &format!(
                    "<fieldset class=\"form-kit__checkbox-group\"><legend class=\"form-kit__fieldset-title\">I give explicit permission</legend>{checkboxes}</fieldset>"
                ),
            ),
        ),
        example(
            "Alert",
            &fk::form(
                "",
                &format!(
                    "<div class=\"form-kit__alert alert alert-info\">{}<span class=\"form-kit__alert-message\">You can edit this form.</span></div>",
                    icon("pencil", None)
                ),
            ),
        ),
    ]
    .concat()
}
