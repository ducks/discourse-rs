//! `Search::GroupedSearchResults.blurb_for` for the indexed-text path
//! (`post_search_data.raw_data`, scrub: false): Rails' `TextHelper.excerpt`
//! around the term, else `TextHelper.truncate`, then `Sanitize.clean`.

/// `GroupedSearchResults::OMISSION`
const OMISSION: &str = "...";

/// `blurb_for(cooked:, term:, blurb_length:, scrub: false)`
pub fn blurb_for(text: &str, term: Option<&str>, blurb_length: usize) -> String {
    let mut blurb = None;
    if let Some(term) = term {
        // The first quoted phrase stands in for the whole term.
        let term = match quoted_phrase(term) {
            Some(p) => p,
            None => term.to_string(),
        };
        blurb = excerpt(text, &term, blurb_length / 2);
    }

    match blurb.filter(|b| !b.trim().is_empty()) {
        Some(b) => sanitize(&b),
        None => sanitize(&html_escape(&truncate(text, blurb_length))),
    }
}

fn quoted_phrase(term: &str) -> Option<String> {
    let start = term.find('"')?;
    let rest = &term[start + 1..];
    let end = rest.find('"')?;
    let inner = &rest[..end];
    (!inner.is_empty()).then(|| inner.to_string())
}

/// `ActionView::Helpers::TextHelper.excerpt(text, phrase, radius:)`: the
/// phrase (matched case-insensitively, kept as found) with `radius` chars
/// either side, "..." marking cut ends, the middle stripped.
fn excerpt(text: &str, phrase: &str, radius: usize) -> Option<String> {
    if phrase.is_empty() {
        return None;
    }
    let re = regex::RegexBuilder::new(&regex::escape(phrase))
        .case_insensitive(true)
        .build()
        .ok()?;
    let m = re.find(text)?;
    let first_part: Vec<char> = text[..m.start()].chars().collect();
    let second_part: Vec<char> = text[m.end()..].chars().collect();
    let prefix = if first_part.len() > radius {
        OMISSION
    } else {
        ""
    };
    let postfix = if second_part.len() > radius {
        OMISSION
    } else {
        ""
    };
    let head: String = first_part[first_part.len().saturating_sub(radius)..]
        .iter()
        .collect();
    let tail: String = second_part.iter().take(radius).collect();
    let affix = format!("{head}{}{tail}", m.as_str());
    // Ruby String#strip: ASCII whitespace and NUL at both ends.
    let affix = affix.trim_matches(|c: char| c.is_ascii_whitespace() || c == '\0');
    Some(format!("{prefix}{affix}{postfix}"))
}

/// Ruby `String#truncate(length)` with the default "..." omission.
fn truncate(text: &str, length: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= length {
        return text.to_string();
    }
    let mut out: String = chars[..length.saturating_sub(OMISSION.len())]
        .iter()
        .collect();
    out.push_str(OMISSION);
    out
}

/// `ERB::Util.html_escape`
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Elements Sanitize drops together with their contents.
const REMOVE_CONTENTS: &[&str] = &[
    "iframe",
    "math",
    "noembed",
    "noframes",
    "noscript",
    "plaintext",
    "script",
    "style",
    "svg",
    "xmp",
];

/// Elements Sanitize pads with a space on each side.
const WHITESPACE_ELEMENTS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "br",
    "dd",
    "div",
    "dl",
    "dt",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "li",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "ul",
];

/// `Sanitize.clean(html)` with the default config: control characters
/// dropped, the text parsed as an HTML fragment, every element removed
/// (some with their contents, block ones padded with spaces), entities
/// decoded, and the text re-escaped. The indexed text is plain prose, so
/// the tokenizer here covers what HTML5 does with the odd literal tag or
/// entity in it rather than a full parser.
fn sanitize(input: &str) -> String {
    let cleaned: String = input
        .chars()
        .filter(|&c| {
            !matches!(c, '\u{1}'..='\u{8}' | '\u{b}' | '\u{e}'..='\u{1f}' | '\u{7f}')
                && !is_noncharacter(c)
        })
        .collect();
    let mut text = String::with_capacity(cleaned.len());
    let bytes = cleaned.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let rest = &cleaned[i..];
            if let Some((tag, end)) = parse_tag(rest) {
                i += end;
                if tag.closing {
                    if WHITESPACE_ELEMENTS.contains(&tag.name.as_str()) {
                        text.push(' ');
                    }
                    continue;
                }
                if WHITESPACE_ELEMENTS.contains(&tag.name.as_str()) {
                    text.push(' ');
                }
                if REMOVE_CONTENTS.contains(&tag.name.as_str()) {
                    // Skip to the matching end tag, or the end of input.
                    let close = format!("</{}", tag.name);
                    let lower = cleaned[i..].to_ascii_lowercase();
                    match lower.find(&close) {
                        Some(pos) => {
                            let after = i + pos;
                            let gt = cleaned[after..].find('>').map(|g| after + g + 1);
                            i = gt.unwrap_or(cleaned.len());
                        }
                        None => i = cleaned.len(),
                    }
                }
                continue;
            }
            if rest.starts_with("<!--") {
                i += match rest.find("-->") {
                    Some(pos) => pos + 3,
                    None => rest.len(),
                };
                continue;
            }
            if rest.starts_with("<!") || rest.starts_with("<?") || rest.starts_with("</") {
                // Bogus comment: everything up to the next '>'.
                i += match rest.find('>') {
                    Some(pos) => pos + 1,
                    None => rest.len(),
                };
                continue;
            }
        }
        let c = cleaned[i..].chars().next().unwrap();
        text.push(c);
        i += c.len_utf8();
    }
    let decoded = html_escape::decode_html_entities(&text);
    let mut out = String::with_capacity(decoded.len());
    for c in decoded.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            _ => out.push(c),
        }
    }
    out
}

