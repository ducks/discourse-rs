//! Port of lib/html_to_markdown.rb, which Email::Receiver turns an
//! incoming email's HTML into a post with.
//!
//! Nokogiri parses with an HTML5 parser (html5ever here) but classifies
//! elements with libxml2's HTML 4 table: `description.inline?` and
//! `block?` below are that table, read off the reference, and the HTML5
//! elements it does not know have no description (neither inline nor, but
//! for the ones HtmlToMarkdown lists, block).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;

use html5ever::serialize::{SerializeOpts, TraversalScope, serialize};
use html5ever::{namespace_url, ns};
use markup5ever_rcdom::{Handle, Node, NodeData, SerializableHandle};

use crate::pretty_text::cleanup::{attr, element_name, parse_document, set_attr};

/// Elements libxml2 describes as inline.
const INLINE: &[&str] = &[
    "a", "abbr", "acronym", "applet", "b", "basefont", "bdo", "big", "br", "button", "cite",
    "code", "del", "dfn", "em", "embed", "font", "i", "iframe", "img", "input", "ins", "kbd",
    "label", "map", "object", "q", "s", "samp", "script", "select", "small", "span", "strike",
    "strong", "sub", "sup", "textarea", "tt", "u", "var",
];

/// Elements libxml2 describes as block (described, not inline).
const BLOCK: &[&str] = &[
    "address",
    "area",
    "base",
    "blockquote",
    "body",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "hr",
    "html",
    "isindex",
    "legend",
    "li",
    "link",
    "menu",
    "meta",
    "noframes",
    "noscript",
    "ol",
    "optgroup",
    "option",
    "p",
    "param",
    "pre",
    "style",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "ul",
];

/// `HTML5_BLOCK_ELEMENTS`
const HTML5_BLOCK: &[&str] = &[
    "article",
    "aside",
    "details",
    "dialog",
    "figcaption",
    "figure",
    "footer",
    "header",
    "main",
    "nav",
    "section",
];

/// The tags HtmlToMarkdown has a visitor for (`remove_not_allowed!`).
const VISITED: &[&str] = &[
    "a",
    "img",
    "kbd",
    "del",
    "ins",
    "small",
    "big",
    "sub",
    "sup",
    "dl",
    "dd",
    "dt",
    "mark",
    "blockquote",
    "div",
    "tr",
    "p",
    "aside",
    "font",
    "span",
    "thead",
    "tbody",
    "tfoot",
    "u",
    "center",
    "tt",
    "code",
    "pre",
    "br",
    "hr",
    "abbr",
    "acronym",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "table",
    "th",
    "td",
    "ul",
    "ol",
    "li",
    "i",
    "em",
    "b",
    "strong",
    "s",
    "strike",
];

/// `ALLOWED`: kept as HTML.
const KEPT_AS_HTML: &[&str] = &[
    "kbd", "del", "ins", "small", "big", "sub", "sup", "dl", "dd", "dt", "mark",
];

const ALLOWED_IMG_SRCS: [&str; 3] = ["http://", "https://", "www."];

/// `HtmlToMarkdown.new(html, opts)`
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub keep_img_tags: bool,
    pub keep_cid_imgs: bool,
}

// ---- the tree ----

fn parent(node: &Handle) -> Option<Handle> {
    let weak = node.parent.take();
    let p = weak.as_ref().and_then(|w| w.upgrade());
    node.parent.set(weak);
    p
}

fn children(node: &Handle) -> Vec<Handle> {
    node.children.borrow().clone()
}

fn is_element(node: &Handle) -> bool {
    matches!(node.data, NodeData::Element { .. })
}

fn is_text(node: &Handle) -> bool {
    matches!(node.data, NodeData::Text { .. })
}

/// Nokogiri's `node.name`: "text" for text, the tag for elements.
fn name(node: &Handle) -> &str {
    match &node.data {
        NodeData::Text { .. } => "text",
        NodeData::Comment { .. } => "comment",
        NodeData::Element { .. } => element_name(node).unwrap_or(""),
        _ => "",
    }
}

fn detach(node: &Handle) {
    if let Some(p) = parent(node) {
        p.children.borrow_mut().retain(|c| !Rc::ptr_eq(c, node));
    }
    node.parent.set(None);
}

