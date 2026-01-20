use actix_web::{dev::Payload, error::ErrorForbidden, FromRequest, HttpRequest};
use diesel::prelude::*;
use futures::future::{err, ok, Ready};

use crate::schema::users;
use crate::DbPool;

// Trust levels matching Discourse
pub const TRUST_LEVEL_NEW_USER: i32 = 0;
pub const TRUST_LEVEL_BASIC: i32 = 1;
pub const TRUST_LEVEL_MEMBER: i32 = 2;
pub const TRUST_LEVEL_REGULAR: i32 = 3;
pub const TRUST_LEVEL_LEADER: i32 = 4;

/// User info extracted from JWT + database
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub user_id: i32,
    pub username: String,
    pub trust_level: i32,
    pub admin: bool,
    pub moderator: bool,
}

impl CurrentUser {
    pub fn is_admin(&self) -> bool {
        self.admin
    }

    pub fn is_moderator(&self) -> bool {
        self.moderator || self.admin || self.trust_level >= TRUST_LEVEL_LEADER
    }

    pub fn is_staff(&self) -> bool {
        self.is_admin() || self.is_moderator()
    }

    pub fn has_trust_level(&self, level: i32) -> bool {
        self.trust_level >= level
    }
}

/// Helper to extract user from request
fn extract_user(req: &HttpRequest) -> Result<CurrentUser, actix_web::Error> {
    // Get the pool from app data
    let pool = req
        .app_data::<actix_web::web::Data<DbPool>>()
        .ok_or_else(|| ErrorForbidden("Database not configured"))?;

    // Get the auth header
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| ErrorForbidden("Missing authorization header"))?;

    // Parse the Bearer token
    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| ErrorForbidden("Invalid authorization header format"))?;

    // Decode the JWT
    let claims = crate::auth::verify_token(token)
        .map_err(|_| ErrorForbidden("Invalid token"))?;

    // Get user from database
    let mut conn = pool
        .get()
        .map_err(|_| ErrorForbidden("Database connection failed"))?;

    let (username, trust_level, admin, moderator): (String, i32, bool, bool) = users::table
        .find(claims.user_id)
        .select((users::username, users::trust_level, users::admin, users::moderator))
        .first(&mut conn)
        .map_err(|_| ErrorForbidden("User not found"))?;

    Ok(CurrentUser {
        user_id: claims.user_id,
        username,
        trust_level,
        admin,
        moderator,
    })
}

// ============================================================================
// Guard Extractors
// ============================================================================

/// Requires user to be authenticated (any trust level)
#[derive(Debug, Clone)]
pub struct AuthenticatedUser(pub CurrentUser);

impl FromRequest for AuthenticatedUser {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        match extract_user(req) {
            Ok(user) => ok(AuthenticatedUser(user)),
            Err(e) => err(e),
        }
    }
}

/// Requires user to be a moderator (trust level 4, moderator flag, or admin)
#[derive(Debug, Clone)]
pub struct ModeratorGuard(pub CurrentUser);

impl FromRequest for ModeratorGuard {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        match extract_user(req) {
            Ok(user) => {
                if user.is_moderator() {
                    ok(ModeratorGuard(user))
                } else {
                    err(ErrorForbidden("Moderator access required"))
                }
            }
            Err(e) => err(e),
        }
    }
}

/// Requires user to be an admin
#[derive(Debug, Clone)]
pub struct AdminGuard(pub CurrentUser);

impl FromRequest for AdminGuard {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        match extract_user(req) {
            Ok(user) => {
                if user.is_admin() {
                    ok(AdminGuard(user))
                } else {
                    err(ErrorForbidden("Admin access required"))
                }
            }
            Err(e) => err(e),
        }
    }
}

/// Requires user to be staff (admin or moderator)
#[derive(Debug, Clone)]
pub struct StaffGuard(pub CurrentUser);

impl FromRequest for StaffGuard {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        match extract_user(req) {
            Ok(user) => {
                if user.is_staff() {
                    ok(StaffGuard(user))
                } else {
                    err(ErrorForbidden("Staff access required"))
                }
            }
            Err(e) => err(e),
        }
    }
}

/// Requires user to have at least trust level 1 (basic)
#[derive(Debug, Clone)]
pub struct TrustLevel1Guard(pub CurrentUser);

impl FromRequest for TrustLevel1Guard {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        match extract_user(req) {
            Ok(user) => {
                if user.has_trust_level(TRUST_LEVEL_BASIC) {
                    ok(TrustLevel1Guard(user))
                } else {
                    err(ErrorForbidden("Trust level 1 required"))
                }
            }
            Err(e) => err(e),
        }
    }
}

/// Requires user to have at least trust level 2 (member)
#[derive(Debug, Clone)]
pub struct TrustLevel2Guard(pub CurrentUser);

impl FromRequest for TrustLevel2Guard {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        match extract_user(req) {
            Ok(user) => {
                if user.has_trust_level(TRUST_LEVEL_MEMBER) {
                    ok(TrustLevel2Guard(user))
                } else {
                    err(ErrorForbidden("Trust level 2 required"))
                }
            }
            Err(e) => err(e),
        }
    }
}

/// Requires user to have at least trust level 3 (regular)
#[derive(Debug, Clone)]
pub struct TrustLevel3Guard(pub CurrentUser);

impl FromRequest for TrustLevel3Guard {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        match extract_user(req) {
            Ok(user) => {
                if user.has_trust_level(TRUST_LEVEL_REGULAR) {
                    ok(TrustLevel3Guard(user))
                } else {
                    err(ErrorForbidden("Trust level 3 required"))
                }
            }
            Err(e) => err(e),
        }
    }
}
