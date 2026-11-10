-- What the browser specs share, applied by scripts/browser-test to its
-- throwaway copy of the test database before the server boots. Each
-- setting is one an admin turns on for the demo; a spec that needs a row
-- of its own makes it through the app (or helpers.js sql()) instead.
INSERT INTO site_settings (name, data_type, value, created_at, updated_at)
VALUES
  -- The interface color selector in the sidebar footer (dark mode).
  ('interface_color_selector', 7, 'sidebar_footer', now(), now()),
  -- The post menu's plugin buttons: Solved on every topic, Assign.
  ('allow_solved_on_all_topics', 5, 't', now(), now()),
  ('assign_enabled', 5, 't', now(), now()),
  -- Chat threads: the drawer's threads route and the footer's item
  -- (223c), the thread specs' threaded channels (160a); a hidden
  -- setting the admin can't save.
  ('chat_threads_enabled', 5, 't', now(), now()),
  -- The composer actions menu's whisper toggle (staff whisper) and
  -- discourse-post-voting's toggle.
  ('whispers_allowed_groups', 20, '3', now(), now()),
  ('post_voting_enabled', 5, 't', now(), now()),
  -- User status: the user menu's item, the modal, the emoji by names
  -- (user-status.spec.js sets statuses only on members of its own).
  ('enable_user_status', 5, 't', now(), now()),
  -- No education message as a member's first posts are composed
  -- (spec/rails_helper.rb's SiteSetting.educate_until_posts = 0): it
  -- would sit over the composer the specs type in. The specs of
  -- composer-messages.spec.js answer /composer_messages themselves.
  ('educate_until_posts', 3, '0', now(), now()),
  -- Similar topics are looked for on a site of any size (the default
  -- waits for 50 topics): composer-messages.spec.js's popup.
  ('minimum_topics_similar', 3, '0', now(), now())
ON CONFLICT (name) DO UPDATE SET value = EXCLUDED.value;

-- The seed has everyone on the rich editor (Discourse's
-- default_composition_mode), and the composer opens in the stored mode;
-- the suite's members write in the markdown editor (composition_mode 0),
-- with its preview, as the specs type into and read the composer's
-- textarea. composer.spec.js gives its own members the mode each case
-- needs, the rich editor's included.
UPDATE user_options SET composition_mode = 0;
