//! preferences/security.gjs (the password, UserPasskeys, the second
//! factor link, the recently used devices with AuthTokenDropdown,
//! UserApiKeys), preferences/apps.gjs, preferences/calendar-subscriptions
//! .gjs, preferences/second-factor.gjs and chat's preferences/chat.gjs.
//! static/js/prefs-security.js drives them.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use super::{Prefs, button};
use crate::AppError;
use crate::topic_list_view::{ListContext, escape, icon, t, t_count, t_with};

fn parse(v: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v.as_str()?)
        .ok()
        .map(|d| d.to_utc())
}

/// `dFormatDate(date)`, medium, without "ago".
fn format_date(cx: &ListContext, v: &Value, leave_ago: bool) -> String {
    parse(v)
        .map(|at| crate::user_profile_view::age_medium(cx, at, leave_ago))
        .unwrap_or_default()
}

/// SecurityController's DEFAULT_AUTH_TOKENS_COUNT
const DEFAULT_AUTH_TOKENS_COUNT: usize = 2;

/// What the security tab reads beside the user.
pub struct Security {
    /// `canUsePasskeys` but for isWebauthnSupported, which the browser
    /// answers: static/js/prefs-security.js removes the section without.
    pub passkeys: bool,
    /// CanCheckEmailsHelper: oneself, an admin, or a moderator where
    /// moderators see emails.
    pub can_check_emails: bool,
}

pub fn render_security(p: &Prefs, sec: &Security) -> Result<String, AppError> {
    let cx = p.cx;
    let u = p.u;
    let mut out = String::new();
    let local = !p.setting("enable_discourse_connect")? && p.setting("enable_local_logins")?;
    let can_change_password = u["is_anonymous"] != true && local;
    if can_change_password {
        let no_password = u["no_password"] == true;
        let staged = u["staged"] == true;
        out.push_str(&format!(
            "<div class=\"control-group pref-password\" data-setting-name=\"user-password\"><label class=\"control-label\">{}</label><div class=\"controls\"><button class=\"btn btn-default\"{} id=\"change-password-button\" data-login=\"{}\" data-set-password=\"{}\">{} {}</button>{}<span class=\"password-progress\"></span></div>",
            escape(&t(cx, "user.password.title")),
            if staged { " disabled=\"\"" } else { "" },
            escape(
                u["email"]
                    .as_str()
                    .filter(|e| !e.is_empty())
                    .or(u["username"].as_str())
                    .unwrap_or_default()
            ),
            escape(&t(cx, "user.change_password.set_password")),
            icon("envelope", None),
            escape(&t(
                cx,
                if no_password {
                    "user.change_password.set_password"
                } else {
                    "user.change_password.action"
                }
            )),
            if staged {
                format!(
                    "<div class=\"instructions\">{}</div>",
                    escape(&t(cx, "user.change_password.staged_user"))
                )
            } else {
                String::new()
            },
        ));
        if !no_password {
            // canRemovePassword: the staff hint, else another login method.
            let can_remove = u["can_remove_password"] == true
                || (u["is_anonymous"] != true
                    && local
                    && (u["associated_accounts"]
                        .as_array()
                        .is_some_and(|a| !a.is_empty())
                        || (sec.passkeys
                            && u["user_passkeys"].as_array().is_some_and(|a| !a.is_empty()))));
            out.push_str(&format!(
                "<div class=\"controls\"><button class=\"btn btn-danger\"{} id=\"remove-password-link\">{} {}</button></div>",
                if can_remove { "" } else { " disabled=\"\"" },
                icon("trash-can", None),
                escape(&t(cx, "user.change_password.remove")),
            ));
            if !can_remove {
                out.push_str(&format!(
                    "<div class=\"instructions\">{}</div>",
                    escape(&t(cx, "user.change_password.remove_disabled"))
                ));
            }
        }
        out.push_str("</div>");
        if sec.passkeys {
            out.push_str(&passkeys(p)?);
        }
        if p.viewing_self() {
            out.push_str(&format!(
                "<div class=\"control-group pref-second-factor\" data-setting-name=\"user-second-factor\"><label class=\"control-label\">{}</label><div class=\"instructions\">{}</div><div class=\"controls pref-second-factor\">{}</div></div>",
                escape(&t(cx, "user.second_factor.title")),
                escape(&t(cx, "user.second_factor.short_description")),
                button(
                    "btn-default btn-second-factor",
                    Some("lock"),
                    Some(&t(cx, "user.second_factor.enable")),
                    ""
                ),
            ));
        }
    }
    if sec.can_check_emails {
        out.push_str(&auth_tokens(p));
    }
    out.push_str(&api_keys(p));
    if p.viewing_self() && p.u["show_mcp_authorizations"] == true {
        return Err(crate::Unsupported("the MCP authorizations list").into());
    }
    out.push_str("<span></span><br><span></span>");
    Ok(out)
}

