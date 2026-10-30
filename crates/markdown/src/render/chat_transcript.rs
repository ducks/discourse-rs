//! chat's chat-transcript.js: the `[chat quote="user;id;time" ...]` block a
//! quoted chat message (or an archived channel's messages) is written as,
//! drawn as a transcript: its author's avatar and name, a link to the
//! message, the channel, the messages cooked with chat's own rules
//! (`customMarkdownCookFn`), and their reactions. A thread's transcript
//! wraps its original message in a details summary.
//!
//! The rule pushes plain tokens; they are rendered here as markdown-it's
//! renderToken writes them, newlines included.

use markdown_it::common::utils::escape_html;

use super::RenderSettings;
use super::bbcode::TagInfo;
use super::context::Context;
use crate::emoji_escape::{UnescapeOptions, perform_emoji_unescape};

/// A token the rule pushes.
enum Tok {
    /// An opening (or closing) tag, `block` as the token has it.
    Tag {
        tag: &'static str,
        open: bool,
        attrs: Vec<(String, String)>,
        block: bool,
    },
    /// `html_inline` or `html_raw`: its content as it is.
    Html(String),
}

fn open(tag: &'static str, attrs: &[(&str, &str)]) -> Tok {
    Tok::Tag {
        tag,
        open: true,
        attrs: attrs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        block: true,
    }
}

fn close(tag: &'static str) -> Tok {
    Tok::Tag {
        tag,
        open: false,
        attrs: Vec::new(),
        block: true,
    }
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
            Tok::Html(content) => out.push_str(content),
            Tok::Tag {
                tag,
                open,
                attrs,
                block,
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

/// `chatTranscriptRule.replace`: the transcript's HTML, None when the
/// quote lacks its user, message id or time (the rule declines).
pub fn build(
    info: &TagInfo,
    content: &str,
    settings: &RenderSettings,
    ctx: &Context,
) -> Option<String> {
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
        tag: "div",
        open: true,
        attrs,
        block: true,
    });

    let unescaped_channel = channel_name.map(unescape);
    if let (Some(channel), true) = (&unescaped_channel, multi_quote) {
        let Some(link) = &channel_link else {
            ctx.refuse("a multi-quote chat transcript without its channel id");
            return Some(String::new());
        };
        toks.push(open("div", &[("class", "chat-transcript-meta")]));
        toks.push(Tok::Html(
            settings
                .chat_quote_original_channel
                .replace("%{channel}", channel)
                .replace("%{channelLink}", &escape(link)),
        ));
        toks.push(close("div"));
    }

    if is_thread {
        toks.push(open("details", &[]));
        toks.push(open("summary", &[]));
        toks.push(open("div", &[("class", "chat-transcript-thread")]));
        toks.push(open("div", &[("class", "chat-transcript-thread-header")]));
        toks.push(inline(open(
            "svg",
            &[(
                "class",
                "fa d-icon d-icon-discourse-threads svg-icon svg-node",
            )],
        )));
        toks.push(inline(open("use", &[("href", "#discourse-threads")])));
        toks.push(close("use"));
        toks.push(close("svg"));
        toks.push(open(
            "span",
            &[("class", "chat-transcript-thread-header__title")],
        ));
        let title = thread_title.map(unescape).unwrap_or_default();
        toks.push(Tok::Html(if title.is_empty() {
            settings.chat_quote_default_thread_title.clone()
        } else {
            title
        }));
        toks.push(close("span"));
        toks.push(close("div"));
    }

    toks.push(open("div", &[("class", "chat-transcript-user")]));
    toks.push(open("div", &[("class", "chat-transcript-user-avatar")]));
    if let Some(img) = super::quotes::avatar_img(ctx.avatar(username), settings) {
        toks.push(Tok::Html(img));
    }
    toks.push(close("div"));
    toks.push(open("div", &[("class", "chat-transcript-username")]));
    // formatUsername is the identity.
    toks.push(Tok::Html(username.to_string()));
    toks.push(close("div"));
    toks.push(open("div", &[("class", "chat-transcript-datetime")]));
    let href = channel_link.clone().unwrap_or_else(|| "null".into());
    match (&channel_link, no_link) {
        (Some(link), false) => {
            toks.push(inline(open(
                "a",
                &[
                    ("href", &format!("{link}/{message_id}")),
                    ("title", message_time),
                ],
            )));
            toks.push(inline(close("a")));
        }
        _ => {
            toks.push(inline(open("span", &[("title", message_time)])));
            if let (Some(channel), false) = (channel_name, multi_quote) {
                toks.push(open(
                    "a",
                    &[("class", "chat-transcript-channel"), ("href", &href)],
                ));
                toks.push(Tok::Html(format!("#{channel}")));
                toks.push(inline(close("a")));
            }
            toks.push(inline(close("span")));
        }
    }
    toks.push(close("div"));
    if let (Some(channel), false) = (&unescaped_channel, multi_quote) {
        toks.push(open(
            "a",
            &[("class", "chat-transcript-channel"), ("href", &href)],
        ));
        toks.push(Tok::Html(format!("#{channel}")));
        toks.push(inline(close("a")));
    }
    toks.push(close("div"));

    toks.push(open("div", &[("class", "chat-transcript-messages")]));
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
            toks.push(Tok::Html(cook(&content[..at])));
            toks.push(Tok::Html(String::new()));
            toks.push(close("div"));
            toks.push(close("summary"));
            toks.push(Tok::Html(cook(&content[at..])));
            toks.push(Tok::Html(String::new()));
            toks.push(close("details"));
        }
        None => {
            toks.push(Tok::Html(cook(content)));
            toks.push(Tok::Html(String::new()));
        }
    }

    if let Some(reactions) = reactions {
        toks.push(open("div", &[("class", "chat-transcript-reactions")]));
        for reaction in reactions.split(';') {
            let mut split = reaction.split(':');
            let emoji = split.next().unwrap_or_default();
            let Some(usernames) = split.next() else {
                // `split[1].split` on undefined throws.
                ctx.refuse("a chat transcript reaction without its users");
                return Some(String::new());
            };
            toks.push(open("div", &[("class", "chat-transcript-reaction")]));
            toks.push(Tok::Html(format!(
                "{} {}",
                unescape(&format!(":{emoji}:")),
                usernames.split(',').count()
            )));
            toks.push(close("div"));
        }
        toks.push(close("div"));
    }

    toks.push(close("div"));
    toks.push(close("div"));
    Some(render(&toks))
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
        "div[data-username]",
        "div[data-datetime]",
        "a.chat-transcript-channel",
        "div.chat-transcript-thread",
        "div.chat-transcript-thread-header",
        "span.chat-transcript-thread-header__title",
    ]);
}
