//! Builds the markdown crate as WebAssembly for the composer's preview and
//! leaves it in OUT_DIR, where src/routes embeds it. A nested cargo with
//! its own target directory, so it never waits on this build's lock; it
//! reruns when the markdown crate or the data it embeds changes.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    for path in [
        "crates/markdown",
        "vendor/discourse-emojis/dist",
        "vendor/discourse/DISCOURSE_REF",
        "Cargo.lock",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let target_dir = out.join("wasm-target");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .args([
            "build",
            "--package",
            "discourse-markdown",
            "--target",
            "wasm32-unknown-unknown",
            "--release",
            "--target-dir",
        ])
        .arg(&target_dir)
        // Small over fast: it is downloaded.
        .env("CARGO_PROFILE_RELEASE_OPT_LEVEL", "s")
        .env("CARGO_PROFILE_RELEASE_LTO", "true")
        .env("CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "1")
        .env("CARGO_PROFILE_RELEASE_STRIP", "true")
        // The host build's flags are not the wasm build's.
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .status()
        .expect("cargo runs");
    assert!(
        status.success(),
        "building the markdown crate as wasm failed"
    );
    let wasm = target_dir.join("wasm32-unknown-unknown/release/discourse_markdown.wasm");
    let bytes = std::fs::read(&wasm).expect("the wasm is built");
    std::fs::write(out.join("discourse_markdown.wasm"), &bytes).expect("OUT_DIR is writable");
    // Its version, for the url the browser caches it under.
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    println!(
        "cargo:rustc-env=MARKDOWN_WASM_VERSION={:016x}",
        hasher.finish()
    );
}
