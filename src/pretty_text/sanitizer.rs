//! Port of pretty-text/sanitizer.js and allow-lister.js, and of the parts of
//! the `xss` library (1.0.15: parser.js, xss.js, default.js) the sanitizer
//! runs on. The rendered HTML is scanned tag by tag: a tag that is not
//! allowed is dropped (script and table with their content), an allowed
//! one is rebuilt from the attributes that pass.
//!
//! The scanner is the library's, quirks included, because the output has
//! to be the same string: it is a character loop over `<`, `>` and quotes,
//! not an HTML parser.

use std::collections::HashMap;

/// A feature's custom check: (tag, attribute, value) -> allowed.
pub type Custom = fn(&str, &str, &str) -> bool;

/// `AllowLister#getAllowList` and `getCustom`: the tags that may appear,
/// and per tag the attributes with the values they may take (`*` for any).
#[derive(Debug, Default)]
pub struct AllowList {
    attrs: HashMap<String, Vec<(String, Vec<String>)>>,
    custom: Vec<Custom>,
    /// `allowedHrefSchemes`
    pub href_schemes: Vec<String>,
    /// `allowedIframes`
    pub iframes: Vec<String>,
}

impl AllowList {
    /// `DEFAULT_LIST`
    pub fn new() -> AllowList {
        let mut list = AllowList::default();
        list.allow(DEFAULT_LIST);
        list
    }

    /// `allowListFeature` for a list of `tag.class`, `tag[attr]` and
    /// `tag[attr=value]` entries.
    pub fn allow(&mut self, entries: &[&str]) {
        for entry in entries {
            // `tag.split(".")`: the tag (with its attribute) and classes.
            let mut classes = entry.split('.');
            let tag_with_attr = classes.next().unwrap_or("");
            let classes: Vec<&str> = classes.collect();
            // ALLOWLIST_REGEX: `([^\[]+)(\[([^=]+)(=(.*))?\])?`
            let (tag, attr) = match tag_with_attr.split_once('[') {
                Some((tag, rest)) => (tag, rest.strip_suffix(']')),
                None => (tag_with_attr, None),
            };
            if tag.is_empty() {
                continue;
            }
            let attrs = self.attrs.entry(tag.to_string()).or_default();
            if !classes.is_empty() {
                let class = entry_mut(attrs, "class");
                class.extend(classes.iter().map(|c| c.to_string()));
            }
            if let Some(attr) = attr {
                match attr.split_once('=') {
                    Some((name, value)) if !value.is_empty() => {
                        entry_mut(attrs, name).push(value.to_string());
                    }
                    Some((name, _)) => *entry_mut(attrs, name) = vec!["*".to_string()],
                    None => *entry_mut(attrs, attr) = vec!["*".to_string()],
                }
            }
        }
    }

    /// `allowList({ custom })`
    pub fn allow_custom(&mut self, custom: Custom) {
        self.custom.push(custom);
    }
}

fn entry_mut<'a>(attrs: &'a mut Vec<(String, Vec<String>)>, name: &str) -> &'a mut Vec<String> {
    if let Some(i) = attrs.iter().position(|(n, _)| n == name) {
        return &mut attrs[i].1;
    }
    attrs.push((name.to_string(), Vec::new()));
    &mut attrs.last_mut().unwrap().1
}

fn is_js_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

fn trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

/// JavaScript `\w`
fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// xss `escapeHtml`
fn escape_html(s: &str) -> String {
    s.replace('<', "&lt;").replace('>', "&gt;")
}

/// xss `escapeAttrValue`
fn escape_attr_value(s: &str) -> String {
    escape_html(&s.replace('"', "&quot;"))
}

/// sanitizer.js `attr(name, value)`
fn attr(name: &str, value: Option<&str>) -> String {
    match value.filter(|v| !v.is_empty()) {
        Some(value) => format!("{name}=\"{}\"", escape_attr_value(value)),
        None => name.to_string(),
    }
}