fn append(parent_node: &Handle, child: &Handle) {
    detach(child);
    child.parent.set(Some(Rc::downgrade(parent_node)));
    parent_node.children.borrow_mut().push(child.clone());
}

/// Puts `new` next to `reference` (before or after it).
fn insert_beside(reference: &Handle, new: &Handle, after: bool) {
    let Some(p) = parent(reference) else {
        return;
    };
    detach(new);
    new.parent.set(Some(Rc::downgrade(&p)));
    let mut kids = p.children.borrow_mut();
    let i = kids
        .iter()
        .position(|c| Rc::ptr_eq(c, reference))
        .expect("a child of its parent");
    kids.insert(if after { i + 1 } else { i }, new.clone());
}

fn new_element(tag: &str) -> Handle {
    Node::new(NodeData::Element {
        name: html5ever::QualName::new(None, ns!(html), tag.into()),
        attrs: RefCell::new(Vec::new()),
        template_contents: RefCell::new(None),
        mathml_annotation_xml_integration_point: false,
    })
}

fn text_of(node: &Handle) -> String {
    crate::pretty_text::cleanup::text(node)
}

fn set_text(node: &Handle, value: &str) {
    if let NodeData::Text { contents } = &node.data {
        *contents.borrow_mut() = value.into();
    }
}

fn previous_element(node: &Handle) -> Option<Handle> {
    let p = parent(node)?;
    let kids = p.children.borrow();
    let i = kids.iter().position(|c| Rc::ptr_eq(c, node))?;
    kids[..i].iter().rev().find(|c| is_element(c)).cloned()
}

fn next_element(node: &Handle) -> Option<Handle> {
    let p = parent(node)?;
    let kids = p.children.borrow();
    let i = kids.iter().position(|c| Rc::ptr_eq(c, node))?;
    kids[i + 1..].iter().find(|c| is_element(c)).cloned()
}

fn ancestors(node: &Handle) -> Vec<Handle> {
    let mut out = Vec::new();
    let mut current = parent(node);
    while let Some(p) = current {
        current = parent(&p);
        out.push(p);
    }
    out
}

/// `css(selector)` for a list of tags: descendants in document order.
fn descendants(node: &Handle, tags: &[&str], out: &mut Vec<Handle>) {
    for child in node.children.borrow().iter() {
        if tags.contains(&name(child)) {
            out.push(child.clone());
        }
        descendants(child, tags, out);
    }
}

fn find_all(node: &Handle, tags: &[&str]) -> Vec<Handle> {
    let mut out = Vec::new();
    descendants(node, tags, &mut out);
    out
}

/// `to_html` of one node.
pub(crate) fn outer_html(node: &Handle) -> String {
    let mut out = Vec::new();
    let handle: SerializableHandle = node.clone().into();
    serialize(
        &mut out,
        &handle,
        SerializeOpts {
            traversal_scope: TraversalScope::IncludeNode,
            ..Default::default()
        },
    )
    .expect("writing to a vector cannot fail");
    String::from_utf8(out).expect("the serializer writes UTF-8")
}

fn inner_html(node: &Handle) -> String {
    children(node).iter().map(outer_html).collect()
}

/// `description&.inline?`
fn described_inline(node: &Handle) -> bool {
    is_element(node) && INLINE.contains(&name(node))
}

/// `block?(node)`
fn block(node: Option<&Handle>) -> bool {
    node.is_some_and(|n| {
        is_element(n) && (BLOCK.contains(&name(n)) || HTML5_BLOCK.contains(&name(n)))
    })
}

fn is_inline(node: &Handle) -> bool {
    is_text(node)
        || (name(node) != "br" && described_inline(node) && children(node).iter().all(is_inline))
}

/// Ruby's `[[:space:]]`: Unicode White_Space.
fn space(c: char) -> bool {
    c.is_whitespace()
}

/// Ruby's `strip`/`lstrip` set: ASCII whitespace and NUL.
fn ruby_ws(c: char) -> bool {
    matches!(c, '\0' | '\t' | '\n' | '\x0B' | '\x0C' | '\r' | ' ')
}

