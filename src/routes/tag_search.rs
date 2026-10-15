//! GET /tags/filter/search: TagsController#search through Tags::Search,
//! the tag choosers' search (the composer's mini-tag-chooser among them):
//! DiscourseTagging.filter_allowed_tags for the term and the chooser's
//! context, as TagsController.tag_counts_json lists tags, then the tags
//! the term matches but the chooser may not take (disabled, with why) and
//! the term itself when it names such a tag (`forbidden`).

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::tagging::{self, AllowedTag, FilterOpts, TagCategory};
use crate::{AppError, AppState, Unsupported};

/// The service's params, cast as its contract casts them.
struct SearchParams {
    q: Option<String>,
    raw_limit: Option<String>,
    limit: Option<i64>,
    category_id: Option<i64>,
    selected_tag_ids: Option<Vec<Value>>,
    selected_tags: Option<Vec<Value>>,
    filter_for_input: bool,
    exclude_synonyms: bool,
    exclude_has_synonyms: bool,
    prioritize_recent_tags: bool,
}

/// ActiveModel's boolean cast: false for the false values, nil for blank.
fn boolean(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(v) => {
            let s = crate::params::scalar(v).unwrap_or_default();
            !(s.is_empty() || matches!(s.to_lowercase().as_str(), "0" | "f" | "false" | "off"))
        }
    }
}

/// ActiveSupportTypeExtensions::Array: a string split on commas (or
/// pipes), a list as it is with integers parsed, anything else wrapped.
fn array(value: Option<&Value>) -> Option<Vec<Value>> {
    let value = value?;
    let cast = |items: Vec<Value>| -> Vec<Value> {
        items
            .into_iter()
            .map(|item| match &item {
                Value::String(s) => s.trim().parse::<i64>().map(|n| json!(n)).unwrap_or(item),
                _ => item,
            })
            .collect()
    };
    Some(match value {
        Value::Null => return None,
        Value::String(s) => {
            let sep = if s.contains(',') { ',' } else { '|' };
            cast(s.split(sep).map(|p| json!(p)).collect())
        }
        Value::Array(items) => cast(items.clone()),
        Value::Object(map) => cast(map.values().cloned().collect()),
        other => vec![other.clone()],
    })
}

impl SearchParams {
    fn parse(p: &Map<String, Value>) -> Self {
        let raw_limit = crate::params::string(p, "limit");
        SearchParams {
            q: crate::params::string(p, "q"),
            limit: raw_limit
                .as_deref()
                .filter(|l| !l.trim().is_empty())
                .and_then(|l| l.trim().parse::<i64>().ok()),
            raw_limit,
            category_id: crate::params::string(p, "categoryId")
                .and_then(|c| c.trim().parse::<i64>().ok()),
            selected_tag_ids: array(p.get("selected_tag_ids")),
            selected_tags: array(p.get("selected_tags")),
            filter_for_input: boolean(p.get("filterForInput")),
            exclude_synonyms: boolean(p.get("excludeSynonyms")),
            exclude_has_synonyms: boolean(p.get("excludeHasSynonyms")),
            prioritize_recent_tags: boolean(p.get("prioritizeRecentTags")),
        }
    }

    /// `limit_is_valid`: digits, at most max_tag_search_results.
    fn limit_valid(&self, settings: &SiteSettings) -> Result<bool, AppError> {
        let Some(raw) = self.raw_limit.as_deref().filter(|r| !r.trim().is_empty()) else {
            return Ok(true);
        };
        if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
            return Ok(false);
        }
        let value = crate::ruby::to_i(raw);
        Ok(value >= 0 && value <= settings.get("max_tag_search_results")?.to_i())
    }

    fn capped_limit(&self, settings: &SiteSettings) -> Result<Option<i64>, AppError> {
        Ok(match self.limit {
            Some(l) => Some(l.min(settings.get("max_tag_search_results")?.to_i())),
            None => None,
        })
    }
}