fn passkeys(p: &Prefs) -> Result<String, AppError> {
    let cx = p.cx;
    let show_actions = p.viewing_self();
    let mut rows = String::new();
    for key in p.u["user_passkeys"].as_array().into_iter().flatten() {
        let used = if key["last_used"].is_null() {
            escape(&t(cx, "user.passkeys.never_used"))
        } else {
            t_with(
                cx,
                "user.passkeys.last_used_date",
                &[("date", &format_date(cx, &key["last_used"], true))],
            )
        };
        rows.push_str(&format!(
            "<div class=\"row\"><div class=\"passkey-left\"><div class=\"row-passkey__name\">{}</div><div class=\"row-passkey__created-date\">{}</div><div class=\"row-passkey__used-date\">{used}</div></div>{}</div>",
            escape(key["name"].as_str().unwrap_or_default()),
            t_with(
                cx,
                "user.passkeys.added_date",
                &[("date", &format_date(cx, &key["created_at"], true))]
            ),
            if show_actions {
                format!(
                    "<div class=\"passkey-right\"><div class=\"actions\">{}</div></div>",
                    dropdown(
                        cx,
                        "passkey-options-dropdown",
                        &format!(" data-passkey-id=\"{}\"", key["id"]),
                        &[
                            ("edit", "pencil", "user.second_factor.edit"),
                            ("delete", "trash-can", "user.second_factor.delete"),
                        ],
                    )
                )
            } else {
                String::new()
            },
        ));
    }
    Ok(format!(
        "<div class=\"control-group pref-passkeys\"><label class=\"control-label\">{}</label><div class=\"instructions\">{}</div><div class=\"pref-passkeys__rows\">{rows}</div>{}</div>",
        escape(&t(cx, "user.passkeys.title")),
        escape(&t(cx, "user.passkeys.short_description")),
        if show_actions {
            format!(
                "<div class=\"controls pref-passkeys__add\">{}</div>",
                button(
                    "btn-default",
                    Some("plus"),
                    Some(&t(cx, "user.passkeys.add_passkey")),
                    ""
                )
            )
        } else {
            String::new()
        },
    ))
}

/// A DropdownSelectBox of actions with a wrench header; preferences.js
/// floats it and prefs-security.js acts on the chosen row.
fn dropdown(cx: &ListContext, class: &str, attrs: &str, rows: &[(&str, &str, &str)]) -> String {
    let items: String = rows
        .iter()
        .enumerate()
        .map(|(i, (id, icon_name, key))| {
            let name = escape(&t(cx, key));
            format!(
                "<li aria-checked=\"false\" role=\"menuitemradio\" data-index=\"{i}\" data-name=\"{name}\" data-value=\"{id}\" title=\"{name}\" tabindex=\"0\" class=\"select-kit-row dropdown-select-box-row\"><div class=\"icons\"><span class=\"selection-indicator\"></span>{}</div><div class=\"texts\"><span class=\"name\">{name}</span></div></li>",
                icon(icon_name, None)
            )
        })
        .collect();
    let filter_by = escape(&t(cx, "select_kit.select_to_filter"));
    format!(
        "<details class=\"select-kit single-select dropdown-select-box {class} prefs-combo\"{attrs}><summary aria-label=\"{filter_by}\" name=\"{filter_by}\" data-name=\"\" tabindex=\"0\" class=\"select-kit-header single-select-header dropdown-select-box-header btn no-text btn-icon btn-default\"><div class=\"select-kit-header-wrapper\">{} &#8203;</div></summary><div class=\"select-kit-body\"><ul aria-live=\"polite\" class=\"select-kit-collection\" role=\"menu\">{items}</ul></div></details>",
        icon("wrench", None),
    )
}

