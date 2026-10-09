//! discourse-solved's components on the server-rendered topic page: the
//! post menu's Solved button (SolvedAcceptAnswerButton,
//! SolvedUnacceptAnswerButton, placed as the post-menu-buttons
//! transformer places it) and the accepted answers under the first post
//! (SolvedAcceptedAnswers on core's DPostAccordion, with
//! SolvedAccordionItemMetadata). static/js/discourse-solved.js does what
//! they do in the browser.
//!
//! The tooltip naming who marked the answer (show_who_marked_solved) is
//! not drawn on the button; the accordion names them.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::post_view::{PostContext, avatar_img, d_icon, tiny_date, user_link_attrs};
use crate::site_settings::{SettingError, SiteSettings};
use crate::topic_list_view::{escape, t};

/// What the Solved UI reads of the site settings.
pub struct SolvedUi {
    pub allow_multiple: bool,
    pub show_who_marked_solved: bool,
    /// `solved_quote_length`
    pub quote_length: i64,
}

impl SolvedUi {
    /// None when the plugin is off.
    pub fn load(settings: &SiteSettings) -> Result<Option<SolvedUi>, SettingError> {
        if !super::enabled(settings)? {
            return Ok(None);
        }
        Ok(Some(SolvedUi {
            allow_multiple: settings.get("solved_allow_multiple_solutions")?.truthy(),
            show_who_marked_solved: settings.get("show_who_marked_solved")?.truthy(),
            quote_length: settings.get("solved_quote_length")?.to_i(),
        }))
    }
}

/// The post's Solved button, if it has one: its HTML and whether it
/// collapses behind show more (when the topic has its one answer already,
/// SolvedAcceptAnswerButton.hidden), in which case it goes among the
/// collapsed buttons.
pub struct Button {
    pub html: String,
    pub collapsed: bool,
}

pub fn button(cx: &PostContext, ui: &SolvedUi, p: &Value) -> Option<Button> {
    let accepted = p["accepted_answer"] == true;
    let can_accept = p["can_accept_answer"] == true;
    if accepted {
        // SolvedUnacceptAnswerButton
        let inner = if can_accept {
            format!(
                "<button class=\"btn btn-icon-text post-action-menu__solved-accepted accepted fade-out btn-flat\" title=\"{}\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
                escape(&t(cx.list, "solved.unaccept_answer")),
                d_icon("square-check", None),
                escape(&t(cx.list, "solved.solution"))
            )
        } else {
            format!(
                "<span class=\"accepted-text\" title=\"{}\"><span>{}</span><span class=\"accepted-label\">{}</span></span>",
                escape(&t(cx.list, "solved.accepted_description")),
                d_icon("check", None),
                escape(&t(cx.list, "solved.solution"))
            )
        };
        return Some(Button {
            html: format!("<span class=\"extra-buttons\">{inner}</span>"),
            collapsed: false,
        });
    }
    if !can_accept {
        return None;
    }
    // SolvedAcceptAnswerButton: labelled for the topic's author.
    let show_label = cx.viewer.is_some() && cx.viewer == cx.topic.created_by_username.as_deref();
    let title = escape(&t(cx.list, "solved.accept_answer"));
    let html = if show_label {
        format!(
            "<button class=\"btn btn-icon-text post-action-menu__solved-unaccepted unaccepted btn-flat\" title=\"{title}\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
            d_icon("far-square-check", None),
            escape(&t(cx.list, "solved.solution"))
        )
    } else {
        format!(
            "<button class=\"btn no-text btn-icon post-action-menu__solved-unaccepted unaccepted btn-flat\" title=\"{title}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
            d_icon("far-square-check", None)
        )
    };
    Some(Button {
        html,
        collapsed: !ui.allow_multiple && p["topic_accepted_answer"] == true,
    })
}

fn date(v: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v.as_str()?)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// userPrioritizedName
fn prioritized_name<'a>(cx: &PostContext, username: &'a str, name: Option<&'a str>) -> &'a str {
    match name.filter(|n| !n.trim().is_empty()) {
        Some(n) if !cx.settings.prioritize_username_in_ux => n,
        _ => username,
    }
}

