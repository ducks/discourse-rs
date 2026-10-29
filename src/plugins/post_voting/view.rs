//! discourse-post-voting's components on the server-rendered topic page:
//! PostVotingVoteControls after the post's avatar, PostVotingAnswerHeader
//! before the first answer, PostVotingComments after the post menu (each
//! comment with its vote and actions, and PostVotingCommentsMenu), the
//! Answer button in the Reply button's place, and the topic page's body
//! classes. static/js/post-voting.js does what the buttons do; the live
//! stream redraws a post once a vote or comment on it is published.

use serde_json::Value;

use crate::post_view::{PostContext, d_icon, tiny_date};
use crate::site_settings::{SettingError, SiteSettings};
use crate::topic_list_view::{escape, t, t_count};

/// What the components read of the site settings.
pub struct PostVotingUi {
    pub comment_enabled: bool,
    pub likes_on_answers: bool,
    /// The comment composer's limits (`min_post_length`,
    /// `post_voting_comment_max_raw_length`).
    pub min_length: i64,
    pub max_length: i64,
}

impl PostVotingUi {
    /// None when the plugin is off.
    pub fn load(settings: &SiteSettings) -> Result<Option<PostVotingUi>, SettingError> {
        if !super::enabled(settings)? {
            return Ok(None);
        }
        Ok(Some(PostVotingUi {
            comment_enabled: settings.get("post_voting_comment_enabled")?.truthy(),
            likes_on_answers: settings
                .get("post_voting_enable_likes_on_answers")?
                .truthy(),
            min_length: settings.get("min_post_length")?.to_i(),
            max_length: settings.get("post_voting_comment_max_raw_length")?.to_i(),
        }))
    }
}

/// What the components read of a post voting topic.
#[derive(Debug, Clone, Default)]
pub struct TopicState {
    /// `topic.posts_count - 1`: the answer count the header shows.
    pub answers_count: i64,
    /// The post the header goes before: the stream's second.
    pub header_post_id: Option<i64>,
    /// The topic's path (without the base path), which the sort links
    /// go to.
    pub url: String,
    /// The stream is sorted by activity (`filter=activity`).
    pub by_activity: bool,
}

impl TopicState {
    /// From the topic view JSON: None unless it is a post voting topic.
    pub fn from_view(view: &Value) -> Option<TopicState> {
        if view["is_post_voting"] != Value::Bool(true) {
            return None;
        }
        Some(TopicState {
            answers_count: view["posts_count"].as_i64().unwrap_or(0) - 1,
            header_post_id: view["post_stream"]["stream"][1].as_i64(),
            url: format!(
                "/t/{}/{}",
                view["slug"].as_str().unwrap_or_default(),
                view["id"]
            ),
            by_activity: false,
        })
    }

    /// A post redrawn on its own (the live stream): its votes and comments
    /// but not the header, which static/js/post-voting.js keeps where the
    /// page drew it.
    pub fn for_redraw(post: &Value) -> Option<TopicState> {
        post.get("post_voting_has_votes")?;
        Some(TopicState {
            answers_count: 0,
            header_post_id: None,
            url: format!(
                "/t/{}/{}",
                post["topic_slug"].as_str().unwrap_or_default(),
                post["topic_id"]
            ),
            by_activity: false,
        })
    }

    /// The topic page's body classes (AddPostVotingTopicPageClass).
    pub fn body_classes(&self) -> &'static str {
        if self.by_activity {
            " post-voting-topic --sort-by-activity"
        } else {
            " post-voting-topic"
        }
    }
}

/// The post belongs to a post voting topic (its serializer has the keys).
fn post_voting_post(p: &Value) -> bool {
    p.get("post_voting_has_votes").is_some()
}

/// A PostVotingButton (DButton): up or down, voted or not.
fn vote_button(cx: &PostContext, direction: &str, voted: bool, disabled: bool) -> String {
    let key = match (direction, voted) {
        ("up", false) => "vote.button.upvote",
        ("up", true) => "vote.button.remove_upvote",
        (_, false) => "vote.button.downvote",
        (_, true) => "vote.button.remove_downvote",
    };
    let label = escape(&t(cx.list, key));
    let mut classes = format!(
        "btn no-text btn-icon btn-transparent post-voting-button --{}vote",
        if direction == "up" { "up" } else { "down" }
    );
    if voted {
        classes.push_str(" --voted");
    }
    format!(
        "<button aria-label=\"{label}\" class=\"{classes}\" data-direction=\"{direction}\" data-voted=\"{voted}\"{} title=\"{label}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
        if disabled { " disabled=\"\"" } else { "" },
        d_icon(if voted { "vote-up-filled" } else { "vote-up" }, None)
    )
}

