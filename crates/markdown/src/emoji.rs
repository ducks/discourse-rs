//! Port of Emoji.gsub_emoji_to_unicode (app/models/emoji.rb), backed by the
//! vendored discourse-emojis data (emojis.json, aliases.json,
//! tonable_emojis.json).

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Deserialize;

const EMOJIS_JSON: &str = include_str!("../../../vendor/discourse-emojis/dist/emojis.json");
const ALIASES_JSON: &str = include_str!("../../../vendor/discourse-emojis/dist/aliases.json");
/// `Emoji.tonable_emojis`, as JSON.
pub const TONABLE_JSON: &str =
    include_str!("../../../vendor/discourse-emojis/dist/tonable_emojis.json");

/// Fitzpatrick modifiers for `:name:t2` .. `:name:t6`.
const TONES: [u32; 5] = [0x1f3fb, 0x1f3fc, 0x1f3fd, 0x1f3fe, 0x1f3ff];

#[derive(Deserialize)]
struct EmojiEntry {
    name: String,
    code: String,
}

/// `Emoji.unicode_replacements`: name -> unicode string.
static UNICODE: LazyLock<HashMap<String, String>> = LazyLock::new(build_unicode_map);

fn code_to_codepoints(code: &str) -> Vec<u32> {
    code.split('-')
        .filter_map(|part| u32::from_str_radix(part, 16).ok())
        .collect()
}

fn codepoints_to_string(points: &[u32]) -> String {
    points.iter().filter_map(|&p| char::from_u32(p)).collect()
}

fn build_unicode_map() -> HashMap<String, String> {
    let emojis: Vec<EmojiEntry> = serde_json::from_str(EMOJIS_JSON).expect("vendored emojis.json");
    let aliases: HashMap<String, Vec<String>> =
        serde_json::from_str(ALIASES_JSON).expect("vendored aliases.json");
    let tonable: Vec<String> =
        serde_json::from_str(TONABLE_JSON).expect("vendored tonable_emojis.json");

    let mut map = HashMap::with_capacity(emojis.len() * 2);
    let mut codes: HashMap<&str, Vec<u32>> = HashMap::with_capacity(emojis.len());
    for e in &emojis {
        if e.name == "tm" {
            continue;
        }
        let points = code_to_codepoints(&e.code);
        map.insert(e.name.clone(), codepoints_to_string(&points));
        codes.insert(&e.name, points);
    }

    // Skin tones: drop a variation selector at index 1, insert the tone there.
    for name in &tonable {
        let Some(points) = codes.get(name.as_str()) else {
            continue;
        };
        for (i, tone) in TONES.iter().enumerate() {
            let mut toned = points.clone();
            if toned.get(1) == Some(&0xFE0F) {
                toned.remove(1);
            }
            let at = 1.min(toned.len());
            toned.insert(at, *tone);
            map.insert(format!("{name}:t{}", i + 2), codepoints_to_string(&toned));
        }
    }

    for (canonical, names) in &aliases {
        if let Some(unicode) = map.get(canonical).cloned() {
            for alias in names {
                map.insert(alias.clone(), unicode.clone());
            }
        }
    }
    map
}

/// `Emoji.lookup_unicode(name)`; the deny list isn't applied yet.
pub fn lookup_unicode(name: &str) -> Option<&'static str> {
    UNICODE.get(name).map(String::as_str)
}

/// `Emoji.gsub_emoji_to_unicode`: replaces `:name:` and `:name:t2` codes
/// (`/:([\w\-+]+(?::t\d)?):/`, ASCII \w) that name a known emoji.
pub fn gsub_emoji_to_unicode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find(':') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match match_code(after) {
            Some((name, len)) => {
                match lookup_unicode(name) {
                    Some(unicode) => out.push_str(unicode),
                    None => {
                        out.push(':');
                        out.push_str(name);
                        out.push(':');
                    }
                }
                rest = &after[len..];
            }
            None => {
                out.push(':');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Matches `name(:tN)?:` at the start of `s`; returns the name and the
/// length consumed including the closing colon.
fn match_code(s: &str) -> Option<(&str, usize)> {
    let is_name_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '+';
    let name_len = s.find(|c: char| !is_name_char(c)).unwrap_or(s.len());
    if name_len == 0 {
        return None;
    }
    let mut end = name_len;
    let tail = &s[end..];
    if let Some(t) = tail.strip_prefix(":t")
        && t.chars().next().is_some_and(|c| c.is_ascii_digit())
        && t[1..].starts_with(':')
    {
        end += 3;
    }
    if !s[end..].starts_with(':') {
        return None;
    }
    Some((&s[..end], end + 1))
}

/// `title.match?(/:[\w\-+]+:/)`: whether a title carries an emoji code.
pub fn has_emoji_code(s: &str) -> bool {
    let mut rest = s;
    while let Some(start) = rest.find(':') {
        let after = &rest[start + 1..];
        let len = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '+'))
            .unwrap_or(after.len());
        if len > 0 && after[len..].starts_with(':') {
            return true;
        }
        rest = after;
    }
    false
}