/// SolvedAcceptedAnswers, after the first post's cooked HTML: the topic's
/// accepted answers as DPostAccordion draws them, the first expanded.
pub fn accepted_answers(cx: &PostContext, ui: &SolvedUi, answers: &[Value]) -> String {
    if answers.is_empty() {
        return String::new();
    }
    let l = cx.list;
    let mut header = format!(
        "<h3 class=\"accepted-answers__title\">{}{}</h3>",
        d_icon("far-square-check", None),
        escape(&t(l, "solved.title"))
    );
    if answers.len() > 1 {
        header.push_str(&format!(
            "<span class=\"accepted-answers__solution-count\">{} {}</span>",
            answers.len(),
            escape(&crate::topic_list_view::t_count(
                l,
                "solved.solution_summary",
                answers.len() as i64,
                &[]
            ))
        ));
    }
    // linesDisplayed: the quote length in lines of 90 characters.
    let lines =
        (ui.quote_length > 0).then(|| ((ui.quote_length as f64) / 90.0).ceil().max(1.0) as i64);
    let mut items = String::new();
    for (index, a) in answers.iter().enumerate() {
        let username = a["username"].as_str().unwrap_or_default();
        let number = a["post_number"].as_i64().unwrap_or(0);
        let topic_id = a["topic_id"].as_i64().unwrap_or(0);
        let url = a["url"].as_str().unwrap_or_default();
        let has_content = a["cooked"].as_str().is_some_and(|c| !c.is_empty());
        let expanded = index == 0;

        // SolvedAccordionItemMetadata
        let mut metadata = format!(
            "<a class=\"user-link\"{}>{}<span>{}</span></a><span class=\"dot-separator\"></span><a class=\"date-link\" href=\"{}\" title=\"{}\">{}</a>",
            user_link_attrs(cx, username, false),
            avatar_img(
                a["avatar_template"].as_str().unwrap_or_default(),
                cx.settings.avatar_size_24,
                ""
            ),
            escape(prioritized_name(cx, username, a["name"].as_str())),
            escape(url),
            escape(&t(l, "post.sr_date")),
            date(&a["created_at"])
                .map(|at| tiny_date(cx, at))
                .unwrap_or_default()
        );
        if ui.show_who_marked_solved
            && let Some(accepter) = a["accepter_username"].as_str()
        {
            let link = format!(
                "<a class=\"user-link accepter-link\"{}>{}</a>",
                user_link_attrs(cx, accepter, false),
                escape(prioritized_name(cx, accepter, a["accepter_name"].as_str()))
            );
            metadata.push_str(&format!(
                "<span class=\"dot-separator\"></span><span class=\"accepter-name\">{}</span>",
                escape(&t(l, "solved.marked_solved_by")).replace("%{user}", &link)
            ));
        }

        let controls = if has_content {
            let label = if expanded { "post.collapse" } else { "expand" };
            format!(
                "<button aria-expanded=\"{expanded}\" aria-label=\"{t}\" class=\"btn no-text btn-icon btn-flat d-post-accordion-item__toggle\" title=\"{t}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
                d_icon(
                    if expanded {
                        "chevron-up"
                    } else {
                        "chevron-down"
                    },
                    None
                ),
                t = escape(&t(l, label))
            )
        } else {
            format!(
                "<a aria-label=\"{t}\" class=\"btn no-text btn-icon btn-flat d-post-accordion-item__jump\" href=\"{}\" title=\"{t}\">{}<span aria-hidden=\"true\">&#8203;</span></a>",
                escape(url),
                d_icon("arrow-down", None),
                t = escape(&t(l, "post.follow_quote"))
            )
        };
        let body = if has_content {
            format!(
                "<div class=\"d-post-accordion-item__body\"><blockquote class=\"d-post-accordion-item__content\" id=\"post-accordion-item-{topic_id}-{number}\"><div class=\"cooked\">{}<div class=\"cooked-selection-barrier\" aria-hidden=\"true\"><br></div></div></blockquote><div class=\"d-post-accordion-item__read-more\"><a class=\"read-more-link\" href=\"{}\">{}</a></div></div>",
                a["cooked"].as_str().unwrap_or_default(),
                escape(url),
                escape(&t(l, "read_more"))
            )
        } else {
            String::new()
        };
        // data-overflowing is "true" until the browser measures the quote.
        items.push_str(&format!(
            "<div class=\"quote d-post-accordion-item{}\"{} data-overflowing=\"true\" data-post=\"{number}\" data-topic=\"{topic_id}\" data-username=\"{}\"{}><div class=\"d-post-accordion-item__header\"><div class=\"d-post-accordion-item__metadata\">{metadata}</div><div class=\"d-post-accordion-item__controls\">{controls}</div></div>{body}</div>",
            if has_content { " d-post-accordion-item--has-content" } else { "" },
            if expanded { " data-expanded=\"\"" } else { "" },
            escape(username),
            lines
                .map(|n| format!(" style=\"--max-lines-displayed: {n}\""))
                .unwrap_or_default()
        ));
    }
    format!(
        "<aside class=\"d-post-accordion accepted-answers\" data-label-expand=\"{}\" data-label-collapse=\"{}\"><div class=\"d-post-accordion__layout\"><div class=\"d-post-accordion__header\">{header}</div><div class=\"d-post-accordion__items\">{items}</div></div></aside>",
        escape(&t(l, "expand")),
        escape(&t(l, "post.collapse"))
    )
}

