//! Authentication extractors.
//!
//! Auth is enforced per-handler via FromRequest extractors, not via a
//! global middleware. The middleware approach was rejected because actix
//! scope-level `.wrap(...)` requires the middleware to live in a `scope`,
//! and stacking multiple `scope("")` siblings makes only the first one
//! reachable — see commit history for the bug this replaces.
//!
//! Three extractors are provided:
//!
//! - `AuthUser`: required auth. Returns 401 if no valid Bearer token.
//! - `MaybeAuthUser`: optional auth. Always succeeds; returns `Some(Claims)`
//!   if a valid token is present, `None` otherwise. Use for endpoints that
//!   want to *know* the user when available without requiring it.
//! - `ReadAuthUser`: respects the `require_auth_for_reads` site setting.
//!   Behaves like `MaybeAuthUser` when the setting is false; behaves like
//!   `AuthUser` when it's true.

use actix_web::{FromRequest, HttpRequest, dev::Payload, error::ErrorUnauthorized, web};
use std::future::{Ready, ready};

use crate::DbPool;
use crate::auth::{Claims, verify_token};
use crate::config::require_auth_for_reads;

/// Parse and verify a Bearer token from the request's Authorization header.
fn claims_from_request(req: &HttpRequest) -> Option<Claims> {
    let header = req
        .headers()
        .get("Authorization")
        .and_then(|h| h.to_str().ok())?;
    let token = header.strip_prefix("Bearer ")?;
    verify_token(token).ok()
}

/// Required-auth extractor. Errors 401 if no valid Bearer token is present.
pub struct AuthUser(pub Claims);

impl FromRequest for AuthUser {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        match claims_from_request(req) {
            Some(claims) => ready(Ok(AuthUser(claims))),
            None => ready(Err(ErrorUnauthorized(serde_json::json!({
                "error": "Missing or invalid authorization token"
            })))),
        }
    }
}

/// Optional-auth extractor. Always succeeds; populated only if a valid
/// token is present. Useful for endpoints that personalize for the caller
/// when known but otherwise work anonymously.
pub struct MaybeAuthUser(pub Option<Claims>);

impl FromRequest for MaybeAuthUser {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        ready(Ok(MaybeAuthUser(claims_from_request(req))))
    }
}

/// Site-setting-aware auth extractor for read endpoints. Falls back to
/// `MaybeAuthUser` behavior when `require_auth_for_reads` is false, and
/// upgrades to required-auth behavior when it's true.
pub struct ReadAuthUser(pub Option<Claims>);

impl FromRequest for ReadAuthUser {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        let pool = req.app_data::<web::Data<DbPool>>();
        let require_auth = pool.map(|p| require_auth_for_reads(p)).unwrap_or(false);
        let claims = claims_from_request(req);

        if require_auth && claims.is_none() {
            return ready(Err(ErrorUnauthorized(serde_json::json!({
                "error": "Authentication required"
            }))));
        }
        ready(Ok(ReadAuthUser(claims)))
    }
}
