//! preferences/interface.gjs and controllers/preferences/interface.js.

use serde_json::{Map, Value, json};

use super::{Combo, Prefs, button, checkbox, combo, save_attrs, save_controls, value_string};
use crate::topic_list_view::{escape, t, t_with};
use crate::{AppError, Unsupported};

/// UserOption::HOMEPAGES (the controller's USER_HOMES), by id.
const USER_HOMES: [(i64, &str); 8] = [
    (1, "latest"),
    (2, "categories"),
    (3, "unread"),
    (4, "new"),
    (5, "top"),
    (6, "bookmarks"),
    (7, "unseen"),
    (8, "hot"),
];

const TEXT_SIZES: [&str; 5] = ["smallest", "smaller", "normal", "larger", "largest"];

/// What the interface tab reads beside the user.
pub struct Interface<'a> {
    /// `site.user_themes` and `site.user_color_schemes`.
    pub user_themes: &'a Value,
    pub user_color_schemes: &'a Value,
    /// The `text_size` cookie (`<size>|<seq>`).
    pub text_size_cookie: Option<&'a str>,
    /// `site.user_tips`: the user tips are on.
    pub user_tips: bool,
}

/// `user.currentTextSize`: the cookie's size when its sequence is at
/// least the option's, else the option's.
pub fn current_text_size(user_option: &Value, cookie: Option<&str>) -> Option<String> {
    if let Some((size, seq)) = cookie.and_then(|c| c.split_once('|')) {
        let option_seq = user_option["text_size_seq"].as_i64().unwrap_or(0);
        if seq.trim().parse::<i64>().is_ok_and(|s| s >= option_seq) {
            return Some(size.to_string());
        }
    }
    user_option["text_size"].as_str().map(str::to_string)
}

/// `siteDefaultHomepage`: default_homepage, else the top menu's first
/// item.
fn site_default_homepage(p: &Prefs) -> Result<String, AppError> {
    let configured = p.s.get("default_homepage")?.to_s();
    if !configured.is_empty() {
        return Ok(configured);
    }
    let top_menu = p.s.get("top_menu")?.to_s();
    Ok(top_menu
        .split('|')
        .next()
        .unwrap_or_default()
        .split(',')
        .next()
        .unwrap_or_default()
        .to_string())
}

