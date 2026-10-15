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

/// `Float#to_s` for the values settings hold: a whole number keeps its
/// `.0`; beyond 1e16 Ruby switches to exponent form, refused by callers'
/// ranges in practice and written as Rust does.
pub fn float_to_s(f: f64) -> String {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e16 {
        format!("{f:.1}")
    } else {
        f.to_string()
    }
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

/// `String#split(sep)` with a non-empty string separator: trailing empty
/// fields dropped, leading and inner ones kept.
pub fn split<'a>(s: &'a str, sep: &str) -> Vec<&'a str> {
    let mut parts: Vec<&str> = s.split(sep).collect();
    while parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    parts
}

/// `CGI.escape`
pub fn cgi_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b' ' => out.push('+'),
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `String#inspect`: quoted, with `"`, `\` and the control characters
/// escaped (`#` too before `{`, `$` or `@`).
pub fn inspect_string(s: &str) -> String {
    let mut out = String::from("\"");
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\u{1b}' => out.push_str("\\e"),
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{b}' => out.push_str("\\v"),
            '\u{c}' => out.push_str("\\f"),
            '#' if matches!(chars.peek(), Some('{' | '$' | '@')) => out.push_str("\\#"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\x{:02X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `Kernel#Integer(string)`: surrounding whitespace, a sign, a `0x`,
/// `0b`, `0o`, `0d` or bare `0` (octal) prefix, digits with single
/// underscores between them, and nothing else; None where Ruby raises.
pub fn integer(s: &str) -> Option<i64> {
    let s = s.trim_matches(|c: char| c.is_ascii_whitespace() || c == '\x0b');
    let (neg, s) = match s.as_bytes().first()? {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    let lower = s.to_ascii_lowercase();
    let (radix, digits) = if let Some(rest) = lower.strip_prefix("0x") {
        (16, rest)
    } else if let Some(rest) = lower.strip_prefix("0b") {
        (2, rest)
    } else if let Some(rest) = lower.strip_prefix("0o") {
        (8, rest)
    } else if let Some(rest) = lower.strip_prefix("0d") {
        (10, rest)
    } else if lower.len() > 1 && lower.starts_with('0') {
        (8, &lower[1..])
    } else {
        (10, lower.as_str())
    };
    // Underscores only between digits.
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
    {
        return None;
    }
    let n = i64::from_str_radix(&digits.replace('_', ""), radix).ok()?;
    Some(if neg { -n } else { n })
}

/// `ActiveModel::Type::Boolean.new.cast`: nil for nothing or "", false
/// for the false values, else true.
pub fn cast_boolean(v: Option<&str>) -> Option<bool> {
    match v? {
        "" => None,
        "0" | "f" | "F" | "false" | "FALSE" | "off" | "OFF" => Some(false),
        _ => Some(true),
    }
}

/// `Array#sort!` with a block on the reference: MRI's ruby_qsort is glibc's
/// qsort_r there (HAVE_GNU_QSORT_R), whose msort_with_tmp is a top-down
/// merge sort taking the left element while the block says <= 0. Blocks
/// that are not a total order (the site texts' and color palettes' sorts)
/// end as that algorithm leaves them, which this reproduces.
pub fn sort_by_block<T>(items: &mut Vec<T>, cmp: &mut impl FnMut(&T, &T) -> std::cmp::Ordering) {
    fn msort<T>(mut items: Vec<T>, cmp: &mut impl FnMut(&T, &T) -> std::cmp::Ordering) -> Vec<T> {
        let n = items.len();
        if n <= 1 {
            return items;
        }
        let right = items.split_off(n / 2);
        let left = msort(items, cmp);
        let right = msort(right, cmp);
        let mut out = Vec::with_capacity(n);
        let mut left = left.into_iter().peekable();
        let mut right = right.into_iter().peekable();
        while let (Some(l), Some(r)) = (left.peek(), right.peek()) {
            if cmp(l, r) != std::cmp::Ordering::Greater {
                out.extend(left.next());
            } else {
                out.extend(right.next());
            }
        }
        out.extend(left);
        out.extend(right);
        out
    }
    let taken = std::mem::take(items);
    *items = msort(taken, cmp);
}

/// `Hash#inspect` (and `Array#inspect`) of a deep-symbolized parameter
/// hash, as Ruby 3.4 writes it: `{key: value}`, a key that isn't a plain
/// symbol quoted (`"a b": 1`).
pub fn inspect_value(value: &serde_json::Value) -> String {
    use serde_json::Value;
    match value {
        Value::Null => "nil".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => inspect_string(s),
        Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(inspect_value)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(map) => {
            if map.is_empty() {
                return "{}".into();
            }
            let pairs: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let plain = k
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                        && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                    let key = if plain { k.clone() } else { inspect_string(k) };
                    format!("{key}: {}", inspect_value(v))
                })
                .collect();
            format!("{{{}}}", pairs.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_drops_trailing_empty_fields() {
        assert_eq!(split("a:b", ":"), vec!["a", "b"]);
        assert_eq!(split("a::", ":"), vec!["a"]);
        assert_eq!(split(":a", ":"), vec!["", "a"]);
        assert!(split(":", ":").is_empty());
        assert_eq!(split("a,,b", ","), vec!["a", "", "b"]);
    }

    #[test]
    fn integer_parses_as_kernel_integer() {
        assert_eq!(integer(" 12 "), Some(12));
        assert_eq!(integer("-1_000"), Some(-1000));
        assert_eq!(integer("0x1f"), Some(31));
        assert_eq!(integer("010"), Some(8));
        assert_eq!(integer("0b11"), Some(3));
        assert_eq!(integer("0"), Some(0));
        for bad in ["", "abc", "1_", "1__0", "12abc", "08", "-", " "] {
            assert_eq!(integer(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn sort_by_block_matches_the_reference() {
        // Recorded with Ruby 3.4.9 on glibc 2.36: a block that puts zeros
        // first without comparing them to each other.
        let a = [3, 0, 2, 0, 1, 3, 0, 2, 1, 0, 3];
        let mut idx: Vec<usize> = (0..a.len()).collect();
        sort_by_block(&mut idx, &mut |&x, &y| {
            if a[x] == 0 {
                std::cmp::Ordering::Less
            } else if a[y] == 0 {
                std::cmp::Ordering::Greater
            } else {
                a[x].cmp(&a[y])
            }
        });
        assert_eq!(idx, [1, 3, 6, 9, 4, 8, 2, 7, 0, 5, 10]);
    }

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
