//! `Sanitize::Config` and the built-in configs (lib/sanitize/config/*.rb).
//!
//! Ruby's configs are hashes deep-merged over DEFAULT; here [`Config`]'s
//! `Default` is DEFAULT and the others are built on it the way
//! `Config.merge` would. Symbols (`:all`, `:data`, `:relative`) are
//! fields, so no attribute or protocol name can be mistaken for one.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub use super::relaxed_properties::RELAXED_CSS_PROPERTIES;
use super::{ProtocolMap, Transformer, set};

/// Allowed attribute names, and whether any valid `data-*` attribute is
/// allowed too (`:data`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AttrAllow {
    pub names: HashSet<String>,
    pub data: bool,
}

impl AttrAllow {
    pub fn new(names: &[&str]) -> AttrAllow {
        AttrAllow {
            names: set(names),
            data: false,
        }
    }

    /// `[:data]` and nothing else.
    pub fn data() -> AttrAllow {
        AttrAllow {
            names: HashSet::new(),
            data: true,
        }
    }

    pub fn with_data(mut self) -> AttrAllow {
        self.data = true;
        self
    }
}

/// Allowed URL protocols; `relative` is `:relative` (no protocol).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Protocols {
    pub schemes: HashSet<String>,
    pub relative: bool,
}

impl Protocols {
    pub fn new(schemes: &[&str]) -> Protocols {
        Protocols {
            schemes: set(schemes),
            relative: false,
        }
    }

    pub fn relative(mut self) -> Protocols {
        self.relative = true;
        self
    }
}

/// `:remove_contents`: true/false, or the elements whose contents go
/// with them.
#[derive(Debug, Clone, PartialEq)]
pub enum RemoveContents {
    All(bool),
    Elements(HashSet<String>),
}

/// `:parser_options` (Gumbo's limits; -1 disables one).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParserOptions {
    pub max_tree_depth: i64,
    pub max_attributes: i64,
}

impl Default for ParserOptions {
    fn default() -> ParserOptions {
        ParserOptions {
            max_tree_depth: 400,
            max_attributes: 400,
        }
    }
}

/// `@import` URL validator (`:import_url_validator`).
pub type UrlValidator = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// `config[:css]`
#[derive(Clone, Default)]
pub struct CssConfig {
    pub allow_comments: bool,
    pub allow_hacks: bool,
    pub at_rules: HashSet<String>,
    pub at_rules_with_properties: HashSet<String>,
    pub at_rules_with_styles: HashSet<String>,
    pub import_url_validator: Option<UrlValidator>,
    pub properties: HashSet<String>,
    pub protocols: Protocols,
}

/// Sanitize's config; `Config::default()` is `Sanitize::Config::DEFAULT`.
#[derive(Clone)]
pub struct Config {
    /// Attributes added to elements, in order (`:add_attributes`).
    pub add_attributes: HashMap<String, Vec<(String, String)>>,
    pub allow_comments: bool,
    pub allow_doctype: bool,
    /// Per-element attributes; `attributes_all` is the `:all` key.
    pub attributes: HashMap<String, AttrAllow>,
    pub attributes_all: Option<AttrAllow>,
    /// `:css`. A config without one sanitizes CSS with DEFAULT's CSS
    /// settings and its own `allow_comments`, as `Sanitize::CSS.new`
    /// merges the whole config over them.
    pub css: Option<CssConfig>,
    pub elements: HashSet<String>,
    pub parser_options: ParserOptions,
    pub protocols: ProtocolMap,
    pub remove_contents: RemoveContents,
    pub transformers: Vec<Transformer>,
    /// Elements replaced by text before and after their contents.
    pub whitespace_elements: HashMap<String, (String, String)>,
}

impl Default for Config {
    fn default() -> Config {
        let whitespace = [
            "address",
            "article",
            "aside",
            "blockquote",
            "br",
            "dd",
            "div",
            "dl",
            "dt",
            "footer",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
            "header",
            "hgroup",
            "hr",
            "li",
            "nav",
            "ol",
            "p",
            "pre",
            "section",
            "ul",
        ];
        Config {
            add_attributes: HashMap::new(),
            allow_comments: false,
            allow_doctype: false,
            attributes: HashMap::new(),
            attributes_all: None,
            css: None,
            elements: HashSet::new(),
            parser_options: ParserOptions::default(),
            protocols: HashMap::new(),
            remove_contents: RemoveContents::Elements(set(&[
                "iframe",
                "math",
                "noembed",
                "noframes",
                "noscript",
                "plaintext",
                "script",
                "style",
                "svg",
                "xmp",
            ])),
            transformers: Vec::new(),
            whitespace_elements: whitespace
                .iter()
                .map(|e| (e.to_string(), (" ".to_string(), " ".to_string())))
                .collect(),
        }
    }
}

impl Config {
    /// The CSS settings `Sanitize::CSS.new(config)` ends up with.
    pub fn effective_css(&self) -> CssConfig {
        match &self.css {
            Some(css) => css.clone(),
            None => CssConfig {
                allow_comments: self.allow_comments,
                ..CssConfig::default()
            },
        }
    }

    /// Sets an element's attribute protocols.
    pub fn set_protocols(&mut self, element: &str, attribute: &str, protocols: Protocols) {
        self.protocols
            .entry(element.to_string())
            .or_default()
            .insert(attribute.to_string(), protocols);
    }

    /// `Sanitize::Config::DEFAULT`
    pub fn default_config() -> Config {
        Config::default()
    }

