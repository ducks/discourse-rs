//! The admin web hooks pages (templates/admin-web-hooks.gjs): the page
//! header with Add webhook (no tabs) over WebhooksList and WebhookItem,
//! WebhooksForm for a new or an existing hook, or a hook's events
//! (templates/admin-web-hooks/show.gjs with WebhookEvents and
//! WebhookEvent). static/js/admin-web-hooks.js does what the components
//! do.

use serde_json::Value;

use crate::admin_form_kit as fk;
use crate::admin_ui::{Action, Button, tr};
use crate::admin_view::{AdminCx, Crumb, PageHeader, t, t_count};
use crate::topic_list_view::{escape, icon};

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// The page: the header, then the outlet in the main area. The new and
/// edit pages have no header, as the reference draws them on load.
pub fn render(cx: &AdminCx, content: &str, with_header: bool) -> String {
    if !with_header {
        return format!(
            "<div class=\"admin-webhooks admin-config-page\"><div class=\"admin-container admin-config-page__main-area\">{content}</div></div>"
        );
    }
    let title = t(cx.i18n, "admin.config.webhooks.title");
    let header = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path: "/admin/api/web_hooks",
            },
        ],
        title,
        actions: crate::admin_ui::actions(
            cx,
            "d-page-header",
            &[Action::Primary(Button {
                label: tr(cx, "admin.web_hooks.add"),
                href: Some("/admin/api/web_hooks/new"),
                ..Default::default()
            })],
        ),
        description: Some(escape(&t(
            cx.i18n,
            "admin.config.webhooks.header_description",
        ))),
        learn_more_url: None,
        tabs: None,
    }
    .render(cx);
    format!(
        "<div class=\"admin-webhooks admin-config-page\">{header}<div class=\"admin-container admin-config-page__main-area\">{content}</div></div>"
    )
}

