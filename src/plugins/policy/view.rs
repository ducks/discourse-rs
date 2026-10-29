//! discourse-policy's decorator on the server-rendered posts
//! (initializers/extend-for-policy.gjs `attachPolicy`): the first `.policy`
//! of the cooked HTML gets its content in `div.policy-body` and the
//! PostPolicy component after it, drawn from the post serializer's
//! `policy_*` keys: the accept and revoke buttons, who accepted and who
//! has not (one list shown at a time, the rest loaded on demand) and, for
//! staff and the author, the settings button. static/js/policy.js does
//! what the component does in the browser; a `policy_change` on the
//! topic's channel redraws the post.

use serde_json::Value;

use crate::pretty_text::cleanup::{append, fragment_root, has_class, new_element, parse, to_html};
use crate::topic_list_view::{ListContext, escape, t};

/// Stands for the component in the document until it is serialized.
const MARKER: &str = "policy-footer-marker";

/// The first element with the class `policy`, quoted or not
/// (`querySelector(".policy")`).
fn first_policy(node: &markup5ever_rcdom::Handle) -> Option<markup5ever_rcdom::Handle> {
    for child in node.children.borrow().iter() {
        if has_class(child, "policy") {
            return Some(child.clone());
        }
        if let Some(found) = first_policy(child) {
            return Some(found);
        }
    }
    None
}

/// What the component reads beside the post.
pub struct Viewer {
    /// `currentUser.staff || currentUser.id === post.user_id`
    pub can_manage: bool,
    /// The not-accepted list is the one shown (`showNotAccepted`).
    pub show_not_accepted: bool,
    /// getRawAvatarSize("tiny")
    pub tiny_avatar_size: i64,
}

/// The cooked HTML with the policy drawn, or as it was without one.
pub fn decorate(cx: &ListContext, viewer: &Viewer, post: &Value, cooked: &str) -> String {
    if !cooked.contains("policy") {
        return cooked.to_string();
    }
    let dom = parse(cooked);
    let Some(policy) = first_policy(&fragment_root(&dom)) else {
        return cooked.to_string();
    };
    let settings = read_settings(&policy);
    // policy.innerHTML = `<div class="policy-body">${policy.innerHTML}</div>`
    let body = new_element("div", &[("class", "policy-body")]);
    let children: Vec<_> = policy.children.borrow_mut().drain(..).collect();
    for child in children {
        append(&body, child);
    }
    append(&policy, body);
    append(&policy, new_element(MARKER, &[]));
    to_html(&dom).replacen(
        &format!("<{MARKER}></{MARKER}>"),
        &footer(cx, viewer, post, &settings),
        1,
    )
}

/// `_buildPolicyAttributes`: the accept and revoke labels the component
/// reads, escaped (`escapeExpression`) as the initializer does.
struct Settings {
    accept: Option<String>,
    revoke: Option<String>,
}

fn read_settings(policy: &markup5ever_rcdom::Handle) -> Settings {
    let data = |name: &str| {
        crate::pretty_text::cleanup::attr(policy, &format!("data-{name}")).filter(|v| !v.is_empty())
    };
    Settings {
        accept: data("accept"),
        revoke: data("revoke"),
    }
}

fn count(post: &Value, key: &str) -> i64 {
    post[key].as_i64().unwrap_or(0)
}

/// A DButton with a translated label (and an icon when given).
fn button(class: &str, icon: Option<&str>, label: &str) -> String {
    let icon_html = icon
        .map(|i| crate::post_view::d_icon(i, None))
        .unwrap_or_default();
    let kind = if icon.is_some() {
        "btn-icon-text"
    } else {
        "btn-text"
    };
    format!(
        "<button class=\"btn {kind} {class}\" type=\"button\">{icon_html}<span class=\"d-button-label\">{}</span></button>",
        escape(label)
    )
}

/// boundAvatarTemplate(template, "tiny")
pub fn tiny_avatar(template: &str, raw_size: i64) -> String {
    format!(
        "<img loading=\"lazy\" alt=\"\" width=\"20\" height=\"20\" src=\"{}\" class=\"avatar\">",
        escape(&template.replace("{size}", &raw_size.to_string()))
    )
}

