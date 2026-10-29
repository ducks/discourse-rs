//! Builds the markdown crate as WebAssembly for the composer's preview and
//! leaves it in OUT_DIR, where src/routes embeds it. A nested cargo with
//! its own target directory, so it never waits on this build's lock; it
//! reruns when the markdown crate or the data it embeds changes.
//!
//! Also lists the vendored highlight.js languages for src/highlight_js.rs
//! and the moment locales for src/moment_locale.rs, the core and plugin
//! stylesheets theme SCSS compiles against for src/stylesheet, and the
//! discourse_math_bundle files for src/plugins/math.rs.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    for path in [
        "crates/markdown",
        "vendor/discourse-emojis/dist",
        "vendor/discourse/DISCOURSE_REF",
        "Cargo.lock",
        "static/vendor/highlightjs/languages",
        "static/vendor/moment/locale",
        "static/vendor/moment-timezone-names",
        "static/vendor/discourse-math-bundle",
        "vendor/discourse/app/assets/stylesheets",
        "vendor/discourse/config/plugin_color_definitions",
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
    moment_locales(&out);
    core_stylesheets(&out);
    math_bundle(&out);
}

/// `MATH_BUNDLE`: every file under static/vendor/discourse-math-bundle
/// (the discourse_math_bundle gem's vendor/assets, mathjax/ and katex/) as
/// (path under it, bytes), sorted by path for a binary search.
fn math_bundle(out: &Path) {
    let base = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
        .join("static/vendor/discourse-math-bundle");
    let mut files = Vec::new();
    let mut dirs = vec![base.clone()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).expect("the math bundle is vendored") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                let relative = path
                    .strip_prefix(&base)
                    .expect("under the bundle")
                    .to_string_lossy()
                    .into_owned();
                files.push((relative, path));
            }
        }
    }
    files.sort();
    let mut table = String::from("pub const MATH_BUNDLE: &[(&str, &[u8])] = &[\n");
    for (relative, path) in files {
        table.push_str(&format!(
            "    ({relative:?}, include_bytes!({:?})),\n",
            path.display().to_string()
        ));
    }
    table.push_str("];\n");
    std::fs::write(out.join("math_bundle.rs"), table).expect("OUT_DIR is writable");
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

/// `MOMENT_LOCALES` and `MOMENT_TIMEZONE_NAMES`: each `<name>.js` in
/// static/vendor/moment/locale and static/vendor/moment-timezone-names as
/// (name, contents), by name, for src/moment_locale.rs.
fn moment_locales(out: &Path) {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let mut table = String::new();
    for (constant, dir) in [
        ("MOMENT_LOCALES", "static/vendor/moment/locale"),
        (
            "MOMENT_TIMEZONE_NAMES",
            "static/vendor/moment-timezone-names",
        ),
    ] {
        let dir = root.join(dir);
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .expect("the moment locales are vendored")
            .filter_map(|entry| {
                let name = entry.ok()?.file_name().into_string().ok()?;
                name.strip_suffix(".js").map(str::to_string)
            })
            .collect();
        names.sort();
        table.push_str(&format!("pub const {constant}: &[(&str, &str)] = &[\n"));
        for name in names {
            let path = dir.join(format!("{name}.js"));
            table.push_str(&format!(
                "    ({name:?}, include_str!({:?})),\n",
                path.display().to_string()
            ));
        }
        table.push_str("];\n");
    }
    std::fs::write(out.join("moment_locales.rs"), table).expect("OUT_DIR is writable");
}

/// `CORE_STYLESHEETS`: every file of the vendored app/assets/stylesheets
/// (Stylesheet::Compiler's ASSET_ROOT) as (path under it, contents), by
/// path; `PLUGIN_COLOR_DEFINITIONS`: the bundled plugins' color definitions
/// as (plugin, contents), in the order config/plugin_color_definitions
/// lists them.
fn core_stylesheets(out: &Path) {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
        .join("vendor/discourse");
    let base = root.join("app/assets/stylesheets");
    let mut files = Vec::new();
    let mut dirs = vec![base.clone()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).expect("the core stylesheets are vendored") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                let relative = path
                    .strip_prefix(&base)
                    .expect("under the stylesheets")
                    .to_string_lossy()
                    .into_owned();
                files.push((relative, path));
            }
        }
    }
    files.sort();
    let mut table = String::from("pub const CORE_STYLESHEETS: &[(&str, &str)] = &[\n");
    for (relative, path) in files {
        table.push_str(&format!(
            "    ({relative:?}, include_str!({:?})),\n",
            path.display().to_string()
        ));
    }
    table.push_str("];\n");
    table.push_str("pub const PLUGIN_COLOR_DEFINITIONS: &[(&str, &str)] = &[\n");
    let list = std::fs::read_to_string(root.join("config/plugin_color_definitions"))
        .expect("the plugin color definitions are listed");
    for line in list.lines().filter(|l| !l.trim().is_empty()) {
        let (plugin, file) = line.split_once(' ').expect("`<plugin> <path>`");
        table.push_str(&format!(
            "    ({plugin:?}, include_str!({:?})),\n",
            root.join(file).display().to_string()
        ));
    }
    table.push_str("];\n");
    std::fs::write(out.join("core_stylesheets.rs"), table).expect("OUT_DIR is writable");
}