/// Selected tag ids as filter_allowed_tags reads them: `selected_tag_ids`
/// by `to_i`, else the ids of the `selected_tags` named.
async fn resolved_selected(
    conn: &mut PgConnection,
    params: &SearchParams,
) -> Result<Vec<i32>, AppError> {
    if let Some(ids) = params.selected_tag_ids.as_ref().filter(|v| !v.is_empty()) {
        return Ok(ids
            .iter()
            .map(|v| match v {
                Value::Number(n) => n.as_i64().unwrap_or(0) as i32,
                other => {
                    crate::ruby::to_i(&crate::params::scalar(other).unwrap_or_default()) as i32
                }
            })
            .collect());
    }
    if let Some(names) = params.selected_tags.as_ref().filter(|v| !v.is_empty()) {
        let lower: Vec<String> = names
            .iter()
            .filter_map(crate::params::scalar)
            .map(|n| n.to_lowercase())
            .collect();
        return Ok(sqlx::query_scalar(
            "SELECT id FROM tags WHERE lower(name) = ANY($1) ORDER BY ctid",
        )
        .bind(&lower)
        .fetch_all(&mut *conn)
        .await?);
    }
    Ok(Vec::new())
}

/// GET /tags/filter/search(.json)
pub async fn search(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = crate::params::parse_query(uri.query().unwrap_or(""));
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // ensure_tags_enabled
    if !settings.get("tagging_enabled")?.truthy() {
        return Ok(super::topics::not_found_response(&state));
    }
    if settings.get("content_localization_enabled")?.truthy() {
        return Err(Unsupported("tag search over localized tags").into());
    }
    let params = SearchParams::parse(&p);
    if !params.limit_valid(&settings)? {
        let message = format!(
            "Limit {}",
            state
                .i18n
                .t("activemodel.errors.messages.invalid")
                .or_else(|| state.i18n.t("errors.messages.invalid"))
                .unwrap_or("is invalid")
        );
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"failed": "FAILED", "errors": [message]})),
        )
            .into_response());
    }
    let term = match params.q.as_deref().filter(|q| !q.trim().is_empty()) {
        Some(q) => Some(tagging::clean_tag(&settings, q)?).filter(|t| !t.is_empty()),
        None => None,
    };
    // fetch_category: the category, when the user may see it.
    let category = match params.category_id {
        Some(id) => {
            let allowed = guardian.allowed_category_ids(&mut conn, &settings).await?;
            match i32::try_from(id) {
                Ok(id) if allowed.contains(&id) => TagCategory::load(&mut conn, id).await?,
                _ => None,
            }
        }
        None => None,
    };
    let selected = resolved_selected(&mut conn, &params).await?;
    let capped = params.capped_limit(&settings)?;
    let mut recent = Vec::new();
    if params.prioritize_recent_tags
        && term.is_none()
        && let Some(user_id) = guardian.user_id()
        && guardian
            .upcoming_change_enabled(&mut conn, &settings, "prioritize_recently_used_tags")
            .await?
    {
        recent = recently_used_by(&mut conn, user_id).await?;
    }
    let opts = FilterOpts {
        guardian: Some(&guardian),
        term: term.clone(),
        limit: capped,
        category: category.as_ref(),
        for_input: params.filter_for_input,
        selected_tag_ids: selected.clone(),
        exclude_synonyms: params.exclude_synonyms,
        exclude_has_synonyms: params.exclude_has_synonyms,
        order_search_results: term.is_some(),
        order_popularity: term.is_none(),
        order_recent_tag_ids: recent,
        ..Default::default()
    };
    let (rows, required) = tagging::filter_allowed_tags(&mut conn, &settings, &opts).await?;
    let mut results = tag_counts_json(&mut conn, &settings, &guardian, &rows).await?;

    // append_disabled_tags
    if let (Some(term), true) = (&term, params.filter_for_input) {
        let mut skip: Vec<i32> = rows.iter().map(|r| r.id).collect();
        skip.extend(selected.iter().copied());
        let visible = crate::tags::visible_tags_where(&guardian, &settings)?;
        let max = settings.get("max_tag_search_results")?.to_i();
        let candidates: Vec<(i32, String, Option<i32>)> = sqlx::query_as(&format!(
            "SELECT tags.id, tags.name, tags.target_tag_id FROM tags \
             WHERE {visible} AND tags.name ~* $1 AND NOT (tags.id = ANY($2)) ORDER BY tags.ctid LIMIT $3"
        ))
        .bind(format!("\\m{}", regex_escape(term)))
        .bind(&skip)
        .bind(max)
        .fetch_all(&mut *conn)
        .await?;
        if !candidates.is_empty() {
            let excluded =
                reject_allowed(&mut conn, &settings, &opts, capped, rows.len(), candidates).await?;
            for (id, name, target) in excluded {
                let reason = explain_exclusion(
                    &mut conn,
                    &state.i18n,
                    &settings,
                    &guardian,
                    &params,
                    (id, &name, target),
                    &selected,
                )
                .await?;
                results.push(json!({
                    "id": id,
                    "name": name,
                    "text": name,
                    "count": 0,
                    "disabled": true,
                    "title": reason,
                }));
            }
        }
    }

    // detect_forbidden_tag
    let mut forbidden: Option<(String, String)> = None;
    if let Some(term) = &term {
        let named = results.iter().any(|r| {
            r["name"]
                .as_str()
                .is_some_and(|n| n.to_lowercase() == term.to_lowercase())
        });
        if !named {
            let visible = crate::tags::visible_tags_where(&guardian, &settings)?;
            let tag: Option<(i32, String, Option<i32>)> = sqlx::query_as(&format!(
                "SELECT tags.id, tags.name, tags.target_tag_id FROM tags \
                 WHERE {visible} AND lower(tags.name) = lower($1) ORDER BY tags.id LIMIT 1"
            ))
            .bind(term)
            .fetch_optional(&mut *conn)
            .await?;
            if let Some(tag) = tag {
                let excluded = reject_allowed(
                    &mut conn,
                    &settings,
                    &opts,
                    capped,
                    rows.len(),
                    vec![tag.clone()],
                )
                .await?;
                if !excluded.is_empty() {
                    let message = explain_exclusion(
                        &mut conn,
                        &state.i18n,
                        &settings,
                        &guardian,
                        &params,
                        (tag.0, &tag.1, tag.2),
                        &selected,
                    )
                    .await?;
                    forbidden = Some((params.q.clone().unwrap_or_default(), message));
                }
            }
        }
    }

    let mut out = Map::new();
    out.insert("results".into(), Value::Array(results));
    if let Some((term, message)) = forbidden {
        out.insert("forbidden".into(), json!(term));
        out.insert("forbidden_message".into(), json!(message));
    }
    if let Some(group) = required {
        out.insert(
            "required_tag_group".into(),
            json!({"name": group.name, "min_count": group.min_count}),
        );
    }
    Ok(Json(Value::Object(out)).into_response())
}

