//! Stylesheet::Compiler and Stylesheet::Importer: SCSS compiled with grass
//! (crates/grass_compiler, patched to write what dart-sass 1.104 writes)
//! against Discourse's core stylesheets, the way Rails compiles a theme's
//! fields and the color definitions. What the compile prints goes through
//! [`super::postcss`] as AssetProcessor#post_css does.

use std::io;
use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/core_stylesheets.rs"));

/// Stylesheet::Compiler::ASSET_ROOT, where the core stylesheets are found.
const ASSET_ROOT: &str = "/discourse/app/assets/stylesheets";
/// A theme's export directory (Theme#with_scss_load_paths).
const THEME_ROOT: &str = "/theme";

/// A theme's SCSS as ThemeStore::ZipExporter exports it for a compile:
/// paths under the export directory (`stylesheets/<name>.scss`,
/// `_entry_loadpath/theme-entrypoint/common.scss` ...) with their text.
#[derive(Debug, Clone, Default)]
pub struct ThemeFiles {
    pub files: Vec<(String, String)>,
}

impl ThemeFiles {
    /// `with_scss_load_paths`: each field at its export path, the entry
    /// points (`common/common.scss` and the like) moved under
    /// `_entry_loadpath/theme-entrypoint/`.
    pub fn from_exported(exported: impl IntoIterator<Item = (String, String)>) -> ThemeFiles {
        const ENTRY_POINTS: [(&str, &str); 5] = [
            ("common/common.scss", "common.scss"),
            ("common/embedded.scss", "embedded.scss"),
            ("common/color_definitions.scss", "color_definitions.scss"),
            ("desktop/desktop.scss", "desktop.scss"),
            ("mobile/mobile.scss", "mobile.scss"),
        ];
        let files = exported
            .into_iter()
            .map(
                |(path, text)| match ENTRY_POINTS.iter().find(|(source, _)| *source == path) {
                    Some((_, entry)) => (format!("_entry_loadpath/theme-entrypoint/{entry}"), text),
                    None => (path, text),
                },
            )
            .collect();
        ThemeFiles { files }
    }
}

/// The files a compile can read: the core stylesheets and a theme's.
#[derive(Debug)]
struct Files<'a> {
    theme: Option<&'a ThemeFiles>,
}

impl Files<'_> {
    fn find(&self, path: &Path) -> Option<&str> {
        let path = path.to_str()?;
        if let Some(relative) = path
            .strip_prefix(ASSET_ROOT)
            .and_then(|p| p.strip_prefix('/'))
        {
            return CORE_STYLESHEETS
                .binary_search_by(|(name, _)| (*name).cmp(relative))
                .ok()
                .map(|i| CORE_STYLESHEETS[i].1);
        }
        let relative = path
            .strip_prefix(THEME_ROOT)
            .and_then(|p| p.strip_prefix('/'))?;
        self.theme?
            .files
            .iter()
            .find(|(name, _)| name == relative)
            .map(|(_, text)| text.as_str())
    }

    fn is_dir(&self, path: &Path) -> bool {
        let Some(path) = path.to_str() else {
            return false;
        };
        let prefix = format!("{}/", path.trim_end_matches('/'));
        if let Some(relative) = prefix.strip_prefix(&format!("{ASSET_ROOT}/")) {
            return CORE_STYLESHEETS
                .iter()
                .any(|(name, _)| name.starts_with(relative));
        }
        match prefix.strip_prefix(&format!("{THEME_ROOT}/")) {
            Some(relative) => self
                .theme
                .is_some_and(|t| t.files.iter().any(|(name, _)| name.starts_with(relative))),
            None => false,
        }
    }
}

impl grass::Fs for Files<'_> {
    fn is_dir(&self, path: &Path) -> bool {
        Files::is_dir(self, path)
    }

    fn is_file(&self, path: &Path) -> bool {
        self.find(path).is_some()
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.find(path)
            .map(|text| text.as_bytes().to_vec())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such stylesheet"))
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(path.to_path_buf())
    }
}

/// `Stylesheet::Compiler.compile`'s Sass part: the source compressed, the
/// core stylesheets first among the load paths, then the theme's. The
/// error is dart-sass's message for the source (SassC::SyntaxError).
pub fn compile(source: &str, theme: Option<&ThemeFiles>) -> Result<String, String> {
    let files = Files { theme };
    let mut options = grass::Options::default()
        .style(grass::OutputStyle::Compressed)
        .quiet(true)
        .fs(&files)
        .load_path(ASSET_ROOT);
    if theme.is_some() {
        options = options
            .load_path(format!("{THEME_ROOT}/_entry_loadpath"))
            .load_path(format!("{THEME_ROOT}/stylesheets"));
    }
    grass::from_string(source.to_string(), &options).map_err(|e| match e.kind() {
        // SassC::SyntaxError#message: the message alone.
        grass::ErrorKind::ParseError { message, .. } => {
            message.split('\n').next().unwrap_or_default().to_string()
        }
        other => format!("{other:?}"),
    })
}