struct Tag {
    name: String,
    closing: bool,
}

/// A start or end tag at the head of `s` (HTML5 tokenizer rules: `<` must
/// be followed by a letter, or `/` and a letter), with its byte length.
fn parse_tag(s: &str) -> Option<(Tag, usize)> {
    let body = s.strip_prefix('<')?;
    let (closing, body) = match body.strip_prefix('/') {
        Some(b) => (true, b),
        None => (false, body),
    };
    if !body.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let name_len = body
        .find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
        .unwrap_or(body.len());
    let name = body[..name_len].to_ascii_lowercase();
    let end = match body.find('>') {
        Some(pos) => pos + 1,
        None => body.len(),
    };
    let consumed = 1 + usize::from(closing) + end;
    Some((Tag { name, closing }, consumed))
}

fn is_noncharacter(c: char) -> bool {
    let u = c as u32;
    (0xFDD0..=0xFDEF).contains(&u) || (u & 0xFFFE) == 0xFFFE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long() -> String {
        "lorem ipsum dolor sit amet ".repeat(30).trim().to_string()
    }

    #[test]
    fn short_text_passes_through() {
        assert_eq!(
            blurb_for("short text here", Some("zzz"), 200),
            "short text here"
        );
        assert_eq!(blurb_for("hello", None, 200), "hello");
    }

    #[test]
    fn excerpts_around_the_term() {
        let text = format!("{} TARGET {}", long(), long());
        let out = blurb_for(&text, Some("target"), 200);
        assert_eq!(
            out,
            "...sum dolor sit amet lorem ipsum dolor sit amet lorem ipsum dolor sit amet lorem ipsum dolor sit amet TARGET lorem ipsum dolor sit amet lorem ipsum dolor sit amet lorem ipsum dolor sit amet lorem ipsum dolor..."
        );
        assert_eq!(
            blurb_for(&format!("target {}", long()), Some("target"), 200),
            "target lorem ipsum dolor sit amet lorem ipsum dolor sit amet lorem ipsum dolor sit amet lorem ipsum dolor..."
        );
        assert_eq!(
            blurb_for(&format!("{} target", long()), Some("target"), 200),
            "...sum dolor sit amet lorem ipsum dolor sit amet lorem ipsum dolor sit amet lorem ipsum dolor sit amet target"
        );
        assert_eq!(
            blurb_for("aa bb cc target dd ee", Some("\"target dd\""), 200),
            "aa bb cc target dd ee"
        );
        assert_eq!(
            blurb_for("Foo TARGET bar", Some("target"), 200),
            "Foo TARGET bar"
        );
        assert_eq!(blurb_for("what a.b c+d", Some("a.b"), 200), "what a.b c+d");
        assert_eq!(
            blurb_for("say \"target\" here", Some("target"), 200),
            "say \"target\" here"
        );
        assert_eq!(blurb_for("    target", Some("target"), 200), "target");
        let text = format!("héllo {} tärget {}", long(), long());
        assert_eq!(
            blurb_for(&text, Some("tärget"), 40),
            "...psum dolor sit amet tärget lorem ipsum dolor s..."
        );
    }

    #[test]
    fn truncates_when_the_term_is_absent() {
        let text = format!("hello there world {}", long());
        assert_eq!(
            blurb_for(&text, Some("hello world"), 100),
            "hello there world lorem ipsum dolor sit amet lorem ipsum dolor sit amet lorem ipsum dolor sit ame..."
        );
        assert_eq!(blurb_for(&"a".repeat(60), Some("zzz"), 60), "a".repeat(60));
        assert_eq!(
            blurb_for(&"a".repeat(61), Some("zzz"), 60),
            format!("{}...", "a".repeat(57))
        );
        assert_eq!(
            blurb_for("hello  world", Some("hello world"), 200),
            "hello  world"
        );
    }

    #[test]
    fn sanitizes_markup_and_entities() {
        assert_eq!(
            blurb_for("x <b>bold</b> & 'q' \"dq\" target", Some("target"), 200),
            "x bold &amp; 'q' \"dq\" target"
        );
        let text = format!("x <b>bold</b> & 'q' \"dq\" {}", long());
        assert_eq!(
            blurb_for(&text, Some("zzz"), 60),
            "x &lt;b&gt;bold&lt;/b&gt; &amp; 'q' \"dq\" lorem ipsum dolor sit amet lorem..."
        );
        assert_eq!(
            blurb_for("a\u{a0}b target", Some("target"), 200),
            "a&nbsp;b target"
        );
        assert_eq!(
            blurb_for(
                "1 < 2 and 3 > 2 target <script>x</script>",
                Some("target"),
                200
            ),
            "1 &lt; 2 and 3 &gt; 2 target "
        );
        assert_eq!(
            blurb_for("AT&T and &copy; target", Some("target"), 200),
            "AT&amp;T and © target"
        );
    }
}