/// `Regexp.escape`
fn regex_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '.' | '*' | '?' | '+' | '^' | '$' | '|' | '(' | ')' | '[' | ']' | '{' | '}' | '\\'
            | '-' | '#' | ' ' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// `Tag.recently_used_by(user)`: the tags of the user's ten newest regular
/// topics, most recently used first.
async fn recently_used_by(conn: &mut PgConnection, user_id: i32) -> Result<Vec<i32>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT topic_tags.tag_id FROM topic_tags \
         JOIN topics ON topics.id = topic_tags.topic_id \
         WHERE topic_tags.topic_id IN (SELECT id FROM topics WHERE user_id = $1 AND archetype = 'regular' \
                                       AND deleted_at IS NULL ORDER BY created_at DESC, id DESC LIMIT 10) \
         GROUP BY topic_tags.tag_id ORDER BY MAX(topics.created_at) DESC, MAX(topics.id) DESC",
    )
    .bind(user_id)
    .fetch_all(conn)
    .await
}

/// `reject_allowed_tags`: below the limit every candidate was left out;
/// at it, those the search would allow without a limit stay in.
async fn reject_allowed(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    opts: &FilterOpts<'_>,
    capped: Option<i64>,
    found: usize,
    candidates: Vec<(i32, String, Option<i32>)>,
) -> Result<Vec<(i32, String, Option<i32>)>, AppError> {
    let at_limit = capped.is_some_and(|c| found as i64 >= c);
    if !at_limit {
        return Ok(candidates);
    }
    let names: Vec<String> = candidates.iter().map(|(_, n, _)| n.clone()).collect();
    let unlimited = FilterOpts {
        guardian: opts.guardian,
        term: opts.term.clone(),
        limit: None,
        category: opts.category,
        for_input: opts.for_input,
        selected_tag_ids: opts.selected_tag_ids.clone(),
        only_tag_names: Some(names),
        exclude_synonyms: opts.exclude_synonyms,
        exclude_has_synonyms: opts.exclude_has_synonyms,
        order_search_results: opts.order_search_results,
        order_popularity: opts.order_popularity,
        ..Default::default()
    };
    let (allowed, _) = tagging::filter_allowed_tags(conn, settings, &unlimited).await?;
    Ok(candidates
        .into_iter()
        .filter(|(_, n, _)| !allowed.iter().any(|a| a.name == *n))
        .collect())
}

