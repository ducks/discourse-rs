//! The customize admin pages (90g7) as Ember draws them: the watched
//! words. Each page sits in admin-customize.gjs's `.admin-config-page`.

use serde_json::Value;

use crate::admin_view::{AdminCx, Crumb, PageHeader, t, t_count, t_with};
use crate::topic_list_view::{escape, icon};

/// admin-customize.gjs around the route's outlet.
pub fn customize_page(inner: &str) -> String {
    format!("<div class=\"admin-config-page\">{inner}</div>")
}

/// templates/admin-customize-robots-txt.gjs from
/// Admin::RobotsTxtController#show's document.
pub fn robots(cx: &AdminCx, doc: &Value) -> String {
    let overridden = doc["overridden"].as_bool() == Some(true);
    let reset = crate::admin_ui::Button {
        class: "btn-default",
        icon: Some("arrow-rotate-left"),
        label: crate::admin_ui::tr(cx, "admin.settings.reset"),
        disabled: !overridden,
        ..Default::default()
    }
    .render();
    format!(
        "<div class=\"robots-txt-edit\"><h3>{}</h3><p>{}</p>{}<textarea class=\"ember-text-area robots-txt-input\">{}</textarea>{}</div>",
        escape(&t(cx.i18n, "admin.customize.robots.title")),
        escape(&t(cx.i18n, "admin.customize.robots.warning")),
        if overridden {
            format!(
                "<div class=\"overridden\">{}</div>",
                escape(&t(cx.i18n, "admin.customize.robots.overridden"))
            )
        } else {
            String::new()
        },
        escape(doc["robots_txt"].as_str().unwrap_or_default()),
        crate::admin_customize_kit::save_controls(cx, &reset)
    )
}

/// What the watched words page shows: the list's document
/// (/admin/customize/watched_words.json), the action, the filter, and
/// whether the words are shown (the checkbox, or a filter).
pub struct WatchedWords<'a> {
    pub list: &'a Value,
    pub action: &'a str,
    pub filter: &'a str,
    pub show_words: bool,
    pub regex: bool,
}

impl WatchedWords<'_> {
    /// WatchedWord.findAll's actions with their words, each filtered as
    /// updateFilteredContent does (`word.includes(filter)`, the filter
    /// lowercased).
    fn actions(&self) -> Vec<(String, Vec<&Value>)> {
        let filter = self.filter.to_lowercase();
        let words: Vec<&Value> = self.list["words"]
            .as_array()
            .map(|w| w.iter().collect())
            .unwrap_or_default();
        self.list["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|key| {
                let own = words
                    .iter()
                    .copied()
                    .filter(|w| w["action"].as_str() == Some(key))
                    .filter(|w| {
                        filter.is_empty()
                            || w["word"].as_str().unwrap_or_default().contains(&filter)
                    })
                    .collect();
                (key.to_string(), own)
            })
            .collect()
    }
}

/// templates/admin-watched-words.gjs with the action's outlet.
pub fn watched_words(cx: &AdminCx, page: &WatchedWords) -> String {
    let title = t(cx.i18n, "admin.config.watched_words.title");
    let header = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path: "/admin/customize/watched_words",
            },
        ],
        title,
        actions: String::new(),
        description: Some(escape(&t(
            cx.i18n,
            "admin.config.watched_words.header_description",
        ))),
        learn_more_url: Some("https://meta.discourse.org/t/241735".into()),
        tabs: None,
    }
    .render(cx);
    let actions = page.actions();
    let toggle = if crate::mobile::mobile_view() {
        crate::admin_ui::Button {
            class: "btn-default menu-toggle",
            icon: Some("bars"),
            ..Default::default()
        }
        .render()
    } else {
        String::new()
    };
    let controls = format!(
        "<div class=\"admin-controls\"><div class=\"controls\"><div class=\"inline-form\">{toggle}<input class=\"ember-text-field no-blur\" placeholder=\"{}\" type=\"text\" value=\"{}\">{}</div></div></div>",
        escape(&t(cx.i18n, "admin.watched_words.search")),
        escape(page.filter),
        crate::admin_ui::Button {
            class: "btn-default",
            label: crate::admin_ui::tr(cx, "admin.watched_words.clear_filter"),
            ..Default::default()
        }
        .render()
    );
    format!(
        "{header}<div class=\"admin-contents\">{controls}<div class=\"admin-nav pull-left\">{}</div><div class=\"admin-detail pull-left watched-words-detail\">{}</div><div class=\"clearfix\"></div></div>",
        watched_words_nav(cx, &actions, page.action),
        watched_words_action(cx, page, &actions)
    )
}

