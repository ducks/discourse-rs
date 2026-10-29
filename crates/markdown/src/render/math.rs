//! discourse-math's rules (plugins/discourse-math, lib/discourse-markdown/
//! discourse-math.js): `$...$` (and `%...%` for asciimath) inline,
//! `\(...\)` with the LaTeX delimiters, `$$...$$` and `\[...\]` as blocks.
//! The math stays text in a `span.math`, `span.asciimath` or `div.math`;
//! the client typesets it with MathJax or KaTeX.

use markdown_it::common::utils::escape_html;
use markdown_it::parser::block::{BlockRule, BlockState};
use markdown_it::parser::inline::{InlineRule, InlineState};
use markdown_it::plugins::cmark::block::code::CodeScanner;
use markdown_it::plugins::cmark::inline::escape::EscapeScanner;
use markdown_it::{MarkdownIt, Node, NodeValue, Renderer};

use super::RenderSettings;
use super::md_utils::{is_punct_char, is_white_space};
use crate::sanitizer::AllowList;

/// A `math_inline` token: its content, and whether `%` delimited it.
#[derive(Debug)]
pub struct MathInline {
    pub content: String,
    pub asciimath: bool,
}

impl NodeValue for MathInline {
    fn render(&self, _: &Node, fmt: &mut dyn Renderer) {
        let class = if self.asciimath { "asciimath" } else { "math" };
        fmt.text_raw(&format!(
            "<span class='{class}'>{}</span>",
            escape_html(&self.content)
        ));
    }
}

/// A `math_block` token.
#[derive(Debug)]
pub struct MathBlock {
    pub content: String,
}

impl NodeValue for MathBlock {
    fn render(&self, _: &Node, fmt: &mut dyn Renderer) {
        fmt.cr();
        fmt.text_raw(&format!(
            "<div class='math'>\n{}\n</div>\n",
            escape_html(&self.content)
        ));
    }
}

/// `additionalPunctuation`: CJK, full-width, Arabic and Thai marks.
const ADDITIONAL_PUNCTUATION: [char; 12] = [
    '\u{3001}', '\u{3002}', '\u{FF0C}', '\u{FF1A}', '\u{FF1B}', '\u{FF0E}', '\u{FF1F}', '\u{FF01}',
    '\u{060C}', '\u{061B}', '\u{061F}', '\u{0E2F}',
];

/// `isSafeBoundary`, on the UTF-16 unit JS reads: a character outside
/// the BMP is a surrogate there, which is never a boundary.
fn is_safe_boundary(c: char, delimiter: char) -> bool {
    if c == delimiter || u32::from(c) > 0xFFFF {
        return false;
    }
    is_white_space(c)
        || c.is_ascii_punctuation()
        || is_punct_char(c)
        || ADDITIONAL_PUNCTUATION.contains(&c)
}

/// `isEscaped`: an odd run of backslashes before the byte at `index`.
fn is_escaped(src: &[u8], index: usize) -> bool {
    src[..index]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
        % 2
        == 1
}

/// `math_input`: a `$` (or `%`) span on one line, with a safe boundary on
/// either side and not doubled.
fn math_input(state: &InlineState, delimiter: char) -> Option<(Node, usize)> {
    let src = state.src.as_str();
    let bytes = src.as_bytes();
    let d = delimiter as u8;
    let (pos, pos_max) = (state.pos, state.pos_max);
    if bytes.get(pos) != Some(&d) || pos_max < pos + 2 || bytes.get(pos + 1) == Some(&d) {
        return None;
    }
    if let Some(prev) = src[..pos].chars().next_back()
        && !is_safe_boundary(prev, delimiter)
    {
        return None;
    }
    let found = (pos + 1..pos_max).find(|&i| bytes[i] == d && !is_escaped(bytes, i))?;
    // JS reads the next unit past posMax too, and none at the end.
    if let Some(next) = src[found + 1..].chars().next()
        && !is_safe_boundary(next, delimiter)
    {
        return None;
    }
    let data = &src[pos + 1..found];
    if data.contains('\n') {
        return None;
    }
    let node = Node::new(MathInline {
        content: data.to_string(),
        asciimath: delimiter == '%',
    });
    Some((node, found + 1 - pos))
}

struct InlineMath;
impl InlineRule for InlineMath {
    const MARKER: char = '$';

    // Silent mode declines (`if (silent || ...) return false`).
    fn check(_: &mut InlineState) -> Option<usize> {
        None
    }

    fn run(state: &mut InlineState) -> Option<(Node, usize)> {
        if !state.md.ext.get::<RenderSettings>()?.math {
            return None;
        }
        math_input(state, '$')
    }
}

struct AsciiMath;
impl InlineRule for AsciiMath {
    const MARKER: char = '%';

    fn check(_: &mut InlineState) -> Option<usize> {
        None
    }

    fn run(state: &mut InlineState) -> Option<(Node, usize)> {
        let settings = state.md.ext.get::<RenderSettings>()?;
        if !(settings.math && settings.math_asciimath) {
            return None;
        }
        math_input(state, '%')
    }
}

/// `findClosingDelimiter`: the first `close` after `start` in the whole
/// source with an even run of backslashes before it.
fn find_closing_delimiter(src: &[u8], start: usize, close: &[u8]) -> Option<usize> {
    (start..=src.len().checked_sub(close.len())?)
        .find(|&i| &src[i..i + close.len()] == close && !is_escaped(src, i))
}

