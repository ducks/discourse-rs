//! Discourse ID's site registration (app/services/discourse_id): the
//! challenge flow both Register and RegenerateCredentials go through
//! (DiscourseId::Concerns::ChallengeFlow), and the challenge Discourse ID
//! reads back at /.well-known/discourse-id-challenge
//! (MetadataController#discourse_id_challenge).

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::site_settings::{Definitions, SiteSettings};
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// Where the challenge token is kept for Discourse ID to read back (Redis
/// in Discourse), and for how long.
const CHALLENGE_KEY: &str = "discourse_id_challenge_token";
const CHALLENGE_SECONDS: i64 = 600;

/// What a flow needs: the site's settings and where to save new ones.
pub struct Cx<'a> {
    pub config: &'a crate::config::Config,
    pub bus: &'a pg_bus::Bus,
    pub defs: &'a Definitions,
    pub i18n: &'a crate::i18n::I18n,
    pub settings: &'a SiteSettings,
}

impl Cx<'_> {
    fn urls(&self) -> Urls<'_> {
        Urls {
            config: self.config,
            settings: self.settings,
        }
    }

    /// `DiscourseId.provider_url`
    fn provider_url(&self) -> Result<String, AppError> {
        Ok(self
            .settings
            .get("discourse_id_provider_url")?
            .presence()
            .unwrap_or_else(|| "https://id.discourse.com".to_string()))
    }

    /// `SiteSetting.name = value` (add_override!, no staff log).
    async fn set(
        &self,
        conn: &mut PgConnection,
        name: &str,
        value: &Value,
    ) -> Result<(), AppError> {
        match crate::site_setting_update::set(
            conn,
            self.bus,
            self.settings,
            self.defs,
            self.i18n,
            name,
            value,
        )
        .await?
        {
            Ok(_) => Ok(()),
            Err(message) => Err(std::io::Error::other(message).into()),
        }
    }
}

/// How a flow ended: done, its policy refusing, or a step failing with
/// its error (`fail!`).
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Done,
    PolicyFailed,
    Failed(String),
}

/// `post_json(path, body)`: the provider's JSON, or the step's error.
async fn post_json(
    cx: &Cx<'_>,
    path: &str,
    body: &Value,
) -> Result<Result<Value, String>, AppError> {
    let url = format!("{}{path}", cx.provider_url()?);
    // Net::HTTP's default open and read timeouts.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(60))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(std::io::Error::other)?;
    let response = match client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.to_string())
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let error = format!("Request to '{url}' failed: {e}.");
            log_error(path, &error);
            return Ok(Err(error));
        }
    };
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if status != 200 {
        let error = format!("Request to '{path}' failed: {status}\nError: {text}");
        log_error(path, &error);
        return Ok(Err(error));
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(json) => Ok(Ok(json)),
        Err(e) => {
            let error = format!("Response from '{path}' invalid JSON: {e}");
            log_error(path, &error);
            Ok(Err(error))
        }
    }
}

fn log_error(step: &str, message: &str) {
    tracing::error!(step, message, "Discourse ID request failed");
}

