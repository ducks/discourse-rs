//! The admin's users pages as the Ember client renders them
//! (templates/admin-users.gjs around templates/admin-users-list/show.gjs)
//! from what Admin::UsersController#index serves: the page header with its
//! actions and tabs, the list's subheader, DFilterControls, and the users'
//! DResponsiveTable. static/js/admin-users.js handles what the controller
//! does in the browser.

use serde_json::Value;

use crate::admin_view::{AdminCx, Crumb, PageAction, PageHeader, t};
use crate::topic_list_view::{ListContext, escape, icon};

/// The lists the tabs name, in their order.
pub const QUERIES: &[&str] = &["active", "new", "staff", "suspended", "silenced", "staged"];

/// DNavItem: the tab, active for the current page.
pub fn nav_item(class: &str, href: &str, label: &str, active: bool) -> String {
    if active {
        format!(
            "<li class=\"active {class}\"><a class=\"active\" href=\"{}\">{}</a></li>",
            escape(href),
            escape(label)
        )
    } else {
        format!(
            "<li class=\"{class}\"><a href=\"{}\">{}</a></li>",
            escape(href),
            escape(label)
        )
    }
}

/// What the users pages' header reads of the viewer.
pub struct HeaderViewer {
    pub admin: bool,
    pub can_invite_to_forum: bool,
}

/// templates/admin-users.gjs's DPageHeader: Send invites and Export, the
/// description, and the tabs (`active` the current one).
pub fn users_header(cx: &AdminCx, viewer: &HeaderViewer, active: &str) -> String {
    let base = cx.base_path;
    let mut actions = Vec::new();
    if viewer.can_invite_to_forum {
        actions.push(PageAction {
            class: "admin-users__header-send-invites".into(),
            kind: "primary",
            label: t(cx.i18n, "admin.invite.button_text"),
            title: Some(t(cx.i18n, "admin.invite.button_title")),
        });
    }
    if viewer.admin {
        actions.push(PageAction {
            class: "admin-users__header-export-users".into(),
            kind: "primary",
            label: t(cx.i18n, "admin.export_csv.button_text"),
            title: Some(t(cx.i18n, "admin.export_csv.button_title.user")),
        });
    }
    let actions = crate::admin_view::page_actions(cx, "d-page-header", &actions);
    let mut tabs = nav_item(
        "admin-users-tabs__settings",
        &format!("{base}/admin/users/settings"),
        &t(cx.i18n, "settings"),
        active == "settings",
    );
    for q in QUERIES {
        tabs.push_str(&nav_item(
            &format!("admin-users-tabs__{q}"),
            &format!("{base}/admin/users/list/{q}"),
            &t(cx.i18n, &format!("admin.users.nav.{q}")),
            active == *q,
        ));
    }
    tabs.push_str(&nav_item(
        "admin-users-tabs__groups",
        &format!("{base}/admin/groups"),
        &t(cx.i18n, "groups.index.title"),
        false,
    ));
    let title = t(cx.i18n, "admin.config.users.title");
    PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path: "/admin/users/list",
            },
        ],
        title,
        actions,
        description: Some(escape(&t(cx.i18n, "admin.config.users.header_description"))),
        learn_more_url: Some(
            "https://meta.discourse.org/t/accessing-a-user-s-admin-page/311859".into(),
        ),
        tabs: Some(tabs),
    }
    .render(cx)
}

/// DPageSubheader with its actions.
pub fn subheader(title: &str, actions: &str) -> String {
    let mut out = format!(
        "<div class=\"d-page-subheader\"><div class=\"d-page-subheader__title-row\"><h2 class=\"d-page-subheader__title\">{}</h2>",
        escape(title)
    );
    if !actions.is_empty() {
        out.push_str(&format!(
            "<div class=\"d-page-subheader__actions\">{actions}</div>"
        ));
    }
    out.push_str("</div></div>");
    out
}

/// The list's state: the route's filter and the controller's params.
pub struct ListState<'a> {
    pub query: &'a str,
    pub filter: &'a str,
    pub order: Option<&'a str>,
    pub asc: bool,
    pub show_emails: bool,
    pub can_check_emails: bool,
    /// The staff list's account type (human, bot or all).
    pub account_type: &'a str,
    /// The new list's activation filter (all, activated, not_activated).
    pub activation: &'a str,
    pub must_approve_users: bool,
    /// Whether a next page may follow (a full page of 100).
    pub more: bool,
}

