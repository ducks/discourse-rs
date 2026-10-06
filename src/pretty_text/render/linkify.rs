//! Port of linkify-it 6.1.0 (the REBuilder sources and the matching half
//! of LinkifyIt), the version markdown-it 15 brings, and of markdown-it's
//! `linkify` core rule: urls, bare hosts with a known TLD and email
//! addresses in text become links.
//!
//! linkify-it's patterns lean on lookaheads, which the regex crate does
//! not have, so they are compiled with fancy-regex from the same sources.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use fancy_regex::Regex;
use markdown_it::parser::inline::{InlineRule, InlineState, Text, TextSpecial};
use markdown_it::plugins::cmark::inline::autolink::Autolink;
use markdown_it::plugins::html::html_inline::HtmlInline;
use markdown_it::{MarkdownIt, Node, NodeValue, Renderer};

use super::RenderSettings;
use super::context::Context;

/// A link the linkify rule made (`markup: "linkify"`, `info: "auto"`).
/// The onebox rule may add a class and a target after the href.
#[derive(Debug)]
pub struct Linkified {
    pub url: String,
    pub class: Option<&'static str>,
    pub target_blank: bool,
}

impl NodeValue for Linkified {
    fn render(&self, node: &Node, fmt: &mut dyn Renderer) {
        let mut attrs = vec![("href", self.url.clone())];
        if let Some(class) = self.class {
            attrs.push(("class", class.to_string()));
        }
        if self.target_blank {
            attrs.push(("target", "_blank".to_string()));
        }
        fmt.open("a", &attrs);
        fmt.contents(&node.children);
        fmt.close("a");
    }
}

/// Whether a node is an automatic link: `<url>` or a linkified one.
pub fn is_auto_link(node: &Node) -> bool {
    node.is::<Autolink>() || node.is::<Linkified>()
}

// uc.micro's classes.
const Z: &str = r"\p{Z}";
const P: &str = r"\p{P}";
const CC: &str = r"\p{Cc}";
/// JavaScript's `.`: anything but a line terminator.
const DOT: &str = r"[^\n\r\x{2028}\x{2029}]";
const TEXT_SEPARATORS: &str = r"[><\x{ff5c}]";

/// The schemas linkify-it knows by default, in its key order.
const SCHEMAS: [&str; 5] = ["http:", "https:", "ftp:", "//", "mailto:"];

/// `nestedPairRE(open, close)`: a bracketed run, nested four deep. The
/// JS bounds each run to 1000 characters, which a backtracking regex
/// would have to unroll; it is unbounded here.
fn nested_pair(open: &str, close: &str, zcc: &str) -> String {
    let atom = format!("(?:(?!{zcc}|{open}|{close}){DOT})");
    let mut pair = format!("{open}{atom}*{close}");
    for _ in 2..=4 {
        pair = format!("{open}(?:{atom}|{pair})*{close}");
    }
    pair
}

/// linkify-it's REBuilder, with Discourse's options: no `---`, no url
/// auth, no fuzzy IPs, and `maxLength` (10000) not enforced.
struct Sources {
    zpcc: String,
    zcc: String,
    port: &'static str,
    host_terminator: String,
    path: String,
    mail_name: &'static str,
    domain_root: String,
    domain: String,
    ipv6_url_host: String,
    ipv6_mail_host: String,
}

