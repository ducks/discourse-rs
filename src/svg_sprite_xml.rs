//! `SvgSprite.symbols_for(name, sprite, strict: false)` for a theme's own
//! sprite: the `<symbol>`s of an SVG document, each without its `<title>`s
//! and serialized as Nokogiri's `to_xml` prints a node of a document
//! parsed with its blank text kept (libxml2 with FORMAT: an element whose
//! children hold no text puts each child on its own line, indented two
//! spaces a level; one holding text prints its children as they are).

/// An element still open: its name, attributes and children so far.
type Open = (String, Vec<(String, String)>, Vec<Node>);

/// A start tag: name, attributes, self-closing, bytes taken.
type StartTag = (String, Vec<(String, String)>, bool, usize);

/// A parsed node.
enum Node {
    Element {
        name: String,
        attrs: Vec<(String, String)>,
        children: Vec<Node>,
    },
    Text(String),
    /// A comment or CDATA section, as written.
    Raw(String),
}

/// The document's symbols as (id, xml), in document order; symbols
/// without an id are left out. None when the document doesn't parse.
pub fn symbols(sprite: &str) -> Option<Vec<(String, String)>> {
    let nodes = parse(sprite)?;
    let mut out = Vec::new();
    collect(&nodes, &mut out);
    Some(out)
}

fn collect(nodes: &[Node], out: &mut Vec<(String, String)>) {
    for node in nodes {
        if let Node::Element {
            name,
            attrs,
            children,
        } = node
        {
            if name == "symbol"
                && let Some((_, id)) = attrs.iter().find(|(k, _)| k == "id")
                && !id.trim().is_empty()
            {
                let mut xml = String::new();
                serialize(node, 0, &mut xml);
                out.push((id.clone(), xml));
            }
            collect(children, out);
        }
    }
}

fn is_title(node: &Node) -> bool {
    matches!(node, Node::Element { name, .. } if name == "title")
}

fn serialize(node: &Node, level: usize, out: &mut String) {
    match node {
        Node::Text(text) => out.push_str(&escape_text(text)),
        Node::Raw(raw) => out.push_str(raw),
        Node::Element {
            name,
            attrs,
            children,
        } => {
            out.push('<');
            out.push_str(name);
            for (k, v) in attrs {
                out.push_str(&format!(" {k}=\"{}\"", escape_attr(v)));
            }
            let kept: Vec<&Node> = children.iter().filter(|c| !is_title(c)).collect();
            if kept.is_empty() {
                out.push_str("/>");
                return;
            }
            out.push('>');
            let format = !kept.iter().any(|c| matches!(c, Node::Text(_)));
            for child in &kept {
                if format {
                    out.push('\n');
                    out.push_str(&"  ".repeat(level + 1));
                }
                serialize(child, level + 1, out);
            }
            if format {
                out.push('\n');
                out.push_str(&"  ".repeat(level));
            }
            out.push_str(&format!("</{name}>"));
        }
    }
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\r', "&#13;")
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\n', "&#10;")
        .replace('\r', "&#13;")
        .replace('\t', "&#9;")
}

/// XML's predefined entities and character references.
fn decode(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let end = rest[i..].find(';')? + i;
        let entity = &rest[i + 1..end];
        let c = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            e if e.starts_with("#x") || e.starts_with("#X") => {
                char::from_u32(u32::from_str_radix(&e[2..], 16).ok()?)?
            }
            e if e.starts_with('#') => char::from_u32(e[1..].parse().ok()?)?,
            _ => return None,
        };
        out.push(c);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// A small XML parser: elements, attributes, text, comments and CDATA;
/// the declaration, processing instructions and doctype are skipped.
fn parse(src: &str) -> Option<Vec<Node>> {
    let mut stack: Vec<Open> = Vec::new();
    let mut top: Vec<Node> = Vec::new();
    let mut rest = src;
    fn push(stack: &mut [Open], top: &mut Vec<Node>, node: Node) {
        match stack.last_mut() {
            Some((_, _, children)) => children.push(node),
            None => top.push(node),
        }
    }
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("<!--") {
            let end = after.find("-->")?;
            push(
                &mut stack,
                &mut top,
                Node::Raw(format!("<!--{}-->", &after[..end])),
            );
            rest = &after[end + 3..];
        } else if let Some(after) = rest.strip_prefix("<![CDATA[") {
            let end = after.find("]]>")?;
            push(
                &mut stack,
                &mut top,
                Node::Raw(format!("<![CDATA[{}]]>", &after[..end])),
            );
            rest = &after[end + 3..];
        } else if rest.starts_with("<?") {
            rest = &rest[rest.find("?>")? + 2..];
        } else if rest.starts_with("<!") {
            rest = &rest[rest.find('>')? + 1..];
        } else if let Some(after) = rest.strip_prefix("</") {
            let end = after.find('>')?;
            let name = after[..end].trim();
            let (open, attrs, children) = stack.pop()?;
            if open != name {
                return None;
            }
            push(
                &mut stack,
                &mut top,
                Node::Element {
                    name: open,
                    attrs,
                    children,
                },
            );
            rest = &after[end + 1..];
        } else if let Some(after) = rest.strip_prefix('<') {
            let (name, attrs, self_closing, consumed) = start_tag(after)?;
            rest = &after[consumed..];
            if self_closing {
                push(
                    &mut stack,
                    &mut top,
                    Node::Element {
                        name,
                        attrs,
                        children: Vec::new(),
                    },
                );
            } else {
                stack.push((name, attrs, Vec::new()));
            }
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            if !stack.is_empty() {
                let text = decode(&rest[..end])?;
                push(&mut stack, &mut top, Node::Text(text));
            }
            rest = &rest[end..];
        }
    }
    stack.is_empty().then_some(top)
}