/// `inlineMathParen` (`math_input_delimited` with `\(` and `\)`).
struct InlineMathParen;
impl InlineRule for InlineMathParen {
    const MARKER: char = '\\';

    fn check(_: &mut InlineState) -> Option<usize> {
        None
    }

    fn run(state: &mut InlineState) -> Option<(Node, usize)> {
        let settings = state.md.ext.get::<RenderSettings>()?;
        if !(settings.math && settings.math_latex_delimiters) {
            return None;
        }
        let bytes = state.src.as_bytes();
        let pos = state.pos;
        if state.pos_max < pos + 2 || !bytes[pos..].starts_with(b"\\(") {
            return None;
        }
        let start = pos + 2;
        let end = find_closing_delimiter(bytes, start, b"\\)")?;
        // JS searches past posMax (a link's label); a span that would end
        // beyond it is left alone here rather than overrun the label.
        if end + 2 > state.pos_max {
            return None;
        }
        let data = &state.src[start..end];
        if data.is_empty() || data.contains('\n') {
            return None;
        }
        let node = Node::new(MathInline {
            content: data.to_string(),
            asciimath: false,
        });
        Some((node, end + 2 - pos))
    }
}

/// `md.utils.isSpace`: space or tab.
fn only_space_after(src: &str, start: usize, max: usize) -> bool {
    src.as_bytes()
        .get(start..max)
        .is_none_or(|rest| rest.iter().all(|&b| b == b' ' || b == b'\t'))
}

/// `bMarks[line] + tShift[line]`, the sentinel past the last line being
/// the source's length.
fn line_start(state: &BlockState, line: usize) -> usize {
    state
        .line_offsets
        .get(line)
        .map_or(state.src.len(), |l| l.first_nonspace)
}

/// `eMarks[line]`, likewise.
fn line_end(state: &BlockState, line: usize) -> usize {
    state
        .line_offsets
        .get(line)
        .map_or(state.src.len(), |l| l.line_end)
}

fn is_marker(state: &BlockState, line: usize, marker: &str) -> bool {
    let (start, max) = (line_start(state, line), line_end(state, line));
    state.src.as_bytes()[start..].starts_with(marker.as_bytes())
        && only_space_after(state.src, start + 2, max)
}

/// What `blockMath` makes of a line: a block of this many lines, a
/// refusal, or nothing it recognizes (`trySingleLineBlockMath`'s null).
enum Found {
    Block(String, usize),
    Refused,
    Nothing,
}

fn block_math(state: &BlockState, silent: bool) -> Found {
    let Some(settings) = state.md.ext.get::<RenderSettings>() else {
        return Found::Nothing;
    };
    if !settings.math {
        return Found::Nothing;
    }
    let start_line = state.line;
    let (start, max) = (line_start(state, start_line), line_end(state, start_line));
    let line = state.src[start..max].trim();

    // trySingleLineBlockMath
    let mut patterns = vec![("$$", "$$")];
    if settings.math_latex_delimiters {
        patterns.push(("\\[", "\\]"));
    }
    for (open, close) in patterns {
        if line.starts_with(open) && line.ends_with(close) && line.len() > open.len() + close.len()
        {
            if silent {
                return Found::Block(String::new(), 1);
            }
            let content = line[open.len()..line.len() - close.len()].trim();
            if content.is_empty() {
                return Found::Refused;
            }
            return Found::Block(content.to_string(), 1);
        }
    }

    let end_marker = if is_marker(state, start_line, "$$") {
        "$$"
    } else if settings.math_latex_delimiters && is_marker(state, start_line, "\\[") {
        "\\]"
    } else {
        return Found::Nothing;
    };
    if silent {
        return Found::Block(String::new(), 1);
    }
    // findClosingBlockLine: the end marker on a later line, whatever its
    // indent, or the end of the block's range.
    let mut next_line = start_line;
    let closed = loop {
        next_line += 1;
        if next_line >= state.line_max {
            break false;
        }
        if is_marker(state, next_line, end_marker) {
            break true;
        }
    };
    // extractMultilineBlockContent: a slice of the source, so a range
    // that ends before it starts (nothing between the markers) is empty.
    let content_start = line_start(state, start_line + 1);
    let content_end = line_end(state, if closed { next_line - 1 } else { next_line });
    let content = state
        .src
        .get(content_start..content_end)
        .unwrap_or("")
        .to_string();
    let consumed = if closed { next_line + 1 } else { next_line } - start_line;
    Found::Block(content, consumed)
}

struct BlockMath;
impl BlockRule for BlockMath {
    fn check(state: &mut BlockState) -> Option<()> {
        match block_math(state, true) {
            Found::Block(..) => Some(()),
            _ => None,
        }
    }

    fn run(state: &mut BlockState) -> Option<(Node, usize)> {
        match block_math(state, false) {
            Found::Block(content, lines) => Some((Node::new(MathBlock { content }), lines)),
            Found::Refused | Found::Nothing => None,
        }
    }
}

pub fn add(md: &mut MarkdownIt) {
    // `math-paren` before `text`, so ahead of the escape that would take
    // `\(`; `math` and `asciimath` after `escape`.
    md.inline
        .add_rule::<InlineMathParen>()
        .before::<EscapeScanner>();
    md.inline.add_rule::<AsciiMath>().after::<EscapeScanner>();
    md.inline.add_rule::<InlineMath>().after::<EscapeScanner>();
    md.block.add_rule::<BlockMath>().after::<CodeScanner>();
}

pub fn allow(list: &mut AllowList) {
    list.allow(&["span.math", "span.asciimath", "div.math"]);
}
