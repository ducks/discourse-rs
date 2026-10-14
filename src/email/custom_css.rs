//! The admin's email CSS (Customize > Email Style): `EmailStyleUpdater`
//! compiling it with Sass, and `Email::Styles#custom_styles` reading the
//! result back with CssParser into declarations per selector, which
//! format_html then adds to the matching elements' style attributes.
//!
//! Only plain rule sets are read: at-rules, nesting the compiler left,
//! escapes, strings holding braces or semicolons, and selectors beyond
//! tags, classes, ids and the descendant and child combinators are
//! refused, since CssParser's and Nokogiri's handling of those is not
//! reproduced.

use crate::Unsupported;

/// `SassC::Engine.new(css, style: :compressed).render` (Dart Sass behind
/// sassc-embedded): the compressed CSS, or the error message's first line.
pub fn compile(css: &str) -> Result<Result<String, String>, Unsupported> {
    // At-rules: imports read files, and Dart Sass prints media queries
    // differently from grass.
    if css.contains('@') {
        return Err(Unsupported("at-rules in the email CSS"));
    }
    let options = grass::Options::default().style(grass::OutputStyle::Compressed);
    match grass::from_string(css.to_string(), &options) {
        Ok(out) if out.is_empty() => Ok(Ok(out)),
        // Dart Sass ends a non-empty stylesheet with a newline.
        Ok(out) => Ok(Ok(format!("{out}\n"))),
        Err(e) => match e.kind() {
            grass::ErrorKind::ParseError { message, .. } => Ok(Err(message
                .split('\n')
                .next()
                .unwrap_or_default()
                .to_string())),
            _ => Err(Unsupported(
                "an email CSS error other than a Sass syntax error",
            )),
        },
    }
}

/// `CssParser::Parser#load_string!` then `each_selector`, collected as
/// `custom_styles` does: each selector's declarations, in the order the
/// selectors first appear, a selector's rules concatenated.
pub fn custom_styles(css: &str) -> Result<Vec<(String, String)>, Unsupported> {
    let css = strip_comments(css)?;
    if css.contains(['@', '\\']) || css.contains("<!--") || css.contains("-->") {
        return Err(Unsupported("at-rules or escapes in the email CSS"));
    }
    let mut quote: Option<char> = None;
    for c in css.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) if matches!(c, '{' | '}' | ';') => {
                return Err(Unsupported("strings holding braces in the email CSS"));
            }
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None => {}
        }
    }
    if quote.is_some() {
        return Err(Unsupported("an unterminated string in the email CSS"));
    }
    let mut styles: Vec<(String, String)> = Vec::new();
    let mut parts: Vec<&str> = css.split('}').collect();
    let rest = parts.pop().unwrap_or_default();
    if !rest.trim().is_empty() {
        return Err(Unsupported("an unclosed rule in the email CSS"));
    }
    for part in parts {
        let Some((selectors, block)) = part.split_once('{') else {
            return Err(Unsupported("a stray brace in the email CSS"));
        };
        if block.contains('{') {
            return Err(Unsupported("nested rules in the email CSS"));
        }
        if selectors.contains(['"', '\'']) {
            return Err(Unsupported("strings in email CSS selectors"));
        }
        let block = block.trim();
        // A rule without declarations is not added.
        if block.is_empty() {
            continue;
        }
        let declarations = declarations_to_s(block)?;
        if declarations.is_empty() {
            return Err(Unsupported("an email CSS rule with no usable declarations"));
        }
        let list: Vec<String> = selectors
            .split(',')
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        for selector in list {
            if !supported_selector(&selector) {
                return Err(Unsupported("this selector in the email CSS"));
            }
            match styles.iter_mut().find(|(s, _)| *s == selector) {
                Some((_, value)) => value.push_str(&declarations),
                None => styles.push((selector, declarations.clone())),
            }
        }
    }
    Ok(styles)
}

