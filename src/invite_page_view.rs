//! The invite page (templates/invites/show.gjs and its controller): the
//! signup progress bar and welcome header, who invited the reader, then
//! the login code form (CodeLoginForm's first step), the classic signup
//! form, or a signed-in member's Accept Invitation button. static/js/
//! invite.js drives the steps after the first.

use serde_json::Value;

use crate::topic_list_view::{ListContext, escape};

/// What the page shows besides the invite's info.
pub struct InviteView<'a> {
    /// `js.*` strings.
    pub t: &'a dyn Fn(&str) -> String,
    pub cx: &'a ListContext<'a>,
    /// invite_info (InvitesController#show_invite).
    pub info: &'a Value,
    pub key: &'a str,
    pub site_title: &'a str,
    pub discourse_connect: bool,
    pub local_logins: bool,
    /// showCodeInviteForm: login codes by email, with local logins.
    pub code_form: bool,
    pub must_approve: bool,
    pub full_name_visible: bool,
    pub full_name_required: bool,
    pub max_username_length: i64,
    /// create_account.disclaimer, when the site has both a ToS and a
    /// privacy policy.
    pub disclaimer: Option<String>,
    /// The path the forms post to (`/invites/show/<key>.json`) and the
    /// login code routes, under the base path.
    pub base_path: &'a str,
    pub csrf_token: &'a str,
}

fn button(classes: &str, label: &str, attrs: &str) -> String {
    format!(
        "<button class=\"btn {classes}\"{attrs}><span class=\"d-button-label\">{}</span></button>",
        escape(label)
    )
}

/// SignupProgressBar at `step`.
fn progress_bar(must_approve: bool, step: &str) -> String {
    let steps: &[&str] = if must_approve {
        &["signup", "activate", "approve", "login"]
    } else {
        &["signup", "activate", "login"]
    };
    let current = steps.iter().position(|s| *s == step).unwrap_or(0);
    let mut out = String::from("<div class=\"signup-progress-bar\">");
    for (i, _) in steps.iter().enumerate() {
        let state = match i.cmp(&current) {
            std::cmp::Ordering::Equal => "active",
            std::cmp::Ordering::Less => "completed",
            std::cmp::Ordering::Greater => "incomplete",
        };
        out.push_str(&format!(
            "<div class=\"signup-progress-bar__segment --{state}\"><div class=\"signup-progress-bar__step\"><div class=\"signup-progress-bar__circle\"></div></div></div>"
        ));
    }
    out.push_str("</div>");
    out
}

/// DInputTip with no validation yet: the failed state the form starts in.
fn input_tip(id: &str) -> String {
    format!("<div class=\"tip bad\" id=\"{id}\"></div>")
}

