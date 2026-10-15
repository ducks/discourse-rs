//! `Search#process_advanced_search!`: the term's words, each tested against
//! the registered advanced filters (core's in lib/search.rb, then the
//! enabled plugins') and then the keyword chain; what neither claims stays
//! in the term.

use std::sync::OnceLock;

use regex::Regex;

use super::SearchError;
use crate::Unsupported;
use crate::site_settings::{SettingError, SiteSettings};

/// A registered `advanced_filter`, in registration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    PersonalDirect,
    AllPms,
    Tagged,
    Bots,
    Humans,
    Whispers,
    Regular,
    Untagged,
    StatusOpen,
    StatusClosed,
    StatusPublic,
    StatusArchived,
    StatusNoreplies,
    StatusSingleUser,
    PostsCount,
    MinPostCount,
    MinPosts,
    MaxPosts,
    First,
    Replies,
    Pinned,
    Wiki,
    Badge,
    Likes,
    Bookmarks,
    Posted,
    Created,
    CreatedBy,
    WatchingTracking,
    Seen,
    Unseen,
    WithImages,
    Category,
    Hashtag,
    Group,
    GroupMessages,
    User,
    At,
    Before,
    After,
    Tags,
    NotTags,
    Filetypes,
    MinViews,
    MaxViews,
    Locale,
    /// poll's `in:polls`
    Polls,
    /// discourse-solved's `status:solved` and `status:unsolved`
    Solved,
    Unsolved,
    /// discourse-topic-voting's `min_vote_count:`
    MinVoteCount,
}

/// The plugin a filter or order comes from, on when its setting is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plugin {
    Core,
    Poll,
    Solved,
    TopicVoting,
}

impl Plugin {
    fn enabled(self, settings: &SiteSettings) -> Result<bool, SettingError> {
        Ok(match self {
            Plugin::Core => true,
            Plugin::Poll => settings.get("poll_enabled")?.truthy(),
            Plugin::Solved => settings.get("solved_enabled")?.truthy(),
            Plugin::TopicVoting => settings.get("topic_voting_enabled")?.truthy(),
        })
    }
}

