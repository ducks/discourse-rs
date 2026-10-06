//! The importer's font definitions (Stylesheet::Importer#font): the base
//! and heading fonts the site settings name, and JetBrains Mono for code,
//! from the discourse-fonts gem's table (0.0.19). Rails puts them in the
//! color definitions file.

/// `DiscourseFonts.fonts`, keyed as the gem keys them.
pub struct Font {
    pub key: &'static str,
    pub name: &'static str,
    pub stack: &'static str,
    pub feature_settings: Option<&'static str>,
    pub variation_settings: Option<&'static str>,
    pub variant_ligatures: Option<&'static str>,
    /// (filename, weight)
    pub variants: &'static [(&'static str, &'static str)],
}

/// `DiscourseFonts::VERSION`, on the font urls.
pub const VERSION: &str = "0.0.19";

pub const FONTS: &[Font] = &[
    Font {
        key: "arial",
        name: "Arial",
        stack: "Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[],
    },
    Font {
        key: "system",
        name: "System",
        stack: "system-ui, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[],
    },
    Font {
        key: "open_sans",
        name: "Open Sans",
        stack: "Open Sans, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[
            ("OpenSans-Regular.woff2", "400"),
            ("OpenSans-Bold.woff2", "700"),
        ],
    },
    Font {
        key: "oxanium",
        name: "Oxanium",
        stack: "Oxanium, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[
            ("oxanium-regular.woff2", "400"),
            ("oxanium-bold.woff2", "700"),
        ],
    },
    Font {
        key: "roboto",
        name: "Roboto",
        stack: "Roboto, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[
            ("Roboto-Regular.woff2", "400"),
            ("Roboto-Bold.woff2", "700"),
        ],
    },
    Font {
        key: "lato",
        name: "Lato",
        stack: "Lato, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[("Lato-Regular.woff2", "400"), ("Lato-Bold.woff2", "700")],
    },
    Font {
        key: "inter",
        name: "Inter",
        stack: "Inter, Arial, sans-serif",
        feature_settings: Some("'calt' 0"),
        variation_settings: None,
        variant_ligatures: None,
        variants: &[("InterVariable.woff2", "100 900")],
    },
    Font {
        key: "noto_sans_jp",
        name: "NotoSansJP",
        stack: "NotoSansJP, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[
            ("NotoSansJP-Regular.woff2", "400"),
            ("NotoSansJP-Bold.woff2", "700"),
        ],
    },
    Font {
        key: "montserrat",
        name: "Montserrat",
        stack: "Montserrat, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[
            ("Montserrat-Regular.woff2", "400"),
            ("Montserrat-Bold.woff2", "700"),
        ],
    },
    Font {
        key: "roboto_condensed",
        name: "RobotoCondensed",
        stack: "RobotoCondensed, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[
            ("RobotoCondensed-Regular.woff2", "400"),
            ("RobotoCondensed-Bold.woff2", "700"),
        ],
    },
    Font {
        key: "source_sans_pro",
        name: "SourceSansPro",
        stack: "SourceSansPro, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[
            ("SourceSansPro-Regular.woff2", "400"),
            ("SourceSansPro-Bold.woff2", "700"),
        ],
    },
    Font {
        key: "oswald",
        name: "Oswald",
        stack: "Oswald, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[
            ("Oswald-Regular.woff2", "400"),
            ("Oswald-Bold.woff2", "700"),
        ],
    },
    Font {
        key: "raleway",
        name: "Raleway",
        stack: "Raleway, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[
            ("Raleway-Regular.woff2", "400"),
            ("Raleway-Bold.woff2", "700"),
        ],
    },
    Font {
        key: "roboto_mono",
        name: "RobotoMono",
        stack: "RobotoMono, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[
            ("RobotoMono-Regular.woff2", "400"),
            ("RobotoMono-Bold.woff2", "700"),
        ],
    },
    Font {
        key: "poppins",
        name: "Poppins",
        stack: "Poppins, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[
            ("Poppins-Regular.woff2", "400"),
            ("Poppins-Bold.woff2", "700"),
        ],
    },
    Font {
        key: "noto_sans",
        name: "NotoSans",
        stack: "NotoSans, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[
            ("NotoSans-Regular.woff2", "400"),
            ("NotoSans-Bold.woff2", "700"),
        ],
    },
    Font {
        key: "roboto_slab",
        name: "RobotoSlab",
        stack: "RobotoSlab, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[
            ("RobotoSlab-Regular.woff2", "400"),
            ("RobotoSlab-Bold.woff2", "700"),
        ],
    },
    Font {
        key: "merriweather",
        name: "Merriweather",
        stack: "Merriweather, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[
            ("Merriweather-Regular.woff2", "400"),
            ("Merriweather-Bold.woff2", "700"),
        ],
    },
    Font {
        key: "ubuntu",
        name: "Ubuntu",
        stack: "Ubuntu, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[
            ("Ubuntu-Regular.woff2", "400"),
            ("Ubuntu-Bold.woff2", "700"),
        ],
    },
    Font {
        key: "pt_sans",
        name: "PTSans",
        stack: "PTSans, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[
            ("PTSans-Regular.woff2", "400"),
            ("PTSans-Bold.woff2", "700"),
        ],
    },
    Font {
        key: "playfair_display",
        name: "PlayfairDisplay",
        stack: "PlayfairDisplay, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[
            ("PlayfairDisplay-Regular.woff2", "400"),
            ("PlayfairDisplay-Bold.woff2", "700"),
        ],
    },
    Font {
        key: "nunito",
        name: "Nunito",
        stack: "Nunito, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[
            ("Nunito-Regular.woff2", "400"),
            ("Nunito-Bold.woff2", "700"),
        ],
    },
    Font {
        key: "lora",
        name: "Lora",
        stack: "Lora, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[("Lora-Regular.woff2", "400"), ("Lora-Bold.woff2", "700")],
    },
    Font {
        key: "mukta",
        name: "Mukta",
        stack: "Mukta, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[("Mukta-Regular.woff2", "400"), ("Mukta-Bold.woff2", "700")],
    },
    Font {
        key: "helvetica",
        name: "Helvetica",
        stack: "Helvetica, Arial, sans-serif",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: None,
        variants: &[],
    },
    Font {
        key: "jet_brains_mono",
        name: "JetBrains Mono",
        stack: "JetBrains Mono, Consolas, Monaco, monospace",
        feature_settings: None,
        variation_settings: None,
        variant_ligatures: Some("none"),
        variants: &[
            ("JetBrainsMono-Regular.woff2", "400"),
            ("JetBrainsMono-Bold.woff2", "700"),
        ],
    },
];

