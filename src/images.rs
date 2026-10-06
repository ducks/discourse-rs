//! Image work done in-process, in place of the tools Rails runs on an
//! upload (FastImage, ImageMagick, oxipng, pngquant, jpegoptim). Each
//! function names the Rails step it stands for and keeps its decisions
//! (sizes, qualities, which result is kept); the bytes it writes are its
//! own, so an image's sha1 and file size differ from Rails' (README,
//! Parity).
//!
//! Untrusted bytes are only ever parsed in Rust: the `image` crate's
//! decoders read uploads, and oxipng (whose deflate is libdeflate, in C)
//! only compresses pixels decoded here.

use std::io::Cursor;

use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, ImageFormat, RgbaImage};

/// The image formats uploads are processed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl Format {
    /// FastImage's type name, as uploads store it.
    pub fn name(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg => "jpeg",
            Format::Gif => "gif",
            Format::Webp => "webp",
        }
    }

    fn image_format(self) -> ImageFormat {
        match self {
            Format::Png => ImageFormat::Png,
            Format::Jpeg => ImageFormat::Jpeg,
            Format::Gif => ImageFormat::Gif,
            Format::Webp => ImageFormat::WebP,
        }
    }

    /// From an extension (`OptimizedImage`'s output format).
    pub fn from_extension(ext: &str) -> Option<Format> {
        match ext.trim_start_matches('.').to_lowercase().as_str() {
            "png" => Some(Format::Png),
            "jpg" | "jpeg" => Some(Format::Jpeg),
            "gif" => Some(Format::Gif),
            "webp" => Some(Format::Webp),
            _ => None,
        }
    }
}

/// What FastImage reads from an image: its type, size, whether it is
/// animated and (for a JPEG) its EXIF orientation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Info {
    pub format: Format,
    pub width: u32,
    pub height: u32,
    pub animated: bool,
    pub orientation: u32,
}

impl Info {
    pub fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

/// The format by its magic bytes.
pub fn sniff(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(Format::Gif)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Format::Png)
    } else if bytes.starts_with(b"\xff\xd8") {
        Some(Format::Jpeg)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(Format::Webp)
    } else {
        None
    }
}

/// A GIF's frame count (FastImage's `animated?` counts image descriptors).
fn gif_frames(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 13 {
        return None;
    }
    let flags = bytes[10];
    let mut i = 13;
    if flags & 0x80 != 0 {
        i += 3 * (1usize << ((flags & 0x07) + 1));
    }
    let skip_sub_blocks = |mut i: usize| -> Option<usize> {
        loop {
            let len = *bytes.get(i)? as usize;
            i += 1;
            if len == 0 {
                return Some(i);
            }
            i += len;
        }
    };
    let mut frames = 0;
    loop {
        match *bytes.get(i)? {
            0x21 => i = skip_sub_blocks(i + 2)?,
            0x2C => {
                frames += 1;
                let local = *bytes.get(i + 9)?;
                i += 10;
                if local & 0x80 != 0 {
                    i += 3 * (1usize << ((local & 0x07) + 1));
                }
                i = skip_sub_blocks(i + 1)?;
            }
            0x3B => return Some(frames),
            _ => return None,
        }
    }
}

/// An APNG: an `acTL` chunk before the first `IDAT`.
fn png_animated(bytes: &[u8]) -> bool {
    let mut i = 8;
    while i + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        let kind = &bytes[i + 4..i + 8];
        if kind == b"acTL" {
            return true;
        }
        if kind == b"IDAT" {
            return false;
        }
        i += 12 + len;
    }
    false
}

/// A WebP with the animation flag in its `VP8X` header.
fn webp_animated(bytes: &[u8]) -> bool {
    bytes.len() > 20 && &bytes[12..16] == b"VP8X" && bytes[20] & 0x02 != 0
}

/// A JPEG's EXIF orientation, 1 without one.
fn jpeg_orientation(bytes: &[u8]) -> u32 {
    exif::Reader::new()
        .read_from_container(&mut Cursor::new(bytes))
        .ok()
        .and_then(|e| {
            e.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                .and_then(|f| f.value.get_uint(0))
        })
        .filter(|o| (1..=8).contains(o))
        .unwrap_or(1)
}

