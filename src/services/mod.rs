//! Service-layer functions: business logic that operates directly on a
//! `&mut PgConnection`. Route handlers are thin wrappers that own the
//! HTTP concerns (auth extraction, JSON serialization, status codes) and
//! delegate the actual work here.
//!
//! Why split this out: services are easy to test against a real DB without
//! spinning up actix, and the same logic is reusable from background jobs.

pub mod likes;
pub mod user_stats;
