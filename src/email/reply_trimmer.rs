//! Port of the email_reply_trimmer gem (0.4.0, MIT, Discourse), which
//! Email::Receiver uses to cut quoted replies, signatures and forwarded
//! headers out of an incoming email's text.
//!
//! The gem's regexes are Ruby's, translated for fancy-regex: `^` and `$`
//! always match at lines (`(?m)`), Ruby's `/m` is `(?s)`, and `\w`, `\d`
//! and `\s` are ASCII while `\b` and the POSIX classes are Unicode, as in
//! Onigmo. `[[:blank:]]` is `[\t\p{Zs}]`, `[[:space:]]` is `\s` (Unicode
//! White_Space), `[[:alpha:]]` is `\p{Alphabetic}` and `[[:word:]]` is
//! Unicode `\w`.

use std::sync::LazyLock;

use fancy_regex::{Captures, Regex};

const DELIMITER: char = 'd';
const EMBEDDED: char = 'b';
const EMPTY: char = 'e';
const EMAIL_HEADER: char = 'h';
const QUOTE: char = 'q';
const SIGNATURE: char = 's';
const TEXT: char = 't';

/// Ruby's ASCII classes and Onigmo's Unicode POSIX ones.
const W: &str = "[A-Za-z0-9_]";
const BLANK: &str = r"[\t\p{Zs}]";

fn re(pattern: &str) -> Regex {
    Regex::new(&format!("(?m){pattern}")).unwrap_or_else(|e| panic!("{pattern}: {e}"))
}

/// A gem regex in Rust syntax: Ruby's ASCII escapes first (inside the
/// classes the gem uses them in, then bare), then the POSIX classes.
fn ruby(pattern: &str) -> String {
    pattern
        .replace(r"[\w.+-]", "[A-Za-z0-9_.+-]")
        .replace(r"[\w.-]", "[A-Za-z0-9_.-]")
        .replace(r"[\w[:blank:]]", r"[A-Za-z0-9_\t\p{Zs}]")
        .replace(r"\w", W)
        .replace(r"\s", r"[ \t\r\n\f\x0B]")
        .replace(r"\d", "[0-9]")
        .replace("[[:blank:]]", BLANK)
        .replace("[[:blank:]*]", r"[\t\p{Zs}*]")
        .replace("[[:blank:]<>-]", r"[\t\p{Zs}<>-]")
        .replace("[[:blank:].:>-]", r"[\t\p{Zs}.:>-]")
        .replace("[[:blank:]>]", r"[\t\p{Zs}>]")
        .replace("[[:blank:]>*]", r"[\t\p{Zs}>*]")
        .replace("[[:space:]]", r"\p{White_Space}")
        .replace("[[:alpha:]]", r"\p{Alphabetic}")
        .replace("[[:word:]]", r"[\p{Alphabetic}\p{M}\p{Nd}\p{Pc}]")
}

fn regexes(patterns: &[&str]) -> Vec<Regex> {
    patterns.iter().map(|p| re(&ruby(p))).collect()
}

/// DelimiterMatcher
static DELIMITER_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"^{BLANK}*[{}]+{BLANK}*$",
        fancy_regex::escape("-_,=+~#*ᐧ—")
    ))
});
static BULLET_MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"\A{BLANK}*[-*]{BLANK}*\z")).unwrap());

fn delimiter(line: &str) -> bool {
    !BULLET_MARKER.is_match(line).unwrap_or(false) && DELIMITER_RE.is_match(line).unwrap_or(false)
}

/// SignatureMatcher
static SIGNATURES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    regexes(&[
        r"(?i)^[[:blank:]]*從我的 iPhone 傳送",
        r"(?i)^[[:blank:]]*[[:word:]]+ from mobile",
        r"(?i)^[[:blank:]]*[\(<]*Sent (from|via|with|by) .+[\)>]*",
        r"(?i)^[[:blank:]]*From my .{1,20}",
        r"(?i)^[[:blank:]]*Get Outlook for ",
        r"(?i)^[[:blank:]]*Envoyé depuis (mon|Yahoo Mail)",
        r"(?i)^[[:blank:]]*Von meinem .+ gesendet",
        r"(?i)^[[:blank:]]*Diese Nachricht wurde von .+ gesendet",
        r"(?i)^[[:blank:]]*Inviato da ",
        r"(?i)^[[:blank:]]*Sendt fra min ",
        r"(?i)^[[:blank:]]*Enviado do meu ",
        r"(?i)^[[:blank:]]*Enviado desde mi ",
        r"(?i)^[[:blank:]]*Verzonden met ",
        r"(?i)^[[:blank:]]*Verstuurd vanaf mijn ",
        r"(?i)^[[:blank:]]*från min ",
    ])
});
static MARKDOWN_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\([^\)]+\)").unwrap());