// Ruby's \d, \w and \S are ASCII; Rust's are Unicode. `^` and `$` are
// line anchors in Ruby (multi_line below).
const FILTERS: &[(Filter, Plugin, &str)] = &[
    (
        Filter::PersonalDirect,
        Plugin::Core,
        r"\Ain:personal-direct\z",
    ),
    (Filter::AllPms, Plugin::Core, r"\Ain:all-pms\z"),
    (Filter::Tagged, Plugin::Core, r"\Ain:tagged\z"),
    (Filter::Bots, Plugin::Core, r"\Ain:bots?\z"),
    (Filter::Humans, Plugin::Core, r"\Ain:humans?\z"),
    (Filter::Whispers, Plugin::Core, r"\Ain:whispers?\z"),
    (Filter::Regular, Plugin::Core, r"\Ain:regular\z"),
    (Filter::Untagged, Plugin::Core, r"\Ain:untagged\z"),
    (Filter::StatusOpen, Plugin::Core, r"\Astatus:open\z"),
    (Filter::StatusClosed, Plugin::Core, r"\Astatus:closed\z"),
    (Filter::StatusPublic, Plugin::Core, r"\Astatus:public\z"),
    (Filter::StatusArchived, Plugin::Core, r"\Astatus:archived\z"),
    (
        Filter::StatusNoreplies,
        Plugin::Core,
        r"\Astatus:noreplies\z",
    ),
    (
        Filter::StatusSingleUser,
        Plugin::Core,
        r"\Astatus:single_user\z",
    ),
    (
        Filter::PostsCount,
        Plugin::Core,
        r"\Aposts_count:([0-9]+)\z",
    ),
    (
        Filter::MinPostCount,
        Plugin::Core,
        r"\Amin_post_count:([0-9]+)\z",
    ),
    (Filter::MinPosts, Plugin::Core, r"\Amin_posts:([0-9]+)\z"),
    (Filter::MaxPosts, Plugin::Core, r"\Amax_posts:([0-9]+)\z"),
    (Filter::First, Plugin::Core, r"\Ain:first|^f\z"),
    (Filter::Replies, Plugin::Core, r"\Ain:replies\z"),
    (Filter::Pinned, Plugin::Core, r"\Ain:pinned\z"),
    (Filter::Wiki, Plugin::Core, r"\Ain:wiki\z"),
    (Filter::Badge, Plugin::Core, r"\Abadge:(.*)\z"),
    (Filter::Likes, Plugin::Core, r"\Ain:(likes)\z"),
    (Filter::Bookmarks, Plugin::Core, r"\Ain:(bookmarks)\z"),
    (Filter::Posted, Plugin::Core, r"\Ain:posted\z"),
    (Filter::Created, Plugin::Core, r"\Ain:(created|mine)\z"),
    (Filter::CreatedBy, Plugin::Core, r"\Acreated:@(.*)\z"),
    (
        Filter::WatchingTracking,
        Plugin::Core,
        r"\Ain:(watching|tracking)\z",
    ),
    (Filter::Seen, Plugin::Core, r"\Ain:seen\z"),
    (Filter::Unseen, Plugin::Core, r"\Ain:unseen\z"),
    (Filter::WithImages, Plugin::Core, r"\Awith:images\z"),
    (Filter::Category, Plugin::Core, r"\Acategor(?:y|ies):(.+)\z"),
    (
        Filter::Hashtag,
        Plugin::Core,
        r"\A\#([\p{L}\p{M}0-9\-:=]+)\z",
    ),
    (Filter::Group, Plugin::Core, r"\Agroup:(.+)\z"),
    (
        Filter::GroupMessages,
        Plugin::Core,
        r"\Agroup_messages:(.+)\z",
    ),
    (Filter::User, Plugin::Core, r"\Auser:(.+)\z"),
    (Filter::At, Plugin::Core, r"\A\@([^ \t\r\n\f\v]+)\z"),
    (Filter::Before, Plugin::Core, r"\Abefore:(.*)\z"),
    (Filter::After, Plugin::Core, r"\Aafter:(.*)\z"),
    (
        Filter::Tags,
        Plugin::Core,
        r"\Atags?:([\p{L}\p{M}0-9,\-_+]+)\z",
    ),
    (
        Filter::NotTags,
        Plugin::Core,
        r"\A\-tags?:([\p{L}\p{M}0-9,\-_+]+)\z",
    ),
    (
        Filter::Filetypes,
        Plugin::Core,
        r"\Afiletypes?:([a-zA-Z0-9,\-_]+)\z",
    ),
    (Filter::MinViews, Plugin::Core, r"\Amin_views:([0-9]+)\z"),
    (Filter::MaxViews, Plugin::Core, r"\Amax_views:([0-9]+)\z"),
    (Filter::Locale, Plugin::Core, r"\Alocale:([a-zA-Z0-9_-]+)\z"),
    (Filter::Polls, Plugin::Poll, r"in:polls"),
    (Filter::Solved, Plugin::Solved, r"status:solved"),
    (Filter::Unsolved, Plugin::Solved, r"status:unsolved"),
    (
        Filter::MinVoteCount,
        Plugin::TopicVoting,
        r"^min_vote_count:([0-9]+)$",
    ),
];

/// A keyword of the chain after the filters, tested on the raw word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Keyword {
    Order,
    InTitle,
    Topic,
    All,
    AllPosts,
    Personal,
    Messages,
    PersonalDirect,
    AllPms,
    GroupMessages,
    PersonalMessages,
    Include,
}

