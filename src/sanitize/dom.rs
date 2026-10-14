//! The DOM Sanitize works on: html5ever parses as Nokogiri::HTML5 (Gumbo)
//! does, with scripting disabled, template contents as children, and
//! Gumbo's depth and attribute limits; and the serializer is Nokogiri's
//! HTML5 one (`to_html(preserve_newline: true)`).

use std::cell::RefCell;
use std::rc::Rc;

use html5ever::tendril::TendrilSink;
use html5ever::tree_builder::TreeBuilderOpts;
use html5ever::{LocalName, ParseOpts, QualName, local_name, namespace_url, ns};
use markup5ever_rcdom::{Handle, Node, NodeData, RcDom};

use super::Error;
use super::config::ParserOptions;
use crate::pretty_text::cleanup;

/// `node[name]`
pub fn attr(node: &Handle, name: &str) -> Option<String> {
    cleanup::attr(node, name)
}

/// `node[name] = value`: replaced in place, else appended.
pub fn set_attr(node: &Handle, name: &str, value: &str) {
    cleanup::set_attr(node, name, value)
}

/// An HTML element with the attributes.
pub fn new_element(tag: &str, attrs: &[(&str, &str)]) -> Handle {
    cleanup::new_element(tag, attrs)
}

pub fn new_text(content: &str) -> Handle {
    cleanup::new_text(content)
}

/// `node.content`: the text of every descendant.
pub fn text(node: &Handle) -> String {
    cleanup::text(node)
}

