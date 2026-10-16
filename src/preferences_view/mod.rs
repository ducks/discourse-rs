//! The user preferences as the Ember app renders them (templates/
//! preferences.gjs and preferences/*.gjs with their controllers'
//! conditions, PreferenceCheckbox, ComboBox, DSaveControls,
//! UsernamePreference, EmailDropdown, UserField, TimezoneInput,
//! PushNotificationSelect, UserNotificationSchedule, and the plugins'
//! connectors on these tabs), from the users#show document.
//!
//! Each tab's form carries what its Save Changes sends (the controller's
//! saveAttrNames read off the model as User#save reads them) for
//! static/js/preferences.js, which updates it from the controls and puts
//! it to /u/:username.json.

pub mod email;
pub mod emails;
pub mod interface;
pub mod navigation_menu;
pub mod notifications;
pub mod profile;
pub mod security;
pub mod tracking;
pub mod users;

use serde_json::{Map, Value, json};

use crate::site_settings::SiteSettings;
use crate::topic_list_view::{ListContext, escape, icon, t, t_with};
use crate::{AppError, Unsupported};

/// The preferences routes, by their path under `/preferences`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PrefTab {
    Account,
    Security,
    Profile,
    Emails,
    Notifications,
    Tracking,
    Users,
    Tags,
    Interface,
    Apps,
    NavigationMenu,
    CalendarSubscriptions,
    Email,
    SecondFactor,
    /// chat's `preferences.chat`
    Chat,
}

impl PrefTab {
    /// `/u/:username/preferences/<path>`; None for a path no route has.
    pub fn from_path(path: &str) -> Option<PrefTab> {
        Some(match path {
            "account" => PrefTab::Account,
            "security" => PrefTab::Security,
            "profile" => PrefTab::Profile,
            "emails" => PrefTab::Emails,
            "notifications" => PrefTab::Notifications,
            "tracking" => PrefTab::Tracking,
            "users" => PrefTab::Users,
            "tags" => PrefTab::Tags,
            "interface" => PrefTab::Interface,
            "apps" => PrefTab::Apps,
            "navigation-menu" => PrefTab::NavigationMenu,
            "calendar-subscriptions" => PrefTab::CalendarSubscriptions,
            "email" => PrefTab::Email,
            "second-factor" => PrefTab::SecondFactor,
            "chat" => PrefTab::Chat,
            _ => return None,
        })
    }

    /// The tab's `subpageTitle` key (`user.preferences_nav.*`).
    pub fn title_key(self) -> &'static str {
        match self {
            PrefTab::Account => "user.preferences_nav.account",
            PrefTab::Security => "user.preferences_nav.security",
            PrefTab::Profile => "user.preferences_nav.profile",
            PrefTab::Emails => "user.preferences_nav.emails",
            PrefTab::Notifications => "user.preferences_nav.notifications",
            PrefTab::Tracking => "user.preferences_nav.tracking",
            PrefTab::Users => "user.preferences_nav.users",
            PrefTab::Tags => "user.preferences_nav.tags",
            PrefTab::Interface => "user.preferences_nav.interface",
            PrefTab::Apps => "user.preferences_nav.apps",
            PrefTab::NavigationMenu => "user.preferences_nav.navigation_menu",
            PrefTab::CalendarSubscriptions => "user.preferences_nav.calendar_subscriptions",
            PrefTab::Email => "user.preferences_nav.email",
            PrefTab::SecondFactor => "user.preferences_nav.second_factor",
            PrefTab::Chat => "chat.title_capitalized",
        }
    }
}

/// An admin client string (`admin_js.*`).
fn ta(cx: &ListContext, key: &str) -> String {
    cx.i18n
        .t(&format!("admin_js.{key}"))
        .unwrap_or(key)
        .to_string()
}

/// What every tab reads.
pub struct Prefs<'a> {
    pub cx: &'a ListContext<'a>,
    pub s: &'a SiteSettings,
    /// users#show's `user`.
    pub u: &'a Value,
    /// `{base}/u/{username_lower}`
    pub user_path: &'a str,
    pub viewer_id: Option<i32>,
    pub viewer_admin: bool,
    pub viewer_staff: bool,
}

impl Prefs<'_> {
    pub(crate) fn viewing_self(&self) -> bool {
        self.viewer_id.is_some() && self.viewer_id.map(i64::from) == self.u["id"].as_i64()
    }

    pub(crate) fn option(&self, key: &str) -> &Value {
        &self.u["user_option"][key]
    }

    pub(crate) fn setting(&self, name: &str) -> Result<bool, AppError> {
        Ok(self.s.get(name)?.truthy())
    }
}

