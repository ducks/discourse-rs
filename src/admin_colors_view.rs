//! The color palettes pages (adminConfig.colorPalettes, 90g7) as Ember
//! draws them: the list from /admin/config/colors.json and a palette's
//! editor from /admin/config/colors/:id.json.

use serde_json::Value;

use crate::admin_customize_kit::area_card;
use crate::admin_view::{AdminCx, t, t_with};
use crate::topic_list_view::{escape, icon};

/// The default theme as the pages read it (the list's extras, the site's
/// user_themes for the editor).
pub struct DefaultTheme {
    pub id: i64,
    pub name: String,
    pub color_scheme_id: Option<i64>,
    pub dark_color_scheme_id: Option<i64>,
}

/// SvgSingleColorPalettePlaceholder.
const PLACEHOLDER: &str = "<svg fill=\"none\" viewBox=\"0 0 558 171\" xmlns=\"http://www.w3.org/2000/svg\"><rect fill=\"var(--primary-low--preview, #e9e9e9)\" height=\"25.616\" rx=\"4\" width=\"120\" x=\"436.298\" y=\"95.819\"></rect><rect fill=\"var(--tertiary--preview, #08c)\" height=\"24.64\" rx=\"4\" width=\"120\" x=\"436.298\" y=\"144.442\"></rect><rect fill=\"var(--tertiary-low--preview, #d1f0ff)\" height=\"71.394\" rx=\"4\" width=\"555.265\" x=\".22\" y=\".341\"></rect><rect fill=\"var(--danger--preview, #c80001)\" height=\"24.64\" rx=\"4\" width=\"30\" x=\"23.451\" y=\"145.418\"></rect><rect fill=\"var(--success--preview, #090)\" height=\"24.64\" rx=\"4\" width=\"30\" x=\"88.578\" y=\"145.418\"></rect><rect fill=\"var(--love--preview, #fa6c8d)\" height=\"24.64\" rx=\"4\" width=\"30\" x=\"153.704\" y=\"145.418\"></rect><rect fill=\"var(--highlight--preview, #ffff4d)\" height=\"24.64\" rx=\"4\" width=\"30\" x=\"218.831\" y=\"145.418\"></rect><rect fill=\"var(--primary--preview, #222)\" height=\"4.526\" rx=\"2.263\" width=\"200.35\" x=\"23.451\" y=\"43.772\"></rect><rect fill=\"var(--primary--preview, #222)\" height=\"4.526\" rx=\"2.263\" width=\"333.847\" x=\"23.451\" y=\"66.295\"></rect><rect fill=\"var(--primary--preview, #222)\" height=\"4.526\" rx=\"2.263\" width=\"267.897\" x=\"23.451\" y=\"90.819\"></rect><rect fill=\"var(--primary--preview, #222)\" height=\"4.526\" rx=\"2.263\" width=\"299.961\" x=\"23.451\" y=\"116.342\"></rect></svg>";

fn normalize_hex(hex: &str) -> String {
    let hex = hex.trim_start_matches('#');
    if hex.chars().count() == 3 {
        hex.chars().flat_map(|c| [c, c]).collect()
    } else {
        hex.to_string()
    }
}

fn rgb(hex: &str) -> (f64, f64, f64) {
    let hex = normalize_hex(hex);
    let part = |i: usize| {
        hex.get(i..i + 2)
            .and_then(|s| u8::from_str_radix(s, 16).ok())
            .unwrap_or(0) as f64
    };
    (part(0), part(2), part(4))
}

fn brightness(hex: &str) -> f64 {
    let (r, g, b) = rgb(hex);
    r * 0.299 + g * 0.587 + b * 0.114
}

/// color-transformations.js' scaleColorLightness (sass' scale-color, by
/// way of HSL).
fn scale_lightness(hex: &str, lightness: f64) -> String {
    let (r, g, b) = rgb(hex);
    let (r, g, b) = (r / 255.0, g / 255.0, b / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let (h, s) = if max == min {
        (0.0, 0.0)
    } else {
        let d = max - min;
        let s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };
        let h = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        (h / 6.0, s)
    };
    let l = if lightness > 0.0 {
        l + (1.0 - l) * (lightness / 100.0)
    } else {
        l * (1.0 + lightness / 100.0)
    };
    let (r, g, b) = if s == 0.0 {
        (l, l, l)
    } else {
        let hue = |p: f64, q: f64, mut t: f64| {
            if t < 0.0 {
                t += 1.0;
            }
            if t > 1.0 {
                t -= 1.0;
            }
            if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 0.5 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            }
        };
        let q = if l < 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };
        let p = 2.0 * l - q;
        (
            hue(p, q, h + 1.0 / 3.0),
            hue(p, q, h),
            hue(p, q, h - 1.0 / 3.0),
        )
    };
    // JS's Math.round: halves up.
    let round = |v: f64| (v * 255.0 + 0.5).floor() as i64;
    format!("#{:02x}{:02x}{:02x}", round(r), round(g), round(b))
}