/// `^prefix[\w.\-]+` after an optional case-insensitive prefix check.
fn starts_with_host_chars(rest: &str, extra: &[char]) -> bool {
    rest.chars()
        .next()
        .is_some_and(|c| is_word(c) || c == '.' || c == '-' || extra.contains(&c))
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

/// `hrefAllowed`: the href (single quotes escaped) when it is an absolute
/// or protocol-relative http url, a root-relative path, an anchor, a
/// mailto, or uses an extra allowed scheme.
fn href_allowed(href: &str, schemes: &[String]) -> Option<String> {
    let href = href.replace('\'', "%27");
    let after_scheme = strip_prefix_ci(&href, "https:")
        .or_else(|| strip_prefix_ci(&href, "http:"))
        .unwrap_or(&href);
    let allowed = after_scheme
        .strip_prefix("//")
        .is_some_and(|rest| starts_with_host_chars(rest, &[]))
        || href
            .strip_prefix('/')
            .is_some_and(|rest| starts_with_host_chars(rest, &[]))
        || href
            .strip_prefix('#')
            .is_some_and(|rest| starts_with_host_chars(rest, &[]))
        || strip_prefix_ci(&href, "mailto:")
            .is_some_and(|rest| starts_with_host_chars(rest, &['@']))
        || schemes.iter().any(|scheme| {
            strip_prefix_ci(&href, scheme)
                .and_then(|rest| rest.strip_prefix("://"))
                .is_some_and(|rest| starts_with_host_chars(rest, &[]))
        })
        || (schemes.iter().any(|s| s == "tel")
            && strip_prefix_ci(&href, "tel://").is_some_and(|rest| {
                starts_with_host_chars(rest.strip_prefix('+').unwrap_or(rest), &[])
            }));
    allowed.then_some(href)
}

/// `sanitizeMediaSrc`: `src` (and `srcset`) of img, source and track keep
/// only allowed urls; a refused one leaves the bare attribute name.
fn sanitize_media_src(tag: &str, name: &str, value: &str, schemes: &[String]) -> Option<String> {
    let checked = match tag {
        "img" | "track" => name == "src",
        "source" => name == "src" || name == "srcset",
        _ => false,
    };
    if !checked {
        return None;
    }
    if value.starts_with("data:image") {
        return Some(attr(name, Some(value)));
    }
    if name == "srcset" {
        let sanitized: Vec<String> = value
            .split(',')
            .map(|candidate| {
                // `v.split(" ", 2)`
                let mut parts = candidate.split(' ');
                let url = parts.next().unwrap_or("");
                let descriptor = parts.next();
                match href_allowed(url, schemes) {
                    Some(url) => match descriptor.filter(|d| !d.is_empty()) {
                        Some(d) => format!("{url} {d}"),
                        None => url,
                    },
                    None => String::new(),
                }
            })
            .collect();
        return Some(attr(name, Some(&sanitized.join(","))));
    }
    Some(attr(name, href_allowed(value, schemes).as_deref()))
}

/// `decodeURIComponent`, or None where it would throw.
fn decode_uri_component(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = value.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// An `allowed_iframes` entry as the regexp `^entry.*$` (case-insensitive)
/// with `*` standing for `[^/?#\\]+`.
fn iframe_matches(pattern: &str, url: &str) -> bool {
    fn matches(pattern: &[char], url: &[char]) -> bool {
        match pattern.split_first() {
            None => true,
            Some(('*', rest)) => {
                let run = url
                    .iter()
                    .take_while(|c| !matches!(c, '/' | '?' | '#' | '\\'))
                    .count();
                (1..=run).rev().any(|n| matches(rest, &url[n..]))
            }
            Some((p, rest)) => url
                .first()
                .is_some_and(|u| u.to_lowercase().eq(p.to_lowercase()) && matches(rest, &url[1..])),
        }
    }
    // `.` does not match a line terminator.
    if url.contains(['\n', '\r', '\u{2028}', '\u{2029}']) {
        return false;
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let url: Vec<char> = url.chars().collect();
    matches(&pattern, &url)
}

/// The iframe `src` check: no dot segment, and one allowed prefix that
/// matches the value both as written and decoded.
fn iframe_allowed(value: &str, iframes: &[String]) -> bool {
    let Some(decoded) = decode_uri_component(value) else {
        return false;
    };
    // `/\/\.+(?:[\/\\?#]|$)/`
    let chars: Vec<char> = decoded.chars().collect();
    let mut dot_segment = false;
    for (i, c) in chars.iter().enumerate() {
        if *c != '/' {
            continue;
        }
        let dots = chars[i + 1..].iter().take_while(|c| **c == '.').count();
        if dots > 0 {
            let after = chars.get(i + 1 + dots);
            if after.is_none_or(|c| matches!(c, '/' | '\\' | '?' | '#')) {
                dot_segment = true;
                break;
            }
        }
    }
    !dot_segment
        && iframes
            .iter()
            .any(|i| iframe_matches(i, &decoded) && iframe_matches(i, value))
}

/// `testDataAttribute`: a key of the tag's list that, read as a pattern
/// (a trailing `*` being one or more word characters), prefixes the name
/// and allows the value.
fn test_data_attribute(for_tag: &[(String, Vec<String>)], name: &str, value: &str) -> bool {
    for_tag.iter().any(|(key, valid)| {
        let prefixes = match key.strip_suffix('*') {
            Some(stem) => name
                .strip_prefix(stem)
                .is_some_and(|rest| rest.chars().next().is_some_and(is_word)),
            None => name.starts_with(key.as_str()),
        };
        prefixes && (valid.iter().any(|v| v == "*") || valid.iter().any(|v| v == value))
    })
}

/// `onIgnoreTagAttr`: the attribute as it stays in the tag, or None.
fn on_attr(list: &AllowList, tag: &str, name: &str, value: &str) -> Option<String> {
    let for_tag = list.attrs.get(tag)?;
    let for_attr = for_tag.iter().find(|(n, _)| n == name).map(|(_, v)| v);
    let listed = for_attr.is_some_and(|v| v.iter().any(|x| x == "*" || x == value));
    let data = !name.contains("data-html-")
        && name.starts_with("data-")
        && (for_tag.iter().any(|(n, _)| n == "data-*")
            || test_data_attribute(for_tag, name, value));
    let href = tag == "a" && name == "href" && href_allowed(value, &list.href_schemes).is_some();
    let iframe = tag == "iframe" && name == "src" && iframe_allowed(value, &list.iframes);
    if listed || data || href || iframe {
        return Some(attr(name, Some(value)));
    }
    if let Some(media) = sanitize_media_src(tag, name, value, &list.href_schemes) {
        return Some(media);
    }
    if tag == "iframe" && name == "src" {
        // This iframe is not allowed.
        return None;
    }
    if tag == "video" && name == "autoplay" {
        return Some("autoplay muted".to_string());
    }
    // Heading ids must begin with `heading--`.
    if matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
        && value.strip_prefix("heading--").is_some_and(|rest| {
            !rest.is_empty()
                && rest
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
    {
        return Some(attr(name, Some(value)));
    }
    list.custom
        .iter()
        .any(|custom| custom(tag, name, value))
        .then(|| attr(name, Some(value)))
}

/// xss `spaceIndex`
fn space_index(chars: &[char]) -> Option<usize> {
    chars.iter().position(|c| is_js_space(*c))
}

/// xss `getTagName`
fn tag_name(html: &[char]) -> String {
    let inner: String = match space_index(html) {
        // `html.slice(1, -1)`
        None if html.len() >= 2 => html[1..html.len() - 1].iter().collect(),
        None => String::new(),
        Some(i) => html[1..=i].iter().collect(),
    };
    let name = trim(&inner).to_lowercase();
    let name = name.strip_prefix('/').unwrap_or(&name);
    name.strip_suffix('/').unwrap_or(name).to_string()
}

fn find_next_equal(chars: &[char], mut i: usize) -> Option<usize> {
    while i < chars.len() {
        match chars[i] {
            ' ' => i += 1,
            '=' => return Some(i),
            _ => return None,
        }
    }
    None
}

fn find_next_quotation_mark(chars: &[char], mut i: usize) -> Option<usize> {
    while i < chars.len() {
        match chars[i] {
            ' ' => i += 1,
            '\'' | '"' => return Some(i),
            _ => return None,
        }
    }
    None
}

fn find_before_equal(chars: &[char], mut i: isize) -> bool {
    while i > 0 {
        match chars[i as usize] {
            ' ' => i -= 1,
            '=' => return true,
            _ => return false,
        }
    }
    false
}

fn strip_quote_wrap(text: &str) -> &str {
    let quoted = text.len() >= 2
        && ((text.starts_with('"') && text.ends_with('"'))
            || (text.starts_with('\'') && text.ends_with('\'')));
    if quoted {
        &text[1..text.len() - 1]
    } else if text == "\"" || text == "'" {
        // `substr(1, length - 2)` of a lone quote.
        ""
    } else {
        text
    }
}

/// xss `parseAttr`: the attributes of a tag, each through `on_attr`.
fn parse_attrs(html: &str, mut on_attr: impl FnMut(&str, &str) -> Option<String>) -> String {
    let mut chars: Vec<char> = html.chars().collect();
    let mut kept: Vec<String> = Vec::new();
    let mut add = |name: &str, value: &str| {
        // REGEXP_ILLEGAL_ATTR_NAME
        let name: String = trim(name)
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '\\' | '_' | ':' | '.' | '-'))
            .collect::<String>()
            .to_lowercase();
        if name.is_empty() {
            return;
        }
        if let Some(kept_attr) = on_attr(&name, value).filter(|a| !a.is_empty()) {
            kept.push(kept_attr);
        }
    };
    let text = |chars: &[char], from: usize, to: usize| -> String {
        chars[from.min(to)..to.min(chars.len())].iter().collect()
    };
    let mut last_pos = 0;
    let mut last_mark_pos: Option<usize> = None;
    let mut name: Option<String> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if name.is_none() && c == '=' {
            name = Some(text(&chars, last_pos, i));
            last_pos = i + 1;
            last_mark_pos = match chars.get(last_pos) {
                Some('"') | Some('\'') => Some(last_pos),
                _ => find_next_quotation_mark(&chars, i + 1),
            };
            i += 1;
            continue;
        }
        if let Some(attr_name) = &name
            && Some(i) == last_mark_pos
        {
            match chars[i + 1..].iter().position(|x| *x == c) {
                None => break,
                Some(offset) => {
                    let j = i + 1 + offset;
                    let value = text(&chars, i + 1, j);
                    add(attr_name, trim(&value));
                    name = None;
                    i = j;
                    last_pos = i + 1;
                    i += 1;
                    continue;
                }
            }
        }
        if is_js_space(c) {
            for x in chars.iter_mut() {
                if is_js_space(*x) {
                    *x = ' ';
                }
            }
            match &name {
                None => match find_next_equal(&chars, i) {
                    None => {
                        let value = text(&chars, last_pos, i);
                        add(trim(&value), "");
                        last_pos = i + 1;
                    }
                    Some(j) => {
                        // `i = j - 1`, then the loop's increment.
                        i = j;
                        continue;
                    }
                },
                Some(attr_name) => {
                    if !find_before_equal(&chars, i as isize - 1) {
                        let value = text(&chars, last_pos, i);
                        add(attr_name, strip_quote_wrap(trim(&value)));
                        name = None;
                        last_pos = i + 1;
                    }
                }
            }
        }
        i += 1;
    }
    if last_pos < chars.len() {
        let rest = text(&chars, last_pos, chars.len());
        match &name {
            None => add(&rest, ""),
            Some(attr_name) => add(attr_name, strip_quote_wrap(trim(&rest))),
        }
    }
    trim(&kept.join(" ")).to_string()
}

/// xss `stripCommentTag`: an unterminated comment takes the rest with it.
fn strip_comments(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        let Some(start) = rest.find("<!--") else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => break,
        }
    }
    out
}

