//! The groups directory (/groups: templates/groups/index.gjs with the
//! GroupList, GroupCard, GroupInfo and GroupMembershipButton components)
//! and a group's members page (/g/:name: templates/group.gjs with
//! GroupNavigation, and templates/group/index.gjs), drawn from the JSON
//! the routes load. static/js/groups.js does the rest in the browser: the
//! type and group menus, the filters, sorting the members.

use serde_json::Value;

use crate::topic_list_view::{ListContext, escape, icon, t, t_with};
use crate::user_card_view::avatar_flair;

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// `i18n(key)` for the admin bundle's keys (admin_js), which Ember merges
/// into its translations for staff.
fn ta(cx: &ListContext, key: &str) -> String {
    cx.i18n
        .t(&format!("admin_js.{key}"))
        .unwrap_or(key)
        .to_string()
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.0),
        _ => true,
    }
}

/// DButton with an icon and a label.
fn button(class: &str, icon_name: &str, label: &str, attrs: &str) -> String {
    format!(
        "<button class=\"btn btn-icon-text {class}\"{attrs} type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
        icon(icon_name, None),
        escape(label)
    )
}

/// GroupInfo: the first of full_name, display_name and name, and the
/// second as the mention name.
fn group_info(group: &Value) -> String {
    let mut names: Vec<&str> = Vec::new();
    for key in ["full_name", "display_name", "name"] {
        let v = s(&group[key]);
        if !v.is_empty() && !names.contains(&v) {
            names.push(v);
        }
    }
    let mention = names
        .get(1)
        .map(|m| {
            format!(
                "<span class=\"group-info-mention-name\">@{}</span>",
                escape(m)
            )
        })
        .unwrap_or_default();
    format!(
        "<span class=\"group-info-details\"><span class=\"group-info-name\">{}</span>{mention}</span>",
        escape(names.first().copied().unwrap_or_default())
    )
}

/// GroupMembershipButton: join, leave or request, else `inner`.
fn membership_button(cx: &ListContext, group: &Value, inner: &str, class: &str) -> String {
    let member = truthy(&group["is_group_user"]);
    let data = format!(
        " data-group-id=\"{}\" data-group-name=\"{}\"",
        group["id"],
        escape(s(&group["name"]))
    );
    let body = if truthy(&group["public_admission"]) && !member {
        button(
            "btn-default group-index-join",
            "user-plus",
            &t(cx, "groups.join"),
            &data,
        )
    } else if truthy(&group["public_exit"]) && member {
        button(
            "btn-danger group-index-leave",
            "user-xmark",
            &t(cx, "groups.leave"),
            &data,
        )
    } else if truthy(&group["allow_membership_requests"]) && !member {
        button(
            "btn-default group-index-request",
            "user-plus",
            &t(cx, "groups.request"),
            &data,
        )
    } else {
        inner.to_string()
    };
    format!("<div class=\"group-membership-button{class}\">{body}</div>")
}

fn flair(group: &Value, url: Option<&str>) -> String {
    let opt = |k: &str| group[k].as_str().filter(|v| !v.is_empty());
    avatar_flair(
        s(&group["name"]),
        url,
        opt("flair_bg_color"),
        opt("flair_color"),
    )
}

/// The card's description (`trustHTML bio_excerpt`). The card is a link,
/// and the HTML parser closes it at a link inside, where Ember's DOM keeps
/// one inside the other: an excerpt with links is left in a template that
/// groups.js puts in place.
fn description(excerpt: &str) -> String {
    if excerpt.contains("<a") {
        format!("<template class=\"group-description-html\">{excerpt}</template>")
    } else {
        excerpt.to_string()
    }
}

