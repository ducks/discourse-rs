use utoipa::OpenApi;

use crate::models::{
    Category, CreatePostInput, NewCategory, NewTopic, NewUser, Notification, Post, Topic,
    UpdateCategory, UpdatePostInput, UpdateTopic, UpdateUser, User,
};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Discourse-rs API",
        version = "0.1.0",
        description = "A Discourse-inspired forum platform built in Rust",
        license(name = "MIT")
    ),
    tags(
        (name = "users", description = "User management endpoints"),
        (name = "topics", description = "Topic management endpoints"),
        (name = "posts", description = "Post management endpoints"),
        (name = "categories", description = "Category management endpoints"),
        (name = "notifications", description = "Notification endpoints"),
        (name = "search", description = "Search endpoints"),
        (name = "moderation", description = "Moderation endpoints"),
        (name = "auth", description = "Authentication endpoints")
    ),
    components(
        schemas(
            User, NewUser, UpdateUser,
            Topic, NewTopic, UpdateTopic,
            Post, CreatePostInput, UpdatePostInput,
            Category, NewCategory, UpdateCategory,
            Notification
        )
    )
)]
pub struct ApiDoc;