/// The secondary navigation (preferences.gjs) over the tab's form.
pub fn shell(p: &Prefs, tab: PrefTab, chat_tab: bool, form_attrs: &str, content: &str) -> String {
    let cx = p.cx;
    let mut out = String::from(
        "<div class=\"user-navigation user-navigation-secondary\">\
         <nav aria-label=\"User secondary - preferences\" class=\"horizontal-overflow-nav\">\
         <div class=\"d-overflow-controls --owned-scroller horizontal-overflow-nav__controls\">\
         <ul class=\"nav-pills action-list\" data-d-scroll-axis=\"horizontal\">",
    );
    let mut items = vec![
        ("account", "circle-user", PrefTab::Account),
        ("security", "lock", PrefTab::Security),
        ("profile", "address-card", PrefTab::Profile),
        ("emails", "envelope", PrefTab::Emails),
        ("notifications", "bell", PrefTab::Notifications),
    ];
    if p.u["can_change_tracking_preferences"] == true {
        items.push(("tracking", "plus", PrefTab::Tracking));
    }
    items.push(("users", "users", PrefTab::Users));
    items.push(("interface", "desktop", PrefTab::Interface));
    items.push(("navigation-menu", "bars", PrefTab::NavigationMenu));
    items.push((
        "calendar-subscriptions",
        "calendar-days",
        PrefTab::CalendarSubscriptions,
    ));
    for (path, icon_name, item) in items {
        out.push_str(&nav_item(
            p,
            path,
            &icon(icon_name, None),
            &t(cx, item.title_key()),
            item == tab,
        ));
    }
    // chat's user-preferences-nav connector.
    if chat_tab {
        out.push_str(&nav_item(
            p,
            "chat",
            &crate::post_view::d_icon("d-chat", None),
            &t(cx, "chat.title_capitalized"),
            tab == PrefTab::Chat,
        ));
    }
    out.push_str("</ul></div></nav></div>");
    let dialog = json!({
        "ok_value": t(cx, "ok_value"),
        "cancel": t(cx, "cancel_value"),
        "yes_value": t(cx, "yes_value"),
        "no_value": t(cx, "no_value"),
        "modal.close": t(cx, "modal.close"),
    });
    out.push_str(&format!(
        "<section class=\"user-content user-preferences\" id=\"user-content\"><span></span>\
         <form class=\"form-vertical\"{form_attrs}>{content}</form>\
         <script type=\"application/json\" id=\"dialog-strings\">{}</script></section>",
        dialog.to_string().replace('<', "\\u003c")
    ));
    out
}

fn nav_item(p: &Prefs, path: &str, icon_html: &str, label: &str, current: bool) -> String {
    let class = format!("user-nav__preferences-{path}");
    format!(
        "<li{} class=\"{class}\"><a class=\"{}\" href=\"{}/preferences/{path}\">{icon_html} <span>{}</span></a></li>",
        if current {
            " aria-current=\"location\""
        } else {
            ""
        },
        if current { "active" } else { "" },
        p.user_path,
        escape(label)
    )
}

/// What a tab's Save Changes sends: `attrs` (User#save's data, a key per
/// saveAttrName the model defines, in User#save's order) and where.
pub fn form_attrs(p: &Prefs, attrs: &Map<String, Value>) -> String {
    let username_lower = p.u["username"].as_str().unwrap_or_default().to_lowercase();
    format!(
        " data-username=\"{}\" data-user-id=\"{}\" data-upload-labels=\"{}\" data-dialog-labels=\"{}\" data-save-url=\"{}.json\" data-save-attrs=\"{}\" data-saved-label=\"{}\" data-saving-label=\"{}\" data-save-label=\"{}\"",
        escape(&username_lower),
        p.u["id"],
        escape(
            &json!({
                "select_file": t(p.cx, "upload_selector.select_file"),
                "uploading": t(p.cx, "upload_selector.uploading"),
                "change": t(p.cx, "upload_selector.change"),
                "delete": t(p.cx, "upload_selector.delete"),
            })
            .to_string()
        ),
        escape(
            &json!({
                "ok_value": t(p.cx, "ok_value"),
                "cancel_value": t(p.cx, "cancel_value"),
                "yes_value": t(p.cx, "yes_value"),
                "no_value": t(p.cx, "no_value"),
                "close": t(p.cx, "modal.close"),
                "generic_error": t(p.cx, "generic_error"),
            })
            .to_string()
        ),
        escape(&format!("{}/u/{username_lower}", p.cx.base_path)),
        escape(&Value::Object(attrs.clone()).to_string()),
        escape(&t(p.cx, "saved")),
        escape(&t(p.cx, "saving")),
        escape(&t(p.cx, "save")),
    )
}

/// `models/user.js`' userFields and userOptionFields, in their order:
/// User#save sends the user's own then its options'.
const USER_FIELDS: &[&str] = &[
    "allowed_pm_usernames",
    "bio_raw",
    "card_background_upload_url",
    "custom_fields",
    "date_of_birth",
    "flair_group_id",
    "ignored_usernames",
    "locale",
    "location",
    "muted_tags",
    "muted_usernames",
    "name",
    "primary_group_id",
    "profile_background_upload_url",
    "sidebar_category_ids",
    "sidebar_tag_names",
    "status",
    "title",
    "tracked_tags",
    "user_fields",
    "user_notification_schedule",
    "watched_tags",
    "watching_first_post_tags",
    "website",
];