/// GroupCard
fn group_card(cx: &ListContext, group: &Value) -> String {
    let name = s(&group["name"]);
    let flair_url = group["flair_url"].as_str().filter(|v| !v.is_empty());
    let flair_html = flair_url
        .map(|url| {
            format!(
                "<span class=\"group-avatar-flair\">{}</span>",
                flair(group, Some(url))
            )
        })
        .unwrap_or_default();
    let status = if truthy(&group["is_group_owner"]) {
        format!(
            "<span class=\"is-group-owner\">{} {}</span>",
            icon("shield-halved", None),
            escape(&t(cx, "groups.index.is_group_owner"))
        )
    } else if truthy(&group["is_group_user"]) {
        format!(
            "<span class=\"is-group-member\">{} {}</span>",
            icon("check", None),
            escape(&t(cx, "groups.index.is_group_user"))
        )
    } else if truthy(&group["public_admission"]) {
        escape(&t(cx, "groups.index.public"))
    } else if group["visibility_level"].as_i64().unwrap_or(0) > 1 {
        format!(
            "{} {}",
            icon("far-eye-slash", None),
            escape(&t(cx, "groups.index.private"))
        )
    } else if truthy(&group["automatic"]) {
        escape(&t(cx, "groups.index.automatic"))
    } else {
        format!(
            "{} {}",
            icon("ban", None),
            escape(&t(cx, "groups.index.closed"))
        )
    };
    format!(
        "<a class=\"group-box\" data-group-name=\"{n}\" href=\"{base}/g/{n}/members\"><div class=\"group-box-inner\"><div class=\"group-info-wrapper\">{flair_html}<span class=\"group-info\">{info}<div class=\"group-user-count\">{user}{count}</div></span></div>\
         <div class=\"group-description\">{desc}</div><div class=\"group-membership\">{button}<span></span></div></div></a>",
        n = escape(name),
        base = cx.base_path,
        info = group_info(group),
        user = icon("user", None),
        count = group["user_count"].as_i64().unwrap_or(0),
        desc = description(s(&group["bio_excerpt"])),
        button = membership_button(cx, group, &status, ""),
    )
}

/// A ComboBox (single select) header and its rows for groups.js to open.
fn combo_box(
    class: &str,
    selected: Option<(&str, &str)>,
    none_label: &str,
    rows: &[(String, String)],
    angle: &str,
) -> String {
    let (label, value) = match selected {
        Some((value, label)) => (escape(label), format!(" data-value=\"{}\"", escape(value))),
        None => (escape(none_label), String::new()),
    };
    let filter_by = format!("Filter by: {label}");
    let mut items = String::new();
    for (i, (value, row_label)) in rows.iter().enumerate() {
        let is_selected = selected.is_some_and(|(v, _)| v == value);
        items.push_str(&format!(
            "<li aria-checked=\"{is_selected}\" role=\"menuitemradio\" data-index=\"{i}\" data-name=\"{l}\" data-value=\"{v}\" title=\"{l}\" tabindex=\"0\" class=\"select-kit-row{}\"><span class=\"name\">{l}</span></li>",
            if is_selected { " is-highlighted is-selected" } else { "" },
            l = escape(row_label),
            v = escape(value),
        ));
    }
    format!(
        "<details class=\"select-kit single-select combobox combo-box{} {class}\"><summary aria-label=\"{filter_by}\" name=\"{filter_by}\" data-name=\"{label}\"{value} tabindex=\"0\" class=\"select-kit-header single-select-header combo-box-header\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"{label}\"{value} title=\"{label}\"><span class=\"name\">{label}</span></div>{}</div></summary>\
         <div class=\"select-kit-body\"><ul aria-live=\"polite\" class=\"select-kit-collection\" role=\"menu\">{items}</ul></div></details>",
        if selected.is_some() {
            " has-selection"
        } else {
            ""
        },
        icon(angle, Some("angle-icon")),
    )
}

/// The groups index route's model and query params.
pub struct Index<'a> {
    /// groups#index's document.
    pub doc: &'a Value,
    pub filter: &'a str,
    pub kind: Option<&'a str>,
    /// `currentUser.can_create_group`
    pub can_create_group: bool,
}