/// `Importer#color_variables`: each color of the scheme as a default.
pub fn color_variables(colors: &[(String, String)]) -> String {
    colors
        .iter()
        .map(|(name, hex)| format!("${name}: #{hex} !default; "))
        .collect()
}

/// `Importer#prepended_scss`: the scheme's colors, the public image path,
/// Discourse's variables and mixins.
pub fn prepended_scss(colors: &[(String, String)], public_image_path: &str) -> String {
    format!(
        "{} $public_image_path: \"{public_image_path}\";  @import \"common/foundation/variables\"; @import \"common/foundation/mixins\"; ",
        color_variables(colors)
    )
}

/// `Importer#import_color_definitions`'s plugin part: each bundled
/// plugin's color definitions, under a comment naming it.
pub fn plugin_color_definitions() -> String {
    PLUGIN_COLOR_DEFINITIONS
        .iter()
        .map(|(name, text)| format!("\n\n// Color definitions from {name}\n\n{text}\n\n"))
        .collect()
}

/// `Theme#to_scss_variable`: the value as a Sass string
/// (`SassC::Script::Value::String.quote(value, sass: true)`), unquoted.
pub fn to_scss_variable(name: &str, value: &str) -> String {
    format!("${name}: unquote({});", quote(value))
}

/// `SassC::Script::Value::String.quote(contents, sass: true)`
fn quote(contents: &str) -> String {
    let needs_quoting = contents.contains(['\n', '\\', '"', '\'']) || contents.contains("#{");
    if !needs_quoting {
        return format!("\"{contents}\"");
    }
    let quote = if contents.contains('"') && !contents.contains('\'') {
        '\''
    } else {
        '"'
    };
    let mut out = contents.replace('\\', "\\\\").replace("#{", "\\#{");
    out = if quote == '"' {
        out.replace('"', "\\\"")
    } else {
        out.replace('\'', "\\'")
    };
    // A newline before a hex digit or whitespace gets a space after its
    // escape, so the two don't read as one escape.
    let mut escaped = String::with_capacity(out.len());
    let mut chars = out.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\n' {
            escaped.push_str("\\a");
            if chars
                .peek()
                .is_some_and(|n| n.is_ascii_hexdigit() || n.is_whitespace())
            {
                escaped.push(' ');
            }
        } else {
            escaped.push(c);
        }
    }
    format!("{quote}{escaped}{quote}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_as_sassc_does() {
        assert_eq!(quote("plain"), "\"plain\"");
        assert_eq!(quote("it's"), "\"it's\"");
        assert_eq!(quote("say \"hi\""), "'say \"hi\"'");
        assert_eq!(quote("both ' and \""), "\"both ' and \\\"\"");
        assert_eq!(quote("a\\b"), "\"a\\\\b\"");
        assert_eq!(quote("#{x}"), "\"\\#{x}\"");
        assert_eq!(quote("a\nb"), "\"a\\a b\"");
        assert_eq!(quote("a\nz"), "\"a\\az\"");
        assert_eq!(quote("a\n"), "\"a\\a\"");
    }

    #[test]
    fn core_stylesheets_are_found() {
        let colors: Vec<(String, String)> = [
            ("primary", "222"),
            ("secondary", "fff"),
            ("tertiary", "08c"),
            ("quaternary", "e45735"),
            ("header_background", "fff"),
            ("header_primary", "333"),
            ("highlight", "ffff4d"),
            ("selected", "d1f0ff"),
            ("hover", "f2f2f2"),
            ("danger", "c80001"),
            ("success", "090"),
            ("love", "fa6c8d"),
        ]
        .iter()
        .map(|(n, h)| (n.to_string(), h.to_string()))
        .collect();
        let source = format!(
            "{}a{{color:$primary;b:$primary-low}}",
            prepended_scss(&colors, "http://localhost/images")
        );
        let css = compile(&source, None).unwrap();
        assert!(css.starts_with("a{color:#222;b:"), "{css}");
    }

    #[test]
    fn theme_files_load_after_the_core() {
        let theme = ThemeFiles::from_exported([
            (
                "common/common.scss".to_string(),
                "@import \"buttons\"; b{x:1}".to_string(),
            ),
            ("stylesheets/buttons.scss".to_string(), "a{y:2}".to_string()),
        ]);
        let css = compile("@import \"theme-entrypoint/common\";", Some(&theme)).unwrap();
        assert_eq!(css, "a{y:2}b{x:1}");
    }
}

/// crates/grass_compiler/PATCHES.md: each patch against what dart-sass
/// 1.104.0 (the reference's sass-embedded) prints for the same input,
/// compressed unless noted.
#[cfg(test)]
mod grass_patches {
    fn compressed(source: &str) -> String {
        let options = grass::Options::default()
            .style(grass::OutputStyle::Compressed)
            .quiet(true);
        grass::from_string(source.to_string(), &options).unwrap()
    }

    fn expanded(source: &str) -> String {
        let options = grass::Options::default().quiet(true);
        grass::from_string(source.to_string(), &options).unwrap()
    }

