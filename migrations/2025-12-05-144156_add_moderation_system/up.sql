-- Add trust_level to users (0 = new user, 1 = basic, 2 = member, 3 = regular, 4 = leader/mod)
ALTER TABLE users ADD COLUMN IF NOT EXISTS trust_level INTEGER NOT NULL DEFAULT 0;

-- Add moderation fields to topics
ALTER TABLE topics ADD COLUMN IF NOT EXISTS locked BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE topics ADD COLUMN IF NOT EXISTS pinned BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE topics ADD COLUMN IF NOT EXISTS closed BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE topics ADD COLUMN IF NOT EXISTS pinned_at TIMESTAMP WITH TIME ZONE;
ALTER TABLE topics ADD COLUMN IF NOT EXISTS closed_at TIMESTAMP WITH TIME ZONE;

-- Add moderation fields to posts
ALTER TABLE posts ADD COLUMN IF NOT EXISTS hidden BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE posts ADD COLUMN IF NOT EXISTS hidden_at TIMESTAMP WITH TIME ZONE;
ALTER TABLE posts ADD COLUMN IF NOT EXISTS hidden_by_user_id INTEGER REFERENCES users(id);
ALTER TABLE posts ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMP WITH TIME ZONE;
ALTER TABLE posts ADD COLUMN IF NOT EXISTS deleted_by_user_id INTEGER REFERENCES users(id);

-- Create user_suspensions table
CREATE TABLE user_suspensions (
    id BIGSERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id),
    suspended_by_user_id INTEGER NOT NULL REFERENCES users(id),
    reason TEXT NOT NULL,
    suspended_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    suspended_until TIMESTAMP WITH TIME ZONE NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_user_suspensions_user_id ON user_suspensions(user_id);
CREATE INDEX idx_user_suspensions_suspended_until ON user_suspensions(suspended_until);

-- Create moderation_actions table for audit log
CREATE TABLE moderation_actions (
    id BIGSERIAL PRIMARY KEY,
    action_type VARCHAR(50) NOT NULL,
    moderator_id INTEGER NOT NULL REFERENCES users(id),
    target_user_id INTEGER REFERENCES users(id),
    target_topic_id INTEGER REFERENCES topics(id),
    target_post_id INTEGER REFERENCES posts(id),
    details JSONB,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_moderation_actions_moderator_id ON moderation_actions(moderator_id);
CREATE INDEX idx_moderation_actions_target_user_id ON moderation_actions(target_user_id);
CREATE INDEX idx_moderation_actions_target_topic_id ON moderation_actions(target_topic_id);
CREATE INDEX idx_moderation_actions_target_post_id ON moderation_actions(target_post_id);
CREATE INDEX idx_moderation_actions_created_at ON moderation_actions(created_at);
