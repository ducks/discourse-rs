//! Port of the `sanitize` gem (7.0.0): an allowlist HTML and CSS
//! sanitizer over an HTML5 parse (Nokogiri::HTML5 there, html5ever
//! here), serialized as Nokogiri's HTML5 serializer does.
//!
//! [`fragment`] and [`document`] are `Sanitize.fragment` and
//! `Sanitize.document`; [`Config`] mirrors Sanitize's config hash, with
//! [`config`] holding DEFAULT, RESTRICTED, BASIC and RELAXED; [`css`] is
//! `Sanitize::CSS`; [`onebox_config`] is Discourse's
//! `Onebox::SanitizeConfig::DISCOURSE_ONEBOX`.

pub mod config;
pub mod css;
mod dom;
mod onebox;
mod relaxed_properties;
mod transformers;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use markup5ever_rcdom::{Handle, NodeData};

pub use config::{AttrAllow, Config, CssConfig, ParserOptions, Protocols, RemoveContents};
pub use dom::{
    add_next_sibling, add_previous_sibling, attr, children, first_child, has_attr, is_element,
    new_element, new_text, next_sibling, parent, previous_sibling, remove_attr, rename, set_attr,
    unlink,
};
pub use onebox::{onebox_base_config, onebox_config};

/// A transformer (`config[:transformers]`): called for every node, in
/// traversal order, before Sanitize's own transformers. It may change
/// the node, replace `env.node` (as renaming an element does), and add
/// nodes to `env.node_allowlist`.
pub type Transformer = Arc<dyn Fn(&mut Env<'_>) + Send + Sync>;

/// `Sanitize::Error` and the parser's `ArgumentError`s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// `When sanitizing a document, "<html>" must be allowlisted.`
    HtmlNotAllowlisted,
    /// Gumbo's `max_tree_depth`: "Document tree depth limit exceeded".
    TreeTooDeep,
    /// Gumbo's `max_attributes`: "Attributes per element limit exceeded".
    TooManyAttributes,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::HtmlNotAllowlisted => {
                "When sanitizing a document, \"<html>\" must be allowlisted."
            }
            Error::TreeTooDeep => "Document tree depth limit exceeded",
            Error::TooManyAttributes => "Attributes per element limit exceeded",
        })
    }
}

impl std::error::Error for Error {}

/// The nodes transformers have allowlisted (`env[:node_allowlist]`).
#[derive(Default)]
pub struct NodeAllowlist {
    // The handles keep the nodes alive so a pointer is never reused.
    nodes: Vec<Handle>,
    ptrs: HashSet<*const markup5ever_rcdom::Node>,
}

impl NodeAllowlist {
    pub fn insert(&mut self, node: &Handle) {
        if self.ptrs.insert(Rc::as_ptr(node)) {
            self.nodes.push(node.clone());
        }
    }

