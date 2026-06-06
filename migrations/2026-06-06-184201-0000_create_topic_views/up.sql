-- Per-(user, topic) view tracking. One row the first time a user enters
-- a topic; thereafter last_viewed_at is updated on each subsequent view.
--
-- The UNIQUE constraint enables idempotent upserts via ON CONFLICT —
-- callers don't have to first SELECT then decide which way to write.
--
-- ON DELETE CASCADE in both FKs so deleting a user or topic wipes their
-- view rows; nothing depends on the history of a deleted entity.

CREATE TABLE topic_views (
    id SERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    topic_id INTEGER NOT NULL REFERENCES topics(id) ON DELETE CASCADE,
    first_viewed_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_viewed_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (user_id, topic_id)
);

-- "All topics a user has entered" is the hot query (powers the trust-
-- level evaluator's topics_entered count). The unique index above
-- already covers it, but an explicit user_id index makes the intent
-- clear and lets pg pick it without prefix-matching the composite.
CREATE INDEX topic_views_user_id_idx ON topic_views (user_id);