/// PostVotingVoteControls, after the post-avatar wrapper: the votes, with
/// the voters menu once the post has some.
pub fn vote_controls(cx: &PostContext, p: &Value) -> String {
    if cx.topic.post_voting.is_none() {
        return String::new();
    }
    let disabled = cx.topic.archived || cx.topic.closed;
    let direction = p["post_voting_user_voted_direction"].as_str();
    let count = p["post_voting_vote_count"].as_i64().unwrap_or(0);
    let toggle = if p["post_voting_has_votes"] == true {
        let label = escape(&t(cx.list, "vote.toggle_voters"));
        format!(
            "<button aria-expanded=\"false\" aria-label=\"{label}\" class=\"btn fk-d-menu__trigger post-voting-post__toggle-voters btn-transparent\" data-identifier=\"post-voting-popup\" data-trigger=\"\" title=\"{label}\" type=\"button\">{count}</button>"
        )
    } else {
        format!("<span class=\"post-voting-post__toggle-voters\">{count}</span>")
    };
    format!(
        "<div class=\"post-voting-post\" data-post-id=\"{}\">{}{toggle}{}</div>",
        p["id"],
        vote_button(cx, "up", direction == Some("up"), disabled),
        vote_button(cx, "down", direction == Some("down"), disabled),
    )
}

/// PostVotingAnswerHeader, before the post-article wrapper of the
/// stream's first answer: the answer count and the sort links.
pub fn answer_header(cx: &PostContext, p: &Value) -> String {
    let Some(state) = cx.topic.post_voting.as_ref() else {
        return String::new();
    };
    if state.answers_count <= 0 || p["id"].as_i64() != state.header_post_id {
        return String::new();
    }
    let url = format!("{}{}", cx.list.base_path, state.url);
    let link = |class: &str, active: bool, href: &str, key: &str| {
        format!(
            "<li><a{} class=\"{class}{}\" href=\"{}\">{}</a></li>",
            if active { " aria-current=\"true\"" } else { "" },
            if active { " active" } else { "" },
            href.replace('"', "&quot;"),
            escape(&t(cx.list, key))
        )
    };
    format!(
        "<nav aria-label=\"{}\" class=\"post-voting-answers-header\"><h5 class=\"post-voting-answers-header__count\">{}</h5><ul class=\"nav-pills post-voting-answers-header__sort\">{}{}</ul></nav>",
        escape(&t(cx.list, "post_voting.topic.sort_by")),
        escape(&t_count(
            cx.list,
            "post_voting.topic.answer_count",
            state.answers_count,
            &[]
        )),
        link(
            "--votes",
            !state.by_activity,
            &url,
            "post_voting.topic.votes"
        ),
        link(
            "--activity",
            state.by_activity,
            &format!("{url}?filter={}", super::ACTIVITY_FILTER),
            "post_voting.topic.activity"
        ),
    )
}

/// The post-menu-buttons transformer: on a post voting topic's post the
/// Answer button after show more and reply, reply gone, and like gone
/// from the answers unless post_voting_enable_likes_on_answers.
pub fn menu_keys(ui: &PostVotingUi, p: &Value, configured: &mut Vec<&str>) {
    if !post_voting_post(p) {
        return;
    }
    let at = configured
        .iter()
        .rposition(|k| *k == "showMore" || *k == "reply")
        .map_or(configured.len(), |i| i + 1);
    configured.insert(at, "post-voting-answer");
    configured.retain(|k| *k != "reply");
    if p["post_number"].as_i64() != Some(1) && !ui.likes_on_answers {
        configured.retain(|k| *k != "like");
    }
}

/// PostVotingAnswerButton: on the question, for someone who may post.
pub fn answer_button(cx: &PostContext, p: &Value) -> Option<String> {
    if !cx.topic.can_create_post || cx.viewer.is_none() || p["post_number"].as_i64() != Some(1) {
        return None;
    }
    let mobile = crate::mobile::mobile_view();
    let (classes, label) = if mobile {
        (
            "btn no-text btn-icon post-action-menu__post-voting-answer reply btn-flat",
            "<span aria-hidden=\"true\">&#8203;</span>".to_string(),
        )
    } else {
        (
            "btn btn-icon-text post-action-menu__post-voting-answer reply create fade-out btn-flat",
            format!(
                "<span class=\"d-button-label\">{}</span>",
                escape(&t(cx.list, "post_voting.topic.answer.label"))
            ),
        )
    };
    Some(format!(
        "<button class=\"{classes}\" data-post-number=\"1\" data-username=\"{}\" title=\"{}\" type=\"button\">{}{label}</button>",
        escape(p["username"].as_str().unwrap_or_default()),
        escape(&t(cx.list, "post_voting.topic.answer.help")),
        d_icon("reply", None),
    ))
}

