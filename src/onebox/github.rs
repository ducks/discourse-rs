//! The GitHub engines (lib/onebox/engine/github_*_onebox.rb and the
//! GithubApi, GithubBody and GitBlobOnebox mixins) and GitlabBlobOnebox:
//! issues, pull requests, commits, repositories, gists, Actions runs and
//! files, rendered from GitHub's API in their Layout.

use std::sync::LazyLock;

use serde_json::{Map, Value, json};

use super::embed_engines::{CloneOk, clone_fail, engine_url, rendered};
use super::engines::{Ctx, Fail, Rendered, layout, link};
use super::fetch::{Fetch, FetchError};
use super::generic::{helpers_truncate, sanitize_default};
use super::matcher::Engine;
use super::ruby_time::{self, Fields};
use super::uri::RubyUri;
use crate::Unsupported;

const ISSUE: &str =
    include_str!("../../vendor/discourse/lib/onebox/templates/githubissue.mustache");
const COMMIT: &str =
    include_str!("../../vendor/discourse/lib/onebox/templates/githubcommit.mustache");
const REPO: &str = include_str!("../../vendor/discourse/lib/onebox/templates/githubrepo.mustache");
const GIST: &str = include_str!("../../vendor/discourse/lib/onebox/templates/githubgist.mustache");
const PULL: &str =
    include_str!("../../vendor/discourse/lib/onebox/templates/githubpullrequest.mustache");
const ACTIONS: &str =
    include_str!("../../vendor/discourse/lib/onebox/templates/githubactions.mustache");
const BLOB: &str = include_str!("../../vendor/discourse/lib/onebox/templates/githubblob.mustache");
const GITLAB_BLOB: &str =
    include_str!("../../vendor/discourse/lib/onebox/templates/gitlabblob.mustache");

fn raised(m: impl Into<String>) -> Fail {
    Fail::Raised(m.into())
}

/// `value[key]` on a parsed hash, raising as Ruby does on nil.
fn at<'v>(v: &'v Value, key: &str) -> Result<&'v Value, Fail> {
    match v {
        Value::Object(o) => Ok(o.get(key).unwrap_or(&Value::Null)),
        Value::Null => Err(raised(format!(
            "NoMethodError: undefined method `[]' for nil ({key})"
        ))),
        _ => Err(Unsupported("GitHub API values of another shape").into()),
    }
}

fn str_at(v: &Value, key: &str) -> Result<Option<String>, Fail> {
    Ok(match at(v, key)? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        _ => return Err(Unsupported("GitHub API values of another shape").into()),
    })
}

fn must_str(v: &Value, key: &str) -> Result<String, Fail> {
    str_at(v, key)?.ok_or_else(|| raised(format!("NoMethodError: {key} is nil")))
}

/// `Onebox::Mixins::GithubApi#load_json`: the body parsed, nil when
/// blank; an API error is the OpenURI::HTTPError Preview rescues.
async fn load_json<F: Fetch>(
    fetch: &mut F,
    ctx: &Ctx<'_>,
    url: &str,
    org: Option<&str>,
) -> Result<Value, Fail> {
    let token = ctx.settings.github_token(org);
    match fetch.github(url, token.as_deref()).await? {
        Ok(body) if crate::ruby::is_blank(&body) => Ok(Value::Null),
        Ok(body) => {
            serde_json::from_str(&body).map_err(|e| raised(format!("MultiJson::ParseError: {e}")))
        }
        Err(FetchError::Refused(u)) => Err(u.into()),
        Err(FetchError::Web(m)) => Err(Fail::Web(m)),
        Err(FetchError::Other(m)) => Err(raised(m)),
    }
}

/// `Onebox::Mixins::GithubBody#compute_body`: comments dropped, the first
/// 80 characters shown and the rest as the excerpt.
fn compute_body(body: Option<&str>) -> (Value, Value) {
    static COMMENT: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?s)<!--.*?-->").unwrap());
    let Some(body) = body else {
        return (Value::Null, Value::Null);
    };
    let body = COMMENT.replace_all(body, "");
    let body = crate::ruby::strip(&body).to_string();
    let chars: Vec<char> = body.chars().collect();
    if chars.is_empty() {
        return (Value::Null, Value::Null);
    }
    if chars.len() > 80 {
        let excerpt: String = chars[80..].iter().collect();
        let excerpt = excerpt.trim_end_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r', '\0']);
        return (
            json!(chars[..80].iter().collect::<String>()),
            json!(excerpt),
        );
    }
    (json!(body), Value::Null)
}

/// `Time.parse(s)`, refusing what the port doesn't read the same.
fn parse_time(s: &str) -> Result<Fields, Fail> {
    match ruby_time::parse(s)? {
        ruby_time::Parsed::Time(f) => Ok(f),
        ruby_time::Parsed::Raised => Err(raised(format!("ArgumentError: {s:?}"))),
    }
}

/// The `_at`, `_at_date` and `_at_time` fields of a GitHub timestamp.
fn time_fields(
    data: &mut Map<String, Value>,
    prefix: &str,
    f: Option<&Fields>,
) -> Result<(), Fail> {
    let fmt = |f: &Fields, p: &str| -> Result<Value, Fail> { Ok(json!(f.strftime(p)?)) };
    let (a, d, t) = match f {
        Some(f) => (
            fmt(f, "%I:%M%p - %d %b %y %Z")?,
            fmt(f, "%F")?,
            fmt(f, "%T")?,
        ),
        None => (Value::Null, Value::Null, Value::Null),
    };
    data.insert(prefix.to_string(), a);
    data.insert(format!("{prefix}_date"), d);
    data.insert(format!("{prefix}_time"), t);
    Ok(())
}

