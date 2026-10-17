//! DiscourseMcp as the admin MCP endpoints read and write it: the
//! primitives core and the bundled plugins register (recorded from the
//! reference by scripts/record-mcp-primitives into
//! vendor/discourse/config/mcp_primitives.json, with the settings their
//! availability reads), the group access rules (McpGroupScope,
//! McpAccessRuleUpdater), the primitives' policies (McpPrimitive,
//! McpPrimitiveUpdater), the OAuth authorizations with their status
//! (DiscourseMcp::AuthorizationStatus) and the activity report
//! (McpActivityReport).

use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

use chrono::NaiveDateTime;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;

use crate::AppError;
use crate::site_settings::{Definitions, SiteSettings};

/// `DiscourseMcp::INITIAL_SCOPE`
pub const INITIAL_SCOPE: &str = "mcp:profile:read";
/// `DiscourseMcp::PROTOCOL_VERSION`
pub const PROTOCOL_VERSION: &str = "2026-07-28";
/// `DiscourseMcp::Primitive::KINDS`
pub const KINDS: [&str; 4] = ["tool", "resource", "resource_template", "prompt"];
/// `Group::AUTO_GROUPS[:admins]`
pub const ADMINS_GROUP_ID: i32 = 1;

const RECORDED: &str = include_str!("../vendor/discourse/config/mcp_primitives.json");

/// A registered primitive, as recorded.
#[derive(Debug, Deserialize)]
pub struct Primitive {
    pub kind: String,
    pub identifier: String,
    pub title: String,
    pub description: String,
    pub provider: String,
    pub risk: String,
    pub required_scopes: Vec<String>,
    /// `JSON.generate(input_schema).bytesize`
    pub schema_bytes: i64,
    /// `consent_relevant?`: destructive, moderation, administration or
    /// external side effect.
    pub consent_relevant: bool,
    /// The plugin that registered it (its availability needs the plugin
    /// enabled), with that plugin's enabled setting.
    pub plugin: Option<String>,
    pub enabled_setting: Option<String>,
    /// The settings its availability lambda reads, each to be truthy.
    pub available_when: Vec<String>,
}

impl Primitive {
    /// `available?`
    pub fn available(&self, settings: &SiteSettings) -> bool {
        let on = |name: &str| settings.get(name).is_ok_and(|v| v.truthy());
        if self.plugin.is_some()
            && let Some(setting) = &self.enabled_setting
            && !on(setting)
        {
            return false;
        }
        self.available_when.iter().all(|s| on(s))
    }

    /// `"#{kind}:#{identifier}"`
    pub fn id(&self) -> String {
        format!("{}:{}", self.kind, self.identifier)
    }
}

/// `DiscourseMcp.registry.all`: by kind (KINDS' order), then identifier.
pub fn registry() -> &'static [Primitive] {
    static ALL: OnceLock<Vec<Primitive>> = OnceLock::new();
    ALL.get_or_init(|| serde_json::from_str(RECORDED).expect("mcp_primitives.json"))
}

/// `DiscourseMcp.registry.find(kind, identifier)`
pub fn find(kind: &str, identifier: &str) -> Option<&'static Primitive> {
    registry()
        .iter()
        .find(|p| p.kind == kind && p.identifier == identifier)
}

/// `DiscourseMcp.registry.scopes`: every required scope, sorted.
pub fn scopes() -> Vec<String> {
    registry()
        .iter()
        .flat_map(|p| p.required_scopes.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// A McpPrimitive row: the policy of one primitive.
#[derive(sqlx::FromRow)]
struct PolicyRow {
    id: i64,
    kind: String,
    identifier: String,
    enabled: bool,
    emergency_blocked: bool,
    consent_required_at: Option<NaiveDateTime>,
}

async fn policies(conn: &mut PgConnection) -> Result<Vec<PolicyRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, kind, identifier, enabled, emergency_blocked, consent_required_at \
         FROM mcp_primitives ORDER BY id",
    )
    .fetch_all(conn)
    .await
}

/// `McpPrimitive.exposed`: enabled and not emergency blocked, with the
/// primitive they name (when it is still registered).
async fn exposed(
    conn: &mut PgConnection,
) -> Result<Vec<(i64, Option<&'static Primitive>)>, sqlx::Error> {
    Ok(policies(conn)
        .await?
        .into_iter()
        .filter(|p| p.enabled && !p.emergency_blocked)
        .map(|p| (p.id, find(&p.kind, &p.identifier)))
        .collect())
}

