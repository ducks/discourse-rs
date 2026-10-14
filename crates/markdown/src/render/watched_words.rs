//! features/censored.js and features/watched-words.js: censored words
//! become ■ in every token's content (`censoredRegexp`), and replace and
//! link words (`watchedWordsReplace`, `watchedWordsLink`) are swapped for
//! their replacement or wrapped in a link in the text of inline content.
//! Also the emoji rule's use of the replace words (`emoji_replace`).
//!
//! The regexps are the server's sources, run as JavaScript runs them with
//! the `u` flag (`gu`, `gui`): by code point, simple case folding. The
//! server refuses words whose case folding JavaScript and Ruby disagree on.
//! Admin regexps (`watched_words_regular_expressions`) go through
//! crate::regexp first, as JavaScript reads them.

use fancy_regex::{Regex, RegexBuilder};
use markdown_it::parser::inline::Text;
use markdown_it::plugins::cmark::block::code::CodeBlock;
use markdown_it::plugins::cmark::block::fence::CodeFence;
use markdown_it::plugins::cmark::inline::image::Image;
use markdown_it::plugins::cmark::inline::link::Link;
use markdown_it::plugins::html::html_block::HtmlBlock;
use markdown_it::plugins::html::html_inline::HtmlInline;
use markdown_it::{MarkdownIt, Node};

use super::RenderSettings;
use super::element::{BlockText, CodeText, Element, RawHtml};
use super::linkify::Linkified;

/// A replace or link word: `regexps_for_action`'s key and attributes.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct WatchedWord {
    pub regexp: String,
    pub replacement: String,
    pub case_sensitive: bool,
    pub html: bool,
    /// `watched_words_regular_expressions`: the word is an admin regexp,
    /// read as JavaScript reads it (crate::regexp).
    #[serde(default)]
    pub regex: bool,
}

/// One of `censoredRegexp`: the source and whether it is case-sensitive.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Censored {
    pub regexp: String,
    pub case_sensitive: bool,
    #[serde(default)]
    pub regex: bool,
}

fn compile(source: &str, case_sensitive: bool, regex: bool) -> Result<Regex, &'static str> {
    if regex {
        let t =
            crate::regexp::translate(source, crate::regexp::Dialect::JsUnicode, case_sensitive)?;
        return Regex::new(&t.source)
            .map_err(|_| "a watched word regexp the renderer can't compile");
    }
    // An inline flag: fancy_regex's builder flag misses the parts its own
    // VM runs (those around lookarounds).
    let source = if case_sensitive {
        source.to_string()
    } else {
        format!("(?i){source}")
    };
    Regex::new(&source).map_err(|_| "a watched word regexp the renderer can't compile")
}

/// `censorFn(list, "■")` over a text: each match's first group becomes ■
/// per UTF-16 unit. Without a group taking part (an admin regexp without
/// one), `groupMatches.find` lands on the replaced string itself, so the
/// match is censored only when it is that whole string.
fn censor_text(text: &str, regexps: &[Regex]) -> Result<String, &'static str> {
    let mut text = text.to_string();
    for re in regexps {
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for caps in re.captures_iter(&text) {
            let caps = caps.map_err(|_| "watched word regexp backtracking limit")?;
            let full = caps.get(0).expect("match");
            out.push_str(&text[last..full.start()]);
            let g = (1..caps.len())
                .find_map(|i| caps.get(i))
                .map_or(text.as_str(), |g| g.as_str());
            let bars = "\u{25A0}".repeat(g.encode_utf16().count());
            out.push_str(&full.as_str().replacen(g, &bars, 1));
            last = full.end();
        }
        out.push_str(&text[last..]);
        text = out;
    }
    Ok(text)
}

