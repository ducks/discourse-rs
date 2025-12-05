use actix_web::web;

pub mod auth;
pub mod posts;
pub mod settings;
pub mod topics;
pub mod users;

pub fn config(cfg: &mut web::ServiceConfig) {
    cfg.configure(auth::configure)
        .configure(settings::configure)
        .configure(users::configure)
        .configure(topics::configure)
        .configure(posts::configure);
}
