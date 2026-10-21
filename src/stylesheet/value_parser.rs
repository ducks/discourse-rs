//! postcss-value-parser 4.2.0's `parse` and `stringify` (MIT), which
//! autoprefixer's transition handling runs on declaration values: the
//! nodes keep every raw, so a value parsed and written out again is the
//! same text.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Word,
    Space,
    String,
    Comment,
    Div,
    Function,
    UnicodeRange,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: Kind,
    pub value: String,
    /// A div's or function's raws around its value or contents.
    pub before: String,
    pub after: String,
    /// A string's quote.
    pub quote: char,
    pub unclosed: bool,
    /// A function's contents.
    pub nodes: Vec<Node>,
}

impl Node {
    fn new(kind: Kind, value: String) -> Node {
        Node {
            kind,
            value,
            before: String::new(),
            after: String::new(),
            quote: '"',
            unclosed: false,
            nodes: Vec::new(),
        }
    }

    pub fn word(value: &str) -> Node {
        Node::new(Kind::Word, value.to_string())
    }

    /// The div autoprefixer makes when a value has none: `, `.
    pub fn comma() -> Node {
        let mut div = Node::new(Kind::Div, ",".to_string());
        div.after = " ".to_string();
        div
    }
}

fn is_unicode_range(s: &[char]) -> bool {
    !s.is_empty()
        && s.iter()
            .all(|c| c.is_ascii_hexdigit() || *c == '?' || *c == '-')
}

fn text(chars: &[char], from: usize, to: usize) -> String {
    chars[from.min(chars.len())..to.min(chars.len())]
        .iter()
        .collect()
}

/// postcss-value-parser's `parse`.
pub fn parse(input: &str) -> Vec<Node> {
    let mut value: Vec<char> = input.chars().collect();
    let max = value.len();
    let code_at = |value: &[char], i: usize| -> Option<char> { value.get(i).copied() };
    let ws = |c: Option<char>| c.is_some_and(|c| (c as u32) <= 32);
    // The containers being filled: the root, then each open function.
    let mut stack: Vec<Node> = vec![Node::new(Kind::Function, String::new())];
    let mut pos = 0;
    let mut name = String::new();
    let mut before = String::new();
    let mut after = String::new();
    let mut closed_to_root = false;
    let in_calc =
        |stack: &[Node]| stack.len() > 1 && stack.last().is_some_and(|p| p.value == "calc");
    while pos < max {
        let code = code_at(&value, pos);
        let c = code.unwrap_or('\0');
        if ws(code) {
            let mut next = pos;
            loop {
                next += 1;
                if !ws(code_at(&value, next)) {
                    break;
                }
            }
            let token = text(&value, pos, next);
            let next_code = code_at(&value, next);
            let balanced = stack.len() > 1;
            let slash_div = next_code == Some('/')
                && code_at(&value, next + 1) != Some('*')
                && if stack.len() == 1 {
                    // `!parent`: true at the root until a function closes,
                    // when the parser's `parent` becomes the root itself.
                    !closed_to_root
                } else {
                    stack.last().is_some_and(|p| p.value != "calc")
                };
            let tokens = &mut stack.last_mut().expect("a container").nodes;
            let prev_div = tokens.last().is_some_and(|p| p.kind == Kind::Div);
            if next_code == Some(')') && balanced {
                after = token;
            } else if prev_div {
                tokens.last_mut().expect("a div").after = token;
            } else if next_code == Some(',') || next_code == Some(':') || slash_div {
                before = token;
            } else {
                tokens.push(Node::new(Kind::Space, token));
            }
            pos = next;
        } else if c == '\'' || c == '"' {
            let quote = c;
            let mut node = Node::new(Kind::String, String::new());
            node.quote = quote;
            let mut next = pos;
            loop {
                let mut escape = false;
                match value[next + 1..].iter().position(|&x| x == quote) {
                    Some(found) => {
                        next = next + 1 + found;
                        let mut escape_pos = next;
                        while escape_pos > 0 && value[escape_pos - 1] == '\\' {
                            escape_pos -= 1;
                            escape = !escape;
                        }
                    }
                    None => {
                        value.push(quote);
                        next = value.len() - 1;
                        node.unclosed = true;
                    }
                }
                if !escape {
                    break;
                }
            }
            node.value = text(&value, pos + 1, next);
            stack.last_mut().expect("a container").nodes.push(node);
            pos = next + 1;
        } else if c == '/' && code_at(&value, pos + 1) == Some('*') {
            let mut node = Node::new(Kind::Comment, String::new());
            let found = value[pos..]
                .windows(2)
                .position(|w| w == ['*', '/'])
                .map(|p| pos + p);
            let next = match found {
                Some(next) => next,
                None => {
                    node.unclosed = true;
                    value.len()
                }
            };
            node.value = text(&value, pos + 2, next);
            stack.last_mut().expect("a container").nodes.push(node);
            pos = next + 2;
        } else if (c == '/' || c == '*') && in_calc(&stack) {
            stack
                .last_mut()
                .expect("a container")
                .nodes
                .push(Node::word(&c.to_string()));
            pos += 1;
        } else if c == '/' || c == ',' || c == ':' {
            let mut node = Node::new(Kind::Div, c.to_string());
            node.before = std::mem::take(&mut before);
            stack.last_mut().expect("a container").nodes.push(node);
            pos += 1;
        } else if c == '(' {
            let mut next = pos;
            loop {
                next += 1;
                if !ws(code_at(&value, next)) {
                    break;
                }
            }
            let open = pos;
            let mut node = Node::new(Kind::Function, std::mem::take(&mut name));
            node.before = text(&value, open + 1, next);
            pos = next;
            let code = code_at(&value, pos);
            if node.value == "url" && code != Some('\'') && code != Some('"') {
                let mut next = next as isize - 1;
                loop {
                    let mut escape = false;
                    let from = (next + 1) as usize;
                    match value
                        .get(from..)
                        .and_then(|rest| rest.iter().position(|&x| x == ')'))
                    {
                        Some(found) => {
                            next = (from + found) as isize;
                            let mut escape_pos = next as usize;
                            while escape_pos > 0 && value[escape_pos - 1] == '\\' {
                                escape_pos -= 1;
                                escape = !escape;
                            }
                        }
                        None => {
                            value.push(')');
                            next = value.len() as isize - 1;
                            node.unclosed = true;
                        }
                    }
                    if !escape {
                        break;
                    }
                }
                let next = next as usize;
                // Whitespace before the close.
                let mut ws_pos = next;
                loop {
                    if ws_pos == 0 {
                        break;
                    }
                    ws_pos -= 1;
                    if !ws(code_at(&value, ws_pos)) {
                        break;
                    }
                }
                if open < ws_pos {
                    if pos != ws_pos + 1 {
                        node.nodes = vec![Node::word(&text(&value, pos, ws_pos + 1))];
                    }
                    if node.unclosed && ws_pos + 1 != next {
                        node.nodes
                            .push(Node::new(Kind::Space, text(&value, ws_pos + 1, next)));
                    } else {
                        node.after = text(&value, ws_pos + 1, next);
                    }
                }
                pos = next + 1;
                stack.last_mut().expect("a container").nodes.push(node);
            } else {
                stack.push(node);
            }
        } else if c == ')' && stack.len() > 1 {
            pos += 1;
            let mut done = stack.pop().expect("an open function");
            closed_to_root = stack.len() == 1;
            done.after = std::mem::take(&mut after);
            stack.last_mut().expect("a container").nodes.push(done);
        } else {
            let mut next = pos;
            loop {
                if code_at(&value, next) == Some('\\') {
                    next += 1;
                }
                next += 1;
                let code = code_at(&value, next);
                let stop = next >= max
                    || ws(code)
                    || matches!(code, Some('\'' | '"' | ',' | ':' | '/' | '('))
                    || (code == Some('*') && in_calc(&stack))
                    || (code == Some(')') && stack.len() > 1);
                if stop {
                    break;
                }
            }
            let token = text(&value, pos, next);
            if code_at(&value, next) == Some('(') {
                name = token;
            } else {
                let chars: Vec<char> = token.chars().collect();
                let kind = if matches!(chars.first(), Some('u' | 'U'))
                    && chars.get(1) == Some(&'+')
                    && is_unicode_range(&chars[2..])
                {
                    Kind::UnicodeRange
                } else {
                    Kind::Word
                };
                stack
                    .last_mut()
                    .expect("a container")
                    .nodes
                    .push(Node::new(kind, token));
            }
            pos = next;
        }
    }
    // Functions left open.
    while stack.len() > 1 {
        let mut open = stack.pop().expect("an open function");
        open.unclosed = true;
        stack.last_mut().expect("a container").nodes.push(open);
    }
    stack.pop().expect("the root").nodes
}