/// `FilterXSS#process` with Discourse's options: allowed tags rebuilt,
/// the others stripped, `script` and `table` with their bodies.
fn filter(html: &str, list: &AllowList) -> String {
    let html: Vec<char> = strip_comments(html).chars().collect();
    let len = html.len();
    let mut out: Vec<char> = Vec::with_capacity(len);
    // StripTagBody's state: where the body being removed began (0 reads
    // as unset in the library) and the ranges to cut afterwards.
    let mut remove: Vec<(usize, usize)> = Vec::new();
    let mut pos_start: Option<usize> = None;

    let mut on_tag = |out: &mut Vec<char>, tag_html: &[char]| {
        let tag = tag_name(tag_html);
        let closing = tag_html.len() >= 2 && tag_html[0] == '<' && tag_html[1] == '/';
        let position = out.len();
        if list.attrs.contains_key(&tag) {
            if closing {
                out.extend(format!("</{tag}>").chars());
                return;
            }
            // getAttrs
            let (attrs_html, self_closing) = match space_index(tag_html) {
                None => (
                    String::new(),
                    tag_html.len() >= 2 && tag_html[tag_html.len() - 2] == '/',
                ),
                Some(i) => {
                    let inner: String = tag_html[i + 1..tag_html.len() - 1].iter().collect();
                    let inner = trim(&inner);
                    match inner.strip_suffix('/') {
                        Some(rest) => (trim(rest).to_string(), true),
                        None => (inner.to_string(), false),
                    }
                }
            };
            let attrs = parse_attrs(&attrs_html, |name, value| on_attr(list, &tag, name, value));
            let mut rebuilt = format!("<{tag}");
            if !attrs.is_empty() {
                rebuilt.push(' ');
                rebuilt.push_str(&attrs);
            }
            if self_closing {
                rebuilt.push_str(" /");
            }
            rebuilt.push('>');
            out.extend(rebuilt.chars());
        } else if tag == "script" || tag == "table" {
            if closing {
                let marker = "[/removed]";
                let end = position + marker.chars().count();
                remove.push((pos_start.unwrap_or(position), end));
                pos_start = None;
                out.extend(marker.chars());
            } else {
                if pos_start.is_none_or(|p| p == 0) {
                    pos_start = Some(position);
                }
                out.extend("[removed]".chars());
            }
        }
        // Any other tag is stripped.
    };

    // parseTag
    let escaped = |chars: &[char]| -> Vec<char> {
        escape_html(&chars.iter().collect::<String>())
            .chars()
            .collect()
    };
    let mut last_pos = 0;
    let mut tag_start: Option<usize> = None;
    let mut quote: Option<char> = None;
    let mut pos = 0;
    while pos < len {
        let c = html[pos];
        match tag_start {
            None => {
                if c == '<' {
                    tag_start = Some(pos);
                }
            }
            Some(start) => match quote {
                None => {
                    if c == '<' {
                        out.extend(escaped(&html[last_pos..pos]));
                        tag_start = Some(pos);
                        last_pos = pos;
                    } else if c == '>' || pos == len - 1 {
                        out.extend(escaped(&html[last_pos..start]));
                        on_tag(&mut out, &html[start..=pos]);
                        last_pos = pos + 1;
                        tag_start = None;
                    } else if c == '"' || c == '\'' {
                        // A quote opens a value only right after `=`.
                        let mut back = 1;
                        loop {
                            let before = if back <= pos {
                                Some(html[pos - back])
                            } else {
                                None
                            };
                            match before {
                                Some('=') => {
                                    quote = Some(c);
                                    break;
                                }
                                Some(b) if is_js_space(b) => back += 1,
                                // `"".trim() === ""` past the start: the
                                // library loops forever there; a tag
                                // always starts with `<`, so it is not
                                // reached.
                                _ => break,
                            }
                        }
                    }
                }
                Some(q) => {
                    if c == q {
                        quote = None;
                    }
                }
            },
        }
        pos += 1;
    }
    if last_pos < len {
        out.extend(escaped(&html[last_pos..]));
    }

    // StripTagBody#remove
    let mut result = String::with_capacity(out.len());
    let mut last = 0;
    for (start, end) in remove {
        result.extend(out[last.min(start)..start.min(out.len())].iter());
        last = end.min(out.len());
    }
    result.extend(out[last..].iter());
    result
}

