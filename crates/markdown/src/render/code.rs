//! features/code.js: fenced code as `<pre data-code-*><code class="lang-*">`.

use markdown_it::common::utils::{escape_html, unescape_all};
use markdown_it::parser::core::Root;
use markdown_it::plugins::cmark::block::fence::CodeFence;
use markdown_it::{Node, NodeValue, Renderer};

use super::RenderSettings;
use super::tight::AfterText;
use crate::sanitizer::AllowList;

/// `TEXT_CODE_CLASSES`
const TEXT_CODE_CLASSES: [&str; 3] = ["text", "pre", "plain"];

#[derive(Debug)]
struct Fence {
    html: String,
}

impl NodeValue for Fence {
    fn render(&self, node: &Node, fmt: &mut dyn Renderer) {
        if node.ext.get::<AfterText>().is_none() {
            fmt.cr();
        }
        fmt.text_raw(&self.html);
        fmt.cr();
    }
}

/// `String#replace(/[^\x00-\x7F]/, "")`: only the first match goes.
fn drop_first_non_ascii(s: &str) -> String {
    match s.char_indices().find(|(_, c)| !c.is_ascii()) {
        Some((i, c)) => format!("{}{}", &s[..i], &s[i + c.len_utf8()..]),
        None => s.to_string(),
    }
}

/// `extractTokenInfo`: the language tag and the `key=value` attributes
/// after it, in order.
fn token_info(info: &str) -> Option<(String, Vec<(String, String)>)> {
    let info = info.trim();
    if info.is_empty() {
        return None;
    }
    let (first, rest) = match info.find(char::is_whitespace) {
        Some(i) => (&info[..i], info[i..].trim_start()),
        None => (info, ""),
    };
    // c++, structured-text and p91 are all valid.
    if !first
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-'))
    {
        return None;
    }
    let tag = unescape_all(&drop_first_non_ascii(first)).into_owned();
    let mut attributes: Vec<(String, String)> = Vec::new();
    if !rest.is_empty() {
        let unescaped = unescape_all(&drop_first_non_ascii(rest)).into_owned();
        for pair in unescaped.split(',') {
            let first_word = pair.split_whitespace().next().unwrap_or("");
            let mut parts = first_word.split('=');
            let (key, value) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            // An invalid pair such as `foo=` is dropped.
            if !key.is_empty() && !value.is_empty() {
                set(&mut attributes, key, value);
            }
        }
    }
    Some((tag, attributes))
}

/// Assignment on a JS object: a new key goes last, an existing one keeps
/// its place.
fn set(attributes: &mut Vec<(String, String)>, key: &str, value: &str) {
    match attributes.iter_mut().find(|(k, _)| k == key) {
        Some(entry) => entry.1 = value.to_string(),
        None => attributes.push((key.to_string(), value.to_string())),
    }
}

fn html(fence: &CodeFence, settings: &RenderSettings) -> String {
    let info = token_info(&fence.info);
    let (tag, mut attributes) = match info {
        Some((tag, attributes)) if !tag.is_empty() => (tag, attributes),
        Some((_, attributes)) => (settings.default_code_lang.clone(), attributes),
        None => (settings.default_code_lang.clone(), Vec::new()),
    };
    let class = if TEXT_CODE_CLASSES.contains(&tag.as_str()) {
        "lang-plaintext".to_string()
    } else if tag == "auto" {
        "lang-auto".to_string()
    } else {
        set(&mut attributes, "wrap", &tag);
        format!("lang-{}", escape_html(&tag))
    };
    let data: Vec<String> = attributes
        .iter()
        .map(|(k, v)| format!("data-code-{}=\"{}\"", escape_html(k), escape_html(v)))
        .collect();
    format!(
        "<pre{}{}><code class=\"{class}\">{}</code></pre>",
        if data.is_empty() { "" } else { " " },
        data.join(" "),
        escape_html(&fence.content)
    )
}

/// A fence left open at the end of a source without a final newline. JS
/// `getLines` adds a line's newline only if there is one, the crate always
/// does, so its content would end in a newline Rails does not write.
fn open_at_end(node: &Node, fence: &CodeFence, source: &str) -> bool {
    let ends_source = node
        .srcmap
        .is_some_and(|map| map.get_byte_offsets().1 >= source.len());
    if !ends_source || source.ends_with('\n') {
        return false;
    }
    // The closing marker, after any blockquote markers and indent.
    let last = source.rsplit('\n').next().unwrap_or_default();
    let last = last.trim_start_matches([' ', '\t', '>']);
    let run = last.chars().take_while(|c| *c == fence.marker).count();
    let closed = run >= fence.marker_len && last[run * fence.marker.len_utf8()..].trim().is_empty();
    !closed
}

pub fn apply(root: &mut Node, settings: &RenderSettings) {
    let source = root
        .cast::<Root>()
        .map(|r| r.content.clone())
        .unwrap_or_default();
    root.walk_mut(|node, _| {
        if node.is::<CodeFence>() {
            let open = open_at_end(node, node.cast::<CodeFence>().unwrap(), &source);
            let fence = node.cast_mut::<CodeFence>().unwrap();
            if open && fence.content.ends_with('\n') {
                fence.content.pop();
            }
        }
        if let Some(fence) = node.cast::<CodeFence>() {
            let html = html(fence, settings);
            node.replace(Fence { html });
        }
    });
}

pub fn allow(list: &mut AllowList) {
    list.allow(&["pre[data-code-*]"]);
    list.allow_custom(|tag, name, value| {
        tag == "code"
            && name == "class"
            && value.len() > "lang-".len()
            && value.starts_with("lang-")
    });
}