/// The `#main-outlet` content of /groups.
pub fn render_index(cx: &ListContext, v: &Index) -> String {
    let mut out =
        String::from("<section class=\"container groups-index\"><div class=\"groups-header\">");
    if v.can_create_group {
        out.push_str(&button(
            "btn-default groups-header-new pull-right",
            "plus",
            &ta(cx, "admin.groups.new.title"),
            "",
        ));
    }
    let types: Vec<(String, String)> = v.doc["extras"]["type_filters"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_str())
        .map(|kind| {
            (
                kind.to_string(),
                t_with(cx, &format!("groups.index.{kind}_groups"), &[]),
            )
        })
        .collect();
    let selected = v
        .kind
        .and_then(|k| types.iter().find(|(id, _)| id == k))
        .map(|(id, label)| (id.as_str(), label.as_str()));
    out.push_str(&format!(
        "<div class=\"groups-header-filters\"><input aria-label=\"{}\" class=\"groups-header-filters-name no-blur\" placeholder=\"{}\" value=\"{}\" type=\"search\">{}</div></div>",
        escape(&t(cx, "groups.index.search_results")),
        escape(&t(cx, "groups.index.all")),
        escape(v.filter),
        combo_box(
            "groups-header-filters-type",
            selected,
            &t(cx, "groups.index.filter"),
            &types,
            "angle-down"
        ),
    ));
    let groups = v.doc["groups"].as_array().cloned().unwrap_or_default();
    if groups.is_empty() {
        out.push_str(&format!(
            "<p role=\"status\">{}</p>",
            escape(&t(cx, "groups.index.empty"))
        ));
    } else {
        out.push_str("<div><div class=\"container\"><div class=\"groups-boxes\">");
        for group in &groups {
            out.push_str(&group_card(cx, group));
        }
        out.push_str(
            "</div></div><div aria-hidden=\"true\" class=\"load-more-sentinel\"></div></div><div class=\"loading-container\"></div>",
        );
    }
    out.push_str("</section>");
    out
}

/// The group route's model, and the members route's.
pub struct Show<'a> {
    /// GroupShowSerializer for the viewer.
    pub group: &'a Value,
    /// `extras.visible_group_names`
    pub group_names: &'a [String],
    /// groups#members' document, when the viewer can see the members.
    pub members: Option<&'a Value>,
    pub viewer: Option<Viewer>,
    pub filter: &'a str,
    pub order: &'a str,
    pub asc: bool,
}

/// What the page reads from the current user.
pub struct Viewer {
    pub admin: bool,
    pub can_send_private_messages: bool,
    pub can_invite_to_forum: bool,
}

/// GroupController#tabs: (name, href, icon, label, count).
fn tabs(
    cx: &ListContext,
    v: &Show,
    can_manage: bool,
) -> Vec<(&'static str, String, &'static str, String, Option<i64>)> {
    let g = v.group;
    let base = format!("{}/g/{}", cx.base_path, escape(s(&g["name"])));
    let mut tabs = vec![
        (
            "members",
            base.clone(),
            "users",
            t(cx, "groups.members.title"),
            g["user_count"].as_i64().filter(|c| *c != 0),
        ),
        (
            "activity",
            format!("{base}/activity"),
            "bars-staggered",
            t(cx, "groups.activity"),
            None,
        ),
    ];
    if can_manage && truthy(&g["allow_membership_requests"]) {
        tabs.push((
            "requests",
            format!("{base}/requests"),
            "user-plus",
            t(cx, "groups.requests.title"),
            None,
        ));
    }
    let show_messages = v.viewer.as_ref().is_some_and(|u| {
        u.can_send_private_messages
            && truthy(&g["has_messages"])
            && (truthy(&g["is_group_user"]) || u.admin)
    });
    if show_messages {
        tabs.push((
            "messages",
            format!("{base}/messages"),
            "envelope",
            t(cx, "groups.messages"),
            None,
        ));
    }
    if can_manage {
        tabs.push((
            "manage",
            format!("{base}/manage"),
            "wrench",
            t(cx, "groups.manage.title"),
            None,
        ));
    }
    tabs.push((
        "permissions",
        format!("{base}/permissions"),
        "id-card",
        t(cx, "groups.permissions.title"),
        None,
    ));
    tabs
}

