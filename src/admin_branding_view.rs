//! The logo and fonts pages (90g6), as Ember draws them: templates/
//! admin-config/logo.gjs over AdminLogoForm (FormKit's image fields with
//! UppyImageUploader, the dark logos' toggles, the collapsed mobile,
//! email and social media sections) and templates/admin-config/fonts.gjs
//! over AdminFontsForm (AdminFontChooser for the base and heading fonts,
//! the default text sizes). Their script is static/js/admin-branding.js.

use serde_json::Value;

use crate::admin_ui;
use crate::admin_view::{AdminCx, Crumb, PageHeader, t};
use crate::topic_list_view::{escape, icon};

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// DPageHeader with its breadcrumbs, no tabs, over the area.
fn page(cx: &AdminCx, key: &str, path: &str, content: &str) -> String {
    let title = t(cx.i18n, &format!("admin.config.{key}.title"));
    let header = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path,
            },
        ],
        title,
        actions: String::new(),
        description: None,
        learn_more_url: None,
        tabs: None,
    }
    .render(cx);
    format!(
        "<div class=\"admin-config-page\">{header}<div class=\"admin-config-area\"><div class=\"admin-config-area__primary-content\">{content}</div></div></div>"
    )
}

/// FKField's container with its title (optional), description, content
/// and help text.
#[allow(clippy::too_many_arguments)]
fn field(
    cx: &AdminCx,
    name: &str,
    control: &str,
    full: bool,
    title: &str,
    // HTML.
    description: Option<&str>,
    help: Option<&str>,
    required: bool,
    content: &str,
) -> String {
    let id = format!("{name}-field");
    let mut out = format!(
        "<div class=\"form-kit__container form-kit__field form-kit__field-{control}{}\" data-control-type=\"{control}\" data-name=\"{name}\" id=\"control-{name}\"><label class=\"form-kit__container-title\" for=\"{id}\"><span>{}</span>",
        if full { " --full" } else { "" },
        escape(title)
    );
    if !required {
        out.push_str(&format!(
            " <span class=\"form-kit__container-optional\">({})</span>",
            escape(&t(cx.i18n, "form_kit.optional"))
        ));
    }
    out.push_str("</label>");
    if let Some(d) = description {
        out.push_str(&format!(
            "<p class=\"form-kit-text form-kit__container-description\" id=\"{id}-description\">{}</p>",
            d
        ));
    }
    out.push_str(&format!(
        "<div class=\"form-kit__container-content\">{content}"
    ));
    if let Some(h) = help {
        out.push_str(&format!(
            "<p class=\"form-kit-text form-kit__container-help-text\" id=\"{id}-help-text\">{}</p>",
            escape(h)
        ));
    }
    out.push_str("</div></div>");
    out
}

