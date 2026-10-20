//! pretty-text/emoji.js's `performEmojiEscape` and `performEmojiUnescape`:
//! a replace over `emojiReplacementRegex` (unicode emoji), then one over
//! `textEmojiRegex` (`:codes:` and emoticons) on what the first left. Both
//! work on UTF-16 units as JavaScript does, since the second replace checks
//! what precedes a match by slicing the *original* string at the index the
//! match has in the *replaced* one.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use crate::emoji::DATA;

const VARIATION_SELECTOR: u16 = 0xFE0F;

/// What `emojiReplacementRegex` (lib/emoji/regex_generator.rb) matches:
/// every `Emoji.unicode_replacements` key, also with a trailing variation
/// selector, as UTF-16 units.
struct Sequences {
    all: HashSet<Vec<u16>>,
    /// `replacements`: key -> name.
    names: HashMap<Vec<u16>, &'static str>,
    first_units: HashSet<u16>,
    longest: usize,
}

static SEQUENCES: LazyLock<Sequences> = LazyLock::new(|| {
    let mut all = HashSet::new();
    let mut names = HashMap::new();
    for (key, name) in &DATA.unicode {
        let units: Vec<u16> = key.encode_utf16().collect();
        if !key.ends_with('\u{FE0F}') {
            let mut with_selector = units.clone();
            with_selector.push(VARIATION_SELECTOR);
            all.insert(with_selector);
        }
        all.insert(units.clone());
        names.insert(units, name.as_str());
    }
    let first_units = all.iter().map(|s| s[0]).collect();
    let longest = all.iter().map(Vec::len).max().unwrap_or(0);
    Sequences {
        all,
        names,
        first_units,
        longest,
    }
});

/// `emojiReplacementRegex` at `i`: where its match there ends. The
/// regex is `☻|♡|<trie>`, so those two match alone even before a variation
/// selector; the trie's optional branches are greedy, so otherwise the
/// match is the longest sequence there.
fn emoji_at(units: &[u16], i: usize) -> Option<usize> {
    let seq = &*SEQUENCES;
    let first = *units.get(i)?;
    if first == 0x263B || first == 0x2661 {
        return Some(i + 1);
    }
    if !seq.first_units.contains(&first) {
        return None;
    }
    let longest = seq.longest.min(units.len() - i);
    (1..=longest)
        .rev()
        .find(|len| seq.all.contains(&units[i..i + len]))
        .map(|len| i + len)
}

/// `new RegExp(`(?:${emojiReplacementRegex})$`).test(s)`: the regex can
/// start anywhere and backtrack, so any sequence ending the string.
fn ends_with_emoji(units: &[u16]) -> bool {
    let seq = &*SEQUENCES;
    (1..=seq.longest.min(units.len())).any(|len| seq.all.contains(&units[units.len() - len..]))
}

/// JavaScript's `\s`.
fn is_js_space(unit: u16) -> bool {
    matches!(
        unit,
        0x09..=0x0D
            | 0x20
            | 0xA0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
            | 0xFEFF
    )
}

/// JavaScript's `\w`.
fn is_js_word(unit: Option<&u16>) -> bool {
    unit.is_some_and(|&u| u < 0x80 && ((u as u8).is_ascii_alphanumeric() || u == u16::from(b'_')))
}

/// JavaScript's `\B` between `units[at - 1]` and `units[at]`.
fn not_boundary(units: &[u16], at: usize) -> bool {
    let before = at.checked_sub(1).and_then(|i| units.get(i));
    is_js_word(before) == is_js_word(units.get(at))
}

/// `isReplaceableInlineEmoji(string, index, inlineEmoji)`: `string` is the
/// original text, `index` a position in whatever is being replaced.
fn replaceable(original: &[u16], index: usize, inline: bool) -> bool {
    if inline {
        return true;
    }
    let before = &original[..index.min(original.len())];
    match before.last() {
        None => true,
        Some(&last) => {
            is_js_space(last)
                || (last < 0x80 && b">.,/#!$%^&*;:{}=-_`~()".contains(&(last as u8)))
                || ends_with_emoji(before)
        }
    }
}