const USER_OPTION_FIELDS: &[&str] = &[
    "allow_private_messages",
    "auto_track_topics_after_msecs",
    "automatically_unpin_topics",
    "bookmark_auto_delete_preference",
    "color_scheme_id",
    "composition_mode",
    "dark_scheme_id",
    "default_calendar",
    "digest_after_minutes",
    "dynamic_favicon",
    "email_digests",
    "email_in_reply_to",
    "email_level",
    "email_messages_level",
    "email_previous_replies",
    "enable_allowed_pm_users",
    "enable_markdown_monospace_font",
    "enable_quoting",
    "enable_smart_lists",
    "enable_upcoming_change_available_notifications",
    "external_links_in_new_tab",
    "hidden_composer_toolbar_buttons",
    "hide_presence",
    "hide_profile",
    "homepage_id",
    "include_tl0_in_digests",
    "interface_color_mode",
    "like_notification_frequency",
    "mailing_list_mode",
    "mailing_list_mode_frequency",
    "new_topic_duration_minutes",
    "notification_level_when_replying",
    "notify_on_linked_posts",
    "push_notification_level",
    "seen_popups",
    "send_shortcut",
    "automatically_translate",
    "show_original_content",
    "sidebar_link_to_filtered_list",
    "sidebar_show_count_of_new_items",
    "skip_new_user_tips",
    "text_size",
    "theme_ids",
    "timezone",
    "title_count_mode",
    "understood_languages",
    "watched_precedence_over_muted",
];

/// User#save's data for `fields` (a tab's saveAttrNames), with the
/// plugins' saveable options (`addSaveableUserOption`) after core's:
/// the user's keys, then the options', a key only where the model has
/// one (an undefined property is no param; null is an empty one).
/// `values` overrides the model's (a controller's own inputs).
pub fn save_attrs(
    u: &Value,
    fields: &[&str],
    plugin_options: &[&str],
    values: &[(&str, Value)],
) -> Map<String, Value> {
    let mut out = Map::new();
    let given = |key: &str, model: Option<&Value>| -> Option<Value> {
        values
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
            .or_else(|| model.cloned())
    };
    for key in USER_FIELDS.iter().filter(|k| fields.contains(k)) {
        let model = u.as_object().and_then(|o| o.get(*key));
        if let Some(v) = given(key, model) {
            out.insert((*key).to_string(), v);
        }
    }
    for key in USER_OPTION_FIELDS
        .iter()
        .chain(plugin_options.iter())
        .filter(|k| fields.contains(k))
    {
        let model = u["user_option"].as_object().and_then(|o| o.get(*key));
        if let Some(v) = given(key, model) {
            out.insert((*key).to_string(), v);
        }
    }
    out
}

/// The account tab's associated accounts: each enabled login with the
/// user's account (and a revoke button) or a connect button, once the
/// accounts are loaded (the user's own page), else the check email button.
fn associated_accounts(p: &Prefs<'_>) -> Result<String, AppError> {
    let cx = p.cx;
    let mut out = format!(
        "<div class=\"control-group pref-associated-accounts\" data-setting-name=\"user-associated-accounts\"><label class=\"control-label\">{}</label>",
        escape(&t(cx, "user.associated_accounts.title"))
    );
    let Some(accounts) = p.u["associated_accounts"].as_array() else {
        out.push_str(&format!(
            "<div class=\"controls\">{}</div></div>",
            button(
                "btn-default check-email",
                Some("envelope"),
                Some(&ta(cx, "admin.users.check_email.text")),
                &format!(
                    " title=\"{}\"",
                    escape(&ta(cx, "admin.users.check_email.title"))
                )
            )
        ));
        return Ok(out);
    };
    out.push_str("<table class=\"associated-accounts\"><tbody>");
    for method in crate::auth::enabled(p.s)? {
        let account = accounts.iter().find(|a| a["name"] == method.name);
        let dashed = method.name.replace('_', "-");
        // prettyName: no provider here overrides it.
        let pretty = cx
            .i18n
            .t(&format!("js.login.{}.name", method.name))
            .unwrap_or(method.name)
            .to_string();
        let name = match (method.provider_url)(p.s)? {
            Some(url) => format!(
                "<a href=\"{}\" rel=\"noopener noreferrer\" target=\"_blank\">{}</a>",
                escape(&url),
                escape(&pretty)
            ),
            None => escape(&pretty),
        };
        let description = account
            .and_then(|a| a["description"].as_str())
            .unwrap_or_default();
        let icon_html = icon(method.icon.unwrap_or("user"), None);
        match account {
            Some(_) => {
                out.push_str(&format!(
                    "<tr class=\"{dashed} account-connected\"><td class=\"associated-account__icon\">{icon_html}</td><td><div class=\"associated-account__name\">{name}</div><div class=\"associated-account__description\">{}</div></td><td class=\"associated-account__actions\">",
                    escape(description)
                ));
                if (method.can_revoke)(p.s)? {
                    out.push_str(&button(
                        "btn-danger no-text revoke-account",
                        Some("trash-can"),
                        None,
                        &format!(
                            " title=\"{}\" data-provider=\"{}\"",
                            escape(&t(cx, "user.associated_accounts.revoke")),
                            escape(method.name)
                        ),
                    ));
                }
                out.push_str("</td></tr>");
            }
            None => {
                out.push_str(&format!(
                    "<tr class=\"{dashed}\"><td class=\"associated-account__icon {dashed}\">{icon_html}</td><td><div class=\"associated-account__name\">{name}</div><div class=\"associated-account__description\"></div></td><td class=\"associated-account__actions\">"
                ));
                if (method.can_connect)(p.s)? {
                    out.push_str(&button(
                        "btn-primary connect-account",
                        Some("plug"),
                        Some(&t(cx, "user.associated_accounts.connect")),
                        &format!(" data-provider=\"{}\"", escape(method.name)),
                    ));
                } else {
                    out.push_str(&escape(&t(cx, "user.associated_accounts.not_connected")));
                }
                out.push_str("</td></tr>");
            }
        }
    }
    out.push_str("</tbody></table></div>");
    Ok(out)
}