/// `"#{ulink.host}/#{org}/#{repo}"` of the link.
fn domain(link: &str) -> Result<String, Fail> {
    let uri = RubyUri::parse(link).ok_or_else(|| raised("URI::InvalidURIError"))?;
    let parts: Vec<&str> = uri.path.split('/').collect();
    Ok(format!(
        "{}/{}/{}",
        uri.host.unwrap_or_default(),
        parts.get(1).copied().unwrap_or(""),
        parts.get(2).copied().unwrap_or("")
    ))
}

fn t(ctx: &Ctx, key: &str) -> Value {
    ctx.i18n.t(key).map_or(Value::Null, |s| json!(s))
}

/// `Emoji.codes_to_img(Onebox::Helpers.sanitize(name))`
fn label_name(ctx: &Ctx, name: &Value) -> Result<Value, Fail> {
    let Value::String(name) = name else {
        return Ok(Value::Null);
    };
    if crate::ruby::is_blank(name) {
        return Ok(Value::Null);
    }
    let sanitized = crate::ruby::strip(&sanitize_default(name)?).to_string();
    if crate::ruby::is_blank(&sanitized) {
        return Ok(Value::Null);
    }
    Ok(json!(crate::category_badge::codes_to_img(
        &ctx.settings.emojis,
        &ctx.settings.base_path,
        &sanitized
    )))
}

static ISSUE_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?m)^http(?:s)?://(?:www\.)?(?:(?:\w)+\.)?github\.com/(?<org>.+)/(?<repo>.+)/(?<type>issues)/(?<item_id>[\d]+)",
    )
    .unwrap()
});

async fn issue<F: Fetch>(url: &str, ctx: &Ctx<'_>, fetch: &mut F) -> Result<Value, Fail> {
    let m = ISSUE_RE
        .captures(url)
        .ok_or_else(|| raised("NoMethodError: match"))?;
    let (org, repo, id) = (&m["org"], &m["repo"], &m["item_id"]);
    let api = format!("https://api.github.com/repos/{org}/{repo}/issues/{id}");
    let result = load_json(fetch, ctx, &api, Some(org)).await?;
    let repo_json = load_json(
        fetch,
        ctx,
        &format!("https://api.github.com/repos/{org}/{repo}"),
        Some(org),
    )
    .await?;
    let created = parse_time(&must_str(&result, "created_at")?)?;
    let closed = match str_at(&result, "closed_at")? {
        Some(c) => Some(parse_time(&c)?),
        None => None,
    };
    let (body, excerpt) = compute_body(str_at(&result, "body")?.as_deref());
    let link = link(url)?;
    let labels = match at(&result, "labels")? {
        Value::Array(ls) => ls
            .iter()
            .map(|l| Ok(json!({ "name": label_name(ctx, at(l, "name")?)? })))
            .collect::<Result<Vec<_>, Fail>>()?,
        Value::Null => return Err(raised("NoMethodError: labels is nil")),
        _ => return Err(Unsupported("GitHub labels of another shape").into()),
    };
    let user = at(&result, "user")?.clone();
    let user_id = super::embed_engines::rs(at(&user, "id")?)?;
    let mut data = Map::new();
    data.insert("link".into(), json!(url));
    data.insert("title".into(), at(&result, "title")?.clone());
    data.insert("body".into(), body);
    data.insert("excerpt".into(), excerpt);
    data.insert("labels".into(), json!(labels));
    data.insert("user".into(), user);
    time_fields(&mut data, "created_at", Some(&created))?;
    time_fields(&mut data, "closed_at", closed.as_ref())?;
    data.insert("closed_by".into(), at(&result, "closed_by")?.clone());
    data.insert(
        "avatar".into(),
        json!(format!(
            "https://avatars1.githubusercontent.com/u/{user_id}?v=2&s=96"
        )),
    );
    data.insert("domain".into(), json!(domain(&link)?));
    data.insert(
        "i18n".into(),
        json!({ "opened": t(ctx, "onebox.github.opened"), "closed": t(ctx, "onebox.github.closed") }),
    );
    data.insert("is_private".into(), at(&repo_json, "private")?.clone());
    // created_at etc. come in the hash's order, the title before them.
    reorder(
        &mut data,
        &[
            "link",
            "title",
            "body",
            "excerpt",
            "labels",
            "user",
            "created_at",
            "created_at_date",
            "created_at_time",
            "closed_at",
            "closed_at_date",
            "closed_at_time",
            "closed_by",
            "avatar",
            "domain",
            "i18n",
            "is_private",
        ],
    );
    Ok(Value::Object(data))
}

fn reorder(data: &mut Map<String, Value>, order: &[&str]) {
    let mut out = Map::new();
    for k in order {
        if let Some(v) = data.remove(*k) {
            out.insert((*k).to_string(), v);
        }
    }
    out.append(data);
    *data = out;
}

/// GithubCommitOnebox's match.
fn commit_match(url: &str) -> Option<(String, String, String)> {
    static PLAIN: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"github\.com/(?<org>[^/]+)/(?<repository>[^/]+)/commit/(?<sha>[^/]+)")
            .unwrap()
    });
    static PULL: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"github\.com/(?<org>[^/]+)/(?<repository>[^/]+)/pull/(?<pr>[^/]+)/commit/(?<sha>[^/]+)",
        )
        .unwrap()
    });
    PLAIN.captures(url).or_else(|| PULL.captures(url)).map(|c| {
        (
            c["org"].to_string(),
            c["repository"].to_string(),
            c["sha"].to_string(),
        )
    })
}

