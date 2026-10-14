//! `WordWatcher` (app/services/word_watcher.rb) and `WatchedWord`'s
//! actions: the watched words of an action, the regexps built from them
//! (one source string serves Ruby on the server and JavaScript in
//! cooking and the client), and the server-side matching, censoring and
//! replacing, which run them as Ruby does.
//!
//! Rails caches each action's list in Redis for a day; the port reads the
//! table, which is what Rails reads once the cache is cleared on every
//! save and destroy.
//!
//! Refused: `watched_words_regular_expressions` (words are admin regexps,
//! valid in Ruby and maybe not in JavaScript), and words with a character
//! Ruby folds to several (`ß` matches `ss` in Onigmo, not here).

use fancy_regex::Regex;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `WatchedWord.actions`, in the enum's order.
pub const ACTIONS: [(&str, i32); 8] = [
    ("block", BLOCK),
    ("censor", CENSOR),
    ("require_approval", REQUIRE_APPROVAL),
    ("flag", FLAG),
    ("link", LINK),
    ("replace", REPLACE),
    ("tag", TAG),
    ("silence", SILENCE),
];
pub const BLOCK: i32 = 1;
pub const CENSOR: i32 = 2;
pub const REQUIRE_APPROVAL: i32 = 3;
pub const FLAG: i32 = 4;
pub const REPLACE: i32 = 5;
pub const TAG: i32 = 6;
pub const SILENCE: i32 = 7;
pub const LINK: i32 = 8;

/// `WatchedWord::MAX_WORDS_PER_ACTION`
pub const MAX_WORDS_PER_ACTION: i64 = 2000;

/// `WordWatcher::REPLACEMENT_LETTER`
pub const REPLACEMENT_LETTER: char = '\u{25A0}';

/// `SPACELESS_SCRIPTS`: Han, Hiragana, Katakana, Hangul, Thai, Lao,
/// Myanmar, Khmer, Tibetan, as the regexp source spells them.
const SPACELESS_SCRIPTS: &str = r"\u4E00-\u9FFF\u3400-\u4DBF\u3040-\u309F\u30A0-\u30FF\uAC00-\uD7AF\u0E00-\u0E7F\u0E80-\u0EFF\u1000-\u109F\u1780-\u17FF\u0F00-\u0FFF";
/// `WORD_CHARS`
const WORD_CHARS: &str = r"[\p{L}\p{M}\p{N}\p{Pc}]";

pub fn action_key(action: i32) -> Option<&'static str> {
    ACTIONS.iter().find(|(_, a)| *a == action).map(|(k, _)| *k)
}

pub fn action_id(key: &str) -> Option<i32> {
    ACTIONS.iter().find(|(k, _)| *k == key).map(|(_, a)| *a)
}

/// `WatchedWord.has_replacement?(action)`
pub fn has_replacement(action: i32) -> bool {
    matches!(action, REPLACE | TAG | LINK)
}

/// A watched word as `words_for_action` plucks it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Word {
    pub word: String,
    pub replacement: Option<String>,
    pub case_sensitive: bool,
    pub html: bool,
}

/// Refuses what isn't ported before any word is read.
pub fn refuse_unported(s: &SiteSettings) -> Result<(), AppError> {
    if s.get("watched_words_regular_expressions")?.truthy() {
        return Err(Unsupported("watched_words_regular_expressions").into());
    }
    Ok(())
}

/// `words_for_action(action)`: the action's words by id, at most
/// MAX_WORDS_PER_ACTION. Refuses words Ruby would case-fold differently.
pub async fn words_for_action(
    conn: &mut PgConnection,
    s: &SiteSettings,
    action: i32,
) -> Result<Vec<Word>, AppError> {
    refuse_unported(s)?;
    let words: Vec<Word> = sqlx::query_as(
        "SELECT word, replacement, case_sensitive, html FROM watched_words \
         WHERE action = $1 ORDER BY id LIMIT $2",
    )
    .bind(action)
    .bind(MAX_WORDS_PER_ACTION)
    .fetch_all(conn)
    .await?;
    for w in &words {
        if !w.case_sensitive && w.word.chars().any(multi_fold) {
            return Err(Unsupported("watched words with full case folding").into());
        }
    }
    Ok(words)
}

/// A character whose case mapping is several characters (`ß`, `ﬁ`,
/// `İ`): Onigmo's full case folding and JavaScript's simple folding part
/// ways on them.
fn multi_fold(c: char) -> bool {
    c.to_lowercase().count() > 1 || c.to_uppercase().count() > 1
}

