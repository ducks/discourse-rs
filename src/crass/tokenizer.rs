//! `Crass::Tokenizer` and `Crass::Scanner` (crass 1.0.7,
//! lib/crass/tokenizer.rb and lib/crass/scanner.rb).
//!
//! The scanner works on characters: `pos` and `marker` are character
//! indexes, and a token's `raw` is the input between the marker and the
//! position, so serializing tokens reproduces the input exactly.

use super::{Node, Numeric, Token, TokenKind};

/// Tokenizer options (`:preserve_comments`, `:preserve_hacks`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub preserve_comments: bool,
    pub preserve_hacks: bool,
}

struct Scanner {
    chars: Vec<char>,
    current: Option<char>,
    marker: usize,
    pos: usize,
}

impl Scanner {
    fn new(input: &str) -> Scanner {
        Scanner {
            chars: input.chars().collect(),
            current: None,
            marker: 0,
            pos: 0,
        }
    }

    /// `consume`: the next character, or None (Ruby's `''`) at the end.
    fn consume(&mut self) -> Option<char> {
        if self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            self.pos += 1;
            self.current = Some(c);
            Some(c)
        } else {
            None
        }
    }

    fn consume_rest(&mut self) -> String {
        let rest: String = self.chars[self.pos..].iter().collect();
        self.current = self.chars[self.pos..].last().copied();
        self.pos = self.chars.len();
        rest
    }

    fn eos(&self) -> bool {
        self.pos == self.chars.len()
    }

    fn mark(&mut self) {
        self.marker = self.pos;
    }

    fn marked(&self) -> String {
        self.chars[self.marker..self.pos].iter().collect()
    }

    /// Up to `n` characters from the position, not consumed.
    fn peek(&self, n: usize) -> &[char] {
        let end = (self.pos + n).min(self.chars.len());
        &self.chars[self.pos..end]
    }

    fn peek1(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    /// `reconsume`. Every caller follows a single-character `consume`, so
    /// `StringScanner#unscan` moves back exactly one character.
    fn reconsume(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
        }
    }

    /// `scan` of a `[class]+` pattern.
    fn scan_while(&mut self, pred: impl Fn(char) -> bool) -> Option<String> {
        let start = self.pos;
        let mut end = start;
        while end < self.chars.len() && pred(self.chars[end]) {
            end += 1;
        }
        self.take(start, end)
    }

    fn take(&mut self, start: usize, end: usize) -> Option<String> {
        if end == start {
            return None;
        }
        self.pos = end;
        self.current = Some(self.chars[end - 1]);
        Some(self.chars[start..end].iter().collect())
    }

    /// `scan(RE_HEX)`: one to six hex digits.
    fn scan_hex(&mut self) -> Option<String> {
        let start = self.pos;
        let mut end = start;
        while end < self.chars.len() && end - start < 6 && self.chars[end].is_ascii_hexdigit() {
            end += 1;
        }
        self.take(start, end)
    }

    /// `scan(RE_NUMBER_DECIMAL)`: `\.[0-9]+`
    fn scan_decimal(&mut self) -> Option<String> {
        let start = self.pos;
        if self.chars.get(start) != Some(&'.') {
            return None;
        }
        let mut end = start + 1;
        while end < self.chars.len() && self.chars[end].is_ascii_digit() {
            end += 1;
        }
        if end == start + 1 {
            return None;
        }
        self.take(start, end)
    }

    /// `scan(RE_NUMBER_EXPONENT)`: `[Ee][+-]?[0-9]+`
    fn scan_exponent(&mut self) -> Option<String> {
        let start = self.pos;
        if !matches!(self.chars.get(start), Some('e' | 'E')) {
            return None;
        }
        let mut end = start + 1;
        if matches!(self.chars.get(end), Some('+' | '-')) {
            end += 1;
        }
        let digits = end;
        while end < self.chars.len() && self.chars[end].is_ascii_digit() {
            end += 1;
        }
        if end == digits {
            return None;
        }
        self.take(start, end)
    }

    /// `scan_until(/\*\//)`
    fn scan_until_comment_close(&mut self) -> Option<String> {
        let start = self.pos;
        let mut i = start;
        while i + 1 < self.chars.len() {
            if self.chars[i] == '*' && self.chars[i + 1] == '/' {
                return self.take(start, i + 2);
            }
            i += 1;
        }
        None
    }
}

fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}

/// `RE_NAME`
fn is_name(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || c >= '\u{80}'
}

/// `RE_NAME_START`
fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c >= '\u{80}'
}

/// `RE_WHITESPACE` (after preprocessing, only these three remain).
fn is_whitespace(c: char) -> bool {
    matches!(c, '\n' | '\t' | ' ')
}

/// `RE_NON_PRINTABLE`
fn is_non_printable(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{e}'..='\u{1f}' | '\u{7f}')
}

/// `valid_escape?(text)`: a backslash not followed by a newline (the end
/// of the input counts as not a newline).
fn valid_escape(text: &[char]) -> bool {
    text.first() == Some(&'\\') && text.get(1) != Some(&'\n')
}

/// `start_identifier?(text)`
fn start_identifier(text: &[char]) -> bool {
    match text.first() {
        Some('-') => {
            let next = text.get(1).copied();
            next == Some('-')
                || next.is_some_and(is_name_start)
                || valid_escape(text.get(1..).unwrap_or(&[]))
        }
        Some(&c) if is_name_start(c) => true,
        Some('\\') => valid_escape(text),
        _ => false,
    }
}

/// `start_number?(text)`
fn start_number(text: &[char]) -> bool {
    match text.first() {
        Some('+' | '-') => {
            text.get(1).copied().is_some_and(is_digit)
                || (text.get(1) == Some(&'.') && text.get(2).copied().is_some_and(is_digit))
        }
        Some('.') => text.get(1).copied().is_some_and(is_digit),
        Some(&c) if is_digit(c) => true,
        _ => false,
    }
}

/// `String#hex` on a run of hex digits (and nothing else).
fn hex(s: &str) -> u32 {
    let digits: String = s.chars().take_while(char::is_ascii_hexdigit).collect();
    u32::from_str_radix(&digits, 16).unwrap_or(0)
}

/// Input preprocessing: newlines normalized and NUL replaced.
pub fn preprocess(input: &str) -> String {
    input
        .replace("\r\n", "\n")
        .replace(['\r', '\u{c}'], "\n")
        .replace('\0', "\u{fffd}")
}

pub struct Tokenizer {
    s: Scanner,
    options: Options,
}

impl Tokenizer {
    pub fn new(input: &str, options: Options) -> Tokenizer {
        Tokenizer {
            s: Scanner::new(&preprocess(input)),
            options,
        }
    }

    /// `Tokenizer.tokenize`
    pub fn tokenize(input: &str, options: Options) -> Vec<Node> {
        let mut t = Tokenizer::new(input, options);
        let mut tokens = Vec::new();
        while let Some(token) = t.consume() {
            tokens.push(Node::Token(token));
        }
        tokens
    }

    fn create_token(&self, kind: TokenKind, value: impl Into<String>) -> Token {
        Token {
            kind,
            pos: self.s.marker,
            raw: self.s.marked(),
            value: value.into(),
            error: false,
        }
    }

    fn token(&self, kind: TokenKind) -> Token {
        self.create_token(kind, "")
    }

    fn delim(&self, c: char) -> Token {
        self.create_token(TokenKind::Delim, c.to_string())
    }

    /// The current character and the next `n - 1`.
    fn current_and_peek(&self, n: usize) -> Vec<char> {
        let mut text: Vec<char> = self.s.current.into_iter().collect();
        text.extend_from_slice(self.s.peek(n - 1));
        text
    }

    fn valid_escape_here(&self) -> bool {
        valid_escape(&self.current_and_peek(2))
    }