async fn commit<F: Fetch>(url: &str, ctx: &Ctx<'_>, fetch: &mut F) -> Result<Value, Fail> {
    let (org, repo, sha) = commit_match(url).ok_or_else(|| raised("NoMethodError: match"))?;
    let api = format!("https://api.github.com/repos/{org}/{repo}/commits/{sha}");
    let mut result = match load_json(fetch, ctx, &api, Some(&org)).await? {
        Value::Object(o) => o,
        Value::Null => return Err(raised("NoMethodError: raw is nil")),
        _ => return Err(Unsupported("GitHub API values of another shape").into()),
    };
    let commit = Value::Object(result.clone());
    let message = must_str(at(&commit, "commit")?, "message")?;
    // String#split("\n") drops trailing empty lines.
    let mut lines: Vec<&str> = message.split('\n').collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let Some(first) = lines.first() else {
        return Err(raised("NoMethodError: undefined method `join' for nil"));
    };
    result.insert("title".into(), json!(first));
    let (body, excerpt) = compute_body(Some(&lines[1..].join("\n")));
    result.insert("body".into(), body);
    result.insert("excerpt".into(), excerpt);
    let date = must_str(at(at(&commit, "commit")?, "committer")?, "date")?;
    time_fields(&mut result, "committed_at", Some(&parse_time(&date)?))?;
    let link = link(url)?;
    result.insert("link".into(), json!(link));
    result.insert("domain".into(), json!(domain(&link)?));
    result.insert(
        "i18n".into(),
        json!({ "committed": t(ctx, "onebox.github.committed") }),
    );
    Ok(Value::Object(result))
}

async fn repo<F: Fetch>(url: &str, ctx: &Ctx<'_>, fetch: &mut F) -> Result<Value, Fail> {
    static M: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"github\.com/(?<org>[^/]+)/(?<repository>[^/]+)").unwrap()
    });
    let c = M
        .captures(url)
        .ok_or_else(|| raised("NoMethodError: match"))?;
    let (org, name) = (c["org"].to_string(), c["repository"].to_string());
    let api = format!("https://api.github.com/repos/{org}/{name}");
    let mut result = match load_json(fetch, ctx, &api, Some(&org)).await? {
        Value::Object(o) => o,
        Value::Null => return Err(raised("NoMethodError: raw is nil")),
        _ => return Err(Unsupported("GitHub API values of another shape").into()),
    };
    result.insert("link".into(), json!(link(url)?));
    let full_name = super::embed_engines::rs(result.get("full_name").unwrap_or(&Value::Null))?;
    let mut title = format!("GitHub - {full_name}");
    let description = match result.get("description") {
        Some(Value::String(d)) if !crate::ruby::is_blank(d) => {
            title.push_str(&format!(": {}", helpers_truncate(d, 50)));
            d.clone()
        }
        _ => ctx
            .i18n
            .t_with("onebox.github.no_description", &[("repo", &full_name)])
            .unwrap_or_default(),
    };
    result.insert("description".into(), json!(description));
    result.insert("title".into(), json!(title));
    let private = result.get("private").cloned().unwrap_or(Value::Null);
    result.insert("is_private".into(), private);
    let hex: String = (0..16)
        .map(|_| format!("{:02x}", rand::random::<u8>()))
        .collect();
    result.insert(
        "thumbnail".into(),
        json!(format!(
            "https://opengraph.githubassets.com/{hex}/{org}/{name}"
        )),
    );
    Ok(Value::Object(result))
}

async fn gist<F: Fetch>(url: &str, ctx: &Ctx<'_>, fetch: &mut F) -> Result<Value, Fail> {
    static M: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"gist\.github\.com/([^/]+/)?(?<sha>[0-9a-f]+)").unwrap()
    });
    let sha = M
        .captures(url)
        .map(|c| c["sha"].to_string())
        .ok_or_else(|| raised("NoMethodError: match"))?;
    let api = format!("https://api.github.com/gists/{sha}");
    // The rate limit (an OpenURI::HTTPError) leaves no files.
    let raw = match load_json(fetch, ctx, &api, None).await {
        Ok(v) => Some(v),
        Err(Fail::Web(_)) => None,
        Err(e) => return Err(e),
    };
    let files: Vec<Value> = match &raw {
        None | Some(Value::Null) => Vec::new(),
        Some(r) => match at(r, "files")? {
            Value::Object(fs) => fs
                .values()
                .map(|f| {
                    let content = must_str(f, "content")?;
                    let mut lines: Vec<&str> = content.split('\n').collect();
                    while lines.last().is_some_and(|l| l.is_empty()) {
                        lines.pop();
                    }
                    Ok(json!({
                        "filename": at(f, "filename")?,
                        "language": at(f, "language")?,
                        "content": lines.iter().take(10).copied().collect::<Vec<_>>().join("\n"),
                        "truncated?": lines.len() > 10,
                    }))
                })
                .collect::<Result<_, Fail>>()?,
            Value::Null => return Err(raised("NoMethodError: files is nil")),
            _ => return Err(Unsupported("gists of another shape").into()),
        },
    };
    let truncated = files.len() > 3;
    Ok(json!({
        "title": "gist.github.com",
        "link": link(url)?,
        "gist_files": files.into_iter().take(3).collect::<Vec<_>>(),
        "truncated_files?": truncated,
        "i18n": {
            "truncated_file": t(ctx, "onebox.github.truncated_file"),
            "more_than_three_files": t(ctx, "onebox.github.more_than_three_files"),
            "show_original": t(ctx, "onebox.github.show_original"),
        },
    }))
}

