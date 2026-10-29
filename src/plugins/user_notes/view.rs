//! discourse-user-notes' components on the server-rendered pages: the
//! post's notes icon (PostMetadataUserNotes, by the name or the avatar),
//! the post admin menu's "Add user note", the user card's, profile's,
//! admin user page's and review item's notes buttons, the review note
//! form's copy option and the notes modal (UserNotesModal, UserNote).
//! static/js/user-notes.js opens the modal and does what its buttons do.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::emojis::Emojis;
use crate::i18n::I18n;
use crate::site_settings::{SettingError, SiteSettings};
use crate::topic_list_view::{escape, icon, unescape_emoji};
use crate::viewer_dates::DateContext;

fn t(i18n: &I18n, key: &str) -> String {
    i18n.t(&format!("js.user_notes.{key}"))
        .unwrap_or(key)
        .to_string()
}

fn t_count(i18n: &I18n, count: i64) -> String {
    i18n.t_with("js.user_notes.show", &[("count", &count.to_string())])
        .unwrap_or_default()
}

/// What the components read, while the plugin is enabled.
pub struct UserNotesUi {
    /// user_notes_icon_placement is `avatar` (else by the name).
    pub avatar_placement: bool,
    pub emojis: Arc<Emojis>,
    /// The viewer is staff: the initializer and most connectors render
    /// only for staff.
    pub staff: bool,
}

impl UserNotesUi {
    /// None when the plugin is off.
    pub fn load(settings: &SiteSettings, staff: bool) -> Result<Option<UserNotesUi>, SettingError> {
        if !super::enabled(settings)? {
            return Ok(None);
        }
        Ok(Some(UserNotesUi {
            avatar_placement: settings.get("user_notes_icon_placement")?.to_s() == "avatar",
            emojis: settings.emojis().clone(),
            staff,
        }))
    }

    /// `dEmoji "memo"` with emoji on, else `dIcon "pen-to-square"`.
    pub fn memo(&self, base_path: &str) -> String {
        if self.emojis.enabled {
            unescape_emoji(":memo:", &self.emojis, base_path)
        } else {
            icon("pen-to-square", None)
        }
    }
}

/// `parseInt(user_notes_count || custom_fields.user_notes_count || 0)`
/// over a user document.
pub fn user_count(u: &Value) -> i64 {
    let count = |v: &Value| match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => Some(js_parse_int(s)),
        _ => None,
    };
    count(&u["user_notes_count"])
        .filter(|c| *c != 0)
        .or_else(|| count(&u["custom_fields"][super::COUNT_FIELD]))
        .unwrap_or(0)
}