/// FastImage.new(file): None when it is not an image it can size. A
/// JPEG's size is as it displays (orientations 5 to 8 swap the sides).
pub fn info(bytes: &[u8]) -> Option<Info> {
    let format = sniff(bytes)?;
    let (mut width, mut height) =
        image::ImageReader::with_format(Cursor::new(bytes), format.image_format())
            .into_dimensions()
            .ok()?;
    let orientation = if format == Format::Jpeg {
        jpeg_orientation(bytes)
    } else {
        1
    };
    if orientation > 4 {
        std::mem::swap(&mut width, &mut height);
    }
    let animated = match format {
        Format::Gif => gif_frames(bytes)? > 1,
        Format::Png => png_animated(bytes),
        Format::Webp => webp_animated(bytes),
        Format::Jpeg => false,
    };
    Some(Info {
        format,
        width,
        height,
        animated,
        orientation,
    })
}

/// The image's first frame, decoded.
pub fn decode(bytes: &[u8], format: Format) -> Result<DynamicImage, image::ImageError> {
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format.image_format());
    let mut limits = image::Limits::default();
    // Decoding is only reached under max_image_megapixels; this bounds
    // the allocation of a lying header.
    limits.max_alloc = Some(1 << 30);
    reader.limits(limits);
    reader.decode()
}

/// `-auto-orient`: the pixels turned as the EXIF orientation says.
pub fn auto_orient(mut image: DynamicImage, orientation: u32) -> DynamicImage {
    if let Some(o) = u8::try_from(orientation)
        .ok()
        .and_then(image::metadata::Orientation::from_exif)
    {
        image.apply_orientation(o);
    }
    image
}

/// The libjpeg standard luminance quantization table (quality 50), in
/// natural order as the DQT segment stores it after de-zigzagging; the
/// estimate only compares sums, so zigzag order is fine.
const STD_LUMINANCE: [u32; 64] = [
    16, 11, 12, 14, 12, 10, 16, 14, 13, 14, 18, 17, 16, 19, 24, 40, 26, 24, 22, 22, 24, 49, 35, 37,
    29, 40, 58, 51, 61, 60, 57, 51, 56, 55, 64, 72, 92, 78, 64, 68, 87, 69, 55, 56, 80, 109, 81,
    87, 95, 98, 103, 104, 103, 62, 77, 113, 121, 112, 100, 120, 92, 101, 103, 99,
];

/// `identify -format %Q`: a JPEG's quality, estimated from its first
/// quantization table as libjpeg scales the standard one (0 when it has
/// none). ImageMagick matches hashed table sums against the same scaling,
/// so the two agree on tables libjpeg made.
pub fn jpeg_quality(bytes: &[u8]) -> u32 {
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            return 0;
        }
        let marker = bytes[i + 1];
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if marker == 0xDA {
            return 0;
        }
        if marker == 0xDB && i + 2 + len <= bytes.len() && len >= 67 {
            let precision = bytes[i + 4] >> 4;
            let table: Vec<u32> = if precision == 0 {
                bytes[i + 5..i + 69].iter().map(|b| u32::from(*b)).collect()
            } else {
                bytes[i + 5..i + 133]
                    .chunks(2)
                    .map(|c| u32::from(u16::from_be_bytes([c[0], c[1]])))
                    .collect()
            };
            let sum: u32 = table.iter().sum();
            let std_sum: u32 = STD_LUMINANCE.iter().sum();
            let scale = f64::from(sum) * 100.0 / f64::from(std_sum);
            let quality = if scale <= 100.0 {
                (200.0 - scale) / 2.0
            } else {
                5000.0 / scale
            };
            return quality.round().clamp(1.0, 100.0) as u32;
        }
        i += 2 + len;
    }
    0
}