/// The page's `<section>`.
pub fn render(v: &InviteView<'_>) -> String {
    let t = v.t;
    let info = v.info;
    let existing = info["existing_user_id"].as_i64().is_some();
    let is_link = info["is_invite_link"].as_bool().unwrap_or(false);
    let email = info["email"].as_str().unwrap_or_default();
    let show_form = v.local_logins && !v.discourse_connect && !existing;
    let mut out = String::new();
    out.push_str(&format!(
        "<section><div class=\"container invites-show clearfix\" data-invite-key=\"{}\" data-base-path=\"{}\" data-csrf=\"{}\" data-invite-link=\"{}\" data-code-form=\"{}\">",
        escape(v.key),
        escape(v.base_path),
        escape(v.csrf_token),
        is_link,
        v.code_form && show_form,
    ));
    // showWelcomeHeader, showSignupProgressBar
    if !existing && !v.discourse_connect {
        out.push_str(&progress_bar(v.must_approve, "signup"));
        out.push_str(&format!(
            "<div class=\"login-welcome-header\"><h1 class=\"login-title\">{}</h1></div>",
            escape(&t("invites.welcome_to").replace("%{site_name}", v.site_title))
        ));
    }
    out.push_str("<div class=\"invite-form\"><div class=\"col-form\">");
    // showInviteIntroduction
    out.push_str(&format!(
        "<div class=\"invited-by\"><p>{}</p><p>{}</p></div>",
        escape(&t("invites.invited_by")),
        crate::user_info_view::user_info(v.cx, &info["invited_by"], None)
    ));
    if !is_link {
        out.push_str(&format!(
            "<p class=\"email-message\">{}</p>",
            t("invites.your_email").replace("%{email}", &escape(email))
        ));
    }
    if v.discourse_connect {
        out.push_str(&format!(
            "<a class=\"btn btn-primary discourse-connect raw-link\" href=\"{}/session/sso\">{}</a>",
            escape(v.base_path),
            escape(&t("continue"))
        ));
    }
    if show_form {
        if v.code_form {
            out.push_str(&code_form_email_step(v, is_link, email));
            if let Some(disclaimer) = &v.disclaimer {
                out.push_str(&format!("<div class=\"disclaimer\">{disclaimer}</div>"));
            }
        } else {
            out.push_str(&classic_form(v, is_link));
        }
    }
    if existing && !v.discourse_connect {
        if info["existing_user_can_redeem"].as_bool() == Some(true) {
            out.push_str(&button(
                "btn-primary accept-invitation",
                &t("invites.accept_invite"),
                " type=\"submit\"",
            ));
            out.push_str("<div class=\"alert alert-error\" hidden></div>");
        } else {
            out.push_str(&format!(
                "<div class=\"alert alert-error\">{}</div>",
                escape(
                    info["existing_user_can_redeem_error"]
                        .as_str()
                        .unwrap_or_default()
                )
            ));
        }
    }
    out.push_str("</div></div></div></section>");
    out
}

/// CodeLoginForm's email step: the invite's locked email and Send code,
/// or the email field of an invite link.
fn code_form_email_step(v: &InviteView<'_>, is_link: bool, email: &str) -> String {
    let t = v.t;
    if !is_link {
        return format!(
            "<div class=\"code-login-form\" data-email=\"{}\"><div class=\"code-login-form__email-step\"><p class=\"code-login-form__instructions\">{}</p>{}<div class=\"code-login-form__error\" role=\"alert\" hidden></div></div></div>",
            escape(email),
            escape(&t("invites.code_instructions").replace("%{email}", email)),
            button(
                "btn-primary code-login-form__continue",
                &t("invites.send_code"),
                " type=\"button\""
            ),
        );
    }
    format!(
        "<div class=\"code-login-form\"><form class=\"form-kit code-login-form__email-step\" novalidate=\"\" hx-boost=\"false\">\
         <div class=\"form-kit__container form-kit__field form-kit__field-input-email --full\" data-name=\"email\" id=\"control-email\" data-control-type=\"input-email\">\
         <label for=\"code-login-email\" class=\"form-kit__container-title\"><span>{}</span></label>\
         <div class=\"form-kit__container-content\"><div class=\"form-kit__control-input-wrapper\">\
         <input class=\"form-kit__control-input\" id=\"code-login-email\" name=\"email\" value=\"\" autocomplete=\"username\" autofocus=\"autofocus\" type=\"email\"></div></div></div>\
         <div aria-live=\"polite\" class=\"code-login-form__error\" role=\"alert\" hidden></div>\
         <div class=\"code-login-form__email-actions\">{}</div></form></div>",
        escape(&t("code_login.email_label")),
        button(
            "btn-primary form-kit__button btn-primary code-login-form__continue",
            &t("code_login.continue_button"),
            " type=\"submit\""
        ),
    )
}

