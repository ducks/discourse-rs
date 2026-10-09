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

/// Settings `PrettyText.escape_emoji` passes to `performEmojiEscape`.
#[derive(Clone, Copy)]
pub struct EmojiEscape {
    /// `enable_emoji && enable_emoji_shortcuts`
    pub shortcuts: bool,
    /// `enable_inline_emoji_translation`
    pub inline: bool,
}

impl EmojiEscape {
    pub fn from_settings(
        s: &crate::site_settings::SiteSettings,
    ) -> Result<Self, crate::site_settings::SettingError> {
        Ok(Self {
            shortcuts: s.get("enable_emoji")?.truthy() && s.get("enable_emoji_shortcuts")?.truthy(),
            inline: s.get("enable_inline_emoji_translation")?.truthy(),
        })
    }
}

/// `Topic.max_fancy_title_length`
const MAX_FANCY_TITLE_LENGTH: usize = 400;

/// `Topic.fancy_title(title)`: HTML-escaped, prettified, emoji escaped,
/// or just escaped when that runs past 400 characters.
pub fn fancy_title(title: &str, emoji: EmojiEscape) -> Result<String, Unsupported> {
    let escaped = crate::category_badge::html_escape(title);
    let fancy = escape_emoji(&crate::html_prettify::render(&escaped), emoji)?;
    Ok(if fancy.chars().count() > MAX_FANCY_TITLE_LENGTH {
        escaped
    } else {
        fancy
    })
}

/// Codepoints `emojiReplacementRegex` can start a match on, widened to
/// whole blocks.
fn is_emoji_char(c: char) -> bool {
    matches!(
        c as u32,
        0x200D
            | 0x203C
            | 0x2049
            | 0x20E3
            | 0x2139
            | 0x2194..=0x2BFF
            | 0x3030
            | 0x303D
            | 0x3297
            | 0x3299
            | 0xFE0F
            | 0x1F000..=0x1FAFF
            | 0xE0020..=0xE007F
    )
}

/// `:`-prefixed keys of pretty-text's emoji `translations`; the others
/// can't match `textEmojiRegex`.
const COLON_TRANSLATIONS: [(&str, &str); 19] = [
    (":)", "slight_smile"),
    (":-)", "slight_smile"),
    (":(", "frowning"),
    (":-(", "frowning"),
    (":'(", "cry"),
    (":'-(", "cry"),
    (":-'(", "cry"),
    (":p", "stuck_out_tongue"),
    (":P", "stuck_out_tongue"),
    (":-P", "stuck_out_tongue"),
    (":O", "open_mouth"),
    (":-O", "open_mouth"),
    (":D", "smiley"),
    (":-D", "smiley"),
    (":|", "expressionless"),
    (":-|", "expressionless"),
    (":/", "confused"),
    (":$", "blush"),
    (":-$", "blush"),
];

/// JS `\B` (ASCII word characters) as lookarounds.
const JS_NOT_BOUNDARY: &str =
    "(?:(?<=[A-Za-z0-9_])(?=[A-Za-z0-9_])|(?<![A-Za-z0-9_])(?![A-Za-z0-9_]))";

static TEXT_EMOJI: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    fancy_regex::Regex::new(&format!(
        r"{JS_NOT_BOUNDARY}:[^\s:]+(?::t[0-9])?:?{JS_NOT_BOUNDARY}"
    ))
    .unwrap()
});
static TEXT_EMOJI_INLINE: LazyLock<fancy_regex::Regex> =
    LazyLock::new(|| fancy_regex::Regex::new(r":[^\s:]+(?::t[0-9])?:?").unwrap());

/// `PrettyText.escape_emoji` (pretty-text's `performEmojiEscape`): text
/// shortcuts like `:)` become `:slight_smile:`. Unicode emoji, which it
/// turns into codes too, aren't ported.
pub fn escape_emoji(s: &str, opts: EmojiEscape) -> Result<String, Unsupported> {
    if s.chars().any(is_emoji_char) {
        return Err(Unsupported("unicode emoji in titles (performEmojiEscape)"));
    }
    if !opts.shortcuts {
        return Ok(s.to_string());
    }
    let re = if opts.inline {
        &*TEXT_EMOJI_INLINE
    } else {
        &*TEXT_EMOJI
    };
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    for m in re.find_iter(s) {
        let m = m.map_err(|_| Unsupported("emoji shortcut regex backtracking"))?;
        let name = COLON_TRANSLATIONS
            .iter()
            .find(|(k, _)| *k == m.as_str())
            .map(|(_, v)| *v);
        let Some(name) = name else { continue };
        let before = &s[..m.start()];
        let replaceable = opts.inline
            || before
                .chars()
                .last()
                .is_none_or(|c| c.is_whitespace() || ">.,/#!$%^&*;:{}=-_`~()".contains(c));
        if replaceable {
            out.push_str(&s[last..m.start()]);
            out.push(':');
            out.push_str(name);
            out.push(':');
            last = m.end();
        }
    }
    out.push_str(&s[last..]);
    Ok(out)
}

