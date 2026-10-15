//! Port of lib/letter_avatar.rb and User.letter_avatar_color: the colour
//! a username's avatar gets, and the avatars `/letter_avatar` serves when
//! external_system_avatars_url is blank.
//!
//! Rails draws the letter with ImageMagick (`-font NimbusSans-Regular
//! -pointsize 280 -gravity Center -annotate -0+34`, FreeType underneath),
//! then resizes it with OptimizedImage.resize. Here the same glyph outline
//! (the URW base35 NimbusSans-Regular, LETTER_AVATAR_FONT) is placed by
//! ImageMagick's arithmetic and rasterized by ab_glyph, without FreeType's
//! light hinting, and resized by src/images.rs. The pixels differ from
//! Rails' only along the glyph's antialiased edges; the files' bytes are
//! our encoders' (README, Parity).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use ab_glyph::{Font, FontRef, PxScale};
use image::{DynamicImage, Rgb, RgbImage};

/// `LetterAvatar::COLORS`: 216 optimally distinct RGB triples.
const COLORS: [[u8; 3]; 216] = [
    [198, 125, 40],
    [61, 155, 243],
    [74, 243, 75],
    [238, 89, 166],
    [52, 240, 224],
    [177, 156, 155],
    [240, 120, 145],
    [111, 154, 78],
    [237, 179, 245],
    [237, 101, 95],
    [89, 239, 155],
    [43, 254, 70],
    [163, 212, 245],
    [65, 152, 142],
    [165, 135, 246],
    [181, 166, 38],
    [187, 229, 206],
    [77, 164, 25],
    [179, 246, 101],
    [234, 93, 37],
    [225, 155, 115],
    [142, 140, 188],
    [223, 120, 140],
    [249, 174, 27],
    [244, 117, 225],
    [137, 141, 102],
    [75, 191, 146],
    [188, 239, 142],
    [164, 199, 145],
    [173, 120, 149],
    [59, 195, 89],
    [222, 198, 220],
    [68, 145, 187],
    [236, 204, 179],
    [159, 195, 72],
    [188, 121, 189],
    [166, 160, 85],
    [181, 233, 37],
    [236, 177, 85],
    [121, 147, 160],
    [234, 218, 110],
    [241, 157, 191],
    [62, 200, 234],
    [133, 243, 34],
    [88, 149, 110],
    [59, 228, 248],
    [183, 119, 118],
    [251, 195, 45],
    [113, 196, 122],
    [197, 115, 70],
    [80, 175, 187],
    [103, 231, 238],
    [240, 72, 133],
    [228, 149, 241],
    [180, 188, 159],
    [172, 132, 85],
    [180, 135, 251],
    [236, 194, 58],
    [217, 176, 109],
    [88, 244, 199],
    [186, 157, 239],
    [113, 230, 96],
    [206, 115, 165],
    [244, 178, 163],
    [230, 139, 26],
    [241, 125, 89],
    [83, 160, 66],
    [107, 190, 166],
    [197, 161, 210],
    [198, 203, 245],
    [238, 117, 19],
    [228, 119, 116],
    [131, 156, 41],
    [145, 178, 168],
    [139, 170, 220],
    [233, 95, 125],
    [87, 178, 230],
    [157, 200, 119],
    [237, 140, 76],
    [229, 185, 186],
    [144, 206, 212],
    [236, 209, 158],
    [185, 189, 79],
    [34, 208, 66],
    [84, 238, 129],
    [133, 140, 134],
    [67, 157, 94],
    [168, 179, 25],
    [140, 145, 240],
    [151, 241, 125],
    [67, 162, 107],
    [200, 156, 21],
    [169, 173, 189],
    [226, 116, 189],
    [133, 231, 191],
    [194, 161, 63],
    [241, 77, 99],
    [241, 217, 53],
    [123, 204, 105],
    [210, 201, 119],
    [229, 108, 155],
    [240, 91, 72],
    [187, 115, 210],
    [240, 163, 100],
    [178, 217, 57],
    [179, 135, 116],
    [204, 211, 24],
    [186, 135, 57],
    [223, 176, 135],
    [204, 148, 151],
    [116, 223, 50],
    [95, 195, 46],
    [123, 160, 236],
    [181, 172, 131],
    [142, 220, 202],
    [240, 140, 112],
    [172, 145, 164],
    [228, 124, 45],
    [135, 151, 243],
    [42, 205, 125],
    [192, 233, 116],
    [119, 170, 114],
    [158, 138, 26],
    [73, 190, 183],
    [185, 229, 243],
    [227, 107, 55],
    [196, 205, 202],
    [132, 143, 60],
    [233, 192, 237],
    [62, 150, 220],
    [205, 201, 141],
    [106, 140, 190],
    [161, 131, 205],
    [135, 134, 158],
    [198, 139, 81],
    [115, 171, 32],
    [101, 181, 67],
    [149, 137, 119],
    [37, 142, 183],
    [183, 130, 175],
    [168, 125, 133],
    [124, 142, 87],
    [236, 156, 171],
    [232, 194, 91],
    [219, 200, 69],
    [144, 219, 34],
    [219, 95, 187],
    [145, 154, 217],
    [165, 185, 100],
    [127, 238, 163],
    [224, 178, 198],
    [119, 153, 120],
    [124, 212, 92],
    [172, 161, 105],
    [231, 155, 135],
    [157, 132, 101],
    [122, 185, 146],
    [53, 166, 51],
    [70, 163, 90],
    [150, 190, 213],
    [210, 107, 60],
    [166, 152, 185],
    [159, 194, 159],
    [39, 141, 222],
    [202, 176, 161],
    [95, 140, 229],
    [168, 142, 87],
    [93, 170, 203],
    [159, 142, 54],
    [14, 168, 39],
    [94, 150, 149],
    [187, 206, 136],
    [157, 224, 166],
    [235, 158, 208],
    [109, 232, 216],
    [141, 201, 87],
    [208, 124, 118],
    [142, 125, 214],
    [19, 237, 174],
    [72, 219, 41],
    [234, 102, 111],
    [168, 142, 79],
    [188, 135, 35],
    [95, 155, 143],
    [148, 173, 116],
    [223, 112, 95],
    [228, 128, 236],
    [206, 114, 54],
    [195, 119, 88],
    [235, 140, 94],
    [235, 202, 125],
    [233, 155, 153],
    [214, 214, 238],
    [246, 200, 35],
    [151, 125, 171],
    [132, 145, 172],
    [131, 142, 118],
    [199, 126, 150],
    [61, 162, 123],
    [58, 176, 151],
    [215, 141, 69],
    [225, 154, 220],
    [220, 77, 167],
    [233, 161, 64],
    [130, 221, 137],
    [81, 191, 129],
    [169, 162, 140],
    [174, 177, 222],
    [236, 174, 47],
    [233, 188, 180],
    [69, 222, 172],
    [71, 232, 93],
    [118, 211, 238],
    [157, 224, 83],
    [218, 105, 73],
    [126, 169, 36],
];

