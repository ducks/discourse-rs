//! The MCP server pages (90g6), as Ember draws them: the config page's
//! DPageHeader with its tabs over AdminMcp's sections (the overview, the
//! capabilities' primitive picker, the group access rules and their form,
//! the OAuth clients with the new client's presets and a client's
//! details, the authorizations, the activity) and the settings tab. Each
//! section's model rides along as JSON for static/js/admin-mcp.js, which
//! filters, saves and loads more.

use serde_json::{Value, json};

use crate::admin_ui::{Button, tr};
use crate::admin_view::{AdminCx, Crumb, PageHeader, t, t_with};
use crate::topic_list_view::{escape, icon};

/// The section a route draws.
pub enum Section {
    Overview(Value),
    Capabilities(Value),
    Access(Value),
    /// The access form: the access model, the rule edited (None: new),
    /// and the groups (id, name) a new rule may name.
    AccessForm(Value, Option<Value>, Vec<(i64, String)>),
    Clients(Value),
    ClientNew,
    ClientDetail(Value),
    Authorizations(Value),
    Activity(Value),
    /// AdminAreaSettings for the mcp area, drawn by the caller.
    Settings(String),
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

fn k(cx: &AdminCx, key: &str) -> String {
    t(cx.i18n, &format!("admin.config.mcp.{key}"))
}

/// `mcpValue(group, value)`
fn value(cx: &AdminCx, group: &str, v: &str) -> String {
    k(cx, &format!("values.{group}.{v}"))
}

/// AdminConfigAreaCard with its heading.
fn card(cx: &AdminCx, class: &str, heading: &str, content: &str) -> String {
    format!(
        "<section class=\"admin-config-area-card{}\"><div class=\"admin-config-area-card__header-wrapper\"><h3 class=\"admin-config-area-card__title\">{}</h3></div><div class=\"admin-config-area-card__content\">{content}</div></section>",
        if class.is_empty() {
            String::new()
        } else {
            format!(" {class}")
        },
        escape(&t(cx.i18n, heading))
    )
}

/// A `<dl>` of (term, definition HTML) pairs.
fn dl(class: &str, items: &[(String, String)]) -> String {
    format!(
        "<dl class=\"{class}\">{}</dl>",
        items
            .iter()
            .map(|(dt, dd)| format!("<div><dt>{}</dt><dd>{dd}</dd></div>", escape(dt)))
            .collect::<String>()
    )
}

fn subheader(
    cx: &AdminCx,
    title: &str,
    description: &str,
    actions: &str,
    learn_more: Option<&str>,
) -> String {
    let mut description = escape(description);
    if let Some(url) = learn_more {
        description.push_str(&format!(
            " <span class=\"d-page-subheader__learn-more\">{}</span>",
            t_with(cx.i18n, "learn_more_with_link", &[("url", url)])
        ));
    }
    crate::admin_ui::subheader(title, actions, Some(&description))
}

/// A subheader's DPageActionButtons, (kind, label) each: the More options
/// menu in the mobile view.
fn actions(cx: &AdminCx, list: &[(&'static str, String)]) -> String {
    let list: Vec<crate::admin_view::PageAction> = list
        .iter()
        .map(|(kind, label)| crate::admin_view::PageAction {
            class: String::new(),
            kind,
            label: label.clone(),
            title: None,
        })
        .collect();
    crate::admin_view::page_actions(cx, "d-page-subheader", &list)
}

/// BackButton on a route below the one it links to (its LinkTo active).
fn back(cx: &AdminCx, path: &str, key: &str) -> String {
    crate::admin_ui::back_button(cx, path, key).replacen("class=\"btn ", "class=\"active btn ", 1)
}

fn time(cx: &AdminCx, v: &Value) -> Option<String> {
    crate::admin_ui::time(v).map(|at| crate::admin_ui::format_date(cx, at, false))
}

fn never(cx: &AdminCx) -> String {
    escape(&k(cx, "never"))
}

fn status(cx: &AdminCx, group: &str, state: &str) -> String {
    format!(
        "<span class=\"admin-mcp__status\" data-state=\"{}\">{}</span>",
        escape(state),
        escape(&value(cx, group, state))
    )
}

fn mobile_label(text: &str) -> String {
    format!(
        "<div class=\"d-table__mobile-label\">{}</div>",
        escape(text)
    )
}

/// A FormKit form's text filter, its field unlabeled.
fn filter_form(class: &str, name: &str, placeholder: &str, full: bool) -> String {
    format!(
        "<form class=\"form-kit {class}\" novalidate=\"\">{}</form>",
        filter_field(name, placeholder, full, "")
    )
}

fn filter_field(name: &str, placeholder: &str, full: bool, class: &str) -> String {
    crate::admin_form_kit::Field {
        name,
        control: "input",
        format: full.then_some("full"),
        inner: crate::admin_form_kit::input(name, "", "text", Some(placeholder), false),
        ..Default::default()
    }
    .render()
    .replacen(
        "class=\"form-kit__container",
        &format!(
            "class=\"{}form-kit__container",
            if class.is_empty() {
                String::new()
            } else {
                format!("{class} ")
            }
        ),
        1,
    )
}

/// An unlabeled FormKit select (aria-labelled).
fn select_field(
    name: &str,
    label: &str,
    options: &[(String, String)],
    selected: &str,
    full: bool,
    class: &str,
) -> String {
    // The selected option is the --selected one (a property, no attribute).
    let select = crate::admin_form_kit::select(name, options, selected, None, false)
        .replace(" selected=\"\"", "")
        .replacen(
            "<select ",
            &format!("<select aria-label=\"{}\" ", escape(label)),
            1,
        );
    crate::admin_form_kit::Field {
        name,
        control: "select",
        format: full.then_some("full"),
        inner: select,
        ..Default::default()
    }
    .render()
    .replacen(
        "class=\"form-kit__container",
        &format!(
            "class=\"{}form-kit__container",
            if class.is_empty() {
                String::new()
            } else {
                format!("{class} ")
            }
        ),
        1,
    )
}

/// DLoadMore around a table, DConditionalLoadingSpinner after.
fn load_more(table: &str) -> String {
    format!(
        "<div class=\"admin-mcp__load-more\">{table}<div aria-hidden=\"true\" class=\"load-more-sentinel\"></div></div><div class=\"loading-container\"></div>"
    )
}

fn header_cells(cells: &[(&str, String)]) -> String {
    format!(
        "<thead class=\"d-table__header\"><tr class=\"d-table__row\">{}</tr></thead>",
        cells
            .iter()
            .map(|(kind, label)| if *kind == "--controls" {
                format!(
                    "<th class=\"d-table__cell --controls\"><span class=\"sr-only\">{}</span></th>",
                    escape(label)
                )
            } else {
                format!("<th class=\"d-table__cell {kind}\">{}</th>", escape(label))
            })
            .collect::<String>()
    )
}

fn overview(cx: &AdminCx, m: &Value) -> String {
    let endpoint = format!(
        "<div class=\"admin-mcp__endpoint\"><code>{}</code>{}</div>{}",
        escape(s(&m["endpoint"])),
        Button {
            class: "btn-transparent",
            icon: Some("copy"),
            title: tr(cx, "admin.config.mcp.copy_endpoint"),
            ..Default::default()
        }
        .render(),
        dl(
            "admin-mcp__summary-list",
            &[
                (
                    k(cx, "overview.protocol"),
                    escape(s(&m["protocol_version"]))
                ),
                (
                    k(cx, "overview.server_version"),
                    escape(s(&m["server_version"]))
                ),
                (
                    k(cx, "overview.status"),
                    escape(&value(cx, "server_status", s(&m["status"]))),
                ),
            ]
        )
    );
    let catalog = &m["catalog"];
    let metrics = &m["metrics"];
    let n = |v: &Value| escape(&v.to_string());
    let mut grid = format!(
        "{}{}{}",
        card(
            cx,
            "admin-mcp__endpoint-card",
            "admin.config.mcp.overview.endpoint_title",
            &endpoint
        ),
        card(
            cx,
            "",
            "admin.config.mcp.overview.catalog_title",
            &dl(
                "admin-mcp__metric-grid",
                &[
                    (k(cx, "overview.tools"), n(&catalog["tools"])),
                    (k(cx, "overview.resources"), n(&catalog["resources"])),
                    (k(cx, "overview.prompts"), n(&catalog["prompts"])),
                    (
                        k(cx, "overview.schema_size"),
                        escape(&t_with(
                            cx.i18n,
                            "admin.config.mcp.overview.schema_size_value",
                            &[("bytes", &catalog["schema_bytes"].to_string())],
                        )),
                    ),
                ]
            )
        ),
        card(
            cx,
            "admin-mcp__usage",
            "admin.config.mcp.overview.activity_title",
            &dl(
                "admin-mcp__metric-grid",
                &[
                    (
                        k(cx, "overview.approved_oauth_clients"),
                        n(&metrics["approved_oauth_clients"]),
                    ),
                    (
                        k(cx, "overview.authorizations"),
                        n(&metrics["authorizations"])
                    ),
                    (k(cx, "overview.tokens"), n(&metrics["tokens"])),
                    (k(cx, "overview.errors"), n(&metrics["errors"])),
                ]
            )
        )
    );
    let checklist = m["setup_checklist"].as_array().cloned().unwrap_or_default();
    if checklist.iter().any(|i| i["complete"] != true) {
        let items: String = checklist
            .iter()
            .map(|i| {
                let done = i["complete"] == true;
                format!(
                    "<li{}>{}<span>{}</span></li>",
                    if done { " class=\"is-complete\"" } else { "" },
                    icon(if done { "circle-check" } else { "circle" }, None),
                    escape(s(&i["label"]))
                )
            })
            .collect();
        grid.push_str(&card(
            cx,
            "admin-mcp__setup",
            "admin.config.mcp.overview.setup_title",
            &format!("<ol class=\"admin-mcp__checklist\">{items}</ol>"),
        ));
    }
    format!(
        "{}<div class=\"admin-mcp__overview-grid\">{grid}</div>",
        subheader(
            cx,
            &k(cx, "overview.title"),
            &k(cx, "overview.description"),
            "",
            None
        )
    )
}

fn access(cx: &AdminCx, m: &Value) -> String {
    let rows: String = m["access_rules"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|rule| {
            let scopes: String = rule["scopes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|scope| format!("<li><code>{}</code></li>", escape(s(scope))))
                .collect();
            format!(
                "<tr class=\"d-table__row admin-mcp__access-row\" data-group-id=\"{}\"><td class=\"d-table__cell --overview\"><span class=\"d-table__overview-name\">{}</span>{}</td><td class=\"d-table__cell --detail\">{}<ul class=\"admin-mcp__access-scopes\">{scopes}</ul></td><td class=\"d-table__cell --controls\">{}</td><td class=\"d-table__cell --controls\">{}</td></tr>",
                rule["group_id"],
                escape(s(&rule["group_name"])),
                if rule["pre_registered"] == true {
                    format!("<small>{}</small>", escape(&k(cx, "access.admins_description")))
                } else {
                    String::new()
                },
                mobile_label(&k(cx, "access.scopes")),
                Button {
                    class: "btn-small admin-mcp__edit-access-rule",
                    icon: Some("pencil"),
                    label: tr(cx, "admin.config.mcp.access.edit"),
                    ..Default::default()
                }
                .render(),
                if rule["deletable"] == true {
                    Button {
                        class: "btn-danger btn-small admin-mcp__delete-access-rule",
                        icon: Some("trash-can"),
                        title: tr(cx, "admin.config.mcp.access.delete"),
                        ..Default::default()
                    }
                    .render()
                } else {
                    String::new()
                }
            )
        })
        .collect();
    format!(
        "<section class=\"admin-mcp__access-section\">{}<table class=\"d-table admin-mcp__access-table\">{}<tbody class=\"d-table__body\">{rows}</tbody></table></section>",
        subheader(
            cx,
            &k(cx, "access.title"),
            &k(cx, "access.description"),
            &actions(cx, &[("primary", k(cx, "access.add"))]),
            None
        ),
        header_cells(&[
            ("--overview", k(cx, "access.groups")),
            ("--detail", k(cx, "access.scopes")),
            ("--controls", k(cx, "access.edit")),
            ("--controls", k(cx, "access.delete")),
        ])
    )
}

fn access_form(cx: &AdminCx, m: &Value, rule: Option<&Value>, groups: &[(i64, String)]) -> String {
    let initial = s(&m["initial_scope"]).to_string();
    let group = match rule {
        Some(rule) => format!(
            "<div class=\"form-kit__field\"><span class=\"form-kit__label\">{}</span><strong>{}</strong></div>",
            escape(&k(cx, "access.group")),
            escape(s(&rule["group_name"]))
        ),
        None => {
            let _ = groups;
            crate::admin_form_kit::Field {
                name: "group_ids",
                control: "custom",
                title: Some(k(cx, "access.group")),
                inner: format!(
                    "<div class=\"form-kit__control-custom\">{}</div>",
                    crate::admin_about_view::chooser(
                        cx,
                        "group-chooser",
                        "group_ids",
                        &[],
                        Some(1)
                    )
                ),
                ..Default::default()
            }
            .render()
        }
    };
    let mut scopes: Vec<String> = rule
        .and_then(|r| r["scopes"].as_array())
        .map(|l| l.iter().map(|v| s(v).to_string()).collect())
        .unwrap_or_default();
    if !initial.is_empty() && !scopes.contains(&initial) {
        scopes.push(initial.clone());
    }
    let selection: Vec<(String, String)> = scopes.iter().map(|v| (v.clone(), v.clone())).collect();
    let scopes_field = crate::admin_form_kit::Field {
        name: "scopes",
        control: "custom",
        format: Some("full"),
        title: Some(k(cx, "access.scopes")),
        description: Some(k(cx, "access.scopes_description")),
        // The initial scope can't be removed (preventRemoval).
        inner: format!(
            "<div class=\"form-kit__control-custom\">{}</div>",
            crate::admin_ui::multi_select_trigger(
                "admin-mcp__scope-select",
                &k(cx, "access.scopes_placeholder"),
                &selection
            )
            .replacen(
                &format!(
                    "{}</span>{}</button>",
                    escape(&initial),
                    icon(
                        "xmark",
                        Some("d-multi-select-trigger__remove-selection-icon")
                    )
                ),
                &format!("{}</span></button>", escape(&initial)),
                1
            )
        ),
        ..Default::default()
    }
    .render();
    let form = format!(
        "<form class=\"form-kit admin-mcp__access-form\" novalidate=\"\">{group}{scopes_field}<div class=\"admin-mcp__access-form-actions\"><button class=\"btn btn-primary form-kit__button\" type=\"submit\"><span class=\"d-button-label\">{}</span></button></div></form>",
        escape(&k(cx, "access.save"))
    );
    format!(
        "{}{}",
        back(
            cx,
            "/admin/config/mcp/access",
            "admin.config.mcp.access.back"
        ),
        card(
            cx,
            "admin-mcp__form-card",
            if rule.is_some() {
                "admin.config.mcp.access.edit_title"
            } else {
                "admin.config.mcp.access.new_title"
            },
            &form
        )
    )
}

/// The primitive groups by the group-by option (scope, provider, kind):
/// (id, label, enabled, total), All first.
pub fn primitive_groups(
    cx: &AdminCx,
    m: &Value,
    group_by: &str,
) -> Vec<(String, String, usize, usize)> {
    let primitives = m["primitives"].as_array().cloned().unwrap_or_default();
    let values_of = |p: &Value| -> Vec<String> {
        match group_by {
            "provider" => p["provider"]
                .as_str()
                .map(|v| vec![v.to_string()])
                .unwrap_or_default(),
            "kind" => p["kind"]
                .as_str()
                .map(|v| vec![v.to_string()])
                .unwrap_or_default(),
            _ => p["required_scopes"]
                .as_array()
                .map(|l| l.iter().map(|v| s(v).to_string()).collect())
                .unwrap_or_default(),
        }
    };
    let mut ids: Vec<String> = Vec::new();
    match group_by {
        "provider" | "kind" => {
            for p in &primitives {
                for v in values_of(p) {
                    if !ids.contains(&v) {
                        ids.push(v);
                    }
                }
            }
        }
        _ => {
            for scope in m["available_scopes"].as_array().into_iter().flatten() {
                let scope = s(scope).to_string();
                if primitives.iter().any(|p| values_of(p).contains(&scope)) {
                    ids.push(scope);
                }
            }
        }
    }
    let mut out = Vec::new();
    let count = |list: &[&Value]| {
        (
            list.iter().filter(|p| p["enabled"] == true).count(),
            list.len(),
        )
    };
    let all: Vec<&Value> = primitives.iter().collect();
    let (enabled, total) = count(&all);
    out.push((
        "all".to_string(),
        k(cx, "primitives.all_primitives"),
        enabled,
        total,
    ));
    for id in ids {
        let members: Vec<&Value> = primitives
            .iter()
            .filter(|p| values_of(p).contains(&id))
            .collect();
        let (enabled, total) = count(&members);
        let label = if group_by == "kind" {
            value(cx, "primitive_kind", &id)
        } else {
            id.clone()
        };
        out.push((id, label, enabled, total));
    }
    out
}

/// One primitive's article.
pub fn primitive(cx: &AdminCx, p: &Value) -> String {
    let available = p["available"] == true;
    let blocked = p["emergency_blocked"] == true;
    let field = s(&p["field_name"]);
    let checkbox = crate::admin_form_kit::checkbox(
        field,
        p["enabled"] == true,
        Some(s(&p["title"])),
        None,
        !available || blocked,
    )
    // No label target, and the checked state is set as a property.
    .replacen(&format!(" for=\"control-{field}-input\""), "", 1)
    .replacen(" checked=\"\"", "", 1)
    .replacen(
        "<span class=\"form-kit__control-checkbox-description\"></span>",
        &format!(
            "<span class=\"form-kit__control-checkbox-description\">{}</span>",
            escape(s(&p["description"]))
        ),
        1,
    );
    let scopes: Vec<&str> = p["required_scopes"]
        .as_array()
        .map(|l| l.iter().map(s).collect())
        .unwrap_or_default();
    format!(
        "<article class=\"admin-mcp__primitive\" data-primitive-id=\"{}\">{}{}{}{}<div class=\"admin-mcp__primitive-actions\">{}</div></article>",
        escape(s(&p["id"])),
        crate::admin_form_kit::Field {
            name: field,
            control: "checkbox",
            inner: checkbox,
            ..Default::default()
        }
        .render(),
        dl(
            "admin-mcp__primitive-meta",
            &[
                (k(cx, "primitives.provider"), escape(s(&p["provider"]))),
                (
                    k(cx, "primitives.kind"),
                    escape(&value(cx, "primitive_kind", s(&p["kind"])))
                ),
                (
                    k(cx, "primitives.risk"),
                    escape(&value(cx, "primitive_risk", s(&p["risk"])))
                ),
                (k(cx, "primitives.scopes"), escape(&scopes.join(", "))),
            ]
        ),
        if available {
            String::new()
        } else {
            "<p class=\"admin-mcp__unavailable\"></p>".to_string()
        },
        if blocked {
            format!(
                "<p class=\"admin-mcp__unavailable\">{}</p>",
                escape(&k(cx, "primitives.emergency_blocked"))
            )
        } else {
            String::new()
        },
        Button {
            class: "btn-transparent btn-small",
            icon: Some(if blocked { "unlock" } else { "ban" }),
            title: tr(
                cx,
                if blocked {
                    "admin.config.mcp.actions.unblock_primitive"
                } else {
                    "admin.config.mcp.actions.block_primitive"
                }
            ),
            ..Default::default()
        }
        .render()
    )
}

fn capabilities(cx: &AdminCx, m: &Value) -> String {
    let primitives = m["primitives"].as_array().cloned().unwrap_or_default();
    let opts = |pairs: Vec<(String, String)>| pairs;
    let group_by = opts(
        ["scope", "provider", "kind"]
            .iter()
            .map(|g| {
                (
                    g.to_string(),
                    k(cx, &format!("primitives.group_by_options.{g}")),
                )
            })
            .collect(),
    );
    let mut risks = vec!["all".to_string()];
    for p in &primitives {
        let r = s(&p["risk"]).to_string();
        if !r.is_empty() && !risks.contains(&r) {
            risks.push(r);
        }
    }
    let risks: Vec<(String, String)> = risks
        .iter()
        .map(|r| (r.clone(), value(cx, "primitive_risk", r)))
        .collect();
    let states: Vec<(String, String)> = ["all", "enabled", "disabled", "unavailable", "blocked"]
        .iter()
        .map(|st| (st.to_string(), value(cx, "primitive_state", st)))
        .collect();
    let filters = format!(
        "<form class=\"form-kit admin-mcp__primitive-filters\" novalidate=\"\">{}{}{}{}</form>",
        filter_field(
            "primitiveFilter",
            &k(cx, "primitives.search_placeholder"),
            true,
            "admin-mcp__filter-search"
        ),
        select_field(
            "primitiveGroupBy",
            &k(cx, "primitives.group_by"),
            &group_by,
            "scope",
            true,
            ""
        ),
        select_field(
            "primitiveRisk",
            &k(cx, "primitives.risk"),
            &risks,
            "all",
            true,
            "admin-mcp__filter-impact"
        ),
        select_field(
            "primitiveState",
            &k(cx, "primitives.state"),
            &states,
            "all",
            true,
            ""
        ),
    );
    let groups: String = primitive_groups(cx, m, "scope")
        .iter()
        .map(|(id, label, enabled, total)| {
            let selected = id == "all";
            format!(
                "<li><button{} class=\"admin-mcp__primitive-group{}\" data-primitive-group-id=\"{}\" type=\"button\"><span>{}</span> <small>{}</small></button></li>",
                if selected { " aria-pressed=\"\"" } else { "" },
                if selected { " is-selected" } else { "" },
                escape(id),
                escape(label),
                escape(&t_with(
                    cx.i18n,
                    "admin.config.mcp.primitives.group_count",
                    &[("enabled", &enabled.to_string()), ("total", &total.to_string())]
                ))
            )
        })
        .collect();
    let list: String = if primitives.is_empty() {
        format!(
            "<p class=\"admin-mcp__empty\">{}</p>",
            escape(&k(cx, "primitives.no_results"))
        )
    } else {
        primitives.iter().map(|p| primitive(cx, p)).collect()
    };
    let total = primitives.len().to_string();
    let selection = format!(
        "<form class=\"form-kit admin-mcp__primitive-selection-form\" novalidate=\"\"><div class=\"admin-mcp__primitive-browser\"><nav aria-label=\"{}\" class=\"admin-mcp__primitive-groups\"><ul class=\"admin-mcp__primitive-group-list\">{groups}</ul></nav><section aria-labelledby=\"mcp-primitive-group-title\" class=\"admin-mcp__primitive-panel\"><header class=\"admin-mcp__primitive-panel-header\"><h3 id=\"mcp-primitive-group-title\">{}</h3><p class=\"admin-mcp__results-count\">{}</p><div class=\"admin-mcp__primitive-group-actions\">{}{}</div></header><div class=\"admin-mcp__primitive-list\">{list}</div></section></div><div class=\"form-kit__section form-kit__actions\"><button class=\"btn btn-primary form-kit__button\" type=\"submit\"><span class=\"d-button-label\">{}</span></button></div></form>",
        escape(&k(cx, "primitives.groups_label")),
        escape(&k(cx, "primitives.all_primitives")),
        escape(&t_with(
            cx.i18n,
            "admin.config.mcp.primitives.results_count",
            &[("visible", &total), ("total", &total)]
        )),
        Button {
            class: "btn-default btn-small",
            label: tr(cx, "admin.config.mcp.primitives.enable_visible"),
            ..Default::default()
        }
        .render(),
        Button {
            class: "btn-default btn-small",
            label: tr(cx, "admin.config.mcp.primitives.disable_visible"),
            ..Default::default()
        }
        .render(),
        escape(&k(cx, "save_primitives"))
    );
    format!(
        "{}<div class=\"admin-mcp__primitives-section\"><p class=\"admin-mcp__primitives-description\">{}</p>{filters}{selection}</div>",
        subheader(
            cx,
            &k(cx, "capabilities.title"),
            &k(cx, "capabilities.description"),
            "",
            None
        ),
        escape(&k(cx, "primitives.picker_description"))
    )
}

/// One client's row.
pub fn client_row(cx: &AdminCx, c: &Value) -> String {
    let blocked = c["blocked"] == true;
    format!(
        "<tr class=\"d-table__row\"><td class=\"d-table__cell --overview\"><a class=\"d-table__overview-link\" href=\"{}/admin/config/mcp/clients/{}\"><span class=\"d-table__overview-name\">{}</span><small>{}</small></a></td><td class=\"d-table__cell --detail\">{}{}</td><td class=\"d-table__cell --detail\">{}{}</td><td class=\"d-table__cell --detail\">{}{}</td><td class=\"d-table__cell --controls\">{}<div class=\"d-table__cell-actions\">{}{}</div></td></tr>",
        cx.base_path,
        c["id"],
        escape(s(&c["name"])),
        escape(s(&c["client_id"])),
        mobile_label(&k(cx, "clients.trust")),
        status(cx, "client_trust", s(&c["trust_state"])),
        mobile_label(&k(cx, "clients.last_seen")),
        time(cx, &c["last_seen_at"]).unwrap_or_else(|| never(cx)),
        mobile_label(&k(cx, "clients.authorizations")),
        escape(&c["authorization_count"].to_string()),
        mobile_label(&k(cx, "clients.actions")),
        if s(&c["registration_type"]) == "cimd" {
            Button {
                class: "btn-small btn-transparent --primary admin-mcp__refresh-client",
                label: tr(cx, "admin.config.mcp.actions.refresh_metadata"),
                ..Default::default()
            }
            .render()
        } else {
            String::new()
        },
        Button {
            class: if blocked {
                "btn-small btn-default admin-mcp__toggle-client-block"
            } else {
                "btn-small btn-danger admin-mcp__toggle-client-block"
            },
            label: tr(
                cx,
                if blocked {
                    "admin.config.mcp.clients.unblock"
                } else {
                    "admin.config.mcp.clients.block"
                }
            ),
            ..Default::default()
        }
        .render()
    )
}

/// The clients' rows, or the no results row.
pub fn client_rows(cx: &AdminCx, clients: &[Value]) -> String {
    if clients.is_empty() {
        return format!(
            "<tr class=\"d-table__row\"><td class=\"d-table__cell\" colspan=\"5\">{}</td></tr>",
            escape(&k(cx, "clients.no_results"))
        );
    }
    clients.iter().map(|c| client_row(cx, c)).collect()
}

fn clients(cx: &AdminCx, m: &Value) -> String {
    let list = m["clients"].as_array().cloned().unwrap_or_default();
    let head = subheader(
        cx,
        &k(cx, "clients.title"),
        &k(cx, "clients.description"),
        &actions(cx, &[("primary", k(cx, "clients.add"))]),
        Some(
            "https://meta.discourse.org/t/connecting-your-apps-to-discourse-mcp-server/411637#p-2033870-connecting-apps-via-oauth-2",
        ),
    );
    if list.is_empty() {
        return format!(
            "{head}{}",
            crate::admin_ui::empty_list(
                cx,
                "",
                "admin.config.mcp.clients.add",
                "/admin/config/mcp/clients/new",
                "admin.config.mcp.clients.empty"
            )
        );
    }
    let table = format!(
        "<table class=\"d-table admin-mcp__table\">{}<tbody class=\"d-table__body\">{}</tbody></table>",
        header_cells(&[
            ("--overview", k(cx, "clients.client")),
            ("--detail", k(cx, "clients.trust")),
            ("--detail", k(cx, "clients.last_seen")),
            ("--detail", k(cx, "clients.authorizations")),
            ("--controls", k(cx, "clients.actions")),
        ]),
        client_rows(cx, &list)
    );
    format!(
        "{head}{}{}",
        filter_form(
            "admin-mcp__table-filter",
            "clientFilter",
            &k(cx, "clients.search_placeholder"),
            false
        ),
        load_more(&table)
    )
}

/// The presets ClientNew offers, in order.
pub const CLIENT_PRESETS: [&str; 6] = [
    "codex",
    "chatgpt",
    "claude_code",
    "mcp_inspector",
    "visual_studio_code",
    "custom",
];

fn client_new(cx: &AdminCx) -> String {
    let items: String = CLIENT_PRESETS
        .iter()
        .map(|p| {
            format!(
                "<div class=\"admin-section-landing-item\" data-client-preset-id=\"{p}\"><div class=\"admin-section-landing-item__content\"><h3 class=\"admin-section-landing-item__title\">{}</h3><p class=\"admin-section-landing-item__description\">{}</p></div><div class=\"admin-section-landing-item__buttons\">{}</div></div>",
                escape(&k(cx, &format!("clients.presets.{p}"))),
                escape(&k(cx, &format!("clients.preset_descriptions.{p}"))),
                Button {
                    class: "btn-default btn-small d-page-action-button",
                    icon: Some("gear"),
                    label: tr(cx, "admin.config.mcp.clients.use_preset"),
                    ..Default::default()
                }
                .render()
            )
        })
        .collect();
    format!(
        "{}<section class=\"admin-mcp__client-presets\"><h2>{}</h2><p>{}</p><div class=\"admin-section-landing-wrapper\">{items}</div></section>",
        back(
            cx,
            "/admin/config/mcp/clients",
            "admin.config.mcp.clients.back"
        ),
        escape(&k(cx, "clients.preset_title")),
        escape(&k(cx, "clients.preset_description"))
    )
}

/// The new client's form for a preset (name, client ID, redirect URIs),
/// as admin-mcp.js swaps it in.
pub fn client_form(cx: &AdminCx, preset: &str, name: &str, client_id: &str, uris: &str) -> String {
    let field = |n: &str, control: &str, title: &str, description: &str, inner: String| {
        crate::admin_form_kit::Field {
            name: n,
            control,
            format: Some("large"),
            title: Some(k(cx, title)),
            description: Some(k(cx, description)),
            inner,
            ..Default::default()
        }
        .render()
    };
    format!(
        "{}{}",
        Button {
            class: "btn-transparent back-button admin-mcp__change-client-preset",
            icon: Some("chevron-left"),
            label: tr(cx, "admin.config.mcp.clients.back_to_presets"),
            ..Default::default()
        }
        .render(),
        card(
            cx,
            "admin-mcp__client-form",
            "admin.config.mcp.clients.new_title",
            &format!(
                "<p>{}</p><form class=\"form-kit\" novalidate=\"\">{}{}{}<button class=\"btn btn-primary form-kit__button\" type=\"submit\"><span class=\"d-button-label\">{}</span></button></form>",
                escape(&k(cx, &format!("clients.preset_help.{preset}"))),
                field(
                    "name",
                    "input",
                    "clients.name",
                    "clients.name_description",
                    crate::admin_form_kit::input("name", name, "text", None, false)
                ),
                field(
                    "client_id",
                    "input",
                    "clients.client_id",
                    "clients.client_id_description",
                    crate::admin_form_kit::input("client_id", client_id, "text", None, false)
                ),
                field(
                    "redirect_uris",
                    "textarea",
                    "clients.redirect_uris",
                    "clients.redirect_uris_description",
                    crate::admin_form_kit::textarea("redirect_uris", uris, false).replacen(
                        "<textarea ",
                        "<textarea style=\"height: 100px;\" ",
                        1
                    )
                ),
                escape(&k(cx, "clients.create"))
            )
        )
    )
}

/// `CLIENT_PRESETS`' values: (name, client ID, redirect URIs).
pub fn preset_values(preset: &str) -> (&'static str, &'static str, &'static str) {
    match preset {
        "codex" => ("Codex", "codex", "http://127.0.0.1/callback"),
        "chatgpt" => (
            "ChatGPT",
            "https://chatgpt.com/oauth/client.json",
            "https://chatgpt.com/connector_platform_oauth_redirect",
        ),
        "claude_code" => (
            "Claude Code",
            "claude-code",
            "http://localhost:8080/callback",
        ),
        "mcp_inspector" => (
            "MCP Inspector",
            "mcp-inspector",
            "http://localhost:6274/oauth/callback\nhttp://127.0.0.1:6276/oauth/callback",
        ),
        "visual_studio_code" => (
            "Visual Studio Code",
            "visual-studio-code",
            "http://127.0.0.1:33418\nhttps://vscode.dev/redirect",
        ),
        _ => ("", "", ""),
    }
}

/// The client details' subheader actions.
pub fn client_actions(cx: &AdminCx, c: &Value) -> String {
    let mut list = vec![(
        "default",
        k(
            cx,
            if c["blocked"] == true {
                "actions.unblock_client"
            } else {
                "actions.block_client"
            },
        ),
    )];
    if s(&c["registration_type"]) == "cimd" {
        list.push(("default", k(cx, "actions.refresh_metadata")));
    }
    actions(cx, &list)
}

/// The client details' card.
pub fn client_details(cx: &AdminCx, c: &Value) -> String {
    let mut items = vec![
        (
            k(cx, "clients.client_id"),
            format!("<code>{}</code>", escape(s(&c["client_id"]))),
        ),
        (
            k(cx, "clients.trust"),
            status(cx, "client_trust", s(&c["trust_state"])),
        ),
    ];
    if let Some(domain) = c["domain"].as_str().filter(|d| !d.is_empty()) {
        items.push((k(cx, "clients.domain"), escape(domain)));
    }
    items.push((
        k(cx, "clients.registration_type"),
        escape(&value(cx, "registration_type", s(&c["registration_type"]))),
    ));
    items.push((
        k(cx, "clients.redirect_uris"),
        format!(
            "<ul class=\"admin-mcp__uri-list\">{}</ul>",
            c["redirect_uris"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|u| format!("<li><code>{}</code></li>", escape(s(u))))
                .collect::<String>()
        ),
    ));
    items.push((
        k(cx, "clients.first_seen"),
        time(cx, &c["first_seen_at"]).unwrap_or_default(),
    ));
    items.push((
        k(cx, "clients.last_seen"),
        time(cx, &c["last_seen_at"]).unwrap_or_else(|| never(cx)),
    ));
    card(
        cx,
        "",
        "admin.config.mcp.clients.detail_title",
        &dl("admin-mcp__detail-list", &items),
    )
}

fn client_detail(cx: &AdminCx, m: &Value) -> String {
    let c = &m["client"];
    format!(
        "{}{}{}",
        back(
            cx,
            "/admin/config/mcp/clients",
            "admin.config.mcp.clients.back"
        ),
        subheader(
            cx,
            s(&c["name"]),
            &k(cx, "clients.detail_description"),
            &client_actions(cx, c),
            None
        ),
        client_details(cx, c)
    )
}

/// The authorizations' rows, or the empty row.
pub fn authorization_rows(cx: &AdminCx, list: &[Value]) -> String {
    if list.is_empty() {
        return format!(
            "<tr class=\"d-table__row\"><td class=\"d-table__cell\" colspan=\"6\">{}</td></tr>",
            escape(&k(cx, "authorizations.empty"))
        );
    }
    list.iter()
        .map(|a| {
            let scopes: Vec<&str> = a["scopes"].as_array().map(|l| l.iter().map(s).collect()).unwrap_or_default();
            format!(
                "<tr class=\"d-table__row\"><td class=\"d-table__cell --overview\"><span class=\"d-table__overview-name\">{}</span></td><td class=\"d-table__cell --detail\">{}{}</td><td class=\"d-table__cell --detail\">{}<code>{}</code></td><td class=\"d-table__cell --detail\">{}{}</td><td class=\"d-table__cell --detail\">{}{}</td><td class=\"d-table__cell --controls\">{}</td></tr>",
                escape(s(&a["client_name"])),
                mobile_label(&k(cx, "authorizations.user")),
                escape(s(&a["username"])),
                mobile_label(&k(cx, "authorizations.scopes")),
                escape(&scopes.join(", ")),
                mobile_label(&k(cx, "authorizations.last_used")),
                time(cx, &a["last_used_at"]).unwrap_or_else(|| never(cx)),
                mobile_label(&k(cx, "authorizations.status")),
                status(cx, "authorization_status", s(&a["status"])),
                if s(&a["status"]) == "revoked" {
                    String::new()
                } else {
                    Button {
                        class: "btn-small btn-danger",
                        label: tr(cx, "admin.config.mcp.actions.revoke"),
                        ..Default::default()
                    }
                    .render()
                }
            )
        })
        .collect()
}

fn authorizations(cx: &AdminCx, m: &Value) -> String {
    let list = m["authorizations"].as_array().cloned().unwrap_or_default();
    let cols: String = ["client", "user", "scopes", "last-used", "status", "actions"]
        .iter()
        .map(|c| format!("<col class=\"admin-mcp__authorizations-{c}-column\">"))
        .collect();
    let table = format!(
        "<table class=\"d-table admin-mcp__table admin-mcp__authorizations-table\"><colgroup>{cols}</colgroup>{}<tbody class=\"d-table__body\">{}</tbody></table>",
        header_cells(&[
            ("--overview", k(cx, "authorizations.client")),
            ("--detail", k(cx, "authorizations.user")),
            ("--detail", k(cx, "authorizations.scopes")),
            ("--detail", k(cx, "authorizations.last_used")),
            ("--detail", k(cx, "authorizations.status")),
            ("--controls", k(cx, "authorizations.actions")),
        ]),
        authorization_rows(cx, &list)
    );
    format!(
        "{}{}{}",
        subheader(
            cx,
            &k(cx, "authorizations.title"),
            &k(cx, "authorizations.description"),
            "",
            None
        ),
        filter_form(
            "admin-mcp__table-filter",
            "authorizationFilter",
            &k(cx, "authorizations.search_placeholder"),
            false
        ),
        load_more(&table)
    )
}

/// The activity metrics' grid (with entries only).
pub fn activity_metrics(cx: &AdminCx, metrics: &Value) -> String {
    let p95 = match metrics["p95_latency_ms"].as_i64() {
        Some(ms) => t_with(
            cx.i18n,
            "admin.config.mcp.activity.duration_value",
            &[("milliseconds", &ms.to_string())],
        ),
        None => k(cx, "activity.no_latency"),
    };
    let n = |v: &Value| escape(&v.to_string());
    format!(
        "<div class=\"admin-mcp__activity-metrics\"><dl class=\"admin-mcp__metric-grid\"><div><dt>{}</dt><dd>{}</dd></div><div><dt>{}</dt><dd class=\"admin-mcp__activity-failed-tool-calls\">{}</dd></div><div><dt>{}</dt><dd>{}</dd></div><div><dt>{}</dt><dd class=\"admin-mcp__activity-p95-latency\">{}</dd></div></dl></div>",
        escape(&k(cx, "activity.tool_calls")),
        n(&metrics["tool_calls"]),
        escape(&k(cx, "activity.failed_tool_calls")),
        n(&metrics["failed_tool_calls"]),
        escape(&k(cx, "activity.rate_limits")),
        n(&metrics["rate_limits"]),
        escape(&k(cx, "activity.p95_latency")),
        escape(&p95)
    )
}

/// The activity's rows, or the empty row.
pub fn activity_rows(cx: &AdminCx, list: &[Value]) -> String {
    if list.is_empty() {
        return format!(
            "<tr class=\"d-table__row\"><td class=\"d-table__cell\" colspan=\"6\">{}</td></tr>",
            escape(&k(cx, "activity.empty"))
        );
    }
    list.iter()
        .map(|e| {
            format!(
                "<tr class=\"d-table__row\"><td class=\"d-table__cell --detail\">{}{}</td><td class=\"d-table__cell --overview\"><span class=\"d-table__overview-name\">{}</span><small>{}</small></td><td class=\"d-table__cell --detail\">{}{}</td><td class=\"d-table__cell --detail\">{}{}</td><td class=\"d-table__cell --detail\">{}{}<!----> ms</td><td class=\"d-table__cell --detail\">{}<code>{}</code></td></tr>",
                mobile_label(&k(cx, "activity.time")),
                time(cx, &e["created_at"]).unwrap_or_default(),
                escape(s(&e["tool"])),
                escape(s(&e["method"])),
                mobile_label(&k(cx, "activity.user")),
                escape(s(&e["username"])),
                mobile_label(&k(cx, "activity.outcome")),
                status(cx, "activity_outcome", s(&e["outcome"])),
                mobile_label(&k(cx, "activity.duration")),
                e["duration_ms"].as_i64().map(|d| d.to_string()).unwrap_or_default(),
                mobile_label(&k(cx, "activity.request_id")),
                escape(s(&e["request_id"]))
            )
        })
        .collect()
}

fn activity(cx: &AdminCx, m: &Value, date_range: &str) -> String {
    let list = m["activity"].as_array().cloned().unwrap_or_default();
    let outcomes: Vec<(String, String)> = ["all", "success", "error", "rate_limited"]
        .iter()
        .map(|o| (o.to_string(), value(cx, "activity_outcome", o)))
        .collect();
    let filters = format!(
        "<form class=\"form-kit admin-mcp__activity-filters\" novalidate=\"\">{}{date_range}{}</form>",
        filter_field(
            "activityFilter",
            &k(cx, "activity.search_placeholder"),
            true,
            "admin-mcp__filter-search"
        ),
        select_field(
            "activityOutcome",
            &k(cx, "activity.outcome"),
            &outcomes,
            "all",
            false,
            "admin-mcp__activity-outcome"
        ),
    );
    let table = format!(
        "<table class=\"d-table admin-mcp__table admin-mcp__activity-table\">{}<tbody class=\"d-table__body\">{}</tbody></table>",
        header_cells(&[
            ("--detail", k(cx, "activity.time")),
            ("--overview", k(cx, "activity.method")),
            ("--detail", k(cx, "activity.user")),
            ("--detail", k(cx, "activity.outcome")),
            ("--detail", k(cx, "activity.duration")),
            ("--detail", k(cx, "activity.request_id")),
        ]),
        activity_rows(cx, &list)
    );
    format!(
        "{}{filters}{}{}",
        subheader(
            cx,
            &k(cx, "activity.title"),
            &k(cx, "activity.description"),
            "",
            None
        ),
        if list.is_empty() {
            String::new()
        } else {
            activity_metrics(cx, &m["metrics"])
        },
        load_more(&table)
    )
}

/// The config page's tabs, the active one by its route.
fn tabs(cx: &AdminCx, active: &str) -> String {
    let items = [
        ("overview", "", "admin.config.mcp.tabs.overview"),
        (
            "capabilities",
            "/capabilities",
            "admin.config.mcp.tabs.capabilities",
        ),
        ("access", "/access", "admin.config.mcp.tabs.access"),
        ("clients", "/clients", "admin.config.mcp.tabs.clients"),
        (
            "authorizations",
            "/authorizations",
            "admin.config.mcp.tabs.authorizations",
        ),
        ("activity", "/activity", "admin.config.mcp.tabs.activity"),
        ("settings", "/settings", "settings"),
    ];
    let classes: Vec<String> = items
        .iter()
        .map(|(id, _, _)| format!("admin-mcp-tabs__{id}"))
        .collect();
    let paths: Vec<String> = items
        .iter()
        .map(|(_, p, _)| format!("/admin/config/mcp{p}"))
        .collect();
    let list: Vec<(&str, &str, &str, bool)> = items
        .iter()
        .enumerate()
        .map(|(i, (id, _, key))| (classes[i].as_str(), paths[i].as_str(), *key, *id == active))
        .collect();
    // DNavItem gives an inactive link an empty class.
    crate::admin_ui::tabs(cx, &list).replace("\"><a href=", "\"><a class=\"\" href=")
}

/// The page: the header (none on the new and edit forms), the section.
/// `date_range` is the activity's DashboardDateRange trigger.
pub fn page(cx: &AdminCx, section: &Section, date_range: &str) -> String {
    let (active, body, header) = match section {
        Section::Overview(m) => ("overview", overview(cx, m), true),
        Section::Capabilities(m) => ("capabilities", capabilities(cx, m), true),
        Section::Access(m) => ("access", access(cx, m), true),
        Section::AccessForm(m, rule, groups) => {
            ("access", access_form(cx, m, rule.as_ref(), groups), false)
        }
        Section::Clients(m) => ("clients", clients(cx, m), true),
        Section::ClientNew => ("clients", client_new(cx), false),
        Section::ClientDetail(m) => ("clients", client_detail(cx, m), true),
        Section::Authorizations(m) => ("authorizations", authorizations(cx, m), true),
        Section::Activity(m) => ("activity", activity(cx, m, date_range), true),
        Section::Settings(outlet) => ("settings", outlet.clone(), true),
    };
    let header = if header {
        let title = k(cx, "title");
        PageHeader {
            crumbs: vec![
                Crumb {
                    label: t(cx.i18n, "admin_title"),
                    path: "/admin",
                },
                Crumb {
                    label: title.clone(),
                    path: "/admin/config/mcp",
                },
            ],
            title,
            actions: String::new(),
            description: Some(escape(&k(cx, "header_description"))),
            learn_more_url: None,
            tabs: Some(tabs(cx, active)),
        }
        .render(cx)
    } else {
        String::new()
    };
    format!(
        "{header}<div class=\"admin-container admin-config-page__main-area admin-mcp\">{body}</div>"
    )
}

/// What admin-mcp.js reads: the section's model and the groups.
pub fn data(section: &Section) -> Value {
    match section {
        Section::Overview(m)
        | Section::Capabilities(m)
        | Section::Access(m)
        | Section::Clients(m)
        | Section::ClientDetail(m)
        | Section::Authorizations(m)
        | Section::Activity(m) => m.clone(),
        Section::AccessForm(m, rule, groups) => json!({
            "model": m,
            "rule": rule,
            "groups": groups.iter().map(|(id, name)| json!({ "value": id.to_string(), "name": name })).collect::<Vec<_>>(),
        }),
        Section::ClientNew | Section::Settings(_) => json!({}),
    }
}

/// The strings admin-mcp.js shows.
pub const STRINGS: &[&str] = &[
    "admin.config.mcp.endpoint_copied",
    "admin.config.mcp.access.saved",
    "admin.config.mcp.access.confirm_delete",
    "admin.config.mcp.access.scope_primitives.one",
    "admin.config.mcp.access.scope_primitives.other",
    "admin.config.mcp.primitives_saved",
    "admin.config.mcp.confirm_emergency_block",
    "admin.config.mcp.primitive_blocked",
    "admin.config.mcp.primitive_unblocked",
    "admin.config.mcp.actions.block_primitive",
    "admin.config.mcp.actions.unblock_primitive",
    "admin.config.mcp.primitives.emergency_blocked",
    "form_kit.errors.required",
    "admin.config.mcp.primitives.results_count",
    "admin.config.mcp.primitives.group_count",
    "admin.config.mcp.primitives.all_primitives",
    "admin.config.mcp.primitives.no_results",
    "admin.config.mcp.clients.created",
    "admin.config.mcp.confirm_block_client",
    "admin.config.mcp.confirm_revoke_authorization",
    "admin.config.mcp.values.primitive_kind.tool",
    "admin.config.mcp.values.primitive_kind.resource",
    "admin.config.mcp.values.primitive_kind.resource_template",
    "admin.config.mcp.values.primitive_kind.prompt",
    "admin.config.mcp.values.authorization_status.revoked",
    "select_kit.default_header_text",
    "select_kit.filter_placeholder",
    "select_kit.no_content",
    "multi_select.search",
    "multi_select.no_results",
    "date_range_picker.presets.label",
    "date_range_picker.presets.last_7_days",
    "date_range_picker.presets.last_30_days",
    "date_range_picker.presets.last_3_months",
    "date_range_picker.presets.last_6_months",
    "date_range_picker.presets.last_year",
    "date_range_picker.start_date",
    "date_range_picker.end_date",
    "date_range_picker.date_placeholder",
    "date_range_picker.apply",
    "dates.previous_month",
    "dates.next_month",
    "cancel",
];
