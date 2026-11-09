//! chat's chat-transcript.js: the `[chat quote="user;id;time" ...]` block a
//! quoted chat message (or an archived channel's messages) is written as,
//! drawn as a transcript: its author's avatar and name, a link to the
//! message, the channel, the messages cooked with chat's own rules
//! (`customMarkdownCookFn`), and their reactions. A thread's transcript
//! wraps its original message in a details summary.
//!
//! The rule pushes plain tokens; they are rendered here as markdown-it's
//! renderToken writes them, newlines included, and written as the rich
//! editor's tree (docs/rich-editor.md) one by one with their nesting, since
//! a thread's tokens open and close across one another.

use markdown_it::common::utils::escape_html;
use serde_json::{Map, Value, json};

use super::RenderSettings;
use super::bbcode::TagInfo;
use super::context::Context;
use crate::emoji_escape::{UnescapeOptions, perform_emoji_unescape};

/// A token the rule pushes.
enum Tok {
    /// An opening (or closing) tag, `block` as the token has it. `kind` is
    /// its type less `_open` / `_close`, or the whole type when `bare`
    /// (the reactions' tokens are pushed with one type for both).
    Tag {
        kind: &'static str,
        tag: &'static str,
        open: bool,
        attrs: Vec<(String, String)>,
        block: bool,
        bare: bool,
    },
    /// `html_inline` (nesting 0) or `html_raw` (1 with the content, then
    /// -1): its content as it is.
    Html {
        kind: &'static str,
        nesting: i8,
        content: String,
    },
}

fn open(kind: &'static str, tag: &'static str, attrs: &[(&str, &str)]) -> Tok {
    Tok::Tag {
        kind,
        tag,
        open: true,
        attrs: attrs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        block: true,
        bare: false,
    }
}

fn close(kind: &'static str, tag: &'static str) -> Tok {
    Tok::Tag {
        kind,
        tag,
        open: false,
        attrs: Vec::new(),
        block: true,
        bare: false,
    }
}

/// The token with one type for its open and close.
fn bare(mut tok: Tok) -> Tok {
    if let Tok::Tag { bare, .. } = &mut tok {
        *bare = true;
    }
    tok
}

fn html_inline(content: impl Into<String>) -> Tok {
    Tok::Html {
        kind: "html_inline",
        nesting: 0,
        content: content.into(),
    }
}

/// `html_raw` pushed open with the HTML, then closed.
fn html_raw(content: String) -> [Tok; 2] {
    [
        Tok::Html {
            kind: "html_raw",
            nesting: 1,
            content,
        },
        Tok::Html {
            kind: "html_raw",
            nesting: -1,
            content: String::new(),
        },
    ]
}

/// The token with `block = false`.
fn inline(mut tok: Tok) -> Tok {
    if let Tok::Tag { block, .. } = &mut tok {
        *block = false;
    }
    tok
}

/// markdown-it's `renderToken` (and the html rules' bare content): a
/// block tag ends its line unless it opens straight onto its own close.
fn render(toks: &[Tok]) -> String {
    let mut out = String::new();
    for (i, tok) in toks.iter().enumerate() {
        match tok {
            Tok::Html { content, .. } => out.push_str(content),
            Tok::Tag {
                tag,
                open,
                attrs,
                block,
                ..
            } => {
                out.push_str(if *open { "<" } else { "</" });
                out.push_str(tag);
                for (name, value) in attrs {
                    out.push_str(&format!(
                        " {}=\"{}\"",
                        escape_html(name),
                        escape_html(value)
                    ));
                }
                let mut need_lf = *block;
                if *block
                    && *open
                    && let Some(Tok::Tag {
                        tag: next,
                        open: false,
                        ..
                    }) = toks.get(i + 1)
                    && next == tag
                {
                    need_lf = false;
                }
                out.push_str(if need_lf { ">\n" } else { ">" });
            }
        }
    }
    out
}