fn sources() -> Sources {
    let zpcc = format!("{Z}|{P}|{CC}");
    let zcc = format!("{Z}|{CC}");
    let pseudo_letter = format!("(?:(?!{TEXT_SEPARATORS}|{zpcc})(?s:.))");
    let ip4 = "(?:(?:25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9][0-9]|[0-9])[.]){3}(?:25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9][0-9]|[0-9])";
    let h16 = "[0-9A-Fa-f]{1,4}";
    let ls32 = format!("(?:(?:{h16}:{h16})|{ip4})");
    let ipv6 = format!(
        "(?:(?:{h16}:){{6}}{ls32}|::(?:{h16}:){{5}}{ls32}|(?:{h16})?::(?:{h16}:){{4}}{ls32}|(?:(?:{h16}:){{0,1}}{h16})?::(?:{h16}:){{3}}{ls32}|(?:(?:{h16}:){{0,2}}{h16})?::(?:{h16}:){{2}}{ls32}|(?:(?:{h16}:){{0,3}}{h16})?::{h16}:{ls32}|(?:(?:{h16}:){{0,4}}{h16})?::{ls32}|(?:(?:{h16}:){{0,5}}{h16})?::{h16}|(?:(?:{h16}:){{0,6}}{h16})?::)"
    );
    let port =
        r"(?::(?:6(?:[0-4][0-9]{3}|5(?:[0-4][0-9]{2}|5(?:[0-2][0-9]|3[0-5])))|[1-5]?[0-9]{1,4}))?";
    let host_terminator =
        format!(r"(?=$|{TEXT_SEPARATORS}|{zpcc})(?!-|_|:[0-9]|\.-|\.(?!$|{zpcc}))");
    let path_terminator = format!("{zpcc}|{TEXT_SEPARATORS}");
    let path = format!(
        concat!(
            r"(?:[/?#](?:",
            "{square}|{round}|{curly}|",
            r#"\"(?:(?!{zcc}|["]){dot}){{1,100}}\"|"#,
            r"\'(?:(?!{zcc}|[']){dot}){{1,100}}\'|",
            r"\'(?={pseudo}|[-])|",
            r"\.{{2,20}}[:]?[a-zA-Z0-9%/&]|",
            r"\.(?!{zcc}|[.]|$)|",
            r"\-{{1,20}}|",
            r",(?!{zcc}|$)|",
            r";(?!{zcc}|$)|",
            r"\!{{1,20}}(?!{zcc}|[!]|$)|",
            r"\?(?!{zcc}|[?]|$)|",
            r"[\\/:%@#&=_~*]|",
            r"(?!{terminator}){dot}",
            r")+|\/)?"
        ),
        square = nested_pair(r"\[", r"\]", &zcc),
        round = nested_pair(r"\(", r"\)", &zcc),
        curly = nested_pair(r"\{", r"\}", &zcc),
        zcc = zcc,
        dot = DOT,
        pseudo = pseudo_letter,
        terminator = path_terminator,
    );
    let xn = r"xn--[a-z0-9\-]{1,59}";
    Sources {
        port,
        host_terminator,
        path,
        mail_name: r"[-!#$%&'*+/=?^_`{|}~a-zA-Z0-9](?:[-!#$%&'*+/=?^_`{|}~a-zA-Z0-9]|[.](?=[-!#$%&'*+/=?^_`{|}~a-zA-Z0-9])){0,63}",
        domain_root: format!("(?:{xn}|{pseudo_letter}{{1,63}})"),
        domain: format!(
            "(?:{xn}|(?:{pseudo_letter})|(?:{pseudo_letter}(?:-|{pseudo_letter}){{0,61}}{pseudo_letter}))"
        ),
        ipv6_url_host: format!(r"\[{ipv6}\]"),
        ipv6_mail_host: format!(r"\[IPv6:{ipv6}\]"),
        zpcc,
        zcc,
    }
}

#[derive(Debug, Clone)]
struct Match {
    /// `http:`, `https:`, `ftp:`, `//`, `mailto:`, or empty for a fuzzy link.
    schema: String,
    index: usize,
    last_index: usize,
}

/// linkify-it, as markdown-it configures it for Discourse: fuzzy links and
/// emails, and the site's own list of TLDs.
#[derive(Debug)]
pub struct LinkifyIt {
    schema_search: Regex,
    schema_at_start: Regex,
    http: Regex,
    relative: Regex,
    mailto: Regex,
    fuzzy_link_search: Regex,
    fuzzy_mail_host_search: Regex,
    mail_name: Regex,
}

/// The start of a regex match at or after `from`.
fn find_from(regex: &Regex, text: &str, from: usize) -> Option<(usize, usize)> {
    let caps = regex.captures_from_pos(text, from).ok()??;
    let whole = caps.get(0)?;
    Some((whole.start(), whole.end()))
}

/// The byte offset `units` UTF-16 code units before `pos`, as JS counts
/// string offsets (clamped to the start).
fn back_utf16(text: &str, pos: usize, units: usize) -> usize {
    let mut left = units;
    let mut at = pos;
    for c in text[..pos].chars().rev() {
        let n = c.len_utf16();
        if n > left {
            break;
        }
        left -= n;
        at -= c.len_utf8();
    }
    at
}

/// Compiled instances by TLD list: the patterns take a while to build and
/// only the site setting changes them.
static COMPILED: LazyLock<Mutex<HashMap<Vec<String>, Arc<LinkifyIt>>>> =
    LazyLock::new(Default::default);

