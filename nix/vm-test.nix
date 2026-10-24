# The NixOS module end to end: a VM with a local PostgreSQL, the schema
# and seeds loaded and secured by discourse-rs-db-init, the server with
# its job worker, Caddy in front, then the team-only options switched on.
# Run: nix build .#checks.x86_64-linux.vm -L
{ pkgs, module }:

let
  adminPassword = "vm-test admin password";
in
pkgs.testers.runNixOSTest {
  name = "discourse-rs";

  nodes.machine =
    { pkgs, ... }:
    {
      imports = [ module ];

      virtualisation.memorySize = 2048;
      virtualisation.diskSize = 4096;

      services.discourse-rs = {
        enable = true;
        # Caddy's internal CA serves localhost, so the proxy is tested
        # without ACME.
        hostname = "localhost";
        forceHttps = false;
        environmentFile = "/etc/discourse-rs/env";
        caddy.enable = true;
      };

      specialisation.team-only.configuration.services.discourse-rs = {
        loginRequired = true;
        inviteOnly = true;
      };

      # Test secrets; a real host keeps this file out of the store.
      environment.etc."discourse-rs/env" = {
        mode = "0600";
        text = ''
          DISCOURSE_SECRET_KEY_BASE=${builtins.hashString "sha512" "discourse-rs vm test"}
          INITIAL_ADMIN_PASSWORD=${adminPassword}
          INITIAL_ADMIN_EMAIL=Admin@Forum.Test
        '';
      };

      environment.systemPackages = [
        pkgs.curl
        pkgs.jq
      ];
    };

  testScript =
    { nodes, ... }:
    ''
      import json
      import shlex

      base = "http://localhost:8080"
      script = "-H 'X-Requested-With: XMLHttpRequest' -H 'Accept: application/json'"
      fresh_site = "${nodes.machine.services.discourse-rs.package}/bin/discourse-rs-fresh-site"

      def status(path):
          return machine.succeed(f"curl -s -o /dev/null -w '%{{http_code}}' '{base}{path}'").strip()

      def expect_200(path):
          code = status(path)
          assert code == "200", f"{path}: {code}"

      def sql(query):
          return machine.succeed(
              f"sudo -u postgres psql -X -At -d discourse -c {shlex.quote(query)}"
          ).strip()

      def login(username, password, jar):
          """POST /session as the login form's script does; the reply."""
          cookies = f"-c {jar} -b {jar}"
          csrf = json.loads(machine.succeed(f"curl -sf {cookies} {script} {base}/session/csrf.json"))["csrf"]
          auth = f"{cookies} {script} -H 'X-CSRF-Token: {csrf}'"
          reply = machine.succeed(
              f"curl -s {auth} --data-urlencode login={shlex.quote(username)} "
              f"--data-urlencode password={shlex.quote(password)} {base}/session"
          )
          return auth, json.loads(reply)

      machine.wait_for_unit("discourse-rs-db-init.service")
      machine.wait_for_unit("discourse-rs.service")
      machine.wait_for_open_port(8080)

      with subtest("schema and seeds loaded, owned by the service role"):
          assert int(sql("SELECT count(*) FROM schema_migrations")) > 0
          assert sql("SELECT tableowner FROM pg_tables WHERE tablename = 'posts'") == "discourse-rs"
          assert sql("SELECT username FROM users WHERE id = 1") == "admin"

      with subtest("the seed's secrets are replaced"):
          seed = {
              "vapid_public_key": "BAGOjc4U2dZziyL8MGXELb1RmtazJPY65gEjFCMbQOBfjv4kOA-PsxrJFi-BBJAAF5aan-Z_OWuT4wchuDZLnTQ=",
              "vapid_private_key": "Pq2dkgm-AKMs_9pGwvBu4VeVd33iCtV0nRjkMLa-2NU=",
              "push_api_secret_key": "96b10b7ce1e537c0a1213353b2a30558",
          }
          for name, value in seed.items():
              now = sql(f"SELECT value FROM site_settings WHERE name = '{name}'")
              assert now and now != value, f"{name}: {now}"
          assert sql("SELECT value FROM site_settings WHERE name = 'vapid_base_url'") == "http://localhost"
          assert sql("SELECT count(*) FROM unsubscribe_keys") == "0"
          assert sql("SELECT email FROM user_emails WHERE user_id = 1 AND \"primary\"") == "admin@forum.test"
          assert sql("SELECT count(*) FROM user_passwords") == "1"

      with subtest("a fresh load needs the admin password"):
          machine.fail(f"env -u INITIAL_ADMIN_PASSWORD {fresh_site} http://localhost")
          machine.fail(f"INITIAL_ADMIN_PASSWORD=password {fresh_site} http://localhost")

      with subtest("the env file's secret is read"):
          machine.fail("journalctl -u discourse-rs.service | grep 'DISCOURSE_SECRET_KEY_BASE not set'")

      with subtest("pages"):
          expect_200("/latest")
          expect_200("/t/parity-fixture-replies-and-posters/35")

      with subtest("assets from discourse-assets"):
          expect_200("/images/discourse-logo-sketch.png")
          expect_200("/images/emoji/twitter/smile.png?v=15")
          expect_200("/fonts/Roboto-Regular.woff2?v=0.0.19")

      with subtest("Caddy proxies to the server"):
          machine.wait_for_unit("caddy.service")
          machine.wait_for_open_port(443)
          machine.wait_until_succeeds(
              "curl -sfk -o /dev/null https://localhost/latest", timeout=60
          )

      with subtest("the seed's credentials no longer log in"):
          _, reply = login("admin", "password", "/tmp/jar-admin-seed")
          assert "error" in reply, reply
          for user in ["user0", "user1", "user2", "user3"]:
              _, reply = login(user, "password", f"/tmp/jar-{user}")
              assert "error" in reply, reply
          # Deactivated and suspended; the purge of unactivated users may
          # already have deleted the ones without posts.
          assert sql(
              "SELECT count(*) FROM users WHERE username IN ('user0', 'user1', 'user2', 'user3') "
              "AND (active OR suspended_till IS NULL OR suspended_till <= now())"
          ) == "0", sql("SELECT username, active, suspended_till FROM users WHERE username LIKE 'user%'")

      with subtest("the admin logs in with the env file's password"):
          auth, reply = login("admin", "${adminPassword}", "/tmp/jar")
          assert "error" not in reply, reply
          current = json.loads(machine.succeed(f"curl -sf -c /tmp/jar -b /tmp/jar {script} {base}/session/current.json"))
          assert current["current_user"]["username"] == "admin", current

      with subtest("a reply is processed by the job worker"):
          # The first emoji outside a quote earns the First Emoji badge
          # (41), which only the process_post job grants.
          assert sql("SELECT count(*) FROM user_badges WHERE user_id = 1 AND badge_id = 41") == "0"
          post = json.loads(machine.succeed(
              f"curl -sf {auth} --data-urlencode topic_id=35 "
              "--data-urlencode 'raw=A reply from the NixOS VM test :smile:' "
              f"{base}/posts.json"
          ))
          post_id = post.get("post", post)["id"]
          machine.wait_until_succeeds(
              "test \"$(sudo -u postgres psql -X -At -d discourse -c "
              "'SELECT count(*) FROM user_badges WHERE user_id = 1 AND badge_id = 41')\" = 1",
              timeout=120,
          )
          assert sql(
              f"SELECT count(*) FROM discourse_rs.jobs WHERE name = 'process_post' AND args->>'post_id' = '{post_id}'"
          ) == "0", sql("SELECT name, last_error FROM discourse_rs.jobs WHERE last_error IS NOT NULL")

      with subtest("the units verify"):
          machine.succeed("systemd-analyze verify /etc/systemd/system/discourse-rs.service")
          machine.succeed("systemd-analyze verify /etc/systemd/system/discourse-rs-db-init.service")

      with subtest("db-init is idempotent"):
          vapid = sql("SELECT value FROM site_settings WHERE name = 'vapid_public_key'")
          machine.succeed("systemctl restart discourse-rs-db-init.service")
          machine.succeed("journalctl -u discourse-rs-db-init.service -b | grep -c 'loading the vendored schema' | grep -qx 1")
          assert sql("SELECT value FROM site_settings WHERE name = 'vapid_public_key'") == vapid

      with subtest("team-only: loginRequired sends the anonymous to the login"):
          machine.succeed("/run/booted-system/specialisation/team-only/bin/switch-to-configuration test")
          machine.wait_for_unit("discourse-rs.service")
          machine.wait_for_open_port(8080)
          env = machine.succeed("systemctl show -p Environment discourse-rs.service")
          assert "DISCOURSE_LOGIN_REQUIRED=true" in env and "DISCOURSE_INVITE_ONLY=true" in env, env
          answer = machine.succeed(
              f"curl -s -o /dev/null -w '%{{http_code}} %{{redirect_url}}' {base}/latest"
          ).strip()
          assert answer == "302 http://localhost:8080/login", answer
    '';
}
