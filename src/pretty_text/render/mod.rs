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
mod checklist;
mod code;
pub mod context;
mod element;
mod emoji;
mod footnotes;
mod html_img;
mod linkify;
mod newline;
mod onebox;
mod poll;
mod quotes;
mod smartquotes;
mod table;
mod text_join;
mod text_post_process;
mod typographer;
mod uploads;

use std::sync::Arc;

use markdown_it::parser::extset::MarkdownItExt;
use markdown_it::plugins::cmark::inline::newline::{Hardbreak, Softbreak};
use markdown_it::{MarkdownIt, Node};

use self::context::{Context, Lookups, Needs};
use self::linkify::LinkifyIt;
use super::CookError;
use super::sanitizer::{AllowList, sanitize};
use crate::Unsupported;
use crate::site_settings::SiteSettings;

/// What the rules read: `discourse.limitedSiteSettings`, the feature
/// switches, and the per-cook options.
#[derive(Debug, Clone)]
pub struct RenderSettings {
    /// `breaks`: a newline inside a paragraph is a `<br>`.
    pub breaks: bool,
    /// `linkify`, compiled with the site's `markdown_linkify_tlds`.
    linkify: Option<Arc<LinkifyIt>>,
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
    /// The local dates plugin (`discourse_local_dates_enabled`), which is
    /// not ported: its tags are refused.
    pub local_dates: bool,
    /// The spoiler plugin's rules (`spoiler_enabled`).
    pub spoiler: bool,
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
    pub fn from_site_settings(
        settings: &SiteSettings,
        i18n: &crate::i18n::I18n,
        config: &crate::config::Config,
    ) -> Result<Self, CookError> {
        let base_path = config.globals.relative_url_root();
        // The typographer's quotes are fixed in the crate's rule.
        if settings.get("markdown_typographer_quotation_marks")?.to_s() != "“|”|‘|’" {
            return Err(
                Unsupported("markdown_typographer_quotation_marks other than the default").into(),
            );
        }
        if settings.get("unicode_usernames")?.truthy() {
            return Err(Unsupported("unicode usernames in mentions").into());
        }
        // getURL on the emoji path: the base path in front, or the
        // external emoji host in its place.
        if config.globals.cdn_url().is_some() {
            return Err(Unsupported("emoji images behind a CDN").into());
        }
        let emoji_base_path = match settings.get("external_emoji_url")?.presence() {
            Some(external) => external,
            None => format!("{base_path}/images/emoji"),
        };
        let split = |name: &str| -> Result<Vec<String>, CookError> {
            Ok(settings
                .get(name)?
                .to_s()
                .split('|')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect())
        };
        let linkify = if settings.get("enable_markdown_linkify")?.truthy() {
            let tlds = split("markdown_linkify_tlds")?;
            Some(Arc::new(LinkifyIt::new(&tlds).map_err(|_| {
                Unsupported("markdown_linkify_tlds that do not compile")
            })?))
        } else {
            None
        };
        let mut avatar_sizes: Vec<i64> = split("avatar_sizes")?
            .iter()
            .map(|s| crate::ruby::to_i(s))
            .collect();
        avatar_sizes.sort_unstable();
        Ok(RenderSettings {
            breaks: !settings.get("traditional_markdown_linebreaks")?.truthy(),
            linkify,
            typographer: settings.get("enable_markdown_typographer")?.truthy(),
            mentions: settings.get("enable_mentions")?.truthy(),
            emoji: settings.get("enable_emoji")?.truthy(),
            emoji_shortcuts: settings.get("enable_emoji_shortcuts")?.truthy(),
            inline_emoji: settings.get("enable_inline_emoji_translation")?.truthy(),
            emoji_set: settings.get("emoji_set")?.to_s(),
            emoji_base_path,
            default_code_lang: settings.get("default_code_lang")?.to_s(),
            heading_anchor_label: i18n
                .t("js.post.heading_anchor")
                .unwrap_or("Heading link")
                .to_string(),
            // buildOptions keeps the iframe prefixes with a host and a path.
            allowed_iframes: split("allowed_iframes")?
                .into_iter()
                .filter(|s| s.matches('/').count() >= 3)
                .collect(),
            allowed_href_schemes: split("allowed_href_schemes")?,
            checklist: settings.get("checklist_enabled")?.truthy(),
            footnotes: settings.get("enable_markdown_footnotes")?.truthy(),
            poll: settings.get("poll_enabled")?.truthy(),
            poll_maximum_options: settings.get("poll_maximum_options")?.to_i(),
            poll_voters_label: i18n
                .t("js.poll.voters.other")
                .unwrap_or("voters")
                .to_string(),
            local_dates: settings.get("discourse_local_dates_enabled")?.truthy(),
            spoiler: settings.get("spoiler_enabled")?.truthy(),
            avatar_sizes,
            base_path: base_path.to_string(),
            secure_uploads: settings.get("secure_uploads")?.truthy(),
            topic_id: None,
            force_quote_link: false,
            post_id: None,
        })
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
    smartquotes::add(&mut md);
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
) -> Result<(String, Needs), CookError> {
    if let Some(what) = table::refuse_glue(raw) {
        return Err(Unsupported(what).into());
    }
    let md = engine(settings, lookups);
    let ctx = md.ext.get::<Context>().expect("context");
    let mut root: Node = md.parse(raw);
    element::seal_inline_code(&mut root);
    footnotes::tail(&mut root);
    bbcode::pair(&mut root, settings);
    if let Some(linkify) = &settings.linkify {
        if let Some(what) = linkify::run(&mut root, linkify, &md) {
            return Err(Unsupported(what).into());
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
    let html = sanitize(&table::unglue(&root.render()), &allow_list(settings))
        .trim()
        .to_string();
    if let Some(what) = ctx.unsupported() {
        return Err(Unsupported(what).into());
    }
    Ok((html, ctx.needs()))
}
