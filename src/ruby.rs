//! Ruby coercions Discourse's semantics lean on, ported exactly so edge cases
//! (e.g. `"3000abc".to_i == 3000`) match.

/// `String#to_i`: optional leading whitespace and sign, then digits with
/// single underscores between them; stops at the first other char; 0 if none.
pub fn to_i(s: &str) -> i64 {
    let s = s.trim_start();
    let (neg, rest) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };

    let mut n: i64 = 0;
    let mut prev_digit = false;
    let bytes = rest.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b.is_ascii_digit() {
            n = n.saturating_mul(10).saturating_add(i64::from(b - b'0'));
            prev_digit = true;
        } else if b == b'_' && prev_digit && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
            prev_digit = false;
        } else {
            break;
        }
    }
    if neg { -n } else { n }
}

/// `String#to_f`, restricted to the plain decimal forms settings use.
pub fn to_f(s: &str) -> f64 {
    let s = s.trim_start();
    let mut end = 0;
    let bytes = s.as_bytes();
    if matches!(bytes.first(), Some(b'-' | b'+')) {
        end = 1;
    }
    let mut seen_dot = false;
    while end < bytes.len() {
        match bytes[end] {
            b'0'..=b'9' => end += 1,
            b'.' if !seen_dot && bytes.get(end + 1).is_some_and(u8::is_ascii_digit) => {
                seen_dot = true;
                end += 1;
            }
            _ => break,
        }
    }
    s[..end].parse().unwrap_or(0.0)
}

/// `String#blank?`: empty or whitespace only.
pub fn is_blank(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

/// `String#strip`: ASCII whitespace (vertical tab included) and NUL only,
/// so a no-break space stays.
pub fn strip(s: &str) -> &str {
    s.trim_matches(|c| matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r' | '\0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_keeps_no_break_spaces() {
        assert_eq!(strip(" \t\n\x0b\x0c\r\0a b\0 \n"), "a b");
        assert_eq!(strip("a\u{a0}"), "a\u{a0}");
        assert_eq!(strip("\u{a0} a \u{2003}"), "\u{a0} a \u{2003}");
    }

    #[test]
    fn to_i_matches_ruby() {
        assert_eq!(to_i("42"), 42);
        assert_eq!(to_i("  -7"), -7);
        assert_eq!(to_i("+3"), 3);
        assert_eq!(to_i("3000abc"), 3000);
        assert_eq!(to_i("1_000"), 1000);
        assert_eq!(to_i("1__0"), 1);
        assert_eq!(to_i("1_"), 1);
        assert_eq!(to_i(""), 0);
        assert_eq!(to_i("abc"), 0);
        assert_eq!(to_i("-"), 0);
    }

    #[test]
    fn to_f_matches_ruby() {
        assert_eq!(to_f("1.5"), 1.5);
        assert_eq!(to_f("2"), 2.0);
        assert_eq!(to_f("-0.25x"), -0.25);
        assert_eq!(to_f("3."), 3.0);
        assert_eq!(to_f("abc"), 0.0);
        assert_eq!(to_f(""), 0.0);
    }

    #[test]
    fn blank_matches_ruby() {
        assert!(is_blank(""));
        assert!(is_blank(" \t\n"));
        assert!(!is_blank(" x "));
    }
}