/// `parseInt(s, 10)`, 0 for NaN.
fn js_parse_int(s: &str) -> i64 {
    let s = s.trim_start();
    let (neg, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let n: i64 = digits
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or(0);
    if neg { -n } else { n }
}

/// `post.user_custom_fields.user_notes_count > 0`
fn post_count(p: &Value) -> i64 {
    match &p["user_custom_fields"][super::COUNT_FIELD] {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(s) => js_parse_int(s),
        _ => 0,
    }
}

/// The post's notes icon, for staff, when its author has notes.
pub fn post_icon(ui: &UserNotesUi, base_path: &str, p: &Value) -> String {
    if !ui.staff || post_count(p) <= 0 {
        return String::new();
    }
    format!(
        "<span class=\"user-notes-icon\" data-user-notes=\"{}\" data-post-id=\"{}\">{}</span>",
        p["user_id"],
        p["id"],
        ui.memo(base_path)
    )
}

/// The post admin menu's button (addPostAdminMenuButton): its class, icon,
/// label and title, for staff.
pub fn admin_menu_button(ui: &UserNotesUi, i18n: &I18n) -> Option<(String, String)> {
    ui.staff
        .then(|| (t(i18n, "attach"), t(i18n, "attach_note_description")))
}

/// ShowUserNotes: "User notes (n)", or "User notes" without any.
fn show_button(i18n: &I18n, user_id: &Value, count: i64, kind: &str) -> String {
    let label = if count > 0 {
        t_count(i18n, count)
    } else {
        t(i18n, "title")
    };
    format!(
        "<button class=\"btn-default show-user-notes-btn btn btn-icon-text\" data-user-notes=\"{user_id}\" \
         data-user-notes-kind=\"{kind}\" data-label-title=\"{}\" data-label-show=\"{}\" type=\"button\">{}\
         <span class=\"d-button-label\">{}</span></button>",
        escape(&t(i18n, "title")),
        escape(&t(i18n, "show")),
        icon("pen-to-square", None),
        escape(&label)
    )
}

/// A flat button holding the memo, titled with the count.
fn memo_button(
    ui: &UserNotesUi,
    i18n: &I18n,
    base_path: &str,
    user_id: &Value,
    count: i64,
    kind: &str,
) -> String {
    format!(
        "<button class=\"btn-flat btn no-text\" data-user-notes=\"{user_id}\" data-user-notes-kind=\"{kind}\" \
         data-label-show=\"{}\" title=\"{}\" type=\"button\">{}</button>",
        escape(&t(i18n, "show")),
        escape(&t_count(i18n, count)),
        ui.memo(base_path)
    )
}

/// The user card's user-card-post-names connector (staff), inside the
/// outlet's div.
pub fn card(ui: &UserNotesUi, i18n: &I18n, base_path: &str, u: &Value) -> String {
    if !ui.staff {
        return String::new();
    }
    let count = user_count(u);
    let button = if count > 0 {
        memo_button(ui, i18n, base_path, &u["id"], count, "card")
    } else {
        String::new()
    };
    format!(
        "<div class=\"user-card-post-names-outlet show-user-notes-on-card\"><div class=\"show-user-notes-on-card\">{button}</div></div>"
    )
}

/// The profile's user-profile-controls connector (staff).
pub fn profile_controls(ui: &UserNotesUi, i18n: &I18n, u: &Value) -> String {
    if !ui.staff {
        return String::new();
    }
    format!(
        "<li class=\"user-profile-controls-outlet show-notes-on-profile\">{}</li>",
        show_button(i18n, &u["id"], user_count(u), "profile")
    )
}

/// The admin user page's admin-user-controls-after connector, for the
/// staff the page is for: drawn when the document has the plugin's
/// `user_notes_count`, which it has while the plugin is on.
pub fn admin_user_controls(i18n: &I18n, u: &Value) -> String {
    if u.get(super::COUNT_FIELD).is_none() {
        return String::new();
    }
    show_button(i18n, &u["id"], user_count(u), "admin")
}

/// The review item's after-reviewable-post-user connector, for an author
/// (shown with the plugin on, staff or not).
pub fn flags(ui: &UserNotesUi, i18n: &I18n, base_path: &str, user: &Value) -> String {
    let count = user_count(user);
    let button = if count > 0 {
        memo_button(ui, i18n, base_path, &user["id"], count, "flags")
    } else {
        String::new()
    };
    format!(
        "<div class=\"after-reviewable-post-user-outlet show-user-notes-on-flags\">\
         <div class=\"after-reviewable-post-user-outlet show-user-notes-on-flags\">{button}</div></div>"
    )
}

/// The review note form's reviewable-note-form-after-note connector
/// (staff): copy the note to the author's user notes.
pub fn copy_note_option(
    ui: &UserNotesUi,
    i18n: &I18n,
    r: &Value,
    target: Option<&Value>,
) -> String {
    if !ui.staff {
        return String::new();
    }
    let username = target
        .and_then(|u| u["username"].as_str())
        .filter(|u| !u.is_empty())
        .unwrap_or("user");
    let target_id = target
        .map(|u| u["id"].to_string())
        .filter(|id| id != "null")
        .unwrap_or_default();
    let id = format!("copy-note-to-user-{}", r["id"]);
    format!(
        "<div class=\"reviewable-note-form-after-note-outlet copy-reviewable-note-to-user-option\">\
         <div class=\"form-kit__container form-kit__field form-kit__field-checkbox --full\" data-name=\"copy_note_to_user\" \
         data-control-type=\"checkbox\"><div class=\"form-kit__container-content\">\
         <label class=\"form-kit__control-checkbox-label\" for=\"{id}\">\
         <input id=\"{id}\" class=\"form-kit__control-checkbox copy-note-to-user\" name=\"copy_note_to_user\" type=\"checkbox\" \
         data-user-id=\"{target_id}\" data-reviewable-id=\"{}\"><span class=\"form-kit__control-checkbox-content\">\
         <span class=\"form-kit__control-checkbox-title\">{}</span></span></label></div></div></div>",
        r["id"],
        escape(
            &i18n
                .t_with(
                    "js.user_notes.copy_reviewable_note",
                    &[("username", username)]
                )
                .unwrap_or_default()
        )
    )
}

/// The dates the modal shows (`dAgeWithTooltip`).
struct Dates<'a> {
    i18n: &'a I18n,
    relative_date_duration: i64,
}

