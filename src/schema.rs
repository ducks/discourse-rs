// @generated automatically by Diesel CLI.

diesel::table! {
    backie_tasks (id) {
        id -> Uuid,
        task_name -> Text,
        task_hash -> Text,
        payload -> Jsonb,
        timeout_msecs -> Int8,
        max_retries -> Int4,
        retries -> Int4,
        created_at -> Timestamptz,
        scheduled_at -> Timestamptz,
        running_at -> Nullable<Timestamptz>,
        done_at -> Nullable<Timestamptz>,
        error -> Nullable<Text>,
    }
}

diesel::table! {
    categories (id) {
        id -> Int4,
        #[max_length = 100]
        name -> Varchar,
        #[max_length = 100]
        slug -> Varchar,
        description -> Nullable<Text>,
        #[max_length = 6]
        color -> Varchar,
        position -> Int4,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    posts (id) {
        id -> Int4,
        topic_id -> Int4,
        user_id -> Int4,
        post_number -> Int4,
        raw -> Text,
        cooked -> Text,
        reply_to_post_number -> Nullable<Int4>,
        deleted_at -> Nullable<Timestamp>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    site_settings (key) {
        key -> Varchar,
        value -> Varchar,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    topics (id) {
        id -> Int4,
        #[max_length = 255]
        title -> Varchar,
        #[max_length = 255]
        slug -> Varchar,
        user_id -> Int4,
        category_id -> Nullable<Int4>,
        views -> Int4,
        posts_count -> Int4,
        pinned -> Bool,
        closed -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    users (id) {
        id -> Int4,
        #[max_length = 60]
        username -> Varchar,
        #[max_length = 254]
        email -> Varchar,
        #[max_length = 255]
        password_hash -> Varchar,
        admin -> Bool,
        moderator -> Bool,
        trust_level -> Int4,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::joinable!(posts -> topics (topic_id));
diesel::joinable!(posts -> users (user_id));
diesel::joinable!(topics -> categories (category_id));
diesel::joinable!(topics -> users (user_id));

diesel::allow_tables_to_appear_in_same_query!(
    backie_tasks,
    categories,
    posts,
    site_settings,
    topics,
    users,
);
