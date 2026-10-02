//! Port of lib/onpdiff.rb and lib/discourse_diff.rb: the O(NP) diff and
//! the three renderings PostRevisionSerializer shows as `body_changes`.
//!
//! DiscourseDiff tokenizes a block's HTML with libxml2's HTML4 SAX parser;
//! here html5ever parses it and the same tokens are read off the tree.

use html5ever::serialize::{SerializeOpts, TraversalScope, serialize};
use markup5ever_rcdom::{Handle, NodeData, SerializableHandle};

use crate::pretty_text::cleanup::{fragment_root, parse};

/// `ONPDiff::DiffLimitExceeded`
#[derive(Debug, PartialEq)]
pub struct DiffLimitExceeded;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Common,
    Add,
    Delete,
}

const DEFAULT_COMPARISON_BUDGET_FACTOR: usize = 200;
const MAX_COMPARISON_BUDGET: usize = 2_000_000;

/// `ONPDiff.new(a, b)`
pub struct OnpDiff<'a> {
    a: &'a [String],
    b: &'a [String],
    m: usize,
    n: usize,
    reverse: bool,
    budget: usize,
    used: usize,
    path: Vec<i64>,
    backtrack: Vec<(i64, i64, i64)>,
}

impl<'a> OnpDiff<'a> {
    pub fn new(a: &'a [String], b: &'a [String]) -> Self {
        let reverse = a.len() > b.len();
        let (a, b) = if reverse { (b, a) } else { (a, b) };
        OnpDiff {
            a,
            b,
            m: a.len(),
            n: b.len(),
            reverse,
            budget: (DEFAULT_COMPARISON_BUDGET_FACTOR * (a.len() + b.len()))
                .min(MAX_COMPARISON_BUDGET),
            used: 0,
            path: Vec::new(),
            backtrack: Vec::new(),
        }
    }

    fn compose(&mut self) -> Result<Vec<(i64, i64)>, DiffLimitExceeded> {
        let (m, n) = (self.m as i64, self.n as i64);
        let offset = m + 1;
        let delta = n - m;
        let size = (m + n + 3) as usize;
        let mut fp = vec![-1i64; size];
        self.path = vec![-1; size];
        let idx = |k: i64| (k + offset) as usize;
        let mut p = -1i64;
        loop {
            p += 1;
            let mut k = -p;
            while k <= delta - 1 {
                fp[idx(k)] = self.snake(k, fp[idx(k - 1)] + 1, fp[idx(k + 1)], offset)?;
                k += 1;
            }
            let mut k = delta + p;
            while k >= delta + 1 {
                fp[idx(k)] = self.snake(k, fp[idx(k - 1)] + 1, fp[idx(k + 1)], offset)?;
                k -= 1;
            }
            fp[idx(delta)] = self.snake(delta, fp[idx(delta - 1)] + 1, fp[idx(delta + 1)], offset)?;
            if fp[idx(delta)] == n {
                break;
            }
        }
        let mut r = self.path[idx(delta)];
        let mut shortest = Vec::new();
        while r != -1 {
            let (x, y, prev) = self.backtrack[r as usize];
            shortest.push((x, y));
            r = prev;
        }
        Ok(shortest)
    }

    fn snake(&mut self, k: i64, p: i64, pp: i64, offset: i64) -> Result<i64, DiffLimitExceeded> {
        let k_offset = (k + offset) as usize;
        let (r, mut y) = if p > pp {
            (self.path[k_offset - 1], p)
        } else {
            (self.path[k_offset + 1], pp)
        };
        let mut x = y - k;
        while (x as usize) < self.m && (y as usize) < self.n {
            self.used += 1;
            if self.used > self.budget {
                return Err(DiffLimitExceeded);
            }
            if self.a[x as usize] != self.b[y as usize] {
                break;
            }
            x += 1;
            y += 1;
        }
        self.path[k_offset] = self.backtrack.len() as i64;
        self.backtrack.push((x, y, r));
        Ok(y)
    }

