//! HasSanitizableFields#sanitize_field: Rails' HTML4 safe list sanitizer
//! (Loofah on libxml2), then `CGI.unescape_html`, then `%7B`/`%7D` back to
//! braces.
//!
//! libxml2 rewrites markup it has to repair (unclosed or misnested
//! elements, block elements inside inline ones, lists, tables) in ways
//! that are not ported, so this takes the fragments whose result follows
//! from the markup alone: text with the XML entities and numeric
//! character references, and properly nested inline elements of the safe
//! list (plus `br` and `img`) with quoted attributes and plain URLs.
//! Everything else is refused.

use crate::Unsupported;

/// `Rails::HTML4::SafeListSanitizer.allowed_tags`, the inline ones and
/// the void ones the port draws.
const INLINE_TAGS: [&str; 22] = [
    "a", "abbr", "acronym", "b", "big", "cite", "code", "del", "dfn", "em", "i", "ins", "kbd",
    "mark", "samp", "small", "span", "strong", "sub", "sup", "time", "tt",
];
const MORE_INLINE: [&str; 1] = ["var"];
const VOID_TAGS: [&str; 2] = ["br", "img"];

/// `Rails::HTML4::SafeListSanitizer.allowed_attributes`
const ALLOWED_ATTRIBUTES: [&str; 13] = [
    "abbr", "alt", "cite", "class", "datetime", "height", "href", "lang", "name", "src", "title",
    "width", "xml:lang",
];

fn refuse(what: &'static str) -> Unsupported {
    Unsupported(what)
}

/// A numeric character reference's character.
fn numeric(code: Option<u32>) -> Result<char, Unsupported> {
    match code {
        // libxml2 maps NUL, 0x80..0x9F and the no-break space its own way.
        Some(c) if c != 0 && !(0x80..=0xa0).contains(&c) => {
            char::from_u32(c).ok_or(refuse("sanitizing invalid character references"))
        }
        _ => Err(refuse("sanitizing invalid character references")),
    }
}

/// Text up to the next tag with its references as libxml2's HTML4 parser
/// reads them: numeric ones decoded with or without their `;`, the XML
/// entities decoded with theirs, a name without `;` (`AT&T`, `&amp=2`)
/// left as it is. Other named references are refused (no entity table).
fn decode_text(text: &str) -> Result<String, Unsupported> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(num) = after.strip_prefix('#') {
            let (hex, digits_from) = match num.strip_prefix(['x', 'X']) {
                Some(h) => (true, h),
                None => (false, num),
            };
            let len = digits_from
                .find(|c: char| {
                    !(if hex {
                        c.is_ascii_hexdigit()
                    } else {
                        c.is_ascii_digit()
                    })
                })
                .unwrap_or(digits_from.len());
            if len == 0 {
                return Err(refuse("sanitizing malformed character references"));
            }
            let digits = &digits_from[..len];
            let code = u32::from_str_radix(digits, if hex { 16 } else { 10 }).ok();
            out.push(numeric(code)?);
            let tail = &digits_from[len..];
            rest = tail.strip_prefix(';').unwrap_or(tail);
            continue;
        }
        let len = after
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(after.len());
        let name = &after[..len];
        if name.is_empty() || !after[len..].starts_with(';') {
            out.push('&');
            rest = after;
            continue;
        }
        let c = match name {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            _ => return Err(refuse("sanitizing named character references")),
        };
        out.push(c);
        rest = &after[len + 1..];
    }
    out.push_str(rest);
    if out.contains('\u{a0}') {
        return Err(refuse("sanitizing no-break spaces"));
    }
    Ok(out)
}

/// A URL Loofah keeps and libxml2 writes back unchanged: relative,
/// fragment, or http(s)/mailto, of URI characters only.
fn plain_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let scheme_ok = url.starts_with('/')
        || url.starts_with('#')
        || lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:");
    scheme_ok
        && url
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(c))
}

/// A start tag's name and attributes (`name="value"` or `'value'`).
struct Tag {
    name: String,
    attributes: Vec<(String, String)>,
    self_closing: bool,
}