/// The shared issue state the topic view serializes (`shared_issue_visible`,
/// `shared_issue_count`, `user_created_shared_issue`).
#[derive(Default, Clone)]
pub struct SharedIssue {
    pub visible: bool,
    pub count: i64,
    pub user_created: bool,
}

impl SharedIssue {
    pub fn from_view(view: &Value) -> SharedIssue {
        SharedIssue {
            visible: view["shared_issue_visible"] == true,
            count: view["shared_issue_count"].as_i64().unwrap_or(0),
            user_created: view["user_created_shared_issue"] == true,
        }
    }
}

/// SolvedSharedIssueButton, after the first post's cooked HTML: "Me too"
/// on an unsolved topic in a support category (or with multiple solutions
/// allowed), disabled for its author and on closed or archived topics.
pub fn shared_issue_button(cx: &PostContext, ui: &SolvedUi, p: &Value) -> String {
    let shared = &cx.topic.shared_issue;
    if !shared.visible || (!cx.topic.accepted_answers.is_empty() && !ui.allow_multiple) {
        return String::new();
    }
    let l = cx.list;
    let author = cx.viewer.is_some() && cx.viewer == p["username"].as_str();
    let disabled = author || cx.topic.closed || cx.topic.archived;
    let title_key = if author {
        "solved.shared_issue.author_title"
    } else if cx.topic.closed {
        "solved.shared_issue.closed_title"
    } else if cx.topic.archived {
        "solved.shared_issue.archived_title"
    } else {
        "solved.shared_issue.title"
    };
    let label = t(l, "solved.shared_issue.label");
    let label = if shared.count == 0 {
        label
    } else {
        crate::topic_list_view::t_with(
            l,
            "solved.shared_issue.label_with_count",
            &[("label", &label), ("count", &shared.count.to_string())],
        )
    };
    format!(
        "<div class=\"solved-shared-issue-row\"><button class=\"btn btn-icon-text btn-default post-action-menu__solved-shared-issue{}{}\"{} title=\"{}\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button></div>",
        if shared.user_created {
            " has-shared-issue"
        } else {
            ""
        },
        if disabled { " disabled" } else { "" },
        if disabled { " disabled=\"\"" } else { "" },
        escape(&t(l, title_key)),
        d_icon("hand", None),
        escape(&label)
    )
}
