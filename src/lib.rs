//! Discourse-rs library exports for binaries and tests.
//!
//! `main.rs` is a thin runtime entrypoint that imports from this library.
//! Integration tests under `tests/` consume the same exports, so anything a
//! route handler needs (types, modules, macros) must be reachable from here.

pub mod auth;
pub mod config;
pub mod guardian;
pub mod jobs;
pub mod markdown;
pub mod middleware;
pub mod models;
pub mod moderation;
pub mod openapi;
pub mod pagination;
pub mod route_helpers;
pub mod routes;
pub mod schema;

use diesel::pg::PgConnection;
use diesel::r2d2::{self, ConnectionManager};

/// Shared connection pool type used across the app and tests.
pub type DbPool = r2d2::Pool<ConnectionManager<PgConnection>>;