/// preferences/interface.gjs
pub fn render(p: &Prefs, i: &Interface) -> Result<(String, Map<String, Value>), AppError> {
    let cx = p.cx;
    let u = p.u;
    let viewing_self = p.viewing_self();
    let mut out = String::from("<span></span>");

    // showThemeSelector, showColorSchemeSelector: the port draws the
    // default theme's stylesheets only.
    let themes = i.user_themes.as_array().map(Vec::len).unwrap_or(0)
        + usize::from(
            !i.user_themes
                .as_array()
                .into_iter()
                .flatten()
                .any(|t| t["default"] == true),
        );
    let schemes = i.user_color_schemes.as_array().map(Vec::len).unwrap_or(0);
    // showInterfaceColorModeSelector
    let theme = i
        .user_themes
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["default"] == true);
    let dark = theme
        .and_then(|t| t["dark_color_scheme_id"].as_i64())
        .unwrap_or(-1);
    let light = theme.and_then(|t| t["color_scheme_id"].as_i64());
    let mode_selector = (dark > 0 && light.is_some_and(|l| l != dark))
        || p.option("dark_scheme_id").as_i64().unwrap_or(-1) > 0;
    if themes > 1 || schemes > 0 || mode_selector {
        return Err(Unsupported("choosing a theme or color palette (169)").into());
    }

    // textSize: one's own is the cookie's when it is newer.
    let option_size = p.option("text_size").as_str().map(str::to_string);
    let text_size = if viewing_self {
        current_text_size(&u["user_option"], i.text_size_cookie)
    } else {
        option_size.clone()
    };
    let make_default = !viewing_self || text_size == option_size;
    out.push_str(&format!(
        "<div class=\"control-group text-size\" data-setting-name=\"user-text-size\"><label class=\"control-label\" for=\"text-size-selector\">{}</label><div class=\"controls\">{}</div>",
        escape(&t(cx, "user.text_size.title")),
        combo(
            cx,
            &Combo {
                id: Some("text-size-selector"),
                component_class: "",
                class: "",
                rows: TEXT_SIZES
                    .iter()
                    .map(|s| (s.to_string(), t(cx, &format!("user.text_size.{s}"))))
                    .collect(),
                value: text_size.clone(),
                none: None,
                key: None,
                filterable: false,
            }
        )
    ));
    // showTextSetDefault, and the checkbox for when it shows after a pick.
    out.push_str(&format!(
        "<div class=\"controls text-size-default\"{}>{}</div></div>",
        if viewing_self && option_size != text_size {
            ""
        } else {
            " hidden"
        },
        checkbox(
            "",
            None,
            make_default,
            &t(cx, "user.text_size_default_on_all_devices"),
            None
        )
    ));

    let mut fields_locale = false;
    if p.setting("allow_user_locale")? {
        // siteSettings.available_locales: LocaleSiteSetting.values.
        let rows: Vec<(String, String)> = crate::setting_enums::recorded("default_locale")
            .and_then(|r| r.get("valid_values"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| {
                Some((
                    v.get("value")?.as_str()?.to_string(),
                    v.get("name")?.as_str()?.to_string(),
                ))
            })
            .collect();
        out.push_str(&format!(
            "<div class=\"control-group pref-locale\" data-setting-name=\"user-locale\"><label class=\"control-label\" for=\"locale-selector\">{}</label><div class=\"controls\">{}</div><div class=\"instructions\">{}</div></div>",
            escape(&t(cx, "user.locale.title")),
            combo(
                cx,
                &Combo {
                    id: Some("locale-selector"),
                    component_class: "",
                    class: "",
                    rows,
                    value: u["locale"].as_str().filter(|l| !l.is_empty()).map(str::to_string),
                    none: Some(t(cx, "user.locale.default")),
                    key: Some("locale"),
                    filterable: true,
                }
            ),
            escape(&t(cx, "user.locale.instructions"))
        ));
        fields_locale = true;
    }
    if p.setting("content_localization_enabled")? {
        return Err(Unsupported("content languages in preferences (168)").into());
    }

    // userSelectableHome: Default, the site's home, the top menu's others,
    // and the user's own when the menu lacks it.
    let site_home = site_default_homepage(p)?;
    let top_menu = p.s.get("top_menu")?.to_s();
    let mut available: Vec<String> = vec![site_home.clone()];
    available.extend(
        top_menu
            .split('|')
            .filter(|m| *m != site_home)
            .map(str::to_string),
    );
    let user_home_id = p.option("homepage_id").as_i64();
    let user_home = user_home_id.and_then(|id| USER_HOMES.iter().find(|(i, _)| *i == id));
    if let Some((_, name)) = user_home
        && !available.iter().any(|a| a == name)
    {
        available.push(name.to_string());
    }
    let mut homes = vec![("-1".to_string(), t(cx, "user.homepage.default"))];
    for m in &available {
        if let Some((id, _)) = USER_HOMES.iter().find(|(_, n)| n == m) {
            homes.push((id.to_string(), t(cx, &format!("filters.{m}.title"))));
        }
    }
    let homepage_id = user_home_id
        .filter(|id| *id != 0)
        .map(|id| id.to_string())
        .unwrap_or_else(|| "-1".to_string());
    out.push_str(&format!(
        "<div class=\"control-group home\" data-setting-name=\"user-home\"><label class=\"control-label\" for=\"home-selector\">{}</label><div class=\"controls\">{}</div></div>",
        escape(&t(cx, "user.home")),
        combo(
            cx,
            &Combo {
                id: Some("home-selector"),
                component_class: "",
                class: "",
                rows: homes,
                value: Some(homepage_id),
                none: None,
                key: Some("homepage_id"),
                filterable: false,
            }
        )
    ));

    if p.setting("enable_composer_toolbar_customization")? {
        return Err(Unsupported("hiding composer toolbar buttons (196)").into());
    }

    let pref = |class: &str, key: &str, label: &str| {
        checkbox(
            class,
            Some(&format!("user-{}", class.trim_start_matches("pref-"))),
            p.option(key) == true,
            &t(cx, label),
            Some(key),
        )
    };
    out.push_str(&format!(
        "<fieldset class=\"control-group other\" data-setting-name=\"user-other-settings\"><legend class=\"control-label\">{}</legend>",
        escape(&t(cx, "user.other_settings"))
    ));
    out.push_str(&pref(
        "pref-external-links",
        "external_links_in_new_tab",
        "user.external_links_in_new_tab",
    ));
    out.push_str(&pref(
        "pref-enable-quoting",
        "enable_quoting",
        "user.enable_quoting",
    ));
    out.push_str(&pref(
        "pref-enable-smart-lists",
        "enable_smart_lists",
        "user.enable_smart_lists",
    ));
    if p.setting("automatically_unpin_topics")? {
        out.push_str(&pref(
            "pref-auto-unpin",
            "automatically_unpin_topics",
            "user.automatically_unpin_topics",
        ));
    }
    out.push_str(&pref(
        "pref-dynamic-favicon",
        "dynamic_favicon",
        "user.dynamic_favicon",
    ));
    out.push_str(&pref(
        "pref-enable-markdown-monospace-font",
        "enable_markdown_monospace_font",
        "user.enable_markdown_monospace_font",
    ));
    let dropdown = |class: &str, id: &str, label: &str, combo_html: String| {
        format!(
            "<div class=\"controls controls-dropdown {class}\" data-setting-name=\"user-{}\"><label for=\"{id}\">{}</label>{combo_html}</div>",
            class.trim_start_matches("pref-"),
            escape(&t(cx, label))
        )
    };
    out.push_str(&dropdown(
        "pref-page-title",
        "user-title-count-mode",
        "user.title_count_mode.title",
        combo(
            cx,
            &Combo {
                id: Some("user-title-count-mode"),
                component_class: "",
                class: "",
                rows: ["notifications", "contextual"]
                    .iter()
                    .map(|v| (v.to_string(), t(cx, &format!("user.title_count_mode.{v}"))))
                    .collect(),
                value: value_string(p.option("title_count_mode")),
                none: None,
                key: Some("title_count_mode"),
                filterable: false,
            },
        ),
    ));
    out.push_str(&dropdown(
        "pref-send-shortcut",
        "user-send-shortcut",
        "user.send_shortcut.title",
        combo(
            cx,
            &Combo {
                id: Some("user-send-shortcut"),
                component_class: "",
                class: "",
                rows: vec![
                    ("enter".to_string(), t(cx, "user.send_shortcut.enter")),
                    (
                        "meta_enter".to_string(),
                        t_with(cx, "user.send_shortcut.meta_enter", &[("meta_key", "Ctrl")]),
                    ),
                ],
                value: value_string(p.option("send_shortcut")),
                none: None,
                key: Some("send_shortcut"),
                filterable: false,
            },
        ),
    ));
    out.push_str(&dropdown(
        "pref-bookmark-after-notification",
        "bookmark-after-notification-mode",
        "user.bookmark_after_notification.title",
        combo(
            cx,
            &Combo {
                id: Some("bookmark-after-notification-mode"),
                component_class: "",
                class: "",
                rows: [
                    (0, "never"),
                    (3, "clear_reminder"),
                    (1, "when_reminder_sent"),
                    (2, "on_owner_reply"),
                ]
                .iter()
                .map(|(v, k)| {
                    (
                        v.to_string(),
                        t(cx, &format!("bookmarks.auto_delete_preference.{k}")),
                    )
                })
                .collect(),
                value: value_string(p.option("bookmark_auto_delete_preference")),
                none: None,
                key: Some("bookmark_auto_delete_preference"),
                filterable: false,
            },
        ),
    ));
    out.push_str(&pref(
        "pref-new-user-tips",
        "skip_new_user_tips",
        "user.skip_new_user_tips.description",
    ));
    if i.user_tips {
        out.push_str(&button(
            "btn-default pref-reset-seen-user-tips",
            None,
            Some(&t(cx, "user.reset_seen_user_tips")),
            " data-setting-name=\"user-reset-seen-user-tips\"",
        ));
    }
    out.push_str("</fieldset><span></span><br><span></span>");
    out.push_str(&save_controls(cx));

    // saveAttrNames: theme_ids while makeThemeDefault (one's own theme is
    // the default one: no theme selector); color schemes go null without
    // the selector; text_size is the chosen one when made the default.
    let mut fields = vec![
        "external_links_in_new_tab",
        "dynamic_favicon",
        "enable_quoting",
        "enable_smart_lists",
        "automatically_unpin_topics",
        "allow_private_messages",
        "enable_allowed_pm_users",
        "homepage_id",
        "hide_presence",
        "text_size",
        "title_count_mode",
        "skip_new_user_tips",
        "seen_popups",
        "color_scheme_id",
        "dark_scheme_id",
        "bookmark_auto_delete_preference",
        "interface_color_mode",
        "enable_markdown_monospace_font",
        "send_shortcut",
        "automatically_translate",
        "understood_languages",
        "hidden_composer_toolbar_buttons",
    ];
    if fields_locale {
        fields.push("locale");
    }
    // makeThemeDefault: always for another's profile; for one's own when no
    // theme is set or the set one is the current (the default here).
    let theme_ids = &u["user_option"]["theme_ids"];
    let first_theme = theme_ids.as_array().and_then(|a| a.first()).cloned();
    let current_theme = default_theme_id(i.user_themes);
    let make_theme_default = !viewing_self
        || theme_ids.is_null()
        || first_theme.as_ref().and_then(Value::as_i64) == current_theme;
    if make_theme_default {
        fields.push("theme_ids");
    }
    let mut values: Vec<(&str, Value)> = vec![
        ("color_scheme_id", Value::Null),
        ("dark_scheme_id", Value::Null),
    ];
    if make_theme_default {
        values.push((
            "theme_ids",
            json!([current_theme.map_or(Value::Null, Value::from)]),
        ));
    }
    let attrs = save_attrs(u, &fields, &[], &values);
    Ok((out, attrs))
}

/// `currentThemeId()`: the default theme's id (the one the port draws).
fn default_theme_id(user_themes: &Value) -> Option<i64> {
    user_themes
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["default"] == true)
        .and_then(|t| t["theme_id"].as_i64())
}
