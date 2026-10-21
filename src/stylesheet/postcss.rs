//! AssetProcessor#post_css (frontend/asset-processor/postcss.js) on what
//! the Sass compile prints: postcss-variable-renamer (its renames are
//! empty), autoprefixer for Discourse's browsers, the light-dark()
//! polyfill (@csstools/postcss-light-dark-function 3.0.3 with
//! @csstools/postcss-progressive-custom-properties 5.1.2), postcss-nesting
//! (14.0.1) un-nesting what the polyfill nests, and
//! postcss-variable-prefixer namespacing the polyfill's toggles by a hash
//! of the input; and Rtlcss.flip_css for a right-to-left target ([`flip`]).
//!
//! post_css's input is always dart-sass's compressed output, so new nodes
//! get the raws PostCSS gives them there; flip parses post_css's own
//! output, the whitespace between nodes kept as their raws. vendor/csstools
//! holds postcss-progressive-custom-properties' feature table and
//! shorthands (MIT-0), extracted from its 5.1.2 build. autoprefixer is
//! [`autoprefixer`], Rtlcss [`rtl`].

#[path = "autoprefixer.rs"]
mod autoprefixer;
#[path = "rtl.rs"]
mod rtl;

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
                Some(b'}') if !top => {
                    // The whitespace before the brace is the container's
                    // raws.after.
                    self.i -= before.len();
                    return Ok((nodes, semicolon));
                }
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
    // dart-sass writes a byte order mark before non-ASCII output, which
    // postcss leaves out of the text it parses and hashes.
    let (bom, css) = unmarked(css);
    let mut nodes = parse(css)?;
    let hash = djb2(unmarked(source).1);
    // postcss runs every plugin's node visitors in one walk (the polyfill,
    // postcss-nesting, the prefixer), then the OnceExit ones in plugin
    // order: autoprefixer, then the progressive custom properties. So
    // autoprefixer sees the polyfilled declarations.
    let mut toggles = 0;
    light_dark(&mut nodes, &mut toggles, false)?;
    autoprefixer::autoprefixer(&mut nodes)?;
    progressive(&mut nodes)?;
    prefix_toggles(&mut nodes, &hash);
    // The root's raws.after: the whitespace after the last node.
    Ok(format!(
        "{bom}{}{}",
        stringify(&nodes),
        &css[css.trim_end().len()..]
    ))
}

/// postcss's Input: the text without a byte order mark, and the mark the
/// result gets back.
fn unmarked(text: &str) -> (&'static str, &str) {
    match text.strip_prefix(['\u{feff}', '\u{fffe}']) {
        Some(rest) => ("\u{feff}", rest),
        None => ("", text),
    }
}

/// `Rtlcss.flip_css`: post_css's output for a right-to-left target, as
/// Stylesheet::Compiler flips it (see [`rtl`]). rtlcss's own postcss gives
/// no byte order mark back, and drops the source map annotation.
pub fn flip(css: &str) -> Result<String, Unsupported> {
    let (_, css) = unmarked(css);
    let mut nodes = parse(css)?;
    rtl::flip(&mut nodes)?;
    Ok(format!(
        "{}{}",
        stringify(&nodes),
        &css[css.trim_end().len()..]
    ))
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
    /// progressive @supports of repeated declarations, color-scheme, a
    /// byte order mark; and autoprefixer: the outdated at-rules,
    /// selectors, properties, values (old flexbox among them) and
    /// transition entries it removes, the property and value prefixes,
    /// transitions and @supports conditions it adds, and what it does
    /// with the polyfill's declarations; and, for an `rtl_` input,
    /// Rtlcss.flip_css of that.
    #[test]
    fn post_css_matches_the_reference() {
        for name in [
            "light_dark",
            "light_dark_at_rules",
            "light_dark_bom",
            "autoprefixer",
            "autoprefixer_values",
            "autoprefixer_selectors",
            "autoprefixer_transitions",
            "autoprefixer_supports",
            "autoprefixer_flexbox",
            "autoprefixer_misc",
            "autoprefixer_light_dark",
            "rtl_flip",
            "rtl_bom",
        ] {
            let dir =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("parity/stylesheets/postcss");
            let input = std::fs::read_to_string(dir.join(format!("{name}.css"))).unwrap();
            let rails = std::fs::read_to_string(dir.join(format!("{name}.out.css"))).unwrap();
            let ours = post_css(&input, &input).unwrap_or_else(|e| panic!("{name}: {e}"));
            // A right-to-left one is flipped, which drops the annotation.
            let (ours, rails) = if name.starts_with("rtl_") {
                (flip(&ours).unwrap(), rails.as_str())
            } else {
                let rails = rails
                    .strip_suffix("\n/*# sourceMappingURL=x.css.map */")
                    .unwrap();
                (ours, rails)
            };
            if ours != rails {
                let at = ours
                    .bytes()
                    .zip(rails.bytes())
                    .position(|(a, b)| a != b)
                    .unwrap_or(ours.len().min(rails.len()));
                panic!(
                    "{name} differs at {at}:\nours:  {}\nrails: {}",
                    &ours[at.saturating_sub(60)..(at + 120).min(ours.len())],
                    &rails[at.saturating_sub(60)..(at + 120).min(rails.len())]
                );
            }
        }
    }

    #[test]
    fn hashes_as_djb2() {
        assert_eq!(djb2(""), "1505");
        assert_eq!(djb2("a"), "2b606");
    }
}
