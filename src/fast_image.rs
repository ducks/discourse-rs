//! `FastImage.size(url)` (the fastimage gem, 2.4.1, as Discourse patches
//! it): an image's size read from the beginning of its file over HTTP,
//! connecting only to addresses the SSRF check allows
//! (FinalDestination::HTTP), certificates unverified, up to four
//! redirects, two seconds a step. None wherever FastImage gives nil (a
//! failed request, a type it doesn't know, a file it can't parse).
//!
//! Refused: SVG, HEIC/HEIF, AVIF and JPEG XL images, and the files
//! whose broken headers make FastImage raise past its own rescue.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use crate::final_destination::{Fetcher, Lookup};
use crate::onebox::uri::RubyUri;
use crate::{AppError, Unsupported};

/// `FastImage::DefaultTimeout`
const TIMEOUT: Duration = Duration::from_secs(2);
/// FastImage follows this many redirects.
const MAX_REDIRECTS: u32 = 4;
/// `FinalDestination::HTTP::MAX_ADDRESSES_PER_FAMILY`
const MAX_ADDRESSES_PER_FAMILY: usize = 5;

/// `FastImage.size(url)`: `(width, height)` as displayed (a JPEG or TIFF
/// whose EXIF orientation turns it has its sides swapped).
pub async fn size(fetcher: &Fetcher<'_>, url: &str) -> Result<Option<(i64, i64)>, AppError> {
    // URI.parse, http(s) only (anything else is opened as a file name,
    // which no url of another site is).
    let Some(mut uri) = RubyUri::parse(url) else {
        return Ok(None);
    };
    let mut redirects = 0;
    loop {
        if !matches!(uri.scheme.as_deref(), Some("http" | "https")) || uri.host.is_none() {
            return Ok(None);
        }
        let host = uri.host.clone().unwrap_or_default();
        let host = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        let port = uri
            .port
            .unwrap_or(if uri.scheme.as_deref() == Some("https") {
                443
            } else {
                80
            });
        // FinalDestination::HTTP#connect: the allowed addresses, IPv6
        // first, five of each family; none is a SocketError.
        let ips = match fetcher.lookup_and_filter_ips(&host).await? {
            Lookup::Allowed(ips) => ips,
            Lookup::Disallowed | Lookup::Failed => return Ok(None),
        };
        let (v6, v4): (Vec<IpAddr>, Vec<IpAddr>) = ips.into_iter().partition(IpAddr::is_ipv6);
        let addrs: Vec<SocketAddr> = v6
            .into_iter()
            .take(MAX_ADDRESSES_PER_FAMILY)
            .chain(v4.into_iter().take(MAX_ADDRESSES_PER_FAMILY))
            .map(|ip| SocketAddr::new(ip, port))
            .collect();
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .danger_accept_invalid_certs(true)
            .connect_timeout(TIMEOUT)
            .resolve_to_addrs(&host, &addrs)
            .build()?;
        let request = client
            .get(uri.to_s())
            .header("Accept-Encoding", "identity")
            .header("Accept", "*/*")
            .header("User-Agent", crate::schema::user_agent());
        let response = match tokio::time::timeout(TIMEOUT, request.send()).await {
            Ok(Ok(r)) => r,
            _ => return Ok(None),
        };
        let status = response.status();
        if status.is_redirection() && redirects < MAX_REDIRECTS {
            redirects += 1;
            let Some(location) = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .filter(|l| !l.is_empty())
                .map(str::to_string)
            else {
                return Ok(None);
            };
            // URI.join(uri, escaped_location(location)); an invalid one
            // leaves the redirect itself, which is no success.
            match uri.merge(&escaped_location(&location)) {
                Some(next) => {
                    uri = next;
                    continue;
                }
                None => return Ok(None),
            }
        }
        if !status.is_success() {
            return Ok(None);
        }
        let gzip = response
            .headers()
            .get("content-encoding")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|e| matches!(e, "deflate" | "gzip" | "x-gzip"));
        return read_size(response, gzip).await;
    }
}

