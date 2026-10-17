//! The table reports: moderator activity, flags, post edits, logins,
//! uploads, ignored users, likes received, flagging ratios, topic views,
//! who's posting (posters by member type), activity by category, and
//! discourse-reactions' reactions.

#![allow(clippy::type_complexity)]

use std::collections::{HashMap, HashSet};

use chrono::{NaiveDate, NaiveDateTime};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::counts::{in_list, lit, parse_id_list};
use super::lists::ruby_float;
use super::{Cx, Report, Viewer, decimal_json, float_json, interp_time, round_to};
use crate::{AppError, Unsupported};

fn ms(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// `PostActionType.flag_types` (or those without an additional message)
/// from the flags table: name keys by id, in position order.
async fn flag_types(
    conn: &mut PgConnection,
    without_message: bool,
) -> Result<Vec<(i64, String)>, AppError> {
    let extra = if without_message {
        " AND NOT require_message"
    } else {
        ""
    };
    Ok(sqlx::query_as(&format!(
        "SELECT id, name_key FROM flags WHERE NOT score_type AND id <> 2{extra} ORDER BY position"
    ))
    .fetch_all(conn)
    .await?)
}

fn ids_sql(types: &[(i64, String)]) -> String {
    types
        .iter()
        .map(|(id, _)| id.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// `report_moderators_activity`
pub async fn moderators_activity(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.labels = json!([
        {"type": "user", "properties": {"username": "username", "id": "user_id", "avatar": "user_avatar_template"}, "title": cx.t("reports.moderators_activity.labels.moderator")?},
        {"property": "flag_count", "type": "number", "title": cx.t("reports.moderators_activity.labels.flag_count")?},
        {"type": "seconds", "property": "time_read", "title": cx.t("reports.moderators_activity.labels.time_read")?},
        {"property": "topic_count", "type": "number", "title": cx.t("reports.moderators_activity.labels.topic_count")?},
        {"property": "pm_count", "type": "number", "title": cx.t("reports.moderators_activity.labels.pm_count")?},
        {"property": "post_count", "type": "number", "title": cx.t("reports.moderators_activity.labels.post_count")?},
        {"property": "revision_count", "type": "number", "title": cx.t("reports.moderators_activity.labels.revision_count")?},
    ]);
    r.modes = vec!["table".into()];
    let flags = ids_sql(&flag_types(conn, true).await?);
    let s = interp_time(r.start_date);
    let e = interp_time(r.end_date);
    let sql = format!(
        "WITH mods AS ( \
           SELECT id AS user_id, username_lower AS username, uploaded_avatar_id \
           FROM users u WHERE u.moderator = 'true' AND u.id > 0 \
         ), time_read AS ( \
           SELECT SUM(uv.time_read) AS time_read, uv.user_id FROM mods m \
           JOIN user_visits uv ON m.user_id = uv.user_id \
           WHERE uv.visited_at >= '{s}' AND uv.visited_at <= '{e}' GROUP BY uv.user_id \
         ), flag_count AS ( \
           WITH period_actions AS ( \
             SELECT agreed_by_id, disagreed_by_id, deferred_by_id FROM post_actions \
             WHERE post_action_type_id IN ({flags}) AND created_at >= '{s}' AND created_at <= '{e}' \
           ), agreed_flags AS ( \
             SELECT pa.agreed_by_id AS user_id, COUNT(*) AS flag_count FROM mods m \
             JOIN period_actions pa ON pa.agreed_by_id = m.user_id GROUP BY agreed_by_id \
           ), disagreed_flags AS ( \
             SELECT pa.disagreed_by_id AS user_id, COUNT(*) AS flag_count FROM mods m \
             JOIN period_actions pa ON pa.disagreed_by_id = m.user_id GROUP BY disagreed_by_id \
           ), deferred_flags AS ( \
             SELECT pa.deferred_by_id AS user_id, COUNT(*) AS flag_count FROM mods m \
             JOIN period_actions pa ON pa.deferred_by_id = m.user_id GROUP BY deferred_by_id \
           ) \
           SELECT COALESCE(af.user_id, df.user_id, def.user_id) AS user_id, \
             COALESCE(af.flag_count, 0) + COALESCE(df.flag_count, 0) + COALESCE(def.flag_count, 0) AS flag_count \
           FROM agreed_flags af \
           FULL OUTER JOIN disagreed_flags df ON df.user_id = af.user_id \
           FULL OUTER JOIN deferred_flags def ON def.user_id = af.user_id \
         ), revision_count AS ( \
           SELECT pr.user_id, COUNT(*) AS revision_count FROM mods m \
           JOIN post_revisions pr ON pr.user_id = m.user_id JOIN posts p ON p.id = pr.post_id \
           WHERE pr.created_at >= '{s}' AND pr.created_at <= '{e}' AND p.user_id <> pr.user_id \
           GROUP BY pr.user_id \
         ), topic_count AS ( \
           SELECT t.user_id, COUNT(*) AS topic_count FROM mods m JOIN topics t ON t.user_id = m.user_id \
           WHERE t.archetype = 'regular' AND t.created_at >= '{s}' AND t.created_at <= '{e}' \
           GROUP BY t.user_id \
         ), post_count AS ( \
           SELECT p.user_id, COUNT(*) AS post_count FROM mods m JOIN posts p ON p.user_id = m.user_id \
           JOIN topics t ON t.id = p.topic_id \
           WHERE t.archetype = 'regular' AND p.created_at >= '{s}' AND p.created_at <= '{e}' \
           GROUP BY p.user_id \
         ), pm_count AS ( \
           SELECT p.user_id, COUNT(*) AS pm_count FROM mods m JOIN posts p ON p.user_id = m.user_id \
           JOIN topics t ON t.id = p.topic_id \
           WHERE t.archetype = 'private_message' AND p.created_at >= '{s}' AND p.created_at <= '{e}' \
           GROUP BY p.user_id \
         ) \
         SELECT m.user_id, m.username, m.uploaded_avatar_id, tr.time_read, fc.flag_count, \
           rc.revision_count, tc.topic_count, pc.post_count, pmc.pm_count \
         FROM mods m \
         LEFT JOIN time_read tr ON tr.user_id = m.user_id \
         LEFT JOIN flag_count fc ON fc.user_id = m.user_id \
         LEFT JOIN revision_count rc ON rc.user_id = m.user_id \
         LEFT JOIN topic_count tc ON tc.user_id = m.user_id \
         LEFT JOIN post_count pc ON pc.user_id = m.user_id \
         LEFT JOIN pm_count pmc ON pmc.user_id = m.user_id \
         ORDER BY m.username"
    );
    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        i32,
        String,
        Option<i32>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    )> = sqlx::query_as(&sql).fetch_all(&mut *conn).await?;
    let mut data = Vec::new();
    for (id, username, avatar, time_read, flag, revision, topic, post, pm) in rows {
        data.push(json!({
            "username": username,
            "user_id": id,
            "user_avatar_template": cx.avatar_template(id, &username, avatar)?,
            "time_read": time_read,
            "flag_count": flag,
            "revision_count": revision,
            "topic_count": topic,
            "post_count": post,
            "pm_count": pm,
        }));
    }
    r.data = Value::Array(data);
    Ok(())
}

/// `report_flags_status`
pub async fn flags_status(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["table".into()];
    r.labels = json!([
        {"type": "post", "properties": {"topic_id": "topic_id", "number": "post_number", "truncated_raw": "post_type"}, "title": cx.t("reports.flags_status.labels.flag")?},
        {"type": "user", "properties": {"username": "staff_username", "id": "staff_id", "avatar": "staff_avatar_template"}, "title": cx.t("reports.flags_status.labels.assigned")?},
        {"type": "user", "properties": {"username": "poster_username", "id": "poster_id", "avatar": "poster_avatar_template"}, "title": cx.t("reports.flags_status.labels.poster")?},
        {"type": "user", "properties": {"username": "flagger_username", "id": "flagger_id", "avatar": "flagger_avatar_template"}, "title": cx.t("reports.flags_status.labels.flagger")?},
        {"type": "seconds", "property": "response_time", "title": cx.t("reports.flags_status.labels.time_to_resolution")?},
    ]);
    let types = flag_types(conn, false).await?;
    let s = interp_time(r.start_date);
    let e = interp_time(r.end_date);
    let sql = format!(
        "WITH period_actions AS ( \
           SELECT id, post_action_type_id, created_at, agreed_at, disagreed_at, deferred_at, \
             agreed_by_id, disagreed_by_id, deferred_by_id, post_id, user_id, \
             COALESCE(disagreed_at, agreed_at, deferred_at) AS responded_at \
           FROM post_actions WHERE post_action_type_id IN ({}) \
             AND created_at >= '{s}' AND created_at <= '{e}' ORDER BY created_at DESC \
         ), poster_data AS ( \
           SELECT pa.id, p.user_id AS poster_id, p.topic_id as topic_id, p.post_number as post_number, \
             u.username_lower AS poster_username, u.uploaded_avatar_id AS poster_avatar_id \
           FROM period_actions pa JOIN posts p ON p.id = pa.post_id JOIN users u ON u.id = p.user_id \
         ), flagger_data AS ( \
           SELECT pa.id, u.id AS flagger_id, u.username_lower AS flagger_username, \
             u.uploaded_avatar_id AS flagger_avatar_id \
           FROM period_actions pa JOIN users u ON u.id = pa.user_id \
         ), staff_data AS ( \
           SELECT pa.id, u.id AS staff_id, u.username_lower AS staff_username, \
             u.uploaded_avatar_id AS staff_avatar_id \
           FROM period_actions pa JOIN users u \
             ON u.id = COALESCE(pa.agreed_by_id, pa.disagreed_by_id, pa.deferred_by_id) \
         ) \
         SELECT sd.staff_username, sd.staff_id, sd.staff_avatar_id, pd.poster_username, pd.poster_id, \
           pd.poster_avatar_id, pd.post_number, pd.topic_id, fd.flagger_username, fd.flagger_id, \
           fd.flagger_avatar_id, pa.post_action_type_id, pa.created_at, pa.agreed_at, pa.disagreed_at, \
           pa.deferred_at, pa.agreed_by_id, pa.disagreed_by_id, pa.deferred_by_id, \
           COALESCE(pa.disagreed_at, pa.agreed_at, pa.deferred_at) AS responded_at \
         FROM period_actions pa \
         FULL OUTER JOIN staff_data sd ON sd.id = pa.id \
         FULL OUTER JOIN flagger_data fd ON fd.id = pa.id \
         FULL OUTER JOIN poster_data pd ON pd.id = pa.id",
        ids_sql(&types)
    );
    let rows = sqlx::query(&sql).fetch_all(&mut *conn).await?;
    use sqlx::Row;
    let mut data = Vec::new();
    for row in rows {
        let mut m = Map::new();
        let type_id: Option<i32> = row.try_get("post_action_type_id")?;
        let name = types
            .iter()
            .find(|(id, _)| Some(*id) == type_id.map(i64::from))
            .map(|(_, n)| n.clone())
            .unwrap_or_default();
        m.insert("post_type".into(), json!(name));
        m.insert(
            "post_number".into(),
            json!(row.try_get::<Option<i32>, _>("post_number")?),
        );
        m.insert(
            "topic_id".into(),
            json!(row.try_get::<Option<i32>, _>("topic_id")?),
        );
        for who in ["staff", "poster", "flagger"] {
            let id: Option<i32> = row.try_get(format!("{who}_id").as_str())?;
            let Some(id) = id else { continue };
            let username: String = row.try_get(format!("{who}_username").as_str())?;
            let avatar: Option<i32> = row.try_get(format!("{who}_avatar_id").as_str())?;
            let avatar = cx.avatar_template(id, &username, avatar)?;
            if who == "flagger" {
                m.insert("flagger_id".into(), json!(id));
                m.insert("flagger_username".into(), json!(username));
            } else {
                m.insert(format!("{who}_username"), json!(username));
                m.insert(format!("{who}_id"), json!(id));
            }
            m.insert(format!("{who}_avatar_template"), json!(avatar));
        }
        let agreed: Option<i32> = row.try_get("agreed_by_id")?;
        let disagreed: Option<i32> = row.try_get("disagreed_by_id")?;
        let deferred: Option<i32> = row.try_get("deferred_by_id")?;
        let resolution = if agreed.is_some() {
            "agreed"
        } else if disagreed.is_some() {
            "disagreed"
        } else if deferred.is_some() {
            "deferred"
        } else {
            "no_action"
        };
        m.insert(
            "resolution".into(),
            json!(cx.t(&format!("reports.flags_status.values.{resolution}"))?),
        );
        let created: Option<NaiveDateTime> = row.try_get("created_at")?;
        let responded: Option<NaiveDateTime> = row.try_get("responded_at")?;
        let response = match (responded, created) {
            (Some(r), Some(c)) => {
                let d = r - c;
                float_json(d.num_microseconds().unwrap_or(0) as f64 / 1_000_000.0)
            }
            _ => Value::Null,
        };
        m.insert("response_time".into(), response);
        data.push(Value::Object(m));
    }
    r.data = Value::Array(data);
    Ok(())
}

/// `secure_category(ids)` over `c`.
fn secure_category(ids: &[i32]) -> String {
    if ids.is_empty() {
        "NOT COALESCE(c.read_restricted, false)".into()
    } else {
        format!(
            "NOT COALESCE(c.read_restricted, false) OR c.id in {}",
            in_list(&ids.iter().map(|i| i64::from(*i)).collect::<Vec<_>>())
        )
    }
}

/// `report_post_edits`
pub async fn post_edits(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    let filter = r.add_category_filter();
    let editor = r.filter_str("editor");
    r.modes = vec!["table".into()];
    r.labels = json!([
        {"type": "date", "property": "created_at", "title": cx.t("reports.post_edits.labels.edited_at")?},
        {"type": "post", "properties": {"topic_id": "topic_id", "number": "post_number", "truncated_raw": "post_raw"}, "title": cx.t("reports.post_edits.labels.post")?},
        {"type": "user", "properties": {"username": "editor_username", "id": "editor_id", "avatar": "editor_avatar_template"}, "title": cx.t("reports.post_edits.labels.editor")?},
        {"type": "user", "properties": {"username": "author_username", "id": "author_id", "avatar": "author_avatar_template"}, "title": cx.t("reports.post_edits.labels.author")?},
        {"type": "text", "property": "edit_reason", "title": cx.t("reports.post_edits.labels.edit_reason")?},
    ]);
    let mut joins = vec!["JOIN topics t ON t.id = p.topic_id".to_string()];
    let mut wheres: Vec<String> = Vec::new();
    if let Some((id, include)) = filter {
        let ids = super::category_ids(conn, cx, Some((id, include))).await?;
        if include {
            wheres.push(format!(
                "t.category_id IN {}",
                in_list(&ids.unwrap_or_default())
            ));
        } else {
            wheres.push(format!("t.category_id = {id}"));
        }
    }
    if !viewer.is_admin() {
        wheres.push("t.archetype != 'private_message'".into());
        joins.push("JOIN categories c ON c.id = t.category_id".into());
        // Guardian.new(report.current_user).secure_category_ids
        let ids = match viewer.guardian {
            Some(g) => g.secure_category_ids(&mut *conn, cx.settings).await?,
            None => Vec::new(),
        };
        wheres.push(secure_category(&ids));
    }
    if editor.is_some() {
        wheres.push("editor.username = $1".into());
    } else {
        wheres.push("editor.id > 0 AND editor.id != author.id".into());
    }
    wheres.push(format!("pr.created_at >= {}", lit(r.start_date)));
    wheres.push(format!("pr.created_at <= {}", lit(r.end_date)));
    let sql = format!(
        "SELECT pr.user_id AS editor_id, editor.username AS editor_username, \
           editor.uploaded_avatar_id AS editor_avatar_id, p.user_id AS author_id, \
           author.username AS author_username, author.uploaded_avatar_id AS author_avatar_id, \
           pr.number AS revision_version, p.version AS post_version, pr.post_id, \
           LEFT(p.raw, 40) AS post_raw, p.topic_id, p.post_number, p.edit_reason, pr.created_at \
         FROM post_revisions pr JOIN posts p ON p.id = pr.post_id \
         JOIN users author ON author.id = p.user_id JOIN users editor ON editor.id = pr.user_id \
         {} WHERE {} ORDER BY pr.created_at ASC",
        joins.join(" "),
        wheres
            .iter()
            .map(|w| format!("({w})"))
            .collect::<Vec<_>>()
            .join(" AND ")
    );
    let mut q = sqlx::query(&sql);
    if let Some(e) = &editor {
        q = q.bind(e);
    }
    let rows = q.fetch_all(&mut *conn).await?;
    use sqlx::Row;
    let mut data = Vec::new();
    for row in rows {
        let editor_id: i32 = row.try_get("editor_id")?;
        let editor_username: String = row.try_get("editor_username")?;
        let author_id: i32 = row.try_get("author_id")?;
        let author_username: String = row.try_get("author_username")?;
        let revision: Option<i32> = row.try_get("revision_version")?;
        let version: i32 = row.try_get("post_version")?;
        let reason: Option<String> = row.try_get("edit_reason")?;
        let created: NaiveDateTime = row.try_get("created_at")?;
        data.push(json!({
            "editor_id": editor_id,
            "editor_username": editor_username,
            "editor_avatar_template": cx.avatar_template(editor_id, &editor_username, row.try_get("editor_avatar_id")?)?,
            "author_id": author_id,
            "author_username": author_username,
            "author_avatar_template": cx.avatar_template(author_id, &author_username, row.try_get("author_avatar_id")?)?,
            "edit_reason": if revision == Some(version) { reason } else { None },
            "created_at": ms(created),
            "post_raw": row.try_get::<Option<String>, _>("post_raw")?,
            "topic_id": row.try_get::<i32, _>("topic_id")?,
            "post_number": row.try_get::<i32, _>("post_number")?,
        }));
    }
    r.data = Value::Array(data);
    Ok(())
}

/// `DiscourseIpInfo.get(ip)[:location]`: no MaxMind database here.
fn location(cx: &Cx<'_>) -> Result<Value, AppError> {
    let globals = &cx.urls.config.globals;
    if globals.get("maxmind_license_key").is_some() || globals.get("maxmind_backup_path").is_some()
    {
        return Err(Unsupported("IP locations from a MaxMind database").into());
    }
    Ok(Value::Null)
}

/// `report_admin_logins`
pub async fn admin_logins(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["table".into()];
    r.labels = json!([
        {"type": "user", "properties": {"username": "username", "id": "user_id", "avatar": "avatar_template"}, "title": cx.t("reports.admin_logins.labels.user")?},
        {"property": "location", "title": cx.t("reports.admin_logins.labels.location")?},
        {"property": "created_at", "type": "precise_date", "title": cx.t("reports.admin_logins.labels.login_at")?},
    ]);
    let admins: Vec<i32> = sqlx::query_scalar("SELECT id FROM users WHERE admin")
        .fetch_all(&mut *conn)
        .await?;
    let limit = r.limit.unwrap_or(20);
    let sql = format!(
        "SELECT t1.created_at created_at, t1.client_ip client_ip, u.username username, \
           u.uploaded_avatar_id uploaded_avatar_id, u.id user_id \
         FROM ( \
           SELECT DISTINCT ON (t.client_ip, t.user_id) t.client_ip, t.user_id, t.created_at \
           FROM user_auth_token_logs t \
           WHERE t.user_id IN ({}) AND t.created_at >= {} AND t.created_at <= {} \
           ORDER BY t.client_ip, t.user_id, t.created_at DESC LIMIT {limit} \
         ) t1 JOIN users u ON u.id = t1.user_id ORDER BY created_at DESC",
        admins
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(","),
        lit(r.start_date),
        lit(r.end_date)
    );
    let rows: Vec<(
        Option<NaiveDateTime>,
        Option<String>,
        String,
        Option<i32>,
        i32,
    )> = sqlx::query_as(&sql.replace("t1.client_ip client_ip", "host(t1.client_ip) client_ip"))
        .fetch_all(&mut *conn)
        .await?;
    let mut data = Vec::new();
    for (created, _ip, username, avatar, id) in rows {
        data.push(json!({
            "avatar_template": cx.avatar_template(id, &username, avatar)?,
            "user_id": id,
            "username": username,
            "location": location(cx)?,
            "created_at": created.map(ms),
        }));
    }
    r.data = Value::Array(data);
    Ok(())
}

/// `report_suspicious_logins`
pub async fn suspicious_logins(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["table".into()];
    r.labels = json!([
        {"type": "user", "properties": {"username": "username", "id": "user_id", "avatar": "avatar_template"}, "title": cx.t("reports.suspicious_logins.labels.user")?},
        {"property": "client_ip", "title": cx.t("reports.suspicious_logins.labels.client_ip")?},
        {"property": "location", "title": cx.t("reports.suspicious_logins.labels.location")?},
        {"property": "browser", "title": cx.t("reports.suspicious_logins.labels.browser")?},
        {"property": "device", "title": cx.t("reports.suspicious_logins.labels.device")?},
        {"property": "os", "title": cx.t("reports.suspicious_logins.labels.os")?},
        {"type": "date", "property": "login_time", "title": cx.t("reports.suspicious_logins.labels.login_time")?},
    ]);
    let rows: Vec<(
        i32,
        String,
        Option<i32>,
        Option<String>,
        Option<String>,
        Option<NaiveDateTime>,
    )> = sqlx::query_as(&format!(
        "SELECT u.id user_id, u.username, u.uploaded_avatar_id, host(t.client_ip), t.user_agent, \
               t.created_at login_time \
             FROM user_auth_token_logs t JOIN users u ON u.id = t.user_id \
             WHERE t.action = 'suspicious' AND t.created_at >= {} AND t.created_at <= {} \
             ORDER BY t.created_at DESC",
        lit(r.start_date),
        lit(r.end_date)
    ))
    .fetch_all(&mut *conn)
    .await?;
    let can_see_ip = viewer.can_see_ip(cx.settings)?.unwrap_or(false);
    let mut data = Vec::new();
    for (id, username, avatar, ip, ua, at) in rows {
        let (browser, device, os) =
            crate::user_private::browser_detection(ua.as_deref().unwrap_or(""));
        let mut m = Map::new();
        m.insert("username".into(), json!(username));
        m.insert("user_id".into(), json!(id));
        m.insert(
            "avatar_template".into(),
            json!(cx.avatar_template(id, &username, avatar)?),
        );
        if can_see_ip {
            m.insert("client_ip".into(), json!(ip.unwrap_or_default()));
        }
        m.insert("location".into(), location(cx)?);
        m.insert(
            "browser".into(),
            json!(cx.t(&format!("browsers.{browser}"))?),
        );
        m.insert(
            "device".into(),
            json!(cx.t(&format!("user_auth_tokens.device.{device}"))?),
        );
        m.insert(
            "os".into(),
            json!(cx.t(&format!("user_auth_tokens.os.{os}"))?),
        );
        m.insert("login_time".into(), json!(at.map(ms)));
        data.push(Value::Object(m));
    }
    r.data = Value::Array(data);
    Ok(())
}

/// `String#truncate(n)`: n characters with "..." at the end.
fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let kept: String = s.chars().take(n.saturating_sub(3)).collect();
    format!("{kept}...")
}

/// `report_top_uploads`
pub async fn top_uploads(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["table".into()];
    let extension = r.filter_str("file_extension");
    let mut choices: Vec<String> = cx
        .settings
        .get("authorized_extensions")?
        .to_s()
        .split('|')
        .map(str::to_string)
        .collect();
    if let Some(e) = &extension {
        choices.push(e.clone());
    }
    let mut unique: Vec<String> = Vec::new();
    for c in choices {
        if !unique.contains(&c) {
            unique.push(c);
        }
    }
    r.add_filter(
        "file_extension",
        json!({"type": "list", "default": extension.clone().unwrap_or_else(|| "any".into()), "choices": unique}),
    );
    r.labels = json!([
        {"type": "link", "properties": ["file_url", "file_name"], "title": cx.t("reports.top_uploads.labels.filename")?},
        {"type": "user", "properties": {"username": "author_username", "id": "author_id", "avatar": "author_avatar_template"}, "title": cx.t("reports.top_uploads.labels.author")?},
        {"type": "text", "property": "extension", "title": cx.t("reports.top_uploads.labels.extension")?},
        {"type": "bytes", "property": "filesize", "title": cx.t("reports.top_uploads.labels.filesize")?},
    ]);
    // Discourse.store.cdn_url: the external store's s3_cdn_url; the local
    // store's CDN is refused.
    let store = crate::file_store::FileStore::for_site(cx.urls.config, cx.settings)?;
    if store.internal() && cx.urls.config.globals.cdn_url().is_some() {
        return Err(Unsupported("top uploads' urls behind a CDN").into());
    }
    let limit = r.limit.unwrap_or(250);
    let mut wheres = vec![
        "up.id > 0".to_string(),
        format!("up.created_at >= {}", lit(r.start_date)),
        format!("up.created_at < {}", lit(r.end_date)),
        "username != 'system'".to_string(),
    ];
    if extension.is_some() {
        wheres.push("up.extension = $1".into());
    }
    let sql = format!(
        "SELECT u.id as user_id, u.username, u.uploaded_avatar_id, up.filesize, up.original_filename, \
           up.extension, up.url \
         FROM uploads up JOIN users u ON u.id = up.user_id \
         WHERE {} ORDER BY up.filesize DESC LIMIT {limit}",
        wheres
            .iter()
            .map(|w| format!("({w})"))
            .collect::<Vec<_>>()
            .join(" AND ")
    );
    let mut q = sqlx::query_as::<
        _,
        (
            i32,
            String,
            Option<i32>,
            i64,
            String,
            Option<String>,
            String,
        ),
    >(&sql);
    if let Some(e) = &extension {
        q = q.bind(e.strip_prefix('.').unwrap_or(e).to_string());
    }
    let rows = q.fetch_all(&mut *conn).await?;
    let mut data = Vec::new();
    for (id, username, avatar, size, name, ext, url) in rows {
        data.push(json!({
            "author_id": id,
            "author_username": username,
            "author_avatar_template": cx.avatar_template(id, &username, avatar)?,
            "filesize": size,
            "extension": ext,
            "file_url": store.cdn_url(&url),
            "file_name": truncate(&name, 25),
        }));
    }
    r.data = Value::Array(data);
    Ok(())
}

/// `report_top_ignored_users`
pub async fn top_ignored_users(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["table".into()];
    r.labels = json!([
        {"type": "user", "properties": {"id": "ignored_user_id", "username": "ignored_username", "avatar": "ignored_user_avatar_template"}, "title": cx.t("reports.top_ignored_users.labels.ignored_user")?},
        {"type": "number", "properties": ["ignores_count"], "title": cx.t("reports.top_ignored_users.labels.ignores_count")?},
        {"type": "number", "properties": ["mutes_count"], "title": cx.t("reports.top_ignored_users.labels.mutes_count")?},
    ]);
    let s = interp_time(r.start_date);
    let e = interp_time(r.end_date);
    let limit = r.limit.unwrap_or(250);
    let sql = format!(
        "WITH ignored_users AS ( \
           SELECT ignored_user_id as user_id, COUNT(*) AS ignores_count FROM ignored_users \
           WHERE created_at >= '{s}' AND created_at <= '{e}' GROUP BY ignored_user_id \
           ORDER BY COUNT(*) DESC LIMIT {limit} \
         ), muted_users AS ( \
           SELECT muted_user_id as user_id, COUNT(*) AS mutes_count FROM muted_users \
           WHERE created_at >= '{s}' AND created_at <= '{e}' GROUP BY muted_user_id \
           ORDER BY COUNT(*) DESC LIMIT {limit} \
         ) \
         SELECT u.id as user_id, u.username as username, u.uploaded_avatar_id as uploaded_avatar_id, \
           ig.ignores_count as ignores_count, COALESCE(mu.mutes_count, 0) as mutes_count, \
           ig.ignores_count + COALESCE(mu.mutes_count, 0) as total \
         FROM users as u JOIN ignored_users as ig ON ig.user_id = u.id \
         LEFT OUTER JOIN muted_users as mu ON mu.user_id = u.id ORDER BY total DESC"
    );
    let rows: Vec<(i32, String, Option<i32>, i64, i64, i64)> =
        sqlx::query_as(&sql).fetch_all(&mut *conn).await?;
    let mut data = Vec::new();
    for (id, username, avatar, ignores, mutes, _) in rows {
        data.push(json!({
            "ignored_user_id": id,
            "ignored_username": username,
            "ignored_user_avatar_template": cx.avatar_template(id, &username, avatar)?,
            "ignores_count": ignores,
            "mutes_count": mutes,
        }));
    }
    r.data = Value::Array(data);
    Ok(())
}

/// The three likes received tables' shared shape.
async fn likes_table(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
    kind: &str,
    sql: &str,
    trust_level: bool,
) -> Result<(), AppError> {
    r.icon = Some("heart".into());
    r.modes = vec!["table".into()];
    r.dates_filtering = true;
    let mut labels = vec![json!({
        "type": "user",
        "properties": {"id": "user_id", "username": "username", "avatar": "user_avatar_template"},
        "title": cx.t(&format!("reports.{kind}.labels.user"))?,
    })];
    if trust_level {
        labels.push(json!({"type": "number", "property": "trust_level", "title": cx.t(&format!("reports.{kind}.labels.trust_level"))?}));
    }
    labels.push(json!({"type": "number", "property": "qtt_like", "title": cx.t(&format!("reports.{kind}.labels.qtt_like"))?}));
    r.labels = Value::Array(labels);
    let rows = sqlx::query(sql)
        .bind(super::bind_time(r.start_date))
        .bind(super::bind_time(r.end_date))
        .fetch_all(&mut *conn)
        .await?;
    use sqlx::Row;
    let mut data = Vec::new();
    for row in rows {
        let id: i32 = row.try_get("user_id")?;
        let username: String = row.try_get("username")?;
        let avatar: Option<i32> = row.try_get("uploaded_avatar_id")?;
        let mut m = Map::new();
        m.insert("user_id".into(), json!(id));
        m.insert("username".into(), json!(username));
        m.insert(
            "user_avatar_template".into(),
            json!(cx.avatar_template(id, &username, avatar)?),
        );
        if trust_level {
            m.insert(
                "trust_level".into(),
                json!(row.try_get::<i32, _>("trust_level")?),
            );
        }
        m.insert("qtt_like".into(), json!(row.try_get::<i64, _>("qtt_like")?));
        data.push(Value::Object(m));
    }
    r.data = Value::Array(data);
    Ok(())
}

/// `report_top_users_by_likes_received`
pub async fn top_users_by_likes_received(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    likes_table(
        conn,
        cx,
        r,
        "top_users_by_likes_received",
        "SELECT ua.user_id AS user_id, u.username as username, u.uploaded_avatar_id as uploaded_avatar_id, \
           COUNT(*) qtt_like \
         FROM user_actions ua INNER JOIN users u on ua.user_id = u.id \
         WHERE ua.created_at::date BETWEEN $1 AND $2 AND ua.action_type = 2 \
         GROUP BY ua.user_id, u.username, u.uploaded_avatar_id ORDER BY qtt_like DESC LIMIT 10",
        false,
    )
    .await
}

/// `report_top_users_by_likes_received_from_a_variety_of_people`
pub async fn likes_from_a_variety(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    likes_table(
        conn,
        cx,
        r,
        "top_users_by_likes_received_from_a_variety_of_people",
        "SELECT p.user_id, u.username as username, u.uploaded_avatar_id as uploaded_avatar_id, \
           COUNT(DISTINCT ua.user_id) qtt_like \
         FROM user_actions ua INNER JOIN posts p ON p.id = ua.target_post_id \
         INNER JOIN users u on p.user_id = u.id \
         WHERE ua.created_at::date BETWEEN $1 AND $2 AND ua.action_type = 1 AND p.user_id > 0 \
         GROUP BY p.user_id, u.username, u.uploaded_avatar_id ORDER BY qtt_like DESC LIMIT 10",
        false,
    )
    .await
}

/// `report_top_users_by_likes_received_from_inferior_trust_level`
pub async fn likes_from_inferior_trust_level(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    likes_table(
        conn,
        cx,
        r,
        "top_users_by_likes_received_from_inferior_trust_level",
        "WITH user_liked_tl_lower AS ( \
           SELECT users.id user_id, users.username as username, \
             users.uploaded_avatar_id as uploaded_avatar_id, users.trust_level, COUNT(*) qtt_like, \
             rank() OVER (PARTITION BY users.trust_level ORDER BY COUNT(*) DESC) \
           FROM users INNER JOIN posts p ON p.user_id = users.id \
           INNER JOIN user_actions ua ON ua.target_post_id = p.id AND ua.action_type = 1 \
           INNER JOIN users u_liked ON ua.user_id = u_liked.id AND u_liked.trust_level < users.trust_level \
           WHERE ua.created_at::date BETWEEN $1 AND $2 \
           GROUP BY users.id ORDER BY trust_level DESC, qtt_like DESC \
         ) SELECT * FROM user_liked_tl_lower WHERE rank <= 10",
        true,
    )
    .await
}

/// `ReviewableScore.statuses`
const AGREED: i32 = 1;
const DISAGREED: i32 = 2;
const IGNORED: i32 = 3;

/// `report_user_flagging_ratio`
pub async fn user_flagging_ratio(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["table".into()];
    r.dates_filtering = true;
    r.labels = json!([
        {"type": "user", "properties": {"username": "username", "id": "user_id", "avatar": "avatar_template"}, "title": cx.t("reports.user_flagging_ratio.labels.user")?},
        {"type": "number", "property": "disagreed_flags", "title": cx.t("reports.user_flagging_ratio.labels.disagreed_flags")?},
        {"type": "number", "property": "agreed_flags", "title": cx.t("reports.user_flagging_ratio.labels.agreed_flags")?},
        {"type": "number", "property": "ignored_flags", "title": cx.t("reports.user_flagging_ratio.labels.ignored_flags")?},
        {"type": "number", "property": "score", "title": cx.t("reports.user_flagging_ratio.labels.score")?},
    ]);
    let agreed = format!("SUM(CASE WHEN rs.status = {AGREED} THEN 1 ELSE 0 END)::numeric");
    let disagreed = format!("SUM(CASE WHEN rs.status = {DISAGREED} THEN 1 ELSE 0 END)::numeric");
    let ignored = format!("SUM(CASE WHEN rs.status = {IGNORED} THEN 1 ELSE 0 END)::numeric");
    let sql = format!(
        "SELECT u.id, u.username, u.uploaded_avatar_id as avatar_id, \
           CASE WHEN u.silenced_till IS NOT NULL THEN 't' ELSE 'f' END as silenced, \
           ({disagreed})::text AS disagreed_flags, ({agreed})::text AS agreed_flags, \
           ({ignored})::text AS ignored_flags, \
           (CASE {disagreed} WHEN 0 THEN {agreed} * {agreed} \
             ELSE ROUND((1-({agreed} / {disagreed})) * ({disagreed} - {agreed})) END)::text AS score \
         FROM users AS u INNER JOIN reviewable_scores AS rs ON rs.user_id = u.id \
         WHERE u.id > 0 AND rs.created_at >= $1 AND rs.created_at <= $2 \
         GROUP BY u.id, u.username, u.uploaded_avatar_id, u.silenced_till \
         ORDER BY CASE {disagreed} WHEN 0 THEN {agreed} * {agreed} \
             ELSE ROUND((1-({agreed} / {disagreed})) * ({disagreed} - {agreed})) END DESC \
         LIMIT 100"
    );
    let rows: Vec<(
        i32,
        String,
        Option<i32>,
        String,
        String,
        String,
        String,
        String,
    )> = sqlx::query_as(&sql)
        .bind(super::bind_time(r.start_date))
        .bind(super::bind_time(r.end_date))
        .fetch_all(&mut *conn)
        .await?;
    let mut data = Vec::new();
    for (id, username, avatar, _, dis, agr, ign, score) in rows {
        data.push(json!({
            "user_id": id,
            "username": username,
            "avatar_template": cx.avatar_template(id, &username, avatar)?,
            "disagreed_flags": decimal_json(&dis),
            "ignored_flags": decimal_json(&ign),
            "agreed_flags": decimal_json(&agr),
            "score": decimal_json(&score),
        }));
    }
    r.data = Value::Array(data);
    Ok(())
}

/// `report_topic_view_stats`
pub async fn topic_view_stats(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["table".into()];
    let filter = r.add_category_filter();
    // [category_id] (nil when none) or its subcategories.
    let ids: Vec<i64> = match filter {
        Some((id, true)) => super::subcategory_ids(conn, cx.settings, id).await?,
        Some((id, false)) => vec![id],
        None => Vec::new(),
    };
    let category = if ids.is_empty() {
        String::new()
    } else {
        format!("AND topics.category_id IN {}", in_list(&ids))
    };
    let rows: Vec<(i32, String, i64, i64, i64)> = sqlx::query_as(&format!(
        "SELECT topic_view_stats.topic_id, topics.title AS topic_title, \
           SUM(topic_view_stats.anonymous_views)::bigint AS total_anonymous_views, \
           SUM(topic_view_stats.logged_in_views)::bigint AS total_logged_in_views, \
           SUM(topic_view_stats.anonymous_views + topic_view_stats.logged_in_views)::bigint AS total_views \
         FROM topic_view_stats INNER JOIN topics ON topics.id = topic_view_stats.topic_id \
         WHERE viewed_at BETWEEN {} AND {} {category} \
         GROUP BY topic_view_stats.topic_id, topics.title ORDER BY total_views DESC LIMIT 100",
        lit(r.start_date),
        lit(r.end_date)
    ))
    .fetch_all(&mut *conn)
    .await?;
    r.labels = json!([
        {"type": "topic", "properties": {"title": "topic_title", "id": "topic_id"}, "title": cx.t("reports.topic_view_stats.labels.topic")?},
        {"property": "total_anonymous_views", "type": "number", "title": cx.t("reports.topic_view_stats.labels.anon_views")?},
        {"property": "total_logged_in_views", "type": "number", "title": cx.t("reports.topic_view_stats.labels.logged_in_views")?},
        {"property": "total_views", "type": "number", "title": cx.t("reports.topic_view_stats.labels.total_views")?},
    ]);
    r.data = Value::Array(
        rows.into_iter()
            .map(|(id, title, anon, logged, total)| {
                json!({
                    "topic_id": id,
                    "topic_title": title,
                    "total_anonymous_views": anon,
                    "total_logged_in_views": logged,
                    "total_views": total,
                })
            })
            .collect(),
    );
    Ok(())
}

/// `Reports::PostersByMemberType::SYNTHETIC_KEYS`
const SYNTHETIC_KEYS: [&str; 3] = ["new_members", "returning", "staff"];
/// `EXCLUDED_GROUP_IDS`: everyone, anonymous, logged in, staff.
const EXCLUDED_GROUP_IDS: [i64; 4] = [0, 4, 5, 3];

enum Token {
    Synthetic(String),
    Group(i64),
}

/// `parse_group_token`
fn parse_group_token(token: &str) -> Option<Token> {
    if SYNTHETIC_KEYS.contains(&token) {
        return Some(Token::Synthetic(token.into()));
    }
    let digits = token.strip_prefix("group:")?;
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().map(Token::Group)
}

/// `parse_category_ids_filter`: the ids (existing, in order, at most
/// 50) and whether the selection came out empty.
async fn parse_category_ids_filter(
    conn: &mut PgConnection,
    r: &Report,
) -> Result<(Option<Vec<i64>>, bool), AppError> {
    let raw = r.filter("category_ids").cloned();
    let Some(raw) = raw.filter(|v| !super::is_blank(v)) else {
        return Ok((None, false));
    };
    let ids = super::counts::requested_category_ids(conn, Some(&raw), 50)
        .await?
        .unwrap_or_default();
    let empty = ids.is_empty();
    Ok((Some(ids), empty))
}

/// Whether the viewer may see a group and its members.
async fn group_visible(
    conn: &mut PgConnection,
    viewer: &Viewer<'_>,
    id: i64,
) -> Result<Option<bool>, AppError> {
    let row: Option<(i32, i32, i32)> = sqlx::query_as(
        "SELECT id, visibility_level, members_visibility_level FROM groups WHERE id = $1",
    )
    .bind(id as i32)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((gid, level, members)) = row else {
        return Ok(None);
    };
    if viewer.is_admin() {
        return Ok(Some(true));
    }
    let Some(g) = viewer.guardian else {
        return Ok(Some(false));
    };
    Ok(Some(
        crate::groups::can_see_group_and_members(conn, g, gid, level, members).await?,
    ))
}

/// `resolve_groups_filter`
async fn resolve_groups_filter(
    conn: &mut PgConnection,
    viewer: &Viewer<'_>,
    raw: Option<&Value>,
) -> Result<Vec<String>, AppError> {
    let items: Vec<String> = match raw {
        Some(Value::String(s)) => s.split(',').map(str::to_string).collect(),
        Some(Value::Array(a)) => a.iter().filter_map(crate::params::scalar).collect(),
        Some(Value::Null) | None => Vec::new(),
        Some(other) => crate::params::scalar(other).into_iter().collect(),
    };
    let mut tokens: Vec<String> = Vec::new();
    for t in items {
        let t = t.trim().to_string();
        if t.is_empty() || tokens.contains(&t) {
            continue;
        }
        tokens.push(t);
    }
    tokens.truncate(10);
    let mut visible: HashSet<i64> = HashSet::new();
    for t in &tokens {
        if !t.starts_with("group:") {
            continue;
        }
        if let Some(Token::Group(id)) = parse_group_token(t)
            && !EXCLUDED_GROUP_IDS.contains(&id)
            && group_visible(conn, viewer, id).await? == Some(true)
        {
            visible.insert(id);
        }
    }
    let resolved: Vec<String> = tokens
        .into_iter()
        .filter(|t| match parse_group_token(t) {
            Some(Token::Synthetic(_)) => true,
            Some(Token::Group(id)) => visible.contains(&id),
            None => false,
        })
        .collect();
    Ok(if resolved.is_empty() {
        SYNTHETIC_KEYS.iter().map(|s| s.to_string()).collect()
    } else {
        resolved
    })
}

/// `base_query_builder`'s FROM and WHERE.
async fn posters_base(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    r: &Report,
    category_ids: Option<&[i64]>,
) -> Result<(String, Vec<String>), AppError> {
    let mut joins = String::new();
    let mut wheres = vec![
        format!("p.created_at >= {}", lit(r.start_date)),
        format!("p.created_at <= {}", lit(r.end_date)),
        "p.deleted_at IS NULL".into(),
        "p.post_type = 1".into(),
        "t.deleted_at IS NULL".into(),
        "t.archetype = 'regular'".into(),
        "u.id > 0".into(),
    ];
    if let Some(ids) = category_ids.filter(|ids| !ids.is_empty()) {
        wheres.push(format!("t.category_id IN {}", in_list(ids)));
    }
    if !viewer.is_admin() {
        joins.push_str(" JOIN categories c ON c.id = t.category_id");
        let ids = match viewer.guardian {
            Some(g) => g.secure_category_ids(&mut *conn, cx.settings).await?,
            None => Vec::new(),
        };
        wheres.push(secure_category(&ids));
    }
    Ok((
        format!(
            "FROM posts p INNER JOIN topics t ON t.id = p.topic_id INNER JOIN users u ON u.id = p.user_id{joins}"
        ),
        wheres,
    ))
}

fn where_sql(wheres: &[String]) -> String {
    wheres
        .iter()
        .map(|w| format!("({w})"))
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// `report_posters_by_member_type`
pub async fn posters_by_member_type(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["posters_by_member_type".into()];
    r.labels = json!([
        {"property": "name", "title": cx.t("reports.posters_by_member_type.labels.member_type")?},
        {"property": "count", "type": "number", "title": cx.t("reports.posters_by_member_type.labels.count")?},
        {"property": "share_formatted", "title": cx.t("reports.posters_by_member_type.labels.share")?},
    ]);
    let (category_ids, empty) = parse_category_ids_filter(conn, r).await?;
    r.add_filter(
        "category_ids",
        json!({"type": "category_list", "default": category_ids}),
    );
    let raw = r.filter("groups").cloned();
    let resolved = resolve_groups_filter(conn, viewer, raw.as_ref()).await?;
    r.add_filter("groups", json!({"type": "groups", "default": resolved}));
    let group_ids: Vec<i64> = resolved
        .iter()
        .filter_map(|t| match parse_group_token(t) {
            Some(Token::Group(id)) => Some(id),
            _ => None,
        })
        .collect();
    let groups: HashMap<i64, String> = if group_ids.is_empty() {
        HashMap::new()
    } else {
        sqlx::query_as::<_, (i32, String, Option<String>)>(&format!(
            "SELECT id, name, full_name FROM groups WHERE id IN {}",
            in_list(&group_ids)
        ))
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .map(|(id, name, full)| {
            // name_full_preferred
            let shown = full.filter(|f| !f.trim().is_empty()).unwrap_or(name);
            (i64::from(id), shown)
        })
        .collect()
    };
    let mut counts: HashMap<String, i64> = HashMap::new();
    let mut total = 0i64;
    if !empty {
        let (from, wheres) = posters_base(conn, cx, viewer, r, category_ids.as_deref()).await?;
        let w = where_sql(&wheres);
        if resolved
            .iter()
            .any(|t| SYNTHETIC_KEYS.contains(&t.as_str()))
        {
            let rows: Vec<(String, i64)> = sqlx::query_as(&format!(
                "SELECT CASE WHEN u.admin OR u.moderator THEN 'staff' \
                   WHEN u.created_at >= {} THEN 'new_members' ELSE 'returning' END AS bucket, \
                   COUNT(*) AS post_count {from} WHERE {w} GROUP BY bucket",
                lit(r.start_date)
            ))
            .fetch_all(&mut *conn)
            .await?;
            counts.extend(rows);
        }
        if !group_ids.is_empty() {
            let rows: Vec<(i32, i64)> = sqlx::query_as(&format!(
                "SELECT gu.group_id, COUNT(*) AS post_count {from} \
                 JOIN group_users gu ON gu.user_id = u.id AND gu.group_id IN {} \
                 WHERE {w} GROUP BY gu.group_id",
                in_list(&group_ids)
            ))
            .fetch_all(&mut *conn)
            .await?;
            for (gid, n) in rows {
                counts.insert(format!("group:{gid}"), n);
            }
        }
        total = sqlx::query_scalar(&format!("SELECT COUNT(*) {from} WHERE {w}"))
            .fetch_one(&mut *conn)
            .await?;
    }
    r.total = Some(json!(total));
    let mut data = Vec::new();
    for token in &resolved {
        let (kind, group_id, name) = match parse_group_token(token) {
            Some(Token::Synthetic(key)) => (
                "synthetic",
                None,
                cx.t(&format!("reports.posters_by_member_type.types.{key}"))?,
            ),
            Some(Token::Group(id)) => match groups.get(&id) {
                Some(name) => ("group", Some(id), name.clone()),
                None => continue,
            },
            None => continue,
        };
        let n = counts.get(token).copied().unwrap_or(0);
        let share = if total == 0 {
            0.0
        } else {
            round_to(n as f64 / total as f64 * 100.0, 2)
        };
        data.push(json!({
            "type": token,
            "kind": kind,
            "group_id": group_id,
            "name": name,
            "count": n,
            "share": float_json(share),
            "share_formatted": format!("{}%", ruby_float(share)),
        }));
    }
    r.data = Value::Array(data);
    Ok(())
}

/// `report_posters_by_member_type_members`
pub async fn posters_by_member_type_members(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["table".into()];
    r.labels = json!([
        {"type": "user", "properties": {"username": "username", "id": "user_id", "avatar": "avatar_template"}, "title": cx.t("reports.posters_by_member_type_members.labels.user")?},
        {"property": "count", "type": "number", "title": cx.t("reports.posters_by_member_type_members.labels.count")?},
        {"property": "share_formatted", "title": cx.t("reports.posters_by_member_type_members.labels.share")?},
    ]);
    let (category_ids, empty) = parse_category_ids_filter(conn, r).await?;
    r.add_filter(
        "category_ids",
        json!({"type": "category_list", "default": category_ids}),
    );
    let token = r.filter_str("group").unwrap_or_default();
    let Some(parsed) = parse_group_token(&token) else {
        r.error = Some("not_found");
        return Ok(());
    };
    if let Token::Group(id) = parsed
        && (EXCLUDED_GROUP_IDS.contains(&id)
            || group_visible(conn, viewer, id).await? != Some(true))
    {
        r.error = Some("not_found");
        return Ok(());
    }
    r.total = Some(json!(0));
    r.data = json!([]);
    if empty {
        return Ok(());
    }
    let (from, mut wheres) = posters_base(conn, cx, viewer, r, category_ids.as_deref()).await?;
    let mut from = from;
    match &parsed {
        Token::Group(id) => {
            from.push_str(&format!(
                " JOIN group_users gu ON gu.user_id = u.id AND gu.group_id = {id}"
            ));
        }
        Token::Synthetic(key) => match key.as_str() {
            "staff" => wheres.push("u.admin OR u.moderator".into()),
            "new_members" => {
                wheres.push("NOT (u.admin OR u.moderator)".into());
                wheres.push(format!("u.created_at >= {}", lit(r.start_date)));
            }
            _ => {
                wheres.push("NOT (u.admin OR u.moderator)".into());
                wheres.push(format!("u.created_at < {}", lit(r.start_date)));
            }
        },
    }
    let w = where_sql(&wheres);
    let rows: Vec<(i32, String, Option<i32>, i64)> = sqlx::query_as(&format!(
        "SELECT u.id AS user_id, u.username, u.uploaded_avatar_id, COUNT(*) AS post_count {from} \
         WHERE {w} GROUP BY u.id, u.username, u.uploaded_avatar_id ORDER BY post_count DESC LIMIT 500"
    ))
    .fetch_all(&mut *conn)
    .await?;
    let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) {from} WHERE {w}"))
        .fetch_one(&mut *conn)
        .await?;
    r.total = Some(json!(total));
    let mut data = Vec::new();
    for (id, username, avatar, n) in rows {
        let share = if total == 0 {
            0.0
        } else {
            round_to(n as f64 / total as f64 * 100.0, 2)
        };
        data.push(json!({
            "user_id": id,
            "username": username,
            "avatar_template": cx.avatar_template(id, &username, avatar)?,
            "count": n,
            "share": float_json(share),
            "share_formatted": format!("{}%", ruby_float(share)),
        }));
    }
    r.data = Value::Array(data);
    Ok(())
}

/// `format_share_change`
fn format_share_change(change: f64) -> String {
    if change == 0.0 {
        return "0%".into();
    }
    let sign = if change > 0.0 { "+" } else { "" };
    format!("{sign}{}%", ruby_float(change))
}

/// `report_activity_by_category`
pub async fn activity_by_category(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["table".into()];
    r.labels = json!([
        {"property": "name", "title": cx.t("reports.activity_by_category.labels.category")?},
        {"property": "topics", "type": "number", "title": cx.t("reports.activity_by_category.labels.topics")?},
        {"property": "posts", "type": "number", "title": cx.t("reports.activity_by_category.labels.posts")?},
        {"property": "page_views", "type": "number", "title": cx.t("reports.activity_by_category.labels.page_views")?},
        {"property": "share_formatted", "title": cx.t("reports.activity_by_category.labels.share")?},
        {"property": "share_change_formatted", "title": cx.t("reports.activity_by_category.labels.vs_prior")?},
    ]);
    let raw = r.filter("category_ids").cloned();
    let requested = raw.as_ref().filter(|v| !super::is_blank(v)).map(|raw| {
        let mut ids: Vec<i64> = Vec::new();
        for id in parse_id_list(raw) {
            if id != 0 && !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids.truncate(50);
        ids
    });
    r.add_filter(
        "category_ids",
        json!({"type": "category_list", "default": requested}),
    );
    if requested.as_ref().is_some_and(Vec::is_empty) {
        r.total = Some(json!(0));
        r.data = json!([]);
        return Ok(());
    }
    let page_views = if super::crawler_scorer_enabled(cx.settings)? {
        "GREATEST(r.page_views - r.likely_crawler_page_views, 0)"
    } else {
        "r.page_views"
    };
    let mut wheres = vec![format!(
        "r.date >= '{}' AND r.date <= '{}'",
        r.prev_start_date().date(),
        r.end_date.date()
    )];
    if let Some(ids) = &requested {
        wheres.push(format!("c.id IN {}", in_list(ids)));
    }
    if !viewer.is_admin() {
        let ids = match viewer.guardian {
            Some(g) => g.secure_category_ids(&mut *conn, cx.settings).await?,
            None => Vec::new(),
        };
        wheres.push(secure_category(&ids));
    }
    let current = format!("'{}'", r.start_date.date());
    let sql = format!(
        "SELECT c.id, c.name, c.color, c.slug, \
           COALESCE(SUM(r.topics) FILTER (WHERE r.date >= {current}), 0)::bigint AS topics_current, \
           COALESCE(SUM(r.posts) FILTER (WHERE r.date >= {current}), 0)::bigint AS posts_current, \
           COALESCE(SUM({page_views}) FILTER (WHERE r.date >= {current}), 0)::bigint AS page_views_current, \
           COALESCE(SUM(r.topics) FILTER (WHERE r.date < {current}), 0)::bigint AS topics_prior, \
           COALESCE(SUM(r.posts) FILTER (WHERE r.date < {current}), 0)::bigint AS posts_prior, \
           COALESCE(SUM({page_views}) FILTER (WHERE r.date < {current}), 0)::bigint AS page_views_prior \
         FROM category_activity_daily_rollups r INNER JOIN categories c ON c.id = r.category_id \
         WHERE {} GROUP BY c.id, c.name, c.color, c.slug \
         HAVING SUM(r.topics + r.posts + {page_views}) > 0",
        where_sql(&wheres)
    );
    #[allow(clippy::type_complexity)]
    let rows: Vec<(i32, String, String, String, i64, i64, i64, i64, i64, i64)> =
        sqlx::query_as(&sql).fetch_all(&mut *conn).await?;
    // The periods' rows, in the query's order.
    let mut current_rows: Vec<(i32, String, String, String, i64, i64, i64)> = Vec::new();
    let mut prior: HashMap<i32, i64> = HashMap::new();
    for (id, name, color, slug, t, p, v, tp, pp, vp) in rows {
        if t + p + v > 0 {
            current_rows.push((id, name, color, slug, t, p, v));
        }
        if tp + pp + vp > 0 {
            prior.insert(id, tp + pp + vp);
        }
    }
    let total_current: i64 = current_rows.iter().map(|c| c.4 + c.5 + c.6).sum();
    let total_prior: i64 = prior.values().sum();
    let mut out: Vec<(i64, Value)> = Vec::new();
    for (id, name, color, slug, t, p, v) in current_rows {
        let activity = t + p + v;
        let share = if total_current == 0 {
            0.0
        } else {
            round_to(activity as f64 / total_current as f64 * 100.0, 2)
        };
        let prior_activity = prior.get(&id).copied().unwrap_or(0);
        let prior_share = if total_prior == 0 {
            0.0
        } else {
            round_to(prior_activity as f64 / total_prior as f64 * 100.0, 2)
        };
        let change = round_to(share - prior_share, 2);
        out.push((
            activity,
            json!({
                "category_id": id,
                "name": name,
                "color": color,
                "slug": slug,
                "topics": t,
                "posts": p,
                "page_views": v,
                "share": float_json(share),
                "share_change": float_json(change),
                "share_formatted": format!("{}%", ruby_float(share)),
                "share_change_formatted": format_share_change(change),
            }),
        ));
    }
    // sort_by { -activity }
    out.sort_by_key(|(a, _)| -a);
    let mut rows: Vec<Value> = out.into_iter().map(|(_, v)| v).collect();
    if requested.is_none() {
        rows.truncate(6);
    }
    r.total = Some(json!(total_current));
    r.data = Value::Array(rows);
    Ok(())
}

/// discourse-reactions' `reactions` report.
pub async fn reactions(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    let main_id = cx
        .settings
        .get("discourse_reactions_reaction_for_like")?
        .to_s()
        .replace('-', "");
    r.icon = Some("discourse-emojis".into());
    r.modes = vec!["table".into()];
    let emoji = |name: &str| {
        crate::topic_list_view::unescape_emoji(
            &crate::admin_site_settings::escape_html(&format!(":{name}:")),
            cx.settings.emojis(),
            cx.base_path(),
        )
    };
    let mut labels = vec![
        json!({"type": "date", "property": "day", "title": cx.t("reports.reactions.labels.day")?}),
        json!({"type": "number", "property": "like_count", "title": main_id, "html_title": emoji(&main_id)}),
    ];
    let enabled: Vec<String> = cx
        .settings
        .get("discourse_reactions_enabled_reactions")?
        .to_s()
        .split('|')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    for reaction in enabled.iter().filter(|r| **r != main_id) {
        labels.push(json!({
            "type": "number",
            "property": format!("{reaction}_count"),
            "title": reaction,
            "html_title": emoji(reaction),
        }));
    }
    r.labels = Value::Array(labels);
    let start = r.start_date.date();
    let end = r.end_date.date();
    let reactions: Vec<(String, i64, NaiveDate)> = sqlx::query_as(
        "SELECT reactions.reaction_value, count(reaction_users.id) as reactions_count, \
           date_trunc('day', reaction_users.created_at)::date as day \
         FROM discourse_reactions_reactions as reactions \
         LEFT OUTER JOIN discourse_reactions_reaction_users as reaction_users \
           on reactions.id = reaction_users.reaction_id \
         WHERE reactions.reaction_users_count IS NOT NULL \
           AND reaction_users.created_at::DATE >= $1::DATE AND reaction_users.created_at::DATE <= $2::DATE \
         GROUP BY reactions.reaction_value, day",
    )
    .bind(start)
    .bind(end)
    .fetch_all(&mut *conn)
    .await?;
    // valid_reactions: the main one and the enabled ones.
    let mut valid: Vec<String> = vec![main_id.clone()];
    for e in &enabled {
        if !valid.contains(e) {
            valid.push(e.clone());
        }
    }
    let likes: Vec<(i64, NaiveDate)> = sqlx::query_as(
        "SELECT count(post_actions.id) as likes_count, date_trunc('day', post_actions.created_at)::date as day \
         FROM post_actions as post_actions \
         WHERE post_actions.created_at::DATE >= $1::DATE AND post_actions.created_at::DATE <= $2::DATE \
           AND post_actions.post_action_type_id = 2 AND post_actions.deleted_at IS NULL \
           AND post_actions.post_id NOT IN ( \
             SELECT discourse_reactions_reaction_users.post_id FROM discourse_reactions_reaction_users \
             INNER JOIN discourse_reactions_reactions \
               ON discourse_reactions_reactions.id = discourse_reactions_reaction_users.reaction_id \
             WHERE discourse_reactions_reaction_users.user_id = post_actions.user_id \
               AND discourse_reactions_reaction_users.post_id = post_actions.post_id \
               AND discourse_reactions_reactions.reaction_value = ANY($3)) \
         GROUP BY day",
    )
    .bind(start)
    .bind(end)
    .bind(&valid)
    .fetch_all(&mut *conn)
    .await?;
    let mut data = Vec::new();
    let mut day = start;
    while day <= end {
        let mut m = Map::new();
        m.insert("day".into(), super::date_json(day));
        let like_count: i64 = likes.iter().filter(|l| l.1 == day).map(|l| l.0).sum();
        let mut like_reactions = 0i64;
        for (value, n, _) in reactions.iter().filter(|x| x.2 == day) {
            if *value == main_id {
                like_reactions += n;
            } else {
                let key = format!("{value}_count");
                let current = m.get(&key).and_then(Value::as_i64).unwrap_or(0);
                m.insert(key, json!(current + n));
            }
        }
        m.insert("like_count".into(), json!(like_reactions + like_count));
        data.push(Value::Object(m));
        day = day
            .succ_opt()
            .ok_or(Unsupported("dates past the calendar"))?;
    }
    r.data = Value::Array(data);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_as_rails_does() {
        assert_eq!(
            truncate("a-really-long-file-name-for-the-report.png", 25),
            "a-really-long-file-nam..."
        );
        assert_eq!(truncate("notes.pdf", 25), "notes.pdf");
    }

    #[test]
    fn formats_share_changes() {
        assert_eq!(format_share_change(0.0), "0%");
        assert_eq!(format_share_change(62.5), "+62.5%");
        assert_eq!(format_share_change(-22.8), "-22.8%");
        assert_eq!(format_share_change(25.0), "+25.0%");
    }
}
