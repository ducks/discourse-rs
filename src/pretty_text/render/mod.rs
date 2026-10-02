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
mod code;
mod emoji;
mod linkify;
mod newline;
mod onebox;
mod table;
mod text_post_process;
mod typographer;

use std::sync::Arc;

use markdown_it::parser::core::CoreRule;
use markdown_it::parser::extset::MarkdownItExt;
use markdown_it::plugins::cmark::inline::newline::{Hardbreak, Softbreak};
use markdown_it::plugins::extra::smartquotes::SmartQuotesRule;
use markdown_it::{MarkdownIt, Node};

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
    code::allow(&mut list);
    table::allow(&mut list);
    if settings.emoji {
        emoji::allow(&mut list);
    }
    list
}

/// markdown-it's default quotes, which RenderSettings insists on.
type SmartQuotes = SmartQuotesRule<'‘', '’', '“', '”'>;

fn engine(settings: &RenderSettings) -> MarkdownIt {
    let mut md = MarkdownIt::new();
    md.ext.insert(settings.clone());
    markdown_it::plugins::cmark::add(&mut md);
    markdown_it::plugins::html::add(&mut md);
    markdown_it::plugins::extra::strikethrough::add(&mut md);
    markdown_it::plugins::extra::tables::add(&mut md);
    newline::add(&mut md);
    anchor::add(&mut md);
    md
}

/// `cook(raw, options)` of discourse-markdown-it's engine: parse, run the
/// core rules in markdown-it's order, render, sanitize, trim.
///
/// The crate sorts its core rules by their constraints, which moves
/// unconstrained ones ahead of the inline parser as soon as another rule
/// is pinned before it. So only the parsers run in the crate's chain and
/// the passes over the parsed tree are called here, in order.
pub fn render(raw: &str, settings: &RenderSettings) -> Result<String, CookError> {
    let md = engine(settings);
    let mut root: Node = md.parse(raw);
    if let Some(linkify) = &settings.linkify {
        if let Some(what) = linkify::run(&mut root, linkify, &md) {
            return Err(Unsupported(what).into());
        }
        onebox::run(&mut root);
    }
    if settings.typographer {
        typographer::apply(&mut root);
        SmartQuotes::run(&mut root, &md);
    }
    text_post_process::apply(&mut root, settings);
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
    Ok(sanitize(&root.render(), &allow_list(settings))
        .trim()
        .to_string())
}