/// `explain_exclusion`: why the chooser may not take a tag.
async fn explain_exclusion(
    conn: &mut PgConnection,
    i18n: &crate::i18n::I18n,
    settings: &SiteSettings,
    guardian: &Guardian,
    params: &SearchParams,
    (id, name, target): (i32, &str, Option<i32>),
    selected: &[i32],
) -> Result<String, AppError> {
    let visible = crate::tags::visible_tags_where(guardian, settings)?;
    if params.exclude_synonyms
        && let Some(target_id) = target
        && tag_visible(conn, &visible, target_id).await?
    {
        let target_name: String = sqlx::query_scalar("SELECT name FROM tags WHERE id = $1")
            .bind(target_id)
            .fetch_one(&mut *conn)
            .await?;
        return Ok(i18n
            .t_with("tags.forbidden.synonym", &[("tag_name", &target_name)])
            .unwrap_or_default());
    }
    if params.exclude_has_synonyms {
        let has: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM tags WHERE target_tag_id = $1)")
                .bind(id)
                .fetch_one(&mut *conn)
                .await?;
        if has {
            return Ok(i18n
                .t_with("tags.forbidden.has_synonyms", &[("tag_name", name)])
                .unwrap_or_default());
        }
    }
    if !selected.is_empty() {
        let group: Option<(i32, String)> = sqlx::query_as(
            "SELECT tg.id, tg.name FROM tag_groups tg \
             JOIN tag_group_memberships tgm ON tgm.tag_group_id = tg.id \
             WHERE tg.one_per_topic AND tgm.tag_id = $1 \
               AND tg.id IN (SELECT tag_group_id FROM tag_group_memberships WHERE tag_id = ANY($2)) \
             ORDER BY tg.id LIMIT 1",
        )
        .bind(id)
        .bind(selected)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some((group_id, group_name)) = group {
            let names: Vec<String> = sqlx::query_scalar(&format!(
                "SELECT tags.name FROM tags JOIN tag_group_memberships tgm ON tgm.tag_id = tags.id \
                 WHERE {visible} AND tags.id = ANY($1) AND tgm.tag_group_id = $2 ORDER BY tags.name"
            ))
            .bind(selected)
            .bind(group_id)
            .fetch_all(&mut *conn)
            .await?;
            if !names.is_empty() {
                return Ok(i18n
                    .t_with(
                        "tags.forbidden.one_tag_per_topic_group",
                        &[
                            ("tag_group_name", &group_name),
                            ("tag_names", &names.join(", ")),
                        ],
                    )
                    .unwrap_or_default());
            }
            return Ok(i18n
                .t("tags.forbidden.one_tag_per_topic_group_without_names")
                .unwrap_or_default()
                .to_string());
        }
    }
    if params.filter_for_input {
        let group: Option<(String, i32)> = sqlx::query_as(
            "SELECT tg.name, tg.parent_tag_id FROM tag_groups tg \
             JOIN tag_group_memberships tgm ON tgm.tag_group_id = tg.id \
             WHERE tgm.tag_id = $1 AND tg.parent_tag_id IS NOT NULL AND NOT (tg.parent_tag_id = ANY($2)) \
             ORDER BY tg.id LIMIT 1",
        )
        .bind(id)
        .bind(selected)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some((group_name, parent_id)) = group {
            let parent: Option<String> = sqlx::query_scalar("SELECT name FROM tags WHERE id = $1")
                .bind(parent_id)
                .fetch_optional(&mut *conn)
                .await?;
            if let Some(parent) = parent
                && tag_visible(conn, &visible, parent_id).await?
            {
                return Ok(i18n
                    .t_with(
                        "tags.forbidden.missing_parent_tag",
                        &[
                            ("parent_tag_name", &parent),
                            ("tag_group_name", &group_name),
                        ],
                    )
                    .unwrap_or_default());
            }
        }
    }
    let allowed = guardian.allowed_category_ids(conn, settings).await?;
    let mut names: Vec<String> = sqlx::query_scalar(
        "SELECT categories.name FROM categories JOIN category_tags ct ON ct.category_id = categories.id \
         WHERE ct.tag_id = $1 AND categories.id = ANY($2) ORDER BY ct.ctid",
    )
    .bind(id)
    .bind(&allowed)
    .fetch_all(&mut *conn)
    .await?;
    names.extend(
        sqlx::query_scalar::<_, String>(
            "SELECT categories.name FROM categories \
             JOIN category_tag_groups ctg ON ctg.category_id = categories.id \
             JOIN tag_group_memberships tgm ON tgm.tag_group_id = ctg.tag_group_id \
             WHERE tgm.tag_id = $1 AND categories.id = ANY($2) ORDER BY ctg.ctid",
        )
        .bind(id)
        .bind(&allowed)
        .fetch_all(&mut *conn)
        .await?,
    );
    if !names.is_empty() {
        let mut unique: Vec<String> = Vec::new();
        for n in names {
            if !unique.contains(&n) {
                unique.push(n);
            }
        }
        unique.sort();
        return Ok(if unique.len() > 3 {
            i18n.t_with(
                "tags.forbidden.restricted_to_truncated",
                &[
                    ("tag_name", name),
                    ("category_names", &unique[..3].join(", ")),
                    ("more_count", &(unique.len() - 3).to_string()),
                ],
            )
            .unwrap_or_default()
        } else {
            i18n.t_count(
                "tags.forbidden.restricted_to",
                unique.len() as i64,
                &[("tag_name", name), ("category_names", &unique.join(", "))],
            )
            .unwrap_or_default()
        });
    }
    Ok(if params.category_id.is_some() {
        i18n.t_with("tags.forbidden.in_this_category", &[("tag_name", name)])
            .unwrap_or_default()
    } else {
        i18n.t_with("tags.forbidden.not_allowed", &[("tag_name", name)])
            .unwrap_or_default()
    })
}

