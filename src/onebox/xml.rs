//! A small XML reader for the well-formed documents the PubMed engine
//! reads with `Nokogiri.XML`: elements, attributes, text, CDATA and the
//! predefined and numeric entities; declarations, doctypes, comments and
//! processing instructions are skipped. Anything malformed is None (the
//! caller refuses rather than guessing at libxml2's recovery).

#[derive(Debug, Clone)]
pub enum Node {
    Element(Element),
    Text(String),
}

#[derive(Debug, Clone)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

impl Element {
    /// `node.text`: every descendant's text.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for c in &self.children {
            match c {
                Node::Text(t) => out.push_str(t),
                Node::Element(e) => out.push_str(&e.text()),
            }
        }
        out
    }

    /// `css(name)`: the descendant elements so named, in document order.
    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a Element>) {
        for c in &self.children {
            if let Node::Element(e) = c {
                if e.name == name {
                    out.push(e);
                }
                e.find_all(name, out);
            }
        }
    }

    pub fn descendants_named(&self, name: &str) -> Vec<&Element> {
        let mut out = Vec::new();
        self.find_all(name, &mut out);
        out
    }
}

fn decode(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let end = after.find(';')?;
        let name = &after[..end];
        let c = match name {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code =
                    if let Some(h) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                        u32::from_str_radix(h, 16).ok()?
                    } else {
                        name.strip_prefix('#')?.parse().ok()?
                    };
                char::from_u32(code)?
            }
        };
        out.push(c);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

struct Parser<'a> {
    s: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        &self.s[self.pos..]
    }

    fn skip_until(&mut self, end: &str) -> Option<()> {
        let i = self.rest().find(end)?;
        self.pos += i + end.len();
        Some(())
    }

    fn skip_ws(&mut self) {
        let n = self.rest().len() - self.rest().trim_start().len();
        self.pos += n;
    }

    fn name(&mut self) -> Option<String> {
        let r = self.rest();
        let end = r
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/' || c == '=')
            .unwrap_or(r.len());
        if end == 0 {
            return None;
        }
        self.pos += end;
        Some(r[..end].to_string())
    }

    /// Children until `</close>` (or the end for the document).
    fn children(&mut self, close: Option<&str>) -> Option<Vec<Node>> {
        let mut out = Vec::new();
        loop {
            if self.pos >= self.s.len() {
                return close.is_none().then_some(out);
            }
            let r = self.rest();
            if r.starts_with("</") {
                self.pos += 2;
                let name = self.name()?;
                self.skip_ws();
                if !self.rest().starts_with('>') || Some(name.as_str()) != close {
                    return None;
                }
                self.pos += 1;
                return Some(out);
            } else if r.starts_with("<!--") {
                self.skip_until("-->")?;
            } else if r.starts_with("<![CDATA[") {
                self.pos += 9;
                let i = self.rest().find("]]>")?;
                out.push(Node::Text(self.rest()[..i].to_string()));
                self.pos += i + 3;
            } else if r.starts_with("<!") || r.starts_with("<?") {
                self.skip_until(">")?;
            } else if r.starts_with('<') {
                self.pos += 1;
                let name = self.name()?;
                let mut attrs = Vec::new();
                loop {
                    self.skip_ws();
                    let r = self.rest();
                    if r.starts_with("/>") {
                        self.pos += 2;
                        out.push(Node::Element(Element {
                            name,
                            attrs,
                            children: Vec::new(),
                        }));
                        break;
                    }
                    if r.starts_with('>') {
                        self.pos += 1;
                        let children = self.children(Some(&name))?;
                        out.push(Node::Element(Element {
                            name,
                            attrs,
                            children,
                        }));
                        break;
                    }
                    let attr = self.name()?;
                    self.skip_ws();
                    if !self.rest().starts_with('=') {
                        return None;
                    }
                    self.pos += 1;
                    self.skip_ws();
                    let quote = self.rest().chars().next()?;
                    if quote != '"' && quote != '\'' {
                        return None;
                    }
                    self.pos += 1;
                    let i = self.rest().find(quote)?;
                    let value = decode(&self.rest()[..i])?;
                    self.pos += i + 1;
                    attrs.push((attr, value));
                }
            } else {
                let i = r.find('<').unwrap_or(r.len());
                out.push(Node::Text(decode(&r[..i])?));
                self.pos += i;
            }
        }
    }
}

/// `Nokogiri.XML(s)`: the document's nodes under a nameless root.
pub fn parse(s: &str) -> Option<Element> {
    let mut p = Parser { s, pos: 0 };
    let children = p.children(None)?;
    Some(Element {
        name: String::new(),
        attrs: Vec::new(),
        children,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_elements_and_entities() {
        let doc = parse(
            "<?xml version=\"1.0\"?><!DOCTYPE x><pre>&lt;a b=\"1\"&gt;x&amp;amp;y&lt;/a&gt;</pre>",
        )
        .unwrap();
        assert_eq!(
            doc.descendants_named("pre")[0].text(),
            "<a b=\"1\">x&amp;y</a>"
        );
        let inner = parse(&format!("<root>{}</root>", doc.text())).unwrap();
        assert_eq!(inner.descendants_named("a")[0].text(), "x&y");
        assert!(parse("<a><b></a>").is_none());
    }
}
