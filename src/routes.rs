use actix_web::web;

pub mod posts;
pub mod topics;
pub mod users;

pub fn config(cfg: &mut web::ServiceConfig) {
    cfg.configure(users::configure)
        .configure(topics::configure)
        .configure(posts::configure);
}
