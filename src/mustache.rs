//! Mustache, as the Ruby mustache gem renders the templates Discourse
//! ships (the onebox templates): `{{name}}` escaped with CGI.escapeHTML,
//! `{{{name}}}` and `{{&name}}` as they are, dotted names, sections over
//! a value, a list or an object, inverted sections and comments; a line
//! holding only a section, inverted, closing or comment tag is dropped
//! with its line break (the spec's standalone tags). Partials, lambdas
//! and delimiter changes aren't used by those templates and aren't
//! supported.
//!
//! The context is JSON: `null` is Ruby's nil. As the gem has it, a
//! section is skipped only for nil and false (an empty string renders
//! it), and an inverted one is rendered for nil, false and an empty list
//! or object.

use serde_json::Value;

enum Node {
    Text(String),
    Escaped(String),
    Raw(String),
    Section(String, Vec<Node>),
    Inverted(String, Vec<Node>),
}

/// A tag found in the template, with where its line's standalone
/// whitespace begins and ends.
struct Tag<'a> {
    kind: char,
    name: &'a str,
    /// Where the tag starts and ends in the template.
    start: usize,
    end: usize,
}

fn tags(template: &str) -> Vec<Tag<'_>> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(found) = template[at..].find("{{") {
        let start = at + found;
        let (kind, inner_start, close) = if template[start..].starts_with("{{{") {
            ('{', start + 3, "}}}")
        } else {
            let k = template[start + 2..].chars().next().unwrap_or(' ');
            match k {
                '#' | '^' | '/' | '!' | '&' => (k, start + 3, "}}"),
                _ => (' ', start + 2, "}}"),
            }
        };
        let Some(len) = template[inner_start..].find(close) else {
            break;
        };
        let end = inner_start + len + close.len();
        out.push(Tag {
            kind,
            name: template[inner_start..inner_start + len].trim(),
            start,
            end,
        });
        at = end;
    }
    out
}

/// Whether a tag stands alone on its line, and if so the span to drop:
/// the line's leading whitespace through its line break.
fn standalone(template: &str, tag: &Tag) -> Option<(usize, usize)> {
    if !matches!(tag.kind, '#' | '^' | '/' | '!') {
        return None;
    }
    let line_start = template[..tag.start].rfind('\n').map_or(0, |i| i + 1);
    if !template[line_start..tag.start]
        .chars()
        .all(|c| c == ' ' || c == '\t')
    {
        return None;
    }
    let rest = &template[tag.end..];
    let line_end = rest.find('\n').map_or(template.len(), |i| tag.end + i + 1);
    let trailing = &template[tag.end..line_end];
    if !trailing
        .trim_end_matches(['\n', '\r'])
        .chars()
        .all(|c| c == ' ' || c == '\t')
    {
        return None;
    }
    Some((line_start, line_end))
}

fn parse(template: &str) -> Vec<Node> {
    let all = tags(template);
    // (kind, name, children) frames; the root has kind ' '.
    let mut stack: Vec<(char, String, Vec<Node>)> = vec![(' ', String::new(), Vec::new())];
    let mut at = 0;
    for tag in &all {
        let (text_end, next) = match standalone(template, tag) {
            Some((s, e)) => (s, e),
            None => (tag.start, tag.end),
        };
        if text_end > at {
            stack
                .last_mut()
                .unwrap()
                .2
                .push(Node::Text(template[at..text_end].to_string()));
        }
        at = next.max(at);
        let name = tag.name.to_string();
        match tag.kind {
            '#' | '^' => stack.push((tag.kind, name, Vec::new())),
            '/' => {
                if stack.len() > 1 {
                    let (kind, open, children) = stack.pop().unwrap();
                    let node = if kind == '^' {
                        Node::Inverted(open, children)
                    } else {
                        Node::Section(open, children)
                    };
                    stack.last_mut().unwrap().2.push(node);
                }
            }
            '!' => {}
            '{' | '&' => stack.last_mut().unwrap().2.push(Node::Raw(name)),
            _ => stack.last_mut().unwrap().2.push(Node::Escaped(name)),
        }
    }
    if at < template.len() {
        stack
            .last_mut()
            .unwrap()
            .2
            .push(Node::Text(template[at..].to_string()));
    }
    while stack.len() > 1 {
        let (_, _, children) = stack.pop().unwrap();
        stack.last_mut().unwrap().2.extend(children);
    }
    stack.pop().unwrap().2
}