pub fn find(key: &str) -> Option<&'static Font> {
    FONTS.iter().find(|font| font.key == key)
}

/// `font_css`: a face per variant, the file under `fonts_dir`.
fn faces(font: &Font, fonts_dir: &str) -> String {
    font.variants
        .iter()
        .map(|(filename, weight)| {
            format!(
                "@font-face {{\n  font-family: '{}';\n  src: url(\"{fonts_dir}/{filename}?v={VERSION}\") format(\"woff2\");\n  font-weight: {weight};\n}}\n",
                font.name
            )
        })
        .collect()
}

/// `render_font_special_properties`
fn special_properties(font: &Font, elements: &str) -> String {
    let mut out = format!("{elements} {{\n");
    out.push_str(&format!(
        "  font-variation-settings: {};\n",
        font.variation_settings.unwrap_or("normal")
    ));
    out.push_str(&format!(
        "  font-feature-settings: {};\n",
        font.feature_settings.unwrap_or("normal")
    ));
    if let Some(ligatures) = font.variant_ligatures {
        out.push_str(&format!("  font-variant-ligatures: {ligatures};\n"));
    }
    out.push_str("}\n");
    out
}

/// `font`: the body font, the heading font and the monospace one.
pub fn css(base_font: &str, heading_font: &str, fonts_dir: &str) -> String {
    let mut out = String::new();
    if let Some(font) = find(base_font) {
        out.push_str(&faces(font, fonts_dir));
        out.push_str(&special_properties(font, "html"));
        out.push_str(&format!(":root {{\n  --font-family: {};\n}}\n", font.stack));
    }
    if let Some(font) = find(heading_font) {
        out.push_str(&faces(font, fonts_dir));
        out.push_str(&special_properties(font, "h1, h2, h3, h4, h5, h6"));
        out.push_str(&format!(
            ":root {{\n  --heading-font-family: {};\n}}\n",
            font.stack
        ));
    }
    let mono = find("jet_brains_mono").expect("the gem has JetBrains Mono");
    out.push_str(&faces(mono, fonts_dir));
    out.push_str(&special_properties(mono, "html"));
    out.push_str(&format!(
        ":root {{\n  --d-font-family--monospace: {};\n}}\n",
        mono.stack
    ));
    out
}
