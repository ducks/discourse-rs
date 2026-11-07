//! The rich editor's node tree (docs/rich-editor.md, "The node tree"): the
//! parse as markdown-it's token stream nests it, in JSON. The editor
//! flattens it back into tokens (static/js/rich-editor/parser.js) and runs
//! the extensions' parse specs over them, so each node is written as the
//! token JS pushes for it: its type less `_open` / `_close`, tag, attrs,
//! markup, info and content.
//!
//! The crate's tree is not that stream as it stands, and three things are
//! rebuilt here: a tight list item's paragraph, which the list rule splices
//! away (a `paragraph` with `hidden`); each block's `inline` token, which
//! the inline parser replaces with its children (the inline blocks
//! `blocks.rs` keeps); and the token an Element stands for, which its rule
//! records (`JsToken`). A node no token is written for yet is `unported:`
//! and its name, which no parse spec handles, so the editor refuses the
//! post as unsupported.

use markdown_it::Node;
use markdown_it::parser::core::Root;
use markdown_it::parser::inline::{Text, TextSpecial};
use markdown_it::plugins::cmark::block::blockquote::Blockquote;
use markdown_it::plugins::cmark::block::code::CodeBlock;
use markdown_it::plugins::cmark::block::fence::CodeFence;
use markdown_it::plugins::cmark::block::heading::ATXHeading;
use markdown_it::plugins::cmark::block::hr::ThematicBreak;
use markdown_it::plugins::cmark::block::lheading::SetextHeader;
use markdown_it::plugins::cmark::block::list::{BulletList, ListItem, OrderedList};
use markdown_it::plugins::cmark::block::paragraph::Paragraph;
use markdown_it::plugins::cmark::inline::autolink::Autolink;
use markdown_it::plugins::cmark::inline::backticks::CodeInline;
use markdown_it::plugins::cmark::inline::emphasis::{Em, Strong};
use markdown_it::plugins::cmark::inline::image::Image;
use markdown_it::plugins::cmark::inline::link::Link;
use markdown_it::plugins::cmark::inline::newline::{Hardbreak, Softbreak};
use markdown_it::plugins::extra::strikethrough::Strikethrough;
use markdown_it::plugins::extra::tables::{
    ColumnAlignment, Table, TableBody, TableCell, TableHead, TableRow,
};
use markdown_it::plugins::html::html_block::HtmlBlock;
use markdown_it::plugins::html::html_inline::HtmlInline;
use serde_json::{Map, Value, json};

use super::blocks;
use super::element::{BlockText, CodeText, Element, Holder, JsToken, RawHtml};
use super::emoji::Emoji;
use super::linkify::Linkified;

/// One token's fields; empty ones are left out of the JSON.
#[derive(Default)]
struct Token {
    kind: String,
    tag: &'static str,
    owned_tag: Option<String>,
    attrs: Vec<(String, String)>,
    markup: String,
    info: String,
    content: String,
    hidden: bool,
    /// A single token whose `children` are its own (an image's alt).
    leaf_with_children: bool,
}

impl Token {
    fn new(kind: &str, tag: &'static str) -> Self {
        Token {
            kind: kind.to_string(),
            tag,
            ..Default::default()
        }
    }

    fn markup(mut self, markup: impl Into<String>) -> Self {
        self.markup = markup.into();
        self
    }

    fn attr(mut self, name: &str, value: impl Into<String>) -> Self {
        self.attrs.push((name.to_string(), value.into()));
        self
    }

    fn json(self, children: Option<Vec<Value>>) -> Value {
        let mut node = Map::new();
        node.insert("type".into(), Value::String(self.kind));
        let tag = self.owned_tag.unwrap_or_else(|| self.tag.to_string());
        if !tag.is_empty() {
            node.insert("tag".into(), Value::String(tag));
        }
        if !self.attrs.is_empty() {
            let attrs = self
                .attrs
                .into_iter()
                .map(|(name, value)| json!([name, value]))
                .collect();
            node.insert("attrs".into(), Value::Array(attrs));
        }
        for (key, value) in [
            ("markup", self.markup),
            ("info", self.info),
            ("content", self.content),
        ] {
            if !value.is_empty() {
                node.insert(key.into(), Value::String(value));
            }
        }
        if self.hidden {
            node.insert("hidden".into(), Value::Bool(true));
        }
        if let Some(children) = children {
            if self.leaf_with_children {
                node.insert("nesting".into(), json!(0));
            }
            node.insert("children".into(), Value::Array(children));
        }
        Value::Object(node)
    }
}