fn signature(line: &str) -> bool {
    let stripped = MARKDOWN_LINK.replace_all(line, "$1");
    SIGNATURES
        .iter()
        .any(|r| r.is_match(&stripped).unwrap_or(false))
}

/// EmbeddedEmailMatcher: the regexes preprocess! joins over lines first.
static ON_DATE_SOMEONE_WROTE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    regexes(&[
        r"(?is)^[[:blank:]<>-]*在 (?:(?!\b(?>在|写道)\b).)+?写道[[:blank:].:>-]*$",
        r"(?is)^[[:blank:]<>-]*Op (?:(?!\b(?>Op|het\svolgende\sgeschreven|schreef)\b).)+?(het\svolgende\sgeschreven|schreef[^:]+)[[:blank:].:>-]*$",
        r"(?is)^[[:blank:]<>-]*In message (?:(?!\b(?>In message|writes)\b).)+?writes[[:blank:].:>-]*$",
        r"(?is)^[[:blank:]<>-]*(On|At) (?:(?!\b(?>On|wrote|writes|says|said)\b).)+?(wrote|writes|says|said)[[:blank:].:>-]*$",
        r"(?is)^[[:blank:]<>-]*Le (?:(?!\b(?>Le|nous\sa\sdit|a\s+écrit)\b).)+?(nous\sa\sdit|a\s+écrit)[[:blank:].:>-]*$",
        r"(?is)^[[:blank:]<>-]*Am (?:(?!\b(?>Am|schrieben\sSie)\b).)+?schrieben\sSie[[:blank:].:>-]*$",
        r"(?is)^[[:blank:]<>-]*Am (?:(?!\b(?>Am|geschrieben)\b).)+?(geschrieben|schrieb[^:]+)[[:blank:].:>-]*$",
        r"(?is)^[[:blank:]<>-]*Il (?:(?!\b(?>Il|ha\sscritto)\b).)+?ha\sscritto[[:blank:].:>-]*$",
        r"(?is)^[[:blank:]<>-]*(Dnia|Dňa) (?:(?!\b(?>Dnia|Dňa|napisał)\b).)+?napisał(\(a\))?[[:blank:].:>-]*$",
        r"(?is)^[[:blank:]<>-]*Em (?:(?!\b(?>Em|escreveu)\b).)+?escreveu[[:blank:].:>-]*$",
        r"(?is)^[[:blank:]<>-]*El (?:(?!\b(?>El|escribió)\b).)+?escribió[[:blank:].:>-]*$",
    ])
});
static SOMEONE_WROTE_ON_DATE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    regexes(&[
        r"^.+\bwrote\b[[:space:]]+\bon\b.+[^:]+:",
        r"^.+\bschrieb\b[[:space:]]+\bam\b.+[^:]+:",
    ])
});
static DATE_SOMEONE_WROTE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    regexes(&[
        r"\d{4}.{1,80}пользователь.{0,80}\n?.{0,80}?написал:",
        r"\d{4}.{1,80}\n?.{0,80}?napisał\(a\):",
        r"\d{4}.{1,80}\n?.{0,80}?пише:",
    ])
});
static DATE_SOMEONE_EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    re(&ruby(
        r"(?i)(?!.*<img\b)\d{4}.{1,80}\s?<[^@<>]+@[^@<>.]+\.[^@<>]+>:?$",
    ))
});
static EMBEDDED_OTHERS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    regexes(&[
        // ON_DATE_WROTE_SOMEONE
        r"^[[:blank:]>]*Op\s.+\sschreef\s[^:]+:",
        r"^[[:blank:]>]*Am\s.+\sschrieb\s[^:]+:",
        r"^[[:blank:]>]*Den\s.+\sskrev\s[^:]+:",
        r"^[[:blank:]>]*søn\.\s.+\sskrev\s[^:]+:",
        // ISO_DATE_SOMEONE
        r"^[[:blank:]>]*20\d\d-\d\d-\d\d \d\d:\d\d GMT\+\d\d:\d\d [\w[:blank:]]+$",
        // SOMEONE_VIA_SOMETHING_WROTE
        r"^.+ via .+ wrote:?[[:blank:]]*$",
        r"^.+ via .+ schrieb:?[[:blank:]]*$",
        // SOMEONE_EMAIL_WROTE
        r"^.+\b[\w.+-]+@[\w.-]+\.\w{2,}\b.+wrote:?$",
        // POSTED_BY_SOMEONE_ON_DATE
        r"(?i)^[[:blank:]>]*Posted by .+ on \d{2}/\d{2}/\d{4}$",
        // FORWARDED_EMAIL
        r"(?i)^[[:blank:]>]*Begin forwarded message:",
        r"(?i)^[[:blank:]>*]*-{2,}[[:blank:]]*(Forwarded|Original|Reply) Message[[:blank:]]*-{2,}",
        r"(?i)^[[:blank:]>]*Début du message transféré :",
        r"(?i)^[[:blank:]>*]*-{2,}[[:blank:]]*Message transféré[[:blank:]]*-{2,}",
        r"(?i)^[[:blank:]>*]*-{2,}[[:blank:]]*Ursprüngliche Nachricht[[:blank:]]*-{2,}",
        r"(?i)^[[:blank:]>*]*-{2,}[[:blank:]]*Mensaje original[[:blank:]]*-{2,}",
        r"(?i)^[[:blank:]>*]*-{2,}[[:blank:]]*原始邮件[[:blank:]]*-{2,}",
    ])
});