/// `IFRAME_REGEXP`: an `<iframe>` whose tag has no `src` attribute goes,
/// with everything up to its closing tag or the end.
fn strip_iframes_without_src(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut pos = 0;
    while let Some(found) = lower[pos..].find("<iframe") {
        let start = pos + found;
        let Some(tag_end) = lower[start..].find('>') else {
            break;
        };
        let tag = &lower[start + "<iframe".len()..start + tag_end];
        // `\s+src\s*=`
        let has_src = tag.match_indices("src").any(|(i, _)| {
            tag[..i].chars().next_back().is_some_and(is_js_space)
                && tag[i + 3..]
                    .trim_start_matches(is_js_space)
                    .starts_with('=')
        });
        if has_src {
            out.push_str(&html[pos..start + tag_end + 1]);
            pos = start + tag_end + 1;
            continue;
        }
        out.push_str(&html[pos..start]);
        let body = start + tag_end + 1;
        // `<\/iframe\s*>` or the end.
        let mut end = lower.len();
        let mut search = body;
        while let Some(close) = lower[search..].find("</iframe") {
            let after = search + close + "</iframe".len();
            let rest = lower[after..].trim_start_matches(is_js_space);
            if rest.starts_with('>') {
                end = lower.len() - rest.len() + 1;
                break;
            }
            search = after;
        }
        pos = end;
    }
    out.push_str(&html[pos..]);
    out
}