/// `fetch_pr_status`: nil unless github_pr_status_enabled.
async fn pr_status<F: Fetch>(
    pr: &Value,
    api: &str,
    org: &str,
    ctx: &Ctx<'_>,
    fetch: &mut F,
) -> Result<Option<(String, Value)>, Fail> {
    if !ctx.settings.github_pr_status {
        return Ok(None);
    }
    let truthy = |v: &Value| !matches!(v, Value::Null | Value::Bool(false));
    if truthy(at(pr, "merged")?) {
        return Ok(Some(("merged".into(), at(pr, "merged_at")?.clone())));
    }
    if at(pr, "state")?.as_str() == Some("closed") {
        return Ok(Some(("closed".into(), at(pr, "closed_at")?.clone())));
    }
    if truthy(at(pr, "draft")?) {
        return Ok(Some(("draft".into(), at(pr, "created_at")?.clone())));
    }
    // `rescue StandardError => nil` around the reviews.
    let reviews = match load_json(fetch, ctx, &format!("{api}/reviews"), Some(org)).await {
        Ok(r) => r,
        Err(Fail::App(e)) => return Err(Fail::App(e)),
        Err(_) => return Ok(None),
    };
    let mut latest: Vec<(Value, Value, Value)> = Vec::new(); // (user id, state, submitted_at)
    if let Value::Array(rs) = &reviews {
        for r in rs {
            let id = r
                .get("user")
                .and_then(|u| u.get("id"))
                .cloned()
                .unwrap_or(Value::Null);
            let state = r.get("state").cloned().unwrap_or(Value::Null);
            if id.is_null() || !matches!(state.as_str(), Some("CHANGES_REQUESTED" | "APPROVED")) {
                continue;
            }
            let at = r.get("submitted_at").cloned().unwrap_or(Value::Null);
            let at_s = at
                .as_str()
                .ok_or(Unsupported("reviews without a submitted_at"))?;
            match latest.iter_mut().find(|(u, _, _)| *u == id) {
                Some(existing) => {
                    if existing.2.as_str().is_some_and(|e| at_s > e) {
                        *existing = (id, state, at);
                    }
                }
                None => latest.push((id, state, at)),
            }
        }
    } else if !reviews.is_null() {
        return Ok(None);
    }
    for state in ["CHANGES_REQUESTED", "APPROVED"] {
        let times: Vec<&str> = latest
            .iter()
            .filter(|(_, s, _)| s.as_str() == Some(state))
            .filter_map(|(_, _, t)| t.as_str())
            .collect();
        if let Some(max) = times.iter().max() {
            return Ok(Some((state.to_lowercase(), json!(max))));
        }
    }
    Ok(Some(("open".into(), at(pr, "created_at")?.clone())))
}

async fn pull_request<F: Fetch>(url: &str, ctx: &Ctx<'_>, fetch: &mut F) -> Result<Value, Fail> {
    static M: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"github\.com/(?<org>[^/]+)/(?<repository>[^/]+)/pull/(?<number>[^/]+)")
            .unwrap()
    });
    let c = M
        .captures(url)
        .ok_or_else(|| raised("NoMethodError: match"))?;
    let (org, name, number) = (
        c["org"].to_string(),
        c["repository"].to_string(),
        c["number"].to_string(),
    );
    let api = format!("https://api.github.com/repos/{org}/{name}/pulls/{number}");
    let raw = load_json(fetch, ctx, &api, Some(&org)).await?;
    let mut result = match &raw {
        Value::Object(o) => o.clone(),
        Value::Null => return Err(raised("NoMethodError: raw is nil")),
        _ => return Err(Unsupported("GitHub API values of another shape").into()),
    };
    let link = link(url)?;
    result.insert("link".into(), json!(link));
    let status = pr_status(&raw, &api, &org, ctx, fetch).await?;
    let status_name = status.as_ref().map(|(s, _)| s.clone());
    result.insert("pr_status".into(), json!(status_name));
    let key = status_name
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or("default".into());
    result.insert(
        "pr_status_title".into(),
        t(ctx, &format!("onebox.github.pr_title.{key}")),
    );
    let timestamp = match status.as_ref().map(|(_, t)| t) {
        Some(t) if !t.is_null() => t.clone(),
        _ => result.get("created_at").cloned().unwrap_or(Value::Null),
    };
    let Value::String(timestamp) = timestamp else {
        return Err(raised(
            "TypeError: no implicit conversion of nil into String",
        ));
    };
    time_fields(&mut result, "status_date", Some(&parse_time(&timestamp)?))?;
    let uri = RubyUri::parse(&link).ok_or_else(|| raised("URI::InvalidURIError"))?;
    let segments: Vec<String> = uri.path.split('/').map(str::to_string).collect();
    let link_org = segments.get(1).cloned().unwrap_or_default();
    let link_repo = segments.get(2).cloned().unwrap_or_default();
    result.insert(
        "domain".into(),
        json!(format!(
            "{}/{link_org}/{link_repo}",
            uri.host.clone().unwrap_or_default()
        )),
    );
    let (b, e) = compute_body(str_at(&raw, "body")?.as_deref());
    result.insert("body".into(), b);
    result.insert("excerpt".into(), e);
    static COMMIT: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"commits/([0-9a-fA-F]+)").unwrap());
    static COMMENT: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"#issuecomment-(\d+)").unwrap());
    static REVIEW: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"#discussion_r(\d+)").unwrap());
    let base = format!("https://api.github.com/repos/{org}/{name}");
    let commit = match COMMIT.captures(&link) {
        Some(m) => load_json(fetch, ctx, &format!("{base}/commits/{}", &m[1]), Some(&org)).await?,
        None => Value::Null,
    };
    result.insert("commit".into(), commit.clone());
    let truthy = |v: &Value| !matches!(v, Value::Null | Value::Bool(false));
    if truthy(&commit) {
        let message = must_str(at(&commit, "commit")?, "message")?;
        let rest: String = message.split_inclusive('\n').skip(1).collect();
        let (b, e) = compute_body(Some(&rest));
        result.insert("body".into(), b);
        result.insert("excerpt".into(), e);
    } else {
        let comment = match COMMENT.captures(&link) {
            Some(m) => {
                load_json(
                    fetch,
                    ctx,
                    &format!("{base}/issues/comments/{}", &m[1]),
                    Some(&org),
                )
                .await?
            }
            None => Value::Null,
        };
        result.insert("comment".into(), comment.clone());
        if truthy(&comment) {
            let (b, e) = compute_body(str_at(&comment, "body")?.as_deref());
            result.insert("body".into(), b);
            result.insert("excerpt".into(), e);
        } else {
            let review = match REVIEW.captures(&link) {
                Some(m) => {
                    load_json(
                        fetch,
                        ctx,
                        &format!("{base}/pulls/comments/{}", &m[1]),
                        Some(&org),
                    )
                    .await?
                }
                None => Value::Null,
            };
            result.insert("discussion".into(), review.clone());
            if truthy(&review) {
                let (b, e) = compute_body(str_at(&review, "body")?.as_deref());
                result.insert("body".into(), b);
                result.insert("excerpt".into(), e);
            } else {
                result.insert("pr".into(), json!(true));
            }
        }
    }
    result.insert("number".into(), json!(number));
    let n = |k: &str| super::embed_engines::rs(raw.get(k).unwrap_or(&Value::Null));
    let summary = ctx
        .i18n
        .t_with(
            "onebox.github.pr_summary",
            &[
                ("commits", &n("commits")?),
                ("changed_files", &n("changed_files")?),
                ("additions", &n("additions")?),
                ("deletions", &n("deletions")?),
            ],
        )
        .unwrap_or_default();
    let label_key = status_name
        .filter(|s| !s.is_empty())
        .unwrap_or("open".into());
    result.insert(
        "i18n".into(),
        json!({
            "opened": t(ctx, "onebox.github.opened"),
            "commit_by": t(ctx, "onebox.github.commit_by"),
            "comment_by": t(ctx, "onebox.github.comment_by"),
            "review_by": t(ctx, "onebox.github.review_by"),
            "status_date_label": t(ctx, &format!("onebox.github.status_date.{label_key}")),
            "pr_summary": summary,
        }),
    );
    let private = raw
        .get("base")
        .and_then(|b| b.get("repo"))
        .and_then(|r| r.get("private"))
        .cloned()
        .unwrap_or(Value::Null);
    result.insert("is_private".into(), private);
    let prefix = format!("{link_org}:");
    for side in ["base", "head"] {
        let Some(Value::Object(o)) = result.get_mut(side) else {
            return Err(raised(format!("NoMethodError: {side} is nil")));
        };
        let Some(Value::String(label)) = o.get("label").cloned() else {
            return Err(raised(format!("NoMethodError: {side}.label is nil")));
        };
        if let Some(rest) = label.strip_prefix(&prefix) {
            o.insert("label".into(), json!(rest));
        }
    }
    Ok(Value::Object(result))
}