/// The signup form login codes replace: email (for invite links),
/// username, password and name.
fn classic_form(v: &InviteView<'_>, is_link: bool) -> String {
    let t = v.t;
    let info = v.info;
    let mut out = String::from("<form class=\"invite-signup\" hx-boost=\"false\">");
    if is_link {
        out.push_str(&format!(
            "<div class=\"input email-input input-group\"><input id=\"new-account-email\" class=\"ember-text-field ember-view\" autofocus=\"autofocus\" name=\"email\" type=\"email\"><label class=\"alt-placeholder\" for=\"new-account-email\">{}</label>{}<div class=\"instructions\">{}</div></div>",
            escape(&t("user.email.title")),
            input_tip("account-email-validation"),
            escape(&t("user.email.instructions")),
        ));
    }
    let username = info["username"].as_str().unwrap_or_default();
    out.push_str(&format!(
        "<div class=\"input username-input input-group\"><input autocomplete=\"off\" class=\"{}\" id=\"new-account-username\" maxlength=\"{}\" name=\"username\" value=\"{}\" type=\"text\"><label class=\"alt-placeholder\" for=\"new-account-username\">{}</label>{}</div>",
        if username.is_empty() { "" } else { "value-entered" },
        v.max_username_length,
        escape(username),
        escape(&t("user.username.title")),
        input_tip("username-validation"),
    ));
    let name_input = |required: bool| {
        format!(
            "<div class=\"input name-input input-group{}\"><input aria-describedby=\"fullname-validation fullname-validation-more-info\" class=\"ember-text-field ember-view\" name=\"name\" id=\"new-account-name\" type=\"text\"><label class=\"alt-placeholder\" for=\"new-account-name\">{}</label>{}</div>",
            if required { " name-required" } else { "" },
            escape(&t("invites.name_label")),
            input_tip("fullname-validation"),
        )
    };
    if v.full_name_visible && v.full_name_required {
        out.push_str(&name_input(true));
    }
    out.push_str(&format!(
        "<div class=\"input password-input input-group\"><input placeholder=\"\" autocomplete=\"new-password\" id=\"new-account-password\" class=\"ember-text-field ember-view\" type=\"password\"><label class=\"alt-placeholder\" for=\"new-account-password\">{}</label>\
         <button class=\"btn no-text btn-icon btn-transparent toggle-password-mask\" title=\"{}\" type=\"button\"><svg class=\"fa d-icon d-icon-far-eye svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#far-eye\"></use></svg></button>\
         <div class=\"create-account__password-info\"><div class=\"create-account__password-tip-validation\">{}<div class=\"caps-lock-warning hidden\"><svg class=\"fa d-icon d-icon-triangle-exclamation svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#triangle-exclamation\"></use></svg> {}</div></div></div></div>",
        escape(&t("invites.password_label")),
        escape(&t("login.show_password_title")),
        input_tip("password-validation"),
        escape(&t("login.caps_lock_warning")),
    ));
    if v.full_name_visible && !v.full_name_required {
        out.push_str(&name_input(false));
    }
    out.push_str(&format!(
        "<div class=\"invitation-cta\">{}<div class=\"invitation-cta__info\"><span class=\"invitation-cta__signed-up\">{}</span><a class=\"btn btn-flat invitation-cta__sign-in\" href=\"{}/login\"><span class=\"d-button-label\">{}</span></a></div></div>",
        button(
            "btn-primary invitation-cta__accept",
            &t("invites.accept_invite"),
            " type=\"submit\""
        ),
        escape(&t("login.previous_sign_up")),
        escape(v.base_path),
        escape(&t("log_in")),
    ));
    out.push_str(&format!(
        "<div class=\"disclaimer\">{}</div>",
        v.disclaimer.as_deref().unwrap_or_default()
    ));
    out.push_str("<div class=\"alert alert-error\" hidden></div></form>");
    out
}

/// What the profile's Invites tab (user-invited and user-invited/show)
/// draws from.
pub struct InvitedView<'a> {
    pub cx: &'a ListContext<'a>,
    /// users#invited's document for the filter.
    pub doc: &'a Value,
    pub filter: &'a str,
    pub user_path: &'a str,
    /// currentUser.can_invite_to_forum and not a hidden profile.
    pub can_invite_to_forum: bool,
    pub viewing_self: bool,
    pub viewer_admin: bool,
    pub viewer_staff: bool,
    pub allow_bulk_invite: bool,
    pub mobile: bool,
    /// The search the list was narrowed by.
    pub search: &'a str,
}

const ENVELOPE_ZERO: &str = include_str!("../templates/empty_states/envelope_zero.svg");

