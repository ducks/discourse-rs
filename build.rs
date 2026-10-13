//! Builds the markdown crate as WebAssembly for the composer's preview and
//! leaves it in OUT_DIR, where src/routes embeds it. A nested cargo with
//! its own target directory, so it never waits on this build's lock; it
//! reruns when the markdown crate or the data it embeds changes.
//!
//! Also lists the vendored highlight.js languages for src/highlight_js.rs.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    for path in [
        "crates/markdown",
        "vendor/discourse-emojis/dist",
        "vendor/discourse/DISCOURSE_REF",
        "Cargo.lock",
        "static/vendor/highlightjs/languages",
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
    highlight_js_languages(&out);
}

/// `HIGHLIGHT_JS_LANGUAGES`: each `<name>.min.js` in
/// static/vendor/highlightjs/languages as (name, contents), by name.
fn highlight_js_languages(out: &Path) {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
        .join("static/vendor/highlightjs/languages");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("the highlight.js languages are vendored")
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            name.strip_suffix(".min.js").map(str::to_string)
        })
        .collect();
    names.sort();
    let mut table = String::from("pub const HIGHLIGHT_JS_LANGUAGES: &[(&str, &str)] = &[\n");
    for name in names {
        let path = dir.join(format!("{name}.min.js"));
        table.push_str(&format!(
            "    ({name:?}, include_str!({:?})),\n",
            path.display().to_string()
        ));
    }
    table.push_str("];\n");
    std::fs::write(out.join("highlight_js_languages.rs"), table).expect("OUT_DIR is writable");
}
