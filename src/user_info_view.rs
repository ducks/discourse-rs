//! `DUserInfo` (ui-kit/d-user-info.gjs) at its default small size, as the
//! about page's staff lists and the users directory's rows draw it.

use serde_json::Value;

use crate::topic_list_view::{ListContext, escape};

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// A user (a BasicUserSerializer's fields) with its avatar at the large
/// size (48px) titled with the username; `heading_level` makes the
/// username link a heading (`@headingLevel`).
pub fn user_info(cx: &ListContext, u: &Value, heading_level: Option<u8>) -> String {
    let username = escape(s(&u["username"]));
    let path = format!(
        "{}/u/{}",
        cx.base_path,
        escape(&s(&u["username"]).to_lowercase())
    );
    let template = s(&u["avatar_template"]);
    let avatar = format!(
        "<img alt=\"\" width=\"48\" height=\"48\" src=\"{}{}\" class=\"avatar\" title=\"{username}\">",
        if template.starts_with('/') && !template.starts_with("//") {
            cx.base_path
        } else {
            ""
        },
        escape(&template.replace("{size}", "48"))
    );
    let name = u["name"].as_str().filter(|n| !n.is_empty());
    // prioritizeNameInUx
    let name_first = cx.settings.prioritize_name && name.is_some_and(|n| !n.trim().is_empty());
    let name_html = name
        .map(|n| {
            format!(
                " <span class=\"name-wrapper\"><a data-user-card=\"{username}\" href=\"{path}\"><span class=\"name\">{}</span></a></span>",
                escape(n)
            )
        })
        .unwrap_or_default();
    let (heading, role) = match heading_level {
        Some(level) => (format!(" aria-level=\"{level}\""), " role=\"heading\""),
        None => (String::new(), ""),
    };
    format!(
        "<div class=\"user-info small\" data-username=\"{username}\"><div class=\"user-image\"><div class=\"user-image-inner\"><a aria-hidden=\"true\" data-user-card=\"{username}\" href=\"{path}\">{avatar}</a></div></div>\
         <div class=\"user-detail\"><div class=\"name-line{}\"><span class=\"username-wrapper\"><a{heading} data-user-card=\"{username}\" href=\"{path}\"{role}><span class=\"username\">{username}</span></a></span>{name_html}</div>\
         <div class=\"title\">{}</div></div></div>",
        if name_first { " --name-first" } else { "" },
        escape(s(&u["title"])),
    )
}