/// The body read until its size parses (or can't).
async fn read_size(
    mut response: reqwest::Response,
    gzip: bool,
) -> Result<Option<(i64, i64)>, AppError> {
    let mut body = Vec::new();
    loop {
        let chunk = match tokio::time::timeout(TIMEOUT, response.chunk()).await {
            Ok(Ok(chunk)) => chunk,
            // A read timeout or a broken connection.
            _ => return Ok(None),
        };
        let eof = chunk.is_none();
        if let Some(chunk) = chunk {
            body.extend_from_slice(&chunk);
        }
        let parsed = if gzip {
            // Zlib::GzipReader over what has come: what decodes so far.
            let mut decoded = Vec::new();
            let mut decoder = flate2::read::GzDecoder::new(&body[..]);
            let ok = std::io::Read::read_to_end(&mut decoder, &mut decoded).is_ok();
            if !ok && eof && decoded.is_empty() {
                return Ok(None);
            }
            parse(&decoded, eof)
        } else {
            parse(&body, eof)
        };
        match parsed {
            Parsed::More => {}
            Parsed::Done(size) => return Ok(size),
            Parsed::Refused(what) => return Err(Unsupported(what).into()),
        }
    }
}

/// `escaped_location`: the location as given when URI takes it, else
/// `URI::DEFAULT_PARSER.escape`d.
fn escaped_location(location: &str) -> String {
    if RubyUri::parse(location).is_some() {
        return location.to_string();
    }
    let mut out = String::new();
    for b in location.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'();/?:@&=+$,[]".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Where parsing the bytes read so far got.
#[derive(Debug, PartialEq)]
enum Parsed {
    /// More of the file is needed.
    More,
    /// The size, or nil.
    Done(Option<(i64, i64)>),
    Refused(&'static str),
}

/// The stream ran out (FastImage::CannotParseImage once the file ends).
#[derive(Debug)]
enum Stop {
    Short,
    Fail,
    Refused(&'static str),
}

/// FiberStream over the bytes read so far.
struct Stream<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Stream<'a> {
    fn peek(&self, n: usize) -> Result<&'a [u8], Stop> {
        self.bytes.get(self.pos..self.pos + n).ok_or(Stop::Short)
    }

    fn read(&mut self, n: usize) -> Result<&'a [u8], Stop> {
        let s = self.peek(n)?;
        self.pos += n;
        Ok(s)
    }

    fn skip(&mut self, n: usize) -> Result<(), Stop> {
        if self.pos + n > self.bytes.len() {
            return Err(Stop::Short);
        }
        self.pos += n;
        Ok(())
    }

    fn read_byte(&mut self) -> Result<u8, Stop> {
        Ok(self.read(1)?[0])
    }

    /// `read(2).unpack('n')`
    fn read_int(&mut self) -> Result<u16, Stop> {
        let b = self.read(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
}

/// `parse_packets` for the size: the type, then its parser.
fn parse(bytes: &[u8], eof: bool) -> Parsed {
    let mut s = Stream { bytes, pos: 0 };
    match size_of(&mut s) {
        Ok(size) => Parsed::Done(size),
        Err(Stop::Short) if eof => Parsed::Done(None),
        Err(Stop::Short) => Parsed::More,
        Err(Stop::Fail) => Parsed::Done(None),
        Err(Stop::Refused(what)) => Parsed::Refused(what),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Type {
    Bmp,
    Gif,
    Jpeg,
    Png,
    Tiff,
    Psd,
    Webp,
    Ico,
}

/// `TypeParser#type`: None for an unknown type.
fn image_type(s: &Stream<'_>) -> Result<Option<Type>, Stop> {
    let head = s.peek(2)?;
    let svg = Stop::Refused("sizing SVG images from other sites");
    let iso = Stop::Refused("sizing HEIC, AVIF and JPEG XL images from other sites");
    Ok(match head {
        b"BM" => Some(Type::Bmp),
        b"GI" => Some(Type::Gif),
        [0xff, 0xd8] => Some(Type::Jpeg),
        [0x89, b'P'] => Some(Type::Png),
        b"II" | b"MM" => match &s.peek(11)?[8..11] {
            b"APC" | b"CR\x02" => None,
            _ => Some(Type::Tiff),
        },
        b"8B" => Some(Type::Psd),
        [0xff, 0x0a] => return Err(iso),
        [0, 0] => match s.peek(3)?[2] {
            0 => match &s.peek(12)?[4..12] {
                b"ftypavif" | b"ftypavis" | b"ftypheic" | b"ftypmif1" => return Err(iso),
                _ if &s.peek(7)?[4..7] == b"JXL" => return Err(iso),
                _ => None,
            },
            1 | 2 => Some(Type::Ico),
            _ => None,
        },
        b"RI" => (&s.peek(12)?[8..12] == b"WEBP").then_some(Type::Webp),
        b"<s" => {
            if s.peek(4)? == b"<svg" {
                return Err(svg);
            }
            None
        }
        _ => {
            let ws = |b: u8| matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0x0c | 0x0b);
            let maybe_svg = (ws(head[0]) && (ws(head[1]) || head[1] == b'<'))
                || (head[0] == b'<' && matches!(head[1], b'?' | b'!'))
                || head == [0xef, 0xbb];
            if !maybe_svg {
                return Ok(None);
            }
            // Up to a thousand bytes for an <svg before any <html; a file
            // that ends first is unknown.
            for n in 1..=100 {
                let peeked = s.peek(10 * n)?;
                let lower = peeked.to_ascii_lowercase();
                let svg_at = find(peeked, b"<svg");
                let html_at = find(&lower, b"<html");
                if let Some(svg_at) = svg_at
                    && html_at.is_none_or(|h| svg_at < h)
                {
                    return Err(svg);
                }
            }
            None
        }
    })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn u16_le(b: &[u8]) -> i64 {
    i64::from(u16::from_le_bytes([b[0], b[1]]))
}

fn u32_be(b: &[u8]) -> i64 {
    i64::from(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// The size by type (`parse_size`).
fn size_of(s: &mut Stream<'_>) -> Result<Option<(i64, i64)>, Stop> {
    let Some(kind) = image_type(s)? else {
        return Ok(None);
    };
    match kind {
        Type::Png => {
            let d = &s.read(25)?[16..24];
            Ok(Some((u32_be(&d[0..4]), u32_be(&d[4..8]))))
        }
        Type::Gif => {
            let d = &s.read(11)?[6..10];
            Ok(Some((u16_le(&d[0..2]), u16_le(&d[2..4]))))
        }
        Type::Bmp => {
            let d = &s.read(32)?[14..29];
            if d[0] == 12 {
                Ok(Some((u16_le(&d[4..6]), u16_le(&d[6..8]))))
            } else {
                let w = i32::from_le_bytes([d[4], d[5], d[6], d[7]]);
                let h = i32::from_le_bytes([d[8], d[9], d[10], d[11]]);
                Ok(Some((i64::from(w), i64::from(h).abs())))
            }
        }
        Type::Psd => {
            let d = s.read(26)?;
            Ok(Some((u32_be(&d[18..22]), u32_be(&d[14..18]))))
        }
        Type::Ico => {
            let count = u16_le(&s.read(6)?[4..6]);
            let mut sizes = Vec::new();
            for _ in 0..count {
                let d = s.read(16)?;
                let side = |b: u8| if b == 0 { 256 } else { i64::from(b) };
                sizes.push((side(d[0]), side(d[1])));
            }
            sizes.sort_by_key(|(w, h)| w * h);
            match sizes.last() {
                Some(size) => Ok(Some(*size)),
                None => Err(Stop::Fail),
            }
        }
        Type::Webp => webp(s),
        Type::Jpeg => jpeg(s),
        Type::Tiff => {
            let exif = Exif::parse(&mut Main(s))?;
            let (Some(w), Some(h)) = (exif.width, exif.height) else {
                return Err(Stop::Refused("TIFF images without their size in EXIF"));
            };
            Ok(Some(if exif.rotated() { (h, w) } else { (w, h) }))
        }
    }
}

fn webp(s: &mut Stream<'_>) -> Result<Option<(i64, i64)>, Stop> {
    let head = s.read(16)?;
    let vp8 = &head[12..16];
    s.read(4)?;
    match vp8 {
        b"VP8 " => {
            let d = s.read(10)?;
            Ok(Some((
                u16_le(&d[6..8]) & 0x3fff,
                u16_le(&d[8..10]) & 0x3fff,
            )))
        }
        b"VP8L" => {
            s.skip(1)?;
            let b = s.read(4)?;
            let (b1, b2, b3, b4) = (
                i64::from(b[0]),
                i64::from(b[1]),
                i64::from(b[2]),
                i64::from(b[3]),
            );
            Ok(Some((
                1 + (((b2 & 0x3f) << 8) | b1),
                1 + (((b4 & 0xf) << 10) | (b3 << 2) | ((b2 & 0xc0) >> 6)),
            )))
        }
        b"VP8X" => {
            s.read(4)?;
            let b: Vec<i64> = s.read(6)?.iter().map(|b| i64::from(*b)).collect();
            Ok(Some((
                1 + b[0] + (b[1] << 8) + (b[2] << 16),
                1 + b[3] + (b[4] << 8) + (b[5] << 16),
            )))
        }
        _ => Err(Stop::Fail),
    }
}

/// `Jpeg#dimensions`: the frame size, sides swapped when the first EXIF
/// that parses says the image is turned.
fn jpeg(s: &mut Stream<'_>) -> Result<Option<(i64, i64)>, Stop> {
    let mut exif: Option<Exif> = None;
    s.skip(2)?;
    loop {
        // :started
        if s.read_byte()? != 0xff {
            continue;
        }
        // :sof
        let mut marker = s.read_byte()?;
        while marker == 0xff {
            marker = s.read_byte()?;
        }
        match marker {
            0xe1 => {
                let len = i64::from(s.read_int()?) - 2;
                if len < 0 {
                    return Err(Stop::Refused("JPEG images with a broken EXIF segment"));
                }
                let data = s.read(len as usize)?;
                if data.get(..4) == Some(b"Exif".as_slice()) {
                    let mut io = Io {
                        bytes: data,
                        pos: 6,
                    };
                    let parsed = Exif::parse(&mut io).ok();
                    if exif.is_none() {
                        exif = parsed;
                    }
                }
            }
            0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => {
                s.skip(3)?;
                let height = i64::from(s.read_int()?);
                let width = i64::from(s.read_int()?);
                return Ok(Some(if exif.as_ref().is_some_and(Exif::rotated) {
                    (height, width)
                } else {
                    (width, height)
                }));
            }
            _ => {
                // :skipframe
                let len = i64::from(s.read_int()?) - 2;
                if len < 0 {
                    return Err(Stop::Refused("JPEG images with a broken segment length"));
                }
                s.skip(len as usize)?;
            }
        }
    }
}

/// What EXIF reads from: the main stream (a TIFF), or an APP1 segment's
/// StringIO (a JPEG), whose reads past its end come back short or nil.
trait ExifIo {
    /// `read(n)`: None for nil.
    fn read(&mut self, n: usize) -> Result<Option<Vec<u8>>, Stop>;
    /// `skip(n)`, or `read(n)` where there is no skip.
    fn skip(&mut self, n: i64) -> Result<(), Stop>;
}

struct Main<'s, 'a>(&'s mut Stream<'a>);

impl ExifIo for Main<'_, '_> {
    fn read(&mut self, n: usize) -> Result<Option<Vec<u8>>, Stop> {
        Ok(Some(self.0.read(n)?.to_vec()))
    }

    fn skip(&mut self, n: i64) -> Result<(), Stop> {
        if n < 0 {
            return Err(Stop::Refused("TIFF images with a broken EXIF offset"));
        }
        self.0.skip(n as usize)
    }
}

struct Io<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl ExifIo for Io<'_> {
    fn read(&mut self, n: usize) -> Result<Option<Vec<u8>>, Stop> {
        if n > 0 && self.pos >= self.bytes.len() {
            return Ok(None);
        }
        let end = (self.pos + n).min(self.bytes.len());
        let out = self.bytes[self.pos.min(end)..end].to_vec();
        self.pos = end;
        Ok(Some(out))
    }

    fn skip(&mut self, n: i64) -> Result<(), Stop> {
        // StringIO#read of a negative length raises (rescued as nil).
        if n < 0 {
            return Err(Stop::Fail);
        }
        self.read(n as usize).map(|_| ())
    }
}

/// `FastImageParsing::Exif`: the first IFD's width, height and
/// orientation.
struct Exif {
    width: Option<i64>,
    height: Option<i64>,
    orientation: i64,
}

impl Exif {
    fn rotated(&self) -> bool {
        self.orientation >= 5
    }

    /// Fail where Ruby raises (nil unpacked and used, a bad byte order).
    fn parse(io: &mut dyn ExifIo) -> Result<Exif, Stop> {
        let order = io.read(2)?.ok_or(Stop::Fail)?;
        let little = match order.as_slice() {
            b"II" => true,
            b"MM" => false,
            _ => return Err(Stop::Fail),
        };
        // `unpack(@short)[0]` / `unpack(@long)[0]`: nil when short.
        let short = |b: &[u8]| -> Option<i64> {
            let b: [u8; 2] = b.get(..2)?.try_into().ok()?;
            Some(i64::from(if little {
                u16::from_le_bytes(b)
            } else {
                u16::from_be_bytes(b)
            }))
        };
        let long = |b: &[u8]| -> Option<i64> {
            let b: [u8; 4] = b.get(..4)?.try_into().ok()?;
            Some(i64::from(if little {
                u32::from_le_bytes(b)
            } else {
                u32::from_be_bytes(b)
            }))
        };
        io.read(2)?;
        let offset = long(&io.read(4)?.ok_or(Stop::Fail)?).ok_or(Stop::Fail)?;
        io.skip(offset - 8)?;
        let mut width = None;
        let mut height = None;
        let mut orientation = None;
        let tag_count = short(&io.read(2)?.ok_or(Stop::Fail)?).ok_or(Stop::Fail)?;
        for _ in 0..tag_count {
            let tag = short(&io.read(2)?.ok_or(Stop::Fail)?);
            let data_type = short(&io.read(2)?.ok_or(Stop::Fail)?);
            io.read(4)?;
            let data = if data_type == Some(4) {
                long(&io.read(4)?.ok_or(Stop::Fail)?)
            } else {
                let d = short(&io.read(2)?.ok_or(Stop::Fail)?);
                io.read(2)?;
                d
            };
            match tag {
                Some(0x0100) => width = data,
                Some(0x0101) => height = data,
                Some(0x0112) => orientation = data,
                _ => {}
            }
            if width.is_some() && height.is_some() && orientation.is_some() {
                break;
            }
        }
        Ok(Exif {
            width,
            height,
            orientation: orientation.unwrap_or(1),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(bytes: &[u8]) -> Parsed {
        parse(bytes, true)
    }

    #[test]
    fn sizes_the_common_formats() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&[0, 0, 1, 0x2c, 0, 0, 0, 0xc8, 8]);
        assert_eq!(size(&png), Parsed::Done(Some((300, 200))));
        assert_eq!(parse(&png[..20], false), Parsed::More);
        assert_eq!(parse(&png[..20], true), Parsed::Done(None));
        let gif = b"GIF89a\x40\x01\xf0\x00\x00";
        assert_eq!(size(gif), Parsed::Done(Some((320, 240))));
        assert_eq!(size(b"not an image at all"), Parsed::Done(None));
        assert!(matches!(size(b"<svg xmlns='x'></svg>"), Parsed::Refused(_)));
        assert!(matches!(size(b"  <?xml?><svg></svg>"), Parsed::Refused(_)));
    }

    #[test]
    fn sizes_jpegs_as_displayed() {
        // SOI, an APP1 with EXIF orientation 6 (big endian), SOF0 100x50.
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1];
        let mut exif = b"Exif\0\0MM\0\x2a\0\0\0\x08".to_vec();
        exif.extend_from_slice(&[0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0]);
        let len = (exif.len() + 2) as u16;
        jpeg.extend_from_slice(&len.to_be_bytes());
        jpeg.extend_from_slice(&exif);
        jpeg.extend_from_slice(&[0xff, 0xc0, 0, 17, 8, 0, 50, 0, 100]);
        assert_eq!(size(&jpeg), Parsed::Done(Some((50, 100))));
    }

    #[test]
    fn escapes_locations_as_ruby() {
        assert_eq!(escaped_location("/a b.png"), "/a%20b.png");
        assert_eq!(escaped_location("/ok.png"), "/ok.png");
    }
}