/// The tokens as the editor's tree nodes, each on its own: its type, tag,
/// attrs and nesting, the wrapper's open token with the transcript's
/// source as its content (`wrapperDivToken.content`).
fn tree(toks: &[Tok], source: &str) -> Vec<Value> {
    toks.iter()
        .enumerate()
        .map(|(i, tok)| {
            let mut node = Map::new();
            match tok {
                Tok::Tag {
                    kind,
                    tag,
                    open,
                    attrs,
                    bare,
                    ..
                } => {
                    let kind = match (bare, open) {
                        (true, _) => kind.to_string(),
                        (false, true) => format!("{kind}_open"),
                        (false, false) => format!("{kind}_close"),
                    };
                    node.insert("type".into(), json!(kind));
                    node.insert("tag".into(), json!(tag));
                    if !attrs.is_empty() {
                        let attrs: Vec<Value> = attrs.iter().map(|(k, v)| json!([k, v])).collect();
                        node.insert("attrs".into(), json!(attrs));
                    }
                    if i == 0 {
                        node.insert("content".into(), json!(source));
                    }
                    node.insert("nesting".into(), json!(if *open { 1 } else { -1 }));
                }
                Tok::Html {
                    kind,
                    nesting,
                    content,
                } => {
                    node.insert("type".into(), json!(kind));
                    if !content.is_empty() {
                        node.insert("content".into(), json!(content));
                    }
                    if *nesting != 0 {
                        node.insert("nesting".into(), json!(nesting));
                    }
                }
            }
            Value::Object(node)
        })
        .collect()
}

/// discourse/lib/escape
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            '`' => out.push_str("&#x60;"),
            c => out.push(c),
        }
    }
    out
}

/// `options.getURL(path)`: a root-relative path under the base path.
fn get_url(path: &str, settings: &RenderSettings) -> String {
    if settings.base_path.is_empty()
        || path == settings.base_path
        || path.starts_with(&format!("{}/", settings.base_path))
    {
        path.to_string()
    } else {
        format!("{}{path}", settings.base_path)
    }
}

/// A JS truthy attribute: present and not empty.
fn truthy(info: &TagInfo, name: &str) -> bool {
    info.attr(name).is_some_and(|v| !v.is_empty())
}

