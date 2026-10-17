//! The about page (90g6), as Ember draws it: AdminConfigAreasAbout's
//! DPageHeader over its collapsable cards (general settings, contact
//! information, your organization, the group listing), each a FormKit
//! form with its own save, and the language selector when content
//! localization is on. Its script is static/js/admin-about.js.

use serde_json::Value;

use crate::admin_branding_view::image_uploader;
use crate::admin_view::{AdminCx, Crumb, PageHeader, t, t_with};
use crate::topic_list_view::{escape, icon};

/// The area's settings by name: the setting's `value`.
fn value<'a>(settings: &'a [Value], name: &str) -> &'a str {
    settings
        .iter()
        .find(|s| s["setting"].as_str() == Some(name))
        .and_then(|s| s["value"].as_str())
        .unwrap_or_default()
}

fn k(cx: &AdminCx, key: &str) -> String {
    t(cx.i18n, &format!("admin.config_areas.about.{key}"))
}

/// FKField's container: `@format="large"` unless `format` says, the
/// optional label unless required, the description and help text.
struct Field<'a> {
    name: &'a str,
    control: &'a str,
    title: String,
    description: Option<String>,
    help: Option<String>,
    required: bool,
    large: bool,
    inner: String,
}

impl Field<'_> {
    fn render(&self, cx: &AdminCx) -> String {
        let mut out = format!(
            "<div class=\"form-kit__container form-kit__field form-kit__field-{c}{}\" data-control-type=\"{c}\" data-name=\"{n}\" id=\"control-{n}\">",
            if self.large { " --large" } else { "" },
            c = self.control,
            n = self.name
        );
        if self.control != "checkbox" {
            out.push_str(&format!(
                "<label class=\"form-kit__container-title\" for=\"control-{}-input\"><span>{}</span>",
                self.name,
                escape(&self.title)
            ));
            if !self.required {
                out.push_str(&format!(
                    " <span class=\"form-kit__container-optional\">({})</span>",
                    escape(&t(cx.i18n, "form_kit.optional"))
                ));
            }
            out.push_str("</label>");
        }
        if let Some(d) = &self.description {
            out.push_str(&format!(
                "<p class=\"form-kit-text form-kit__container-description\">{}</p>",
                escape(d)
            ));
        }
        out.push_str(&format!(
            "<div class=\"form-kit__container-content\">{}",
            self.inner
        ));
        if let Some(h) = &self.help {
            out.push_str(&format!(
                "<p class=\"form-kit-text form-kit__container-help-text\">{}</p>",
                escape(h.trim_end())
            ));
        }
        out.push_str("</div></div>");
        out
    }
}

fn input(name: &str, kind: &str, value: &str, placeholder: Option<String>) -> String {
    format!(
        "<div class=\"form-kit__control-input-wrapper\"><input class=\"form-kit__control-input\" id=\"control-{name}-input\" name=\"{name}\"{} type=\"{kind}\" value=\"{}\"></div>",
        placeholder
            .map(|p| format!(" placeholder=\"{}\"", escape(&p)))
            .unwrap_or_default(),
        escape(value)
    )
}