fn icon(name: &str) -> String {
    format!(
        "<svg class=\"fa d-icon d-icon-{name} svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#{name}\"></use></svg>"
    )
}

fn icon_button(classes: &str, icon_name: &str, label: &str, attrs: &str) -> String {
    format!(
        "<button class=\"btn btn-icon-text {classes}\" type=\"button\"{attrs}>{}<span class=\"d-button-label\">{}</span></button>",
        icon(icon_name),
        escape(label)
    )
}

fn date(cx: &ListContext, at: &Value) -> String {
    match at
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
    {
        Some(at) => crate::user_profile_view::age_medium(cx, at.to_utc(), false),
        None => String::new(),
    }
}

/// rawDate (longDate) as a crate::viewer_dates template.
fn raw_date(cx: &ListContext, at: &Value) -> String {
    at.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|at| crate::viewer_dates::long_marker(cx, at.to_utc()))
        .unwrap_or_default()
}

/// user-invited.gjs's secondary navigation and user-invited/show.gjs.
pub fn invited(v: &InvitedView<'_>) -> String {
    use crate::topic_list_view::{t, t_with};
    let cx = v.cx;
    let doc = v.doc;
    let mut out = String::new();
    let details = doc["can_see_invite_details"] == true;
    let counts = &doc["counts"];
    if details {
        let total = counts["total"].as_i64().unwrap_or(0);
        let label = |name: &str| {
            if total > 0 {
                let n = counts[name].as_i64().unwrap_or(0).to_string();
                t_with(
                    cx,
                    &format!("user.invited.{name}_tab_with_count"),
                    &[("count", &n)],
                )
            } else {
                t(cx, &format!("user.invited.{name}_tab"))
            }
        };
        out.push_str("<div class=\"user-navigation user-navigation-secondary\"><nav aria-label=\"User secondary - invites\" class=\"horizontal-overflow-nav\"><div class=\"d-overflow-controls --owned-scroller horizontal-overflow-nav__controls\"><ul class=\"nav-pills action-list\" data-d-scroll-axis=\"horizontal\">");
        for name in ["pending", "expired", "redeemed"] {
            let active = v.filter == name;
            out.push_str(&format!(
                "<li{}><a class=\"{}\" href=\"{}/invited/{name}\">{}</a></li>",
                if active { " class=\"active\"" } else { "" },
                if active { "active" } else { "" },
                v.user_path,
                escape(&label(name))
            ));
        }
        out.push_str("</ul></div></nav></div>");
    }
    if !v.can_invite_to_forum {
        out.push_str(&format!(
            "<div class=\"alert alert-error invite-error\">{}</div>",
            escape(doc["error"].as_str().unwrap_or_default())
        ));
        return out;
    }
    let invites = doc["invites"].as_array().cloned().unwrap_or_default();
    let can_create = v.can_invite_to_forum && v.viewing_self;
    let can_bulk = v.viewer_admin && v.allow_bulk_invite && v.viewing_self;
    out.push_str(&format!(
        "<div class=\"user-content --loaded\" id=\"user-content\" data-filter=\"{}\" data-user-path=\"{}\">",
        escape(v.filter),
        escape(v.user_path),
    ));
    out.push_str("<section class=\"user-additional-controls\">");
    if counts[v.filter].as_i64().unwrap_or(0) > 5 {
        out.push_str(&format!(
            "<div class=\"user-invite-search\"><form hx-boost=\"false\"><input class=\"ember-text-field ember-view\" placeholder=\"{}\" type=\"text\" value=\"{}\"></form></div>",
            escape(&t(cx, "user.invited.search")),
            escape(v.search)
        ));
    }
    out.push_str("<section class=\"user-invite-buttons\">");
    if !invites.is_empty() {
        if can_create {
            out.push_str(&icon_button(
                "btn-default invite-button",
                "plus",
                &t(cx, "user.invited.create"),
                "",
            ));
        }
        if can_bulk && !v.mobile {
            out.push_str(&icon_button(
                "btn-default bulk-invite-button",
                "upload",
                &t(cx, "user.invited.bulk_invite.text"),
                "",
            ));
        }
    }
    if !invites.is_empty() && v.viewer_staff {
        if v.filter == "expired" {
            out.push_str(&icon_button(
                "btn-default bulk-remove-expired",
                "xmark",
                &t(cx, "user.invited.remove_all"),
                "",
            ));
        }
        if v.filter == "pending" && invites.iter().any(|i| i["email"].is_string()) {
            out.push_str(&icon_button(
                "btn-default reinvite-all",
                "arrows-rotate",
                &t(cx, "user.invited.reinvite_all"),
                "",
            ));
        }
    }
    out.push_str("</section></section><section>");
    if invites.is_empty() {
        out.push_str(&format!(
            "<div class=\"empty-state__container --empty-channels-list --with-image\"><div class=\"empty-state\"><div class=\"empty-state__image\">{ENVELOPE_ZERO}</div><div class=\"empty-state__title\" data-test-title>{}</div>",
            escape(&t(cx, "user.invited.none.title"))
        ));
        if can_create {
            out.push_str(&format!(
                "<div class=\"empty-state__cta\"><button class=\"btn btn-primary invite-button\" type=\"button\"><span class=\"d-button-label\">{}</span></button></div>",
                escape(&t(cx, "user.invited.none.cta"))
            ));
        }
        if can_bulk {
            out.push_str(&format!(
                "<div class=\"empty-state__tip\">{} {} <button class=\"btn btn-link bulk-invite-button\" type=\"button\"><span class=\"d-button-label\">{}</span></button> {}</div>",
                icon("upload"),
                escape(&t(cx, "user.invited.none.tip.prefix")),
                escape(&t(cx, "user.invited.none.tip.action")),
                escape(&t(cx, "user.invited.none.tip.suffix")),
            ));
        }
        out.push_str("</div></div>");
    } else if v.filter == "redeemed" {
        out.push_str(&redeemed_table(cx, &invites, details));
    } else {
        out.push_str(&invite_table(cx, &invites, v.filter == "expired"));
    }
    out.push_str("</section></div>");
    out
}

