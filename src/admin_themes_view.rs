//! The themes and components pages (adminConfig.customize, 90g7) as
//! Ember draws them: the themes' grid, the components' list and the theme
//! site settings, under the area's header.

use serde_json::Value;

use crate::admin_view::{AdminCx, Crumb, PageHeader, t, t_with};
use crate::topic_list_view::{escape, icon};

/// The area's tab.
#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Themes,
    Components,
    ThemeSiteSettings,
}

/// templates/admin-config/customize.gjs: the header (the tab's crumb
/// after the area's) over the outlet.
pub fn page(cx: &AdminCx, tab: Tab, outlet: &str) -> String {
    let (crumb, path) = match tab {
        Tab::Themes => (
            "admin.config_areas.themes_and_components.themes.title",
            "/admin/config/customize/themes",
        ),
        Tab::Components => (
            "admin.config_areas.themes_and_components.components.title",
            "/admin/config/customize/components",
        ),
        Tab::ThemeSiteSettings => (
            "admin.config_areas.themes_and_components.theme_site_settings.title",
            "/admin/config/customize/theme-site-settings",
        ),
    };
    let header = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: t(cx.i18n, crumb),
                path,
            },
        ],
        title: t(cx.i18n, "admin.config.themes_and_components.title"),
        actions: String::new(),
        description: Some(escape(&t(
            cx.i18n,
            "admin.config.themes_and_components.header_description",
        ))),
        learn_more_url: Some(
            "https://meta.discourse.org/t/beginners-guide-to-using-discourse-themes/91966".into(),
        ),
        tabs: Some(crate::admin_ui::tabs(
            cx,
            &[
                (
                    "",
                    "/admin/config/customize/themes",
                    "admin.config.themes.title",
                    tab == Tab::Themes,
                ),
                (
                    "",
                    "/admin/config/customize/components",
                    "admin.config.components.title",
                    tab == Tab::Components,
                ),
                (
                    "",
                    "/admin/config/customize/theme-site-settings",
                    "admin.config.theme_site_settings.title",
                    tab == Tab::ThemeSiteSettings,
                ),
            ],
        )),
    }
    .render(cx);
    format!("{header}<div class=\"admin-container admin-config-page__main-area\">{outlet}</div>")
}

fn install_action(cx: &AdminCx, class: &str, label: &str) -> String {
    crate::admin_ui::actions(
        cx,
        "d-page-subheader",
        &[crate::admin_ui::Action::Primary(crate::admin_ui::Button {
            class,
            label: crate::admin_ui::tr(cx, label),
            ..Default::default()
        })],
    )
}

/// Ember's `dasherize` for the card's class.
fn dasherize(name: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if c.is_uppercase() {
            if prev_lower {
                out.push('-');
            }
            out.extend(c.to_lowercase());
            prev_lower = false;
        } else if c == ' ' || c == '_' {
            out.push('-');
            prev_lower = false;
        } else {
            out.push(c);
            prev_lower = c.is_lowercase() || c.is_ascii_digit();
        }
    }
    out
}

const PLACEHOLDERS: &str = include_str!("../static/admin/themes-grid-placeholder.svg.txt");

/// ThemesGridPlaceholder: one of its four drawings in the theme's colors
/// (its palette's, else the site's variables). Ember picks the drawing at
/// random; the theme's id picks it here.
fn placeholder(theme: &Value) -> String {
    let id = theme["id"].as_i64().unwrap_or_default();
    let variant = id.rem_euclid(4) as usize;
    let colors = theme["color_scheme"]["colors"].as_array();
    let color = |index: usize, name: &str| match colors {
        Some(c) => format!(
            "#{}",
            c.get(index)
                .and_then(|x| x["hex"].as_str())
                .unwrap_or_default()
        ),
        None => format!("var(--{name})"),
    };
    let mut svg = PLACEHOLDERS
        .lines()
        .nth(variant)
        .unwrap_or_default()
        .replace("%GRADIENT%", &format!("bgGradient-{id}-{variant}"));
    for (index, name) in [
        (0, "primary"),
        (1, "secondary"),
        (2, "tertiary"),
        (3, "quaternary"),
        (6, "highlight"),
        (7, "danger"),
        (8, "success"),
        (9, "love"),
    ] {
        svg = svg.replace(&format!("%{name}%"), &escape(&color(index, name)));
    }
    svg
}