/// The model's `description`: the event types' names, or `*` for every
/// event.
fn description(hook: &Value) -> String {
    if hook["wildcard_web_hook"].as_bool().unwrap_or(false) {
        return "*".into();
    }
    hook["web_hook_event_types"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| format!("{}_event", s(&t["name"]).to_lowercase()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// WebhookStatus
fn status(cx: &AdminCx, hook: &Value, statuses: &Value) -> String {
    let last = hook["last_delivery_status"].as_i64().unwrap_or(1);
    let name = statuses
        .as_array()
        .into_iter()
        .flatten()
        .find(|st| st["id"].as_i64() == Some(last))
        .map(|st| s(&st["name"]).to_string())
        .unwrap_or_default();
    let class = ["--inactive", "--critical", "--success", "--inactive"]
        .get((last - 1).max(0) as usize)
        .copied()
        .unwrap_or("--inactive");
    format!(
        "<div class=\"status-label {class}\" role=\"status\"><div class=\"status-label-indicator\"></div><div class=\"status-label-text\">{}</div></div>",
        escape(&t(
            cx.i18n,
            &format!("admin.web_hooks.delivery_status.{name}")
        ))
    )
}

/// WebhooksList: the hooks' table in DLoadMore, or the empty list.
pub fn list(cx: &AdminCx, doc: &Value) -> String {
    let hooks = doc["web_hooks"].as_array().cloned().unwrap_or_default();
    let mut out = String::from("<div class=\"container admin-api_keys\">");
    if hooks.is_empty() {
        out.push_str(&crate::admin_ui::empty_list(
            cx,
            "admin-web_hooks__add-web_hook",
            "admin.web_hooks.add",
            "/admin/api/web_hooks/new",
            "admin.web_hooks.none",
        ));
        out.push_str("</div>");
        return out;
    }
    let statuses = &doc["extras"]["delivery_statuses"];
    out.push_str("<div><table class=\"d-table admin-web_hooks__items\"><thead class=\"d-table__header\"><tr class=\"d-table__row\">");
    for key in [
        "admin.web_hooks.delivery_status.title",
        "admin.web_hooks.payload_url",
        "admin.web_hooks.description_label",
        "admin.web_hooks.controls",
    ] {
        out.push_str(&format!(
            "<th class=\"d-table__header-cell\">{}</th>",
            escape(&t(cx.i18n, key))
        ));
    }
    out.push_str("</tr></thead><tbody class=\"d-table__body\">");
    for hook in &hooks {
        let id = hook["id"].as_i64().unwrap_or(0);
        let base = format!("{}/admin/api/web_hooks/{id}", cx.base_path);
        let menu = format!(
            "<ul class=\"dropdown-menu\"><li class=\"dropdown-menu__item\">{}</li><li class=\"dropdown-menu__item\">{}</li></ul>",
            Button {
                class: "admin-web-hook__show",
                icon: Some("list"),
                label: tr(cx, "admin.web_hooks.show"),
                title: tr(cx, "admin.web_hooks.show"),
                href: Some(&format!("/admin/api/web_hooks/{id}")),
                ..Default::default()
            }
            .render(),
            Button {
                class: "btn-danger admin-web-hook__delete",
                icon: Some("trash-can"),
                label: tr(cx, "admin.web_hooks.delete"),
                title: tr(cx, "admin.web_hooks.delete"),
                ..Default::default()
            }
            .render()
        );
        out.push_str(&format!(
            "<tr class=\"d-table__row\" data-web-hook-id=\"{id}\"><td class=\"d-table__cell --overview key\"><a href=\"{base}\">{}</a></td><td class=\"d-table__cell --detail key-url\"><a href=\"{base}/edit\">{}</a></td><td class=\"d-table__cell --detail key-description\">{}</td><td class=\"d-table__cell --controls key-controls\"><div class=\"d-table__cell-actions\">{}{}</div></td></tr>",
            status(cx, hook, statuses),
            escape(s(&hook["payload_url"])),
            escape(&description(hook)),
            Button {
                class: "btn-default btn-small",
                label: tr(cx, "admin.web_hooks.edit"),
                title: tr(cx, "admin.web_hooks.edit"),
                href: Some(&format!("/admin/api/web_hooks/{id}/edit")),
                ..Default::default()
            }
            .render(),
            crate::admin_ui::menu(
                "",
                "webhook-menu",
                "ellipsis-vertical",
                &t(cx.i18n, "admin.config_areas.user_fields.more_options.title"),
                &menu
            )
        ));
    }
    out.push_str("</tbody></table>");
    let total = doc["total_rows_web_hooks"].as_i64().unwrap_or(0);
    out.push_str(&format!(
        "<div class=\"load-more-sentinel\" aria-hidden=\"true\"{}></div></div></div>",
        if (hooks.len() as i64) < total {
            format!(
                " data-load-more=\"{}\"",
                escape(s(&doc["load_more_web_hooks"]))
            )
        } else {
            String::new()
        }
    ));
    out
}

/// What WebhooksForm fills its fields from.
pub struct FormData<'a> {
    /// The hook, None for a new one.
    pub hook: Option<&'a Value>,
    /// The index's extras (event types, content types).
    pub extras: &'a Value,
    /// The selected categories' (id, name).
    pub categories: Vec<(String, String)>,
    /// The selected groups' (id, name).
    pub groups: Vec<(String, String)>,
    pub tagging_enabled: bool,
}

/// WebhooksForm
pub fn form(cx: &AdminCx, list: &crate::topic_list_view::ListContext, d: &FormData) -> String {
    let hook = d.hook.cloned().unwrap_or(Value::Null);
    let is_new = d.hook.is_none();
    let content_type = hook["content_type"].as_i64().unwrap_or(1).to_string();
    let wildcard = hook["wildcard_web_hook"].as_bool().unwrap_or(false);
    let optional = Some(t(cx.i18n, "form_kit.optional"));
    let mut f = fk::Field {
        name: "payload_url",
        control: "input",
        title: Some(t(cx.i18n, "admin.web_hooks.payload_url")),
        format: Some("large"),
        inner: fk::input(
            "payload_url",
            s(&hook["payload_url"]),
            "text",
            Some(&t(cx.i18n, "admin.web_hooks.payload_url_placeholder")),
            false,
        ),
        ..Default::default()
    }
    .render();
    let content_types: Vec<(String, String)> = d.extras["content_types"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            (
                c["id"].as_i64().unwrap_or(0).to_string(),
                s(&c["name"]).to_string(),
            )
        })
        .collect();
    f.push_str(
        &fk::Field {
            name: "content_type",
            control: "select",
            title: Some(t(cx.i18n, "admin.web_hooks.content_type")),
            format: Some("large"),
            inner: fk::select("content_type", &content_types, &content_type, None, false),
            ..Default::default()
        }
        .render(),
    );
    f.push_str(
        &fk::Field {
            name: "secret",
            control: "input",
            title: Some(t(cx.i18n, "admin.web_hooks.secret")),
            format: Some("large"),
            optional: optional.clone(),
            description: Some(t(cx.i18n, "admin.web_hooks.secret_placeholder")),
            inner: fk::input("secret", s(&hook["secret"]), "text", None, false),
            ..Default::default()
        }
        .render(),
    );
    // The event chooser: the hook's types, or the defaults for a new one.
    let chosen: Vec<String> = if is_new {
        d.extras["default_event_types"].clone()
    } else {
        hook["web_hook_event_types"].clone()
    }
    .as_array()
    .into_iter()
    .flatten()
    .map(|t| s(&t["name"]).to_string())
    .collect();
    let mut selector = String::new();
    for (group, types) in d.extras["grouped_event_types"]
        .as_object()
        .into_iter()
        .flatten()
    {
        selector.push_str(&format!(
            "<div class=\"event-group\">{}",
            escape(&t(
                cx.i18n,
                &format!("admin.web_hooks.{group}_event.group_name")
            ))
        ));
        for ty in types.as_array().into_iter().flatten() {
            let name = s(&ty["name"]);
            selector.push_str(&format!(
                "<label class=\"hook-event\"><input class=\"ember-checkbox\" name=\"event-choice\" type=\"checkbox\"{} data-id=\"{}\" data-name=\"{}\" data-group=\"{}\"> {}</label>",
                if chosen.iter().any(|c| c == name) {
                    " checked=\"\""
                } else {
                    ""
                },
                ty["id"].as_i64().unwrap_or(0),
                escape(name),
                escape(group),
                escape(&t(cx.i18n, &format!("admin.web_hooks.{group}_event.{name}")))
            ));
        }
        selector.push_str("</div>");
    }
    let radio = |value: &str, label: &str, checked: bool| {
        format!(
            "<div class=\"form-kit__field form-kit__field-radio\"><label class=\"form-kit__control-radio-label\"><input class=\"form-kit__control-radio\"{} name=\"wildcard\" type=\"radio\" value=\"{value}\"><span class=\"form-kit__control-radio-content\">{}</span></label></div>",
            if checked { " checked=\"\"" } else { "" },
            escape(label)
        )
    };
    // Shown while individual events are picked; kept for the script
    // otherwise.
    let individual = if wildcard {
        format!(
            "<template class=\"event-selector-template\"><div class=\"event-selector\">{selector}</div></template>"
        )
    } else {
        format!("<div class=\"event-selector\">{selector}</div>")
    };
    f.push_str(
        &fk::Field {
            name: "wildcard",
            control: "radio-group",
            title: Some(t(cx.i18n, "admin.web_hooks.event_chooser")),
            format: Some("full"),
            inner: format!(
                "<fieldset class=\"form-kit__fieldset form-kit__control-radio-group\" name=\"wildcard\">{}{individual}{}</fieldset>",
                radio(
                    "individual",
                    &t(cx.i18n, "admin.web_hooks.individual_event"),
                    !wildcard
                ),
                radio(
                    "wildcard",
                    &t(cx.i18n, "admin.web_hooks.wildcard_event"),
                    wildcard
                )
            ),
            ..Default::default()
        }
        .render(),
    );
    f.push_str(
        &fk::Field {
            name: "categories",
            control: "custom",
            title: Some(t(cx.i18n, "admin.web_hooks.categories_filter")),
            format: Some("large"),
            optional: optional.clone(),
            description: Some(t(cx.i18n, "admin.web_hooks.categories_filter_instructions")),
            inner: format!(
                "<div class=\"form-kit__control-custom\">{}</div>",
                crate::preferences_view::tracking::multi_select(
                    list,
                    "category-selector",
                    "category",
                    "categories",
                    &d.categories
                )
            ),
            ..Default::default()
        }
        .render(),
    );
    if d.tagging_enabled {
        let tags: Vec<(String, String)> = hook["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| {
                let name = t
                    .as_str()
                    .or_else(|| t["name"].as_str())
                    .unwrap_or_default();
                let id = t["id"].as_i64().map(|i| i.to_string());
                (id.unwrap_or_else(|| name.to_string()), name.to_string())
            })
            .collect();
        f.push_str(
            &fk::Field {
                name: "tags",
                control: "tag-chooser",
                title: Some(t(cx.i18n, "admin.web_hooks.tags_filter")),
                format: Some("large"),
                optional: optional.clone(),
                description: Some(t(cx.i18n, "admin.web_hooks.tags_filter_instructions")),
                inner: crate::preferences_view::tracking::multi_select(
                    list,
                    "tag-chooser form-kit__control-tag-chooser",
                    "tag",
                    "tags",
                    &tags,
                ),
                ..Default::default()
            }
            .render(),
        );
    }
    f.push_str(
        &fk::Field {
            name: "group_names",
            control: "custom",
            title: Some(t(cx.i18n, "admin.web_hooks.groups_filter")),
            format: Some("large"),
            optional,
            description: Some(t(cx.i18n, "admin.web_hooks.groups_filter_instructions")),
            inner: format!(
                "<div class=\"form-kit__control-custom\"><div class=\"group-selector-wrapper\">{}</div></div>",
                crate::admin_ui::multi_select_trigger("group-selector", "", &d.groups)
            ),
            ..Default::default()
        }
        .render(),
    );
    for (name, key) in [
        ("verify_certificate", "admin.web_hooks.verify_certificate"),
        ("active", "admin.web_hooks.active"),
    ] {
        let default = name == "verify_certificate";
        f.push_str(
            &fk::Field {
                name,
                control: "checkbox",
                format: Some("large"),
                inner: fk::checkbox(
                    name,
                    hook[name].as_bool().unwrap_or(default),
                    Some(&t(cx.i18n, key)),
                    None,
                    false,
                ),
                ..Default::default()
            }
            .render(),
        );
    }
    f.push_str(&fk::actions(&format!(
        "{}{}",
        fk::submit(
            "save",
            &t(
                cx.i18n,
                if is_new {
                    "admin.web_hooks.create"
                } else {
                    "admin.web_hooks.save"
                }
            )
        ),
        fk::button(
            "btn-default",
            &t(cx.i18n, "admin.web_hooks.cancel"),
            Some("/admin/api/web_hooks")
        )
    )));
    format!(
        "{}<div class=\"admin-config-area user-field\"><div class=\"admin-config-area__primary-content\"><div class=\"admin-config-area-card\"><div class=\"web-hook-container\"><div class=\"conditional-loading-section\"><p>{}</p>{}</div></div></div></div></div>",
        crate::admin_ui::back_button(cx, "/admin/api/web_hooks", "admin.web_hooks.back"),
        escape(&t(cx.i18n, "admin.web_hooks.detailed_instruction")),
        fk::form("", &f)
    )
}