    /// `consume`: the next token, None at the end of the input.
    pub fn consume(&mut self) -> Option<Token> {
        loop {
            if self.s.eos() {
                return None;
            }
            self.s.mark();
            let Some(comment) = self.consume_comments() else {
                break;
            };
            if self.options.preserve_comments {
                return Some(comment);
            }
        }

        if self.s.scan_while(is_whitespace).is_some() {
            return Some(self.token(TokenKind::Whitespace));
        }

        let c = self.s.consume()?;
        let token = match c {
            '"' | '\'' => self.consume_string(None),
            '#' => {
                if self.s.peek1().is_some_and(is_name) || valid_escape(self.s.peek(2)) {
                    let id = start_identifier(self.s.peek(3));
                    let name = self.consume_name();
                    self.create_token(TokenKind::Hash { id }, name)
                } else {
                    self.delim(c)
                }
            }
            '$' => self.match_token(TokenKind::SuffixMatch, c),
            '(' => self.token(TokenKind::LParen),
            ')' => self.token(TokenKind::RParen),
            '*' => {
                if self.s.peek1() == Some('=') {
                    self.s.consume();
                    self.token(TokenKind::SubstringMatch)
                } else if self.options.preserve_hacks && self.s.peek1().is_some_and(is_name_start) {
                    self.s.reconsume();
                    self.consume_ident()
                } else {
                    self.delim(c)
                }
            }
            '+' => {
                if start_number(&self.current_and_peek(3)) {
                    self.s.reconsume();
                    self.consume_numeric()
                } else {
                    self.delim(c)
                }
            }
            ',' => self.token(TokenKind::Comma),
            '-' => {
                let next_two: Vec<char> = self.s.peek(2).to_vec();
                let mut next_three = vec![c];
                next_three.extend_from_slice(&next_two);
                if start_number(&next_three) {
                    self.s.reconsume();
                    self.consume_numeric()
                } else if next_two == ['-', '>'] {
                    self.s.consume();
                    self.s.consume();
                    self.token(TokenKind::Cdc)
                } else if start_identifier(&next_three) {
                    self.s.reconsume();
                    self.consume_ident()
                } else {
                    self.delim(c)
                }
            }
            '.' => {
                if start_number(&self.current_and_peek(3)) {
                    self.s.reconsume();
                    self.consume_numeric()
                } else {
                    self.delim(c)
                }
            }
            ':' => self.token(TokenKind::Colon),
            ';' => self.token(TokenKind::Semicolon),
            '<' => {
                if self.s.peek(3) == ['!', '-', '-'] {
                    self.s.consume();
                    self.s.consume();
                    self.s.consume();
                    self.token(TokenKind::Cdo)
                } else {
                    self.delim(c)
                }
            }
            '@' => {
                if start_identifier(self.s.peek(3)) {
                    let name = self.consume_name();
                    self.create_token(TokenKind::AtKeyword, name)
                } else {
                    self.delim(c)
                }
            }
            '[' => self.token(TokenKind::LBracket),
            '\\' => {
                if self.valid_escape_here() {
                    self.s.reconsume();
                    self.consume_ident()
                } else {
                    let mut t = self.delim(c);
                    t.error = true;
                    t
                }
            }
            ']' => self.token(TokenKind::RBracket),
            '^' => self.match_token(TokenKind::PrefixMatch, c),
            '{' => self.token(TokenKind::LBrace),
            '}' => self.token(TokenKind::RBrace),
            'U' | 'u' => {
                let next = self.s.peek(2);
                if next.first() == Some(&'+')
                    && next
                        .get(1)
                        .is_some_and(|&c| c.is_ascii_hexdigit() || c == '?')
                {
                    self.s.consume();
                    self.consume_unicode_range()
                } else {
                    self.s.reconsume();
                    self.consume_ident()
                }
            }
            '|' => match self.s.peek1() {
                Some('=') => {
                    self.s.consume();
                    self.token(TokenKind::DashMatch)
                }
                Some('|') => {
                    self.s.consume();
                    self.token(TokenKind::Column)
                }
                _ => self.delim(c),
            },
            '~' => self.match_token(TokenKind::IncludeMatch, c),
            _ if is_digit(c) => {
                self.s.reconsume();
                self.consume_numeric()
            }
            _ if is_name_start(c) => {
                self.s.reconsume();
                self.consume_ident()
            }
            _ => self.delim(c),
        };
        Some(token)
    }