/// `Slug.for(title)` with slug_generation_method `ascii`, for titles
/// without emoji codes, on English sites (other locales have their own
/// transliteration rules).
pub fn slug_for(title: &str, locale: &str) -> Result<String, Unsupported> {
    if !title.is_ascii() && locale != "en" {
        return Err(Unsupported(
            "slugs for non-ASCII titles in locales other than English (I18n transliteration)",
        ));
    }
    if crate::emoji::has_emoji_code(title) {
        return Err(Unsupported("slugs for titles with emoji codes"));
    }
    // tr("'", "").parameterize: transliterated, then everything but
    // letters, digits, dashes and underscores a separator.
    let title = transliterate(title);
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

/// `ActiveSupport::Inflector.transliterate` on an English site: NFC, then
/// each non-ASCII character by I18n's default approximations with
/// Discourse's `transliterate.en.yml` rule over them, else `?`.
pub fn transliterate(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfc()
        .flat_map(|c| -> Vec<char> {
            if c.is_ascii() {
                return vec![c];
            }
            match c {
                // config/locales/transliterate.en.yml
                'ț' | 'Ț' => return vec!['t'],
                'ș' | 'Ș' => return vec!['s'],
                _ => {}
            }
            match APPROXIMATIONS.iter().find(|(from, _)| *from == c) {
                Some((_, to)) => to.chars().collect(),
                None => vec!['?'],
            }
        })
        .collect()
}

/// I18n::Backend::Transliterator::HashTransliterator::DEFAULT_APPROXIMATIONS
const APPROXIMATIONS: &[(char, &str)] = &[
    ('À', "A"),
    ('Á', "A"),
    ('Â', "A"),
    ('Ã', "A"),
    ('Ä', "A"),
    ('Å', "A"),
    ('Æ', "AE"),
    ('Ç', "C"),
    ('È', "E"),
    ('É', "E"),
    ('Ê', "E"),
    ('Ë', "E"),
    ('Ì', "I"),
    ('Í', "I"),
    ('Î', "I"),
    ('Ï', "I"),
    ('Ð', "D"),
    ('Ñ', "N"),
    ('Ò', "O"),
    ('Ó', "O"),
    ('Ô', "O"),
    ('Õ', "O"),
    ('Ö', "O"),
    ('×', "x"),
    ('Ø', "O"),
    ('Ù', "U"),
    ('Ú', "U"),
    ('Û', "U"),
    ('Ü', "U"),
    ('Ý', "Y"),
    ('Þ', "Th"),
    ('ß', "ss"),
    ('ẞ', "SS"),
    ('à', "a"),
    ('á', "a"),
    ('â', "a"),
    ('ã', "a"),
    ('ä', "a"),
    ('å', "a"),
    ('æ', "ae"),
    ('ç', "c"),
    ('è', "e"),
    ('é', "e"),
    ('ê', "e"),
    ('ë', "e"),
    ('ì', "i"),
    ('í', "i"),
    ('î', "i"),
    ('ï', "i"),
    ('ð', "d"),
    ('ñ', "n"),
    ('ò', "o"),
    ('ó', "o"),
    ('ô', "o"),
    ('õ', "o"),
    ('ö', "o"),
    ('ø', "o"),
    ('ù', "u"),
    ('ú', "u"),
    ('û', "u"),
    ('ü', "u"),
    ('ý', "y"),
    ('þ', "th"),
    ('ÿ', "y"),
    ('Ā', "A"),
    ('ā', "a"),
    ('Ă', "A"),
    ('ă', "a"),
    ('Ą', "A"),
    ('ą', "a"),
    ('Ć', "C"),
    ('ć', "c"),
    ('Ĉ', "C"),
    ('ĉ', "c"),
    ('Ċ', "C"),
    ('ċ', "c"),
    ('Č', "C"),
    ('č', "c"),
    ('Ď', "D"),
    ('ď', "d"),
    ('Đ', "D"),
    ('đ', "d"),
    ('Ē', "E"),
    ('ē', "e"),
    ('Ĕ', "E"),
    ('ĕ', "e"),
    ('Ė', "E"),
    ('ė', "e"),
    ('Ę', "E"),
    ('ę', "e"),
    ('Ě', "E"),
    ('ě', "e"),
    ('Ĝ', "G"),
    ('ĝ', "g"),
    ('Ğ', "G"),
    ('ğ', "g"),
    ('Ġ', "G"),
    ('ġ', "g"),
    ('Ģ', "G"),
    ('ģ', "g"),
    ('Ĥ', "H"),
    ('ĥ', "h"),
    ('Ħ', "H"),
    ('ħ', "h"),
    ('Ĩ', "I"),
    ('ĩ', "i"),
    ('Ī', "I"),
    ('ī', "i"),
    ('Ĭ', "I"),
    ('ĭ', "i"),
    ('Į', "I"),
    ('į', "i"),
    ('İ', "I"),
    ('ı', "i"),
    ('Ĳ', "IJ"),
    ('ĳ', "ij"),
    ('Ĵ', "J"),
    ('ĵ', "j"),
    ('Ķ', "K"),
    ('ķ', "k"),
    ('ĸ', "k"),
    ('Ĺ', "L"),
    ('ĺ', "l"),
    ('Ļ', "L"),
    ('ļ', "l"),
    ('Ľ', "L"),
    ('ľ', "l"),
    ('Ŀ', "L"),
    ('ŀ', "l"),
    ('Ł', "L"),
    ('ł', "l"),
    ('Ń', "N"),
    ('ń', "n"),
    ('Ņ', "N"),
    ('ņ', "n"),
    ('Ň', "N"),
    ('ň', "n"),
    ('ŉ', "'n"),
    ('Ŋ', "NG"),
    ('ŋ', "ng"),
    ('Ō', "O"),
    ('ō', "o"),
    ('Ŏ', "O"),
    ('ŏ', "o"),
    ('Ő', "O"),
    ('ő', "o"),
    ('Œ', "OE"),
    ('œ', "oe"),
    ('Ŕ', "R"),
    ('ŕ', "r"),
    ('Ŗ', "R"),
    ('ŗ', "r"),
    ('Ř', "R"),
    ('ř', "r"),
    ('Ś', "S"),
    ('ś', "s"),
    ('Ŝ', "S"),
    ('ŝ', "s"),
    ('Ş', "S"),
    ('ş', "s"),
    ('Š', "S"),
    ('š', "s"),
    ('Ţ', "T"),
    ('ţ', "t"),
    ('Ť', "T"),
    ('ť', "t"),
    ('Ŧ', "T"),
    ('ŧ', "t"),
    ('Ũ', "U"),
    ('ũ', "u"),
    ('Ū', "U"),
    ('ū', "u"),
    ('Ŭ', "U"),
    ('ŭ', "u"),
    ('Ů', "U"),
    ('ů', "u"),
    ('Ű', "U"),
    ('ű', "u"),
    ('Ų', "U"),
    ('ų', "u"),
    ('Ŵ', "W"),
    ('ŵ', "w"),
    ('Ŷ', "Y"),
    ('ŷ', "y"),
    ('Ÿ', "Y"),
    ('Ź', "Z"),
    ('ź', "z"),
    ('Ż', "Z"),
    ('ż', "z"),
    ('Ž', "Z"),
    ('ž', "z"),
];

/// `TextCleaner.clean(message, strip_whitespaces:, strip_zero_width_spaces:
/// true)` as chat's contract cleans a message: whitespace normalized,
/// stripped, zero width spaces removed.
pub fn clean_message(message: &str, strip_whitespaces: bool) -> String {
    let mut text = normalize_whitespaces(message);
    if strip_whitespaces {
        text = text.trim_matches(is_ruby_strip).to_string();
    }
    text.replace('\u{200b}', "")
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
            slug_for("A new topic from user1 for the posting slice", "en").unwrap(),
            "a-new-topic-from-user1-for-the-posting-slice"
        );
        assert_eq!(
            slug_for("Don't stop: me_now!", "en").unwrap(),
            "dont-stop-me-now"
        );
        assert_eq!(slug_for("12345", "en").unwrap(), "topic");
        // Values from Rails' Slug.for.
        assert_eq!(
            slug_for(
                "Congratulations, you’ve been granted moderator status!",
                "en"
            )
            .unwrap(),
            "congratulations-you-ve-been-granted-moderator-status"
        );
        assert_eq!(
            slug_for("Crème brûlée à Zürich", "en").unwrap(),
            "creme-brulee-a-zurich"
        );
        assert_eq!(
            slug_for("Ștefan’s café — naïve", "en").unwrap(),
            "stefan-s-cafe-naive"
        );
        assert!(slug_for("Crème brûlée", "fr").is_err());
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