/// `User.color_index`
fn color_index(username: &str, length: usize) -> usize {
    let hex = format!("{:x}", md5::compute(username.as_bytes()));
    (u64::from_str_radix(&hex[..15], 16).unwrap_or(0) % length as u64) as usize
}

/// `User.letter_avatar_color(normalized_username)`: an MD5-derived pick
/// from the palette as 6 lowercase hex digits, or from
/// restrict_letter_avatar_colors (`restricted`, when present) as written
/// there. None where Rails' `colors[index, 6]` is nil (entries that are
/// not six characters put the index past the end).
pub fn color(normalized_username: &str, restricted: Option<&str>) -> Option<String> {
    match restricted {
        Some(colors) => {
            let length = colors.matches('|').count() + 1;
            let num = color_index(normalized_username, length);
            let index = num * 6 + num;
            let chars: Vec<char> = colors.chars().collect();
            if index > chars.len() {
                return None;
            }
            Some(chars[index..(index + 6).min(chars.len())].iter().collect())
        }
        None => {
            let [r, g, b] = COLORS[color_index(normalized_username, COLORS.len())];
            Some(format!("{r:02x}{g:02x}{b:02x}"))
        }
    }
}

/// `LetterAvatar::VERSION`
const VERSION: i32 = 5;
/// `LetterAvatar::FULLSIZE`: every avatar is drawn at this size and resized.
pub const FULLSIZE: u32 = 120 * 3;
/// `LetterAvatar::POINTSIZE`, in pixels (ImageMagick's 72 dpi).
const POINTSIZE: f32 = 280.0;
/// The vertical half of `-annotate -0+34`.
const VERTICAL_OFFSET: f64 = 34.0;
/// FreeType's size metrics for NimbusSans-Regular at 280 pixels, as
/// ImageMagick's GetTypeMetrics reports them for the reference's Type 1
/// font.
const ASCENT: f64 = 205.0;
const DESCENT: f64 = -76.0;
/// `#FFFFFFCC`: the letter's alpha.
const FILL_ALPHA: f64 = 0xCC as f64 / 255.0;
/// Changes when the pixels this module draws do. With the font file it
/// makes `version`, as ImageMagick's version and font list make Rails'.
const RENDERER: &str = "discourse-rs letter avatars 1";
/// Where Debian's fonts-urw-base35 puts the font ImageMagick draws with.
pub const DEFAULT_FONT: &str = "/usr/share/fonts/opentype/urw-base35/NimbusSans-Regular.otf";

