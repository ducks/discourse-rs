//! The customize pages built around AceEditor (90g7): the email style's
//! HTML and CSS, the form templates' list and form.

use serde_json::Value;

use crate::admin_view::{AdminCx, Crumb, PageHeader, t};
use crate::topic_list_view::escape;

/// AceEditor before admin-customize.js loads Ace into it: the wrapper,
/// DConditionalLoadingSpinner's container and the editor's element with
/// its content (which `ace.edit` takes as the value) and mode.
pub fn ace_editor(cx: &AdminCx, content: &str, mode: &str, attrs: &str) -> String {
    format!(
        "<div class=\"ace-wrapper\" data-disabled=\"false\"><div class=\"loading-container inline-spinner\"><div class=\"ace\" data-ace-mode=\"{}\" data-ace-src=\"{}\"{attrs}>{}</div></div></div>",
        escape(mode),
        escape(&crate::assets::url(cx.base_path, "ace-editor-bundle.js")),
        escape(content)
    )
}

/// templates/admin-customize-email-style.gjs with the field's editor.
pub fn email_style(cx: &AdminCx, doc: &Value, field: &str) -> String {
    let style = &doc["email_style"];
    let header = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: t(cx.i18n, "admin.customize.email_style.heading"),
                path: "/admin/customize/email_style",
            },
        ],
        title: t(cx.i18n, "admin.config.email_appearance.title"),
        actions: String::new(),
        description: Some(escape(&t(
            cx.i18n,
            "admin.config.email_appearance.header_description",
        ))),
        learn_more_url: None,
        tabs: Some(crate::admin_ui::tabs(
            cx,
            &[
                (
                    "",
                    "/admin/customize/email_style/html",
                    "admin.customize.email_style.html",
                    field == "html",
                ),
                (
                    "",
                    "/admin/customize/email_style/css",
                    "admin.customize.email_style.css",
                    field == "css",
                ),
            ],
        )),
    }
    .render(cx);
    let value = style[field].as_str().unwrap_or_default();
    let default = style[&format!("default_{field}")]
        .as_str()
        .unwrap_or_default();
    let reset = crate::admin_ui::Button {
        class: "btn-default",
        label: crate::admin_ui::tr(cx, "admin.customize.email_style.reset"),
        disabled: value == default,
        ..Default::default()
    }
    .render();
    let save = crate::admin_ui::Button {
        class: "btn-primary",
        label: crate::admin_ui::tr(cx, "admin.customize.save"),
        disabled: true,
        ..Default::default()
    }
    .render();
    format!(
        "{header}<div class=\"admin-container\"><div>{}<div class=\"admin-footer\"><div class=\"buttons\">{reset}</div></div></div><div class=\"admin-footer\"><div class=\"buttons\">{save}</div></div></div>",
        ace_editor(cx, value, if field == "css" { "scss" } else { field }, "")
    )
}

/// FormTemplate's InfoHeader.
fn info_header(cx: &AdminCx) -> String {
    format!(
        "<div class=\"form-templates--info\"><h2>{}</h2><p class=\"desc\">{}</p></div>",
        escape(&t(cx.i18n, "admin.form_templates.title")),
        escape(&t(cx.i18n, "admin.form_templates.help"))
    )
}