/// PostVotingComments, in the post-menu__after outlet: the post's
/// comments, then the comments menu for someone who may post.
pub fn comments(cx: &PostContext, ui: &PostVotingUi, p: &Value) -> String {
    if !ui.comment_enabled || !post_voting_post(p) || !p["reply_to_post_number"].is_null() {
        return String::new();
    }
    let disabled = cx.topic.closed || cx.topic.archived;
    let list = p["comments"].as_array().map_or(&[][..], Vec::as_slice);
    let mut html = String::new();
    for c in list {
        html.push_str(&comment(cx, c, disabled));
    }
    if cx.topic.can_create_post {
        let more = p["comments_count"].as_i64().unwrap_or(0) - list.len() as i64;
        html.push_str(&comments_menu(cx, more));
    }
    let last = list.last().and_then(|c| c["id"].as_i64()).unwrap_or(0);
    format!(
        "<div class=\"post-voting-comments\" data-post-id=\"{}\" data-post-number=\"{}\" data-last-comment-id=\"{last}\" data-comments-count=\"{}\" data-can-create-post=\"{}\" data-disabled=\"{disabled}\" data-min-length=\"{}\" data-max-length=\"{}\">{html}</div>",
        p["id"],
        p["post_number"],
        p["comments_count"].as_i64().unwrap_or(0),
        cx.topic.can_create_post,
        ui.min_length,
        ui.max_length
    )
}

/// PostVotingComment: its vote, cooked text, author, date and actions.
pub fn comment(cx: &PostContext, c: &Value, disabled: bool) -> String {
    let id = &c["id"];
    let count = c["post_voting_vote_count"].as_i64().unwrap_or(0);
    let count_html = if count != 0 {
        format!("<span class=\"post-voting-comments__vote-count\">{count}</span>")
    } else {
        "<span class=\"post-voting-comments__vote-count --none\">0</span>".to_string()
    };
    let voted = c["user_voted"] == true;
    let author = match c["username"].as_str() {
        Some(username) => format!(
            "<a class=\"post-voting-comments__username\" data-user-card=\"{}\" href=\"{}/u/{}\">{}</a>",
            escape(username),
            cx.list.base_path,
            escape(&username.to_lowercase()),
            escape(username)
        ),
        None => format!(
            "<span class=\"post-voting-comments__username --deleted\">{}</span>",
            escape(&t(
                cx.list,
                "post_voting.post.post_voting_comment.user.deleted"
            ))
        ),
    };
    let created = c["created_at"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|at| tiny_date(cx, at.with_timezone(&chrono::Utc)))
        .unwrap_or_default();
    format!(
        "<div class=\"post-voting-comments__comment\" id=\"post-voting-comment-{id}\" data-comment-id=\"{id}\" data-user-id=\"{}\" data-raw=\"{}\"><div class=\"post-voting-comments__vote\">{count_html}{}</div><div class=\"post-voting-comments__comment-post\"><span class=\"post-voting-comments__comment-cooked\">{}</span><span class=\"post-voting-comments__separator\">–</span>{author}<span class=\"post-voting-comments__created\">{created}</span>{}</div></div>",
        c["user_id"],
        escape(c["raw"].as_str().unwrap_or_default()),
        vote_button(cx, "up", voted, disabled),
        c["cooked"].as_str().unwrap_or_default(),
        comment_actions(cx, c, disabled),
    )
}

/// PostVotingCommentActions: edit, flag and delete, for the comment's
/// author and staff (the flag inside, as the component nests it).
fn comment_actions(cx: &PostContext, c: &Value, disabled: bool) -> String {
    let Some(viewer) = cx.viewer else {
        return String::new();
    };
    let mine = c["username"].as_str() == Some(viewer);
    if disabled || !(mine || cx.staff) {
        return String::new();
    }
    let action = |class: &str, key: &str, icon: &str| {
        let label = escape(&t(cx.list, key));
        format!(
            "<button aria-label=\"{label}\" class=\"btn no-text btn-icon {class} btn-transparent\" title=\"{label}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
            d_icon(icon, None)
        )
    };
    format!(
        "<span class=\"post-voting-comments__comment-actions\">{}{}{}</span>",
        action(
            "post-voting-comments__comment-actions-edit-link",
            "post_voting.post.post_voting_comment.actions.edit",
            "pencil"
        ),
        action(
            "post-voting-comments__comment-actions-flag-link",
            "post_voting.post.post_voting_comment.actions.flag",
            "flag"
        ),
        action(
            "post-voting-comments__comment-actions-delete-link",
            "post_voting.post.post_voting_comment.actions.delete",
            "far-trash-can"
        ),
    )
}

