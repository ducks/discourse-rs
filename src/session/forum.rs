//! The `_forum_session` cookie: Rails' cookie store session, which holds
//! the CSRF token and the server session id.

use axum::http::HeaderMap;

use super::cookie::{Map, Scalar};
use super::current::{self, SESSION_COOKIE};
use super::{csrf, token};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// The `_forum_session` cookie's content, created when absent.
pub(crate) struct ForumSession {
    pub(crate) map: Map,
    pub(crate) changed: bool,
}

pub(crate) fn load_forum_session(state: &AppState, headers: &HeaderMap) -> ForumSession {
    let existing = current::cookie(headers, SESSION_COOKIE)
        .and_then(|raw| state.keys.codec.decrypt(SESSION_COOKIE, raw).ok());
    match existing {
        Some(map) => ForumSession {
            map,
            changed: false,
        },
        None => {
            let mut map = Map::new();
            map.insert(
                "session_id".into(),
                Scalar::Str(token::new_unhashed_token()),
            );
            ForumSession { map, changed: true }
        }
    }
}

impl ForumSession {
    pub(crate) fn csrf_token(&mut self) -> String {
        if let Some(Scalar::Str(t)) = self.map.get("_csrf_token") {
            return t.clone();
        }
        let t = csrf::generate();
        self.map
            .insert("_csrf_token".into(), Scalar::Str(t.clone()));
        self.changed = true;
        t
    }

    /// `Set-Cookie` for the session cookie, when the session changed.
    pub(crate) fn set_cookie(
        &self,
        state: &AppState,
        settings: &SiteSettings,
    ) -> Result<Option<String>, AppError> {
        if !self.changed {
            return Ok(None);
        }
        let pairs: Vec<(&str, Scalar)> = self
            .map
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect();
        let value = state
            .keys
            .codec
            .encrypt(SESSION_COOKIE, &pairs, false, None);
        let mut cookie = format!(
            "{SESSION_COOKIE}={value}; path={}; HttpOnly",
            if state.config.globals.relative_url_root().is_empty() {
                "/"
            } else {
                state.config.globals.relative_url_root()
            }
        );
        if settings.get("force_https")?.truthy() {
            cookie.push_str("; Secure");
        }
        let same_site = settings.get("same_site_cookies")?.to_s();
        if same_site != "Disabled" && !same_site.is_empty() {
            cookie.push_str(&format!("; SameSite={same_site}"));
        }
        Ok(Some(cookie))
    }
}

impl ForumSession {
    /// `session[:server_session_id] ||= session.delete(:secure_session_id) ||
    /// SecureRandom.hex`: the key of this browser's server session.
    pub(crate) fn server_session_id(&mut self) -> String {
        if let Some(Scalar::Str(id)) = self.map.get("server_session_id") {
            return id.clone();
        }
        let id = match self.map.remove("secure_session_id") {
            Some(Scalar::Str(id)) => id,
            _ => crate::accounts::random_hex(),
        };
        self.map
            .insert("server_session_id".into(), Scalar::Str(id.clone()));
        self.changed = true;
        id
    }
}
