//! `Crass::Parser` and `Crass::TokenScanner` (lib/crass/parser.rb,
//! lib/crass/token-scanner.rb).

use super::tokenizer::{Options, Tokenizer};
use super::{
    AtRule, Declaration, ErrorNode, Function, Node, Property, QualifiedRule, Selector, SimpleBlock,
    StyleRule, Token, TokenKind,
};

/// `DEFAULT_MAXIMUM_DEPTH`
const MAXIMUM_DEPTH: usize = 25;

/// `TokenScanner`: a cursor over tokens (or already parsed nodes).
pub struct TokenScanner {
    tokens: Vec<Node>,
    pos: usize,
    current: Option<usize>,
}

impl TokenScanner {
    pub fn new(tokens: Vec<Node>) -> TokenScanner {
        TokenScanner {
            tokens,
            pos: 0,
            current: None,
        }
    }

    fn consume(&mut self) -> Option<Node> {
        if self.pos < self.tokens.len() {
            self.current = Some(self.pos);
            self.pos += 1;
            Some(self.tokens[self.pos - 1].clone())
        } else {
            self.current = None;
            None
        }
    }

    fn peek(&self) -> Option<&Node> {
        self.tokens.get(self.pos)
    }

    fn current(&self) -> Option<&Node> {
        self.current.map(|i| &self.tokens[i])
    }

    fn reconsume(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
        }
    }

    /// What `collect` returns: the tokens consumed since `start`.
    fn since(&self, start: usize) -> Vec<Node> {
        self.tokens
            .get(start..self.pos)
            .map(<[Node]>::to_vec)
            .unwrap_or_default()
    }
}

fn token_kind(node: Option<&Node>) -> Option<&TokenKind> {
    match node {
        Some(Node::Token(t)) => Some(&t.kind),
        _ => None,
    }
}

fn token_value(node: Option<&Node>) -> String {
    match node {
        Some(Node::Token(t)) => t.value.clone(),
        _ => String::new(),
    }
}

fn is_whitespace_token(node: Option<&Node>) -> bool {
    matches!(token_kind(node), Some(TokenKind::Whitespace))
}

/// `BLOCK_END_TOKENS`
fn block_end(kind: &TokenKind) -> Option<TokenKind> {
    match kind {
        TokenKind::LBrace => Some(TokenKind::RBrace),
        TokenKind::LBracket => Some(TokenKind::RBracket),
        TokenKind::LParen => Some(TokenKind::RParen),
        _ => None,
    }
}

fn kind_str(kind: &TokenKind) -> &'static str {
    match kind {
        TokenKind::LBrace => "{",
        TokenKind::RBrace => "}",
        TokenKind::LBracket => "[",
        TokenKind::RBracket => "]",
        TokenKind::LParen => "(",
        TokenKind::RParen => ")",
        _ => "",
    }
}

fn error(value: &'static str) -> Node {
    Node::Error(ErrorNode {
        value,
        tokens: None,
    })
}

pub struct Parser {
    depth: usize,
    tokens: TokenScanner,
}

impl Parser {
    /// A parser over a CSS string.
    pub fn new(input: &str, options: Options) -> Parser {
        Parser::from_tokens(Tokenizer::tokenize(input, options))
    }

    /// A parser over tokens or parsed nodes.
    pub fn from_tokens(tokens: Vec<Node>) -> Parser {
        Parser {
            depth: 0,
            tokens: TokenScanner::new(tokens),
        }
    }

    fn consume_at_rule(&mut self, input: &mut TokenScanner) -> Node {
        let start = input.pos;
        let name = token_value(input.consume().as_ref());
        let mut prelude = Vec::new();
        let mut block = None;
        while let Some(token) = input.consume() {
            match &token {
                Node::Token(t) if t.kind == TokenKind::Comment => continue,
                Node::Token(t) if t.kind == TokenKind::Semicolon => break,
                Node::Token(t) if t.kind == TokenKind::LBrace => {
                    block = Some(match self.consume_simple_block(input) {
                        Node::SimpleBlock(b) => b.value,
                        // Unreachable at the depth at-rules are consumed.
                        _ => Vec::new(),
                    });
                    break;
                }
                Node::SimpleBlock(b) if b.start == "{" => {
                    block = Some(b.value.clone());
                    break;
                }
                _ => {
                    input.reconsume();
                    if let Some(v) = self.consume_component_value(input) {
                        prelude.push(v);
                    }
                }
            }
        }
        Node::AtRule(AtRule {
            name,
            prelude,
            block,
            tokens: input.since(start),
        })
    }

