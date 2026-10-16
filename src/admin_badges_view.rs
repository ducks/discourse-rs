//! The admin badges pages (components/admin-badges.gjs): the page header
//! with New, Bulk award and Group settings over the tabs, then the
//! outlet: AdminBadgesIndex (the list and the intro), AdminBadgesShow (the
//! list and the badge's FormKit form), AdminBadgesAward (the list and the
//! CSV form), or the badges area's AdminAreaSettings.
//! static/js/admin-badges.js does what the components do.

use serde_json::Value;

use crate::admin_form_kit as fk;
use crate::admin_ui::{Action, Button, tr};
use crate::admin_view::{AdminCx, Crumb, PageHeader, t};
use crate::topic_list_view::{escape, icon};

/// Which badges page.
pub enum Page {
    Index,
    Settings,
    /// `/admin/badges/:badge_id`, `new` included.
    Show(String),
    /// `/admin/badges/award/:badge_id`.
    Award(String),
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// The page: the header, then the outlet in the main area.
pub fn render(cx: &AdminCx, page: &Page, content: &str) -> String {
    let title = t(cx.i18n, "admin.config.badges.title");
    let mut crumbs = vec![
        Crumb {
            label: t(cx.i18n, "admin_title"),
            path: "/admin",
        },
        Crumb {
            label: title.clone(),
            path: "/admin/badges",
        },
    ];
    // AdminAreaSettings adds its own crumb.
    if matches!(page, Page::Settings) {
        crumbs.push(Crumb {
            label: t(cx.i18n, "settings"),
            path: "/admin/badges/settings",
        });
    }
    let actions = crate::admin_ui::actions(
        cx,
        "d-page-header",
        &[
            Action::Primary(Button {
                class: "new-badge",
                icon: Some("plus"),
                label: tr(cx, "admin.badges.new"),
                href: Some("/admin/badges/new"),
                ..Default::default()
            }),
            Action::Default(Button {
                class: "award-badge",
                icon: Some("upload"),
                label: tr(cx, "admin.badges.mass_award.title"),
                href: Some("/admin/badges/award/new"),
                ..Default::default()
            }),
            Action::Default(Button {
                class: "edit-groupings-btn",
                icon: Some("gear"),
                label: tr(cx, "admin.badges.group_settings"),
                title: tr(cx, "admin.badges.group_settings"),
                ..Default::default()
            }),
        ],
    );
    let tabs = crate::admin_ui::tabs(
        cx,
        &[
            (
                "admin-badges-tabs__settings",
                "/admin/badges/settings",
                "settings",
                matches!(page, Page::Settings),
            ),
            (
                "admin-badges-tabs__index",
                "/admin/badges",
                "admin.config.badges.title",
                matches!(page, Page::Index | Page::Show(_)),
            ),
        ],
    );
    let header = PageHeader {
        crumbs,
        title,
        actions,
        description: Some(escape(&t(
            cx.i18n,
            "admin.config.badges.header_description",
        ))),
        learn_more_url: Some(
            "https://meta.discourse.org/t/understanding-and-using-badges/32540".into(),
        ),
        tabs: Some(tabs),
    }
    .render(cx);
    format!(
        "<div class=\"badges\">{header}<div class=\"admin-container admin-config-page__main-area\">{content}</div></div>"
    )
}

/// `badge.badge_type.name`, from the document's types.
fn type_name<'a>(doc: &'a Value, badge: &Value) -> &'a str {
    doc["badge_types"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["id"] == badge["badge_type_id"])
        .map(|t| s(&t["name"]))
        .unwrap_or_default()
}

/// dIconOrImage: the image, else the icon as convertIconClass reads it.
pub fn icon_or_image(badge: &Value) -> String {
    if let Some(image) = badge["image_url"].as_str().filter(|i| !i.is_empty()) {
        return format!("<img src='{}' alt=''>", escape(image));
    }
    let name = s(&badge["icon"])
        .replace("far fa-", "far-")
        .replace("fab fa-", "fab-")
        .replace("fas fa-", "")
        .replace("fa-", "");
    let name = name.trim();
    if name.is_empty() {
        return String::new();
    }
    crate::post_view::d_icon(name, None)
}