/// The recently used devices: the active one first, then by when each
/// was last seen; past the first two, hidden until Show all.
fn auth_tokens(p: &Prefs) -> String {
    let cx = p.cx;
    let mut tokens: Vec<&Value> = p.u["user_auth_tokens"]
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    tokens.sort_by(|a, b| {
        if a["is_active"] == true {
            std::cmp::Ordering::Less
        } else if b["is_active"] == true {
            std::cmp::Ordering::Greater
        } else {
            b["seen_at"]
                .as_str()
                .unwrap_or_default()
                .cmp(a["seen_at"].as_str().unwrap_or_default())
        }
    });
    // The rows past the first two wait in a template for Show all.
    let mut rows = String::new();
    let mut rest = String::new();
    for (i, token) in tokens.iter().enumerate() {
        let active = token["is_active"] == true;
        let s = |k: &str| escape(token[k].as_str().unwrap_or_default());
        let token_json = escape(
            &json!({
                "id": token["id"],
                "seen_at": token["seen_at"],
                "seen_at_html": format_date(cx, &token["seen_at"], false),
                "location": token["location"],
                "icon": token["icon"],
                "browser": token["browser"],
                "device": token["device"],
            })
            .to_string(),
        );
        let list = if i < DEFAULT_AUTH_TOKENS_COUNT {
            &mut rows
        } else {
            &mut rest
        };
        list.push_str(&format!(
            "<div class=\"row auth-token\" data-token=\"{token_json}\"><div class=\"auth-token-icon\">{}</div>{}<div class=\"auth-token-first\">{}</div><div class=\"auth-token-second\">{}</div></div>",
            icon(token["icon"].as_str().unwrap_or("question"), None),
            if active {
                String::new()
            } else {
                dropdown(
                    cx,
                    "auth-token-dropdown",
                    "",
                    &[
                        ("notYou", "user-xmark", "user.auth_tokens.not_you"),
                        ("logOut", "right-from-bracket", "user.log_out"),
                    ],
                )
            },
            t_with(
                cx,
                "user.auth_tokens.device_location",
                &[
                    ("device", &s("device")),
                    ("ip", &s("client_ip")),
                    ("location", &s("location")),
                ]
            ),
            if active {
                t_with(
                    cx,
                    "user.auth_tokens.browser_active",
                    &[("browser", &s("browser"))],
                )
            } else {
                t_with(
                    cx,
                    "user.auth_tokens.browser_last_seen",
                    &[
                        ("browser", &s("browser")),
                        ("date", &format_date(cx, &token["seen_at"], false)),
                    ],
                )
            },
        ));
    }
    let show_all = if tokens.len() > DEFAULT_AUTH_TOKENS_COUNT {
        format!(
            "<a href=\"\" class=\"auth-tokens-toggle\" data-show-few=\"{}\" data-show-all=\"{}\">{} <span>{}</span></a>",
            escape(&t(cx, "user.auth_tokens.show_few")),
            escape(&t_with(
                cx,
                "user.auth_tokens.show_all",
                &[("count", &tokens.len().to_string())]
            )),
            icon("angle-down", None),
            escape(&t_with(
                cx,
                "user.auth_tokens.show_all",
                &[("count", &tokens.len().to_string())]
            )),
        )
    } else {
        String::new()
    };
    format!(
        "<div class=\"control-group pref-auth-tokens\" data-setting-name=\"user-auth-tokens\"><label class=\"control-label\">{}</label><div class=\"instructions\">{}</div><div class=\"auth-tokens\">{rows}</div><template class=\"auth-tokens-rest\">{rest}</template>{show_all}<a class=\"pull-right text-danger log-out-all\" href=\"\">{} <span>{}</span></a></div>",
        escape(&t(cx, "user.auth_tokens.title")),
        escape(&t(cx, "user.auth_tokens.short_description")),
        icon("right-from-bracket", None),
        escape(&t(cx, "user.auth_tokens.log_out_all")),
    )
}