/// DNativeSelect of the filter controls: its options, the current one
/// selected, `--active` off its default.
fn native_select(cx: &AdminCx, options: &[(&str, &str)], value: &str, default: &str) -> String {
    let selected_label = options
        .iter()
        .find(|(v, _)| *v == value)
        .map(|(_, l)| t(cx.i18n, l))
        .unwrap_or_default();
    let mut out = format!(
        "<div class=\"d-filter-controls__dropdowns\"><select aria-label=\"{}\" class=\"d-filter-controls__dropdown{} d-native-select\">",
        escape(&selected_label),
        if value == default { "" } else { " --active" }
    );
    for (v, label) in options {
        out.push_str(&format!(
            "<option class=\"d-native-select__option{}\" value=\"{v}\"{}>{}</option>",
            if *v == value { " --selected" } else { "" },
            if *v == value { " selected" } else { "" },
            escape(&t(cx.i18n, label))
        ));
    }
    out.push_str("</select></div>");
    out
}

fn reset_button(cx: &AdminCx, disabled: bool, label: bool) -> String {
    let reset = t(cx.i18n, "filter_controls.reset");
    if label {
        format!(
            "<button aria-label=\"{r}\" class=\"btn btn-icon-text btn-default d-filter-controls__reset\"{} title=\"{r}\" type=\"button\">{}<span class=\"d-button-label\">{r}</span></button>",
            if disabled { " disabled=\"\"" } else { "" },
            icon("arrow-rotate-left", None),
            r = escape(&reset)
        )
    } else {
        format!(
            "<button aria-label=\"{r}\" class=\"btn no-text btn-icon btn-default d-filter-controls__reset\"{} title=\"{r}\" type=\"button\">{}<span aria-hidden=\"true\"> &#8203; </span></button>",
            if disabled { " disabled=\"\"" } else { "" },
            icon("arrow-rotate-left", None),
            r = escape(&reset)
        )
    }
}

/// DTableHeaderToggle (automatic): the classes the caller gives first.
fn header_toggle(
    cx: &AdminCx,
    s: &ListState,
    class: Option<&str>,
    field: &str,
    label_key: &str,
) -> String {
    let current = s.order == Some(field);
    let (aria_sort, pressed, chevron) = match (current, s.asc) {
        (true, true) => ("ascending", "mixed", Some("chevron-up")),
        (true, false) => ("descending", "true", Some("chevron-down")),
        _ => ("none", "false", None),
    };
    // `i18n(labelKey + "_long", { defaultValue: i18n(labelKey) })`
    let label = crate::admin_view::t_opt(cx.i18n, &format!("{label_key}_long"))
        .unwrap_or_else(|| t(cx.i18n, label_key));
    let open = match class {
        Some(class) => format!(
            "<div class=\"{class} directory-table__column-header sortable\" aria-sort=\"{aria_sort}\" role=\"columnheader\" data-order=\"{field}\">"
        ),
        None => format!(
            "<div aria-sort=\"{aria_sort}\" class=\"directory-table__column-header sortable\" role=\"columnheader\" data-order=\"{field}\">"
        ),
    };
    format!(
        "{open}<div aria-label=\"{}\" aria-pressed=\"{pressed}\" class=\"header-contents\" id=\"table-header-toggle-{field}\" role=\"button\" tabindex=\"0\"><span class=\"text\">{}{}</span></div></div>",
        escape(&crate::admin_view::t_with(
            cx.i18n,
            "directory.sort.label",
            &[("criteria", &label)]
        )),
        escape(&label),
        chevron
            .map(|c| format!(" {}", icon(c, None)))
            .unwrap_or_default(),
    )
}

/// `dFormatDuration`: durationTiny, an em dash for no number.
fn duration(list: &ListContext, seconds: &Value) -> String {
    match seconds.as_f64() {
        Some(s) => escape(&crate::user_profile_view::duration(
            list,
            s.round() as i64,
            "tiny",
        )),
        None => "&mdash;".into(),
    }
}

/// `rawDate`: the long date with the time.
fn raw_date(list: &ListContext, at: &Value) -> Option<String> {
    let at = chrono::DateTime::parse_from_rfc3339(at.as_str()?).ok()?;
    Some(crate::topic_list_view::long_date(list, at.to_utc()))
}

fn cell(class: &str, title: Option<String>, label: &str, value: &str) -> String {
    format!(
        "<div class=\"directory-table__cell {class}\"{}><span class=\"directory-table__label\"><span>{}</span></span><span class=\"directory-table__value\">{value}</span></div>",
        title
            .map(|t| format!(" title=\"{}\"", escape(&t)))
            .unwrap_or_default(),
        escape(label)
    )
}