/// UppyImageUploader as FKControlImage draws it: the image with its
/// lightbox and Change and Delete, or the placeholder and the picker.
pub fn image_uploader(
    cx: &AdminCx,
    id: &str,
    url: &str,
    size: Option<(i64, i64)>,
    placeholder: Option<&str>,
) -> String {
    let input = format!("{id}__input");
    let placeholder = placeholder.filter(|p| !p.is_empty());
    // For admin-branding.js, which draws the uploader again.
    let ph = placeholder
        .map(|p| format!(" data-placeholder=\"{}\"", escape(p)))
        .unwrap_or_default();
    let picker = |label_class: &str, label: &str| {
        format!(
            "<label class=\"{label_class}\" for=\"{input}\" tabindex=\"0\">{}<div class=\"pick-files-button\"><input accept=\"image/*\" id=\"{input}\" type=\"file\"></div>{}</label>",
            icon("upload", None),
            escape(&t(cx.i18n, label))
        )
    };
    if url.is_empty() {
        let uploading = escape(&t(cx.i18n, "upload_selector.uploading"));
        let overlay = placeholder
            .filter(|p| !p.is_empty())
            .map(|p| {
                format!(
                    "<div class=\"placeholder-overlay\" style=\"background-image: url({})\"></div>",
                    escape(p)
                )
            })
            .unwrap_or_default();
        return format!(
            "<div class=\"file-uploader no-image form-kit__control-image no-repeat contain-image\" id=\"{id}\"{ph}><div class=\"file-uploader__preview input-xxlarge\" style=\"background-image: url()\">{overlay}<div class=\"file-uploader__controls\">{}<div class=\"file-uploader__progress-status hidden\"><div aria-label=\"{uploading}\n          0%\" class=\"progress-bar-container\" role=\"progressbar\"><div class=\"progress-bar\" style=\"width: 0%\"></div></div><span>{uploading} 0%</span></div></div></div></div>",
            picker("btn btn-transparent", "upload_selector.select_file")
        );
    }
    let target = size
        .map(|(w, h)| format!(" data-target-height=\"{h}\" data-target-width=\"{w}\""))
        .unwrap_or_default();
    let full = format!(
        "{}{}",
        if url.starts_with('/') && !url.starts_with("//") {
            cx.base_path
        } else {
            ""
        },
        url
    );
    format!(
        "<div class=\"file-uploader has-image form-kit__control-image no-repeat contain-image\" id=\"{id}\"{ph}><div class=\"file-uploader__preview input-xxlarge\" style=\"background-image: url({u})\"><a class=\"lightbox\" href=\"{u}\" rel=\"nofollow ugc noopener\"{target}><div class=\"meta\"><span class=\"informations\">x</span></div></a><div class=\"expand-overlay\"><button class=\"btn no-text btn-icon btn-default btn-small image-uploader-lightbox-btn\" title=\"{expand}\" type=\"button\">{expand_icon}<span aria-hidden=\"true\">\u{200b}</span></button></div></div><div class=\"file-uploader__controls\">{change}<button class=\"btn btn-icon-text btn-danger btn-small\" type=\"button\">{trash}<span class=\"d-button-label\">{delete}</span></button></div></div>",
        u = escape(&full),
        expand = escape(&t(cx.i18n, "expand")),
        expand_icon = icon("discourse-expand", None),
        change = picker("btn btn-default btn-small", "upload_selector.change"),
        trash = icon("trash-can", None),
        delete = escape(&t(cx.i18n, "upload_selector.delete")),
    )
}

/// DToggleSwitch as FKControlToggle draws it.
fn toggle(on: bool) -> String {
    format!(
        "<div class=\"d-toggle-switch\"><label class=\"d-toggle-switch__label\"><button aria-checked=\"{on}\" class=\"d-toggle-switch__checkbox form-kit__control-toggle\" role=\"switch\" type=\"button\"></button><span class=\"d-toggle-switch__checkbox-slider\">{}</span></label></div>",
        if on {
            icon("check", None)
        } else {
            String::new()
        }
    )
}

/// What the logo form shows of a branding setting: its value (a URL),
/// its upload's size and its placeholder, from
/// /admin/config/site_settings.json's branding settings.
fn setting<'a>(settings: &'a [Value], name: &str) -> Option<&'a Value> {
    settings.iter().find(|v| s(&v["setting"]) == name)
}

/// One of the logo form's image fields.
fn image_field(cx: &AdminCx, settings: &[Value], name: &str, with_placeholder: bool) -> String {
    let k = |part: &str| {
        crate::admin_view::t_opt(cx.i18n, &format!("admin.config.logo.form.{name}.{part}"))
    };
    let v = setting(settings, name);
    let url = v.map(|v| s(&v["value"])).unwrap_or_default();
    let size = v.and_then(|v| {
        Some((
            v["upload"]["width"].as_i64()?,
            v["upload"]["height"].as_i64()?,
        ))
    });
    let placeholder = if with_placeholder {
        v.map(|v| s(&v["placeholder"]))
    } else {
        None
    };
    field(
        cx,
        name,
        "image",
        false,
        &k("title").unwrap_or_default(),
        k("description").map(|d| escape(&d)).as_deref(),
        k("help_text").as_deref(),
        false,
        &image_uploader(cx, &format!("{name}-field-{name}"), url, size, placeholder),
    )
}

/// A dark logo's toggle and, when on (a dark logo set), its field in a
/// section; off, the field waits in a template for admin-branding.js.
fn dark(cx: &AdminCx, settings: &[Value], name: &str) -> String {
    let on = setting(settings, name).is_some_and(|v| !s(&v["value"]).is_empty());
    let required = format!("{name}_required");
    let title = t(cx.i18n, &format!("admin.config.logo.form.{name}.required"));
    let section = format!(
        "<div class=\"form-kit__section\">{}</div>",
        image_field(cx, settings, name, false)
    );
    format!(
        "{}{}",
        field(
            cx,
            &required,
            "toggle",
            true,
            &title,
            None,
            None,
            false,
            &toggle(on)
        ),
        if on {
            section
        } else {
            format!(
                "<template class=\"admin-logo-form__dark\" data-for=\"{required}\">{section}</template>"
            )
        }
    )
}

