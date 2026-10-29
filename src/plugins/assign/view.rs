//! discourse-assign's components on the server-rendered pages: who a
//! topic and its posts are assigned to (AssignedToFirstPost,
//! AssignedToPost in the post-content-cooked-html outlet), the post
//! menu's assign button (AssignButton, placed as customizePostMenu places
//! it), the topic footer's Assign button and reassign menu (the "assign"
//! footer button and TopicLevelAssignMenu), the assign modal
//! (AssignUserForm in the assign and edit assignments modals), the small
//! actions' icons and the assignee "tags" topic lists show
//! (addTagsHtmlCallback). static/js/discourse-assign.js does what they do
//! in the browser.

use serde_json::{Value, json};

use crate::post_view::{PostContext, d_icon};
use crate::site_settings::{SettingError, SiteSettings};
use crate::topic_list_view::{escape, t, t_with};

/// What the assign UI reads of the site settings, while the plugin is on.
#[derive(Debug, Clone)]
pub struct AssignUi {
    pub public: bool,
    pub prioritize_full_name: bool,
    /// `assigns_user_url_path`, `{username}` in it.
    pub user_url_path: String,
    /// `enable_assign_status`, with `assign_statuses`.
    pub statuses: Option<Vec<String>>,
}

impl AssignUi {
    /// None while the plugin is off.
    pub fn load(settings: &SiteSettings) -> Result<Option<AssignUi>, SettingError> {
        if !super::enabled(settings)? {
            return Ok(None);
        }
        Ok(Some(AssignUi {
            public: super::public(settings)?,
            prioritize_full_name: settings.get("prioritize_full_name_in_ux")?.truthy(),
            user_url_path: settings.get("assigns_user_url_path")?.to_s(),
            statuses: if super::status_enabled(settings)? {
                Some(super::statuses(settings)?)
            } else {
                None
            },
        }))
    }

    /// `assignedToUserPath`
    pub fn user_path(&self, base_path: &str, username: &str) -> String {
        format!(
            "{base_path}{}",
            self.user_url_path.replace("{username}", username)
        )
    }

    /// `prioritizedAssigneeName`: the name with prioritize_full_name_in_ux
    /// (or for a group), else the username.
    pub fn name(&self, assignee: &Value) -> String {
        let username = assignee["username"].as_str().filter(|u| !u.is_empty());
        let name = assignee["name"].as_str().filter(|n| !n.is_empty());
        match username {
            Some(username) if !self.prioritize_full_name => username.to_string(),
            _ => name.or(username).unwrap_or_default().to_string(),
        }
    }
}

/// `assignedToGroupPath`
pub fn group_path(base_path: &str, name: &str) -> String {
    format!("{base_path}/g/{name}/assigned/everyone")
}

/// `assignedToPostPath`
pub fn post_path(base_path: &str, post_id: &str) -> String {
    format!("{base_path}/p/{post_id}")
}

/// The topic view's assign keys the posts and the footer read.
pub fn topic_assign(view: &Value) -> Value {
    json!({
        "id": view["id"],
        "can_assign": view["can_assign"],
        "assigned_to_user": view["assigned_to_user"],
        "assigned_to_group": view["assigned_to_group"],
        "indirectly_assigned_to": view["indirectly_assigned_to"],
    })
}

/// [`topic_assign`] without a topic view at hand (a post redrawn alone):
/// the topic view's keys built for it.
pub async fn topic_assign_for(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    urls: &crate::url::Urls<'_>,
    guardian: &crate::guardian::Guardian,
    topic_id: i32,
    category_id: Option<i32>,
) -> Result<Value, crate::plugins::PluginError> {
    let Some(view) =
        super::keys::TopicView::load(conn, settings, guardian, topic_id, category_id, &[]).await?
    else {
        return Ok(Value::Null);
    };
    let mut out = serde_json::Map::new();
    out.insert("id".into(), json!(topic_id));
    view.topic_keys(
        &super::keys::Cx {
            settings,
            urls,
            logo: None,
        },
        guardian,
        &mut out,
    )?;
    Ok(topic_assign(&Value::Object(out)))
}

fn indirect(topic: &Value) -> Vec<(String, &Value)> {
    topic["indirectly_assigned_to"]
        .as_object()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v)).collect())
        .unwrap_or_default()
}