/// AdminWatchedWordsActionNav: each action's link, its count while it has
/// words.
pub fn watched_words_nav(cx: &AdminCx, actions: &[(String, Vec<&Value>)], active: &str) -> String {
    let items: String = actions
        .iter()
        .map(|(key, words)| {
            let count = if words.is_empty() {
                String::new()
            } else {
                format!(" <span class=\"count\">({})</span>", words.len())
            };
            format!(
                "<li class=\"{key}\"><a{} href=\"{}/admin/customize/watched_words/action/{key}\">{}{count}</a></li>",
                if key == active { " class=\"active\"" } else { "" },
                cx.base_path,
                escape(&t(cx.i18n, &format!("admin.watched_words.actions.{key}")))
            )
        })
        .collect();
    format!("<ul class=\"nav nav-stacked\">{items}</ul>")
}

/// templates/admin-watched-words/action.gjs.
fn watched_words_action(
    cx: &AdminCx,
    page: &WatchedWords,
    actions: &[(String, Vec<&Value>)],
) -> String {
    let key = page.action;
    let button = |class: &'static str, icon: &'static str, label: &str| {
        crate::admin_ui::Button {
            class,
            icon: Some(icon),
            label: crate::admin_ui::tr(cx, label),
            ..Default::default()
        }
        .render()
    };
    let download = format!(
        "<a class=\"btn btn-icon-text btn-default download-link\" href=\"{}/admin/customize/watched_words/action/{}/download\">{}<span class=\"d-button-label\">{}</span></a>",
        cx.base_path,
        escape(key),
        icon("download", None),
        escape(&t(cx.i18n, "admin.watched_words.download"))
    );
    let uploader = format!(
        "<div class=\"watched-words-uploader\"><label class=\"btn btn-default\">{}{}<input class=\"hidden-upload-field\" type=\"file\"></label></div>",
        icon("upload", None),
        escape(&t(cx.i18n, "admin.watched_words.form.upload"))
    );
    let mut out = format!(
        "<div class=\"watched-word-controls\">{download} {uploader} {} {}</div><p class=\"about\">{}</p>",
        button(
            "btn-default watched-word-test",
            "far-eye",
            "admin.watched_words.test.button_label"
        ),
        button(
            "btn-danger clear-all",
            "trash-can",
            "admin.watched_words.clear_all"
        ),
        escape(&t(
            cx.i18n,
            &format!("admin.watched_words.action_descriptions.{key}")
        ))
    );
    if page.regex {
        out.push_str(&format!(
            "<p>{}</p>",
            t_with(
                cx.i18n,
                "admin.watched_words.regex_warning",
                &[("basePath", cx.base_path)]
            )
        ));
    }
    out.push_str(&watched_word_form(cx, key, page.regex));
    let words = actions
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, w)| w.as_slice())
        .unwrap_or_default();
    out.push_str(&watched_words_list(
        cx,
        key,
        words,
        page.show_words || !page.filter.is_empty(),
        page.show_words,
    ));
    out
}

/// The show words checkbox (while the action has words) and the list,
/// shown while it is checked or a filter is set.
pub fn watched_words_list(
    cx: &AdminCx,
    key: &str,
    words: &[&Value],
    shown: bool,
    checked: bool,
) -> String {
    let mut out = String::new();
    if !words.is_empty() {
        out.push_str(&format!(
            "<label class=\"show-words-checkbox\"><input class=\"ember-checkbox\" type=\"checkbox\"{}> {}</label>",
            if checked { " checked=\"\"" } else { "" },
            escape(&t_count(
                cx.i18n,
                "admin.watched_words.show_words",
                words.len() as i64,
                &[]
            ))
        ));
    }
    if shown {
        let boxes: String = words
            .iter()
            .map(|w| {
                format!(
                    "<div class=\"watched-word-box\">{}</div>",
                    watched_word(cx, key, w)
                )
            })
            .collect();
        out.push_str(&format!(
            "<div class=\"watched-words-list watched-words-{}\">{boxes}</div>",
            escape(key)
        ));
    }
    out
}