/// UserApiKeys: the apps given a user API key, each revocable (and the
/// revoke undoable while the page is open).
pub fn api_keys(p: &Prefs) -> String {
    let cx = p.cx;
    let Some(keys) = p.u["user_api_keys"].as_array().filter(|k| !k.is_empty()) else {
        return String::new();
    };
    let mut rows = String::new();
    for key in keys {
        // The date after the label in the viewer's zone: the div carries
        // its template (crate::viewer_dates), its text drawn after the span.
        let (expires_tz, expires) = match parse(&key["expires_at"]) {
            Some(at) => {
                let (tz, date) = crate::viewer_dates::text(
                    cx,
                    &format!(" {}", crate::viewer_dates::long_marker(cx, at)),
                );
                (
                    tz,
                    format!(
                        "<span>{}</span>{date}",
                        escape(&t(
                            cx,
                            if at <= cx.now {
                                "user.api_expired_at"
                            } else {
                                "user.api_expires_at"
                            }
                        )),
                    ),
                )
            }
            None => (
                String::new(),
                format!(
                    "<span>{}</span> {}",
                    escape(&t(cx, "user.api_expires_at")),
                    escape(&t(cx, "user.api_expires_never"))
                ),
            ),
        };
        let scopes: Vec<&str> = key["scopes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let scopes_html = if scopes.is_empty() {
            String::new()
        } else {
            format!(
                "<div class=\"user-api-key__scopes-toggle\"><a href=\"\">{} <span>{}</span></a></div><ul class=\"user-api-key__scopes-list\" style=\"display: none;\">{}</ul>",
                icon("caret-down", None),
                escape(&t_count(
                    cx,
                    "user.api_show_permissions",
                    scopes.len() as i64,
                    &[("count", &scopes.len().to_string())]
                )),
                scopes
                    .iter()
                    .map(|s| format!(
                        "<li class=\"user-api-key__scopes-list-item\">{}</li>",
                        escape(s)
                    ))
                    .collect::<String>()
            )
        };
        rows.push_str(&format!(
            "<div class=\"row user-api-key\" data-key-id=\"{}\"><div class=\"user-api-key__info\"><div class=\"user-api-key__name\">{}</div><div class=\"user-api-key__dates\"><div class=\"user-api-key__date-approved\"><span>{}</span> {}</div><div class=\"user-api-key__date-last-used\"><span>{}</span> {}</div><div class=\"user-api-key__date-expires\"{expires_tz}>{expires}</div></div></div><div class=\"user-api-key__actions\">{}{}</div>{scopes_html}</div>",
            key["id"],
            escape(key["application_name"].as_str().unwrap_or_default()),
            escape(&t(cx, "user.api_approved")),
            format_date(cx, &key["created_at"], false),
            escape(&t(cx, "user.api_last_used_at")),
            format_date(cx, &key["last_used_at"], false),
            button(
                "btn-default btn-small revoke-api-key",
                None,
                Some(&t(cx, "user.revoke_access")),
                ""
            ),
            button(
                "btn-default btn-small undo-revoke-api-key",
                None,
                Some(&t(cx, "user.undo_revoke_access")),
                " style=\"display: none;\""
            ),
        ));
    }
    format!(
        "<div class=\"control-group pref-user-api-keys\"><label class=\"control-label pref-user-api-keys__label\">{}</label><div class=\"user-api-keys\">{rows}</div></div>",
        escape(&t(cx, "user.apps"))
    )
}

pub fn render_apps(p: &Prefs) -> String {
    format!("{}<span></span>", api_keys(p))
}