    /// Walks the shortest path, calling `step` for every token.
    fn walk(
        &mut self,
        mut step: impl FnMut(&'a str, Op),
    ) -> Result<(), DiffLimitExceeded> {
        let path = self.compose()?;
        let (add, delete) = if self.reverse {
            (Op::Delete, Op::Add)
        } else {
            (Op::Add, Op::Delete)
        };
        let (mut px, mut py) = (0i64, 0i64);
        for &(sx, sy) in path.iter().rev() {
            while px < sx || py < sy {
                if sy - sx > py - px {
                    step(&self.b[py as usize], add);
                    py += 1;
                } else if sy - sx < py - px {
                    step(&self.a[px as usize], delete);
                    px += 1;
                } else {
                    step(&self.a[px as usize], Op::Common);
                    px += 1;
                    py += 1;
                }
            }
        }
        Ok(())
    }

    /// `#diff`
    pub fn diff(mut self) -> Result<Vec<(String, Op)>, DiffLimitExceeded> {
        let mut ses = Vec::new();
        self.walk(|t, op| ses.push((t.to_string(), op)))?;
        Ok(ses)
    }

    /// `#short_diff`: runs of one operation joined.
    pub fn short_diff(mut self) -> Result<Vec<(String, Op)>, DiffLimitExceeded> {
        let mut ses: Vec<(String, Op)> = Vec::new();
        self.walk(|t, op| match ses.last_mut() {
            Some(last) if last.1 == op => last.0.push_str(t),
            _ => ses.push((t.to_string(), op)),
        })?;
        Ok(ses)
    }

    /// `#paragraph_diff`
    pub fn paragraph_diff(self) -> Result<Vec<(String, Op)>, DiffLimitExceeded> {
        let ses = self.diff()?;
        let mut out: Vec<(String, Op)> = Vec::new();
        let mut i = 0usize;
        while i < ses.len() {
            if ses[i].1 == Op::Common {
                out.push(ses[i].clone());
            } else {
                let (op, opposite) = if ses[i].1 == Op::Add {
                    (Op::Add, Op::Delete)
                } else {
                    (Op::Delete, Op::Add)
                };
                let mut j = i + 1;
                while j < ses.len() && ses[j].1 == op {
                    j += 1;
                }
                if j >= ses.len() {
                    out.extend_from_slice(&ses[i..]);
                    i = j;
                } else {
                    let mut k = j;
                    j -= 1;
                    while k < ses.len() && ses[k].1 == opposite {
                        k += 1;
                    }
                    k -= 1;
                    let num_before = j - i + 1;
                    let num_after = k - j;
                    if num_after > 1 {
                        if num_before > num_after {
                            let i2 = i + num_before - num_after;
                            out.extend_from_slice(&ses[i..i2]);
                            i = i2;
                        } else if num_after > num_before {
                            k -= num_after - num_before;
                        }
                        // pair_paragraphs(ses, i, j)
                        let pairs = j - i + 1;
                        for n in 0..pairs {
                            out.push(ses[i + n].clone());
                            out.push(ses[i + n + pairs].clone());
                        }
                    } else {
                        out.extend_from_slice(&ses[i..=k]);
                    }
                    i = k;
                }
            }
            i += 1;
        }
        Ok(out)
    }
}

/// The size of what differs between two strings, by character
/// (PostRevisor#diff_size); `None` when the diff is too large to compute.
pub fn diff_size(before: &str, after: &str) -> Option<usize> {
    let a: Vec<String> = before.chars().map(String::from).collect();
    let b: Vec<String> = after.chars().map(String::from).collect();
    let ses = OnpDiff::new(&a, &b).short_diff().ok()?;
    Some(
        ses.iter()
            .filter(|(_, op)| *op != Op::Common)
            .map(|(s, _)| s.chars().count())
            .sum(),
    )
}

/// What `body_changes` holds.
#[derive(Debug, PartialEq)]
pub struct BodyChanges {
    pub inline: String,
    pub side_by_side: String,
    pub side_by_side_markdown: String,
}

/// `DiscourseDiff.new(cooked..)` and `DiscourseDiff.new(raw..)` rendered.
pub fn body_changes(
    cooked_before: &str,
    cooked_after: &str,
    raw_before: &str,
    raw_after: &str,
) -> Result<BodyChanges, DiffLimitExceeded> {
    let blocks = block_by_block_diff(cooked_before, cooked_after)?;
    Ok(BodyChanges {
        inline: inline_html(&blocks)?,
        side_by_side: side_by_side_html(&blocks)?,
        side_by_side_markdown: side_by_side_markdown(raw_before, raw_after)?,
    })
}

const MAX_DIFFERENCE: usize = 200;
const CLASS_ATTRIBUTE: &str = " class=\"";

fn block_by_block_diff(before: &str, after: &str) -> Result<Vec<(String, Op)>, DiffLimitExceeded> {
    let a = tokenize_html_blocks(before);
    let b = tokenize_html_blocks(after);
    OnpDiff::new(&a, &b).paragraph_diff()
}

/// The pair `i` and its opposite next to it, as (before, after) blocks.
fn paired(blocks: &[(String, Op)], i: usize) -> Option<(&str, &str)> {
    let op = blocks[i].1;
    let opposite = if op == Op::Delete { Op::Add } else { Op::Delete };
    let next = blocks.get(i + 1).filter(|b| b.1 == opposite)?;
    Some(if op == Op::Delete {
        (&blocks[i].0, &next.0)
    } else {
        (&next.0, &blocks[i].0)
    })
}

fn inline_html(blocks: &[(String, Op)]) -> Result<String, DiffLimitExceeded> {
    let mut inline = String::new();
    let mut i = 0;
    while i < blocks.len() {
        let (text, op) = &blocks[i];
        if *op == Op::Common {
            inline.push_str(text);
        } else if let Some((first, second)) = paired(blocks, i) {
            let (a, b) = (tokenize_html(first), tokenize_html(second));
            for (t, op) in OnpDiff::new(&a, &b).diff()? {
                match op {
                    Op::Common => inline.push_str(&t),
                    Op::Delete => inline.push_str(&add_class_or_wrap_in_tags(&t, "del")),
                    Op::Add => inline.push_str(&add_class_or_wrap_in_tags(&t, "ins")),
                }
            }
            i += 1;
        } else {
            let klass = if *op == Op::Delete { "del" } else { "ins" };
            inline.push_str(&add_class_or_wrap_in_tags(text, klass));
        }
        i += 1;
    }
    Ok(format!("<div class=\"inline-diff\">{inline}</div>"))
}

fn side_by_side_html(blocks: &[(String, Op)]) -> Result<String, DiffLimitExceeded> {
    let (mut left, mut right) = (String::new(), String::new());
    let mut i = 0;
    while i < blocks.len() {
        let (text, op) = &blocks[i];
        if *op == Op::Common {
            left.push_str(text);
            right.push_str(text);
        } else if let Some((first, second)) = paired(blocks, i) {
            let (a, b) = (tokenize_html(first), tokenize_html(second));
            for (t, op) in OnpDiff::new(&a, &b).diff()? {
                match op {
                    Op::Common => {
                        left.push_str(&t);
                        right.push_str(&t);
                    }
                    Op::Delete => left.push_str(&add_class_or_wrap_in_tags(&t, "del")),
                    Op::Add => right.push_str(&add_class_or_wrap_in_tags(&t, "ins")),
                }
            }
            i += 1;
        } else if *op == Op::Delete {
            left.push_str(&add_class_or_wrap_in_tags(text, "del"));
        } else {
            right.push_str(&add_class_or_wrap_in_tags(text, "ins"));
        }
        i += 1;
    }
    Ok(format!(
        "<div class=\"revision-content --previous\">{left}</div><div class=\"revision-content --current\">{right}</div>"
    ))
}

fn side_by_side_markdown(before: &str, after: &str) -> Result<String, DiffLimitExceeded> {
    let a = tokenize_line(&escape_html(before));
    let b = tokenize_line(&escape_html(after));
    let lines = OnpDiff::new(&a, &b).short_diff()?;
    let mut table = String::from("<table class=\"markdown\">");
    let mut i = 0;
    while i < lines.len() {
        table.push_str("<tr>");
        let (text, op) = &lines[i];
        if *op == Op::Common {
            table.push_str(&format!("<td class=\"--previous\">{text}</td>"));
            table.push_str(&format!("<td class=\"--current\">{text}</td>"));
        } else if let Some((first, second)) = paired(&lines, i) {
            let (mut a, mut b) = (tokenize_markdown(first), tokenize_markdown(second));
            if a.len().abs_diff(b.len()) > MAX_DIFFERENCE {
                a = tokenize_line(first);
                b = tokenize_line(second);
            }
            let (mut deleted, mut inserted) = (String::new(), String::new());
            for (t, op) in OnpDiff::new(&a, &b).short_diff()? {
                match op {
                    Op::Common => {
                        deleted.push_str(&t);
                        inserted.push_str(&t);
                    }
                    Op::Delete => deleted.push_str(&format!("<del>{t}</del>")),
                    Op::Add => inserted.push_str(&format!("<ins>{t}</ins>")),
                }
            }
            table.push_str(&format!("<td class=\"--previous\">{deleted}</td>"));
            table.push_str(&format!("<td class=\"--current\">{inserted}</td>"));
            i += 1;
        } else if *op == Op::Delete {
            table.push_str(&format!("<td class=\"--previous diff-del\">{text}</td>"));
            table.push_str("<td class=\"--current\"></td>");
        } else {
            table.push_str("<td class=\"--previous\"></td>");
            table.push_str(&format!("<td class=\"--current diff-ins\">{text}</td>"));
        }
        table.push_str("</tr>");
        i += 1;
    }
    table.push_str("</table>");
    Ok(table)
}

/// `CGI.escapeHTML`
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// `text.scan(/[^\r\n]+[\r\n]*/)`
fn tokenize_line(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_breaks = false;
    for c in text.chars() {
        let brk = c == '\r' || c == '\n';
        if brk {
            if !cur.is_empty() {
                cur.push(c);
                in_breaks = true;
            }
        } else {
            if in_breaks {
                out.push(std::mem::take(&mut cur));
                in_breaks = false;
            }
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Ruby's `\w`, which is ASCII-only.
fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn tokenize_markdown(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut t = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if is_word(c) {
            t.push(c);
        } else if (c == ' ' || c == '\t') && !t.is_empty() && t.chars().all(is_word) {
            while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
                t.push(chars[i]);
                i += 1;
            }
            i -= 1;
            tokens.push(std::mem::take(&mut t));
        } else {
            if !t.is_empty() {
                tokens.push(std::mem::take(&mut t));
            }
            tokens.push(c.to_string());
        }
        i += 1;
    }
    if !t.is_empty() {
        tokens.push(t);
    }
    tokens
}

/// `Nokogiri::HTML5.fragment(html).search("./*").map(&:to_html)`
fn tokenize_html_blocks(html: &str) -> Vec<String> {
    let dom = parse(html);
    let root = fragment_root(&dom);
    let children = root.children.borrow();
    children
        .iter()
        .filter(|c| matches!(c.data, NodeData::Element { .. }))
        .map(|c| {
            let mut out = Vec::new();
            let handle: SerializableHandle = c.clone().into();
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
        })
        .collect()
}

const AUTOCLOSING_TAGS: [&str; 9] = ["area", "base", "br", "col", "embed", "hr", "img", "input", "meta"];

/// `HtmlTokenizer.tokenize`: start tags with their attributes, end tags
/// but for void elements, and text split into words and single
/// non-word characters, all escaped.
fn tokenize_html(html: &str) -> Vec<String> {
    let dom = parse(html);
    let mut tokens = Vec::new();
    for child in fragment_root(&dom).children.borrow().iter() {
        tokenize_node(child, &mut tokens);
    }
    tokens
}

fn tokenize_node(node: &Handle, tokens: &mut Vec<String>) {
    match &node.data {
        NodeData::Element { name, attrs, .. } => {
            let name = &*name.local;
            let mut tag = format!("<{name}");
            for a in attrs.borrow().iter() {
                tag.push_str(&format!(" {}=\"{}\"", &*a.name.local, escape_html(&a.value)));
            }
            tag.push('>');
            tokens.push(tag);
            // libxml2's HTML4 parser does not know `source` and `track` are
            // void: what follows one is inside it until its parent closes.
            let mut open = Vec::new();
            for child in node.children.borrow().iter() {
                tokenize_node(child, tokens);
                if let NodeData::Element { name, .. } = &child.data {
                    if matches!(&*name.local, "source" | "track") {
                        open.push(name.local.to_string());
                    }
                }
            }
            for unclosed in open.iter().rev() {
                tokens.push(format!("</{unclosed}>"));
            }
            if !AUTOCLOSING_TAGS.contains(&name) && !matches!(name, "source" | "track") {
                tokens.push(format!("</{name}>"));
            }
        }
        NodeData::Text { contents } => characters(&contents.borrow(), tokens),
        _ => {}
    }
}

/// `HtmlTokenizer#characters`: whitespace after a tag joins the tag;
/// text is `scan(/\W|\w+[ \t]*/)`.
fn characters(text: &str, tokens: &mut Vec<String>) {
    if text.chars().all(char::is_whitespace) {
        if let Some(last) = tokens.last_mut().filter(|t| t.starts_with('<')) {
            last.push_str(text);
            return;
        }
    }
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if is_word(chars[i]) {
            let start = i;
            while i < chars.len() && is_word(chars[i]) {
                i += 1;
            }
            while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
                i += 1;
            }
            tokens.push(escape_html(&chars[start..i].iter().collect::<String>()));
        } else {
            tokens.push(escape_html(&chars[i].to_string()));
            i += 1;
        }
    }
}

fn add_class_or_wrap_in_tags(html_or_text: &str, klass: &str) -> String {
    if html_or_text.starts_with("</") {
        return html_or_text.to_string();
    }
    let chevron = html_or_text.find('>');
    let Some(chevron) = chevron.filter(|_| html_or_text.starts_with('<')) else {
        return format!("<{klass}>{html_or_text}</{klass}>");
    };
    match html_or_text.find(CLASS_ATTRIBUTE) {
        Some(class_index) if class_index <= chevron => {
            let at = class_index + CLASS_ATTRIBUTE.len();
            format!("{}diff-{klass} {}", &html_or_text[..at], &html_or_text[at..])
        }
        _ => format!(
            "{}{CLASS_ATTRIBUTE}diff-{klass}\"{}",
            &html_or_text[..chevron],
            &html_or_text[chevron..]
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_diff_of_characters() {
        assert_eq!(diff_size("abc", "abd"), Some(2));
        assert_eq!(diff_size("same", "same"), Some(0));
    }

    #[test]
    fn markdown_tokens() {
        assert_eq!(
            tokenize_markdown("Reply one, also"),
            vec!["Reply ", "one", ",", " ", "also"]
        );
    }
}
