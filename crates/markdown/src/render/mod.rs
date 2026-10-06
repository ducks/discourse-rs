//! The renderer: Discourse's markdown (frontend/discourse-markdown-it) on
//! the markdown-it crate, a Rust port of the library Discourse uses.
//!
//! `setup.js` builds markdown-it with `html: true`, `breaks` unless
//! traditional_markdown_linebreaks, linkify and the typographer by their
//! settings, then adds Discourse's features. The features are ported one
//! by one as rules and node renderers in the modules here;
//! tests/pretty_text.rs lists the recorded corpus entries that do not cook
//! byte-equal to Rails yet.

mod anchor;
mod bbcode;
mod blocks;
mod checklist;
mod code;
pub mod context;
mod element;
mod emoji;
mod footnotes;
mod html_img;
mod link_pipes;
pub mod linkify;
pub mod local_dates;
mod md_utils;
mod newline;
mod onebox;
mod poll;
mod quotes;
mod smartquotes;
mod table;
mod text_join;
mod text_post_process;
mod tight;
mod typographer;
mod untrimmed;
mod uploads;

use std::sync::Arc;

use markdown_it::parser::extset::MarkdownItExt;
use markdown_it::plugins::cmark::inline::newline::{Hardbreak, Softbreak};
use markdown_it::{MarkdownIt, Node};

use self::context::{Context, Lookups, Needs};
use self::linkify::LinkifyIt;
use crate::Unsupported;
use crate::sanitizer::{AllowList, sanitize};

/// What the rules read: `discourse.limitedSiteSettings`, the feature
/// switches, and the per-cook options. The server builds it from the site
/// settings; the composer's preview receives it as JSON.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RenderSettings {
    /// `breaks`: a newline inside a paragraph is a `<br>`.
    pub breaks: bool,
    /// `markdown_linkify_tlds` when linkify is on (`enable_markdown_linkify`).
    pub linkify_tlds: Option<Vec<String>>,
    /// `linkify`, compiled from linkify_tlds by `compiled`.
    #[serde(skip)]
    pub linkify: Option<Arc<LinkifyIt>>,
    pub typographer: bool,
    /// `features.mentions`
    pub mentions: bool,
    /// `features.emoji`, `emojiShortcuts` and `inlineEmoji`.
    pub emoji: bool,
    pub emoji_shortcuts: bool,
    pub inline_emoji: bool,
    /// `emojiSet`, and where its images are served from.
    pub emoji_set: String,
    pub emoji_base_path: String,
    /// `default_code_lang`: the language of a fence that names none.
    pub default_code_lang: String,
    /// `i18n("post.heading_anchor")`
    pub heading_anchor_label: String,
    /// `allowedIframes`: url prefixes an iframe may load.
    pub allowed_iframes: Vec<String>,
    /// `allowedHrefSchemes`: schemes links may use besides http(s).
    pub allowed_href_schemes: Vec<String>,
    /// The checklist plugin's rule (`checklist_enabled`).
    pub checklist: bool,
    /// The footnote plugin (`enable_markdown_footnotes`).
    pub footnotes: bool,
    /// The poll plugin (`poll_enabled`), its option limit and the
    /// `poll.voters` label for no voters.
    pub poll: bool,
    pub poll_maximum_options: i64,
    pub poll_voters_label: String,
    /// The local dates plugin (`discourse_local_dates_enabled`).
    pub local_dates: bool,
    /// `discourse_local_dates_email_format` and `_email_timezone`: the
    /// `data-email-preview` of a date.
    pub local_dates_email_format: String,
    pub local_dates_email_timezone: String,
    /// The spoiler plugin's rules (`spoiler_enabled`).
    pub spoiler: bool,
    /// The policy plugin's `[policy]` (`policy_enabled`).
    pub policy: bool,
    /// `avatar_sizes`, ascending.
    pub avatar_sizes: Vec<i64>,
    /// `paths.baseUri`
    pub base_path: String,
    /// `limitedSiteSettings.secureUploads`
    pub secure_uploads: bool,
    /// `topicId`: a quote from another topic links to it.
    pub topic_id: Option<i64>,
    /// `forceQuoteLink`
    pub force_quote_link: bool,
    /// `postId`: prefixes heading anchors.
    pub post_id: Option<i64>,
}

impl MarkdownItExt for RenderSettings {}

