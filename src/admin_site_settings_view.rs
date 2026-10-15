//! The admin's site settings pages as the Ember client renders them
//! (templates/admin-site-settings.gjs around
//! admin-site-settings-category.gjs, components/site-setting.gjs with the
//! setting field registry) from the document GET /admin/site_settings
//! serves: the page header, AdminSiteSettingsFilterControls, the category
//! nav and the category's settings as SiteSettingFilter picks them for the
//! route's filter, onlyOverridden and dependsOn query params.
//! static/js/site-settings.js edits and saves them.
//!
//! The settings whose editor is not ported (uploads, the select-kit
//! pickers of categories, tags, emoji, colors and the like) say so in
//! place of their editor.

use serde_json::Value;

use crate::admin_view::{AdminCx, Crumb, PageHeader, t, t_count, t_opt, t_with};
use crate::topic_list_view::{escape, icon};

/// The route's filters.
#[derive(Default)]
pub struct Filters {
    pub filter: String,
    pub only_overridden: bool,
    pub depends_on: Option<String>,
}

impl Filters {
    pub fn applied(&self) -> bool {
        !self.filter.is_empty() || self.only_overridden || self.depends_on.is_some()
    }
}

/// A category of the nav: its key, its settings (indexes into the
/// document), its count's text, whether more matched than shown.
pub struct Category {
    pub key: String,
    pub settings: Vec<usize>,
    pub count: String,
    pub has_more: bool,
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// A setting's value as the client compares it (`toString`).
fn value_string(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

/// SiteSettingObjectHelper#overridden: the value differs from the default.
fn overridden(setting: &Value) -> bool {
    value_string(&setting["default"]) != value_string(&setting["value"])
}

/// SiteSettingMatcher with its filter split into `terms`.
struct Matcher<'a> {
    terms: Vec<String>,
    spaced_terms: Vec<String>,
    or_mode: bool,
    stripped: String,
    setting: &'a Value,
}

impl<'a> Matcher<'a> {
    fn new(filter: &str, setting: &'a Value) -> Matcher<'a> {
        let or_terms: Vec<String> = filter
            .strip_prefix("any:")
            .map(|rest| {
                rest.split('|')
                    .map(|f| f.trim().to_string())
                    .filter(|f| !f.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let or_mode = !or_terms.is_empty();
        let terms = if or_mode {
            or_terms
        } else {
            vec![filter.to_string()]
        };
        Matcher {
            spaced_terms: terms.iter().map(|t| t.replace('_', " ")).collect(),
            terms,
            or_mode,
            stripped: filter
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect(),
            setting,
        }
    }

    fn name_match(&self) -> bool {
        let name = s(&self.setting["setting"]).to_lowercase();
        let spaced = name.replace('_', " ");
        self.terms
            .iter()
            .any(|t| name.contains(t.as_str()) || spaced.contains(t.as_str()))
    }

    fn keyword_match(&self) -> bool {
        self.setting["keywords"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .any(|k| {
                let k = k.replace('_', " ").to_lowercase();
                self.spaced_terms.iter().any(|t| k.contains(t.as_str()))
            })
    }

    fn description_match(&self) -> bool {
        let desc = s(&self.setting["description"]).to_lowercase();
        self.terms.iter().any(|t| desc.contains(t.as_str()))
    }

    fn value_match(&self) -> bool {
        let value = value_string(&self.setting["value"]).to_lowercase();
        self.terms.iter().any(|t| value.contains(t.as_str()))
    }

    /// isFuzzyNameMatch, with the gaps' penalty as its strength.
    fn fuzzy_name_match(&self) -> Option<i64> {
        if self.or_mode || self.stripped.chars().count() < 3 {
            return None;
        }
        let name = s(&self.setting["setting"]).to_lowercase();
        let query: Vec<char> = self.stripped.to_lowercase().chars().collect();
        // fuzzyRegex: the query's characters in order, anything between.
        if !subsequence(&name.chars().collect::<Vec<_>>(), &query) {
            return None;
        }
        let stripped_name: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        if stripped_name.chars().count() > query.len() + 25 {
            return None;
        }
        if stripped_name.contains(&self.stripped.to_lowercase()) {
            return Some(0);
        }
        // fuzzyRegexGaps: greedy `.*a(.*)b(.*)c`, its non-empty groups.
        let gaps = greedy_gaps(&stripped_name.chars().collect::<Vec<_>>(), &query)?;
        let strength = -(gaps as i64);
        (strength >= -2).then_some(strength)
    }
}

fn subsequence(haystack: &[char], needle: &[char]) -> bool {
    let mut i = 0;
    for c in haystack {
        if i < needle.len() && *c == needle[i] {
            i += 1;
        }
    }
    i == needle.len()
}

/// `.*q0(.*)q1(.*)...q(n-1)` matched the way a backtracking engine
/// matches it: each `.*` takes as much as it can, so the query's
/// characters sit at their last possible positions. The non-empty groups
/// (the gaps between consecutive characters) are what
/// `gapResult.filter(gap => gap !== "").length - 1` counts.
fn greedy_gaps(name: &[char], query: &[char]) -> Option<usize> {
    let mut positions = vec![0usize; query.len()];
    let mut end = name.len();
    for (qi, qc) in query.iter().enumerate().rev() {
        let pos = name[..end].iter().rposition(|c| c == qc)?;
        positions[qi] = pos;
        end = pos;
    }
    Some(positions.windows(2).filter(|w| w[1] > w[0] + 1).count())
}

/// SiteSettingFilter#filterSettings: the categories in the document's
/// order with the matching settings, "All" first with at most 100 of them.
pub fn filter(settings: &[Value], f: &Filters) -> Vec<Category> {
    let mut plugin_filter = None;
    let mut change_filter = None;
    let words: Vec<String> = f
        .filter
        .to_lowercase()
        .split(' ')
        .filter(|w| !w.is_empty())
        .filter_map(|w| {
            if let Some(p) = w.strip_prefix("plugin:") {
                plugin_filter = Some(p.trim().to_string());
                None
            } else if let Some(c) = w.strip_prefix("upcoming_change_default_override:") {
                change_filter = Some(c.trim().to_string());
                None
            } else {
                Some(w.to_string())
            }
        })
        .collect();
    let filter = words.join(" ").trim().to_string();
    let mut keys: Vec<String> = Vec::new();
    for setting in settings {
        let c = s(&setting["category"]).to_string();
        if !keys.contains(&c) {
            keys.push(c);
        }
    }
    let mut out = vec![Category {
        key: "all_results".into(),
        settings: Vec::new(),
        count: String::new(),
        has_more: false,
    }];
    let mut matches: Vec<(usize, i64)> = Vec::new();
    for key in keys {
        let mut matched: Vec<(usize, i64)> = Vec::new();
        for (i, setting) in settings.iter().enumerate() {
            if s(&setting["category"]) != key {
                continue;
            }
            if f.only_overridden && !overridden(setting) {
                continue;
            }
            if let Some(dep) = &f.depends_on {
                let depends = setting["depends_on"]
                    .as_array()
                    .is_some_and(|d| d.iter().any(|n| n == dep.as_str()));
                if depends {
                    matched.push((i, 0));
                }
                continue;
            }
            if plugin_filter
                .as_deref()
                .is_some_and(|p| s(&setting["plugin"]) != p)
            {
                continue;
            }
            if let Some(c) = &change_filter
                && s(&setting["upcoming_change_default_override_metadata"]["change_setting_name"])
                    != c
            {
                continue;
            }
            if filter.is_empty() {
                matched.push((i, 0));
                continue;
            }
            let m = Matcher::new(&filter, setting);
            if m.name_match() {
                matched.push((i, 10));
            } else if m.keyword_match() {
                matched.push((i, 5));
            } else if m.description_match() || m.value_match() {
                matched.push((i, 0));
            } else if let Some(strength) = m.fuzzy_name_match() {
                matched.push((i, strength));
            }
        }
        if !matched.is_empty() {
            matches.extend(matched.iter().copied());
            let count = matched.len();
            // sortSettings: stable, by weight.
            matched.sort_by_key(|a| std::cmp::Reverse(a.1));
            out.push(Category {
                key,
                settings: matched.into_iter().map(|(i, _)| i).collect(),
                count: count.to_string(),
                has_more: false,
            });
        }
    }
    let has_more = matches.len() > 100;
    let mut all: Vec<(usize, i64)> = matches.iter().take(100).copied().collect();
    all.sort_by_key(|a| std::cmp::Reverse(a.1));
    out[0].settings = all.into_iter().map(|(i, _)| i).collect();
    out[0].count = if has_more {
        "100+".into()
    } else {
        matches.len().to_string()
    };
    out[0].has_more = has_more;
    out
}

/// What the page needs besides the settings.
pub struct Page<'a> {
    pub settings: &'a [Value],
    /// The document's `default_theme` (for the themeable settings' notice).
    pub default_theme: &'a Value,
    /// The site's groups (id, name) for the group setting's ComboBox.
    pub groups: &'a [(i64, String)],
    /// The site's categories (id, name) for the category setting's chooser.
    pub categories: &'a [(i64, String)],
    pub filters: &'a Filters,
    pub category: &'a str,
}

/// The page: the header, the filter controls, the nav, the category's
/// settings.
pub fn render(cx: &AdminCx, page: &Page, categories: &[Category]) -> String {
    let i18n = cx.i18n;
    let title = t(i18n, "admin.config.site_settings.title");
    let mut out = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path: "/admin/site_settings",
            },
        ],
        title,
        actions: String::new(),
        description: Some(escape(&t(
            i18n,
            "admin.config.site_settings.header_description",
        ))),
        learn_more_url: None,
        tabs: None,
    }
    .render(cx);
    // AdminSiteSettingsFilterControls
    out.push_str("<div class=\"admin-controls admin-site-settings-filter-controls\"><div class=\"controls\"><div class=\"inline-form\">");
    if crate::mobile::mobile_view() {
        out.push_str(&format!(
            "<button class=\"btn no-text btn-icon btn-default menu-toggle\" type=\"button\">{}<span aria-hidden=\"true\"> &#8203; </span></button>",
            icon("bars", None)
        ));
    }
    out.push_str(&format!(
        "<input autocomplete=\"off\" class=\"no-blur admin-site-settings-filter-controls__input\" id=\"setting-filter\" placeholder=\"{}\" value=\"{}\" type=\"text\"><button class=\"btn btn-text btn-default\" id=\"clear-filter\" type=\"button\"><span class=\"d-button-label\">{}</span></button></div></div><div class=\"search controls\"><label><input id=\"setting-filter-toggle-overridden\" class=\"toggle-overridden\" type=\"checkbox\"{}> {}</label></div></div>",
        escape(&t(i18n, "type_to_filter")),
        escape(&page.filters.filter),
        escape(&t(i18n, "admin.site_settings.clear_filter")),
        if page.filters.only_overridden { " checked" } else { "" },
        escape(&t(i18n, "admin.settings.show_overriden"))
    ));
    // AdminSiteSettingsCategoryNav
    out.push_str("<div class=\"admin-nav admin-site-settings-category-nav pull-left\">");
    out.push_str(&nav(cx, page, categories));
    out.push_str("</div><div class=\"admin-detail pull-left\">");
    let current = categories.iter().find(|c| c.key == page.category);
    let visible: Vec<&Value> = current
        .map(|c| {
            c.settings
                .iter()
                .map(|i| &page.settings[*i])
                .filter(|setting| visible_setting(page.settings, setting, &page.filters.filter))
                .collect()
        })
        .unwrap_or_default();
    if visible.is_empty() {
        out.push_str(&format!(
            "<br>{}",
            escape(&t(i18n, "admin.site_settings.no_results"))
        ));
    } else {
        out.push_str("<section class=\"form-horizontal settings\">");
        for setting in &visible {
            out.push_str(&setting_row(cx, page, setting));
        }
        if current.is_some_and(|c| c.has_more) {
            out.push_str(&format!(
                "<p class=\"warning\">{}</p>",
                escape(&t_count(
                    i18n,
                    "admin.site_settings.more_site_setting_results",
                    100,
                    &[]
                ))
            ));
        }
        out.push_str("</section>");
    }
    out.push_str("</div><div class=\"clearfix\"></div>");
    out.push_str(&client_strings(cx, page.settings));
    out
}

/// The nav's list, the current category active.
pub fn nav(cx: &AdminCx, page: &Page, categories: &[Category]) -> String {
    let mut out =
        String::from("<ul class=\"nav nav-stacked admin-site-settings-category-nav__list\">");
    let query = query_string(page.filters);
    for c in categories {
        let name = t(
            cx.i18n,
            &format!("admin.site_settings.categories.{}", c.key),
        );
        let key = escape(&c.key);
        out.push_str(&format!(
            "<li class=\"admin-site-settings-category-nav__item {key}\"><a class=\"{}{key}\" title=\"{}\" href=\"{}/admin/site_settings/category/{key}{}\">{}{}</a></li>",
            if c.key == page.category { "active " } else { "" },
            escape(&name),
            cx.base_path,
            escape(&query),
            escape(&name),
            if page.filters.applied() {
                format!(" <span class=\"count\">({})</span>", escape(&c.count))
            } else {
                String::new()
            }
        ));
    }
    out.push_str("</ul>");
    out
}

/// The route's query params as Ember keeps them on the links.
fn query_string(f: &Filters) -> String {
    let mut parts = Vec::new();
    if let Some(d) = &f.depends_on {
        parts.push(format!("dependsOn={}", encode(d)));
    }
    if !f.filter.is_empty() {
        parts.push(format!("filter={}", encode(&f.filter)));
    }
    if f.only_overridden {
        parts.push("onlyOverridden=true".into());
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

fn encode(s: &str) -> String {
    form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// AdminSiteSettingStore#isVisible: a setting hidden by a dependency shows
/// once the dependency is met, or when the filter names it.
fn visible_setting(settings: &[Value], setting: &Value, filter: &str) -> bool {
    if s(&setting["depends_behavior"]) != "hidden" {
        return true;
    }
    if dependencies_satisfied(settings, setting) {
        return true;
    }
    let normalize = |v: &str| v.to_lowercase().replace('_', " ").trim().to_string();
    !filter.trim().is_empty() && normalize(filter) == normalize(s(&setting["setting"]))
}

/// dependenciesSatisfied: each parent true, or one of its allowed values.
fn dependencies_satisfied(settings: &[Value], setting: &Value) -> bool {
    setting["depends_on"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .all(|name| {
            let Some(parent) = settings.iter().find(|p| p["setting"] == name) else {
                return true;
            };
            let value = value_string(&parent["value"]);
            match setting["depends_on_values"][name].as_array() {
                Some(allowed) => allowed.iter().any(|a| value_string(a) == value),
                None => value == "true",
            }
        })
}

/// SiteSetting#componentType, as the row's class names it.
fn component_type(setting: &Value) -> String {
    const TYPES: &[&str] = &[
        "bool",
        "date",
        "datetime",
        "integer",
        "enum",
        "list",
        "url_list",
        "host_list",
        "category_list",
        "value_list",
        "category",
        "uploaded_image_list",
        "compact_list",
        "secret_list",
        "upload",
        "group_list",
        "group",
        "tag_list",
        "tag_group_list",
        "color",
        "simple_list",
        "emoji_list",
        "named_list",
        "file_size_restriction",
        "file_types_list",
        "font_list",
        "locale_list",
        "locale_enum",
        "topic",
        "icon",
    ];
    let kind = s(&setting["type"]);
    let kind = match (kind, setting["list_type"].as_str()) {
        ("list", Some(list_type)) => format!("{list_type}_list"),
        _ => kind.to_string(),
    };
    if TYPES.contains(&kind.as_str()) {
        kind
    } else {
        "string".into()
    }
}

/// The FormKit field a setting's definition resolves to (the setting
/// field registry), None for a legacy editor.
fn form_kit_type(setting: &Value) -> Option<&'static str> {
    let kind = s(&setting["type"]);
    if setting["list_type"].is_null() {
        if setting["textarea"] == Value::Bool(true) {
            return Some("textarea");
        }
        if setting["secret"] == Value::Bool(true) {
            return Some("password");
        }
    }
    match kind {
        "string" | "float" | "username" => Some("input"),
        "email" => Some("input-email"),
        "enum" | "locale_enum" => Some("select"),
        "bool" => Some("checkbox"),
        "integer" => Some("input-number"),
        "group" | "category" | "tag_list" | "tag_group_list" => Some("custom"),
        "icon" => Some("icon"),
        _ => None,
    }
}

fn setting_row(cx: &AdminCx, page: &Page, setting: &Value) -> String {
    let i18n = cx.i18n;
    let name = s(&setting["setting"]);
    let kind = component_type(setting);
    let over = overridden(setting);
    let themeable = setting["themeable"] == Value::Bool(true);
    let disabled = themeable || setting["disabled"] == Value::Bool(true);
    let mut classes = format!("row setting {}", kind.replace('_', "-"));
    if over {
        classes.push_str(" overridden");
    }
    if disabled {
        classes.push_str(" disabled");
    }
    let label = setting["label"]
        .as_str()
        .filter(|l| !l.is_empty())
        .unwrap_or(s(&setting["humanized_name"]));
    let filters = format!("{{\"subject\":\"{name}\",\"action_name\":\"change_site_setting\"}}");
    let depends_on: Vec<&str> = setting["depends_on"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let hidden_dependent = s(&setting["depends_behavior"]) == "hidden" && !depends_on.is_empty();
    let mut out = format!(
        "<div class=\"{classes}\" data-setting=\"{}\" data-type=\"{}\" data-default=\"{}\" data-value=\"{}\"{}{}><div class=\"setting-label\"><h3>{} <a class=\"staff-action-log-link\" title=\"{}\" href=\"{}/admin/logs/staff_action_logs?filters={}&amp;force_refresh=true\"><span class=\"history-icon\">{}</span></a></h3></div><div class=\"setting-value\">",
        escape(name),
        escape(s(&setting["type"])),
        escape(&value_string(&setting["default"])),
        escape(&value_string(&setting["value"])),
        setting["requires_confirmation"]
            .as_str()
            .map(|r| format!(" data-requires-confirmation=\"{}\"", escape(r)))
            .unwrap_or_default(),
        if hidden_dependent {
            format!(" data-depends-on=\"{}\"", escape(&depends_on.join(",")))
        } else {
            String::new()
        },
        escape(label),
        escape(&t(i18n, "admin.settings.history")),
        cx.base_path,
        escape(&encode_component(&filters)),
        icon("clock-rotate-left", None)
    );
    let edit_button = if !setting["json_schema"].is_null() || !setting["schema"].is_null() {
        Some("admin.site_settings.json_schema.edit")
    } else if !setting["objects_schema"].is_null() {
        Some("admin.customize.theme.edit_objects_theme_setting")
    } else {
        None
    };
    let description = format!("<div class=\"desc\">{}</div>", s(&setting["description"]));
    let validation = format!(
        "<div><div class=\"validation-error hidden\">{} </div></div>",
        icon("xmark", None)
    );
    if let Some(key) = edit_button {
        out.push_str(&format!(
            "<button class=\"btn btn-icon-text btn-default setting-value-edit-button\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>{description}",
            icon("pencil", None),
            escape(&t(i18n, key))
        ));
    } else {
        let display_description = kind != "bool";
        match form_kit_type(setting) {
            Some(control) => out.push_str(&form_kit(cx, page, setting, control, disabled)),
            None => out.push_str(&legacy(cx, page, setting, &kind, disabled)),
        }
        out.push_str(&validation);
        if display_description {
            out.push_str(&description);
        }
        if themeable {
            out.push_str(&format!(
                "<div class=\"setting-override-warning setting-theme-warning\"><p class=\"setting-theme-warning__text\">{} {}</p></div>",
                icon("paintbrush", None),
                t_with(
                    i18n,
                    "admin.theme_site_settings.site_setting_warning",
                    &[
                        ("basePath", cx.base_path),
                        ("defaultThemeName", &escape(s(&page.default_theme["name"]))),
                        ("defaultThemeId", &page.default_theme["id"].to_string()),
                    ]
                )
            ));
        }
        let change = &setting["upcoming_change_default_override_metadata"];
        if change.is_object() {
            let short = ![
                "icon", "enum", "email", "username", "bool", "integer", "float",
            ]
            .contains(&s(&setting["type"]));
            let text = if short {
                t_with(
                    i18n,
                    "admin.upcoming_changes.default_warning_short",
                    &[
                        ("basePath", cx.base_path),
                        ("changeNamesFilter", s(&change["change_setting_name"])),
                    ],
                )
            } else {
                t_with(
                    i18n,
                    "admin.upcoming_changes.default_warning",
                    &[
                        ("basePath", cx.base_path),
                        ("changeNamesFilter", s(&change["change_setting_name"])),
                        ("oldDefaultValue", &value_string(&change["old_default"])),
                        ("newDefaultValue", &value_string(&change["new_default"])),
                    ],
                )
            };
            out.push_str(&format!(
                "<div class=\"setting-override-warning setting-upcoming-change-warning\"><p class=\"setting-upcoming-change-warning__text\">{} {text}</p></div>",
                icon("flask", None)
            ));
        }
        if !depends_on.is_empty() {
            let humanized: Vec<&str> = setting["depends_on_humanized_names"]
                .as_array()
                .map(|a| a.iter().map(|v| v.as_str().unwrap_or_default()).collect())
                .unwrap_or_default();
            let links: Vec<String> = depends_on
                .iter()
                .enumerate()
                .map(|(i, dep)| {
                    let label = humanized
                        .get(i)
                        .filter(|h| !h.is_empty())
                        .map(|h| h.to_string())
                        .unwrap_or_else(|| dep.replace('_', " "));
                    format!(
                        "<a href=\"{}/admin/site_settings/category/all_results?filter={}\">{}</a>",
                        cx.base_path,
                        encode_component(dep),
                        escape(&label)
                    )
                })
                .collect();
            let key = if setting["depends_on_values"]
                .as_object()
                .is_some_and(|o| !o.is_empty())
            {
                "admin.site_settings.depends_on_values_notice"
            } else {
                "admin.site_settings.depends_on_notice"
            };
            out.push_str(&format!(
                "<div class=\"setting-override-warning setting-depends-on-notice\"><p class=\"setting-depends-on-notice__text\">{} {}</p></div>",
                icon("link", None),
                t_count(
                    i18n,
                    key,
                    depends_on.len() as i64,
                    &[("dependencyLinks", &links.join(", "))]
                )
            ));
        }
    }
    out.push_str("</div>");
    // The undo button while overridden; admin/site-settings.js swaps in
    // the save and cancel controls while the value is changed.
    if over && !disabled {
        out.push_str(&format!(
            "<button class=\"btn btn-icon-text btn-default undo setting-controls__undo\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
            icon("arrow-rotate-left", None),
            escape(&t(i18n, "admin.settings.reset"))
        ));
    }
    out.push_str("</div>");
    out
}

/// `encodeURIComponent`
fn encode_component(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// A FormKit form with the setting's field (SettingDefinitionField,
/// format full, no title or description but the checkbox's).
fn form_kit(cx: &AdminCx, page: &Page, setting: &Value, control: &str, disabled: bool) -> String {
    let i18n = cx.i18n;
    let name = escape(s(&setting["setting"]));
    let value = value_string(&setting["value"]);
    let id = format!("setting-{name}");
    let dis = if disabled { " disabled" } else { "" };
    let inner = match control {
        "input" | "input-email" | "input-number" => {
            let kind = match control {
                "input-email" => "email",
                "input-number" => "number",
                _ => "text",
            };
            let minmax = if control == "input-number" {
                let mut a = String::new();
                if let Some(max) = setting["max"].as_i64() {
                    a.push_str(&format!(" max=\"{max}\""));
                }
                if let Some(min) = setting["min"].as_i64() {
                    a.push_str(&format!(" min=\"{min}\""));
                }
                a
            } else {
                String::new()
            };
            format!(
                "<div class=\"form-kit__control-input-wrapper\"><input class=\"form-kit__control-input\" id=\"{id}\" name=\"{name}\" value=\"{}\" type=\"{kind}\"{minmax}{dis}></div>",
                escape(&value)
            )
        }
        "password" => format!(
            "<div class=\"form-kit__control-password-wrapper\"><input autocomplete=\"new-password\" class=\"form-kit__control-password\" id=\"{id}\" name=\"{name}\" value=\"{}\" type=\"password\"{dis}><button class=\"btn no-text btn-icon btn-transparent form-kit__control-password-toggle\" role=\"switch\" type=\"button\">{}<span aria-hidden=\"true\"> &#8203; </span></button></div>",
            escape(&value),
            icon("far-eye", None)
        ),
        "textarea" => format!(
            "<textarea class=\"form-kit__control-textarea\" id=\"{id}\" name=\"{name}\"{dis}>{}</textarea>",
            escape(&value)
        ),
        "checkbox" => format!(
            "<label class=\"form-kit__control-checkbox-label\"><input class=\"form-kit__control-checkbox\" id=\"{id}\" name=\"{name}\" type=\"checkbox\"{}{dis}><span class=\"form-kit__control-checkbox-checkmark\">{}</span><span class=\"form-kit__control-checkbox-content\"><span class=\"form-kit__control-checkbox-description\">{}</span></span></label>",
            if value == "true" { " checked" } else { "" },
            icon("check", None),
            s(&setting["description"])
        ),
        "select" => select(cx, setting, &id, &value, disabled),
        "custom" => match s(&setting["type"]) {
            "group" => format!(
                "<div class=\"form-kit__control-custom\">{}</div>",
                group_combo(cx, page, setting, &value, disabled)
            ),
            "category" => format!(
                "<div class=\"form-kit__control-custom\">{}</div>",
                category_combo(cx, page, &value, disabled)
            ),
            // TagListSetting's TagChooser, closed.
            "tag_list" => {
                let values: Vec<&str> = value.split('|').filter(|v| !v.is_empty()).collect();
                let names: Vec<String> = values.iter().map(|v| v.to_string()).collect();
                format!(
                    "<div class=\"form-kit__control-custom\">{}</div>",
                    multi_header(cx, "tag-chooser", &names, &values)
                )
            }
            other => format!(
                "<div class=\"form-kit__control-custom\">{}</div>",
                unsupported(i18n, other)
            ),
        },
        other => unsupported(i18n, other),
    };
    let field_class = match control {
        "input-number" => "input-number".to_string(),
        other => other.to_string(),
    };
    format!(
        "<form class=\"form-kit\" novalidate=\"\"><div class=\"form-kit__container form-kit__field form-kit__field-{field_class} --full\" data-name=\"{name}\" id=\"control-{name}\" data-control-type=\"{control}\"><div class=\"form-kit__container-content\">{inner}</div></div></form>"
    )
}

/// The enum and locale_enum's native select: the none option when blank
/// is allowed, an invalid selection kept as none, the choices.
fn select(cx: &AdminCx, setting: &Value, id: &str, value: &str, disabled: bool) -> String {
    let i18n = cx.i18n;
    let name = escape(s(&setting["setting"]));
    let raw: Vec<&Value> = setting["valid_values"]
        .as_array()
        .or(setting["choices"].as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    // SettingFieldEnum's includeNone; FKControlSelect adds the none option
    // whenever the value is blank.
    let enum_none =
        raw.iter().any(|v| v.as_str() == Some("")) || setting["allows_none"] == Value::Bool(true);
    let include_none = enum_none || value.is_empty();
    let none_label = if enum_none {
        "admin.settings.none"
    } else if value.is_empty() {
        "select_placeholder"
    } else {
        "none_placeholder"
    };
    let locale = s(&setting["type"]) == "locale_enum";
    let translate = setting["translate_names"] == Value::Bool(true);
    let mut choices: Vec<(String, String)> = Vec::new();
    for v in raw {
        let (val, label) = match v {
            Value::Object(_) => {
                let val = value_string(&v["value"]);
                let label = if locale {
                    // languageNameLookup: the localized name, then the native.
                    let native = s(&v["native_name"]);
                    match t_opt(i18n, s(&v["name"])) {
                        Some(n) if n != native => format!("{n} ({native})"),
                        _ => native.to_string(),
                    }
                } else if translate && !s(&v["name"]).is_empty() {
                    t(i18n, s(&v["name"]))
                } else {
                    value_string(&v["name"])
                };
                (val, label)
            }
            Value::Null => continue,
            Value::String(v) if v.is_empty() => continue,
            other => (value_string(other), value_string(other)),
        };
        choices.push((val, label));
    }
    let mut out = format!(
        "<select class=\"form-kit__control-select d-native-select\" id=\"{id}\" name=\"{name}\"{}>",
        if disabled { " disabled" } else { "" }
    );
    if include_none {
        out.push_str(&format!(
            "<option class=\"d-native-select__option{}\" value=\"__NONE__\"{}>{}</option>",
            if value.is_empty() { " --selected" } else { "" },
            if value.is_empty() { " selected" } else { "" },
            escape(&t(i18n, none_label))
        ));
    }
    let known = value.is_empty() || choices.iter().any(|(v, _)| v == value);
    if !known {
        out.push_str(&format!(
            "<option disabled selected value=\"{}\">{}</option>",
            escape(value),
            escape(&t(i18n, "admin.settings.none"))
        ));
    }
    for (val, label) in &choices {
        let selected = val == value;
        out.push_str(&format!(
            "<option class=\"d-native-select__option{} form-kit__control-option\" value=\"{}\"{}>{}</option>",
            if selected { " --selected" } else { "" },
            escape(val),
            if selected { " selected" } else { "" },
            escape(label)
        ));
    }
    if !known {
        out.push_str(&format!(
            "<option class=\"d-native-select__option form-kit__control-option\" value=\"{v}\">{v}</option>",
            v = escape(value)
        ));
    }
    out.push_str("</select>");
    out
}

/// The group setting's clearable ComboBox of the site's groups, less the
/// disallowed ones.
fn group_combo(cx: &AdminCx, page: &Page, setting: &Value, value: &str, disabled: bool) -> String {
    let i18n = cx.i18n;
    let disallowed: Vec<&str> = s(&setting["disallowed_groups"])
        .split('|')
        .filter(|v| !v.is_empty())
        .collect();
    let options: Vec<Value> = page
        .groups
        .iter()
        .filter(|(id, _)| !disallowed.contains(&id.to_string().as_str()))
        .map(|(id, name)| {
            let name = if name == "everyone" {
                "everyone (legacy)".to_string()
            } else {
                name.clone()
            };
            serde_json::json!({ "id": id.to_string(), "name": name })
        })
        .collect();
    let selected = options
        .iter()
        .find(|o| o["id"] == value)
        .map(|o| s(&o["name"]).to_string());
    let label = selected.clone().unwrap_or_default();
    let aria = match &selected {
        Some(name) => t_with(i18n, "select_kit.filter_by", &[("name", name)]),
        None => t(i18n, "select_kit.select_to_filter"),
    };
    format!(
        "<details class=\"select-kit single-select combobox combo-box{}{}\" data-options=\"{}\"><summary aria-label=\"{a}\" name=\"{a}\" data-name=\"{n}\" data-value=\"{}\" tabindex=\"0\" class=\"select-kit-header single-select-header combo-box-header\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" title=\"{n}\"><span class=\"name\">{n}</span></div>{}</div></summary><div class=\"select-kit-body\"></div></details>",
        if selected.is_some() {
            " has-selection"
        } else {
            ""
        },
        if disabled { " is-disabled" } else { "" },
        escape(&Value::Array(options.clone()).to_string()),
        if selected.is_some() {
            escape(value)
        } else {
            String::new()
        },
        icon("angle-down", Some("angle-icon")),
        a = escape(&aria),
        n = escape(&label)
    )
}

/// The category setting's CategoryChooser, closed: its header names the
/// category, or "(no category)" (none, or Uncategorized allowed).
fn category_combo(cx: &AdminCx, page: &Page, value: &str, disabled: bool) -> String {
    let i18n = cx.i18n;
    let selected = page
        .categories
        .iter()
        .find(|(id, _)| id.to_string() == value)
        .map(|(_, name)| name.clone());
    let label = selected.clone().unwrap_or_else(|| t(i18n, "category.none"));
    let aria = t_with(i18n, "select_kit.filter_by", &[("name", &label)]);
    format!(
        "<details class=\"select-kit single-select combobox combo-box category-chooser{}{}\" data-value=\"{}\"><summary aria-label=\"{a}\" name=\"{a}\" data-name=\"{n}\"{} tabindex=\"0\" class=\"select-kit-header single-select-header combo-box-header\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"{n}\" title=\"{n}\"><span class=\"name\">{n}</span></div>{}</div></summary><div class=\"select-kit-body\"></div></details>",
        if selected.is_some() {
            " has-selection"
        } else {
            ""
        },
        if disabled { " is-disabled" } else { "" },
        escape(value),
        if selected.is_some() {
            format!(" data-value=\"{}\"", escape(value))
        } else {
            String::new()
        },
        icon("angle-down", Some("angle-icon")),
        a = escape(&aria),
        n = escape(&label),
    )
}

/// ValueList: each value's input with remove and (with more than one)
/// shift buttons, then the ComboBox of the choices not listed, headed by
/// the add key's label if any.
fn value_list(cx: &AdminCx, value: &str, delimiter: char, add_key: Option<&str>) -> String {
    let values: Vec<&str> = value.split(delimiter).filter(|v| !v.is_empty()).collect();
    let small = |class: &str, name: &str| {
        format!(
            "<button class=\"btn no-text btn-icon btn-default {class} btn-small\" type=\"button\">{}<span aria-hidden=\"true\"> &#8203; </span></button>",
            icon(name, None)
        )
    };
    let mut out = String::from("<div><div class=\"value-list\">");
    if !values.is_empty() {
        out.push_str("<div class=\"values\">");
        for (i, v) in values.iter().enumerate() {
            out.push_str(&format!(
                "<div class=\"value\" data-index=\"{i}\">{} <input class=\"value-input\" title=\"{v}\" value=\"{v}\" type=\"text\">",
                small("remove-value-btn", "xmark"),
                v = escape(v),
            ));
            if values.len() > 1 {
                out.push_str(&format!(
                    " {} {}",
                    small("shift-up-value-btn", "arrow-up"),
                    small("shift-down-value-btn", "arrow-down")
                ));
            }
            out.push_str("</div>");
        }
        out.push_str("</div>");
    }
    let label = add_key.map(|k| t(cx.i18n, k)).unwrap_or_default();
    let aria = if label.is_empty() {
        t(cx.i18n, "select_kit.select_to_filter")
    } else {
        t_with(cx.i18n, "select_kit.filter_by", &[("name", &label)])
    };
    let named = if label.is_empty() {
        String::new()
    } else {
        format!(" data-name=\"{}\"", escape(&label))
    };
    out.push_str(&format!(
        "<details class=\"select-kit single-select combobox combo-box\"><summary aria-label=\"{a}\" name=\"{a}\" data-name=\"{n}\" data-value=\"\" tabindex=\"0\" class=\"select-kit-header single-select-header combo-box-header\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\"{named} title=\"{n}\"><span class=\"name\">{n}</span></div>{}</div></summary><div class=\"select-kit-body\"></div></details></div></div>",
        icon("angle-down", Some("angle-icon")),
        a = escape(&aria),
        n = escape(&label),
    ));
    out
}

/// SecretValueList: each key and secret (lines of `key|secret`), the
/// secret masked while the setting is secret, then the new pair's inputs.
fn secret_list(setting: &Value, value: &str) -> String {
    let secret_type = if setting["secret"] == Value::Bool(true) {
        "password"
    } else {
        "text"
    };
    let pairs: Vec<&str> = if value.is_empty() {
        Vec::new()
    } else {
        value.split('\n').collect()
    };
    let button = |class: &str, name: &str| {
        format!(
            "<button class=\"btn no-text btn-icon {class} btn-small\" type=\"button\">{}<span aria-hidden=\"true\"> &#8203; </span></button>",
            icon(name, None)
        )
    };
    let mut out = String::from("<div class=\"value-list secret-value-list\">");
    if !pairs.is_empty() {
        out.push_str("<div class=\"values\">");
        for (i, pair) in pairs.iter().enumerate() {
            let mut parts = pair.split('|');
            let key = parts.next().unwrap_or_default();
            let secret = parts.next().unwrap_or_default();
            out.push_str(&format!(
                "<div class=\"value\" data-index=\"{i}\">{} <input class=\"value-input\" value=\"{}\" type=\"text\"> <input class=\"value-input\" value=\"{}\" type=\"{secret_type}\"></div>",
                button("btn-default remove-value-btn", "xmark"),
                escape(key),
                escape(secret)
            ));
        }
        out.push_str("</div>");
    }
    // Its placeholders read `this.setting`, which the component never has.
    out.push_str(&format!(
        "<div class=\"value\"><input placeholder=\"\" class=\"new-value-input key\" type=\"text\"> <input class=\"new-value-input secret\" type=\"{secret_type}\"> {}</div></div>",
        button("add-value-btn", "plus")
    ));
    out
}

/// A closed MultiSelect's header: the selection's names, or Select….
fn multi_header(cx: &AdminCx, class: &str, names: &[String], values: &[&str]) -> String {
    format!(
        "<details class=\"select-kit multi-select {class}{}\"><summary data-name=\"{}\" data-value=\"{}\" tabindex=\"0\" class=\"select-kit-header multi-select-header\"><div class=\"select-kit-header-wrapper\"><span class=\"formatted-selection\">{}</span>{}</div></summary><div class=\"select-kit-body\"></div></details>",
        if values.is_empty() {
            ""
        } else {
            " has-selection"
        },
        escape(&names.join(",")),
        escape(&values.join(",")),
        escape(&if values.is_empty() {
            t(cx.i18n, "select_kit.default_header_text")
        } else {
            names.join(", ")
        }),
        icon("plus", Some("angle-icon")),
    )
}

/// FileTypesList's buttons: (class, label key, the types each adds).
const FILE_TYPES: [(&str, &str, &[&str]); 4] = [
    (
        "image",
        "add_image_types",
        &[
            "gif", "png", "jpeg", "jpg", "heic", "heif", "webp", "avif", "svg",
        ],
    ),
    (
        "video",
        "add_video_types",
        &["mov", "mp4", "webm", "m4v", "3gp", "ogv", "avi", "mpeg"],
    ),
    (
        "",
        "add_audio_types",
        &["mp3", "ogg", "m4a", "wav", "aac", "flac"],
    ),
    (
        "document",
        "add_document_types",
        &["txt", "pdf", "doc", "docx", "csv"],
    ),
];

/// The legacy editors: simple_list's SimpleList, the list-setting
/// MultiSelect's closed header, the font-list ComboBox's; any other says
/// it is not ported.
fn legacy(cx: &AdminCx, page: &Page, setting: &Value, kind: &str, disabled: bool) -> String {
    let i18n = cx.i18n;
    let value = value_string(&setting["value"]);
    let values: Vec<&str> = value.split('|').filter(|v| !v.is_empty()).collect();
    match kind {
        "simple_list" => {
            let small = |class: &str, name: &str| {
                format!(
                    "<button class=\"btn no-text btn-icon btn-default {class} btn-small\" type=\"button\">{}<span aria-hidden=\"true\"> &#8203; </span></button>",
                    icon(name, None)
                )
            };
            let mut out = String::from("<div><div class=\"simple-list value-list\">");
            if !values.is_empty() {
                out.push_str("<div class=\"values\">");
                for (i, v) in values.iter().enumerate() {
                    out.push_str(&format!(
                        "<div class=\"value\" data-index=\"{i}\">{} <input class=\"value-input\" title=\"{v}\" value=\"{v}\" type=\"text\">",
                        small("remove-value-btn", "xmark"),
                        v = escape(v),
                    ));
                    if values.len() > 1 {
                        out.push_str(&format!(
                            " {} {}",
                            small("shift-up-value-btn", "arrow-up"),
                            small("shift-down-value-btn", "arrow-down")
                        ));
                    }
                    out.push_str("</div>");
                }
                out.push_str("</div>");
            }
            let choices: Vec<String> = setting["choices"]
                .as_array()
                .map(|a| a.iter().map(value_string).collect())
                .unwrap_or_default();
            let predefined =
                !setting["allow_any"].as_bool().unwrap_or(false) && !choices.is_empty();
            if predefined {
                // The ComboBox of the choices not in the list yet, while
                // any are left; site-settings.js brings it back from the
                // choices.
                let all: Vec<Value> = choices
                    .iter()
                    .map(|c| serde_json::json!({ "id": c, "name": c }))
                    .collect();
                let options = escape(&Value::Array(all).to_string());
                out.push_str(&format!(
                    "<div class=\"simple-list-input\" data-choices=\"{options}\">"
                ));
                if choices.iter().any(|c| !values.contains(&c.as_str())) {
                    out.push_str(&format!(
                        "<details class=\"select-kit single-select combobox combo-box add-value-input\" data-options=\"{options}\"><summary aria-label=\"{a}\" name=\"{a}\" data-name=\"\" data-value=\"\" tabindex=\"0\" class=\"select-kit-header single-select-header combo-box-header\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" title=\"\"><span class=\"name\"></span></div>{}</div></summary><div class=\"select-kit-body\"></div></details>",
                        icon("angle-down", Some("angle-icon")),
                        a = escape(&t(i18n, "select_kit.select_to_filter")),
                    ));
                }
            } else {
                out.push_str(&format!(
                    "<div class=\"simple-list-input\"><input autocapitalize=\"off\" autocomplete=\"off\" autocorrect=\"off\" class=\"add-value-input\" placeholder=\"{}\" type=\"text\"{}> <button class=\"btn no-text btn-icon add-value-btn btn-default btn-small\" disabled=\"\" type=\"button\">{}<span aria-hidden=\"true\"> &#8203; </span></button>",
                    escape(&t(i18n, "admin.site_settings.simple_list.add_item")),
                    if disabled { " disabled" } else { "" },
                    icon("plus", None)
                ));
            }
            out.push_str("</div></div></div>");
            out
        }
        // List, UrlList and SiteSettingValueList: ValueList.
        "list" | "url_list" | "value_list" => {
            let delimiter = if kind == "list" { '|' } else { '\n' };
            value_list(
                cx,
                &value,
                delimiter,
                (kind == "url_list").then_some("admin.site_settings.add_url"),
            )
        }
        "secret_list" => secret_list(setting, &value),
        // UploadedImageList: the button to its modal.
        "uploaded_image_list" => format!(
            "<div><button class=\"btn uploaded-image-list-edit\"{} type=\"button\"><span class=\"d-button-label\">{}</span></button></div>",
            if disabled { " disabled" } else { "" },
            escape(&t(i18n, "admin.site_settings.uploaded_image_list.label"))
        ),
        "compact_list" | "group_list" | "file_types_list" | "host_list" => {
            let names = multi_names(page, setting, &values);
            let header = format!(
                "<details class=\"select-kit multi-select list-setting{}\"><summary data-name=\"{}\" data-value=\"{}\" tabindex=\"0\" class=\"select-kit-header multi-select-header\"><div class=\"select-kit-header-wrapper\"><span class=\"formatted-selection\">{}</span>{}</div></summary><div class=\"select-kit-body\"></div></details>",
                if values.is_empty() {
                    ""
                } else {
                    " has-selection"
                },
                escape(&names.replace(", ", ",")),
                escape(&values.join(",")),
                escape(&if values.is_empty() {
                    t(i18n, "select_kit.default_header_text")
                } else {
                    names
                }),
                icon("plus", Some("angle-icon")),
            );
            match kind {
                // SettingGroupList and HostList wrap their chooser in a div.
                "group_list" | "host_list" => format!("<div>{header}</div>"),
                "file_types_list" => {
                    let mut out = header;
                    for (class, key, types) in FILE_TYPES {
                        out.push_str(&format!(
                            " <button class=\"btn btn btn-small btn-default file-types-list__button{}\" data-types=\"{}\" title=\"{}\" type=\"button\"><span class=\"d-button-label\">{}</span></button>",
                            if class.is_empty() { String::new() } else { format!(" {class}") },
                            types.join("|"),
                            escape(&t_with(
                                i18n,
                                "admin.site_settings.file_types_list.add_types_title",
                                &[("types", &types.join(", "))]
                            )),
                            escape(&t(i18n, &format!("admin.site_settings.file_types_list.{key}")))
                        ));
                    }
                    out
                }
                _ => header,
            }
        }
        // SettingCategoryList's CategorySelector, closed.
        "category_list" => {
            let names: Vec<String> = values
                .iter()
                .map(|v| {
                    page.categories
                        .iter()
                        .find(|(id, _)| id.to_string() == *v)
                        .map(|(_, n)| n.clone())
                        .unwrap_or_else(|| v.to_string())
                })
                .collect();
            format!(
                "<div>{}</div>",
                multi_header(cx, "category-selector", &names, &values)
            )
        }
        "upload" => upload(cx, setting, &value, disabled),
        "emoji_list" => emoji_list(cx, setting, &values),
        "font_list" => font_list(cx, setting, &value),
        "file_size_restriction" => file_size(cx, &value),
        "color" => color_input(setting, &value),
        other => unsupported(i18n, other),
    }
}

/// ColorInput (onlyHex off): the value as typed and the native picker with
/// it as #rrggbb when it is a hex color. welcome_banner_text_color falls
/// back to --primary, which only the browser knows: site-settings.js fills
/// that picker in.
fn color_input(setting: &Value, value: &str) -> String {
    let hex = value.strip_prefix('#').unwrap_or(value);
    let valid = (hex.len() == 3 || hex.len() == 6) && hex.chars().all(|c| c.is_ascii_hexdigit());
    let picker = if valid {
        let full = if hex.len() == 3 {
            hex.chars().flat_map(|c| [c, c]).collect::<String>()
        } else {
            hex.to_string()
        };
        format!("#{full}")
    } else {
        value.to_string()
    };
    format!(
        "<div class=\"color-picker\"{}><input placeholder=\"\" class=\"hex-input\" type=\"text\" value=\"{}\"> <input class=\"picker\" title=\"{p}\" type=\"color\" value=\"{p}\"></div>",
        if s(&setting["setting"]) == "welcome_banner_text_color" {
            " data-fallback-primary=\"\""
        } else {
            ""
        },
        escape(value),
        p = escape(&picker),
    )
}

/// FileSizeInput: the size in the largest unit it fills (KB, MB, GB) and
/// the unit's ComboBox. Its options name themselves by `label`, which the
/// ComboBox doesn't read: the header's name stays empty.
fn file_size(cx: &AdminCx, value: &str) -> String {
    let i18n = cx.i18n;
    let kb: f64 = value.trim().parse().unwrap_or(0.0);
    let unit = if kb >= 1024.0 * 1024.0 {
        "gb"
    } else if kb >= 1024.0 {
        "mb"
    } else {
        "kb"
    };
    // `number`: nothing for 0 or blank; JS's Number#toString otherwise.
    let number = match unit {
        _ if kb == 0.0 => String::new(),
        "kb" => value.trim().to_string(),
        "mb" => format!("{}", kb / 1024.0),
        _ => format!("{}", kb / 1024.0 / 1024.0),
    };
    let options: Vec<Value> = ["kb", "mb", "gb"]
        .iter()
        .map(|u| {
            serde_json::json!({
                "id": u,
                "name": t(i18n, &format!("number.human.storage_units.units.{u}")),
            })
        })
        .collect();
    let label = t(i18n, &format!("number.human.storage_units.units.{unit}"));
    let aria = t(i18n, "select_kit.filter_by").replace("%{name}", "[missing %{name} value]");
    format!(
        "<div class=\"file-size-picker\"><input class=\"file-size-input\" step=\"any\" type=\"number\" value=\"{}\"> <details class=\"select-kit single-select combobox combo-box has-selection file-size-unit-selector\" data-options=\"{}\"><summary aria-label=\"{a}\" name=\"{a}\" data-name=\"\" data-value=\"{unit}\" tabindex=\"0\" class=\"select-kit-header single-select-header combo-box-header\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-value=\"{unit}\" title=\"\"><span class=\"name\">{}</span></div>{}</div></summary><div class=\"select-kit-body\"></div></details></div>",
        escape(&number),
        escape(&Value::Array(options).to_string()),
        escape(&label),
        icon("angle-down", Some("angle-icon")),
        a = escape(&aria),
    )
}

/// FontList's FontSelector: a ComboBox of the fonts, each named in its own
/// font class (body-font-*, heading-font-* for heading_font).
fn font_list(cx: &AdminCx, setting: &Value, value: &str) -> String {
    let prefix = if s(&setting["setting"]) == "heading_font" {
        "heading-font-"
    } else {
        "body-font-"
    };
    let options: Vec<Value> = setting["choices"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|c| {
                    let id = value_string(&c["value"]);
                    serde_json::json!({
                        "id": id,
                        "name": value_string(&c["name"]),
                        "classNames": format!("{prefix}{}", id.replace('_', "-")),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let selected = options.iter().find(|o| o["id"] == value);
    let name = selected
        .map(|o| s(&o["name"]).to_string())
        .unwrap_or_default();
    let aria = match selected {
        Some(_) => t_with(cx.i18n, "select_kit.filter_by", &[("name", &name)]),
        None => t(cx.i18n, "select_kit.select_to_filter"),
    };
    format!(
        "<details class=\"select-kit single-select combobox combo-box font-selector{}\" data-options=\"{}\"><summary aria-label=\"{a}\" name=\"{a}\" data-name=\"{n}\" data-value=\"{}\" tabindex=\"0\" class=\"select-kit-header single-select-header combo-box-header\"><div class=\"select-kit-header-wrapper\"><span class=\"name {}\">{n}</span>{}</div></summary><div class=\"select-kit-body\"></div></details>",
        if selected.is_some() {
            " has-selection"
        } else {
            ""
        },
        escape(&Value::Array(options.clone()).to_string()),
        if selected.is_some() {
            escape(value)
        } else {
            String::new()
        },
        selected
            .map(|o| s(&o["classNames"]).to_string())
            .unwrap_or_default(),
        icon("angle-down", Some("angle-icon")),
        a = escape(&aria),
        n = escape(&name),
    )
}

/// EmojiValueList: each emoji with its image, remove and (with more than
/// one) shift buttons, and the Add Emoji picker button. The images of the
/// value's and the default's emoji ride along (data-emoji-urls) for
/// site-settings.js to redraw the list with.
fn emoji_list(cx: &AdminCx, setting: &Value, values: &[&str]) -> String {
    let i18n = cx.i18n;
    let emojis = cx.settings.emojis();
    let url = |name: &str| emojis.url(name, cx.base_path).unwrap_or_default();
    let small = |class: &str, name: &str| {
        format!(
            "<button class=\"btn no-text btn-icon btn-default {class} btn-small\" type=\"button\">{}<span aria-hidden=\"true\"> &#8203; </span></button>",
            icon(name, None)
        )
    };
    let mut urls = serde_json::Map::new();
    let default = value_string(&setting["default"]);
    for name in values
        .iter()
        .copied()
        .chain(default.split('|').filter(|v| !v.is_empty()))
    {
        urls.insert(name.to_string(), Value::String(url(name)));
    }
    let mut out = format!(
        "<div class=\"value-list emoji-list\" data-emoji-urls=\"{}\">",
        escape(&Value::Object(urls).to_string())
    );
    if !values.is_empty() {
        out.push_str("<ul class=\"values emoji-value-list\">");
        for (i, v) in values.iter().enumerate() {
            out.push_str(&format!(
                "<li class=\"value\" data-index=\"{i}\">{} <div class=\"value-input emoji-details can-edit\" role=\"button\"> <img class=\"emoji-list-emoji\" height=\"15px\" src=\"{}\" width=\"15px\"> <span class=\"emoji-name\">{}</span> </div>",
                small("remove-value-btn", "xmark"),
                escape(&url(v)),
                escape(v)
            ));
            if values.len() > 1 {
                out.push_str(&format!(
                    " {} {}",
                    small("shift-up-value-btn", "arrow-up"),
                    small("shift-down-value-btn", "arrow-down")
                ));
            }
            out.push_str("</li>");
        }
        out.push_str("</ul>");
    }
    out.push_str(&format!(
        "<div class=\"value\"><button aria-expanded=\"false\" class=\"btn no-text fk-d-menu__trigger emoji-picker-trigger btn-default\" data-identifier=\"emoji-picker\" data-trigger=\"\" type=\"button\">{} <span class=\"d-button-label\">{}</span></button></div></div>",
        icon("far-face-smile", None),
        escape(&t(i18n, "admin.site_settings.emoji_list.add_emoji_button.label"))
    ));
    out
}

/// SiteSettingUpload: the file's preview (an image's lightbox link with
/// its size, or the file's name and a download link) and its Change and
/// Delete controls, or with no file the button to pick one.
fn upload(cx: &AdminCx, setting: &Value, value: &str, disabled: bool) -> String {
    let i18n = cx.i18n;
    let name = s(&setting["setting"]);
    let id = format!("site-setting-file-uploader-{name}");
    let input_id = format!("{id}__input");
    let extensions = s(&setting["authorized_extensions"]);
    let accept = if extensions.is_empty() {
        "image/*".to_string()
    } else {
        extensions
            .split('|')
            .map(|e| format!(".{e}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    // authorizesOneOrMoreExtensions for staff.
    let listed = |key: &str| {
        cx.settings
            .get(key)
            .map(|v| v.to_s().split('|').any(|e| !e.is_empty()))
            .unwrap_or(false)
    };
    let not_allowed = extensions.is_empty()
        && !listed("authorized_extensions")
        && !listed("authorized_extensions_for_staff");
    let disabled = disabled || not_allowed;
    let title = if disabled && not_allowed {
        format!(
            " title=\"{}\"",
            escape(&t(i18n, "post.errors.no_uploads_authorized"))
        )
    } else {
        String::new()
    };
    let is_image = {
        let lower = value.to_ascii_lowercase();
        [
            ".png", ".webp", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".heic", ".heif", ".avif",
            ".jxl",
        ]
        .iter()
        .any(|e| lower.ends_with(e))
    };
    // getURLWithCDN: the base path for a local path, no CDN configured.
    let cdn = if value.starts_with('/') && !value.starts_with("//") {
        format!("{}{value}", cx.base_path)
    } else {
        value.to_string()
    };
    let upload = &setting["upload"];
    let file_name = match s(&upload["original_filename"]) {
        "" => value.rsplit('/').next().unwrap_or_default().to_string(),
        n => n.to_string(),
    };
    let filesize = s(&upload["human_filesize"]);
    let pick = |label_class: &str, label: &str| {
        format!(
            "<label class=\"{label_class}{}\" for=\"{input_id}\" tabindex=\"0\"{title}>{} <div class=\"pick-files-button\"><input accept=\"{}\" id=\"{input_id}\" type=\"file\"{}></div> {}</label>",
            if disabled { " disabled" } else { "" },
            icon("upload", None),
            escape(&accept),
            if disabled { " disabled" } else { "" },
            escape(&t(i18n, label))
        )
    };
    let mut out = format!(
        "<div class=\"file-uploader {}{}\" id=\"{id}\"><div class=\"file-uploader__preview input-xxlarge{}\"{}>",
        if value.is_empty() {
            "no-file"
        } else {
            "has-file"
        },
        if is_image && !value.is_empty() {
            " has-image"
        } else {
            ""
        },
        if name == "welcome_banner_image" {
            " --bg-size-cover"
        } else {
            ""
        },
        if is_image && !value.is_empty() {
            format!(" style=\"background-image: url({})\"", escape(&cdn))
        } else {
            String::new()
        },
    );
    if value.is_empty() {
        out.push_str(&format!(
            "<div class=\"file-uploader__controls\">{}<div class=\"file-uploader__progress-status hidden\"><div aria-label=\"{}\" class=\"progress-bar-container\" role=\"progressbar\"><div class=\"progress-bar\" style=\"width: 0%\"></div></div><span>{}</span></div></div>",
            pick("btn btn-transparent", "upload_selector.select_file"),
            escape(&format!("{} 0%", t(i18n, "upload_selector.uploading"))),
            escape(&format!("{} 0%", t(i18n, "upload_selector.uploading"))),
        ));
    } else if is_image {
        let dims = match (upload["width"].as_i64(), upload["height"].as_i64()) {
            (Some(w), Some(h)) => format!("{w}x{h} "),
            _ => String::new(),
        };
        out.push_str(&format!(
            "<a class=\"lightbox\" href=\"{c}\" rel=\"nofollow ugc noopener\" title=\"{}\"><div class=\"meta\"><span class=\"informations\">{}{}</span></div></a><div class=\"expand-overlay\"><button class=\"btn no-text btn-icon btn-default btn-small file-uploader-lightbox-btn\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\"> &#8203; </span></button></div>",
            escape(&file_name),
            dims,
            escape(filesize),
            escape(&t(i18n, "expand")),
            icon("discourse-expand", None),
            c = escape(&cdn),
        ));
    } else {
        out.push_str(&format!(
            "<div class=\"file-info\"><div class=\"file-icon\">{}</div><div class=\"file-details\"><span class=\"file-name\">{n}</span>{}</div><a class=\"btn btn-default btn-small no-text download-btn\" download=\"{n}\" href=\"{}\" rel=\"nofollow ugc noopener noreferrer\" target=\"_blank\" title=\"{}\">{}</a></div>",
            icon("file", None),
            if filesize.is_empty() {
                String::new()
            } else {
                format!("<span class=\"file-size\">{}</span>", escape(filesize))
            },
            escape(&cdn),
            escape(&t(i18n, "admin.site_settings.download_file")),
            icon("download", None),
            n = escape(&file_name),
        ));
    }
    out.push_str("</div>");
    // restrictionsInfo
    let mut parts = Vec::new();
    if !extensions.is_empty() {
        let list = extensions
            .split('|')
            .map(|e| format!(".{e}"))
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(t_with(
            i18n,
            "admin.site_settings.upload_restrictions.extensions",
            &[("extensions", &list)],
        ));
    }
    if let Some(kb) = setting["max_file_size_kb"].as_i64() {
        let size = if kb >= 1024 {
            format!(
                "{} MB",
                crate::admin_dashboard_view::to_fixed(kb as f64 / 1024.0, 1)
            )
        } else {
            format!("{kb} KB")
        };
        parts.push(t_with(
            i18n,
            "admin.site_settings.upload_restrictions.max_size",
            &[("size", &size)],
        ));
    }
    if !parts.is_empty() {
        out.push_str(&format!(
            "<div class=\"file-uploader__restrictions\">{}</div>",
            escape(&parts.join(" · "))
        ));
    }
    if !value.is_empty() {
        out.push_str(&format!(
            "<div class=\"file-uploader__controls\">{} <button class=\"btn btn-icon-text btn-danger btn-small\"{} type=\"button\">{}<span class=\"d-button-label\">{}</span></button></div>",
            pick("btn btn-default btn-small ", "upload_selector.change"),
            if disabled { " disabled" } else { "" },
            icon("trash-can", None),
            escape(&t(i18n, "upload_selector.delete"))
        ));
    }
    out.push_str("</div>");
    out
}

/// What a list-setting header names: the values, or for a group list the
/// groups' names (the site's groups).
fn multi_names(page: &Page, setting: &Value, values: &[&str]) -> String {
    let groups = setting["list_type"] == "group" || s(&setting["type"]) == "group_list";
    values
        .iter()
        .map(|v| {
            let group = page.groups.iter().find(|(id, _)| id.to_string() == *v);
            if groups && let Some((_, name)) = group {
                return name.clone();
            }
            setting["valid_values"]
                .as_array()
                .and_then(|a| a.iter().find(|o| value_string(&o["value"]) == *v))
                .map(|o| value_string(&o["name"]))
                .unwrap_or_else(|| v.to_string())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// An editor not ported yet, said in its place.
fn unsupported(i18n: &crate::i18n::I18n, kind: &str) -> String {
    let _ = i18n;
    format!(
        "<div class=\"setting-editor-unsupported\" role=\"note\">Not ported yet: the {} setting editor.</div>",
        escape(&kind.replace('_', " "))
    )
}

/// The strings site-settings.js renders with.
fn client_strings(cx: &AdminCx, settings: &[Value]) -> String {
    const KEYS: &[&str] = &[
        "admin.settings.save",
        "admin.settings.cancel",
        "admin.settings.reset",
        "admin.site_settings.dirty_banner.one",
        "admin.site_settings.dirty_banner.other",
        "admin.site_settings.save.one",
        "admin.site_settings.save.other",
        "admin.site_settings.discard.one",
        "admin.site_settings.discard.other",
        "no_value",
        "select_kit.select_to_filter",
        "upload_selector.select_file",
        "select_kit.default_header_text",
        "admin.site_settings.file_types_list.add_types_toast",
        "generic_error",
    ];
    let mut map: serde_json::Map<String, Value> = KEYS
        .iter()
        .map(|k| {
            (
                k.to_string(),
                Value::String(t_opt(cx.i18n, k).unwrap_or_else(|| format!("[en.{k}]"))),
            )
        })
        .collect();
    // confirmChanges' prompt and button for each setting that asks, its
    // own or the default.
    for setting in settings
        .iter()
        .filter(|s| !s["requires_confirmation"].is_null())
    {
        let name = s(&setting["setting"]);
        for part in ["prompt", "confirm"] {
            let base = "admin.site_settings.requires_confirmation_messages";
            let text = t_opt(cx.i18n, &format!("{base}.{name}.{part}"))
                .unwrap_or_else(|| t(cx.i18n, &format!("{base}.default.{part}")));
            map.insert(format!("confirm.{name}.{part}"), Value::String(text));
        }
    }
    let dialog = serde_json::json!({
        "ok_value": t(cx.i18n, "ok_value"),
        "cancel": t(cx.i18n, "cancel"),
        "modal.close": t(cx.i18n, "modal.close"),
    });
    format!(
        "<script type=\"application/json\" id=\"site-settings-strings\">{}</script><script type=\"application/json\" id=\"dialog-strings\">{}</script>",
        Value::Object(map).to_string().replace('<', "\\u003c"),
        dialog.to_string().replace('<', "\\u003c")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings() -> Vec<Value> {
        vec![
            json!({"setting": "title", "category": "required", "description": "The name", "value": "Discourse", "default": "Discourse", "keywords": []}),
            json!({"setting": "site_description", "category": "required", "description": "Describe", "value": "x", "default": "", "keywords": []}),
            json!({"setting": "min_post_length", "category": "posting", "description": "Minimum", "value": "20", "default": "20", "keywords": ["characters"]}),
        ]
    }

    #[test]
    fn the_all_category_comes_first_and_categories_keep_their_order() {
        let cats = filter(&settings(), &Filters::default());
        let keys: Vec<&str> = cats.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["all_results", "required", "posting"]);
        assert_eq!(cats[0].count, "3");
    }

    #[test]
    fn filters_match_names_keywords_and_overrides() {
        let f = Filters {
            filter: "characters".into(),
            ..Default::default()
        };
        let cats = filter(&settings(), &f);
        assert_eq!(cats.len(), 2);
        assert_eq!(cats[1].key, "posting");
        let f = Filters {
            only_overridden: true,
            ..Default::default()
        };
        let cats = filter(&settings(), &f);
        assert_eq!(cats[0].settings, vec![1]);
        let f = Filters {
            filter: "plugin:poll".into(),
            ..Default::default()
        };
        assert_eq!(filter(&settings(), &f).len(), 1);
    }

    #[test]
    fn fuzzy_names_match_with_few_gaps() {
        let s = json!({"setting": "min_post_length", "description": "", "keywords": []});
        assert!(Matcher::new("minpost", &s).fuzzy_name_match().is_some());
        assert!(Matcher::new("mpl", &s).fuzzy_name_match().is_some());
        assert!(Matcher::new("zzz", &s).fuzzy_name_match().is_none());
    }
}
