//! The upcoming changes page (90g6), as Ember draws it: DPageHeader over
//! AdminConfigAreasUpcomingChanges' DFilterControls (the text filter and,
//! in its drawer, the status, type, impact role and enabled dropdowns)
//! and the table of UpcomingChangeItem rows. The list itself rides along
//! as JSON for static/js/admin-upcoming-changes.js, which filters it and
//! saves who each change is enabled for.

use serde_json::{Value, json};

use crate::admin_view::{AdminCx, Crumb, PageHeader, t, t_with};
use crate::topic_list_view::{escape, icon};

/// The filters the URL asks for (DFilterControls' query params).
pub struct Filters {
    pub text: String,
    pub status: Option<String>,
    pub kind: Option<String>,
    pub impact_role: Option<String>,
    pub enabled: Option<String>,
}

/// `encodeURIComponent`
fn uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

fn k(cx: &AdminCx, key: &str) -> String {
    t(cx.i18n, &format!("admin.upcoming_changes.{key}"))
}

/// The staff group's name (`site.groupsById[staff].name`).
const STAFF: &str = "staff";

/// A drawer dropdown: its key, query param and (value, label) options.
pub type Dropdown = (&'static str, &'static str, Vec<(String, String)>);

/// The four dropdowns.
pub fn dropdowns(cx: &AdminCx) -> Vec<Dropdown> {
    let f = |key: &str| k(cx, &format!("filter.{key}"));
    let opts = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(v, key)| (v.to_string(), f(key)))
            .collect()
    };
    vec![
        (
            "status",
            "status",
            opts(&[
                ("all", "status_all"),
                ("experimental", "status_experimental"),
                ("alpha", "status_alpha"),
                ("beta", "status_beta"),
                ("stable", "status_stable"),
            ]),
        ),
        (
            "type",
            "type",
            opts(&[
                ("all", "impact_type_all"),
                ("feature", "impact_type_feature"),
                ("other", "impact_type_other"),
                ("site_setting_default", "impact_type_site_setting_default"),
            ]),
        ),
        (
            "impactRole",
            "impactRole",
            opts(&[
                ("all", "impact_role_all"),
                ("admins", "impact_role_admins"),
                ("moderators", "impact_role_moderators"),
                ("staff", "impact_role_staff"),
                ("all_members", "impact_role_all_members"),
                ("developers", "impact_role_developers"),
            ]),
        ),
        (
            "enabled",
            "enabled",
            vec![
                ("all".into(), f("enabled_all")),
                ("enabled".into(), f("enabled")),
                (
                    "enabled_for_staff".into(),
                    t_with(
                        cx.i18n,
                        "admin.upcoming_changes.filter.enabled_for_staff",
                        &[("staffGroupName", STAFF)],
                    ),
                ),
                (
                    "enabled_for_specific_groups".into(),
                    f("enabled_for_specific_groups"),
                ),
                ("disabled".into(), f("disabled")),
            ],
        ),
    ]
}

fn native_option(value: &str, label: &str, selected: bool) -> String {
    format!(
        "<option class=\"d-native-select__option{}\" value=\"{}\">{}</option>",
        if selected { " --selected" } else { "" },
        escape(value),
        escape(label)
    )
}

fn badge(class: &str, icon_name: &str, label: &str, info: bool) -> String {
    format!(
        "<span class=\"d-table-badge upcoming-change__badge --has-tooltip {class}\"><span class=\"d-table-badge__content upcoming-change__badge-content\">{} {}</span>{}</span>",
        icon(icon_name, None),
        escape(label),
        if info {
            format!(
                "<span class=\"d-table-badge__info upcoming-change__badge-info\">{}</span>",
                icon("info", None)
            )
        } else {
            String::new()
        }
    )
}

/// UpcomingChangeBadges
fn badges(cx: &AdminCx, change: &Value) -> String {
    let status = s(&change["status"]);
    let role = s(&change["impact_role"]);
    let kind = s(&change["impact_type"]);
    let role_icon = match role {
        "admins" | "moderators" | "staff" => "shield-halved",
        "all_members" => "users",
        "developers" => "code",
        _ => "",
    };
    let type_icon = match kind {
        "feature" => "wand-magic-sparkles",
        "other" => "discourse-other-tab",
        "site_setting_default" => "gear",
        _ => "",
    };
    format!(
        "<div class=\"d-table__badges upcoming-change__badges\"><span class=\"fk-d-tooltip__trigger\" aria-expanded=\"false\" data-trigger=\"\" role=\"button\"><span class=\"fk-d-tooltip__trigger-container\">{}</span></span>{}{}</div>",
        badge(
            &format!("--status-{status}"),
            "flask",
            &k(cx, &format!("statuses.{status}")),
            true
        ),
        badge(
            &format!("--impact-role-{role}"),
            role_icon,
            &k(cx, &format!("impact_roles.{role}")),
            false
        ),
        badge(
            &format!("--impact-type-{kind}"),
            type_icon,
            &k(cx, &format!("impact_types.{kind}_type")),
            false
        ),
    )
}