/// ThemesGridCard.
fn theme_card(cx: &AdminCx, theme: &Value) -> String {
    let tr = |k: &str| escape(&t(cx.i18n, k));
    let name = theme["name"].as_str().unwrap_or_default();
    let id = &theme["id"];
    let default = theme["default"].as_bool() == Some(true);
    let system = theme["system"].as_bool() == Some(true);
    let light = theme["screenshot_light_url"]
        .as_str()
        .filter(|s| !s.is_empty());
    let dark = theme["screenshot_dark_url"]
        .as_str()
        .filter(|s| !s.is_empty());
    let image = match light.or(dark) {
        Some(src) => {
            let toggle = if light.is_some() && dark.is_some() {
                let label = t(
                    cx.i18n,
                    "admin_onboarding_banner.select_theme.show_dark_screenshot",
                );
                crate::admin_ui::Button {
                    class: "btn-flat theme-card-preview__screenshot-toggle",
                    icon: Some("moon"),
                    title: Some(label.clone()),
                    attrs: format!(" aria-label=\"{}\"", escape(&label)),
                    ..Default::default()
                }
                .render()
            } else {
                String::new()
            };
            format!(
                "<img alt=\"{}\" class=\"theme-card-preview__image\" src=\"{}\" data-light=\"{}\" data-dark=\"{}\">{toggle}",
                escape(name),
                escape(src),
                escape(light.unwrap_or_default()),
                escape(dark.unwrap_or_default())
            )
        }
        None => placeholder(theme),
    };
    let description = theme["description"]
        .as_str()
        .filter(|d| !d.is_empty())
        .map(|d| {
            format!(
                "<p class=\"theme-card-preview__description\">{}</p>",
                escape(d)
            )
        })
        .unwrap_or_default();
    let pending = theme["remote_theme"]["commits_behind"]
        .as_i64()
        .unwrap_or(0)
        > 0;
    let mut badges = String::new();
    if pending {
        badges.push_str(&format!(
            "<span class=\"theme-card__badge\" title=\"{}\">{} {}</span>",
            tr("admin.customize.theme.updates_available_tooltip"),
            icon("arrows-rotate", None),
            tr("admin.customize.theme.update_available")
        ));
    }
    if default {
        badges.push_str(&format!(
            "<span class=\"theme-card__badge --default\" title=\"{}\">{} {}</span>",
            tr("admin.customize.theme.default_theme"),
            icon("paintbrush", None),
            tr("admin.customize.theme.default")
        ));
    }
    let selectable = theme["user_selectable"].as_bool() == Some(true);
    if selectable {
        badges.push_str(&format!(
            "<span class=\"theme-card__badge --selectable\" title=\"{}\">{} {}</span>",
            tr("admin.customize.theme.user_selectable"),
            icon("user-check", None),
            tr("admin.customize.theme.user_selectable_badge_label")
        ));
    }
    let edit_path = format!("/admin/customize/themes/{id}");
    let item = |b: crate::admin_ui::Button| {
        format!("<li class=\"dropdown-menu__item\">{}</li>", b.render())
    };
    let mut items = item(crate::admin_ui::Button {
        class: "theme-card__button set-default",
        icon: Some(if default { "star" } else { "far-star" }),
        label: crate::admin_ui::tr(
            cx,
            if default {
                "admin.customize.theme.default_theme"
            } else {
                "admin.customize.theme.set_default_theme"
            },
        ),
        disabled: default,
        ..Default::default()
    });
    if pending {
        items.push_str(&item(crate::admin_ui::Button {
            class: "theme-card__button update",
            icon: Some("cloud-arrow-down"),
            label: crate::admin_ui::tr(cx, "admin.customize.theme.update_to_latest"),
            ..Default::default()
        }));
    }
    items.push_str(&item(crate::admin_ui::Button {
        class: "theme-card__button set-selectable",
        icon: Some(if selectable {
            "user-xmark"
        } else {
            "user-check"
        }),
        label: crate::admin_ui::tr(
            cx,
            if selectable {
                "admin.customize.theme.user_selectable_unavailable_button_label"
            } else {
                "admin.customize.theme.user_selectable_button_label"
            },
        ),
        ..Default::default()
    }));
    items.push_str(&format!(
        "<li class=\"dropdown-menu__item\"><a class=\"btn btn-transparent theme-card__button preview\" href=\"{}/admin/themes/{id}/preview\" rel=\"noopener noreferrer\" target=\"_blank\" title=\"{}\">{} {}</a></li>",
        cx.base_path,
        tr("admin.customize.explain_preview"),
        icon("eye", None),
        tr("admin.customize.theme.preview")
    ));
    items.push_str(&item(crate::admin_ui::Button {
        class: "theme-card__button btn-transparent --danger delete",
        icon: Some("trash-can"),
        label: crate::admin_ui::tr(cx, "admin.customize.delete"),
        disabled: default || system,
        ..Default::default()
    }));
    let menu = crate::admin_ui::menu(
        "theme-card__footer-menu btn-flat",
        "theme-card__footer-menu",
        "ellipsis",
        "",
        &format!(
            "<div class=\"fk-d-menu__inner-content\"><ul class=\"dropdown-menu\">{items}</ul></div>"
        ),
    )
    .replacen(" title=\"\"", "", 1);
    format!(
        "<section class=\"admin-config-area-card theme-card{} {}\" data-theme-id=\"{id}\" data-default=\"{default}\" data-user-selectable=\"{selectable}\"><div class=\"admin-config-area-card__content\"><div class=\"theme-card-preview__image-wrapper\">{image}</div><div class=\"theme-card-preview__content\"><a class=\"theme-card__title\" href=\"{}{edit_path}\">{}</a>{description}</div><div class=\"theme-card__footer\"><div class=\"theme-card__badges\">{badges}</div><div class=\"theme-card__controls\">{}<div class=\"theme-card__footer-actions\">{menu}</div></div></div></div></section>",
        if default { " --default" } else { "" },
        escape(&dasherize(name)),
        cx.base_path,
        escape(name),
        crate::admin_ui::Button {
            class: "btn-default theme-card__button edit",
            label: crate::admin_ui::tr(cx, "admin.customize.theme.edit"),
            href: Some(&edit_path),
            ..Default::default()
        }
        .render()
    )
}

