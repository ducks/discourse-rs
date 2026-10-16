//! The admin API keys pages (templates/admin-api-keys.gjs): the page
//! header with Add API key and the Settings and API keys tabs (none on a
//! key's page) over ApiKeysList and ApiKeyItem, ApiKeysShow, ApiKeysNew's
//! form (no header there, as the reference draws it on load), or the
//! user_api category's AdminAreaSettings. static/js/admin-api-keys.js does
//! what the components do.

use serde_json::Value;

use crate::admin_form_kit as fk;
use crate::admin_ui::{Action, Button, tr};
use crate::admin_view::{AdminCx, Crumb, PageHeader, t};
use crate::topic_list_view::{escape, icon};

/// Which page.
#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Index,
    Settings,
    Show,
    New,
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// The page: the header (none on New), then the outlet's content in the
/// main area.
pub fn render(cx: &AdminCx, tab: Tab, content: &str) -> String {
    let main =
        format!("<div class=\"admin-container admin-config-page__main-area\">{content}</div>");
    if tab == Tab::New {
        return main;
    }
    let title = t(cx.i18n, "admin.config.api_keys.title");
    let mut crumbs = vec![
        Crumb {
            label: t(cx.i18n, "admin_title"),
            path: "/admin",
        },
        Crumb {
            label: title.clone(),
            path: "/admin/api/keys",
        },
    ];
    if tab == Tab::Settings {
        crumbs.push(Crumb {
            label: t(cx.i18n, "settings"),
            path: "/admin/api/keys/settings",
        });
    }
    let tabs = (tab != Tab::Show).then(|| {
        crate::admin_ui::tabs(
            cx,
            &[
                (
                    "admin-api-keys-tabs__settings",
                    "/admin/api/keys/settings",
                    "settings",
                    tab == Tab::Settings,
                ),
                (
                    "admin-api-keys-tabs__index",
                    "/admin/api/keys",
                    "admin.config.api_keys.title",
                    tab == Tab::Index,
                ),
            ],
        )
    });
    let actions = crate::admin_ui::actions(
        cx,
        "d-page-header",
        &[Action::Primary(Button {
            label: tr(cx, "admin.api_keys.add"),
            href: Some("/admin/api/keys/new"),
            ..Default::default()
        })],
    );
    let header = PageHeader {
        crumbs,
        title,
        actions,
        description: Some(escape(&t(
            cx.i18n,
            "admin.config.api_keys.header_description",
        ))),
        learn_more_url: None,
        tabs,
    }
    .render(cx);
    format!("{header}{main}")
}

/// ApiKeyItem's SCOPE_ICONS
fn scope_icon(mode: &str) -> &'static str {
    match mode {
        "global" => "globe",
        "read_only" => "eye",
        _ => "bullseye",
    }
}

/// `truncatedKey`
fn truncated(key: &Value) -> String {
    format!("{} ...", s(&key["truncated_key"]))
}

fn user_link(cx: &AdminCx, user: &Value, with_name: bool) -> String {
    format!(
        "<a href=\"{}/admin/users/{}/{}\">{}{}</a>",
        cx.base_path,
        user["id"].as_i64().unwrap_or(0),
        escape(s(&user["username"])),
        crate::admin_ui::avatar(user),
        if with_name {
            format!(" {}", escape(s(&user["username"])))
        } else {
            String::new()
        }
    )
}

/// ApiKeysList: the keys' table, or AdminConfigAreaEmptyList.
pub fn list(cx: &AdminCx, keys: &[Value]) -> String {
    let mut out = String::from("<div class=\"container admin-api_keys\">");
    if keys.is_empty() {
        out.push_str(&crate::admin_ui::empty_list(
            cx,
            "admin-api_keys__add-api_key",
            "admin.api_keys.add",
            "/admin/api/keys/new",
            "admin.api_keys.no_api_keys",
        ));
        out.push_str("</div>");
        return out;
    }
    out.push_str("<table class=\"d-table admin-api_keys__items\"><thead class=\"d-table__header\"><tr class=\"d-table__row\">");
    for key in [
        "admin.api.key",
        "admin.api.description",
        "admin.api.user",
        "admin.api.created",
        "admin.api.scope",
        "admin.api.last_used",
    ] {
        out.push_str(&format!(
            "<th class=\"d-table__header-cell\">{}</th>",
            escape(&t(cx.i18n, key))
        ));
    }
    out.push_str(
        "<th class=\"d-table__header-cell\"></th></tr></thead><tbody class=\"d-table__body\">",
    );
    for key in keys {
        out.push_str(&item(cx, key));
    }
    out.push_str("</tbody></table></div>");
    out
}

