-- Drop moderation_actions table
DROP TABLE IF EXISTS moderation_actions;

-- Drop user_suspensions table
DROP TABLE IF EXISTS user_suspensions;

-- Remove moderation fields from posts
ALTER TABLE posts DROP COLUMN IF EXISTS deleted_by_user_id;
ALTER TABLE posts DROP COLUMN IF EXISTS deleted_at;
ALTER TABLE posts DROP COLUMN IF EXISTS hidden_by_user_id;
ALTER TABLE posts DROP COLUMN IF EXISTS hidden_at;
ALTER TABLE posts DROP COLUMN IF EXISTS hidden;

-- Remove moderation fields from topics
ALTER TABLE topics DROP COLUMN IF EXISTS closed_at;
ALTER TABLE topics DROP COLUMN IF EXISTS pinned_at;
ALTER TABLE topics DROP COLUMN IF EXISTS closed;
ALTER TABLE topics DROP COLUMN IF EXISTS pinned;
ALTER TABLE topics DROP COLUMN IF EXISTS locked;

-- Remove trust_level from users
ALTER TABLE users DROP COLUMN IF EXISTS trust_level;