/// postcss-value-parser's `stringify`.
pub fn stringify(nodes: &[Node]) -> String {
    nodes.iter().map(stringify_node).collect()
}

fn stringify_node(node: &Node) -> String {
    match node.kind {
        Kind::Word | Kind::Space | Kind::UnicodeRange => node.value.clone(),
        Kind::String => {
            let quote = node.quote.to_string();
            format!(
                "{quote}{}{}",
                node.value,
                if node.unclosed { "" } else { &quote }
            )
        }
        Kind::Comment => format!("/*{}{}", node.value, if node.unclosed { "" } else { "*/" }),
        Kind::Div => format!("{}{}{}", node.before, node.value, node.after),
        Kind::Function => format!(
            "{}({}{}{}{}",
            node.value,
            node.before,
            stringify(&node.nodes),
            node.after,
            if node.unclosed { "" } else { ")" }
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_come_back_as_they_were() {
        for value in [
            "transform .3s ease,opacity .3s",
            "all 0.2s cubic-bezier(0.4, 0, 0.2, 1) , color 1s",
            "url( a b.png ) , url(\"x\")",
            "calc(1px + 2px*3) / 2",
            "'a\\'b' \"c",
            "f(g(h)",
            "a/* c */b",
            "u+0025-00FF",
        ] {
            assert_eq!(stringify(&parse(value)), value);
        }
    }

    #[test]
    fn splits_as_the_parser_does() {
        let nodes = parse("transform .3s,opacity 1s");
        let kinds: Vec<Kind> = nodes.iter().map(|n| n.kind).collect();
        assert_eq!(
            kinds,
            [
                Kind::Word,
                Kind::Space,
                Kind::Word,
                Kind::Div,
                Kind::Word,
                Kind::Space,
                Kind::Word
            ]
        );
        let nodes = parse("a , b");
        assert_eq!(nodes[1].before, " ");
        assert_eq!(nodes[1].after, " ");
    }
}