fn header_cells(cx: &ListContext, keys: &[&str]) -> String {
    keys.iter()
        .map(|k| {
            let label = if k.is_empty() {
                String::new()
            } else {
                escape(&crate::topic_list_view::t(cx, k))
            };
            format!("<th class=\"d-table__header-cell\">{label}</th>")
        })
        .collect()
}

fn detail_cell(cx: &ListContext, label_key: &str, classes: &str, content: &str) -> String {
    detail_cell_with(cx, label_key, classes, "", content)
}

/// detail_cell with attributes (already escaped) for the cell.
fn detail_cell_with(
    cx: &ListContext,
    label_key: &str,
    classes: &str,
    attrs: &str,
    content: &str,
) -> String {
    format!(
        "<td class=\"d-table__cell --detail{classes}\"{attrs}><div class=\"d-table__mobile-label\">{}</div>{content}</td>",
        escape(&crate::topic_list_view::t(cx, label_key))
    )
}

fn redeemed_table(cx: &ListContext, invites: &[Value], details: bool) -> String {
    use crate::topic_list_view::t;
    let mut keys = vec!["user.invited.user", "user.invited.redeemed_at"];
    if details {
        keys.extend([
            "user.last_seen",
            "user.invited.topics_entered",
            "user.invited.posts_read_count",
            "user.invited.time_read",
            "user.invited.days_visited",
            "user.invited.invited_via",
        ]);
    }
    let mut out = format!(
        "<table class=\"d-table user-invite-list\"><thead class=\"d-table__header\"><tr class=\"d-table__row\">{}</tr></thead><tbody class=\"d-table__body\">",
        header_cells(cx, &keys)
    );
    for invite in invites {
        let u = &invite["user"];
        let username = u["username"].as_str().unwrap_or_default();
        let path = format!("{}/u/{}", cx.base_path, escape(&username.to_lowercase()));
        let template = u["avatar_template"].as_str().unwrap_or_default();
        let prefix = if template.starts_with('/') && !template.starts_with("//") {
            cx.base_path
        } else {
            ""
        };
        let avatar = format!(
            "<img loading=\"lazy\" alt=\"\" width=\"20\" height=\"20\" src=\"{prefix}{}\" class=\"avatar\">",
            escape(&template.replace("{size}", "20"))
        );
        out.push_str(&format!(
            "<tr class=\"d-table__row\"><td class=\"d-table__cell --overview\"><a href=\"{path}\">{avatar}</a> <a href=\"{path}\">{}</a></td>",
            escape(username)
        ));
        out.push_str(&detail_cell(
            cx,
            "user.invited.redeemed_at",
            "",
            &date(cx, &invite["redeemed_at"]),
        ));
        if details {
            let n = |k: &str| u[k].as_i64().unwrap_or(0);
            let number = |k: &str| crate::user_profile_view::number(cx, n(k));
            out.push_str(&detail_cell(
                cx,
                "user.last_seen",
                "",
                &date(cx, &u["last_seen_at"]),
            ));
            out.push_str(&detail_cell(
                cx,
                "user.invited.topics_entered",
                "",
                &number("topics_entered"),
            ));
            out.push_str(&detail_cell(
                cx,
                "user.invited.posts_read_count",
                "",
                &number("posts_read_count"),
            ));
            let read = crate::user_profile_view::duration(cx, n("time_read"), "tiny");
            out.push_str(&detail_cell(
                cx,
                "user.invited.time_read",
                "",
                &escape(&read),
            ));
            let visited = format!(
                "<div><span title=\"{}\">{}</span> / <span title=\"{}\">{}</span></div>",
                escape(&t(cx, "user.invited.days_visited")),
                n("days_visited"),
                escape(&t(cx, "user.invited.account_age_days")),
                n("days_since_created"),
            );
            out.push_str(&detail_cell(cx, "user.invited.days_visited", "", &visited));
            let source = escape(invite["invite_source"].as_str().unwrap_or_default());
            out.push_str(&detail_cell(cx, "user.invited.invited_via", "", &source));
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table>");
    out
}

/// A link invite's `user.invited.invited_via_link` (its shortKey).
fn via_link(cx: &ListContext, invite: &Value, key: &str) -> String {
    let short: String = key.chars().take(4).collect();
    let count = invite["redemption_count"].as_i64().unwrap_or(0).to_string();
    let max = invite["max_redemptions_allowed"]
        .as_i64()
        .unwrap_or(0)
        .to_string();
    crate::topic_list_view::t_with(
        cx,
        "user.invited.invited_via_link",
        &[
            ("key", &format!("{short}...")),
            ("count", &count),
            ("max", &max),
        ],
    )
}

/// The invite's row menu: remove, and copy for a link.
fn invite_actions(cx: &ListContext, invite: &Value) -> String {
    use crate::topic_list_view::t;
    let copy = if invite["link"].is_string() {
        format!(
            "<li class=\"dropdown-menu__item\"><button class=\"btn btn-icon-text btn-transparent copy-invite-link\" type=\"button\" data-copied=\"{}\">{}<span class=\"d-button-label\">{}</span></button></li>",
            escape(&t(cx, "user.invited.invite.link_copied")),
            icon("copy"),
            escape(&t(cx, "user.invited.invite.copy_link"))
        )
    } else {
        String::new()
    };
    let edit = escape(&t(cx, "user.invited.edit"));
    format!(
        "<td class=\"d-table__cell --controls invite-actions\"><div class=\"d-table__cell-actions\">\
         <button class=\"btn btn-default btn-small edit-invite\" title=\"{edit}\" type=\"button\"><span class=\"d-button-label\">{edit}</span></button>\
         <button aria-expanded=\"false\" class=\"btn no-text btn-icon fk-d-menu__trigger invites-menu-trigger btn-default btn-small\" title=\"{}\" data-identifier=\"invites-menu\" type=\"button\">{}</button>\
         <div class=\"invites-menu-content\" hidden><ul class=\"dropdown-menu\">\
         <li class=\"dropdown-menu__item\"><button class=\"btn btn-icon-text btn-transparent --danger remove-invite\" type=\"button\" data-removed=\"{}\">{}<span class=\"d-button-label\">{}</span></button></li>{copy}\
         </ul></div></div></td>",
        escape(&t(cx, "more_options")),
        icon("ellipsis-vertical"),
        escape(&t(cx, "user.invited.removed")),
        icon("trash-can"),
        escape(&t(cx, "user.invited.remove")),
    )
}

fn invite_table(cx: &ListContext, invites: &[Value], expired_tab: bool) -> String {
    use crate::topic_list_view::t;
    let keys = [
        "user.invited.invited_via",
        "user.invited.sent",
        "user.invited.expires_at",
        "",
    ];
    let mut out = format!(
        "<table class=\"d-table user-invite-list\"><thead class=\"d-table__header\"><tr class=\"d-table__row\">{}</tr></thead><tbody class=\"d-table__body\">",
        header_cells(cx, &keys)
    );
    for invite in invites {
        let id = invite["id"].as_i64().unwrap_or(0);
        out.push_str(&format!(
            "<tr class=\"d-table__row\" data-invite-id=\"{id}\"><td class=\"d-table__cell --overview invite-type\"><div class=\"invite-shortkey\">"
        ));
        if let Some(link) = invite["link"].as_str() {
            out.push_str(&format!(
                "<input class=\"invite-link-target\" disabled=\"\" id=\"invite-link-{id}\" type=\"text\" value=\"{}\">",
                escape(link)
            ));
        }
        match (invite["email"].as_str(), invite["invite_key"].as_str()) {
            (Some(email), _) => out.push_str(&format!("{} {}", icon("envelope"), escape(email))),
            (None, Some(key)) => out.push_str(&format!(
                "{} {}",
                icon("link"),
                escape(&via_link(cx, invite, key))
            )),
            (None, None) => out.push_str(&format!(
                "{} <em>{}</em>",
                icon("link"),
                escape(&t(cx, "user.invited.generating_link"))
            )),
        }
        out.push_str("</div><div class=\"invite-details\">");
        if let Some(description) = invite["description"].as_str().filter(|d| !d.is_empty()) {
            out.push_str(&format!(
                "<div class=\"invite-description\">{}</div>",
                escape(description)
            ));
        }
        out.push_str("<div class=\"invite-groups\">");
        for g in invite["groups"].as_array().into_iter().flatten() {
            let name = escape(g["name"].as_str().unwrap_or_default());
            out.push_str(&format!(
                "<span class=\"invite-extra\"><a class=\"invite-extra-item-link\" href=\"{}/g/{name}\">{} {name}</a></span>",
                cx.base_path,
                icon("users"),
            ));
        }
        out.push_str("</div>");
        if let Some(topic) = invite["topics"].as_array().and_then(|t| t.first()) {
            out.push_str(&format!(
                "<span class=\"invite-extra invite-topic\"><a class=\"invite-extra-item-link\" href=\"{}/t/{}/{}\">{} {}</a></span>",
                cx.base_path,
                escape(topic["slug"].as_str().unwrap_or_default()),
                topic["id"].as_i64().unwrap_or(0),
                icon("file-lines"),
                escape(topic["title"].as_str().unwrap_or_default())
            ));
        }
        out.push_str("</div></td>");
        out.push_str(&detail_cell(
            cx,
            "user.invited.sent",
            " invite-updated-at",
            &date(cx, &invite["updated_at"]),
        ));
        let expires = if !expired_tab && invite["expired"] == true {
            t(cx, "user.invited.expired")
        } else {
            raw_date(cx, &invite["expires_at"])
        };
        // rawDate's text after the label, in the viewer's zone: the cell
        // carries the template (crate::viewer_dates).
        let (tz, expires) = crate::viewer_dates::text(cx, &expires);
        out.push_str(&detail_cell_with(
            cx,
            "user.invited.expires_at",
            " invite-expires-at",
            &tz,
            &expires,
        ));
        if invite["can_delete_invite"] == true {
            out.push_str(&invite_actions(cx, invite));
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table>");
    out
}
