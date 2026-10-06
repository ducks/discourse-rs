//! markdown-it 15's character classes, as Discourse's features take them
//! from `md.utils`. The crate's `is_punct_char` is markdown-it 13's, which
//! counted punctuation only; since 14 symbols count too (`>`, `$`, `+`).

use std::sync::LazyLock;

use regex::Regex;

static SYMBOL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\p{S}$").unwrap());

/// `isPunctChar`: ucmicro's P or S.
pub fn is_punct_char(c: char) -> bool {
    markdown_it::common::utils::is_punct_char(c) || SYMBOL.is_match(c.encode_utf8(&mut [0; 4]))
}

/// `isWhiteSpace`: Zs and the ASCII spaces, by markdown-it's own list.
pub fn is_white_space(c: char) -> bool {
    matches!(
        c,
        '\u{09}'..='\u{0D}'
            | ' '
            | '\u{A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_count_as_punctuation() {
        for c in ['>', '$', '+', '`', '.', '“'] {
            assert!(is_punct_char(c), "{c}");
        }
        assert!(!is_punct_char('a'));
        assert!(is_white_space('\u{A0}'));
        assert!(!is_white_space('\u{2028}'));
        assert!(!is_white_space('\u{85}'));
    }
}