/// `Context#fetch`: the name looked up from the innermost context out; a
/// dotted name's first part found so, the rest inside it.
fn lookup<'a>(stack: &[&'a Value], name: &str) -> Option<&'a Value> {
    if name == "." {
        return stack.last().copied();
    }
    let mut parts = name.split('.');
    let first = parts.next()?;
    let mut value = stack.iter().rev().find_map(|ctx| ctx.get(first))?;
    for part in parts {
        value = value.get(part)?;
    }
    Some(value)
}

/// `value.to_s`
fn to_s(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

fn render_nodes(nodes: &[Node], stack: &mut Vec<&Value>, out: &mut String) {
    for node in nodes {
        match node {
            Node::Text(t) => out.push_str(t),
            Node::Escaped(name) => out.push_str(&crate::discourse_diff::escape_html(&to_s(
                lookup(stack, name),
            ))),
            Node::Raw(name) => out.push_str(&to_s(lookup(stack, name))),
            Node::Section(name, children) => match lookup(stack, name) {
                None | Some(Value::Null) | Some(Value::Bool(false)) => {}
                Some(Value::Bool(true)) => render_nodes(children, stack, out),
                Some(Value::Array(items)) => {
                    for item in items {
                        stack.push(item);
                        render_nodes(children, stack, out);
                        stack.pop();
                    }
                }
                Some(value) => {
                    stack.push(value);
                    render_nodes(children, stack, out);
                    stack.pop();
                }
            },
            Node::Inverted(name, children) => {
                let empty = match lookup(stack, name) {
                    None | Some(Value::Null) | Some(Value::Bool(false)) => true,
                    Some(Value::Array(a)) => a.is_empty(),
                    Some(Value::Object(o)) => o.is_empty(),
                    Some(_) => false,
                };
                if empty {
                    render_nodes(children, stack, out);
                }
            }
        }
    }
}

/// `Mustache.render(template, context)`
pub fn render(template: &str, context: &Value) -> String {
    let nodes = parse(template);
    let mut out = String::new();
    render_nodes(&nodes, &mut vec![context], &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn interpolates_and_escapes_as_cgi() {
        assert_eq!(
            render(
                "<a href=\"{{u}}\">{{{t}}}</a>{{&t}}",
                &json!({"u": "a&b\"'", "t": "<b>"})
            ),
            "<a href=\"a&amp;b&quot;&#39;\"><b></a><b>"
        );
        assert_eq!(render("[{{missing}}]", &json!({})), "[]");
        assert_eq!(render("{{n}}", &json!({"n": 3})), "3");
    }

    #[test]
    fn sections_as_the_gem_has_them() {
        let t = "{{#s}}yes{{/s}}{{^s}}no{{/s}}";
        assert_eq!(render(t, &json!({"s": true})), "yes");
        assert_eq!(render(t, &json!({"s": false})), "no");
        assert_eq!(render(t, &json!({"s": null})), "no");
        // An empty string renders a section and not its inverse.
        assert_eq!(render(t, &json!({"s": ""})), "yes");
        assert_eq!(render(t, &json!({"s": []})), "no");
        assert_eq!(
            render(
                "{{#l}}<{{x}}{{y}}>{{/l}}",
                &json!({"y": "!", "l": [{"x": 1}, {"x": 2}]})
            ),
            "<1!><2!>"
        );
        assert_eq!(
            render("{{#o}}{{a.b}}{{/o}}", &json!({"o": {"a": {"b": "c"}}})),
            "c"
        );
    }

    #[test]
    fn drops_standalone_tag_lines() {
        let t = "<p>\n  {{#a}}\n  <b>{{a}}</b>\n  {{/a}}\n  {{! note }}\n</p>\n";
        assert_eq!(render(t, &json!({"a": "x"})), "<p>\n  <b>x</b>\n</p>\n");
        assert_eq!(render(t, &json!({})), "<p>\n</p>\n");
        // Not alone on its line: kept in place.
        assert_eq!(
            render("a {{#s}}b{{/s}} c\n", &json!({"s": true})),
            "a b c\n"
        );
    }
}