/// The `censored` core rule: every token's content.
pub fn censor(root: &mut Node, settings: &RenderSettings) -> Result<(), &'static str> {
    if settings.censored.is_empty() {
        return Ok(());
    }
    let regexps = settings
        .censored
        .iter()
        .map(|c| compile(&c.regexp, c.case_sensitive, c.regex))
        .collect::<Result<Vec<_>, _>>()?;
    let mut failed = None;
    root.walk_mut(|node, _| {
        if failed.is_some() {
            return;
        }
        let content: Option<&mut String> = if let Some(t) = node.cast_mut::<Text>() {
            Some(&mut t.content)
        } else if let Some(t) = node.cast_mut::<CodeText>() {
            Some(&mut t.0)
        } else if let Some(t) = node.cast_mut::<BlockText>() {
            Some(&mut t.0)
        } else if let Some(t) = node.cast_mut::<RawHtml>() {
            Some(&mut t.0)
        } else if let Some(t) = node.cast_mut::<HtmlInline>() {
            Some(&mut t.content)
        } else if let Some(t) = node.cast_mut::<HtmlBlock>() {
            Some(&mut t.content)
        } else if let Some(t) = node.cast_mut::<CodeFence>() {
            Some(&mut t.content)
        } else if let Some(t) = node.cast_mut::<CodeBlock>() {
            Some(&mut t.content)
        } else {
            None
        };
        if let Some(content) = content
            && !content.is_empty()
        {
            match censor_text(content, &regexps) {
                Ok(censored) => *content = censored,
                Err(e) => failed = Some(e),
            }
        }
    });
    match failed {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

struct Matcher {
    pattern: Regex,
    replacement: String,
    link: bool,
    html: bool,
}

struct Found {
    /// Byte offsets in the text (ordered as JavaScript's UTF-16 ones).
    start: usize,
    end: usize,
    replacement: String,
    link: bool,
    html: bool,
}

const MAX_MATCHES: usize = 100;

/// `findAllMatches`: each matcher's matches (their first group), up to
/// 100 in all, sorted by position (stable).
fn find_all(text: &str, matchers: &[Matcher]) -> Result<Vec<Found>, &'static str> {
    let mut found = Vec::new();
    'outer: for m in matchers {
        if found.len() >= MAX_MATCHES {
            break;
        }
        for caps in m.pattern.captures_iter(text) {
            let caps = caps.map_err(|_| "watched word regexp backtracking limit")?;
            let Some(g) = caps.get(1) else {
                continue;
            };
            found.push(Found {
                start: g.start(),
                end: g.end(),
                replacement: m.replacement.clone(),
                link: m.link,
                html: m.html,
            });
            if found.len() >= MAX_MATCHES {
                break 'outer;
            }
        }
    }
    found.sort_by_key(|f| f.start);
    Ok(found)
}

/// `/^<a[>\s]/i`
fn is_link_open(html: &str) -> bool {
    let b = html.as_bytes();
    b.len() >= 3
        && b[..2].eq_ignore_ascii_case(b"<a")
        && html[2..]
            .chars()
            .next()
            .is_some_and(|c| c == '>' || c.is_whitespace())
}

/// `/^<\/a\s*>/i`
fn is_link_close(html: &str) -> bool {
    let b = html.as_bytes();
    if b.len() < 3 || !b[..3].eq_ignore_ascii_case(b"</a") {
        return false;
    }
    html[3..].trim_start().starts_with('>')
}

/// Text the rule leaves alone: markdown and automatic links (their
/// content is skipped), mentions and hashtags, images, inline code.
fn skipped(node: &Node) -> bool {
    if node.is::<Link>() || node.is::<Linkified>() || node.is::<Image>() {
        return true;
    }
    if super::linkify::is_auto_link(node) {
        return true;
    }
    if let Some(e) = node.cast::<Element>() {
        if e.tag == "a" {
            return true;
        }
        let class = e
            .attrs
            .iter()
            .find(|(k, _)| k == "class")
            .map(|(_, v)| v.as_str());
        if e.tag == "span" && matches!(class, Some("mention" | "hashtag-raw" | "hashtag")) {
            return true;
        }
    }
    false
}

fn is_inline_block(node: &Node) -> bool {
    use markdown_it::plugins::cmark::block::heading::ATXHeading;
    use markdown_it::plugins::cmark::block::lheading::SetextHeader;
    use markdown_it::plugins::cmark::block::paragraph::Paragraph;
    node.is::<Paragraph>() || node.is::<ATXHeading>() || node.is::<SetextHeader>()
}