/// AdminConfigAreaCardSection, collapsable and collapsed: its content in a
/// template until opened.
fn card_section(class: &str, heading: &str, content: &str) -> String {
    format!(
        "<section class=\"admin-config-area-card-section {class}\"><div class=\"admin-config-area-card-section__header-wrapper collapsable\" role=\"button\"><h4 class=\"admin-config-area-card-section__title\">{}</h4>{}</div><template class=\"admin-config-area-card-section__template\"><div class=\"admin-config-area-card-section__content\">{content}</div></template></section>",
        escape(heading),
        icon("angle-right", None)
    )
}

/// SimpleList with allowAny, the values `|` separated.
fn simple_list(cx: &AdminCx, id: &str, value: &str) -> String {
    let values: Vec<&str> = value.split('|').filter(|v| !v.is_empty()).collect();
    let mut out = format!("<div class=\"simple-list value-list\" id=\"{id}\">");
    if !values.is_empty() {
        out.push_str("<ul class=\"values\">");
        for (i, v) in values.iter().enumerate() {
            out.push_str(&format!(
                "<li class=\"value\" data-index=\"{i}\"><button class=\"btn no-text btn-icon btn-default remove-value-btn btn-small\" type=\"button\">{}<span aria-hidden=\"true\">\u{200b}</span></button><input class=\"value-input\" type=\"text\" value=\"{}\"></li>",
                icon("xmark", None),
                escape(v)
            ));
        }
        out.push_str("</ul>");
    }
    out.push_str(&format!(
        "<div class=\"simple-list-input\"><input autocapitalize=\"off\" autocomplete=\"off\" autocorrect=\"off\" class=\"add-value-input\" placeholder=\"{}\" type=\"text\" value=\"\"><button class=\"btn no-text btn-icon btn-default add-value-btn btn-small\" disabled=\"\" type=\"button\">{}<span aria-hidden=\"true\">\u{200b}</span></button></div></div>",
        escape(&t(cx.i18n, "admin.site_settings.simple_list.add_item")),
        icon("plus", None)
    ));
    out
}

/// The logo page: AdminLogoForm over the branding settings.
pub fn logo(cx: &AdminCx, settings: &[Value]) -> String {
    let k = |key: &str| t(cx.i18n, &format!("admin.config.logo.form.{key}"));
    let mut form = String::new();
    form.push_str(&image_field(cx, settings, "logo", false));
    form.push_str(&dark(cx, settings, "logo_dark"));
    form.push_str(&image_field(cx, settings, "large_icon", true));
    form.push_str(&image_field(cx, settings, "favicon", true));
    form.push_str(&image_field(cx, settings, "logo_small", false));
    form.push_str(&dark(cx, settings, "logo_small_dark"));
    let screenshots = setting(settings, "manifest_screenshots")
        .map(|v| s(&v["value"]))
        .unwrap_or_default();
    let mobile = format!(
        "{}{}{}{}{}",
        image_field(cx, settings, "mobile_logo", true),
        dark(cx, settings, "mobile_logo_dark"),
        image_field(cx, settings, "manifest_icon", false),
        field(
            cx,
            "manifest_screenshots",
            "custom",
            true,
            &k("manifest_screenshots.title"),
            Some(&escape(&k("manifest_screenshots.description"))),
            None,
            false,
            &format!(
                "<div class=\"form-kit__control-custom\">{}</div>",
                simple_list(cx, "manifest_screenshots-field", screenshots)
            ),
        ),
        image_field(cx, settings, "apple_touch_icon", true),
    );
    form.push_str(&card_section(
        "admin-logo-form__mobile-section",
        &k("mobile"),
        &mobile,
    ));
    form.push_str(&card_section(
        "admin-logo-form__email-section",
        &k("email"),
        &image_field(cx, settings, "digest_logo", true),
    ));
    form.push_str(&card_section(
        "admin-logo-form__social-media-section",
        &k("social_media"),
        &image_field(cx, settings, "opengraph_image", true),
    ));
    form.push_str(&submit(cx));
    // The values the form sends, as formData holds them.
    let data: serde_json::Map<String, Value> = LOGO_SETTINGS
        .iter()
        .filter_map(|name| setting(settings, name).map(|v| (name.to_string(), v["value"].clone())))
        .collect();
    page(
        cx,
        "logo",
        "/admin/config/logo",
        &format!(
            "<div class=\"loading-container\"><form class=\"form-kit admin-logo-form\" novalidate=\"\" data-values=\"{}\" data-upload-type=\"branding\">{form}</form></div>",
            escape(&Value::Object(data).to_string())
        ),
    )
}