/// DBadgeButton
fn badge_button(doc: &Value, badge: &Value) -> String {
    let title = badge["description"]
        .as_str()
        .filter(|d| !d.is_empty())
        .map(|d| {
            format!(
                " title=\"{}\"",
                escape(&crate::user_card_view::strip_tags(d))
            )
        })
        .unwrap_or_default();
    format!(
        "<span class=\"user-badge badge-type-{}{}\" data-badge-name=\"{}\"{title}>{}<span class=\"badge-display-name\">{}</span></span>",
        escape(&type_name(doc, badge).to_lowercase()),
        if badge["enabled"].as_bool().unwrap_or(false) {
            ""
        } else {
            " disabled"
        },
        escape(s(&badge["name"])),
        icon_or_image(badge),
        escape(s(&badge["name"]))
    )
}

/// DFilterControls' ResetButton.
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

/// AdminBadgesList: DFilterControls over the badges (name and
/// description), each linking to `prefix` + its id; `active` the current
/// badge's id.
pub fn list(cx: &AdminCx, doc: &Value, prefix: &str, active: Option<&str>, filter: &str) -> String {
    let terms: Vec<String> = filter
        .split(',')
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    let badges: Vec<&Value> = doc["badges"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| {
            terms.is_empty()
                || terms.iter().any(|term| {
                    ["name", "description"]
                        .iter()
                        .any(|k| s(&b[*k]).to_lowercase().contains(term))
                })
        })
        .collect();
    let mut out = format!(
        "<div class=\"content-list\"><div class=\"d-filter-controls\"><div class=\"d-filter-controls__inputs\"><div class=\"filter-input-container\">{}<input autocapitalize=\"none\" autocomplete=\"off\" autocorrect=\"off\" class=\"filter-input d-filter-controls__input\" placeholder=\"{}\" type=\"text\" value=\"{}\"></div></div>{}</div>",
        icon("magnifying-glass", Some("-left")),
        escape(&t(cx.i18n, "admin.badges.filter_placeholder")),
        escape(filter),
        reset_button(cx, filter.is_empty(), false)
    );
    if badges.is_empty() && !filter.is_empty() {
        out.push_str(&format!(
            "<div class=\"d-filter-controls__no-results\"><p>{}</p>{}</div></div>",
            escape(&t(cx.i18n, "admin.badges.no_badges_found")),
            reset_button(cx, false, true)
        ));
        return out;
    }
    out.push_str("<ul class=\"admin-badge-list\">");
    for badge in badges {
        let id = badge["id"].to_string();
        out.push_str(&format!(
            "<li class=\"admin-badge-list-item\"><a{} href=\"{}{}{id}\">{}</a></li>",
            if active == Some(id.as_str()) {
                " class=\"active\""
            } else {
                ""
            },
            cx.base_path,
            escape(prefix),
            badge_button(doc, badge)
        ));
    }
    out.push_str("</ul></div>");
    out
}

/// AdminBadgesIndex's intro.
pub fn intro(cx: &AdminCx) -> String {
    format!(
        "<section class=\"current-badge content-body\"><h2>{}</h2><p>{}</p></section>",
        escape(&t(cx.i18n, "admin.badges.badge_intro.title")),
        escape(&t(cx.i18n, "admin.badges.badge_intro.description"))
    )
}

/// `<form.Section>`
fn section(title: &str, inner: &str) -> String {
    format!(
        "<div class=\"form-kit__section\"><div class=\"form-kit__section-header\"><div class=\"form-kit__section-heading\"><h2 class=\"form-kit__section-title\">{}</h2></div></div>{inner}</div>",
        escape(title)
    )
}

