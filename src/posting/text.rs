//! The text helpers posting validates and stores with: lib/text_cleaner.rb,
//! lib/text_sentinel.rb, StrippedLengthValidator's sanitizing, Slug.for
//! (ascii) and the post word count.

use std::sync::LazyLock;

use regex::Regex;

use crate::Unsupported;

/// `TextCleaner.normalize_whitespaces`: exotic spaces become plain ones.
pub fn normalize_whitespaces(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{a0}'
            | '\u{1680}'
            | '\u{180e}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}' => ' ',
            c => c,
        })
        .collect()
}

/// The title options of `TextCleaner.clean` that vary by setting.
pub struct TitleOptions {
    pub prettify: bool,
    pub allow_uppercase_posts: bool,
    pub remove_extraneous_space: bool,
}

static PERIODS_AT_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([^.])\.+(\s*)\z").unwrap());
static EXTRANEOUS_SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+([!?]\s*)\z").unwrap());
static BANGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"!+").unwrap());
static QUESTIONS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\?+").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r" +").unwrap());

/// `TextCleaner.clean_title`
pub fn clean_title(title: &str, o: &TitleOptions) -> String {
    let mut text = title.to_string();
    if o.prettify {
        text = BANGS.replace_all(&text, "!").into_owned();
        text = QUESTIONS.replace_all(&text, "?").into_owned();
        if !o.allow_uppercase_posts && text == text.to_uppercase() {
            text = text.to_lowercase();
        }
        // `text.split(" ", 2)`: awk-style, leading whitespace skipped.
        let trimmed = text.trim_start_matches(is_ruby_space);
        if let Some(first) = trimmed
            .split(is_ruby_space)
            .next()
            .filter(|f| !f.is_empty())
            && first == first.to_lowercase()
        {
            let rest = trimmed[first.len()..].trim_start_matches(is_ruby_space);
            let mut chars = first.chars();
            let capitalized: String = chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default();
            text = if rest.is_empty() {
                capitalized
            } else {
                format!("{capitalized} {rest}")
            };
        }
        text = PERIODS_AT_END.replace(&text, "$1$2").into_owned();
        if o.remove_extraneous_space {
            text = EXTRANEOUS_SPACE.replace(&text, "$1").into_owned();
        }
    }
    text = SPACES.replace_all(&text, " ").into_owned();
    text = normalize_whitespaces(&text);
    text = text.trim_matches(is_ruby_strip).to_string();
    text.replace('\u{200b}', "")
}

/// What `String#split(" ")` splits on.
fn is_ruby_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r')
}

/// What `String#strip` removes.
fn is_ruby_strip(c: char) -> bool {
    is_ruby_space(c) || c == '\0'
}

/// `TextSentinel`
pub struct Sentinel<'a> {
    pub text: &'a str,
    pub min_entropy: Option<i64>,
    pub max_word_length: Option<i64>,
    pub allow_uppercase_posts: bool,
    /// The site's locale skips the word-length check (ja, ko, zh).
    pub skip_word_length: bool,
}

impl Sentinel<'_> {
    pub fn entropy(&self) -> i64 {
        let mut bytes: Vec<u8> = self.text.trim_matches(is_ruby_strip).bytes().collect();
        bytes.sort_unstable();
        bytes.dedup();
        bytes.len() as i64
    }

    pub fn valid(&self) -> bool {
        !self.text.trim().is_empty()
            && self.seems_meaningful()
            && self.seems_pronounceable()
            && self.seems_unpretentious()
            && self.seems_quiet()
    }

    pub fn seems_meaningful(&self) -> bool {
        self.min_entropy.is_none_or(|m| self.entropy() >= m)
    }

    pub fn seems_pronounceable(&self) -> bool {
        self.text.chars().any(char::is_alphanumeric)
    }

    pub fn seems_unpretentious(&self) -> bool {
        let Some(max) = self.max_word_length.filter(|_| !self.skip_word_length) else {
            return true;
        };
        let mut run = 0;
        for c in self.text.chars() {
            if c.is_alphanumeric() {
                run += 1;
                if run > max {
                    return false;
                }
            } else {
                run = 0;
            }
        }
        true
    }

    pub fn seems_quiet(&self) -> bool {
        self.allow_uppercase_posts
            || self.text.chars().any(|c| {
                c.is_lowercase() || (c.is_alphabetic() && !c.is_uppercase() && !c.is_lowercase())
            })
            || !self.text.chars().any(char::is_alphabetic)
    }
}