async fn actions<F: Fetch>(url: &str, ctx: &Ctx<'_>, fetch: &mut F) -> Result<Value, Fail> {
    static RUN: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?m)^https?://(?:www\.)?(?:(?:\w)+\.)?github\.com/(?<org>.+)/(?<repo>.+)/actions/runs/(?<run_id>[0-9]+)",
        )
        .unwrap()
    });
    static PR_RUN: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?m)^https?://(?:www\.)?(?:(?:\w)+\.)?github\.com/(?<org>.+)/(?<repo>.+)/pull/(?<pr_id>[0-9]*)/checks\?check_run_id=(?<check_run_id>[0-9]+)",
        )
        .unwrap()
    });
    let (org, repo, api, pr) = if let Some(c) = PR_RUN.captures(url) {
        (
            c["org"].to_string(),
            c["repo"].to_string(),
            format!(
                "https://api.github.com/repos/{}/{}/check-runs/{}",
                &c["org"], &c["repo"], &c["check_run_id"]
            ),
            Some(c["pr_id"].to_string()),
        )
    } else if let Some(c) = RUN.captures(url) {
        (
            c["org"].to_string(),
            c["repo"].to_string(),
            format!(
                "https://api.github.com/repos/{}/{}/actions/runs/{}",
                &c["org"], &c["repo"], &c["run_id"]
            ),
            None,
        )
    } else {
        return Err(Unsupported("GitHub Actions URLs of another shape").into());
    };
    let result = load_json(fetch, ctx, &api, Some(&org)).await?;
    let status = match (
        str_at(&result, "status")?.as_deref(),
        str_at(&result, "conclusion")?.as_deref(),
    ) {
        (Some("completed"), Some("success")) => "success",
        (Some("completed"), Some("failure")) => "failure",
        (Some("in_progress"), _) => "pending",
        _ => "unknown",
    };
    let title = match pr {
        None => {
            let message = must_str(at(&result, "head_commit")?, "message")?;
            json!(message.split_inclusive('\n').next().unwrap_or(""))
        }
        Some(pr_id) => {
            let pr_url = format!("https://api.github.com/repos/{org}/{repo}/pulls/{pr_id}");
            at(&load_json(fetch, ctx, &pr_url, Some(&org)).await?, "title")?.clone()
        }
    };
    let mut data = Map::new();
    data.insert("link".into(), json!(url));
    data.insert("title".into(), title);
    data.insert("name".into(), at(&result, "name")?.clone());
    data.insert("run_number".into(), at(&result, "run_number")?.clone());
    data.insert(status.into(), json!(true));
    Ok(Value::Object(data))
}

// ---- GitBlobOnebox ----

