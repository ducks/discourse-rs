//! Admin regexps (`watched_words_regular_expressions`) as each engine that
//! runs them reads them: Ruby (Onigmo) on the server, JavaScript with the
//! `u` flag in cooking and the client, and JavaScript without flags in the
//! emoji rule's pass over the replace words. A source is parsed against a
//! grammar all three read alike and written out as a fancy-regex source
//! that behaves as the chosen engine does: `\w`, `\d`, `\s`, `\b`, `.`,
//! `^` and `$` mean different things in each, and so do case folding and
//! `\p{..}`. Anything outside the grammar (backreferences, named groups,
//! inline flags, atomic groups, possessive or nested quantifiers, class
//! sets, POSIX brackets, lookbehinds that aren't one fixed run of
//! characters, escapes one engine doesn't know, astral characters in the
//! flagless dialect, patterns that can match nothing) is refused rather
//! than guessed at.

use std::fmt::Write as _;

/// The engine a source is translated for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// Onigmo, as `Regexp.new(source, case_sensitive ? nil : IGNORECASE)`.
    Ruby,
    /// JavaScript with `u` (and `i` when not case-sensitive).
    JsUnicode,
    /// JavaScript without flags (case-sensitive, by UTF-16 unit; the
    /// caller refuses text with astral characters).
    JsLegacy,
}

/// A translated source and the capture groups it has (the same groups,
/// in the same order, as the original).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translated {
    pub source: String,
    pub groups: usize,
}

/// `\p{..}` names all three engines read the same: the general categories.
const CATEGORIES: [&str; 38] = [
    "L", "Lu", "Ll", "Lt", "Lm", "Lo", "M", "Mn", "Mc", "Me", "N", "Nd", "Nl", "No", "P", "Pc",
    "Pd", "Ps", "Pe", "Pi", "Pf", "Po", "S", "Sm", "Sc", "Sk", "So", "Z", "Zs", "Zl", "Zp", "C",
    "Cc", "Cf", "Co", "Cn", "Cs", "LC",
];

/// JavaScript's `\s`: WhiteSpace and LineTerminator.
const JS_SPACE: &str =
    r"\t\n\x0B\x0C\r \xA0\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";
/// Ruby's `\s`.
const RUBY_SPACE: &str = r"\t\n\x0B\x0C\r ";
/// Onigmo's Unicode word characters, which `\b` looks at.
const RUBY_WORD: &str = r"\p{Alphabetic}\p{M}\p{N}\p{Pc}\p{Join_Control}";
/// JavaScript's word characters; with `u` and `i`, also ſ and K (Kelvin).
const JS_WORD: &str = "0-9A-Za-z_";
const JS_WORD_FOLDED: &str = r"0-9A-Za-z_\x{17F}\x{212A}";

/// Translates `source` for `dialect`, case-insensitive unless
/// `case_sensitive` (the flagless dialect is always case-sensitive).
pub fn translate(
    source: &str,
    dialect: Dialect,
    case_sensitive: bool,
) -> Result<Translated, &'static str> {
    let fold = !case_sensitive && dialect != Dialect::JsLegacy;
    let mut p = Parser {
        chars: source.chars().collect(),
        pos: 0,
        dialect,
        fold,
        groups: 0,
        out: String::new(),
    };
    let min = p.alternation(false)?;
    if p.pos != p.chars.len() {
        return Err("a watched word regexp with an unbalanced `)`");
    }
    if min == 0 {
        return Err("a watched word regexp that can match nothing");
    }
    let source = if fold {
        format!("(?i){}", p.out)
    } else {
        p.out
    };
    Ok(Translated {
        source,
        groups: p.groups,
    })
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
    dialect: Dialect,
    fold: bool,
    groups: usize,
    out: String,
}

/// One character as fancy-regex reads it literally, in or out of a class.
fn literal(c: char, out: &mut String) {
    if c.is_ascii_alphanumeric() {
        out.push(c);
    } else {
        let _ = write!(out, r"\x{{{:X}}}", c as u32);
    }
}