/// The tree's root: `{"type": "doc", "children": [...]}`.
pub fn doc(root: &Node) -> Value {
    let source = root
        .cast::<Root>()
        .map(|r| r.content.as_str())
        .unwrap_or("");
    let mut children = Vec::new();
    let walker = Walker { source };
    walker.block_children(root, &mut children);
    json!({ "type": "doc", "children": children })
}

struct Walker<'a> {
    source: &'a str,
}

/// Blocks whose inline content is their only child in JS (an `inline`
/// token), rather than a paragraph of it.
fn holds_inline(node: &Node) -> bool {
    node.is::<Paragraph>()
        || node.is::<ATXHeading>()
        || node.is::<SetextHeader>()
        || node.is::<TableCell>()
}

impl Walker<'_> {
    /// A block's children: its blocks, and its inline content grouped back
    /// into the `inline` tokens it came from.
    fn block_children(&self, node: &Node, out: &mut Vec<Value>) {
        if holds_inline(node) {
            let mut content = blocks::inline_blocks(node)
                .map(|(_, sources)| sources.concat())
                .unwrap_or_default();
            // heading.mjs trims what follows the #s.
            if node.is::<ATXHeading>() {
                content = content.trim().to_string();
            }
            out.push(self.inline(content, &node.children.iter().collect::<Vec<_>>()));
            return;
        }
        let Some((owners, sources)) = blocks::inline_blocks(node) else {
            for child in &node.children {
                self.block(child, out);
            }
            return;
        };
        let mut i = 0;
        while i < node.children.len() {
            let Some(owner) = owners[i] else {
                self.block(&node.children[i], out);
                i += 1;
                continue;
            };
            let mut run = Vec::new();
            while i < node.children.len() && owners[i] == Some(owner) {
                run.push(&node.children[i]);
                i += 1;
            }
            // A tight list item's text: its paragraph, hidden
            // (markdown-it's markTightParagraphs).
            let mut paragraph = Token::new("paragraph", "p");
            paragraph.hidden = node.is::<ListItem>();
            let inline = self.inline(sources[owner].to_string(), &run);
            out.push(paragraph.json(Some(vec![inline])));
        }
    }

    fn inline(&self, content: String, nodes: &[&Node]) -> Value {
        let mut children = Vec::new();
        for node in nodes {
            self.inline_node(node, &mut children);
        }
        let mut token = Token::new("inline", "");
        token.content = content;
        token.json(Some(children))
    }

    fn inline_children(&self, node: &Node) -> Vec<Value> {
        let mut out = Vec::new();
        for child in &node.children {
            self.inline_node(child, &mut out);
        }
        out
    }

    fn block(&self, node: &Node, out: &mut Vec<Value>) {
        if node.is::<Holder>() {
            for child in &node.children {
                self.block(child, out);
            }
            return;
        }
        let token = if node.is::<Paragraph>() {
            Token::new("paragraph", "p")
        } else if let Some(heading) = node.cast::<ATXHeading>() {
            Token::new("heading", heading_tag(heading.level))
                .markup("#".repeat(heading.level as usize))
        } else if let Some(heading) = node.cast::<SetextHeader>() {
            Token::new("heading", heading_tag(heading.level)).markup(heading.marker.to_string())
        } else if node.is::<Blockquote>() {
            Token::new("blockquote", "blockquote").markup(">")
        } else if let Some(list) = node.cast::<BulletList>() {
            let mut items = Vec::new();
            for item in &node.children {
                items.push(self.list_item(item, list.marker, None));
            }
            out.push(
                Token::new("bullet_list", "ul")
                    .markup(list.marker.to_string())
                    .json(Some(items)),
            );
            return;
        } else if let Some(list) = node.cast::<OrderedList>() {
            let mut items = Vec::new();
            for item in &node.children {
                let number = self.item_number(item);
                items.push(self.list_item(item, list.marker, Some(number)));
            }
            let mut token = Token::new("ordered_list", "ol").markup(list.marker.to_string());
            if list.start != 1 {
                token = token.attr("start", list.start.to_string());
            }
            out.push(token.json(Some(items)));
            return;
        } else if let Some(code) = node.cast::<CodeBlock>() {
            let mut token = Token::new("code_block", "code");
            token.content = code.content.clone();
            out.push(token.json(None));
            return;
        } else if let Some(fence) = node.cast::<CodeFence>() {
            let mut token = Token::new("fence", "code")
                .markup(fence.marker.to_string().repeat(fence.marker_len));
            token.info = fence.info.clone();
            token.content = fence.content.clone();
            out.push(token.json(None));
            return;
        } else if let Some(hr) = node.cast::<ThematicBreak>() {
            out.push(
                Token::new("hr", "hr")
                    .markup(hr.marker.to_string().repeat(hr.marker_len))
                    .json(None),
            );
            return;
        } else if let Some(html) = node.cast::<HtmlBlock>() {
            let mut token = Token::new("html_block", "");
            token.content = html.content.clone();
            out.push(token.json(None));
            return;
        } else if let Some(html) = node.cast::<RawHtml>() {
            let mut token = Token::new("html_raw", "");
            token.content = html.0.clone();
            out.push(token.json(None));
            return;
        } else if let Some(text) = node.cast::<BlockText>() {
            let mut token = Token::new("text", "");
            token.content = text.0.clone();
            out.push(token.json(None));
            return;
        } else if let Some(table) = node.cast::<Table>() {
            out.push(self.table(node, table));
            return;
        } else if let Some(element) = node.cast::<Element>() {
            let token = element_token(node, element);
            let mut children = Vec::new();
            self.block_children(node, &mut children);
            out.push(token.json(Some(children)));
            return;
        } else {
            out.push(unported(node));
            return;
        };
        let mut children = Vec::new();
        self.block_children(node, &mut children);
        out.push(token.json(Some(children)));
    }

    /// `list_item`: the list's marker as markup and, in an ordered list,
    /// the item's number as written in `info`.
    fn list_item(&self, item: &Node, marker: char, number: Option<String>) -> Value {
        if !item.is::<ListItem>() {
            return unported(item);
        }
        let mut token = Token::new("list_item", "li").markup(marker.to_string());
        token.info = number.unwrap_or_default();
        let mut children = Vec::new();
        self.block_children(item, &mut children);
        token.json(Some(children))
    }

    /// The digits an ordered item starts with in the source.
    fn item_number(&self, item: &Node) -> String {
        let Some(map) = item.srcmap else {
            return String::new();
        };
        let (start, _) = map.get_byte_offsets();
        let line = self.source.get(start..).unwrap_or("");
        let line = line.split('\n').next().unwrap_or("");
        let from = line
            .find(|c: char| c.is_ascii_digit())
            .unwrap_or(line.len());
        line[from..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect()
    }

    fn table(&self, node: &Node, table: &Table) -> Value {
        let mut sections = Vec::new();
        for section in &node.children {
            let (kind, tag, head) = if section.is::<TableHead>() {
                ("thead", "thead", true)
            } else if section.is::<TableBody>() {
                ("tbody", "tbody", false)
            } else {
                sections.push(unported(section));
                continue;
            };
            let mut rows = Vec::new();
            for row in &section.children {
                if !row.is::<TableRow>() {
                    rows.push(unported(row));
                    continue;
                }
                let mut cells = Vec::new();
                for (index, cell) in row.children.iter().enumerate() {
                    let (kind, tag) = if head { ("th", "th") } else { ("td", "td") };
                    let mut token = Token::new(kind, tag);
                    let align = match table.alignments.get(index) {
                        Some(ColumnAlignment::Left) => "left",
                        Some(ColumnAlignment::Right) => "right",
                        Some(ColumnAlignment::Center) => "center",
                        _ => "",
                    };
                    if !align.is_empty() {
                        token = token.attr("style", format!("text-align:{align}"));
                    }
                    let mut children = Vec::new();
                    self.block_children(cell, &mut children);
                    cells.push(token.json(Some(children)));
                }
                rows.push(Token::new("tr", "tr").json(Some(cells)));
            }
            sections.push(Token::new(kind, tag).json(Some(rows)));
        }
        Token::new("table", "table").json(Some(sections))
    }

    fn inline_node(&self, node: &Node, out: &mut Vec<Value>) {
        if node.is::<Holder>() {
            for child in &node.children {
                self.inline_node(child, out);
            }
            return;
        }
        let token = if let Some(text) = node.cast::<Text>() {
            text_token(&text.content)
        } else if let Some(text) = node.cast::<TextSpecial>() {
            text_token(&text.content)
        } else if let Some(text) = node.cast::<BlockText>() {
            text_token(&text.0)
        } else if node.is::<Softbreak>() {
            Token::new("softbreak", "br")
        } else if node.is::<Hardbreak>() {
            Token::new("hardbreak", "br")
        } else if let Some(em) = node.cast::<Em>() {
            let token = Token::new("em", "em").markup(em.marker.to_string());
            out.push(token.json(Some(self.inline_children(node))));
            return;
        } else if let Some(strong) = node.cast::<Strong>() {
            let token = Token::new("strong", "strong").markup(strong.marker.to_string().repeat(2));
            out.push(token.json(Some(self.inline_children(node))));
            return;
        } else if let Some(s) = node.cast::<Strikethrough>() {
            let token = Token::new("s", "s").markup(s.marker.to_string().repeat(2));
            out.push(token.json(Some(self.inline_children(node))));
            return;
        } else if let Some(link) = node.cast::<Link>() {
            let mut token = Token::new("link", "a").attr("href", link.url.clone());
            if let Some(title) = &link.title {
                token = token.attr("title", title.clone());
            }
            out.push(token.json(Some(self.inline_children(node))));
            return;
        } else if let Some(link) = node.cast::<Autolink>() {
            let mut token = Token::new("link", "a")
                .attr("href", link.url.clone())
                .markup("autolink");
            token.info = "auto".into();
            out.push(token.json(Some(self.inline_children(node))));
            return;
        } else if let Some(link) = node.cast::<Linkified>() {
            let mut token = Token::new("link", "a").attr("href", link.url.clone());
            if let Some(class) = link.class {
                token = token.attr("class", class);
            }
            if link.target_blank {
                token = token.attr("target", "_blank");
            }
            token = token.markup("linkify");
            token.info = "auto".into();
            out.push(token.json(Some(self.inline_children(node))));
            return;
        } else if let Some(image) = node.cast::<Image>() {
            let mut token = Token::new("image", "img")
                .attr("src", image.url.clone())
                .attr("alt", "");
            if let Some(title) = &image.title {
                token = token.attr("title", title.clone());
            }
            token.content = self.label(node);
            token.leaf_with_children = true;
            out.push(token.json(Some(self.inline_children(node))));
            return;
        } else if let Some(code) = node.cast::<CodeInline>() {
            let mut token = Token::new("code_inline", "code")
                .markup(code.marker.to_string().repeat(code.marker_len));
            token.content = code_text(node);
            token
        } else if let Some(html) = node.cast::<HtmlInline>() {
            let mut token = Token::new("html_inline", "");
            token.content = html.content.clone();
            token
        } else if let Some(html) = node.cast::<RawHtml>() {
            let mut token = Token::new("html_raw", "");
            token.content = html.0.clone();
            token
        } else if let Some(emoji) = node.cast::<Emoji>() {
            let mut token = Token::new("emoji", "img");
            for (name, value) in emoji.attrs() {
                token = token.attr(name, value);
            }
            token
        } else if let Some(element) = node.cast::<Element>() {
            let mut token = element_token(node, element);
            if token.kind == "code_inline" {
                token.content = code_text(node);
                out.push(token.json(None));
            } else {
                out.push(token.json(Some(self.inline_children(node))));
            }
            return;
        } else {
            out.push(unported(node));
            return;
        };
        out.push(token.json(None));
    }

    /// An image's alt as written: the source its children span, or their
    /// text where they have no source.
    fn label(&self, node: &Node) -> String {
        let span = node
            .children
            .first()
            .and_then(|first| first.srcmap)
            .zip(node.children.last().and_then(|last| last.srcmap))
            .map(|(first, last)| (first.get_byte_offsets().0, last.get_byte_offsets().1));
        match span.and_then(|(start, end)| self.source.get(start..end)) {
            Some(label) => label.to_string(),
            None => node.collect_text(),
        }
    }
}