/// `words_for_action_exist?`
pub async fn exists(conn: &mut PgConnection, action: i32) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM watched_words WHERE action = $1)")
        .bind(action)
        .fetch_one(conn)
        .await
}

/// `word_to_regexp(word, match_word:)` for plain words: the regexp
/// metacharacters escaped with a backslash (not Regexp.escape), `*` as
/// `\S*`, wrapped in the word boundaries when matching a word.
pub fn word_to_regexp(word: &str, match_word: bool) -> String {
    let mut re = String::with_capacity(word.len() + 8);
    for c in word.chars() {
        if ".*+?^${}()|[]\\".contains(c) {
            re.push('\\');
        }
        re.push(c);
    }
    let re = re.replace(r"\*", r"\S*");
    if match_word {
        match_word_regexp(&re)
    } else {
        re
    }
}

/// `match_word_regexp`: a word between non-word characters, or next to a
/// spaceless script.
fn match_word_regexp(re: &str) -> String {
    let s = SPACELESS_SCRIPTS;
    let w = WORD_CHARS;
    format!("(?:(?<!{w})|(?<=[{s}])|(?=[{s}]))({re})(?:(?!{w})|(?=[{s}])|(?<=[{s}]))")
}

/// One of `compiled_regexps_for_action`: the source and whether it is
/// case-sensitive.
#[derive(Debug, Clone)]
pub struct Compiled {
    pub source: String,
    pub case_sensitive: bool,
}

impl Compiled {
    /// The regexp as Ruby runs it (`Regexp::IGNORECASE` unless
    /// case-sensitive).
    pub fn regex(&self) -> Result<Regex, AppError> {
        ruby_regex(&self.source, self.case_sensitive)
    }
}

/// A regexp source compiled as Ruby would run it.
pub fn ruby_regex(source: &str, case_sensitive: bool) -> Result<Regex, AppError> {
    // An inline flag: fancy_regex's builder flag misses the parts its own
    // VM runs (those around lookarounds).
    let source = if case_sensitive {
        source.to_string()
    } else {
        format!("(?i){source}")
    };
    Regex::new(&source)
        .map_err(|_| Unsupported("a watched word regexp the port can't compile").into())
}

/// `compiled_regexps_for_action`: the words grouped by case sensitivity
/// (in the order each group is first seen), each group's word regexps
/// joined with `|` and wrapped once in the word boundaries.
pub fn compile(words: &[Word]) -> Vec<Compiled> {
    let mut groups: Vec<(bool, Vec<String>)> = Vec::new();
    for w in words {
        let re = word_to_regexp(&w.word, false);
        match groups.iter_mut().find(|(cs, _)| *cs == w.case_sensitive) {
            Some((_, list)) => list.push(re),
            None => groups.push((w.case_sensitive, vec![re])),
        }
    }
    groups
        .into_iter()
        .map(|(case_sensitive, list)| Compiled {
            source: match_word_regexp(&list.join("|")),
            case_sensitive,
        })
        .collect()
}

pub async fn compiled_for_action(
    conn: &mut PgConnection,
    s: &SiteSettings,
    action: i32,
) -> Result<Vec<Compiled>, AppError> {
    Ok(compile(&words_for_action(conn, s, action).await?))
}

/// `serialized_regexps_for_action`: `[{source => {case_sensitive:}}]`.
pub fn serialize(compiled: &[Compiled]) -> Value {
    Value::Array(
        compiled
            .iter()
            .map(|c| {
                let mut m = Map::new();
                m.insert(
                    c.source.clone(),
                    json!({ "case_sensitive": c.case_sensitive }),
                );
                Value::Object(m)
            })
            .collect(),
    )
}

/// `regexps_for_action`: each word's regexp (with the boundaries) to its
/// attributes, `{word:, replacement:, case_sensitive:}` (replacement only
/// when there is one, `html: true` only when set); None without words.
pub fn regexps(words: &[Word]) -> Option<Map<String, Value>> {
    if words.is_empty() {
        return None;
    }
    let mut out = Map::new();
    for w in words {
        let mut attrs = Map::new();
        attrs.insert("word".into(), json!(w.word));
        if let Some(r) = &w.replacement {
            attrs.insert("replacement".into(), json!(r));
        }
        attrs.insert("case_sensitive".into(), json!(w.case_sensitive));
        if w.html {
            attrs.insert("html".into(), json!(true));
        }
        out.insert(word_to_regexp(&w.word, true), Value::Object(attrs));
    }
    Some(out)
}

