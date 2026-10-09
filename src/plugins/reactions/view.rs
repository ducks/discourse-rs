//! discourse-reactions' post menu on the server-rendered topic page: the
//! like button's place taken by the reaction button
//! (ReactionsActionButton, DiscourseReactionsReactionButton) and the
//! reactions summary before the buttons (ReactionsActionSummary,
//! DiscourseReactionsCounter, DiscourseReactionsList).
//! static/js/discourse-reactions.js does what they do in the browser.
//!
//! A like icon other than the heart is drawn as the icon itself for both
//! states; Ember swaps in its `far-` variant when the icon set has one.

use serde_json::Value;

use crate::post_view::d_icon;
use crate::site_settings::{SettingError, SiteSettings};
use crate::topic_list_view::{ListContext, escape, t, t_count, t_with};

/// PostActionType.types[:like]
const LIKE: i64 = 2;

/// The settings the reactions UI reads.
pub struct ReactionsUi {
    /// `discourse_reactions_reaction_for_like`
    pub main: String,
    /// `discourse_reactions_like_icon`
    pub like_icon: String,
    /// The picker's reactions: `discourse_reactions_enabled_reactions`.
    pub enabled: Vec<String>,
    pub desaturated: bool,
    pub enable_emoji: bool,
    pub emoji_set: String,
    pub prioritize_username: bool,
}

impl ReactionsUi {
    /// None when the plugin is off.
    pub fn load(settings: &SiteSettings) -> Result<Option<ReactionsUi>, SettingError> {
        if !super::enabled(settings)? {
            return Ok(None);
        }
        Ok(Some(ReactionsUi {
            main: settings
                .get("discourse_reactions_reaction_for_like")?
                .to_s(),
            like_icon: settings.get("discourse_reactions_like_icon")?.to_s(),
            enabled: settings
                .get("discourse_reactions_enabled_reactions")?
                .to_s()
                .split('|')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
            desaturated: settings
                .get("discourse_reactions_desaturated_reaction_panel")?
                .truthy(),
            enable_emoji: settings.get("enable_emoji")?.truthy(),
            emoji_set: settings.get("emoji_set")?.to_s(),
            prioritize_username: settings.get("prioritize_username_in_ux")?.truthy(),
        }))
    }

    fn liked_icon(&self) -> &str {
        if self.like_icon == "heart" {
            "d-liked"
        } else {
            &self.like_icon
        }
    }

    fn unliked_icon(&self) -> &str {
        if self.like_icon == "heart" {
            "d-unliked"
        } else {
            &self.like_icon
        }
    }

    /// `emojiUrlFor`: a standard emoji's image, a `:tN` tone in its path.
    pub fn emoji_url(&self, base_path: &str, name: &str) -> String {
        let path = match name.split_once(":t") {
            Some((base, tone)) => format!("{base}/{tone}"),
            None => name.to_string(),
        };
        format!(
            "{base_path}/images/emoji/{}/{path}.png?v={}",
            self.emoji_set,
            crate::emoji::image_version()
        )
    }
}

/// What the reaction controls read about the post and the viewer.
pub struct PostReactions<'a> {
    pub list: &'a ListContext<'a>,
    pub ui: &'a ReactionsUi,
    pub post: &'a Value,
    /// The viewer is a member.
    pub member: bool,
    pub topic_archived: bool,
    pub topic_closed: bool,
    /// The small avatar's size (DUserAvatar @size="small").
    pub avatar_size: i64,
}