/// The themes tab (AdminConfigAreasThemes, ThemesGrid): the themes, the
/// default first, then the system ones; the install more card while only
/// the two system themes are there; the design wizard.
pub fn themes(cx: &AdminCx, doc: &Value, can_run_design_wizard: bool) -> String {
    let description = t_with(
        cx.i18n,
        "admin.config_areas.themes_and_components.themes.description",
        &[(
            "themeSiteSettingsUrl",
            &format!(
                "{}/admin/config/customize/theme-site-settings",
                cx.base_path
            ),
        )],
    );
    let mut out = crate::admin_ui::subheader(
        &t(
            cx.i18n,
            "admin.config_areas.themes_and_components.themes.title",
        ),
        &install_action(
            cx,
            "install-theme",
            "admin.config_areas.themes_and_components.themes.install",
        ),
        Some(&description),
    );
    let list: Vec<&Value> = doc["themes"].as_array().into_iter().flatten().collect();
    // ThemesGrid's sortedThemes (toSorted with its comparator).
    let mut sorted = list.clone();
    sorted.sort_by(|a, b| {
        use std::cmp::Ordering;
        let def = |x: &Value| x["default"].as_bool() == Some(true);
        let id = |x: &Value| x["id"].as_i64().unwrap_or_default();
        if def(a) {
            return Ordering::Less;
        }
        if def(b) {
            return Ordering::Greater;
        }
        if id(a) < 0 {
            return Ordering::Less;
        }
        if id(b) < 0 {
            return Ordering::Greater;
        }
        Ordering::Equal
    });
    let cards: String = sorted.iter().map(|th| theme_card(cx, th)).collect();
    let install_more = if list.len() == 2
        && list.iter().all(|th| th["system"].as_bool() == Some(true))
    {
        format!(
            "<section class=\"admin-config-area-card theme-card --install-more\"><div class=\"admin-config-area-card__content\"><div class=\"theme-card__install-more-icon\">{}</div><div class=\"theme-card__content\"><h3 class=\"theme-card__title\">{}</h3><p class=\"theme-card__description\">{}</p></div><div class=\"theme-card__footer\">{}</div></div></section>",
            icon("plus", None),
            escape(&t(cx.i18n, "admin.customize.theme.install_more_themes")),
            escape(&t(
                cx.i18n,
                "admin.customize.theme.install_more_themes_description"
            )),
            crate::admin_ui::Button {
                class: "btn-primary theme-card__install-button",
                icon: Some("upload"),
                label: crate::admin_ui::tr(
                    cx,
                    "admin.config_areas.themes_and_components.themes.install"
                ),
                ..Default::default()
            }
            .render()
        )
    } else {
        String::new()
    };
    out.push_str(&format!(
        "<div class=\"themes-cards-container\">{cards}{install_more}</div>"
    ));
    if can_run_design_wizard {
        out.push_str(&format!(
            "<div class=\"admin-config-area-themes__design-wizard\">{}<span class=\"admin-config-area-themes__design-wizard-description\">{}</span></div>",
            crate::admin_ui::Button {
                class: "btn-default admin-config-area-themes__design-wizard-button",
                icon: Some("wand-magic"),
                label: crate::admin_ui::tr(cx, "design_wizard.launch"),
                ..Default::default()
            }
            .render(),
            escape(&t(cx.i18n, "design_wizard.launch_description"))
        ));
    }
    out
}

