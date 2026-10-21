//! AssetProcessor#post_css (frontend/asset-processor/postcss.js) on what
//! the Sass compile prints: postcss-variable-renamer (its renames are
//! empty), autoprefixer for Discourse's browsers, the light-dark()
//! polyfill (@csstools/postcss-light-dark-function 3.0.3 with
//! @csstools/postcss-progressive-custom-properties 5.1.2), postcss-nesting
//! (14.0.1) un-nesting what the polyfill nests, and
//! postcss-variable-prefixer namespacing the polyfill's toggles by a hash
//! of the input.
//!
//! The input is always dart-sass's compressed output, so the port parses
//! that form (no whitespace between nodes) and writes new nodes with the
//! raws PostCSS gives them there. vendor/csstools holds
//! postcss-progressive-custom-properties' feature table and shorthands
//! (MIT-0), extracted from its 5.1.2 build.
//!
//! What autoprefixer would change is refused, except the prefixes it adds
//! to a handful of properties for these browsers: the port has not ported
//! its removal of outdated prefixes, its value, selector and at-rule
//! prefixes, or its transition and @supports rewriting.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use serde_json::Value;

use crate::Unsupported;

// ---------------------------------------------------------------------------
// The tree.

#[derive(Debug, Clone)]
enum Node {
    Rule(Rule),
    AtRule(AtRule),
    Decl(Decl),
    Comment(Comment),
}

#[derive(Debug, Clone)]
struct Rule {
    before: Option<String>,
    selector: String,
    between: Option<String>,
    nodes: Vec<Node>,
    semicolon: Option<bool>,
    after: Option<String>,
}

#[derive(Debug, Clone)]
struct AtRule {
    before: Option<String>,
    name: String,
    after_name: Option<String>,
    params: String,
    between: Option<String>,
    nodes: Option<Vec<Node>>,
    semicolon: Option<bool>,
    after: Option<String>,
}

#[derive(Debug, Clone)]
struct Decl {
    before: Option<String>,
    prop: String,
    between: String,
    value: String,
    important: Option<String>,
}

#[derive(Debug, Clone)]
struct Comment {
    before: Option<String>,
    text: String,
}

impl Decl {
    fn variable(&self) -> bool {
        self.prop.starts_with("--")
    }
}

// ---------------------------------------------------------------------------
// Parsing dart-sass's compressed output.

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn whitespace(&mut self) -> String {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            self.i += 1;
        }
        self.text[start..self.i].to_string()
    }

    fn comment_end(&self) -> Result<usize, Unsupported> {
        self.text[self.i + 2..]
            .find("*/")
            .map(|end| self.i + 2 + end + 2)
            .ok_or(Unsupported("an unclosed comment in compiled CSS"))
    }

    /// Up to (not including) one of `stops` outside strings, comments and
    /// brackets.
    fn until(&mut self, stops: &[u8]) -> Result<String, Unsupported> {
        let start = self.i;
        let mut depth: Vec<u8> = Vec::new();
        while let Some(c) = self.peek() {
            if depth.is_empty() && stops.contains(&c) {
                break;
            }
            match c {
                b'\\' => {
                    self.i += 2;
                    continue;
                }
                b'"' | b'\'' => {
                    self.i += 1;
                    while let Some(d) = self.peek() {
                        self.i += 1;
                        if d == b'\\' {
                            self.i += 1;
                        } else if d == c {
                            break;
                        }
                    }
                    continue;
                }
                b'/' if self.s.get(self.i + 1) == Some(&b'*') => {
                    self.i = self.comment_end()?;
                    continue;
                }
                b'(' => depth.push(b')'),
                b'[' => depth.push(b']'),
                b'{' if !depth.is_empty() => depth.push(b'}'),
                b')' | b']' | b'}' if depth.last() == Some(&c) => {
                    depth.pop();
                }
                _ => {}
            }
            self.i += 1;
        }
        self.i = self.i.min(self.s.len());
        Ok(self.text[start..self.i].to_string())
    }

    /// A container's children, and whether its last declaration ended with
    /// a semicolon (`raws.semicolon`).
    fn nodes(&mut self, top: bool) -> Result<(Vec<Node>, bool), Unsupported> {
        let mut nodes = Vec::new();
        let mut semicolon = false;
        loop {
            let before = self.whitespace();
            match self.peek() {
                None if top => return Ok((nodes, semicolon)),
                None => return Err(Unsupported("unbalanced braces in compiled CSS")),
                Some(b'}') if !top => return Ok((nodes, semicolon)),
                Some(b'}') => return Err(Unsupported("unbalanced braces in compiled CSS")),
                Some(b';') => {
                    self.i += 1;
                }
                Some(b'/') if self.s.get(self.i + 1) == Some(&b'*') => {
                    let end = self.comment_end()?;
                    nodes.push(Node::Comment(Comment {
                        before: Some(before),
                        text: self.text[self.i..end].to_string(),
                    }));
                    self.i = end;
                }
                Some(b'@') => {
                    semicolon = false;
                    self.i += 1;
                    let start = self.i;
                    while self
                        .peek()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                    {
                        self.i += 1;
                    }
                    let name = self.text[start..self.i].to_string();
                    let after_name = self.whitespace();
                    let raw = self.until(b";{}")?;
                    let params = raw.trim_end().to_string();
                    let between = raw[params.len()..].to_string();
                    if self.peek() == Some(b'{') {
                        self.i += 1;
                        let (children, child_semicolon) = self.nodes(false)?;
                        let after = self.whitespace();
                        self.i += 1;
                        let semicolon = (!children.is_empty()).then_some(child_semicolon);
                        nodes.push(Node::AtRule(AtRule {
                            before: Some(before),
                            name,
                            after_name: Some(after_name),
                            params,
                            between: Some(between),
                            nodes: Some(children),
                            semicolon,
                            after: Some(after),
                        }));
                    } else {
                        if self.peek() == Some(b';') {
                            self.i += 1;
                        }
                        nodes.push(Node::AtRule(AtRule {
                            before: Some(before),
                            name,
                            after_name: Some(after_name),
                            params,
                            between: Some(between),
                            nodes: None,
                            semicolon: None,
                            after: None,
                        }));
                    }
                }
                Some(_) => {
                    let start = self.i;
                    let head = self.until(b";{}")?;
                    if self.peek() == Some(b'{') {
                        if head.trim_start().starts_with("--") {
                            return Err(Unsupported("a custom property holding a block"));
                        }
                        semicolon = false;
                        self.i += 1;
                        let selector = head.trim_end().to_string();
                        let between = head[selector.len()..].to_string();
                        let (children, child_semicolon) = self.nodes(false)?;
                        let after = self.whitespace();
                        self.i += 1;
                        let semicolon = (!children.is_empty()).then_some(child_semicolon);
                        nodes.push(Node::Rule(Rule {
                            before: Some(before),
                            selector,
                            between: Some(between),
                            nodes: children,
                            semicolon,
                            after: Some(after),
                        }));
                    } else {
                        nodes.push(Node::Decl(declaration(&self.text[start..self.i], before)?));
                        semicolon = false;
                        if self.peek() == Some(b';') {
                            self.i += 1;
                            semicolon = true;
                        }
                    }
                }
            }
        }
    }
}

/// A declaration's text: its prop, the colon and the spaces after it,
/// the value and an `!important`.
fn declaration(text: &str, before: String) -> Result<Decl, Unsupported> {
    let colon = text
        .find(':')
        .ok_or(Unsupported("a declaration without a colon in compiled CSS"))?;
    let prop = text[..colon].trim_end().to_string();
    let rest = &text[colon + 1..];
    let value_start = rest.len() - rest.trim_start().len();
    let between = format!("{}:{}", &text[prop.len()..colon], &rest[..value_start]);
    let mut value = rest[value_start..].trim_end().to_string();
    let mut important = None;
    let lower = value.to_ascii_lowercase();
    if let Some(bang) = lower.rfind('!')
        && lower[bang + 1..].trim() == "important"
    {
        let kept = value[..bang].trim_end().to_string();
        important = Some(value[kept.len()..].to_string());
        value = kept;
    }
    Ok(Decl {
        before: Some(before),
        prop,
        between,
        value,
        important,
    })
}

fn parse(css: &str) -> Result<Vec<Node>, Unsupported> {
    let mut parser = Parser {
        s: css.as_bytes(),
        text: css,
        i: 0,
    };
    Ok(parser.nodes(true)?.0)
}

// ---------------------------------------------------------------------------
// Writing it out (Stringifier, with its raws detected as PostCSS detects
// them for nodes the plugins made).

struct Raws {
    semicolon: bool,
}

/// `rawSemicolon`: the first container (in document order) whose last
/// child is a declaration and which knows whether it ends with a
/// semicolon.
fn detect_semicolon(nodes: &[Node]) -> Option<bool> {
    for node in nodes {
        let (kids, semicolon) = match node {
            Node::Rule(r) => (Some(&r.nodes), r.semicolon),
            Node::AtRule(a) => (a.nodes.as_ref(), a.semicolon),
            _ => (None, None),
        };
        if let Some(kids) = kids {
            if matches!(kids.last(), Some(Node::Decl(_)))
                && let Some(s) = semicolon
            {
                return Some(s);
            }
            if let Some(s) = detect_semicolon(kids) {
                return Some(s);
            }
        }
    }
    None
}

fn stringify(nodes: &[Node]) -> String {
    let raws = Raws {
        semicolon: detect_semicolon(nodes).unwrap_or(false),
    };
    let mut out = String::new();
    body(&mut out, nodes, None, &raws);
    out
}