/// `<form.CheckboxGroup>` with its title.
fn checkbox_group(title: Option<&str>, inner: &str) -> String {
    format!(
        "<fieldset class=\"form-kit__fieldset form-kit__checkbox-group\">{}{inner}</fieldset>",
        title
            .map(|t| format!(
                "<legend class=\"form-kit__fieldset-title\">{}</legend>",
                escape(t)
            ))
            .unwrap_or_default()
    )
}

/// A checkbox field of a group, its title beside the box.
fn checkbox_field(
    name: &str,
    title: &str,
    checked: bool,
    disabled: bool,
    full: bool,
    description: &str,
) -> String {
    fk::Field {
        name,
        control: "checkbox",
        format: full.then_some("full"),
        disabled,
        inner: fk::checkbox(name, checked, Some(title), None, disabled).replace(
            "<span class=\"form-kit__control-checkbox-description\"></span>",
            &format!(
                "<span class=\"form-kit__control-checkbox-description\">{}</span>",
                escape(description)
            ),
        ),
        ..Default::default()
    }
    .render()
}

/// A system badge's read-only text with its Customize text link.
fn readonly(cx: &AdminCx, name: &str, title_key: &str, html: &str, i18n_name: &str) -> String {
    format!(
        "<div class=\"form-kit__container\" data-name=\"{name}\"><span class=\"form-kit__container-title\">{}</span><div class=\"form-kit__container-content\"><span class=\"readonly-field\">{html}</span><a href=\"{}/admin/customize/site_texts?q={}\">{}</a></div></div>",
        escape(&t(cx.i18n, title_key)),
        cx.base_path,
        escape(&format!("badges.{i18n_name}.{name}")),
        icon("pencil", None)
    )
}

/// The badge show form's inputs: the badge (or the new badge's
/// defaults), the document, and enable_badge_sql.
pub struct ShowData<'a> {
    pub badge: &'a Value,
    pub doc: &'a Value,
    pub badge_sql: bool,
}

/// DIconGridPicker as FKControlIcon draws it (its caret, the selected
/// name).
fn icon_picker(cx: &AdminCx, value: &str) -> String {
    let (title, label) = if value.is_empty() {
        (String::new(), t(cx.i18n, "d_icon_grid_picker.select_icon"))
    } else {
        (
            format!(
                " title=\"{}\"",
                escape(
                    &t(cx.i18n, "d_icon_grid_picker.selected_icon").replace("%{iconName}", value)
                )
            ),
            value.to_string(),
        )
    };
    format!(
        "<div class=\"d-icon-grid-picker{} form-kit__control-icon\"{}><button aria-expanded=\"false\" class=\"btn no-text fk-d-menu__trigger d-icon-grid-picker-trigger btn-default\"{title} data-identifier=\"d-icon-grid-picker\" data-trigger=\"\" type=\"button\">{}<span class=\"d-icon-grid-picker__label\">{}</span>{}</button></div>",
        if value.is_empty() { "" } else { " --has-label" },
        if value.is_empty() {
            String::new()
        } else {
            format!(" data-value=\"{}\"", escape(value))
        },
        if value.is_empty() {
            String::new()
        } else {
            crate::post_view::d_icon(value, None)
        },
        escape(&label),
        icon("angle-down", Some("d-icon-grid-picker__caret"))
    )
}

/// FKControlImage (UppyImageUploader, type badge_image).
fn image_control(cx: &AdminCx, url: &str) -> String {
    let uploading = escape(&t(cx.i18n, "upload_selector.uploading"));
    let (class, preview) = if url.is_empty() {
        ("no-image", String::new())
    } else {
        ("has-image", escape(url))
    };
    format!(
        "<div class=\"file-uploader {class} form-kit__control-image no-repeat contain-image\"><div class=\"file-uploader__preview input-xxlarge\" style=\"background-image: url({preview})\"><div class=\"file-uploader__controls\"><label class=\"btn btn-transparent\" tabindex=\"0\">{} <div class=\"pick-files-button\"><input accept=\"image/*\" type=\"file\"></div> {}</label><div class=\"file-uploader__progress-status hidden\"><div aria-label=\"{uploading}\n          0%\" class=\"progress-bar-container\" role=\"progressbar\"><div class=\"progress-bar\" style=\"width: 0%\"></div></div><span>{uploading} 0%</span></div></div></div></div>",
        icon("upload", None),
        escape(&t(cx.i18n, "upload_selector.select_file"))
    )
}