/// The post-content-cooked-html outlet: the first post's assignees, or
/// a post's own.
pub fn post_assignments(cx: &PostContext, ui: &AssignUi, p: &Value) -> String {
    if p["post_number"] == 1 {
        first_post(cx, ui)
    } else {
        post(cx, ui, p)
    }
}

/// AssignedToFirstPost
fn first_post(cx: &PostContext, ui: &AssignUi) -> String {
    let topic = &cx.topic.assign;
    let base = cx.list.base_path;
    let user = topic["assigned_to_user"]
        .as_object()
        .map(|_| &topic["assigned_to_user"]);
    let group = topic["assigned_to_group"]
        .as_object()
        .map(|_| &topic["assigned_to_group"]);
    let posts = indirect(topic);
    if user.is_none() && group.is_none() && posts.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "<p class=\"assigned-to\">{}",
        d_icon(
            if user.is_some() {
                "user-plus"
            } else {
                "group-plus"
            },
            None
        )
    );
    let assigned_topic_to = |class: &str, assignee: &Value, path: String| {
        // The translation is HTML, the name escaped into it.
        format!(
            "<span class=\"assignee\"><span class=\"{class}\">{}</span></span>",
            t_with(
                cx.list,
                "discourse_assign.assigned_topic_to",
                &[("username", &escape(&ui.name(assignee))), ("path", &path)],
            )
        )
    };
    if let Some(user) = user {
        out.push_str(&assigned_topic_to(
            "assigned-to--user",
            user,
            ui.user_path(base, user["username"].as_str().unwrap_or_default()),
        ));
    }
    if let Some(group) = group {
        out.push_str(&assigned_topic_to(
            "assigned-to--group",
            group,
            group_path(base, group["name"].as_str().unwrap_or_default()),
        ));
    }
    if user.is_none() && group.is_none() {
        out.push_str(&format!(
            "<span class=\"assign-text\">{}</span>",
            escape(&t(cx.list, "discourse_assign.assigned"))
        ));
    }
    for (post_id, assignment) in posts {
        out.push_str(&format!(
            "<span class=\"assignee\"><a class=\"assigned-indirectly\" href=\"{}\">{}</a></span>",
            escape(&post_path(base, &post_id)),
            escape(&t_with(
                cx.list,
                "discourse_assign.assign_post_to_multiple",
                &[
                    ("post_number", &assignment["post_number"].to_string()),
                    ("username", &ui.name(&assignment["assigned_to"])),
                ],
            ))
        ));
    }
    out.push_str("</p>");
    out
}

