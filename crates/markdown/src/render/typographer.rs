//! features/custom-typographer-replacements.js, which takes the place of
//! markdown-it's `replacements` rule: (tm) and (pa), plus-minus, arrows,
//! ellipses, dashes. (c), (r) and (p) are deliberately left alone.
//!
//! The JS tests its global `SCOPED_ABBR_RE` with `.test()`, which carries
//! `lastIndex` from one block to the next, so whether a block is scanned
//! can depend on what was cooked before it. That is not reproduced: every
//! text is scanned from its start.

use markdown_it::Node;
use markdown_it::parser::inline::Text;
use markdown_it::plugins::cmark::inline::image::Image;

use regex::Regex;
use std::sync::LazyLock as Lazy;

static SCOPED: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\((tm|pa)\)").unwrap());
static RARE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\+-|\.\.\.|\?\?\?\?|!!!!|,,|--|-->|<--|->|<-|<->|<-->").unwrap());
static RIGHT_ARROW: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)(^|\s)-{1,2}>(\s|$)").unwrap());
static LEFT_ARROW: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)(^|\s)<-{1,2}(\s|$)").unwrap());
static BOTH_ARROW: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)(^|\s)<-{1,2}>(\s|$)").unwrap());
static ELLIPSIS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\.{3,}").unwrap());
static MARK_ELLIPSIS: Lazy<Regex> = Lazy::new(|| Regex::new(r"([?!])…").unwrap());
static MARKS: Lazy<Regex> = Lazy::new(|| Regex::new(r"([?!]){4,}").unwrap());
static COMMAS: Lazy<Regex> = Lazy::new(|| Regex::new(r",{2,}").unwrap());

/// `str.replace(/(pre)DASHES(?=post|$)/gm, "$1" + with)`: the dashes at a
/// position whose previous character passes `pre` (or that starts a line)
/// and whose next passes `post` (or that ends one). The previous character
/// is part of the match, so it cannot belong to the match before.
fn replace_dashes(
    text: &str,
    dashes: &str,
    with: char,
    pre: impl Fn(char) -> bool,
    post: impl Fn(char) -> bool,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_end = 0;
    let mut pos = 0;
    while pos < text.len() {
        let rest = &text[pos..];
        let c = rest.chars().next().unwrap();
        if let Some(after) = rest.strip_prefix(dashes) {
            let previous = text[..pos].chars().next_back();
            let next = after.chars().next();
            let line_start = previous.is_none_or(|p| p == '\n');
            let pre_ok =
                line_start || previous.is_some_and(|p| pre(p) && pos - p.len_utf8() >= last_end);
            let post_ok = next.is_none_or(|n| n == '\n' || post(n));
            if pre_ok && post_ok {
                out.push(with);
                pos += dashes.len();
                last_end = pos;
                continue;
            }
        }
        out.push(c);
        pos += c.len_utf8();
    }
    out
}

/// `replaceRareStr`
fn replace_rare(text: &str) -> String {
    let text = text.replace("+-", "±");
    let text = RIGHT_ARROW.replace_all(&text, " \u{2192} ");
    let text = LEFT_ARROW.replace_all(&text, " \u{2190} ");
    let text = BOTH_ARROW.replace_all(&text, " \u{2194} ");
    let text = ELLIPSIS.replace_all(&text, "…");
    let text = MARK_ELLIPSIS.replace_all(&text, "$1..");
    let text = MARKS.replace_all(&text, "$1$1$1");
    let text = COMMAS.replace_all(&text, ",");
    // em-dash, then the two en-dash forms.
    let text = replace_dashes(&text, "---", '\u{2014}', |p| p != '-', |n| n != '-');
    let text = replace_dashes(
        &text,
        "--",
        '\u{2013}',
        char::is_whitespace,
        char::is_whitespace,
    );
    replace_dashes(
        &text,
        "--",
        '\u{2013}',
        |p| p != '-' && !p.is_whitespace(),
        |n| n != '-' && !n.is_whitespace(),
    )
}