/// STRIP_CSS_COMMENTS_RX
fn strip_comments(css: &str) -> Result<String, Unsupported> {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start + 2..].find("*/") else {
            return Err(Unsupported("an unterminated comment in the email CSS"));
        };
        rest = &rest[start + 2 + end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Tags, `*`, classes and ids, joined by descendant or child combinators:
/// what the port's selector engine matches as Nokogiri does.
fn supported_selector(selector: &str) -> bool {
    if selector.is_empty() {
        return false;
    }
    let spaced = selector.replace('>', " > ");
    let mut expect_compound = true;
    for token in spaced.split_whitespace() {
        if token == ">" {
            if expect_compound {
                return false;
            }
            expect_compound = true;
            continue;
        }
        if !supported_compound(token) {
            return false;
        }
        expect_compound = false;
    }
    !expect_compound
}

fn supported_compound(token: &str) -> bool {
    let name = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    let (tag, rest) = match token.find(['.', '#']) {
        Some(i) => token.split_at(i),
        None => (token, ""),
    };
    let tag_ok = tag.is_empty()
        || tag == "*"
        || (tag.starts_with(|c: char| c.is_ascii_lowercase())
            && tag
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    if !tag_ok || (tag.is_empty() && rest.is_empty()) {
        return false;
    }
    // Each `.class` or `#id` after the tag: `rest` starts with one of the
    // two, so the first piece of the split is empty.
    rest.split(['.', '#']).skip(1).all(name)
}

/// `RuleSet#parse_declarations!` then `Declarations#to_s`.
fn declarations_to_s(block: &str) -> Result<String, Unsupported> {
    let mut declarations: Vec<(String, String, bool)> = Vec::new();
    let mut continuation: Option<String> = None;
    for piece in block.split(';') {
        let decs = match continuation.take() {
            Some(c) => format!("{c};{piece}"),
            None => piece.to_string(),
        };
        if decs.matches('(').count() > decs.matches(')').count() {
            continuation = Some(decs);
            continue;
        }
        let Some(colon) = decs.find(':') else {
            continue;
        };
        let property = decs[..colon].trim().to_lowercase();
        let value = decs[colon + 1..].trim();
        if property.is_empty() || value.is_empty() || value.eq_ignore_ascii_case("!important") {
            continue;
        }
        if !property.starts_with(|c: char| c.is_ascii_lowercase() || c == '-')
            || value.contains(['\n', '\r', '\x0c', '\t'])
        {
            return Err(Unsupported("this declaration in the email CSS"));
        }
        // Value#value=: the first `!important` taken out.
        let (value, important) = match find_important(value) {
            Some((start, end)) => (format!("{}{}", &value[..start], &value[end..]), true),
            None => (value.to_string(), false),
        };
        let value = value.trim().to_string();
        if value.is_empty() {
            return Err(Unsupported("an empty declaration in the email CSS"));
        }
        match declarations.iter_mut().find(|(p, _, _)| *p == property) {
            Some((_, v, imp)) => {
                if !*imp || important {
                    *v = value;
                    *imp = important;
                }
            }
            None => declarations.push((property, value, important)),
        }
    }
    if continuation.is_some() {
        return Err(Unsupported("an unclosed parenthesis in the email CSS"));
    }
    let mut out = String::new();
    for (p, v, important) in declarations {
        out.push_str(&format!(
            "{p}: {v}{}; ",
            if important { " !important" } else { "" }
        ));
    }
    Ok(out.trim().to_string())
}

/// IMPORTANT_IN_PROPERTY_RX (`/\s*!important\b\s*/i`): where its first
/// match starts and ends.
fn find_important(value: &str) -> Option<(usize, usize)> {
    let lower = value.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find("!important") {
        let at = from + i;
        let after = at + "!important".len();
        let boundary = lower[after..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        if boundary {
            let start = value[..at].trim_end().len();
            let end = after + (value[after..].len() - value[after..].trim_start().len());
            return Some((start, end));
        }
        from = after;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_like_dart_sass() {
        let ok = |css: &str| compile(css).unwrap().unwrap();
        assert_eq!(ok("a { color: red; }"), "a{color:red}\n");
        assert_eq!(
            ok(".foo .bar { color: #ffffff; padding: 0.5em 0px; }"),
            ".foo .bar{color:#fff;padding:.5em 0px}\n"
        );
        assert_eq!(
            ok("$c: blue;\nh1, h2 { b { color: $c } }"),
            "h1 b,h2 b{color:blue}\n"
        );
        assert_eq!(ok(""), "");
        assert_eq!(
            compile("a { color: red").unwrap(),
            Err("expected \"}\".".into())
        );
        assert_eq!(
            compile("a { color: $x; }").unwrap(),
            Err("Undefined variable.".into())
        );
        assert!(compile("@import 'x';").is_err());
    }

    #[test]
    fn reads_declarations_per_selector() {
        assert_eq!(
            custom_styles("p{color:#333}.body a,h1{font-size:14px !important}\n").unwrap(),
            vec![
                ("p".to_string(), "color: #333;".to_string()),
                (
                    ".body a".to_string(),
                    "font-size: 14px !important;".to_string()
                ),
                ("h1".to_string(), "font-size: 14px !important;".to_string()),
            ]
        );
        assert_eq!(
            custom_styles("a { color: red; COLOR: blue } a{margin:0}").unwrap(),
            vec![("a".to_string(), "color: blue;margin: 0;".to_string())]
        );
        assert_eq!(
            custom_styles("a{color:red !important;color:blue}").unwrap(),
            vec![("a".to_string(), "color: red !important;".to_string())]
        );
        assert_eq!(
            custom_styles("a{background:url(x;y.png)} b{}").unwrap(),
            vec![("a".to_string(), "background: url(x;y.png);".to_string())]
        );
        assert!(custom_styles("a:hover{color:red}").is_err());
        assert!(custom_styles("@media print{a{color:red}}").is_err());
        assert!(custom_styles("div > p.x#y, *.z {color:red}").is_ok());
        assert!(custom_styles("a[href]{color:red}").is_err());
    }
}
