use actix_web::web;

pub mod auth;
pub mod categories;
pub mod jobs;
pub mod likes;
pub mod moderation;
pub mod notifications;
pub mod posts;
pub mod reads;
pub mod search;
pub mod settings;
pub mod topics;
pub mod users;

pub fn config(cfg: &mut web::ServiceConfig) {
    cfg.configure(auth::configure)
        .configure(settings::configure)
        .configure(users::configure)
        .configure(categories::configure)
        .configure(topics::configure)
        .configure(posts::configure)
        .configure(likes::configure)
        .configure(reads::configure)
        .configure(jobs::configure)
        .configure(moderation::configure)
        .configure(notifications::configure)
        .configure(search::configure);
}
