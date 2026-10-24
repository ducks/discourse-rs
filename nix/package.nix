# The discourse-rs binary, built as CI builds it: the toolchain from
# rust-toolchain.toml (with its wasm32 target, for build.rs's nested cargo
# that compiles the composer preview), the crates Cargo.lock pins.
{
  lib,
  rust-bin,
  makeRustPlatform,
}:

let
  toolchain = rust-bin.fromRustupToolchainFile ../rust-toolchain.toml;
  rustPlatform = makeRustPlatform {
    cargo = toolchain;
    rustc = toolchain;
  };
  manifest = lib.importTOML ../Cargo.toml;
  fs = lib.fileset;
in
rustPlatform.buildRustPackage {
  pname = "discourse-rs";
  inherit (manifest.package) version;

  # What the build reads, so editing docs or parity goldens doesn't
  # rebuild it. tests/ is there for Cargo.toml's [[test]] target; seed/ is
  # installed for a fresh site's database.
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
      ../static
      ../templates
      ../tests
      ../seed
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  # The release workflow's binary, and the tool the NixOS module secures
  # a fresh site's seed with; src/bin/parity.rs is a dev tool.
  cargoBuildFlags = [
    "--bin"
    "discourse-rs"
    "--bin"
    "discourse-rs-fresh-site"
  ];

  # cargo-auditable would wrap rustc for build.rs's nested wasm cargo too,
  # through the environment it inherits: not what CI builds.
  auditable = false;

  # The tests need Postgres: the VM test and CI run them.
  doCheck = false;

  # The schema and seeds scripts/db-load loads, for a fresh site's
  # database (the NixOS module's discourse-rs-db-init).
  postInstall = ''
    install -Dm644 vendor/discourse/db/structure.sql \
      $out/share/discourse-rs/db/structure.sql
    install -Dm644 -t $out/share/discourse-rs/seed seed/*.sql
    install -Dm644 vendor/discourse/DISCOURSE_REF \
      $out/share/discourse-rs/DISCOURSE_REF
  '';

  meta = {
    description = "A Rust port of Discourse";
    license = lib.licenses.gpl3Plus;
    mainProgram = "discourse-rs";
    platforms = lib.platforms.linux;
  };
}