/// Ruby's `String#lstrip` whitespace: null, tab, newline, vertical tab,
/// form feed, carriage return, space.
fn ruby_space(c: char) -> bool {
    matches!(c, '\0' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ')
}

/// `censor_text_with_regexp`: each match (its leading whitespace kept)
/// becomes ■ per character.
fn censor_with(text: &str, re: &Regex) -> Result<String, AppError> {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for m in re.find_iter(text) {
        let m = m.map_err(|_| Unsupported("watched word regexp backtracking limit"))?;
        out.push_str(&text[last..m.start()]);
        let matched = m.as_str();
        let rest = matched.trim_start_matches(ruby_space);
        out.push_str(&matched[..matched.len() - rest.len()]);
        out.extend(std::iter::repeat_n(
            REPLACEMENT_LETTER,
            rest.chars().count(),
        ));
        last = m.end();
    }
    out.push_str(&text[last..]);
    Ok(out)
}

/// `WordWatcher.censor_text`
pub fn censor_text(text: &str, censor: &[Compiled]) -> Result<String, AppError> {
    let mut text = text.to_string();
    if text.trim().is_empty() {
        return Ok(text);
    }
    for c in censor {
        text = censor_with(&text, &c.regex()?)?;
    }
    Ok(text)
}

/// `WordWatcher.censor(html)`: every text node of the fragment censored,
/// then the fragment serialized again; untouched without censor words.
pub fn censor_html(html: &str, censor: &[Compiled]) -> Result<String, AppError> {
    use crate::pretty_text::cleanup::{fragment_root, parse, to_html};
    use markup5ever_rcdom::{Handle, NodeData};
    if censor.is_empty() {
        return Ok(html.to_string());
    }
    let regexps = censor
        .iter()
        .map(Compiled::regex)
        .collect::<Result<Vec<_>, _>>()?;
    fn walk(node: &Handle, regexps: &[Regex]) -> Result<(), AppError> {
        if let NodeData::Text { contents } = &node.data {
            let mut text = contents.borrow().to_string();
            for re in regexps {
                text = censor_with(&text, re)?;
            }
            *contents.borrow_mut() = text.into();
        }
        for child in node.children.borrow().iter() {
            walk(child, regexps)?;
        }
        Ok(())
    }
    let dom = parse(html);
    walk(&fragment_root(&dom), &regexps)?;
    Ok(to_html(&dom))
}

/// `replace_text` / `replace_link` (`replace` over `regexps_for_action`):
/// each word's matches become its replacement, after the match's first
/// character when that is a non-word one (`\W`, ASCII).
pub fn replace_text(text: &str, words: &[Word]) -> Result<String, AppError> {
    let mut text = text.to_string();
    if text.trim().is_empty() {
        return Ok(text);
    }
    for w in words {
        let re = ruby_regex(&word_to_regexp(&w.word, true), w.case_sensitive)?;
        let replacement = w.replacement.as_deref().unwrap_or("");
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for m in re.find_iter(&text) {
            let m = m.map_err(|_| Unsupported("watched word regexp backtracking limit"))?;
            out.push_str(&text[last..m.start()]);
            if let Some(first) = m.as_str().chars().next()
                && !(first.is_ascii_alphanumeric() || first == '_')
            {
                out.push(first);
            }
            out.push_str(replacement);
            last = m.end();
        }
        out.push_str(&text[last..]);
        text = out;
    }
    Ok(text)
}

/// `WordWatcher.apply_to_text`: censored, then the replace words, then the
/// link words.
pub async fn apply_to_text(
    conn: &mut PgConnection,
    s: &SiteSettings,
    text: &str,
) -> Result<String, AppError> {
    let censor = compiled_for_action(&mut *conn, s, CENSOR).await?;
    let text = censor_text(text, &censor)?;
    let text = replace_text(&text, &words_for_action(&mut *conn, s, REPLACE).await?)?;
    replace_text(&text, &words_for_action(&mut *conn, s, LINK).await?)
}

/// `word_matches_for_action?(action, all_matches:)` in plain mode: the
/// first group of every match of each regexp, compacted, unique and
/// sorted (by bytes, as Ruby sorts strings).
pub fn matches(text: &str, compiled: &[Compiled], all: bool) -> Result<Vec<String>, AppError> {
    let mut found = Vec::new();
    for c in compiled {
        let re = c.regex()?;
        for caps in re.captures_iter(text) {
            let caps = caps.map_err(|_| Unsupported("watched word regexp backtracking limit"))?;
            if let Some(g) = caps.get(1) {
                found.push(g.as_str().to_string());
            }
            if !all {
                return Ok(found);
            }
        }
    }
    found.sort();
    found.dedup();
    Ok(found)
}

/// Whether a tag word matches `text` (TopicCreator's
/// `word_watcher.word_matches?` over "#{title} #{raw}"). Tagging the topic
/// isn't ported, so a match is refused by the caller.
pub async fn tag_word_matches(
    conn: &mut PgConnection,
    s: &SiteSettings,
    text: &str,
) -> Result<bool, AppError> {
    for w in words_for_action(conn, s, TAG).await? {
        let re = ruby_regex(&word_to_regexp(&w.word, true), w.case_sensitive)?;
        if re
            .is_match(text)
            .map_err(|_| Unsupported("watched word regexp backtracking limit"))?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `WatchedWordsValidator`'s message for blocked words, or None.
pub fn blocked_message(i18n: &crate::i18n::I18n, matches: &[String]) -> Option<String> {
    match matches {
        [] => None,
        [word] => i18n.t_with(
            "contains_blocked_word",
            &[("word", &crate::category_badge::html_escape(word))],
        ),
        words => i18n.t_with(
            "contains_blocked_words",
            &[(
                "words",
                &crate::category_badge::html_escape(&words.join(", ")),
            )],
        ),
    }
}

/// `WordWatcher.new(text).should_block?` as the validator reports it:
/// the message for the blocked words in `text`, or None.
pub async fn block_error(
    conn: &mut PgConnection,
    s: &SiteSettings,
    i18n: &crate::i18n::I18n,
    text: &str,
) -> Result<Option<String>, AppError> {
    let compiled = compiled_for_action(conn, s, BLOCK).await?;
    if compiled.is_empty() {
        return Ok(None);
    }
    Ok(blocked_message(i18n, &matches(text, &compiled, true)?))
}

/// `CensoredWordsValidator`: the censored words in `value` (each regexp's
/// scan, stripped, unique, then downcased and unique again), joined, or
/// None.
pub fn censored_words(value: &str, censor: &[Compiled]) -> Result<Option<String>, AppError> {
    let mut words: Vec<String> = Vec::new();
    for c in censor {
        let re = c.regex()?;
        for caps in re.captures_iter(value) {
            let caps = caps.map_err(|_| Unsupported("watched word regexp backtracking limit"))?;
            if let Some(g) = caps.get(1) {
                let w = g.as_str().trim_matches(ruby_space).to_string();
                if !w.trim().is_empty() && !words.contains(&w) {
                    words.push(w);
                }
            }
        }
    }
    let mut lower: Vec<String> = Vec::new();
    for w in words {
        let w = w.to_lowercase();
        if !lower.contains(&w) {
            lower.push(w);
        }
    }
    Ok((!lower.is_empty()).then(|| lower.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(w: &str, cs: bool) -> Word {
        Word {
            word: w.into(),
            replacement: None,
            case_sensitive: cs,
            html: false,
        }
    }

    #[test]
    fn regexp_like_rails() {
        assert_eq!(word_to_regexp("a.b", false), r"a\.b");
        assert_eq!(word_to_regexp("ba*d", false), r"ba\S*d");
        assert!(
            word_to_regexp("ba*d", true)
                .starts_with(r"(?:(?<![\p{L}\p{M}\p{N}\p{Pc}])|(?<=[\u4E00-\u9FFF")
        );
    }

    #[test]
    fn censor_like_rails() {
        let c = compile(&[word("bad", false), word("shit*", false)]);
        assert_eq!(c.len(), 1);
        assert_eq!(
            censor_text("bad 😀bad  bad", &c).unwrap(),
            "\u{25A0}\u{25A0}\u{25A0} 😀\u{25A0}\u{25A0}\u{25A0}  \u{25A0}\u{25A0}\u{25A0}"
        );
        assert_eq!(
            censor_text("badly shitty notbad", &c).unwrap(),
            "badly \u{25A0}\u{25A0}\u{25A0}\u{25A0}\u{25A0}\u{25A0} notbad"
        );
    }

    #[test]
    fn matches_sorted_unique() {
        let c = compile(&[word("zed", false), word("abc", false)]);
        assert_eq!(
            matches("ZED abc zed", &c, true).unwrap(),
            vec!["ZED".to_string(), "abc".into(), "zed".into()]
        );
    }

    #[test]
    fn cjk_boundaries() {
        let c = compile(&[word("猫", false)]);
        assert_eq!(
            matches("我的猫很好", &c, true).unwrap(),
            vec!["猫".to_string()]
        );
    }
}