/// The user avatars of a list with its load more link.
fn users(viewer: &Viewer, class: &str, users: &[Value], remaining: i64) -> String {
    let mut out = format!("<div class=\"users {class}\">");
    for user in users {
        out.push_str(&tiny_avatar(
            user["avatar_template"].as_str().unwrap_or(""),
            viewer.tiny_avatar_size,
        ));
    }
    if remaining > 0 {
        out.push_str(&format!(
            "<a class=\"load-more-users\" href> + {remaining} {}</a>",
            crate::post_view::d_icon("user", None)
        ));
    }
    out.push_str("</div>");
    out
}

/// The PostPolicy component.
fn footer(cx: &ListContext, viewer: &Viewer, post: &Value, settings: &Settings) -> String {
    let accepted = post["policy_accepted"] == true;
    let revoked = post["policy_revoked"] == true;
    let mut out = format!(
        "<div class=\"policy-footer\" data-post-id=\"{}\"><div class=\"policy-actions\">",
        post["id"]
    );
    if post["policy_can_accept"] == true {
        let mut classes = String::from("accept btn-accept-policy");
        if !accepted || revoked {
            classes.push_str(" btn-primary");
        }
        let label = settings
            .accept
            .clone()
            .unwrap_or_else(|| t(cx, "discourse_policy.accept_policy"));
        out.push_str(&button(
            &classes,
            accepted.then_some("check"),
            &escape(&label),
        ));
    }
    if post["policy_can_revoke"] == true {
        let mut classes = String::from("revoke btn-revoke-policy");
        if !revoked || accepted {
            classes.push_str(" btn-danger");
        }
        let label = settings
            .revoke
            .clone()
            .unwrap_or_else(|| t(cx, "discourse_policy.revoke_policy"));
        out.push_str(&button(
            &classes,
            revoked.then_some("check"),
            &escape(&label),
        ));
    }
    out.push_str("</div><div class=\"user-lists\">");
    let accepted_count = count(post, "policy_accepted_by_count");
    let not_accepted_count = count(post, "policy_not_accepted_by_count");
    // policyHasUsers: `(a || 0 + b || 0) > 0`, as JavaScript groups it.
    let has_users = not_accepted_count != 0 || accepted_count != 0;
    if !has_users && post["policy_stats"] == true {
        out.push_str(&format!(
            "<span class=\"no-possible-users\">{}</span>",
            escape(&t(cx, "discourse_policy.no_possible_users"))
        ));
    }
    let empty = Vec::new();
    if accepted_count != 0 {
        out.push_str(&format!(
            "<a class=\"toggle toggle-accepted\" href title=\"{}\"><span class=\"user-count\">{accepted_count}</span>{}</a>",
            escape(&t(cx, "discourse_policy.accepted_tooltip")),
            crate::post_view::d_icon("user-check", None)
        ));
        let list = post["policy_accepted_by"].as_array().unwrap_or(&empty);
        if !viewer.show_not_accepted && !list.is_empty() {
            out.push_str(&users(
                viewer,
                "accepted",
                list,
                accepted_count - list.len() as i64,
            ));
        }
    }
    if not_accepted_count != 0 {
        out.push_str(&format!(
            "<div class=\"separator\"></div><a class=\"toggle toggle-not-accepted\" href title=\"{}\"><span class=\"user-count\">{not_accepted_count}</span>{}</a>",
            escape(&t(cx, "discourse_policy.not_accepted_tooltip")),
            crate::post_view::d_icon("user-xmark", None)
        ));
        if viewer.show_not_accepted {
            let list = post["policy_not_accepted_by"].as_array().unwrap_or(&empty);
            out.push_str(&users(
                viewer,
                "not-accepted",
                list,
                not_accepted_count - list.len() as i64,
            ));
        }
    }
    out.push_str("</div>");
    if viewer.can_manage {
        out.push_str(&format!(
            "<button class=\"btn no-text btn-icon btn-default edit-policy-settings-btn\" type=\"button\">{}</button>",
            crate::post_view::d_icon("gear", None)
        ));
    }
    out.push_str("</div>");
    out
}