/// PostAssignmentsDisplay for a post after the first: AssignedToPost with
/// its menu for who may assign.
fn post(cx: &PostContext, ui: &AssignUi, p: &Value) -> String {
    let key = p["id"].to_string();
    let Some(assigned_to) = cx.topic.assign["indirectly_assigned_to"]
        .get(&key)
        .map(|a| &a["assigned_to"])
        .filter(|a| a.is_object())
    else {
        return String::new();
    };
    let base = cx.list.base_path;
    let is_user = assigned_to["username"].is_string();
    let (icon, href, name) = if is_user {
        (
            "user-plus",
            ui.user_path(base, assigned_to["username"].as_str().unwrap_or_default()),
            if ui.prioritize_full_name {
                ui.name(assigned_to)
            } else {
                assigned_to["username"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            },
        )
    } else {
        (
            "group-plus",
            group_path(base, assigned_to["name"].as_str().unwrap_or_default()),
            assigned_to["name"].as_str().unwrap_or_default().to_string(),
        )
    };
    let menu = if p["can_assign"] == true {
        format!(
            "<button aria-expanded=\"false\" class=\"btn no-text btn-icon fk-d-menu__trigger post-assign-menu-trigger btn-transparent more-button\" data-identifier=\"post-assign-menu\" data-trigger=\"\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>\
             <template class=\"post-assign-menu\"><ul class=\"dropdown-menu\">\
             <li class=\"dropdown-menu__item\"><button class=\"btn btn-icon-text btn-transparent unassign-btn\" data-target-type=\"Post\" data-target-id=\"{}\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button></li>\
             <li class=\"dropdown-menu__item\"><button class=\"btn btn-icon-text btn-transparent edit-assignment-btn\" data-target-type=\"Post\" data-target-id=\"{}\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button></li>\
             </ul></template>",
            d_icon("ellipsis", None),
            p["id"],
            d_icon("user-plus", None),
            escape(&t(cx.list, "discourse_assign.unassign.title")),
            p["id"],
            d_icon("group-plus", None),
            escape(&t(cx.list, "discourse_assign.reassign.title_w_ellipsis")),
        )
    } else {
        String::new()
    };
    format!(
        "<p class=\"assigned-to\">{}<span class=\"assign-text\">{}</span><a class=\"assigned-to-username\" href=\"{}\">{}</a>{menu}</p>",
        d_icon(icon, None),
        escape(&t(cx.list, "discourse_assign.assigned_to")),
        escape(&href),
        escape(&name),
    )
}

/// The post menu's assign button (AssignButton), for a post after the
/// first the viewer may assign: its HTML, and whether it collapses (all
/// but when it is assigned to the viewer).
pub fn menu_button(cx: &PostContext, p: &Value) -> Option<(String, bool)> {
    if p["post_number"] == 1 || p["can_assign"] != true {
        return None;
    }
    let assigned = p["assigned_to_user"].is_object() || p["assigned_to_group"].is_object();
    let (class, icon, title) = if assigned {
        (
            "post-action-menu__unassign-post unassign-post",
            "user-xmark",
            "discourse_assign.unassign_post.title",
        )
    } else {
        (
            "post-action-menu__assign-post assign-post",
            "user-plus",
            "discourse_assign.assign_post.title",
        )
    };
    let mine = p["assigned_to_user"]["username"]
        .as_str()
        .is_some_and(|u| Some(u) == cx.viewer);
    Some((
        format!(
            "<button class=\"btn no-text btn-icon {class} btn-flat\" data-target-type=\"Post\" data-target-id=\"{}\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
            p["id"],
            escape(&t(cx.list, title)),
            d_icon(icon, None)
        ),
        !mine,
    ))
}

/// The topic footer's assign controls for a member who may assign: the
/// Assign button while nothing is assigned, else (on desktop) the
/// reassign menu; in the mobile view the unassign button naming the
/// assignee.
pub fn footer(cx: &PostContext, ui: &AssignUi, can_assign_user: bool) -> String {
    let controls = footer_controls(cx, ui, can_assign_user);
    if controls.is_empty() {
        return controls;
    }
    controls + &modal(cx, ui)
}

fn footer_controls(cx: &PostContext, ui: &AssignUi, can_assign_user: bool) -> String {
    let topic = &cx.topic.assign;
    if cx.viewer.is_none() {
        return String::new();
    }
    // canAssignTopic: the topic's can_assign, else the user's.
    let can_assign = topic["can_assign"].as_bool().unwrap_or(can_assign_user);
    if !can_assign {
        return String::new();
    }
    let user = topic["assigned_to_user"]
        .as_object()
        .map(|_| &topic["assigned_to_user"]);
    let group = topic["assigned_to_group"]
        .as_object()
        .map(|_| &topic["assigned_to_group"]);
    let assigned = user.is_some() || group.is_some();
    let posts = indirect(topic);
    let mobile = crate::mobile::mobile_view();
    let id = topic["id"].as_i64().unwrap_or(cx.topic.id);
    // defaultTitle
    let title = match user
        .and_then(|u| u["username"].as_str())
        .or_else(|| group.and_then(|g| g["name"].as_str()))
    {
        Some(username) => t_with(
            cx.list,
            "discourse_assign.unassign.help",
            &[("username", username)],
        ),
        None => t(cx.list, "discourse_assign.assign.help"),
    };
    if !assigned && posts.is_empty() {
        return format!(
            "<button class=\"btn btn-icon-text btn-default topic-footer-button assign\" aria-label=\"{title}\" data-target-type=\"Topic\" data-target-id=\"{id}\" id=\"topic-footer-button-assign\" title=\"{title}\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
            d_icon("user-plus", None),
            escape(&t(cx.list, "discourse_assign.assign.title")),
            title = escape(&title),
        );
    }
    if mobile {
        let label = escape(&t(cx.list, "discourse_assign.assigned_to"));
        let inner = match (user, group) {
            (Some(user), _) => format!(
                "<span class=\"unassign-label\"><span class=\"text\">{label}&nbsp;</span><span class=\"username\">{}</span></span>",
                escape(user["username"].as_str().unwrap_or_default())
            ),
            (None, Some(group)) => format!(
                "<span class=\"unassign-label\">{label}</span> @{}",
                escape(group["name"].as_str().unwrap_or_default())
            ),
            _ => return String::new(),
        };
        return format!(
            "<button class=\"btn btn-icon-text btn-default topic-footer-button assign unassign-mobile-header\" aria-label=\"{title}\" data-target-type=\"Topic\" data-target-id=\"{id}\" title=\"{title}\" type=\"button\"><span class=\"d-button-label\">{inner}</span></button>",
            title = escape(&title),
        );
    }
    // TopicLevelAssignMenu: unassign from the topic, from each post,
    // reassign to me, edit.
    let mut items = String::new();
    let item = |action: &str, attrs: &str, icon: &str, label: &str| {
        format!(
            "<li class=\"dropdown-menu__item\"><button class=\"btn btn-icon-text btn-transparent reassign-item\" data-action=\"{action}\"{attrs} type=\"button\">{}<span class=\"d-button-label\">{}</span></button></li>",
            d_icon(icon, None),
            escape(label)
        )
    };
    if assigned {
        let mut username = user
            .and_then(|u| u["username"].as_str())
            .or_else(|| group.and_then(|g| g["name"].as_str()))
            .unwrap_or_default()
            .to_string();
        if let Some(user) = user
            && ui.prioritize_full_name
        {
            username = ui.name(user);
        }
        items.push_str(&item(
            "unassign",
            &format!(" data-target-type=\"Topic\" data-target-id=\"{id}\""),
            if user.is_some() {
                "user-xmark"
            } else {
                "group-times"
            },
            &t_with(
                cx.list,
                "discourse_assign.topic_level_menu.unassign_from_topic",
                &[("username", &username)],
            ),
        ));
    }
    for (post_id, assignment) in &posts {
        let assignee = &assignment["assigned_to"];
        items.push_str(&item(
            "unassign",
            &format!(" data-target-type=\"Post\" data-target-id=\"{post_id}\""),
            if assignee["username"].is_string() {
                "user-xmark"
            } else {
                "group-times"
            },
            &t_with(
                cx.list,
                "discourse_assign.topic_level_menu.unassign_from_post",
                &[
                    ("assignee", &ui.name(assignee)),
                    ("post_number", &assignment["post_number"].to_string()),
                ],
            ),
        ));
    }
    let mine = user
        .and_then(|u| u["username"].as_str())
        .is_some_and(|u| Some(u) == cx.viewer);
    if assigned && !mine {
        items.push_str(&item(
            "reassign-self",
            &format!(
                " data-target-type=\"Topic\" data-target-id=\"{id}\" data-username=\"{}\"",
                escape(cx.viewer.unwrap_or_default())
            ),
            "user-plus",
            &t(
                cx.list,
                "discourse_assign.topic_level_menu.reassign_topic_to_me",
            ),
        ));
    }
    items.push_str(&item(
        "reassign",
        &format!(" data-target-type=\"Topic\" data-target-id=\"{id}\""),
        "pencil",
        &t(
            cx.list,
            "discourse_assign.topic_level_menu.edit_assignments",
        ),
    ));
    let label = if assigned {
        t(cx.list, "discourse_assign.reassign.title")
    } else {
        t(
            cx.list,
            "discourse_assign.topic_level_menu.unassign_with_ellipsis",
        )
    };
    format!(
        "<div class=\"select-kit dropdown-select-box topic-footer-dropdown reassign\" id=\"topic-footer-dropdown-reassign\"><button aria-expanded=\"false\" class=\"btn btn-icon-text btn-default topic-footer-button fk-d-menu__trigger reassign-trigger\" data-identifier=\"topic-footer-dropdown-reassign\" data-trigger=\"\" title=\"{}\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button><template class=\"reassign-menu\"><ul class=\"dropdown-menu\">{items}</ul></template></div>",
        escape(&title),
        d_icon(if assigned { "user-plus" } else { "user-xmark" }, None),
        escape(&label),
    )
}

/// The assign modal's template (the assign and edit assignments modals'
/// AssignUserForm), filled and shown by discourse-assign.js.
pub fn modal(cx: &PostContext, ui: &AssignUi) -> String {
    if cx.viewer.is_none() {
        return String::new();
    }
    let status = match &ui.statuses {
        Some(statuses) => {
            let options: String = statuses
                .iter()
                .map(|s| format!("<option value=\"{0}\">{0}</option>", escape(s)))
                .collect();
            format!(
                "<div class=\"control-group assign-status\"><label for=\"assign-status\">{}</label><select id=\"assign-status\" name=\"status\" class=\"combo-box\">{options}</select></div>",
                escape(&t(cx.list, "discourse_assign.assign_modal.status_label"))
            )
        }
        None => String::new(),
    };
    format!(
        "<template id=\"assign-modal-template\"><div class=\"modal d-modal assign\" role=\"dialog\" aria-modal=\"true\"><div class=\"d-modal__container\">\
         <div class=\"d-modal__header\"><div class=\"d-modal__title\"><h1 class=\"d-modal__title-text\" data-title-topic=\"{}\" data-title-post=\"{}\" data-title-edit=\"{}\"></h1></div>\
         <button class=\"btn no-text btn-icon btn-transparent modal-close\" title=\"{}\" type=\"button\">{}</button></div>\
         <div class=\"d-modal__body\"><form class=\"assign-user-form\">\
         <div class=\"control-group target\"><label for=\"assignee\">{}</label>\
         <input id=\"assignee\" name=\"assignee\" class=\"email-group-user-chooser\" autocomplete=\"off\" list=\"assign-suggestions\" placeholder=\"{}\"><datalist id=\"assign-suggestions\"></datalist>\
         <span class=\"error-label\" hidden>{}</span></div>{status}\
         <div class=\"control-group assign-note\"><label for=\"assign-note\">{} <span class=\"label-optional\">{}</span></label><textarea id=\"assign-note\" name=\"note\"></textarea></div>\
         </form></div>\
         <div class=\"d-modal__footer\"><button class=\"btn btn-text btn-primary assign-submit\" type=\"button\"><span class=\"d-button-label\">{}</span></button>\
         <button class=\"btn btn-text btn-flat modal-close\" type=\"button\"><span class=\"d-button-label\">{}</span></button></div>\
         </div></div></template>",
        escape(&t(cx.list, "discourse_assign.assign_modal.title")),
        escape(&t(cx.list, "discourse_assign.assign_post_modal.title")),
        escape(&t(cx.list, "discourse_assign.assign_modal.reassign_title")),
        escape(&t(cx.list, "modal.close")),
        d_icon("xmark", None),
        escape(&t(cx.list, "discourse_assign.assign_modal.assignee_label")),
        escape(&t(cx.list, "discourse_assign.assign_modal.choose_assignee")),
        escape(&t(cx.list, "discourse_assign.assign_modal.choose_assignee")),
        escape(&t(cx.list, "discourse_assign.assign_modal.note_label")),
        escape(&t(cx.list, "discourse_assign.assign_modal.optional_label")),
        escape(&t(cx.list, "discourse_assign.assign_modal.assign")),
        escape(&t(cx.list, "cancel")),
    )
}

/// The small action codes assign records (isAssignSmallAction): drawn as
/// small actions even when they are whispers.
pub fn is_small_action(code: &str) -> bool {
    matches!(
        code,
        "assigned"
            | "unassigned"
            | "reassigned"
            | "assigned_group"
            | "unassigned_group"
            | "reassigned_group"
            | "assigned_to_post"
            | "assigned_group_to_post"
            | "unassigned_from_post"
            | "unassigned_group_from_post"
            | "details_change"
            | "note_change"
            | "status_change"
    )
}

/// `addPostSmallActionIcon`
pub fn small_action_icon(code: &str) -> Option<&'static str> {
    Some(match code {
        "assigned" | "assigned_to_post" | "reassigned" => "user-plus",
        "assigned_group" | "assigned_group_to_post" | "reassigned_group" => "group-plus",
        "unassigned" | "unassigned_from_post" => "user-xmark",
        "unassigned_group" | "unassigned_group_from_post" => "group-times",
        _ => return None,
    })
}