/// getPreviewColor: primary-low or tertiary-low from the scheme.
fn preview_color(colors: &[Value], name: &str) -> String {
    let find = |n: &str| {
        colors
            .iter()
            .find(|c| c["name"].as_str() == Some(n))
            .and_then(|c| c["hex"].as_str())
    };
    let (Some(base), Some(secondary)) = (find(name), find("secondary")) else {
        return "#ffffff".into();
    };
    let (lighter, darker) = if name == "primary" {
        (90.0, -78.0)
    } else {
        (85.0, -65.0)
    };
    if brightness(base) < brightness(secondary) {
        scale_lightness(base, lighter)
    } else {
        scale_lightness(base, darker)
    }
}

/// ColorPalettePreview: its style and the placeholder drawing.
pub fn preview(scheme: &Value) -> String {
    let colors = scheme["colors"].as_array().cloned().unwrap_or_default();
    let style = if scheme["is_builtin_default"].as_bool() == Some(true) || colors.is_empty() {
        "--primary-low--preview: #e9e9e9; --tertiary-low--preview: #d1f0ff;".to_string()
    } else {
        let vars: Vec<String> = colors
            .iter()
            .map(|c| {
                let mut hex = c["hex"]
                    .as_str()
                    .filter(|h| !h.is_empty())
                    .or_else(|| c["default_hex"].as_str())
                    .unwrap_or_default()
                    .to_string();
                if !hex.is_empty() && !hex.starts_with('#') {
                    hex = format!("#{hex}");
                }
                format!(
                    "--{}--preview: {hex}",
                    c["name"].as_str().unwrap_or_default().replace('_', "-")
                )
            })
            .collect();
        format!(
            "--primary-low--preview: {}; --tertiary-low--preview: {}; {};",
            preview_color(&colors, "primary"),
            preview_color(&colors, "tertiary"),
            vars.join("; ")
        )
    };
    format!(
        "<div class=\"color-palette__preview\" style=\"{}\">{PLACEHOLDER}</div>",
        escape(&style)
    )
}

/// The list's query params (DFilterControls' textFilterQueryParam and
/// dropdownFilterQueryParam).
pub struct ListFilters {
    pub filter: String,
    pub kind: String,
}