    /// A directory of files for the import tests.
    fn files(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("grass-{name}-{}", std::process::id()));
        for (path, text) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    #[test]
    fn import_only_files_are_for_import_alone() {
        let dir = files(
            "import-only",
            &[
                ("_lib.scss", "a{x:plain}"),
                ("_lib.import.scss", "a{x:import-only}"),
                ("lp/_other.scss", "b{y:from-load-path}"),
            ],
        );
        let compile = |source: &str| {
            let options = grass::Options::default()
                .style(grass::OutputStyle::Compressed)
                .quiet(true)
                .load_path(&dir)
                .load_path(dir.join("lp"));
            grass::from_string(source.to_string(), &options).unwrap()
        };
        assert_eq!(compile("@import \"lib\";"), "a{x:import-only}");
        assert_eq!(compile("@use \"lib\";"), "a{x:plain}");
        // An explicit extension is looked up in the load paths too.
        assert_eq!(compile("@import \"other.scss\";"), "b{y:from-load-path}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_declaration_after_a_nested_rule_keeps_its_place() {
        assert_eq!(compressed(".a{b:1;.c{d:2}e:3}"), ".a{b:1}.a .c{d:2}.a{e:3}");
    }

    #[test]
    fn empty_custom_properties_are_written() {
        assert_eq!(compressed("a{--x:;--y: ;}"), "a{--x:;--y: }");
    }

    #[test]
    fn a_backtracked_selector_leaves_division_alone() {
        assert_eq!(
            compressed(".p{a:not(.q){x:y}z:1/2}"),
            ".p a:not(.q){x:y}.p{z:1/2}"
        );
    }

    #[test]
    fn interpolation_writes_values_expanded() {
        assert_eq!(
            compressed("$l: 1, 2;a{--x: #{$l};y: #{mix(red, blue)};z: #{0.5}}"),
            "a{--x: 1, 2;y:rgb(50%, 0%, 50%);z:0.5}"
        );
    }

    #[test]
    fn compressed_media_queries_and_supports_conditions() {
        assert_eq!(
            compressed(
                "@media (min-width: 1px) and (max-width: 2px){a{b:c}}@media screen and (min-width: 1px){a{b:c}}@media (min-width: 1px), print{a{b:c}}"
            ),
            "@media(min-width: 1px)and (max-width: 2px){a{b:c}}@media screen and (min-width: 1px){a{b:c}}@media(min-width: 1px),print{a{b:c}}"
        );
        assert_eq!(
            compressed("@supports (display: grid){a{b:c}}@supports not (display: grid){a{b:c}}"),
            "@supports(display: grid){a{b:c}}@supports not (display: grid){a{b:c}}"
        );
    }

    #[test]
    fn numbers_are_written_as_dart_sass_writes_them() {
        let source =
            "a{b:-0.5;c:0.5;d:1.123456789012;e:-0.00000000001;f:100.000000000049;g:0.1+0.2}";
        assert_eq!(
            compressed(source),
            "a{b:-0.5;c:.5;d:1.123456789;e:0;f:100;g:.3}"
        );
        assert_eq!(
            expanded(source),
            "a {\n  b: -0.5;\n  c: 0.5;\n  d: 1.123456789;\n  e: 0;\n  f: 100;\n  g: 0.3;\n}\n"
        );
    }

    #[test]
    fn colors_keep_their_channels_and_space() {
        let source = "a{b:lighten(#08c, 10%);c:mix(red, blue);d:rgba(#fff, 0.5);e:#FFFFFF;f:darken(#808080, 0%);g:scale-color(#222, $lightness: 90%);h:hsl(120, 50%, 50%);i:rgb(1.5, 2, 3);j:hsla(0, 0%, 50%, 0.5)}";
        assert_eq!(
            compressed(source),
            "a{b:#0af;c:rgb(50%,0%,50%);d:rgba(255,255,255,.5);e:#fff;f:gray;g:hsl(0,0%,91.3333333333%);h:rgb(25%,75%,25%);i:hsl(220,33.3333333333%,.8823529412%);j:hsla(0,0%,50%,.5)}"
        );
        assert_eq!(
            expanded(source),
            "a {\n  b: #00aaff;\n  c: rgb(50%, 0%, 50%);\n  d: rgba(255, 255, 255, 0.5);\n  e: #FFFFFF;\n  f: gray;\n  g: rgb(91.3333333333%, 91.3333333333%, 91.3333333333%);\n  h: hsl(120, 50%, 50%);\n  i: rgb(0.5882352941%, 0.7843137255%, 1.1764705882%);\n  j: hsla(0, 0%, 50%, 0.5);\n}\n"
        );
    }

    #[test]
    fn calculations_take_identifiers_and_constants() {
        assert_eq!(
            compressed("a{b:calc(alpha + 0.1);c:calc(pi * 1px);d:calc(var(--x) + e)}"),
            "a{b:calc(alpha + .1);c:3.1415926536px;d:calc(var(--x) + 2.7182818285)}"
        );
    }
}
