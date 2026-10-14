//! Discourse's `Onebox::SanitizeConfig` (lib/onebox/sanitize_config.rb),
//! what `Onebox::Preview#sanitize` runs every engine's HTML through.

use std::sync::Arc;

use regex::Regex;

use super::config::{AttrAllow, Config, Protocols};
use super::dom::{self, attr, children, remove_attr, set_attr};
use super::{Env, Transformer, set};

/// `Onebox::SanitizeConfig::ONEBOX`, with `allowed_iframe_regexes`
/// merged in as `Onebox::Preview#sanitize` does (None is Ruby's nil:
/// every iframe with a `src` allowed).
pub fn onebox_base_config(allowed_iframe_regexes: Option<Vec<Regex>>) -> Config {
    let mut config = Config::relaxed();
    config.elements.extend(set(&[
        "audio", "details", "embed", "iframe", "source", "video", "svg", "path", "use",
    ]));
    for (element, allow) in [
        (
            "a",
            AttrAllow::new(&["href", "hreflang", "name", "rel", "target"]),
        ),
        ("audio", AttrAllow::new(&["controls", "controlslist"])),
        ("embed", AttrAllow::new(&["height", "src", "type", "width"])),
        (
            "iframe",
            AttrAllow::new(&[
                "allowfullscreen",
                "frameborder",
                "height",
                "scrolling",
                "src",
                "width",
                "data-original-href",
                "data-unsanitized-src",
            ]),
        ),
        ("source", AttrAllow::new(&["src", "type"])),
        (
            "video",
            AttrAllow::new(&[
                "controls",
                "height",
                "loop",
                "width",
                "autoplay",
                "muted",
                "poster",
                "controlslist",
                "playsinline",
            ]),
        ),
        ("path", AttrAllow::new(&["d", "fill-rule"])),
        (
            "svg",
            AttrAllow::new(&["aria-hidden", "width", "height", "viewbox"]),
        ),
        ("div", AttrAllow::data()),
        ("span", AttrAllow::data()),
        ("use", AttrAllow::new(&["href"])),
    ] {
        config.attributes.insert(element.to_string(), allow);
    }
    config.add_attributes.insert(
        "iframe".to_string(),
        vec![
            ("seamless".to_string(), "seamless".to_string()),
            (
                "sandbox".to_string(),
                "allow-same-origin allow-scripts allow-forms allow-popups \
                 allow-popups-to-escape-sandbox allow-presentation"
                    .to_string(),
            ),
        ],
    );
    let http = Protocols::new(&["http", "https"]).relative();
    config.set_protocols("embed", "src", http.clone());
    config.set_protocols("iframe", "src", http.clone());
    config.set_protocols("source", "src", http);
    config.set_protocols("use", "href", Protocols::default().relative());
    if let Some(css) = &mut config.css {
        css.properties.insert("--aspect-ratio".to_string());
    }

    let regexes = Arc::new(allowed_iframe_regexes);
    let transformers: [Transformer; 3] = [
        Arc::new(links),
        Arc::new(move |env: &mut Env<'_>| iframes(env, regexes.as_ref().as_deref())),
        Arc::new(svg_children),
    ];
    config.transformers.extend(transformers);
    config
}

/// `Onebox::SanitizeConfig::DISCOURSE_ONEBOX` (ONEBOX plus `data-*` on
/// `aside`), with `allowed_iframe_regexes` merged in.
pub fn onebox_config(allowed_iframe_regexes: Option<Vec<Regex>>) -> Config {
    let mut config = onebox_base_config(allowed_iframe_regexes);
    config
        .attributes
        .insert("aside".to_string(), AttrAllow::data());
    config
}

/// Links get an `href`; absolute ones `rel="nofollow ugc noopener"`,
/// others lose their `target`.
fn links(env: &mut Env<'_>) {
    if env.node_name != "a" {
        return;
    }
    let a = &env.node;
    if attr(a, "href").is_none() {
        set_attr(a, "href", "#");
    }
    let href = attr(a, "href").unwrap_or_default();
    if is_absolute(&href) {
        set_attr(a, "rel", "nofollow ugc noopener");
    } else {
        remove_attr(a, "target");
    }
}

/// `%r{\A(?:[a-z]+:)?//}`
fn is_absolute(href: &str) -> bool {
    let scheme_len = href.bytes().take_while(u8::is_ascii_lowercase).count();
    let rest = if scheme_len > 0 && href.as_bytes().get(scheme_len) == Some(&b':') {
        &href[scheme_len + 1..]
    } else {
        href
    };
    rest.starts_with("//")
}

/// Iframes whose `src` no allowed regex matches lose it, keeping it in
/// `data-unsanitized-src`.
fn iframes(env: &mut Env<'_>, regexes: Option<&[Regex]>) {
    if env.node_name != "iframe" {
        return;
    }
    let iframe = &env.node;
    let src = attr(iframe, "src");
    let allowed = match (&src, regexes) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(src), Some(regexes)) => regexes.iter().any(|r| r.is_match(src)),
    };
    if !allowed {
        set_attr(iframe, "data-unsanitized-src", src.as_deref().unwrap_or(""));
        remove_attr(iframe, "src");
    }
}

/// Only `svg`, `path` and `use` elements stay inside an `svg`.
fn svg_children(env: &mut Env<'_>) {
    if env.node_name != "svg" {
        return;
    }
    // Nokogiri's `traverse`: children first, then the node.
    fn traverse(node: &markup5ever_rcdom::Handle) {
        for child in children(node) {
            traverse(&child);
        }
        let keep = match &node.data {
            markup5ever_rcdom::NodeData::Element { name, .. } => {
                matches!(&*name.local, "svg" | "path" | "use")
            }
            _ => false,
        };
        if !keep {
            dom::unlink(node);
        }
    }
    traverse(&env.node);
}