    fn consume_component_value(&mut self, input: &mut TokenScanner) -> Option<Node> {
        let token = input.consume()?;
        Some(match &token {
            Node::Token(t)
                if matches!(
                    t.kind,
                    TokenKind::LBrace | TokenKind::LBracket | TokenKind::LParen
                ) =>
            {
                self.consume_simple_block(input)
            }
            Node::Token(t) if t.kind == TokenKind::Function => self.consume_function(input),
            _ => token,
        })
    }

    fn consume_declaration(&mut self, input: &mut TokenScanner) -> Node {
        let start = input.pos;
        let name = token_value(input.consume().as_ref());
        while is_whitespace_token(input.peek()) {
            input.consume();
        }
        if !matches!(token_kind(input.peek()), Some(TokenKind::Colon)) {
            return error("invalid");
        }
        input.consume();
        let mut value = Vec::new();
        while input.peek().is_some() {
            if let Some(v) = self.consume_component_value(input) {
                value.push(v);
            }
        }
        let tokens = input.since(start);

        // `!important`: the last two tokens that aren't whitespace,
        // comments or semicolons.
        let significant: Vec<usize> = value
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                !matches!(
                    token_kind(Some(n)),
                    Some(TokenKind::Whitespace | TokenKind::Comment | TokenKind::Semicolon)
                )
            })
            .map(|(i, _)| i)
            .collect();
        let mut important = false;
        if significant.len() >= 2 {
            let bang = significant[significant.len() - 2];
            let word = significant[significant.len() - 1];
            let is_bang = matches!(&value[bang], Node::Token(t) if t.kind == TokenKind::Delim && t.value == "!");
            let is_important = matches!(&value[word], Node::Token(t) if t.kind == TokenKind::Ident && t.value.to_lowercase() == "important");
            if is_bang && is_important {
                important = true;
                value.truncate(bang);
            }
        }
        Node::Declaration(Declaration {
            name,
            value,
            important,
            tokens,
        })
    }

    /// `consume_declarations` (non-strict: comments, semicolons and
    /// whitespace are kept).
    fn consume_declarations(&mut self, input: &mut TokenScanner) -> Vec<Node> {
        let mut declarations = Vec::new();
        while let Some(token) = input.consume() {
            match token_kind(Some(&token)) {
                Some(TokenKind::Comment | TokenKind::Semicolon | TokenKind::Whitespace) => {
                    declarations.push(token);
                }
                Some(TokenKind::AtKeyword) => {
                    input.reconsume();
                    declarations.push(self.consume_at_rule(input));
                }
                Some(TokenKind::Ident) => {
                    let mut decl_tokens = vec![token];
                    while let Some(next) = input.peek() {
                        if matches!(token_kind(Some(next)), Some(TokenKind::Semicolon)) {
                            break;
                        }
                        if let Some(v) = self.consume_component_value(input) {
                            decl_tokens.push(v);
                        }
                    }
                    declarations
                        .push(self.consume_declaration(&mut TokenScanner::new(decl_tokens)));
                }
                _ => {
                    declarations.push(error("invalid"));
                    input.reconsume();
                    while let Some(next) = input.peek() {
                        if matches!(token_kind(Some(next)), Some(TokenKind::Semicolon)) {
                            break;
                        }
                        self.consume_component_value(input);
                    }
                }
            }
        }
        declarations
    }

    fn consume_function(&mut self, input: &mut TokenScanner) -> Node {
        self.depth += 1;
        let result = if self.depth > MAXIMUM_DEPTH {
            self.discard_block(input)
        } else {
            let current = input.current().cloned();
            let name = token_value(current.as_ref());
            let mut tokens: Vec<Node> = current.into_iter().collect();
            let start = input.pos;
            let mut value = Vec::new();
            while let Some(token) = input.consume() {
                match token_kind(Some(&token)) {
                    Some(TokenKind::RParen) => break,
                    Some(TokenKind::Comment) => continue,
                    _ => {
                        input.reconsume();
                        if let Some(v) = self.consume_component_value(input) {
                            value.push(v);
                        }
                    }
                }
            }
            tokens.extend(input.since(start));
            Node::Function(Function {
                name,
                value,
                tokens,
            })
        };
        self.depth -= 1;
        result
    }

    fn consume_qualified_rule(&mut self, input: &mut TokenScanner) -> Node {
        let start = input.pos;
        let mut prelude = Vec::new();
        let block;
        loop {
            let Some(token) = input.consume() else {
                return error("invalid");
            };
            match &token {
                Node::Token(t) if t.kind == TokenKind::LBrace => {
                    block = match self.consume_simple_block(input) {
                        Node::SimpleBlock(b) => b.value,
                        // Unreachable at the depth rules are consumed.
                        _ => Vec::new(),
                    };
                    break;
                }
                Node::SimpleBlock(b) if b.start == "{" => {
                    block = b.value.clone();
                    break;
                }
                _ => {
                    input.reconsume();
                    if let Some(v) = self.consume_component_value(input) {
                        prelude.push(v);
                    }
                }
            }
        }
        Node::QualifiedRule(QualifiedRule {
            prelude,
            block,
            tokens: input.since(start),
        })
    }

    fn consume_rules(&mut self, top_level: bool) -> Vec<Node> {
        let mut input = std::mem::replace(&mut self.tokens, TokenScanner::new(Vec::new()));
        let mut rules = Vec::new();
        while let Some(token) = input.consume() {
            match token_kind(Some(&token)) {
                Some(TokenKind::Comment | TokenKind::Whitespace) => rules.push(token),
                Some(TokenKind::Cdc | TokenKind::Cdo) => {
                    if !top_level {
                        input.reconsume();
                        rules.push(self.consume_qualified_rule(&mut input));
                    }
                }
                Some(TokenKind::AtKeyword) => {
                    input.reconsume();
                    rules.push(self.consume_at_rule(&mut input));
                }
                _ => {
                    input.reconsume();
                    rules.push(self.consume_qualified_rule(&mut input));
                }
            }
        }
        self.tokens = input;
        rules
    }

    fn consume_simple_block(&mut self, input: &mut TokenScanner) -> Node {
        self.depth += 1;
        let result = if self.depth > MAXIMUM_DEPTH {
            self.discard_block(input)
        } else {
            let current = input.current().cloned();
            let start_kind = token_kind(current.as_ref())
                .cloned()
                .unwrap_or(TokenKind::LBrace);
            let end_kind = block_end(&start_kind).unwrap_or(TokenKind::RBrace);
            let mut tokens: Vec<Node> = current.into_iter().collect();
            let start = input.pos;
            let mut value = Vec::new();
            while let Some(token) = input.consume() {
                if token_kind(Some(&token)) == Some(&end_kind) {
                    break;
                }
                input.reconsume();
                if let Some(v) = self.consume_component_value(input) {
                    value.push(v);
                }
            }
            tokens.extend(input.since(start));
            Node::SimpleBlock(SimpleBlock {
                start: kind_str(&start_kind),
                end: kind_str(&end_kind),
                value,
                tokens,
            })
        };
        self.depth -= 1;
        result
    }

    /// `discard_block`: an over-nested block or function skipped without
    /// recursing, kept as an error node with its tokens.
    fn discard_block(&mut self, input: &mut TokenScanner) -> Node {
        let opening = input.current().cloned();
        let first = match &opening {
            Some(Node::Function(_)) => TokenKind::RParen,
            Some(Node::Token(t)) if t.kind == TokenKind::Function => TokenKind::RParen,
            other => block_end(token_kind(other.as_ref()).unwrap_or(&TokenKind::LBrace))
                .unwrap_or(TokenKind::RBrace),
        };
        let mut stack = vec![first];
        let mut tokens: Vec<Node> = opening.into_iter().collect();
        let start = input.pos;
        while !stack.is_empty() {
            let Some(token) = input.consume() else { break };
            match &token {
                Node::Token(t) => match &t.kind {
                    k @ (TokenKind::LBrace | TokenKind::LBracket | TokenKind::LParen) => {
                        stack.extend(block_end(k));
                    }
                    TokenKind::Function => stack.push(TokenKind::RParen),
                    k if Some(k) == stack.last() => {
                        stack.pop();
                    }
                    _ => {}
                },
                Node::Function(_) => stack.push(TokenKind::RParen),
                _ => {}
            }
        }
        tokens.extend(input.since(start));
        Node::Error(ErrorNode {
            value: "maximum-depth-exceeded",
            tokens: Some(tokens),
        })
    }

    fn create_style_rule(&mut self, rule: QualifiedRule) -> Node {
        Node::StyleRule(StyleRule {
            selector: Selector {
                value: parse_value(&rule.prelude),
                tokens: rule.prelude,
            },
            children: parse_properties_nodes(rule.block),
        })
    }

    fn style_rules(&mut self, rules: Vec<Node>) -> Vec<Node> {
        rules
            .into_iter()
            .map(|rule| match rule {
                Node::QualifiedRule(q) => self.create_style_rule(q),
                other => other,
            })
            .collect()
    }

    /// `parse_properties` on this parser's tokens.
    fn parse_properties(&mut self) -> Vec<Node> {
        let mut input = std::mem::replace(&mut self.tokens, TokenScanner::new(Vec::new()));
        let declarations = self.consume_declarations(&mut input);
        self.tokens = input;
        declarations
            .into_iter()
            .map(|decl| match decl {
                Node::Declaration(d) => {
                    let mut children = d.value.clone();
                    if matches!(token_kind(children.last()), Some(TokenKind::Semicolon)) {
                        children.pop();
                    }
                    Node::Property(Property {
                        name: d.name,
                        value: parse_value(&d.value),
                        children,
                        important: d.important,
                        tokens: d.tokens,
                    })
                }
                other => other,
            })
            .collect()
    }
}