    /// `$=`, `^=`, `~=`: the match token, else a delim.
    fn match_token(&mut self, kind: TokenKind, c: char) -> Token {
        if self.s.peek1() == Some('=') {
            self.s.consume();
            self.token(kind)
        } else {
            self.delim(c)
        }
    }

    fn consume_bad_url(&mut self) -> String {
        let mut text = String::new();
        while !self.s.eos() {
            if self.valid_escape_here() {
                text.push_str(&self.consume_escaped());
            } else if valid_escape(self.s.peek(2)) {
                self.s.consume();
                text.push_str(&self.consume_escaped());
            } else {
                match self.s.consume() {
                    Some(')') => break,
                    Some(c) => text.push(c),
                    None => {}
                }
            }
        }
        text
    }

    fn consume_comments(&mut self) -> Option<Token> {
        if self.s.peek(2) != ['/', '*'] {
            return None;
        }
        self.s.consume();
        self.s.consume();
        let text = match self.s.scan_until_comment_close() {
            Some(mut text) => {
                text.truncate(text.len() - 2);
                text
            }
            None => self.s.consume_rest(),
        };
        Some(self.create_token(TokenKind::Comment, text))
    }

    fn consume_escaped(&mut self) -> String {
        if self.s.eos() {
            return "\u{fffd}".into();
        }
        if let Some(hex_str) = self.s.scan_hex() {
            if self.s.peek1().is_some_and(is_whitespace) {
                self.s.consume();
            }
            let codepoint = hex(&hex_str);
            return match char::from_u32(codepoint) {
                Some(c) if codepoint != 0 => c.to_string(),
                _ => "\u{fffd}".into(),
            };
        }
        self.s.consume().map(String::from).unwrap_or_default()
    }

    fn consume_ident(&mut self) -> Token {
        let value = self.consume_name();
        if self.s.peek1() != Some('(') {
            return self.create_token(TokenKind::Ident, value);
        }
        self.s.consume();
        if value.to_lowercase() == "url" {
            while self.s.peek(2).len() == 2 && self.s.peek(2).iter().all(|&c| is_whitespace(c)) {
                self.s.consume();
            }
            let next = self.s.peek(2);
            let quoted = match next.first() {
                Some('"' | '\'') => true,
                Some(&c) if is_whitespace(c) => matches!(next.get(1), Some('"' | '\'')),
                _ => false,
            };
            if quoted {
                self.create_token(TokenKind::Function, value)
            } else {
                self.consume_url()
            }
        } else {
            self.create_token(TokenKind::Function, value)
        }
    }

    fn consume_name(&mut self) -> String {
        let mut result = String::new();
        while !self.s.eos() {
            if let Some(m) = self.s.scan_while(is_name) {
                result.push_str(&m);
                continue;
            }
            let c = self.s.consume();
            if self.valid_escape_here() {
                result.push_str(&self.consume_escaped());
            } else if c == Some('*') && self.options.preserve_hacks {
                // Non-standard: the IE * hack. The character after the
                // star is what's kept.
                if let Some(next) = self.s.consume() {
                    result.push(next);
                }
            } else {
                self.s.reconsume();
                return result;
            }
        }
        result
    }

    fn consume_number(&mut self) -> Numeric {
        let mut repr = String::new();
        let mut integer = true;
        if matches!(self.s.peek1(), Some('+' | '-'))
            && let Some(c) = self.s.consume()
        {
            repr.push(c);
        }
        if let Some(d) = self.s.scan_while(is_digit) {
            repr.push_str(&d);
        }
        if let Some(m) = self.s.scan_decimal() {
            repr.push_str(&m);
            integer = false;
        }
        if let Some(m) = self.s.scan_exponent() {
            repr.push_str(&m);
            integer = false;
        }
        let value = convert_string_to_number(&repr);
        Numeric {
            value: if integer { value.trunc() } else { value },
            integer,
            repr,
        }
    }