fn heading_tag(level: u8) -> &'static str {
    ["h1", "h2", "h3", "h4", "h5", "h6"][usize::from(level.clamp(1, 6)) - 1]
}

fn text_token(content: &str) -> Token {
    let mut token = Token::new("text", "");
    token.content = content.to_string();
    token
}

/// The text inside inline code (`CodeText` once sealed).
fn code_text(node: &Node) -> String {
    let mut content = String::new();
    for child in &node.children {
        if let Some(text) = child.cast::<CodeText>() {
            content.push_str(&text.0);
        } else if let Some(text) = child.cast::<Text>() {
            content.push_str(&text.content);
        } else if let Some(text) = child.cast::<BlockText>() {
            content.push_str(&text.0);
        } else {
            content.push_str(&child.collect_text());
        }
    }
    content
}

/// The token an Element's rule recorded, else `unported:` and the tag.
fn element_token(node: &Node, element: &Element) -> Token {
    let mut token = match node.ext.get::<JsToken>() {
        Some(js) => Token::new(&js.kind, "").markup(js.markup.clone()),
        None => Token::new(&format!("unported:{}", element.tag), ""),
    };
    token.owned_tag = Some(element.tag.clone());
    token.attrs = element.attrs.clone();
    token
}

/// A node no token is written for yet: refused by the editor, with its
/// name in the message.
fn unported(node: &Node) -> Value {
    let name = node.name().rsplit("::").next().unwrap_or("node");
    Token::new(&format!("unported:{name}"), "").json(None)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::super::{RenderSettings, render, rich_editor_tree};

    /// The site's settings as the preview receives them, with the
    /// typographer, onebox, censored and watched words on, which the tree
    /// leaves out.
    fn settings() -> RenderSettings {
        serde_json::from_value::<RenderSettings>(json!({
            "breaks": true, "linkify_tlds": ["com", "org"], "typographer": true,
            "mentions": true, "emoji": true, "emoji_shortcuts": true, "inline_emoji": true,
            "emoji_set": "twitter", "emoji_base_path": "/images/emoji",
            "default_code_lang": "auto", "heading_anchor_label": "Heading anchor",
            "allowed_iframes": [], "allowed_href_schemes": [], "checklist": true,
            "footnotes": true, "poll": true, "poll_maximum_options": 20,
            "poll_voters_label": "voters", "local_dates": true,
            "local_dates_email_format": "YYYY-MM-DDTHH:mm:ss[Z]",
            "local_dates_email_timezone": "Etc/UTC", "spoiler": true, "policy": false,
            "avatar_sizes": [24, 48], "base_path": "", "secure_uploads": false,
            "topic_id": null, "force_quote_link": false, "post_id": null, "chat": false,
            "onebox": true,
            "censored": [{"regexp": "(?:\\W|^)(heck)(?=\\W|$)", "case_sensitive": false}],
            "watched_replace": [{"regexp": "(?:\\W|^)(darn)(?=\\W|$)", "replacement": "dang",
                                 "case_sensitive": false, "html": false}],
        }))
        .unwrap()
        .compiled()
        .unwrap()
    }

    /// The tree with adjacent text merged, as the editor's parser merges
    /// it (docs/rich-editor.md: the test is the document, not the split).
    fn tree(raw: &str) -> Value {
        let mut tree = rich_editor_tree(raw, &settings()).unwrap();
        merge_text(&mut tree);
        tree
    }

    fn merge_text(node: &mut Value) {
        let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) else {
            return;
        };
        let mut merged: Vec<Value> = Vec::new();
        for mut child in children.drain(..) {
            merge_text(&mut child);
            let is_text = child["type"] == "text";
            if let Some(last) = merged.last_mut()
                && is_text
                && last["type"] == "text"
            {
                let joined = format!(
                    "{}{}",
                    last["content"].as_str().unwrap_or(""),
                    child["content"].as_str().unwrap_or("")
                );
                last["content"] = Value::String(joined);
                continue;
            }
            merged.push(child);
        }
        *children = merged;
    }

    // The spec's builders (tests/browser/rich-editor.spec.js). An inline
    // token's content is its source, which the spec leaves out: `inline`
    // takes it.
    fn t(content: &str) -> Value {
        json!({"type": "text", "content": content})
    }
    fn inline(content: &str, children: Vec<Value>) -> Value {
        json!({"type": "inline", "content": content, "children": children})
    }
    fn p(content: &str, children: Vec<Value>) -> Value {
        json!({"type": "paragraph", "tag": "p", "children": [inline(content, children)]})
    }
    fn ph(content: &str, children: Vec<Value>) -> Value {
        json!({"type": "paragraph", "tag": "p", "hidden": true, "children": [inline(content, children)]})
    }
    fn softbreak() -> Value {
        json!({"type": "softbreak", "tag": "br"})
    }
    fn bq(children: Vec<Value>) -> Value {
        json!({"type": "blockquote", "tag": "blockquote", "markup": ">", "children": children})
    }
    fn hr(markup: &str) -> Value {
        json!({"type": "hr", "tag": "hr", "markup": markup})
    }
    fn h(level: usize, content: &str, children: Vec<Value>) -> Value {
        json!({"type": "heading", "tag": format!("h{level}"), "markup": "#".repeat(level),
               "children": [inline(content, children)]})
    }
    fn fence(info: &str, content: &str) -> Value {
        let mut node = json!({"type": "fence", "tag": "code", "markup": "```", "content": content});
        if !info.is_empty() {
            node["info"] = json!(info);
        }
        node
    }
    fn li(markup: &str, info: &str, children: Vec<Value>) -> Value {
        let mut node =
            json!({"type": "list_item", "tag": "li", "markup": markup, "children": children});
        if !info.is_empty() {
            node["info"] = json!(info);
        }
        node
    }
    fn ul(markup: &str, items: Vec<Value>) -> Value {
        json!({"type": "bullet_list", "tag": "ul", "markup": markup, "children": items})
    }
    fn ol(start: u32, items: Vec<Value>) -> Value {
        let mut node =
            json!({"type": "ordered_list", "tag": "ol", "markup": ".", "children": items});
        if start != 1 {
            node["attrs"] = json!([["start", start.to_string()]]);
        }
        node
    }
    fn img(src: &str, alt: &str, title: Option<&str>) -> Value {
        let mut attrs = vec![json!(["src", src]), json!(["alt", ""])];
        if let Some(title) = title {
            attrs.push(json!(["title", title]));
        }
        json!({"type": "image", "tag": "img", "attrs": attrs, "content": alt,
               "nesting": 0, "children": [t(alt)]})
    }
    fn em(markup: &str, children: Vec<Value>) -> Value {
        json!({"type": "em", "tag": "em", "markup": markup, "children": children})
    }
    fn strong(markup: &str, children: Vec<Value>) -> Value {
        json!({"type": "strong", "tag": "strong", "markup": markup, "children": children})
    }
    fn s(children: Vec<Value>) -> Value {
        json!({"type": "s", "tag": "s", "markup": "~~", "children": children})
    }
    fn link(href: &str, title: Option<&str>, children: Vec<Value>) -> Value {
        let mut attrs = vec![json!(["href", href])];
        if let Some(title) = title {
            attrs.push(json!(["title", title]));
        }
        json!({"type": "link", "tag": "a", "attrs": attrs, "children": children})
    }
    fn code(content: &str) -> Value {
        json!({"type": "code_inline", "tag": "code", "markup": "`", "content": content})
    }
    fn bbcode(name: &str, children: Vec<Value>) -> Value {
        json!({"type": format!("bbcode_{name}"), "tag": "span",
               "attrs": [["class", format!("bbcode-{name}")]],
               "markup": format!("[{name}]"), "children": children})
    }
    fn doc(children: Vec<Value>) -> Value {
        json!({"type": "doc", "children": children})
    }

    fn check(cases: Vec<(&str, Value)>) {
        for (raw, expected) in cases {
            assert_eq!(tree(raw), expected, "the tree of {raw:?}");
        }
    }

    #[test]
    fn paragraphs_and_breaks() {
        check(vec![
            ("Hello", doc(vec![p("Hello", vec![t("Hello")])])),
            (
                "Hello\nWorld",
                doc(vec![p(
                    "Hello\nWorld",
                    vec![t("Hello"), softbreak(), t("World")],
                )]),
            ),
            (
                "Hello\n\nWorld",
                doc(vec![
                    p("Hello", vec![t("Hello")]),
                    p("World", vec![t("World")]),
                ]),
            ),
        ]);
    }

    #[test]
    fn blockquotes() {
        check(vec![
            ("> Hello", doc(vec![bq(vec![p("Hello", vec![t("Hello")])])])),
            (
                "> Hello\n> World",
                doc(vec![bq(vec![p(
                    "Hello\nWorld",
                    vec![t("Hello"), softbreak(), t("World")],
                )])]),
            ),
            (
                "> Hello\n\n> World",
                doc(vec![
                    bq(vec![p("Hello", vec![t("Hello")])]),
                    bq(vec![p("World", vec![t("World")])]),
                ]),
            ),
        ]);
    }

    #[test]
    fn horizontal_rules() {
        check(vec![
            ("Hey\n\n---", doc(vec![p("Hey", vec![t("Hey")]), hr("---")])),
            ("***", doc(vec![hr("***")])),
            ("- - -", doc(vec![hr("---")])),
        ]);
    }

    #[test]
    fn headings() {
        for level in 1..=6 {
            let raw = format!("{} Hello", "#".repeat(level));
            check(vec![(&raw, doc(vec![h(level, "Hello", vec![t("Hello")])]))]);
        }
        check(vec![(
            "# Hello\nWorld",
            doc(vec![
                h(1, "Hello", vec![t("Hello")]),
                p("World", vec![t("World")]),
            ]),
        )]);
        // Setext: the underline's character is the markup.
        let mut setext = h(2, "Hello", vec![t("Hello")]);
        setext["markup"] = json!("-");
        check(vec![("Hello\n---", doc(vec![setext]))]);
        // The parse leaves heading anchors to the cook.
        assert!(!tree("# Hello").to_string().contains("anchor"));
    }

    #[test]
    fn code_blocks() {
        check(vec![
            ("```\nHello\n```", doc(vec![fence("", "Hello\n")])),
            (
                "```\nHello\nWorld\n```",
                doc(vec![fence("", "Hello\nWorld\n")]),
            ),
            (
                "```\nHello\n\nWorld\n```",
                doc(vec![fence("", "Hello\n\nWorld\n")]),
            ),
            (
                "```ruby\nHello\n```\n\nWorld",
                doc(vec![fence("ruby", "Hello\n"), p("World", vec![t("World")])]),
            ),
            (
                "    indented\n",
                doc(vec![
                    json!({"type": "code_block", "tag": "code", "content": "indented\n"}),
                ]),
            ),
        ]);
    }

    #[test]
    fn ordered_lists() {
        check(vec![
            (
                "1. Hello",
                doc(vec![ol(
                    1,
                    vec![li(".", "1", vec![ph("Hello", vec![t("Hello")])])],
                )]),
            ),
            (
                "1. Hello\n2. World",
                doc(vec![ol(
                    1,
                    vec![
                        li(".", "1", vec![ph("Hello", vec![t("Hello")])]),
                        li(".", "2", vec![ph("World", vec![t("World")])]),
                    ],
                )]),
            ),
            (
                "5. Hello\n\n6. World",
                doc(vec![ol(
                    5,
                    vec![
                        li(".", "5", vec![p("Hello", vec![t("Hello")])]),
                        li(".", "6", vec![p("World", vec![t("World")])]),
                    ],
                )]),
            ),
        ]);
    }

    #[test]
    fn bullet_lists() {
        check(vec![
            (
                "* Hello",
                doc(vec![ul(
                    "*",
                    vec![li("*", "", vec![ph("Hello", vec![t("Hello")])])],
                )]),
            ),
            (
                "* Hello\n* World",
                doc(vec![ul(
                    "*",
                    vec![
                        li("*", "", vec![ph("Hello", vec![t("Hello")])]),
                        li("*", "", vec![ph("World", vec![t("World")])]),
                    ],
                )]),
            ),
            (
                "* Hello\n\n* World",
                doc(vec![ul(
                    "*",
                    vec![
                        li("*", "", vec![p("Hello", vec![t("Hello")])]),
                        li("*", "", vec![p("World", vec![t("World")])]),
                    ],
                )]),
            ),
            // A tight item's text, then a nested list: the text alone is
            // the hidden paragraph.
            (
                "- a\n  - b\n- c",
                doc(vec![ul(
                    "-",
                    vec![
                        li(
                            "-",
                            "",
                            vec![
                                ph("a", vec![t("a")]),
                                ul("-", vec![li("-", "", vec![ph("b", vec![t("b")])])]),
                            ],
                        ),
                        li("-", "", vec![ph("c", vec![t("c")])]),
                    ],
                )]),
            ),
        ]);
    }

    #[test]
    fn images() {
        check(vec![
            (
                "![alt](src)",
                doc(vec![p("![alt](src)", vec![img("src", "alt", None)])]),
            ),
            (
                "![alt](src \"title\")",
                doc(vec![p(
                    "![alt](src \"title\")",
                    vec![img("src", "alt", Some("title"))],
                )]),
            ),
        ]);
    }

    #[test]
    fn marks() {
        check(vec![
            (
                "*Hello*",
                doc(vec![p("*Hello*", vec![em("*", vec![t("Hello")])])]),
            ),
            (
                "_Hello_",
                doc(vec![p("_Hello_", vec![em("_", vec![t("Hello")])])]),
            ),
            (
                "**Hello**",
                doc(vec![p("**Hello**", vec![strong("**", vec![t("Hello")])])]),
            ),
            (
                "__Hello__",
                doc(vec![p("__Hello__", vec![strong("__", vec![t("Hello")])])]),
            ),
            (
                "[text](href)",
                doc(vec![p(
                    "[text](href)",
                    vec![link("href", None, vec![t("text")])],
                )]),
            ),
            (
                "[text](href \"title\")",
                doc(vec![p(
                    "[text](href \"title\")",
                    vec![link("href", Some("title"), vec![t("text")])],
                )]),
            ),
            (
                "Hel`lo wo`rld",
                doc(vec![p(
                    "Hel`lo wo`rld",
                    vec![t("Hel"), code("lo wo"), t("rld")],
                )]),
            ),
            (
                "___[`Hello`](https://example.com)___",
                doc(vec![p(
                    "___[`Hello`](https://example.com)___",
                    vec![em(
                        "_",
                        vec![strong(
                            "__",
                            vec![link("https://example.com", None, vec![code("Hello")])],
                        )],
                    )],
                )]),
            ),
            (
                "~~Hello~~",
                doc(vec![p("~~Hello~~", vec![s(vec![t("Hello")])])]),
            ),
        ]);
    }

    #[test]
    fn typographer_is_off() {
        for raw in [
            "hello -- world",
            "hello --- world",
            "hello... world",
            "+-5",
            "(tm)",
            "(pa)",
            "\"hello\" world",
            "it's here",
        ] {
            check(vec![(raw, doc(vec![p(raw, vec![t(raw)])]))]);
        }
        check(vec![(
            "`hello -- world`",
            doc(vec![p("`hello -- world`", vec![code("hello -- world")])]),
        )]);
        // The cook, with the same settings, has it on.
        let (html, _) = render("hello -- world", &settings(), Default::default()).unwrap();
        assert_eq!(html, "<p>hello – world</p>");
    }

    #[test]
    fn bbcode_inline() {
        check(vec![
            (
                "[s]Hello[/s]",
                doc(vec![p("[s]Hello[/s]", vec![bbcode("s", vec![t("Hello")])])]),
            ),
            (
                "Hey [s]wrod[/s] ~~uord~~ World",
                doc(vec![p(
                    "Hey [s]wrod[/s] ~~uord~~ World",
                    vec![
                        t("Hey "),
                        bbcode("s", vec![t("wrod")]),
                        t(" "),
                        s(vec![t("uord")]),
                        t(" World"),
                    ],
                )]),
            ),
            (
                "___[s][`Hello`](https://example.com)[/s]___",
                doc(vec![p(
                    "___[s][`Hello`](https://example.com)[/s]___",
                    vec![em(
                        "_",
                        vec![strong(
                            "__",
                            vec![bbcode(
                                "s",
                                vec![link("https://example.com", None, vec![code("Hello")])],
                            )],
                        )],
                    )],
                )]),
            ),
            (
                "[u]Hello[/u]",
                doc(vec![p("[u]Hello[/u]", vec![bbcode("u", vec![t("Hello")])])]),
            ),
            (
                "[u]Hello[/u] [u]World[/u]",
                doc(vec![p(
                    "[u]Hello[/u] [u]World[/u]",
                    vec![
                        bbcode("u", vec![t("Hello")]),
                        t(" "),
                        bbcode("u", vec![t("World")]),
                    ],
                )]),
            ),
            (
                "[b]x[/b]",
                doc(vec![p("[b]x[/b]", vec![bbcode("b", vec![t("x")])])]),
            ),
            (
                "[i]x[/i]",
                doc(vec![p("[i]x[/i]", vec![bbcode("i", vec![t("x")])])]),
            ),
        ]);
    }

    #[test]
    fn links_made_by_rules() {
        let linkified = json!({"type": "link", "tag": "a", "attrs": [["href", "https://example.com"]],
                               "markup": "linkify", "info": "auto", "children": [t("https://example.com")]});
        // Alone on its line: a onebox in the cook, a plain link here.
        check(vec![(
            "https://example.com",
            doc(vec![p("https://example.com", vec![linkified])]),
        )]);
        let autolink = json!({"type": "link", "tag": "a", "attrs": [["href", "https://example.com"]],
                              "markup": "autolink", "info": "auto", "children": [t("https://example.com")]});
        check(vec![(
            "<https://example.com>",
            doc(vec![p("<https://example.com>", vec![autolink])]),
        )]);
    }

    #[test]
    fn censored_and_watched_words_are_left_out() {
        check(vec![(
            "heck darn",
            doc(vec![p("heck darn", vec![t("heck darn")])]),
        )]);
        let (html, _) = render("heck darn", &settings(), Default::default()).unwrap();
        assert_ne!(html, "<p>heck darn</p>");
    }

    #[test]
    fn emoji_is_its_token() {
        let tree = tree("hi :smile:");
        let emoji = &tree["children"][0]["children"][0]["children"][1];
        assert_eq!(emoji["type"], "emoji");
        assert_eq!(emoji["tag"], "img");
        assert_eq!(emoji["attrs"][3], json!(["alt", ":smile:"]));
    }

    #[test]
    fn what_is_not_written_yet_is_unported() {
        // A mention's span records no token yet: the editor refuses it.
        let tree = tree("hi @sam");
        let mention = &tree["children"][0]["children"][0]["children"][1];
        assert!(
            mention["type"].as_str().unwrap().starts_with("unported:"),
            "{mention}"
        );
    }

    #[test]
    fn tables_hold_inline_cells() {
        let tree = tree("| a | b |\n|:--|--:|\n| c | d |");
        let table = &tree["children"][0];
        assert_eq!(table["type"], "table");
        let th = &table["children"][0]["children"][0]["children"][0];
        assert_eq!(th["type"], "th");
        assert_eq!(th["attrs"], json!([["style", "text-align:left"]]));
        assert_eq!(th["children"][0]["type"], "inline");
        assert_eq!(th["children"][0]["children"][0], t("a"));
        let td = &table["children"][1]["children"][0]["children"][1];
        assert_eq!(td["type"], "td");
        assert_eq!(td["children"][0]["children"][0], t("d"));
    }
}