/// `jpegoptim --strip-all`'s lossless part that we do: every APP segment
/// but JFIF's, and comments, removed. (jpegoptim also rewrites the Huffman
/// tables; that is not done, so files stay a few percent larger.)
pub fn strip_jpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&bytes[..2]);
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        if marker == 0xDA {
            out.extend_from_slice(&bytes[i..]);
            return Some(out);
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        let end = i + 2 + len;
        if end > bytes.len() {
            return None;
        }
        let drop = (0xE1..=0xEF).contains(&marker) || marker == 0xFE;
        if !drop {
            out.extend_from_slice(&bytes[i..end]);
        }
        i = end;
    }
    None
}

/// The pixels as oxipng's raw input, keeping their channels and depth.
fn raw_png(image: &DynamicImage) -> Option<oxipng::RawImage> {
    use oxipng::{BitDepth, ColorType};
    let (w, h) = (image.width(), image.height());
    let sixteen =
        |samples: &[u16]| -> Vec<u8> { samples.iter().flat_map(|s| s.to_be_bytes()).collect() };
    let (color, depth, data) = match image {
        DynamicImage::ImageLuma8(i) => (
            ColorType::Grayscale {
                transparent_shade: None,
            },
            BitDepth::Eight,
            i.as_raw().clone(),
        ),
        DynamicImage::ImageLumaA8(i) => (
            ColorType::GrayscaleAlpha,
            BitDepth::Eight,
            i.as_raw().clone(),
        ),
        DynamicImage::ImageRgb8(i) => (
            ColorType::RGB {
                transparent_color: None,
            },
            BitDepth::Eight,
            i.as_raw().clone(),
        ),
        DynamicImage::ImageRgba8(i) => (ColorType::RGBA, BitDepth::Eight, i.as_raw().clone()),
        DynamicImage::ImageLuma16(i) => (
            ColorType::Grayscale {
                transparent_shade: None,
            },
            BitDepth::Sixteen,
            sixteen(i.as_raw()),
        ),
        DynamicImage::ImageLumaA16(i) => (
            ColorType::GrayscaleAlpha,
            BitDepth::Sixteen,
            sixteen(i.as_raw()),
        ),
        DynamicImage::ImageRgb16(i) => (
            ColorType::RGB {
                transparent_color: None,
            },
            BitDepth::Sixteen,
            sixteen(i.as_raw()),
        ),
        DynamicImage::ImageRgba16(i) => (ColorType::RGBA, BitDepth::Sixteen, sixteen(i.as_raw())),
        other => (
            ColorType::RGBA,
            BitDepth::Eight,
            other.to_rgba8().into_raw(),
        ),
    };
    oxipng::RawImage::new(w, h, color, depth, data).ok()
}

/// `oxipng --opt 3 --interlace 0 --strip all`
fn oxipng(raw: &oxipng::RawImage) -> Option<Vec<u8>> {
    let mut options = oxipng::Options::from_preset(3);
    options.interlace = Some(false);
    options.strip = oxipng::StripChunks::All;
    raw.create_optimized_png(&options).ok()
}

/// `pngquant --quality=0-100 --speed=3 --skip-if-larger --force 256`,
/// written out by oxipng.
fn pngquant(image: &DynamicImage) -> Option<Vec<u8>> {
    let rgba = image.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    let pixels: Vec<imagequant::RGBA> = rgba
        .pixels()
        .map(|p| imagequant::RGBA::new(p[0], p[1], p[2], p[3]))
        .collect();
    let mut attr = imagequant::new();
    attr.set_speed(3).ok()?;
    attr.set_quality(0, 100).ok()?;
    attr.set_max_colors(256).ok()?;
    let mut img = attr.new_image(pixels, w, h, 0.0).ok()?;
    let mut result = attr.quantize(&mut img).ok()?;
    result.set_dithering_level(1.0).ok()?;
    let (palette, indexes) = result.remapped(&mut img).ok()?;
    let raw = oxipng::RawImage::new(
        w as u32,
        h as u32,
        oxipng::ColorType::Indexed { palette },
        oxipng::BitDepth::Eight,
        indexes,
    )
    .ok()?;
    oxipng(&raw)
}