/// `&(?![#\w]+;)` -> `&amp;`
fn escape_bare_ampersands(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let run = after
            .find(|c: char| !(is_word(c) || c == '#'))
            .unwrap_or(after.len());
        if run > 0 && after[run..].starts_with(';') {
            out.push('&');
        } else {
            out.push_str("&amp;");
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// `sanitize(text, allowLister)`
pub fn sanitize(text: &str, list: &AllowList) -> String {
    if text.is_empty() {
        return String::new();
    }
    // Allow things like <3 and <_<: `<([^A-Za-z\/\!]|$)` -> `&lt;$1`.
    let mut prepared = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '<' {
            prepared.push(c);
            continue;
        }
        match chars.peek() {
            Some(next) if next.is_ascii_alphabetic() || *next == '/' || *next == '!' => {
                prepared.push('<');
            }
            Some(_) => {
                prepared.push_str("&lt;");
                prepared.push(chars.next().unwrap());
            }
            None => prepared.push_str("&lt;"),
        }
    }
    let filtered = filter(&prepared, list).replace("[removed]", "");
    escape_bare_ampersands(&strip_iframes_without_src(&filtered))
        .replace("&#39;", "'")
        .replace(" />", ">")
}

/// Always allowed, whatever the features.
const DEFAULT_LIST: &[&str] = &[
    "a.anchor",
    "a.attachment",
    "a.hashtag",
    "a.mention",
    "a.mention-group",
    "a.onebox",
    "a.inline-onebox",
    "a.inline-onebox-loading",
    "a[class=inline-onebox --gh-status-draft]",
    "a[class=inline-onebox --gh-status-open]",
    "a[class=inline-onebox --gh-status-approved]",
    "a[class=inline-onebox --gh-status-changes_requested]",
    "a[class=inline-onebox --gh-status-merged]",
    "a[class=inline-onebox --gh-status-closed]",
    "a[data-bbcode]",
    "a[data-word]",
    "a[name]",
    "a[rel=nofollow]",
    "a[rel=ugc]",
    "a[target=_blank]",
    "a[title]",
    "abbr[title]",
    "aside.quote",
    "aside[data-*]",
    "audio",
    "audio[controls]",
    "audio[preload]",
    "b",
    "big",
    "blockquote",
    "br",
    "code",
    "dd",
    "del",
    "div",
    "div.quote-controls",
    "div.title",
    "div[align]",
    "div[lang]",
    "div[data-*]",
    "div[dir]",
    "dl",
    "dt",
    "em",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "iframe",
    "iframe[frameborder]",
    "iframe[height]",
    "iframe[marginheight]",
    "iframe[marginwidth]",
    "iframe[width]",
    "iframe[allowfullscreen]",
    "iframe[allow]",
    "img[alt]",
    "img[role]",
    "img[height]",
    "img[title]",
    "img[width]",
    "img[data-thumbnail]",
    "ins",
    "kbd",
    "li",
    "mark",
    "ol",
    "ol[reversed]",
    "ol[start]",
    "ol[type]",
    "p",
    "p[lang]",
    "picture",
    "pre",
    "s",
    "small",
    "span[lang]",
    "span.excerpt",
    "div.excerpt",
    "div.video-container",
    "div.video-placeholder-container",
    "div.onebox-placeholder-container",
    "span.placeholder-icon video",
    "span.hashtag",
    "span.mention",
    "strike",
    "strong",
    "sub",
    "sup",
    "source[data-orig-src]",
    "source[type]",
    "track",
    "track[default]",
    "track[label]",
    "track[kind]",
    "track[srclang]",
    "ul",
    "video",
    "video[controls]",
    "video[controlslist]",
    "video[crossorigin]",
    "video[height]",
    "video[loop]",
    "video[muted]",
    "video[playsinline]",
    "video[poster]",
    "video[preload]",
    "video[width]",
    "ruby",
    "ruby[lang]",
    "rb",
    "rb[lang]",
    "rp",
    "rt",
    "rt[lang]",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(html: &str) -> String {
        sanitize(html, &AllowList::new())
    }

    #[test]
    fn drops_what_is_not_allowed() {
        assert_eq!(clean("<b>x</b><script>alert(1)</script>y"), "<b>x</b>y");
        assert_eq!(clean("<p onclick=\"x()\">a</p>"), "<p>a</p>");
        assert_eq!(
            clean("<span style=\"color:red\">x</span>"),
            "<span>x</span>"
        );
        assert_eq!(clean("a <!-- c --> b"), "a  b");
        assert_eq!(clean("<blink>x</blink>"), "x");
    }

    #[test]
    fn keeps_allowed_attributes_and_urls() {
        assert_eq!(
            clean("<a href=\"https://example.com/x\" title=\"t\" data-x=\"1\">l</a>"),
            "<a href=\"https://example.com/x\" title=\"t\">l</a>"
        );
        assert_eq!(clean("<a href=\"javascript:alert(1)\">l</a>"), "<a>l</a>");
        assert_eq!(clean("<img src=x onerror=alert(1)>"), "<img src>");
        assert_eq!(
            clean("<img src=\"/a.png\" alt=\"a\">"),
            "<img src=\"/a.png\" alt=\"a\">"
        );
        assert_eq!(
            clean("<span class=\"mention\">@a</span>"),
            "<span class=\"mention\">@a</span>"
        );
        assert_eq!(clean("<span class=\"other\">@a</span>"), "<span>@a</span>");
    }

    #[test]
    fn text_and_entities() {
        assert_eq!(clean("I <3 you <_<"), "I &lt;3 you &lt;_&lt;");
        assert_eq!(clean("a & b &amp; c &#35; d"), "a &amp; b &amp; c &#35; d");
        assert_eq!(clean("<br />"), "<br>");
    }

    #[test]
    fn iframes_need_an_allowed_src() {
        let mut list = AllowList::new();
        list.iframes = vec!["https://www.google.com/maps/embed?".to_string()];
        assert_eq!(
            sanitize(
                "<iframe src=\"https://www.google.com/maps/embed?pb=1\" width=\"600\"></iframe>",
                &list
            ),
            "<iframe src=\"https://www.google.com/maps/embed?pb=1\" width=\"600\"></iframe>"
        );
        assert_eq!(
            sanitize(
                "a<iframe src=\"https://evil.example.com/\"></iframe>b",
                &list
            ),
            "ab"
        );
        assert_eq!(
            sanitize(
                "<iframe src=\"https://www.google.com/maps/embed?/../x\"></iframe>",
                &list
            ),
            ""
        );
    }
}