impl LinkifyIt {
    /// The instance for this TLD list, compiled on first use.
    pub fn for_tlds(tlds: &[String]) -> Result<Arc<LinkifyIt>, String> {
        if let Some(found) = COMPILED.lock().expect("linkify cache").get(tlds) {
            return Ok(found.clone());
        }
        let compiled = Arc::new(LinkifyIt::new(tlds)?);
        COMPILED
            .lock()
            .expect("linkify cache")
            .insert(tlds.to_vec(), compiled.clone());
        Ok(compiled)
    }

    /// `linkify.tlds(list)` then `set({ fuzzyLink: true })`.
    pub fn new(tlds: &[String]) -> Result<LinkifyIt, String> {
        let s = sources();
        let compile = |source: String| {
            Regex::new(&format!("(?i){source}")).map_err(|e| format!("linkify: {e}"))
        };
        // get_tld: the list sorted, deduplicated and reversed, then xn--.
        let mut list: Vec<&str> = tlds.iter().map(String::as_str).collect();
        list.sort_unstable_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        list.dedup();
        list.reverse();
        let tld = match list.join("|") {
            joined if joined.is_empty() => r"\$#none#\$".to_string(),
            joined => joined,
        };
        let tld = format!(r"{tld}|xn--[a-z0-9\-]{{1,59}}");
        let schema_names = SCHEMAS.join("|");
        let url_host_port = format!(
            r"(?:{}|(?:(?:(?:{})\.){{0,10}}{})){}{}",
            s.ipv6_url_host, s.domain, s.domain, s.port, s.host_terminator
        );
        let fuzzy_url_host_port = format!(
            r"(?:(?:(?:(?:{})\.){{1,10}}(?:{tld}))){}",
            s.domain, s.host_terminator
        );
        let mail_host = format!(
            r"(?:{}|(?:(?:(?:{})\.){{0,4}}{})){}",
            s.ipv6_mail_host, s.domain, s.domain, s.host_terminator
        );
        let fuzzy_mail_host = format!(
            r"(?:{}|(?:(?:(?:{})[.]){{1,4}}{})){}",
            s.ipv6_mail_host, s.domain, s.domain_root, s.host_terminator
        );
        let schema_search = format!(r"(^|(?!_)(?:[><\x{{ff5c}}]|{}))({schema_names})", s.zpcc);
        Ok(LinkifyIt {
            schema_at_start: compile(format!("^{schema_search}"))?,
            schema_search: compile(schema_search)?,
            http: compile(format!(r"^//{url_host_port}{}", s.path))?,
            relative: compile(format!(
                r"^(?:localhost|{}|(?:(?:{})[.]){{1,10}}{}){}{}{}",
                s.ipv6_url_host, s.domain, s.domain_root, s.port, s.host_terminator, s.path
            ))?,
            mailto: compile(format!("^{}@{mail_host}", s.mail_name))?,
            fuzzy_link_search: compile(format!(
                r"(^|(?![.:/\-_@])(?:[$+<=>^`|\x{{ff5c}}]|{}))(?:(?![$+<=>^`|\x{{ff5c}}]){fuzzy_url_host_port}{})",
                s.zpcc, s.path
            ))?,
            fuzzy_mail_host_search: compile(format!("@{fuzzy_mail_host}"))?,
            // No `i` flag in the JS; the class names both cases.
            mail_name: Regex::new(&format!(
                r#"(?:^|{TEXT_SEPARATORS}|"|\(|{})({})$"#,
                s.zcc, s.mail_name
            ))
            .map_err(|e| format!("linkify: {e}"))?,
        })
    }

    /// `matchAtStart(text)`: a link with a schema starting the text, as
    /// its normalized url.
    fn match_at_start(&self, text: &str) -> Option<String> {
        let caps = self.schema_at_start.captures(text).ok()??;
        let whole = caps.get(0)?;
        let schema = caps.get(2)?.as_str();
        let len = self.schema_length(text, schema, whole.end());
        if len == 0 {
            return None;
        }
        let index = caps.get(1).map_or(0, |m| m.end());
        let raw = &text[index..whole.end() + len];
        // LinkifyIt#normalize
        if schema.eq_ignore_ascii_case("mailto:")
            && !raw.to_ascii_lowercase().starts_with("mailto:")
        {
            Some(format!("mailto:{raw}"))
        } else {
            Some(raw.to_string())
        }
    }

