//! The ICU MessageFormat the client formats `*_MF` translations with
//! (`I18n.messageFormat`): text, `{arg}`, `{arg, select, ...}` and
//! `{arg, plural, =N {...} one {...} other {...}}` with `#` for the count.
//! Plural categories are English's; apostrophe quoting and plural offsets
//! are not handled (the locale strings this formats use neither).

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Arg<'a> {
    Str(&'a str),
    Num(i64),
    Bool(bool),
}

impl Arg<'_> {
    fn text(&self) -> String {
        match self {
            Arg::Str(s) => (*s).to_string(),
            Arg::Num(n) => n.to_string(),
            Arg::Bool(b) => b.to_string(),
        }
    }
}

#[derive(Debug)]
enum Node {
    Text(String),
    Arg(String),
    /// `#` inside a plural arm.
    Count,
    Select(String, Vec<(String, Vec<Node>)>),
    Plural(String, Vec<(String, Vec<Node>)>),
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseError(String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "message format: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// Formats `message` with `args`; a missing argument formats empty, as
/// the client's compiled formatter does with `undefined`.
pub fn format(message: &str, args: &[(&str, Arg)]) -> Result<String, ParseError> {
    let chars: Vec<char> = message.chars().collect();
    let mut pos = 0;
    let nodes = parse(&chars, &mut pos, false)?;
    if pos != chars.len() {
        return Err(ParseError(format!("unbalanced '}}' at {pos}")));
    }
    let args: HashMap<&str, &Arg> = args.iter().map(|(k, v)| (*k, v)).collect();
    let mut out = String::new();
    render(&nodes, &args, None, &mut out);
    Ok(out)
}

fn parse(chars: &[char], pos: &mut usize, in_plural: bool) -> Result<Vec<Node>, ParseError> {
    let mut nodes = Vec::new();
    let mut text = String::new();
    while *pos < chars.len() {
        match chars[*pos] {
            '}' => break,
            '#' if in_plural => {
                if !text.is_empty() {
                    nodes.push(Node::Text(std::mem::take(&mut text)));
                }
                nodes.push(Node::Count);
                *pos += 1;
            }
            '{' => {
                if !text.is_empty() {
                    nodes.push(Node::Text(std::mem::take(&mut text)));
                }
                *pos += 1;
                nodes.push(parse_placeholder(chars, pos, in_plural)?);
            }
            c => {
                text.push(c);
                *pos += 1;
            }
        }
    }
    if !text.is_empty() {
        nodes.push(Node::Text(text));
    }
    Ok(nodes)
}

/// After `{`: `name}`, or `name, select|plural, arms}`.
fn parse_placeholder(chars: &[char], pos: &mut usize, in_plural: bool) -> Result<Node, ParseError> {
    let name = take_until(chars, pos, &[',', '}']).trim().to_string();
    if name.is_empty() {
        return Err(ParseError(format!("empty argument at {pos}")));
    }
    match chars.get(*pos) {
        Some('}') => {
            *pos += 1;
            Ok(Node::Arg(name))
        }
        Some(',') => {
            *pos += 1;
            let kind = take_until(chars, pos, &[',', '}']).trim().to_string();
            if chars.get(*pos) != Some(&',') {
                return Err(ParseError(format!("{kind} without arms at {pos}")));
            }
            *pos += 1;
            let plural = match kind.as_str() {
                "select" => false,
                "plural" => true,
                other => return Err(ParseError(format!("unsupported type {other}"))),
            };
            let mut arms = Vec::new();
            loop {
                skip_whitespace(chars, pos);
                match chars.get(*pos) {
                    Some('}') => {
                        *pos += 1;
                        break;
                    }
                    None => return Err(ParseError("unterminated placeholder".into())),
                    _ => {}
                }
                let key = take_until(chars, pos, &['{', '}']).trim().to_string();
                if chars.get(*pos) != Some(&'{') || key.is_empty() {
                    return Err(ParseError(format!("bad arm at {pos}")));
                }
                *pos += 1;
                let body = parse(chars, pos, plural || in_plural)?;
                if chars.get(*pos) != Some(&'}') {
                    return Err(ParseError("unterminated arm".into()));
                }
                *pos += 1;
                arms.push((key, body));
            }
            Ok(if plural {
                Node::Plural(name, arms)
            } else {
                Node::Select(name, arms)
            })
        }
        _ => Err(ParseError("unterminated argument".into())),
    }
}

fn take_until(chars: &[char], pos: &mut usize, stops: &[char]) -> String {
    let start = *pos;
    while *pos < chars.len() && !stops.contains(&chars[*pos]) {
        *pos += 1;
    }
    chars[start..*pos].iter().collect()
}

fn skip_whitespace(chars: &[char], pos: &mut usize) {
    while *pos < chars.len() && chars[*pos].is_whitespace() {
        *pos += 1;
    }
}

fn render(nodes: &[Node], args: &HashMap<&str, &Arg>, count: Option<i64>, out: &mut String) {
    for node in nodes {
        match node {
            Node::Text(text) => out.push_str(text),
            Node::Arg(name) => {
                if let Some(arg) = args.get(name.as_str()) {
                    out.push_str(&arg.text());
                }
            }
            Node::Count => {
                if let Some(n) = count {
                    out.push_str(&n.to_string());
                }
            }
            Node::Select(name, arms) => {
                let value = args.get(name.as_str()).map(|a| a.text());
                let arm = arms
                    .iter()
                    .find(|(key, _)| Some(key) == value.as_ref())
                    .or_else(|| arms.iter().find(|(key, _)| key == "other"));
                if let Some((_, body)) = arm {
                    render(body, args, count, out);
                }
            }
            Node::Plural(name, arms) => {
                let n = match args.get(name.as_str()) {
                    Some(Arg::Num(n)) => *n,
                    Some(arg) => arg.text().parse().unwrap_or(0),
                    None => 0,
                };
                let exact = format!("={n}");
                let category = if n == 1 { "one" } else { "other" };
                let arm = arms
                    .iter()
                    .find(|(key, _)| *key == exact)
                    .or_else(|| arms.iter().find(|(key, _)| key == category))
                    .or_else(|| arms.iter().find(|(key, _)| key == "other"));
                if let Some((_, body)) = arm {
                    render(body, args, Some(n), out);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const READ_MORE: &str = "{ HAS_UNREAD_AND_NEW, select,\n  true {\n    { UNREAD, plural,\n         =0 {}\n        one {There is <a href=\"{unreadUrl}\"># unread</a>}\n      other {There are <a href=\"{unreadUrl}\"># unread</a>}\n    }\n    { NEW, plural,\n         =0 {}\n        one { and <a href=\"{newUrl}\"># new</a> topic remaining,}\n      other { and <a href=\"{newUrl}\"># new</a> topics remaining,}\n    }\n  }\n  false {\n    { UNREAD, plural,\n         =0 {}\n        one {There is <a href=\"{unreadUrl}\"># unread</a> topic remaining,}\n      other {There are <a href=\"{unreadUrl}\"># unread</a> topics remaining,}\n    }\n    { NEW, plural,\n         =0 {}\n        one {There is <a href=\"{newUrl}\"># new</a> topic remaining,}\n      other {There are <a href=\"{newUrl}\"># new</a> topics remaining,}\n    }\n  }\n  other {}\n}\n{ HAS_CATEGORY, select,\n  true { or browse other topics in {categoryLink}}\n  false { or <a href=\"{basePath}/latest\">view latest topics</a>}\n  other {}\n}\n";

    fn collapse(s: &str) -> String {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn formats_the_read_more_message() {
        let out = format(
            READ_MORE,
            &[
                ("HAS_UNREAD_AND_NEW", Arg::Bool(false)),
                ("UNREAD", Arg::Num(0)),
                ("NEW", Arg::Num(3)),
                ("HAS_CATEGORY", Arg::Bool(true)),
                ("categoryLink", Arg::Str("<a>General</a>")),
                ("newUrl", Arg::Str("/new?subset=topics")),
                ("unreadUrl", Arg::Str("/new?subset=replies")),
            ],
        )
        .unwrap();
        assert_eq!(
            collapse(&out),
            "There are <a href=\"/new?subset=topics\">3 new</a> topics remaining, or browse other topics in <a>General</a>"
        );

        let out = format(
            READ_MORE,
            &[
                ("HAS_UNREAD_AND_NEW", Arg::Bool(true)),
                ("UNREAD", Arg::Num(1)),
                ("NEW", Arg::Num(1)),
                ("HAS_CATEGORY", Arg::Bool(false)),
                ("basePath", Arg::Str("")),
                ("newUrl", Arg::Str("/new")),
                ("unreadUrl", Arg::Str("/unread")),
            ],
        )
        .unwrap();
        assert_eq!(
            collapse(&out),
            "There is <a href=\"/unread\">1 unread</a> and <a href=\"/new\">1 new</a> topic remaining, or <a href=\"/latest\">view latest topics</a>"
        );
    }

    #[test]
    fn rejects_unbalanced_messages() {
        assert!(format("{a, select, true {x}", &[]).is_err());
        assert!(format("x}", &[]).is_err());
        assert!(format("{a, number}", &[]).is_err());
    }
}