/// A character whose case mapping is several characters: Onigmo folds
/// fully (`ß` matches `ss`), JavaScript and fancy-regex simply.
fn multi_fold(c: char) -> bool {
    c.to_lowercase().count() > 1 || c.to_uppercase().count() > 1
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Alternatives up to a `)` or the end; the shortest match length.
    fn alternation(&mut self, lookbehind: bool) -> Result<usize, &'static str> {
        let mut min = self.sequence(lookbehind)?;
        while self.eat('|') {
            if lookbehind {
                return Err("a watched word regexp with alternatives in a lookbehind");
            }
            self.out.push('|');
            min = min.min(self.sequence(lookbehind)?);
        }
        Ok(min)
    }

    fn sequence(&mut self, lookbehind: bool) -> Result<usize, &'static str> {
        let mut min = 0usize;
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let (atom_min, quantifiable) = self.atom(lookbehind)?;
            let atom_min = if matches!(self.peek(), Some('*' | '+' | '?' | '{')) {
                if !quantifiable {
                    return Err("a watched word regexp quantifying what can't be");
                }
                if lookbehind {
                    return Err("a watched word regexp with a quantifier in a lookbehind");
                }
                let low = self.quantifier()?;
                atom_min.saturating_mul(low)
            } else {
                atom_min
            };
            min = min.saturating_add(atom_min);
        }
        Ok(min)
    }

    /// `*`, `+`, `?`, `{n}`, `{n,}`, `{n,m}`, each maybe lazy; the least
    /// number of repeats.
    fn quantifier(&mut self) -> Result<usize, &'static str> {
        let c = self.peek().expect("quantifier");
        self.pos += 1;
        let low = match c {
            '*' => {
                self.out.push('*');
                0
            }
            '+' => {
                self.out.push('+');
                1
            }
            '?' => {
                self.out.push('?');
                0
            }
            _ => {
                let low = self.number()?;
                let high = if self.eat(',') {
                    if self.peek() == Some('}') {
                        None
                    } else {
                        Some(self.number()?)
                    }
                } else {
                    Some(low)
                };
                if !self.eat('}') {
                    return Err("a watched word regexp with a `{` that isn't a quantifier");
                }
                if high.is_some_and(|h| h < low) || low > 1000 || high.is_some_and(|h| h > 1000) {
                    return Err("a watched word regexp with an unusual repeat count");
                }
                let _ = match high {
                    Some(h) if h == low => write!(self.out, "{{{low}}}"),
                    Some(h) => write!(self.out, "{{{low},{h}}}"),
                    None => write!(self.out, "{{{low},}}"),
                };
                low
            }
        };
        if self.eat('?') {
            self.out.push('?');
        }
        if matches!(self.peek(), Some('*' | '+' | '?' | '{')) {
            return Err("a watched word regexp with a possessive or repeated quantifier");
        }
        Ok(low)
    }

    fn number(&mut self) -> Result<usize, &'static str> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.pos == start || self.pos - start > 4 {
            return Err("a watched word regexp with a `{` that isn't a quantifier");
        }
        Ok(self.chars[start..self.pos]
            .iter()
            .collect::<String>()
            .parse()
            .unwrap_or(0))
    }

    /// One atom: its shortest match and whether it takes a quantifier.
    fn atom(&mut self, lookbehind: bool) -> Result<(usize, bool), &'static str> {
        let c = self.peek().expect("atom");
        self.pos += 1;
        match c {
            '(' => self.group(lookbehind),
            '[' => {
                self.class()?;
                Ok((1, true))
            }
            '.' => {
                self.out.push_str(match self.dialect {
                    Dialect::Ruby => r"[^\n]",
                    _ => r"[^\n\r\x{2028}\x{2029}]",
                });
                Ok((1, true))
            }
            '^' | '$' => {
                if lookbehind {
                    return Err("a watched word regexp with an anchor in a lookbehind");
                }
                // Ruby's anchors are at line ends; JavaScript's (no `m`) at
                // the text's.
                let anchor = match (self.dialect, c) {
                    (Dialect::Ruby, '^') => "(?m:^)",
                    (Dialect::Ruby, _) => "(?m:$)",
                    (_, '^') => "^",
                    _ => "$",
                };
                self.out.push_str(anchor);
                Ok((0, false))
            }
            '\\' => self.escape(lookbehind),
            '*' | '+' | '?' | '{' | '}' | ']' | ')' => {
                Err("a watched word regexp with a stray metacharacter")
            }
            c => {
                self.char_literal(c)?;
                Ok((1, true))
            }
        }
    }

    fn char_literal(&mut self, c: char) -> Result<(), &'static str> {
        if self.dialect == Dialect::JsLegacy && c as u32 > 0xFFFF {
            return Err("a watched word regexp with astral characters");
        }
        if self.fold && multi_fold(c) {
            return Err("watched words with full case folding");
        }
        literal(c, &mut self.out);
        Ok(())
    }

    fn group(&mut self, lookbehind: bool) -> Result<(usize, bool), &'static str> {
        if lookbehind {
            return Err("a watched word regexp with a group in a lookbehind");
        }
        if !self.eat('?') {
            self.groups += 1;
            self.out.push('(');
            let min = self.alternation(false)?;
            if !self.eat(')') {
                return Err("a watched word regexp with an unclosed `(`");
            }
            self.out.push(')');
            return Ok((min, true));
        }
        let kind = match (self.peek(), self.peek_at(1)) {
            (Some(':'), _) => {
                self.pos += 1;
                "?:"
            }
            (Some('='), _) => {
                self.pos += 1;
                "?="
            }
            (Some('!'), _) => {
                self.pos += 1;
                "?!"
            }
            (Some('<'), Some('=')) => {
                self.pos += 2;
                "?<="
            }
            (Some('<'), Some('!')) => {
                self.pos += 2;
                "?<!"
            }
            _ => {
                return Err(
                    "a watched word regexp with named groups, inline flags or other `(?` forms",
                );
            }
        };
        self.out.push('(');
        self.out.push_str(kind);
        let behind = kind.starts_with("?<");
        let min = self.alternation(behind)?;
        if !self.eat(')') {
            return Err("a watched word regexp with an unclosed `(`");
        }
        self.out.push(')');
        if kind == "?:" {
            Ok((min, true))
        } else {
            Ok((0, false))
        }
    }

    fn hex(&mut self, digits: usize) -> Result<char, &'static str> {
        let end = self.pos + digits;
        if end > self.chars.len()
            || !self.chars[self.pos..end]
                .iter()
                .all(char::is_ascii_hexdigit)
        {
            return Err("a watched word regexp with an escape the engines read differently");
        }
        let s: String = self.chars[self.pos..end].iter().collect();
        self.pos = end;
        char::from_u32(u32::from_str_radix(&s, 16).unwrap_or(0xD800))
            .ok_or("a watched word regexp with a lone surrogate")
    }

    /// `\p{Name}` or `\P{Name}` after the `p`; the name.
    fn property(&mut self) -> Result<String, &'static str> {
        if !self.eat('{') {
            return Err("a watched word regexp with an escape the engines read differently");
        }
        let start = self.pos;
        while self.peek().is_some_and(|c| c != '}') {
            self.pos += 1;
        }
        let name: String = self.chars[start..self.pos].iter().collect();
        if !self.eat('}') || !CATEGORIES.contains(&name.as_str()) {
            return Err("a watched word regexp with a property the engines read differently");
        }
        Ok(name)
    }

    /// A character a backslash escapes, shared by classes: `\n` and the
    /// like, `\xHH`, `\uHHHH` and the escaped syntax characters.
    fn escaped_char(&mut self, c: char, in_class: bool) -> Result<Option<char>, &'static str> {
        Ok(match c {
            'n' => Some('\n'),
            't' => Some('\t'),
            'r' => Some('\r'),
            'f' => Some('\x0C'),
            'v' => Some('\x0B'),
            'x' => Some(self.hex(2)?),
            'u' => Some(self.hex(4)?),
            '^' | '$' | '\\' | '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|'
            | '/' => Some(c),
            '-' if in_class => Some(c),
            _ => None,
        })
    }

    fn escape(&mut self, lookbehind: bool) -> Result<(usize, bool), &'static str> {
        let Some(c) = self.peek() else {
            return Err("a watched word regexp ending in a backslash");
        };
        self.pos += 1;
        let ruby = self.dialect == Dialect::Ruby;
        match c {
            'd' | 'D' | 'w' | 'W' | 's' | 'S' => {
                let set = self.shorthand(c.to_ascii_lowercase());
                // Unfolded: Ruby's ASCII `\w` doesn't take K (Kelvin) under
                // IGNORECASE, and JavaScript's sets already hold what folds.
                if c.is_ascii_uppercase() {
                    let _ = write!(self.out, "(?-i:[^{set}])");
                } else {
                    let _ = write!(self.out, "(?-i:[{set}])");
                }
                Ok((1, true))
            }
            'b' | 'B' => {
                if lookbehind {
                    return Err("a watched word regexp with a word boundary in a lookbehind");
                }
                let w = match self.dialect {
                    Dialect::Ruby => RUBY_WORD,
                    Dialect::JsUnicode if self.fold => JS_WORD_FOLDED,
                    _ => JS_WORD,
                };
                if c == 'b' {
                    let _ = write!(
                        self.out,
                        "(?-i:(?:(?<=[{w}])(?![{w}])|(?<![{w}])(?=[{w}])))"
                    );
                } else {
                    let _ = write!(
                        self.out,
                        "(?-i:(?:(?<=[{w}])(?=[{w}])|(?<![{w}])(?![{w}])))"
                    );
                }
                Ok((0, false))
            }
            'p' | 'P' => {
                let name = self.property()?;
                match self.dialect {
                    // Without `u` it's the letter and the braces.
                    Dialect::JsLegacy => {
                        self.out.push(c);
                        literal('{', &mut self.out);
                        for n in name.chars() {
                            self.out.push(n);
                        }
                        literal('}', &mut self.out);
                        Ok((name.chars().count() + 3, true))
                    }
                    // Onigmo doesn't fold a property under IGNORECASE.
                    Dialect::Ruby => {
                        let _ = write!(self.out, r"(?-i:\{c}{{{name}}})");
                        Ok((1, true))
                    }
                    Dialect::JsUnicode => {
                        let _ = write!(self.out, r"\{c}{{{name}}}");
                        Ok((1, true))
                    }
                }
            }
            c => match self.escaped_char(c, false)? {
                Some(ch) => {
                    let _ = ruby;
                    self.char_literal(ch)?;
                    Ok((1, true))
                }
                None => Err("a watched word regexp with an escape the engines read differently"),
            },
        }
    }

    /// The members of `\d`, `\w` or `\s` for the dialect, for a class.
    fn shorthand(&self, c: char) -> &'static str {
        match (c, self.dialect) {
            ('d', _) => "0-9",
            ('w', Dialect::JsUnicode) if self.fold => JS_WORD_FOLDED,
            ('w', _) => JS_WORD,
            (_, Dialect::Ruby) => RUBY_SPACE,
            _ => JS_SPACE,
        }
    }

    /// A class after its `[`.
    fn class(&mut self) -> Result<(), &'static str> {
        self.out.push('[');
        if self.eat('^') {
            self.out.push('^');
        }
        if matches!(self.peek(), Some(']')) {
            return Err("a watched word regexp with an empty class or a leading `]`");
        }
        // The previous member, when it's a single character a range may
        // start from.
        let mut prev: Option<char> = None;
        loop {
            let Some(c) = self.peek() else {
                return Err("a watched word regexp with an unclosed `[`");
            };
            self.pos += 1;
            match c {
                ']' => break,
                '[' => return Err("a watched word regexp with a nested class or POSIX bracket"),
                '&' if self.peek() == Some('&') => {
                    return Err("a watched word regexp with a class intersection");
                }
                '-' if prev.is_some() && !matches!(self.peek(), Some(']') | None) => {
                    let low = prev.take().expect("range start");
                    let high = match self.peek() {
                        Some('\\') => {
                            self.pos += 1;
                            let e = self
                                .peek()
                                .ok_or("a watched word regexp with an unclosed `[`")?;
                            self.pos += 1;
                            self.escaped_char(e, true)?
                                .ok_or("a watched word regexp with a range to a set")?
                        }
                        Some('[') => {
                            return Err(
                                "a watched word regexp with a nested class or POSIX bracket",
                            );
                        }
                        Some(h) => {
                            self.pos += 1;
                            h
                        }
                        None => return Err("a watched word regexp with an unclosed `[`"),
                    };
                    if high < low {
                        return Err("a watched word regexp with a backwards range");
                    }
                    // The range's start is already out.
                    self.out.push('-');
                    self.class_char(high)?;
                }
                '\\' => {
                    let Some(e) = self.peek() else {
                        return Err("a watched word regexp with an unclosed `[`");
                    };
                    self.pos += 1;
                    match e {
                        'd' | 's' => {
                            prev = None;
                            self.out.push_str(self.shorthand(e));
                        }
                        'w' => {
                            if self.fold && self.dialect == Dialect::Ruby {
                                return Err(
                                    "a watched word regexp with `\\w` in a case-insensitive class",
                                );
                            }
                            prev = None;
                            self.out.push_str(self.shorthand(e));
                        }
                        'p' | 'P' => {
                            let name = self.property()?;
                            prev = None;
                            match self.dialect {
                                Dialect::JsLegacy => {
                                    self.out.push(e);
                                    literal('{', &mut self.out);
                                    self.out.push_str(&name);
                                    literal('}', &mut self.out);
                                }
                                Dialect::Ruby if self.fold => {
                                    return Err(
                                        "a watched word regexp with a property in a case-insensitive class",
                                    );
                                }
                                _ => {
                                    let _ = write!(self.out, r"\{e}{{{name}}}");
                                }
                            }
                        }
                        e => {
                            let ch = self.escaped_char(e, true)?.ok_or(
                                "a watched word regexp with an escape the engines read differently",
                            )?;
                            self.class_char(ch)?;
                            prev = Some(ch);
                        }
                    }
                }
                c => {
                    self.class_char(c)?;
                    prev = Some(c);
                }
            }
        }
        if self.out.ends_with('[') || self.out.ends_with("[^") {
            return Err("a watched word regexp with an empty class or a leading `]`");
        }
        self.out.push(']');
        Ok(())
    }

    fn class_char(&mut self, c: char) -> Result<(), &'static str> {
        if self.dialect == Dialect::JsLegacy && c as u32 > 0xFFFF {
            return Err("a watched word regexp with astral characters");
        }
        if self.fold && multi_fold(c) {
            return Err("watched words with full case folding");
        }
        literal(c, &mut self.out);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fancy_regex::Regex;

    fn re(source: &str, dialect: Dialect, cs: bool) -> Regex {
        Regex::new(&translate(source, dialect, cs).unwrap().source).unwrap()
    }

    fn is(source: &str, dialect: Dialect, cs: bool, text: &str) -> bool {
        re(source, dialect, cs).is_match(text).unwrap()
    }

    #[test]
    fn shorthands_per_engine() {
        // Ruby's \w is ASCII and doesn't fold to K (Kelvin); JavaScript's
        // folds with `iu`.
        assert!(!is(r"\w", Dialect::Ruby, false, "\u{212A}"));
        assert!(is(r"\w", Dialect::JsUnicode, false, "\u{212A}"));
        assert!(!is(r"\w", Dialect::JsUnicode, true, "\u{212A}"));
        assert!(!is(r"\w", Dialect::Ruby, true, "é"));
        assert!(!is(r"\d", Dialect::Ruby, true, "٣"));
        assert!(!is(r"\s", Dialect::Ruby, true, "\u{A0}"));
        assert!(is(r"\s", Dialect::JsUnicode, true, "\u{A0}"));
        assert!(is(r"\S", Dialect::Ruby, true, "\u{A0}"));
        assert!(is(r"\W", Dialect::Ruby, false, "\u{212A}"));
    }

    #[test]
    fn word_boundaries_per_engine() {
        // Onigmo's \b is Unicode, superscripts and ZWJ included.
        assert!(!is(r"x\b", Dialect::Ruby, true, "xé"));
        assert!(!is(r"x\b", Dialect::Ruby, true, "x²"));
        assert!(!is(r"x\b", Dialect::Ruby, true, "x\u{200D}"));
        assert!(is(r"x\b", Dialect::Ruby, true, "x-"));
        assert!(is(r"x\b", Dialect::JsUnicode, true, "xé"));
        assert!(is(r"\bcat\b", Dialect::JsUnicode, false, "a CAT!"));
        assert!(!is(r"\Bcat", Dialect::JsUnicode, false, "a cat"));
    }

    #[test]
    fn dots_and_anchors() {
        assert!(is(r"a.b", Dialect::Ruby, true, "a\rb"));
        assert!(!is(r"a.b", Dialect::JsUnicode, true, "a\rb"));
        assert!(is(r"^b", Dialect::Ruby, true, "a\nb"));
        assert!(!is(r"^b", Dialect::JsUnicode, true, "a\nb"));
        assert!(is(r"a$", Dialect::Ruby, true, "a\nb"));
        assert!(!is(r"a$", Dialect::JsUnicode, true, "a\nb"));
    }

    #[test]
    fn properties() {
        assert!(!is(r"\p{Lu}", Dialect::Ruby, false, "é"));
        assert!(is(r"\p{Lu}", Dialect::JsUnicode, false, "é"));
        assert!(is(r"\p{L}", Dialect::JsLegacy, true, "p{L}"));
        assert!(is(r"[\p{L}]", Dialect::JsLegacy, true, "}"));
        assert!(!is(r"[\p{L}]", Dialect::JsLegacy, true, "é"));
    }

    #[test]
    fn classes_and_quantifiers() {
        assert!(is(r"c[a-c]t+s?", Dialect::Ruby, true, "cbttt"));
        assert!(is(r"[^a-z\d]x{2,3}?", Dialect::JsUnicode, true, "!xx"));
        assert!(is(r"[-a]", Dialect::Ruby, true, "-"));
        assert!(is(r"[a-]", Dialect::Ruby, true, "-"));
        assert!(is(r"[\]\-]", Dialect::Ruby, true, "]"));
        assert!(is(r"(?<=a)b(?!c)", Dialect::Ruby, true, "abd"));
        assert_eq!(
            translate(r"(a)|(b(c))", Dialect::Ruby, true)
                .unwrap()
                .groups,
            3
        );
    }

    #[test]
    fn refusals() {
        for bad in [
            r"\1",
            r"(?<n>a)",
            r"(?i)a",
            r"(?>a)",
            r"a*+",
            r"a**",
            r"a{,2}",
            r"[[:alpha:]]",
            r"[a&&b]",
            r"(?<=a+)b",
            r"\h",
            r"\A",
            r"\-",
            r"a{",
            r"]",
            r"}",
            r"[]",
            r"(a",
            r"a)",
            r"x*",
            r"a|",
            r"\p{Alpha}",
            r"[z-a]",
            r"\k<a>",
            r"(?=a)*",
            r"^",
        ] {
            assert!(
                translate(bad, Dialect::Ruby, true).is_err(),
                "{bad} should be refused"
            );
        }
        assert!(translate("ß", Dialect::Ruby, false).is_err());
        assert!(translate("ß", Dialect::Ruby, true).is_ok());
        assert!(translate("😀", Dialect::JsLegacy, true).is_err());
    }
}
