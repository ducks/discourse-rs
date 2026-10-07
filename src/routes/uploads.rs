//! Port of UploadsController#create: POST /uploads.json, a multipart form
//! with the file and its upload_type.

use axum::Json;
use axum::extract::{FromRequest, Multipart, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use super::session::{bad_csrf, csrf_ok};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::uploads::{self, NewUpload, Outcome};
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// The form's fields: text values, and the file's name and bytes.
#[derive(Default)]
struct Form {
    fields: Vec<(String, String)>,
    file: Option<(String, Vec<u8>)>,
}

impl Form {
    fn get(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

async fn read_form(request: Request, state: &AppState) -> Result<Form, AppError> {
    let mut form = Form::default();
    if let Some(query) = request.uri().query() {
        for (k, v) in form_urlencoded::parse(query.as_bytes()) {
            form.fields.push((k.into_owned(), v.into_owned()));
        }
    }
    let multipart = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("multipart/form-data"));
    if !multipart {
        return Err(Unsupported("uploads not sent as multipart/form-data").into());
    }
    let mut parts = Multipart::from_request(request, state)
        .await
        .map_err(|e| std::io::Error::other(e.body_text()))?;
    while let Some(field) = parts
        .next_field()
        .await
        .map_err(|e| std::io::Error::other(e.body_text()))?
    {
        let name = field.name().unwrap_or_default().to_string();
        match field.file_name().map(str::to_string) {
            Some(filename) if name == "file" || name == "files[]" => {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| std::io::Error::other(e.body_text()))?;
                if form.file.is_none() {
                    form.file = Some((filename, bytes.to_vec()));
                }
            }
            Some(_) => return Err(Unsupported("upload form files other than file").into()),
            None => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| std::io::Error::other(e.body_text()))?;
                form.fields.push((name, text));
            }
        }
    }
    Ok(form)
}

/// `String#parameterize(separator: "_")` for an ASCII upload type.
fn parameterize(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        if c.is_ascii_alphanumeric() || c == '-' {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

fn errors(status: StatusCode, messages: Vec<String>) -> Response {
    (status, Json(json!({ "errors": messages }))).into_response()
}

/// POST /uploads/lookup-urls: per short url of an upload that exists, its
/// url and short path (`PrettyText::Helpers.lookup_upload_urls`), for the
/// composer's preview. Secure uploads, which would need a permission check
/// here, are refused while cooking already.
pub async fn lookup_urls(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: axum::http::Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    let form: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !csrf_ok(&state, &headers, &form, uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    if guardian.user_id().is_none() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let short_urls = match p.get("short_urls") {
        Some(serde_json::Value::Array(urls)) => serde_json::Value::Array(urls.clone()),
        _ => serde_json::Value::Array(Vec::new()),
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let host = crate::pretty_text::Host::from_state(&state);
    let found = crate::pretty_text::helpers::Helpers {
        host: &host,
        conn: &mut conn,
        settings: &settings,
    }
    .upload_urls(&short_urls)
    .await?;
    let uploads: Vec<serde_json::Value> = found
        .as_object()
        .into_iter()
        .flatten()
        .map(|(short_url, paths)| {
            json!({
                "short_url": short_url,
                "url": paths["url"],
                "short_path": paths["short_path"],
            })
        })
        .collect();
    Ok(Json(uploads).into_response())
}

/// POST /uploads.json
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    request: Request,
) -> Result<Response, AppError> {
    let path = request.uri().path().to_string();
    let form = read_form(request, &state).await?;
    if !csrf_ok(&state, &headers, &form.fields, &path, "POST") {
        return Ok(bad_csrf());
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let upload_type = match form.get("upload_type").filter(|t| !t.trim().is_empty()) {
        Some(t) => t,
        None if form.get("type").is_some_and(|t| !t.trim().is_empty()) => {
            return Err(Unsupported("the deprecated type param").into());
        }
        None => return Ok(super::accounts::param_missing("upload_type")),
    };
    let upload_type: String = parameterize(upload_type).chars().take(51).collect();
    if upload_type == "avatar" {
        let mut conn = state.pool.acquire().await?;
        let settings =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
        if settings.get("discourse_connect_overrides_avatar")?.truthy()
            || settings.get("auth_overrides_avatar")?.truthy()
            || !guardian.in_setting_groups(&settings, "uploaded_avatars_allowed_groups")?
        {
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"failed": "FAILED"})),
            )
                .into_response());
        }
    }
    if form.get("for_site_setting") == Some("true") {
        return Err(Unsupported("site setting uploads").into());
    }
    if form
        .get("retain_hours")
        .is_some_and(|h| crate::ruby::to_i(h) > 0)
        && guardian.is_admin()
    {
        return Err(Unsupported("retain_hours").into());
    }
    let Some((filename, bytes)) = &form.file else {
        if form.get("url").is_some_and(|u| !u.trim().is_empty())
            && headers.contains_key(crate::session::api_key::HEADER_API_KEY)
        {
            return Err(Unsupported("uploads downloaded from a url").into());
        }
        let message = state
            .i18n
            .t("upload.file_missing")
            .unwrap_or("upload.file_missing");
        return Ok(errors(
            StatusCode::UNPROCESSABLE_ENTITY,
            vec![message.to_string()],
        ));
    };

    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let up = NewUpload {
        user_id,
        staff: guardian.is_staff(),
        upload_type: &upload_type,
        filename,
        bytes,
        pasted: form.get("pasted") == Some("true"),
        for_private_message: form.get("for_private_message") == Some("true"),
    };
    let outcome = uploads::create(
        &mut conn,
        &settings,
        &state.i18n,
        &urls,
        &crate::file_store::FileStore::for_site(&state.config, &settings)?,
        &up,
    )
    .await?;
    Ok(match outcome {
        Outcome::Created(upload) => (StatusCode::OK, Json(upload)).into_response(),
        Outcome::Invalid(messages) => errors(StatusCode::UNPROCESSABLE_ENTITY, messages),
    })
}

#[cfg(test)]
mod tests {
    use super::parameterize;

    #[test]
    fn upload_types_parameterize() {
        assert_eq!(parameterize("composer"), "composer");
        assert_eq!(parameterize("Custom Emoji!"), "custom_emoji");
    }
}