/// `Onebox::FileTypeFinder.from_file_name`
fn file_type(name: &str) -> String {
    let lower = name.to_lowercase();
    const EXTENSIONLESS: &[(&str, &str)] = &[
        ("cmake.in", "cmake"),
        ("gruntfile", "js"),
        ("gulpfile", "js"),
        ("artisan", "php"),
        ("berksfile", "rb"),
        ("capfile", "rb"),
        ("cheffile", "rb"),
        ("cheffile.lock", "rb"),
        ("gemfile", "rb"),
        ("guardfile", "rb"),
        ("rakefile", "rb"),
        ("thorfile", "rb"),
        ("vagrantfile", "rb"),
        ("boxfile", "yaml"),
    ];
    const LONG: &[(&str, &str)] = &[
        (".bib", "tex"),
        (".html.hbs", "handlebars"),
        (".html.handlebars", "handlebars"),
        (".latex", "tex"),
        (".ru", "rb"),
        (".simplecov", "rb"),
        (".sty", "tex"),
    ];
    if let Some((_, t)) = EXTENSIONLESS.iter().find(|(n, _)| *n == lower) {
        return t.to_string();
    }
    if let Some((_, t)) = LONG.iter().find(|(e, _)| lower.ends_with(e)) {
        return t.to_string();
    }
    match lower.rfind('.') {
        Some(i) => lower[i + 1..].to_string(),
        None => String::new(),
    }
}

const MAX_LINES: i64 = 20;
const MAX_CHARS: usize = 5000;

/// `calc_range`
fn calc_range(from: Option<&str>, to: Option<&str>, size: i64) -> Map<String, Value> {
    static NUM: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\d+").unwrap());
    let num = |s: Option<&str>| {
        s.and_then(|s| NUM.find(s))
            .map(|m| crate::ruby::to_i(m.as_str()))
    };
    let (from, to) = (num(from), num(to));
    let range_provided = from.is_some() || to.is_some();
    let mut truncated = false;
    let mut from = from.unwrap_or(1);
    let mut to = to.unwrap_or(-1);
    let one_liner = to == -1 && range_provided;
    if one_liner {
        to = from;
    }
    if !range_provided {
        from = 1;
        to = MAX_LINES;
        truncated = size > MAX_LINES;
    }
    if from > to {
        std::mem::swap(&mut from, &mut to);
    }
    if from > size {
        from = 1;
    }
    if to > size {
        to = size;
    }
    let mut selected = 0;
    if one_liner {
        selected = from;
        from = (from - 10).max(1);
        to = (to + 10).min(size);
        if from > size {
            from = size;
        }
    }
    if to - from > MAX_LINES && !one_liner {
        truncated = true;
        to = from + MAX_LINES - 1;
    }
    let mut m = Map::new();
    m.insert("from".into(), json!(from));
    m.insert("from_minus_one".into(), json!(from - 1));
    m.insert("to".into(), json!(to));
    m.insert("one_liner".into(), json!(one_liner));
    m.insert("selected_one_liner".into(), json!(selected));
    m.insert("range_provided".into(), json!(range_provided));
    m.insert("truncated".into(), json!(truncated));
    m
}

/// `removeLeadingIndentation`
fn remove_leading_indentation(s: &str) -> String {
    let lines: Vec<&str> = s.split_inclusive('\n').collect();
    let mut min = 100usize;
    for l in &lines {
        let l = l.strip_suffix('\n').unwrap_or(l);
        let spaces = l.len() - l.trim_start_matches(' ').len();
        if l.chars().count() == spaces || l.is_empty() {
            continue;
        }
        if spaces <= 1 {
            min = spaces;
            break;
        }
        min = min.min(spaces);
    }
    // gsub(/^[ ]{min}/, ""): at the start of every line in the line.
    lines
        .iter()
        .map(|l| {
            let mut out = String::new();
            for part in l.split_inclusive('\n') {
                if part.len() >= min && part.as_bytes()[..min].iter().all(|b| *b == b' ') {
                    out.push_str(&part[min..]);
                } else {
                    out.push_str(part);
                }
            }
            out
        })
        .collect()
}

struct Blob {
    org_or_user: String,
    repo: String,
    sha1: String,
    file: String,
    from: Option<String>,
    to: Option<String>,
}