/// `LetterAvatar::Identity`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub color: [u8; 3],
    pub letter: String,
}

impl Identity {
    /// `Identity.from_username`: the palette colour of the name as given
    /// and its first character upcased.
    pub fn from_username(username: &str) -> Identity {
        Identity {
            color: COLORS[color_index(username, COLORS.len())],
            letter: username
                .chars()
                .next()
                .map(|c| c.to_uppercase().collect())
                .unwrap_or_default(),
        }
    }
}

/// The font file's bytes (None when it can't be read) and the version
/// they make, read once per path.
struct LoadedFont {
    bytes: Option<Vec<u8>>,
    version: String,
}

fn loaded(path: &Path) -> Arc<LoadedFont> {
    static FONTS: LazyLock<Mutex<HashMap<PathBuf, Arc<LoadedFont>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    let mut fonts = FONTS.lock().unwrap_or_else(|e| e.into_inner());
    fonts
        .entry(path.to_path_buf())
        .or_insert_with(|| {
            let bytes = std::fs::read(path).ok();
            let mut identity = RENDERER.as_bytes().to_vec();
            identity.push(0);
            identity.extend_from_slice(bytes.as_deref().unwrap_or_default());
            Arc::new(LoadedFont {
                version: format!("{VERSION}_{:x}", md5::compute(&identity)),
                bytes,
            })
        })
        .clone()
}

/// `LetterAvatar.version`
pub fn version(font: &Path) -> String {
    loaded(font).version.clone()
}

/// `cached_path`: `<tmp>/letter_avatars/<version>/<letter>/<r_g_b>/<size>.png`.
pub fn cached_path(tmp_dir: &Path, version: &str, identity: &Identity, size: u32) -> PathBuf {
    let [r, g, b] = identity.color;
    tmp_dir
        .join("letter_avatars")
        .join(version)
        .join(&identity.letter)
        .join(format!("{r}_{g}_{b}"))
        .join(format!("{size}.png"))
}

/// `OptimizedImage.safe_path?` over the part of the path that varies: a
/// letter outside Ruby's ASCII `[\w\-./]`, or one `File.expand_path`
/// would change, makes the resize raise.
fn safe_letter(letter: &str) -> bool {
    letter != "."
        && letter
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

#[derive(Debug)]
pub enum GenerateError {
    Io(std::io::Error),
    /// The font is missing or not one ab_glyph reads.
    Font(PathBuf),
}

impl std::fmt::Display for GenerateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GenerateError::Io(e) => e.fmt(f),
            GenerateError::Font(path) => write!(
                f,
                "letter avatar font {} (LETTER_AVATAR_FONT) is missing or unreadable",
                path.display()
            ),
        }
    }
}

impl std::error::Error for GenerateError {}

impl From<std::io::Error> for GenerateError {
    fn from(e: std::io::Error) -> Self {
        GenerateError::Io(e)
    }
}