/// The `watched-words` core rule.
pub fn apply(
    root: &mut Node,
    settings: &RenderSettings,
    md: &MarkdownIt,
) -> Result<(), &'static str> {
    if settings.watched_replace.is_empty() && settings.watched_link.is_empty() {
        return Ok(());
    }
    let mut matchers = Vec::new();
    for (words, link) in [
        (&settings.watched_replace, false),
        (&settings.watched_link, true),
    ] {
        for w in words {
            matchers.push(Matcher {
                pattern: compile(&w.regexp, w.case_sensitive, w.regex)?,
                replacement: w.replacement.clone(),
                link,
                html: !link && w.html,
            });
        }
    }
    fn visit(
        node: &mut Node,
        matchers: &[Matcher],
        md: &MarkdownIt,
        html_level: &mut i32,
    ) -> Result<(), &'static str> {
        let mut level = 0;
        let html_level = if is_inline_block(node) {
            &mut level
        } else {
            html_level
        };
        // From the end, as the rule does: an html `</a>` raises the level,
        // an `<a ...>` lowers it again.
        let mut i = node.children.len();
        while i > 0 {
            i -= 1;
            if skipped(&node.children[i]) {
                continue;
            }
            if let Some(html) = node.children[i].cast::<HtmlInline>() {
                if is_link_open(&html.content) && *html_level > 0 {
                    *html_level -= 1;
                }
                if is_link_close(&html.content) {
                    *html_level += 1;
                }
                continue;
            }
            let Some(text) = node.children[i].cast::<Text>() else {
                visit(&mut node.children[i], matchers, md, html_level)?;
                continue;
            };
            let text = text.content.clone();
            let found = find_all(&text, matchers)?;
            if found.is_empty() {
                continue;
            }
            let mut nodes = Vec::new();
            let mut last = 0;
            for f in found {
                if f.start < last {
                    continue;
                }
                if f.start > last {
                    nodes.push(Node::new(Text {
                        content: text[last..f.start].to_string(),
                    }));
                }
                if f.link {
                    let url = md.link_formatter.normalize_link(&f.replacement);
                    if *html_level == 0 && md.link_formatter.validate_link(&url).is_some() {
                        let mut link = Node::new(Linkified {
                            url,
                            class: None,
                            target_blank: false,
                        });
                        link.children.push(Node::new(Text {
                            content: text[f.start..f.end].to_string(),
                        }));
                        nodes.push(link);
                    }
                } else if f.html {
                    nodes.push(Node::new(HtmlInline {
                        content: f.replacement.clone(),
                    }));
                } else {
                    nodes.push(Node::new(Text {
                        content: f.replacement.clone(),
                    }));
                }
                last = f.end;
            }
            if last < text.len() {
                nodes.push(Node::new(Text {
                    content: text[last..].to_string(),
                }));
            }
            node.children.splice(i..=i, nodes);
        }
        Ok(())
    }
    visit(root, &matchers, md, &mut 0)
}