/// The components' parent themes, as parentThemesCell names them.
fn parent_themes(cx: &AdminCx, component: &Value) -> Option<String> {
    let mut names: Vec<String> = component["parent_themes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|th| th["name"].as_str().map(str::to_string))
        .collect();
    names.sort();
    let key = "admin.config_areas.themes_and_components.components.";
    match names.len() {
        0 => None,
        1 => Some(names[0].clone()),
        2 => Some(t_with(
            cx.i18n,
            &format!("{key}parent_themes_two"),
            &[("name1", &names[0]), ("name2", &names[1])],
        )),
        3 => Some(t_with(
            cx.i18n,
            &format!("{key}parent_themes_three"),
            &[
                ("name1", &names[0]),
                ("name2", &names[1]),
                ("name3", &names[2]),
            ],
        )),
        n => Some(crate::admin_view::t_count(
            cx.i18n,
            &format!("{key}parent_themes_more_than_three"),
            (n - 3) as i64,
            &[
                ("name1", &names[0]),
                ("name2", &names[1]),
                ("name3", &names[2]),
            ],
        )),
    }
}

/// The components tab (AdminConfigAreasComponents): DFilterControls over
/// the table while there are components (or a filter), else the empty
/// list.
pub fn components(cx: &AdminCx, doc: &Value, name: &str, status: &str, any: bool) -> String {
    let key = |k: &str| format!("admin.config_areas.themes_and_components.components.{k}");
    let tr = |k: &str| escape(&t(cx.i18n, &key(k)));
    let mut out = crate::admin_ui::subheader(
        &t(cx.i18n, &key("title")),
        &install_action(cx, "install-component", &key("install")),
        Some(&tr("description")),
    );
    out.push_str("<div class=\"container\">");
    let list: Vec<&Value> = doc["components"].as_array().into_iter().flatten().collect();
    if any {
        let options = [
            ("all", "filter_by_all"),
            ("used", "filter_by_used"),
            ("unused", "filter_by_unused"),
            ("updates_available", "filter_by_updates_available"),
        ];
        let status = if options.iter().any(|(v, _)| *v == status) {
            status
        } else {
            "all"
        };
        let select: String = options
            .iter()
            .map(|(v, k)| {
                format!(
                    "<option class=\"d-native-select__option{}\"{} value=\"{v}\">{}</option>",
                    if *v == status { " --selected" } else { "" },
                    if *v == status && status != "all" {
                        " selected=\"\""
                    } else {
                        ""
                    },
                    tr(k)
                )
            })
            .collect();
        let label = options
            .iter()
            .find(|(v, _)| *v == status)
            .map(|(_, k)| t(cx.i18n, &key(k)))
            .unwrap_or_default();
        let active = !name.is_empty() || status != "all";
        out.push_str(&format!(
            "<div class=\"d-filter-controls\"><div class=\"d-filter-controls__inputs\"><div class=\"filter-input-container\">{}<input autocapitalize=\"none\" autocomplete=\"off\" autocorrect=\"off\" class=\"filter-input d-filter-controls__input\" placeholder=\"{}\" type=\"text\" value=\"{}\"></div></div><div class=\"d-filter-controls__dropdowns\"><select aria-label=\"{}\" class=\"d-filter-controls__dropdown d-native-select{}\">{select}</select></div>{}</div>",
            icon("magnifying-glass", Some("-left")),
            tr("search_components"),
            escape(name),
            escape(&label),
            if status != "all" { " --active" } else { "" },
            crate::admin_ui::Button {
                class: "btn-default d-filter-controls__reset",
                icon: Some("arrow-rotate-left"),
                title: crate::admin_ui::tr(cx, "filter_controls.reset"),
                disabled: !active,
                attrs: format!(" aria-label=\"{}\"", escape(&t(cx.i18n, "filter_controls.reset"))),
                ..Default::default()
            }
            .render()
        ));
        if list.is_empty() {
            out.push_str(&format!(
                "<div class=\"d-filter-controls__no-results\"><p>{}</p>{}</div>",
                tr("no_components_found"),
                crate::admin_ui::Button {
                    class: "btn-default d-filter-controls__reset-button",
                    label: crate::admin_ui::tr(cx, "filter_controls.reset"),
                    ..Default::default()
                }
                .render()
            ));
        } else {
            out.push_str(&format!(
                "<div data-components-list=\"\"><table class=\"d-table component-list\"><thead class=\"d-table__header\"><tr class=\"d-table__row\"><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\"></th></tr></thead><tbody class=\"d-table__body\">{}</tbody></table><div class=\"loading-container\"></div><div aria-hidden=\"true\" class=\"load-more-sentinel\"{}></div></div>",
                tr("name"),
                tr("used_on"),
                tr("enabled"),
                list.iter().map(|c| component_row(cx, c)).collect::<String>(),
                if doc["has_more"].as_bool() == Some(true) {
                    " data-has-more=\"\""
                } else {
                    ""
                }
            ));
        }
    }
    out.push_str("<div class=\"loading-container\">");
    if !any {
        out.push_str(&format!(
            "<div class=\"admin-config-area-empty-list\"><span class=\"admin-config-area-empty-list__title\">{}</span></div>",
            tr("no_components")
        ));
    }
    out.push_str("</div></div>");
    out
}

/// The components' rows, alone (the next page's, for the script).
pub fn component_rows(cx: &AdminCx, doc: &Value) -> String {
    doc["components"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| component_row(cx, c))
        .collect()
}

/// ComponentRow.
fn component_row(cx: &AdminCx, component: &Value) -> String {
    let key = |k: &str| format!("admin.config_areas.themes_and_components.components.{k}");
    let tr = |k: &str| escape(&t(cx.i18n, &key(k)));
    let id = &component["id"];
    let remote = &component["remote_theme"];
    let has_updates = remote["commits_behind"].as_i64().unwrap_or(0) > 0;
    let mut overview = format!(
        "<a class=\"d-table__overview-link\" href=\"{}/admin/customize/themes/{id}\"><div class=\"d-table__overview-name\">{}</div></a>",
        cx.base_path,
        escape(component["name"].as_str().unwrap_or_default())
    );
    if let Some(authors) = remote["authors"].as_str().filter(|a| !a.is_empty()) {
        overview.push_str(&format!(
            "<div class=\"d-table__overview-author admin-config-components__author-name\">{}</div>",
            escape(&t_with(cx.i18n, &key("by_author"), &[("name", authors)]))
        ));
    }
    if let Some(description) = component["description"].as_str().filter(|d| !d.is_empty()) {
        let about = remote["about_url"]
            .as_str()
            .map(|u| {
                format!(
                    "<a href=\"{}\">{} {}</a>",
                    escape(u),
                    tr("learn_more"),
                    icon("up-right-from-square", None)
                )
            })
            .unwrap_or_default();
        overview.push_str(&format!(
            "<div class=\"d-table__overview-about admin-config-components__description\">{} {about}</div>",
            escape(description)
        ));
    }
    if has_updates {
        overview.push_str(&format!(
            "<div class=\"d-table__overview-about admin-config-components__update-available\">{}</div>",
            tr("update_available")
        ));
    }
    let used = match parent_themes(cx, component) {
        Some(names) => escape(&names),
        None => format!(
            "<div class=\"status-label --inactive\"><div class=\"status-label-indicator\"></div><div class=\"status-label-text\">{}</div></div>",
            tr("badge_unused")
        ),
    };
    let enabled = component["enabled"].as_bool() == Some(true);
    let item = |b: crate::admin_ui::Button| {
        format!("<li class=\"dropdown-menu__item\">{}</li>", b.render())
    };
    let mut items = format!(
        "<li class=\"dropdown-menu__item\"><a class=\"btn btn-icon-text btn-transparent admin-config-components__preview\" href=\"{}/admin/themes/{id}/preview\" rel=\"noopener noreferrer\" target=\"_blank\">{}<span class=\"d-button-label\">{}</span></a></li>",
        cx.base_path,
        icon("desktop", None),
        tr("preview")
    );
    if remote["is_git"].as_bool() == Some(true) {
        items.push_str(&item(if has_updates {
            crate::admin_ui::Button {
                class: "btn-transparent admin-config-components__update",
                icon: Some("cloud-arrow-down"),
                label: crate::admin_ui::tr(cx, &key("update")),
                ..Default::default()
            }
        } else {
            crate::admin_ui::Button {
                class: "btn-transparent admin-config-components__check-updates",
                icon: Some("arrows-rotate"),
                label: crate::admin_ui::tr(cx, &key("check_update")),
                ..Default::default()
            }
        }));
    }
    items.push_str(&format!(
        "<li class=\"dropdown-menu__item\"><a class=\"btn btn-icon-text btn-transparent admin-config-components__export\" href=\"{}/admin/customize/themes/{id}/export\">{}<span class=\"d-button-label\">{}</span></a></li>",
        cx.base_path,
        icon("download", None),
        tr("export")
    ));
    items.push_str(&item(crate::admin_ui::Button {
        class: "btn-danger admin-config-components__delete",
        icon: Some("trash-can"),
        label: crate::admin_ui::tr(cx, &key("delete")),
        ..Default::default()
    }));
    let edit_path = format!("/admin/customize/themes/{id}");
    format!(
        "<tr class=\"d-table__row admin-config-components__component-row{}\" data-component-id=\"{id}\" data-component-name=\"{}\"><td class=\"d-table__cell --overview\">{overview}</td><td class=\"d-table__cell --detail admin-config-components__parent-themes\"><div class=\"d-table__mobile-label\">{}</div><div class=\"admin-config-components__parent-themes-list\">{used}</div></td><td class=\"d-table__cell --detail\"><div class=\"d-table__mobile-label\">{}</div><div class=\"d-toggle-switch\"><label class=\"d-toggle-switch__label\"><button aria-checked=\"{enabled}\" class=\"d-toggle-switch__checkbox admin-config-components__toggle\" role=\"switch\" type=\"button\"></button><span class=\"d-toggle-switch__checkbox-slider\">{}</span></label></div></td><td class=\"d-table__cell --controls\"><div class=\"d-table__cell-actions\">{}{}</div></td></tr>",
        if has_updates { " has-update" } else { "" },
        escape(component["name"].as_str().unwrap_or_default()),
        tr("used_on"),
        tr("enabled"),
        if enabled {
            icon("check", None)
        } else {
            String::new()
        },
        crate::admin_ui::Button {
            class: "btn-default admin-config-components__edit",
            label: crate::admin_ui::tr(cx, &key("edit")),
            href: Some(&edit_path),
            ..Default::default()
        }
        .render(),
        crate::admin_ui::menu(
            "btn-default admin-config-components__more-actions",
            "component-menu",
            "ellipsis",
            &t(cx.i18n, "admin.config_areas.flags.more_options.title"),
            &format!(
                "<div class=\"fk-d-menu__inner-content\"><ul class=\"dropdown-menu\">{items}</ul></div>"
            ),
        )
    )
}

/// The theme site settings tab (ThemeSiteSettings): the help with the
/// current theme, DFilterControls and the settings' table.
pub fn theme_site_settings(cx: &AdminCx, doc: &Value, current: Option<(i64, &str)>) -> String {
    let key = "admin.config_areas.themes_and_components.theme_site_settings.";
    let mut out = crate::admin_ui::subheader(
        &t(cx.i18n, &format!("{key}title")),
        "",
        Some(&escape(&t(cx.i18n, &format!("{key}description")))),
    );
    let (current_id, current_name) = current.unwrap_or((0, ""));
    let help = t_with(
        cx.i18n,
        "admin.theme_site_settings.help",
        &[
            ("currentTheme", &escape(current_name)),
            ("basePath", cx.base_path),
            ("currentThemeId", &current_id.to_string()),
        ],
    );
    let filter = format!(
        "<div class=\"d-filter-controls\"><div class=\"d-filter-controls__inputs\"><div class=\"filter-input-container\">{}<input autocapitalize=\"none\" autocomplete=\"off\" autocorrect=\"off\" class=\"filter-input d-filter-controls__input\" placeholder=\"{}\" type=\"text\" value=\"\"></div></div>{}</div>",
        icon("magnifying-glass", Some("-left")),
        escape(&t(cx.i18n, "admin.theme_site_settings.filter")),
        crate::admin_ui::Button {
            class: "btn-default d-filter-controls__reset",
            icon: Some("arrow-rotate-left"),
            title: crate::admin_ui::tr(cx, "filter_controls.reset"),
            disabled: true,
            attrs: format!(
                " aria-label=\"{}\"",
                escape(&t(cx.i18n, "filter_controls.reset"))
            ),
            ..Default::default()
        }
        .render()
    );
    let mut rows = String::new();
    if let Some(settings) = doc["themes_with_site_setting_overrides"].as_object() {
        for (name, s) in settings {
            let default = match &s["default"] {
                Value::String(v) => v.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            let themes = s["themes"].as_array().cloned().unwrap_or_default();
            let overridden = if themes.is_empty() {
                "-".to_string()
            } else {
                themes
                    .iter()
                    .map(|th| {
                        let value = match &th["value"] {
                            Value::String(v) => v.clone(),
                            other => other.to_string(),
                        };
                        format!(
                            "<span aria-expanded=\"false\" class=\"fk-d-tooltip__trigger\" data-trigger=\"\" role=\"button\"><span class=\"fk-d-tooltip__trigger-container\"><a class=\"theme-link\" data-theme-id=\"{}\" href=\"{}/admin/customize/themes/{}\">{}</a></span><template class=\"d-tooltip-content\">{}</template></span>",
                            th["theme_id"],
                            cx.base_path,
                            th["theme_id"],
                            escape(th["theme_name"].as_str().unwrap_or_default()),
                            escape(&t_with(
                                cx.i18n,
                                "admin.theme_site_settings.overridden_value",
                                &[("value", &value)]
                            ))
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            };
            let theme_names: Vec<String> = themes
                .iter()
                .filter_map(|th| th["theme_name"].as_str().map(str::to_lowercase))
                .collect();
            rows.push_str(&format!(
                "<tr class=\"admin-theme-site-settings-row d-table__row\" data-setting-name=\"{}\" data-search=\"{}\"><td class=\"admin-theme-site-settings-row__setting\"><p class=\"setting-label\">{}</p><div class=\"setting-description\">{}</div></td><td class=\"admin-theme-site-settings-row__default\">{}</td><td class=\"admin-theme-site-settings-row__overridden\">{overridden}</td></tr>",
                escape(name),
                escape(&format!(
                    "{}\n{}\n{}\n{}",
                    s["humanized_name"].as_str().unwrap_or_default(),
                    name,
                    s["description"].as_str().unwrap_or_default(),
                    theme_names.join(",")
                )
                .to_lowercase()),
                escape(s["humanized_name"].as_str().unwrap_or_default()),
                escape(s["description"].as_str().unwrap_or_default()),
                escape(&default)
            ));
        }
    }
    out.push_str(&format!(
        "<div class=\"theme-site-settings\"><div class=\"d-page-subheader\"><div class=\"d-page-subheader__title-row\"></div><p class=\"d-page-subheader__description\">{help}</p></div>{filter}<table class=\"d-table admin-theme-site-settings\"><thead class=\"d-table__header\"><tr><th>{}</th><th>{}</th><th>{}</th></tr></thead><tbody>{rows}</tbody></table></div>",
        escape(&t(cx.i18n, "admin.theme_site_settings.setting")),
        escape(&t(cx.i18n, "admin.theme_site_settings.default_value")),
        escape(&t(cx.i18n, "admin.theme_site_settings.overridden_by")),
    ));
    out
}