/// `Crass.parse` / `Parser.parse_stylesheet`
pub fn parse(input: &str, options: Options) -> Vec<Node> {
    let mut parser = Parser::new(input, options);
    let rules = parser.consume_rules(true);
    parser.style_rules(rules)
}

/// `Crass.parse_properties` on a string.
pub fn parse_properties(input: &str, options: Options) -> Vec<Node> {
    Parser::new(input, options).parse_properties()
}

/// `Parser.parse_properties` on already parsed nodes (a block's value).
pub fn parse_properties_nodes(input: Vec<Node>) -> Vec<Node> {
    Parser::from_tokens(input).parse_properties()
}

/// `Parser.parse_rules` on already parsed nodes (a block's value).
pub fn parse_rules(input: Vec<Node>) -> Vec<Node> {
    let mut parser = Parser::from_tokens(input);
    let rules = parser.consume_rules(false);
    parser.style_rules(rules)
}

/// `parse_value`: the unescaped text of a selector or property value.
pub fn parse_value(nodes: &[Node]) -> String {
    let mut string = String::new();
    for node in nodes {
        match node {
            Node::Token(Token {
                kind, value, raw, ..
            }) => match kind {
                TokenKind::Comment | TokenKind::Semicolon => {}
                TokenKind::AtKeyword | TokenKind::Ident => string.push_str(value),
                TokenKind::Function => {
                    string.push_str(value);
                    string.push('(');
                }
                _ => string.push_str(raw),
            },
            Node::Function(f) => string.push_str(&parse_value(&f.tokens)),
            Node::AtRule(AtRule { tokens, .. })
            | Node::SimpleBlock(SimpleBlock { tokens, .. })
            | Node::Declaration(Declaration { tokens, .. })
            | Node::QualifiedRule(QualifiedRule { tokens, .. })
            | Node::Property(Property { tokens, .. })
            | Node::Error(ErrorNode {
                tokens: Some(tokens),
                ..
            }) => string.push_str(&parse_value(tokens)),
            Node::StyleRule(_) | Node::Error(_) => {}
        }
    }
    crate::ruby::strip(&string).to_string()
}