/// `blank?`
fn blank(s: &str) -> bool {
    s.chars().all(space)
}

// ---- the passes ----

fn remove_not_allowed(node: &Handle) {
    for child in children(node) {
        remove_not_allowed(&child);
    }
    let n = name(node);
    if n != "text" && !VISITED.contains(&n) {
        detach(node);
    }
}

fn remove_hidden(body: &Handle) {
    fn walk(node: &Handle, out: &mut Vec<Handle>) {
        for child in node.children.borrow().iter() {
            if attr(child, "hidden").is_some() {
                out.push(child.clone());
            }
            walk(child, out);
        }
    }
    let mut hidden = Vec::new();
    walk(body, &mut hidden);
    for n in hidden {
        detach(&n);
    }
    for dimension in ["width", "height"] {
        for img in find_all(body, &["img"]) {
            if let Some(v) = attr(&img, dimension)
                && crate::ruby::to_i(&v) <= 0
            {
                detach(&img);
            }
        }
    }
}

fn nest_sibling_lists(body: &Handle) {
    let lists: Vec<Handle> = find_all(body, &["ul", "ol"])
        .into_iter()
        .filter(|l| parent(l).is_some_and(|p| matches!(name(&p), "ul" | "ol")))
        .collect();
    for list in lists {
        if let Some(previous) = previous_element(&list)
            && name(&previous) == "li"
        {
            append(&previous, &list);
        }
    }
}

/// `hoist_line_breaks!`: a `<br>` inside an inline element splits it.
fn hoist_line_breaks(body: &Handle) {
    let mut marked: Vec<Handle> = find_all(body, &["br"]);
    loop {
        let mut changed = false;
        // css("br.klass"): the marked breaks still in the tree, in order.
        let pass: Vec<Handle> = {
            let mut ordered = find_all(body, &["br"]);
            ordered.retain(|b| marked.iter().any(|m| Rc::ptr_eq(m, b)));
            ordered
        };
        for br in pass {
            let Some(p) = parent(&br) else {
                continue;
            };
            if block(Some(&p)) {
                marked.retain(|m| !Rc::ptr_eq(m, &br));
                continue;
            }
            let kids = children(&p);
            let at = kids
                .iter()
                .position(|c| Rc::ptr_eq(c, &br))
                .expect("its parent's child");
            let before = &kids[..=at];
            let after = &kids[at + 1..];
            if before.len() > 1 {
                let b = new_element(name(&p));
                for c in &before[..before.len() - 1] {
                    append(&b, c);
                }
                if !blank(&inner_html(&b)) {
                    insert_beside(&p, &b, false);
                }
            }
            if !after.is_empty() {
                let a = new_element(name(&p));
                for c in after {
                    append(&a, c);
                }
                if !blank(&inner_html(&a)) {
                    insert_beside(&p, &a, true);
                }
            }
            // parent.replace(br)
            insert_beside(&p, &br, false);
            detach(&p);
            changed = true;
        }
        if !changed {
            break;
        }
    }
}

fn remove_whitespaces(node: &Handle) {
    if name(node) == "pre" {
        return;
    }
    let kids = children(node);
    let mut i = 0;
    while i < kids.len() {
        let inline = is_inline(&kids[i]);
        let mut j = i;
        while j < kids.len() && is_inline(&kids[j]) == inline {
            j += 1;
        }
        let run = &kids[i..j];
        if inline {
            if collapse_spaces(run, true) {
                remove_trailing_space(run);
            }
        } else {
            for n in run {
                remove_whitespaces(n);
            }
        }
        i = j;
    }
}

fn collapse_spaces(nodes: &[Handle], mut was_space: bool) -> bool {
    for node in nodes {
        if is_text(node) {
            let mut out = String::new();
            for c in text_of(node).chars() {
                if space(c) {
                    if !was_space {
                        out.push(' ');
                    }
                    was_space = true;
                } else {
                    out.push(c);
                    was_space = false;
                }
            }
            set_text(node, &out);
        } else {
            for child in children(node) {
                was_space = collapse_spaces(&[child], was_space);
            }
        }
    }
    was_space
}

