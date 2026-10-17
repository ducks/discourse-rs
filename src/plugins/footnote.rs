//! footnote (plugins/footnote/plugin.rb): its server side. The markup is
//! the markdown renderer's (pretty_text::render::footnotes), the inline
//! tooltips are static/js/inline-footnotes.js. Here,
//! `before_post_process_cooked`: the footnote ids the renderer numbers
//! from 1 in every post made unique to the post, so two footnoted posts
//! on one page do not share `#fn1`.

use markup5ever_rcdom::{Handle, RcDom};

use crate::pretty_text::cleanup::{attr, element_name, fragment_root, has_class, set_attr};
use crate::site_settings::{SettingError, SiteSettings};

/// `enabled_site_setting :enable_markdown_footnotes`: the plugin's event
/// handlers run only while it is on.
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("enable_markdown_footnotes")?.truthy())
}

/// Ruby's `str[from..-1].to_i`: the characters from `from` on (none past
/// the end, where Ruby's slice is nil or empty and either reads as 0),
/// read as String#to_i does: leading whitespace, an optional sign, then
/// digits with single underscores between them; anything else ends it.
fn ruby_tail_to_i(s: &str, from: usize) -> i64 {
    let tail: String = s.chars().skip(from).collect();
    let rest = tail.trim_start_matches([' ', '\t', '\n', '\u{b}', '\u{c}', '\r']);
    let (negative, digits) = match rest.as_bytes().first() {
        Some(b'-') => (true, &rest[1..]),
        Some(b'+') => (false, &rest[1..]),
        _ => (false, rest),
    };
    let mut value: i64 = 0;
    let mut last_was_digit = false;
    let bytes = digits.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'0'..=b'9' => {
                value = value.saturating_mul(10).saturating_add(i64::from(b - b'0'));
                last_was_digit = true;
            }
            b'_' if last_was_digit && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) => {
                last_was_digit = false;
            }
            _ => break,
        }
    }
    if negative { -value } else { value }
}

/// The elements under the fragment in document order, each with whether
/// it has a `sup.footnote-ref` ancestor.
fn elements(dom: &RcDom) -> Vec<(Handle, bool)> {
    fn walk(node: &Handle, in_ref: bool, out: &mut Vec<(Handle, bool)>) {
        for child in node.children.borrow().iter() {
            let Some(name) = element_name(child) else {
                continue;
            };
            out.push((child.clone(), in_ref));
            let is_ref = name == "sup" && has_class(child, "footnote-ref");
            walk(child, in_ref || is_ref, out);
        }
    }
    let mut out = Vec::new();
    walk(&fragment_root(dom), false, &mut out);
    out
}

/// `on(:before_post_process_cooked)`: in the post's cooked document, the
/// back references point at `#footnote-ref-<post>-<n>`, the references
/// at `#footnote-<post>-<n>` with the id `footnote-ref-<post>-<n>`, and
/// the list items take the id `footnote-<post>-<n>`, `n` read from the
/// old value as the plugin slices it (`fnref1:1` reads as 1). The three
/// passes run in the plugin's order.
pub fn before_post_process_cooked(dom: &RcDom, post_id: i32) {
    let elements = elements(dom);
    for (e, _) in &elements {
        if element_name(e) == Some("a") && has_class(e, "footnote-backref") {
            let id = ruby_tail_to_i(&attr(e, "href").unwrap_or_default(), 6);
            set_attr(e, "href", &format!("#footnote-ref-{post_id}-{id}"));
        }
    }
    for (e, in_ref) in &elements {
        if *in_ref && element_name(e) == Some("a") {
            let id = ruby_tail_to_i(&attr(e, "href").unwrap_or_default(), 3);
            set_attr(e, "href", &format!("#footnote-{post_id}-{id}"));
            let id = ruby_tail_to_i(&attr(e, "id").unwrap_or_default(), 5);
            set_attr(e, "id", &format!("footnote-ref-{post_id}-{id}"));
        }
    }
    for (e, _) in &elements {
        if element_name(e) == Some("li") && has_class(e, "footnote-item") {
            let id = ruby_tail_to_i(&attr(e, "id").unwrap_or_default(), 2);
            set_attr(e, "id", &format!("footnote-{post_id}-{id}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pretty_text::cleanup::{parse, to_html};

    #[test]
    fn ruby_slices_read_as_to_i() {
        assert_eq!(ruby_tail_to_i("#fnref1", 6), 1);
        assert_eq!(ruby_tail_to_i("#fnref12:3", 6), 12);
        assert_eq!(ruby_tail_to_i("#fn", 3), 0);
        assert_eq!(ruby_tail_to_i("", 5), 0);
        assert_eq!(ruby_tail_to_i("fnrefx", 5), 0);
        assert_eq!(ruby_tail_to_i("fn 1_0", 2), 10);
        assert_eq!(ruby_tail_to_i("fn-4", 2), -4);
        assert_eq!(ruby_tail_to_i("fn1__0", 2), 1);
        assert_eq!(ruby_tail_to_i("éé#fn7", 5), 7);
    }

    #[test]
    fn ids_become_the_posts() {
        let dom = parse(
            "<p>a<sup class=\"footnote-ref\"><a href=\"#fn1\" id=\"fnref1\">[1]</a></sup>\
             b<sup class=\"footnote-ref\"><a href=\"#fn1\" id=\"fnref1:1\">[1:1]</a></sup></p>\
             <hr class=\"footnotes-sep\">\n<ol class=\"footnotes-list\">\
             <li id=\"fn1\" class=\"footnote-item\"><p>x <a href=\"#fnref1\" class=\"footnote-backref\">\u{21a9}</a>\
             <a href=\"#fnref1:1\" class=\"footnote-backref\">\u{21a9}</a></p></li></ol>",
        );
        before_post_process_cooked(&dom, 42);
        assert_eq!(
            to_html(&dom),
            "<p>a<sup class=\"footnote-ref\"><a href=\"#footnote-42-1\" id=\"footnote-ref-42-1\">[1]</a></sup>\
             b<sup class=\"footnote-ref\"><a href=\"#footnote-42-1\" id=\"footnote-ref-42-1\">[1:1]</a></sup></p>\
             <hr class=\"footnotes-sep\">\n<ol class=\"footnotes-list\">\
             <li id=\"footnote-42-1\" class=\"footnote-item\"><p>x <a href=\"#footnote-ref-42-1\" class=\"footnote-backref\">\u{21a9}</a>\
             <a href=\"#footnote-ref-42-1\" class=\"footnote-backref\">\u{21a9}</a></p></li></ol>"
        );
    }

    #[test]
    fn a_reference_without_an_id_gets_one() {
        let dom = parse("<sup class=\"footnote-ref\"><span><a>x</a></span></sup>");
        before_post_process_cooked(&dom, 7);
        assert_eq!(
            to_html(&dom),
            "<sup class=\"footnote-ref\"><span><a href=\"#footnote-7-0\" id=\"footnote-ref-7-0\">x</a></span></sup>"
        );
    }
}
