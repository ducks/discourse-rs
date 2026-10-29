//! The styleguide plugin (plugins/styleguide): /styleguide and
//! /styleguide/:category/:section for the members of
//! `styleguide_allowed_groups` while `styleguide_enabled` is on
//! (`Guardian#can_see_styleguide?`), and `can_see_styleguide` on /site.json.
//!
//! Rails serves an empty page there and the Ember client draws its own
//! components into it; this port has no Ember components, so the pages
//! (view.rs) show the port's own markup under the same sections. The
//! sections lib/styleguide.js lists, in its order.

pub mod view;

use crate::guardian::Guardian;
use crate::site_settings::{SettingError, SiteSettings};

/// `CATEGORIES`
pub const CATEGORIES: [&str; 4] = ["syntax", "atoms", "molecules", "organisms"];

/// One of lib/styleguide.js's `SECTIONS`.
#[derive(Debug, PartialEq)]
pub struct Section {
    pub category: &'static str,
    pub id: &'static str,
    /// `priority`, 100 when the section names none.
    pub priority: i32,
}

const fn s(category: &'static str, id: &'static str, priority: i32) -> Section {
    Section {
        category,
        id,
        priority,
    }
}

const SECTIONS: &[Section] = &[
    s("syntax", "bem", 0),
    s("atoms", "typography", 0),
    s("atoms", "font-scale", 1),
    s("atoms", "buttons", 2),
    s("atoms", "colors", 3),
    s("atoms", "icons", 4),
    s("atoms", "forms", 5),
    s("atoms", "spinners", 6),
    s("atoms", "otp", 7),
    s("atoms", "date-time-inputs", 100),
    s("atoms", "dropdowns", 100),
    s("atoms", "topic-link", 100),
    s("atoms", "shortcut", 100),
    s("atoms", "segmented-control", 100),
    s("molecules", "bread-crumbs", 100),
    s("molecules", "categories", 100),
    s("molecules", "char-counter", 100),
    s("molecules", "combo-button", 100),
    s("molecules", "empty-state", 100),
    s("molecules", "navigation-bar", 100),
    s("molecules", "navigation-stacked", 100),
    s("molecules", "post-menu", 100),
    s("molecules", "roving-focus", 100),
    s("molecules", "tooltips", 100),
    s("molecules", "menus", 100),
    s("molecules", "context-menu", 100),
    s("molecules", "multi-select", 100),
    s("molecules", "toasts", 100),
    s("molecules", "dialog", 100),
    s("molecules", "drag-and-drop", 100),
    s("molecules", "signup-cta", 100),
    s("molecules", "topic-list-item", 100),
    s("molecules", "topic-notifications", 100),
    s("molecules", "topic-timer-info", 100),
    s("molecules", "virtual-list", 100),
    s("molecules", "tabs", 100),
    s("molecules", "overflow-controls", 100),
    s("organisms", "post", 0),
    s("organisms", "post-list", 1),
    s("organisms", "post-oneboxes", 2),
    s("organisms", "topic-map", 3),
    s("organisms", "topic-footer-buttons", 4),
    s("organisms", "topic-list", 5),
    s("organisms", "basic-topic-list", 100),
    s("organisms", "categories-list", 100),
    s("organisms", "docked-composer", 100),
    s("organisms", "modal", 100),
    s("organisms", "navigation", 100),
    s("organisms", "site-header", 100),
    s("organisms", "more-topics", 100),
];

/// `allCategories()`: each category with its sections by priority, then
/// id.
pub fn all_categories() -> Vec<(&'static str, Vec<&'static Section>)> {
    CATEGORIES
        .iter()
        .map(|category| {
            let mut sections: Vec<&Section> = SECTIONS
                .iter()
                .filter(|s| s.category == *category)
                .collect();
            sections.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.id.cmp(b.id)));
            (*category, sections)
        })
        .filter(|(_, sections)| !sections.is_empty())
        .collect()
}

/// `sectionById(id)`: by id alone, whatever category the path names.
pub fn section_by_id(id: &str) -> Option<&'static Section> {
    SECTIONS.iter().find(|s| s.id == id)
}

/// `enabled_site_setting :styleguide_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("styleguide_enabled")?.truthy())
}

/// `Guardian#can_see_styleguide?`
pub fn can_see(guardian: &Guardian, settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(enabled(settings)? && guardian.in_setting_groups(settings, "styleguide_allowed_groups")?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_sort_by_priority_then_id() {
        let all = all_categories();
        let ids = |category: &str| -> Vec<&str> {
            all.iter()
                .find(|(c, _)| *c == category)
                .unwrap()
                .1
                .iter()
                .map(|s| s.id)
                .collect()
        };
        assert_eq!(
            all.iter().map(|(c, _)| *c).collect::<Vec<_>>(),
            CATEGORIES.to_vec()
        );
        assert_eq!(
            ids("atoms")[..10],
            [
                "typography",
                "font-scale",
                "buttons",
                "colors",
                "icons",
                "forms",
                "spinners",
                "otp",
                "date-time-inputs",
                "dropdowns",
            ]
        );
        assert_eq!(ids("molecules")[0], "bread-crumbs");
        assert_eq!(ids("molecules")[1], "categories");
        assert_eq!(
            ids("organisms")[6..],
            [
                "basic-topic-list",
                "categories-list",
                "docked-composer",
                "modal",
                "more-topics",
                "navigation",
                "site-header",
            ]
        );
        assert_eq!(section_by_id("buttons").unwrap().category, "atoms");
        assert!(section_by_id("nope").is_none());
    }
}