impl DateContext for Dates<'_> {
    fn i18n(&self) -> &I18n {
        self.i18n
    }
    fn now(&self) -> DateTime<Utc> {
        crate::clock::now()
    }
    fn relative_date_duration(&self) -> i64 {
        self.relative_date_duration
    }
}

/// One note of the modal (UserNote), its raw cooked as DCookText cooks it.
fn note(i18n: &I18n, dates: &Dates, base_path: &str, n: &Value, cooked: &str) -> String {
    let by = &n["created_by"];
    let username = by["username"].as_str().unwrap_or("");
    let avatar = crate::post_view::avatar_img(
        by["avatar_template"].as_str().unwrap_or(""),
        24,
        &format!(" title=\"{}\" loading=\"lazy\"", escape(username)),
    );
    let mut info = format!("<span class=\"username\">{}</span>", escape(username));
    let at = n["created_at"]
        .as_str()
        .and_then(super::parse_time)
        .map(|t| t.with_timezone(&Utc));
    info.push_str("<span class=\"post-date\">");
    if let Some(at) = at {
        info.push_str(&crate::viewer_dates::tiny_span(dates, at));
    }
    info.push_str("</span>");
    if !n["reviewable_id"].is_null() {
        info.push_str(&format!(
            "<a class=\"btn btn-small btn-default show-reviewable\" href=\"{base_path}/review/{}\">{}</a>",
            escape(&crate::params::scalar(&n["reviewable_id"]).unwrap_or_default()),
            escape(&t(i18n, "show_flag"))
        ));
    }
    if n["can_delete"] == true {
        info.push_str(&format!(
            "<button class=\"btn-small btn-danger btn no-text btn-icon remove-user-note\" title=\"{}\" type=\"button\">{}\
             <span aria-hidden=\"true\">&#8203;</span></button>",
            escape(&t(i18n, "remove")),
            icon("far-trash-can", None)
        ));
    }
    let show_post = if n.get("post_id").is_some_and(|p| !p.is_null()) {
        format!(
            "<a class=\"btn btn-default btn-small show-post\" href=\"{}\">{}</a>",
            escape(n["post_url"].as_str().unwrap_or("")),
            escape(&t(i18n, "show_post"))
        )
    } else {
        String::new()
    };
    format!(
        "<div class=\"user-note\" data-note-id=\"{}\"><div class=\"posted-by\"><a data-user-card=\"{}\" href=\"{base_path}/u/{}\">{avatar}</a></div>\
         <div class=\"note-contents\"><div class=\"note-info\">{info}</div><div class=\"cooked\">{cooked}</div>{show_post}</div>\
         <div class=\"clearfix\"></div></div>",
        escape(n["id"].as_str().unwrap_or("")),
        escape(username),
        escape(&username.to_lowercase()),
    )
}