/// A user's row of the table.
pub fn row(cx: &AdminCx, list: &ListContext, s: &ListState, user: &Value) -> String {
    let base = cx.base_path;
    let n = |k: &str| user[k].as_f64().map(|f| f.round() as i64).unwrap_or(0);
    let username = user["username"].as_str().unwrap_or_default();
    let id = user["id"].as_i64().unwrap_or(0);
    let actionable = user["can_be_deleted"] == Value::Bool(true)
        || user["can_be_suspended"] == Value::Bool(true);
    let mut out = format!(
        "<div class=\"user{} directory-table__row\" data-user-id=\"{id}\" data-can-be-actioned=\"{actionable}\"><div class=\"directory-table__cell username\"><a class=\"avatar\" data-user-card=\"{u}\" href=\"{base}/u/{u}\">{}</a><a href=\"{base}/admin/users/{id}/{u}\">{u}</a>",
        if user["active"] == Value::Bool(false) {
            " not-activated"
        } else {
            ""
        },
        crate::post_view::avatar_img(
            user["avatar_template"].as_str().unwrap_or_default(),
            24,
            &format!(" title=\"{}\"", escape(username))
        ),
        u = escape(username)
    );
    if user["staged"] == Value::Bool(true) {
        out.push_str(&titled_icon("far-envelope", &t(cx.i18n, "user.staged")));
    }
    out.push_str("</div>");
    out.push_str(&format!(
        "<div class=\"directory-table__cell email{}\"><span class=\"directory-table__value\">{}</span></div>",
        if s.show_emails { "" } else { " hidden" },
        escape(user["email"].as_str().unwrap_or_default())
    ));
    out.push_str(&cell(
        "last-emailed",
        raw_date(list, &user["last_emailed_at"]),
        &t(cx.i18n, "admin.users.last_emailed"),
        &duration(list, &user["last_emailed_age"]),
    ));
    out.push_str(&cell(
        "last-seen",
        Some(raw_date(list, &user["last_seen_at"]).unwrap_or_default()),
        &t(cx.i18n, "last_seen"),
        &duration(list, &user["last_seen_age"]),
    ));
    let reasons = s.query == "silenced" || s.query == "suspended";
    if !reasons {
        out.push_str(&cell(
            "topics-entered",
            None,
            &t(cx.i18n, "admin.user.topics_entered"),
            &crate::user_profile_view::number(list, n("topics_entered")),
        ));
    }
    out.push_str(&cell(
        "posts-read",
        None,
        &t(cx.i18n, "admin.user.posts_read_count"),
        &crate::user_profile_view::number(list, n("posts_read_count")),
    ));
    out.push_str(&cell(
        "time-read",
        None,
        &t(cx.i18n, "admin.user.time_read"),
        &duration(list, &user["time_read"]),
    ));
    out.push_str(&cell(
        "created",
        Some(raw_date(list, &user["created_at"]).unwrap_or_default()),
        &t(cx.i18n, "created"),
        &duration(list, &user["created_at_age"]),
    ));
    for (query, key, class) in [
        ("silenced", "silence_reason", "silence_reason"),
        ("suspended", "suspend_reason", "suspend_reason"),
    ] {
        if s.query == query {
            let reason = user[key].as_str().unwrap_or_default();
            out.push_str(&cell(
                class,
                Some(strip_html(reason)),
                &t(cx.i18n, &format!("admin.users.{key}")),
                reason,
            ));
        }
    }
    if s.must_approve_users {
        let approved = user["approved"].as_bool().unwrap_or(false);
        out.push_str(&format!(
            "<div class=\"directory-table__cell\"><span class=\"directory-table__label\"><span>{}</span></span><span class=\"directory-table__value\">{}</span></div>",
            escape(&t(cx.i18n, "admin.users.approved")),
            escape(&t(cx.i18n, if approved { "yes_value" } else { "no_value" }))
        ));
    }
    let admin = user["admin"] == Value::Bool(true);
    let moderator = user["moderator"] == Value::Bool(true);
    let second_factor = user["second_factor_enabled"] == Value::Bool(true);
    let mut icons: Vec<String> = Vec::new();
    if admin {
        icons.push(titled_icon("shield-halved", &t(cx.i18n, "admin.title")));
    }
    if moderator {
        icons.push(titled_icon("shield-halved", &t(cx.i18n, "admin.moderator")));
    }
    if second_factor {
        icons.push(titled_icon(
            "lock",
            &t(cx.i18n, "admin.user.second_factor_enabled"),
        ));
    }
    out.push_str(&format!(
        "<div class=\"directory-table__cell user-role{}\"><span class=\"directory-table__label\"><span>{}</span></span><span class=\"directory-table__value\">{icons}</span></div></div>",
        if admin || moderator || second_factor {
            ""
        } else {
            " --empty"
        },
        escape(&t(cx.i18n, "admin.users.status")),
        icons = icons.join(" ")
    ));
    out
}

