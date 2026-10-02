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
mod table;
mod text_post_process;
mod typographer;

use markdown_it::parser::core::CoreRule;
use markdown_it::parser::extset::MarkdownItExt;
use markdown_it::plugins::cmark::inline::newline::{Hardbreak, Softbreak};
use markdown_it::plugins::extra::smartquotes::SmartQuotesRule;
use markdown_it::{MarkdownIt, Node};

use super::CookError;
use crate::Unsupported;
use crate::site_settings::SiteSettings;

/// What the rules read: `discourse.limitedSiteSettings`, the feature
/// switches, and the per-cook options.
#[derive(Debug, Clone)]
pub struct RenderSettings {
    /// `breaks`: a newline inside a paragraph is a `<br>`.
    pub breaks: bool,
    pub linkify: bool,
    pub typographer: bool,
    /// `features.mentions`
    pub mentions: bool,
    /// `default_code_lang`: the language of a fence that names none.
    pub default_code_lang: String,
    /// `i18n("post.heading_anchor")`
    pub heading_anchor_label: String,
    /// `postId`: prefixes heading anchors.
    pub post_id: Option<i64>,
}

impl MarkdownItExt for RenderSettings {}

impl RenderSettings {
    pub fn from_site_settings(
        settings: &SiteSettings,
        i18n: &crate::i18n::I18n,
    ) -> Result<Self, CookError> {
        // The typographer's quotes are fixed in the crate's rule.
        if settings.get("markdown_typographer_quotation_marks")?.to_s() != "“|”|‘|’" {
            return Err(
                Unsupported("markdown_typographer_quotation_marks other than the default").into(),
            );
        }
        if settings.get("unicode_usernames")?.truthy() {
            return Err(Unsupported("unicode usernames in mentions").into());
        }
        Ok(RenderSettings {
            breaks: !settings.get("traditional_markdown_linebreaks")?.truthy(),
            linkify: settings.get("enable_markdown_linkify")?.truthy(),
            typographer: settings.get("enable_markdown_typographer")?.truthy(),
            mentions: settings.get("enable_mentions")?.truthy(),
            default_code_lang: settings.get("default_code_lang")?.to_s(),
            heading_anchor_label: i18n
                .t("js.post.heading_anchor")
                .unwrap_or("Heading link")
                .to_string(),
            post_id: None,
        })
    }
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
    anchor::add(&mut md);
    md
}

/// `cook(raw, options)` of discourse-markdown-it's engine: parse, run the
/// core rules Discourse adds in markdown-it's order, render, trim.
///
/// The crate sorts its core rules by their constraints, which moves
/// unconstrained ones ahead of the inline parser as soon as another rule
/// is pinned before it. So only the parsers run in the crate's chain and
/// the passes over the parsed tree are called here, in order.
pub fn render(raw: &str, settings: &RenderSettings) -> String {
    let md = engine(settings);
    let mut root: Node = md.parse(raw);
    if settings.typographer {
        typographer::apply(&mut root);
        SmartQuotes::run(&mut root, &md);
    }
    text_post_process::apply(&mut root, settings);
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
    root.render().trim().to_string()
}