/// DTableHeaderToggle (automatic), its class first.
fn header_toggle(cx: &ListContext, v: &Show, class: &str, field: &str, label_key: &str) -> String {
    let current = v.order == field;
    let (aria_sort, pressed, chevron) = match (current, v.asc) {
        (true, true) => ("ascending", "mixed", Some("chevron-up")),
        (true, false) => ("descending", "true", Some("chevron-down")),
        _ => ("none", "false", None),
    };
    let label = t(cx, label_key);
    format!(
        "<div class=\"{class} directory-table__column-header sortable\" aria-sort=\"{aria_sort}\" role=\"columnheader\" data-order=\"{field}\"><div aria-label=\"{}\" aria-pressed=\"{pressed}\" class=\"header-contents\" id=\"table-header-toggle-{field}\" role=\"button\" tabindex=\"0\"><span class=\"text\">{}{}</span></div></div>",
        escape(&t_with(cx, "directory.sort.label", &[("criteria", &label)])),
        escape(&label),
        chevron
            .map(|c| format!(" {}", icon(c, None)))
            .unwrap_or_default(),
    )
}

/// A member's date cell (`dAgeWithTooltip` medium), `--empty` without one.
fn date_cell(
    cx: &ListContext,
    class: &str,
    label_key: &str,
    at: &Value,
    always_label: bool,
) -> String {
    let parsed = at
        .as_str()
        .and_then(|a| chrono::DateTime::parse_from_rfc3339(a).ok())
        .map(|a| a.with_timezone(&chrono::Utc));
    let label = format!(
        "<span class=\"directory-table__label\"><span>{}</span></span>",
        escape(&t(cx, label_key))
    );
    let value = parsed
        .map(|a| crate::user_profile_view::age_medium(cx, a, false))
        .unwrap_or_default();
    let empty = if parsed.is_none() && !always_label {
        "--empty"
    } else {
        ""
    };
    format!(
        "<div class=\"directory-table__cell{empty} {class}\">{}<span class=\"directory-table__value\">{value}</span></div>",
        if parsed.is_some() || always_label {
            label
        } else {
            String::new()
        }
    )
}

/// The `#main-outlet` content of /g/:name (the members tab).
pub fn render_show(cx: &ListContext, v: &Show) -> String {
    let g = v.group;
    let name = s(&g["name"]);
    let admin = v.viewer.as_ref().is_some_and(|u| u.admin);
    // currentUser.canManageGroup
    let can_manage_group =
        v.viewer.is_some() && (truthy(&g["can_admin_group"]) || truthy(&g["is_group_owner"]));
    let mut out = format!(
        "<span></span><div class=\"container group group-{}\"><div class=\"group-details-container\"><div class=\"group-info\">",
        escape(name)
    );
    let flair_url = g["flair_icon"]
        .as_str()
        .filter(|v| !v.is_empty())
        .or_else(|| g["flair_url"].as_str().filter(|v| !v.is_empty()));
    if flair_url.is_some() || truthy(&g["flair_bg_color"]) {
        out.push_str(&format!(
            "<div class=\"group-avatar-flair\">{}</div>",
            flair(g, flair_url)
        ));
    }
    out.push_str(&format!(
        "<div class=\"group-info-names\">{}",
        group_info(g)
    ));
    if can_manage_group && truthy(&g["automatic"]) {
        out.push_str(&format!(
            "<span aria-expanded=\"false\" class=\"fk-d-tooltip__trigger group-automatic-tooltip\" data-trigger=\"\" role=\"button\" data-tooltip=\"{}\"><span class=\"fk-d-tooltip__trigger-container\">{} {}</span></span>",
            escape(&ta(cx, "admin.groups.manage.membership.automatic_tooltip")),
            icon("gear", None),
            escape(&ta(cx, "admin.groups.manage.membership.automatic"))
        ));
    }
    out.push_str("</div><div class=\"group-details-button\">");
    out.push_str(&membership_button(cx, g, "", ""));
    if admin {
        if truthy(&g["automatic"]) {
            out.push_str(&button(
                "btn-default",
                "circle-question",
                &ta(cx, "admin.groups.delete"),
                "",
            ));
        } else {
            out.push_str(&button(
                "btn-danger",
                "trash-can",
                &ta(cx, "admin.groups.delete"),
                " data-test-selector=\"delete-group-button\"",
            ));
        }
    }
    if v.viewer.is_some() && truthy(&g["messageable"]) {
        out.push_str(&button(
            "btn-primary group-message-button",
            "envelope",
            &t(cx, "groups.message"),
            "",
        ));
    }
    out.push_str("</div></div>");
    if let Some(bio) = g["bio_cooked"].as_str().filter(|b| !b.is_empty()) {
        out.push_str(&format!("<div class=\"group-bio\">{bio}</div>"));
    }
    out.push_str(
        "</div><div class=\"user-content-wrapper\"><section class=\"user-primary-navigation\">",
    );
    let rows: Vec<(String, String)> = v
        .group_names
        .iter()
        .map(|n| (n.clone(), n.clone()))
        .collect();
    out.push_str(&combo_box(
        "group-dropdown",
        Some((name, name)),
        name,
        &rows,
        "angle-right",
    ));
    out.push_str("<nav class=\"horizontal-overflow-nav\"><div class=\"d-overflow-controls --owned-scroller horizontal-overflow-nav__controls\"><ul class=\"nav-pills action-list group-nav\" data-d-scroll-axis=\"horizontal\">");
    for (tab, href, tab_icon, label, count) in tabs(cx, v, can_manage_group) {
        let count = count
            .map(|c| format!(" <span class=\"count\">({c})</span>"))
            .unwrap_or_default();
        out.push_str(&format!(
            "<li><a class=\"{}{tab}\" title=\"{l}\" href=\"{href}\">{} {l}{count}</a></li>",
            if tab == "members" { "active " } else { "" },
            icon(tab_icon, None),
            l = escape(&label),
        ));
    }
    out.push_str("</ul></div></nav></section>");
    out.push_str(&members_section(
        cx,
        v,
        can_manage_group && !truthy(&g["automatic"]),
    ));
    out.push_str("</div></div>");
    out
}

