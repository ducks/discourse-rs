//! features/emoji.js: `:name:` codes, skin tones, emoticon shortcuts and
//! unicode emoji in text become emoji images. JavaScript indexes strings
//! by UTF-16 unit and so does this, since the name length limit and the
//! boundary checks count that way.

use super::md_utils::is_punct_char;
use markdown_it::parser::inline::Text;

use markdown_it::{Node, NodeValue, Renderer};

use super::RenderSettings;
use crate::emoji::DATA;
use crate::pretty_text::sanitizer::AllowList;

const MAX_NAME_LENGTH: usize = 60;
const ZERO_WIDTH_SPACE: u16 = 0x200b;
const COLON: u16 = 58;
const VARIATION_SELECTOR: char = '\u{FE0F}';

#[derive(Debug)]
pub struct Emoji {
    url: String,
    title: String,
    only_emoji: bool,
}

impl NodeValue for Emoji {
    fn render(&self, _: &Node, fmt: &mut dyn Renderer) {
        let class = if self.only_emoji {
            "emoji only-emoji"
        } else {
            "emoji"
        };
        fmt.self_close(
            "img",
            &[
                ("src", self.url.clone()),
                ("title", self.title.clone()),
                ("class", class.into()),
                ("alt", self.title.clone()),
                ("loading", "lazy".into()),
                ("width", "20".into()),
                ("height", "20".into()),
            ],
        );
    }
}

/// `md.utils.isSpace`
fn is_space(unit: u16) -> bool {
    unit == 0x09 || unit == 0x20
}

/// `isValidEmojiPrecedingChar`
fn valid_preceding(unit: u16) -> bool {
    is_space(unit)
        || char::from_u32(u32::from(unit)).is_some_and(is_punct_char)
        || unit == ZERO_WIDTH_SPACE
}