fn embedded(line: &str) -> bool {
    let m = |r: &Regex| r.is_match(line).unwrap_or(false);
    // EMBEDDED_REGEXES' order, which only matters for speed.
    ON_DATE_SOMEONE_WROTE.iter().any(m)
        || EMBEDDED_OTHERS[..4].iter().any(m)
        || DATE_SOMEONE_WROTE.iter().any(m)
        || m(&DATE_SOMEONE_EMAIL)
        || SOMEONE_WROTE_ON_DATE.iter().any(m)
        || EMBEDDED_OTHERS[4..].iter().any(m)
}

/// EmailHeaderMatcher
static EMAIL_HEADERS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    let with_date = [
        "Sendt",
        "Sent|Date",
        "Date|Le",
        "Gesendet",
        "Enviada em",
        "Enviado",
        "Fecha",
        "Data",
        "Datum",
        "Skickat",
        "发送时间",
    ];
    let with_text = [
        "Fra|Til|Emne",
        "From|To|Cc|Reply-To|Subject",
        "De|Expéditeur|À|Destinataire|Répondre à|Objet",
        "Von|An|Betreff",
        "De|Para|Assunto",
        "De|Para|Asunto",
        "Da|Risposta|A|Oggetto",
        "Van|Beantwoorden - Aan|Aan|Onderwerp",
        "Från|Till|Ämne",
        "发件人|收件人|主题",
    ];
    let mut out: Vec<Regex> = with_date
        .iter()
        .map(|h| re(&ruby(&format!(r"^[[:blank:]*]*(?:{h})[[:blank:]*]*:.*\d+"))))
        .collect();
    out.extend(with_text.iter().map(|h| {
        re(&ruby(&format!(
            r"(?i)^[[:blank:]*]*(?:{h})[[:blank:]*]*:.*[[:word:]]+"
        )))
    }));
    out
});

static EMPTY_LINE: LazyLock<Regex> = LazyLock::new(|| re(&format!("^{BLANK}*$")));
static QUOTE_LINE: LazyLock<Regex> = LazyLock::new(|| re(&format!("^{BLANK}*>")));

fn identify_line_content(line: &str) -> char {
    if EMPTY_LINE.is_match(line).unwrap_or(false) {
        EMPTY
    } else if delimiter(line) {
        DELIMITER
    } else if signature(line) {
        SIGNATURE
    } else if embedded(line) {
        EMBEDDED
    } else if EMAIL_HEADERS
        .iter()
        .any(|r| r.is_match(line).unwrap_or(false))
    {
        EMAIL_HEADER
    } else if QUOTE_LINE.is_match(line).unwrap_or(false) {
        QUOTE
    } else {
        TEXT
    }
}

/// Ruby's `String#strip`: ASCII whitespace and NUL off both ends.
pub fn ruby_strip(s: &str) -> &str {
    s.trim_matches(|c: char| matches!(c, '\0' | '\t' | '\n' | '\x0B' | '\x0C' | '\r' | ' '))
}