/// A select-kit MultiSelect chooser (UserChooser, GroupChooser), closed:
/// the selection's names or Select…, the choices for admin-about.js.
pub(crate) fn chooser(
    cx: &AdminCx,
    class: &str,
    name: &str,
    chosen: &[(String, String)],
    max: Option<u32>,
) -> String {
    let label = if chosen.is_empty() {
        t(cx.i18n, "select_kit.default_header_text")
    } else {
        chosen
            .iter()
            .map(|(_, n)| n.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let values: Vec<&str> = chosen.iter().map(|(v, _)| v.as_str()).collect();
    format!(
        "<details class=\"select-kit multi-select {class}{}\" data-name=\"{name}\" data-value=\"{}\"{}><summary class=\"select-kit-header multi-select-header\" data-name=\"{}\" tabindex=\"0\"><div class=\"select-kit-header-wrapper\"><span class=\"formatted-selection\">{}</span>{}</div></summary><div class=\"select-kit-body\"></div></details>",
        if chosen.is_empty() {
            ""
        } else {
            " has-selection"
        },
        escape(&values.join("|")),
        max.map(|m| format!(" data-maximum=\"{m}\""))
            .unwrap_or_default(),
        escape(
            &chosen
                .iter()
                .map(|(_, n)| n.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ),
        escape(&label),
        icon("plus", Some("angle-icon"))
    )
}

fn submit(cx: &AdminCx) -> String {
    format!(
        "<button class=\"btn btn-primary form-kit__button\" type=\"submit\"><span class=\"d-button-label\">{}</span></button>",
        escape(&k(cx, "update"))
    )
}

/// AdminConfigAreaCard, collapsable: the heading, its toggle, the
/// description and the content.
fn card(
    cx: &AdminCx,
    class: &str,
    heading: &str,
    description: Option<&str>,
    content: &str,
) -> String {
    format!(
        "<section class=\"admin-config-area-card {class}\"><div class=\"admin-config-area-card__header-wrapper\"><h3 class=\"admin-config-area-card__title\">{}</h3><button class=\"btn no-text btn-transparent admin-config-area-card__toggle-button\" title=\"{}\" type=\"button\">{}</button></div><div class=\"admin-config-area-card__content\">{}{content}</div></section>",
        escape(heading),
        escape(&t(cx.i18n, "sidebar.toggle_section")),
        icon("angle-down", None),
        description
            .map(|d| format!(
                "<p class=\"admin-config-area-card__description\">{}</p>",
                escape(d)
            ))
            .unwrap_or_default()
    )
}

fn form(section: &str, inner: &str) -> String {
    format!(
        "<form class=\"form-kit\" novalidate=\"\" data-about-section=\"{section}\" data-values=\"{{}}\" data-upload-type=\"site_setting\">{inner}</form>"
    )
}

/// AdminConfigAreasAboutGeneralSettings for the default locale.
fn general(cx: &AdminCx, settings: &[Value], toolbar: &str) -> String {
    let banner = value(settings, "about_banner_image");
    let mut out = Field {
        name: "name",
        control: "input",
        title: k(cx, "community_name"),
        description: None,
        help: None,
        required: true,
        large: true,
        inner: input(
            "name",
            "text",
            value(settings, "title"),
            Some(k(cx, "community_name_placeholder")),
        ),
    }
    .render(cx);
    out.push_str(
        &Field {
            name: "summary",
            control: "input",
            title: k(cx, "community_summary"),
            description: None,
            help: None,
            required: false,
            large: true,
            inner: input("summary", "text", value(settings, "site_description"), None),
        }
        .render(cx),
    );
    // FKControlComposer: DEditor, in its Markdown mode.
    out.push_str(
        &Field {
            name: "extendedDescription",
            control: "composer",
            title: k(cx, "community_description"),
            description: None,
            help: None,
            required: false,
            large: false,
            inner: format!(
                "<div class=\"d-editor form-kit__control-composer\"><div class=\"d-editor-container --markdown-editor-enabled\"><div class=\"d-editor-textarea-column\"><div class=\"d-editor-textarea-wrapper\"><div class=\"d-overflow-controls d-editor-button-bar__wrap\"><div class=\"d-overflow-controls__content d-editor-button-bar\" role=\"toolbar\" data-d-scroll-axis=\"horizontal\">{toolbar}</div></div><textarea autocomplete=\"off\" class=\"d-editor-input\" name=\"extendedDescription\">{}</textarea></div></div><div class=\"d-editor-preview-wrapper\"><div class=\"d-editor-preview\">{}</div></div></div></div>",
                escape(value(settings, "extended_site_description")),
                value(settings, "extended_site_description_cooked")
            ),
        }
        .render(cx),
    );
    out.push_str(
        &Field {
            name: "communityTitle",
            control: "input",
            title: k(cx, "community_title"),
            description: Some(k(cx, "community_title_help")),
            help: None,
            required: false,
            large: true,
            inner: input(
                "communityTitle",
                "text",
                value(settings, "short_site_description"),
                None,
            ),
        }
        .render(cx),
    );
    out.push_str(
        &Field {
            name: "aboutBannerImage",
            control: "image",
            title: k(cx, "banner_image"),
            description: None,
            help: Some(k(cx, "banner_image_help")),
            required: false,
            large: false,
            inner: image_uploader(
                cx,
                "aboutBannerImage-field-aboutBannerImage",
                banner,
                None,
                None,
            ),
        }
        .render(cx),
    );
    out.push_str(&submit(cx));
    form("general_settings", &out)
}

fn contact(
    cx: &AdminCx,
    settings: &[Value],
    groups: &[(i64, String)],
    contact_name: Option<(String, String)>,
) -> String {
    let mut out = String::new();
    for (name, setting, kind, key) in [
        (
            "communityOwner",
            "community_owner",
            "text",
            "community_owner",
        ),
        ("contactEmail", "contact_email", "email", "contact_email"),
        ("contactURL", "contact_url", "url", "contact_url"),
    ] {
        let control = match kind {
            "email" => "input-email",
            "url" => "input-url",
            _ => "input",
        };
        out.push_str(
            &Field {
                name,
                control,
                title: k(cx, key),
                description: Some(k(cx, &format!("{key}_help")).trim_end().to_string()),
                help: None,
                required: false,
                large: true,
                inner: input(
                    name,
                    kind,
                    value(settings, setting),
                    Some(k(cx, &format!("{key}_placeholder"))),
                ),
            }
            .render(cx),
        );
    }
    out.push_str(
        &Field {
            name: "contactUsername",
            control: "custom",
            title: k(cx, "site_contact_name"),
            description: Some(k(cx, "site_contact_name_help").trim_end().to_string()),
            help: None,
            required: false,
            large: true,
            inner: format!(
                "<div class=\"form-kit__control-custom\">{}</div>",
                chooser(
                    cx,
                    "user-chooser",
                    "contactUsername",
                    &contact_name.into_iter().collect::<Vec<_>>(),
                    Some(1)
                )
            ),
        }
        .render(cx),
    );
    let group = value(settings, "site_contact_group_name");
    let chosen: Vec<(String, String)> = groups
        .iter()
        .filter(|(id, _)| id.to_string() == group)
        .map(|(id, n)| (id.to_string(), n.clone()))
        .collect();
    out.push_str(
        &Field {
            name: "contactGroupName",
            control: "custom",
            title: k(cx, "site_contact_group"),
            description: Some(k(cx, "site_contact_group_help").trim_end().to_string()),
            help: None,
            required: false,
            large: true,
            inner: format!(
                "<div class=\"form-kit__control-custom\">{}</div>",
                chooser(cx, "group-chooser", "contactGroupName", &chosen, Some(1))
            ),
        }
        .render(cx),
    );
    out.push_str(&submit(cx));
    form("contact_information", &out)
}

fn organization(cx: &AdminCx, settings: &[Value]) -> String {
    let mut out = String::new();
    for (name, setting, kind, key, has_help) in [
        ("companyName", "company_name", "text", "company_name", false),
        ("companyURL", "company_url", "url", "company_url", false),
        (
            "governingLaw",
            "governing_law",
            "text",
            "governing_law",
            true,
        ),
        (
            "cityForDisputes",
            "city_for_disputes",
            "text",
            "city_for_disputes",
            true,
        ),
    ] {
        out.push_str(
            &Field {
                name,
                control: if kind == "url" { "input-url" } else { "input" },
                title: k(cx, key),
                description: has_help.then(|| k(cx, &format!("{key}_help")).trim_end().to_string()),
                help: None,
                required: false,
                large: true,
                inner: input(
                    name,
                    kind,
                    value(settings, setting),
                    Some(k(cx, &format!("{key}_placeholder"))),
                ),
            }
            .render(cx),
        );
        if name == "companyName" {
            // <form.Alert @type="info">
            out.push_str(&format!(
                "<div class=\"alert alert-info form-kit__alert\"><span class=\"form-kit__alert-message\">{}</span></div>",
                escape(k(cx, "company_name_warning").trim_end())
            ));
        }
    }
    out.push_str(&submit(cx));
    form("your_organization", &out)
}

fn extra_groups(cx: &AdminCx, settings: &[Value], groups: &[(i64, String)]) -> String {
    let ids: Vec<&str> = value(settings, "about_page_extra_groups")
        .split('|')
        .filter(|v| !v.is_empty())
        .collect();
    let chosen: Vec<(String, String)> = ids
        .iter()
        .filter_map(|id| {
            groups
                .iter()
                .find(|(g, _)| g.to_string() == *id)
                .map(|(g, n)| (g.to_string(), n.clone()))
        })
        .collect();
    let order = value(settings, "about_page_extra_groups_order");
    let choices: Vec<(String, String)> = [
        "alphabetically",
        "order of creation",
        "order of theme setting",
    ]
    .iter()
    .map(|c| (c.to_string(), c.to_string()))
    .collect();
    let kx = |key: &str| k(cx, &format!("extra_groups.{key}"));
    let mut out = Field {
        name: "aboutPageExtraGroups",
        control: "custom",
        title: kx("groups"),
        description: None,
        help: None,
        required: false,
        large: true,
        inner: format!(
            "<div class=\"form-kit__control-custom\">{}</div>",
            chooser(cx, "group-chooser", "aboutPageExtraGroups", &chosen, None)
        ),
    }
    .render(cx);
    out.push_str(
        &Field {
            name: "aboutPageExtraGroupsInitialMembers",
            control: "input-number",
            title: kx("initial_members"),
            description: Some(kx("initial_members_description")),
            help: None,
            required: true,
            large: true,
            inner: input(
                "aboutPageExtraGroupsInitialMembers",
                "number",
                value(settings, "about_page_extra_groups_initial_members"),
                None,
            ),
        }
        .render(cx),
    );
    out.push_str(
        &Field {
            name: "aboutPageExtraGroupsOrder",
            control: "select",
            title: kx("order"),
            description: None,
            help: None,
            required: true,
            large: true,
            inner: crate::admin_form_kit::select(
                "aboutPageExtraGroupsOrder",
                &choices,
                order,
                None,
                false,
            ),
        }
        .render(cx),
    );
    out.push_str(
        &Field {
            name: "aboutPageExtraGroupsShowDescription",
            control: "checkbox",
            title: String::new(),
            description: None,
            help: None,
            required: false,
            large: true,
            inner: crate::admin_form_kit::checkbox(
                "aboutPageExtraGroupsShowDescription",
                value(settings, "about_page_extra_groups_show_description") == "true",
                Some(&kx("show_description")),
                None,
                false,
            ),
        }
        .render(cx),
    );
    out.push_str(&submit(cx));
    form("extra_groups", &out)
}

/// What the page needs beside the area's settings.
pub struct About<'a> {
    pub settings: &'a [Value],
    pub groups: &'a [(i64, String)],
    /// The site contact's username and name to show.
    pub contact_name: Option<(String, String)>,
    pub toolbar: &'a str,
    /// The language selector's locales (value, name) when content
    /// localization is on.
    pub locales: Option<Vec<(String, String)>>,
    pub default_locale: &'a str,
}

/// The about page.
pub fn page(cx: &AdminCx, about: &About) -> String {
    let title = t(cx.i18n, "admin.config.about.title");
    let header = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path: "/admin/config/about",
            },
        ],
        title,
        actions: String::new(),
        description: Some(t_with(
            cx.i18n,
            "admin.config.about.header_description",
            &[("basePath", cx.base_path)],
        )),
        learn_more_url: Some(
            "https://meta.discourse.org/t/understanding-and-customizing-the-about-page/332161"
                .into(),
        ),
        tabs: None,
    }
    .render(cx);
    let toolbar = about.locales.as_ref().map(|locales| {
        format!(
            "<div class=\"admin-config-area-about__language-toolbar\"><form class=\"form-kit admin-config-area-about__locale-form\" novalidate=\"\">{}</form></div>",
            Field {
                name: "locale",
                control: "select",
                title: k(cx, "locale_selector.title"),
                description: None,
                help: None,
                required: true,
                large: true,
                inner: crate::admin_form_kit::select("locale", locales, about.default_locale, None, false)
                    .replacen("form-kit__control-select", "form-kit__control-select admin-config-area-about__locale-selector-dropdown", 1),
            }
            .render(cx)
            .replacen(" id=\"control-locale\"", " id=\"control-locale\" data-locale-field=\"\"", 1)
        )
    });
    let s = about.settings;
    let cards = format!(
        "{}{}{}{}",
        card(
            cx,
            "admin-config-area-about__general-settings-section",
            &k(cx, "general_settings"),
            None,
            &general(cx, s, about.toolbar)
        ),
        card(
            cx,
            "admin-config-area-about__contact-information-section",
            &k(cx, "contact_information"),
            None,
            &contact(cx, s, about.groups, about.contact_name.clone())
        ),
        card(
            cx,
            "admin-config-area-about__your-organization-section",
            &k(cx, "your_organization"),
            Some(&k(cx, "your_organization_description")),
            &organization(cx, s)
        ),
        card(
            cx,
            "admin-config-area-about__extra-groups-section",
            &k(cx, "extra_groups.heading"),
            Some(&k(cx, "extra_groups.description")),
            &extra_groups(cx, s, about.groups)
        ),
    );
    let groups = Value::Array(
        about
            .groups
            .iter()
            .map(|(id, n)| serde_json::json!({ "value": id.to_string(), "name": n }))
            .collect(),
    );
    format!(
        "{header}<div class=\"admin-container admin-config-page__main-area\" data-groups=\"{}\" data-default-locale=\"{}\"><div class=\"admin-config-area\"><div class=\"admin-config-area__primary-content\">{}<div class=\"loading-container\">{cards}</div></div></div></div>",
        escape(&groups.to_string()),
        escape(about.default_locale),
        toolbar.unwrap_or_default()
    )
}

/// The strings admin-about.js shows.
pub const STRINGS: &[&str] = &[
    "admin.config_areas.about.toasts.general_settings_saved",
    "admin.config_areas.about.toasts.contact_information_saved",
    "admin.config_areas.about.toasts.your_organization_saved",
    "admin.config_areas.about.toasts.extra_groups_saved",
    "admin.config_areas.about.locale_selector.description",
    "select_kit.default_header_text",
    "select_kit.filter_placeholder",
    "select_kit.no_content",
    "form_kit.errors.required",
    "upload_selector.select_file",
    "upload_selector.change",
    "upload_selector.delete",
    "upload_selector.uploading",
    "expand",
    "generic_error",
];