/// The notes modal's body (UserNotesModal): the form adding a note, then
/// the notes, newest first. `cooked` follows the document's notes.
#[allow(clippy::too_many_arguments)]
pub fn modal(
    i18n: &I18n,
    ui: &UserNotesUi,
    relative_date_duration: i64,
    base_path: &str,
    user_id: &str,
    post_id: Option<i64>,
    doc: &Value,
    cooked: &[String],
) -> String {
    let dates = Dates {
        i18n,
        relative_date_duration,
    };
    let field_id = format!("user-notes-content-{}", escape(user_id));
    let mut out = format!(
        "<div class=\"user-notes-modal-body\" data-user-id=\"{}\" data-post-id=\"{}\" data-title=\"{}\" \
         data-delete-confirm=\"{}\">",
        escape(user_id),
        post_id.map(|p| p.to_string()).unwrap_or_default(),
        escape(&t(i18n, "title")),
        escape(&t(i18n, "delete_confirm")),
    );
    out.push_str(&format!(
        "<form class=\"form-kit user-notes-form\" novalidate=\"\"><div class=\"form-kit__container form-kit__field form-kit__field-textarea --full\" \
         data-name=\"content\" data-control-type=\"textarea\"><label for=\"{field_id}\" class=\"form-kit__container-title\"><span>{}</span></label>\
         <div class=\"form-kit__container-content\"><textarea id=\"{field_id}\" class=\"form-kit__control-textarea\" name=\"content\" required></textarea>\
         </div></div><div class=\"form-kit__section form-kit__actions\"><button class=\"btn btn-primary form-kit__button btn-primary\" type=\"submit\">\
         <span class=\"d-button-label\">{}</span></button></div></form>",
        escape(&t(i18n, "attach_note_description")),
        escape(&t(i18n, "attach"))
    ));
    let notes = doc["user_notes"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    for (i, n) in notes.iter().enumerate() {
        out.push_str(&note(
            i18n,
            &dates,
            base_path,
            n,
            cooked.get(i).map(String::as_str).unwrap_or(""),
        ));
    }
    // What user-notes.js puts by a post's name when it gains notes.
    out.push_str(&format!(
        "<template class=\"user-notes-icon-template\">{}</template></div>",
        ui.memo(base_path)
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_read_as_parse_int_does() {
        assert_eq!(
            user_count(&json!({"custom_fields": {"user_notes_count": "3"}})),
            3
        );
        assert_eq!(user_count(&json!({"user_notes_count": 2})), 2);
        assert_eq!(
            user_count(&json!({"user_notes_count": 0, "custom_fields": {"user_notes_count": "4"}})),
            4
        );
        assert_eq!(user_count(&json!({})), 0);
        assert_eq!(js_parse_int("12abc"), 12);
        assert_eq!(js_parse_int("x"), 0);
    }

    #[test]
    fn posts_show_the_icon_only_with_notes() {
        let i18n = I18n::vendored().unwrap();
        let ui = UserNotesUi {
            avatar_placement: false,
            emojis: Arc::new(Emojis::default()),
            staff: true,
        };
        let with = json!({"id": 9, "user_id": 3, "user_custom_fields": {"user_notes_count": "1"}});
        let html = post_icon(&ui, "", &with);
        assert!(html.contains("class=\"user-notes-icon\""));
        assert!(html.contains("data-user-notes=\"3\""));
        assert!(html.contains("d-icon-pen-to-square"));
        assert_eq!(post_icon(&ui, "", &json!({"id": 9, "user_id": 3})), "");
        let member = UserNotesUi { staff: false, ..ui };
        assert_eq!(post_icon(&member, "", &with), "");
        assert!(admin_menu_button(&member, &i18n).is_none());
    }
}
