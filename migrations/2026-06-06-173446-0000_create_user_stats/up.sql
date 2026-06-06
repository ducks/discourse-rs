-- Per-user aggregated activity counters. Updated incrementally by the
-- services that own the underlying events (post create bumps post_count,
-- topic create bumps topic_count, etc). The trust-level evaluator reads
-- this table; nothing else should read these counters in hot paths.
--
-- One row per user, created on user signup. ON DELETE CASCADE so deleting
-- a user wipes their stats too.

CREATE TABLE user_stats (
    user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,

    -- Content the user has produced.
    post_count INTEGER NOT NULL DEFAULT 0,
    topic_count INTEGER NOT NULL DEFAULT 0,

    -- Reading activity. Populated by the read-tracker endpoint in a
    -- later PR; columns exist now so we can backfill without a migration.
    time_read INTEGER NOT NULL DEFAULT 0,          -- seconds, capped server-side
    posts_read_count INTEGER NOT NULL DEFAULT 0,
    topics_entered INTEGER NOT NULL DEFAULT 0,
    days_visited INTEGER NOT NULL DEFAULT 0,

    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT user_stats_non_negative CHECK (
        post_count >= 0
        AND topic_count >= 0
        AND time_read >= 0
        AND posts_read_count >= 0
        AND topics_entered >= 0
        AND days_visited >= 0
    )
);

-- Backfill: create a stats row for every existing user. Idempotent because
-- ON CONFLICT is implicit (PRIMARY KEY collision raises) — but this is a
-- fresh migration so no conflicts expected.
INSERT INTO user_stats (user_id, post_count, topic_count)
SELECT
    u.id,
    (SELECT COUNT(*) FROM posts p WHERE p.user_id = u.id AND p.deleted_at IS NULL),
    (SELECT COUNT(*) FROM topics t WHERE t.user_id = u.id)
FROM users u;
