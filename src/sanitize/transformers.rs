//! Sanitize's own transformers (lib/sanitize/transformers/*.rb).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::LazyLock;

use markup5ever_rcdom::NodeData;

use super::config::{AttrAllow, Config, RemoveContents};
use super::css::Css;
use super::dom::{
    self, attr, children, move_children_before, new_doctype, new_text, parent, remove_attr,
    set_attr, unlink,
};
use super::{Env, ProtocolMap, Step, protocol};

/// `REGEX_DATA_ATTR`:
/// `\Adata-(?!xml)[a-z_][\w.à-öø-ſǝ-ʯ-]*\z`
fn data_attr(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("data-") else {
        return false;
    };
    if rest.starts_with("xml") {
        return false;
    }
    let mut chars = rest.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(c, '_' | '.' | '-')
            || ('\u{e0}'..='\u{f6}').contains(&c)
            || ('\u{f8}'..='\u{17f}').contains(&c)
            || ('\u{1dd}'..='\u{2af}').contains(&c)
    })
}

/// `CleanElement`
struct CleanElement {
    add_attributes: HashMap<String, Vec<(String, String)>>,
    attributes: HashMap<String, AttrAllow>,
    attributes_all: Option<AttrAllow>,
    elements: HashSet<String>,
    protocols: ProtocolMap,
    remove_all_contents: bool,
    remove_element_contents: HashSet<String>,
    whitespace_elements: HashMap<String, (String, String)>,
}

pub(super) fn clean_element(config: &Config) -> Step {
    // Each element's attributes include those allowed on all elements.
    let attributes = config
        .attributes
        .iter()
        .map(|(element, allow)| {
            let mut allow = allow.clone();
            if let Some(all) = &config.attributes_all {
                allow.names.extend(all.names.iter().cloned());
                allow.data |= all.data;
            }
            (element.clone(), allow)
        })
        .collect();
    let (remove_all_contents, remove_element_contents) = match &config.remove_contents {
        RemoveContents::All(all) => (*all, HashSet::new()),
        RemoveContents::Elements(elements) => (false, elements.clone()),
    };
    let clean = CleanElement {
        add_attributes: config.add_attributes.clone(),
        attributes,
        attributes_all: config.attributes_all.clone(),
        elements: config.elements.clone(),
        protocols: config.protocols.clone(),
        remove_all_contents,
        remove_element_contents,
        whitespace_elements: config.whitespace_elements.clone(),
    };
    Box::new(move |env| clean.call(env))
}

/// Ruby's `\s` (ASCII whitespace, vertical tab included).
const S: &str = r"[\t\n\x0B\x0C\r ]";

static META_CHARSET: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    fancy_regex::Regex::new(&format!(";{S}*charset{S}*={S}*(?!utf-8)")).expect("valid regex")
});
static META_CHARSET_VALUE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(&format!(";{S}*charset{S}*=.+\\z")).expect("valid regex"));