/// `LetterAvatar.generate(username, size)`, blocking: the cached file of
/// that size, made from the full-size drawing (drawn first when missing).
/// As in Rails, the full-size file is served as drawn when a smaller size
/// made it first, and resized in place when it is asked for first. The
/// path may not exist: Rails' resize raises for a letter outside its safe
/// paths, and then so does the caller's File.ctime.
pub fn generate(
    tmp_dir: &Path,
    font: &Path,
    username: &str,
    size: u32,
) -> Result<PathBuf, GenerateError> {
    // DistributedMutex "letter_avatar_<version>_<username>", in one process.
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let loaded = loaded(font);
    let identity = Identity::from_username(username);
    let size = size.min(FULLSIZE);
    let filename = cached_path(tmp_dir, &loaded.version, &identity, size);
    if filename.exists() {
        return Ok(filename);
    }
    let fullsize = cached_path(tmp_dir, &loaded.version, &identity, FULLSIZE);
    if !fullsize.exists() {
        let font_ref = loaded
            .bytes
            .as_deref()
            .and_then(|b| FontRef::try_from_slice(b).ok())
            .ok_or_else(|| GenerateError::Font(font.to_path_buf()))?;
        write_atomically(&fullsize, &render(&font_ref, &identity)?)?;
    }
    if !safe_letter(&identity.letter) {
        tracing::warn!(letter = %identity.letter, "letter avatar resize refused: unsafe path");
        return Ok(filename);
    }
    let original = std::fs::read(&fullsize)?;
    match crate::images::thumbnail(&original, crate::images::Format::Png, size, size, false, 92) {
        Some(resized) => write_atomically(&filename, &resized)?,
        None => tracing::warn!(path = %fullsize.display(), "letter avatar resize failed"),
    }
    Ok(filename)
}

fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("png.{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// `generate_fullsize`: the letter drawn on its colour at FULLSIZE, as a
/// PNG.
fn render(font: &FontRef<'_>, identity: &Identity) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    DynamicImage::ImageRgb8(draw(font, identity))
        .write_with_encoder(image::codecs::png::PngEncoder::new(&mut out))
        .map_err(std::io::Error::other)?;
    Ok(out)
}

/// The pixels of `generate_fullsize`, placed as ImageMagick's
/// AnnotateImage (CenterGravity) and RenderFreetype place them.
fn draw(font: &FontRef<'_>, identity: &Identity) -> RgbImage {
    let [r, g, b] = identity.color;
    let mut image = RgbImage::from_pixel(FULLSIZE, FULLSIZE, Rgb([r, g, b]));
    let Some(c) = identity.letter.chars().next() else {
        return image;
    };
    let units = font.units_per_em().unwrap_or(1000.0);
    // ab_glyph scales by the font's height, ImageMagick by the em.
    let scale = PxScale::from(POINTSIZE * font.height_unscaled() / units);
    let px_per_unit = f64::from(POINTSIZE / units);
    // RenderFreetype draws a glyph the font lacks as the space.
    let mut id = font.glyph_id(c);
    if id.0 == 0 {
        id = font.glyph_id(' ');
    }
    let outlined = font.outline_glyph(id.with_scale_and_position(scale, ab_glyph::point(0.0, 0.0)));
    // metrics.width: the advance rounded to whole pixels (hinting does)
    // or the outline's right edge if further, rounded up.
    let advance = (f64::from(font.h_advance_unscaled(id)) * px_per_unit).round();
    let x_max = outlined
        .as_ref()
        .map_or(0.0, |o| f64::from(o.px_bounds().max.x));
    let width = advance.max(x_max).ceil();
    // CenterGravity with one line, the geometry's -0+34 added.
    let offset_x = f64::from(FULLSIZE) / 2.0 - width / 2.0;
    let offset_y = f64::from(FULLSIZE) / 2.0 + VERTICAL_OFFSET + (ASCENT + DESCENT) / 2.0;
    // Each bitmap pixel lands at ceil(point - 0.5).
    let x0 = (offset_x - 0.5).ceil() as i64;
    let y0 = (offset_y - 0.5).ceil() as i64;
    let Some(outlined) = outlined else {
        return image;
    };
    let bounds = outlined.px_bounds();
    let (left, top) = (bounds.min.x as i64, bounds.min.y as i64);
    outlined.draw(|x, y, coverage| {
        let (dx, dy) = (x0 + left + i64::from(x), y0 + top + i64::from(y));
        if dx < 0 || dy < 0 || dx >= i64::from(FULLSIZE) || dy >= i64::from(FULLSIZE) {
            return;
        }
        // FreeType's 256 grey levels, then CompositePixelOver of white at
        // the fill's alpha, written at depth 8.
        let grey = (f64::from(coverage) * 256.0).floor().clamp(0.0, 255.0);
        let alpha = grey / 255.0 * FILL_ALPHA;
        let pixel = image.get_pixel_mut(dx as u32, dy as u32);
        for channel in pixel.0.iter_mut() {
            let value = 255.0 * alpha + f64::from(*channel) * (1.0 - alpha);
            *channel = (value + 0.5).floor().clamp(0.0, 255.0) as u8;
        }
    });
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_is_complete() {
        assert_eq!(COLORS[0], [198, 125, 40]);
        assert_eq!(COLORS.len(), 216);
    }

    #[test]
    fn identities_match_rails() {
        // LetterAvatar::Identity.from_username on the reference.
        let cases = [
            ("sam", "S", [95, 155, 143]),
            ("trent", "T", [240, 91, 72]),
            ("0zero", "0", [241, 217, 53]),
            ("_under", "_", [236, 156, 171]),
            ("system", "S", [188, 239, 142]),
            ("é", "É", [103, 231, 238]),
            ("ж", "Ж", [205, 201, 141]),
        ];
        for (name, letter, color) in cases {
            let id = Identity::from_username(name);
            assert_eq!((id.letter.as_str(), id.color), (letter, color), "{name}");
        }
    }

    #[test]
    fn restricted_colors_are_picked_from_the_setting() {
        let restricted = "ff0000|00ff00|0000ff";
        let picked = color("alice", Some(restricted)).unwrap();
        let index = color_index("alice", 3);
        assert_eq!(picked, restricted.split('|').nth(index).unwrap());
        // `colors[index, 6]` past the end is nil: "a|b" is 3 characters,
        // the second entry's index 7.
        let second = ["alice", "bob", "carol", "dave", "eve"]
            .into_iter()
            .find(|n| color_index(n, 2) == 1)
            .unwrap();
        assert_eq!(color(second, Some("a|b")), None);
    }

    /// parity/letter_avatars holds Rails' `generate_fullsize` output for a
    /// few names (ImageMagick 7.1.2, FreeType 2.12, the reference's
    /// NimbusSans-Regular). Ours differs only where FreeType's light
    /// hinting moves a horizontal edge (the overshoot of round letters)
    /// within a pixel.
    #[test]
    fn drawing_matches_rails_but_for_the_edges() {
        let font_path = std::env::var("LETTER_AVATAR_FONT").unwrap_or(DEFAULT_FONT.into());
        let bytes = std::fs::read(&font_path)
            .unwrap_or_else(|e| panic!("LETTER_AVATAR_FONT {font_path}: {e}"));
        let font = FontRef::try_from_slice(&bytes).unwrap();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("parity/letter_avatars");
        for name in ["alice", "oscar", "0zero", "_under"] {
            let want = image::open(dir.join(format!("{name}.png")))
                .unwrap()
                .to_rgb8();
            let got = draw(&font, &Identity::from_username(name));
            assert_eq!(want.dimensions(), got.dimensions());
            let diffs: Vec<i32> = want
                .pixels()
                .zip(got.pixels())
                .map(|(a, b)| {
                    (0..3)
                        .map(|c| (i32::from(a[c]) - i32::from(b[c])).abs())
                        .max()
                        .unwrap_or(0)
                })
                .collect();
            let differing = diffs.iter().filter(|d| **d > 0).count();
            assert!(differing < 2000, "{name}: {differing} pixels differ");
            // Shown at 48 (OptimizedImage.resize of both), the hinting's
            // shifts average out.
            let small = |png: &[u8]| {
                let thumb =
                    crate::images::thumbnail(png, crate::images::Format::Png, 48, 48, false, 92)
                        .unwrap();
                image::load_from_memory(&thumb).unwrap().to_rgb8()
            };
            let mut ours = Vec::new();
            DynamicImage::ImageRgb8(got)
                .write_with_encoder(image::codecs::png::PngEncoder::new(&mut ours))
                .unwrap();
            let theirs = std::fs::read(dir.join(format!("{name}.png"))).unwrap();
            let (a, b) = (small(&theirs), small(&ours));
            let diffs: Vec<i32> = a
                .pixels()
                .zip(b.pixels())
                .flat_map(|(a, b)| (0..3).map(move |c| (i32::from(a[c]) - i32::from(b[c])).abs()))
                .collect();
            let max = diffs.iter().copied().max().unwrap_or(0);
            let mean = diffs.iter().sum::<i32>() as f64 / diffs.len() as f64;
            assert!(
                max <= 40 && mean < 2.0,
                "{name} at 48: max {max}, mean {mean}"
            );
        }
    }

    #[test]
    fn unsafe_letters_are_not_resized() {
        assert!(safe_letter("A"));
        assert!(safe_letter("_"));
        assert!(!safe_letter("É"));
        assert!(!safe_letter("."));
    }
}