/// `textEmojiRegex(inlineEmoji)` at `p`: `/:[^\s:]+(?::t\d)?:?/` inline,
/// else `/\B:[^\s:]+(?::t\d)?:?\B/`, its end. Without inline emoji the
/// closing `\B` backtracks: the tone and colon dropped, then the name
/// shortened.
fn text_emoji_at(units: &[u16], p: usize, inline: bool) -> Option<usize> {
    const COLON: u16 = b':' as u16;
    if units.get(p) != Some(&COLON) || (!inline && !not_boundary(units, p)) {
        return None;
    }
    let name_start = p + 1;
    let run = units[name_start..]
        .iter()
        .take_while(|&&u| u != COLON && !is_js_space(u))
        .count();
    if run == 0 {
        return None;
    }
    let name_end = name_start + run;
    let tone = units.get(name_end) == Some(&COLON)
        && units.get(name_end + 1) == Some(&u16::from(b't'))
        && units
            .get(name_end + 2)
            .is_some_and(|&u| (u16::from(b'0')..=u16::from(b'9')).contains(&u));
    let colon_at = |end: usize| units.get(end) == Some(&COLON);
    let mut ends = Vec::with_capacity(4);
    if tone {
        let after_tone = name_end + 3;
        if colon_at(after_tone) {
            ends.push(after_tone + 1);
        }
        ends.push(after_tone);
    }
    if colon_at(name_end) {
        ends.push(name_end + 1);
    }
    ends.push(name_end);
    if inline {
        return ends.first().copied();
    }
    if let Some(end) = ends.into_iter().find(|&e| not_boundary(units, e)) {
        return Some(end);
    }
    (name_start + 1..name_end)
        .rev()
        .find(|&e| not_boundary(units, e))
}

/// `String.prototype.replace` with a global regex and a function.
fn replace_all(
    units: &[u16],
    find: impl Fn(&[u16], usize) -> Option<usize>,
    mut replace: impl FnMut(&[u16], usize) -> Option<Vec<u16>>,
) -> Vec<u16> {
    let mut out = Vec::with_capacity(units.len());
    let mut i = 0;
    while i < units.len() {
        match find(units, i) {
            Some(end) => {
                let m = &units[i..end];
                match replace(m, i) {
                    Some(r) => out.extend(r),
                    None => out.extend_from_slice(m),
                }
                i = end;
            }
            None => {
                out.push(units[i]);
                i += 1;
            }
        }
    }
    out
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// `allTranslations[m]` (emoticons; no plugin adds any here).
fn translation(m: &[u16]) -> Option<&'static str> {
    DATA.translations
        .get(&String::from_utf16(m).ok()?)
        .map(String::as_str)
}

/// `replacements[m] || replacements[m[0]]`: `m[0]` is one UTF-16 unit.
fn replacement(m: &[u16]) -> Option<&'static str> {
    let names = &SEQUENCES.names;
    names
        .get(m)
        .or_else(|| m.first().and_then(|u| names.get(&[*u][..])))
        .copied()
}

/// `performEmojiEscape(string, {emojiShortcuts, inlineEmoji})`: unicode
/// emoji and (with shortcuts) emoticons to `:name:` codes, each only after
/// a space, punctuation or another emoji unless inline emoji is on.
pub fn perform_emoji_escape(s: &str, shortcuts: bool, inline: bool) -> String {
    let original = utf16(s);
    let replace = |m: &[u16], index: usize| -> Option<Vec<u16>> {
        if !replaceable(&original, index, inline) {
            return None;
        }
        if let Some(name) = translation(m) {
            return shortcuts.then(|| utf16(&format!(":{name}:")));
        }
        replacement(m).map(|name| utf16(&format!(":{name}:")))
    };
    let pass = replace_all(&original, emoji_at, replace);
    let pass = replace_all(&pass, |u, p| text_emoji_at(u, p, inline), replace);
    String::from_utf16_lossy(&pass)
}

/// The `performEmojiUnescape` options of a quoted topic's title
/// (discourse-markdown-it's quotes.js): no custom emoji, no deny list,
/// `lazy`.
pub struct UnescapeOptions<'a> {
    pub emoji_set: &'a str,
    /// `getURL(emojiCDNUrl || "/images/emoji")`.
    pub emoji_base_path: &'a str,
    pub shortcuts: bool,
    pub inline: bool,
    pub lazy: bool,
}

/// `buildEmojiUrl(code, opts)` without custom emoji: the image of a known
/// emoji or alias, a tone as a path segment.
fn emoji_url(code: &str, opts: &UnescapeOptions) -> Option<String> {
    let code = code.to_lowercase();
    // `code.match(/([^:]+):?/)`
    let no_tone = code.split(':').find(|s| !s.is_empty())?;
    if opts.emoji_set.is_empty() || !DATA.exists(no_tone) {
        return None;
    }
    Some(format!(
        "{}/{}/{}.png?v={}",
        opts.emoji_base_path,
        opts.emoji_set,
        code.replacen(":t", "/", 1),
        crate::emoji::image_version()
    ))
}