async fn blob<F: Fetch>(
    url: &str,
    ctx: &Ctx<'_>,
    fetch: &mut F,
    github: bool,
) -> Result<Value, Fail> {
    static GH: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?is)github\.com/(?<org>[^/]+)/(?<repo>[^/]+)/blob/(?<sha1>[^/]+)/(?<file>[^#]+)(#(L(?<from>[^-]*)(-L(?<to>.*))?))?",
        )
        .unwrap()
    });
    static GL: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?is)gitlab\.com/(?<org>[^/]+)/(?<repo>[^/]+)/blob/(?<sha1>[^/]+)/(?<file>[^#]+)(#(L(?<from>[^-]*)(-L(?<to>.*))?))?",
        )
        .unwrap()
    });
    let c = if github { &GH } else { &GL }
        .captures(url)
        .ok_or_else(|| raised("NoMethodError: match"))?;
    let m = Blob {
        org_or_user: c["org"].to_string(),
        repo: c["repo"].to_string(),
        sha1: c["sha1"].to_string(),
        file: c["file"].to_string(),
        from: c.name("from").map(|m| m.as_str().to_string()),
        to: c.name("to").map(|m| m.as_str().to_string()),
    };
    let link = link(url)?;
    let lang = file_type(&m.file);
    let raw_url = if github {
        format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}",
            m.org_or_user, m.repo, m.sha1, m.file
        )
    } else {
        format!(
            "https://gitlab.com/{}/{}/raw/{}/{}",
            m.org_or_user, m.repo, m.sha1, m.file
        )
    };
    let mut content = Value::Null;
    let mut binary = Value::Null;
    let mut lines = Value::Null;
    let mut cr_results = Value::Null;
    let mut truncated = Value::Null;
    let mut model_file = Value::Null;
    let mut selected_one_liner = json!(0);
    static GH_BLOB: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\Ahttps?://(www\.)?github\.com.*/blob/").unwrap());
    if lang == "stl" && GH_BLOB.is_match(&link) {
        model_file = json!(lang);
        content = json!(format!(
            "https://render.githubusercontent.com/view/solid?url={raw_url}"
        ));
    } else {
        let body = if github {
            match fetch
                .github(
                    &raw_url,
                    ctx.settings.github_token(Some(&m.org_or_user)).as_deref(),
                )
                .await?
            {
                Ok(b) => b,
                Err(FetchError::Refused(u)) => return Err(u.into()),
                Err(FetchError::Web(e)) => return Err(Fail::Web(e)),
                Err(FetchError::Other(e)) => return Err(raised(e)),
            }
        } else {
            match fetch.open(&raw_url, &[]).await? {
                Ok(b) => b,
                Err(FetchError::Refused(u)) => return Err(u.into()),
                Err(FetchError::Web(e)) => return Err(Fail::Web(e)),
                Err(FetchError::Other(e)) => return Err(raised(e)),
            }
        };
        if body.contains('\0') {
            binary = json!(true);
        } else {
            let all: Vec<&str> = body.split_inclusive('\n').collect();
            let size = all.len() as i64;
            let cr = calc_range(m.from.as_deref(), m.to.as_deref(), size);
            let from = cr["from"].as_i64().unwrap();
            let to = cr["to"].as_i64().unwrap();
            let range_provided = cr["range_provided"] == true;
            selected_one_liner = cr["selected_one_liner"].clone();
            let mut t = cr["truncated"].clone();
            // contents_lines[(from - 1)..(to - 1)]
            let slice = |from: i64, to: i64| -> Vec<&str> {
                let (a, b) = (from - 1, to - 1);
                if a < 0 || a > size || b < a - 1 {
                    return Vec::new();
                }
                let end = (b + 1).min(size).max(a);
                all[a as usize..end as usize].to_vec()
            };
            let mut text = if range_provided {
                let selected = selected_one_liner.as_i64().unwrap_or(0);
                let dedented = remove_leading_indentation(&slice(from, to).concat());
                let mut output = String::new();
                let mut array = Vec::new();
                for (i, line) in dedented.split_inclusive('\n').enumerate() {
                    let n = i as i64 + from;
                    array.push(json!({
                        "line_number": n,
                        "data": line.replace('\n', ""),
                        "selected": selected == n,
                    }));
                    output.push_str(&format!("{n}: {line}"));
                }
                lines = json!(array);
                output
            } else {
                slice(from, to).concat()
            };
            if text.chars().count() > MAX_CHARS {
                text = text.chars().take(MAX_CHARS + 1).collect();
                t = json!(true);
            }
            truncated = t;
            content = json!(text);
            cr_results = Value::Object(cr);
        }
    }
    let has_lines = !lines.is_null();
    let sha1 = if m.sha1.chars().count() == 40 {
        m.sha1.chars().take(9).collect()
    } else {
        m.sha1.clone()
    };
    let i18n = if github {
        json!({
            "binary_file": t(ctx, "onebox.github.binary_file"),
            "truncated_file": t(ctx, "onebox.github.truncated_file"),
            "show_original": t(ctx, "onebox.github.show_original"),
            "requires_iframe": t(ctx, "onebox.github.requires_iframe"),
        })
    } else {
        json!({
            "truncated_file": t(ctx, "onebox.gitlab.truncated_file"),
            "show_original": t(ctx, "onebox.gitlab.show_original"),
        })
    };
    Ok(json!({
        "title": sanitize_default(&m.file)?,
        "link": link,
        "i18n": i18n,
        "content": content,
        "binary": binary,
        "lang": format!("lang-{lang}"),
        "lines": lines,
        "has_lines": has_lines,
        "selected_one_liner": selected_one_liner,
        "cr_results": cr_results,
        "truncated": truncated,
        "model_file": model_file,
        "width": 480,
        "height": 360,
        "sha1": sha1,
        "domain": format!(
            "{}/{}/{}",
            if github { "github.com" } else { "gitlab.com" },
            m.org_or_user,
            m.repo
        ),
    }))
}

/// The GitHub engines' and GitlabBlobOnebox's previews: the Layout, as
/// HTML and as the placeholder.
pub async fn preview<F: Fetch>(
    engine: &Engine,
    url: &str,
    ctx: &Ctx<'_>,
    fetch: &mut F,
) -> Result<Rendered, Fail> {
    let url = engine_url(engine, url, false);
    let (name, view, data) = match engine.name {
        "GithubIssueOnebox" => ("githubissue", ISSUE, issue(&url, ctx, fetch).await),
        "GithubCommitOnebox" => ("githubcommit", COMMIT, commit(&url, ctx, fetch).await),
        "GithubRepoOnebox" => ("githubrepo", REPO, repo(&url, ctx, fetch).await),
        "GithubGistOnebox" => ("githubgist", GIST, gist(&url, ctx, fetch).await),
        "GithubPullRequestOnebox" => (
            "githubpullrequest",
            PULL,
            pull_request(&url, ctx, fetch).await,
        ),
        "GithubActionsOnebox" => ("githubactions", ACTIONS, actions(&url, ctx, fetch).await),
        "GithubBlobOnebox" => ("githubblob", BLOB, blob(&url, ctx, fetch, true).await),
        "GitlabBlobOnebox" => (
            "gitlabblob",
            GITLAB_BLOB,
            blob(&url, ctx, fetch, false).await,
        ),
        _ => return Err(Unsupported("onebox engines not ported").into()),
    };
    if let Err(Fail::App(_)) = &data {
        return data.map(|_| Rendered::default());
    }
    let html = data
        .as_ref()
        .map(|d| Some(layout(name, view, d)))
        .map_err(clone_fail);
    let placeholder = html.clone_ok();
    rendered(html, placeholder)
}

