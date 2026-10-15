//! preferences/navigation-menu.gjs: what the sidebar's topic list links
//! do with new and unread items.

use serde_json::{Map, Value};

use super::{Prefs, checkbox, save_attrs, save_controls};
use crate::topic_list_view::{escape, t};

pub fn render(p: &Prefs) -> (String, Map<String, Value>) {
    let cx = p.cx;
    let out = format!(
        "<div class=\"control-group preferences-navigation-menu-navigation\" data-setting-name=\"user-navigation-menu-navigation\"><legend class=\"control-label\">{}</legend><div class=\"controls\"><label>{}</label>{}{}</div></div>{}",
        escape(&t(cx, "user.experimental_sidebar.navigation_section")),
        escape(&t(
            cx,
            "user.experimental_sidebar.navigation_section_instruction"
        )),
        checkbox(
            "pref-link-to-filtered-list",
            None,
            p.option("sidebar_link_to_filtered_list") == true,
            &t(
                cx,
                "user.experimental_sidebar.link_to_filtered_list_checkbox_description"
            ),
            Some("sidebar_link_to_filtered_list")
        ),
        checkbox(
            "pref-show-count-new-items",
            None,
            p.option("sidebar_show_count_of_new_items") == true,
            &t(
                cx,
                "user.experimental_sidebar.show_count_new_items_checkbox_description"
            ),
            Some("sidebar_show_count_of_new_items")
        ),
        save_controls(cx),
    );
    let attrs = save_attrs(
        p.u,
        &[
            "sidebar_link_to_filtered_list",
            "sidebar_show_count_of_new_items",
        ],
        &[],
        &[],
    );
    (out, attrs)
}