    fn consume_numeric(&mut self) -> Token {
        let number = self.consume_number();
        if start_identifier(self.s.peek(3)) {
            let unit = self.consume_name();
            self.token(TokenKind::Dimension { number, unit })
        } else if self.s.peek1() == Some('%') {
            self.s.consume();
            self.token(TokenKind::Percentage(number))
        } else {
            self.token(TokenKind::Number(number))
        }
    }

    fn consume_string(&mut self, ending: Option<char>) -> Token {
        let ending = ending.or(self.s.current);
        let mut value = String::new();
        while !self.s.eos() {
            let Some(c) = self.s.consume() else { break };
            if Some(c) == ending {
                break;
            }
            match c {
                '\n' => {
                    self.s.reconsume();
                    let mut t = self.create_token(TokenKind::BadString, value);
                    t.error = true;
                    return t;
                }
                '\\' => match self.s.peek1() {
                    None => {}
                    Some('\n') => {
                        self.s.consume();
                    }
                    Some(_) => value.push_str(&self.consume_escaped()),
                },
                _ => value.push(c),
            }
        }
        self.create_token(TokenKind::String, value)
    }

    fn consume_unicode_range(&mut self) -> Token {
        let mut value = self.s.scan_hex().unwrap_or_default();
        while value.chars().count() < 6 {
            if self.s.peek1() != Some('?') {
                break;
            }
            if let Some(c) = self.s.consume() {
                value.push(c);
            }
        }
        if value.contains('?') {
            let start = hex(&value.replace('?', "0"));
            let end = hex(&value.replace('?', "F"));
            return self.token(TokenKind::UnicodeRange { start, end });
        }
        let start = hex(&value);
        let next = self.s.peek(2);
        let end = if next.first() == Some(&'-') && next.get(1).is_some_and(char::is_ascii_hexdigit)
        {
            self.s.consume();
            hex(&self.s.scan_hex().unwrap_or_default())
        } else {
            start
        };
        self.token(TokenKind::UnicodeRange { start, end })
    }

    fn consume_url(&mut self) -> Token {
        let mut value = String::new();
        self.s.scan_while(is_whitespace);
        while !self.s.eos() {
            let Some(c) = self.s.consume() else { break };
            match c {
                ')' => break,
                _ if is_whitespace(c) => {
                    self.s.scan_while(is_whitespace);
                    if self.s.eos() || self.s.peek1() == Some(')') {
                        self.s.consume();
                        break;
                    }
                    value.push_str(&self.consume_bad_url());
                    return self.create_token(TokenKind::BadUrl, value);
                }
                '"' | '\'' | '(' => return self.bad_url(value),
                _ if is_non_printable(c) => return self.bad_url(value),
                '\\' => {
                    if self.valid_escape_here() {
                        value.push_str(&self.consume_escaped());
                    } else {
                        return self.bad_url(value);
                    }
                }
                _ => value.push(c),
            }
        }
        self.create_token(TokenKind::Url, value)
    }

    fn bad_url(&mut self, mut value: String) -> Token {
        value.push_str(&self.consume_bad_url());
        let mut t = self.create_token(TokenKind::BadUrl, value);
        t.error = true;
        t
    }
}

/// `convert_string_to_number`, as a float. Crass's arithmetic is exact
/// (Rational) before the conversion; parsing the representation gives
/// the same correctly rounded value, clamped to `Float::MAX`.
fn convert_string_to_number(repr: &str) -> f64 {
    let mut s = repr.to_string();
    // "1." or "." alone can't be produced; a bare sign or nothing can be
    // (e.g. "+" before a "."-number is consumed by start_number checks).
    if s.is_empty() || s == "+" || s == "-" {
        s.push('0');
    }
    let value: f64 = s.parse().unwrap_or(0.0);
    if value.is_infinite() {
        if value > 0.0 { f64::MAX } else { -f64::MAX }
    } else {
        value
    }
}