/// templates/admin-customize-form-templates/index.gjs: the templates'
/// table (each with its categories, view, edit and delete) and New
/// Template.
pub fn form_templates(
    cx: &AdminCx,
    templates: &[Value],
    categories: &crate::topic_list_view::ListContext,
) -> String {
    let tr = |k: &str| escape(&t(cx.i18n, k));
    let mut out = format!("<div class=\"form-templates\">{}", info_header(cx));
    if !templates.is_empty() {
        out.push_str(&format!(
            "<table class=\"form-templates__table grid\"><thead><th class=\"col heading\">{}</th><th class=\"col heading\">{}</th><th class=\"col heading sr-only\">{}</th></thead><tbody>",
            tr("admin.form_templates.list_table.headings.name"),
            tr("admin.form_templates.list_table.headings.active_categories"),
            tr("admin.form_templates.list_table.headings.actions")
        ));
        for template in templates {
            let badges: String = template["category_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_i64)
                .filter_map(|id| categories.categories.get(&id))
                .map(|c| crate::topic_list_view::category_badge(categories, c))
                .collect();
            let button = |class: &'static str, icon_name: &'static str, title: &str| {
                crate::admin_ui::Button {
                    class,
                    icon: Some(icon_name),
                    title: crate::admin_ui::tr(cx, title),
                    ..Default::default()
                }
                .render()
            };
            out.push_str(&format!(
                "<tr class=\"admin-list-item\" data-template-id=\"{}\"><td class=\"col first\">{}</td><td class=\"col categories\">{badges}</td><td class=\"col action\">{}{}{}</td></tr>",
                template["id"],
                escape(template["name"].as_str().unwrap_or_default()),
                button(
                    "btn-default btn-view-template",
                    "far-eye",
                    "admin.form_templates.list_table.actions.view"
                ),
                button(
                    "btn-default btn-edit-template",
                    "pencil",
                    "admin.form_templates.list_table.actions.edit"
                ),
                button(
                    "btn-danger btn-delete-template",
                    "trash-can",
                    "admin.form_templates.list_table.actions.delete"
                ),
            ));
        }
        out.push_str("</tbody></table>");
    }
    out.push_str(
        &crate::admin_ui::Button {
            class: "btn-primary",
            icon: Some("plus"),
            label: crate::admin_ui::tr(cx, "admin.form_templates.new_template"),
            title: crate::admin_ui::tr(cx, "admin.form_templates.new_template"),
            href: Some("/admin/customize/form-templates/new"),
            ..Default::default()
        }
        .render(),
    );
    out.push_str("</div>");
    out
}

/// The quick insert fields' types and icons (FormTemplateForm's
/// quickInsertFields).
const QUICK_INSERT: [(&str, &str); 8] = [
    ("checkbox", "square-check"),
    ("input", "grip-lines"),
    ("textarea", "align-left"),
    ("dropdown", "circle-chevron-down"),
    ("upload", "cloud-arrow-up"),
    ("multiselect", "bullseye"),
    ("tagchooser", "bullseye"),
    ("composer", "fab-markdown"),
];

/// FormTemplate's Form for a new template, or the template given.
pub fn form_template_form(cx: &AdminCx, template: Option<&Value>) -> String {
    let tr = |k: &str| escape(&t(cx.i18n, k));
    let name = template
        .and_then(|t| t["name"].as_str())
        .unwrap_or_default();
    let content = template
        .and_then(|t| t["template"].as_str())
        .unwrap_or_default();
    let quick: String = QUICK_INSERT
        .iter()
        .map(|(kind, icon_name)| {
            crate::admin_ui::Button {
                class: &format!("btn-flat btn-icon-text quick-insert-{kind}"),
                icon: Some(icon_name),
                label: crate::admin_ui::tr(
                    cx,
                    &format!("admin.form_templates.quick_insert_fields.{kind}"),
                ),
                attrs: format!(" data-field-type=\"{kind}\""),
                ..Default::default()
            }
            .render()
        })
        .collect();
    let button = |class: &str, icon_name: &'static str, label: &str, disabled: bool| {
        crate::admin_ui::Button {
            class,
            icon: Some(icon_name),
            label: crate::admin_ui::tr(cx, label),
            disabled,
            ..Default::default()
        }
        .render()
    };
    let empty = content.is_empty();
    format!(
        "{}<div class=\"form-templates__form\" data-template-id=\"{}\"><div class=\"control-group\"><label for=\"template-name\">{}</label><input class=\"ember-text-field form-templates__form-name-input\" name=\"template-name\" placeholder=\"{}\" type=\"text\" value=\"{}\"></div><div class=\"control-group form-templates__editor\"><div class=\"form-templates__quick-insert-field-buttons\"><span>{}</span>{quick}{}</div>{}</div><div class=\"control-group\">{}</div><div class=\"footer-buttons\">{}{}{}</div></div>",
        info_header(cx),
        template.map(|t| t["id"].to_string()).unwrap_or_default(),
        tr("admin.form_templates.new_template_form.name.label"),
        tr("admin.form_templates.new_template_form.name.placeholder"),
        escape(name),
        tr("admin.form_templates.quick_insert_fields.add_new_field"),
        crate::admin_ui::Button {
            class: "btn-flat btn-icon-text form-templates__validations-modal-button",
            icon: Some("circle-check"),
            label: crate::admin_ui::tr(cx, "admin.form_templates.validations_modal.button_title"),
            ..Default::default()
        }
        .render(),
        button(
            "btn-default form-templates__preview-button",
            "eye",
            "admin.form_templates.new_template_form.preview",
            empty || name.is_empty()
        ),
        ace_editor(cx, content, "yaml", ""),
        button(
            "btn-primary",
            "check",
            "admin.form_templates.new_template_form.submit",
            empty || name.is_empty()
        ),
        button(
            "btn-default",
            "xmark",
            "admin.form_templates.new_template_form.cancel",
            false
        ),
        if template.is_some() {
            button(
                "btn-danger",
                "trash-can",
                "admin.form_templates.view_template.delete",
                false,
            )
        } else {
            String::new()
        },
    )
}

/// The page's root for the form: new or edit.
pub fn form_template_page(cx: &AdminCx, template: Option<&Value>) -> String {
    let class = if template.is_some() {
        "edit-form-template"
    } else {
        "new-form-template"
    };
    format!(
        "<div class=\"{class}\">{}</div>",
        form_template_form(cx, template)
    )
}