/// `Parser.stringify`: a tree back to CSS from its original text.
pub fn stringify(nodes: &[Node]) -> String {
    let mut string = String::new();
    stringify_into(nodes, &mut string);
    string
}

fn stringify_into(nodes: &[Node], string: &mut String) {
    for node in nodes {
        match node {
            Node::AtRule(rule) => {
                string.push('@');
                string.push_str(&rule.name);
                stringify_into(&rule.prelude, string);
                match &rule.block {
                    Some(block) => {
                        string.push('{');
                        stringify_into(block, string);
                        string.push('}');
                    }
                    None => string.push(';'),
                }
            }
            Node::SimpleBlock(block) => {
                string.push_str(block.start);
                stringify_into(&block.value, string);
                string.push_str(block.end);
            }
            Node::StyleRule(rule) => {
                stringify_into(&rule.selector.tokens, string);
                string.push('{');
                stringify_into(&rule.children, string);
                string.push('}');
            }
            Node::Token(t) => string.push_str(&t.raw),
            Node::Function(Function { tokens, .. })
            | Node::Declaration(Declaration { tokens, .. })
            | Node::QualifiedRule(QualifiedRule { tokens, .. })
            | Node::Property(Property { tokens, .. })
            | Node::Error(ErrorNode {
                tokens: Some(tokens),
                ..
            }) => stringify_into(tokens, string),
            Node::Error(_) => {}
        }
    }
}