/// Admin::McpController#primitives_payload
pub async fn primitives_payload(
    conn: &mut PgConnection,
    settings: &SiteSettings,
) -> Result<Value, AppError> {
    let records: HashMap<(String, String), PolicyRow> = policies(&mut *conn)
        .await?
        .into_iter()
        .map(|p| ((p.kind.clone(), p.identifier.clone()), p))
        .collect();
    let primitives: Vec<Value> = KINDS
        .iter()
        .flat_map(|kind| registry().iter().filter(move |p| p.kind == *kind))
        .map(|p| {
            let record = records.get(&(p.kind.clone(), p.identifier.clone()));
            let digest = format!("{:x}", Sha256::digest(p.identifier.as_bytes()));
            json!({
                "id": p.id(),
                "field_name": format!("primitive_{}_{}", p.kind, &digest[..16]),
                "identifier": p.identifier,
                "kind": p.kind,
                "title": p.title,
                "description": p.description,
                "provider": p.provider,
                "risk": p.risk,
                "required_scopes": p.required_scopes,
                "enabled": record.is_some_and(|r| r.enabled),
                "emergency_blocked": record.is_some_and(|r| r.emergency_blocked),
                "available": p.available(settings),
            })
        })
        .collect();
    Ok(json!({
        "initial_scope": INITIAL_SCOPE,
        "available_scopes": scopes(),
        "primitives": primitives,
    }))
}

/// `McpGroupScope.ensure_admins!`: the admins' group gets every scope the
/// first time anyone asks.
pub async fn ensure_admins(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM mcp_group_scopes WHERE group_id = $1)")
            .bind(ADMINS_GROUP_ID)
            .fetch_one(&mut *conn)
            .await?;
    if exists {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO mcp_group_scopes (group_id, scope, created_at, updated_at) \
         SELECT $1, s, now(), now() FROM unnest($2::text[]) AS s \
         ON CONFLICT (group_id, scope) DO NOTHING",
    )
    .bind(ADMINS_GROUP_ID)
    .bind(scopes())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Admin::McpController#access_rules_payload
pub async fn access_rules(conn: &mut PgConnection) -> Result<Vec<Value>, AppError> {
    ensure_admins(&mut *conn).await?;
    let rows: Vec<(i32, String, Option<String>)> = sqlx::query_as(
        "SELECT s.group_id, s.scope, g.name FROM mcp_group_scopes s \
         LEFT JOIN groups g ON g.id = s.group_id ORDER BY s.group_id, s.scope",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut out: Vec<Value> = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let group_id = rows[i].0;
        let mut scopes = BTreeSet::new();
        let name = rows[i].2.clone();
        while i < rows.len() && rows[i].0 == group_id {
            scopes.insert(rows[i].1.clone());
            i += 1;
        }
        let Some(name) = name else {
            continue;
        };
        scopes.insert(INITIAL_SCOPE.to_string());
        out.push(json!({
            "group_id": group_id,
            "group_name": name,
            "scopes": scopes.into_iter().collect::<Vec<_>>(),
            "pre_registered": group_id == ADMINS_GROUP_ID,
            "deletable": group_id != ADMINS_GROUP_ID,
        }));
    }
    Ok(out)
}

/// What an updater refuses: the parameter Discourse::InvalidParameters
/// names, or NotFound.
pub enum Refusal {
    InvalidParameters(&'static str),
    NotFound,
}

async fn group_exists(conn: &mut PgConnection, group_id: i64) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM groups WHERE id = $1)")
        .bind(group_id as i32)
        .fetch_one(conn)
        .await
}

/// Ruby's `Array#to_s` of strings, as log_custom writes it.
fn inspect_list(items: &[String]) -> String {
    crate::schema_objects::inspect(&json!(items))
}

/// McpAccessRuleUpdater.replace!: the group's scopes (always with the
/// initial one), logged.
pub async fn replace_access(
    conn: &mut PgConnection,
    actor: i32,
    group_id: i64,
    scopes: &[String],
) -> Result<Result<(), Refusal>, AppError> {
    let mut wanted: BTreeSet<String> = scopes.iter().cloned().collect();
    wanted.insert(INITIAL_SCOPE.to_string());
    let wanted: Vec<String> = wanted.into_iter().collect();
    if !group_exists(&mut *conn, group_id).await? {
        return Ok(Err(Refusal::InvalidParameters("group_id")));
    }
    let registered = self::scopes();
    if wanted.iter().any(|s| !registered.contains(s)) {
        return Ok(Err(Refusal::InvalidParameters("scopes")));
    }
    sqlx::query("DELETE FROM mcp_group_scopes WHERE group_id = $1")
        .bind(group_id as i32)
        .execute(&mut *conn)
        .await?;
    for scope in &wanted {
        sqlx::query(
            "INSERT INTO mcp_group_scopes (group_id, scope, created_at, updated_at) \
             VALUES ($1, $2, now(), now())",
        )
        .bind(group_id as i32)
        .bind(scope)
        .execute(&mut *conn)
        .await?;
    }
    crate::staff_action_logs::log_custom(
        &mut *conn,
        actor,
        "mcp_access_rule_updated",
        &[
            ("group_id", group_id.to_string()),
            ("scopes", inspect_list(&wanted)),
        ],
        None,
        None,
    )
    .await?;
    Ok(Ok(()))
}