/// `FileHelper.optimize_image!`: each step of the pipeline runs on the
/// current file and is kept only when smaller (PNG: oxipng, then pngquant
/// when allowed; JPEG: the metadata stripped). None when nothing was
/// smaller. Keeping metadata (strip_image_metadata off) is not supported:
/// the PNG steps work from pixels.
pub fn optimize(bytes: &[u8], format: Format, allow_pngquant: bool) -> Option<Vec<u8>> {
    match format {
        Format::Png => {
            let image = decode(bytes, Format::Png).ok()?;
            let mut best: Option<Vec<u8>> = None;
            let current_len = |best: &Option<Vec<u8>>| best.as_ref().map_or(bytes.len(), Vec::len);
            if let Some(out) = raw_png(&image).as_ref().and_then(oxipng)
                && !out.is_empty()
                && out.len() < current_len(&best)
            {
                best = Some(out);
            }
            if allow_pngquant
                && let Some(out) = pngquant(&image)
                && !out.is_empty()
                && out.len() < current_len(&best)
            {
                best = Some(out);
            }
            best
        }
        Format::Jpeg => strip_jpeg(bytes).filter(|out| out.len() < bytes.len()),
        Format::Gif | Format::Webp => None,
    }
}

/// The image written as `format` (a JPEG at `quality`, flattened on
/// `background` as ImageMagick's JPEG writer drops alpha; a WebP lossless,
/// the encoder this crate has).
pub fn encode(
    image: &DynamicImage,
    format: Format,
    quality: u8,
    background: Option<[u8; 3]>,
) -> Result<Vec<u8>, image::ImageError> {
    let mut out = Vec::new();
    match format {
        Format::Jpeg => {
            let rgb = match background {
                Some(bg) => flatten(image, bg),
                None => DynamicImage::ImageRgb8(image.to_rgb8()),
            };
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
            rgb.write_with_encoder(encoder)?;
        }
        Format::Png => {
            image.write_with_encoder(image::codecs::png::PngEncoder::new(&mut out))?;
        }
        Format::Gif => {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut out);
            encoder.encode_frame(image::Frame::new(image.to_rgba8()))?;
        }
        Format::Webp => {
            let encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut out);
            DynamicImage::ImageRgba8(image.to_rgba8()).write_with_encoder(encoder)?;
        }
    }
    Ok(out)
}

/// `-background <color> -flatten`: transparency composited on a colour.
fn flatten(image: &DynamicImage, bg: [u8; 3]) -> DynamicImage {
    let rgba = image.to_rgba8();
    let mut out = image::RgbImage::new(rgba.width(), rgba.height());
    for (src, dst) in rgba.pixels().zip(out.pixels_mut()) {
        let a = u32::from(src[3]);
        for c in 0..3 {
            dst[c] = ((u32::from(src[c]) * a + u32::from(bg[c]) * (255 - a) + 127) / 255) as u8;
        }
    }
    DynamicImage::ImageRgb8(out)
}

/// The image resized to fill `width`x`height`, cropping the overflow
/// around `centre` (0.5, 0.5 for `-gravity center -extent`, 0.5, 0.0 for
/// `-gravity north -crop`), with ImageMagick's downscaling filter: Mitchell
/// for images with transparency, Lanczos otherwise.
fn fill(image: &DynamicImage, width: u32, height: u32, centre: (f64, f64)) -> Option<RgbaImage> {
    let src = DynamicImage::ImageRgba8(image.to_rgba8());
    let filter = if image.color().has_alpha() {
        FilterType::Mitchell
    } else {
        FilterType::Lanczos3
    };
    let mut dst = DynamicImage::ImageRgba8(RgbaImage::new(width.max(1), height.max(1)));
    Resizer::new()
        .resize(
            &src,
            &mut dst,
            &ResizeOptions::new()
                .resize_alg(ResizeAlg::Convolution(filter))
                .fit_into_destination(Some(centre)),
        )
        .ok()?;
    Some(dst.to_rgba8())
}