fn mobile_label(cx: &AdminCx, key: &str) -> String {
    format!(
        "<div class=\"d-table__mobile-label\">{}</div>",
        escape(&t(cx.i18n, key))
    )
}

/// ApiKeyItem
fn item(cx: &AdminCx, key: &Value) -> String {
    let id = key["id"].as_i64().unwrap_or(0);
    let mode = s(&key["scope_mode"]);
    let revoked = !key["revoked_at"].is_null();
    let mut out = format!(
        "<tr class=\"d-table__row\" data-api-key-id=\"{id}\"><td class=\"d-table__cell --overview key\"><div class=\"d-table__value-wrapper\">{}",
        escape(&truncated(key))
    );
    if revoked {
        out.push_str(&format!(
            "<div class=\"status-label --inactive\"><div class=\"status-label-indicator\"></div><div class=\"status-label-text\">{}</div></div>",
            escape(&t(cx.i18n, "admin.api.revoked"))
        ));
    }
    out.push_str("</div></td>");
    // shortDescription: past 39 characters, the first 40 and an ellipsis.
    let full = s(&key["description"]);
    let description = if full.encode_utf16().count() < 40 {
        full.to_string()
    } else {
        format!(
            "{}...",
            String::from_utf16_lossy(&full.encode_utf16().take(40).collect::<Vec<_>>())
        )
    };
    out.push_str(&format!(
        "<td class=\"d-table__cell --detail key-description\">{}{}</td>",
        mobile_label(cx, "admin.api.description"),
        escape(&description)
    ));
    out.push_str(&format!(
        "<td class=\"d-table__cell --detail key-user\">{}{}</td>",
        mobile_label(cx, "admin.api.user"),
        if key["user"].is_object() {
            user_link(cx, &key["user"], false)
        } else {
            escape(&t(cx.i18n, "admin.api.all_users"))
        }
    ));
    out.push_str(&format!(
        "<td class=\"d-table__cell --detail key-created\">{}<div class=\"d-table__value-wrapper\">{}{}</div></td>",
        mobile_label(cx, "admin.api.created"),
        if key["created_by"].is_object() {
            user_link(cx, &key["created_by"], false)
        } else {
            String::new()
        },
        crate::admin_ui::time(&key["created_at"])
            .map(|at| crate::admin_ui::format_date(cx, at, false))
            .unwrap_or_default()
    ));
    out.push_str(&format!(
        "<td class=\"d-table__cell --detail key-scope\">{}<div class=\"d-table__value-wrapper\">{}{}</div></td>",
        mobile_label(cx, "admin.api.scope"),
        icon(scope_icon(mode), None),
        escape(&t(cx.i18n, &format!("admin.api.scopes.{mode}")))
    ));
    out.push_str(&format!(
        "<td class=\"d-table__cell --detail key-last-used\">{}{}</td>",
        mobile_label(cx, "admin.api.last_used"),
        match crate::admin_ui::time(&key["last_used_at"]) {
            Some(at) => crate::admin_ui::format_date(cx, at, false),
            None => escape(&t(cx.i18n, "admin.api.never_used")),
        }
    ));
    let menu_item = if revoked {
        Button {
            class: "api-key-item__undo-revoke",
            icon: Some("arrow-rotate-left"),
            label: tr(cx, "admin.api_keys.undo_revoke"),
            title: tr(cx, "admin.api.undo_revoke"),
            ..Default::default()
        }
    } else {
        Button {
            class: "btn-danger api-key-item__revoke",
            icon: Some("xmark"),
            label: tr(cx, "admin.api_keys.revoke"),
            title: tr(cx, "admin.api.revoke"),
            ..Default::default()
        }
    };
    out.push_str(&format!(
        "<td class=\"d-table__cell --controls key-controls\"><div class=\"d-table__cell-actions\">{}{}</div></td></tr>",
        Button {
            class: "btn-default btn-small",
            label: tr(cx, "admin.api_keys.edit"),
            title: tr(cx, "admin.api.show_details"),
            href: Some(&format!("/admin/api/keys/{id}")),
            ..Default::default()
        }
        .render(),
        crate::admin_ui::menu(
            "btn-default",
            "api_key-menu",
            "ellipsis-vertical",
            &t(cx.i18n, "admin.config_areas.user_fields.more_options.title"),
            &format!(
                "<ul class=\"dropdown-menu\"><li class=\"dropdown-menu__item\">{}</li></ul>",
                menu_item.render()
            )
        )
    ));
    out
}

