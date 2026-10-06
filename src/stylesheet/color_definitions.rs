//! color_definitions.scss with color_transformations.scss: a color
//! scheme's colors as the custom properties every other stylesheet reads.
//! What plugins add to the file (chat, calendar) and the font faces are not
//! part of it.

use super::sass_color::{Color, number};

/// A scheme's base colors, by their names in colors.scss.
pub struct SchemeColors {
    pub primary: Color,
    pub secondary: Color,
    pub tertiary: Color,
    pub quaternary: Color,
    pub header_background: Color,
    pub header_primary: Color,
    pub highlight: Color,
    pub selected: Color,
    pub hover: Color,
    pub danger: Color,
    pub success: Color,
    pub love: Color,
}

impl SchemeColors {
    /// From `name → hex` pairs (a resolved scheme), each falling back to
    /// colors.scss.
    pub fn from_hex(get: impl Fn(&str) -> Option<String>) -> SchemeColors {
        let color = |name: &str, default: &str| {
            get(name)
                .and_then(|hex| Color::hex(&hex))
                .unwrap_or_else(|| Color::hex(default).unwrap())
        };
        SchemeColors {
            primary: color("primary", "#222"),
            secondary: color("secondary", "#fff"),
            tertiary: color("tertiary", "#08c"),
            quaternary: color("quaternary", "#e45735"),
            header_background: color("header_background", "#fff"),
            header_primary: color("header_primary", "#333"),
            highlight: color("highlight", "#ffff4d"),
            selected: color("selected", "#d1f0ff"),
            hover: color("hover", "#f2f2f2"),
            danger: color("danger", "#c80001"),
            success: color("success", "#090"),
            love: color("love", "#fa6c8d"),
        }
    }
}

/// `dark-light-diff`
fn diff(adjusted: &Color, comparison: &Color, lightness: f64, darkness: f64) -> Color {
    if adjusted.brightness() < comparison.brightness() {
        adjusted.scale_lightness(lightness / 100.0)
    } else {
        adjusted.scale_lightness(darkness / 100.0)
    }
}

