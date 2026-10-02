//! features/table.js: a table sits in `<div class="md-table">`.

use markdown_it::plugins::extra::tables::Table;
use markdown_it::{Node, NodeValue, Renderer};

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
        fmt.cr();
    }
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