fn body(out: &mut String, nodes: &[Node], semicolon: Option<bool>, raws: &Raws) {
    let mut last = nodes.len().saturating_sub(1);
    while last > 0 && matches!(nodes[last], Node::Comment(_)) {
        last -= 1;
    }
    let semicolon = semicolon.unwrap_or(raws.semicolon);
    for (i, node) in nodes.iter().enumerate() {
        let before = match node {
            Node::Rule(r) => r.before.as_deref(),
            Node::AtRule(a) => a.before.as_deref(),
            Node::Decl(d) => d.before.as_deref(),
            Node::Comment(c) => c.before.as_deref(),
        };
        // Created nodes in compressed output have nothing before them.
        out.push_str(before.unwrap_or(""));
        let semicolon = last != i || semicolon;
        match node {
            Node::Decl(d) => {
                out.push_str(&d.prop);
                out.push_str(&d.between);
                out.push_str(&d.value);
                if let Some(important) = &d.important {
                    out.push_str(important);
                }
                if semicolon {
                    out.push(';');
                }
            }
            Node::Comment(c) => out.push_str(&c.text),
            Node::Rule(r) => {
                out.push_str(&r.selector);
                out.push_str(r.between.as_deref().unwrap_or(""));
                out.push('{');
                if !r.nodes.is_empty() {
                    body(out, &r.nodes, r.semicolon, raws);
                }
                out.push_str(r.after.as_deref().unwrap_or(""));
                out.push('}');
            }
            Node::AtRule(a) => {
                out.push('@');
                out.push_str(&a.name);
                match &a.after_name {
                    Some(after_name) => out.push_str(after_name),
                    None if !a.params.is_empty() => out.push(' '),
                    None => {}
                }
                out.push_str(&a.params);
                match &a.nodes {
                    Some(kids) => {
                        out.push_str(a.between.as_deref().unwrap_or(""));
                        out.push('{');
                        if !kids.is_empty() {
                            body(out, kids, a.semicolon, raws);
                        }
                        out.push_str(a.after.as_deref().unwrap_or(""));
                        out.push('}');
                    }
                    None => {
                        out.push_str(a.between.as_deref().unwrap_or(""));
                        if semicolon {
                            out.push(';');
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The pipeline.

/// `post_css`: the compiled CSS as Rails serves it, the source map
/// annotation left out. `source` names the input for the toggles' hash
/// (Rails hashes the compile's output with its annotation).
pub fn post_css(css: &str, source: &str) -> Result<String, Unsupported> {
    let mut nodes = parse(css)?;
    autoprefixer(&mut nodes, true)?;
    let hash = djb2(source);
    let mut toggles = 0;
    light_dark(&mut nodes, &mut toggles, false)?;
    progressive(&mut nodes)?;
    prefix_toggles(&mut nodes, &hash);
    // The root's raws.after: the whitespace after the last node.
    Ok(stringify(&nodes) + &css[css.trim_end().len()..])
}

/// postcss-variable-prefixer's `hashString` (djb2, as hex, at most eight
/// digits).
fn djb2(text: &str) -> String {
    let mut hash: u32 = 5381;
    for unit in text.encode_utf16() {
        hash = (hash << 5).wrapping_add(hash).wrapping_add(u32::from(unit));
    }
    let hex = format!("{hash:x}");
    hex.chars().take(8).collect()
}

const TOGGLE_PREFIX: &str = "--csstools-light-dark-toggle-";
const SCHEME_LIGHT: &str = "--csstools-color-scheme--light";

/// postcss-variable-prefixer: the polyfill's toggles in any declaration
/// get the hash.
fn prefix_toggles(nodes: &mut [Node], hash: &str) {
    let renamed = format!("--{hash}-{}", &TOGGLE_PREFIX[2..]);
    for node in nodes {
        match node {
            Node::Decl(d) => {
                if d.prop.starts_with(TOGGLE_PREFIX) {
                    d.prop = format!("--{hash}-{}", &d.prop[2..]);
                }
                if d.value.contains(TOGGLE_PREFIX) {
                    d.value = d.value.replace(TOGGLE_PREFIX, &renamed);
                }
            }
            Node::Rule(r) => prefix_toggles(&mut r.nodes, hash),
            Node::AtRule(a) => {
                if let Some(kids) = a.nodes.as_mut() {
                    prefix_toggles(kids, hash);
                }
            }
            Node::Comment(_) => {}
        }
    }
}

// ---------------------------------------------------------------------------
// autoprefixer, for frontend/discourse/config/targets.js's browsers (the
// last Chrome and Firefox, Safari 16.4). Measured on the reference: the
// prefixes it adds to properties, the outdated prefixed properties,
// values and keyframes it removes; anything else it would change is
// refused.

/// Properties autoprefixer prefixes for these browsers, with their
/// prefixes in the order it inserts them.
const PREFIXED_PROPS: [(&str, &[&str]); 8] = [
    ("backdrop-filter", &["-webkit-"]),
    ("box-decoration-break", &["-webkit-"]),
    ("hyphens", &["-webkit-"]),
    ("text-decoration", &["-webkit-"]),
    ("text-decoration-skip", &["-webkit-"]),
    ("text-decoration-skip-ink", &["-webkit-"]),
    ("text-size-adjust", &["-webkit-", "-moz-"]),
    ("user-select", &["-webkit-"]),
];

/// The prefixed properties autoprefixer removes when the unprefixed one
/// follows them (in the run of the same property's declarations).
const REMOVED_PROPS: &[&str] = &[
    "-moz-align-content",
    "-moz-align-self",
    "-moz-animation",
    "-moz-animation-delay",
    "-moz-animation-direction",
    "-moz-animation-duration",
    "-moz-animation-fill-mode",
    "-moz-animation-iteration-count",
    "-moz-animation-name",
    "-moz-animation-play-state",
    "-moz-animation-timing-function",
    "-moz-appearance",
    "-moz-backface-visibility",
    "-moz-background-origin",
    "-moz-background-size",
    "-moz-border-end",
    "-moz-border-image",
    "-moz-border-radius",
    "-moz-border-start",
    "-moz-box-align",
    "-moz-box-direction",
    "-moz-box-flex",
    "-moz-box-ordinal-group",
    "-moz-box-orient",
    "-moz-box-pack",
    "-moz-box-shadow",
    "-moz-box-sizing",
    "-moz-column-count",
    "-moz-column-fill",
    "-moz-column-gap",
    "-moz-column-rule",
    "-moz-column-rule-color",
    "-moz-column-rule-style",
    "-moz-column-rule-width",
    "-moz-columns",
    "-moz-column-span",
    "-moz-column-width",
    "-moz-flex-basis",
    "-moz-flex-flow",
    "-moz-flex-shrink",
    "-moz-flex-wrap",
    "-moz-font-feature-settings",
    "-moz-font-language-override",
    "-moz-font-variant-ligatures",
    "-moz-hyphens",
    "-moz-margin-end",
    "-moz-margin-start",
    "-moz-padding-end",
    "-moz-padding-start",
    "-moz-perspective",
    "-moz-perspective-origin",
    "-moz-tab-size",
    "-moz-text-align-last",
    "-moz-text-decoration-color",
    "-moz-text-decoration-line",
    "-moz-text-decoration-style",
    "-moz-transform",
    "-moz-transform-origin",
    "-moz-transform-style",
    "-moz-transition",
    "-moz-transition-delay",
    "-moz-transition-duration",
    "-moz-transition-property",
    "-moz-transition-timing-function",
    "-moz-user-select",
    "-ms-animation",
    "-ms-animation-delay",
    "-ms-animation-direction",
    "-ms-animation-duration",
    "-ms-animation-fill-mode",
    "-ms-animation-iteration-count",
    "-ms-animation-name",
    "-ms-animation-play-state",
    "-ms-animation-timing-function",
    "-ms-appearance",
    "-ms-backdrop-filter",
    "-ms-backface-visibility",
    "-ms-background-clip",
    "-ms-border-bottom-left-radius",
    "-ms-border-bottom-right-radius",
    "-ms-border-radius",
    "-ms-border-top-left-radius",
    "-ms-border-top-right-radius",
    "-ms-flex",
    "-ms-flex-align",
    "-ms-flex-direction",
    "-ms-flex-flow",
    "-ms-flex-item-align",
    "-ms-flex-line-pack",
    "-ms-flex-negative",
    "-ms-flex-order",
    "-ms-flex-pack",
    "-ms-flex-positive",
    "-ms-flex-preferred-size",
    "-ms-flex-wrap",
    "-ms-flow-from",
    "-ms-flow-into",
    "-ms-grid-area",
    "-ms-grid-column",
    "-ms-grid-column-align",
    "-ms-grid-column-end",
    "-ms-grid-row",
    "-ms-grid-row-align",
    "-ms-grid-row-end",
    "-ms-grid-template",
    "-ms-grid-template-areas",
    "-ms-hyphens",
    "-ms-place-self",
    "-ms-region-fragment",
    "-ms-scroll-snap-coordinate",
    "-ms-scroll-snap-destination",
    "-ms-scroll-snap-points-x",
    "-ms-scroll-snap-points-y",
    "-ms-scroll-snap-type",
    "-ms-text-size-adjust",
    "-ms-text-spacing",
    "-ms-touch-action",
    "-ms-transform",
    "-ms-transform-origin",
    "-ms-transform-style",
    "-ms-transition",
    "-ms-transition-delay",
    "-ms-transition-duration",
    "-ms-transition-property",
    "-ms-transition-timing-function",
    "-ms-user-select",
    "-ms-writing-mode",
    "-o-animation",
    "-o-animation-delay",
    "-o-animation-direction",
    "-o-animation-duration",
    "-o-animation-fill-mode",
    "-o-animation-iteration-count",
    "-o-animation-name",
    "-o-animation-play-state",
    "-o-animation-timing-function",
    "-o-backface-visibility",
    "-o-background-origin",
    "-o-background-size",
    "-o-border-bottom-left-radius",
    "-o-border-bottom-right-radius",
    "-o-border-image",
    "-o-border-radius",
    "-o-border-top-left-radius",
    "-o-border-top-right-radius",
    "-o-object-fit",
    "-o-object-position",
    "-o-tab-size",
    "-o-text-overflow",
    "-o-transform",
    "-o-transform-origin",
    "-o-transform-style",
    "-o-transition",
    "-o-transition-delay",
    "-o-transition-duration",
    "-o-transition-property",
    "-o-transition-timing-function",
    "-webkit-align-content",
    "-webkit-align-items",
    "-webkit-align-self",
    "-webkit-animation",
    "-webkit-animation-delay",
    "-webkit-animation-direction",
    "-webkit-animation-duration",
    "-webkit-animation-fill-mode",
    "-webkit-animation-iteration-count",
    "-webkit-animation-name",
    "-webkit-animation-play-state",
    "-webkit-animation-timing-function",
    "-webkit-appearance",
    "-webkit-backface-visibility",
    "-webkit-background-clip",
    "-webkit-background-origin",
    "-webkit-background-size",
    "-webkit-border-after",
    "-webkit-border-before",
    "-webkit-border-bottom-left-radius",
    "-webkit-border-bottom-right-radius",
    "-webkit-border-end",
    "-webkit-border-image",
    "-webkit-border-radius",
    "-webkit-border-start",
    "-webkit-border-top-left-radius",
    "-webkit-border-top-right-radius",
    "-webkit-box-align",
    "-webkit-box-direction",
    "-webkit-box-flex",
    "-webkit-box-ordinal-group",
    "-webkit-box-pack",
    "-webkit-box-shadow",
    "-webkit-box-sizing",
    "-webkit-clip-path",
    "-webkit-column-count",
    "-webkit-column-fill",
    "-webkit-column-gap",
    "-webkit-column-rule",
    "-webkit-column-rule-color",
    "-webkit-column-rule-style",
    "-webkit-column-rule-width",
    "-webkit-columns",
    "-webkit-column-span",
    "-webkit-column-width",
    "-webkit-filter",
    "-webkit-flex",
    "-webkit-flex-basis",
    "-webkit-flex-direction",
    "-webkit-flex-flow",
    "-webkit-flex-grow",
    "-webkit-flex-shrink",
    "-webkit-flex-wrap",
    "-webkit-flow-from",
    "-webkit-flow-into",
    "-webkit-font-feature-settings",
    "-webkit-font-kerning",
    "-webkit-font-language-override",
    "-webkit-font-variant-ligatures",
    "-webkit-justify-content",
    "-webkit-margin-after",
    "-webkit-margin-before",
    "-webkit-margin-end",
    "-webkit-margin-start",
    "-webkit-mask",
    "-webkit-mask-box-image",
    "-webkit-mask-box-image-outset",
    "-webkit-mask-box-image-repeat",
    "-webkit-mask-box-image-slice",
    "-webkit-mask-box-image-source",
    "-webkit-mask-box-image-width",
    "-webkit-mask-clip",
    "-webkit-mask-composite",
    "-webkit-mask-image",
    "-webkit-mask-origin",
    "-webkit-mask-position",
    "-webkit-mask-position-x",
    "-webkit-mask-position-y",
    "-webkit-mask-repeat",
    "-webkit-mask-size",
    "-webkit-order",
    "-webkit-padding-after",
    "-webkit-padding-before",
    "-webkit-padding-end",
    "-webkit-padding-start",
    "-webkit-perspective",
    "-webkit-perspective-origin",
    "-webkit-print-color-adjust",
    "-webkit-region-fragment",
    "-webkit-scroll-snap-coordinate",
    "-webkit-scroll-snap-destination",
    "-webkit-scroll-snap-points-x",
    "-webkit-scroll-snap-points-y",
    "-webkit-scroll-snap-type",
    "-webkit-shape-image-threshold",
    "-webkit-shape-margin",
    "-webkit-shape-outside",
    "-webkit-text-decoration-color",
    "-webkit-text-decoration-line",
    "-webkit-text-decoration-style",
    "-webkit-text-emphasis",
    "-webkit-text-emphasis-color",
    "-webkit-text-emphasis-position",
    "-webkit-text-emphasis-style",
    "-webkit-text-orientation",
    "-webkit-transform",
    "-webkit-transform-origin",
    "-webkit-transform-style",
    "-webkit-transition",
    "-webkit-transition-delay",
    "-webkit-transition-duration",
    "-webkit-transition-property",
    "-webkit-transition-timing-function",
    "-webkit-writing-mode",
];

/// Old flexbox properties autoprefixer maps to other names: not ported.
const FLEXBOX_HACKS: &[&str] = &[
    "box-align",
    "box-direction",
    "box-flex",
    "box-ordinal-group",
    "box-orient",
    "box-pack",
    "flex-align",
    "flex-item-align",
    "flex-line-pack",
    "flex-negative",
    "flex-order",
    "flex-pack",
    "flex-positive",
    "flex-preferred-size",
];

/// Prefixed values autoprefixer removes when a later declaration of the
/// property has the value in the second column.
const REMOVED_VALUES: &[(&str, &str)] = &[
    ("-moz-linear-gradient", "linear-gradient"),
    ("-moz-radial-gradient", "radial-gradient"),
    (
        "-moz-repeating-linear-gradient",
        "repeating-linear-gradient",
    ),
    (
        "-moz-repeating-radial-gradient",
        "repeating-radial-gradient",
    ),
    ("-ms-linear-gradient", "linear-gradient"),
    ("-ms-radial-gradient", "radial-gradient"),
    ("-ms-repeating-linear-gradient", "repeating-linear-gradient"),
    ("-ms-repeating-radial-gradient", "repeating-radial-gradient"),
    ("-o-linear-gradient", "linear-gradient"),
    ("-o-radial-gradient", "radial-gradient"),
    ("-o-repeating-linear-gradient", "repeating-linear-gradient"),
    ("-o-repeating-radial-gradient", "repeating-radial-gradient"),
    ("-webkit-image-set", "image-set"),
    ("-webkit-linear-gradient", "linear-gradient"),
    ("-webkit-radial-gradient", "radial-gradient"),
    (
        "-webkit-repeating-linear-gradient",
        "repeating-linear-gradient",
    ),
    (
        "-webkit-repeating-radial-gradient",
        "repeating-radial-gradient",
    ),
    ("-moz-grabbing", "grabbing"),
    ("-moz-grab", "grab"),
    ("-moz-zoom-in", "zoom-in"),
    ("-moz-zoom-out", "zoom-out"),
    ("-webkit-grabbing", "grabbing"),
    ("-webkit-grab", "grab"),
    ("-webkit-zoom-in", "zoom-in"),
    ("-webkit-zoom-out", "zoom-out"),
    ("-moz-box", "flex"),
    ("-moz-inline-box", "inline-flex"),
    ("-ms-flexbox", "flex"),
    ("-ms-grid", "grid"),
    ("-ms-inline-flexbox", "inline-flex"),
    ("-ms-inline-grid", "inline-grid"),
    ("-webkit-box", "flex"),
    ("-webkit-flex", "flex"),
    ("-webkit-inline-box", "inline-flex"),
    ("-webkit-inline-flex", "inline-flex"),
    ("-moz-crisp-edges", "pixelated"),
    ("-webkit-optimize-contrast", "pixelated"),
    ("-webkit-sticky", "sticky"),
    ("-moz-calc", "calc"),
    ("-moz-fit-content", "fit-content"),
    ("-moz-max-content", "max-content"),
    ("-moz-min-content", "min-content"),
    ("-webkit-calc", "calc"),
    ("-webkit-fit-content", "fit-content"),
    ("-webkit-max-content", "max-content"),
    ("-webkit-min-content", "min-content"),
];

/// Every property, value, selector or at-rule autoprefixer knows a prefix
/// for (its data/prefixes.js), the unprefixed name.
const KNOWN_FEATURES: &[&str] = &[
    "align-content",
    "align-items",
    "align-self",
    "animation",
    "animation-delay",
    "animation-direction",
    "animation-duration",
    "animation-fill-mode",
    "animation-iteration-count",
    "animation-name",
    "animation-play-state",
    "animation-timing-function",
    "any-link",
    "appearance",
    "autofill",
    "backdrop",
    "backdrop-filter",
    "backface-visibility",
    "background-clip",
    "background-origin",
    "background-size",
    "border-block-end",
    "border-block-start",
    "border-bottom-left-radius",
    "border-bottom-right-radius",
    "border-image",
    "border-inline-end",
    "border-inline-start",
    "border-radius",
    "border-top-left-radius",
    "border-top-right-radius",
    "box-decoration-break",
    "box-shadow",
    "box-sizing",
    "break-after",
    "break-before",
    "break-inside",
    "calc",
    "clip-path",
    "color-adjust",
    "column-count",
    "column-fill",
    "column-gap",
    "column-rule",
    "column-rule-color",
    "column-rule-style",
    "column-rule-width",
    "columns",
    "column-span",
    "column-width",
    "cross-fade",
    "element",
    "file-selector-button",
    "fill",
    "fill-available",
    "filter",
    "fit-content",
    "flex",
    "flex-basis",
    "flex-direction",
    "flex-flow",
    "flex-grow",
    "flex-shrink",
    "flex-wrap",
    "flow-from",
    "flow-into",
    "font-feature-settings",
    "font-kerning",
    "font-language-override",
    "font-variant-ligatures",
    "fullscreen",
    "full-screen",
    "grab",
    "grabbing",
    "grid-area",
    "grid-column",
    "grid-column-align",
    "grid-column-end",
    "grid-column-start",
    "grid-row",
    "grid-row-align",
    "grid-row-end",
    "grid-row-start",
    "grid-template",
    "grid-template-areas",
    "grid-template-columns",
    "grid-template-rows",
    "hyphens",
    "image-rendering",
    "image-set",
    "inline-flex",
    "inline-grid",
    "isolate",
    "isolate-override",
    "justify-content",
    "keyframes",
    "linear-gradient",
    "margin-block-end",
    "margin-block-start",
    "margin-inline-end",
    "margin-inline-start",
    "mask",
    "mask-border",
    "mask-border-outset",
    "mask-border-repeat",
    "mask-border-slice",
    "mask-border-source",
    "mask-border-width",
    "mask-clip",
    "mask-composite",
    "mask-image",
    "mask-origin",
    "mask-position",
    "mask-position-x",
    "mask-position-y",
    "mask-repeat",
    "mask-size",
    "max-content",
    "min-content",
    "object-fit",
    "object-position",
    "order",
    "overscroll-behavior",
    "padding-block-end",
    "padding-block-start",
    "padding-inline-end",
    "padding-inline-start",
    "perspective",
    "perspective-origin",
    "pixelated",
    "placeholder",
    "placeholder-shown",
    "place-self",
    "plaintext",
    "print-color-adjust",
    "radial-gradient",
    "read-only",
    "read-write",
    "region-fragment",
    "repeating-linear-gradient",
    "repeating-radial-gradient",
    "resolution",
    "scroll-snap-coordinate",
    "scroll-snap-destination",
    "scroll-snap-points-x",
    "scroll-snap-points-y",
    "scroll-snap-type",
    "selection",
    "shape-image-threshold",
    "shape-margin",
    "shape-outside",
    "sticky",
    "stretch",
    "tab-size",
    "text-align-last",
    "text-decoration",
    "text-decoration-color",
    "text-decoration-line",
    "text-decoration-skip",
    "text-decoration-skip-ink",
    "text-decoration-style",
    "text-emphasis",
    "text-emphasis-color",
    "text-emphasis-position",
    "text-emphasis-style",
    "text-orientation",
    "text-overflow",
    "text-size-adjust",
    "text-spacing",
    "touch-action",
    "transform",
    "transform-origin",
    "transform-style",
    "transition",
    "transition-delay",
    "transition-duration",
    "transition-property",
    "transition-timing-function",
    "user-select",
    "viewport",
    "writing-mode",
    "zoom-in",
    "zoom-out",
    "box",
    "flexbox",
    "input-placeholder",
    "box-orient",
    "box-direction",
    "box-flex",
    "box-pack",
    "box-align",
    "box-ordinal-group",
    "flex-align",
    "flex-pack",
    "flex-order",
    "flex-item-align",
    "flex-line-pack",
    "flex-negative",
    "flex-positive",
    "flex-preferred-size",
    "min-device-pixel-ratio",
    "max-device-pixel-ratio",
    "device-pixel-ratio",
    "available",
    "background-inline-policy",
    "border-before",
    "border-after",
    "border-start",
    "border-end",
    "margin-before",
    "margin-after",
    "margin-start",
    "margin-end",
    "padding-before",
    "padding-after",
    "padding-start",
    "padding-end",
    "mask-box-image",
    "mask-box-image-outset",
    "mask-box-image-repeat",
    "mask-box-image-slice",
    "mask-box-image-source",
    "mask-box-image-width",
    "mask-source-type",
    "content-zoom-chaining",
    "zoom",
];

/// The value keywords and functions autoprefixer prefixes or rewrites for
/// these browsers.
const PREFIXED_VALUES: &[&str] = &["fill", "fill-available", "stretch", "available"];
const PREFIXED_VALUE_FUNCTIONS: &[&str] = &["cross-fade(", "element("];

const VENDORS: [&str; 4] = ["-webkit-", "-moz-", "-ms-", "-o-"];

fn vendor_split(name: &str) -> Option<(&'static str, &str)> {
    let lower = name.to_ascii_lowercase();
    VENDORS
        .iter()
        .find(|v| lower.starts_with(**v))
        .map(|v| (*v, &name[v.len()..]))
}

/// `unprefixed(prop)`: without its vendor prefix.
fn unprefixed(prop: &str) -> String {
    let lower = prop.to_ascii_lowercase();
    match vendor_split(&lower) {
        Some((_, base)) => base.to_string(),
        None => lower,
    }
}

fn known(name: &str) -> bool {
    KNOWN_FEATURES.contains(&name.to_ascii_lowercase().as_str())
}

/// The vendor-prefixed names a value mentions (`-webkit-box`,
/// `-moz-linear-gradient`...), lowercased.
fn prefixed_names(value: &str) -> Vec<String> {
    let lower = value.to_ascii_lowercase();
    let mut out = Vec::new();
    for vendor in VENDORS {
        for (i, _) in lower.match_indices(vendor) {
            if i > 0
                && lower[..i]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                continue;
            }
            let start = i + vendor.len();
            let end = lower[start..]
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .map_or(lower.len(), |e| start + e);
            out.push(lower[i..end].to_string());
        }
    }
    out
}

fn words(value: &str) -> impl Iterator<Item = String> + '_ {
    value
        .split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')')
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
}

const REFUSED: Unsupported = Unsupported("CSS autoprefixer rewrites (169a)");

/// autoprefixer's work on the stylesheet.
fn autoprefixer(nodes: &mut Vec<Node>, root: bool) -> Result<(), Unsupported> {
    let mut i = 0;
    while i < nodes.len() {
        match &mut nodes[i] {
            Node::AtRule(a) => {
                let name = a.name.to_ascii_lowercase();
                if let Some((_, base)) = vendor_split(&name) {
                    if base == "keyframes" {
                        // Prefixed keyframes are not needed by any of the
                        // browsers: removed. Root#removeChild gives the
                        // first node's raws.before to the next one.
                        let removed = nodes.remove(i);
                        if root
                            && i == 0
                            && let (Node::AtRule(gone), Some(next)) = (removed, nodes.first_mut())
                        {
                            let before = gone.before;
                            match next {
                                Node::Rule(n) => n.before = before,
                                Node::AtRule(n) => n.before = before,
                                Node::Decl(n) => n.before = before,
                                Node::Comment(n) => n.before = before,
                            }
                        }
                        continue;
                    }
                    if known(base) {
                        return Err(REFUSED);
                    }
                }
                let params = a.params.to_ascii_lowercase();
                if name == "supports"
                    && (PREFIXED_PROPS.iter().any(|(p, _)| params.contains(p))
                        || prefixed_names(&params)
                            .iter()
                            .any(|n| vendor_split(n).is_some_and(|(_, b)| known(b))))
                {
                    return Err(REFUSED);
                }
                if name == "media"
                    && (params.contains("resolution") || params.contains("device-pixel-ratio"))
                {
                    return Err(REFUSED);
                }
                if let Some(kids) = a.nodes.as_mut() {
                    autoprefixer(kids, false)?;
                }
            }
            Node::Rule(r) => {
                let lower = r.selector.to_ascii_lowercase();
                for vendor in VENDORS {
                    for (at, _) in lower.match_indices(&format!(":{vendor}")) {
                        let start = at + 1 + vendor.len();
                        let end = lower[start..]
                            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                            .map_or(lower.len(), |e| start + e);
                        if known(&lower[start..end]) {
                            return Err(REFUSED);
                        }
                    }
                }
                autoprefixer(&mut r.nodes, false)?;
            }
            Node::Decl(_) | Node::Comment(_) => {}
        }
        i += 1;
    }
    prefix_declarations(nodes)
}

/// Whether a later declaration of the same run (declarations of the same
/// unprefixed property; others end it) passes `test`.
fn group_down(nodes: &[Node], i: usize, test: impl Fn(&Decl) -> bool) -> bool {
    let Node::Decl(d) = &nodes[i] else {
        return false;
    };
    let base = unprefixed(&d.prop);
    for node in &nodes[i + 1..] {
        let Node::Decl(o) = node else {
            continue;
        };
        if unprefixed(&o.prop) != base {
            return false;
        }
        if test(o) {
            return true;
        }
    }
    false
}

/// The declarations of one container: `remove` (outdated prefixes),
/// then `add`.
fn prefix_declarations(nodes: &mut Vec<Node>) -> Result<(), Unsupported> {
    // Removal.
    let mut i = 0;
    while i < nodes.len() {
        let Node::Decl(d) = &nodes[i] else {
            i += 1;
            continue;
        };
        if d.variable() {
            i += 1;
            continue;
        }
        let prop = d.prop.to_ascii_lowercase();
        if let Some((_, base)) = vendor_split(&prop) {
            let base = base.to_string();
            if FLEXBOX_HACKS.contains(&base.as_str()) {
                let line_clamp = nodes
                    .iter()
                    .any(|n| matches!(n, Node::Decl(o) if o.prop.eq_ignore_ascii_case("-webkit-line-clamp")));
                let flex = nodes.iter().any(|n| {
                    matches!(n, Node::Decl(o) if matches!(o.prop.to_ascii_lowercase().as_str(), "flex-direction" | "flex-flow"))
                });
                // -webkit-box-orient stays beside -webkit-line-clamp or
                // without flex-direction.
                if prop == "-webkit-box-orient" && (line_clamp || !flex) {
                    i += 1;
                    continue;
                }
                return Err(REFUSED);
            }
            if REMOVED_PROPS.contains(&prop.as_str()) {
                let hack = prop == "-webkit-background-clip" && d.value == "text";
                if !hack && group_down(nodes, i, |o| o.prop.eq_ignore_ascii_case(&base)) {
                    nodes.remove(i);
                    continue;
                }
            }
        }
        let Node::Decl(d) = &nodes[i] else {
            unreachable!()
        };
        let removable = prefixed_names(&d.value).into_iter().find_map(|name| {
            REMOVED_VALUES
                .iter()
                .find(|(prefixed, _)| *prefixed == name)
                .map(|(_, plain)| *plain)
        });
        if let Some(plain) = removable
            && group_down(nodes, i, |o| o.value.to_ascii_lowercase().contains(plain))
        {
            nodes.remove(i);
            continue;
        }
        i += 1;
    }

    // What remains: refusals, and the prefixes added.
    let mut i = 0;
    while i < nodes.len() {
        let Node::Decl(d) = &nodes[i] else {
            i += 1;
            continue;
        };
        let d = d.clone();
        if d.variable() {
            i += 1;
            continue;
        }
        let prop = d.prop.to_ascii_lowercase();
        let value = d.value.to_ascii_lowercase();
        if words(&value).any(|w| PREFIXED_VALUES.contains(&w.as_str()) || w.ends_with("-available"))
            || PREFIXED_VALUE_FUNCTIONS.iter().any(|f| value.contains(f))
        {
            return Err(REFUSED);
        }
        let transitions = matches!(
            unprefixed(&prop).as_str(),
            "transition" | "transition-property" | "will-change"
        );
        if transitions
            && (words(&value).any(|w| PREFIXED_PROPS.iter().any(|(p, _)| *p == w))
                || !prefixed_names(&value).is_empty())
        {
            return Err(REFUSED);
        }
        let Some((_, prefixes)) = PREFIXED_PROPS.iter().find(|(p, _)| *p == prop) else {
            i += 1;
            continue;
        };
        // TextDecoration: basic values are not prefixed.
        const BASIC: [&str; 8] = [
            "none",
            "underline",
            "overline",
            "line-through",
            "blink",
            "inherit",
            "initial",
            "unset",
        ];
        if prop == "text-decoration" && value.split_whitespace().all(|w| BASIC.contains(&w)) {
            i += 1;
            continue;
        }
        let mut inserted = 0;
        for prefix in prefixes.iter() {
            // UserSelect: no prefix for `contain`.
            if prop == "user-select" && value == "contain" {
                continue;
            }
            // otherPrefixes: a value with another vendor's prefix.
            if VENDORS.iter().any(|v| v != prefix && value.contains(v)) {
                continue;
            }
            let (prefixed_prop, prefixed_value) =
                if prop == "text-decoration-skip-ink" && value == "auto" {
                    (format!("{prefix}text-decoration-skip"), "ink".to_string())
                } else {
                    (format!("{prefix}{}", d.prop), d.value.clone())
                };
            // isAlready: the prefixed property in the run around it.
            let already = nodes.iter().any(|n| {
                matches!(n, Node::Decl(o) if o.prop.eq_ignore_ascii_case(&format!("{prefix}{}", d.prop)) || o.prop.eq_ignore_ascii_case(&prefixed_prop))
            });
            if already {
                continue;
            }
            let mut clone = d.clone();
            clone.prop = prefixed_prop;
            clone.value = prefixed_value;
            nodes.insert(i + inserted, Node::Decl(clone));
            inserted += 1;
        }
        i += inserted + 1;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// light-dark(): @csstools/postcss-light-dark-function with `preserve`.

/// A component value of a declaration's value (css-parser-algorithms).
#[derive(Debug, Clone)]
enum Component {
    /// A token: its text.
    Token(String),
    Whitespace(String),
    Comment(String),
    Comma,
    /// A function: its name (without the parenthesis), its children, and
    /// whether it was closed.
    Function {
        name: String,
        children: Vec<Component>,
        closed: bool,
    },
    /// A simple block: its opening bracket, children, and closing one.
    Block {
        open: char,
        children: Vec<Component>,
        close: String,
    },
}

impl Component {
    fn text(&self) -> String {
        match self {
            Component::Token(t) | Component::Whitespace(t) | Component::Comment(t) => t.clone(),
            Component::Comma => ",".to_string(),
            Component::Function {
                name,
                children,
                closed,
            } => {
                let mut out = format!("{name}(");
                for c in children {
                    out.push_str(&c.text());
                }
                if *closed {
                    out.push(')');
                }
                out
            }
            Component::Block {
                open,
                children,
                close,
            } => {
                let mut out = open.to_string();
                for c in children {
                    out.push_str(&c.text());
                }
                out.push_str(close);
                out
            }
        }
    }
}

/// Tokenizes a value into component values (enough of css-tokenizer and
/// css-parser-algorithms for the polyfill's argument counting).
fn components(value: &str) -> Vec<Component> {
    let chars: Vec<char> = value.chars().collect();
    let mut i = 0;
    parse_components(&chars, &mut i, None)
}

fn parse_components(chars: &[char], i: &mut usize, close: Option<char>) -> Vec<Component> {
    let mut out = Vec::new();
    while *i < chars.len() {
        let c = chars[*i];
        if Some(c) == close {
            return out;
        }
        if c.is_whitespace() {
            let start = *i;
            while *i < chars.len() && chars[*i].is_whitespace() {
                *i += 1;
            }
            out.push(Component::Whitespace(chars[start..*i].iter().collect()));
            continue;
        }
        if c == '/' && chars.get(*i + 1) == Some(&'*') {
            let start = *i;
            *i += 2;
            while *i < chars.len() && !(chars[*i] == '*' && chars.get(*i + 1) == Some(&'/')) {
                *i += 1;
            }
            *i = (*i + 2).min(chars.len());
            out.push(Component::Comment(chars[start..*i].iter().collect()));
            continue;
        }
        if c == ',' {
            *i += 1;
            out.push(Component::Comma);
            continue;
        }
        if c == '"' || c == '\'' {
            let start = *i;
            *i += 1;
            while *i < chars.len() && chars[*i] != c {
                if chars[*i] == '\\' {
                    *i += 1;
                }
                *i += 1;
            }
            *i = (*i + 1).min(chars.len());
            out.push(Component::Token(chars[start..*i].iter().collect()));
            continue;
        }
        if c == '(' || c == '[' || c == '{' {
            let closing = match c {
                '(' => ')',
                '[' => ']',
                _ => '}',
            };
            *i += 1;
            let children = parse_components(chars, i, Some(closing));
            let close = if *i < chars.len() {
                *i += 1;
                closing.to_string()
            } else {
                String::new()
            };
            out.push(Component::Block {
                open: c,
                children,
                close,
            });
            continue;
        }
        if c == ')' || c == ']' || c == '}' {
            *i += 1;
            out.push(Component::Token(c.to_string()));
            continue;
        }
        // An ident, number, hash or delimiter run up to a boundary; an
        // ident followed by `(` is a function.
        let start = *i;
        let is_name =
            |c: char| c.is_alphanumeric() || c == '-' || c == '_' || c == '\\' || !c.is_ascii();
        if is_name(c) || c == '#' || c == '.' || c == '+' || c == '%' {
            *i += 1;
            while *i < chars.len() && (is_name(chars[*i]) || chars[*i] == '.' || chars[*i] == '%') {
                if chars[*i] == '\\' {
                    *i += 1;
                }
                *i += 1;
            }
            *i = (*i).min(chars.len());
            let text: String = chars[start..*i].iter().collect();
            if *i < chars.len() && chars[*i] == '(' && !text.starts_with('#') {
                if text.eq_ignore_ascii_case("url") {
                    // An unquoted url() is one token.
                    let mut j = *i + 1;
                    while j < chars.len() && chars[j].is_whitespace() {
                        j += 1;
                    }
                    if j < chars.len() && chars[j] != '"' && chars[j] != '\'' {
                        while *i < chars.len() && chars[*i] != ')' {
                            *i += 1;
                        }
                        *i = (*i + 1).min(chars.len());
                        out.push(Component::Token(chars[start..*i].iter().collect()));
                        continue;
                    }
                }
                *i += 1;
                let children = parse_components(chars, i, Some(')'));
                let closed = *i < chars.len();
                if closed {
                    *i += 1;
                }
                out.push(Component::Function {
                    name: text,
                    children,
                    closed,
                });
            } else {
                out.push(Component::Token(text));
            }
            continue;
        }
        *i += 1;
        out.push(Component::Token(c.to_string()));
    }
    out
}

fn is_light_dark(c: &Component) -> bool {
    matches!(c, Component::Function { name, .. } if name.eq_ignore_ascii_case("light-dark"))
}

/// `parseLightDark`: the light and dark arguments of a light-dark()
/// with exactly two, each one component, nested light-dark()s in them
/// resolved to their own side.
fn parse_light_dark(c: &Component) -> Option<(Component, Component)> {
    let Component::Function { children, .. } = c else {
        return None;
    };
    if !is_light_dark(c) {
        return None;
    }
    let args: Vec<&Component> = children
        .iter()
        .filter(|c| !matches!(c, Component::Whitespace(_) | Component::Comment(_)))
        .collect();
    let [light, comma, dark] = args.as_slice() else {
        return None;
    };
    if !matches!(comma, Component::Comma)
        || matches!(light, Component::Comma)
        || matches!(dark, Component::Comma)
    {
        return None;
    }
    let mut light = (*light).clone();
    let mut dark = (*dark).clone();
    if matches!(light, Component::Function { .. }) {
        light = resolve_nested(light, true);
    }
    if matches!(dark, Component::Function { .. }) {
        dark = resolve_nested(dark, false);
    }
    Some((light, dark))
}

/// `recurseLightDark`: nested light-dark()s replaced by their light (or
/// dark) side, the node itself too.
fn resolve_nested(c: Component, light: bool) -> Component {
    if let Some((l, d)) = parse_light_dark(&c) {
        return resolve_nested(if light { l } else { d }, light);
    }
    match c {
        Component::Function {
            name,
            children,
            closed,
        } => Component::Function {
            name,
            children: children
                .into_iter()
                .map(|c| resolve_nested(c, light))
                .collect(),
            closed,
        },
        Component::Block {
            open,
            children,
            close,
        } => Component::Block {
            open,
            children: children
                .into_iter()
                .map(|c| resolve_nested(c, light))
                .collect(),
            close,
        },
        other => other,
    }
}

/// `transformLightDark`: each light-dark() made a `var()` of a new toggle
/// falling back to its light side; the toggles with their values.
fn transform_light_dark(value: &str, toggles: &mut usize) -> (String, Vec<(String, String)>) {
    let mut made = Vec::new();
    let replaced: Vec<Component> = components(value)
        .into_iter()
        .map(|c| replace_light_dark(c, toggles, &mut made))
        .collect();
    let text = replaced.iter().map(Component::text).collect();
    (text, made)
}

fn replace_light_dark(
    c: Component,
    toggles: &mut usize,
    made: &mut Vec<(String, String)>,
) -> Component {
    if let Some((light, dark)) = parse_light_dark(&c) {
        let name = format!("{TOGGLE_PREFIX}-{toggles}");
        *toggles += 1;
        made.push((name.clone(), format!("var({SCHEME_LIGHT}) {}", dark.text())));
        return Component::Function {
            name: "var".to_string(),
            children: vec![
                Component::Token(name),
                Component::Comma,
                Component::Whitespace(" ".to_string()),
                light,
            ],
            closed: true,
        };
    }
    match c {
        Component::Function {
            name,
            children,
            closed,
        } => Component::Function {
            name,
            children: children
                .into_iter()
                .map(|c| replace_light_dark(c, toggles, made))
                .collect(),
            closed,
        },
        Component::Block {
            open,
            children,
            close,
        } => Component::Block {
            open,
            children: children
                .into_iter()
                .map(|c| replace_light_dark(c, toggles, made))
                .collect(),
            close,
        },
        other => other,
    }
}

/// `colorSchemes`: whether a color-scheme value names light, dark.
fn color_schemes(value: &str) -> (bool, bool) {
    let (mut light, mut dark) = (false, false);
    for c in components(value) {
        if let Component::Token(t) = c {
            if !t
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '-' || c == '_')
            {
                continue;
            }
            let lower = t.to_ascii_lowercase();
            if lower.contains("light") {
                light = true;
            } else if lower.contains("dark") {
                dark = true;
            }
        }
    }
    (light, dark)
}

/// The polyfill on every declaration, in document order: a container's
/// own declarations, then what it contains. `supports` is whether an
/// enclosing @supports mentions light-dark() (hasSupportsAtRuleAncestor).
fn light_dark(
    nodes: &mut Vec<Node>,
    toggles: &mut usize,
    supports: bool,
) -> Result<(), Unsupported> {
    let mut i = 0;
    while i < nodes.len() {
        match &mut nodes[i] {
            Node::Rule(r) => {
                let selector = r.selector.clone();
                let Polyfilled { nested, dark_media } =
                    rule_light_dark(&mut r.nodes, toggles, supports)?;
                let dark_rule = dark_media.map(|decl| Rule {
                    before: r.before.clone(),
                    selector: r.selector.clone(),
                    between: r.between.clone(),
                    nodes: vec![Node::Decl(decl)],
                    semicolon: r.semicolon,
                    after: r.after.clone(),
                });
                if !nested.is_empty() {
                    // postcss-nesting moved the nested `& *` rule out after
                    // the rule, and marked the rule's last semicolon.
                    r.semicolon = Some(true);
                    let inner = Node::Rule(Rule {
                        before: Some(String::new()),
                        selector: nest_selector(&selector),
                        between: None,
                        nodes: nested,
                        semicolon: None,
                        after: None,
                    });
                    nodes.insert(
                        i + 1,
                        Node::AtRule(AtRule {
                            before: Some(String::new()),
                            name: "supports".to_string(),
                            after_name: None,
                            params: "not (color: light-dark(tan, tan))".to_string(),
                            between: None,
                            nodes: Some(vec![inner]),
                            semicolon: None,
                            after: None,
                        }),
                    );
                    i += 1;
                }
                if let Some(rule) = dark_rule {
                    nodes.insert(
                        i + 1,
                        Node::AtRule(AtRule {
                            before: Some(String::new()),
                            name: "media".to_string(),
                            after_name: None,
                            params: "(prefers-color-scheme: dark)".to_string(),
                            between: None,
                            nodes: Some(vec![Node::Rule(rule)]),
                            semicolon: None,
                            after: None,
                        }),
                    );
                    i += 1;
                }
            }
            Node::AtRule(a) => {
                let supports = supports
                    || (a.name.eq_ignore_ascii_case("supports") && has_light_dark(&a.params));
                let mut dark_at_rule = None;
                if let Some(kids) = a.nodes.as_mut() {
                    // Declarations directly in an at-rule (@font-face,
                    // @page...): postcss-nesting leaves the nested rule in
                    // place there, its `&` made `:scope`.
                    let Polyfilled { nested, dark_media } =
                        rule_light_dark(kids, toggles, supports)?;
                    light_dark(kids, toggles, supports)?;
                    if !nested.is_empty() {
                        kids.push(Node::AtRule(AtRule {
                            before: Some(String::new()),
                            name: "supports".to_string(),
                            after_name: None,
                            params: "not (color: light-dark(tan, tan))".to_string(),
                            between: Some(String::new()),
                            nodes: Some(vec![Node::Rule(Rule {
                                before: Some(String::new()),
                                selector: ":scope *".to_string(),
                                between: Some(String::new()),
                                nodes: nested,
                                semicolon: None,
                                after: Some(String::new()),
                            })]),
                            semicolon: None,
                            after: Some(String::new()),
                        }));
                    }
                    dark_at_rule = dark_media.map(|decl| AtRule {
                        before: a.before.clone(),
                        name: a.name.clone(),
                        after_name: a.after_name.clone(),
                        params: a.params.clone(),
                        between: a.between.clone(),
                        nodes: Some(vec![Node::Decl(decl)]),
                        semicolon: a.semicolon,
                        after: a.after.clone(),
                    });
                }
                if let Some(at_rule) = dark_at_rule {
                    nodes.insert(
                        i + 1,
                        Node::AtRule(AtRule {
                            before: Some(String::new()),
                            name: "media".to_string(),
                            after_name: None,
                            params: "(prefers-color-scheme: dark)".to_string(),
                            between: None,
                            nodes: Some(vec![Node::AtRule(at_rule)]),
                            semicolon: None,
                            after: None,
                        }),
                    );
                    i += 1;
                }
            }
            Node::Decl(_) | Node::Comment(_) => {}
        }
        i += 1;
    }
    Ok(())
}

fn has_light_dark(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.match_indices("light-dark(").any(|(i, _)| {
        i == 0
            || !lower[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

/// What the polyfill puts after a rule: its nested `& *` rule's
/// declarations, and the dark override of `color-scheme: light dark`.
struct Polyfilled {
    nested: Vec<Node>,
    dark_media: Option<Decl>,
}

/// The polyfill on a container's own declarations.
fn rule_light_dark(
    nodes: &mut Vec<Node>,
    toggles: &mut usize,
    supports: bool,
) -> Result<Polyfilled, Unsupported> {
    let mut nested = Vec::new();
    let mut dark_media = None;
    let mut i = 0;
    while i < nodes.len() {
        let Node::Decl(d) = &nodes[i] else {
            i += 1;
            continue;
        };
        let d = d.clone();
        if d.prop.eq_ignore_ascii_case("color-scheme") {
            if nodes
                .iter()
                .any(|n| matches!(n, Node::Decl(o) if o.prop == SCHEME_LIGHT))
            {
                i += 1;
                continue;
            }
            let (light, dark) = color_schemes(&d.value);
            if light && dark {
                // The light toggle, and its dark override in an @media
                // after the rule.
                let mut clone = d.clone();
                clone.prop = SCHEME_LIGHT.to_string();
                clone.value = "initial".to_string();
                nodes.insert(i, Node::Decl(clone));
                let mut dark = d.clone();
                dark.prop = SCHEME_LIGHT.to_string();
                dark.value = " ".to_string();
                dark_media = Some(dark);
                i += 2;
                continue;
            }
            let value = if dark {
                Some(" ")
            } else if light {
                Some("initial")
            } else {
                None
            };
            if let Some(value) = value {
                let mut clone = d.clone();
                clone.prop = SCHEME_LIGHT.to_string();
                clone.value = value.to_string();
                nodes.insert(i, Node::Decl(clone));
                i += 1;
            }
            i += 1;
            continue;
        }
        if !has_light_dark(&d.value) {
            i += 1;
            continue;
        }
        // hasFallback: an earlier declaration of the property.
        let fallback = nodes[..i]
            .iter()
            .any(|n| matches!(n, Node::Decl(o) if o.prop.eq_ignore_ascii_case(&d.prop)));
        if fallback || supports {
            i += 1;
            continue;
        }
        let (value, made) = transform_light_dark(&d.value, toggles);
        if value == d.value {
            i += 1;
            continue;
        }
        let mut inserted = Vec::new();
        for (prop, toggle) in &made {
            let mut clone = d.clone();
            clone.prop = prop.clone();
            clone.value = toggle.clone();
            inserted.push(Node::Decl(clone));
        }
        let mut fallback = d.clone();
        fallback.value = value;
        inserted.push(Node::Decl(fallback));
        if d.variable() {
            nested.extend(inserted.iter().cloned().map(|n| match n {
                Node::Decl(mut c) => {
                    c.before = Some(String::new());
                    Node::Decl(c)
                }
                other => other,
            }));
        }
        let count = inserted.len();
        nodes.splice(i..i, inserted);
        i += count + 1;
    }
    Ok(Polyfilled { nested, dark_media })
}

/// postcss-nesting's `& *` under a rule: the rule's one compound selector
/// itself, else `:is()` of its selectors, combinators spaced.
fn nest_selector(selector: &str) -> String {
    let selectors = split_top(selector, ',');
    if selectors.len() == 1
        && let Some(parts) = compound(&selectors[0])
    {
        return format!("{} *", sort_compound(parts).concat());
    }
    let spaced: Vec<String> = selectors.iter().map(|s| space_combinators(s)).collect();
    format!(":is({}) *", spaced.join(","))
}

/// `text` split on `sep` outside brackets and strings.
fn split_top(text: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in text.chars() {
        if escaped {
            escaped = false;
            current.push(c);
            continue;
        }
        match (quote, c) {
            (_, '\\') => escaped = true,
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(' | '[') => depth += 1,
            (None, ')' | ']') => depth -= 1,
            (None, c) if c == sep && depth == 0 => {
                out.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    out.push(current);
    out
}

/// The simple selectors of a compound selector, or None for one with a
/// combinator or a pseudo-element.
fn compound(selector: &str) -> Option<Vec<String>> {
    let chars: Vec<char> = selector.chars().collect();
    let mut parts = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        let c = chars[i];
        if c.is_whitespace() || c == '>' || c == '+' || c == '~' {
            return None;
        }
        let name_end = |mut j: usize| {
            while j < chars.len()
                && (chars[j].is_alphanumeric()
                    || chars[j] == '-'
                    || chars[j] == '_'
                    || chars[j] == '\\'
                    || !chars[j].is_ascii())
            {
                if chars[j] == '\\' {
                    j += 1;
                }
                j += 1;
            }
            j.min(chars.len())
        };
        match c {
            '[' => {
                let mut depth = 0;
                while i < chars.len() {
                    match chars[i] {
                        '[' => depth += 1,
                        ']' => {
                            depth -= 1;
                            if depth == 0 {
                                i += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
            ':' => {
                if chars.get(i + 1) == Some(&':') {
                    return None;
                }
                i = name_end(i + 1);
                let name: String = chars[start + 1..i]
                    .iter()
                    .collect::<String>()
                    .to_ascii_lowercase();
                if matches!(
                    name.as_str(),
                    "before" | "after" | "first-line" | "first-letter"
                ) {
                    return None;
                }
                if chars.get(i) == Some(&'(') {
                    let mut depth = 0;
                    while i < chars.len() {
                        match chars[i] {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    i += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                }
            }
            '.' | '#' => i = name_end(i + 1),
            '*' => i += 1,
            _ => {
                i = name_end(i);
                if i == start {
                    return None;
                }
            }
        }
        parts.push(chars[start..i].iter().collect());
    }
    (!parts.is_empty()).then_some(parts)
}

/// `sortCompoundSelectorsInsideComplexSelector`: universal, tag, id,
/// class, attribute, pseudo (a second tag goes in `:is()`).
fn sort_compound(parts: Vec<String>) -> Vec<String> {
    let order = |p: &str| match p.chars().next() {
        Some('*') => 0,
        Some('#') => 4,
        Some('.') => 5,
        Some('[') => 6,
        Some(':') => 7,
        _ => 1,
    };
    let mut parts = parts;
    parts.sort_by_key(|p| order(p));
    parts
}

/// A complex selector with its non-descendant combinators spaced, as
/// postcss-selector-parser writes them after the resolve.
fn space_combinators(selector: &str) -> String {
    let mut out = String::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let chars: Vec<char> = selector.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, '(' | '[') => depth += 1,
            (None, ')' | ']') => depth -= 1,
            (None, '>' | '+' | '~')
                if depth == 0 && !(c == '+' && out.ends_with(|p: char| p.is_ascii_digit())) =>
            {
                let trimmed = out.trim_end().to_string();
                out = trimmed;
                out.push(' ');
                out.push(c);
                out.push(' ');
                i += 1;
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
                continue;
            }
            _ => {}
        }
        out.push(c);
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// @csstools/postcss-progressive-custom-properties: a declaration that
// repeats a property (a custom property, or one with var()) and uses
// newer features moves into an @supports of them after its rule.

struct Table {
    features: Vec<Value>,
    shorthands: Vec<(String, Vec<String>)>,
}

static TABLE: LazyLock<Table> = LazyLock::new(|| {
    let doc: Value = serde_json::from_str(include_str!(
        "../../vendor/csstools/progressive-custom-properties-5.1.2.json"
    ))
    .expect("the vendored feature table is JSON");
    let features = doc["features"].as_array().cloned().unwrap_or_default();
    let shorthands = doc["shorthands"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|pair| {
            let name = pair[0].as_str()?.to_string();
            let longhands = pair[1]
                .as_array()?
                .iter()
                .filter_map(|l| l.as_str().map(str::to_string))
                .collect();
            Some((name, longhands))
        })
        .collect();
    Table {
        features,
        shorthands,
    }
});

/// A postcss-value-parser node.
#[derive(Debug, Clone)]
struct VNode {
    kind: &'static str,
    value: String,
    nodes: Vec<VNode>,
}

/// postcss-value-parser's `parse`.
fn value_nodes(value: &str) -> Vec<VNode> {
    let chars: Vec<char> = value.chars().collect();
    let mut i = 0;
    value_parse(&chars, &mut i, false)
}

fn value_parse(chars: &[char], i: &mut usize, in_function: bool) -> Vec<VNode> {
    let node = |kind: &'static str, value: String| VNode {
        kind,
        value,
        nodes: Vec::new(),
    };
    let mut out: Vec<VNode> = Vec::new();
    while *i < chars.len() {
        let c = chars[*i];
        if in_function && c == ')' {
            *i += 1;
            return out;
        }
        if c.is_whitespace() {
            let start = *i;
            while *i < chars.len() && chars[*i].is_whitespace() {
                *i += 1;
            }
            let text: String = chars[start..*i].iter().collect();
            // Whitespace beside a div, or at a function's edges, belongs
            // to them.
            let next_div = *i < chars.len() && matches!(chars[*i], ',' | '/' | ':');
            let at_edge = (in_function
                && (out.is_empty() || (*i < chars.len() && chars[*i] == ')')))
                || *i >= chars.len()
                || out.is_empty();
            let after_div = out.last().is_some_and(|n| n.kind == "div");
            if !next_div && !at_edge && !after_div {
                out.push(node("space", text));
            }
            continue;
        }
        if c == '"' || c == '\'' {
            *i += 1;
            let start = *i;
            while *i < chars.len() && chars[*i] != c {
                if chars[*i] == '\\' {
                    *i += 1;
                }
                *i += 1;
            }
            let text: String = chars[start..(*i).min(chars.len())].iter().collect();
            *i = (*i + 1).min(chars.len());
            out.push(node("string", text));
            continue;
        }
        if c == '/' && chars.get(*i + 1) == Some(&'*') {
            let start = *i + 2;
            *i += 2;
            while *i < chars.len() && !(chars[*i] == '*' && chars.get(*i + 1) == Some(&'/')) {
                *i += 1;
            }
            let text: String = chars[start..(*i).min(chars.len())].iter().collect();
            *i = (*i + 2).min(chars.len());
            out.push(node("comment", text));
            continue;
        }
        if matches!(c, ',' | '/' | ':') {
            *i += 1;
            out.push(node("div", c.to_string()));
            continue;
        }
        if c == '(' {
            *i += 1;
            let mut f = node("function", String::new());
            f.nodes = value_parse(chars, i, true);
            out.push(f);
            continue;
        }
        if c == ')' {
            *i += 1;
            out.push(node("word", ")".to_string()));
            continue;
        }
        let start = *i;
        while *i < chars.len()
            && !chars[*i].is_whitespace()
            && !matches!(chars[*i], '"' | '\'' | ',' | '/' | ':' | '(' | ')')
        {
            if chars[*i] == '\\' {
                *i += 1;
            }
            *i += 1;
        }
        *i = (*i).min(chars.len());
        let text: String = chars[start..*i].iter().collect();
        if *i < chars.len() && chars[*i] == '(' {
            *i += 1;
            let mut f = node("function", text.clone());
            if text.eq_ignore_ascii_case("url") {
                // url()'s contents are one word.
                let inner_start = *i;
                while *i < chars.len() && chars[*i] != ')' {
                    *i += 1;
                }
                let inner: String = chars[inner_start..*i].iter().collect();
                *i = (*i + 1).min(chars.len());
                let trimmed = inner.trim();
                if !trimmed.is_empty() {
                    f.nodes.push(node(
                        if trimmed.starts_with(['"', '\'']) {
                            "string"
                        } else {
                            "word"
                        },
                        trimmed.trim_matches(['"', '\'']).to_string(),
                    ));
                }
            } else {
                f.nodes = value_parse(chars, i, true);
            }
            out.push(f);
        } else {
            out.push(node("word", text));
        }
    }
    out
}

/// postcss-value-parser's `unit`: a number's unit (possibly empty), None
/// for a value that does not start with a number.
fn unit(value: &str) -> Option<String> {
    let b = value.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let digits_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i > digits_start;
    if i < b.len() && b[i] == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        digits = true;
    }
    if !digits {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    Some(value[i..].to_string())
}

fn walk_values(nodes: &[VNode], f: &mut dyn FnMut(&VNode)) {
    for n in nodes {
        f(n);
        if n.kind == "function" {
            walk_values(&n.nodes, f);
        }
    }
}

/// The feature table's `matches`; `dimension` is the node's unit when it
/// is the node the walk is on (`conditionsFromValue` sets it as it goes,
/// so a node's children have none yet).
fn matches(matcher: &Value, node: &VNode, dimension: Option<&str>) -> bool {
    if matcher["isVariable"] == true && matches!(node.kind, "word" | "string" | "function") {
        return true;
    }
    if matcher["type"].as_str() != Some(node.kind) {
        return false;
    }
    if does_not_match_value(matcher, node, dimension) {
        return false;
    }
    if let Some(m_nodes) = matcher["nodes"].as_array() {
        let r_nodes = &node.nodes;
        for t in 0..m_nodes.len().max(r_nodes.len()) {
            let mut o = t;
            let mut a = t;
            while m_nodes.get(o).is_some_and(|m| m["type"] == "space") {
                o += 1;
            }
            while r_nodes.get(a).is_some_and(|r| r.kind == "space") {
                a += 1;
            }
            match (m_nodes.get(o), r_nodes.get(a)) {
                (None, None) => {}
                (Some(m), Some(r)) => {
                    if m["anyRemainingArguments"] == true {
                        return true;
                    }
                    if !matches(m, r, None) {
                        return false;
                    }
                }
                _ => return false,
            }
        }
    }
    true
}

fn does_not_match_value(matcher: &Value, node: &VNode, dimension: Option<&str>) -> bool {
    let m_value = matcher["value"].as_str();
    if matcher["type"] == "space"
        && node.kind == "space"
        && m_value.map(str::trim) == Some(node.value.trim())
    {
        return false;
    }
    if let (Some(d), Some(unit)) = (matcher.get("dimension"), dimension) {
        return d["unit"].as_str() != Some(unit);
    }
    if matcher["type"] == "string" {
        return m_value != Some(node.value.as_str());
    }
    m_value.map(str::to_lowercase) != Some(node.value.to_lowercase())
}

/// `doublePositionGradients`
fn double_position_gradients(node: &VNode) -> Vec<String> {
    let mut out = Vec::new();
    if node.kind != "function" {
        return out;
    }
    let name = node.value.to_lowercase();
    const KINDS: [&str; 6] = [
        "conic-gradient",
        "linear-gradient",
        "radial-gradient",
        "repeating-conic-gradient",
        "repeating-linear-gradient",
        "repeating-radial-gradient",
    ];
    if !KINDS.contains(&name.as_str()) {
        return out;
    }
    const KEYWORDS: [&str; 15] = [
        "at",
        "bottom",
        "center",
        "circle",
        "closest-corner",
        "closest-side",
        "ellipse",
        "farthest-corner",
        "farthest-side",
        "from",
        "in",
        "left",
        "right",
        "to",
        "top",
    ];
    let (mut count, mut keyword, mut interpolation) = (0, false, false);
    for n in &node.nodes {
        if n.kind == "word" && KEYWORDS.contains(&n.value.to_lowercase().as_str()) {
            keyword = true;
        }
        if n.kind == "div" && n.value.trim() == "," {
            count = 0;
            keyword = false;
            continue;
        }
        if n.kind == "word" && n.value.eq_ignore_ascii_case("in") {
            interpolation = true;
            continue;
        }
        if n.kind == "word" || n.kind == "function" {
            count += 1;
        }
        if interpolation {
            out.push(
                match name.as_str() {
                    "conic-gradient" => "(background: conic-gradient(in oklch, red 0deg, red 0deg 1deg, red 2deg))",
                    "linear-gradient" => "(background: linear-gradient(in oklch, red 0%, red 0% 1%, red 2%))",
                    "radial-gradient" => "(background: radial-gradient(in oklch, red, red 1px 2px, red 3px))",
                    "repeating-conic-gradient" => "(background: repeating-conic-gradient(in oklch from 0deg, red 0deg, red 0deg 1deg, red 2deg))",
                    "repeating-linear-gradient" => "(background: repeating-linear-gradient(in oklch, red 0%, red 0% 1%, red 2%))",
                    _ => "(background: repeating-radial-gradient(in oklch, red, red 1px 2px, red 3px))",
                }
                .to_string(),
            );
            break;
        }
        if !keyword && count == 3 {
            out.push(
                match name.as_str() {
                    "conic-gradient" => "(background: conic-gradient(red 0deg, red 0deg 1deg, red 2deg))",
                    "linear-gradient" => "(background: linear-gradient(red 0%, red 0% 1%, red 2%))",
                    "radial-gradient" => "(background: radial-gradient(red, red 1px 2px, red 3px))",
                    "repeating-conic-gradient" => "(background: repeating-conic-gradient(from 0deg, red 0deg, red 0deg 1deg, red 2deg))",
                    "repeating-linear-gradient" => "(background: repeating-linear-gradient(red 0%, red 0% 1%, red 2%))",
                    _ => "(background: repeating-radial-gradient(red, red 1px 2px, red 3px))",
                }
                .to_string(),
            );
            break;
        }
    }
    out
}

/// `conditionsFromValue`: the @supports conditions of the features a
/// value uses; none for a declaration that is not a custom property
/// unless it uses var().
fn conditions(decl: &Decl) -> Vec<String> {
    let value = &decl.value;
    let candidates: Vec<&Value> = TABLE
        .features
        .iter()
        .filter(|f| {
            f["sniff"].as_str().is_some_and(|s| value.contains(s))
                && f["only_on_property"]
                    .as_str()
                    .is_none_or(|p| p == decl.prop)
        })
        .collect();
    let mut found = BTreeSet::new();
    let mut uses_var = false;
    walk_values(&value_nodes(value), &mut |n| {
        let dimension = unit(&n.value);
        if n.kind == "function" && n.value.eq_ignore_ascii_case("var") {
            uses_var = true;
        }
        'features: for feature in &candidates {
            for matcher in feature["matchers"].as_array().into_iter().flatten() {
                if matches(matcher, n, dimension.as_deref()) {
                    found.insert(format!(
                        "({}: {})",
                        feature["property"].as_str().unwrap_or_default(),
                        feature["supports"].as_str().unwrap_or_default()
                    ));
                    break 'features;
                }
            }
        }
        found.extend(double_position_gradients(n));
    });
    if !decl.variable() && !uses_var {
        return Vec::new();
    }
    found.into_iter().collect()
}

fn progressive(nodes: &mut Vec<Node>) -> Result<(), Unsupported> {
    let mut i = 0;
    while i < nodes.len() {
        let wrappers = match &mut nodes[i] {
            Node::Rule(r) => {
                let mut kids = std::mem::take(&mut r.nodes);
                let wrappers = container_progressive(&mut kids, |kids| {
                    Node::Rule(Rule {
                        before: Some("\n".to_string()),
                        selector: r.selector.clone(),
                        between: r.between.clone(),
                        nodes: kids,
                        semicolon: r.semicolon,
                        after: r.after.clone(),
                    })
                })?;
                r.nodes = kids;
                wrappers
            }
            Node::AtRule(a) => {
                if let Some(mut kids) = a.nodes.take() {
                    progressive(&mut kids)?;
                    let wrappers = container_progressive(&mut kids, |kids| {
                        Node::AtRule(AtRule {
                            before: Some("\n".to_string()),
                            name: a.name.clone(),
                            after_name: a.after_name.clone(),
                            params: a.params.clone(),
                            between: a.between.clone(),
                            nodes: Some(kids),
                            semicolon: a.semicolon,
                            after: a.after.clone(),
                        })
                    })?;
                    a.nodes = Some(kids);
                    wrappers
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        };
        let count = wrappers.len();
        nodes.splice(i + 1..i + 1, wrappers);
        i += count + 1;
    }
    Ok(())
}

/// The plugin's walk over one container's declarations; the @supports
/// rules that go after it.
fn container_progressive(
    nodes: &mut Vec<Node>,
    clone_parent: impl Fn(Vec<Node>) -> Node,
) -> Result<Vec<Node>, Unsupported> {
    let mut seen: Vec<String> = Vec::new();
    // (condition, the declarations of its clone)
    let mut groups: Vec<(String, Vec<Node>)> = Vec::new();
    let mut last: Option<usize> = None;
    let mut i = 0;
    while i < nodes.len() {
        let Node::Decl(d) = &nodes[i] else {
            i += 1;
            continue;
        };
        let key = if d.variable() {
            d.prop.clone()
        } else {
            d.prop.to_lowercase()
        };
        if !seen.contains(&key) {
            seen.push(key);
            i += 1;
            continue;
        }
        if !d.variable() && !d.value.to_lowercase().contains("var(") {
            i += 1;
            continue;
        }
        if d.value.eq_ignore_ascii_case("initial") || d.value.trim().is_empty() {
            i += 1;
            continue;
        }
        let condition = conditions(d).join(" and ");
        if condition.is_empty() {
            i += 1;
            continue;
        }
        let d = d.clone();
        // cloneDeclarations: a shorthand takes its later longhands along.
        let mut moved = vec![Node::Decl(d.clone())];
        if let Some((_, longhands)) = TABLE
            .shorthands
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(&d.prop))
        {
            for n in &nodes[i + 1..] {
                if let Node::Decl(o) = n
                    && longhands.contains(&o.prop)
                {
                    moved.push(Node::Decl(o.clone()));
                }
            }
        }
        match last {
            Some(g) if groups[g].0 == condition => groups[g].1.extend(moved),
            _ => {
                groups.push((condition, moved));
                last = Some(groups.len() - 1);
            }
        }
        nodes.remove(i);
    }
    Ok(groups
        .into_iter()
        .map(|(condition, decls)| {
            Node::AtRule(AtRule {
                before: Some("\n\n".to_string()),
                name: "supports".to_string(),
                after_name: None,
                params: condition,
                between: None,
                nodes: Some(vec![clone_parent(decls)]),
                semicolon: None,
                after: Some("\n".to_string()),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(css: &str) -> String {
        post_css(css, css).unwrap()
    }

    #[test]
    fn leaves_plain_css_alone() {
        let css = ":root{--x: 1px;color:red}@media(width < 10px){a{b:c}}";
        assert_eq!(run(css), css);
    }

    /// What AssetProcessor#post_css made of each input on the reference
    /// (parity/stylesheets/postcss, recorded by scripts/record-postcss
    /// with the source map file `x.css.map`): light-dark() in custom
    /// properties and other declarations, nested, with `!important`, under
    /// list, complex and pseudo-element selectors and in @media, in
    /// at-rules that hold declarations (@font-face, @page), the
    /// progressive @supports of repeated declarations, color-scheme; and autoprefixer's removed
    /// keyframes, properties and values and its added prefixes.
    #[test]
    fn post_css_matches_the_reference() {
        for name in ["light_dark", "light_dark_at_rules", "autoprefixer"] {
            let dir =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("parity/stylesheets/postcss");
            let input = std::fs::read_to_string(dir.join(format!("{name}.css"))).unwrap();
            let rails = std::fs::read_to_string(dir.join(format!("{name}.out.css"))).unwrap();
            let rails = rails
                .strip_suffix("\n/*# sourceMappingURL=x.css.map */")
                .unwrap();
            assert_eq!(post_css(&input, &input).unwrap(), rails, "{name}");
        }
    }

    #[test]
    fn hashes_as_djb2() {
        assert_eq!(djb2(""), "1505");
        assert_eq!(djb2("a"), "2b606");
    }
}
