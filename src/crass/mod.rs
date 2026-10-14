//! Port of the `crass` gem (1.0.7), the CSS Syntax Module Level 3 parser
//! Sanitize uses: the tokenizer, the parser (`parse`, `parse_properties`,
//! `parse_rules`) and `Parser.stringify`, which serializes a tree from
//! its tokens' original text.
//!
//! Crass's nodes are hashes keyed by `:node`; here tokens are [`Token`]
//! (with a [`TokenKind`]) and parsed constructs are the other [`Node`]
//! variants, each keeping the fields `stringify` and Sanitize read.

mod parser;
mod tokenizer;

pub use parser::{
    Parser, parse, parse_properties, parse_properties_nodes, parse_rules, parse_value, stringify,
};
pub use tokenizer::{Options, Tokenizer, preprocess};

/// A number's representation, value, and whether it was an integer.
#[derive(Debug, Clone, PartialEq)]
pub struct Numeric {
    pub repr: String,
    pub value: f64,
    pub integer: bool,
}

/// A token's `:node` type, with the fields particular to it.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    AtKeyword,
    BadString,
    BadUrl,
    Cdc,
    Cdo,
    Colon,
    Column,
    Comma,
    Comment,
    DashMatch,
    Delim,
    Dimension { number: Numeric, unit: String },
    Function,
    Hash { id: bool },
    Ident,
    IncludeMatch,
    Number(Numeric),
    Percentage(Numeric),
    PrefixMatch,
    Semicolon,
    String,
    SubstringMatch,
    SuffixMatch,
    UnicodeRange { start: u32, end: u32 },
    Url,
    Whitespace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
}

/// A token: its type, position, original text and (for the types that
/// have one) its unescaped `:value`.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub pos: usize,
    pub raw: String,
    pub value: String,
    pub error: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AtRule {
    pub name: String,
    pub prelude: Vec<Node>,
    pub block: Option<Vec<Node>>,
    pub tokens: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SimpleBlock {
    pub start: &'static str,
    pub end: &'static str,
    pub value: Vec<Node>,
    pub tokens: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: String,
    pub value: Vec<Node>,
    pub tokens: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    pub name: String,
    pub value: Vec<Node>,
    pub important: bool,
    pub tokens: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QualifiedRule {
    pub prelude: Vec<Node>,
    pub block: Vec<Node>,
    pub tokens: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Selector {
    pub value: String,
    pub tokens: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StyleRule {
    pub selector: Selector,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub name: String,
    pub value: String,
    pub children: Vec<Node>,
    pub important: bool,
    pub tokens: Vec<Node>,
}

/// An `:error` node; only the maximum-depth one keeps its tokens.
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorNode {
    pub value: &'static str,
    pub tokens: Option<Vec<Node>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Token(Token),
    AtRule(AtRule),
    SimpleBlock(SimpleBlock),
    Function(Function),
    Declaration(Declaration),
    QualifiedRule(QualifiedRule),
    StyleRule(StyleRule),
    Property(Property),
    Error(ErrorNode),
}

impl Node {
    pub fn token(&self) -> Option<&Token> {
        match self {
            Node::Token(t) => Some(t),
            _ => None,
        }
    }

    /// Whether this is a token of the given kind.
    pub fn is(&self, kind: &TokenKind) -> bool {
        self.token().is_some_and(|t| &t.kind == kind)
    }
}

#[cfg(test)]
mod tests;