impl PostReactions<'_> {
    fn like_action(&self) -> Option<&Value> {
        self.post["actions_summary"]
            .as_array()
            .and_then(|a| a.iter().find(|x| x["id"].as_i64() == Some(LIKE)))
    }

    /// ActionSummary#canToggle
    fn can_toggle(&self) -> bool {
        self.like_action()
            .is_some_and(|l| l["can_undo"] == true || l["can_act"] == true)
    }

    fn reactions(&self) -> &[Value] {
        self.post["reactions"].as_array().map_or(&[], Vec::as_slice)
    }

    fn current(&self) -> Option<&Value> {
        Some(&self.post["current_user_reaction"]).filter(|v| v.is_object())
    }

    /// reactionsHiddenForUser
    pub fn hidden_for_user(&self) -> bool {
        self.post["hidden"] == true && self.post["can_see_hidden_post"] != true
    }

    /// DiscourseReactionsActions#canReact
    fn can_react(&self) -> bool {
        if self.topic_archived {
            return false;
        }
        if !self.member {
            return true;
        }
        self.current().is_none_or(|c| c["can_undo"] == true) && self.can_toggle()
    }

    /// DiscourseReactionsActions#classes
    fn classes(&self) -> String {
        let reactions = self.reactions();
        let mut classes = Vec::new();
        if reactions
            .iter()
            .any(|r| r["id"].as_str() != Some(&self.ui.main))
        {
            classes.push("custom-reaction-used");
        }
        if self.post["yours"] == true {
            classes.push("my-post");
        }
        if !reactions.is_empty() {
            classes.push("has-reactions");
        }
        if self.current().is_some() {
            classes.push("has-reacted");
        }
        if self.post["current_user_used_main_reaction"] == true {
            classes.push("has-used-main-reaction");
        }
        if self.can_react() {
            classes.push("can-toggle-reaction");
        }
        classes.join(" ")
    }

    fn reaction_users_count(&self) -> i64 {
        self.post["reaction_users_count"].as_i64().unwrap_or(0)
    }

    /// The data the browser side reads: the post, the main reaction and
    /// the picker's reactions.
    fn data_attrs(&self) -> String {
        let current = self.current().and_then(|c| c["id"].as_str()).unwrap_or("");
        let mut attrs = format!(
            " data-post-id=\"{}\" data-base-path=\"{}\" data-main-reaction=\"{}\" data-current-reaction=\"{}\"",
            self.post["id"],
            escape(self.list.base_path),
            escape(&self.ui.main),
            escape(current)
        );
        // deferAnonymousAction: here, the login page.
        if !self.member {
            attrs.push_str(&format!(
                " data-reaction-login-url=\"{}/login\"",
                self.list.base_path
            ));
        }
        attrs
    }

    /// DiscourseReactionsPicker#reactionInfo and #optimalColsCount, for
    /// the browser to draw the picker from. The emoji picker that
    /// discourse_reactions_allow_any_emoji adds is not ported.
    fn picker(&self) -> Value {
        let mut ids = self.ui.enabled.clone();
        if !ids.contains(&self.ui.main) {
            ids.insert(0, self.ui.main.clone());
        }
        let current = self.current();
        let entries: Vec<Value> = ids
            .iter()
            .map(|id| {
                let is_used = if *id == self.ui.main {
                    self.post["current_user_used_main_reaction"] == true
                } else {
                    current.is_some_and(|c| c["id"].as_str() == Some(id))
                };
                let can_undo = if !self.member {
                    !(self.topic_archived || self.topic_closed)
                } else if let Some(c) = current {
                    c["can_undo"] == true && self.can_toggle()
                } else {
                    self.can_toggle()
                };
                let title = if can_undo {
                    t_with(
                        self.list,
                        "discourse_reactions.picker.react_with",
                        &[("reaction", id)],
                    )
                } else {
                    t(self.list, "discourse_reactions.picker.cant_remove_reaction")
                };
                serde_json::json!({
                    "id": id,
                    "title": title,
                    "canUndo": can_undo,
                    "isUsed": is_used,
                    "html": self.reaction_emoji(id),
                })
            })
            .collect();
        serde_json::json!({"cols": optimal_cols(entries.len()), "reactions": entries})
    }

    /// ReactionsActionSummary: the counter, when someone reacted.
    pub fn summary(&self) -> String {
        if self.post["deleted"] == true
            || self.post["deleted_at"].is_string()
            || self.reaction_users_count() <= 0
            || self.hidden_for_user()
        {
            return String::new();
        }
        format!(
            "<div class=\"reactions-actions-summary\"><div class=\"discourse-reactions-actions {}\" id=\"discourse-reactions-actions-{}-left\"{}>{}</div></div>",
            self.classes(),
            self.post["id"],
            self.data_attrs(),
            self.counter()
        )
    }

    /// DiscourseReactionsCounter, at the left.
    fn counter(&self) -> String {
        let count = self.reaction_users_count();
        if count == 0 {
            return String::new();
        }
        let reactions = self.reactions();
        let only_like = reactions.len() == 1 && reactions[0]["id"].as_str() == Some(&self.ui.main);
        let mut list = String::new();
        for r in reactions {
            let id = r["id"].as_str().unwrap_or("");
            let emoji = if r["count"].as_i64().unwrap_or(0) > 0 {
                self.reaction_emoji(id)
            } else {
                String::new()
            };
            list.push_str(&format!(
                "<span class=\"discourse-reactions-list-emoji\" id=\"discourse-reactions-list-emoji-{}-{}\">{emoji}</span>",
                self.post["id"],
                escape(id)
            ));
        }
        format!(
            "<button aria-expanded=\"false\" aria-haspopup=\"dialog\" aria-label=\"{}\" class=\"discourse-reactions-counter{}\" id=\"discourse-reactions-counter-{}-left\" type=\"button\" data-users-menu=\"{}\"><span class=\"discourse-reactions-list\"><span class=\"reactions\">{list}</span></span><span aria-hidden=\"true\" class=\"reactions-counter\">{count}</span></button>",
            escape(&t_count(
                self.list,
                "discourse_reactions.counter.aria_label",
                count,
                &[]
            )),
            if only_like { " only-like" } else { "" },
            self.post["id"],
            escape(&self.users_menu().to_string())
        )
    }

    /// What DiscourseReactionsUsersMenu needs: the list's url, the filters
    /// (when there is more than one reaction), the totals and labels.
    fn users_menu(&self) -> Value {
        let filters: Vec<Value> = self
            .reactions()
            .iter()
            .map(|r| {
                let id = r["id"].as_str().unwrap_or("");
                serde_json::json!({"id": id, "count": r["count"], "html": self.reaction_emoji(id)})
            })
            .collect();
        serde_json::json!({
            "url": format!("{}/discourse-reactions/posts/{}/reactions-users-list.json", self.list.base_path, self.post["id"]),
            "total": self.reaction_users_count(),
            "filters": if filters.len() > 1 { filters } else { Vec::new() },
            "all": t(self.list, "discourse_reactions.users_popup.all"),
            "basePath": self.list.base_path,
            "avatarSize": self.avatar_size,
            "prioritizeUsername": self.ui.prioritize_username,
            "emoji": {
                "set": self.ui.emoji_set,
                "version": crate::emoji::image_version(),
                "desaturated": false,
            },
            "likedIcon": d_icon("d-liked", Some("users-popup__reaction")),
        })
    }

    /// discourse-reactions-emoji: the emoji, or the like icon for the main
    /// reaction where emoji are off.
    fn reaction_emoji(&self, id: &str) -> String {
        if !self.ui.enable_emoji && id == self.ui.main {
            return d_icon(self.ui.liked_icon(), None).replacen(
                "aria-hidden=\"true\"",
                &format!("aria-label=\"{}\"", escape(id)),
                1,
            );
        }
        let class = if self.ui.desaturated {
            "emoji desaturated"
        } else {
            "emoji"
        };
        format!(
            "<img width=\"20\" height=\"20\" src=\"{}\" title=\"{id}\" alt=\"{id}\" class=\"{class}\">",
            crate::category_badge::html_escape(&self.ui.emoji_url(self.list.base_path, id)),
            id = escape(id)
        )
    }

    /// ReactionsActionButton: in the like button's place, unless the post
    /// shows no like (`show_like`, as the like button decides) and has
    /// none.
    pub fn button(&self, show_like: bool, like_count: i64) -> String {
        if self.hidden_for_user() || !(show_like || like_count > 0) {
            return String::new();
        }
        let inner = if self.post["yours"] == true {
            String::new()
        } else {
            self.reaction_button()
        };
        // showReactionsPicker: anonymous readers who may react, members on
        // others' posts.
        let picker =
            if (!self.member && self.can_react()) || (self.member && self.post["yours"] != true) {
                format!(" data-picker=\"{}\"", escape(&self.picker().to_string()))
            } else {
                String::new()
            };
        format!(
            "<div class=\"discourse-reactions-actions-button-shim\"><div class=\"discourse-reactions-actions {}\" id=\"discourse-reactions-actions-{}-right\"{}{picker}>{inner}</div></div>",
            self.classes(),
            self.post["id"],
            self.data_attrs()
        )
    }

    /// DiscourseReactionsReactionButton#title
    fn title(&self) -> Option<String> {
        let l = self.list;
        if !self.member {
            return Some(t(l, "discourse_reactions.main_reaction.unauthenticated"));
        }
        let like = self.like_action()?;
        let undo_blank = like["can_undo"].is_null() || like["can_undo"] == false;
        let can_toggle = self.can_toggle();
        let mut title = if can_toggle && undo_blank {
            t(l, "discourse_reactions.main_reaction.add")
        } else if can_toggle {
            t(l, "discourse_reactions.main_reaction.remove")
        } else {
            t(l, "discourse_reactions.main_reaction.cant_remove")
        };
        if let Some(current) = self.current().filter(|_| undo_blank) {
            if current["can_undo"] == true {
                let id = current["id"].as_str().unwrap_or("");
                title = t_with(
                    l,
                    "discourse_reactions.picker.remove_reaction",
                    &[("reaction", id)],
                );
            } else {
                title = t(l, "discourse_reactions.picker.cant_remove_reaction");
            }
        }
        Some(title)
    }

    fn reaction_button(&self) -> String {
        let title = self.title();
        let title_attr = title
            .as_deref()
            .map(|t| format!(" title=\"{}\"", escape(t)))
            .unwrap_or_default();
        let button = if self.post["current_user_used_main_reaction"] == true {
            format!(
                "<button class=\"btn no-text btn-icon btn-toggle-reaction-like btn-flat btn-icon no-text reaction-button\"{title_attr} type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
                d_icon(self.ui.liked_icon(), None)
            )
        } else if let Some(current) = self.current() {
            let id = current["id"].as_str().unwrap_or("");
            format!(
                "<button class=\"btn no-text btn-icon no-text btn-flat reaction-button\"{title_attr} type=\"button\"><img alt=\":{}\" class=\"btn-toggle-reaction-emoji reaction-button\" src=\"{}\"></button>",
                escape(id),
                crate::category_badge::html_escape(&self.ui.emoji_url(self.list.base_path, id))
            )
        } else {
            format!(
                "<button class=\"btn no-text btn-icon btn-toggle-reaction-like btn-flat btn-icon no-text reaction-button\"{title_attr} type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
                d_icon(self.ui.unliked_icon(), None)
            )
        };
        format!("<div class=\"discourse-reactions-reaction-button\"{title_attr}>{button}</div>")
    }
}

/// DiscourseReactionsPicker#optimalColsCount on a desktop (columns of 5
/// to 8).
fn optimal_cols(count: usize) -> usize {
    let cols = [5, 6, 7, 8];
    if count < cols[0] {
        return count;
    }
    let mut x = cols[0];
    for (index, &i) in cols.iter().enumerate() {
        let rest = count % i;
        if rest == 0 {
            x = i;
            break;
        }
        if index > 0 && rest > count % (i - 1) {
            x = i;
        }
    }
    x
}

#[cfg(test)]
mod tests {
    use super::optimal_cols;

    #[test]
    fn picker_columns_as_ember_counts_them() {
        assert_eq!(optimal_cols(3), 3);
        assert_eq!(optimal_cols(5), 5);
        assert_eq!(optimal_cols(7), 7);
        assert_eq!(optimal_cols(9), 5);
        assert_eq!(optimal_cols(11), 6);
        assert_eq!(optimal_cols(12), 6);
    }
}