/// The calendar tab, as it stands once its status has loaded.
pub fn render_calendar(p: &Prefs, has_subscription: bool) -> String {
    let cx = p.cx;
    let state = if has_subscription {
        format!(
            "<div class=\"calendar-subscriptions__active\"><p class=\"calendar-subscriptions__active-status\">{} {}</p><div class=\"calendar-subscriptions__actions\">{}{}</div></div>",
            icon("check", None),
            escape(&t(cx, "user.calendar_subscriptions.active_subscription")),
            button(
                "btn-primary calendar-regenerate",
                Some("arrows-rotate"),
                Some(&t(cx, "user.calendar_subscriptions.regenerate")),
                ""
            ),
            button(
                "btn-danger calendar-revoke",
                None,
                Some(&t(cx, "user.calendar_subscriptions.revoke")),
                ""
            ),
        )
    } else {
        button(
            "btn-primary calendar-generate",
            Some("calendar-days"),
            Some(&t(cx, "user.calendar_subscriptions.generate")),
            "",
        )
    };
    let strings = json!({
        "regenerate_confirm": t(cx, "user.calendar_subscriptions.regenerate_confirm"),
        "revoke_confirm": t(cx, "user.calendar_subscriptions.revoke_confirm"),
        "urls_warning": t(cx, "user.calendar_subscriptions.urls_warning"),
        "bookmarks": t(cx, "user.calendar_subscriptions.bookmarks"),
        "bookmarks_description": t(cx, "user.calendar_subscriptions.bookmarks_description"),
        "copy": t(cx, "user.calendar_subscriptions.copy"),
        "copied": t(cx, "user.calendar_subscriptions.copied"),
        "add_to_google": t(cx, "user.calendar_subscriptions.add_to_google"),
        "add_to_outlook": t(cx, "user.calendar_subscriptions.add_to_outlook"),
        "add_to_apple": t(cx, "user.calendar_subscriptions.add_to_apple"),
        "active_subscription": t(cx, "user.calendar_subscriptions.active_subscription"),
        "regenerate": t(cx, "user.calendar_subscriptions.regenerate"),
        "revoke": t(cx, "user.calendar_subscriptions.revoke"),
        "generate": t(cx, "user.calendar_subscriptions.generate"),
    });
    format!(
        "<div class=\"calendar-subscriptions\" data-strings=\"{}\"><p class=\"calendar-subscriptions__description\">{}</p>{state}</div>",
        escape(&strings.to_string()),
        escape(&t(cx, "user.calendar_subscriptions.description")),
    )
}

/// What the second factor page lists.
pub struct SecondFactorPage<'a> {
    pub totps: &'a [Value],
    pub security_keys: &'a [Value],
}