    /// `Sanitize::Config::RESTRICTED`
    pub fn restricted() -> Config {
        Config {
            elements: set(RESTRICTED_ELEMENTS),
            ..Config::default()
        }
    }

    /// `Sanitize::Config::BASIC`
    pub fn basic() -> Config {
        let mut elements = set(RESTRICTED_ELEMENTS);
        elements.extend(set(BASIC_ELEMENTS));
        let mut config = Config {
            elements,
            attributes: basic_attributes(),
            add_attributes: HashMap::from([(
                "a".to_string(),
                vec![("rel".to_string(), "nofollow".to_string())],
            )]),
            ..Config::default()
        };
        let web = Protocols::new(&["http", "https"]).relative();
        config.set_protocols(
            "a",
            "href",
            Protocols::new(&["ftp", "http", "https", "mailto"]).relative(),
        );
        config.set_protocols("blockquote", "cite", web.clone());
        config.set_protocols("q", "cite", web);
        config
    }

    /// `Sanitize::Config::RELAXED`: BASIC's elements, attributes and
    /// protocols and more, but not its `add_attributes`.
    pub fn relaxed() -> Config {
        let basic = Config::basic();
        let mut config = Config {
            elements: basic.elements,
            attributes: basic.attributes,
            protocols: basic.protocols,
            ..Config::default()
        };
        config.elements.extend(set(RELAXED_ELEMENTS));
        config.allow_doctype = true;
        config.attributes_all = Some(AttrAllow::new(&[
            "class",
            "dir",
            "hidden",
            "id",
            "lang",
            "style",
            "tabindex",
            "title",
            "translate",
        ]));
        for (element, attrs) in [
            ("a", &["href", "hreflang", "name", "rel"][..]),
            ("col", &["span", "width"]),
            ("colgroup", &["span", "width"]),
            ("data", &["value"]),
            ("del", &["cite", "datetime"]),
            (
                "img",
                &["align", "alt", "border", "height", "src", "srcset", "width"],
            ),
            ("ins", &["cite", "datetime"]),
            ("li", &["value"]),
            ("ol", &["reversed", "start", "type"]),
            ("style", &["media", "scoped", "type"]),
            (
                "table",
                &[
                    "align",
                    "bgcolor",
                    "border",
                    "cellpadding",
                    "cellspacing",
                    "frame",
                    "rules",
                    "sortable",
                    "summary",
                    "width",
                ],
            ),
            (
                "td",
                &[
                    "abbr", "align", "axis", "colspan", "headers", "rowspan", "valign", "width",
                ],
            ),
            (
                "th",
                &[
                    "abbr", "align", "axis", "colspan", "headers", "rowspan", "scope", "sorted",
                    "valign", "width",
                ],
            ),
            ("ul", &["type"]),
        ] {
            config
                .attributes
                .insert(element.to_string(), AttrAllow::new(attrs));
        }
        let web = Protocols::new(&["http", "https"]).relative();
        config.set_protocols("del", "cite", web.clone());
        config.set_protocols("img", "src", web.clone());
        config.set_protocols("ins", "cite", web);
        config.css = Some(relaxed_css());
        config
    }
}

/// `Sanitize::Config::RELAXED[:css]`
pub fn relaxed_css() -> CssConfig {
    CssConfig {
        allow_comments: true,
        allow_hacks: true,
        at_rules: HashSet::new(),
        at_rules_with_properties: set(&[
            "bottom-center",
            "bottom-left",
            "bottom-left-corner",
            "bottom-right",
            "bottom-right-corner",
            "font-face",
            "left-bottom",
            "left-middle",
            "left-top",
            "page",
            "right-bottom",
            "right-middle",
            "right-top",
            "top-center",
            "top-left",
            "top-left-corner",
            "top-right",
            "top-right-corner",
        ]),
        at_rules_with_styles: set(&[
            "-moz-keyframes",
            "-o-keyframes",
            "-webkit-keyframes",
            "container",
            "document",
            "keyframes",
            "media",
            "supports",
        ]),
        import_url_validator: None,
        properties: set(RELAXED_CSS_PROPERTIES),
        protocols: Protocols::new(&["http", "https"]).relative(),
    }
}

fn basic_attributes() -> HashMap<String, AttrAllow> {
    [
        ("a", &["href"][..]),
        ("abbr", &["title"]),
        ("blockquote", &["cite"]),
        ("dfn", &["title"]),
        ("q", &["cite"]),
        ("time", &["datetime", "pubdate"]),
    ]
    .into_iter()
    .map(|(e, a)| (e.to_string(), AttrAllow::new(a)))
    .collect()
}

const RESTRICTED_ELEMENTS: &[&str] = &["b", "em", "i", "strong", "u"];

const BASIC_ELEMENTS: &[&str] = &[
    "a",
    "abbr",
    "blockquote",
    "br",
    "cite",
    "code",
    "dd",
    "dfn",
    "dl",
    "dt",
    "kbd",
    "li",
    "mark",
    "ol",
    "p",
    "pre",
    "q",
    "s",
    "samp",
    "small",
    "strike",
    "sub",
    "sup",
    "time",
    "ul",
    "var",
];

/// The elements RELAXED adds to BASIC's.
pub const RELAXED_ELEMENTS: &[&str] = &[
    "address",
    "article",
    "aside",
    "bdi",
    "bdo",
    "body",
    "caption",
    "col",
    "colgroup",
    "data",
    "del",
    "div",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "img",
    "ins",
    "main",
    "nav",
    "rp",
    "rt",
    "ruby",
    "section",
    "span",
    "style",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "wbr",
];