/// McpAccessRuleUpdater.delete!: the group's scopes removed (never the
/// admins'), logged with what they were.
pub async fn delete_access(
    conn: &mut PgConnection,
    actor: i32,
    group_id: i64,
) -> Result<Result<(), Refusal>, AppError> {
    if !group_exists(&mut *conn, group_id).await? || group_id == ADMINS_GROUP_ID as i64 {
        return Ok(Err(Refusal::InvalidParameters("group_id")));
    }
    let removed: Vec<String> =
        sqlx::query_scalar("SELECT scope FROM mcp_group_scopes WHERE group_id = $1 ORDER BY scope")
            .bind(group_id as i32)
            .fetch_all(&mut *conn)
            .await?;
    sqlx::query("DELETE FROM mcp_group_scopes WHERE group_id = $1")
        .bind(group_id as i32)
        .execute(&mut *conn)
        .await?;
    crate::staff_action_logs::log_custom(
        &mut *conn,
        actor,
        "mcp_access_rule_deleted",
        &[
            ("group_id", group_id.to_string()),
            ("scopes", inspect_list(&removed)),
        ],
        None,
        None,
    )
    .await?;
    Ok(Ok(()))
}

/// `McpOauthAuthorization.require_consent!(scopes:)`: the active
/// authorizations holding any of the scopes (all of them for none) lose
/// their tokens and must consent again.
async fn require_consent(conn: &mut PgConnection, scopes: &[String]) -> Result<(), sqlx::Error> {
    let filter = if scopes.is_empty() {
        ""
    } else {
        " AND a.id IN (SELECT mcp_oauth_authorization_id FROM mcp_oauth_authorization_scopes \
         WHERE name = ANY($1))"
    };
    let ids: Vec<i64> = sqlx::query_scalar(&format!(
        "SELECT a.id FROM mcp_oauth_authorizations a \
         WHERE a.status = 'active' AND a.revoked_at IS NULL{filter}"
    ))
    .bind(scopes)
    .fetch_all(&mut *conn)
    .await?;
    if ids.is_empty() {
        return Ok(());
    }
    for table in ["mcp_oauth_access_tokens", "mcp_oauth_refresh_tokens"] {
        sqlx::query(&format!(
            "UPDATE {table} SET revoked_at = now(), updated_at = now() \
             WHERE mcp_oauth_authorization_id = ANY($1) AND revoked_at IS NULL"
        ))
        .bind(&ids)
        .execute(&mut *conn)
        .await?;
    }
    sqlx::query(
        "UPDATE mcp_oauth_authorizations SET status = 'consent_required', updated_at = now() \
         WHERE id = ANY($1)",
    )
    .bind(&ids)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The ids of the exposed policies whose primitive is consent relevant.
async fn exposed_consent_relevant(conn: &mut PgConnection) -> Result<BTreeSet<i64>, sqlx::Error> {
    Ok(exposed(conn)
        .await?
        .into_iter()
        .filter(|(_, p)| p.is_some_and(|p| p.consent_relevant))
        .map(|(id, _)| id)
        .collect())
}

/// McpPrimitiveUpdater.update!: each registered primitive enabled when
/// its id is given; newly exposed consent relevant ones ask their
/// scopes' authorizations to consent again; logged.
pub async fn update_primitives(
    conn: &mut PgConnection,
    actor: i32,
    primitive_ids: &[String],
) -> Result<(), AppError> {
    let wanted: BTreeSet<&str> = primitive_ids.iter().map(String::as_str).collect();
    let before = exposed_consent_relevant(&mut *conn).await?;
    // `registry.all` without a kind: every primitive by identifier.
    let mut all: Vec<&Primitive> = registry().iter().collect();
    all.sort_by(|a, b| a.identifier.cmp(&b.identifier));
    for p in all {
        let enabled = wanted.contains(p.id().as_str());
        // find_or_initialize_by then save!: a new row, or the changed one.
        let row: Option<(i64, bool)> = sqlx::query_as(
            "SELECT id, enabled FROM mcp_primitives WHERE kind = $1 AND identifier = $2",
        )
        .bind(&p.kind)
        .bind(&p.identifier)
        .fetch_optional(&mut *conn)
        .await?;
        match row {
            None => {
                sqlx::query(
                    "INSERT INTO mcp_primitives (kind, identifier, enabled, created_at, updated_at) \
                     VALUES ($1, $2, $3, now(), now())",
                )
                .bind(&p.kind)
                .bind(&p.identifier)
                .bind(enabled)
                .execute(&mut *conn)
                .await?;
            }
            Some((id, current)) if current != enabled => {
                sqlx::query(
                    "UPDATE mcp_primitives SET enabled = $2, updated_at = now() WHERE id = $1",
                )
                .bind(id)
                .bind(enabled)
                .execute(&mut *conn)
                .await?;
            }
            Some(_) => {}
        }
    }
    let after = exposed_consent_relevant(&mut *conn).await?;
    let newly: Vec<i64> = after.difference(&before).copied().collect();
    if !newly.is_empty() {
        sqlx::query(
            "UPDATE mcp_primitives SET consent_required_at = now(), updated_at = now() \
             WHERE id = ANY($1)",
        )
        .bind(&newly)
        .execute(&mut *conn)
        .await?;
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT kind, identifier FROM mcp_primitives WHERE id = ANY($1)")
                .bind(&newly)
                .fetch_all(&mut *conn)
                .await?;
        let mut scopes: Vec<String> = Vec::new();
        for (kind, identifier) in rows {
            for s in find(&kind, &identifier)
                .map(|p| p.required_scopes.clone())
                .unwrap_or_default()
            {
                if !scopes.contains(&s) {
                    scopes.push(s);
                }
            }
        }
        require_consent(&mut *conn, &scopes).await?;
    }
    crate::staff_action_logs::log_custom(
        &mut *conn,
        actor,
        "mcp_primitives_updated",
        &[("primitive_count", wanted.len().to_string())],
        None,
        None,
    )
    .await?;
    Ok(())
}

/// McpPrimitiveUpdater.set_emergency_block!: the primitive's block set;
/// unblocking a consent relevant one back into exposure asks for consent
/// again; logged either way.
pub async fn set_emergency_block(
    conn: &mut PgConnection,
    actor: i32,
    primitive_id: &str,
    blocked: bool,
) -> Result<Result<(), Refusal>, AppError> {
    let (kind, identifier) = primitive_id.split_once(':').unwrap_or((primitive_id, ""));
    let Some(primitive) = find(kind, identifier) else {
        return Ok(Err(Refusal::NotFound));
    };
    let row: Option<(i64, bool, bool)> = sqlx::query_as(
        "SELECT id, enabled, emergency_blocked FROM mcp_primitives \
         WHERE kind = $1 AND identifier = $2",
    )
    .bind(kind)
    .bind(identifier)
    .fetch_optional(&mut *conn)
    .await?;
    let (enabled, was_blocked) = row.map_or((false, false), |(_, e, b)| (e, b));
    if row.is_none() || was_blocked != blocked {
        let was_exposed = enabled && !was_blocked;
        let expands = !was_exposed && enabled && !blocked && primitive.consent_relevant;
        match row {
            None => {
                sqlx::query(
                    "INSERT INTO mcp_primitives (kind, identifier, emergency_blocked, \
                     consent_required_at, created_at, updated_at) \
                     VALUES ($1, $2, $3, CASE WHEN $4 THEN now() END, now(), now())",
                )
                .bind(kind)
                .bind(identifier)
                .bind(blocked)
                .bind(expands)
                .execute(&mut *conn)
                .await?;
            }
            Some((id, _, _)) => {
                sqlx::query(
                    "UPDATE mcp_primitives SET emergency_blocked = $2, \
                     consent_required_at = CASE WHEN $3 THEN now() ELSE consent_required_at END, \
                     updated_at = now() WHERE id = $1",
                )
                .bind(id)
                .bind(blocked)
                .bind(expands)
                .execute(&mut *conn)
                .await?;
            }
        }
        if expands {
            require_consent(&mut *conn, &primitive.required_scopes).await?;
        }
    }
    crate::staff_action_logs::log_custom(
        &mut *conn,
        actor,
        "mcp_primitive_emergency_block_updated",
        &[
            ("primitive", primitive_id.to_string()),
            ("blocked", blocked.to_string()),
        ],
        None,
        None,
    )
    .await?;
    Ok(Ok(()))
}

/// Admin::McpController#overview
pub async fn overview(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &crate::i18n::I18n,
    base_url: &str,
) -> Result<Value, AppError> {
    let exposed: Vec<&Primitive> = exposed(&mut *conn)
        .await?
        .into_iter()
        .filter_map(|(_, p)| p.filter(|p| p.available(settings)))
        .collect();
    let enabled = settings.get("mcp_server_enabled")?.truthy();
    let count = |sql: &'static str| sqlx::query_scalar::<_, i64>(sql);
    let approved = count("SELECT count(*) FROM mcp_oauth_clients WHERE trust_state = 'approved'")
        .fetch_one(&mut *conn)
        .await?;
    let authorizations = count(
        "SELECT count(*) FROM mcp_oauth_authorizations WHERE status = 'active' AND revoked_at IS NULL",
    )
    .fetch_one(&mut *conn)
    .await?;
    let tokens = count(
        "SELECT count(*) FROM mcp_oauth_access_tokens WHERE revoked_at IS NULL AND expires_at > now()",
    )
    .fetch_one(&mut *conn)
    .await?;
    let errors = count(
        "SELECT count(*) FROM mcp_audit_logs WHERE occurred_at > now() - interval '24 hours' \
         AND outcome <> 'success'",
    )
    .fetch_one(&mut *conn)
    .await?;
    let kind_count = |kinds: &[&str]| {
        exposed
            .iter()
            .filter(|p| kinds.contains(&p.kind.as_str()))
            .count()
    };
    let t = |key: &str| i18n.t(key).unwrap_or_default().to_string();
    Ok(json!({
        "enabled": enabled,
        "endpoint": format!("{base_url}/mcp"),
        "protocol_version": PROTOCOL_VERSION,
        "server_version": crate::schema::discourse_version(),
        "status": if enabled { "available" } else { "disabled" },
        "catalog": {
            "tools": kind_count(&["tool"]),
            "resources": kind_count(&["resource", "resource_template"]),
            "prompts": kind_count(&["prompt"]),
            "schema_bytes": exposed.iter().map(|p| p.schema_bytes).sum::<i64>(),
        },
        "metrics": {
            "approved_oauth_clients": approved,
            "authorizations": authorizations,
            "tokens": tokens,
            "errors": errors,
        },
        "approved_oauth_clients": approved,
        "active_authorizations": authorizations,
        "active_tokens": tokens,
        "recent_errors": errors,
        "setup_checklist": [
            { "label": t("mcp.admin.site_enabled"), "complete": enabled },
            { "label": t("mcp.admin.primitives_enabled"), "complete": !exposed.is_empty() },
            { "label": t("mcp.admin.client_registered"), "complete": approved > 0 },
        ],
        "warnings": [],
    }))
}

/// `DiscourseMcp::Access.eligible_scopes(user)`: the scopes the user's
/// groups grant (with the initial one), none for an unavailable user.
pub async fn eligible_scopes(
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<Vec<String>, AppError> {
    let user: Option<(bool, bool, bool, bool)> = sqlx::query_as(
        "SELECT active, (suspended_till IS NOT NULL AND suspended_till > now()), staged, admin \
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((active, suspended, staged, admin)) = user else {
        return Ok(Vec::new());
    };
    if !user_available(active, suspended, staged) {
        return Ok(Vec::new());
    }
    if admin {
        ensure_admins(&mut *conn).await?;
    }
    let mut scopes: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT s.scope FROM mcp_group_scopes s \
         JOIN group_users gu ON gu.group_id = s.group_id \
         WHERE gu.user_id = $1 AND s.scope = ANY($2)",
    )
    .bind(user_id)
    .bind(scopes())
    .fetch_all(&mut *conn)
    .await?;
    if !scopes.is_empty() && !scopes.iter().any(|s| s == INITIAL_SCOPE) {
        scopes.push(INITIAL_SCOPE.into());
    }
    scopes.sort();
    Ok(scopes)
}

/// `DiscourseMcp::Access.eligible_for_exposed_primitive?`: the server on,
/// and an exposed, available primitive whose scopes the user may grant.
pub async fn eligible_for_exposed_primitive(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
) -> Result<bool, AppError> {
    if !settings.get("mcp_server_enabled")?.truthy() {
        return Ok(false);
    }
    let eligible = eligible_scopes(&mut *conn, user_id).await?;
    if eligible.is_empty() {
        return Ok(false);
    }
    Ok(exposed(&mut *conn).await?.into_iter().any(|(_, p)| {
        p.is_some_and(|p| {
            p.available(settings) && p.required_scopes.iter().all(|s| eligible.contains(s))
        })
    }))
}

/// `DiscourseMcp::Access.user_available?`
fn user_available(active: bool, suspended: bool, staged: bool) -> bool {
    active && !suspended && !staged
}

/// `DiscourseMcp::OAuth::ClientResolver.allowed_by_current_policy?`
fn client_allowed(
    settings: &SiteSettings,
    registration_type: &str,
    trust: &str,
    client_id: &str,
) -> bool {
    if registration_type == "pre_registered" {
        return trust == "approved";
    }
    if registration_type != "cimd" || trust != "approved" {
        return false;
    }
    match settings
        .get("mcp_oauth_client_id_metadata_policy")
        .map(|v| v.to_s())
        .unwrap_or_default()
        .as_str()
    {
        "any_domain" => true,
        "approved_domains" => {
            let host = url::Url::parse(client_id)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string));
            let Some(host) = host else {
                return false;
            };
            settings
                .get("mcp_oauth_client_id_metadata_domains")
                .map(|v| v.to_s())
                .unwrap_or_default()
                .split('|')
                .any(|d| d.eq_ignore_ascii_case(&host))
        }
        _ => false,
    }
}

