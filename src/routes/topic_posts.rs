//! GET /t/:topic_id/posts(.json): TopicsController#posts, a chunk of a
//! topic's posts by id (or after a post number, for megatopics), as the
//! post stream loads more of them (PostStream#loadIntoIdentityMap and
//! #fetchNextWindow). TopicViewPostsSerializer: the posts without the
//! stream, its gaps or the timeline lookup, then the suggested topics when
//! asked for, and the topic's id.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};

use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_view::{Options, TopicView, TopicViewError};
use crate::url::Urls;
use crate::{AppError, AppState};

/// What topics#posts reads of its params.
#[derive(Debug, Default, PartialEq)]
pub struct PostsParams {
    pub post_ids: Option<Vec<i32>>,
    /// `post_number` with `asc` (filter_post_number).
    pub filter_post_number: Option<(i64, bool)>,
    pub include_suggested: bool,
}

/// `ActiveRecord::Type::Boolean.new.deserialize`: blank is nil, the false
/// values are false, anything else true.
fn ar_boolean(value: Option<&str>) -> bool {
    match value {
        None | Some("") => false,
        Some(v) => !matches!(v.to_ascii_lowercase().as_str(), "0" | "f" | "false" | "off"),
    }
}

impl PostsParams {
    /// From the query string; `Err` names a param that is not ported.
    pub fn parse(query: &str) -> Result<Self, &'static str> {
        let p = crate::params::parse_query(query);
        for (key, what) in [
            ("filter", "topics#posts with a filter"),
            ("username_filters", "topics#posts with username filters"),
            ("include_raw", "topics#posts with include_raw"),
        ] {
            if p.get(key)
                .is_some_and(|v| v != &Value::String(String::new()))
            {
                return Err(what);
            }
        }
        // post_ids.present?: a list of ids, or one (Post.where(id: "5")).
        let post_ids = match p.get("post_ids") {
            Some(Value::Array(list)) => Some(
                list.iter()
                    .filter_map(crate::params::scalar)
                    .map(|s| crate::ruby::to_i(&s) as i32)
                    .collect::<Vec<_>>(),
            ),
            Some(Value::String(s)) if !s.is_empty() => Some(vec![crate::ruby::to_i(s) as i32]),
            _ => None,
        };
        let filter_post_number = crate::params::string(&p, "post_number")
            .filter(|s| !s.trim().is_empty())
            .map(|s| {
                (
                    crate::ruby::to_i(&s),
                    ar_boolean(crate::params::string(&p, "asc").as_deref()),
                )
            });
        Ok(Self {
            post_ids,
            filter_post_number,
            include_suggested: crate::params::string(&p, "include_suggested").as_deref()
                == Some("true"),
        })
    }
}

/// GET /t/{topic_id}/posts and /t/{topic_id}/posts.json
pub async fn posts(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(topic_id): Path<String>,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    // `constraints: { topic_id: /\d+/ }`: anything else matches no route.
    if topic_id.is_empty() || !topic_id.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(super::topics::not_found_response(&state));
    }
    let Ok(topic_id) = topic_id.parse::<i32>() else {
        return Ok(super::topics::not_found_response(&state));
    };
    let params = PostsParams::parse(uri.query().unwrap_or_default()).map_err(crate::Unsupported)?;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let mut view = TopicView {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        options: Options {
            post_ids: params.post_ids,
            filter_post_number: params.filter_post_number,
            include_suggested: Some(params.include_suggested),
            raise_access_errors: true,
            ..Default::default()
        },
        post_types: Vec::new(),
        logo_small_url: None,
    };
    let rendered = match view.render(topic_id).await {
        Ok(r) => r,
        Err(TopicViewError::NotFound) => return Ok(super::topics::not_found_response(&state)),
        // hide_inaccessible_topic_when_detailed_404_disabled: a topic the
        // viewer cannot see is not found unless detailed_404 is on.
        Err(TopicViewError::InvalidAccess) => {
            return Ok(if settings.get("detailed_404")?.truthy() {
                super::search::invalid_access(&state)
            } else {
                super::topics::not_found_response(&state)
            });
        }
        Err(TopicViewError::NotLoggedIn) => {
            return Ok(super::login_required::not_logged_in(&state));
        }
        Err(e) => return Err(e.into()),
    };
    Ok(Json(posts_json(rendered.json)).into_response())
}

/// TopicViewPostsSerializer from the full topic view: post_stream with its
/// posts alone, the related messages, suggested topics and group name the
/// view carries, then the id.
pub fn posts_json(mut view: Value) -> Value {
    let mut out = Map::new();
    let posts = view["post_stream"]["posts"].take();
    out.insert("post_stream".into(), serde_json::json!({ "posts": posts }));
    for key in [
        "related_messages",
        "suggested_topics",
        "suggested_group_name",
    ] {
        if let Some(value) = view.get_mut(key).map(Value::take) {
            out.insert(key.into(), value);
        }
    }
    out.insert("id".into(), view["id"].take());
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_what_the_post_stream_sends() {
        let p =
            PostsParams::parse("post_ids%5B%5D=5&post_ids%5B%5D=7&include_suggested=true").unwrap();
        assert_eq!(p.post_ids, Some(vec![5, 7]));
        assert!(p.include_suggested);
        assert_eq!(p.filter_post_number, None);

        let p = PostsParams::parse("post_number=30&asc=true&include_suggested=false").unwrap();
        assert_eq!(p.filter_post_number, Some((30, true)));
        assert!(!p.include_suggested);
        assert_eq!(
            PostsParams::parse("post_number=30")
                .unwrap()
                .filter_post_number,
            Some((30, false))
        );
        assert_eq!(
            PostsParams::parse("post_ids=9").unwrap().post_ids,
            Some(vec![9])
        );
        assert_eq!(PostsParams::parse("").unwrap(), PostsParams::default());
        assert!(PostsParams::parse("filter=summary").is_err());
    }

    #[test]
    fn active_record_booleans() {
        assert!(!ar_boolean(None));
        assert!(!ar_boolean(Some("")));
        assert!(!ar_boolean(Some("false")));
        assert!(!ar_boolean(Some("0")));
        assert!(!ar_boolean(Some("OFF")));
        assert!(ar_boolean(Some("true")));
        assert!(ar_boolean(Some("yes")));
    }

    #[test]
    fn keeps_the_posts_serializer_keys() {
        let view = serde_json::json!({
            "post_stream": {"posts": [{"id": 1}], "stream": [1, 2], "gaps": {}},
            "timeline_lookup": [[1, 0]],
            "suggested_topics": [],
            "id": 9,
            "title": "t",
        });
        assert_eq!(
            posts_json(view),
            serde_json::json!({"post_stream": {"posts": [{"id": 1}]}, "suggested_topics": [], "id": 9})
        );
    }
}
