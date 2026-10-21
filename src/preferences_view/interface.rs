//! preferences/interface.gjs and controllers/preferences/interface.js.

use serde_json::{Map, Value, json};

use super::{
    Combo, Prefs, button, checkbox, combo, combo_rows, save_attrs, save_controls, value_string,
};
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
    /// `site.default_light_color_scheme` and `default_dark_color_scheme`.
    pub default_light_color_scheme: &'a Value,
    pub default_dark_color_scheme: &'a Value,
    /// The session's userColorSchemeId and userDarkSchemeId, and the
    /// InterfaceColor service's colorMode (crate::color_mode::Head).
    pub session_scheme_id: i32,
    pub session_dark_scheme_id: i32,
    pub color_mode: Option<&'a str>,
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

    // showThemeSelector: another theme would draw its own stylesheets,
    // and the port draws the default theme's.
    let themes = list_themes(i.user_themes);
    if themes.len() > 1 {
        return Err(Unsupported("choosing a theme (258a)").into());
    }
    let palettes = color_schemes(p, i, &themes);
    out.push_str(&palettes.html);

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
    // save: the palettes go null without the selector, else the chosen
    // ones (makeColorSchemeDefault starts checked).
    let mut values: Vec<(&str, Value)> = vec![
        ("color_scheme_id", palettes.save_light),
        ("dark_scheme_id", palettes.save_dark),
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

/// A theme as `listThemes` lists it.
struct ListedTheme {
    id: Option<i64>,
    color_scheme_id: Option<i64>,
    dark_color_scheme_id: Option<i64>,
    only_theme_color_schemes: bool,
}

/// `listThemes(site)`: the user themes, after a nameless default when none
/// of them is the default.
fn list_themes(user_themes: &Value) -> Vec<ListedTheme> {
    let themes = user_themes.as_array().cloned().unwrap_or_default();
    let mut out = Vec::new();
    if !themes.iter().any(|t| t["default"] == true) {
        out.push(ListedTheme {
            id: None,
            color_scheme_id: None,
            dark_color_scheme_id: None,
            only_theme_color_schemes: false,
        });
    }
    out.extend(themes.iter().map(|t| ListedTheme {
        id: t["theme_id"].as_i64(),
        color_scheme_id: t["color_scheme_id"].as_i64(),
        dark_color_scheme_id: t["dark_color_scheme_id"].as_i64(),
        only_theme_color_schemes: t["only_theme_color_schemes"] == true,
    }));
    out
}

/// A palette as `listColorSchemes` lists it (`is_dark` None for the
/// theme default).
struct ListedScheme {
    id: i64,
    name: String,
    is_dark: Option<bool>,
    preview: Value,
}

/// `listColorSchemes(site, {darkOnly, currentTheme})`: the user selectable
/// palettes (light ones first), only the theme's own when it limits them,
/// after the theme default (-1) unless it does.
fn list_color_schemes(
    p: &Prefs,
    i: &Interface,
    theme: Option<&ListedTheme>,
    dark_only: bool,
) -> Vec<ListedScheme> {
    let mut schemes = i.user_color_schemes.as_array().cloned().unwrap_or_default();
    let limit = theme.is_some_and(|t| {
        t.only_theme_color_schemes && schemes.iter().any(|s| s["theme_id"].as_i64() == t.id)
    });
    if !dark_only {
        // A stable sort, as Array#sort is.
        schemes.sort_by_key(|s| s["is_dark"] == true);
    }
    let mut out: Vec<ListedScheme> = schemes
        .iter()
        .filter(|s| !limit || s["theme_id"].as_i64() == theme.and_then(|t| t.id))
        .filter(|s| !dark_only || s["is_dark"] == true)
        .map(|s| ListedScheme {
            id: s["id"].as_i64().unwrap_or_default(),
            name: s["name"].as_str().unwrap_or_default().to_string(),
            is_dark: Some(s["is_dark"] == true),
            preview: s.clone(),
        })
        .collect();
    if !limit {
        let default = if dark_only {
            i.default_dark_color_scheme
        } else {
            i.default_light_color_scheme
        };
        out.insert(
            0,
            ListedScheme {
                id: -1,
                name: t(p.cx, "user.color_schemes.default_description"),
                is_dark: None,
                preview: json!({"colors": default["colors"].as_array().cloned().unwrap_or_default()}),
            },
        );
    }
    out
}

/// What the color palette fieldset drew and what Save Changes sends.
struct Palettes {
    html: String,
    save_light: Value,
    save_dark: Value,
}

/// showColorSchemeSelector's fieldset: the light and dark
/// ColorPalettePickers, the interface color mode, and (while previewing,
/// static/js/preferences.js) Undo and the all devices checkbox.
fn color_schemes(p: &Prefs, i: &Interface, themes: &[ListedTheme]) -> Palettes {
    let cx = p.cx;
    let own = p.viewing_self();
    // currentThemeId: the theme the page draws, the default one.
    let current_id = default_theme_id(i.user_themes);
    let theme = themes.iter().find(|t| t.id.is_some() && t.id == current_id);
    let light = list_color_schemes(p, i, theme, false);
    let dark = list_color_schemes(p, i, theme, true);
    let only = theme.is_some_and(|t| t.only_theme_color_schemes);
    let show_light =
        light.len() > 1 && (!only || light.iter().filter(|s| s.is_dark != Some(true)).count() > 1);
    let show_dark =
        dark.len() > 1 && (!only || dark.iter().filter(|s| s.is_dark == Some(true)).count() > 1);
    let default_dark = theme
        .and_then(|t| t.dark_color_scheme_id)
        .filter(|id| *id != 0)
        .unwrap_or(-1);

    // The route's selectedColorSchemeId and selectedDarkColorSchemeId:
    // one's own from the session (getSelectedColorSchemeId leaves out the
    // theme's palette when it is not user selectable), another's from
    // their options.
    let (selected_light, selected_dark) = if own {
        let session = i64::from(i.session_scheme_id);
        let hidden = session != 0
            && theme.and_then(|t| t.color_scheme_id) == Some(session)
            && !light.iter().any(|s| s.id == session);
        (
            (session != 0 && !hidden).then_some(session),
            Some(i64::from(i.session_dark_scheme_id)),
        )
    } else {
        (
            p.option("color_scheme_id").as_i64(),
            p.option("dark_scheme_id").as_i64(),
        )
    };
    let show_mode = (default_dark > 0
        && theme
            .and_then(|t| t.color_scheme_id)
            .is_some_and(|l| l != 0 && Some(l) != theme.and_then(|t| t.dark_color_scheme_id)))
        || selected_dark.is_some_and(|d| d > 0);
    if !(show_light || show_dark || show_mode) {
        return Palettes {
            html: String::new(),
            save_light: Value::Null,
            save_dark: Value::Null,
        };
    }

    let picker = |schemes: &[ListedScheme], selected: Option<i64>, key: &str| {
        combo_rows(
            cx,
            &Combo {
                id: None,
                component_class: "color-palette-picker",
                class: "",
                rows: schemes
                    .iter()
                    .map(|s| (s.id.to_string(), s.name.clone()))
                    .collect(),
                value: selected.map(|s| s.to_string()),
                none: None,
                key: Some(key),
                filterable: false,
            },
            " color-palette-picker-row",
            &|value, name, active| {
                let preview = schemes
                    .iter()
                    .find(|s| s.id.to_string() == value)
                    .map(|s| crate::admin_colors_view::preview_style(&s.preview))
                    .unwrap_or_default();
                format!(
                    "{}<div style=\"{}\" class=\"color-palette-picker-row__preview\">{}</div><div class=\"color-palette-picker-row__name\">{}</div>",
                    if active {
                        active_badge(cx)
                    } else {
                        String::new()
                    },
                    escape(&preview),
                    crate::admin_colors_view::PLACEHOLDER,
                    escape(name)
                )
            },
        )
    };
    let subgroup = |class: &str, label: &str, control: String| {
        format!(
            "<div class=\"control-subgroup {class}\"><div class=\"instructions\">{}</div><div class=\"controls\">{control}</div></div>",
            escape(&t(cx, label))
        )
    };
    let mut html = String::new();
    if show_light {
        html.push_str(&subgroup(
            "light-color-scheme",
            "user.color_schemes.regular",
            picker(&light, selected_light, "color_scheme_id"),
        ));
    }
    if show_dark {
        html.push_str(&subgroup(
            "dark-color-scheme",
            "user.color_schemes.dark",
            picker(&dark, selected_dark, "dark_scheme_id"),
        ));
    }
    if show_mode {
        // selectedInterfaceColorMode: one's own the service's mode, else
        // the option.
        let mode = match i.color_mode.filter(|_| own) {
            Some("auto") => Some("1".to_string()),
            Some("light") => Some("2".to_string()),
            Some("dark") => Some("3".to_string()),
            _ => value_string(p.option("interface_color_mode")),
        };
        html.push_str(&subgroup(
            "interface-color-mode",
            "user.color_schemes.interface_mode",
            combo(
                cx,
                &Combo {
                    id: None,
                    component_class: "",
                    class: "",
                    rows: [(1, "auto"), (2, "light"), (3, "dark")]
                        .iter()
                        .map(|(id, k)| {
                            (
                                id.to_string(),
                                t(cx, &format!("user.color_schemes.interface_modes.{k}")),
                            )
                        })
                        .collect(),
                    value: mode,
                    none: None,
                    key: Some("interface_color_mode"),
                    filterable: false,
                },
            ),
        ));
    }
    // previewingColorScheme's Undo and checkbox, which the script puts
    // in after the controls while a palette is previewed.
    let preview = format!(
        "{}<div class=\"controls color-scheme-checkbox\">{}</div>",
        button(
            "btn-default btn-small undo-preview",
            Some("arrow-rotate-left"),
            Some(&t(cx, "user.color_schemes.undo")),
            "",
        ),
        checkbox(
            "",
            None,
            true,
            &t(cx, "user.color_scheme_default_on_all_devices"),
            None
        )
    );
    let data = json!({
        "own": own,
        "themeId": current_id,
        "themeLight": theme.and_then(|t| t.color_scheme_id),
        "themeDark": theme.and_then(|t| t.dark_color_scheme_id),
        "defaultDark": default_dark,
        "sessionLight": own.then_some(i.session_scheme_id),
        "sessionDark": own.then_some(i.session_dark_scheme_id),
        "colorMode": i.color_mode.filter(|_| own),
        "optionMode": p.option("interface_color_mode"),
        "modelLight": p.option("color_scheme_id"),
        "modelDark": p.option("dark_scheme_id"),
        "activeLabel": t(cx, "user.color_schemes.picker.active"),
        "previewHtml": preview,
    });
    let instructions = if show_dark {
        format!(
            "<div class=\"instructions\">{}</div>",
            escape(&t(cx, "user.color_schemes.interface_mode_instructions"))
        )
    } else {
        String::new()
    };
    Palettes {
        html: format!(
            "<fieldset class=\"control-group color-scheme\" data-setting-name=\"user-color-scheme\" data-color-schemes=\"{}\"><legend class=\"control-label\">{}</legend><div class=\"controls\">{html}</div>{instructions}</fieldset>",
            escape(&data.to_string()),
            escape(&t(cx, "user.color_scheme"))
        ),
        save_light: selected_light.map_or(Value::Null, Value::from),
        save_dark: selected_dark.map_or(Value::Null, Value::from),
    }
}

/// ColorPalettePickerRow's badge on the selected palette.
fn active_badge(cx: &crate::topic_list_view::ListContext) -> String {
    format!(
        "<span class=\"color-palette-picker-row__badge --active\">{}</span>",
        escape(&t(cx, "user.color_schemes.picker.active"))
    )
}
