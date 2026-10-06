//! lib/stylesheet: the stylesheets the pages load. The color definitions
//! are computed from the site's color scheme as Rails compiles them; the
//! rest is ported from Discourse's SCSS by hand into static/css.

pub mod color_definitions;
pub mod sass_color;

#[cfg(test)]
mod tests {
    use super::color_definitions::{SchemeColors, css, properties};

    /// What Rails compiled for the base light scheme on the reference
    /// (scripts: fetched from /stylesheets/color_definitions_light-default_*).
    const RAILS_LIGHT: &str =
        include_str!("../../parity/stylesheets/color_definitions_light-default.css");

    /// The reference's Dark scheme (color_schemes 13, base -2).
    const RAILS_DARK: &str = include_str!("../../parity/stylesheets/color_definitions_dark-13.css");
    const DARK: &[(&str, &str)] = &[
        ("primary", "dddddd"),
        ("secondary", "222222"),
        ("tertiary", "099dd7"),
        ("quaternary", "c14924"),
        ("header_background", "111111"),
        ("header_primary", "dddddd"),
        ("highlight", "a87137"),
        ("selected", "052e3d"),
        ("hover", "313131"),
        ("danger", "e45735"),
        ("success", "1ca551"),
        ("love", "fa6c8d"),
    ];

    #[test]
    fn light_color_definitions_match_rails() {
        assert_matches(&css(&SchemeColors::from_hex(|_| None)), RAILS_LIGHT);
    }

    #[test]
    fn dark_color_definitions_match_rails() {
        let colors = SchemeColors::from_hex(|name| {
            DARK.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, hex)| hex.to_string())
        });
        assert_matches(&css(&colors), RAILS_DARK);
    }

    fn assert_matches(ours: &str, rails: &str) {
        let rails = properties(rails);
        let mut differ = Vec::new();
        for (name, value) in properties(ours) {
            // Rails' build rewrites light-dark() for older browsers.
            if name == "--topic-timeline-handle-color" {
                continue;
            }
            match rails.iter().find(|(n, _)| *n == name) {
                Some((_, theirs)) if *theirs == value => {}
                Some((_, theirs)) => differ.push(format!("{name}: ours {value}, rails {theirs}")),
                None => differ.push(format!("{name}: not in rails")),
            }
        }
        assert!(differ.is_empty(), "{}", differ.join("\n"));
    }
}