/// The `:root` block of color_definitions.scss.
pub fn css(c: &SchemeColors) -> String {
    let light = c.primary.brightness() < c.secondary.brightness();
    // dark-light-choose
    let choose = |l: &Color, d: &Color| if light { l.clone() } else { d.clone() };
    let choose_text = |l: &str, d: &str| if light { l.to_string() } else { d.to_string() };
    let hex = |s: &str| Color::hex(s).unwrap();
    let (p, s, t) = (&c.primary, &c.secondary, &c.tertiary);

    let primary_very_low = diff(p, s, 97.0, -82.0);
    let primary_low = diff(p, s, 90.0, -78.0);
    let primary_low_mid = diff(p, s, 70.0, -45.0);
    let primary_medium = diff(p, s, 50.0, -35.0);
    let primary_high = diff(p, s, 30.0, -25.0);
    let primary_very_high = diff(p, s, 15.0, -10.0);
    let primary_n = [
        (50, 97.0, -82.0),
        (100, 94.0, -80.0),
        (200, 90.0, -78.0),
        (300, 80.0, -60.0),
        (400, 70.0, -45.0),
        (500, 60.0, -40.0),
        (600, 50.0, -35.0),
        (700, 38.0, -30.0),
        (800, 30.0, -25.0),
        (900, 15.0, -10.0),
    ]
    .map(|(n, l, d)| (n, diff(p, s, l, d)));
    let primary_n_get = |n: i32| primary_n.iter().find(|(k, _)| *k == n).unwrap().1.clone();
    let header = |percent: f64| Color::srgb_scale(&c.header_primary, &c.header_background, percent);

    let secondary_low = diff(s, p, 70.0, -70.0);
    let secondary_medium = diff(s, p, 50.0, -50.0);
    let secondary_high = diff(s, p, 30.0, -35.0);
    let secondary_very_high = diff(s, p, 7.0, -7.0);

    let tertiary_very_low = diff(t, s, 90.0, -75.0);
    let tertiary_low = diff(t, s, 85.0, -65.0);
    let tertiary_medium = diff(t, s, 50.0, -45.0);
    let tertiary_high = diff(t, s, 20.0, -25.0);
    let tertiary_hover = choose(&diff(t, s, -25.0, -25.0), &diff(t, p, 20.0, 70.0));
    let tertiary_n = [
        (25, 93.0, -80.0),
        (50, 90.0, -75.0),
        (100, 88.0, -72.0),
        (200, 87.0, -69.0),
        (300, 85.0, -65.0),
        (400, 74.0, -58.0),
        (500, 63.0, -52.0),
        (600, 50.0, -45.0),
        (700, 40.0, -38.0),
        (800, 30.0, -31.0),
        (900, 20.0, -25.0),
    ]
    .map(|(n, l, d)| (n, diff(t, s, l, d)));

    let quaternary_low = diff(&c.quaternary, s, 70.0, -70.0);
    let highlight_bg = diff(&c.highlight, s, 70.0, -80.0);
    let highlight_low = diff(&c.highlight, s, 70.0, -80.0);
    let highlight_medium = diff(&c.highlight, s, 50.0, -55.0);
    let highlight_high = diff(&c.highlight, s, -50.0, -10.0);
    let danger_low = diff(&c.danger, s, 90.0, -68.0);
    let danger_low_mid = diff(&c.danger.with_alpha(0.7), s, 50.0, -60.0);
    let danger_medium = diff(&c.danger, s, 30.0, -35.0);
    let danger_hover = diff(&c.danger, s, -20.0, -20.0);
    let success_very_low = diff(&c.success, s, 90.0, -70.0);
    let success_low = diff(&c.success, s, 80.0, -60.0);
    let success_medium = diff(&c.success, s, 50.0, -40.0);
    let success_hover = diff(&c.success, s, -20.0, -20.0);
    let love_low = diff(&c.love, s, 85.0, -60.0);
    let selected_hover = diff(&c.selected, s, 20.0, 20.0);
    let blend_primary_secondary_5 = Color::srgb_scale(p, s, 5.0);

    // variables.scss
    let google = hex("#fff");
    let instagram = hex("#e1306c");
    let facebook = hex("#0866ff");
    let github = hex("#100e0f");
    let discord = hex("#7289da");
    let twitter = hex("#000");
    let gold = Color::written_rgb(231, 195, 0, "rgb(231, 195, 0)");

    let inline_code_bg = if light {
        primary_n_get(100).to_css()
    } else {
        // Sass reads rgb() with an alpha and writes it back as rgba().
        "rgba(0, 0, 0, 0.35)".to_string()
    };
    let opacity = |l: f64, d: f64| number(if light { l } else { d });

    let mut vars: Vec<(String, String)> = Vec::new();
    let mut put = |name: &str, value: String| vars.push((name.to_string(), value));
    let scheme = if light { "light" } else { "dark" };
    put("--scheme-type", scheme.to_string());
    put("--primary", p.to_css());
    put("--secondary", s.to_css());
    put("--tertiary", t.to_css());
    put("--quaternary", c.quaternary.to_css());
    put("--header_background", c.header_background.to_css());
    put("--header_primary", c.header_primary.to_css());
    put("--highlight", c.highlight.to_css());
    put("--danger", c.danger.to_css());
    put("--success", c.success.to_css());
    put("--love", c.love.to_css());
    put("--d-selected", c.selected.to_css());
    put("--d-selected-text-color", "var(--primary)".into());
    put("--d-selected-hover", selected_hover.to_css());
    put("--d-hover", c.hover.to_css());
    put("--always-black-rgb", "0, 0, 0".into());
    put("--primary-rgb", p.to_rgb_list());
    put("--primary-low-rgb", primary_low.to_rgb_list());
    put("--primary-very-low-rgb", primary_very_low.to_rgb_list());
    put("--secondary-rgb", s.to_rgb_list());
    put("--header_background-rgb", c.header_background.to_rgb_list());
    put("--tertiary-rgb", t.to_rgb_list());
    put("--highlight-rgb", c.highlight.to_rgb_list());
    put("--success-rgb", c.success.to_rgb_list());
    put("--primary-very-low", primary_very_low.to_css());
    put("--primary-low", primary_low.to_css());
    put("--primary-low-mid", primary_low_mid.to_css());
    put("--primary-medium", primary_medium.to_css());
    put("--primary-high", primary_high.to_css());
    put("--primary-very-high", primary_very_high.to_css());
    for (n, color) in &primary_n {
        put(&format!("--primary-{n}"), color.to_css());
    }
    put("--header_primary-low", header(10.0).to_css());
    put("--header_primary-low-mid", header(35.0).to_css());
    put("--header_primary-medium", header(55.0).to_css());
    put("--header_primary-high", header(70.0).to_css());
    put("--header_primary-very-high", header(90.0).to_css());
    put("--secondary-low", secondary_low.to_css());
    put("--secondary-medium", secondary_medium.to_css());
    put("--secondary-high", secondary_high.to_css());
    put("--secondary-very-high", secondary_very_high.to_css());
    put("--tertiary-very-low", tertiary_very_low.to_css());
    put("--tertiary-low", tertiary_low.to_css());
    put("--tertiary-medium", tertiary_medium.to_css());
    put("--tertiary-high", tertiary_high.to_css());
    put("--tertiary-hover", tertiary_hover.to_css());
    for (n, color) in &tertiary_n {
        put(&format!("--tertiary-{n}"), color.to_css());
    }
    put("--quaternary-low", quaternary_low.to_css());
    put("--highlight-bg", highlight_bg.to_css());
    put("--highlight-low", highlight_low.to_css());
    put("--highlight-medium", highlight_medium.to_css());
    put("--highlight-high", highlight_high.to_css());
    put("--danger-low", danger_low.to_css());
    put("--danger-low-mid", danger_low_mid.to_css());
    put("--danger-medium", danger_medium.to_css());
    put("--danger-hover", danger_hover.to_css());
    put("--success-very-low", success_very_low.to_css());
    put("--success-low", success_low.to_css());
    put("--success-medium", success_medium.to_css());
    put("--success-hover", success_hover.to_css());
    put("--love-low", love_low.to_css());
    put("--wiki", "green".into());
    put(
        "--blend-primary-secondary-5",
        blend_primary_secondary_5.to_css(),
    );
    put(
        "--primary-med-or-secondary-med",
        choose(&primary_medium, &secondary_medium).to_css(),
    );
    put(
        "--primary-med-or-secondary-high",
        choose(&primary_medium, &secondary_high).to_css(),
    );
    put(
        "--primary-high-or-secondary-low",
        choose(&primary_high, &secondary_low).to_css(),
    );
    put(
        "--primary-low-mid-or-secondary-high",
        choose(&primary_low_mid, &secondary_high).to_css(),
    );
    put(
        "--primary-low-mid-or-secondary-low",
        choose(&primary_low_mid, &secondary_low).to_css(),
    );
    put(
        "--primary-or-primary-low-mid",
        choose(p, &primary_low_mid).to_css(),
    );
    put(
        "--highlight-low-or-medium",
        choose(&highlight_low, &highlight_medium).to_css(),
    );
    put(
        "--tertiary-or-tertiary-low",
        choose(t, &tertiary_low).to_css(),
    );
    put(
        "--tertiary-low-or-tertiary-high",
        choose(&tertiary_low, &tertiary_high).to_css(),
    );
    put(
        "--tertiary-med-or-tertiary",
        choose(&tertiary_medium, t).to_css(),
    );
    put("--secondary-or-primary", choose(s, p).to_css());
    put("--tertiary-or-white", choose_text(&t.to_css(), "#fff"));
    put(
        "--facebook-or-white",
        choose_text(&facebook.to_css(), "#fff"),
    );
    put("--twitter-or-white", choose_text(&twitter.to_css(), "#fff"));
    put("--hljs-attr", choose_text("#015692", "#88aece"));
    put("--hljs-attribute", choose_text("#803378", "#c59bc1"));
    put("--hljs-addition", choose_text("#2f6f44", "#76c490"));
    put("--hljs-bg", inline_code_bg.clone());
    put("--inline-code-bg", inline_code_bg);
    put("--hljs-comment", primary_n_get(500).to_css());
    put("--hljs-deletion", choose_text("#c02d2e", "#de7176"));
    put("--hljs-keyword", choose_text("#015692", "#88aece"));
    put("--hljs-title", choose_text("#b75501", "#f08d49"));
    put("--hljs-name", choose_text("#b75501", "#f08d49"));
    put("--hljs-punctuation", choose_text("#535a60", "#ccc"));
    put("--hljs-symbol", choose_text("#54790d", "#b5bd68"));
    put("--hljs-variable", choose_text("#54790d", "#b5bd68"));
    put("--hljs-string", choose_text("#54790d", "#b5bd68"));
    put("--google", google.to_css());
    put("--google-hover", google.adjust_lightness(-5.0).to_css());
    put("--instagram", instagram.to_css());
    put(
        "--instagram-hover",
        instagram.adjust_lightness(-15.0).to_css(),
    );
    put("--facebook", facebook.to_css());
    put(
        "--facebook-hover",
        facebook.adjust_lightness(-15.0).to_css(),
    );
    put("--cas", hex("#70ba61").to_css());
    put("--twitter", twitter.to_css());
    put("--github", github.to_css());
    put("--github-hover", github.adjust_lightness(20.0).to_css());
    put("--discord", discord.to_css());
    put("--discord-hover", discord.adjust_lightness(-10.0).to_css());
    for (name, suffix) in [
        ("text", "text-color"),
        ("text-hover", "text-color--hover"),
        ("background", "bg-color"),
        ("background-hover", "bg-color--hover"),
        ("icon", "icon-color"),
        ("icon-hover", "icon-color--hover"),
    ] {
        put(
            &format!("--discourse_id-{name}"),
            format!("var(--d-button-primary-{suffix})"),
        );
    }
    put("--discourse_id-border", "var(--d-button-border)".into());
    put("--gold", gold.to_css());
    put("--silver", hex("#c0c0c0").to_css());
    put("--bronze", hex("#cd7f32").to_css());
    for (name, value) in [
        ("--d-link-color", "var(--tertiary)"),
        ("--title-color--read", "var(--primary-medium)"),
        ("--content-border-color", "var(--primary-low)"),
        ("--input-border-color", "var(--primary-400)"),
        ("--table-border-color", "var(--content-border-color)"),
        ("--metadata-color", "var(--primary-medium)"),
        ("--d-badge-card-background-color", "var(--secondary)"),
        ("--mention-background-color", "var(--primary-low)"),
        ("--title-color", "var(--primary)"),
        ("--title-color--header", "var(--header_primary)"),
        ("--excerpt-color", "var(--primary-high)"),
    ] {
        put(name, value.to_string());
    }
    put(
        "--shadow-modal",
        format!("0 8px 60px rgba(0, 0, 0, {})", opacity(0.6, 1.0)),
    );
    put(
        "--shadow-composer",
        format!("0 -1px 40px rgba(0, 0, 0, {})", opacity(0.22, 0.45)),
    );
    put(
        "--shadow-card",
        format!("0 4px 14px rgba(0, 0, 0, {})", opacity(0.15, 0.5)),
    );
    put(
        "--shadow-dropdown",
        format!("0 2px 12px 0 rgba(0, 0, 0, {})", opacity(0.1, 0.25)),
    );
    put("--shadow-menu-panel", "var(--shadow-dropdown)".into());
    put(
        "--shadow-header",
        "0 0 0 1px var(--content-border-color)".into(),
    );
    put(
        "--shadow-footer-nav",
        format!("0 0 2px 0 rgba(0, 0, 0, {})", opacity(0.2, 0.4)),
    );
    put("--shadow-focus-danger", "0 0 6px 0 var(--danger)".into());
    put(
        "--shadow-docked-composer",
        "0 4px 30px rgb(0, 0, 0, 0.15)".into(),
    );
    put(
        "--float-kit-arrow-stroke-color",
        "var(--primary-low)".into(),
    );
    put("--float-kit-arrow-fill-color", "var(--secondary)".into());
    put(
        "--topic-timeline-border-color",
        choose(&tertiary_low, &tertiary_high).to_css(),
    );
    put(
        "--topic-timeline-handle-color",
        "light-dark(var(--tertiary-400), var(--tertiary))".into(),
    );

    let body: Vec<String> = vars.iter().map(|(n, v)| format!("{n}: {v};")).collect();
    format!(
        ":root {{\n  color-scheme: {scheme};\n  {}\n}}\n",
        body.join("\n  ")
    )
}

/// The custom properties of a stylesheet as name → value, for comparing
/// with Rails' compiled color definitions.
pub fn properties(css: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for declaration in css.split([';', '{', '}']) {
        let declaration = declaration.trim();
        if let Some(rest) = declaration.strip_prefix("--")
            && let Some((name, value)) = rest.split_once(':')
        {
            out.push((format!("--{}", name.trim()), value.trim().to_string()));
        }
    }
    out
}