#[derive(sqlx::FromRow)]
struct AuthorizationRow {
    id: i64,
    user_id: i32,
    username: Option<String>,
    user_active: Option<bool>,
    user_suspended: Option<bool>,
    user_staged: Option<bool>,
    user_admin: Option<bool>,
    client_name: String,
    client_id: String,
    client_registration_type: String,
    client_trust_state: String,
    client_metadata_hash: Option<String>,
    resource: String,
    status: String,
    authorization_metadata_hash: Option<String>,
    consented_at: NaiveDateTime,
    revoked_at: Option<NaiveDateTime>,
}

/// Admin::McpAuthorizationsController#index's page
/// (McpOauthAuthorizationsPage), each with its status.
pub async fn authorizations(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    limit: i64,
    cursor: Option<i64>,
    filter: &str,
) -> Result<Value, AppError> {
    let mut sql = String::from(
        "SELECT a.id, a.user_id, u.username, u.active AS user_active, \
         (u.suspended_till IS NOT NULL AND u.suspended_till > now()) AS user_suspended, \
         u.staged AS user_staged, u.admin AS user_admin, c.name AS client_name, \
         c.client_id, c.registration_type AS client_registration_type, \
         c.trust_state AS client_trust_state, c.metadata_hash AS client_metadata_hash, \
         a.resource, a.status, a.client_metadata_hash AS authorization_metadata_hash, \
         a.consented_at, a.revoked_at \
         FROM mcp_oauth_authorizations a \
         LEFT JOIN users u ON u.id = a.user_id \
         JOIN mcp_oauth_clients c ON c.id = a.mcp_oauth_client_id WHERE TRUE",
    );
    if cursor.is_some() {
        sql.push_str(" AND a.id < $2");
    }
    if !filter.is_empty() {
        sql.push_str(
            " AND (u.username ILIKE $1 OR c.name ILIKE $1 OR c.client_id ILIKE $1 \
             OR a.status ILIKE $1 OR EXISTS (SELECT 1 FROM mcp_oauth_authorization_scopes s \
             WHERE s.mcp_oauth_authorization_id = a.id AND s.name ILIKE $1))",
        );
    }
    sql.push_str(" ORDER BY a.id DESC LIMIT $3");
    let pattern = format!("%{}%", crate::ruby::sanitize_sql_like(filter));
    let mut rows: Vec<AuthorizationRow> = sqlx::query_as(&sql)
        .bind(&pattern)
        .bind(cursor.unwrap_or(0))
        .bind(limit + 1)
        .fetch_all(&mut *conn)
        .await?;
    let next_cursor = if rows.len() as i64 > limit {
        rows.pop();
        rows.last().map(|r| r.id)
    } else {
        None
    };
    let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    let scope_rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT mcp_oauth_authorization_id, name FROM mcp_oauth_authorization_scopes \
         WHERE mcp_oauth_authorization_id = ANY($1) ORDER BY id",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    let token_rows: Vec<(i64, Option<NaiveDateTime>, bool)> = sqlx::query_as(
        "SELECT mcp_oauth_authorization_id, last_used_at, \
         (revoked_at IS NULL AND expires_at > now()) FROM mcp_oauth_access_tokens \
         WHERE mcp_oauth_authorization_id = ANY($1)",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    // AuthorizationStatus: the users' eligible scopes and the consents
    // the primitives require.
    if rows.iter().any(|r| {
        r.user_admin == Some(true)
            && user_available(
                r.user_active.unwrap_or(false),
                r.user_suspended.unwrap_or(false),
                r.user_staged.unwrap_or(false),
            )
    }) {
        ensure_admins(&mut *conn).await?;
    }
    let user_ids: Vec<i32> = rows.iter().map(|r| r.user_id).collect();
    let eligible_rows: Vec<(i32, String)> = sqlx::query_as(
        "SELECT DISTINCT gu.user_id, s.scope FROM mcp_group_scopes s \
         JOIN group_users gu ON gu.group_id = s.group_id \
         WHERE gu.user_id = ANY($1) AND s.scope = ANY($2)",
    )
    .bind(&user_ids)
    .bind(scopes())
    .fetch_all(&mut *conn)
    .await?;
    let requirements: Vec<(Vec<String>, NaiveDateTime)> = policies(&mut *conn)
        .await?
        .into_iter()
        .filter_map(|p| {
            let at = p.consent_required_at?;
            find(&p.kind, &p.identifier).map(|prim| (prim.required_scopes.clone(), at))
        })
        .collect();
    let server_enabled = settings.get("mcp_server_enabled")?.truthy();
    let mut out = Vec::new();
    for r in &rows {
        let scopes: Vec<String> = scope_rows
            .iter()
            .filter(|(a, _)| *a == r.id)
            .map(|(_, s)| s.clone())
            .collect();
        let available = user_available(
            r.user_active.unwrap_or(false),
            r.user_suspended.unwrap_or(false),
            r.user_staged.unwrap_or(false),
        ) && r.username.is_some();
        let mut eligible: Vec<String> = if available {
            eligible_rows
                .iter()
                .filter(|(u, _)| *u == r.user_id)
                .map(|(_, s)| s.clone())
                .collect()
        } else {
            Vec::new()
        };
        if !eligible.is_empty() && !eligible.iter().any(|s| s == INITIAL_SCOPE) {
            eligible.push(INITIAL_SCOPE.into());
        }
        let consent_required_at = requirements
            .iter()
            .filter(|(req, _)| req.is_empty() || req.iter().all(|s| scopes.contains(s)))
            .map(|(_, at)| *at)
            .max();
        let status = if r.status == "revoked" || r.revoked_at.is_some() {
            "revoked"
        } else if r.status == "consent_required" {
            "consent_required"
        } else if !server_enabled {
            "server_disabled"
        } else if !available {
            "user_unavailable"
        } else if r.client_trust_state == "blocked" {
            "client_blocked"
        } else if !client_allowed(
            settings,
            &r.client_registration_type,
            &r.client_trust_state,
            &r.client_id,
        ) {
            "client_not_approved"
        } else if r.authorization_metadata_hash != r.client_metadata_hash
            || consent_required_at.is_some_and(|at| r.consented_at < at)
        {
            "consent_required"
        } else if scopes.iter().any(|s| !eligible.contains(s)) {
            "access_removed"
        } else {
            "active"
        };
        let tokens: Vec<&(i64, Option<NaiveDateTime>, bool)> =
            token_rows.iter().filter(|(a, _, _)| *a == r.id).collect();
        let last_used = tokens.iter().filter_map(|(_, at, _)| *at).max();
        let time = |t: Option<NaiveDateTime>| {
            t.map_or(Value::Null, |t| json!(crate::topic_list::time_json(t)))
        };
        out.push(json!({
            "id": r.id,
            "username": r.username,
            "client_name": r.client_name,
            "client_id": r.client_id,
            "resource": r.resource,
            "status": status,
            "scopes": scopes,
            "consented_at": time(Some(r.consented_at)),
            "revoked_at": time(r.revoked_at),
            "last_used_at": time(last_used),
            "token_count": tokens.iter().filter(|(_, _, active)| *active).count(),
        }));
    }
    Ok(json!({ "authorizations": out, "meta": { "next_cursor": next_cursor } }))
}