/// UpcomingChangeItem's `enabledForOptions`: no one, then what
/// `allow_enabled_for` permits (all of them when unset).
pub fn enabled_for_options(cx: &AdminCx, change: &Value) -> Vec<(String, String)> {
    let allow: Vec<&str> = match change["allow_enabled_for"].as_array() {
        Some(list) => list.iter().filter_map(Value::as_str).collect(),
        None => vec!["everyone", "staff", "specific_groups"],
    };
    let mut out = vec![("no_one".to_string(), k(cx, "enabled_for_options.no_one"))];
    if allow.contains(&"everyone") {
        out.push(("everyone".into(), k(cx, "enabled_for_options.everyone")));
    }
    if allow.contains(&"staff") {
        out.push((STAFF.into(), "Staff".into()));
    }
    if allow.contains(&"specific_groups") {
        out.push((
            "groups".into(),
            k(cx, "enabled_for_options.specific_groups"),
        ));
    }
    out
}

/// The group selection under "Specific group(s)": GroupSelector's
/// DMultiSelect and its save button (disabled until the groups change).
pub fn group_selection(cx: &AdminCx, groups: &str) -> String {
    let selection: Vec<(String, String)> = groups
        .split(',')
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .map(|g| (g.to_string(), g.to_string()))
        .collect();
    format!(
        "<div class=\"upcoming-change__group-selection-wrapper\"><div class=\"group-selector-wrapper\">{}</div><span class=\"fk-d-tooltip__trigger\" aria-expanded=\"false\" data-trigger=\"\" role=\"button\"><span class=\"fk-d-tooltip__trigger-container\"><button class=\"btn btn-icon btn-primary no-text upcoming-change__save-groups\" disabled=\"\" type=\"button\">{}<span aria-hidden=\"true\">\u{200b}</span></button></span></span></div>",
        crate::admin_ui::multi_select_trigger(
            "group-selector",
            &k(cx, "select_groups"),
            &selection
        ),
        icon("check", None),
    )
}