/// `addGroupPostSmallActionCode`: whose `who` is a group.
pub fn is_group_code(code: &str) -> bool {
    matches!(
        code,
        "assigned_group"
            | "reassigned_group"
            | "unassigned_group"
            | "assigned_group_to_post"
            | "unassigned_group_from_post"
    )
}

/// The small action's extra class (addPostSmallActionClassesCallback):
/// `private-assign` while assigns aren't public.
pub fn small_action_class(ui: &AssignUi, code: &str) -> &'static str {
    if code.contains("assigned") && !ui.public {
        " private-assign"
    } else {
        ""
    }
}

/// The topic list's assignee "tags" (addTagsHtmlCallback): the topic's
/// assignee, then its posts', each linking to its assignments.
pub fn tags_html(ui: &AssignUi, base_path: &str, topic: &Value) -> Vec<String> {
    let mut assigned: Vec<(Value, Value, Option<String>)> = Vec::new();
    let direct = if topic["assigned_to_user"].is_object() {
        Some(topic["assigned_to_user"].clone())
    } else if topic["assigned_to_group"].is_object() {
        Some(topic["assigned_to_group"].clone())
    } else {
        None
    };
    if let Some(assignee) = direct {
        assigned.push((assignee, topic["assignment_note"].clone(), None));
    }
    for (post_id, a) in indirect(topic) {
        if a["assigned_to"].is_object() {
            assigned.push((
                a["assigned_to"].clone(),
                a["assignment_note"].clone(),
                Some(post_id),
            ));
        }
    }
    assigned
        .into_iter()
        .map(|(assignee, note, post_id)| {
            let username = assignee["username"].as_str();
            let path = match (&post_id, username) {
                (Some(id), _) => post_path(base_path, id),
                (None, Some(u)) => ui.user_path(base_path, u),
                (None, None) => group_path(base_path, assignee["name"].as_str().unwrap_or_default()),
            };
            let note = match &note {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            format!(
                "<a class=\"assigned-to discourse-tag simple\" href=\"{}\" data-auto-route=\"true\">{}<span title=\"{}\">{}</span></a>",
                path,
                crate::topic_list_view::icon(if username.is_some() { "user-plus" } else { "group-plus" }, None),
                escape(&note),
                escape(&ui.name(&assignee))
            )
        })
        .collect()
}

/// user-activity/assigned: the topics assigned to the user (their first
/// page, ListController#messages_assigned), with the search box, or the
/// empty state.
pub fn user_assigned_list(cx: &crate::topic_list_view::ListContext, topics: &[Value]) -> String {
    if topics.is_empty() {
        // DEmptyState; the body is the translation's HTML.
        return format!(
            "<div class=\"empty-state\"><span class=\"empty-state__title\">{}</span><div class=\"empty-state__body\"><p>{}</p></div></div>",
            escape(&t(cx, "user.no_assignments_title")),
            t_with(
                cx,
                "user.no_assignments_body",
                &[
                    (
                        "preferencesUrl",
                        &format!("{}/my/preferences/notifications", cx.base_path),
                    ),
                    ("icon", &crate::topic_list_view::icon("user-plus", None)),
                ],
            )
        );
    }
    let header = |class: &str, key: &str, sortable: bool| {
        let label = escape(&t(cx, key));
        if sortable {
            format!(
                "<th class=\"topic-list-data {class} sortable num\" data-sort-order=\"{class}\" scope=\"col\"><button>{label}</button></th>"
            )
        } else {
            format!(
                "<th class=\"topic-list-data {class}\" data-sort-order=\"{class}\" scope=\"col\"><span>{label}</span></th>"
            )
        }
    };
    let rows: String = topics
        .iter()
        .map(|topic| crate::topic_list_view::suggested_row(cx, topic))
        .collect();
    format!(
        "<div class=\"topic-search-div\"><div class=\"inline-form full-width\"><input autocomplete=\"off\" class=\"no-blur\" placeholder=\"{}\" type=\"search\"></div></div>\
         <div class=\"paginated-topics-list\"><div><div class=\"loading-container\">\
         <table class=\"topic-list\"><caption class=\"sr-only\">{}</caption><thead class=\"topic-list-header\"><tr>{}{}{}{}</tr></thead>\
         <tbody class=\"topic-list-body\">{rows}</tbody></table></div></div><div class=\"loading-container\"></div>\
         <div aria-hidden=\"true\" class=\"load-more-sentinel\"></div></div>",
        escape(&t(cx, "discourse_assign.topic_search_placeholder")),
        escape(&t(cx, "sr_topic_list_caption")),
        header("default", "topic.title", false),
        header("posts", "replies", true),
        header("views", "views", true),
        header("activity", "activity", true),
    )
}
