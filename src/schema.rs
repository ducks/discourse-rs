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
    moderation_actions (id) {
        id -> Int8,
        #[max_length = 50]
        action_type -> Varchar,
        moderator_id -> Int4,
        target_user_id -> Nullable<Int4>,
        target_topic_id -> Nullable<Int4>,
        target_post_id -> Nullable<Int4>,
        details -> Nullable<Jsonb>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    notifications (id) {
        id -> Int8,
        user_id -> Int4,
        #[max_length = 50]
        notification_type -> Varchar,
        data -> Jsonb,
        read -> Bool,
        topic_id -> Nullable<Int4>,
        post_id -> Nullable<Int4>,
        acting_user_id -> Nullable<Int4>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    post_likes (id) {
        id -> Int4,
        user_id -> Int4,
        post_id -> Int4,
        created_at -> Timestamp,
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
        hidden -> Bool,
        hidden_at -> Nullable<Timestamptz>,
        hidden_by_user_id -> Nullable<Int4>,
        deleted_by_user_id -> Nullable<Int4>,
        like_count -> Int4,
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
    topic_views (id) {
        id -> Int4,
        user_id -> Int4,
        topic_id -> Int4,
        first_viewed_at -> Timestamp,
        last_viewed_at -> Timestamp,
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
        locked -> Bool,
        pinned_at -> Nullable<Timestamptz>,
        closed_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    user_stats (user_id) {
        user_id -> Int4,
        post_count -> Int4,
        topic_count -> Int4,
        time_read -> Int4,
        posts_read_count -> Int4,
        topics_entered -> Int4,
        days_visited -> Int4,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    user_suspensions (id) {
        id -> Int8,
        user_id -> Int4,
        suspended_by_user_id -> Int4,
        reason -> Text,
        suspended_at -> Timestamptz,
        suspended_until -> Timestamptz,
        created_at -> Timestamptz,
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
        likes_given -> Int4,
        likes_received -> Int4,
    }
}

diesel::joinable!(moderation_actions -> posts (target_post_id));
diesel::joinable!(moderation_actions -> topics (target_topic_id));
diesel::joinable!(notifications -> posts (post_id));
diesel::joinable!(notifications -> topics (topic_id));
diesel::joinable!(post_likes -> posts (post_id));
diesel::joinable!(post_likes -> users (user_id));
diesel::joinable!(posts -> topics (topic_id));
diesel::joinable!(topic_views -> topics (topic_id));
diesel::joinable!(topic_views -> users (user_id));
diesel::joinable!(topics -> categories (category_id));
diesel::joinable!(topics -> users (user_id));
diesel::joinable!(user_stats -> users (user_id));

diesel::allow_tables_to_appear_in_same_query!(
    backie_tasks,
    categories,
    moderation_actions,
    notifications,
    post_likes,
    posts,
    site_settings,
    topic_views,
    topics,
    user_stats,
    user_suspensions,
    users,
);