fn parse_tag(inner: &str) -> Result<Tag, Unsupported> {
    let mut chars = inner.char_indices().peekable();
    let mut name = String::new();
    while let Some(&(_, c)) = chars.peek() {
        if c.is_ascii_alphanumeric() {
            name.push(c.to_ascii_lowercase());
            chars.next();
        } else {
            break;
        }
    }
    let mut attributes: Vec<(String, String)> = Vec::new();
    let mut self_closing = false;
    loop {
        while chars.peek().is_some_and(|(_, c)| c.is_ascii_whitespace()) {
            chars.next();
        }
        let Some(&(_, c)) = chars.peek() else {
            break;
        };
        if c == '/' {
            chars.next();
            while chars.peek().is_some_and(|(_, c)| c.is_ascii_whitespace()) {
                chars.next();
            }
            if chars.peek().is_some() {
                return Err(refuse("sanitizing malformed tags"));
            }
            self_closing = true;
            break;
        }
        let mut attr = String::new();
        while let Some(&(_, c)) = chars.peek() {
            if c.is_ascii_alphanumeric() || c == '-' || c == ':' || c == '_' {
                attr.push(c.to_ascii_lowercase());
                chars.next();
            } else {
                break;
            }
        }
        if attr.is_empty() || chars.next().map(|(_, c)| c) != Some('=') {
            return Err(refuse("sanitizing unquoted or valueless attributes"));
        }
        let quote = match chars.next() {
            Some((_, q @ ('"' | '\''))) => q,
            _ => return Err(refuse("sanitizing unquoted or valueless attributes")),
        };
        let mut raw = String::new();
        loop {
            match chars.next() {
                None => return Err(refuse("sanitizing malformed tags")),
                Some((_, c)) if c == quote => break,
                Some((_, c)) => raw.push(c),
            }
        }
        if chars
            .peek()
            .is_some_and(|(_, c)| !c.is_ascii_whitespace() && *c != '/')
        {
            return Err(refuse("sanitizing malformed tags"));
        }
        if attributes.iter().any(|(a, _)| *a == attr) {
            return Err(refuse("sanitizing repeated attributes"));
        }
        attributes.push((attr, decode_text(&raw)?));
    }
    Ok(Tag {
        name,
        attributes,
        self_closing,
    })
}

/// The sanitized attributes as libxml2 writes them, unescaped.
fn attributes_html(tag: &Tag, extra: &[&str]) -> Result<String, Unsupported> {
    let mut out = String::new();
    for (name, value) in &tag.attributes {
        if !ALLOWED_ATTRIBUTES.contains(&name.as_str()) && !extra.contains(&name.as_str()) {
            continue;
        }
        if (name == "href" || name == "src") && !plain_url(value) {
            return Err(refuse(
                "sanitizing links other than plain http(s), mailto or relative ones",
            ));
        }
        if value.contains('"') {
            return Err(refuse("sanitizing attribute values with double quotes"));
        }
        out.push_str(&format!(" {name}=\"{value}\""));
    }
    Ok(out)
}