/// The component alone for a post, drawn again in the state policy.js
/// brought it to (the live route).
pub fn footer_for(cx: &ListContext, viewer: &Viewer, post: &Value) -> Option<String> {
    let cooked = post["cooked"].as_str()?;
    let dom = parse(cooked);
    let policy = first_policy(&fragment_root(&dom))?;
    let settings = read_settings(&policy);
    Some(footer(cx, viewer, post, &settings))
}

/// The policy builder's form data (`policyToFormData`), from the policy
/// the settings button edits or empty to insert one.
#[derive(Debug, Default, PartialEq)]
pub struct BuilderValues {
    pub groups: String,
    pub version: String,
    pub renew: String,
    pub renew_start: String,
    pub reminder: String,
    pub accept: String,
    pub revoke: String,
    pub add_users_to_group: String,
    pub private: bool,
}

impl BuilderValues {
    /// `_buildPolicyAttributes` over a post's first `.policy`: only
    /// `data-groups` (not the legacy `data-group`), the version at least
    /// 1, the button labels defaulted and escaped.
    pub fn from_cooked(cx: &ListContext, cooked: &str) -> Option<BuilderValues> {
        let dom = parse(cooked);
        let policy = first_policy(&fragment_root(&dom))?;
        let data = |name: &str| {
            crate::pretty_text::cleanup::attr(&policy, &format!("data-{name}"))
                .filter(|v| !v.is_empty())
        };
        let version = data("version")
            .filter(|v| crate::ruby::to_i(v) >= 1)
            .unwrap_or_else(|| "1".to_string());
        Some(BuilderValues {
            groups: data("groups").unwrap_or_default(),
            version,
            renew: data("renew").unwrap_or_default(),
            renew_start: data("renew-start").unwrap_or_default(),
            reminder: data("reminder").unwrap_or_default(),
            accept: escape(
                &data("accept").unwrap_or_else(|| t(cx, "discourse_policy.accept_policy")),
            ),
            revoke: escape(
                &data("revoke").unwrap_or_else(|| t(cx, "discourse_policy.revoke_policy")),
            ),
            add_users_to_group: data("add-users-to-group").unwrap_or_default(),
            private: data("private").as_deref() == Some("true"),
        })
    }

    /// A new policy's: version 1.
    pub fn new_policy() -> BuilderValues {
        BuilderValues {
            version: "1".to_string(),
            ..Default::default()
        }
    }
}

/// A site group the choosers offer: its name and whether it is automatic.
pub struct SiteGroup {
    pub id: i32,
    pub name: String,
    pub automatic: bool,
}

/// PolicyGroupInput's GroupChooser (a native multiple select here): the
/// site's groups but everyone, or only the real ones.
fn group_chooser(name: &str, groups: &[SiteGroup], selected: &str, real_only: bool) -> String {
    let chosen: Vec<&str> = selected.split(',').filter(|g| !g.is_empty()).collect();
    let mut out = format!(
        "<select class=\"policy-group-input\" id=\"control-{n}-input\" name=\"{n}\" multiple>",
        n = escape(name)
    );
    for g in groups {
        // AUTO_GROUPS: ids 0 to 14.
        let skip = if real_only {
            g.automatic || (0..=14).contains(&g.id)
        } else {
            g.id == 0
        };
        if skip {
            continue;
        }
        out.push_str(&format!(
            "<option value=\"{v}\"{}>{v}</option>",
            if chosen.contains(&g.name.as_str()) {
                " selected"
            } else {
                ""
            },
            v = escape(&g.name)
        ));
    }
    out.push_str("</select>");
    out
}