/// AdminBadgesShow's form.
pub fn show(cx: &AdminCx, d: &ShowData) -> String {
    let b = d.badge;
    let read_only = b["system"].as_bool().unwrap_or(false);
    let flag = |k: &str| b[k].as_bool().unwrap_or(false);
    let i18n_name = s(&b["i18n_name"]);
    let mut out = format!(
        "<h2 class=\"current-badge-header\">{}<span class=\"badge-display-name\">{}</span></h2>",
        icon_or_image(b),
        escape(s(&b["name"]))
    );
    // Status: FKControlQuestion.
    let enabled = flag("enabled");
    out.push_str(
        &fk::Field {
            name: "enabled",
            control: "question",
            title: Some(t(cx.i18n, "admin.badges.status")),
            inner: format!(
                "<div class=\"form-kit__inline-radio\"><label class=\"form-kit__control-radio-label --yes\"><input class=\"form-kit__control-radio\"{} name=\"enabled\" type=\"radio\" value=\"true\"> {}</label><label class=\"form-kit__control-radio-label --no\"><input class=\"form-kit__control-radio\"{} name=\"enabled\" type=\"radio\" value=\"false\"> {}</label></div>",
                if enabled { " checked" } else { "" },
                escape(&t(cx.i18n, "admin.badges.enabled")),
                if enabled { "" } else { " checked" },
                escape(&t(cx.i18n, "admin.badges.disabled"))
            ),
            ..Default::default()
        }
        .render(),
    );
    if read_only {
        out.push_str(&readonly(
            cx,
            "name",
            "admin.badges.name",
            &escape(s(&b["name"])),
            i18n_name,
        ));
    } else {
        out.push_str(
            &fk::Field {
                name: "name",
                control: "input",
                title: Some(t(cx.i18n, "admin.badges.name")),
                inner: fk::input("name", s(&b["name"]), "text", None, false),
                ..Default::default()
            }
            .render(),
        );
    }
    // Design.
    let types: Vec<(String, String)> = d.doc["badge_types"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| (t["id"].to_string(), s(&t["name"]).to_string()))
        .collect();
    let mut design = fk::Field {
        name: "badge_type_id",
        control: "select",
        title: Some(t(cx.i18n, "admin.badges.badge_type")),
        disabled: read_only,
        inner: fk::select(
            "badge_type_id",
            &types,
            &b["badge_type_id"].to_string(),
            None,
            read_only,
        ),
        ..Default::default()
    }
    .render();
    let image_url = s(&b["image_url"]);
    let image_active = !image_url.is_empty();
    design.push_str(&format!(
        "<div class=\"form-kit__conditional-display\"><div class=\"form-kit__inline-radio\"><label class=\"form-kit__control-radio-label\"><input class=\"form-kit__control-radio\"{} type=\"radio\" value=\"choose-icon\"><span>{}</span></label><label class=\"form-kit__control-radio-label\"><input class=\"form-kit__control-radio\"{} type=\"radio\" value=\"upload-image\"><span>{}</span></label></div><div class=\"form-kit__conditional-display-content{}\">{}</div><div class=\"form-kit__conditional-display-content{}\">{}</div></div>",
        if image_active { "" } else { " checked" },
        escape(&t(cx.i18n, "admin.badges.select_an_icon")),
        if image_active { " checked" } else { "" },
        escape(&t(cx.i18n, "admin.badges.upload_an_image")),
        if image_active { " hidden" } else { "" },
        fk::Field {
            name: "icon",
            control: "icon",
            format: Some("small"),
            inner: icon_picker(cx, s(&b["icon"])),
            ..Default::default()
        }
        .render(),
        if image_active { "" } else { " hidden" },
        fk::Field {
            name: "image_url",
            control: "image",
            inner: image_control(cx, image_url),
            ..Default::default()
        }
        .render(),
    ));
    for (name, key) in [
        ("description", "admin.badges.description"),
        ("long_description", "admin.badges.long_description"),
    ] {
        if read_only {
            design.push_str(&readonly(
                cx,
                name,
                key,
                // sanitize(text), trusted.
                &discourse_markdown::sanitizer::sanitize(
                    s(&b[name]),
                    &discourse_markdown::sanitizer::AllowList::new(),
                ),
                i18n_name,
            ));
        } else {
            design.push_str(
                &fk::Field {
                    name,
                    control: "textarea",
                    title: Some(t(cx.i18n, key)),
                    optional: Some(t(cx.i18n, "form_kit.optional")),
                    inner: fk::textarea(name, s(&b[name]), false),
                    ..Default::default()
                }
                .render(),
            );
        }
    }
    out.push_str(&section(
        &t(cx.i18n, "admin.badges.sections.design"),
        &design,
    ));
    // Query, with enable_badge_sql.
    if d.badge_sql {
        let query = s(&b["query"]);
        let mut q = fk::Field {
            name: "query",
            control: "code",
            title: Some(t(cx.i18n, "admin.badges.query")),
            format: Some("full"),
            optional: Some(t(cx.i18n, "form_kit.optional")),
            disabled: read_only,
            inner: format!(
                "<textarea class=\"form-kit__control-code\" data-lang=\"sql\"{} name=\"query\">{}</textarea>",
                if read_only { " disabled=\"\"" } else { "" },
                escape(query)
            ),
            ..Default::default()
        }
        .render();
        let triggers: Vec<(String, String)> = d.doc["admin_badges"]["triggers"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, v)| {
                (
                    v.to_string(),
                    t(cx.i18n, &format!("admin.badges.trigger_type.{k}")),
                )
            })
            .collect();
        q.push_str(&format!(
            "<div class=\"badge-query-controls\"{}>",
            if query.trim().is_empty() {
                " hidden"
            } else {
                ""
            }
        ));
        q.push_str(&format!(
            "<div class=\"form-kit__container\"><div class=\"form-kit__container-content\">{}{}</div></div>",
            fk::button(
                "btn-default preview-badge",
                &t(cx.i18n, "admin.badges.preview.link_text"),
                None
            ),
            fk::button(
                "btn-default preview-badge-plan",
                &t(cx.i18n, "admin.badges.preview.plan_text"),
                None
            )
        ));
        q.push_str(&checkbox_group(
            None,
            &format!(
                "{}{}",
                checkbox_field(
                    "auto_revoke",
                    &t(cx.i18n, "admin.badges.auto_revoke"),
                    flag("auto_revoke"),
                    read_only,
                    false,
                    ""
                ),
                checkbox_field(
                    "target_posts",
                    &t(cx.i18n, "admin.badges.target_posts"),
                    flag("target_posts"),
                    read_only,
                    false,
                    ""
                )
            ),
        ));
        q.push_str(
            &fk::Field {
                name: "trigger",
                control: "select",
                title: Some(t(cx.i18n, "admin.badges.trigger")),
                disabled: read_only,
                inner: fk::select(
                    "trigger",
                    &triggers,
                    &b["trigger"].to_string(),
                    None,
                    read_only,
                ),
                ..Default::default()
            }
            .render(),
        );
        q.push_str("</div>");
        out.push_str(&section(&t(cx.i18n, "admin.badges.sections.query"), &q));
    }
    // Settings.
    let groupings = d.doc["badge_groupings"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let current = groupings
        .iter()
        .find(|g| g["id"] == b["badge_grouping_id"])
        .map(|g| s(&g["name"]).to_string())
        .unwrap_or_default();
    let items: String = groupings
        .iter()
        .map(|g| {
            format!(
                "<li class=\"dropdown-menu__item form-kit__control-menu-item\" data-value=\"{}\"><button class=\"btn no-text btn-flat\" type=\"button\">{}</button></li>",
                g["id"],
                escape(s(&g["name"]))
            )
        })
        .collect();
    let mut settings = fk::Field {
        name: "badge_grouping_id",
        control: "menu",
        title: Some(t(cx.i18n, "admin.badges.badge_grouping")),
        inner: format!(
            "<button aria-expanded=\"false\" class=\"btn no-text fk-d-menu__trigger -trigger form-kit__control-menu-trigger\" data-trigger=\"\" data-value=\"{}\" type=\"button\"><span class=\"d-button-label\">{}</span>{}</button><template class=\"form-kit-menu-content\"><ul class=\"dropdown-menu\">{items}</ul></template>",
            b["badge_grouping_id"],
            escape(&current),
            icon("angle-down", None)
        ),
        ..Default::default()
    }
    .render();
    settings.push_str(&checkbox_group(
        Some(&t(cx.i18n, "admin.badges.usage_heading")),
        &format!(
            "{}{}",
            checkbox_field(
                "allow_title",
                &t(cx.i18n, "admin.badges.allow_title"),
                flag("allow_title"),
                false,
                true,
                ""
            ),
            checkbox_field(
                "multiple_grant",
                &t(cx.i18n, "admin.badges.multiple_grant"),
                flag("multiple_grant"),
                read_only,
                true,
                ""
            )
        ),
    ));
    let listable = flag("listable");
    let show_posts = flag("show_posts");
    let posts_disabled = !listable || !show_posts;
    settings.push_str(&checkbox_group(
        Some(&t(cx.i18n, "admin.badges.visibility_heading")),
        &format!(
            "{}{}{}",
            checkbox_field(
                "listable",
                &t(cx.i18n, "admin.badges.listable"),
                listable,
                read_only,
                true,
                ""
            ),
            checkbox_field(
                "show_posts",
                &t(cx.i18n, "admin.badges.show_posts"),
                show_posts,
                read_only,
                true,
                ""
            ),
            checkbox_field(
                "show_in_post_header",
                &t(cx.i18n, "admin.badges.show_in_post_header"),
                flag("show_in_post_header"),
                posts_disabled,
                true,
                // postHeaderDescription: disabled and not `data.system`, which
                // the form data never holds.
                &if posts_disabled {
                    t(cx.i18n, "admin.badges.show_in_post_header_disabled")
                } else {
                    String::new()
                }
            )
        ),
    ));
    out.push_str(&section(
        &t(cx.i18n, "admin.badges.sections.settings"),
        &settings,
    ));
    let mut actions = fk::submit("", &t(cx.i18n, "submit"));
    if !read_only {
        actions.push_str(&format!(
            "<button class=\"btn no-text form-kit__button badge-form__delete-badge-btn btn-danger\" type=\"button\">{}</button>",
            escape(&t(cx.i18n, "admin.badges.delete"))
        ));
    }
    out.push_str(&fk::actions(&actions));
    // What the script sends that no control holds.
    let attrs = format!(
        " data-image-url=\"{}\" data-image-upload-id=\"{}\" data-post-header-note=\"{}\"",
        escape(image_url),
        escape(&match &b["image_upload_id"] {
            Value::Null => String::new(),
            v => v.to_string(),
        }),
        escape(&t(cx.i18n, "admin.badges.show_in_post_header_disabled"))
    );
    fk::form("badge-form current-badge content-body", &out).replacen(
        " novalidate=\"\">",
        &format!(" novalidate=\"\"{attrs}>"),
        1,
    )
}