/// One UpcomingChangeItem row.
pub fn row(cx: &AdminCx, entry: &Value) -> String {
    let change = &entry["upcoming_change"];
    let name = s(&entry["setting"]);
    let enabled_for = s(&change["enabled_for"]);
    let plugin = match entry["plugin"].as_str() {
        Some(p) if !p.is_empty() => format!(
            "<span class=\"upcoming-change__plugin\">{} {}</span>",
            icon("plug", None),
            escape(p)
        ),
        _ => String::new(),
    };
    let description = match entry["description"].as_str() {
        Some(d) if !d.is_empty() => {
            let d = crate::admin_setting_links::linkify(cx.settings, cx.base_path, d);
            let image = match change["image"]["url"].as_str() {
                Some(url) => format!(
                    "<a class=\"lightbox upcoming-change__image-preview\" data-large-src=\"{u}\" data-target-height=\"{}\" data-target-width=\"{}\" href=\"{u}\" rel=\"nofollow ugc noopener\">{} {}</a>",
                    change["image"]["height"],
                    change["image"]["width"],
                    icon("far-image", None),
                    escape(&k(cx, "preview")),
                    u = escape(url)
                ),
                None => String::new(),
            };
            let learn_more = match change["learn_more_url"].as_str() {
                Some(url) => format!(
                    "<span class=\"upcoming-change__learn-more\">{}</span>",
                    t_with(cx.i18n, "feedback_with_link", &[("url", url)])
                ),
                None => String::new(),
            };
            format!(
                "<div class=\"d-table__overview-about upcoming-change__description\">{d} <div class=\"upcoming-change__description-details\">{image} {learn_more}</div></div>"
            )
        }
        _ => String::new(),
    };
    let depends_on: Vec<&str> = entry["depends_on"]
        .as_array()
        .map(|l| l.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let depends_notice = if !depends_on.is_empty() && entry["depends_on_met"] != true {
        let links = depends_on
            .iter()
            .enumerate()
            .map(|(i, dep)| {
                let label = entry["depends_on_humanized_names"][i]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| dep.replace('_', " "));
                format!(
                    "<a href=\"{}/admin/site_settings/category/all_results?filter={}\">{}</a>",
                    cx.base_path,
                    uri_component(dep),
                    escape(&label)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "<div class=\"upcoming-change__depends-on-notice\">{} {}</div>",
            icon("triangle-exclamation", None),
            t_with(
                cx.i18n,
                "admin.upcoming_changes.depends_on_notice",
                &[("dependencyLinks", &links)]
            )
        )
    } else {
        String::new()
    };
    let permanent_soon = if s(&change["status"]) == "stable" && change["permanent_warning"] != false
    {
        format!(
            "<div class=\"upcoming-change__status-notice\">{} {}</div>",
            icon("triangle-exclamation", None),
            escape(&k(cx, "permanent_soon_notice"))
        )
    } else {
        String::new()
    };
    let options: String = enabled_for_options(cx, change)
        .iter()
        .map(|(v, label)| native_option(v, label, v == enabled_for))
        .collect();
    let related = format!(
        "{}/admin/site_settings?dependsOn={}&amp;filter=all_results",
        cx.base_path,
        uri_component(name)
    );
    let shown = enabled_for != "no_one";
    let dependents = if shown
        && entry["dependents"]
            .as_array()
            .is_some_and(|d| !d.is_empty())
    {
        format!(
            "<div class=\"upcoming-change__dependents\"><a class=\"\" href=\"{related}\">{}</a></div>",
            escape(&k(cx, "show_related_settings"))
        )
    } else {
        String::new()
    };
    let overrides = if shown && entry["overriding_defaults"] == true {
        format!(
            "<div class=\"upcoming-change__default-override-setting\"><a class=\"\" href=\"{}/admin/site_settings?filter={}\">{}</a></div>",
            cx.base_path,
            uri_component(&format!("upcoming_change_default_override:{name}")),
            escape(&k(cx, "show_related_settings"))
        )
    } else {
        String::new()
    };
    let groups = if enabled_for == "groups" {
        group_selection(cx, s(&entry["groups"]))
    } else {
        String::new()
    };
    format!(
        "<tr class=\"d-table__row upcoming-change-row\" data-upcoming-change=\"{}\"><td class=\"d-table__cell --overview\">{plugin}<div class=\"d-table__overview-name\">{}</div>{description}{depends_notice}{permanent_soon}{}</td><td class=\"d-table__cell --detail upcoming-change__toggle-cell\"><div class=\"d-table__mobile-label\">{}</div><select class=\"d-native-select upcoming-change__enabled-for\">{options}</select>{dependents}{overrides}{groups}</td></tr>",
        escape(name),
        escape(s(&entry["humanized_name"])),
        badges(cx, change),
        escape(&k(cx, "enabled_for")),
    )
}

/// The table of the given rows.
pub fn table(cx: &AdminCx, rows: &[&Value]) -> String {
    format!(
        "<table class=\"d-table upcoming-changes-table\"><thead class=\"d-table__header\"><tr class=\"d-table__row\"><th class=\"d-table__header-cell upcoming-change__name-header\">{}</th><th class=\"d-table__header-cell upcoming-change__enabled-header\">{}</th></tr></thead><tbody class=\"d-table__body\">{}</tbody></table>",
        escape(&k(cx, "name")),
        escape(&k(cx, "enabled_for")),
        rows.iter().map(|r| row(cx, r)).collect::<String>()
    )
}

/// The page.
pub fn page(cx: &AdminCx, list: &[Value], filters: &Filters) -> String {
    let title = t(cx.i18n, "admin.config.upcoming_changes.title");
    let header = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path: "/admin/config/upcoming-changes",
            },
        ],
        title,
        actions: String::new(),
        description: Some(t(
            cx.i18n,
            "admin.config.upcoming_changes.header_description",
        )),
        learn_more_url: Some("https://meta.discourse.org/t/-/392894".into()),
        tabs: None,
    }
    .render(cx);
    // The dropdowns the URL selects open the drawer.
    let chosen = [
        &filters.status,
        &filters.kind,
        &filters.impact_role,
        &filters.enabled,
    ];
    let drawer = dropdowns(cx)
        .into_iter()
        .zip(chosen)
        .map(|((key, _, options), value)| {
            let value = value
                .as_deref()
                .filter(|v| options.iter().any(|(o, _)| o == v))
                .unwrap_or("all");
            (key, options, value.to_string())
        })
        .collect::<Vec<_>>();
    let open = drawer.iter().any(|(_, _, v)| v != "all");
    let active = open || !filters.text.is_empty();
    let reset = escape(&t(cx.i18n, "filter_controls.reset"));
    let dropdowns_html = if open {
        format!(
            "<div class=\"d-filter-controls__dropdowns\">{}</div>",
            drawer
                .iter()
                .map(|(key, options, value)| dropdown(key, options, value))
                .collect::<String>()
        )
    } else {
        String::new()
    };
    let controls = format!(
        "<div class=\"d-filter-controls --dropdowns-in-filter-drawer\"><div class=\"d-filter-controls__inputs\"><div class=\"filter-input-container\">{}<input autocapitalize=\"none\" autocomplete=\"off\" autocorrect=\"off\" class=\"filter-input d-filter-controls__input\" placeholder=\"{}\" type=\"text\"{}></div><button aria-expanded=\"{open}\" class=\"btn no-text btn-icon btn-default d-filter-controls__toggle-filters\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">\u{200b}</span></button><button aria-label=\"{reset}\" class=\"btn no-text btn-icon btn-default d-filter-controls__reset\"{} title=\"{reset}\" type=\"button\">{}<span aria-hidden=\"true\">\u{200b}</span></button></div>{dropdowns_html}</div>",
        icon("magnifying-glass", Some("-left")),
        escape(&k(cx, "filter.search_placeholder")),
        // The URL's text is the value property the script sets.
        if filters.text.is_empty() {
            " value=\"\""
        } else {
            ""
        },
        escape(&t(cx.i18n, "filter_controls.toggle")),
        icon(
            if open {
                "filter-circle-xmark"
            } else {
                "filter"
            },
            None
        ),
        if active { "" } else { " disabled=\"\"" },
        icon("arrow-rotate-left", None),
    );
    let rows: Vec<&Value> = list.iter().collect();
    let empty = if list.is_empty() {
        format!(
            "<div class=\"admin-config-area-empty-list\">{}</div>",
            k(cx, "no_changes")
        )
    } else {
        String::new()
    };
    let data = json!({ "changes": list, "dropdowns": drawer.iter().map(|(key, options, _)| json!({
        "key": key,
        "options": options.iter().map(|(v, l)| json!([v, l])).collect::<Vec<_>>(),
    })).collect::<Vec<_>>() });
    format!(
        "{header}<div class=\"admin-config-page__main-area\">{controls}{}{empty}</div><script type=\"application/json\" id=\"upcoming-changes-data\">{}</script>",
        table(cx, &rows),
        data.to_string().replace('<', "\\u003c")
    )
}