    /// `testSchemaAt`: how much of the text after a schema is its link.
    fn schema_length(&self, text: &str, schema: &str, pos: usize) -> usize {
        let tail = &text[pos..];
        let regex = match schema.to_lowercase().as_str() {
            "http:" | "https:" | "ftp:" => &self.http,
            "//" => {
                // Not inside `scheme://` or after another slash.
                let third_back = text[..pos].chars().rev().nth(2);
                if matches!(third_back, Some(':') | Some('/')) {
                    return 0;
                }
                &self.relative
            }
            "mailto:" => &self.mailto,
            _ => return 0,
        };
        match regex.find(tail) {
            Ok(Some(m)) => m.end(),
            _ => 0,
        }
    }

    /// The name before an `@host` match at `at`: `mail_name_validator` on
    /// the 65 code units before it. Returns where the name starts.
    fn mail_name_start(&self, text: &str, at: usize) -> Option<usize> {
        let from = back_utf16(text, at, 65);
        let caps = self.mail_name.captures(&text[from..at]).ok()??;
        Some(from + caps.get(1)?.start())
    }

    /// `match(text)`: the three searches advance together; at each step
    /// the earliest candidate wins, the longer on a tie, a schema before an
    /// email before a fuzzy link, and the next search starts where it ended.
    fn matches(&self, text: &str) -> Vec<Match> {
        let mut result = Vec::new();
        if text.is_empty() {
            return result;
        }
        let mut pos = 0;
        // Each search's `lastIndex`, and whether it has run out.
        let (mut mail_last, mut mail_done) = (0, false);
        let (mut link_last, mut link_done) = (0, false);
        let (mut schema_last, mut schema_done) = (0, false);
        let mut mail_candidate: Option<Match> = None;
        let mut link_candidate: Option<Match> = None;
        let mut schema_prefix: Option<(String, usize, usize)> = None;
        loop {
            // `Math.max(pos - 1, 0)`: one character back.
            let scan_from = back_utf16(text, pos, 1);
            if !mail_done && mail_candidate.as_ref().is_none_or(|c| c.index < pos) {
                mail_last = mail_last.max(scan_from);
                loop {
                    let Some((start, end)) =
                        find_from(&self.fuzzy_mail_host_search, text, mail_last)
                    else {
                        mail_done = true;
                        mail_candidate = None;
                        break;
                    };
                    mail_last = end;
                    let Some(name) = self.mail_name_start(text, start) else {
                        continue;
                    };
                    mail_candidate = Some(Match {
                        schema: "mailto:".to_string(),
                        index: name,
                        last_index: end,
                    });
                    if name >= pos {
                        break;
                    }
                    mail_last = mail_last.max(scan_from);
                }
            }
            if !link_done && link_candidate.as_ref().is_none_or(|c| c.index < pos) {
                link_last = link_last.max(scan_from);
                loop {
                    let found = self
                        .fuzzy_link_search
                        .captures_from_pos(text, link_last)
                        .ok()
                        .flatten();
                    let Some(caps) = found else {
                        link_done = true;
                        link_candidate = None;
                        break;
                    };
                    let whole = caps.get(0).unwrap();
                    link_last = whole.end();
                    let index = whole.start() + caps.get(1).map_or(0, |m| m.as_str().len());
                    link_candidate = Some(Match {
                        schema: String::new(),
                        index,
                        last_index: whole.end(),
                    });
                    if index >= pos {
                        break;
                    }
                    link_last = link_last.max(scan_from);
                }
            }
            let fuzzy = earlier(mail_candidate.as_ref(), link_candidate.as_ref());
            let mut schema_candidate = None;
            while !schema_done {
                let prefix = match schema_prefix.take() {
                    Some(prefix) => prefix,
                    None => {
                        schema_last = schema_last.max(scan_from);
                        let found = self
                            .schema_search
                            .captures_from_pos(text, schema_last)
                            .ok()
                            .flatten();
                        let Some(caps) = found else {
                            schema_done = true;
                            break;
                        };
                        let whole = caps.get(0).unwrap();
                        schema_last = whole.end();
                        let schema = caps.get(2).unwrap().as_str().to_string();
                        let index = whole.start() + caps.get(1).map_or(0, |m| m.as_str().len());
                        (schema, index, whole.end())
                    }
                };
                if prefix.1 < pos {
                    continue;
                }
                if fuzzy.is_some_and(|f| prefix.1 > f.index) {
                    schema_prefix = Some(prefix);
                    break;
                }
                let len = self.schema_length(text, &prefix.0, prefix.2);
                if len > 0 {
                    schema_candidate = Some(Match {
                        schema: prefix.0.to_lowercase(),
                        index: prefix.1,
                        last_index: prefix.2 + len,
                    });
                    break;
                }
            }
            // The schema's, then the email if it wins over that, then the
            // fuzzy link if it wins over the result; the email or link taken
            // is used up.
            let picks = [
                schema_candidate.as_ref(),
                mail_candidate.as_ref(),
                link_candidate.as_ref(),
            ];
            let mut best: Option<usize> = None;
            for (i, pick) in picks.iter().enumerate() {
                if let Some(c) = pick
                    && best.is_none_or(|b| wins(c, picks[b].unwrap()))
                {
                    best = Some(i);
                }
            }
            let candidate = match best {
                None => break,
                Some(0) => schema_candidate.unwrap(),
                Some(1) => mail_candidate.take().unwrap(),
                Some(_) => link_candidate.take().unwrap(),
            };
            pos = candidate.last_index;
            result.push(candidate);
        }
        result
    }
}

