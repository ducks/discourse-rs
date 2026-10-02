//! Port of linkify-it 5.0.2 (lib/re.mjs and the matching half of
//! index.mjs) and of markdown-it's `linkify` core rule: urls, bare hosts
//! with a known TLD and email addresses in text become links.
//!
//! linkify-it's patterns lean on lookaheads, which the regex crate does
//! not have, so they are compiled with fancy-regex from the same sources.

use std::sync::Arc;

use fancy_regex::Regex;
use markdown_it::parser::inline::{Text, TextSpecial};
use markdown_it::plugins::cmark::inline::autolink::Autolink;
use markdown_it::plugins::html::html_inline::HtmlInline;
use markdown_it::{MarkdownIt, Node, NodeValue, Renderer};

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

struct Sources {
    zpcc: String,
    auth: String,
    port: &'static str,
    host_terminator: String,
    path: String,
    email_name: &'static str,
    xn: &'static str,
    domain_root: String,
    domain: String,
    host: String,
}

/// lib/re.mjs, without the `---` option.
fn sources() -> Sources {
    let zpcc = format!("{Z}|{P}|{CC}");
    let zcc = format!("{Z}|{CC}");
    let pseudo_letter = format!("(?:(?!{TEXT_SEPARATORS}|{zpcc})(?s:.))");
    let auth = format!(r"(?:(?:(?!{zcc}|[@/\[\]()]){DOT}){{1,50}}@)?");
    let port =
        r"(?::(?:6(?:[0-4][0-9]{3}|5(?:[0-4][0-9]{2}|5(?:[0-2][0-9]|3[0-5])))|[1-5]?[0-9]{1,4}))?";
    let host_terminator =
        format!(r"(?=$|{TEXT_SEPARATORS}|{zpcc})(?!-|_|:[0-9]|\.-|\.(?!$|{zpcc}))");
    let path = format!(
        concat!(
            "(?:",
            r"[/?#]",
            "(?:",
            r#"(?!{zcc}|{sep}|[()\[\]{{}}.,"'?!\-;]){dot}|"#,
            r"\[(?:(?!{zcc}|\]){dot})*\]|",
            r"\((?:(?!{zcc}|[)]){dot})*\)|",
            r"\{{(?:(?!{zcc}|[}}]){dot})*\}}|",
            r#"\"(?:(?!{zcc}|["]){dot})+\"|"#,
            r"\'(?:(?!{zcc}|[']){dot})+\'|",
            r"\'(?={pseudo}|[-])|",
            r"\.{{2,}}[a-zA-Z0-9%/&]|",
            r"\.(?!{zcc}|[.]|$)|",
            r"\-+|",
            r",(?!{zcc}|$)|",
            r";(?!{zcc}|$)|",
            r"\!+(?!{zcc}|[!]|$)|",
            r"\?(?!{zcc}|[?]|$)",
            ")+",
            r"|\/",
            ")?"
        ),
        zcc = zcc,
        sep = TEXT_SEPARATORS,
        dot = DOT,
        pseudo = pseudo_letter,
    );
    let xn = r"xn--[a-z0-9\-]{1,59}";
    let domain_root = format!("(?:{xn}|{pseudo_letter}{{1,63}})");
    let domain = format!(
        "(?:{xn}|(?:{pseudo_letter})|(?:{pseudo_letter}(?:-|{pseudo_letter}){{0,61}}{pseudo_letter}))"
    );
    let host = format!(r"(?:(?:(?:(?:{domain})\.)*{domain}))");
    Sources {
        zpcc,
        auth,
        port,
        host_terminator,
        path,
        email_name: r#"[\-;:&=\+\$,\.a-zA-Z0-9_][\-;:&=\+\$,\"\.a-zA-Z0-9_]{0,63}"#,
        xn,
        domain_root,
        domain,
        host,
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
/// emails, no fuzzy IPs, and the site's own list of TLDs.
#[derive(Debug)]
pub struct LinkifyIt {
    schema_search: Regex,
    http: Regex,
    no_http: Regex,
    mailto: Regex,
    host_fuzzy_test: Regex,
    link_no_ip_fuzzy: Regex,
    email_fuzzy: Regex,
}

impl LinkifyIt {
    /// `linkify.tlds(list)`: the list replaces the default one, so the
    /// two-letter country codes are not added.
    pub fn new(tlds: &[String]) -> Result<LinkifyIt, String> {
        let s = sources();
        let mut tld_list: Vec<String> = tlds.to_vec();
        tld_list.push(s.xn.to_string());
        let tlds = tld_list.join("|");
        let compile = |source: String| {
            Regex::new(&format!("(?i){source}")).map_err(|e| format!("linkify: {e}"))
        };
        let host_no_ip_fuzzy = format!(r"(?:(?:(?:{})\.)+(?:{tlds}))", s.domain);
        let host_fuzzy = format!(
            r"(?:(?:(25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){{3}}(25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)|{host_no_ip_fuzzy})"
        );
        let before_link = format!(r"(^|(?![.:/\-_@])(?:[$+<=>^`|\x{{ff5c}}]|{}))", s.zpcc);
        Ok(LinkifyIt {
            // The schemas: http:, https:, ftp:, //, mailto:.
            schema_search: compile(format!(
                r"(^|(?!_)(?:[><\x{{ff5c}}]|{}))(http:|https:|ftp:|//|mailto:)",
                s.zpcc
            ))?,
            http: compile(format!(
                r"^//{}{}{}{}{}",
                s.auth, s.host, s.port, s.host_terminator, s.path
            ))?,
            no_http: compile(format!(
                r"^{}(?:localhost|(?:(?:{})\.)+{}){}{}{}",
                s.auth, s.domain, s.domain_root, s.port, s.host_terminator, s.path
            ))?,
            mailto: compile(format!("^{}@{}{}", s.email_name, s.host, s.host_terminator))?,
            host_fuzzy_test: compile(format!(
                r"localhost|www\.|\.[0-9]{{1,3}}\.|(?:\.(?:{tlds})(?:{}|>|$))",
                s.zpcc
            ))?,
            link_no_ip_fuzzy: compile(format!(
                r"{before_link}((?![$+<=>^`|\x{{ff5c}}]){host_no_ip_fuzzy}{}{}{})",
                s.port, s.host_terminator, s.path
            ))?,
            email_fuzzy: compile(format!(
                r#"(^|{TEXT_SEPARATORS}|"|\(|{Z}|{CC})({}@{host_fuzzy}{})"#,
                s.email_name, s.host_terminator
            ))?,
        })
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
                &self.no_http
            }
            "mailto:" => &self.mailto,
            _ => return 0,
        };
        match regex.find(tail) {
            Ok(Some(m)) => m.end(),
            _ => 0,
        }
    }

    /// Every match of a global regex, as `exec` in a loop finds them.
    fn each(regex: &Regex, text: &str, mut f: impl FnMut(&fancy_regex::Captures)) {
        let mut pos = 0;
        while pos <= text.len() {
            let Ok(Some(caps)) = regex.captures_from_pos(text, pos) else {
                break;
            };
            let whole = caps.get(0).unwrap();
            f(&caps);
            pos = if whole.end() > whole.start() {
                whole.end()
            } else {
                // An empty match advances by one character.
                match text[whole.end()..].chars().next() {
                    Some(c) => whole.end() + c.len_utf8(),
                    None => break,
                }
            };
        }
    }

    /// `match(text)`: the links in the text, schemed ones first where two
    /// start together, none overlapping.
    fn matches(&self, text: &str) -> Vec<Match> {
        let mut schemed = Vec::new();
        let mut fuzzy_link = Vec::new();
        let mut fuzzy_email = Vec::new();
        if text.is_empty() {
            return Vec::new();
        }
        Self::each(&self.schema_search, text, |caps| {
            let whole = caps.get(0).unwrap();
            let schema = caps.get(2).unwrap().as_str();
            let len = self.schema_length(text, schema, whole.end());
            if len > 0 {
                schemed.push(Match {
                    schema: schema.to_lowercase(),
                    index: whole.start() + caps.get(1).map_or(0, |m| m.as_str().len()),
                    last_index: whole.end() + len,
                });
            }
        });
        if self.host_fuzzy_test.is_match(text).unwrap_or(false) {
            Self::each(&self.link_no_ip_fuzzy, text, |caps| {
                let whole = caps.get(0).unwrap();
                fuzzy_link.push(Match {
                    schema: String::new(),
                    index: whole.start() + caps.get(1).map_or(0, |m| m.as_str().len()),
                    last_index: whole.end(),
                });
            });
        }
        if text.contains('@') {
            Self::each(&self.email_fuzzy, text, |caps| {
                let whole = caps.get(0).unwrap();
                fuzzy_email.push(Match {
                    schema: "mailto:".to_string(),
                    index: whole.start() + caps.get(1).map_or(0, |m| m.as_str().len()),
                    last_index: whole.end(),
                });
            });
        }

        // The earliest candidate of the three lists; on a tie the longest,
        // then schemed before email before fuzzy link.
        fn choose<'a>(a: Option<&'a Match>, b: Option<&'a Match>) -> Option<&'a Match> {
            match (a, b) {
                (None, b) => b,
                (a, None) => a,
                (Some(a), Some(b)) if a.index != b.index => {
                    Some(if a.index < b.index { a } else { b })
                }
                (Some(a), Some(b)) => Some(if a.last_index >= b.last_index { a } else { b }),
            }
        }
        let lists = [&schemed, &fuzzy_email, &fuzzy_link];
        let mut indexes = [0usize; 3];
        let mut result = Vec::new();
        let mut last_index = 0;
        loop {
            let candidates = [
                lists[0].get(indexes[0]),
                lists[1].get(indexes[1]),
                lists[2].get(indexes[2]),
            ];
            let Some(candidate) = choose(choose(candidates[0], candidates[1]), candidates[2])
            else {
                break;
            };
            let from = candidates
                .iter()
                .position(|c| c.is_some_and(|c| std::ptr::eq(c, candidate)))
                .unwrap_or(2);
            indexes[from] += 1;
            if candidate.index < last_index {
                continue;
            }
            last_index = candidate.last_index;
            result.push(candidate.clone());
        }
        result
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
                Some(nodes) => {
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
