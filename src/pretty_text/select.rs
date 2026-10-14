//! A small Nokogiri-style CSS selector engine over an rcdom tree, for the
//! selectors the ported code uses: tags (`*` too), ids, classes,
//! attributes (`[a]`, `=`, `^=`, `$=`, `*=`, `~=`), `:not(...)`,
//! `:first-child`, `:last-child`, the descendant, child (`>`), adjacent
//! (`+`) and general sibling (`~`) combinators, and selector groups.
//! Like Nokogiri's, results are in document order and tags match by
//! local name whatever the element's namespace.

use std::rc::Rc;

use markup5ever_rcdom::{Handle, NodeData};

use crate::pretty_text::cleanup::{attr, element_name};

#[derive(Debug, Clone)]
enum AttrOp {
    Exists,
    Equals(String),
    StartsWith(String),
    EndsWith(String),
    Contains(String),
    Includes(String),
}

#[derive(Debug, Clone, Default)]
struct Compound {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<(String, AttrOp)>,
    not: Vec<Compound>,
    first_child: bool,
    last_child: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Combinator {
    Descendant,
    Child,
    Adjacent,
    Sibling,
}

/// A complex selector, rightmost compound first, each with the combinator
/// to its left.
type Complex = Vec<(Compound, Option<Combinator>)>;

/// A parsed selector group.
#[derive(Debug, Clone)]
pub struct Selector(Vec<Complex>);

fn ident_end(r: &str) -> usize {
    r.find(|ch: char| !(ch.is_alphanumeric() || ch == '-' || ch == '_'))
        .unwrap_or(r.len())
}

fn parse_compound(s: &str) -> (Compound, &str) {
    let mut c = Compound::default();
    let mut rest = s;
    if let Some(r) = rest.strip_prefix('*') {
        rest = r;
    } else {
        let end = ident_end(rest);
        if end > 0 {
            c.tag = Some(rest[..end].to_ascii_lowercase());
            rest = &rest[end..];
        }
    }
    loop {
        if let Some(r) = rest.strip_prefix('.') {
            let end = ident_end(r);
            c.classes.push(r[..end].to_string());
            rest = &r[end..];
        } else if let Some(r) = rest.strip_prefix('#') {
            let end = ident_end(r);
            c.id = Some(r[..end].to_string());
            rest = &r[end..];
        } else if let Some(r) = rest.strip_prefix('[') {
            let close = r.find(']').expect("selectors are written closed");
            let inner = &r[..close];
            let unquote = |v: &str| {
                v.trim()
                    .trim_matches(|ch| ch == '"' || ch == '\'')
                    .to_string()
            };
            let name = |n: &str| n.trim().to_string();
            let entry = if let Some((n, v)) = inner.split_once("$=") {
                (name(n), AttrOp::EndsWith(unquote(v)))
            } else if let Some((n, v)) = inner.split_once("^=") {
                (name(n), AttrOp::StartsWith(unquote(v)))
            } else if let Some((n, v)) = inner.split_once("*=") {
                (name(n), AttrOp::Contains(unquote(v)))
            } else if let Some((n, v)) = inner.split_once("~=") {
                (name(n), AttrOp::Includes(unquote(v)))
            } else if let Some((n, v)) = inner.split_once('=') {
                (name(n), AttrOp::Equals(unquote(v)))
            } else {
                (name(inner), AttrOp::Exists)
            };
            c.attrs.push(entry);
            rest = &r[close + 1..];
        } else if let Some(r) = rest.strip_prefix(":not(") {
            let close = r.find(')').expect("selectors are written closed");
            let (inner, _) = parse_compound(r[..close].trim());
            c.not.push(inner);
            rest = &r[close + 1..];
        } else if let Some(r) = rest.strip_prefix(":first-child") {
            c.first_child = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix(":last-child") {
            c.last_child = true;
            rest = r;
        } else {
            break;
        }
    }
    (c, rest)
}

fn parse_complex(s: &str) -> Complex {
    let mut parts: Vec<(Compound, Option<Combinator>)> = Vec::new();
    let mut rest = s.trim();
    let mut pending: Option<Combinator> = None;
    while !rest.is_empty() {
        let (c, r) = parse_compound(rest);
        assert!(r.len() < rest.len(), "unsupported selector: {s}");
        parts.push((c, pending.take()));
        let trimmed = r.trim_start();
        if let Some(r) = trimmed.strip_prefix('>') {
            pending = Some(Combinator::Child);
            rest = r.trim_start();
        } else if let Some(r) = trimmed.strip_prefix('+') {
            pending = Some(Combinator::Adjacent);
            rest = r.trim_start();
        } else if let Some(r) = trimmed.strip_prefix('~') {
            pending = Some(Combinator::Sibling);
            rest = r.trim_start();
        } else if !trimmed.is_empty() {
            pending = Some(Combinator::Descendant);
            rest = trimmed;
        } else {
            rest = trimmed;
        }
    }
    parts.into_iter().rev().collect()
}

impl Selector {
    pub fn parse(s: &str) -> Selector {
        Selector(s.split(',').map(parse_complex).collect())
    }

    /// Whether the element matches (`node.matches?(selector)`).
    pub fn matches(&self, node: &Handle) -> bool {
        self.0.iter().any(|c| matches_complex(node, c))
    }
}

pub(crate) fn is_element(node: &Handle) -> bool {
    matches!(node.data, NodeData::Element { .. })
}

pub(crate) fn element_children(node: &Handle) -> Vec<Handle> {
    node.children
        .borrow()
        .iter()
        .filter(|c| is_element(c))
        .cloned()
        .collect()
}

pub(crate) fn parent(node: &Handle) -> Option<Handle> {
    let weak = node.parent.take();
    let parent = weak.as_ref().and_then(|w| w.upgrade());
    node.parent.set(weak);
    parent
}

/// The element siblings before the node, nearest first.
pub(crate) fn previous_elements(node: &Handle) -> Vec<Handle> {
    let Some(p) = parent(node) else {
        return Vec::new();
    };
    let siblings = element_children(&p);
    let Some(i) = siblings.iter().position(|s| Rc::ptr_eq(s, node)) else {
        return Vec::new();
    };
    siblings[..i].iter().rev().cloned().collect()
}

fn matches_compound(node: &Handle, c: &Compound) -> bool {
    let Some(name) = element_name(node) else {
        return false;
    };
    if c.tag.as_deref().is_some_and(|t| t != name) {
        return false;
    }
    if let Some(id) = &c.id
        && attr(node, "id").as_deref() != Some(id.as_str())
    {
        return false;
    }
    if !c.classes.is_empty() {
        let class = attr(node, "class").unwrap_or_default();
        let tokens: Vec<&str> = class
            .split([' ', '\t', '\n', '\r'])
            .filter(|t| !t.is_empty())
            .collect();
        if !c.classes.iter().all(|k| tokens.contains(&k.as_str())) {
            return false;
        }
    }
    for (name, op) in &c.attrs {
        let Some(value) = attr(node, name) else {
            return false;
        };
        let ok = match op {
            AttrOp::Exists => true,
            AttrOp::Equals(v) => &value == v,
            AttrOp::StartsWith(v) => value.starts_with(v.as_str()),
            AttrOp::EndsWith(v) => !v.is_empty() && value.ends_with(v.as_str()),
            AttrOp::Contains(v) => value.contains(v.as_str()),
            AttrOp::Includes(v) => value.split_whitespace().any(|t| t == v),
        };
        if !ok {
            return false;
        }
    }
    if c.not.iter().any(|n| matches_compound(node, n)) {
        return false;
    }
    if c.first_child || c.last_child {
        let Some(p) = parent(node) else {
            return false;
        };
        let siblings = element_children(&p);
        if c.first_child && !siblings.first().is_some_and(|s| Rc::ptr_eq(s, node)) {
            return false;
        }
        if c.last_child && !siblings.last().is_some_and(|s| Rc::ptr_eq(s, node)) {
            return false;
        }
    }
    true
}

fn matches_complex(node: &Handle, parts: &[(Compound, Option<Combinator>)]) -> bool {
    let Some(((compound, combinator), rest)) = parts.split_first() else {
        return true;
    };
    if !matches_compound(node, compound) {
        return false;
    }
    match combinator {
        None => rest.is_empty(),
        Some(Combinator::Child) => parent(node).is_some_and(|p| matches_complex(&p, rest)),
        Some(Combinator::Descendant) => {
            let mut ancestor = parent(node);
            while let Some(a) = ancestor {
                if matches_complex(&a, rest) {
                    return true;
                }
                ancestor = parent(&a);
            }
            false
        }
        Some(Combinator::Adjacent) => previous_elements(node)
            .first()
            .is_some_and(|p| matches_complex(p, rest)),
        Some(Combinator::Sibling) => previous_elements(node)
            .iter()
            .any(|p| matches_complex(p, rest)),
    }
}

fn descendants(node: &Handle, out: &mut Vec<Handle>) {
    for child in node.children.borrow().iter() {
        if is_element(child) {
            out.push(child.clone());
        }
        descendants(child, out);
    }
}

/// Every element under `root` (not `root` itself), in document order.
pub(crate) fn all_descendants(root: &Handle) -> Vec<Handle> {
    let mut out = Vec::new();
    descendants(root, &mut out);
    out
}

/// `root.css(selector)`: the elements under `root` that match, in
/// document order.
pub fn select(root: &Handle, selector: &str) -> Vec<Handle> {
    let selector = Selector::parse(selector);
    all_descendants(root)
        .into_iter()
        .filter(|n| selector.matches(n))
        .collect()
}

/// `root.at_css(selector)`
pub fn select_first(root: &Handle, selector: &str) -> Option<Handle> {
    let selector = Selector::parse(selector);
    all_descendants(root)
        .into_iter()
        .find(|n| selector.matches(n))
}

/// `node.ancestors(selector)`: the node's ancestors that match, nearest
/// first.
pub fn ancestors(node: &Handle, selector: &str) -> Vec<Handle> {
    let selector = Selector::parse(selector);
    let mut out = Vec::new();
    let mut current = parent(node);
    while let Some(p) = current {
        if selector.matches(&p) {
            out.push(p.clone());
        }
        current = parent(&p);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pretty_text::cleanup::parse_document;

    fn count(html: &str, selector: &str) -> usize {
        select(&parse_document(html).document, selector).len()
    }

    #[test]
    fn selectors() {
        let html = r#"<div class="footer highlight" id="f"><a href="/x">x</a><p>1</p><p>2</p></div><blockquote><p>q</p></blockquote><img src="a.svg"><img src="b.png" class="emoji">"#;
        assert_eq!(count(html, ".footer.highlight"), 1);
        assert_eq!(count(html, "#f > p"), 2);
        assert_eq!(count(html, ".footer a"), 1);
        assert_eq!(count(html, "div > p:first-child"), 0);
        assert_eq!(count(html, "div > a:first-child"), 1);
        assert_eq!(count(html, "div > p:last-child"), 1);
        assert_eq!(count(html, "blockquote > p"), 1);
        assert_eq!(count(html, "img[src$=\".svg\"]"), 1);
        assert_eq!(count(html, "img[src^='b']"), 1);
        assert_eq!(count(html, "img[src*=\".p\"]"), 1);
        assert_eq!(count(html, "img:not(.emoji)"), 1);
        assert_eq!(count(html, "svg, img"), 2);
        assert_eq!(count(html, "a + p"), 1);
        assert_eq!(count(html, "a ~ p"), 2);
    }
}