/// `sanitize_field(field, additional_attributes:)` for the fragments the
/// port reads (see the module); Err for the rest.
pub fn sanitize_field(field: &str, additional_attributes: &[&str]) -> Result<String, Unsupported> {
    let mut out = String::new();
    let mut open: Vec<String> = Vec::new();
    let mut rest = field;
    loop {
        // The next tag: `<` then a letter, `/` or `!`.
        let next = rest.char_indices().find(|&(i, c)| {
            c == '<'
                && rest[i + 1..]
                    .chars()
                    .next()
                    .is_some_and(|n| n.is_ascii_alphabetic() || n == '/' || n == '!')
        });
        let Some((i, _)) = next else {
            out.push_str(&decode_text(rest)?);
            break;
        };
        out.push_str(&decode_text(&rest[..i])?);
        let after = &rest[i + 1..];
        if after.starts_with('!') {
            return Err(refuse("sanitizing comments and declarations"));
        }
        let Some(end) = after.find('>') else {
            return Err(refuse("sanitizing unterminated tags"));
        };
        let inner = &after[..end];
        if inner.contains('<') {
            return Err(refuse("sanitizing malformed tags"));
        }
        rest = &after[end + 1..];
        if let Some(closing) = inner.strip_prefix('/') {
            let name = closing.trim_end().to_ascii_lowercase();
            if open.pop().as_deref() != Some(name.as_str()) {
                return Err(refuse("sanitizing misnested or stray end tags"));
            }
            out.push_str(&format!("</{name}>"));
            continue;
        }
        let tag = parse_tag(inner)?;
        let inline =
            INLINE_TAGS.contains(&tag.name.as_str()) || MORE_INLINE.contains(&tag.name.as_str());
        if VOID_TAGS.contains(&tag.name.as_str()) {
            out.push_str(&format!(
                "<{}{}>",
                tag.name,
                attributes_html(&tag, additional_attributes)?
            ));
            continue;
        }
        if !inline {
            return Err(refuse("sanitizing block or unlisted elements"));
        }
        if tag.self_closing {
            return Err(refuse("sanitizing self-closed elements"));
        }
        if tag.name == "a" && open.iter().any(|o| o == "a") {
            return Err(refuse("sanitizing nested links"));
        }
        out.push_str(&format!(
            "<{}{}>",
            tag.name,
            attributes_html(&tag, additional_attributes)?
        ));
        open.push(tag.name);
    }
    if !open.is_empty() {
        return Err(refuse("sanitizing unclosed elements"));
    }
    Ok(out.replace("%7B", "{").replace("%7D", "}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recorded from `sanitize_field` on the reference (Rails 8.1,
    /// rails-html-sanitizer's HTML4 safe list).
    #[test]
    fn sanitizes_like_rails() {
        let table = [
            ("plain text", "plain text"),
            ("AT&T", "AT&T"),
            ("&copy 2020", "&copy 2020"),
            ("&copyx", "&copyx"),
            ("&b", "&b"),
            ("x&amp", "x&amp"),
            (
                "<a href=\"/?a=1&copy=2\">c</a>",
                "<a href=\"/?a=1&copy=2\">c</a>",
            ),
            ("<a href=\"/?a=1&b=2\">c</a>", "<a href=\"/?a=1&b=2\">c</a>"),
            (
                "<a href=\"/?a=1&amp=2\">c</a>",
                "<a href=\"/?a=1&amp=2\">c</a>",
            ),
            ("&#169", "©"),
            ("&#x41x", "Ax"),
            ("a & b", "a & b"),
            ("a &amp; b", "a & b"),
            ("a < b > c", "a < b > c"),
            ("Tom's \"quote\"", "Tom's \"quote\""),
            ("&#169; &#xA9;", "© ©"),
            ("%7Bname%7D and %7b", "{name} and %7b"),
            ("<b>bold</b> and <i>it</i>", "<b>bold</b> and <i>it</i>"),
            (
                "<a href=\"/my/preferences\">profile</a>",
                "<a href=\"/my/preferences\">profile</a>",
            ),
            (
                "<a href='https://x.com/?a=1&b=2' target=\"_blank\" rel=\"nofollow\">x</a>",
                "<a href=\"https://x.com/?a=1&b=2\">x</a>",
            ),
            ("<br>line<br/>break", "<br>line<br>break"),
            (
                "<img src=\"/a.png\" alt=\"A\" onerror=\"x\">",
                "<img src=\"/a.png\" alt=\"A\">",
            ),
            ("<SPAN TITLE=\"up\">u</SPAN>", "<span title=\"up\">u</span>"),
            ("line1\nline2", "line1\nline2"),
            ("<em>é ü 中文 😀</em>", "<em>é ü 中文 😀</em>"),
            (
                "<a href=\"/x?y=1&amp;z=2\">q</a>",
                "<a href=\"/x?y=1&z=2\">q</a>",
            ),
            ("x <3 y", "x <3 y"),
            (
                "<a href=\"HTTP://EXAMPLE.COM\">u</a>",
                "<a href=\"HTTP://EXAMPLE.COM\">u</a>",
            ),
            ("<code>&lt;tag&gt;</code>", "<code><tag></code>"),
            (
                "<a name=\"n\" lang=\"en\" xml:lang=\"en\">n</a>",
                "<a name=\"n\" lang=\"en\" xml:lang=\"en\">n</a>",
            ),
            ("  leading and trailing  ", "  leading and trailing  "),
            (
                "<a href=\"/x\">a</a><a href=\"/y\">b</a>",
                "<a href=\"/x\">a</a><a href=\"/y\">b</a>",
            ),
            (
                "<time datetime=\"2020-01-01\">t</time>",
                "<time datetime=\"2020-01-01\">t</time>",
            ),
        ];
        for (input, expected) in table {
            assert_eq!(sanitize_field(input, &[]).unwrap(), expected, "{input:?}");
        }
    }

    #[test]
    fn refuses_what_libxml2_repairs() {
        for input in [
            "&nbsp;x",
            "&copy;",
            "<script>alert(1)</script>",
            "<p>one<p>two",
            "<b>unclosed",
            "</b>stray",
            "<!-- c -->",
            "<b><i>nested</b></i>",
            "<a href=\"javascript:alert(1)\">x</a>",
            "<a href=\" /space\">s</a>",
            "<a href=\"/x\" title=hi>u</a>",
            "<a>in<a>nested</a></a>",
            "<ul><li>a</li></ul>",
            "a<b",
            "<span title=\"a &quot;q&quot;\">s</span>",
        ] {
            assert!(sanitize_field(input, &[]).is_err(), "{input:?}");
        }
    }
}
