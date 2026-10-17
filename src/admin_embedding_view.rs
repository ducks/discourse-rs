//! The embedding pages (adminEmbedding, 90g7) as Ember draws them, from
//! the embedding's document (/admin/customize/embedding.json).

use serde_json::Value;

use crate::admin_customize_kit::{
    area_card, category_chooser, checkbox_field, chooser, custom_field, input_field,
};
use crate::admin_view::{AdminCx, Crumb, PageHeader, t, t_with};
use crate::topic_list_view::{ListContext, escape, icon};

/// The page's tab.
#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Hosts,
    Settings,
    PostsAndTopics,
    Crawlers,
}

/// templates/admin-embedding.gjs: the header with its tabs (none on the
/// forms, whose routes leave DPageHeader headless) over the outlet.
pub fn page(cx: &AdminCx, tab: Option<Tab>, outlet: &str) -> String {
    let header = match tab {
        None => String::new(),
        Some(tab) => header(cx, tab),
    };
    format!(
        "<div class=\"admin-embedding admin-config-page\">{header}<div class=\"admin-container admin-config-page__main-area\">{outlet}</div></div>"
    )
}

fn header(cx: &AdminCx, tab: Tab) -> String {
    let title = t(cx.i18n, "admin.config.embedding.title");
    let actions = crate::admin_ui::actions(
        cx,
        "d-page-header",
        &[crate::admin_ui::Action::Primary(crate::admin_ui::Button {
            class: "admin-embedding__header-add-host",
            label: crate::admin_ui::tr(cx, "admin.embedding.add_host"),
            title: crate::admin_ui::tr(cx, "admin.embedding.add_host"),
            href: Some("/admin/customize/embedding/new"),
            ..Default::default()
        })],
    );
    let tabs = crate::admin_ui::tabs(
        cx,
        &[
            (
                "admin-embedding-tabs__settings",
                "/admin/customize/embedding/settings",
                "admin.embedding.nav.settings",
                tab == Tab::Settings,
            ),
            (
                "admin-embedding-tabs__hosts",
                "/admin/customize/embedding",
                "admin.embedding.nav.hosts",
                tab == Tab::Hosts,
            ),
            (
                "admin-embedding-tabs__posts-and-topics",
                "/admin/customize/embedding/posts_and_topics",
                "admin.embedding.nav.posts_and_topics",
                tab == Tab::PostsAndTopics,
            ),
            (
                "admin-embedding-tabs__crawlers",
                "/admin/customize/embedding/crawlers",
                "admin.embedding.nav.crawlers",
                tab == Tab::Crawlers,
            ),
        ],
    );
    PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path: "/admin/customize/embedding",
            },
        ],
        title,
        actions,
        description: Some(escape(&t(
            cx.i18n,
            "admin.config.embedding.header_description",
        ))),
        learn_more_url: Some(
            "https://meta.discourse.org/t/embed-discourse-comments-on-another-website-via-javascript/31963"
                .into(),
        ),
        tabs: Some(tabs),
    }
    .render(cx)
}