/// AdminBadgesAward's section: the badge with its CSV form, or the
/// note to pick one.
pub fn award(cx: &AdminCx, badge: Option<&Value>) -> String {
    let mut out = format!(
        "<section class=\"current-badge content-body\"><h2>{}</h2><p>{}</p>",
        escape(&t(cx.i18n, "admin.badges.mass_award.title")),
        escape(&t(cx.i18n, "admin.badges.mass_award.description"))
    );
    let Some(b) = badge else {
        out.push_str(&format!(
            "<span class=\"badge-required\">{}</span></section>",
            escape(&t(cx.i18n, "admin.badges.mass_award.no_badge_selected"))
        ));
        return out;
    };
    let grant_existing = if b["multiple_grant"].as_bool().unwrap_or(false) {
        format!(
            "<label class=\"grant-existing-holders\"><input class=\"grant-existing-holders-checkbox\" type=\"checkbox\"> {}</label>",
            escape(&t(
                cx.i18n,
                "admin.badges.mass_award.grant_existing_holders"
            ))
        )
    } else {
        String::new()
    };
    out.push_str(&format!(
        "<form class=\"form-horizontal\" data-badge-id=\"{}\"><div class=\"badge-preview control-group\">{}<span class=\"badge-display-name\">{}</span></div><div class=\"control-group\"><h4>{}</h4><input accept=\".csv\" id=\"massAwardCSVUpload\" type=\"file\"></div><div class=\"control-group\"><label class=\"checkbox-label\"><input type=\"checkbox\"> {}</label>{grant_existing}</div>{}<a class=\"btn btn-normal\" href=\"{}/admin/badges\">{}<span>{}</span></a></form></section>",
        b["id"],
        icon_or_image(b),
        escape(s(&b["name"])),
        escape(&t(cx.i18n, "admin.badges.mass_award.upload_csv")),
        escape(&t(cx.i18n, "admin.badges.mass_award.replace_owners")),
        Button {
            class: "btn-primary",
            icon: Some("certificate"),
            label: tr(cx, "admin.badges.mass_award.perform"),
            disabled: true,
            ..Default::default()
        }
        .render()
        .replacen("type=\"button\"", "type=\"submit\"", 1),
        cx.base_path,
        icon("xmark", None),
        escape(&t(cx.i18n, "cancel"))
    ));
    out
}