/// templates/group/index.gjs
fn members_section(cx: &ListContext, v: &Show, can_manage: bool) -> String {
    let g = v.group;
    let admin = v.viewer.as_ref().is_some_and(|u| u.admin);
    let mut out =
        String::from("<section class=\"user-content\"><div class=\"group-members-actions\">");
    if can_manage {
        out.push_str(&format!(
            "<button class=\"btn no-text btn-icon btn-default bulk-select\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
            escape(&t(cx, "topics.bulk.toggle")),
            icon("list", None)
        ));
    }
    if truthy(&g["can_see_members"]) {
        let placeholder = if admin {
            "groups.members.filter_placeholder_admin"
        } else {
            "groups.members.filter_placeholder"
        };
        out.push_str(&format!(
            "<input placeholder=\"{}\" autocomplete=\"off\" class=\"ember-text-field ember-view group-username-filter no-blur\" type=\"text\" value=\"{}\">",
            escape(&t(cx, placeholder)),
            escape(v.filter)
        ));
    }
    if can_manage {
        out.push_str("<div class=\"group-members-manage\">");
        out.push_str(&button(
            "btn-default group-members-add",
            "plus",
            &t(cx, "groups.manage.add_members"),
            "",
        ));
        if v.viewer.as_ref().is_some_and(|u| u.can_invite_to_forum) {
            out.push_str(&button(
                "btn-default group-members-invite",
                "plus",
                &t(cx, "groups.manage.invite_members"),
                "",
            ));
        }
        out.push_str("</div>");
    }
    out.push_str("</div>");
    let members: Vec<Value> = v
        .members
        .and_then(|m| m["members"].as_array().cloned())
        .unwrap_or_default();
    if members.is_empty() {
        let key = if !truthy(&g["can_see_members"]) {
            "groups.members.forbidden"
        } else if !v.filter.is_empty() {
            "groups.members.no_filter_matches"
        } else {
            "groups.empty.members"
        };
        out.push_str(&format!("<br><div>{}</div></section>", escape(&t(cx, key))));
        return out;
    }
    let owners: Vec<i64> = v
        .members
        .and_then(|m| m["owners"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|o| o["id"].as_i64())
        .collect();
    out.push_str(&format!(
        "<div><div class=\"horizontal-scroll-sync__container directory-table-container\"><div class=\"horizontal-scroll-sync__top-scroll\"><div class=\"horizontal-scroll-sync__fake-content\"></div></div><div class=\"horizontal-scroll-sync__content\"><div class=\"directory-table group-members{}\" role=\"table\"><div class=\"directory-table__header\">",
        if can_manage {
            " group-members--can-manage"
        } else {
            ""
        }
    ));
    out.push_str(&header_toggle(
        cx,
        v,
        "directory-table__column-header--username username",
        "username_lower",
        "username",
    ));
    out.push_str("<div class=\"directory-table__column-header directory-table__column-header--can-manage\"></div>");
    out.push_str(&header_toggle(
        cx,
        v,
        "directory-table__column-header--added",
        "added_at",
        "groups.member_added",
    ));
    out.push_str(&header_toggle(
        cx,
        v,
        "directory-table__column-header--last-posted",
        "last_posted_at",
        "last_post",
    ));
    out.push_str(&header_toggle(
        cx,
        v,
        "directory-table__column-header--last-seen",
        "last_seen_at",
        "last_seen",
    ));
    if can_manage {
        out.push_str("<div class=\"directory-table__column-header directory-table__column-header--member-settings\"></div>");
    }
    out.push_str("</div><div class=\"directory-table__body\">");
    let name = s(&g["name"]);
    for m in &members {
        let owner = m["id"].as_i64().is_some_and(|id| owners.contains(&id));
        let primary = s(&m["primary_group_name"]) == name && !name.is_empty();
        let info = crate::user_info_view::user_info(cx, m, None).replacen(
            "class=\"name-line",
            "class=\"name-line --show-status",
            1,
        );
        out.push_str(&format!(
            "<div class=\"directory-table__row\"><div class=\"directory-table__cell directory-table__cell--username group-member\" colspan=\"2\">{info}</div>"
        ));
        out.push_str(
            "<div class=\"directory-table__cell directory-table__cell--can-manage group-owner\">",
        );
        if owner || primary {
            out.push_str(&format!(
                "<span class=\"directory-table__label\"><span>{}</span></span>",
                escape(&t(cx, "groups.members.status"))
            ));
        }
        out.push_str("<span class=\"directory-table__value\">");
        if owner {
            out.push_str(&format!(
                "{} {}<br>",
                icon("shield-halved", None),
                escape(&t(cx, "groups.members.owner"))
            ));
        }
        if primary {
            out.push_str(&escape(&t(cx, "groups.members.primary")));
        }
        out.push_str("</span></div>");
        out.push_str(&date_cell(
            cx,
            "directory-table__cell--added",
            "groups.member_added",
            &m["added_at"],
            true,
        ));
        out.push_str(&date_cell(
            cx,
            "directory-table__cell--last-posted",
            "last_post",
            &m["last_posted_at"],
            false,
        ));
        out.push_str(&date_cell(
            cx,
            "directory-table__cell--last-seen",
            "last_seen",
            &m["last_seen_at"],
            false,
        ));
        if can_manage {
            out.push_str(&format!(
                "<div class=\"directory-table__cell directory-table__cell--member-settings member-settings\"><details class=\"select-kit single-select dropdown-select-box group-member-dropdown\"><summary aria-label=\"{l}\" name=\"{l}\" data-name=\"\" tabindex=\"0\" class=\"select-kit-header single-select-header dropdown-select-box-header btn no-text btn-icon btn-default\"><div class=\"select-kit-header-wrapper\">{} &#8203;</div></summary><div class=\"select-kit-body\"></div></details></div>",
                icon("wrench", None),
                l = escape(&t(cx, "select_kit.select_to_filter")),
            ));
        }
        out.push_str("</div>");
    }
    out.push_str("</div></div></div></div><div aria-hidden=\"true\" class=\"load-more-sentinel\"></div></div><div class=\"loading-container\"></div></section>");
    out
}