static HTML_COMMENTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!--(.*?)-->").unwrap());
static EMOJI_CODES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r":\w+(:\w+)?:").unwrap());
static DOTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.{2,}").unwrap());
static COMMAS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r",{2,}").unwrap());
static UPLOADS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"!\[.*\]\(.+\)").unwrap());

/// `StrippedLengthValidator.get_sanitized_value`
pub fn sanitized_length(value: &str, strip_uploads: bool) -> usize {
    let mut v = HTML_COMMENTS.replace_all(value, "").into_owned();
    v = EMOJI_CODES.replace_all(&v, "X").into_owned();
    v = DOTS.replace_all(&v, "…").into_owned();
    v = COMMAS.replace_all(&v, ",").into_owned();
    if strip_uploads {
        v = UPLOADS.replace_all(&v, "").into_owned();
    }
    v.trim_matches(is_ruby_strip).chars().count()
}

/// `raw.scan(/[[:word:]]+/).size`
pub fn word_count(raw: &str) -> i32 {
    let mut count = 0;
    let mut in_word = false;
    for c in raw.chars() {
        let word = c.is_alphanumeric() || c == '_';
        if word && !in_word {
            count += 1;
        }
        in_word = word;
    }
    count
}

/// `Slug.for(title)` with slug_generation_method `ascii`, for titles
/// `String#parameterize` keeps as they are (ASCII without emoji codes).
pub fn slug_for(title: &str) -> Result<String, Unsupported> {
    if !title.is_ascii() {
        return Err(Unsupported(
            "slugs for non-ASCII titles (I18n transliteration)",
        ));
    }
    if crate::emoji::has_emoji_code(title) {
        return Err(Unsupported("slugs for titles with emoji codes"));
    }
    // tr("'", "").parameterize
    let mut s = String::new();
    for c in title.chars().filter(|&c| c != '\'') {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('-') {
            s.push('-');
        }
    }
    // parameterize squeezes separators and trims them; prettify_slug then
    // turns underscores into dashes, truncates, squeezes and trims again.
    let mut slug = String::new();
    for c in s.chars().map(|c| if c == '_' { '-' } else { c }).take(255) {
        if c == '-' && slug.ends_with('-') {
            continue;
        }
        slug.push(c);
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() || slug.chars().all(|c| c.is_ascii_digit()) {
        return Ok("topic".to_string());
    }
    Ok(slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> TitleOptions {
        TitleOptions {
            prettify: true,
            allow_uppercase_posts: false,
            remove_extraneous_space: false,
        }
    }

    #[test]
    fn titles_are_cleaned_like_rails() {
        assert_eq!(
            clean_title("A new topic from user1 for the posting slice", &opts()),
            "A new topic from user1 for the posting slice"
        );
        assert_eq!(clean_title("hello   world...", &opts()), "Hello world");
        assert_eq!(clean_title("WHAT IS THIS!!!", &opts()), "What is this!");
    }

    #[test]
    fn slugs() {
        assert_eq!(
            slug_for("A new topic from user1 for the posting slice").unwrap(),
            "a-new-topic-from-user1-for-the-posting-slice"
        );
        assert_eq!(slug_for("Don't stop: me_now!").unwrap(), "dont-stop-me-now");
        assert_eq!(slug_for("12345").unwrap(), "topic");
    }

    #[test]
    fn words() {
        assert_eq!(
            word_count(
                "A reply from user1 to the replies and posters topic, written for the posting slice."
            ),
            15
        );
    }

    #[test]
    fn stripped_lengths() {
        assert_eq!(sanitized_length("  short  ", false), 5);
        assert_eq!(sanitized_length("hi :smile: ...", false), 6);
    }
}