/// templates/admin-web-hooks/show.gjs: the summary and WebhookEvents for
/// the URL's status filter.
pub fn show(cx: &AdminCx, hook: &Value, events: &Value, status: Option<&str>) -> String {
    let id = hook["id"].as_i64().unwrap_or(0);
    let mut out = format!(
        "<a class=\"go-back active\" href=\"{}/admin/api/web_hooks\">{}{}</a>",
        cx.base_path,
        icon("arrow-left", None),
        escape(&t(cx.i18n, "admin.web_hooks.back"))
    );
    out.push_str(&format!(
        "<div class=\"admin-webhooks__summary\" data-web-hook-id=\"{id}\"><h1>{}{}{}</h1><div><span class=\"admin-webhooks__description-label\">{}:</span> {}</div></div>",
        escape(s(&hook["payload_url"])),
        Button {
            class: "btn-default no-text admin-webhooks__edit-button",
            icon: Some("far-pen-to-square"),
            title: tr(cx, "admin.web_hooks.edit"),
            href: Some(&format!("/admin/api/web_hooks/{id}/edit")),
            ..Default::default()
        }
        .render(),
        Button {
            class: "destroy btn-danger admin-webhooks__delete-button",
            icon: Some("xmark"),
            title: tr(cx, "delete"),
            ..Default::default()
        }
        .render(),
        escape(&t(cx.i18n, "admin.web_hooks.description_label")),
        escape(&description(hook))
    ));
    // WebhookEvents
    let list = events["web_hook_events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let failed = list.iter().any(|e| {
        let st = e["status"].as_i64().unwrap_or(0);
        !(200..=299).contains(&st) && st != 0
    });
    let statuses = vec![
        (
            "successful".to_string(),
            t(cx.i18n, "admin.web_hooks.events.filter_status.successful"),
        ),
        (
            "failed".to_string(),
            t(cx.i18n, "admin.web_hooks.events.filter_status.failed"),
        ),
    ];
    out.push_str(&format!(
        "<div class=\"web-hook-events-listing\" data-web-hook-id=\"{id}\"><div class=\"web-hook-events-actions\">{}{}{}</div>",
        crate::admin_ui::combo_box(
            cx,
            "delivery-status-filters",
            &statuses,
            status,
            Some(&t(cx.i18n, "admin.web_hooks.events.filter_status.all"))
        ),
        Button {
            class: "btn-default web-hook-events__redeliver-failed",
            icon: Some("arrows-rotate"),
            label: tr(cx, "admin.web_hooks.events.redeliver_failed"),
            disabled: !failed,
            ..Default::default()
        }
        .render(),
        Button {
            class: "btn-default webhook-events__ping-button",
            icon: Some("paper-plane"),
            label: tr(cx, "admin.web_hooks.events.ping"),
            ..Default::default()
        }
        .render()
    ));
    if list.is_empty() {
        out.push_str(&format!(
            "<p>{}</p></div>",
            escape(&t(cx.i18n, "admin.web_hooks.events.none"))
        ));
        return out;
    }
    out.push_str(
        "<div><div class=\"web-hook-events content-list\"><div class=\"heading-container\">",
    );
    for (class, key) in [
        ("first status", "status"),
        ("event-id", "event_id"),
        ("timestamp", "timestamp"),
        ("completion", "completion"),
        ("actions", "actions"),
    ] {
        out.push_str(&format!(
            "<div class=\"col heading {class}\">{}</div>",
            escape(&t(cx.i18n, &format!("admin.web_hooks.events.{key}")))
        ));
    }
    out.push_str("</div><ul>");
    for e in &list {
        out.push_str(&event(cx, e));
    }
    let total = events["total_rows_web_hook_events"].as_i64().unwrap_or(0);
    out.push_str(&format!(
        "</ul></div><div class=\"loading-container\"></div><div class=\"load-more-sentinel\" aria-hidden=\"true\"{}></div></div></div>",
        if (list.len() as i64) < total {
            format!(
                " data-load-more=\"{}\"",
                escape(s(&events["load_more_web_hook_events"]))
            )
        } else {
            String::new()
        }
    ));
    out
}