/// `dIcon(name, { title })`: the icon in a titled span.
fn titled_icon(name: &str, title: &str) -> String {
    format!(
        "<span class=\"svg-icon-title\" title=\"{}\">{}</span>",
        escape(title),
        icon(name, None)
    )
}

/// The controller's `stripHtml`: the text of a reason's HTML.
fn strip_html(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    crate::onebox::entities::decode(&out).unwrap_or(out)
}

/// templates/admin-users-list/show.gjs: the subheader, the filter
/// controls, and the users' table (or the empty states).
pub fn list(cx: &AdminCx, list: &ListContext, s: &ListState, users: &[Value]) -> String {
    let title = t(cx.i18n, &format!("admin.users.titles.{}", s.query));
    let mut actions = Vec::new();
    if s.can_check_emails {
        let (class, key) = if s.show_emails {
            (
                "admin-users__subheader-hide-emails",
                "admin.users.hide_emails",
            )
        } else {
            (
                "admin-users__subheader-show-emails",
                "admin.users.show_emails",
            )
        };
        actions.push(PageAction {
            class: class.into(),
            kind: "default",
            label: t(cx.i18n, key),
            title: None,
        });
    }
    let actions = crate::admin_view::page_actions(cx, "d-page-subheader", &actions);
    let mut out = subheader(&title, &actions);
    // DFilterControls
    let staff = s.query == "staff";
    let new = s.query == "new";
    let has_active_filters = !s.filter.is_empty()
        || (staff && s.account_type != "human")
        || (new && s.activation != "all");
    out.push_str(&format!(
        "<div class=\"d-filter-controls\"><div class=\"d-filter-controls__inputs\"><div class=\"filter-input-container\">{}<input autocapitalize=\"none\" autocomplete=\"off\" autocorrect=\"off\" class=\"filter-input d-filter-controls__input\" value=\"{}\" placeholder=\"{}\" type=\"text\"></div></div>",
        icon("magnifying-glass", Some("-left")),
        escape(s.filter),
        escape(&t(cx.i18n, "search_hint"))
    ));
    if staff {
        out.push_str(&native_select(
            cx,
            &[
                ("human", "admin.users.account_type_filter.human"),
                ("bot", "admin.users.account_type_filter.bot"),
                ("all", "admin.users.account_type_filter.all"),
            ],
            s.account_type,
            "human",
        ));
    } else if new {
        out.push_str(&native_select(
            cx,
            &[
                ("all", "admin.users.activation_filter.all"),
                ("activated", "admin.users.activation_filter.activated"),
                (
                    "not_activated",
                    "admin.users.activation_filter.not_activated",
                ),
            ],
            s.activation,
            "all",
        ));
    }
    out.push_str(&reset_button(cx, !has_active_filters, false));
    out.push_str("</div>");
    if users.is_empty() {
        // DConditionalLoadingSpinner, not spinning.
        out.push_str("<div class=\"loading-container\"></div>");
        if !has_active_filters {
            out.push_str(&format!(
                "<p class=\"admin-users-list__no-results\">{}</p>",
                escape(&t(cx.i18n, "search.no_results"))
            ));
        } else {
            out.push_str(&format!(
                "<div class=\"d-filter-controls__no-results\"><p>{}</p>{}</div>",
                escape(&t(cx.i18n, "search.no_results")),
                reset_button(cx, false, true)
            ));
        }
        return out;
    }
    let mut columns = 7;
    if s.show_emails {
        columns += 1;
    }
    if s.must_approve_users {
        columns += 1;
    }
    out.push_str(&format!(
        "<div class=\"users-list-container\"><div class=\"horizontal-scroll-sync__container directory-table-container\"><div class=\"horizontal-scroll-sync__top-scroll\"><div class=\"horizontal-scroll-sync__fake-content\"></div></div><div class=\"horizontal-scroll-sync__content\"><div aria-label=\"{}\" class=\"directory-table users-list {}\" role=\"table\" style=\"grid-template-columns: minmax(min-content, 2fr) repeat({columns}, minmax(min-content, 1fr))\"><div class=\"directory-table__header\"><div class=\"directory-table__column-header-wrapper\">",
        escape(&title),
        escape(s.query)
    ));
    out.push_str(&format!(
        "<button class=\"btn no-text btn-icon btn-flat bulk-select\" type=\"button\">{}<span aria-hidden=\"true\"> &#8203; </span></button>",
        icon("list-check", None)
    ));
    out.push_str(&header_toggle(
        cx,
        s,
        Some("directory-table__column-header--username"),
        "username",
        "username",
    ));
    out.push_str("</div>");
    out.push_str(&header_toggle(
        cx,
        s,
        Some(if s.show_emails {
            "directory-table__column-header--email"
        } else {
            "hidden"
        }),
        "email",
        "email",
    ));
    out.push_str(&header_toggle(
        cx,
        s,
        None,
        "last_emailed",
        "admin.users.last_emailed",
    ));
    out.push_str(&header_toggle(cx, s, None, "seen", "last_seen"));
    if s.query != "silenced" && s.query != "suspended" {
        out.push_str(&header_toggle(
            cx,
            s,
            None,
            "topics_viewed",
            "admin.user.topics_entered",
        ));
    }
    out.push_str(&header_toggle(
        cx,
        s,
        None,
        "posts_read",
        "admin.user.posts_read_count",
    ));
    out.push_str(&header_toggle(
        cx,
        s,
        None,
        "read_time",
        "admin.user.time_read",
    ));
    out.push_str(&header_toggle(cx, s, None, "created", "created"));
    if s.query == "silenced" {
        out.push_str(&header_toggle(
            cx,
            s,
            Some("directory-table__column-header--silence-reason"),
            "silence_reason",
            "admin.users.silence_reason",
        ));
    }
    if s.query == "suspended" {
        out.push_str(&header_toggle(
            cx,
            s,
            Some("directory-table__column-header--suspend-reason"),
            "suspend_reason",
            "admin.users.suspend_reason",
        ));
    }
    if s.must_approve_users {
        out.push_str(&format!(
            "<div class=\"directory-table__column-header\">{}</div>",
            escape(&t(cx.i18n, "admin.users.approved"))
        ));
    }
    out.push_str("<div class=\"directory-table__column-header\">&nbsp;</div></div><div class=\"directory-table__body\">");
    for user in users {
        out.push_str(&row(cx, list, s, user));
    }
    out.push_str("</div></div></div></div>");
    // DLoadMore's sentinel, for admin-users.js, while a next page may follow.
    if s.more {
        out.push_str(
            "<div class=\"load-more-sentinel\" aria-hidden=\"true\" data-page=\"1\"></div>",
        );
    }
    out.push_str("</div>");
    // What bulk select adds, for admin-users.js: the header's Select all
    // and Clear all, and the bulk actions' trigger once a row is checked.
    let label_button = |class: &str, key: &str| {
        format!(
            "<button class=\"btn btn-text btn-flat {class}\" type=\"button\"><span class=\"d-button-label\">{}</span></button>",
            escape(&t(cx.i18n, key))
        )
    };
    out.push_str(&format!(
        "<template id=\"admin-users-bulk-header\" data-staff-cant-be-actioned=\"{}\">{}{}</template>",
        escape(&t(cx.i18n, "admin.users.bulk_actions.staff_cant_be_actioned")),
        label_button("bulk-select-all", "admin.users.bulk_actions.select_all"),
        label_button("bulk-clear-all", "admin.users.bulk_actions.clear_all")
    ));
    out.push_str(&format!(
        "<template id=\"admin-users-bulk-actions\"><div class=\"bulk-actions-dropdown\"><button aria-expanded=\"false\" class=\"btn fk-d-menu__trigger btn-default\" data-identifier=\"bulk-select-admin-users-dropdown\" data-trigger=\"\" type=\"button\"><span class=\"d-button-label\">{}</span>{}</button></div></template>",
        escape(&t(cx.i18n, "admin.users.bulk_actions.title")),
        icon("angle-down", None)
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons_lose_their_markup_for_the_title() {
        assert_eq!(
            strip_html("<a href=\"/x\">spam</a> &amp; more"),
            "spam & more"
        );
    }
}