/// AdminWatchedWord.
fn watched_word(cx: &AdminCx, key: &str, word: &Value) -> String {
    let mut out = format!(
        "<div class=\"watched-word\" data-id=\"{}\">{}<span class=\"watched-word__content\">{}</span>",
        word["id"],
        crate::admin_ui::Button {
            class: "btn-transparent delete-word-record",
            icon: Some("xmark"),
            ..Default::default()
        }
        .render(),
        escape(word["word"].as_str().unwrap_or_default())
    );
    let replacement = word["replacement"].as_str().unwrap_or_default();
    match key {
        "replace" | "link" => out.push_str(&format!(
            " \u{2192} <span class=\"replacement\">{}</span>",
            escape(replacement)
        )),
        "tag" => {
            out.push_str(" \u{2192} ");
            for tag in replacement.split(',') {
                out.push_str(&format!("<span class=\"tag\">{}</span>", escape(tag)));
            }
        }
        _ => {}
    }
    if word["case_sensitive"].as_bool() == Some(true) {
        out.push_str(&format!(
            " <span class=\"case-sensitive\">{}</span>",
            escape(&t(cx.i18n, "admin.watched_words.case_sensitive"))
        ));
    }
    if word["html"].as_bool() == Some(true) {
        out.push_str(&format!(
            " <span class=\"html\">{}</span>",
            escape(&t(cx.i18n, "admin.watched_words.html"))
        ));
    }
    out.push_str("</div>");
    out
}

/// An empty creatable MultiSelect (WatchedWords; the TagChooser with
/// allowAny).
fn multi_select(id: &str, class: &str, none: &str, attrs: &str) -> String {
    crate::admin_customize_kit::chooser(
        Some(id),
        class,
        none,
        &[],
        None,
        &format!(" data-creatable=\"\"{attrs}"),
    )
}

/// WatchedWordForm for the action.
fn watched_word_form(cx: &AdminCx, key: &str, regex: bool) -> String {
    let tr = |k: &str| escape(&t(cx.i18n, k));
    let placeholder = if regex {
        "admin.watched_words.form.placeholder_regexp"
    } else {
        "admin.watched_words.form.placeholder"
    };
    let mut out = format!(
        "<form class=\"watched-word-form\"><div class=\"watched-word-input\"><label for=\"watched-word\">{}</label> {}</div>",
        tr("admin.watched_words.form.label"),
        multi_select(
            "watched-words",
            "watched-word-input-field",
            &t(cx.i18n, "admin.watched_words.form.words_or_phrases"),
            &format!(" data-placeholder=\"{}\"", tr(placeholder))
        )
    );
    let text = |id: &str, label: &str, placeholder: &str| {
        format!(
            "<div class=\"watched-word-input\"><label for=\"{id}\">{}</label> <input autocapitalize=\"off\" autocorrect=\"off\" class=\"ember-text-field watched-word-input-field\" id=\"{id}\" placeholder=\"{}\" type=\"text\"></div>",
            tr(label),
            tr(placeholder)
        )
    };
    match key {
        "replace" => out.push_str(&text(
            "watched-replacement",
            "admin.watched_words.form.replace_label",
            "admin.watched_words.form.replace_placeholder",
        )),
        "tag" => out.push_str(&format!(
            "<div class=\"watched-word-input\"><label for=\"watched-tag\">{}</label> {}</div>",
            tr("admin.watched_words.form.tag_label"),
            multi_select(
                "watched-tag",
                "tag-chooser watched-word-input-field",
                &t(cx.i18n, "select_kit.default_header_text"),
                &format!(
                    " data-search=\"tags\" data-placeholder=\"{}\"",
                    tr("select_kit.filter_placeholder")
                )
            )
        )),
        "link" => out.push_str(&text(
            "watched-link",
            "admin.watched_words.form.link_label",
            "admin.watched_words.form.link_placeholder",
        )),
        _ => {}
    }
    let checkbox = |id: &str, class: &str, label: &str, description: &str| {
        format!(
            "<div class=\"watched-word-input\"><label for=\"{id}\">{}</label> <label class=\"{class} checkbox-label\"><input class=\"ember-checkbox\" type=\"checkbox\"> {}</label></div>",
            tr(label),
            tr(description)
        )
    };
    out.push_str(&checkbox(
        "watched-case-sensitivity",
        "case-sensitivity-checkbox",
        "admin.watched_words.form.case_sensitivity_label",
        "admin.watched_words.form.case_sensitivity_description",
    ));
    if key == "replace" {
        out.push_str(&checkbox(
            "watched-html",
            "html-checkbox",
            "admin.watched_words.form.html_label",
            "admin.watched_words.form.html_description",
        ));
    }
    out.push_str(&format!(
        "<button class=\"btn btn-primary\" disabled=\"\" type=\"submit\"><span class=\"d-button-label\">{}</span></button></form>",
        tr("admin.watched_words.form.add")
    ));
    out
}