/// Ruby's `String#split("\n")`: trailing empty pieces dropped.
fn split_lines(s: &str) -> Vec<String> {
    let mut lines: Vec<String> = s.split('\n').map(str::to_string).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

/// `pattern =~ re`: the char index of the first match in the pattern
/// string (ASCII, so bytes are chars).
fn find(pattern: &str, re: &str) -> Option<usize> {
    Regex::new(re)
        .expect("a pattern regex")
        .find(pattern)
        .ok()
        .flatten()
        .map(|m| m.start())
}

static PGP_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\A-----BEGIN PGP SIGNED MESSAGE-----\n(?:Hash: [A-Za-z0-9_]+)?[ \t\r\n\f\x0B]+",
    )
    .unwrap()
});
static PGP_SIGNATURE: LazyLock<Regex> =
    LazyLock::new(|| re(r"^-----BEGIN PGP SIGNATURE-----$[\s\S]+^-----END PGP SIGNATURE-----"));
static UNSUBSCRIBE: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)^Unsubscribe: .+@.+(\n.+http:.+)?[ \t\r\n\f\x0B]*\z"));
static ALIAS_QUOTE: LazyLock<Regex> = LazyLock::new(|| re(r#"^.*>{5} "[^"\n]+" == .+ writes:"#));
static ENCLOSED_NAMED: LazyLock<Regex> =
    LazyLock::new(|| re(r"^>>> ?(.+) ?>>>$\n([\s\S]+?)\n^<<< ?\1 ?<<<$"));
static ENCLOSED: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"^>{{4,}}{BLANK}*$\n([\s\S]+?)\n^<{{4,}}{BLANK}*$"
    ))
});
static QUOTE_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| re(&format!(r"^((?:{BLANK}*\p{{Alphabetic}}*[>|])+)")));
static QUOTE_MARK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\p{Alphabetic}+>|\|)").unwrap());
static LINE_START: LazyLock<Regex> = LazyLock::new(|| re("^"));
static JOINED_NEWLINES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n+\s*").unwrap());

fn gsub(re: &Regex, text: &str, f: impl Fn(&Captures) -> String) -> String {
    re.replace_all(text, |c: &Captures| f(c)).into_owned()
}

/// `preprocess!`
fn preprocess(text: &str) -> String {
    let mut text = text.replace("\r\n", "\n");
    text = PGP_HEADER.replace(&text, "").into_owned();
    text = PGP_SIGNATURE.replace_all(&text, "").into_owned();
    text = UNSUBSCRIBE.replace_all(&text, "").into_owned();
    text = ALIAS_QUOTE.replace_all(&text, "").into_owned();
    let quote_lines = |s: &str| LINE_START.replace_all(s, "> ").into_owned();
    text = gsub(&ENCLOSED_NAMED, &text, |c| quote_lines(&c[2]));
    text = gsub(&ENCLOSED, &text, |c| quote_lines(&c[1]));
    text = gsub(&QUOTE_PREFIX, &text, |c| {
        QUOTE_MARK.replace_all(&c[1], ">").into_owned()
    });
    for r in ON_DATE_SOMEONE_WROTE
        .iter()
        .chain(SOMEONE_WROTE_ON_DATE.iter())
        .chain(DATE_SOMEONE_WROTE.iter())
        .chain(std::iter::once(&*DATE_SOMEONE_EMAIL))
    {
        text = gsub(r, &text, |c| {
            let m = &c[0];
            if m.matches('\n').count() > 4 {
                m.to_string()
            } else {
                JOINED_NEWLINES.replace_all(m, " ").into_owned()
            }
        });
    }
    ruby_strip(&text).to_string()
}

static CODE_BLOCK: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)^```[A-Za-z0-9_]*$\n.*?^```$"));