/// `replaceScopedStr`
fn replace_scoped(text: &str) -> String {
    SCOPED
        .replace_all(text, |caps: &regex::Captures| {
            if caps[1].eq_ignore_ascii_case("tm") {
                "™"
            } else {
                "¶"
            }
        })
        .into_owned()
}

/// The `replacements` core rule, over every text outside an autolink. An
/// image's alt text is the image token's own children in JS, which it
/// does not visit.
pub fn apply(root: &mut Node) {
    fn visit(node: &mut Node, in_autolink: bool) {
        if node.is::<Image>() {
            return;
        }
        if let Some(text) = node.cast_mut::<Text>() {
            if SCOPED.is_match(&text.content) {
                text.content = replace_scoped(&text.content);
            }
            if !in_autolink && RARE.is_match(&text.content) {
                text.content = replace_rare(&text.content);
            }
            return;
        }
        let in_autolink = in_autolink || super::linkify::is_auto_link(node);
        for child in node.children.iter_mut() {
            visit(child, in_autolink);
        }
    }
    visit(root, false);
}

/// markdown-it's own `replacements` rule, which chat's engine enables in
/// place of Discourse's: (c), (r) and (tm), and the rare replacements
/// without arrows, an ellipsis from two dots on.
pub fn markdown_it_apply(root: &mut Node) {
    static RARE_MD: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"\+-|\.\.|\?\?\?\?|!!!!|,,|--").unwrap());
    static SCOPED_MD: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\((c|tm|r)\)").unwrap());
    static DOTS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\.{2,}").unwrap());
    fn rare(text: &str) -> String {
        let text = text.replace("+-", "±");
        let text = DOTS.replace_all(&text, "…");
        let text = MARK_ELLIPSIS.replace_all(&text, "$1..");
        let text = MARKS.replace_all(&text, "$1$1$1");
        let text = COMMAS.replace_all(&text, ",");
        let text = replace_dashes(&text, "---", '\u{2014}', |p| p != '-', |n| n != '-');
        let text = replace_dashes(
            &text,
            "--",
            '\u{2013}',
            char::is_whitespace,
            char::is_whitespace,
        );
        replace_dashes(
            &text,
            "--",
            '\u{2013}',
            |p| p != '-' && !p.is_whitespace(),
            |n| n != '-' && !n.is_whitespace(),
        )
    }
    fn visit(node: &mut Node, in_autolink: bool) {
        if node.is::<Image>() {
            return;
        }
        if let Some(text) = node.cast_mut::<Text>() {
            if in_autolink {
                return;
            }
            if SCOPED_MD.is_match(&text.content) {
                text.content = SCOPED_MD
                    .replace_all(&text.content, |caps: &regex::Captures| {
                        match caps[1].to_ascii_lowercase().as_str() {
                            "c" => "©",
                            "r" => "®",
                            _ => "™",
                        }
                    })
                    .into_owned();
            }
            if RARE_MD.is_match(&text.content) {
                text.content = rare(&text.content);
            }
            return;
        }
        let in_autolink = in_autolink || super::linkify::is_auto_link(node);
        for child in node.children.iter_mut() {
            visit(child, in_autolink);
        }
    }
    visit(root, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashes_arrows_and_marks() {
        assert_eq!(replace_rare("a -- b --- c"), "a – b — c");
        assert_eq!(replace_rare("a--b and a---b"), "a–b and a—b");
        assert_eq!(replace_rare("----"), "----");
        assert_eq!(replace_rare("a -> b <- c <-> d"), "a → b ← c ↔ d");
        assert_eq!(
            replace_rare("wait... what?.... no!!!!!"),
            "wait… what?.. no!!!"
        );
        assert_eq!(replace_rare("1 +- 2,, 3"), "1 ± 2, 3");
        assert_eq!(replace_scoped("(c) (TM) (pa) (r)"), "(c) ™ ¶ (r)");
    }
}