/// The PolicyBuilder modal's content (PolicyBuilderForm in DModal): the
/// form, with what static/js/policy.js needs to validate it, write the
/// markdown and title the modal, and its footer's button (Insert for a
/// new policy, Save changes for an edit).
pub fn builder(
    cx: &ListContext,
    groups: &[SiteGroup],
    v: &BuilderValues,
    edit_raw_url: Option<&str>,
) -> String {
    use crate::admin_form_kit::{Field, checkbox, input, select};
    let insert = edit_raw_url.is_none();
    let label = |key: &str| t(cx, &format!("discourse_policy.builder.{key}.label"));
    let tip = |key: &str| {
        Some(t(
            cx,
            &format!("discourse_policy.builder.{key}.description"),
        ))
    };
    let mut fields = String::new();
    fields.push_str(
        &Field {
            name: "groups",
            control: "custom",
            title: Some(label("groups")),
            format: Some("large"),
            tooltip: tip("groups"),
            inner: format!(
                "<div class=\"policy-builder-form__groups\">{}</div>",
                group_chooser("groups", groups, &v.groups, false)
            ),
            ..Default::default()
        }
        .render(),
    );
    for (name, key, kind, value, format) in [
        ("version", "version", "number", &v.version, "small"),
        ("renew", "renew", "number", &v.renew, "small"),
        ("renewStart", "renew-start", "date", &v.renew_start, "large"),
    ] {
        fields.push_str(
            &Field {
                name,
                control: if kind == "date" {
                    "input-date"
                } else {
                    "input-number"
                },
                title: Some(label(key)),
                format: Some(format),
                tooltip: tip(key),
                inner: input(name, value, kind, None, false),
                ..Default::default()
            }
            .render(),
        );
    }
    fields.push_str(
        &Field {
            name: "reminder",
            control: "custom",
            title: Some(label("reminder")),
            format: Some("large"),
            tooltip: tip("reminder"),
            inner: select(
                "reminder",
                &[
                    ("daily".to_string(), t(cx, "daily")),
                    ("weekly".to_string(), t(cx, "weekly")),
                ],
                &v.reminder,
                Some(&t(cx, "discourse_policy.builder.reminder.no_reminder")),
                false,
            ),
            ..Default::default()
        }
        .render(),
    );
    for (name, value) in [("accept", &v.accept), ("revoke", &v.revoke)] {
        fields.push_str(
            &Field {
                name,
                control: "input-text",
                title: Some(label(name)),
                format: Some("large"),
                tooltip: tip(name),
                inner: input(name, value, "text", None, false),
                ..Default::default()
            }
            .render(),
        );
    }
    fields.push_str(
        &Field {
            name: "addUsersToGroup",
            control: "custom",
            title: Some(label("add-users-to-group")),
            format: Some("large"),
            tooltip: tip("add-users-to-group"),
            inner: format!(
                "<div class=\"policy-builder-form__add-users-to-group\">{}</div>",
                group_chooser("addUsersToGroup", groups, &v.add_users_to_group, true)
            ),
            ..Default::default()
        }
        .render(),
    );
    let private_tip = tip("private");
    fields.push_str(
        &Field {
            name: "private",
            control: "checkbox",
            title: Some(label("private")),
            inner: checkbox(
                "private",
                v.private,
                Some(&label("private")),
                private_tip.as_deref(),
                false,
            ),
            ..Default::default()
        }
        .render(),
    );
    let button = if insert {
        t(cx, "discourse_policy.builder.insert")
    } else {
        t(cx, "discourse_policy.builder.save")
    };
    format!(
        "<div class=\"policy-builder\" data-title=\"{}\" data-insert=\"{insert}\" data-error-group=\"{}\" data-error-version=\"{}\" data-template=\"{}\" data-edit-reason=\"{}\" data-raw-url=\"{}\">{}<div class=\"policy-builder__footer\"><button class=\"btn btn-text btn-primary\" type=\"button\"><span class=\"d-button-label\">{}</span></button></div></div>",
        escape(&t(cx, "discourse_policy.builder.title")),
        escape(&t(cx, "discourse_policy.builder.errors.group")),
        escape(&t(cx, "discourse_policy.builder.errors.version")),
        escape(&t(cx, "discourse_policy.accept_policy_template")),
        escape(&t(cx, "discourse_policy.edit_reason")),
        escape(edit_raw_url.unwrap_or_default()),
        crate::admin_form_kit::form("policy-builder-form", &fields),
        escape(&button)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_escaped_twice_as_the_initializer_and_glimmer_do() {
        assert_eq!(
            button("accept", None, &escape("A & B")),
            "<button class=\"btn btn-text accept\" type=\"button\"><span class=\"d-button-label\">A &amp;amp; B</span></button>"
        );
    }
}
