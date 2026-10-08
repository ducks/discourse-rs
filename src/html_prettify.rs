//! Port of `HtmlPrettify` (lib/html_prettify.rb), Discourse's SmartyPants,
//! with the options `Topic.fancy_title` uses (`[2]`): quotes, backticks,
//! ellipses, fractions and old-school dashes (`---` em, `--` en).
//!
//! Ruby's `\w`, `\d` and `\s` are ASCII here while `\b` and `\B` follow
//! Unicode word characters, so the patterns spell the classes out.

use std::sync::LazyLock;

use fancy_regex::Regex as FancyRegex;
use regex::Regex;

const LSQUO: &str = "&lsquo;";
const LDQUO: &str = "&ldquo;";
const RSQUO: &str = "&rsquo;";
const RDQUO: &str = "&rdquo;";
const MDASH: &str = "&mdash;";
const NDASH: &str = "&ndash;";
const HELLIP: &str = "&hellip;";

const W: &str = "[A-Za-z0-9_]";
const S: &str = "[ \\t\\r\\n\\x0B\\x0C]";
const PUNCT: &str = r##"[!"#$%'()*+,\-./:;<=>?@\[\\\]^_`{|}~]"##;

fn fancy(pattern: &str) -> FancyRegex {
    FancyRegex::new(pattern).expect("HtmlPrettify pattern")
}

static TAG_SOUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([^<]*)(<[^>]*>)").unwrap());
static PRE_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<(/?)(?:pre|code|kbd|script|math)[\s>]").unwrap());
static FRACTIONS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?m)({S}+|^)(1/4|1/2|3/4)([,.;]|{S}|$)")).unwrap());

/// `educate_quotes`' substitutions, in order.
static QUOTE_RULES: LazyLock<Vec<(FancyRegex, String)>> = LazyLock::new(|| {
    let dec_dashes = format!("{NDASH}|{MDASH}");
    let opener = format!(r"({S}|&nbsp;|=|--|&[mn]dash;|{dec_dashes}|&#x201[34];)");
    let close = r"([^ \t\r\n\[{(\-])";
    vec![
        (fancy(&format!(r"\A'(?={PUNCT}\B)")), RSQUO.into()),
        (fancy(&format!(r#"\A"(?={PUNCT}\B)"#)), RDQUO.into()),
        (fancy(&format!(r#""'(?={W})"#)), format!("{LDQUO}{LSQUO}")),
        (fancy(&format!(r#"'"(?={W})"#)), format!("{LSQUO}{LDQUO}")),
        (fancy(r"'(?=[0-9][0-9]s)"), RSQUO.into()),
        (
            fancy(&format!("{opener}'(?={W})")),
            format!("${{1}}{LSQUO}"),
        ),
        (fancy(&format!("{close}'")), format!("${{1}}{RSQUO}")),
        (
            fancy(&format!(r"(?m)'({S}|s\b|$)")),
            format!("{RSQUO}${{1}}"),
        ),
        (fancy("'"), LSQUO.into()),
        (
            fancy(&format!(r#"{opener}"(?={W})"#)),
            format!("${{1}}{LDQUO}"),
        ),
        (fancy(&format!(r#"{close}""#)), format!("${{1}}{RDQUO}")),
        (
            fancy(&format!(r#"(?m)"({S}|s\b|$)"#)),
            format!("{RDQUO}${{1}}"),
        ),
        (fancy("\""), LDQUO.into()),
    ]
});

enum Token<'a> {
    Tag(&'a str),
    Text(&'a str),
}

fn tokenize(s: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut prev_end = 0;
    for c in TAG_SOUP.captures_iter(s) {
        let text = c.get(1).unwrap().as_str();
        if !text.is_empty() {
            tokens.push(Token::Text(text));
        }
        tokens.push(Token::Tag(c.get(2).unwrap().as_str()));
        prev_end = c.get(0).unwrap().end();
    }
    if prev_end < s.len() {
        tokens.push(Token::Text(&s[prev_end..]));
    }
    tokens
}

/// `HtmlPrettify.render(html)`.
pub fn render(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_pre = false;
    let mut prev_last_char: Option<char> = None;
    for token in tokenize(html) {
        match token {
            Token::Tag(tag) => {
                result.push_str(tag);
                if let Some(c) = PRE_TAG.captures(tag) {
                    in_pre = &c[1] != "/";
                }
            }
            Token::Text(text) => {
                let last_char = text.chars().last();
                let t = if in_pre {
                    text.to_string()
                } else {
                    educate(text, prev_last_char)
                };
                prev_last_char = last_char;
                result.push_str(&t);
            }
        }
    }
    result
}

fn educate(text: &str, prev_last_char: Option<char>) -> String {
    let t = text.replace("&#39;", "'").replace("&quot;", "\"");
    let t = t.replace("---", MDASH).replace("--", NDASH);
    let t = t.replace("...", HELLIP).replace(". . .", HELLIP);
    let t = FRACTIONS
        .replace_all(&t, |c: &regex::Captures| {
            let frac = match &c[2] {
                "1/2" => "&frac12;",
                "1/4" => "&frac14;",
                _ => "&frac34;",
            };
            format!("{}{frac}{}", &c[1], &c[3])
        })
        .into_owned();
    let t = t.replace("``", LDQUO).replace("''", RDQUO);
    let after_text = prev_last_char.is_some_and(|c| !" \t\r\n\x0B\x0C".contains(c));
    match t.as_str() {
        "'" if after_text => RSQUO.into(),
        "'" => LSQUO.into(),
        "\"" if after_text => RDQUO.into(),
        "\"" => LDQUO.into(),
        _ => educate_quotes(t),
    }
}

fn educate_quotes(mut s: String) -> String {
    for (re, rep) in QUOTE_RULES.iter() {
        s = re.replace_all(&s, rep.as_str()).into_owned();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn tags_and_pre_blocks() {
        assert_eq!(
            render("<p>it's</p><code>it's -- x</code> \"done\""),
            "<p>it&rsquo;s</p><code>it's -- x</code> &ldquo;done&rdquo;"
        );
        assert_eq!(render("<b>\"</b>"), "<b>&ldquo;</b>");
        assert_eq!(render("x<b>\"</b>"), "x<b>&rdquo;</b>");
    }
}