fn opts() -> ParseOpts {
    ParseOpts {
        tree_builder: TreeBuilderOpts {
            scripting_enabled: false,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// `Nokogiri::HTML5.fragment(html)`: the nodes, under a document node
/// that stands for the fragment.
pub(super) fn parse_fragment(html: &str, options: &ParserOptions) -> Result<Handle, Error> {
    let context = QualName::new(None, ns!(html), local_name!("body"));
    let dom = html5ever::parse_fragment(RcDom::default(), opts(), context, vec![])
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .expect("reading from a byte slice cannot fail");
    let html_root = dom
        .document
        .children
        .borrow()
        .first()
        .cloned()
        .unwrap_or_else(|| dom.document.clone());
    let frag = Node::new(NodeData::Document);
    let nodes = std::mem::take(&mut *html_root.children.borrow_mut());
    for node in nodes {
        append(&frag, node);
    }
    // The RcDom clears whatever is still reachable from it when dropped;
    // nothing is now.
    drop(dom);
    finish(&frag, options)?;
    Ok(frag)
}

/// `Nokogiri::HTML5.parse(html)`
pub(super) fn parse_document(html: &str, options: &ParserOptions) -> Result<Handle, Error> {
    let dom = html5ever::parse_document(RcDom::default(), opts())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .expect("reading from a byte slice cannot fail");
    let doc = dom.document.clone();
    // Keep the tree: dropping the RcDom would clear the document's
    // descendants, so take the document's children into a fresh node.
    let root = Node::new(NodeData::Document);
    let nodes = std::mem::take(&mut *doc.children.borrow_mut());
    for node in nodes {
        append(&root, node);
    }
    drop(dom);
    finish(&root, options)?;
    Ok(root)
}

/// Template contents moved into their elements (Gumbo keeps them as
/// children), then Gumbo's limits checked.
fn finish(root: &Handle, options: &ParserOptions) -> Result<(), Error> {
    fn walk(node: &Handle, depth: i64, options: &ParserOptions) -> Result<(), Error> {
        if let NodeData::Element {
            template_contents,
            attrs,
            ..
        } = &node.data
        {
            if options.max_tree_depth >= 0 && depth > options.max_tree_depth {
                return Err(Error::TreeTooDeep);
            }
            if options.max_attributes >= 0 && attrs.borrow().len() as i64 > options.max_attributes {
                return Err(Error::TooManyAttributes);
            }
            if let Some(contents) = template_contents.borrow_mut().take() {
                let nodes = std::mem::take(&mut *contents.children.borrow_mut());
                for child in nodes {
                    append(node, child);
                }
            }
        }
        let children = node.children.borrow().clone();
        let child_depth = if matches!(node.data, NodeData::Element { .. }) {
            depth + 1
        } else {
            depth
        };
        for child in &children {
            walk(child, child_depth, options)?;
        }
        Ok(())
    }
    walk(root, 1, options)
}

pub fn is_element(node: &Handle) -> bool {
    matches!(node.data, NodeData::Element { .. })
}

pub fn parent(node: &Handle) -> Option<Handle> {
    let weak = node.parent.take();
    let parent = weak.as_ref().and_then(|p| p.upgrade());
    node.parent.set(weak);
    parent
}

fn index_in_parent(node: &Handle) -> Option<(Handle, usize)> {
    let parent = parent(node)?;
    let i = parent
        .children
        .borrow()
        .iter()
        .position(|c| Rc::ptr_eq(c, node))?;
    Some((parent, i))
}

pub fn children(node: &Handle) -> Vec<Handle> {
    node.children.borrow().clone()
}

pub fn first_child(node: &Handle) -> Option<Handle> {
    node.children.borrow().first().cloned()
}

pub fn previous_sibling(node: &Handle) -> Option<Handle> {
    let (parent, i) = index_in_parent(node)?;
    let children = parent.children.borrow();
    i.checked_sub(1).map(|j| children[j].clone())
}

pub fn next_sibling(node: &Handle) -> Option<Handle> {
    let (parent, i) = index_in_parent(node)?;
    parent.children.borrow().get(i + 1).cloned()
}

/// `node.unlink`
pub fn unlink(node: &Handle) {
    if let Some((parent, i)) = index_in_parent(node) {
        parent.children.borrow_mut().remove(i);
    }
    node.parent.set(None);
}

pub(super) fn append(parent: &Handle, child: Handle) {
    unlink(&child);
    child.parent.set(Some(Rc::downgrade(parent)));
    parent.children.borrow_mut().push(child);
}

fn insert_at(node: &Handle, new: Handle, after: bool) {
    unlink(&new);
    let Some((parent, i)) = index_in_parent(node) else {
        return;
    };
    new.parent.set(Some(Rc::downgrade(&parent)));
    parent
        .children
        .borrow_mut()
        .insert(if after { i + 1 } else { i }, new);
}

/// `node.add_previous_sibling(new)`
pub fn add_previous_sibling(node: &Handle, new: Handle) {
    insert_at(node, new, false);
}

/// `node.add_next_sibling(new)`
pub fn add_next_sibling(node: &Handle, new: Handle) {
    insert_at(node, new, true);
}

/// `node.add_previous_sibling(node.children)`: the children moved out,
/// in order, before the node.
pub(super) fn move_children_before(node: &Handle) {
    for child in children(node) {
        add_previous_sibling(node, child);
    }
}

pub fn has_attr(node: &Handle, name: &str) -> bool {
    attr(node, name).is_some()
}

/// `node.remove_attribute(name)`
pub fn remove_attr(node: &Handle, name: &str) {
    if let NodeData::Element { attrs, .. } = &node.data {
        attrs.borrow_mut().retain(|a| &*a.name.local != name);
    }
}

/// `node.name = name`: an element can't be renamed in place here, so a
/// new one with the attributes and children takes its place; the new
/// node is returned (a transformer sets `env.node` to it).
pub fn rename(node: &Handle, name: &str) -> Handle {
    let NodeData::Element {
        name: qual, attrs, ..
    } = &node.data
    else {
        return node.clone();
    };
    let renamed = Node::new(NodeData::Element {
        name: QualName::new(qual.prefix.clone(), qual.ns.clone(), LocalName::from(name)),
        attrs: RefCell::new(attrs.borrow().clone()),
        template_contents: RefCell::new(None),
        mathml_annotation_xml_integration_point: false,
    });
    for child in children(node) {
        append(&renamed, child);
    }
    if parent(node).is_some() {
        add_previous_sibling(node, renamed.clone());
        unlink(node);
    }
    renamed
}

/// A doctype node (`create_internal_subset(name, nil, nil)`).
pub(super) fn new_doctype(name: &str) -> Handle {
    Node::new(NodeData::Doctype {
        name: name.into(),
        public_id: "".into(),
        system_id: "".into(),
    })
}

pub(super) fn prepend(parent: &Handle, child: Handle) {
    unlink(&child);
    child.parent.set(Some(Rc::downgrade(parent)));
    parent.children.borrow_mut().insert(0, child);
}

const VOID_ELEMENTS: [&str; 18] = [
    "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input",
    "keygen", "link", "meta", "param", "source", "track", "wbr",
];

const UNESCAPED_TEXT_ELEMENTS: [&str; 8] = [
    "style",
    "script",
    "xmp",
    "iframe",
    "noembed",
    "noframes",
    "plaintext",
    "noscript",
];

/// Whether an HTML-namespace element has one of the names.
fn is_one_of(node: &Handle, names: &[&str]) -> bool {
    match &node.data {
        NodeData::Element { name, .. } => name.ns == ns!(html) && names.contains(&&*name.local),
        _ => false,
    }
}

fn escape(out: &mut String, s: &str, attr: bool) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            '"' if attr => out.push_str("&quot;"),
            '<' if !attr => out.push_str("&lt;"),
            '>' if !attr => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
}

/// Nokogiri's `html_standard_serialize(preserve_newline: true)`.
pub(super) fn serialize(node: &Handle) -> String {
    let mut out = String::new();
    output_node(&mut out, node);
    out
}

fn should_prepend_newline(node: &Handle) -> bool {
    let NodeData::Element { name, .. } = &node.data else {
        return false;
    };
    if !matches!(&*name.local, "pre" | "textarea" | "listing") {
        return false;
    }
    match node.children.borrow().first().map(|c| &c.data) {
        Some(NodeData::Text { contents }) => contents.borrow().starts_with('\n'),
        _ => false,
    }
}

fn output_node(out: &mut String, node: &Handle) {
    match &node.data {
        NodeData::Element { name, attrs, .. } => {
            out.push('<');
            out.push_str(&name.local);
            for a in attrs.borrow().iter() {
                out.push(' ');
                let local = &*a.name.local;
                if a.name.ns == ns!(xml) {
                    out.push_str("xml:");
                } else if a.name.ns == ns!(xmlns) {
                    if local != "xmlns" {
                        out.push_str("xmlns:");
                    }
                } else if a.name.ns == ns!(xlink) {
                    out.push_str("xlink:");
                } else if let Some(prefix) = &a.name.prefix {
                    out.push_str(prefix);
                    out.push(':');
                }
                out.push_str(local);
                out.push_str("=\"");
                escape(out, &a.value, true);
                out.push('"');
            }
            out.push('>');
            if !is_one_of(node, &VOID_ELEMENTS) {
                if should_prepend_newline(node) {
                    out.push('\n');
                }
                for child in node.children.borrow().iter() {
                    output_node(out, child);
                }
                out.push_str("</");
                out.push_str(&name.local);
                out.push('>');
            }
        }
        NodeData::Text { contents } => {
            let raw = parent(node).is_some_and(|p| is_one_of(&p, &UNESCAPED_TEXT_ELEMENTS));
            if raw {
                out.push_str(&contents.borrow());
            } else {
                escape(out, &contents.borrow(), false);
            }
        }
        NodeData::Comment { contents } => {
            out.push_str("<!--");
            out.push_str(contents);
            out.push_str("-->");
        }
        NodeData::ProcessingInstruction { contents, .. } => {
            out.push_str("<?");
            out.push_str(contents);
            out.push('>');
        }
        NodeData::Doctype { name, .. } => {
            out.push_str("<!DOCTYPE ");
            out.push_str(name);
            out.push('>');
        }
        NodeData::Document => {
            for child in node.children.borrow().iter() {
                output_node(out, child);
            }
        }
    }
}