impl RenderSettings {
    /// Compiles linkify from linkify_tlds, as the settings are received.
    pub fn compiled(mut self) -> Result<Self, Unsupported> {
        self.linkify = match &self.linkify_tlds {
            Some(tlds) => Some(
                LinkifyIt::for_tlds(tlds)
                    .map_err(|_| Unsupported("markdown_linkify_tlds that do not compile"))?,
            ),
            None => None,
        };
        Ok(self)
    }
}

/// What the enabled features put on the allow list (their `allowList`
/// calls), on top of the default one.
fn allow_list(settings: &RenderSettings) -> AllowList {
    let mut list = AllowList::new();
    list.iframes = settings.allowed_iframes.clone();
    list.href_schemes = settings.allowed_href_schemes.clone();
    anchor::allow(&mut list);
    bbcode::allow(&mut list, settings);
    code::allow(&mut list);
    quotes::allow(&mut list);
    table::allow(&mut list);
    text_post_process::allow(&mut list);
    uploads::allow(&mut list);
    if settings.poll {
        poll::allow(&mut list);
    }
    if settings.footnotes {
        footnotes::allow(&mut list);
    }
    if settings.checklist {
        checklist::allow(&mut list);
    }
    if settings.emoji {
        emoji::allow(&mut list);
    }
    if settings.local_dates {
        local_dates::allow(&mut list);
    }
    list
}

fn engine(settings: &RenderSettings, lookups: Lookups) -> MarkdownIt {
    let mut md = MarkdownIt::new();
    md.ext.insert(settings.clone());
    md.ext.insert(Context::with(lookups));
    markdown_it::plugins::cmark::add(&mut md);
    markdown_it::plugins::html::add(&mut md);
    html_img::add(&mut md);
    markdown_it::plugins::extra::strikethrough::add(&mut md);
    markdown_it::plugins::extra::tables::add(&mut md);
    newline::add(&mut md);
    bbcode::add(&mut md);
    checklist::add(&mut md);
    footnotes::add(&mut md);
    anchor::add(&mut md);
    link_pipes::add(&mut md);
    blocks::add(&mut md);
    tight::add(&mut md);
    linkify::add(&mut md);
    md
}

/// One pass of `cook(raw, options)` of discourse-markdown-it's engine:
/// parse, run the core rules in markdown-it's order, render, sanitize,
/// trim. Returns the HTML and what the pass wanted to look up.
///
/// The crate sorts its core rules by their constraints, which moves
/// unconstrained ones ahead of the inline parser as soon as another rule
/// is pinned before it. So only the parsers run in the crate's chain and
/// the passes over the parsed tree are called here, in order.
pub fn render(
    raw: &str,
    settings: &RenderSettings,
    lookups: Lookups,
) -> Result<(String, Needs), Unsupported> {
    if let Some(what) = table::refuse_glue(raw) {
        return Err(Unsupported(what));
    }
    let md = engine(settings, lookups);
    let ctx = md.ext.get::<Context>().expect("context");
    let raw = link_pipes::protect(&md, raw);
    let mut root: Node = md.parse(&raw);
    untrimmed::restore(&mut root);
    element::seal_inline_code(&mut root);
    footnotes::tail(&mut root);
    bbcode::pair(&mut root, settings);
    if let Some(linkify) = &settings.linkify {
        if let Some(what) = linkify::run(&mut root, linkify, &md) {
            return Err(Unsupported(what));
        }
        onebox::run(&mut root);
    }
    if settings.typographer {
        typographer::apply(&mut root);
        smartquotes::run(&mut root, &md);
    }
    // markdown-it's text_join, before the core rules Discourse adds.
    text_join::apply(&mut root);
    checklist::run(&mut root, settings);
    uploads::run(&mut root, settings, ctx);
    text_post_process::apply(&mut root, settings, ctx);
    emoji::run(&mut root, settings);
    anchor::apply(&mut root, settings);
    if settings.breaks {
        root.walk_mut(|node, _| {
            if node.is::<Softbreak>() {
                node.replace(Hardbreak);
            }
        });
    }
    code::apply(&mut root, settings);
    table::apply(&mut root);
    tight::apply(&mut root);
    let html = sanitize(&table::unglue(&root.render()), &allow_list(settings))
        .trim()
        .to_string();
    if let Some(what) = ctx.unsupported() {
        return Err(Unsupported(what));
    }
    Ok((html, ctx.needs()))
}