/// The hosts tab (templates/admin-embedding/index.gjs): the full app
/// toggle, the snippet's card (the desktop view's, collapsed), the hosts'
/// table; or the empty list.
pub fn hosts(
    cx: &AdminCx,
    list: &ListContext,
    doc: &Value,
    full_app: bool,
    uncategorized: i64,
) -> String {
    let hosts = doc["embeddable_hosts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if hosts.is_empty() {
        return crate::admin_ui::empty_list(
            cx,
            "admin-embedding__add-host",
            "admin.embedding.add_host",
            "/admin/customize/embedding/new",
            "admin.embedding.get_started",
        );
    }
    let tr = |k: &str| escape(&t(cx.i18n, k));
    let mut out = format!(
        "<div class=\"admin-embedding-index__full-app-toggle\"><div class=\"d-toggle-switch\"><label class=\"d-toggle-switch__label\"><button aria-checked=\"{full_app}\" aria-label=\"{}\" class=\"d-toggle-switch__checkbox\" role=\"switch\" type=\"button\"></button><span class=\"d-toggle-switch__checkbox-slider\">{}</span></label></div><div class=\"admin-embedding-index__full-app-toggle-text\"><span class=\"admin-embedding-index__full-app-toggle-label\">{}</span><span class=\"admin-embedding-index__full-app-toggle-description\">{}</span></div></div>",
        tr("admin.embedding.full_app_mode"),
        if full_app {
            icon("check", None)
        } else {
            String::new()
        },
        tr("admin.embedding.full_app_mode"),
        tr("admin.embedding.full_app_mode_description")
    );
    if !crate::mobile::mobile_view() {
        out.push_str(&area_card(
            "admin-embedding-index__code",
            &t(cx.i18n, "admin.embedding.configuration_snippet"),
            None,
            Some(&t(cx.i18n, "sidebar.toggle_section")),
        ));
    }
    let author = match doc["embedding"]["embed_by_username"]
        .as_str()
        .filter(|s| !s.is_empty())
    {
        Some(name) => escape(&t_with(
            cx.i18n,
            "admin.embedding.post_author_with_default",
            &[("author", name)],
        )),
        None => tr("admin.embedding.post_author"),
    };
    out.push_str(&format!(
        "<table class=\"d-table\"><thead class=\"d-table__header\"><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\">{author}</th></thead><tbody class=\"d-table__body\">",
        tr("admin.embedding.host"),
        tr("admin.embedding.allowed_paths"),
        tr("admin.embedding.category"),
        tr("admin.embedding.tags")
    ));
    let menu = crate::admin_ui::menu(
        "",
        "embedding-host-menu",
        "ellipsis-vertical",
        &t(cx.i18n, "admin.embedding.more_options.title"),
        &format!(
            "<div class=\"fk-d-menu__inner-content\"><ul class=\"dropdown-menu\"><li class=\"dropdown-menu__item\">{}</li></ul></div>",
            crate::admin_ui::Button {
                class: "btn-transparent --danger admin-embeddable-host-item__delete",
                icon: Some("trash-can"),
                label: crate::admin_ui::tr(cx, "admin.embedding.delete"),
                ..Default::default()
            }
            .render()
        ),
    );
    let label = |k: &str| format!("<div class=\"d-table__mobile-label\">{k}</div>");
    for host in &hosts {
        let s = |v: &Value| escape(v.as_str().unwrap_or_default());
        // Category.findById(category_id || uncategorized): none drawn for
        // an id the site doesn't have.
        let category = host["category_id"]
            .as_i64()
            .filter(|id| *id != 0)
            .or(Some(uncategorized))
            .and_then(|id| list.categories.get(&id))
            .map(|c| crate::topic_list_view::category_badge_html(list, c, false, false))
            .unwrap_or_default();
        let tags: Vec<&str> = host["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t["name"].as_str())
            .collect();
        let edit = format!("/admin/customize/embedding/{}", host["id"]);
        out.push_str(&format!(
            "<tr class=\"d-table__row\" data-host-id=\"{}\"><td class=\"d-table__cell --overview\">{}</td><td class=\"d-table__cell --detail\">{}{}</td><td class=\"d-table__cell --detail\">{}{category}</td><td class=\"d-table__cell --detail\">{}{}</td><td class=\"d-table__cell --detail\">{}{}</td><td class=\"d-table__cell --controls\"><div class=\"d-table__cell-actions\">{}{menu}</div></td></tr>",
            host["id"],
            s(&host["host"]),
            label(&tr("admin.embedding.allowed_paths")),
            s(&host["allowed_paths"]),
            label(&tr("admin.embedding.category")),
            label(&tr("admin.embedding.tags")),
            escape(&tags.join(", ")),
            label(&author),
            s(&host["user"]),
            crate::admin_ui::Button {
                class: "btn-default btn-small admin-embeddable-host-item__edit",
                label: crate::admin_ui::tr(cx, "admin.embedding.edit"),
                href: Some(&edit),
                ..Default::default()
            }
            .render()
        ));
    }
    out.push_str("</tbody></table>");
    out
}

/// The users' chooser attributes (UserChooser, maximum one).
fn user_search(cx: &AdminCx) -> String {
    format!(
        " data-search=\"users\" data-maximum=\"1\" data-placeholder=\"{}\"",
        escape(&t(cx.i18n, "select_kit.filter_placeholder"))
    )
}

/// AdminEmbeddingHostForm for a new host, or the host given.
pub fn host_form(cx: &AdminCx, list: &ListContext, host: Option<&Value>) -> String {
    let tr = |k: &str| t(cx.i18n, k);
    let optional = tr("form_kit.optional");
    let s = |k: &str| {
        host.and_then(|h| h[k].as_str())
            .unwrap_or_default()
            .to_string()
    };
    let category = host
        .and_then(|h| h["category_id"].as_i64())
        .and_then(|id| list.categories.get(&id));
    let tags: Vec<String> = host
        .and_then(|h| h["tags"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect();
    // The tags' ids by name, which the save sends.
    let tag_ids: serde_json::Map<String, Value> = host
        .and_then(|h| h["tags"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|t| Some((t["name"].as_str()?.to_string(), t["id"].clone())))
        .collect();
    let tag_ids = Value::Object(tag_ids);
    let user: Vec<String> = host
        .and_then(|h| h["user"].as_str())
        .filter(|u| !u.is_empty())
        .map(|u| vec![u.to_string()])
        .unwrap_or_default();
    let none = tr("select_kit.default_header_text");
    let fields = [
        crate::admin_form_kit::Field {
            name: "host",
            control: "input",
            title: Some(tr("admin.embedding.host")),
            format: Some("large"),
            inner: crate::admin_form_kit::input(
                "host",
                &s("host"),
                "text",
                Some("example.com"),
                false,
            ),
            ..Default::default()
        }
        .render(),
        input_field(
            "allowed_paths",
            &tr("admin.embedding.allowed_paths"),
            Some(&optional),
            &s("allowed_paths"),
            Some("/blog/.*"),
            Some("large"),
        ),
        custom_field(
            "category",
            &tr("admin.embedding.category"),
            Some(&optional),
            None,
            &category_chooser(cx, list, "admin-embedding-host-form__category", category),
        ),
        custom_field(
            "tags",
            &tr("admin.embedding.tags"),
            Some(&optional),
            None,
            &chooser(
                None,
                "tag-chooser admin-embedding-host-form__tags",
                &none,
                &tags,
                None,
                &format!(
                    " data-search=\"tags\" data-tag-ids=\"{}\" data-placeholder=\"{}\"",
                    escape(&tag_ids.to_string()),
                    escape(&tr("category.tags_placeholder"))
                ),
            ),
        ),
        custom_field(
            "user",
            &tr("admin.embedding.post_author"),
            Some(&optional),
            Some(&tr("admin.embedding.post_author_description")),
            &chooser(
                None,
                "user-chooser admin-embedding-host-form__post_author",
                &none,
                &user,
                Some(1),
                &user_search(cx),
            ),
        ),
        crate::admin_form_kit::submit("", &tr("admin.embedding.host_form.save")),
    ];
    let heading = tr(if host.is_some() {
        "admin.embedding.host_form.edit_header"
    } else {
        "admin.embedding.host_form.add_header"
    });
    let form = format!(
        "<form class=\"form-kit\" novalidate=\"\" data-host-form=\"{}\">{}</form>",
        host.map(|h| h["id"].to_string()).unwrap_or_default(),
        fields.concat()
    );
    format!(
        "{}<div class=\"admin-config-area\"><div class=\"admin-config-area__primary-content admin-embedding-host-form\">{}</div></div>",
        crate::admin_ui::back_button(cx, "/admin/customize/embedding", "admin.embedding.back")
            .replacen("class=\"btn", "class=\"active btn", 1),
        area_card("", &heading, Some(&form), None)
    )
}

/// The posts and topics tab's form.
pub fn posts_and_topics(cx: &AdminCx, embedding: &Value) -> String {
    let tr = |k: &str| t(cx.i18n, k);
    let optional = tr("form_kit.optional");
    let user: Vec<String> = embedding["embed_by_username"]
        .as_str()
        .filter(|u| !u.is_empty())
        .map(|u| vec![u.to_string()])
        .unwrap_or_default();
    let limit = match &embedding["embed_post_limit"] {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    format!(
        "{}<form class=\"form-kit\" novalidate=\"\" data-embedding-type=\"posts_and_topics\">{}{}{}<fieldset class=\"form-kit__fieldset form-kit__checkbox-group\">{}{}</fieldset>{}</form>",
        crate::admin_ui::subheader(&tr("admin.embedding.posts_and_topics"), "", None),
        custom_field(
            "embed_by_username",
            &tr("admin.embedding.embed_by_username"),
            None,
            None,
            &chooser(
                None,
                "user-chooser admin-embedding-posts-and-topics-form__embed_by_username",
                &tr("select_kit.default_header_text"),
                &user,
                Some(1),
                &user_search(cx),
            ),
        ),
        input_field(
            "embed_post_limit",
            &tr("admin.embedding.embed_post_limit"),
            Some(&optional),
            &limit,
            None,
            Some("large"),
        ),
        input_field(
            "embed_title_scrubber",
            &tr("admin.embedding.embed_title_scrubber"),
            Some(&optional),
            embedding["embed_title_scrubber"]
                .as_str()
                .unwrap_or_default(),
            Some("- site.com$"),
            Some("large"),
        ),
        checkbox_field(
            "embed_truncate",
            &tr("admin.embedding.embed_truncate"),
            embedding["embed_truncate"].as_bool() == Some(true)
        ),
        checkbox_field(
            "embed_unlisted",
            &tr("admin.embedding.embed_unlisted"),
            embedding["embed_unlisted"].as_bool() == Some(true)
        ),
        crate::admin_form_kit::submit("", &tr("admin.embedding.save")),
    )
}

/// The crawlers tab's form.
pub fn crawlers(cx: &AdminCx, embedding: &Value) -> String {
    let tr = |k: &str| t(cx.i18n, k);
    let optional = tr("form_kit.optional");
    let field = |name: &str, placeholder: &str| {
        input_field(
            name,
            &tr(&format!("admin.embedding.{name}")),
            Some(&optional),
            embedding[name].as_str().unwrap_or_default(),
            Some(placeholder),
            Some("large"),
        )
    };
    format!(
        "{}<form class=\"form-kit\" novalidate=\"\" data-embedding-type=\"crawlers\">{}{}{}{}</form>",
        crate::admin_ui::subheader(
            &tr("admin.embedding.crawlers"),
            "",
            Some(&escape(&tr("admin.embedding.crawlers_description")))
        ),
        field("allowed_embed_selectors", "article, #story, .post"),
        field("blocked_embed_selectors", ".ad-unit, header"),
        field("allowed_embed_classnames", "emoji, classname"),
        crate::admin_form_kit::submit("", &tr("admin.embedding.save")),
    )
}