/// The emoji rule's pass over the replace words (applyEmoji): each key
/// tried as a JavaScript regexp without flags (so without `u`: `\p{L}` in
/// a class is the characters `p{L}`, and without `i`), every match's
/// first group then replaced at its first occurrence in the text, the
/// regexp's `lastIndex` carried over the changing text. None when the
/// text holds characters past the BMP (JavaScript would match their
/// halves) or a replacement uses `$` patterns.
pub fn emoji_replace(content: &str, words: &[WatchedWord]) -> Result<String, &'static str> {
    let mut content = content.to_string();
    for w in words {
        let source = if w.regex {
            crate::regexp::translate(&w.regexp, crate::regexp::Dialect::JsLegacy, true)?.source
        } else {
            w.regexp
                .replace(r"[\p{L}\p{M}\p{N}\p{Pc}]", r"[p\{L\}MNPc]")
        };
        let re = RegexBuilder::new(&source)
            .build()
            .map_err(|_| "a watched word regexp the renderer can't compile")?;
        let count = re.find_iter(&content).count();
        if count == 0 {
            continue;
        }
        if content.chars().any(|c| c as u32 > 0xFFFF) {
            return Err("watched replace words in text with astral characters next to emoji");
        }
        if w.replacement.contains('$') {
            return Err("a watched replace word with `$` next to emoji");
        }
        let mut last_index = 0usize;
        for _ in 0..count {
            let byte = content.char_indices().nth(last_index).map(|(b, _)| b);
            let byte = match byte {
                Some(b) => Some(b),
                None if last_index == content.chars().count() => Some(content.len()),
                None => None,
            };
            let caps = match byte {
                Some(b) => re
                    .captures_from_pos(&content, b)
                    .map_err(|_| "watched word regexp backtracking limit")?,
                None => None,
            };
            match caps {
                Some(caps) => {
                    let full = caps.get(0).expect("match");
                    last_index = content[..full.end()].chars().count();
                    if let Some(g) = caps.get(1) {
                        let g = g.as_str().to_string();
                        content = content.replacen(&g, &w.replacement, 1);
                    }
                }
                None => last_index = 0,
            }
        }
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAT: &str = r"(?:(?<![\p{L}\p{M}\p{N}\p{Pc}])|(?<=[\u4E00-\u9FFF\u3400-\u4DBF\u3040-\u309F\u30A0-\u30FF\uAC00-\uD7AF\u0E00-\u0E7F\u0E80-\u0EFF\u1000-\u109F\u1780-\u17FF\u0F00-\u0FFF])|(?=[\u4E00-\u9FFF\u3400-\u4DBF\u3040-\u309F\u30A0-\u30FF\uAC00-\uD7AF\u0E00-\u0E7F\u0E80-\u0EFF\u1000-\u109F\u1780-\u17FF\u0F00-\u0FFF]))(cat)(?:(?![\p{L}\p{M}\p{N}\p{Pc}])|(?=[\u4E00-\u9FFF\u3400-\u4DBF\u3040-\u309F\u30A0-\u30FF\uAC00-\uD7AF\u0E00-\u0E7F\u0E80-\u0EFF\u1000-\u109F\u1780-\u17FF\u0F00-\u0FFF])|(?<=[\u4E00-\u9FFF\u3400-\u4DBF\u3040-\u309F\u30A0-\u30FF\uAC00-\uD7AF\u0E00-\u0E7F\u0E80-\u0EFF\u1000-\u109F\u1780-\u17FF\u0F00-\u0FFF]))";

    #[test]
    fn emoji_quirk_matches_inside_words() {
        let w = WatchedWord {
            regexp: CAT.into(),
            replacement: "dog".into(),
            case_sensitive: false,
            html: false,
            regex: false,
        };
        assert_eq!(
            emoji_replace("concatenate :smile:", std::slice::from_ref(&w)).unwrap(),
            "condogenate :smile:"
        );
        assert_eq!(
            emoji_replace(":cat: cat:smile:", &[w]).unwrap(),
            ":dog: dog:smile:"
        );
    }

    #[test]
    fn censor_counts_utf16_units() {
        let re = compile(&CAT.replace("(cat)", r"(shit\S*)"), false, false).unwrap();
        assert_eq!(
            censor_text("shit\u{1D482}", &[re]).unwrap(),
            "\u{25A0}".repeat(6)
        );
    }

    #[test]
    fn regex_censor_without_groups_only_takes_whole_strings() {
        let re = compile(r"secret\d+", false, true).unwrap();
        let re = std::slice::from_ref(&re);
        assert_eq!(censor_text("a secret42 b", re).unwrap(), "a secret42 b");
        assert_eq!(censor_text("SECRET42", re).unwrap(), "\u{25A0}".repeat(8));
        let re = compile(r"darn(it)?", false, true).unwrap();
        assert_eq!(
            censor_text("darn darnit", &[re]).unwrap(),
            "darn darn\u{25A0}\u{25A0}"
        );
    }

    #[test]
    fn regex_emoji_quirk() {
        let w = WatchedWord {
            regexp: "(cat)".into(),
            replacement: "dog".into(),
            case_sensitive: false,
            html: false,
            regex: true,
        };
        assert_eq!(
            emoji_replace("concatenate :smile:", &[w]).unwrap(),
            "condogenate :smile:"
        );
    }
}
