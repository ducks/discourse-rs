use comrak::{markdown_to_html, Options};

/// Render raw markdown to cooked HTML
pub fn render(raw: &str) -> String {
    let mut options = Options::default();

    // GitHub Flavored Markdown extensions
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.extension.superscript = true;

    // Parsing options
    options.parse.smart = true;

    // Render options
    options.render.unsafe_ = false; // Sanitize HTML
    options.render.escape = true;

    markdown_to_html(raw, &options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_markdown() {
        let raw = "Hello **world**!";
        let cooked = render(raw);
        assert!(cooked.contains("<strong>world</strong>"));
    }

    #[test]
    fn test_code_block() {
        let raw = "```rust\nfn main() {}\n```";
        let cooked = render(raw);
        assert!(cooked.contains("<code"));
    }

    #[test]
    fn test_strikethrough() {
        let raw = "~~deleted~~";
        let cooked = render(raw);
        assert!(cooked.contains("<del>deleted</del>"));
    }

    #[test]
    fn test_autolink() {
        let raw = "Check out https://example.com";
        let cooked = render(raw);
        assert!(cooked.contains("<a href=\"https://example.com\""));
    }

    #[test]
    fn test_table() {
        let raw = "| A | B |\n|---|---|\n| 1 | 2 |";
        let cooked = render(raw);
        assert!(cooked.contains("<table>"));
    }

    #[test]
    fn test_xss_prevention() {
        let raw = "<script>alert('xss')</script>";
        let cooked = render(raw);
        assert!(!cooked.contains("<script>"));
    }
}
