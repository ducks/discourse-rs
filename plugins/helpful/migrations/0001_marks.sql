CREATE TABLE marks (
  post_id integer NOT NULL,
  user_id integer NOT NULL,
  created_at timestamp NOT NULL DEFAULT now(),
  PRIMARY KEY (post_id, user_id)
);