/// A DButton.
pub(crate) fn button(
    class: &str,
    icon_name: Option<&str>,
    label: Option<&str>,
    attrs: &str,
) -> String {
    let icon_html = icon_name.map(|i| icon(i, None)).unwrap_or_default();
    match label {
        Some(label) => format!(
            "<button class=\"btn{} {class}\"{attrs} type=\"button\">{icon_html}<span class=\"d-button-label\">{}</span></button>",
            if icon_name.is_some() {
                " btn-icon-text"
            } else {
                " btn-text"
            },
            escape(label)
        ),
        None => format!(
            "<button class=\"btn no-text btn-icon {class}\"{attrs} type=\"button\">{icon_html}<span aria-hidden=\"true\">&#8203;</span></button>"
        ),
    }
}

/// PreferenceCheckbox, bound to the save attribute `key` (None for one
/// the controller reads itself).
pub(crate) fn checkbox(
    class: &str,
    setting: Option<&str>,
    checked: bool,
    label: &str,
    key: Option<&str>,
) -> String {
    format!(
        "<div class=\"controls{}\"{}><label class=\"checkbox-label\"><input class=\"ember-checkbox\"{}{} type=\"checkbox\"> {}</label></div>",
        if class.is_empty() {
            String::new()
        } else {
            format!(" {class}")
        },
        setting
            .map(|s| format!(" data-setting-name=\"{s}\""))
            .unwrap_or_default(),
        if checked { " checked" } else { "" },
        key.map(|k| format!(" data-pref=\"{k}\""))
            .unwrap_or_default(),
        escape(label)
    )
}

/// A ComboBox: its header, and its rows for static/js/preferences.js to
/// float under it. `rows` are (value, name); `value` the selected one.
pub(crate) struct Combo<'a> {
    pub id: Option<&'a str>,
    /// The component's own classes (before has-selection).
    pub component_class: &'a str,
    /// The invocation's `class` (after the component's).
    pub class: &'a str,
    pub rows: Vec<(String, String)>,
    pub value: Option<String>,
    /// `options.none`: the label when nothing is selected, and a row.
    pub none: Option<String>,
    /// The save attribute it sets.
    pub key: Option<&'a str>,
    pub filterable: bool,
}

pub(crate) fn combo(cx: &ListContext, c: &Combo) -> String {
    let selected = c
        .value
        .as_ref()
        .and_then(|v| c.rows.iter().find(|(value, _)| value == v));
    let (name, value_attr) = match selected {
        Some((value, name)) => (name.clone(), format!(" data-value=\"{}\"", escape(value))),
        None => (c.none.clone().unwrap_or_default(), String::new()),
    };
    let filter_by = t_with(cx, "select_kit.filter_by", &[("name", &name)]);
    let mut rows = String::new();
    let mut index = 0;
    if let Some(none) = &c.none {
        rows.push_str(&format!(
            "<li aria-checked=\"{}\" role=\"menuitemradio\" data-index=\"0\" data-name=\"{n}\" data-value=\"\" title=\"{n}\" tabindex=\"0\" class=\"select-kit-row none{}\"><span class=\"name\">{n}</span></li>",
            selected.is_none(),
            if selected.is_none() {
                " is-highlighted"
            } else {
                ""
            },
            n = escape(none)
        ));
        index = 1;
    }
    for (value, row_name) in &c.rows {
        let is = selected.is_some_and(|(v, _)| v == value);
        rows.push_str(&format!(
            "<li aria-checked=\"{is}\" role=\"menuitemradio\" data-index=\"{index}\" data-name=\"{n}\" data-value=\"{v}\" title=\"{n}\" tabindex=\"0\" class=\"select-kit-row{}\"><span class=\"name\">{n}</span></li>",
            if is { " is-highlighted is-selected" } else { "" },
            n = escape(row_name),
            v = escape(value),
        ));
        index += 1;
    }
    let filter = if c.filterable {
        format!(
            "<div class=\"select-kit-filter\"><input class=\"filter-input\" autocomplete=\"off\" placeholder=\"{}\" spellcheck=\"false\" type=\"search\">{}</div>",
            escape(&t(cx, "select_kit.filter_placeholder")),
            icon("magnifying-glass", Some("filter-icon"))
        )
    } else {
        String::new()
    };
    let classes = [
        "select-kit single-select combobox combo-box",
        c.component_class,
        if selected.is_some() {
            "has-selection"
        } else {
            ""
        },
        c.class,
    ]
    .iter()
    .filter(|s| !s.is_empty())
    .copied()
    .collect::<Vec<_>>()
    .join(" ");
    format!(
        "<details{} class=\"{classes} prefs-combo\"{}><summary aria-label=\"{fb}\" name=\"{fb}\" data-name=\"{n}\"{value_attr} tabindex=\"0\"{} class=\"select-kit-header single-select-header combo-box-header\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"{n}\"{value_attr} title=\"{n}\"><span class=\"name\">{n}</span></div>{}</div></summary><div{} class=\"select-kit-body\">{filter}<ul aria-live=\"polite\" class=\"select-kit-collection\" role=\"menu\">{rows}</ul></div></details>",
        c.id.map(|i| format!(" id=\"{i}\"")).unwrap_or_default(),
        c.key
            .map(|k| format!(" data-pref=\"{k}\""))
            .unwrap_or_default(),
        c.id.map(|i| format!(" id=\"{i}-header\""))
            .unwrap_or_default(),
        icon("angle-down", Some("angle-icon")),
        c.id.map(|i| format!(" id=\"{i}-body\""))
            .unwrap_or_default(),
        fb = escape(&filter_by),
        n = escape(&name),
    )
}