/// The strings admin-badges.js shows.
pub fn strings(cx: &AdminCx) -> String {
    crate::admin_ui::strings(
        cx,
        "admin-badges-strings",
        &[
            "admin.badges.badge_groupings.modal_title",
            "admin.badges.badge_grouping",
            "admin.badges.new",
            "admin.badges.save",
            "admin.badges.delete_confirm",
            "admin.badges.icon_or_image",
            "admin.badges.no_badges_found",
            "admin.badges.mass_award.success.one",
            "admin.badges.mass_award.success.other",
            "admin.badges.mass_award.aborted",
            "admin.badges.mass_award.csv_has_unmatched_users",
            "admin.badges.mass_award.csv_has_unmatched_users_truncated_list.one",
            "admin.badges.mass_award.csv_has_unmatched_users_truncated_list.other",
            "admin.badges.preview.modal_title",
            "admin.badges.preview.sql_error_header",
            "admin.badges.preview.error_help",
            "admin.badges.preview.bad_count_warning.header",
            "admin.badges.preview.bad_count_warning.text",
            "admin.badges.preview.grant_count.one",
            "admin.badges.preview.grant_count.other",
            "admin.badges.preview.no_grant_count",
            "admin.badges.preview.sample",
            "admin.badges.preview.grant.with",
            "admin.badges.preview.grant.with_post",
            "admin.badges.preview.grant.with_post_time",
            "admin.badges.preview.grant.with_time",
            "d_icon_grid_picker.search_placeholder",
            "d_icon_grid_picker.search_label",
            "d_icon_grid_picker.select_icon",
            "d_icon_grid_picker.selected_icon",
            "d_icon_grid_picker.no_results",
            "form_kit.errors.required",
            "filter_controls.reset",
            "uploading",
            "saved",
            "cancel",
            "generic_error",
        ],
    )
}