/// preferences/second-factor.gjs, its own section in the preferences
/// outlet (no tab is current).
pub fn render_second_factor(p: &Prefs, page: &SecondFactorPage) -> String {
    let cx = p.cx;
    let me = p.viewing_self();
    let item = |name: &Value, default_key: &str, class: &str, id: &Value| {
        let name = name
            .as_str()
            .filter(|n| !n.is_empty())
            .map(escape)
            .unwrap_or_else(|| escape(&t(cx, default_key)));
        format!(
            "<div class=\"second-factor-item row\"><div class=\"details\">{name}</div>{}</div>",
            if me {
                format!(
                    "<div class=\"actions\">{}</div>",
                    dropdown(
                        cx,
                        class,
                        &format!(" data-second-factor-id=\"{id}\""),
                        &[
                            ("edit", "pencil", "user.second_factor.edit"),
                            ("disable", "trash-can", "user.second_factor.disable"),
                        ],
                    )
                )
            } else {
                String::new()
            }
        )
    };
    let totps: String = page
        .totps
        .iter()
        .map(|f| {
            item(
                &f["name"],
                "user.second_factor.totp.default_name",
                "token-based-auth-dropdown",
                &f["id"],
            )
        })
        .collect();
    let keys: String = page
        .security_keys
        .iter()
        .map(|f| {
            item(
                &f["name"],
                "user.second_factor.security_key.default_name",
                "security-key-dropdown",
                &f["id"],
            )
        })
        .collect();
    let u = p.u;
    let backup = if u["second_factor_enabled"] == true {
        let details = if u["second_factor_backup_enabled"] == true {
            let count = u["second_factor_remaining_backup_codes"]
                .as_i64()
                .unwrap_or(0);
            t_count(
                cx,
                "user.second_factor_backup.manage",
                count,
                &[("count", &count.to_string())],
            )
        } else {
            button(
                "btn-default new-second-factor-backup",
                Some("plus"),
                Some(&t(cx, "user.second_factor_backup.enable_long")),
                "",
            )
        };
        let actions = if u["second_factor_backup_enabled"] == true && me {
            format!(
                "<div class=\"actions\">{}</div>",
                dropdown(
                    cx,
                    "two-factor-backup-dropdown",
                    "",
                    &[
                        (
                            "edit",
                            "arrows-rotate",
                            "user.second_factor_backup.regenerate"
                        ),
                        ("disable", "trash-can", "user.second_factor_backup.disable"),
                    ],
                )
            )
        } else {
            String::new()
        };
        format!("<div class=\"details\">{details}</div>{actions}")
    } else {
        escape(&t(cx, "user.second_factor_backup.enable_prerequisites"))
    };
    let disable_all = if !page.totps.is_empty() || !page.security_keys.is_empty() {
        format!(
            "<div class=\"control-group pref-second-factor-disable-all\"><div class=\"controls -actions\">{}<a class=\"cancel\" href=\"{}/preferences/security\">{}</a></div></div>",
            button(
                "btn-danger second-factor-disable-all",
                Some("ban"),
                Some(&t(cx, "user.second_factor.disable_all")),
                ""
            ),
            p.user_path,
            escape(&t(cx, "cancel")),
        )
    } else {
        String::new()
    };
    format!(
        "<section class=\"user-content user-preferences solo-preference second-factor\"><div class=\"loading-container\"><div class=\"form-vertical\"><div class=\"control-group totp\"><div class=\"controls\"><h2>{}</h2>{totps}{}</div></div><div class=\"control-group security-key\"><div class=\"controls\"><h2>{}</h2>{keys}{}</div></div><div class=\"control-group pref-second-factor-backup\"><div class=\"controls pref-second-factor-backup\"><h2>{}</h2><div class=\"second-factor-item row\">{backup}</div></div></div>{disable_all}</div></div></section>",
        escape(&t(cx, "user.second_factor.totp.title")),
        button(
            "btn-default new-totp",
            Some("plus"),
            Some(&t(cx, "user.second_factor.totp.add")),
            ""
        ),
        escape(&t(cx, "user.second_factor.security_key.title")),
        button(
            "btn-default new-security-key",
            Some("plus"),
            Some(&t(cx, "user.second_factor.security_key.add")),
            ""
        ),
        escape(&t(cx, "user.second_factor_backup.title")),
    )
}

/// The strings prefs-security.js's dialogs and actions show.
pub fn strings(
    cx: &ListContext,
    viewer_username: &str,
    viewer_path: &str,
    viewer_has_passkeys: bool,
) -> String {
    let doc = json!({
        "confirm_access_title": t(cx, "user.confirm_access.title"),
        "instructions": t(cx, "user.confirm_access.instructions"),
        "logged_in_as": t(cx, "user.confirm_access.logged_in_as"),
        "fine_print": t(cx, "user.confirm_access.fine_print"),
        "password_label": t(cx, "user.password.title"),
        "password_confirm": t(cx, "user.password.confirm"),
        "forgot_password": t(cx, "user.confirm_access.forgot_password"),
        "incorrect_password": t(cx, "user.confirm_access.incorrect_password"),
        "password_reset_email_sent": t(cx, "user.confirm_access.password_reset_email_sent"),
        "cannot_send_password_reset_email": t(cx, "user.confirm_access.cannot_send_password_reset_email"),
        "viewer_username": viewer_username,
        "viewer_path": viewer_path,
        "viewer_has_passkeys": viewer_has_passkeys,
        "passkey_confirm": t(cx, "user.passkeys.confirm_button"),
        "set_password": t(cx, "user.change_password.set_password"),
        "viewer_profile": t_with(cx, "user.profile_possessive", &[("username", viewer_username)]),
        "in_progress": t(cx, "user.change_password.in_progress"),
        "success": t(cx, "user.change_password.success"),
        "error": t(cx, "user.change_password.error"),
        "remove": t(cx, "user.change_password.remove"),
        "remove_detail": t(cx, "user.change_password.remove_detail"),
        "was_this_you": t(cx, "user.auth_tokens.was_this_you"),
        "was_this_you_description": t(cx, "user.auth_tokens.was_this_you_description"),
        "extended_description": t(cx, "user.second_factor.extended_description"),
        "details": t(cx, "user.auth_tokens.details"),
        "latest_post": t(cx, "user.auth_tokens.latest_post"),
        "secure_account": t(cx, "user.auth_tokens.secure_account"),
        "browser_and_device": t(cx, "user.auth_tokens.browser_and_device"),
        "cancel": t(cx, "cancel"),
        "modal_close": t(cx, "modal.close"),
    });
    format!(
        "<script type=\"application/json\" id=\"prefs-security-strings\">{}</script>",
        doc.to_string().replace('<', "\\u003c")
    )
}