/// DSaveControls
pub(crate) fn save_controls(cx: &ListContext) -> String {
    format!(
        "<div class=\"controls save-button\"><button class=\"btn btn-primary save-changes\" type=\"button\"><span class=\"d-button-label\">{}</span></button></div>",
        escape(&t(cx, "save"))
    )
}

/// A value as select-kit compares it (`String(value)`).
pub(crate) fn value_string(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        v => Some(v.to_string()),
    }
}

/// What the account tab reads beside the user.
pub struct Account {
    /// `model.availableTitles`: the visible groups' titles and the
    /// title-granting badges' names (UserBadge.findByUsername), unique
    /// and sorted.
    pub titles: Vec<String>,
    /// `currentUser.can_delete_account`
    pub viewer_can_delete_account: bool,
    /// `currentUser.can_upload_avatar`
    pub viewer_can_upload_avatar: bool,
    /// The selectable_avatars setting's uploads, as the client setting
    /// holds them (their urls).
    pub selectable_avatars: Vec<String>,
    /// `siteSettings.site_logo_small_url`
    pub site_logo_small_url: Option<String>,
}

/// What the avatar selector modal (modal/avatar-selector.gjs) reads, for
/// static/js/preferences.js to draw it: the user's templates and upload
/// ids, what the viewer may choose and the strings.
fn avatar_selector(p: &Prefs, a: &Account) -> Result<Value, AppError> {
    let cx = p.cx;
    let u = p.u;
    let s = p.s;
    // allowsImages(staff): an image type among the authorized extensions.
    let mut extensions = s.get("authorized_extensions")?.to_s();
    if p.viewer_staff {
        extensions.push('|');
        extensions.push_str(&s.get("authorized_extensions_for_staff")?.to_s());
    }
    let allows_images = extensions.split('|').any(|e| {
        let e = e.trim().trim_start_matches('.').to_lowercase();
        e == "*"
            || matches!(
                e.as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" | "heic" | "heif" | "webp" | "avif"
            )
    });
    let allow_upload = a.viewer_can_upload_avatar && allows_images;
    let gravatar_name = s.get("gravatar_name")?.to_s();
    Ok(json!({
        "username": u["username"],
        "user": {
            "id": u["id"],
            "avatar_template": u["avatar_template"],
            "system_avatar_template": u["system_avatar_template"],
            "system_avatar_upload_id": u["system_avatar_upload_id"],
            "gravatar_avatar_template": u["gravatar_avatar_template"],
            "gravatar_avatar_upload_id": u["gravatar_avatar_upload_id"],
            "custom_avatar_template": u["custom_avatar_template"],
            "custom_avatar_upload_id": u["custom_avatar_upload_id"],
            "use_logo_small_as_avatar": u["use_logo_small_as_avatar"],
            "trust_level": u["trust_level"],
            "admin": u["admin"],
            "moderator": u["moderator"],
            "email": u["email"],
        },
        "selectable_avatars_mode": s.get("selectable_avatars_mode")?.to_s(),
        "selectable_avatars": a.selectable_avatars,
        "site_logo_small_url": a.site_logo_small_url,
        "allow_upload": allow_upload,
        "allow_gravatar": allow_upload && s.get("gravatar_enabled")?.truthy(),
        "labels": {
            "title": t(cx, "user.change_avatar.title"),
            "use_custom": t(cx, "user.change_avatar.use_custom"),
            "logo_small": t(cx, "user.change_avatar.logo_small"),
            "letter_based": t(cx, "user.change_avatar.letter_based"),
            "gravatar": t_with(cx, "user.change_avatar.gravatar", &[
                ("gravatarName", &gravatar_name),
                ("gravatarBaseUrl", &s.get("gravatar_base_url")?.to_s()),
                ("gravatarLoginUrl", &s.get("gravatar_login_url")?.to_s()),
            ]),
            "refresh_gravatar_title": t_with(cx, "user.change_avatar.refresh_gravatar_title", &[("gravatarName", &gravatar_name)]),
            "gravatar_failed": t_with(cx, "user.change_avatar.gravatar_failed", &[("gravatarName", &gravatar_name)]),
            "uploaded_avatar": t(cx, "user.change_avatar.uploaded_avatar"),
            "uploaded_avatar_empty": t(cx, "user.change_avatar.uploaded_avatar_empty"),
            "upload_title": t(cx, "user.change_avatar.upload_title"),
            "image_is_not_a_square": t(cx, "user.change_avatar.image_is_not_a_square"),
            "upload": t(cx, "upload"),
            "uploading": t(cx, "uploading"),
            "save": t(cx, "save"),
            "cancel": t(cx, "cancel"),
            "close": t(cx, "modal.close"),
        },
    }))
}