/// `request_challenge` and `store_challenge_token`: a token for this
/// site's domain (and path), kept for Discourse ID to read back.
async fn challenge(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
) -> Result<Result<String, String>, AppError> {
    let urls = cx.urls();
    let domain = urls.current_hostname()?;
    let path = cx.config.globals.relative_url_root().to_string();
    let mut body = json!({ "domain": domain });
    if !path.is_empty() {
        body["path"] = json!(path);
    }
    let json = match post_json(cx, "/challenge", &body).await? {
        Ok(json) => json,
        Err(error) => return Ok(Err(error)),
    };
    let got = |key: &str| match &json[key] {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if json["domain"].as_str() != Some(domain.as_str()) {
        let error = format!(
            "Domain mismatch in challenge response (expected: {domain}, got: {})",
            got("domain")
        );
        log_error("request_challenge", &error);
        return Ok(Err(error));
    }
    if !path.is_empty() && json["path"].as_str() != Some(path.as_str()) {
        let error = format!(
            "Path mismatch in challenge response (expected: {path}, got: {})",
            got("path")
        );
        log_error("request_challenge", &error);
        return Ok(Err(error));
    }
    // Redis's setex of the token's to_s.
    let token = got("token");
    crate::owned_schema::cached_setex(&mut *conn, CHALLENGE_KEY, CHALLENGE_SECONDS, &token).await?;
    Ok(Ok(token))
}

/// GET /.well-known/discourse-id-challenge's JSON: the token waiting for
/// Discourse ID with the site's domain (and path), None without one.
pub async fn challenge_json(
    conn: &mut PgConnection,
    config: &crate::config::Config,
    settings: &SiteSettings,
) -> Result<Option<Value>, AppError> {
    let Some(token) = crate::owned_schema::cached_get(&mut *conn, CHALLENGE_KEY).await? else {
        return Ok(None);
    };
    if crate::ruby::is_blank(&token) {
        return Ok(None);
    }
    let urls = Urls { config, settings };
    let mut out = json!({ "token": token, "domain": urls.current_hostname()? });
    let path = config.globals.relative_url_root();
    if !path.is_empty() {
        out["path"] = json!(path);
    }
    Ok(Some(out))
}

/// `SiteSetting.site_#{name}_url`: the upload's full CDN URL, "" without.
async fn upload_url(conn: &mut PgConnection, cx: &Cx<'_>, name: &str) -> Result<String, AppError> {
    let id = cx.settings.get(name)?.to_i();
    if id == 0 {
        return Ok(String::new());
    }
    let url: Option<String> = sqlx::query_scalar("SELECT url FROM uploads WHERE id = $1")
        .bind(i32::try_from(id).unwrap_or(0))
        .fetch_optional(&mut *conn)
        .await?;
    match url {
        Some(url) => Ok(cx.urls().full_cdn_url(&url)?),
        None => Ok(String::new()),
    }
}

fn credential(cx: &Cx<'_>, name: &str) -> Result<String, AppError> {
    Ok(cx.settings.get(name)?.to_s().to_string())
}

/// DiscourseId::Register: registers the site (or, with `update`, its
/// metadata) with Discourse ID, keeping the credentials it returns.
/// Refused by its policy when the site is registered already, unless
/// forced or updating. Logged by `actor` (the system user without one).
pub async fn register(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    force: bool,
    update: bool,
    actor: Option<i32>,
) -> Result<Outcome, AppError> {
    let client_id = credential(cx, "discourse_id_client_id")?;
    let client_secret = credential(cx, "discourse_id_client_secret")?;
    // not_already_registered?
    if !force
        && !update
        && !(crate::ruby::is_blank(&client_id) && crate::ruby::is_blank(&client_secret))
    {
        return Ok(Outcome::PolicyFailed);
    }
    let token = match challenge(conn, cx).await? {
        Ok(token) => token,
        Err(error) => return Ok(Outcome::Failed(error)),
    };
    // register_with_challenge: the body less its nil values (compact).
    let presence = |s: String| (!crate::ruby::is_blank(&s)).then_some(s);
    let mut body = serde_json::Map::new();
    body.insert(
        "client_name".into(),
        json!(cx.settings.get("title")?.to_s()),
    );
    body.insert(
        "redirect_uri".into(),
        json!(format!(
            "{}/auth/discourse_id/callback",
            cx.urls().base_url()?
        )),
    );
    body.insert("challenge_token".into(), json!(token));
    if let Some(url) = presence(upload_url(conn, cx, "logo").await?) {
        body.insert("logo_uri".into(), json!(url));
    }
    if let Some(url) = presence(upload_url(conn, cx, "logo_small").await?) {
        body.insert("logo_small_uri".into(), json!(url));
    }
    if let Some(description) = presence(cx.settings.get("site_description")?.to_s().to_string()) {
        body.insert("description".into(), json!(description));
    }
    if update {
        body.insert("update".into(), json!(true));
        body.insert("client_id".into(), json!(client_id));
        body.insert("client_secret".into(), json!(client_secret));
    }
    let data = match post_json(cx, "/register", &Value::Object(body)).await? {
        Ok(data) => data,
        Err(error) => return Ok(Outcome::Failed(error)),
    };
    if update {
        return Ok(Outcome::Done);
    }
    // store_credentials, then log_action.
    cx.set(conn, "discourse_id_client_id", &data["client_id"])
        .await?;
    cx.set(conn, "discourse_id_client_secret", &data["client_secret"])
        .await?;
    let new_id = match &data["client_id"] {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    crate::staff_action_logs::log_custom(
        conn,
        actor.unwrap_or(-1),
        "discourse_id_register",
        &[("client_id", new_id)],
        None,
        None,
    )
    .await?;
    Ok(Outcome::Done)
}

/// DiscourseId::RegenerateCredentials: a new client secret for the
/// site's client, logged by `actor`.
pub async fn regenerate(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    actor: i32,
) -> Result<Outcome, AppError> {
    let client_id = credential(cx, "discourse_id_client_id")?;
    let client_secret = credential(cx, "discourse_id_client_secret")?;
    // credentials_configured?
    if crate::ruby::is_blank(&client_id) || crate::ruby::is_blank(&client_secret) {
        return Ok(Outcome::PolicyFailed);
    }
    let token = match challenge(conn, cx).await? {
        Ok(token) => token,
        Err(error) => return Ok(Outcome::Failed(error)),
    };
    let body = json!({
        "client_id": client_id,
        "client_secret": client_secret,
        "challenge_token": token,
    });
    let data = match post_json(cx, "/regenerate", &body).await? {
        Ok(data) => data,
        Err(error) => return Ok(Outcome::Failed(error)),
    };
    if !data.is_object() {
        // `data["client_secret"]` on an array or a string raises.
        return Err(Unsupported("a Discourse ID regenerate response that isn't an object").into());
    }
    cx.set(conn, "discourse_id_client_secret", &data["client_secret"])
        .await?;
    crate::staff_action_logs::log_custom(
        conn,
        actor,
        "discourse_id_regenerate_credentials",
        &[("client_id", client_id)],
        None,
        None,
    )
    .await?;
    Ok(Outcome::Done)
}

/// GET /.well-known/discourse-id-challenge(.json): the token for Discourse
/// ID to check, cached for five minutes (`expires_in`); not found without
/// one. Open to anyone, login required or not.
pub async fn challenge_page(
    axum::extract::State(state): axum::extract::State<crate::AppState>,
) -> Result<axum::response::Response, AppError> {
    use axum::response::IntoResponse;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    match challenge_json(&mut conn, &state.config, &settings).await? {
        Some(json) => Ok((
            [(axum::http::header::CACHE_CONTROL, "max-age=300, private")],
            axum::Json(json),
        )
            .into_response()),
        None => Ok(crate::routes::topics::not_found_response(&state)),
    }
}