/// `buildEmojiUnicodeReplacer`: unicode emoji to `:name:` codes, with a
/// zero-width space in front of one that follows a word character, and
/// every variation selector removed. The generated regex is a trie of
/// the replacement keys, each also with a trailing variation selector, so
/// a match is the longest key at a position.
fn replace_unicode(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let mut found: Option<(usize, &str)> = None;
        // Only a non-ASCII char, or a keycap base, can start an emoji.
        if !chars[i].is_ascii() || matches!(chars[i], '#' | '*' | '0'..='9') {
            let longest = (DATA.unicode_max_chars + 1).min(chars.len() - i);
            for len in (1..=longest).rev() {
                let candidate: String = chars[i..i + len].iter().collect();
                let name = DATA.unicode.get(&candidate).or_else(|| {
                    candidate
                        .strip_suffix(VARIATION_SELECTOR)
                        .and_then(|bare| DATA.unicode.get(bare))
                });
                if let Some(name) = name {
                    found = Some((len, name));
                    break;
                }
            }
        }
        match found {
            Some((len, name)) => {
                // `!/\B/.test(before)`: the unit before is a word character.
                let before_is_word = out
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
                if before_is_word {
                    out.push('\u{200b}');
                }
                out.push(':');
                out.push_str(name);
                out.push(':');
                i += len;
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out.replace(VARIATION_SELECTOR, "")
}

fn slice(units: &[u16], start: usize, end: usize) -> String {
    let end = end.min(units.len());
    String::from_utf16_lossy(&units[start.min(end)..end])
}

/// `getEmojiName`: the text between the colon at `pos` and the next one,
/// taking a `:t2:`..`:t6:` tone along.
fn emoji_name(content: &[u16], pos: usize, inline_emoji: bool) -> Option<String> {
    if content.get(pos) != Some(&COLON) {
        return None;
    }
    if pos > 0 && !inline_emoji && !valid_preceding(content[pos - 1]) {
        return None;
    }
    let pos = pos + 1;
    if content.get(pos) == Some(&COLON) {
        return None;
    }
    let mut length = 0;
    while length < MAX_NAME_LENGTH {
        length += 1;
        if content.get(pos + length) == Some(&COLON) {
            let tone = slice(content, pos + length + 1, pos + length + 4);
            let mut tone = tone.chars();
            if tone.next() == Some('t')
                && tone.next().is_some_and(|c| ('2'..='6').contains(&c))
                && tone.next() == Some(':')
            {
                length += 3;
            }
            break;
        }
        if pos + length > content.len() {
            return None;
        }
    }
    if length == MAX_NAME_LENGTH {
        return None;
    }
    Some(slice(content, pos, pos + length))
}

/// `imageFor` / `buildEmojiUrl`: the image of a known emoji or alias; a
/// tone becomes a path segment.
fn emoji_node(code: &str, settings: &RenderSettings) -> Option<Emoji> {
    let code = code.to_lowercase();
    let without_tone = code.split(':').next().filter(|s| !s.is_empty())?;
    if settings.emoji_set.is_empty() || !DATA.exists(without_tone) {
        return None;
    }
    Some(Emoji {
        url: format!(
            "{}/{}/{}.png?v={}",
            settings.emoji_base_path,
            settings.emoji_set,
            code.replacen(":t", "/", 1),
            crate::emoji::image_version()
        ),
        title: format!(":{code}:"),
        only_emoji: false,
    })
}

/// `getEmojiTokenByTranslation`: an emoticon at `pos`, standing alone.
/// The walk follows the translation tree as far as the text goes and
/// gives up on the first character that leaves it.
fn emoticon(content: &[u16], start: usize, settings: &RenderSettings) -> Option<(usize, Emoji)> {
    let mut pos = start;
    let mut prefix = String::new();
    let mut found: Option<&String> = None;
    loop {
        let continues = DATA
            .translations
            .keys()
            .any(|k| k.len() > prefix.len() && k.starts_with(prefix.as_str()));
        if !continues || pos >= content.len() {
            break;
        }
        let next = char::from_u32(u32::from(content[pos]))?;
        prefix.push(next);
        if !DATA
            .translations
            .keys()
            .any(|k| k.starts_with(prefix.as_str()))
        {
            return None;
        }
        found = DATA.translations.get(&prefix);
        pos += 1;
    }
    let name = found?;
    if start > 0 && !valid_preceding(content[start - 1]) {
        return None;
    }
    if pos < content.len() && !is_space(content[pos]) {
        return None;
    }
    emoji_node(name, settings).map(|emoji| (pos, emoji))
}

enum Piece {
    Text(String),
    Emoji(Emoji),
}

/// `applyEmoji`: the text split around the emoji in it, or None when it
/// has none.
fn apply(content: &str, settings: &RenderSettings) -> Option<Vec<Piece>> {
    let replaced = replace_unicode(content);
    let units: Vec<u16> = replaced.encode_utf16().collect();
    let mut result: Option<Vec<Piece>> = None;
    let mut start = 0;
    let mut end = units.len();

    let mut i = 0;
    while i + 1 < units.len() {
        let mut token: Option<(usize, Emoji)> = None;
        if let Some(name) = emoji_name(&units, i, settings.inline_emoji)
            && let Some(emoji) = emoji_node(&name, settings)
        {
            token = Some((name.encode_utf16().count() + 2, emoji));
        }
        if settings.emoji_shortcuts
            && token.is_none()
            && let Some((pos, emoji)) = emoticon(&units, i, settings)
        {
            token = Some((pos - i, emoji));
        }
        if let Some((offset, emoji)) = token {
            let pieces = result.get_or_insert_with(Vec::new);
            if i > start {
                pieces.push(Piece::Text(slice(&units, start, i)));
            }
            pieces.push(Piece::Emoji(emoji));
            start = i + offset;
            end = start;
            i += offset;
        } else {
            i += 1;
        }
    }

    let mut pieces = match result {
        Some(pieces) => pieces,
        // The unicode replacer may have changed the text (a stripped
        // variation selector) without leaving an emoji.
        None if replaced != content => return Some(vec![Piece::Text(replaced)]),
        None => return None,
    };
    if end < units.len() {
        pieces.push(Piece::Text(slice(&units, end, units.len())));
    }

    // Up to three emoji alone in the text, separated by single spaces,
    // are shown large.
    let emoji_count = pieces
        .iter()
        .filter(|p| matches!(p, Piece::Emoji(_)))
        .count();
    let alone = pieces.len() <= 5
        && emoji_count <= 3
        && matches!(pieces.first(), Some(Piece::Emoji(_)))
        && matches!(pieces.last(), Some(Piece::Emoji(_)))
        && pieces.iter().enumerate().all(|(index, piece)| match piece {
            Piece::Emoji(_) => true,
            Piece::Text(text) => {
                text == " " && index > 0 && matches!(pieces[index - 1], Piece::Emoji(_))
            }
        });
    if alone {
        for piece in pieces.iter_mut() {
            if let Piece::Emoji(emoji) = piece {
                emoji.only_emoji = true;
            }
        }
    }
    Some(pieces)
}

/// The `emoji` core rule: textReplace over every text outside an autolink.
pub fn run(root: &mut Node, settings: &RenderSettings) {
    if !settings.emoji {
        return;
    }
    fn visit(node: &mut Node, settings: &RenderSettings) {
        if super::linkify::is_auto_link(node) {
            return;
        }
        let mut i = 0;
        while i < node.children.len() {
            let replaced = node.children[i]
                .cast::<Text>()
                .and_then(|text| apply(&text.content, settings));
            match replaced {
                Some(pieces) => {
                    let nodes: Vec<Node> = pieces
                        .into_iter()
                        .map(|piece| match piece {
                            Piece::Text(content) => Node::new(Text { content }),
                            Piece::Emoji(emoji) => Node::new(emoji),
                        })
                        .collect();
                    let count = nodes.len();
                    node.children.splice(i..=i, nodes);
                    i += count;
                }
                None => {
                    visit(&mut node.children[i], settings);
                    i += 1;
                }
            }
        }
    }
    visit(root, settings);
}

pub fn allow(list: &mut AllowList) {
    list.allow(&[
        "img[class=emoji]",
        "img[class=emoji emoji-custom]",
        "img[class=emoji emoji-custom only-emoji]",
        "img[class=emoji only-emoji]",
        "img[loading=lazy]",
        "img[width=20]",
        "img[height=20]",
    ]);
}