/// `b` takes `a`'s place: it starts earlier, or at the same place and ends
/// later.
fn wins(b: &Match, a: &Match) -> bool {
    b.index < a.index || (b.index == a.index && b.last_index > a.last_index)
}

/// `a` unless `b` wins over it.
fn earlier<'a>(a: Option<&'a Match>, b: Option<&'a Match>) -> Option<&'a Match> {
    match (a, b) {
        (None, b) => b,
        (Some(a), Some(b)) if wins(b, a) => Some(b),
        (a, _) => a,
    }
}

/// Discourse sets `mdurl.decode.defaultChars` to `;/?:@&=+$,# `, and
/// normalizeLinkText adds `%`: what stays encoded in a link's text.
fn normalize_link_text(url: &str) -> Result<String, &'static str> {
    use mdurl::urlencode::{AsciiSet, decode};
    const KEEP: AsciiSet = AsciiSet::from(";/?:@&=+$,# %");
    // Punycode hosts are shown decoded, which is not ported.
    if url.to_ascii_lowercase().contains("xn--") {
        return Err("punycode hosts in links");
    }
    Ok(decode(url, KEEP).into_owned())
}

/// `isSchemeChar`
fn is_scheme_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.')
}

/// markdown-it's `linkify` inline rule: at `://`, a schema written just
/// before it starts a link, taken whole before emphasis, code spans or
/// anything else can claim part of it (`http://a.com/__init__.py`).
struct InlineLinkify;

impl InlineRule for InlineLinkify {
    const MARKER: char = ':';

    fn run(state: &mut InlineState) -> Option<(Node, usize)> {
        let linkify = state.md.ext.get::<RenderSettings>()?.linkify.clone()?;
        if state.link_level > 0 {
            return None;
        }
        let pos = state.pos;
        if !state.src[pos..state.pos_max].starts_with("://") {
            return None;
        }
        // The schema is the end of the pending text: up to ten scheme
        // characters, starting with a letter.
        let pending = state.trailing_text_get().as_bytes();
        let most = pending.len().min(10).min(pos);
        let proto_len = pending
            .iter()
            .rev()
            .take(most)
            .take_while(|b| is_scheme_char(**b))
            .count();
        if proto_len == 0 || !pending[pending.len() - proto_len].is_ascii_alphabetic() {
            return None;
        }
        let proto_start = pos - proto_len;
        if state.src.as_bytes()[proto_start..pos] != pending[pending.len() - proto_len..] {
            return None;
        }
        let mut url = linkify.match_at_start(&state.src[proto_start..])?;
        if url.len() <= proto_len {
            return None;
        }
        url.truncate(url.trim_end_matches('*').len());
        // The JS matches the rest of the whole source; a nested parse that
        // ends earlier keeps the link inside it.
        if proto_start + url.len() > state.pos_max {
            return None;
        }
        let ctx = state.md.ext.get::<Context>()?;
        if !url.is_ascii() {
            ctx.refuse("non-ASCII urls in links (punycode and percent-encoding)");
        }
        let full_url = state.md.link_formatter.normalize_link(&url);
        state.md.link_formatter.validate_link(&full_url)?;
        let content = normalize_link_text(&url).unwrap_or_else(|what| {
            ctx.refuse(what);
            url.clone()
        });
        state.trailing_text_pop(proto_len);
        state.pos = proto_start;
        let mut node = Node::new(Linkified {
            url: full_url,
            class: None,
            target_blank: false,
        });
        node.children.push(Node::new(Text { content }));
        Some((node, url.len()))
    }
}

