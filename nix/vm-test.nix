# The NixOS module end to end: a VM with a local PostgreSQL, the schema
# and seeds loaded by discourse-rs-db-init, the server with its job
# worker, Caddy in front. Run: nix build .#checks.x86_64-linux.vm -L
{ pkgs, module }:

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

      # A test secret; a real host keeps this file out of the store.
      environment.etc."discourse-rs/env" = {
        mode = "0600";
        text = ''
          DISCOURSE_SECRET_KEY_BASE=${builtins.hashString "sha512" "discourse-rs vm test"}
        '';
      };

      environment.systemPackages = [
        pkgs.curl
        pkgs.jq
      ];
    };

  testScript = ''
    import json
    import shlex

    base = "http://localhost:8080"
    jar = "-c /tmp/jar -b /tmp/jar"
    script = "-H 'X-Requested-With: XMLHttpRequest' -H 'Accept: application/json'"

    def status(path):
        return machine.succeed(f"curl -s -o /dev/null -w '%{{http_code}}' '{base}{path}'").strip()

    def expect_200(path):
        code = status(path)
        assert code == "200", f"{path}: {code}"

    def sql(query):
        return machine.succeed(
            f"sudo -u postgres psql -X -At -d discourse -c {shlex.quote(query)}"
        ).strip()

    machine.wait_for_unit("discourse-rs-db-init.service")
    machine.wait_for_unit("discourse-rs.service")
    machine.wait_for_open_port(8080)

    with subtest("schema and seeds loaded, owned by the service role"):
        assert int(sql("SELECT count(*) FROM schema_migrations")) > 0
        assert sql("SELECT tableowner FROM pg_tables WHERE tablename = 'posts'") == "discourse-rs"
        assert sql("SELECT username FROM users WHERE id = 1") == "admin"

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

    with subtest("log in as the seed's admin"):
        csrf = json.loads(machine.succeed(f"curl -sf {jar} {script} {base}/session/csrf.json"))["csrf"]
        auth = f"{jar} {script} -H 'X-CSRF-Token: {csrf}'"
        reply = json.loads(machine.succeed(
            f"curl -sf {auth} --data-urlencode login=admin --data-urlencode password=password {base}/session"
        ))
        assert "error" not in reply, reply
        current = json.loads(machine.succeed(f"curl -sf {jar} {script} {base}/session/current.json"))
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
        machine.succeed("systemctl restart discourse-rs-db-init.service")
        machine.succeed("journalctl -u discourse-rs-db-init.service -b | grep -c 'loading the vendored schema' | grep -qx 1")
  '';
}