/// `inline_data` of the GitHub engines (the title of the issue, pull
/// request, commit or repository, read only with an access token for the
/// org); an error gives none, as Ruby's rescue does.
pub async fn inline_data<F: Fetch>(
    engine: &Engine,
    url: &str,
    ctx: &Ctx<'_>,
    fetch: &mut F,
) -> Result<Option<String>, crate::AppError> {
    let url = engine_url(engine, url, false);
    let result = inline_title(engine, &url, ctx, fetch).await;
    match result {
        Ok(t) => Ok(t),
        Err(Fail::App(e)) => Err(e),
        Err(_) => Ok(None),
    }
}

async fn inline_title<F: Fetch>(
    engine: &Engine,
    url: &str,
    ctx: &Ctx<'_>,
    fetch: &mut F,
) -> Result<Option<String>, Fail> {
    match engine.name {
        "GithubIssueOnebox" => {
            let m = ISSUE_RE.captures(url).ok_or_else(|| raised("match"))?;
            let (org, repo, id) = (
                m["org"].to_string(),
                m["repo"].to_string(),
                m["item_id"].to_string(),
            );
            if ctx.settings.github_token(Some(&org)).is_none() {
                return Ok(None);
            }
            let api = format!("https://api.github.com/repos/{org}/{repo}/issues/{id}");
            let r = load_json(fetch, ctx, &api, Some(&org)).await?;
            let title = super::embed_engines::rs(at(&r, "title")?)?;
            Ok(Some(format!(
                "{title} - Issue #{id} - {org}/{repo} - GitHub"
            )))
        }
        "GithubCommitOnebox" => {
            let (org, repo, sha) = commit_match(url).ok_or_else(|| raised("match"))?;
            if ctx.settings.github_token(Some(&org)).is_none() {
                return Ok(None);
            }
            let api = format!("https://api.github.com/repos/{org}/{repo}/commits/{sha}");
            let r = load_json(fetch, ctx, &api, Some(&org)).await?;
            let message = must_str(at(&r, "commit")?, "message")?;
            let first = message.split('\n').next().unwrap_or("").to_string();
            let sha: String = must_str(&r, "sha")?.chars().take(7).collect();
            Ok(Some(format!("{first} - {org}/{repo}@{sha} - GitHub")))
        }
        "GithubRepoOnebox" => {
            static M: LazyLock<regex::Regex> = LazyLock::new(|| {
                regex::Regex::new(r"github\.com/(?<org>[^/]+)/(?<repository>[^/]+)").unwrap()
            });
            let c = M.captures(url).ok_or_else(|| raised("match"))?;
            let (org, name) = (c["org"].to_string(), c["repository"].to_string());
            if ctx.settings.github_token(Some(&org)).is_none() {
                return Ok(None);
            }
            let api = format!("https://api.github.com/repos/{org}/{name}");
            let r = load_json(fetch, ctx, &api, Some(&org)).await?;
            let mut title = format!(
                "GitHub - {}",
                super::embed_engines::rs(at(&r, "full_name")?)?
            );
            if let Some(d) = str_at(&r, "description")?.filter(|d| !crate::ruby::is_blank(d)) {
                title.push_str(&format!(" - {}", helpers_truncate(&d, 50)));
            }
            Ok(Some(title))
        }
        "GithubPullRequestOnebox" => {
            if ctx.settings.github_pr_status {
                return Err(
                    Unsupported("inline pull requests with discourse-github's status").into(),
                );
            }
            static M: LazyLock<regex::Regex> = LazyLock::new(|| {
                regex::Regex::new(
                    r"github\.com/(?<org>[^/]+)/(?<repository>[^/]+)/pull/(?<number>[^/]+)",
                )
                .unwrap()
            });
            static SHA: LazyLock<regex::Regex> =
                LazyLock::new(|| regex::Regex::new(r"/commits?/([0-9a-fA-F]+)").unwrap());
            let c = M.captures(url).ok_or_else(|| raised("match"))?;
            let (org, name, number) = (
                c["org"].to_string(),
                c["repository"].to_string(),
                c["number"].to_string(),
            );
            if ctx.settings.github_token(Some(&org)).is_none() {
                return Ok(None);
            }
            if let Some(s) = SHA.captures(url) {
                let api = format!(
                    "https://api.github.com/repos/{org}/{name}/commits/{}",
                    &s[1]
                );
                let commit = load_json(fetch, ctx, &api, Some(&org)).await?;
                let message = must_str(at(&commit, "commit")?, "message")?;
                let first = message.split('\n').next().unwrap_or("").to_string();
                let sha: String = must_str(&commit, "sha")?.chars().take(7).collect();
                return Ok(Some(format!("{first} - {org}/{name}@{sha} - GitHub")));
            }
            let api = format!("https://api.github.com/repos/{org}/{name}/pulls/{number}");
            let pr = load_json(fetch, ctx, &api, Some(&org)).await?;
            let title = super::embed_engines::rs(at(&pr, "title")?)?;
            Ok(Some(format!(
                "{title} - Pull Request #{number} - {org}/{name} - GitHub"
            )))
        }
        _ => Ok(None),
    }
}