fn submit(cx: &AdminCx) -> String {
    format!(
        "<button class=\"btn btn-primary form-kit__button\" type=\"submit\"><span class=\"d-button-label\">{}</span></button>",
        escape(&t(cx.i18n, "submit"))
    )
}

/// `MAIN_FONTS` and `MORE_FONTS` (admin/lib/constants).
pub const MAIN_FONTS: [(&str, &str); 9] = [
    ("open_sans", "Open Sans"),
    ("roboto", "Roboto"),
    ("lato", "Lato"),
    ("inter", "Inter"),
    ("montserrat", "Montserrat"),
    ("poppins", "Poppins"),
    ("merriweather", "Merriweather"),
    ("mukta", "Mukta"),
    ("helvetica", "Helvetica"),
];
pub const MORE_FONTS: [(&str, &str); 17] = [
    ("arial", "Arial"),
    ("system", "System"),
    ("oxanium", "Oxanium"),
    ("noto_sans_jp", "NotoSansJP"),
    ("roboto_condensed", "RobotoCondensed"),
    ("source_sans_pro", "SourceSansPro"),
    ("oswald", "Oswald"),
    ("raleway", "Raleway"),
    ("roboto_mono", "RobotoMono"),
    ("noto_sans", "NotoSans"),
    ("roboto_slab", "RobotoSlab"),
    ("ubuntu", "Ubuntu"),
    ("pt_sans", "PTSans"),
    ("playfair_display", "PlayfairDisplay"),
    ("nunito", "Nunito"),
    ("lora", "Lora"),
    ("jet_brains_mono", "JetBrains Mono"),
];
/// `DEFAULT_TEXT_SIZES`
pub const TEXT_SIZES: [&str; 5] = ["smallest", "smaller", "normal", "larger", "largest"];

/// A DButton with its block (no label: `no-text`).
fn option_button(class: &str, text: &str, value: &str) -> String {
    format!(
        "<button class=\"btn no-text {class}\" type=\"button\" data-value=\"{}\">{}</button>",
        escape(value),
        escape(text)
    )
}

/// AdminFontChooser: the main fonts, the more fonts when open (the
/// chosen font among them opens it), the toggle.
fn font_chooser(cx: &AdminCx, selected: &str) -> String {
    let more = MORE_FONTS.iter().any(|(k, _)| *k == selected);
    let button = |(key, name): &(&str, &str)| {
        option_button(
            &format!(
                "admin-fonts-form__button-option font btn-flat body-font-{}{}",
                key.replace('_', "-"),
                if *key == selected { " active" } else { "" }
            ),
            name,
            key,
        )
    };
    let mut out: String = MAIN_FONTS.iter().map(button).collect();
    let more_buttons: String = MORE_FONTS.iter().map(button).collect();
    if more {
        out.push_str(&more_buttons);
    }
    out.push_str(&format!(
        "<template class=\"admin-fonts-form__more-fonts\">{more_buttons}</template><button class=\"btn no-text btn-default admin-fonts-form__more\" type=\"button\" data-more=\"{}\" data-fewer=\"{}\">{}</button>",
        escape(&t(cx.i18n, "admin.config.fonts.form.more_fonts")),
        escape(&t(cx.i18n, "admin.config.fonts.form.fewer_fonts")),
        escape(&t(
            cx.i18n,
            if more {
                "admin.config.fonts.form.fewer_fonts"
            } else {
                "admin.config.fonts.form.more_fonts"
            }
        ))
    ));
    out
}