/// WebhookEvent: the row, with its request and response for the script.
pub fn event(cx: &AdminCx, e: &Value) -> String {
    let status = e["status"].as_i64().unwrap_or(0);
    let class = if status == 0 {
        ""
    } else if (200..=299).contains(&status) {
        "text-successful"
    } else {
        "text-danger"
    };
    let created = crate::admin_ui::time(&e["created_at"]);
    // completion: the duration in milliseconds, to the hundredth of a
    // second below.
    let seconds = (e["duration"].as_f64().unwrap_or(0.0) / 10.0).floor() / 100.0;
    let seconds_text = crate::admin_ui::js_number(seconds);
    // I18n's `one` only for exactly 1.
    let completion = t_count(
        cx.i18n,
        "admin.web_hooks.events.completed_in",
        if seconds == 1.0 { 1 } else { 2 },
        &[("count", &seconds_text)],
    );
    let button = |class: &str, icon_name: &str, key: &str| {
        Button {
            class,
            icon: Some(icon_name),
            label: tr(cx, key),
            ..Default::default()
        }
        .render()
    };
    format!(
        "<li data-event=\"{}\"><div class=\"col first status\"><span class=\"{class}\">{status}</span></div><div class=\"col event-id\">{}</div><div class=\"col timestamp\" data-time=\"{}\"{}>{}</div><div class=\"col completion\">{}</div><div class=\"col actions\">{}{}{}</div></li>",
        escape(&e.to_string()),
        e["id"].as_i64().unwrap_or(0),
        created.map(|c| c.timestamp_millis()).unwrap_or(0),
        // moment(created_at).format(...) in the viewer's zone, drawn
        // before the page paints (static/js/dates.js).
        created
            .map(|c| crate::viewer_dates::tz_attr(
                "text",
                &crate::viewer_dates::marker(c, "YYYY-MM-DD HH:mm:ss")
            ))
            .unwrap_or_default(),
        created
            .map(|c| c.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_default(),
        escape(&completion),
        button(
            "btn-default web-hook-event__request",
            "ellipsis-vertical",
            "admin.web_hooks.events.request"
        ),
        button(
            "btn-default web-hook-event__response",
            "ellipsis-vertical",
            "admin.web_hooks.events.response"
        ),
        button(
            "btn-default web-hook-event__redeliver",
            "arrows-rotate",
            "admin.web_hooks.events.redeliver"
        ),
    )
}

/// The strings admin-web-hooks.js shows.
pub fn strings(cx: &AdminCx) -> String {
    crate::admin_ui::strings(
        cx,
        "admin-web-hooks-strings",
        &[
            "admin.web_hooks.delete_confirm",
            "admin.web_hooks.events.headers",
            "admin.web_hooks.events.payload",
            "admin.web_hooks.events.body",
            "admin.web_hooks.events.redeliver_confirm",
            "admin.web_hooks.events.redeliver_failed_confirm.one",
            "admin.web_hooks.events.redeliver_failed_confirm.other",
            "admin.web_hooks.events.no_events_to_redeliver",
            "admin.web_hooks.events.incoming.one",
            "admin.web_hooks.events.incoming.other",
            "admin.web_hooks.secret_too_short",
            "admin.web_hooks.secret_invalid",
            "form_kit.errors.required",
            "form_kit.errors.invalid_url",
            "form_kit.errors.too_short.one",
            "form_kit.errors.too_short.other",
            "multi_select.search",
            "multi_select.no_results",
            "delete",
            "cancel",
            "generic_error",
        ],
    )
}
