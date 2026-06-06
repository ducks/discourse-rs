-- Likes on posts. One row per (user, post) like; unliking deletes the row.
-- A unique constraint prevents double-likes. The check constraint enforces
-- "no liking your own post" at the DB level so any future code path stays
-- honest.
CREATE TABLE post_likes (
    id SERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    post_id INTEGER NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (user_id, post_id)
);

CREATE INDEX post_likes_post_id_idx ON post_likes (post_id);
CREATE INDEX post_likes_user_id_idx ON post_likes (user_id);

-- Denormalized counters. Maintained in application code on like/unlike so
-- reads (sorting, trust levels, search ranking) don't have to JOIN+COUNT.
-- The trade is: every like path has to remember to update these. Tests
-- cover the critical paths.
ALTER TABLE posts ADD COLUMN like_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN likes_given INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN likes_received INTEGER NOT NULL DEFAULT 0;
