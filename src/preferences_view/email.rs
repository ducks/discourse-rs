//! preferences/email.gjs: changing the primary address (or, with ?new,
//! adding one), which static/js/preferences.js validates as
//! EmailController does and sends to users_email#update or #create. The
//! template's own form, inside the tab's, is one an HTML parser drops.

use super::Prefs;
use crate::topic_list_view::{escape, icon, t};

/// The form; `new` is the route's `new` query param.
pub fn render(p: &Prefs, new: bool, viewer_admin: bool) -> String {
    let cx = p.cx;
    let email = if new {
        String::new()
    } else {
        p.u["email"].as_str().unwrap_or_default().to_string()
    };
    // EmailController#saveEmail's success message, by who is changing it.
    let success = if p.u["admin"] == true || p.u["moderator"] == true {
        t(cx, "user.change_email.success_staff")
    } else if viewer_admin {
        t(cx, "user.change_email.success_via_admin")
    } else {
        t(cx, "user.change_email.success")
    };
    format!(
        "<section class=\"user-content user-preferences solo-preference\" data-email-new=\"{new}\" data-email-old=\"{e}\" data-email-labels=\"{}\"><div class=\"control-group\"><label class=\"control-label\">{}</label><div class=\"controls\"><input placeholder=\"\" autofocus=\"\" value=\"{e}\" id=\"change-email\" class=\"ember-text-field input-xxlarge\" type=\"text\"><div class=\"instructions\">{}</div><div class=\"tip good\"></div></div></div><div class=\"controls save-button\"><button class=\"btn btn-primary\" disabled=\"\" type=\"submit\"><span class=\"d-button-label\">{}</span></button><a class=\"cancel\" href=\"{}/preferences/account\">{}</a></div></section>",
        escape(
            &serde_json::json!({
                "invalid": t(cx, "user.email.invalid"),
                "saving": t(cx, "saving"),
                "save": if new { t(cx, "user.add_email.add") } else { t(cx, "user.change") },
                "success": success,
                "error": t(cx, "user.change_email.error"),
                "back": t(cx, "user.change_email.back_to_preferences"),
                "x_icon": icon("xmark", None),
                "back_icon": icon("arrow-left", None),
            })
            .to_string()
        ),
        escape(&t(
            cx,
            if new {
                "user.add_email.title"
            } else {
                "user.change_email.title"
            }
        )),
        escape(&t(cx, "user.email.instructions")),
        escape(&if new {
            t(cx, "user.add_email.add")
        } else {
            t(cx, "user.change")
        }),
        p.user_path,
        escape(&t(cx, "cancel")),
        e = escape(&email),
    )
}