/// A start tag after its `<`: the name, attributes (decoded), whether it
/// closes itself, and the bytes it took.
fn start_tag(s: &str) -> Option<StartTag> {
    let name_end = s.find(|c: char| c.is_whitespace() || c == '>' || c == '/')?;
    let name = s[..name_end].to_string();
    if name.is_empty() {
        return None;
    }
    let mut i = name_end;
    let mut attrs = Vec::new();
    loop {
        let tail = &s[i..];
        let trimmed = tail.trim_start();
        i += tail.len() - trimmed.len();
        if trimmed.starts_with("/>") {
            return Some((name, attrs, true, i + 2));
        }
        if trimmed.starts_with('>') {
            return Some((name, attrs, false, i + 1));
        }
        let eq = trimmed.find('=')?;
        let key = trimmed[..eq].trim().to_string();
        let after = trimmed[eq + 1..].trim_start();
        let quote = after.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let close = after[1..].find(quote)? + 1;
        let value = decode(&after[1..close])?;
        attrs.push((key, value));
        i += trimmed.len() - after.len() + close + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_without_titles_as_nokogiri_prints_them() {
        let sprite = "<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\">\n  <symbol id=\"a\" viewBox=\"0 0 1 1\"><title>A</title><path d=\"M0 0\"/><path d=\"M1&amp;1\"/></symbol>\n  <symbol id=\"b\">\n    <path d=\"x\"/>\n  </symbol>\n  <symbol><path/></symbol>\n</svg>";
        let found = symbols(sprite).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].0, "a");
        assert_eq!(
            found[0].1,
            "<symbol id=\"a\" viewBox=\"0 0 1 1\">\n  <path d=\"M0 0\"/>\n  <path d=\"M1&amp;1\"/>\n</symbol>"
        );
        assert_eq!(
            found[1].1,
            "<symbol id=\"b\">\n    <path d=\"x\"/>\n  </symbol>"
        );
        assert!(symbols("<svg><symbol id=\"x\"></svg>").is_none());
    }

    /// The reference's `SvgSprite.symbols_for("theme_1_2.svg", sprite,
    /// strict: false)` for this sprite.
    #[test]
    fn as_the_reference_reads_a_theme_sprite() {
        let sprite = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE svg>\n<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\">\n  <defs>\n    <symbol id=\"my-one\" viewBox=\"0 0 10 10\"><title>One &amp; only</title><path d=\"M0 0L10 10\"/><g fill='red'><circle cx=\"5\" cy=\"5\" r=\"2\"/></g></symbol>\n    <symbol id=\"my-two\" viewBox=\"0 0 10 10\">\n      <title>Two</title>\n      <use xlink:href=\"#my-one\" x=\"1\"/>\n      <!-- a comment -->\n      <text x=\"1\">a &lt; b</text>\n    </symbol>\n    <symbol viewBox=\"0 0 1 1\"><path d=\"M1 1\"/></symbol>\n  </defs>\n</svg>\n";
        let found = symbols(sprite).unwrap();
        assert_eq!(
            found,
            vec![
                (
                    "my-one".to_string(),
                    "<symbol id=\"my-one\" viewBox=\"0 0 10 10\">\n  <path d=\"M0 0L10 10\"/>\n  <g fill=\"red\">\n    <circle cx=\"5\" cy=\"5\" r=\"2\"/>\n  </g>\n</symbol>".to_string()
                ),
                (
                    "my-two".to_string(),
                    "<symbol id=\"my-two\" viewBox=\"0 0 10 10\">\n      \n      <use xlink:href=\"#my-one\" x=\"1\"/>\n      <!-- a comment -->\n      <text x=\"1\">a &lt; b</text>\n    </symbol>".to_string()
                ),
            ]
        );
    }
}
