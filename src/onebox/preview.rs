//! lib/onebox/preview.rb: an engine's HTML as Onebox.preview gives it,
//! widths capped (`process_html`) and then sanitized.

use crate::pretty_text::cleanup::{all_elements, attr, parse, set_attr, to_html};

/// `max_width`, as Oneboxer.compute_external_onebox asks.
const MAX_WIDTH: i64 = 695;

/// `Preview#process_html`: anything wider than the onebox scaled down to
/// it, the HTML reserialized by the HTML5 parser either way.
pub fn process_html(html: &str) -> String {
    let dom = parse(html);
    for element in all_elements(&dom) {
        let Some(width) = attr(&element, "width") else {
            continue;
        };
        let width = crate::ruby::to_i(&width);
        if width > MAX_WIDTH {
            let height = attr(&element, "height").map_or(0, |h| crate::ruby::to_i(&h));
            if height > 0 {
                let ratio = height as f64 / width as f64;
                set_attr(
                    &element,
                    "height",
                    &((MAX_WIDTH as f64 * ratio).floor() as i64).to_string(),
                );
            }
            set_attr(&element, "width", &MAX_WIDTH.to_string());
        }
    }
    to_html(&dom)
}

/// `Preview#to_s` / `#placeholder_html` after the engine: processed, then
/// sanitized with DISCOURSE_ONEBOX and the iframe allowlist.
pub fn finish(html: &str, allowed_iframes: &[regex::Regex]) -> Result<String, crate::Unsupported> {
    let config = crate::sanitize::onebox_config(Some(allowed_iframes.to_vec()));
    crate::sanitize::fragment(&process_html(html), &config)
        .map_err(|_| crate::Unsupported("onebox HTML past Gumbo's limits"))
}