const TRANSLATIONS_JSON: &str =
    include_str!("../../../vendor/discourse-emojis/dist/translations.json");
const DISCOURSE_REF: &str = include_str!("../../../vendor/discourse/DISCOURSE_REF");

/// What cooking needs to know about emoji, built once.
pub struct EmojiData {
    /// `emojis`: the canonical names.
    pub names: std::collections::HashSet<String>,
    /// `aliasMap`: alias -> canonical name.
    pub aliases: HashMap<String, String>,
    /// `Emoji.unicode_replacements`: the emoji itself -> its name
    /// (`name:tN` for a skin tone).
    pub unicode: HashMap<String, String>,
    /// The longest key of `unicode`, in chars.
    pub unicode_max_chars: usize,
    /// `translations`: emoticon -> name.
    pub translations: HashMap<String, String>,
}

pub static DATA: LazyLock<EmojiData> = LazyLock::new(|| {
    let emojis: Vec<EmojiEntry> = serde_json::from_str(EMOJIS_JSON).expect("vendored emojis.json");
    let alias_lists: HashMap<String, Vec<String>> =
        serde_json::from_str(ALIASES_JSON).expect("vendored aliases.json");
    let tonable: std::collections::HashSet<String> =
        serde_json::from_str::<Vec<String>>(TONABLE_JSON)
            .expect("vendored tonable_emojis.json")
            .into_iter()
            .collect();
    let translations: HashMap<String, String> =
        serde_json::from_str(TRANSLATIONS_JSON).expect("vendored translations.json");

    let mut aliases = HashMap::new();
    for (name, list) in alias_lists {
        for alias in list {
            aliases.insert(alias, name.clone());
        }
    }

    // Emoji.unicode_replacements
    let mut unicode = HashMap::new();
    for e in &emojis {
        // Kept as symbols.
        if matches!(
            e.name.as_str(),
            "registered" | "copyright" | "trade_mark" | "left_right_arrow"
        ) {
            continue;
        }
        let points = code_to_codepoints(&e.code);
        if points.is_empty() {
            continue;
        }
        unicode.insert(codepoints_to_string(&points), e.name.clone());
        if tonable.contains(&e.name) {
            for (i, tone) in TONES.iter().enumerate() {
                let mut toned = points.clone();
                if toned.get(1) == Some(&0xFE0F) {
                    toned.remove(1);
                }
                toned.insert(1.min(toned.len()), *tone);
                unicode.insert(
                    codepoints_to_string(&toned),
                    format!("{}:t{}", e.name, i + 2),
                );
            }
        }
    }
    for (symbol, name) in [
        ("\u{2639}", "frowning"),
        ("\u{263B}", "slight_smile"),
        ("\u{2661}", "heart"),
        ("\u{2665}", "heart"),
    ] {
        unicode.insert(symbol.to_string(), name.to_string());
    }
    let unicode_max_chars = unicode.keys().map(|k| k.chars().count()).max().unwrap_or(0);

    EmojiData {
        names: emojis.into_iter().map(|e| e.name).collect(),
        aliases,
        unicode,
        unicode_max_chars,
        translations,
    }
});

impl EmojiData {
    /// `emojis.has(name) || aliasMap.has(name)`
    pub fn exists(&self, name: &str) -> bool {
        self.names.contains(name) || self.aliases.contains_key(name)
    }
}

/// `IMAGE_VERSION` of pretty-text/emoji/version.js: the `?v=` on emoji
/// image URLs, recorded by scripts/vendor-discourse.
pub fn image_version() -> &'static str {
    DISCOURSE_REF
        .lines()
        .find_map(|l| l.strip_prefix("emoji_image_version="))
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_known_codes_and_aliases() {
        assert_eq!(
            gsub_emoji_to_unicode("Welcome to Discourse! :wave:"),
            "Welcome to Discourse! 👋"
        );
        assert_eq!(gsub_emoji_to_unicode(":waving_hand:"), "👋");
        assert_eq!(gsub_emoji_to_unicode(":waving_hand:t2:"), "👋\u{1f3fb}");
        // Tones exist for canonical names only, exactly as in Emoji.lookup_unicode.
        assert_eq!(gsub_emoji_to_unicode(":wave:t2:"), ":wave:t2:");
        assert_eq!(gsub_emoji_to_unicode("time 10:30: ok"), "time 10:30: ok");
        assert_eq!(
            gsub_emoji_to_unicode(":not_an_emoji_xyz:"),
            ":not_an_emoji_xyz:"
        );
        assert_eq!(gsub_emoji_to_unicode("no emoji"), "no emoji");
    }

    #[test]
    fn detects_emoji_codes() {
        assert!(has_emoji_code("Hi :wave:"));
        assert!(has_emoji_code("::x:"));
        assert!(!has_emoji_code("10:30 meeting"));
        assert!(!has_emoji_code("plain"));
    }
}
