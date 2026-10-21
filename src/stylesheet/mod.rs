//! lib/stylesheet: the stylesheets Rails compiles from SCSS. The color
//! definitions and the themes' stylesheets are compiled as Rails compiles
//! them (grass with Discourse's core stylesheets, then the asset
//! processor's PostCSS); Discourse's own stylesheets are ported by hand
//! into static/css.

pub mod fonts;
pub mod manager;
pub mod postcss;
pub mod scss;

#[cfg(test)]
mod compile_tests {
    use super::{postcss, scss};
    use std::path::Path;

    const ANNOTATION: &str = "/*# sourceMappingURL=x.css.map */";

    /// What Stylesheet::Compiler fed dart-sass on the reference, what it
    /// printed (`sass.css`) and what AssetProcessor#post_css made of that
    /// (`postcss.css`), recorded by rails runner on rs-ref-a with the
    /// source map file `x.css.map`: the color definitions of several
    /// schemes, of Horizon's palettes with its color_definitions field,
    /// and Horizon's common, desktop and mobile stylesheets.
    fn fixtures() -> Vec<std::path::PathBuf> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("parity/stylesheets/compile");
        let mut dirs: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.join("input.scss").exists())
            .collect();
        dirs.sort();
        dirs
    }

    fn horizon_files() -> scss::ThemeFiles {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("parity/stylesheets/compile/horizon_files");
        let mut files = Vec::new();
        let mut dirs = vec![root.clone()];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else {
                    let relative = path
                        .strip_prefix(&root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    files.push((relative, std::fs::read_to_string(&path).unwrap()));
                }
            }
        }
        scss::ThemeFiles { files }
    }

    #[test]
    fn grass_prints_what_dart_sass_prints() {
        let theme = horizon_files();
        for dir in fixtures() {
            let name = dir.file_name().unwrap().to_string_lossy().into_owned();
            let input = std::fs::read_to_string(dir.join("input.scss")).unwrap();
            let rails = std::fs::read_to_string(dir.join("sass.css")).unwrap();
            let rails = rails
                .strip_suffix(ANNOTATION)
                .unwrap()
                .trim_end_matches('\n');
            let theme = name.starts_with("horizon_").then_some(&theme);
            let ours = scss::compile(&input, theme).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(ours == rails, "{name}: grass differs from dart-sass");
        }
    }

    #[test]
    fn post_css_matches_the_asset_processor() {
        for dir in fixtures() {
            let name = dir.file_name().unwrap().to_string_lossy().into_owned();
            let sass = std::fs::read_to_string(dir.join("sass.css")).unwrap();
            let css = sass
                .strip_suffix(ANNOTATION)
                .unwrap()
                .trim_end_matches('\n');
            let rails = std::fs::read_to_string(dir.join("postcss.css")).unwrap();
            let rails = rails
                .strip_suffix(ANNOTATION)
                .unwrap()
                .strip_suffix('\n')
                .unwrap();
            let ours = postcss::post_css(css, &sass).unwrap_or_else(|e| panic!("{name}: {e}"));
            if ours != rails {
                let at = ours
                    .bytes()
                    .zip(rails.bytes())
                    .position(|(a, b)| a != b)
                    .unwrap_or(ours.len().min(rails.len()));
                panic!(
                    "{name}: differs at {at}:\nours:  {}\nrails: {}",
                    &ours[at.saturating_sub(80)..(at + 200).min(ours.len())],
                    &rails[at.saturating_sub(80)..(at + 200).min(rails.len())]
                );
            }
        }
    }
}