/// `performEmojiUnescape(string, opts)`: unicode emoji, `:codes:` and
/// (with shortcuts) emoticons to emoji images.
pub fn perform_emoji_unescape(s: &str, opts: &UnescapeOptions) -> String {
    let original = utf16(s);
    let replace = |m: &[u16], index: usize| -> Option<Vec<u16>> {
        let emoticon = opts.shortcuts.then(|| translation(m)).flatten();
        let unicode = replacement(m);
        let value = match (emoticon, unicode) {
            (Some(name), _) | (None, Some(name)) => name.to_string(),
            // `m.slice(1, m.length - 1)`
            (None, None) => match m.len().checked_sub(1) {
                Some(end) if end > 1 => String::from_utf16_lossy(&m[1..end]),
                _ => String::new(),
            },
        };
        let ending_colon = m.last() == Some(&u16::from(b':'));
        let url = emoji_url(&value, opts)?;
        if !((emoticon.is_some() || ending_colon || unicode.is_some())
            && replaceable(&original, index, opts.inline))
        {
            return None;
        }
        let lazy = if opts.lazy { "loading='lazy' " } else { "" };
        Some(utf16(&format!(
            "<img width=\"20\" height=\"20\" src='{url}' title='{value}' {lazy}alt='{value}' class='emoji'>"
        )))
    };
    let pass = replace_all(&original, emoji_at, replace);
    let pass = replace_all(&pass, |u, p| text_emoji_at(u, p, opts.inline), replace);
    String::from_utf16_lossy(&pass)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn escape(s: &str) -> String {
        perform_emoji_escape(s, true, false)
    }

    #[test]
    fn escapes_unicode_emoji_after_space_or_punctuation() {
        assert_eq!(escape("party \u{1f389} time"), "party :tada: time");
        assert_eq!(escape("\u{1f389}"), ":tada:");
        assert_eq!(escape("(\u{1f389})"), "(:tada:)");
        // After a word character the emoji stays, unless inline emoji.
        assert_eq!(escape("party\u{1f389}"), "party\u{1f389}");
        assert_eq!(
            perform_emoji_escape("party\u{1f389}", true, true),
            "party:tada:"
        );
        // After another emoji.
        assert_eq!(escape("\u{1f389}\u{1f389}"), ":tada::tada:");
    }

    #[test]
    fn variation_selectors_and_tones() {
        // A key that never had a selector matches with one, which then
        // names nothing and the text stays.
        assert_eq!(escape("a \u{1f389}\u{fe0f}"), "a \u{1f389}\u{fe0f}");
        assert_eq!(escape("a \u{1f44d}\u{1f3fd}"), "a :+1:t4:");
        // ♡ matches alone, before the selector.
        assert_eq!(escape("a \u{2661}\u{fe0f}"), "a :heart:\u{fe0f}");
    }

    #[test]
    fn emoticons_check_the_original_string() {
        // ":D" sits at index 25 of the replaced text, past the end of the
        // original, whose last unit is "D", so it stays (Rails agrees).
        assert_eq!(escape("Smile :) and :D"), "Smile :slight_smile: and :D");
        assert_eq!(escape("x:)"), "x:)");
        assert_eq!(perform_emoji_escape("Smile :)", false, false), "Smile :)");
        // ":tada::)" is checked against "🎉:)" at index 6: past its end,
        // so the whole string, which ends in ")".
        assert_eq!(escape("\u{1f389}:)"), ":tada::slight_smile:");
    }

    #[test]
    fn text_regex_backtracks_to_a_boundary() {
        // `\B` after "a:" fails ("a" is a word character); the match
        // backtracks to ":" + "x" ... none of which is an emoticon.
        assert_eq!(text_emoji_at(&utf16(":xa"), 0, false), Some(2));
        assert_eq!(text_emoji_at(&utf16(":ab:"), 0, false), Some(4));
        assert_eq!(text_emoji_at(&utf16("a:b"), 1, false), None);
        assert_eq!(text_emoji_at(&utf16(":ab:t2:"), 0, false), Some(7));
        // No backtracking limit: a long name ends one short of the end.
        let long = format!(":{}", "a".repeat(100_000));
        assert_eq!(text_emoji_at(&utf16(&long), 0, false), Some(100_000));
    }

    #[test]
    fn unescapes_a_title() {
        let opts = UnescapeOptions {
            emoji_set: "twitter",
            emoji_base_path: "/images/emoji",
            shortcuts: true,
            inline: false,
            lazy: true,
        };
        let v = crate::emoji::image_version();
        assert_eq!(
            perform_emoji_unescape("a \u{1f389} :wave: :)", &opts),
            format!(
                "a <img width=\"20\" height=\"20\" src='/images/emoji/twitter/tada.png?v={v}' title='tada' loading='lazy' alt='tada' class='emoji'> \
                 <img width=\"20\" height=\"20\" src='/images/emoji/twitter/wave.png?v={v}' title='wave' loading='lazy' alt='wave' class='emoji'> \
                 <img width=\"20\" height=\"20\" src='/images/emoji/twitter/slight_smile.png?v={v}' title='slight_smile' loading='lazy' alt='slight_smile' class='emoji'>"
            )
        );
        assert_eq!(
            perform_emoji_unescape("no :nope: here", &opts),
            "no :nope: here"
        );
    }
}
