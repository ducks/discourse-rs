//! features/table.js: a table sits in `<div class="md-table">`.
//!
//! Its `table_close` rule returns `</table>\n</div>` with no newline after
//! it, and markdown-it starts a block on a new line only after a hidden
//! token, so whatever follows a table follows `</div>` on the same line
//! (`</div><p>`, `</div></blockquote>`). This crate's blocks start with
//! `cr()`, which would add one: the wrapper ends with [`GLUE`], which `cr()`
//! does not take for a line end, and [`unglue`] removes it and the newline
//! put after it.

use markdown_it::plugins::extra::tables::Table;
use markdown_it::{Node, NodeValue, Renderer};

use crate::pretty_text::sanitizer::AllowList;

#[derive(Debug)]
struct MdTable;

impl NodeValue for MdTable {
    fn render(&self, node: &Node, fmt: &mut dyn Renderer) {
        fmt.cr();
        fmt.open("div", &[("class", "md-table".into())]);
        fmt.cr();
        fmt.contents(&node.children);
        fmt.cr();
        fmt.close("div");
        fmt.text_raw(GLUE);
    }
}

/// Marks where nothing may follow on a new line: U+FFFF, a noncharacter,
/// reserved for uses like this one. (Not a NUL: the crate turns every NUL
/// in its output into U+FFFD.) A post containing it is refused, see
/// [`refuse_glue`].
pub const GLUE: &str = "\u{FFFF}";

/// The rendered HTML without [`GLUE`] and the newline a block put after it.
pub fn unglue(html: &str) -> String {
    html.replace("\u{FFFF}\n", "").replace('\u{FFFF}', "")
}

/// A post that contains [`GLUE`] itself would lose it to [`unglue`].
pub fn refuse_glue(raw: &str) -> Option<&'static str> {
    raw.contains('\u{FFFF}')
        .then_some("U+FFFF in a post (the table renderer's marker)")
}

pub fn apply(root: &mut Node) {
    // Children first, so a wrapped table is not visited again.
    root.walk_post_mut(|node, _| {
        if node.is::<Table>() {
            let table = std::mem::take(node);
            let mut wrapper = Node::new(MdTable);
            wrapper.srcmap = table.srcmap;
            wrapper.children.push(table);
            *node = wrapper;
        }
    });
}

pub fn allow(list: &mut AllowList) {
    // The alignment markdown-it writes on cells, and nothing else.
    list.allow_custom(|tag, name, value| {
        (tag == "th" || tag == "td")
            && name == "style"
            && matches!(
                value,
                "text-align:right" | "text-align:left" | "text-align:center"
            )
    });
    list.allow(&[
        "table",
        "tbody",
        "thead",
        "tr",
        "th",
        "th[colspan]",
        "th[rowspan]",
        "td",
        "td[colspan]",
        "td[rowspan]",
        "div.md-table",
    ]);
}