pub fn add(md: &mut MarkdownIt) {
    md.inline.add_rule::<InlineLinkify>();
}

/// The text of one node split around its links, or None without any.
fn split(
    text: &str,
    after_special: bool,
    linkify: &LinkifyIt,
    md: &MarkdownIt,
    unsupported: &mut Option<&'static str>,
) -> Option<Vec<Node>> {
    let mut links = linkify.matches(text);
    if links.is_empty() {
        return None;
    }
    // A link right after an escaped character is not one.
    if links[0].index == 0 && after_special {
        links.remove(0);
    }
    let mut nodes = Vec::new();
    let mut last_pos = 0;
    for link in &links {
        let raw = &text[link.index..link.last_index];
        // LinkifyIt#normalize
        let url = if link.schema.is_empty() {
            format!("http://{raw}")
        } else if link.schema == "mailto:" && !raw.to_ascii_lowercase().starts_with("mailto:") {
            format!("mailto:{raw}")
        } else {
            raw.to_string()
        };
        if !url.is_ascii() {
            *unsupported = Some("non-ASCII urls in links (punycode and percent-encoding)");
        }
        let full_url = md.link_formatter.normalize_link(&url);
        if md.link_formatter.validate_link(&full_url).is_none() {
            continue;
        }
        let url_text = if link.schema.is_empty() {
            normalize_link_text(&format!("http://{raw}"))
                .map(|t| t.strip_prefix("http://").map(str::to_string).unwrap_or(t))
        } else if link.schema == "mailto:" && !raw.to_ascii_lowercase().starts_with("mailto:") {
            normalize_link_text(&format!("mailto:{raw}"))
                .map(|t| t.strip_prefix("mailto:").map(str::to_string).unwrap_or(t))
        } else {
            normalize_link_text(raw)
        };
        let url_text = match url_text {
            Ok(text) => text,
            Err(what) => {
                *unsupported = Some(what);
                raw.to_string()
            }
        };
        if link.index > last_pos {
            nodes.push(Node::new(Text {
                content: text[last_pos..link.index].to_string(),
            }));
        }
        let mut node = Node::new(Linkified {
            url: full_url,
            class: None,
            target_blank: false,
        });
        node.children.push(Node::new(Text { content: url_text }));
        nodes.push(node);
        last_pos = link.last_index;
    }
    if last_pos < text.len() {
        nodes.push(Node::new(Text {
            content: text[last_pos..].to_string(),
        }));
    }
    Some(nodes)
}

fn is_link_open(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    lower
        .strip_prefix("<a")
        .is_some_and(|rest| rest.starts_with('>') || rest.starts_with(char::is_whitespace))
}

fn is_link_close(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    lower
        .strip_prefix("</a")
        .is_some_and(|rest| rest.trim_start().starts_with('>'))
}

/// markdown-it's `linkify` core rule: every text outside a link, also
/// outside one written as inline html. Returns what it met that is not
/// ported, if anything.
pub fn run(root: &mut Node, linkify: &Arc<LinkifyIt>, md: &MarkdownIt) -> Option<&'static str> {
    fn visit(
        node: &mut Node,
        linkify: &LinkifyIt,
        md: &MarkdownIt,
        unsupported: &mut Option<&'static str>,
    ) {
        if super::text_post_process::is_link(node) {
            return;
        }
        // The tokens are walked from the end, counting `</a>` and `<a>`.
        let mut html_link_level = 0;
        let mut i = node.children.len();
        while i > 0 {
            i -= 1;
            if let Some(html) = node.children[i].cast::<HtmlInline>() {
                if is_link_open(&html.content) && html_link_level > 0 {
                    html_link_level -= 1;
                }
                if is_link_close(&html.content) {
                    html_link_level += 1;
                }
            }
            if html_link_level > 0 {
                continue;
            }
            let after_special = i > 0 && node.children[i - 1].is::<TextSpecial>();
            let replaced = node.children[i]
                .cast::<Text>()
                .and_then(|text| split(&text.content, after_special, linkify, md, unsupported));
            match replaced {
                Some(mut nodes) => {
                    // The pieces keep the text's source, which smartquotes
                    // groups by.
                    let srcmap = node.children[i].srcmap;
                    nodes.iter_mut().for_each(|n| n.srcmap = srcmap);
                    node.children.splice(i..=i, nodes);
                }
                None => visit(&mut node.children[i], linkify, md, unsupported),
            }
        }
    }
    let mut unsupported = None;
    visit(root, linkify, md, &mut unsupported);
    unsupported
}
