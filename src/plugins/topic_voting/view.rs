//! discourse-topic-voting's components on the server-rendered pages:
//! VoteBox (the vote button and count beside a topic's title) and the
//! vote count the topic list shows among a topic's tags. static/js/
//! topic-voting.js does what the buttons do.

use serde_json::Value;

use super::UserVotes;
use crate::i18n::I18n;
use crate::topic_list_view::icon;

fn t(i18n: &I18n, key: &str) -> String {
    i18n.t(&format!("js.topic_voting.{key}"))
        .unwrap_or(key)
        .to_string()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// VoteCount#displayCount
fn display_count(i18n: &I18n, count: i64) -> String {
    if count >= 1000 {
        // toFixed(1): halves round up, where Rust's formatting rounds to even.
        let thousands = format!("{:.1}", (count as f64 / 100.0).round() / 10.0);
        let thousands = thousands.strip_suffix(".0").unwrap_or(&thousands);
        return t(i18n, "vote_count_thousands").replace("%{count}", thousands);
    }
    count.to_string()
}

/// What the vote box reads about the viewer: nothing for anonymous.
pub struct Voter {
    pub votes: UserVotes,
}

/// The title's vote box (`<div class="voting title-voting">`) for a topic
/// view the viewer can vote on, else nothing.
pub fn title_voting(
    i18n: &I18n,
    base_path: &str,
    show_who_voted: bool,
    view: &Value,
    voter: Option<&Voter>,
) -> String {
    if view["can_vote"] != Value::Bool(true) {
        return String::new();
    }
    let count = view["vote_count"].as_i64().unwrap_or(0);
    let voted = view["user_voted"] == Value::Bool(true);
    let closed = view["closed"] == Value::Bool(true);
    let locked = voter.is_some_and(|v| v.votes.limit == Some(0));
    // VoteButton#buttonClasses, #ariaLabel, #buttonIcon
    let classes = if voted && !locked {
        "btn-success btn-small voting-wrapper__button"
    } else {
        "btn-default btn-small voting-wrapper__button"
    };
    let label = if closed {
        t(i18n, "voting_closed_description")
    } else if locked {
        t(i18n, "locked_description")
    } else if voted {
        t(i18n, "remove_vote")
    } else {
        t(i18n, "vote_title")
    };
    let label = escape(&label);
    let button_icon = icon(if voted { "vote-up-filled" } else { "vote-up" }, None);
    let button = if closed {
        format!(
            "<span aria-expanded=\"false\" class=\"fk-d-tooltip__trigger\" data-identifier=\"vote-closed-tooltip\" \
             data-trigger=\"\" role=\"button\" data-tooltip=\"{label}\"><span class=\"fk-d-tooltip__trigger-container\">\
             <button aria-label=\"{label}\" class=\"btn no-text btn-icon {classes}\" disabled=\"\" type=\"button\">\
             {button_icon}<span aria-hidden=\"true\">\u{200b}</span></button></span></span>"
        )
    } else if voter.is_some() {
        format!(
            "<button aria-expanded=\"false\" aria-label=\"{label}\" class=\"btn no-text btn-icon fk-d-menu__trigger \
             topic-voting-menu-trigger {classes}\" title=\"{label}\" data-identifier=\"topic-voting-menu\" \
             data-trigger=\"\" type=\"button\">{button_icon}</button>"
        )
    } else {
        format!(
            "<button aria-label=\"{label}\" class=\"btn no-text btn-icon {classes}\" title=\"{label}\" \
             type=\"button\" data-login-url=\"{base_path}/login\">{button_icon}\
             <span aria-hidden=\"true\">\u{200b}</span></button>"
        )
    };
    let no_votes = if count == 0 { " no-votes" } else { "" };
    let shown = display_count(i18n, count);
    // VoteCount: a menu of the voters for a member when they're shown.
    let count_html = if show_who_voted && voter.is_some() {
        format!(
            "<button aria-label=\"{}\" aria-expanded=\"false\" class=\"fk-d-menu__trigger vote-count-voters-trigger \
             voting-wrapper__count{no_votes}\" data-identifier=\"vote-count-voters\" data-trigger=\"\" type=\"button\">\
             <span class=\"voting-wrapper__count-text\">{shown}</span></button>",
            escape(&t(i18n, "show_voters"))
        )
    } else {
        format!(
            "<div class=\"voting-wrapper__count{no_votes}\"><div class=\"voting-wrapper__count-text\">{shown}</div></div>"
        )
    };
    // What topic-voting.js needs to act as the components do.
    let mut data = format!(
        " data-topic-id=\"{}\" data-vote-count=\"{count}\" data-user-voted=\"{voted}\" data-closed=\"{closed}\"",
        view["id"]
    );
    if let Some(v) = voter {
        let labels = serde_json::json!({
            "see_votes": t(i18n, "see_votes"),
            "remove_vote": t(i18n, "remove_vote"),
            "vote_title": t(i18n, "vote_title"),
            "watch_topic": t(i18n, "watch_topic"),
            "locked_description": t(i18n, "locked_description"),
            "no_votes_yet": t(i18n, "no_votes_yet"),
            "and_more_voters": t(i18n, "and_more_voters"),
            "loading": i18n.t("js.loading").unwrap_or("Loading..."),
        });
        data.push_str(&format!(
            " data-labels=\"{}\" data-base-path=\"{}\"",
            escape(&labels.to_string()),
            escape(base_path)
        ));
        data.push_str(&format!(
            " data-votes-exceeded=\"{}\" data-votes-left=\"{}\" data-vote-limit=\"{}\" data-watching=\"{}\"",
            v.votes.reached(),
            v.votes.left().map(|l| l.to_string()).unwrap_or_default(),
            v.votes.limit.map(|l| l.to_string()).unwrap_or_default(),
            view["details"]["notification_level"].as_i64() == Some(3),
        ));
    }
    format!(
        "<div class=\"voting title-voting\"><div class=\"voting-wrapper\"{data}>{button}{count_html}</div></div>"
    )
}

/// extend-category-for-voting's tags callback: a topic list item's vote
/// count, as a tag linking to the topic, first among its tags.
pub fn list_vote_count(i18n: &I18n, topic: &Value, url: &str) -> String {
    if topic["can_vote"] != Value::Bool(true) {
        return String::new();
    }
    let count = topic["vote_count"].as_i64().unwrap_or(0);
    let voted = topic["user_voted"] == Value::Bool(true);
    let title = if voted {
        format!(" title='{}'", escape(&t(i18n, "voted")))
    } else {
        String::new()
    };
    let label = i18n
        .t_count("js.topic_voting.votes", count, &[])
        .unwrap_or_else(|| count.to_string());
    format!(
        "<a href='{}' class='list-vote-count vote-count-{count} discourse-tag simple{}'{title}>{}</a>",
        escape(url),
        if voted { " voted" } else { "" },
        escape(&label)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_from_a_thousand_are_shortened() {
        let i18n = I18n::vendored().unwrap();
        assert_eq!(display_count(&i18n, 999), "999");
        assert_eq!(display_count(&i18n, 1000), "1k");
        assert_eq!(display_count(&i18n, 1250), "1.3k");
    }
}