fn remove_trailing_space(nodes: &[Handle]) {
    let Some(last) = nodes.last() else {
        return;
    };
    if is_text(last) {
        let t = text_of(last);
        if let Some(stripped) = t.strip_suffix(' ') {
            set_text(last, stripped);
        }
    } else {
        let kids = children(last);
        if !kids.is_empty() {
            remove_trailing_space(&kids);
        }
    }
}

// ---- the visitors ----

struct Converter {
    opts: Options,
    allowed_hrefs: Vec<String>,
    within_html_block: bool,
}

static LINE_START_NOT_BLANK: LazyLock<fancy_regex::Regex> =
    LazyLock::new(|| fancy_regex::Regex::new(r"(?m)^(?![ \t\r\n\f\x0B]*$)").unwrap());
static MANY_NEWLINES: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\n{2,}").unwrap());

/// `escape` as Nokogiri serializes a text node.
fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('\u{a0}', "&nbsp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('\u{a0}', "&nbsp;")
        .replace('"', "&quot;")
}

/// `strip_newlines`: newlines to spaces, runs of spaces squeezed.
fn strip_newlines(s: &str) -> String {
    let replaced = s.replace('\n', " ");
    let mut out = String::new();
    for c in replaced.chars() {
        if c == ' ' && out.ends_with(' ') {
            continue;
        }
        out.push(c);
    }
    out
}

impl Converter {
    fn traverse(&mut self, node: &Handle, within_html_block: bool) -> String {
        let changed = within_html_block;
        if within_html_block {
            self.within_html_block = true;
        }
        let text: String = children(node).iter().map(|n| self.visit(n)).collect();
        if changed {
            self.within_html_block = false;
        }
        text
    }