const KEYWORDS: &[(Keyword, &str)] = &[
    (Keyword::Order, r"\Aorder:[a-zA-Z0-9_]+\z"),
    (Keyword::InTitle, r"\Ain:title\z"),
    (Keyword::Topic, r"\Atopic:([0-9]+)\z"),
    (Keyword::All, r"\Ain:all\z"),
    (Keyword::AllPosts, r"\Ain:all-posts\z"),
    (Keyword::Personal, r"\Ain:personal\z"),
    (Keyword::Messages, r"\Ain:messages\z"),
    (Keyword::PersonalDirect, r"\Ain:personal-direct\z"),
    (Keyword::AllPms, r"\Ain:all-pms\z"),
    (Keyword::GroupMessages, r"\Agroup_messages:(.+)\z"),
    (Keyword::PersonalMessages, r"\Apersonal_messages:(.+)\z"),
    (Keyword::Include, r"\Ainclude:(invisible|unlisted)\z"),
];

fn build(pattern: &str) -> Regex {
    regex::RegexBuilder::new(pattern)
        .case_insensitive(true)
        .multi_line(true)
        .build()
        .expect("static search pattern")
}

fn filter_regexes() -> &'static [Regex] {
    static CELL: OnceLock<Vec<Regex>> = OnceLock::new();
    CELL.get_or_init(|| FILTERS.iter().map(|(_, _, p)| build(p)).collect())
}

fn keyword_regexes() -> &'static [Regex] {
    static CELL: OnceLock<Vec<Regex>> = OnceLock::new();
    CELL.get_or_init(|| KEYWORDS.iter().map(|(_, p)| build(p)).collect())
}

/// `process_advanced_search!`'s tokenizer: a run of non-quote, non-space
/// characters with an optional quoted phrase glued on; unbalanced quotes
/// vanish.
pub fn words(term: &str) -> Vec<String> {
    let chars: Vec<char> = term.chars().collect();
    let is_ws = |c: char| matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r');
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        while i < chars.len() && chars[i] != '"' && !is_ws(chars[i]) {
            i += 1;
        }
        if i < chars.len()
            && chars[i] == '"'
            && let Some(close) = chars[i + 1..].iter().position(|&c| c == '"')
            && close > 0
        {
            i += close + 2;
        }
        if i == start {
            i += 1;
            continue;
        }
        out.push(chars[start..i].iter().collect());
    }
    out
}

/// What the term asked for besides its words.
#[derive(Debug, Default, Clone)]
pub struct Advanced {
    /// The words left over, joined with single spaces.
    pub term: String,
    /// Each matched filter with its first capture (`$1`), in the order the
    /// words matched them.
    pub filters: Vec<(Filter, Option<String>)>,
    /// `@order`, as the symbol the keyword names.
    pub order: Option<String>,
    pub in_title: bool,
    /// `in:all-posts`: every matching post, not one per topic.
    pub all_posts: bool,
    /// `include:unlisted`, for those who can see unlisted topics.
    pub include_invisible: bool,
}