/// PostVotingCommentsMenu, closed: Add a comment, and Show more while
/// the post has comments not drawn.
fn comments_menu(cx: &PostContext, more: i64) -> String {
    let label_button = |class: &str, label: &str| {
        format!(
            "<button class=\"btn btn-text btn-transparent --primary btn-small {class}\" type=\"button\"><span class=\"d-button-label\">{}</span></button>",
            escape(label)
        )
    };
    let show_more = if more > 0 {
        label_button(
            "post-voting-comments__actions-show-more",
            &t_count(
                cx.list,
                "post_voting.post.post_voting_comment.show",
                more,
                &[],
            ),
        )
    } else {
        String::new()
    };
    format!(
        "<div class=\"post-voting-comments__footer\"><div class=\"post-voting-comments__actions\">{}{show_more}</div></div>",
        label_button(
            "post-voting-comments__actions-add",
            &t(cx.list, "post_voting.post.post_voting_comment.add")
        ),
    )
}

/// The labels static/js/post-voting.js draws its composer, editor,
/// confirmation and voters menu with, as data attributes of the topic.
pub fn client_labels(cx: &PostContext) -> String {
    let labels = serde_json::json!({
        "submit": t(cx.list, "post_voting.post.post_voting_comment.submit"),
        "edit": t(cx.list, "post_voting.post.post_voting_comment.edit"),
        "cancel": t(cx.list, "post_voting.post.post_voting_comment.cancel"),
        "delete_confirm": t(cx.list, "post_voting.post.post_voting_comment.delete.confirm"),
        "too_short_one": t(cx.list, "post_voting.post.post_voting_comment.composer.too_short.one"),
        "too_short_other": t(cx.list, "post_voting.post.post_voting_comment.composer.too_short.other"),
        "too_long_one": t(cx.list, "post_voting.post.post_voting_comment.composer.too_long.one"),
        "too_long_other": t(cx.list, "post_voting.post.post_voting_comment.composer.too_long.other"),
        "upvotes": t(cx.list, "vote.upvotes"),
        "downvotes": t(cx.list, "vote.downvotes"),
        "voters_more_one": t(cx.list, "post_voting.topic.voters_count_diff.one"),
        "voters_more_other": t(cx.list, "post_voting.topic.voters_count_diff.other"),
        "loading": t(cx.list, "loading"),
        "vote_icon": d_icon("vote-up-filled", None),
        "reply_icon": d_icon("reply", None),
        "pencil_icon": d_icon("pencil", None),
        "flag_icon": d_icon("flag", None),
        "trash_icon": d_icon("far-trash-can", None),
        "edit_label": t(cx.list, "post_voting.post.post_voting_comment.actions.edit"),
        "flag_label": t(cx.list, "post_voting.post.post_voting_comment.actions.flag"),
        "delete_label": t(cx.list, "post_voting.post.post_voting_comment.actions.delete"),
        "upvote_label": t(cx.list, "vote.button.upvote"),
        "remove_upvote_label": t(cx.list, "vote.button.remove_upvote"),
        "deleted_user": t(cx.list, "post_voting.post.post_voting_comment.user.deleted"),
        "show_more_one": t(cx.list, "post_voting.post.post_voting_comment.show.one"),
        "show_more_other": t(cx.list, "post_voting.post.post_voting_comment.show.other"),
        "unvoted_icon": d_icon("vote-up", None),
        // What the components read of the current user.
        "viewer": cx.viewer,
        "staff": cx.staff,
        "login_url": format!("{}/login", cx.list.base_path),
    });
    format!(
        "<template id=\"post-voting-labels\" data-labels=\"{}\"></template>",
        escape(&labels.to_string())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_button_takes_the_reply_buttons_place() {
        let ui = PostVotingUi {
            comment_enabled: true,
            likes_on_answers: false,
            min_length: 20,
            max_length: 600,
        };
        let answer = serde_json::json!({"post_number": 2, "post_voting_has_votes": false});
        let mut keys = vec!["read", "like", "copyLink", "flag", "showMore", "reply"];
        menu_keys(&ui, &answer, &mut keys);
        assert_eq!(
            keys,
            ["read", "copyLink", "flag", "showMore", "post-voting-answer"]
        );
        let question = serde_json::json!({"post_number": 1, "post_voting_has_votes": true});
        let mut keys = vec!["read", "like", "reply", "showMore", "flag"];
        menu_keys(&ui, &question, &mut keys);
        assert_eq!(
            keys,
            ["read", "like", "showMore", "post-voting-answer", "flag"]
        );
        // Not a post voting topic's post: as configured.
        let other = serde_json::json!({"post_number": 2});
        let mut keys = vec!["like", "reply"];
        menu_keys(&ui, &other, &mut keys);
        assert_eq!(keys, ["like", "reply"]);
    }
}