/// `EmailReplyTrimmer.trim(text, true)`: the reply and what was cut.
/// None for a blank text.
pub fn trim(text: &str) -> Option<(String, String)> {
    if text.chars().all(char::is_whitespace) {
        return None;
    }
    let text = preprocess(text);

    let (text, blocks) = hoist_code_blocks(&text);

    let mut lines = split_lines(&text);
    let lines_dup = lines.clone();
    let mut pattern: String = lines.iter().map(|l| identify_line_content(l)).collect();

    // remove everything after the first delimiter
    if let Some(index) = find(&pattern, "d") {
        let underscore_separator_before_embedded_email = Regex::new(r"\A_+\z")
            .unwrap()
            .is_match(&lines[index])
            .unwrap_or(false)
            && pattern[index + 1..].contains('b');
        if !underscore_separator_before_embedded_email {
            pattern.truncate(index);
            lines.truncate(index);
        }
    }

    // remove all mobile signatures
    while let Some(index) = pattern.find('s') {
        pattern.remove(index);
        lines.remove(index);
    }

    // when the reply is at the end of the email
    if find(&pattern, "^b[^t]+t[et]*$").is_some() {
        let index = find(&pattern, "t[et]*$").expect("matched above");
        pattern.clear();
        lines = lines.split_off(index);
    }

    // an embedded email marker not followed by a quote
    if let Some(index) = find(&pattern, "te*b[^q]*$") {
        pattern.truncate(index + 1);
        lines.truncate(index + 1);
    }

    // an embedded email marker followed by a huge quote
    let huge = Regex::new("te*b[eqbh]*([te]*)$").unwrap();
    if let Ok(Some(c)) = huge.captures(&pattern) {
        let texts = c.get(1).map_or(0, |m| m.as_str().matches('t').count());
        if texts < 7 && find(&pattern, "bq[eqbh]*t").is_none() {
            let index = find(&pattern, "te*b[eqbh]*[te]*$").expect("matched above");
            pattern.truncate(index + 1);
            lines.truncate(index + 1);
        }
    }

    // some text before a huge quote ending the email
    if let Some(index) = find(&pattern, "t?e*[qbe]+$") {
        pattern.truncate(index + 1);
        lines.truncate(index + 1);
    }

    // remaining embedded email markers
    while let Some(index) = pattern.find('b') {
        pattern.remove(index);
        lines.remove(index);
    }

    // email headers spanning several lines
    if let Ok(Some(m)) = Regex::new("h+[hte]+h+e").unwrap().find(&pattern) {
        let (start, len) = (m.start(), m.as_str().len());
        pattern.replace_range(start..start + len, &"h".repeat(len));
    }

    // at least 3 consecutive email headers: everything up to them
    if let Some(index) = find(&pattern, "t[eq]*h{3,}") {
        pattern.truncate(index + 1);
        lines.truncate(index + 1);
    }

    // remaining email headers
    while let Some(index) = pattern.find('h') {
        pattern.remove(index);
        lines.remove(index);
    }

    // trailing quotes after some text
    if pattern.contains('t')
        && let Some(index) = find(&pattern, "[eq]+$")
    {
        pattern.truncate(index);
        lines.truncate(index);
    }

    let mut trimmed = ruby_strip(&lines.join("\n")).to_string();
    for (token, block) in &blocks {
        trimmed = trimmed.replace(token.as_str(), block);
    }
    Some((trimmed, compute_elided(&lines_dup, &lines)))
}

/// `hoist_code_blocks`: each fenced block swapped for a random token.
fn hoist_code_blocks(text: &str) -> (String, Vec<(String, String)>) {
    let mut blocks = Vec::new();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for m in CODE_BLOCK.find_iter(text).flatten() {
        let token = crate::accounts::random_hex();
        out.push_str(&text[last..m.start()]);
        out.push_str(&token);
        blocks.push((token, m.as_str().to_string()));
        last = m.end();
    }
    out.push_str(&text[last..]);
    (out, blocks)
}

/// `compute_elided`: the lines the trim dropped, in order.
fn compute_elided(text: &[String], lines: &[String]) -> String {
    let mut elided: Vec<&str> = Vec::new();
    let (mut t, mut l) = (0, 0);
    while t < text.len() {
        while l < lines.len() && t < text.len() && text[t] == lines[l] {
            t += 1;
            l += 1;
        }
        elided.push(text.get(t).map_or("", String::as_str));
        t += 1;
    }
    ruby_strip(&elided.join("\n")).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delimiters_and_bullets() {
        assert!(delimiter("--"));
        assert!(delimiter("  ____ "));
        assert!(!delimiter("-"));
        assert!(!delimiter(" * "));
    }

    #[test]
    fn ruby_split_and_strip() {
        assert_eq!(split_lines("a\n\nb\n\n"), vec!["a", "", "b"]);
        assert_eq!(ruby_strip("\0 a\u{a0}\n"), "a\u{a0}");
    }
}