/// Parses the term. Filters and keywords not ported yet are refused.
pub fn process(
    term: &str,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
) -> Result<Advanced, SearchError> {
    let logged_in = guardian.user_id().is_some();
    let filter_res = filter_regexes();
    let keyword_res = keyword_regexes();
    let mut adv = Advanced::default();
    let mut kept = Vec::new();
    for word in words(term) {
        let cleaned: String = word.chars().filter(|c| *c != '"' && *c != '\'').collect();
        let mut found = false;
        for (i, (filter, plugin, _)) in FILTERS.iter().enumerate() {
            if !plugin.enabled(settings)? {
                continue;
            }
            if let Some(caps) = filter_res[i].captures(&cleaned) {
                adv.filters
                    .push((*filter, caps.get(1).map(|m| m.as_str().to_string())));
                found = true;
            }
        }
        if word == "l" {
            adv.order = Some("latest".into());
            continue;
        }
        if word == "r" {
            if logged_in {
                adv.order = Some("read".into());
            }
            continue;
        }
        if word == "t" {
            adv.in_title = true;
            continue;
        }
        let keyword = KEYWORDS
            .iter()
            .zip(keyword_res)
            .find(|(_, re)| re.is_match(&word))
            .map(|((k, _), _)| *k);
        match keyword {
            Some(Keyword::Order) => {
                adv.order = Some(word.to_lowercase().replace("order:", ""));
            }
            Some(Keyword::InTitle) => adv.in_title = true,
            Some(Keyword::AllPosts) => adv.all_posts = true,
            Some(Keyword::Include) => {
                if guardian.can_see_unlisted_topics() {
                    adv.include_invisible = true;
                }
            }
            Some(_) => return Err(Unsupported(unported_keyword(&word)).into()),
            None if found => {}
            None => kept.push(word),
        }
    }
    for (filter, _) in &adv.filters {
        if let Some(reason) = unported_filter(*filter) {
            return Err(Unsupported(reason).into());
        }
    }
    if adv.order.is_some() {
        return Err(Unsupported("advanced search orders (order:, l, r)").into());
    }
    adv.term = kept.join(" ");
    Ok(adv)
}

fn unported_keyword(_word: &str) -> &'static str {
    "advanced search keywords (topic:, in:all, in:personal, in:messages, \
     in:personal-direct, in:all-pms, group_messages:, personal_messages:)"
}

/// The filters not ported yet, and why.
fn unported_filter(filter: Filter) -> Option<&'static str> {
    match filter {
        Filter::Category
        | Filter::Hashtag
        | Filter::At
        | Filter::User
        | Filter::CreatedBy
        | Filter::Group
        | Filter::Badge
        | Filter::Tagged
        | Filter::Untagged
        | Filter::Bots
        | Filter::Humans
        | Filter::Whispers
        | Filter::Regular
        | Filter::First
        | Filter::Replies
        | Filter::Pinned
        | Filter::Wiki
        | Filter::Likes
        | Filter::Bookmarks
        | Filter::Posted
        | Filter::Created
        | Filter::WatchingTracking
        | Filter::Seen
        | Filter::Unseen
        | Filter::Polls
        | Filter::Tags
        | Filter::NotTags => None,
        Filter::PersonalDirect | Filter::AllPms | Filter::GroupMessages => {
            Some("personal message searches (in:personal-direct, in:all-pms, group_messages:)")
        }
        _ => Some(
            "advanced search filters status:, posts_count:, min_posts:, max_posts:, \
             min_views:, max_views:, min_vote_count:, with:images, filetypes:, before:, \
             after: and locale:",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_like_process_advanced_search() {
        assert_eq!(words("hello\tworld"), vec!["hello", "world"]);
        assert_eq!(words("foo\"bar baz\" x"), vec!["foo\"bar baz\"", "x"]);
        assert_eq!(words("\"a b\" c"), vec!["\"a b\"", "c"]);
        assert_eq!(words("a \" b"), vec!["a", "b"]);
    }

    #[test]
    fn filter_patterns_follow_ruby() {
        let re = filter_regexes();
        let find = |f: Filter| {
            let i = FILTERS.iter().position(|(g, _, _)| *g == f).unwrap();
            &re[i]
        };
        assert!(find(Filter::Hashtag).is_match("#general"));
        assert!(find(Filter::Hashtag).is_match("#=general:sub-general"));
        assert!(!find(Filter::Hashtag).is_match("#gen.eral"));
        assert!(find(Filter::Category).is_match("CATEGORIES:a,b"));
        assert!(find(Filter::First).is_match("in:firstly"));
        assert!(find(Filter::First).is_match("f"));
        assert!(find(Filter::Polls).is_match("xin:pollsy"));
        assert!(!find(Filter::PostsCount).is_match("posts_count:\u{661}"));
        assert!(find(Filter::MinVoteCount).is_match("min_vote_count:3"));
    }
}