    fn visit(&mut self, node: &Handle) -> String {
        let tag = name(node).to_string();
        match tag.as_str() {
            "text" => {
                if self.within_html_block {
                    escape_text(&text_of(node))
                } else {
                    text_of(node)
                }
            }
            "a" => {
                let href = attr(node, "href").filter(|h| !blank(h));
                match href {
                    Some(h) if self.allowed_hrefs.iter().any(|a| h.starts_with(a.as_str())) => {
                        format!("[{}]({h})", self.traverse(node, false))
                    }
                    _ => self.traverse(node, false),
                }
            }
            "img" => self.visit_img(node),
            t if KEPT_AS_HTML.contains(&t) => {
                format!("<{t}>{}</{t}>", self.traverse(node, true))
            }
            "blockquote" => {
                let text = self.traverse(node, false);
                let text = text.trim_matches(ruby_ws);
                let text = MANY_NEWLINES.replace_all(text, "\n\n");
                let text = regex::Regex::new("(?m)^").unwrap().replace_all(&text, "> ");
                format!("\n\n{text}\n\n")
            }
            "div" => {
                let prefix = if block(previous_element(node).as_ref()) {
                    ""
                } else {
                    "\n"
                };
                format!("{prefix}{}\n", self.traverse(node, false))
            }
            "p" => format!("\n\n{}\n\n", self.traverse(node, false)),
            "aside" | "font" | "span" | "thead" | "tbody" | "tfoot" | "u" | "center" => {
                self.traverse(node, false)
            }
            "tt" => format!("`{}`", self.traverse(node, false)),
            "code" => {
                if ancestors(node).iter().any(|a| name(a) == "pre") {
                    self.traverse(node, false)
                } else {
                    format!("`{}`", self.traverse(node, false))
                }
            }
            "pre" => {
                let text = self.traverse(node, false);
                let fence = if text.contains('`') { "~~~" } else { "```" };
                let lang = find_all(node, &["code"])
                    .first()
                    .and_then(|c| attr(c, "class"))
                    .and_then(|class| {
                        regex::Regex::new(r"lang-([A-Za-z0-9_]+)")
                            .unwrap()
                            .captures(&class)
                            .map(|c| c[1].to_string())
                    })
                    .unwrap_or_default();
                let again = self.traverse(node, false);
                format!("\n\n{fence}{lang}\n{again}\n{fence}\n\n")
            }
            "br" => "\n".into(),
            "hr" => "\n\n---\n\n".into(),
            "abbr" | "acronym" => {
                let title = attr(node, "title").filter(|t| !blank(t));
                let inner = self.traverse(node, true);
                match title {
                    Some(t) => format!("<abbr title=\"{}\">{inner}</abbr>", escape_attr(&t)),
                    None => format!("<abbr>{inner}</abbr>"),
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let n: usize = tag[1..].parse().expect("a heading level");
                format!("{} {}", "#".repeat(n), self.traverse(node, false))
            }
            "table" => self.visit_table(node),
            "tr" => {
                let text = self.traverse(node, false);
                if self.within_html_block {
                    format!("<tr>\n{text}</tr>\n")
                } else {
                    text
                }
            }
            "th" | "td" => {
                let text = self.traverse(node, false);
                if self.within_html_block {
                    let mut attrs = String::new();
                    if let NodeData::Element { attrs: a, .. } = &node.data {
                        for at in a.borrow().iter() {
                            if matches!(&*at.name.local, "rowspan" | "colspan") {
                                attrs.push_str(&format!(
                                    " {}=\"{}\"",
                                    at.name.local,
                                    escape_attr(&at.value)
                                ));
                            }
                        }
                    }
                    format!("<{tag}{attrs}>\n\n{text}\n\n</{tag}>\n")
                } else {
                    text
                }
            }
            "ul" | "ol" => {
                let prefix = if block(previous_element(node).as_ref()) {
                    ""
                } else {
                    "\n"
                };
                let nested = ancestors(node)
                    .iter()
                    .any(|a| matches!(name(a), "ul" | "ol" | "li"));
                let suffix = if nested && next_element(node).is_none() {
                    ""
                } else {
                    "\n"
                };
                format!("{prefix}{}{suffix}", self.traverse(node, false))
            }
            "li" => self.visit_li(node),
            "i" | "em" => self.emphasis(
                node,
                &tag,
                |t| if t.contains('*') { "_" } else { "*" },
                false,
            ),
            "b" | "strong" => self.emphasis(
                node,
                &tag,
                |t| if t.contains('*') { "__" } else { "**" },
                false,
            ),
            "s" | "strike" => self.emphasis(node, &tag, |_| "~~", true),
            _ => String::new(),
        }
    }

    fn visit_img(&mut self, node: &Handle) -> String {
        let Some(src) = attr(node, "src").filter(|s| !blank(s)) else {
            return String::new();
        };
        if let Some(alt) = attr(node, "alt").filter(|a| !blank(a)) {
            set_attr(node, "alt", &strip_newlines(&alt));
        }
        if let Some(title) = attr(node, "title").filter(|t| !blank(t)) {
            set_attr(node, "title", &strip_newlines(&title));
        }
        if self.opts.keep_img_tags || (self.opts.keep_cid_imgs && src.starts_with("cid:")) {
            return outer_html(node);
        }
        if ALLOWED_IMG_SRCS.iter().any(|a| src.starts_with(a)) {
            let width = attr(node, "width").map_or(0, |v| crate::ruby::to_i(&v));
            let height = attr(node, "height").map_or(0, |v| crate::ruby::to_i(&v));
            let dimensions = if width > 0 && height > 0 {
                format!("|{width}x{height}")
            } else {
                String::new()
            };
            let alt = attr(node, "alt")
                .or_else(|| attr(node, "title"))
                .unwrap_or_default();
            return format!("![{alt}{dimensions}]({src})");
        }
        String::new()
    }

    /// `extract_rows`
    fn extract_rows(&self, table: &Handle) -> Option<Vec<Handle>> {
        if ancestors(table).iter().any(|a| name(a) == "table") {
            return None;
        }
        let rows = find_all(table, &["tr"]);
        let first = rows.first()?;
        let headers = find_all(first, &["td", "th"]).len();
        if rows[1..]
            .iter()
            .any(|r| find_all(r, &["td"]).len() != headers)
        {
            return None;
        }
        Some(rows)
    }

    fn visit_table(&mut self, node: &Handle) -> String {
        match self.extract_rows(node) {
            Some(rows) => {
                let headers = find_all(&rows[0], &["td", "th"]);
                let cell = |c: &mut Self, td: &Handle| c.traverse(td, false).replace('\n', "<br>");
                let mut text = format!(
                    "| {} |\n",
                    headers
                        .iter()
                        .map(|td| cell(self, td))
                        .collect::<Vec<_>>()
                        .join(" | ")
                );
                text.push_str(&format!("| {} |\n", vec!["-"; headers.len()].join(" | ")));
                for row in &rows[1..] {
                    let cells: Vec<String> = find_all(row, &["td"])
                        .iter()
                        .map(|td| cell(self, td))
                        .collect();
                    text.push_str(&format!("| {} |\n", cells.join(" | ")));
                }
                format!("\n\n{text}\n\n")
            }
            None => format!("<table>\n{}</table>", self.traverse(node, true)),
        }
    }

    fn visit_li(&mut self, node: &Handle) -> String {
        let text = self.traverse(node, false);
        let lists: Vec<Handle> = ancestors(node)
            .into_iter()
            .filter(|a| matches!(name(a), "ul" | "ol"))
            .collect();
        let marker = if lists.first().is_some_and(|l| name(l) == "ol") {
            "1. "
        } else {
            "- "
        };
        let indent = " ".repeat(marker.len()).repeat(lists.len().max(1));
        let last = parent(node)
            .and_then(|p| children(&p).into_iter().filter(is_element).last())
            .is_some_and(|l| Rc::ptr_eq(&l, node));
        let suffix = if last { "" } else { "\n" };
        let text = MANY_NEWLINES.replace_all(&text, "\n\n");
        let text = LINE_START_NOT_BLANK.replace_all(&text, indent.as_str());
        let text = text.trim_start_matches(ruby_ws);
        format!("{marker}{text}{suffix}")
    }

    /// The emphasis, strong and strike visitors.
    fn emphasis(
        &mut self,
        node: &Handle,
        tag: &str,
        wrap: impl Fn(&str) -> &'static str,
        strike: bool,
    ) -> String {
        let text = self.traverse(node, false);
        if text.is_empty() {
            return String::new();
        }
        if blank(&text) {
            return " ".into();
        }
        let as_html = if strike {
            text.contains('\n') || text.contains("~~")
        } else {
            text.contains('\n') || (text.contains('*') && text.contains('_'))
        };
        if as_html {
            return format!("<{tag}>{text}</{tag}>");
        }
        let prefix = if text.starts_with(' ') { " " } else { "" };
        let suffix = if text.chars().count() > 1 && text.ends_with(' ') {
            " "
        } else {
            ""
        };
        let w = wrap(&text);
        format!("{prefix}{w}{}{w}{suffix}", text.trim_matches(ruby_ws))
    }
}

/// `HtmlToMarkdown.new(html, opts).to_markdown`
pub fn to_markdown(html: &str, opts: &Options, allowed_href_schemes: &str) -> String {
    let dom = parse_document(html);
    let body = find_all(&dom.document, &["body"]).into_iter().next();
    let Some(body) = body else {
        return String::new();
    };
    remove_not_allowed(&body);
    remove_hidden(&body);
    nest_sibling_lists(&body);
    hoist_line_breaks(&body);
    remove_whitespaces(&body);

    let mut allowed_hrefs: Vec<String> = allowed_href_schemes
        .split('|')
        .filter(|s| !s.is_empty())
        .map(|s| format!("{s}:"))
        .collect();
    allowed_hrefs.extend(ALLOWED_IMG_SRCS.iter().map(|s| s.to_string()));
    allowed_hrefs.push("mailto:".into());
    let mut converter = Converter {
        opts: opts.clone(),
        allowed_hrefs,
        within_html_block: false,
    };
    let text = converter.traverse(&body, false);
    MANY_NEWLINES
        .replace_all(&text, "\n\n")
        .trim_matches(ruby_ws)
        .to_string()
}

// ---- Email::Receiver's HTML extracters ----

/// `HTML_EXTRACTERS`: which mail client wrote the HTML, by a mark it
/// leaves (case-sensitive, as Ruby's regexes here are).
const EXTRACTERS: [(&str, &str); 10] = [
    ("gmail", r#"class="gmail_(signature|extra)"#),
    ("outlook", r#"id="(divRplyFwdMsg|Signature)""#),
    ("word", r#"class="WordSection1""#),
    ("exchange", r#"name="message(Body|Reply)Section""#),
    ("apple_mail", r#"id="AppleMailSignature""#),
    ("mozilla", r#"class="moz-"#),
    ("protonmail", r#"class="protonmail_"#),
    ("zimbra", r#"data-marker="__"#),
    ("newton", r#"(id|class)="cm_"#),
    ("front", r#"class="front-"#),
];

/// Every element under `root`, in document order.
fn all_elements(root: &Handle) -> Vec<Handle> {
    fn walk(node: &Handle, out: &mut Vec<Handle>) {
        for child in node.children.borrow().iter() {
            if is_element(child) {
                out.push(child.clone());
            }
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

/// `following-sibling::*`
fn following_siblings(node: &Handle) -> Vec<Handle> {
    let Some(p) = parent(node) else {
        return Vec::new();
    };
    let kids = children(&p);
    let i = kids
        .iter()
        .position(|c| Rc::ptr_eq(c, node))
        .expect("its parent's child");
    kids[i + 1..]
        .iter()
        .filter(|c| is_element(c))
        .cloned()
        .collect()
}

/// `contains(concat(' ', normalize-space(@class), ' '), ' name ')`
fn has_class_token(node: &Handle, class: &str) -> bool {
    attr(node, "class").is_some_and(|c| c.split_ascii_whitespace().any(|t| t == class))
}

fn starts_with_attr(node: &Handle, name: &str, prefix: &str) -> bool {
    attr(node, name).is_some_and(|v| v.starts_with(prefix))
}

/// An XPath union: the nodes in document order, each once.
fn union(root: &Handle, picked: Vec<Handle>) -> Vec<Handle> {
    all_elements(root)
        .into_iter()
        .filter(|n| picked.iter().any(|p| Rc::ptr_eq(p, n)))
        .collect()
}

/// The matches and their following siblings.
fn with_following(root: &Handle, pred: impl Fn(&Handle) -> bool) -> Vec<Handle> {
    let mut picked = Vec::new();
    for n in all_elements(root).into_iter().filter(|n| pred(n)) {
        picked.extend(following_siblings(&n));
        picked.push(n);
    }
    picked
}

/// `NodeSet#remove` then `NodeSet#to_html`: each unlinked in order, then
/// serialized.
fn remove_all(root: &Handle, picked: Vec<Handle>) -> String {
    let nodes = union(root, picked);
    for n in &nodes {
        detach(n);
    }
    nodes.iter().map(outer_html).collect()
}

/// `Email::Receiver#select_body`'s HTML side when a known client wrote it:
/// `extract_from_<client>`, then `to_markdown(html, elided_html)` (the
/// markdown trimmed, the elided converted). None when no client matches.
pub fn extract(
    html: &str,
    opts: &Options,
    schemes: &str,
) -> Result<Option<(Option<String>, String)>, crate::Unsupported> {
    let found = EXTRACTERS
        .iter()
        .filter_map(|(name, re)| {
            regex::Regex::new(re)
                .expect("an extracter regex")
                .find(html)
                .map(|m| (m.start(), *name))
        })
        .min_by_key(|(start, _)| *start);
    let Some(found) = found else {
        return Ok(None);
    };
    let dom = crate::pretty_text::cleanup::parse(html);
    let root = crate::pretty_text::cleanup::fragment_root(&dom);
    let doc_html = |dom: &markup5ever_rcdom::RcDom| crate::pretty_text::cleanup::to_html(dom);
    let (kept, elided) = match found.1 {
        "gmail" => {
            let picked = all_elements(&root)
                .into_iter()
                .filter(|n| {
                    has_class_token(n, "gmail_signature") || has_class_token(n, "gmail_extra")
                })
                .collect();
            let elided = remove_all(&root, picked);
            (doc_html(&dom), elided)
        }
        "outlook" => {
            let id = |n: &Handle, v: &str| attr(n, "id").as_deref() == Some(v);
            let mut picked = with_following(&root, |n| id(n, "Signature"));
            picked.extend(all_elements(&root).into_iter().filter(|n| name(n) == "hr"));
            picked.extend(with_following(&root, |n| id(n, "divRplyFwdMsg")));
            let elided = remove_all(&root, picked);
            (doc_html(&dom), elided)
        }
        "word" => {
            let mut picked = Vec::new();
            for section in all_elements(&root)
                .into_iter()
                .filter(|n| has_class_token(n, "WordSection1"))
            {
                let first = children(&section)
                    .into_iter()
                    .filter(is_element)
                    .find(|c| !matches!(name(c), "p" | "ul" | "ol"));
                if let Some(first) = first {
                    picked.extend(following_siblings(&first));
                    picked.push(first);
                }
            }
            let elided = remove_all(&root, picked);
            // doc.at(".WordSection1").to_html raises when it is gone.
            let section = all_elements(&root)
                .into_iter()
                .find(|n| has_class_token(n, "WordSection1"))
                .ok_or(crate::Unsupported(
                    "a Word email without its WordSection1 (NoMethodError)",
                ))?;
            (outer_html(&section), elided)
        }
        "exchange" => {
            let named = |v: &str| -> Vec<Handle> {
                all_elements(&root)
                    .into_iter()
                    .filter(|n| name(n) == "div" && attr(n, "name").as_deref() == Some(v))
                    .collect()
            };
            let reply = named("messageReplySection");
            let body = named("messageBodySection");
            let concat = |nodes: &[Handle]| nodes.iter().map(outer_html).collect::<String>();
            if !reply.is_empty() && !body.is_empty() {
                let elided = remove_all(&root, reply);
                (concat(&named("messageBodySection")), elided)
            } else if !reply.is_empty() {
                (concat(&reply), String::new())
            } else if !body.is_empty() {
                (concat(&body), String::new())
            } else {
                (doc_html(&dom), String::new())
            }
        }
        "apple_mail" => {
            // [@id='AppleMailSignature'][position()=last()]: the last such
            // child of its parent. DocumentFragment#css also tries each
            // top-level node with `self::`, where every one is last.
            let mut picked = Vec::new();
            for n in all_elements(&root) {
                if attr(&n, "id").as_deref() != Some("AppleMailSignature") {
                    continue;
                }
                let top_level = parent(&n).is_some_and(|p| Rc::ptr_eq(&p, &root));
                let later = following_siblings(&n)
                    .iter()
                    .any(|s| attr(s, "id").as_deref() == Some("AppleMailSignature"));
                if top_level || !later {
                    picked.extend(following_siblings(&n));
                }
            }
            let elided = remove_all(&root, picked);
            (doc_html(&dom), elided)
        }
        "mozilla" => {
            let picked = with_following(&root, |n| {
                ["moz-cite", "moz-signature", "moz-forward"]
                    .iter()
                    .any(|p| starts_with_attr(n, "class", p))
            });
            let elided = remove_all(&root, picked);
            (doc_html(&dom), elided)
        }
        "protonmail" => {
            let picked = with_following(&root, |n| starts_with_attr(n, "class", "protonmail_"));
            let elided = remove_all(&root, picked);
            (doc_html(&dom), elided)
        }
        "zimbra" => {
            let picked = all_elements(&root)
                .into_iter()
                .filter(|n| attr(n, "data-marker").is_some())
                .collect();
            let elided = remove_all(&root, picked);
            (doc_html(&dom), elided)
        }
        "newton" => {
            let picked = all_elements(&root)
                .into_iter()
                .filter(|n| starts_with_attr(n, "id", "cm_") || starts_with_attr(n, "class", "cm_"))
                .collect();
            let elided = remove_all(&root, picked);
            (doc_html(&dom), elided)
        }
        "front" => {
            let picked = all_elements(&root)
                .into_iter()
                .filter(|n| starts_with_attr(n, "class", "front-"))
                .collect();
            let elided = remove_all(&root, picked);
            (doc_html(&dom), elided)
        }
        _ => unreachable!("a listed extracter"),
    };
    let markdown = to_markdown(&kept, opts, schemes);
    let trimmed = super::reply_trimmer::trim(&markdown).map(|(t, _)| t);
    Ok(Some((trimmed, to_markdown(&elided, opts, schemes))))
}
