-- Backie jobs table
-- This is the schema required by backie for PostgreSQL job queue
CREATE TABLE backie_tasks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    task_name TEXT NOT NULL,
    task_hash TEXT NOT NULL UNIQUE,
    payload JSONB NOT NULL,
    timeout_msecs BIGINT NOT NULL,
    max_retries INTEGER NOT NULL DEFAULT 0,
    retries INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    scheduled_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    running_at TIMESTAMP WITH TIME ZONE,
    done_at TIMESTAMP WITH TIME ZONE,
    error TEXT
);

CREATE INDEX backie_tasks_scheduled_at_idx ON backie_tasks (scheduled_at);
CREATE INDEX backie_tasks_task_name_idx ON backie_tasks (task_name);
CREATE INDEX backie_tasks_done_at_idx ON backie_tasks (done_at);
