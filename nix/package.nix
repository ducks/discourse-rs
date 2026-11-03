# The discourse-rs binary, built as CI builds it: the toolchain from
# rust-toolchain.toml (with its wasm32 target, for build.rs's nested cargo
# that compiles the composer preview), the crates Cargo.lock pins.
{
  lib,
  callPackage,
  makeWrapper,
  rust-bin,
  makeRustPlatform,
  symlinkJoin,
}:

let
  toolchain = rust-bin.fromRustupToolchainFile ../rust-toolchain.toml;
  rustPlatform = makeRustPlatform {
    cargo = toolchain;
    rustc = toolchain;
  };
  manifest = lib.importTOML ../Cargo.toml;
  fs = lib.fileset;
  frontend = callPackage ./frontend.nix { };
  binary = rustPlatform.buildRustPackage {
    pname = "discourse-rs";
    inherit (manifest.package) version;

    # What the build reads, so editing docs or parity goldens doesn't
    # rebuild it. tests/ is there for Cargo.toml's [[test]] target; seed/ for
    # the clean install `discourse-rs setup` embeds.
    src = fs.toSource {
      root = ../.;
      fileset = fs.unions [
        ../Cargo.toml
        ../Cargo.lock
        ../build.rs
        ../rust-toolchain.toml
        ../src
        ../crates
        ../vendor
        # Generated language, locale and math bundles still embed vendor
        # files. The runtime JS/CSS package must not change this derivation.
        (fs.difference ../static (
          fs.unions [
            ../static/js
            ../static/css
            ../static/site.css
            (fs.maybeMissing ../static/manifest.json)
          ]
        ))
        ../templates
        ../tests
        ../seed
      ];
    };

    cargoLock.lockFile = ../Cargo.lock;

    # The release workflow's binary; src/bin/parity.rs is a dev tool.
    cargoBuildFlags = [
      "--bin"
      "discourse-rs"
    ];

    # cargo-auditable would wrap rustc for build.rs's nested wasm cargo too,
    # through the environment it inherits: not what CI builds.
    auditable = false;

    # The tests need Postgres: the VM test and CI run them.
    doCheck = false;

    # The vendored schema, for restoring a backup by hand (the binary embeds
    # it, and `discourse-rs setup` loads it with the clean install's seed).
    postInstall = ''
      install -Dm644 vendor/discourse/db/structure.sql \
        $out/share/discourse-rs/db/structure.sql
      install -Dm644 vendor/discourse/DISCOURSE_REF \
        $out/share/discourse-rs/DISCOURSE_REF
    '';

    meta = {
      description = "A Rust port of Discourse";
      license = lib.licenses.gpl3Plus;
      mainProgram = "discourse-rs";
      platforms = lib.platforms.linux;
    };
  };
in
symlinkJoin {
  name = "discourse-rs-${manifest.package.version}";
  inherit (manifest.package) version;
  pname = "discourse-rs";
  paths = [ binary ];
  nativeBuildInputs = [ makeWrapper ];
  postBuild = ''
    wrapProgram $out/bin/discourse-rs \
      --set-default DISCOURSE_FRONTEND_DIR ${frontend}
  '';
  passthru = {
    unwrapped = binary;
    inherit frontend;
  };
  inherit (binary) meta;
}