/// `tag_visible?`: the tag is among the guardian's visible tags.
async fn tag_visible(
    conn: &mut PgConnection,
    visible: &str,
    tag_id: i32,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM tags WHERE {visible} AND tags.id = $1)"
    ))
    .bind(tag_id)
    .fetch_one(conn)
    .await
}

/// `TagsController.tag_counts_json` for filter_allowed_tags' rows.
pub async fn tag_counts_json(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    rows: &[AllowedTag],
) -> Result<Vec<Value>, AppError> {
    let show_pm = guardian.can_tag_pms(settings)?
        && settings
            .get("display_personal_messages_tag_counts")?
            .truthy();
    let target_ids: Vec<i32> = rows.iter().filter_map(|r| r.target_tag_id).collect();
    let targets: Vec<(i32, String, Option<String>)> = if target_ids.is_empty() {
        Vec::new()
    } else {
        let visible = crate::tags::visible_tags_where(guardian, settings)?;
        sqlx::query_as(&format!(
            "SELECT tags.id, tags.name, tags.slug FROM tags WHERE {visible} AND tags.id = ANY($1)"
        ))
        .bind(&target_ids)
        .fetch_all(&mut *conn)
        .await?
    };
    Ok(rows
        .iter()
        .map(|t| {
            let target = t.target_tag_id.map(|id| {
                targets
                    .iter()
                    .find(|(tid, _, _)| *tid == id)
                    .map(|(id, name, slug)| json!({"id": id, "name": name, "slug": slug}))
                    .unwrap_or(Value::Null)
            });
            let mut out = json!({
                "id": t.id,
                "text": t.name,
                "name": t.name,
                "slug": t.slug.as_deref().filter(|s| !s.is_empty()).map(str::to_string)
                    .unwrap_or_else(|| format!("{}-tag", t.id)),
                "description": t.description,
                "count": t.topic_count,
                "pm_only": t.topic_count == 0 && t.pm_topic_count > 0,
                "target_tag": target.unwrap_or(Value::Null),
            });
            if show_pm {
                out["pm_count"] = json!(t.pm_topic_count);
            }
            out
        })
        .collect())
}
