CREATE TABLE echoes (
  id serial PRIMARY KEY,
  body text NOT NULL,
  user_id integer,
  created_at timestamp NOT NULL DEFAULT now()
);
CREATE INDEX echoes_user ON echoes (user_id);