/// templates/admin-config/color-palettes/index.gjs.
pub fn list(
    cx: &AdminCx,
    doc: &Value,
    theme: Option<&DefaultTheme>,
    filters: &ListFilters,
) -> String {
    let tr = |k: &str| escape(&t(cx.i18n, k));
    let header = format!(
        "<div class=\"d-page-header\"><div class=\"d-page-header__breadcrumbs\"><ul class=\"d-breadcrumbs\"><li class=\"d-breadcrumbs__item\"><a class=\"d-breadcrumbs__link\" href=\"{b}/admin\">{}</a> <span class=\"separator\">{}</span></li><li aria-current=\"page\" class=\"d-breadcrumbs__item\"><a class=\"d-breadcrumbs__link\" href=\"{b}/admin/config/colors\">{}</a></li></ul></div></div>",
        tr("admin_title"),
        icon("angle-right", None),
        tr("admin.config.color_palettes.title"),
        b = cx.base_path
    );
    let actions = crate::admin_ui::actions(
        cx,
        "d-page-subheader",
        &[crate::admin_ui::Action::Primary(crate::admin_ui::Button {
            class: "create-new-palette",
            icon: Some("plus"),
            label: crate::admin_ui::tr(cx, "admin.customize.new"),
            ..Default::default()
        })],
    );
    let description = format!(
        "{} <span class=\"d-page-subheader__learn-more\">{}</span>",
        tr("admin.config.color_palettes.header_description"),
        t_with(
            cx.i18n,
            "learn_more_with_link",
            &[(
                "url",
                "https://meta.discourse.org/t/allow-users-to-select-new-color-palettes/60857"
            )]
        )
    );
    let subheader = crate::admin_ui::subheader(
        &t(cx.i18n, "admin.config.color_palettes.title"),
        &actions,
        Some(&description),
    );
    let palettes: Vec<&Value> = doc["palettes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| {
            p["is_base"].as_bool() != Some(true) || p["is_builtin_default"].as_bool() == Some(true)
        })
        .collect();
    let options = [
        ("all", "admin.customize.colors.filters.all"),
        (
            "user_selectable",
            "admin.customize.colors.filters.user_selectable",
        ),
        ("from_theme", "admin.customize.colors.filters.from_theme"),
    ];
    let kind = if options.iter().any(|(v, _)| *v == filters.kind) {
        filters.kind.as_str()
    } else {
        "all"
    };
    let label = options
        .iter()
        .find(|(v, _)| *v == kind)
        .map(|(_, k)| t(cx.i18n, k))
        .unwrap_or_default();
    let select: String = options
        .iter()
        .map(|(v, k)| {
            format!(
                "<option class=\"d-native-select__option{}\"{} value=\"{v}\">{}</option>",
                if *v == kind { " --selected" } else { "" },
                if *v == kind && kind != "all" {
                    " selected=\"\""
                } else {
                    ""
                },
                tr(k)
            )
        })
        .collect();
    let active = !filters.filter.is_empty() || kind != "all";
    // DFilterControls' filter: minItemsForFilter (8) palettes or more.
    let controls = if palettes.len() >= 8 {
        format!(
            "<div class=\"d-filter-controls\"><div class=\"d-filter-controls__inputs\"><div class=\"filter-input-container\">{}<input autocapitalize=\"none\" autocomplete=\"off\" autocorrect=\"off\" class=\"filter-input d-filter-controls__input\" placeholder=\"{}\" type=\"text\" value=\"{}\"></div></div><div class=\"d-filter-controls__dropdowns\"><select aria-label=\"{}\" class=\"d-filter-controls__dropdown d-native-select{}\">{select}</select></div>{}</div>",
            icon("magnifying-glass", Some("-left")),
            tr("admin.customize.colors.filters.search_placeholder"),
            escape(&filters.filter),
            escape(&label),
            if kind != "all" { " --active" } else { "" },
            crate::admin_ui::Button {
                class: "btn-default d-filter-controls__reset",
                icon: Some("arrow-rotate-left"),
                title: crate::admin_ui::tr(cx, "filter_controls.reset"),
                disabled: !active,
                attrs: format!(" aria-label=\"{}\"", tr("filter_controls.reset")),
                ..Default::default()
            }
            .render()
        )
    } else {
        String::new()
    };
    let needle = filters.filter.to_lowercase();
    let shown: Vec<&Value> = palettes
        .iter()
        .copied()
        .filter(|p| match kind {
            "user_selectable" => p["user_selectable"].as_bool() == Some(true),
            "from_theme" => !p["theme_id"].is_null(),
            _ => true,
        })
        .filter(|p| {
            needle.is_empty()
                || ["name", "theme_name"].iter().any(|k| {
                    p[*k]
                        .as_str()
                        .is_some_and(|v| v.to_lowercase().contains(&needle))
                })
        })
        .collect();
    let content = if shown.is_empty() {
        format!(
            "<div class=\"d-filter-controls__no-results\"><p>{}</p>{}</div>",
            tr("admin.customize.colors.filters.no_results"),
            crate::admin_ui::Button {
                class: "btn-default d-filter-controls__reset-button",
                label: crate::admin_ui::tr(cx, "filter_controls.reset"),
                ..Default::default()
            }
            .render()
        )
    } else {
        format!(
            "<ul class=\"color-palette__list\">{}</ul>",
            shown
                .iter()
                .map(|p| list_item(cx, p, theme))
                .collect::<String>()
        )
    };
    format!("{header}{subheader}{controls}{content}")
}