/// `chatTranscriptRule.replace`: the transcript's HTML and its tokens for
/// the editor's tree, None when the quote lacks its user, message id or
/// time (the rule declines).
pub fn build(
    info: &TagInfo,
    content: &str,
    settings: &RenderSettings,
    ctx: &Context,
) -> Option<(String, Vec<Value>)> {
    let quote = info.attr("quote").filter(|q| !q.is_empty())?;
    let mut parts = quote.split(';');
    let username = parts.next().filter(|p| !p.is_empty())?;
    let message_id = parts.next().filter(|p| !p.is_empty())?;
    let message_time = parts.next().filter(|p| !p.is_empty())?;

    let reactions = info.attr("reactions").filter(|r| !r.is_empty());
    let multi_quote = truthy(info, "multiQuote");
    let no_link = truthy(info, "noLink");
    let channel_name = info.attr("channel").filter(|c| !c.is_empty());
    let channel_id = info.attr("channelId").filter(|c| !c.is_empty());
    let thread_id = info.attr("threadId").filter(|t| !t.is_empty());
    let thread_title = info.attr("threadTitle").filter(|t| !t.is_empty());
    // names a transcript that didn't come from a chat channel; never linked
    let source = info.attr("source").filter(|s| !s.is_empty());
    let source_type = info.attr("sourceType").filter(|s| !s.is_empty());
    let channel_link = channel_id.map(|id| get_url(&format!("/chat/c/-/{id}"), settings));
    let chained = truthy(info, "chained");

    let unescape = |s: &str| {
        perform_emoji_unescape(
            s,
            &UnescapeOptions {
                emoji_set: &settings.emoji_set,
                emoji_base_path: &settings.emoji_base_path,
                shortcuts: settings.emoji_shortcuts,
                inline: settings.inline_emoji,
                lazy: true,
                custom: None,
                denied: &[],
            },
        )
    };

    let is_thread = thread_id.is_some() && content.contains("[chat");
    let mut toks = Vec::new();

    let mut classes = vec!["chat-transcript"];
    if chained {
        classes.push("chat-transcript-chained");
    }
    let mut attrs: Vec<(String, String)> = vec![
        ("class".into(), classes.join(" ")),
        ("data-message-id".into(), message_id.into()),
        ("data-username".into(), username.into()),
        ("data-datetime".into(), message_time.into()),
    ];
    if chained {
        attrs.push(("data-chained".into(), "true".into()));
    }
    if let Some(r) = reactions {
        attrs.push(("data-reactions".into(), r.into()));
    }
    if let Some(c) = channel_name {
        attrs.push(("data-channel-name".into(), c.into()));
    }
    if let Some(c) = channel_id {
        attrs.push(("data-channel-id".into(), c.into()));
    }
    if let Some(s) = source {
        attrs.push(("data-source".into(), s.into()));
    }
    if let Some(s) = source_type {
        attrs.push(("data-source-type".into(), s.into()));
    }
    if multi_quote {
        attrs.push(("data-multiquote".into(), "true".into()));
    }
    if let Some(t) = thread_id {
        attrs.push(("data-thread-id".into(), t.into()));
    }
    if let Some(t) = thread_title {
        attrs.push(("data-thread-title".into(), t.into()));
    }
    toks.push(Tok::Tag {
        kind: "div_chat_transcript_wrap",
        tag: "div",
        open: true,
        attrs,
        block: true,
        bare: false,
    });

    let unescaped_channel = channel_name.map(unescape);
    if let (Some(channel), true) = (&unescaped_channel, multi_quote) {
        let Some(link) = &channel_link else {
            ctx.refuse("a multi-quote chat transcript without its channel id");
            return Some((String::new(), Vec::new()));
        };
        toks.push(open(
            "div_chat_transcript_meta",
            "div",
            &[("class", "chat-transcript-meta")],
        ));
        toks.push(html_inline(
            settings
                .chat_quote_original_channel
                .replace("%{channel}", channel)
                .replace("%{channelLink}", &escape(link)),
        ));
        toks.push(close("div_chat_transcript_meta", "div"));
    }

    if is_thread {
        toks.push(open("details_chat_transcript_wrap", "details", &[]));
        toks.push(open("summary_chat_transcript", "summary", &[]));
        toks.push(open(
            "div_thread",
            "div",
            &[("class", "chat-transcript-thread")],
        ));
        toks.push(open(
            "div_thread_header",
            "div",
            &[("class", "chat-transcript-thread-header")],
        ));
        toks.push(inline(open(
            "svg_thread_header",
            "svg",
            &[(
                "class",
                "fa d-icon d-icon-discourse-threads svg-icon svg-node",
            )],
        )));
        toks.push(inline(open(
            "use_svg_thread",
            "use",
            &[("href", "#discourse-threads")],
        )));
        toks.push(close("use_svg_thread", "use"));
        toks.push(close("svg_thread_header", "svg"));
        toks.push(open(
            "span_thread_title",
            "span",
            &[("class", "chat-transcript-thread-header__title")],
        ));
        let title = thread_title.map(unescape).unwrap_or_default();
        toks.push(html_inline(if title.is_empty() {
            settings.chat_quote_default_thread_title.clone()
        } else {
            title
        }));
        toks.push(close("span_thread_title", "span"));
        toks.push(close("div_thread_header", "div"));
    }

    toks.push(open(
        "div_chat_transcript_user",
        "div",
        &[("class", "chat-transcript-user")],
    ));
    toks.push(open(
        "div_chat_transcript_user_avatar",
        "div",
        &[("class", "chat-transcript-user-avatar")],
    ));
    if let Some(img) = super::quotes::avatar_img(ctx.avatar(username), settings) {
        toks.push(html_inline(img));
    }
    toks.push(close("div_chat_transcript_user_avatar", "div"));
    toks.push(open(
        "div_chat_transcript_username",
        "div",
        &[("class", "chat-transcript-username")],
    ));
    // formatUsername is the identity.
    toks.push(html_inline(username));
    toks.push(close("div_chat_transcript_username", "div"));
    toks.push(open(
        "div_chat_transcript_datetime",
        "div",
        &[("class", "chat-transcript-datetime")],
    ));
    let href = channel_link.clone().unwrap_or_else(|| "null".into());
    match (&channel_link, no_link) {
        (Some(link), false) => {
            toks.push(inline(open(
                "link",
                "a",
                &[
                    ("href", &format!("{link}/{message_id}")),
                    ("title", message_time),
                ],
            )));
            toks.push(inline(close("link", "a")));
        }
        _ => {
            toks.push(inline(open("span", "span", &[("title", message_time)])));
            if let (Some(channel), false) = (channel_name, multi_quote) {
                toks.push(open(
                    "link",
                    "a",
                    &[("class", "chat-transcript-channel"), ("href", &href)],
                ));
                toks.push(html_inline(format!("#{channel}")));
                toks.push(inline(close("link", "a")));
            }
            toks.push(inline(close("span", "span")));
        }
    }
    toks.push(close("div_chat_transcript_datetime", "div"));
    if let (Some(channel), false) = (&unescaped_channel, multi_quote) {
        toks.push(open(
            "link",
            "a",
            &[("class", "chat-transcript-channel"), ("href", &href)],
        ));
        toks.push(html_inline(format!("#{channel}")));
        toks.push(inline(close("link", "a")));
    }
    toks.push(close("div_chat_transcript_user", "div"));

    toks.push(open(
        "div_chat_transcript_messages",
        "div",
        &[("class", "chat-transcript-messages")],
    ));
    let cook = |raw: &str| super::render_chat_fragment(raw, settings, ctx);
    // `/\[chat/i`
    let thread_start = (0..content.len()).find(|&i| {
        content.is_char_boundary(i)
            && content.as_bytes()[i] == b'['
            && content
                .get(i + 1..i + 5)
                .is_some_and(|w| w.eq_ignore_ascii_case("chat"))
    });
    match thread_start.filter(|_| is_thread) {
        Some(at) => {
            toks.extend(html_raw(cook(&content[..at])));
            toks.push(close("div_thread", "div"));
            toks.push(close("summary_chat_transcript", "summary"));
            toks.extend(html_raw(cook(&content[at..])));
            toks.push(close("details_chat_transcript_wrap", "details"));
        }
        None => toks.extend(html_raw(cook(content))),
    }

    if let Some(reactions) = reactions {
        toks.push(bare(open(
            "div_chat_transcript_reactions",
            "div",
            &[("class", "chat-transcript-reactions")],
        )));
        for reaction in reactions.split(';') {
            let mut split = reaction.split(':');
            let emoji = split.next().unwrap_or_default();
            let Some(usernames) = split.next() else {
                // `split[1].split` on undefined throws.
                ctx.refuse("a chat transcript reaction without its users");
                return Some((String::new(), Vec::new()));
            };
            toks.push(bare(open(
                "div_chat_transcript_reaction",
                "div",
                &[("class", "chat-transcript-reaction")],
            )));
            toks.push(html_inline(format!(
                "{} {}",
                unescape(&format!(":{emoji}:")),
                usernames.split(',').count()
            )));
            toks.push(bare(close("div_chat_transcript_reaction", "div")));
        }
        toks.push(bare(close("div_chat_transcript_reactions", "div")));
    }

    toks.push(close("div_chat_transcript_messages", "div"));
    toks.push(close("div_chat_transcript_wrap", "div"));
    Some((render(&toks), tree(&toks, content)))
}

/// chat-transcript.js's `allowList`.
pub fn allow(list: &mut crate::sanitizer::AllowList) {
    list.allow(&[
        "svg[class=fa d-icon d-icon-discourse-threads svg-icon svg-node]",
        "use[href=#discourse-threads]",
        "div[class=chat-transcript]",
        "details[class=chat-transcript]",
        "div[class=chat-transcript chat-transcript-chained]",
        "details[class=chat-transcript chat-transcript-chained]",
        "div.chat-transcript-meta",
        "div.chat-transcript-user",
        "div.chat-transcript-username",
        "div.chat-transcript-user-avatar",
        "div.chat-transcript-messages",
        "div.chat-transcript-datetime",
        "div.chat-transcript-reactions",
        "div.chat-transcript-reaction",
        "span[title]",
        "div[data-message-id]",
        "div[data-channel-name]",
        "div[data-channel-id]",
        "div[data-source]",
        "div[data-source-type]",
        "div[data-username]",
        "div[data-datetime]",
        "a.chat-transcript-channel",
        "div.chat-transcript-thread",
        "div.chat-transcript-thread-header",
        "span.chat-transcript-thread-header__title",
    ]);
}