/// The fonts page: AdminFontsForm with the site's fonts and text size.
pub fn fonts(cx: &AdminCx) -> String {
    let get = |name: &str| cx.settings.get(name).map(|v| v.to_s()).unwrap_or_default();
    let (base, heading, size) = (
        get("base_font"),
        get("heading_font"),
        get("default_text_size"),
    );
    let k = |key: &str| t(cx.i18n, &format!("admin.config.fonts.form.{key}"));
    let custom = |inner: String| format!("<div class=\"form-kit__control-custom\">{inner}</div>");
    let sizes: String = TEXT_SIZES
        .iter()
        .map(|size_key| {
            option_button(
                &format!(
                    "admin-fonts-form__button-option text-size btn-flat {size_key}{}",
                    if *size_key == size { " active" } else { "" }
                ),
                size_key,
                size_key,
            )
        })
        .collect();
    let form = format!(
        "{}{}{}{}",
        field(
            cx,
            "base_font",
            "custom",
            true,
            &k("base_font.title"),
            None,
            None,
            true,
            &custom(font_chooser(cx, &base))
        ),
        field(
            cx,
            "heading_font",
            "custom",
            true,
            &k("heading_font.title"),
            None,
            None,
            true,
            &custom(font_chooser(cx, &heading))
        ),
        field(
            cx,
            "default_text_size",
            "custom",
            true,
            &k("default_text_size.title"),
            Some(&escape(&k("default_text_size.description"))),
            None,
            true,
            &custom(sizes)
        ),
        submit(cx)
    );
    page(
        cx,
        "fonts",
        "/admin/config/fonts",
        &format!(
            "<form class=\"form-kit admin-fonts-form\" novalidate=\"\" data-default-text-size=\"{}\">{form}</form>",
            escape(&size)
        ),
    )
}

/// The welcome banner's site texts (AdminWelcomeBannerForm's fields
/// and keys, with whether the field is required).
pub const WELCOME_BANNER_TEXTS: [(&str, &str, &str, bool); 6] = [
    (
        "headerNewMembers",
        "js.welcome_banner.header.new_members",
        "header_new_members",
        true,
    ),
    (
        "headerLoggedInMembers",
        "js.welcome_banner.header.logged_in_members",
        "header_logged_in",
        true,
    ),
    (
        "headerAnonymousMembers",
        "js.welcome_banner.header.anonymous_members",
        "header_anonymous",
        true,
    ),
    (
        "subheaderLoggedInMembers",
        "js.welcome_banner.subheader.logged_in_members",
        "subheader_logged_in",
        false,
    ),
    (
        "subheaderAnonymousMembers",
        "js.welcome_banner.subheader.anonymous_members",
        "subheader_anonymous",
        false,
    ),
    (
        "searchPlaceholder",
        "js.welcome_banner.search_placeholder",
        "search_placeholder",
        true,
    ),
];

/// What AdminWelcomeBannerForm loads: the themes (id, name, enabled),
/// the settings, the locale and its texts (by field name).
pub struct WelcomeBanner<'a> {
    pub themes: &'a [(i64, String, bool)],
    pub image: &'a str,
    pub text_color: &'a str,
    pub page_visibility: &'a str,
    pub location: &'a str,
    pub locale: &'a str,
    pub locales: &'a [(String, String)],
    pub texts: &'a [(String, String)],
}

/// FKControlColor with `@prefixHex` and named colors allowed.
fn color_control(name: &str, value: &str) -> String {
    let bare = value.trim_start_matches('#');
    let hex = bare.len() == 3 || bare.len() == 6;
    let valid = hex && bare.chars().all(|c| c.is_ascii_hexdigit());
    let light = if !valid {
        true
    } else {
        let full: String = if bare.len() == 3 {
            bare.chars().flat_map(|c| [c, c]).collect()
        } else {
            bare.to_string()
        };
        let channel = |i: usize| i64::from_str_radix(&full[i..i + 2], 16).unwrap_or(0) as f64;
        (0.299 * channel(0) + 0.587 * channel(2) + 0.114 * channel(4)) / 255.0 > 0.5
    };
    format!(
        "<div class=\"form-kit__control-color\"><div class=\"form-kit__control-color-input\"><span class=\"form-kit__control-color-input-prefix\">#</span><input class=\"form-kit__control-color-input-hex\" name=\"{name}\" type=\"text\" value=\"{}\"><span class=\"form-kit__control-color-picker-wrapper {}\"><input class=\"form-kit__control-color-input-picker\" type=\"color\" value=\"#{}\">{}</span></div></div>",
        escape(bare),
        if light { "--is-light" } else { "--is-dark" },
        if valid {
            escape(bare)
        } else {
            "000000".to_string()
        },
        icon("eye-dropper", None)
    )
}