/// ColorPaletteListItem.
fn list_item(cx: &AdminCx, scheme: &Value, theme: Option<&DefaultTheme>) -> String {
    let tr = |k: &str| escape(&t(cx.i18n, k));
    let id = scheme["id"].as_i64();
    let builtin = scheme["is_builtin_default"].as_bool() == Some(true);
    let can_edit = !builtin && id.is_some();
    let theme_id = scheme["theme_id"].as_i64();
    let can_delete = !builtin && theme_id.is_none();
    let show_default = if builtin {
        theme.and_then(|t| t.color_scheme_id).is_some()
    } else {
        true
    };
    let default_light = match (builtin, theme) {
        (true, Some(t)) => t.color_scheme_id.is_none(),
        (false, Some(t)) => t.color_scheme_id.is_some() && t.color_scheme_id == id,
        _ => false,
    };
    // A built-in default's dark badge reads defaultDarkTheme, which the
    // list never sets.
    let default_dark = match (builtin, theme) {
        (false, Some(t)) => t.dark_color_scheme_id.is_some() && t.dark_color_scheme_id == id,
        _ => false,
    };
    let name = escape(scheme["name"].as_str().unwrap_or_default());
    let title = if can_edit {
        format!(
            "<h3><a href=\"{}/admin/config/colors/{}\">{name}</a></h3>",
            cx.base_path,
            id.unwrap_or_default()
        )
    } else {
        format!("<h3>{name}</h3>")
    };
    let theme_link = theme_id
        .map(|tid| {
            format!(
                "<a href=\"{}/admin/customize/themes/{tid}\">{} {}</a>",
                cx.base_path,
                icon("link", None),
                escape(scheme["theme_name"].as_str().unwrap_or_default())
            )
        })
        .unwrap_or_default();
    let mut badges = String::new();
    if default_light {
        badges.push_str(&format!(
            "<span class=\"theme-card__badge --default\" title=\"{}\">{} {}</span>",
            tr("admin.customize.colors.default_light_badge.title"),
            icon("sun", None),
            tr("admin.customize.colors.default_light_badge.text")
        ));
    }
    if default_dark {
        badges.push_str(&format!(
            "<span class=\"theme-card__badge --default\" title=\"{}\">{} {}</span>",
            tr("admin.customize.colors.default_dark_badge.title"),
            icon("moon", None),
            tr("admin.customize.colors.default_dark_badge.text")
        ));
    }
    if scheme["user_selectable"].as_bool() == Some(true) {
        badges.push_str(&format!(
            "<span class=\"theme-card__badge --selectable\" title=\"{}\">{} {}</span>",
            tr("admin.customize.theme.user_selectable"),
            icon("user-check", None),
            tr("admin.customize.theme.user_selectable_badge_label")
        ));
    }
    let edit_href = id.map(|i| format!("/admin/config/colors/{i}"));
    let edit = crate::admin_ui::Button {
        class: "btn-default",
        label: crate::admin_ui::tr(cx, "admin.customize.colors.edit"),
        disabled: !can_edit,
        href: if can_edit { edit_href.as_deref() } else { None },
        ..Default::default()
    }
    .render();
    let tooltip = if can_edit {
        String::new()
    } else {
        crate::admin_form_kit::tooltip_trigger(
            &t(cx.i18n, "admin.customize.colors.system_palette"),
            "",
        )
        .replace("circle-question", "circle-info")
        .replacen("fk-d-tooltip__trigger \"", "fk-d-tooltip__trigger\"", 1)
    };
    let menu = if show_default {
        let theme_name = theme.map(|t| t.name.as_str()).unwrap_or("Default");
        let mut items = String::new();
        let item = |b: crate::admin_ui::Button| {
            format!("<li class=\"dropdown-menu__item\">{}</li>", b.render())
        };
        if !builtin {
            let selectable = scheme["user_selectable"].as_bool() == Some(true);
            items.push_str(&item(crate::admin_ui::Button {
                class: "btn-transparent palette-user-selectable",
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
        }
        for (mode, star, on) in [
            ("light", "far-star", default_light),
            ("dark", "star", default_dark),
        ] {
            items.push_str(&item(crate::admin_ui::Button {
                class: "btn-transparent btn-palette-default",
                icon: Some(star),
                label: Some(t_with(
                    cx.i18n,
                    &format!("admin.customize.colors.set_default_{mode}"),
                    &[("theme", theme_name)],
                )),
                disabled: on,
                attrs: format!(" data-mode=\"{mode}\""),
                ..Default::default()
            }));
        }
        if can_delete {
            items.push_str(&item(crate::admin_ui::Button {
                class: "btn-transparent --danger palette-delete",
                icon: Some("trash-can"),
                label: crate::admin_ui::tr(cx, "admin.customize.delete"),
                ..Default::default()
            }));
        }
        format!(
            "<button aria-expanded=\"false\" class=\"btn no-text btn-icon fk-d-menu__trigger -trigger btn-flat\" data-trigger=\"\" type=\"button\">{}</button><template class=\"color-palette-menu\"><div class=\"fk-d-menu__inner-content\"><ul class=\"dropdown-menu\">{items}</ul></div></template>",
            icon("ellipsis", None)
        )
    } else {
        String::new()
    };
    format!(
        "<li class=\"admin-config-area-card color-palette\" data-palette-id=\"{}\" data-user-selectable=\"{}\"><div class=\"color-palette__container\">{}<div class=\"color-palette__details\">{title}<div class=\"color-palette__theme-link\">{theme_link}</div><div class=\"color-palette__badges\">{badges}</div></div><div class=\"color-palette__controls\"><div class=\"fk-d-button-tooltip\">{edit}{tooltip}</div>{menu}</div></div></li>",
        scheme["id"],
        scheme["user_selectable"].as_bool() == Some(true),
        preview(scheme)
    )
}

/// FKField `@type="toggle"` without its title, in a form row.
fn toggle_row(name: &str, description: &str, on: bool) -> String {
    format!(
        "<div class=\"form-kit__row\"><div class=\"form-kit__container form-kit__field form-kit__field-toggle --full\" data-control-type=\"toggle\" data-name=\"{name}\" id=\"control-{name}\"><p class=\"form-kit-text form-kit__container-description\">{}</p><div class=\"form-kit__container-content\"><div class=\"d-toggle-switch\"><label class=\"d-toggle-switch__label\"><button aria-checked=\"{on}\" class=\"d-toggle-switch__checkbox form-kit__control-toggle\" role=\"switch\" type=\"button\"></button><span class=\"d-toggle-switch__checkbox-slider\">{}</span></label></div></div></div></div>",
        escape(description),
        if on {
            icon("check", None)
        } else {
            String::new()
        }
    )
}

/// The palette's editor (AdminConfigAreasColorPalette).
pub fn editor(cx: &AdminCx, palette: &Value, theme: Option<&DefaultTheme>) -> String {
    let tr = |k: &str| escape(&t(cx.i18n, k));
    let id = palette["id"].as_i64();
    let installed = !palette["theme_id"].is_null();
    let name = escape(palette["name"].as_str().unwrap_or_default());
    let edit_name = if installed {
        String::new()
    } else {
        crate::admin_ui::Button {
            class: "btn-flat admin-config-color-palettes__edit-name",
            icon: Some("pencil"),
            ..Default::default()
        }
        .render()
    };
    let delete = if installed {
        String::new()
    } else {
        crate::admin_ui::Button {
            class: "btn-danger delete-palette",
            label: crate::admin_ui::tr(cx, "admin.config_areas.color_palettes.delete"),
            ..Default::default()
        }
        .render()
    };
    let owner = if installed {
        format!(
            "<div class=\"admin-config-color-palettes__theme-owner\">{}<span>{} <a href=\"{}/admin/customize/themes/{}\">{}</a></span></div>",
            icon("circle-info", None),
            tr("admin.customize.theme_owner"),
            cx.base_path,
            palette["theme_id"],
            escape(palette["theme_name"].as_str().unwrap_or_default())
        )
    } else {
        String::new()
    };
    let theme_name = theme.map(|t| t.name.as_str()).unwrap_or_default();
    let options = format!(
        "{}{}{}",
        toggle_row(
            "default_light_on_theme",
            &t_with(
                cx.i18n,
                "admin.config_areas.color_palettes.color_options.toggle_default_light_on_theme",
                &[("themeName", theme_name)]
            ),
            theme.is_some_and(|t| t.color_scheme_id.is_some() && t.color_scheme_id == id)
        ),
        toggle_row(
            "default_dark_on_theme",
            &t_with(
                cx.i18n,
                "admin.config_areas.color_palettes.color_options.toggle_default_dark_on_theme",
                &[("themeName", theme_name)]
            ),
            theme.is_some_and(|t| t.dark_color_scheme_id.is_some() && t.dark_color_scheme_id == id)
        ),
        toggle_row(
            "user_selectable",
            &t(
                cx.i18n,
                "admin.config_areas.color_palettes.color_options.toggle_description"
            ),
            palette["user_selectable"].as_bool() == Some(true)
        )
    );
    let colors: String = palette["colors"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, c)| color_item(cx, c, i))
        .collect();
    let colors = format!(
        "<div class=\"form-kit__container form-kit__field form-kit__field-custom --full\" data-control-type=\"custom\" data-name=\"colors\" id=\"control-colors\"><div class=\"form-kit__container-content\"><div class=\"form-kit__control-custom\"><div class=\"color-palette-editor\"><div class=\"color-palette-editor__colors-list\">{colors}</div></div></div></div></div>"
    );
    let save = format!(
        "<div class=\"admin-config-color-palettes__save-card\">{}{}</div>",
        crate::admin_ui::Button {
            class: "btn-default copy-to-clipboard",
            label: crate::admin_ui::tr(cx, "admin.config_areas.color_palettes.copy_to_clipboard"),
            ..Default::default()
        }
        .render(),
        crate::admin_form_kit::submit(
            "",
            &t(cx.i18n, "admin.config_areas.color_palettes.save_changes")
        )
    );
    format!(
        "<div class=\"admin-config-area__primary-content\"><div class=\"admin-config-color-palettes__back\"><a class=\"active\" href=\"{}/admin/config/colors\">{} {}</a></div><form class=\"form-kit\" data-palette-id=\"{}\" novalidate=\"\"><div class=\"admin-config-color-palettes__header\"><div class=\"admin-config-color-palettes__top-controls\"><div class=\"admin-config-color-palettes__name-control\"><h2 class=\"admin-config-color-palettes__name\">{name}</h2>{edit_name}</div><div class=\"admin-config-color-palettes__top-actions\">{}{delete}</div></div>{owner}</div><div class=\"admin-config-color-palettes__sections\">{}{}{}</div></form></div>",
        cx.base_path,
        icon("angle-left", None),
        tr("admin.customize.colors.back_to_colors"),
        palette["id"],
        crate::admin_ui::Button {
            class: "btn-default duplicate-palette",
            label: crate::admin_ui::tr(cx, "admin.config_areas.color_palettes.duplicate"),
            ..Default::default()
        }
        .render(),
        area_card(
            "",
            &t(cx.i18n, "admin.config_areas.color_palettes.color_options.title"),
            Some(&options),
            None
        ),
        area_card(
            "",
            &t(cx.i18n, "admin.config_areas.color_palettes.colors.title"),
            Some(&colors),
            None
        ),
        area_card("", "", Some(&save), None).replacen(
            "<div class=\"admin-config-area-card__header-wrapper\"><h3 class=\"admin-config-area-card__title\"></h3></div>",
            "",
            1
        ),
    )
}

/// A color's row in ColorPaletteEditor: its description and name, the
/// Picker (the color input, the hex text input) and revert.
fn color_item(cx: &AdminCx, color: &Value, position: usize) -> String {
    let name = color["name"].as_str().unwrap_or_default();
    let translated =
        crate::admin_view::t_opt(cx.i18n, &format!("admin.customize.colors.{name}.name"))
            .unwrap_or_else(|| name.to_string());
    let description = crate::admin_view::t_opt(
        cx.i18n,
        &format!("admin.customize.colors.{name}.description"),
    )
    .unwrap_or_default();
    let hex = normalize_hex(color["hex"].as_str().unwrap_or_default());
    let overridden = color["default_hex"]
        .as_str()
        .is_some_and(|d| !d.is_empty() && Some(d) != color["hex"].as_str());
    let info = if description.is_empty() {
        format!(
            "<div class=\"color-palette-editor__color-description form-kit__container-title\">{}</div>",
            escape(&translated)
        )
    } else {
        format!(
            "<div class=\"color-palette-editor__color-description form-kit__container-title\">{}</div><div class=\"color-palette-editor__color-name\">{}</div>",
            escape(&description),
            escape(&translated)
        )
    };
    format!(
        "<div class=\"color-palette-editor__colors-item\" data-color-name=\"{}\" data-default-hex=\"{}\"><div class=\"color-palette-editor__color-info\">{info}</div><div class=\"color-palette-editor__color-controls\"><div class=\"color-palette-editor__picker form-kit__control-input\"><input class=\"color-palette-editor__input\" data-position=\"{position}\" type=\"color\" value=\"#{hex}\"><div class=\"color-palette-editor__input-wrapper\">{}<input class=\"color-palette-editor__text-input\" data-position=\"{position}\" maxlength=\"6\" type=\"text\" value=\"{hex}\"></div></div>{}</div></div>",
        escape(name),
        escape(color["default_hex"].as_str().unwrap_or_default()),
        icon("hashtag", Some("color-palette-editor__icon")),
        crate::admin_ui::Button {
            class: if overridden {
                "btn-flat color-palette-editor__revert"
            } else {
                "btn-flat color-palette-editor__revert --hidden"
            },
            icon: Some("arrow-rotate-left"),
            ..Default::default()
        }
        .render()
    )
}