/// preferences/account.gjs
pub fn account(p: &Prefs, a: &Account) -> Result<(String, Map<String, Value>), AppError> {
    let cx = p.cx;
    let u = p.u;
    let username = u["username"].as_str().unwrap_or_default();
    let mut out = String::new();

    // UsernamePreference
    out.push_str(&format!(
        "<div class=\"control-group pref-username\" data-setting-name=\"user-username\"><label class=\"control-label\">{}</label><div class=\"username-preference\" data-username=\"{}\" data-min-length=\"{}\" data-max-length=\"{}\" data-labels=\"{}\"><div class=\"controls\"><span class=\"static username-preference__current-username\">{}</span>",
        escape(&t(cx, "user.username.title")),
        escape(username),
        p.s.get("min_username_length")?.to_i(),
        p.s.get("max_username_length")?.to_i(),
        escape(
            &json!({
                "change": t(cx, "user.change"),
                "saving": t(cx, "saving"),
                "cancel": t(cx, "cancel"),
                "taken": t(cx, "user.change_username.taken"),
                "too_short": t(cx, "user.name.too_short"),
                "confirm": t(cx, "user.change_username.confirm"),
                "yes": t(cx, "yes_value"),
                "no": t(cx, "no_value"),
            })
            .to_string()
        ),
        escape(username)
    ));
    if u["can_edit_username"] == true {
        out.push(' ');
        out.push_str(&button(
            "btn-default btn-small username-preference__edit-username",
            Some("pencil"),
            None,
            &format!(" title=\"{}\"", escape(&t(cx, "user.username.edit"))),
        ));
    }
    out.push_str("</div>");
    if p.setting("enable_mentions")? {
        out.push_str(&format!(
            "<div class=\"instructions\">{}</div>",
            t_with(
                cx,
                "user.username.short_instructions",
                &[("username", &escape(username))]
            )
        ));
    }
    out.push_str("</div></div>");

    if !(p.setting("discourse_connect_overrides_avatar")? || p.setting("auth_overrides_avatar")?) {
        let data = |key: &str| {
            value_string(&u[key])
                .map(|v| format!(" data-{}=\"{}\"", key.replace('_', "-"), escape(&v)))
                .unwrap_or_default()
        };
        out.push_str(&format!(
            "<div class=\"control-group pref-avatar\" data-setting-name=\"user-avatar\"><label class=\"control-label\" id=\"profile-picture\">{}</label><input{}{} id=\"user-avatar-uploads\" type=\"hidden\"><div class=\"controls\">{} {}</div></div>",
            escape(&t(cx, "user.avatar.title")),
            data("custom_avatar_upload_id"),
            data("system_avatar_upload_id"),
            crate::user_profile_view::avatar(cx, u["avatar_template"].as_str().unwrap_or_default(), 144),
            button(
                "btn-default pad-left",
                Some("pencil"),
                None,
                &format!(
                    " id=\"edit-avatar\" data-avatar-selector=\"{}\"",
                    escape(&avatar_selector(p, a)?.to_string())
                )
            ),
        ));
    }

    // CanCheckEmailsHelper
    let can_check_emails = p.viewing_self()
        || p.viewer_admin
        || (p.viewer_staff && p.setting("moderators_view_emails")?);
    if can_check_emails {
        out.push_str(&format!(
            "<div class=\"control-group pref-email\" data-setting-name=\"user-email\" data-labels=\"{}\"><label class=\"control-label\">{}</label>",
            escape(
                &json!({
                    "primary": t(cx, "user.email.primary_label"),
                    "unconfirmed": t(cx, "user.email.unconfirmed_label"),
                    "instructions": t(cx, "user.email.instructions"),
                    "resending": t(cx, "user.email.resending_label"),
                    "resent": t(cx, "user.email.resent_label"),
                })
                .to_string()
            ),
            escape(&t(cx, "user.email.title"))
        ));
        if let Some(email) = u["email"].as_str() {
            out.push_str(&emails(p, email)?);
        } else {
            out.push_str(&format!(
                "<div class=\"controls\">{}</div>",
                button(
                    "btn-default check-email",
                    Some("envelope"),
                    Some(&ta(cx, "admin.users.check_email.text")),
                    &format!(
                        " title=\"{}\"",
                        escape(&ta(cx, "admin.users.check_email.title"))
                    )
                )
            ));
        }
        out.push_str("</div>");
    }
    out.push_str("<span></span>");

    // canUpdateAssociatedAccounts: findAll() lists the enabled login
    // methods.
    if can_check_emails
        && u["second_factor_enabled"] != true
        && u["is_anonymous"] != true
        && crate::site::external_logins_enabled(p.s)?
    {
        out.push_str(&associated_accounts(p)?);
    }

    if p.setting("enable_names")? {
        out.push_str(&format!(
            "<div class=\"control-group pref-name\" data-setting-name=\"user-name\"><label class=\"control-label\">{}</label><div class=\"controls\">",
            escape(&t(cx, "user.name.title"))
        ));
        if u["can_edit_name"] == true {
            out.push_str(&format!(
                "<input placeholder=\"\" maxlength=\"255\" class=\"ember-text-field input-xxlarge\" type=\"text\" value=\"{}\" data-pref=\"name\">",
                escape(u["name"].as_str().unwrap_or_default())
            ));
        } else {
            out.push_str(&format!(
                "<span class=\"static\">{}</span>",
                escape(u["name"].as_str().unwrap_or_default())
            ));
        }
        out.push_str(&format!(
            "</div><div class=\"instructions\">{}</div></div>",
            escape(&t(
                cx,
                if p.s.get("full_name_requirement")?.to_s() == "required_at_signup" {
                    "user.name.instructions_required"
                } else {
                    "user.name.instructions"
                }
            ))
        ));
    }

    if !a.titles.is_empty() {
        out.push_str(&format!(
            "<div class=\"control-group pref-title\" data-setting-name=\"user-title\"><label class=\"control-label\">{}</label><div class=\"controls\">{}</div><div class=\"instructions\">{}</div></div>",
            escape(&t(cx, "user.title.title")),
            combo(
                cx,
                &Combo {
                    id: None,
                    component_class: "",
                    class: "",
                    rows: a.titles.iter().map(|t| (t.clone(), t.clone())).collect(),
                    value: u["title"].as_str().map(str::to_string),
                    none: Some(t(cx, "user.title.none")),
                    key: Some("title"),
                    filterable: false,
                }
            ),
            escape(&t(cx, "user.title.instructions"))
        ));
    }

    let groups: Vec<&Value> = u["groups"].as_array().into_iter().flatten().collect();
    // availableFlairs: the visible groups with a flair.
    let flairs: Vec<&&Value> = groups
        .iter()
        .filter(|g| g["flair_url"].as_str().is_some_and(|f| !f.is_empty()))
        .collect();
    if !flairs.is_empty() {
        out.push_str(&format!(
            "<div class=\"control-group pref-flair\" data-setting-name=\"user-flair\"><label class=\"control-label\">{}</label><div class=\"controls\">{}</div><div class=\"instructions\">{}</div></div>",
            escape(&t(cx, "user.flair.title")),
            combo(
                cx,
                &Combo {
                    id: None,
                    component_class: "flair-chooser",
                    class: "",
                    rows: flairs
                        .iter()
                        .map(|g| (g["id"].to_string(), g["name"].as_str().unwrap_or_default().to_string()))
                        .collect(),
                    value: value_string(&u["flair_group_id"]),
                    none: Some(t(cx, "user.flair.none")),
                    key: Some("flair_group_id"),
                    filterable: false,
                }
            ),
            escape(&t(cx, "user.flair.instructions"))
        ));
    }

    if p.setting("enable_user_status")? {
        return Err(Unsupported("the user status in preferences (130)").into());
    }

    // filteredGroups: not automatic, but moderators.
    let filtered: Vec<&&Value> = groups
        .iter()
        .filter(|g| g["automatic"] != true || g["id"] == 2)
        .collect();
    if !filtered.is_empty() && p.setting("user_selected_primary_groups")? {
        // The ComboBox takes no onChange: what it shows is not saved.
        out.push_str(&format!(
            "<div class=\"control-group pref-primary-group\" data-setting-name=\"user-primary-group\"><label class=\"control-label\">{}</label><div class=\"controls\">{}</div></div>",
            escape(&t(cx, "user.primary_group.title")),
            combo(
                cx,
                &Combo {
                    id: None,
                    component_class: "",
                    class: "",
                    rows: filtered
                        .iter()
                        .map(|g| (g["id"].to_string(), g["name"].as_str().unwrap_or_default().to_string()))
                        .collect(),
                    value: value_string(&u["primary_group_id"]),
                    none: Some(t(cx, "user.primary_group.none")),
                    key: None,
                    filterable: false,
                }
            ),
        ));
    }

    // canDownloadPosts: user.viewingSelf
    if p.viewing_self() {
        out.push_str(&format!(
            "<div class=\"control-group pref-data-export\" data-setting-name=\"user-data-export\"><label class=\"control-label\">{}</label><div class=\"controls\">{}</div><div class=\"instructions\">{}</div></div>",
            escape(&t(cx, "user.download_archive.title")),
            button(
                "btn-default btn-request-archive",
                Some("download"),
                Some(&t(cx, "user.download_archive.button_text")),
                " data-unported=\"exporting your data (200)\""
            ),
            escape(&t(cx, "user.download_archive.description"))
        ));
    }
    out.push_str("<span></span><br><span></span>");
    out.push_str(&save_controls(cx));
    // model.canDeleteAccount
    if !p.setting("enable_discourse_connect")? && u["can_delete_account"] == true {
        out.push_str(&format!(
            "<div class=\"control-group delete-account\"><br><div class=\"controls\">{}</div></div>",
            button(
                "btn-danger delete-account-button",
                Some("trash-can"),
                Some(&t(cx, "user.delete_account")),
                &format!(
                    "{} data-confirm=\"{}\" data-deleted=\"{}\" data-not-allowed=\"{}\" data-cancel=\"{}\"",
                    if a.viewer_can_delete_account {
                        ""
                    } else {
                        " disabled"
                    },
                    escape(&t(cx, "user.delete_account_confirm")),
                    escape(&t(cx, "user.deleted_yourself")),
                    escape(&t(cx, "user.delete_yourself_not_allowed")),
                    escape(&t(cx, "composer.cancel")),
                )
            )
        ));
    }

    let attrs = save_attrs(
        u,
        &[
            "name",
            "title",
            "primary_group_id",
            "flair_group_id",
            "status",
        ],
        &[],
        &[],
    );
    Ok((out, attrs))
}