/// AdminFormRow
fn form_row(cx: &AdminCx, label: &str, inner: &str) -> String {
    format!(
        "<div class=\"row\"><div class=\"form-element label-area\"><label>{}</label></div><div class=\"form-element input-area\">{inner}</div></div>",
        escape(&t(cx.i18n, label))
    )
}

/// ApiKeysShow
pub fn show(cx: &AdminCx, key: &Value) -> String {
    let mut out = crate::admin_ui::back_button(cx, "/admin/api/keys", "admin.api_keys.back");
    out.push_str(&format!(
        "<div class=\"api-key api-key-show\" data-api-key-id=\"{}\" data-description=\"{}\">",
        key["id"].as_i64().unwrap_or(0),
        escape(s(&key["description"]))
    ));
    out.push_str(&form_row(cx, "admin.api.key", &escape(&truncated(key))));
    let description = match s(&key["description"]) {
        "" => t(cx.i18n, "admin.api.no_description"),
        d => d.to_string(),
    };
    out.push_str(&form_row(
        cx,
        "admin.api.description",
        &format!(
            "<span>{}</span><div class=\"controls\">{}</div>",
            escape(&description),
            Button {
                class: "btn-default api-key-show__edit-description",
                label: tr(cx, "admin.api_keys.edit"),
                ..Default::default()
            }
            .render()
        ),
    ));
    out.push_str(&form_row(
        cx,
        "admin.api.user",
        &if key["user"].is_object() {
            user_link(cx, &key["user"], true)
        } else {
            escape(&t(cx.i18n, "admin.api.all_users"))
        },
    ));
    let date = |k: &str| {
        crate::admin_ui::time(&key[k])
            .map(|at| crate::admin_ui::format_date(cx, at, true))
            .unwrap_or_default()
    };
    out.push_str(&form_row(cx, "admin.api.created", &date("created_at")));
    out.push_str(&form_row(cx, "admin.api.updated", &date("updated_at")));
    out.push_str(&form_row(
        cx,
        "admin.api.last_used",
        &if key["last_used_at"].is_null() {
            escape(&t(cx.i18n, "admin.api.never_used"))
        } else {
            date("last_used_at")
        },
    ));
    let revoked = if key["revoked_at"].is_null() {
        format!(
            "<span>{}</span><div class=\"controls\">{}</div>",
            escape(&t(cx.i18n, "no_value")),
            Button {
                class: "btn-danger api-key-show__revoke",
                label: tr(cx, "admin.api.revoke"),
                ..Default::default()
            }
            .render()
        )
    } else {
        format!(
            "{}<div class=\"controls\">{}{}</div>",
            date("revoked_at"),
            Button {
                class: "btn-default api-key-show__undo-revoke",
                label: tr(cx, "admin.api.undo_revoke"),
                ..Default::default()
            }
            .render(),
            Button {
                class: "btn-danger api-key-show__delete",
                label: tr(cx, "admin.api.delete"),
                ..Default::default()
            }
            .render()
        )
    };
    out.push_str(&form_row(cx, "admin.api.revoked", &revoked));
    let scopes = key["api_key_scopes"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if !scopes.is_empty() {
        out.push_str(&format!(
            "<h2 class=\"scopes-title\">{}</h2><table class=\"scopes-table grid\"><thead><tr>",
            escape(&t(cx.i18n, "admin.api.scopes.title"))
        ));
        for k in [
            "admin.api.scopes.resource",
            "admin.api.scopes.action",
            "admin.api.scopes.allowed_urls",
            "admin.api.scopes.allowed_parameters",
        ] {
            out.push_str(&format!("<td>{}</td>", escape(&t(cx.i18n, k))));
        }
        out.push_str("</tr></thead><tbody>");
        for scope in &scopes {
            let resource = s(&scope["resource"]);
            let mut params = String::new();
            for p in scope["parameters"].as_array().into_iter().flatten() {
                let p = s(p);
                let allowed = &scope["allowed_parameters"][p];
                let value = match allowed {
                    Value::Array(a) if !a.is_empty() => a
                        .iter()
                        .map(|v| s(v).to_string())
                        .collect::<Vec<_>>()
                        .join(","),
                    Value::String(v) if !v.is_empty() => v.clone(),
                    _ => t(cx.i18n, "admin.api.scopes.any_parameter"),
                };
                params.push_str(&format!(
                    "<div><b>{}:</b> {}</div>",
                    escape(p),
                    escape(&value)
                ));
            }
            out.push_str(&format!(
                "<tr><td>{}</td><td>{} {}</td><td>{}</td><td>{params}</td></tr>",
                escape(resource),
                escape(s(&scope["action"])),
                fk::tooltip_trigger(
                    &t(
                        cx.i18n,
                        &format!(
                            "admin.api.scopes.descriptions.{resource}.{}",
                            s(&scope["key"])
                        )
                    ),
                    "scope-tooltip"
                ),
                urls_button(&scope["urls"])
            ));
        }
        out.push_str("</tbody></table>");
    }
    out.push_str("</div>");
    out
}

/// The scope's Allowed URLs button, its URLs for the modal.
fn urls_button(urls: &Value) -> String {
    Button {
        class: "btn-info",
        icon: Some("link"),
        attrs: format!(" data-urls=\"{}\"", escape(&urls.to_string())),
        ..Default::default()
    }
    .render()
}

/// ApiKeysNew: the form (description, user level, scope; the user
/// chooser and the granular scopes' table shown by the script when their
/// mode is picked), the scopes in a template for it.
pub fn new(cx: &AdminCx, scopes: &Value) -> String {
    let mut form = fk::Field {
        name: "description",
        control: "input",
        title: Some(t(cx.i18n, "admin.api.description")),
        format: Some("large"),
        inner: fk::input("description", "", "text", None, false),
        ..Default::default()
    }
    .render();
    form.push_str(
        &fk::Field {
            name: "user_mode",
            control: "select",
            title: Some(t(cx.i18n, "admin.api.user_mode")),
            format: Some("large"),
            inner: fk::select(
                "user_mode",
                &[
                    ("all".into(), t(cx.i18n, "admin.api.all_users")),
                    ("single".into(), t(cx.i18n, "admin.api.single_user")),
                ],
                "all",
                None,
                false,
            ),
            ..Default::default()
        }
        .render(),
    );
    form.push_str(&format!(
        "<template class=\"api-keys-new__user-field\">{}</template>",
        fk::Field {
            name: "user",
            control: "custom",
            title: Some(t(cx.i18n, "admin.api.user")),
            format: Some("large"),
            inner: user_chooser(cx),
            ..Default::default()
        }
        .render()
    ));
    form.push_str(
        &fk::Field {
            name: "scope_mode",
            control: "select",
            title: Some(t(cx.i18n, "admin.api.scope_mode")),
            format: Some("large"),
            inner: fk::select(
                "scope_mode",
                &["global", "read_only", "granular"]
                    .iter()
                    .map(|m| (m.to_string(), t(cx.i18n, &format!("admin.api.scopes.{m}"))))
                    .collect::<Vec<_>>(),
                "global",
                None,
                false,
            ),
            ..Default::default()
        }
        .render(),
    );
    form.push_str(&format!(
        "<template class=\"api-keys-new__scopes\">{}</template>",
        scopes_table(cx, scopes)
    ));
    form.push_str(&fk::actions(&format!(
        "{}{}",
        fk::submit("save", &t(cx.i18n, "admin.api_keys.save")),
        fk::button(
            "btn-default",
            &t(cx.i18n, "admin.api_keys.cancel"),
            Some("/admin/api/keys")
        )
    )));
    format!(
        "{}<div class=\"admin-config-area\"><div class=\"admin-config-area__primary-content\"><div class=\"admin-config-area-card\"><div class=\"conditional-loading-section\">{}</div></div></div></div>",
        crate::admin_ui::back_button(cx, "/admin/api/keys", "admin.api_keys.back"),
        fk::form("", &form)
    )
}

/// EmailGroupUserChooser (maximum one), closed: its filter is drawn in
/// the body as it opens.
fn user_chooser(cx: &AdminCx) -> String {
    format!(
        "<div class=\"form-kit__control-custom\"><details class=\"select-kit multi-select user-chooser email-group-user-chooser\" data-placeholder=\"{}\"{}><summary class=\"select-kit-header multi-select-header\" data-name=\"\" tabindex=\"0\"><div class=\"select-kit-header-wrapper\"><span class=\"formatted-selection\">{}</span>{}</div></summary><div class=\"select-kit-body\"></div></details></div>",
        escape(&t(cx.i18n, "admin.api.user_placeholder")),
        crate::admin_ui::chooser_labels(cx),
        escape(&t(cx.i18n, "select_kit.default_header_text")),
        icon("plus", Some("angle-icon"))
    )
}

/// The granular scopes' table: each resource's scopes with their
/// checkbox, URLs button and parameters.
fn scopes_table(cx: &AdminCx, scopes: &Value) -> String {
    // FormKit's Object and Collection divs sit inside the tbody, which
    // markup can't hold: admin-api-keys.js wraps the rows by their
    // data-resource as it inserts the table.
    let mut out = format!(
        "<h2 class=\"scopes-title\">{}</h2><p>{}</p><table class=\"scopes-table grid\"><thead><tr><td></td><td>{}</td><td>{}</td></tr></thead><tbody>",
        escape(&t(cx.i18n, "admin.api.scopes.title")),
        escape(&t(cx.i18n, "admin.api.scopes.description")),
        escape(&t(cx.i18n, "admin.api.scopes.allowed_urls")),
        escape(&t(cx.i18n, "admin.api.scopes.optional_allowed_parameters"))
    );
    let mut resources: Vec<(&String, &Value)> = scopes
        .as_object()
        .map(|o| o.iter().filter(|(k, _)| k.as_str() != "global").collect())
        .unwrap_or_default();
    resources.sort_by(|a, b| a.0.cmp(b.0));
    for (resource, list) in resources {
        out.push_str(&format!(
            "<tr class=\"scope-resource-name\" data-resource=\"{r}\"><td><b>{r}</b></td><td></td><td></td><td></td></tr>",
            r = escape(resource)
        ));
        for (index, scope) in list.as_array().into_iter().flatten().enumerate() {
            let key = s(&scope["key"]);
            let name = format!("scopes.{resource}.{index}");
            let checkbox = fk::Field {
                name: &format!("{name}.enabled"),
                control: "checkbox",
                inner: fk::checkbox(
                    &format!("{name}.enabled"),
                    false,
                    Some(key),
                    Some(&t(
                        cx.i18n,
                        &format!("admin.api.scopes.descriptions.{resource}.{key}"),
                    )),
                    false,
                ),
                ..Default::default()
            }
            .render();
            let params: String = scope["params"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| {
                    let p = s(p);
                    let field = format!("{name}.params.{p}");
                    fk::Field {
                        name: &field,
                        control: "input",
                        inner: fk::input(&field, "", "text", Some(p), false),
                        ..Default::default()
                    }
                    .render()
                })
                .collect();
            out.push_str(&format!(
                "<tr data-resource=\"{r}\" data-scope-id=\"{r}:{}\"><td>{checkbox}</td><td>{}</td><td><div class=\"form-kit__object\">{params}</div></td></tr>",
                escape(key),
                urls_button(&scope["urls"]),
                r = escape(resource)
            ));
        }
    }
    out.push_str("</tbody></table>");
    out
}

/// The strings admin-api-keys.js shows.
pub fn strings(cx: &AdminCx) -> String {
    crate::admin_ui::strings(
        cx,
        "admin-api-keys-strings",
        &[
            "admin.api.scopes.allowed_urls",
            "admin.api.scopes.one_or_more",
            "admin.api.scopes.title",
            "admin.api.not_shown_again",
            "admin.api.description",
            "admin.api.scope_mode",
            "admin.api.scopes.global",
            "admin.api.scopes.read_only",
            "admin.api.scopes.granular",
            "admin.api.description_placeholder",
            "admin.api_keys.copy_key",
            "admin.api_keys.continue",
            "admin.api_keys.save",
            "admin.api_keys.cancel",
            "admin.api_keys.edit",
            "admin.api_keys.key_copied_to_clipboard",
            "form_kit.errors.required",
            "generic_error",
        ],
    )
}