/// One of the drawer's DNativeSelects.
pub fn dropdown(key: &str, options: &[(String, String)], value: &str) -> String {
    let label = options.first().map(|(_, l)| l.as_str()).unwrap_or_default();
    format!(
        "<select aria-label=\"{}\" class=\"d-native-select d-filter-controls__dropdown d-filter-controls__dropdown--{key}{}\" data-dropdown-key=\"{key}\">{}</select>",
        escape(label),
        if value == "all" { "" } else { " --active" },
        options
            .iter()
            .map(|(v, l)| native_option(v, l, v == value))
            .collect::<String>()
    )
}

/// The strings admin-upcoming-changes.js shows.
pub const STRINGS: &[&str] = &[
    "admin.upcoming_changes.groups_updated",
    "admin.upcoming_changes.change_enabled_for_success",
    "admin.upcoming_changes.change_disabled",
    "admin.upcoming_changes.enabled_for_options.no_one",
    "admin.upcoming_changes.enabled_for_options.everyone",
    "admin.upcoming_changes.enabled_for_options.staff",
    "admin.upcoming_changes.enabled_for_options.specific_groups_with_group_names.one",
    "admin.upcoming_changes.enabled_for_options.specific_groups_with_group_names.other",
    "admin.upcoming_changes.select_groups",
    "admin.upcoming_changes.save_groups",
    "admin.upcoming_changes.add_groups_to_enable",
    "admin.upcoming_changes.no_changes_to_save",
    "admin.upcoming_changes.filter.search_placeholder",
    "admin.upcoming_changes.show_related_settings",
    "admin.upcoming_changes.status_descriptions.experimental",
    "admin.upcoming_changes.status_descriptions.alpha",
    "admin.upcoming_changes.status_descriptions.beta",
    "admin.upcoming_changes.status_descriptions.stable",
    "filter_controls.reset",
    "multi_select.search",
    "multi_select.no_results",
];
