use crate::middleware::{AuthMiddleware, ReadAuthMiddleware};

/// Helper macros for configuring routes with authentication
///
/// These macros make it easy for plugins to apply auth to their routes:
///
/// - `readable!`: Routes respect `require_auth_for_reads` setting (public by default)
/// - `writable!`: Routes always require authentication
/// - `protected!`: Routes always require authentication (alias for writable)

/// Configure GET endpoints that respect the require_auth_for_reads setting
#[macro_export]
macro_rules! readable {
    ($($service:expr),+ $(,)?) => {{
        use actix_web::web;
        use $crate::middleware::ReadAuthMiddleware;

        web::scope("")
            .wrap(ReadAuthMiddleware)
            $(.service($service))+
    }};
}

/// Configure POST/PUT/DELETE endpoints that always require authentication
#[macro_export]
macro_rules! writable {
    ($($service:expr),+ $(,)?) => {{
        use actix_web::web;
        use $crate::middleware::AuthMiddleware;

        web::scope("")
            .wrap(AuthMiddleware)
            $(.service($service))+
    }};
}

/// Configure endpoints that always require authentication
#[macro_export]
macro_rules! protected {
    ($($service:expr),+ $(,)?) => {{
        use actix_web::web;
        use $crate::middleware::AuthMiddleware;

        web::scope("")
            .wrap(AuthMiddleware)
            $(.service($service))+
    }};
}