/// chat's preferences/chat.gjs: a FormKit form of the chat options.
pub fn render_chat(p: &Prefs) -> Result<String, AppError> {
    let cx = p.cx;
    let checkbox = |name: &str, title_key: &str| {
        format!(
            "<div class=\"form-kit__container form-kit__field form-kit__field-checkbox --large\" data-name=\"{name}\" id=\"control-{name}\" data-control-type=\"checkbox\"><div class=\"form-kit__container-content\"><label class=\"form-kit__control-checkbox-label\"><input class=\"form-kit__control-checkbox\" id=\"chat-pref-{name}\" name=\"{name}\" type=\"checkbox\"{}><span class=\"form-kit__control-checkbox-checkmark\">{}</span><span class=\"form-kit__control-checkbox-content\"><span class=\"form-kit__control-checkbox-title\"><span>{}</span></span><span class=\"form-kit__control-checkbox-description\"></span></span></label></div></div>",
            if p.option(name) == true {
                " checked"
            } else {
                ""
            },
            icon("check", None),
            escape(&t(cx, title_key)),
        )
    };
    let optional = format!("({})", escape(&t(cx, "form_kit.optional")));
    let section = |title_key: &str, body: String| {
        format!(
            "<div class=\"form-kit__section\"><div class=\"form-kit__section-header\"><div class=\"form-kit__section-heading\"><h2 class=\"form-kit__section-title\">{}</h2></div></div>{body}</div>",
            escape(&t(cx, title_key))
        )
    };
    let reaction_type = p
        .option("chat_quick_reaction_type")
        .as_str()
        .unwrap_or("frequent");
    let mut personalization = String::new();
    if p.setting("enable_emoji")? {
        let radios: String = [
            ("frequent", "chat.quick_reaction_type.options.frequent"),
            ("custom", "chat.quick_reaction_type.options.custom"),
        ]
        .iter()
        .map(|(value, key)| {
            format!(
                "<div class=\"form-kit__field form-kit__field-radio\"><label for=\"chat-quick-reaction-{value}\" class=\"form-kit__control-radio-label\"><input class=\"form-kit__control-radio\" id=\"chat-quick-reaction-{value}\" name=\"chat_quick_reaction_type\" type=\"radio\" value=\"{value}\"{}><span class=\"form-kit__control-radio-content\">{}</span></label></div>",
                if reaction_type == *value { " checked" } else { "" },
                escape(&t(cx, key))
            )
        })
        .collect();
        personalization.push_str(&format!(
            "<div class=\"form-kit__container form-kit__field form-kit__field-radio-group --large\" data-name=\"chat_quick_reaction_type\" id=\"control-chat_quick_reaction_type\" data-control-type=\"radio-group\"><label for=\"chat-quick-reaction-type\" class=\"form-kit__container-title\"><span>{}</span> <span class=\"form-kit__container-optional\">{optional}</span></label><div class=\"form-kit__container-content\"><fieldset class=\"form-kit__fieldset form-kit__control-radio-group\" name=\"chat_quick_reaction_type\" id=\"chat-quick-reaction-type\">{radios}</fieldset></div></div>",
            escape(&t(cx, "chat.quick_reaction_type.title")),
        ));
        // CHAT_QUICK_REACTIONS_CUSTOM_DEFAULT
        let custom = p
            .option("chat_quick_reactions_custom")
            .as_str()
            .filter(|c| !c.is_empty())
            .unwrap_or("+1|heart|tada");
        let pickers: String = custom
            .split('|')
            .enumerate()
            .map(|(i, emoji)| {
                format!(
                    "<button aria-expanded=\"false\" class=\"btn no-text fk-d-menu__trigger emoji-picker-trigger btn-default chat-custom-reaction\" data-identifier=\"emoji-picker\" data-trigger=\"\" data-index=\"{i}\" data-emoji=\"{e}\" type=\"button\"><img width=\"20\" height=\"20\" src=\"{}\" title=\"{e}\" alt=\"{e}\" class=\"emoji\"> &#8203; </button>",
                    escape(&p.s.emojis().url(emoji, cx.base_path).unwrap_or_default()),
                    e = escape(emoji),
                )
            })
            .collect();
        let t_full = |key: &str| t(cx, key);
        personalization.push_str(&crate::plugins::chat::actions_view::picker_template(
            &t_full,
            cx.base_path,
            p.s.emojis(),
        ));
        personalization.push_str(&format!(
            "<div class=\"form-kit__container form-kit__field form-kit__field-custom --large\" data-name=\"chat_quick_reactions_custom\" id=\"control-chat_quick_reactions_custom\" data-control-type=\"custom\"{}><label class=\"form-kit__container-title\"><span>{}</span> <span class=\"form-kit__container-optional\">{optional}</span></label><div class=\"form-kit__container-content\"><div class=\"form-kit__control-custom\">{pickers}</div></div></div>",
            if reaction_type == "custom" {
                ""
            } else {
                " style=\"display: none;\""
            },
            escape(&t(cx, "chat.quick_reaction_type.options.custom")),
        ));
    }
    let mode = p
        .option("chat_separate_sidebar_mode")
        .as_str()
        .unwrap_or("default");
    let options: String = ["always", "fullscreen", "never"]
        .iter()
        .map(|v| {
            format!(
                "<option class=\"d-native-select__option form-kit__control-option\" value=\"{v}\"{}>{}</option>",
                if mode == *v { " selected" } else { "" },
                escape(&t(
                    cx,
                    &format!("admin.site_settings.chat_separate_sidebar_mode.{v}")
                ))
            )
        })
        .collect();
    personalization.push_str(&format!(
        "<div class=\"form-kit__container form-kit__field form-kit__field-select --large\" data-name=\"chat_separate_sidebar_mode\" id=\"control-chat_separate_sidebar_mode\" data-control-type=\"select\"><label for=\"chat-separate-sidebar-mode\" class=\"form-kit__container-title\"><span>{}</span> <span class=\"form-kit__container-optional\">{optional}</span></label><div class=\"form-kit__container-content\"><select class=\"form-kit__control-select d-native-select\" id=\"chat-separate-sidebar-mode\" name=\"chat_separate_sidebar_mode\">{options}</select></div></div>",
        escape(&t(cx, "chat.separate_sidebar_mode.title")),
    ));
    let saved_attrs = json!({
        "chat_enabled": p.option("chat_enabled"),
        "chat_announce_new_messages": p.option("chat_announce_new_messages"),
        "chat_new_message_sound": p.option("chat_new_message_sound"),
        "show_thread_title_prompts": p.option("show_thread_title_prompts"),
        "chat_email_frequency": p.option("chat_email_frequency"),
        "chat_separate_sidebar_mode": p.option("chat_separate_sidebar_mode"),
        "chat_quick_reaction_type": p.option("chat_quick_reaction_type"),
        "chat_quick_reactions_custom": p.option("chat_quick_reactions_custom"),
    });
    Ok(format!(
        "<div class=\"form-kit chat-preferences\" data-save-url=\"{}.json\" data-chat-attrs=\"{}\" data-saved-label=\"{}\">{}{}{}<div class=\"save-controls\"><button class=\"btn btn-primary form-kit__button\" type=\"submit\"><span class=\"d-button-label\">{}</span></button></div></div>",
        p.user_path,
        escape(&saved_attrs.to_string()),
        escape(&t(cx, "saved")),
        checkbox("chat_enabled", "chat.enable"),
        section("chat.personalization_title", personalization),
        section(
            "chat.accessibility_title",
            format!(
                "{}{}",
                checkbox(
                    "chat_announce_new_messages",
                    "chat.announce_new_messages.title"
                ),
                checkbox("chat_new_message_sound", "chat.new_message_sound.title")
            )
        ),
        escape(&t(cx, "submit")),
    ))
}
