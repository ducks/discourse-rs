//! `Sanitize::CSS` (lib/sanitize/css.rb): CSS sanitized on a crass parse
//! tree and serialized from the tokens that remain.

use crate::crass::{self, AtRule, Node, Options, Property, TokenKind};

use super::config::CssConfig;
use super::protocol_allowed;

/// A `Sanitize::CSS` instance.
pub struct Css {
    config: CssConfig,
}

/// `image_function?`
fn image_function(name: &str) -> bool {
    matches!(name, "image" | "image-set" | "-webkit-image-set")
}

impl Css {
    pub fn new(config: CssConfig) -> Css {
        Css { config }
    }

    fn options(&self) -> Options {
        Options {
            preserve_comments: self.config.allow_comments,
            preserve_hacks: self.config.allow_hacks,
        }
    }

    /// `#properties`: inline style properties (a `style` attribute).
    pub fn properties(&self, css: &str) -> String {
        let tree = crass::parse_properties(css, self.options());
        crass::stringify(&self.tree(tree))
    }

    /// `#stylesheet`: a whole stylesheet.
    pub fn stylesheet(&self, css: &str) -> String {
        let tree = crass::parse(css, self.options());
        crass::stringify(&self.tree(tree))
    }

    /// `#tree!`: the nodes the config allows, sanitized. (Ruby leaves
    /// nils where nodes were removed; nothing reads them.)
    pub fn tree(&self, tree: Vec<Node>) -> Vec<Node> {
        let mut preceded_by_property = false;
        let mut out = Vec::new();
        for node in tree {
            match node {
                Node::AtRule(rule) => {
                    preceded_by_property = false;
                    out.extend(self.at_rule(rule));
                }
                Node::Token(t) if t.kind == TokenKind::Comment => {
                    if self.config.allow_comments {
                        out.push(Node::Token(t));
                    }
                }
                Node::Property(prop) => {
                    let prop = self.property(prop);
                    preceded_by_property = prop.is_some();
                    out.extend(prop);
                }
                Node::Token(t) if t.kind == TokenKind::Semicolon => {
                    // Only a semicolon after an allowed property stays, so
                    // removed ones leave no redundant semicolons.
                    if preceded_by_property {
                        preceded_by_property = false;
                        out.push(Node::Token(t));
                    }
                }
                Node::StyleRule(mut rule) => {
                    preceded_by_property = false;
                    rule.children = self.tree(std::mem::take(&mut rule.children));
                    out.push(Node::StyleRule(rule));
                }
                Node::Token(t) if t.kind == TokenKind::Whitespace => out.push(Node::Token(t)),
                _ => {}
            }
        }
        out
    }

    /// `at_rule!`
    fn at_rule(&self, mut rule: AtRule) -> Option<Node> {
        let name = rule.name.to_lowercase();
        if self.config.at_rules_with_styles.contains(&name) {
            let styles = crass::parse_rules(rule.block.take().unwrap_or_default());
            rule.block = Some(self.tree(styles));
        } else if self.config.at_rules_with_properties.contains(&name) {
            let props = crass::parse_properties_nodes(rule.block.take().unwrap_or_default());
            rule.block = Some(self.tree(props));
        } else if self.config.at_rules.contains(&name) {
            if name == "import" && !self.import_url_allowed(&rule) {
                return None;
            }
            if rule.block.is_some() {
                return None;
            }
        } else {
            return None;
        }
        Some(Node::AtRule(rule))
    }

    /// `import_url_allowed?`
    fn import_url_allowed(&self, rule: &AtRule) -> bool {
        let Some(validator) = &self.config.import_url_validator else {
            return true;
        };
        let url = rule.tokens.iter().find_map(|t| match t {
            Node::Token(t) if matches!(t.kind, TokenKind::Url | TokenKind::String) => {
                Some(t.value.as_str())
            }
            _ => None,
        });
        match url {
            Some(url) => validator(url),
            None => false,
        }
    }

    /// `property!`
    fn property(&self, prop: Property) -> Option<Node> {
        let mut name = prop.name.to_lowercase();
        // Preserve IE * and _ hacks if desired.
        if self.config.allow_hacks && (name.starts_with('*') || name.starts_with('_')) {
            name.remove(0);
        }
        if !self.config.properties.contains(&name) {
            return None;
        }

        let mut nodes = prop.children.clone();
        let mut combined_value = String::new();
        let mut i = 0;
        while i < nodes.len() {
            let child = nodes[i].clone();
            i += 1;
            match &child {
                Node::Token(t) if t.kind == TokenKind::Ident => {
                    combined_value.push_str(&t.value.to_lowercase());
                }
                Node::Function(f) => {
                    let name = f.name.to_lowercase();
                    if name == "url" && !self.valid_url(&child) {
                        return None;
                    }
                    if image_function(&name) && !self.valid_image(&child) {
                        return None;
                    }
                    combined_value.push_str(&name);
                    if name == "expression" || combined_value == "expression" {
                        return None;
                    }
                    nodes.extend(f.value.iter().cloned());
                }
                Node::Token(t) if t.kind == TokenKind::Function => {
                    let lowercase_value = t.value.to_lowercase();
                    combined_value.push_str(&lowercase_value);
                    if lowercase_value == "expression" || combined_value == "expression" {
                        return None;
                    }
                }
                Node::Token(t) if t.kind == TokenKind::Url => {
                    if !self.valid_url(&child) {
                        return None;
                    }
                }
                Node::Token(t) if t.kind == TokenKind::BadUrl => return None,
                _ => {}
            }
        }
        Some(Node::Property(prop))
    }

    /// `valid_url?`: a `url()` function or URL token whose URL uses an
    /// allowed protocol.
    fn valid_url(&self, node: &Node) -> bool {
        let url = match node {
            Node::Function(f) => {
                if f.name.to_lowercase() != "url" {
                    return false;
                }
                // At most one string, and otherwise only whitespace.
                let mut url_string = None;
                for token in &f.value {
                    match token {
                        Node::Token(t) if t.kind == TokenKind::String => {
                            if url_string.is_some() {
                                return false;
                            }
                            url_string = Some(t.value.as_str());
                        }
                        Node::Token(t) if t.kind == TokenKind::Whitespace => {}
                        _ => return false,
                    }
                }
                match url_string {
                    Some(url) => url,
                    None => return false,
                }
            }
            Node::Token(t) if t.kind == TokenKind::Url => t.value.as_str(),
            _ => return false,
        };
        protocol_allowed(url, &self.config.protocols)
    }

    /// `valid_image?`: an image function whose strings all use allowed
    /// protocols.
    fn valid_image(&self, node: &Node) -> bool {
        let Node::Function(f) = node else {
            return false;
        };
        if !image_function(&f.name.to_lowercase()) {
            return false;
        }
        f.value.iter().all(|token| match token {
            Node::Token(t) if t.kind == TokenKind::String => {
                protocol_allowed(&t.value, &self.config.protocols)
            }
            _ => true,
        })
    }
}