/// The account tab's emails: the list with each one's EmailDropdown when
/// secondary emails are on, else the one address.
fn emails(p: &Prefs, email: &str) -> Result<String, AppError> {
    let cx = p.cx;
    let u = p.u;
    let mut out = String::new();
    let can_edit = u["can_edit_email"] == true;
    if p.setting("enable_secondary_emails")? {
        let mut list: Vec<(String, bool, bool)> = vec![(email.to_string(), true, true)];
        for e in u["secondary_emails"].as_array().into_iter().flatten() {
            list.push((e.as_str().unwrap_or_default().to_string(), false, true));
        }
        for e in u["unconfirmed_emails"].as_array().into_iter().flatten() {
            list.push((e.as_str().unwrap_or_default().to_string(), false, false));
        }
        list.sort_by(|a, b| locale_compare(&a.0, &b.0));
        out.push_str("<div class=\"emails\">");
        for (address, primary, confirmed) in &list {
            out.push_str(&format!(
                "<div class=\"row email\"><div class=\"email-first\">{}</div><div class=\"email-second\">",
                escape(address)
            ));
            if *primary {
                out.push_str(&format!(
                    "<span class=\"primary\">{}</span>",
                    escape(&t(cx, "user.email.primary_label"))
                ));
            }
            if !confirmed {
                out.push_str(&format!(
                    "<span class=\"unconfirmed\">{}</span> &bull; <button class=\"resend-email-confirmation\" type=\"button\">{}</button>",
                    escape(&t(cx, "user.email.unconfirmed_label")),
                    escape(&t(cx, "user.email.resend_label"))
                ));
            }
            out.push_str("</div>");
            if can_edit {
                out.push_str(&email_dropdown(p, *primary, *confirmed));
            }
            out.push_str("</div>");
        }
        out.push_str("</div>");
        if p.s.get("max_allowed_secondary_emails")?.to_i() > 0 && can_edit {
            out.push_str(&format!(
                "<div class=\"controls\"><a href=\"{}/preferences/email?new=1\">{} {}</a></div>",
                p.user_path,
                icon("plus", None),
                escape(&t(cx, "user.email.add_email"))
            ));
        }
    } else {
        out.push_str(&format!(
            "<div class=\"controls\"><span class=\"static\">{}</span>",
            escape(email)
        ));
        if can_edit {
            out.push_str(&format!(
                "<a class=\"btn btn-default btn-small btn-icon pad-left no-text\" href=\"{}/preferences/email\">{}</a>",
                p.user_path,
                icon("pencil", None)
            ));
        }
        out.push_str("</div>");
    }
    let mut instructions = String::new();
    if p.setting("auth_overrides_email")? {
        instructions.push_str(&t(cx, "user.email.auth_override_instructions"));
        instructions.push(' ');
    }
    instructions.push_str(&t(cx, "user.email.instructions"));
    out.push_str(&format!(
        "<div class=\"instructions\">{}</div>",
        escape(&instructions)
    ));
    Ok(out)
}