    pub fn contains(&self, node: &Handle) -> bool {
        self.ptrs.contains(&Rc::as_ptr(node))
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// What a transformer is called with (Sanitize's env hash).
pub struct Env<'a> {
    pub config: &'a Config,
    pub is_allowlisted: bool,
    pub node: Handle,
    /// `node.name.downcase`: the element's name, `text`, `comment`,
    /// `document`, `#document-fragment`, or a doctype's name.
    pub node_name: String,
    pub node_allowlist: &'a mut NodeAllowlist,
}

type Step = Box<dyn Fn(&mut Env<'_>)>;

/// A `Sanitize` instance: the config and its transformer chain.
pub struct Sanitizer<'a> {
    config: &'a Config,
    transformers: Vec<Step>,
}

impl<'a> Sanitizer<'a> {
    pub fn new(config: &'a Config) -> Sanitizer<'a> {
        let mut transformers: Vec<Step> = Vec::new();
        for t in &config.transformers {
            let t = t.clone();
            transformers.push(Box::new(move |env| t(env)));
        }

        // Sanitize's own transformers run after any custom ones.
        transformers.push(transformers::clean_element(config));
        if !config.allow_comments {
            transformers.push(Box::new(transformers::clean_comment));
        }
        let scss = || Rc::new(css::Css::new(config.effective_css()));
        let mut shared_css = None;
        if config.elements.contains("style") {
            let scss = shared_css.get_or_insert_with(scss).clone();
            transformers.push(transformers::css_clean_element(scss));
        }
        let style_allowed = config
            .attributes
            .values()
            .chain(config.attributes_all.iter())
            .any(|a| a.names.contains("style"));
        if style_allowed {
            let scss = shared_css.get_or_insert_with(scss).clone();
            transformers.push(transformers::css_clean_attribute(scss));
        }
        transformers.push(Box::new(transformers::clean_doctype));
        // CleanCDATA: html5ever gives CDATA sections as text already,
        // which is what Sanitize turns them into.
        Sanitizer {
            config,
            transformers,
        }
    }

    /// `Sanitize#fragment`
    pub fn fragment(&self, html: &str) -> Result<String, Error> {
        let frag = dom::parse_fragment(&preprocess(html), &self.config.parser_options)?;
        self.run(&frag, Some(&frag));
        Ok(dom::serialize(&frag))
    }

    /// `Sanitize#document`
    pub fn document(&self, html: &str) -> Result<String, Error> {
        let doc = dom::parse_document(&preprocess(html), &self.config.parser_options)?;
        self.node(&doc)?;
        Ok(dom::serialize(&doc))
    }

    /// `Sanitize#node!`: sanitizes a node and its descendants in place. A
    /// document needs `html` allowlisted.
    pub fn node(&self, node: &Handle) -> Result<(), Error> {
        if matches!(node.data, NodeData::Document) && !self.config.elements.contains("html") {
            return Err(Error::HtmlNotAllowlisted);
        }
        self.run(node, None);
        Ok(())
    }

    fn run(&self, node: &Handle, fragment: Option<&Handle>) {
        let mut allowlist = NodeAllowlist::default();
        self.traverse(node.clone(), fragment, &mut allowlist);
    }

    /// `traverse`: the node, then each child in order; when a child was
    /// unlinked or replaced, the walk goes on from the sibling before
    /// it (or the first child).
    fn traverse(
        &self,
        node: Handle,
        fragment: Option<&Handle>,
        allowlist: &mut NodeAllowlist,
    ) -> Handle {
        let node = self.transform_node(node, fragment, allowlist);
        let mut child = first_child(&node);
        while let Some(c) = child {
            let prev = previous_sibling(&c);
            let c = self.traverse(c, fragment, allowlist);
            let still_child = parent(&c).is_some_and(|p| Rc::ptr_eq(&p, &node));
            child = if still_child {
                next_sibling(&c)
            } else {
                match prev {
                    Some(p) => next_sibling(&p),
                    None => first_child(&node),
                }
            };
        }
        node
    }

    fn transform_node(
        &self,
        node: Handle,
        fragment: Option<&Handle>,
        allowlist: &mut NodeAllowlist,
    ) -> Handle {
        let mut env = Env {
            config: self.config,
            is_allowlisted: false,
            node_name: String::new(),
            node,
            node_allowlist: allowlist,
        };
        for transformer in &self.transformers {
            env.is_allowlisted = env.node_allowlist.contains(&env.node);
            env.node_name = node_name(&env.node, fragment);
            transformer(&mut env);
        }
        env.node
    }
}

fn node_name(node: &Handle, fragment: Option<&Handle>) -> String {
    match &node.data {
        NodeData::Document => {
            if fragment.is_some_and(|f| Rc::ptr_eq(f, node)) {
                "#document-fragment".into()
            } else {
                "document".into()
            }
        }
        NodeData::Doctype { name, .. } => name.to_lowercase(),
        NodeData::Text { .. } => "text".into(),
        NodeData::Comment { .. } => "comment".into(),
        NodeData::Element { name, .. } => name.local.to_lowercase(),
        NodeData::ProcessingInstruction { target, .. } => target.to_lowercase(),
    }
}

/// `Sanitize.fragment(html, config)`
pub fn fragment(html: &str, config: &Config) -> Result<String, Error> {
    Sanitizer::new(config).fragment(html)
}

/// `Sanitize.document(html, config)`
pub fn document(html: &str, config: &Config) -> Result<String, Error> {
    Sanitizer::new(config).document(html)
}

/// `Sanitize.node!(node, config)`
pub fn node(node: &Handle, config: &Config) -> Result<(), Error> {
    Sanitizer::new(config).node(node)
}

/// HTML control characters (`REGEX_HTML_CONTROL_CHARACTERS`) and
/// non-characters (`REGEX_HTML_NON_CHARACTERS`).
fn unsuitable(c: char) -> bool {
    let n = c as u32;
    matches!(n, 0x1..=0x8 | 0xb | 0xe..=0x1f | 0x7f..=0x9f | 0xfdd0..=0xfdef)
        || (n & 0xfffe) == 0xfffe
}

/// `preprocess`: `REGEX_UNSUITABLE_CHARS` removed.
pub fn preprocess(html: &str) -> String {
    html.chars().filter(|&c| !unsuitable(c)).collect()
}

/// `REGEX_PROTOCOL`'s capture: the text before a colon (or an encoded
/// one) that isn't preceded by `/` or `#`, as a browser would read a
/// protocol.
pub(crate) fn protocol(value: &str) -> Option<String> {
    use std::sync::LazyLock;
    static RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?i)\A[\t\n\x0B\x0C\r ]*([^/#]*?)(?::|&#0*58|&#x0*3a)")
            .expect("valid regex")
    });
    RE.captures(value)
        .map(|c| c.get(1).map_or("", |m| m.as_str()).to_lowercase())
}

/// Whether `url` uses one of the protocols (`:relative` meaning none).
pub(crate) fn protocol_allowed(url: &str, protocols: &Protocols) -> bool {
    match protocol(url) {
        Some(p) => protocols.schemes.contains(&p),
        None => protocols.relative,
    }
}

/// A list of strings as a set.
pub(crate) fn set(items: &[&str]) -> HashSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// `{element => {attribute => protocols}}`
pub type ProtocolMap = HashMap<String, HashMap<String, Protocols>>;

#[cfg(test)]
mod tests;