/// `McpOauthAuthorization#revoke!` by an admin, with its staff log.
/// None when there's no such authorization.
pub async fn revoke_authorization(
    conn: &mut PgConnection,
    actor: i32,
    id: i64,
) -> Result<Option<()>, AppError> {
    let row: Option<(i32, i64)> = sqlx::query_as(
        "SELECT user_id, mcp_oauth_client_id FROM mcp_oauth_authorizations WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((user_id, client_id)) = row else {
        return Ok(None);
    };
    revoke(&mut *conn, id, Some(actor), "admin_revoked").await?;
    crate::staff_action_logs::log_custom(
        &mut *conn,
        actor,
        "mcp_authorization_revoked",
        &[
            ("authorization_id", id.to_string()),
            ("subject_user_id", user_id.to_string()),
            ("client_id", client_id.to_string()),
        ],
        None,
        None,
    )
    .await?;
    Ok(Some(()))
}

/// `McpOauthAuthorization#revoke!(by_user:, reason:)`
pub async fn revoke(
    conn: &mut PgConnection,
    id: i64,
    by_user: Option<i32>,
    reason: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE mcp_oauth_authorizations SET status = 'revoked', revoked_at = now(), \
         revoked_by_user_id = $2, revoked_reason = $3, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(by_user)
    .bind(reason)
    .execute(&mut *conn)
    .await?;
    for table in ["mcp_oauth_access_tokens", "mcp_oauth_refresh_tokens"] {
        sqlx::query(&format!(
            "UPDATE {table} SET revoked_at = now() \
             WHERE mcp_oauth_authorization_id = $1 AND revoked_at IS NULL"
        ))
        .bind(id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// McpActivityReport's options.
pub struct ActivityQuery<'a> {
    pub limit: i64,
    pub cursor: Option<i64>,
    pub start_date: Option<&'a Value>,
    pub end_date: Option<&'a Value>,
    pub filter: String,
    pub outcome: Option<String>,
}

/// `Date.iso8601` of a param, Err for the InvalidParameters it raises.
fn parse_date(value: Option<&Value>) -> Result<Option<chrono::NaiveDate>, ()> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .or_else(|_| chrono::NaiveDate::parse_from_str(s, "%Y%m%d"))
            .map(Some)
            .map_err(|_| ()),
        Some(_) => Err(()),
    }
}