/// `String#localeCompare` in English as ICU's root collation orders
/// ASCII text: letters and digits case-insensitively, punctuation
/// ignored at the first level, then lowercase before uppercase.
pub(crate) fn locale_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let primary = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    primary(a)
        .cmp(&primary(b))
        .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
        .then_with(|| b.cmp(a))
}

/// EmailDropdown: change the primary address; make a confirmed one
/// primary; remove one that isn't.
fn email_dropdown(p: &Prefs, primary: bool, confirmed: bool) -> String {
    let cx = p.cx;
    let mut rows = Vec::new();
    if primary {
        rows.push(("updateEmail", "pencil", "user.email.update_email"));
    }
    if !primary && confirmed {
        rows.push(("setPrimaryEmail", "star", "user.email.set_primary"));
    }
    if !primary {
        rows.push(("destroyEmail", "xmark", "user.email.destroy"));
    }
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
        "<details class=\"select-kit single-select dropdown-select-box email-dropdown prefs-combo\" data-email-href=\"{}/preferences/email\"><summary aria-label=\"{filter_by}\" name=\"{filter_by}\" data-name=\"\" tabindex=\"0\" class=\"select-kit-header single-select-header dropdown-select-box-header btn no-text btn-icon btn-default\"><div class=\"select-kit-header-wrapper\">{} &#8203;</div></summary><div class=\"select-kit-body\"><ul aria-live=\"polite\" class=\"select-kit-collection\" role=\"menu\">{items}</ul></div></details>",
        p.user_path,
        icon("wrench", None),
    )
}