impl CleanElement {
    fn call(&self, env: &mut Env<'_>) {
        let node = env.node.clone();
        if !dom::is_element(&node) || env.is_allowlisted {
            return;
        }
        let name = env.node_name.clone();

        // Delete any element that isn't allowlisted, unless the node has
        // already been taken out of the document.
        if !self.elements.contains(&name) && parent(&node).is_some() {
            // Elements like br, div, p are replaced with whitespace to keep
            // the text readable.
            if let Some((before, after)) = self.whitespace_elements.get(&name) {
                dom::add_previous_sibling(&node, new_text(before));
                if !node.children.borrow().is_empty() {
                    dom::add_next_sibling(&node, new_text(after));
                }
            }
            if !node.children.borrow().is_empty()
                && !(self.remove_all_contents || self.remove_element_contents.contains(&name))
            {
                move_children_before(&node);
            }
            unlink(&node);
            return;
        }

        let attr_allowlist = self.attributes.get(&name).or(self.attributes_all.as_ref());
        let NodeData::Element { attrs, .. } = &node.data else {
            return;
        };
        match attr_allowlist {
            None => attrs.borrow_mut().clear(),
            Some(allow) => {
                let snapshot = attrs.borrow().clone();
                for a in snapshot {
                    let attr_name = a.name.local.to_lowercase();
                    let remove = |attrs: &std::cell::RefCell<Vec<html5ever::Attribute>>| {
                        attrs.borrow_mut().retain(|x| x.name != a.name);
                    };
                    if !allow.names.contains(&attr_name)
                        && !(allow.data && attr_name.starts_with("data-") && data_attr(&attr_name))
                    {
                        remove(attrs);
                        continue;
                    }

                    let mut value = a.value.to_string();
                    let protocols = self.protocols.get(&name).and_then(|p| p.get(&attr_name));
                    if let Some(protocols) = protocols {
                        let allowed = match protocol(&value) {
                            Some(p) => protocols.schemes.contains(&p),
                            None => protocols.relative,
                        };
                        if !allowed {
                            remove(attrs);
                            continue;
                        }
                        // Whitespace around URLs is ignored when parsed;
                        // stripping it keeps it from being escaped below.
                        value = crate::ruby::strip(&value).to_string();
                    }

                    // libxml2's server-side-include workaround, kept by
                    // Sanitize for every serializer.
                    if matches!(attr_name.as_str(), "action" | "href" | "src")
                        || (name == "a" && attr_name == "name")
                    {
                        value = value.replace(' ', "%20").replace('"', "%22");
                    }

                    if value != *a.value
                        && let Some(x) = attrs.borrow_mut().iter_mut().find(|x| x.name == a.name)
                    {
                        x.value = value.into();
                    }
                }
            }
        }

        // Add required attributes.
        if let Some(add) = self.add_attributes.get(&name) {
            for (key, value) in add {
                set_attr(&node, key, value);
            }
        }

        match name.as_str() {
            // An allowlisted iframe's content is text that would be
            // serialized unescaped; it always goes.
            "iframe" => {
                for child in children(&node) {
                    unlink(&child);
                }
            }
            // Only a UTF-8 charset, since the output is UTF-8.
            "meta" => {
                if let Some(charset) = attr(&node, "charset")
                    && charset.to_lowercase() != "utf-8"
                {
                    set_attr(&node, "charset", "utf-8");
                }
                if let (Some(equiv), Some(content)) =
                    (attr(&node, "http-equiv"), attr(&node, "content"))
                    && equiv.to_lowercase() == "content-type"
                    && META_CHARSET
                        .is_match(&content.to_lowercase())
                        .unwrap_or(false)
                {
                    let replaced = META_CHARSET_VALUE.replace_all(&content, ";charset=utf-8");
                    set_attr(&node, "content", &replaced);
                }
            }
            // Nokogiri parses <noscript> with scripting disabled, unlike a
            // browser with scripting on; it can't be sanitized reliably.
            "noscript" => unlink(&node),
            _ => {}
        }
    }
}

/// `CleanComment`
pub(super) fn clean_comment(env: &mut Env<'_>) {
    if matches!(env.node.data, NodeData::Comment { .. }) && !env.is_allowlisted {
        unlink(&env.node);
    }
}

/// `CSS::CleanAttribute`: the CSS allowlist on `style` attributes.
pub(super) fn css_clean_attribute(scss: Rc<Css>) -> Step {
    Box::new(move |env| {
        let node = env.node.clone();
        if !dom::is_element(&node) || env.is_allowlisted {
            return;
        }
        let Some(style) = attr(&node, "style") else {
            return;
        };
        let css = scss.properties(&style);
        if crate::ruby::strip(&css).is_empty() {
            remove_attr(&node, "style");
        } else {
            set_attr(&node, "style", &css);
        }
    })
}

/// `CSS::CleanElement`: the CSS allowlist on `<style>` elements.
pub(super) fn css_clean_element(scss: Rc<Css>) -> Step {
    Box::new(move |env| {
        let node = env.node.clone();
        if !dom::is_element(&node) || env.node_name != "style" {
            return;
        }
        let css = scss.stylesheet(&dom::text(&node));
        if crate::ruby::strip(&css).is_empty() {
            unlink(&node);
        } else {
            let css = css.replace("</", "<\\/");
            for child in children(&node) {
                unlink(&child);
            }
            dom::append(&node, new_text(&css));
        }
    })
}

/// `CleanDoctype`
pub(super) fn clean_doctype(env: &mut Env<'_>) {
    if env.is_allowlisted {
        return;
    }
    let node = env.node.clone();
    let NodeData::Doctype { name, .. } = &node.data else {
        return;
    };
    if env.config.allow_doctype {
        if &**name != "html" {
            let document = parent(&node);
            unlink(&node);
            if let Some(document) = document {
                dom::prepend(&document, new_doctype("html"));
            }
        }
    } else {
        unlink(&node);
    }
}

#[cfg(test)]
mod tests {
    use super::data_attr;

    #[test]
    fn data_attribute_names() {
        assert!(data_attr("data-foo"));
        assert!(data_attr("data-foo.bar_baz-1"));
        assert!(!data_attr("data-"));
        assert!(!data_attr("data-xml"));
        assert!(!data_attr("data-xmlfoo"));
        assert!(!data_attr("data-f:oo"));
        assert!(!data_attr("data-éfoo"));
        assert!(data_attr("data-fé"));
        assert!(!data_attr("data-1"));
    }
}