/// `-unsharp 2x0.5+0.7+0`: a Gaussian of radius 2 and sigma 0.5, the
/// difference added back at 0.7, on the colour channels.
fn unsharp(image: &mut RgbaImage) {
    let sigma = 0.5f32;
    let weights: Vec<f32> = (-2i32..=2)
        .map(|x| (-((x * x) as f32) / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f32 = weights.iter().sum();
    let weights: Vec<f32> = weights.iter().map(|w| w / total).collect();
    let (w, h) = (image.width() as i32, image.height() as i32);
    let src = image.clone();
    let blur_pass = |from: &RgbaImage, horizontal: bool| -> Vec<[f32; 3]> {
        let mut out = vec![[0f32; 3]; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0f32; 3];
                for (k, weight) in weights.iter().enumerate() {
                    let d = k as i32 - 2;
                    let (sx, sy) = if horizontal {
                        ((x + d).clamp(0, w - 1), y)
                    } else {
                        (x, (y + d).clamp(0, h - 1))
                    };
                    let p = from.get_pixel(sx as u32, sy as u32);
                    for c in 0..3 {
                        acc[c] += f32::from(p[c]) * weight;
                    }
                }
                out[(y * w + x) as usize] = acc;
            }
        }
        out
    };
    let horizontal = blur_pass(&src, true);
    let mut mid = src.clone();
    for (i, p) in mid.pixels_mut().enumerate() {
        for c in 0..3 {
            p[c] = horizontal[i][c].round().clamp(0.0, 255.0) as u8;
        }
    }
    let blurred = blur_pass(&mid, false);
    for (i, p) in image.pixels_mut().enumerate() {
        for c in 0..3 {
            let orig = f32::from(p[c]);
            p[c] = (orig + (orig - blurred[i][c]) * 0.7)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
}

/// `OptimizedImage.resize` (`crop: false`) or `.crop`: the thumbnail at
/// `width`x`height` as ImageMagick's instructions make it, then
/// `optimize_image` (pngquant for PNGs under MAX_PNGQUANT_SIZE).
pub fn thumbnail(
    bytes: &[u8],
    format: Format,
    width: u32,
    height: u32,
    crop: bool,
    jpeg_quality: u8,
) -> Option<Vec<u8>> {
    let image = auto_orient(
        decode(bytes, format).ok()?,
        jpeg_orientation_of(bytes, format),
    );
    let centre = if crop { (0.5, 0.0) } else { (0.5, 0.5) };
    let mut resized = fill(&image, width, height, centre)?;
    unsharp(&mut resized);
    let out = encode(
        &DynamicImage::ImageRgba8(resized),
        format,
        jpeg_quality,
        None,
    )
    .ok()?;
    let allow_pngquant = format == Format::Png && out.len() < 500_000;
    Some(optimize(&out, format, allow_pngquant).unwrap_or(out))
}

fn jpeg_orientation_of(bytes: &[u8], format: Format) -> u32 {
    if format == Format::Jpeg {
        jpeg_orientation(bytes)
    } else {
        1
    }
}

/// `OptimizedImage.downsize(scale: 0.5)`: half the size, the same format.
pub fn downsize_half(bytes: &[u8], format: Format) -> Option<Vec<u8>> {
    let image = auto_orient(
        decode(bytes, format).ok()?,
        jpeg_orientation_of(bytes, format),
    );
    let w = ((f64::from(image.width()) * 0.5).round() as u32).max(1);
    let h = ((f64::from(image.height()) * 0.5).round() as u32).max(1);
    let resized = fill(&image, w, h, (0.5, 0.5))?;
    encode(&DynamicImage::ImageRgba8(resized), format, 92, None).ok()
}

/// `UploadCreator#execute_convert` to JPEG: oriented, flattened on white,
/// at `quality`.
pub fn to_jpeg(bytes: &[u8], format: Format, quality: u8) -> Option<Vec<u8>> {
    let image = auto_orient(
        decode(bytes, format).ok()?,
        jpeg_orientation_of(bytes, format),
    );
    encode(&image, Format::Jpeg, quality, Some([255, 255, 255])).ok()
}

/// `fix_orientation!`: the JPEG's pixels turned upright, re-encoded at
/// its own quality (ImageMagick keeps the estimated quality).
pub fn fix_orientation(bytes: &[u8]) -> Option<Vec<u8>> {
    let orientation = jpeg_orientation(bytes);
    let image = auto_orient(decode(bytes, Format::Jpeg).ok()?, orientation);
    let quality = match jpeg_quality(bytes) {
        0 => 92,
        q => q as u8,
    };
    encode(&image, Format::Jpeg, quality, None).ok()
}

/// An image ImageMagick reads as PseudoClass (a palette): GIFs and
/// indexed PNGs.
fn has_palette(bytes: &[u8], format: Format) -> bool {
    match format {
        Format::Gif => true,
        // IHDR's colour type, 3 for indexed.
        Format::Png => bytes.get(25) == Some(&3),
        _ => false,
    }
}

/// `Upload#calculate_dominant_color!`: the whole image squeezed to one
/// pixel (`-resize 1x1`, the first frame), as `RRGGBB`. ImageMagick's
/// filter for shrinking is Lanczos, Mitchell for a palette or
/// transparency. None when it does not decode, which Rails saves as an
/// empty colour.
pub fn dominant_color(bytes: &[u8], format: Format) -> Option<String> {
    let image = decode(bytes, format).ok()?;
    let filter = if has_palette(bytes, format) || image.color().has_alpha() {
        FilterType::Mitchell
    } else {
        FilterType::Lanczos3
    };
    let src = DynamicImage::ImageRgba8(image.to_rgba8());
    let mut dst = DynamicImage::ImageRgba8(RgbaImage::new(1, 1));
    Resizer::new()
        .resize(
            &src,
            &mut dst,
            &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(filter)),
        )
        .ok()?;
    let p = dst.to_rgba8().get_pixel(0, 0).0;
    Some(format!("{:02X}{:02X}{:02X}", p[0], p[1], p[2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([
                (x * 7 % 256) as u8,
                (y * 5 % 256) as u8,
                ((x + y) % 256) as u8,
                255,
            ])
        });
        encode(&DynamicImage::ImageRgba8(image), Format::Png, 90, None).unwrap()
    }

    #[test]
    fn info_reads_the_header() {
        let bytes = png(40, 30);
        let info = info(&bytes).unwrap();
        assert_eq!(
            (info.format, info.width, info.height, info.animated),
            (Format::Png, 40, 30, false)
        );
    }

    #[test]
    fn thumbnails_fill_the_size() {
        let bytes = png(400, 300);
        let thumb = thumbnail(&bytes, Format::Png, 200, 100, false, 90).unwrap();
        let i = info(&thumb).unwrap();
        assert_eq!((i.width, i.height), (200, 100));
        let jpeg = to_jpeg(&bytes, Format::Png, 90).unwrap();
        assert_eq!(info(&jpeg).unwrap().format, Format::Jpeg);
        assert!(
            (85..=95).contains(&jpeg_quality(&jpeg)),
            "{}",
            jpeg_quality(&jpeg)
        );
    }

    /// The colours the reference saved for the upload fixtures
    /// (parity/writes/upload_*.json).
    #[test]
    fn dominant_colours_match_the_reference() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("parity/writes/files");
        for (name, format, color) in [
            ("static.gif", Format::Gif, "3366CC"),
            ("animated.gif", Format::Gif, "FF0000"),
            ("photo.jpg", Format::Jpeg, "3C4C5A"),
            ("small.png", Format::Png, "355258"),
        ] {
            let bytes = std::fs::read(dir.join(name)).unwrap();
            assert_eq!(
                dominant_color(&bytes, format).as_deref(),
                Some(color),
                "{name}"
            );
        }
    }

    #[test]
    fn a_flat_image_is_its_own_dominant_colour() {
        let image = RgbaImage::from_pixel(50, 50, image::Rgba([0x25, 0xAA, 0xE2, 255]));
        let bytes = encode(&DynamicImage::ImageRgba8(image), Format::Png, 90, None).unwrap();
        assert_eq!(
            dominant_color(&bytes, Format::Png).as_deref(),
            Some("25AAE2")
        );
    }
}
