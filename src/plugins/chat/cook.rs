//! `Chat::Message.cook`: a chat message's raw text through PrettyText with
//! chat's markdown options (its features and markdown-it rules, quotes
//! always linked, hashtags in the chat composer's context), its slash
//! commands applied: /me as an action naming the author, /shrug and
//! /tableflip appending their faces. Oneboxes come from the cache Rails
//! applies them from, which the port does not keep, so links stay links.

use crate::pretty_text::cleanup::{element_name, fragment_root, parse};
use crate::pretty_text::{CookError, Host, MarkdownOptions};
use crate::topic_list_view::escape;

/// `Chat::Message::BAKED_VERSION`
pub const BAKED_VERSION: i32 = 2;

/// A matched `SLASH_COMMAND_PATTERNS` entry.
enum Command {
    /// /me: the content, cooked, then the author before it.
    Action(String),
    /// /shrug and /tableflip: the content (maybe empty), then the face.
    Text(String, &'static str),
}

/// `match_slash_command`
fn slash_command(message: &str, author_username: &str) -> Option<Command> {
    // A whole message with no line breaks after the command's space.
    let rest_of = |name: &str| -> Option<Option<String>> {
        let rest = message.strip_prefix(name)?;
        if rest.is_empty() {
            return Some(None);
        }
        let body = rest.trim_start_matches([' ', '\t']);
        if body.len() == rest.len() || body.is_empty() || body.contains(['\r', '\n']) {
            return None;
        }
        Some(Some(body.to_string()))
    };
    if !author_username.is_empty()
        && let Some(Some(body)) = rest_of("/me")
    {
        return Some(Command::Action(body));
    }
    if let Some(body) = rest_of("/shrug") {
        return Some(Command::Text(body.unwrap_or_default(), "¯\\_(ツ)_/¯"));
    }
    if let Some(body) = rest_of("/tableflip") {
        return Some(Command::Text(body.unwrap_or_default(), "(╯°□°)╯︵ ┻━┻"));
    }
    None
}

/// The top-level elements of cooked HTML, by name.
fn top_level_elements(cooked: &str) -> Vec<String> {
    let dom = parse(cooked);
    let root = fragment_root(&dom);
    root.children
        .borrow()
        .iter()
        .filter_map(|n| element_name(n).map(str::to_string))
        .collect()
}

/// `format_action`: a single paragraph's content inside
/// `<em class="chat-message-action">`, the author first.
fn format_action(cooked: &str, author_username: &str) -> String {
    let elements = top_level_elements(cooked);
    let Some(inner) = cooked
        .strip_prefix("<p>")
        .and_then(|c| c.strip_suffix("</p>"))
        .filter(|_| elements.len() == 1 && elements[0] == "p")
    else {
        return cooked.to_string();
    };
    format!(
        "<p><em class=\"chat-message-action\">{} {inner}</em></p>",
        escape(author_username)
    )
}

/// `append_command_text`: the face after a single paragraph's content
/// (or in a paragraph of its own).
fn append_command_text(cooked: &str, text: &str) -> String {
    let elements = top_level_elements(cooked);
    if elements.len() > 1 || (elements.len() == 1 && elements[0] != "p") {
        return cooked.to_string();
    }
    if elements.is_empty() {
        return format!("{cooked}<p>{}</p>", escape(text));
    }
    let Some(inner) = cooked
        .strip_prefix("<p>")
        .and_then(|c| c.strip_suffix("</p>"))
    else {
        return cooked.to_string();
    };
    let separator = if inner.is_empty() { "" } else { " " };
    format!("<p>{inner}{separator}{}</p>", escape(text))
}

/// `Chat::Message.cook(message, user_id:, author_username:)`
pub async fn cook(
    host: &Host,
    message: &str,
    user_id: i32,
    author_username: &str,
) -> Result<String, CookError> {
    let command = slash_command(message, author_username);
    let to_cook = match &command {
        Some(Command::Action(body)) => body.as_str(),
        Some(Command::Text(body, _)) => body.as_str(),
        None => message,
    };
    let opts = MarkdownOptions {
        user_id: Some(i64::from(user_id)),
        force_quote_link: true,
        chat: true,
        ..Default::default()
    };
    let cooked = crate::pretty_text::cook(host, to_cook, &opts).await?;
    Ok(match command {
        Some(Command::Action(_)) => format_action(&cooked, author_username),
        Some(Command::Text(_, face)) => append_command_text(&cooked, face),
        None => cooked,
    })
}