/// McpActivityReport#call: the page of audit log entries in the range
/// (the last seven days by default), with the metrics on the first page.
/// Err names the invalid parameter.
pub async fn activity(
    conn: &mut PgConnection,
    q: &ActivityQuery<'_>,
) -> Result<Result<Value, &'static str>, AppError> {
    let Ok(end) = parse_date(q.end_date) else {
        return Ok(Err("end_date"));
    };
    let Ok(start) = parse_date(q.start_date) else {
        return Ok(Err("start_date"));
    };
    // Time.zone is UTC on the reference.
    let today = crate::clock::now().date_naive();
    let end = end.unwrap_or(today);
    let start = start.unwrap_or(today - chrono::Duration::days(6));
    if q.filter.chars().count() > 200 {
        return Ok(Err("filter"));
    }
    if let Some(outcome) = &q.outcome
        && !["success", "error", "rate_limited"].contains(&outcome.as_str())
    {
        return Ok(Err("outcome"));
    }
    if start > end {
        return Ok(Err("start_date"));
    }
    let from = start.and_hms_opt(0, 0, 0).unwrap_or_default();
    let to = end
        .and_hms_micro_opt(23, 59, 59, 999_999)
        .unwrap_or_default();
    let mut filtered = String::from(
        " FROM mcp_audit_logs l LEFT JOIN users u ON u.id = l.user_id \
         LEFT JOIN mcp_oauth_clients c ON c.id = l.mcp_oauth_client_id \
         WHERE l.occurred_at BETWEEN $1 AND $2",
    );
    if q.outcome.is_some() {
        filtered.push_str(" AND l.outcome = $3");
    }
    if !q.filter.is_empty() {
        filtered.push_str(
            " AND (u.username ILIKE $4 OR c.name ILIKE $4 OR c.client_id ILIKE $4 \
             OR l.tool ILIKE $4 OR l.request_id = $5)",
        );
    }
    let pattern = format!("%{}%", crate::ruby::sanitize_sql_like(&q.filter));
    let cursor_sql = if q.cursor.is_some() {
        " AND l.id < $6"
    } else {
        ""
    };
    #[derive(sqlx::FromRow)]
    struct Entry {
        id: i64,
        occurred_at: NaiveDateTime,
        username: Option<String>,
        client_id: Option<String>,
        client_name: Option<String>,
        method: Option<String>,
        tool: Option<String>,
        outcome: String,
        http_status: Option<i32>,
        duration_ms: Option<i32>,
        request_id: Option<String>,
        target: Value,
        occurrences: i32,
    }
    let mut rows: Vec<Entry> = sqlx::query_as(&format!(
        "SELECT l.id, l.occurred_at, u.username, c.client_id, c.name AS client_name, l.method, \
         l.tool, l.outcome, l.http_status, l.duration_ms, l.request_id, l.target, l.occurrences \
         {filtered}{cursor_sql} ORDER BY l.id DESC LIMIT $7"
    ))
    .bind(from)
    .bind(to)
    .bind(q.outcome.as_deref())
    .bind(&pattern)
    .bind(&q.filter)
    .bind(q.cursor.unwrap_or(0))
    .bind(q.limit + 1)
    .fetch_all(&mut *conn)
    .await?;
    let next_cursor = if rows.len() as i64 > q.limit {
        rows.pop();
        rows.last().map(|r| r.id)
    } else {
        None
    };
    let activity: Vec<Value> = rows
        .iter()
        .map(|e| {
            let at = crate::topic_list::time_json(e.occurred_at);
            json!({
                "id": e.id,
                "occurred_at": at,
                "created_at": at,
                "username": e.username,
                "client_id": e.client_id,
                "client_name": e.client_name,
                "method": e.method,
                "tool": e.tool,
                "outcome": e.outcome,
                "http_status": e.http_status,
                "duration_ms": e.duration_ms,
                "request_id": e.request_id,
                "target": e.target,
                "occurrences": e.occurrences,
            })
        })
        .collect();
    let mut out = Map::new();
    out.insert("activity".into(), json!(activity));
    out.insert("next_cursor".into(), json!(next_cursor));
    out.insert("meta".into(), json!({ "next_cursor": next_cursor }));
    if q.cursor.is_none() {
        let (calls, failed, rate_limits, p95): (i64, i64, i64, Option<i32>) = sqlx::query_as(&format!(
            "SELECT \
             COALESCE(SUM(l.occurrences) FILTER (WHERE l.method = 'tools/call' AND l.outcome <> 'rate_limited'), 0)::bigint, \
             COALESCE(SUM(l.occurrences) FILTER (WHERE l.method = 'tools/call' AND l.outcome = 'error'), 0)::bigint, \
             COALESCE(SUM(l.occurrences) FILTER (WHERE l.outcome = 'rate_limited'), 0)::bigint, \
             (PERCENTILE_DISC(0.95) WITHIN GROUP (ORDER BY l.duration_ms) FILTER (WHERE l.method = 'tools/call' \
               AND l.outcome <> 'rate_limited' AND l.duration_ms IS NOT NULL))::integer \
             {filtered}"
        ))
        .bind(from)
        .bind(to)
        .bind(q.outcome.as_deref())
        .bind(&pattern)
        .bind(&q.filter)
        .fetch_one(&mut *conn)
        .await?;
        out.insert(
            "metrics".into(),
            json!({
                "tool_calls": calls,
                "failed_tool_calls": failed,
                "rate_limits": rate_limits,
                "p95_latency_ms": p95,
            }),
        );
    }
    Ok(Ok(Value::Object(out)))
}

/// The settings the recorded availability reads that the definitions
/// don't declare (none, unless the recording and the settings drift).
pub fn check_settings(defs: &Definitions) -> Vec<String> {
    registry()
        .iter()
        .flat_map(|p| p.enabled_setting.iter().chain(p.available_when.iter()))
        .filter(|s| defs.get(s).is_none())
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_recording_is_the_registry() {
        let all = registry();
        assert_eq!(all.len(), 61);
        assert_eq!(all[0].id(), "tool:chat_channel_list");
        assert!(scopes().contains(&INITIAL_SCOPE.to_string()));
        let status = find("tool", "discourse_user_status_set").unwrap();
        assert_eq!(
            status.available_when,
            vec!["enable_user_status".to_string()]
        );
    }

    #[test]
    fn the_recorded_settings_are_declared() {
        let defs = Definitions::vendored().unwrap();
        assert_eq!(check_settings(&defs), Vec::<String>::new());
    }
}