/// A field with `@format="large"`.
fn large(html: &str, control: &str) -> String {
    html.replacen(
        &format!("form-kit__field-{control}\""),
        &format!("form-kit__field-{control} --large\""),
        1,
    )
}

/// FKControlInput, its value given.
fn text_input(name: &str, value: &str, placeholder: &str) -> String {
    format!(
        "<div class=\"form-kit__control-input-wrapper\"><input class=\"form-kit__control-input\" name=\"{name}\" placeholder=\"{}\" type=\"text\" value=\"{}\"></div>",
        escape(placeholder),
        escape(value)
    )
}

/// The welcome banner page: AdminWelcomeBannerForm.
pub fn welcome_banner(cx: &AdminCx, wb: &WelcomeBanner) -> String {
    let k = |key: &str| t(cx.i18n, &format!("admin.config.welcome_banner.form.{key}"));
    let select = |name: &str, key: &str, options: &[&str], selected: &str| {
        let opts: Vec<(String, String)> = options
            .iter()
            .map(|o| (o.to_string(), k(&format!("{key}.options.{o}"))))
            .collect();
        crate::admin_form_kit::select(name, &opts, selected, None, false)
    };
    let selection: Vec<(String, String)> = wb
        .themes
        .iter()
        .filter(|(_, _, on)| *on)
        .map(|(id, name, _)| (id.to_string(), name.clone()))
        .collect();
    let themes = Value::Array(
        wb.themes
            .iter()
            .map(|(id, name, _)| serde_json::json!({ "value": id.to_string(), "name": name }))
            .collect(),
    );
    let mut form = String::new();
    form.push_str(&field(
        cx,
        "enabledThemes",
        "custom",
        false,
        &k("enabled_themes.label"),
        Some(&escape(&k("enabled_themes.description"))),
        None,
        false,
        &format!(
            "<div class=\"form-kit__control-custom\">{}</div>",
            admin_ui::multi_select_trigger(
                "admin-welcome-banner-form__themes",
                &k("enabled_themes.select_label"),
                &selection
            )
        ),
    ));
    form.push_str(&field(
        cx,
        "welcomeBannerImage",
        "image",
        false,
        &k("background_image.label"),
        Some(&escape(&k("background_image.description"))),
        None,
        false,
        &image_uploader(
            cx,
            "welcomeBannerImage-field-welcomeBannerImage",
            wb.image,
            None,
            None,
        ),
    ));
    form.push_str(&field(
        cx,
        "welcomeBannerTextColor",
        "color",
        false,
        &k("text_color.label"),
        Some(&escape(&k("text_color.description"))),
        None,
        false,
        &color_control("welcomeBannerTextColor", wb.text_color),
    ));
    form.push_str(&field(
        cx,
        "welcomeBannerPageVisibility",
        "select",
        false,
        &k("page_visibility.label"),
        Some(&escape(&k("page_visibility.description"))),
        None,
        false,
        &select(
            "welcomeBannerPageVisibility",
            "page_visibility",
            &["top_menu_pages", "homepage", "discovery", "all_pages"],
            wb.page_visibility,
        ),
    ));
    form.push_str(&field(
        cx,
        "welcomeBannerLocation",
        "select",
        false,
        &k("location.label"),
        Some(&escape(&k("location.description"))),
        None,
        false,
        &select(
            "welcomeBannerLocation",
            "location",
            &["above_topic_content", "below_site_header"],
            wb.location,
        ),
    ));
    let mut section = field(
        cx,
        "localeSelector",
        "custom",
        false,
        &k("text_section.locale_label"),
        None,
        None,
        true,
        &format!(
            "<div class=\"form-kit__control-custom\">{}</div>",
            admin_ui::combo_box(
                cx,
                "translation-selector",
                wb.locales,
                Some(wb.locale),
                None
            )
        ),
    );
    for (name, _, key, required) in WELCOME_BANNER_TEXTS {
        let value = wb
            .texts
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .unwrap_or_default();
        let control = if name.starts_with("subheader") {
            "textarea"
        } else {
            "input"
        };
        let inner = if control == "textarea" {
            format!(
                "<textarea class=\"form-kit__control-textarea\" name=\"{name}\">{}</textarea>",
                escape(value)
            )
        } else {
            let placeholder = crate::admin_view::t_opt(
                cx.i18n,
                &format!("admin.config.welcome_banner.form.{key}.placeholder"),
            )
            .unwrap_or_default();
            text_input(name, value, &placeholder)
        };
        // The headers' descriptions are HTML (trustHTML).
        let description = t(
            cx.i18n,
            &format!("admin.config.welcome_banner.form.{key}.description"),
        );
        let description = if key.starts_with("header_") {
            description
        } else {
            escape(&description)
        };
        let html = field(
            cx,
            name,
            control,
            false,
            &k(&format!("{key}.label")),
            Some(&description),
            None,
            required,
            &inner,
        );
        section.push_str(&large(&html, control));
    }
    let mut form = large(&form, "custom");
    form = form.replacen(
        "form-kit__field-color\"",
        "form-kit__field-color --large\"",
        1,
    );
    let section = large(&section, "custom");
    form.push_str(&format!(
        "<div class=\"form-kit__section\"><div class=\"form-kit__section-header\"><div class=\"form-kit__section-heading\"><h2 class=\"form-kit__section-title\">{}</h2></div></div>{section}</div>",
        escape(&k("text_section.title"))
    ));
    form.push_str(&submit(cx));
    let original: serde_json::Map<String, Value> = wb
        .texts
        .iter()
        .map(|(n, v)| (n.clone(), Value::String(v.clone())))
        .chain([
            (
                "welcomeBannerImage".to_string(),
                Value::String(wb.image.to_string()),
            ),
            (
                "welcomeBannerTextColor".to_string(),
                Value::String(wb.text_color.to_string()),
            ),
            (
                "welcomeBannerPageVisibility".to_string(),
                Value::String(wb.page_visibility.to_string()),
            ),
            (
                "welcomeBannerLocation".to_string(),
                Value::String(wb.location.to_string()),
            ),
        ])
        .collect();
    let title = t(cx.i18n, "admin.config.welcome_banner.title");
    let header = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path: "/admin/config/welcome-banner",
            },
        ],
        title,
        actions: String::new(),
        description: Some(t(cx.i18n, "admin.config.welcome_banner.header_description")),
        learn_more_url: None,
        tabs: None,
    }
    .render(cx);
    format!(
        "<div class=\"admin-config-page\">{header}<div class=\"admin-config-area\"><div class=\"admin-config-area__primary-content\"><div class=\"loading-container\"><form class=\"form-kit admin-welcome-banner-form\" novalidate=\"\" data-original=\"{o}\" data-values=\"{o}\" data-upload-type=\"site_setting\" data-themes=\"{}\">{form}</form></div></div></div></div>",
        escape(&themes.to_string()),
        o = escape(&Value::Object(original).to_string()),
    )
}

/// The strings admin-branding.js shows.
pub const STRINGS: &[&str] = &[
    "admin.config.logo.form.saved",
    "admin.config.fonts.form.saved",
    "admin.config.fonts.backfill_modal.title",
    "admin.config.fonts.backfill_modal.description.one",
    "admin.config.fonts.backfill_modal.description.other",
    "admin.config.fonts.backfill_modal.modal_yes",
    "admin.config.fonts.backfill_modal.modal_no",
    "upload_selector.select_file",
    "upload_selector.change",
    "upload_selector.delete",
    "upload_selector.uploading",
    "expand",
    "generic_error",
    "admin.config.welcome_banner.saved",
    "multi_select.search",
    "multi_select.no_results",
];

/// LogoController#update's settings, in the order it names them.
pub const LOGO_SETTINGS: [&str; 15] = [
    "logo",
    "logo_dark",
    "large_icon",
    "favicon",
    "logo_small",
    "logo_small_dark",
    "mobile_logo",
    "mobile_logo_dark",
    "manifest_icon",
    "manifest_screenshots",
    "apple_touch_icon",
    "digest_logo",
    "generate_topic_og_image",
    "opengraph_image",
    "x_summary_large_image",
];
