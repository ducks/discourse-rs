ALTER TABLE users DROP COLUMN likes_received;
ALTER TABLE users DROP COLUMN likes_given;
ALTER TABLE posts DROP COLUMN like_count;

DROP TABLE post_likes;
