//! An incoming email as Email::Receiver reads it through the mail gem: the
//! sender, recipients, subject, ids and date, the text and HTML parts
//! (`fix_charset`), and `Email::Cleaner#execute`, the raw message the gem
//! re-serializes into incoming_emails.raw.
//!
//! mail-parser decodes the header values (addresses, encoded words,
//! dates). The structure (header fields, multipart bodies, transfer
//! encodings) is read here, because the cleaner has to reproduce the
//! gem's own output byte for byte: fields in its FIELD_ORDER, re-folded
//! and re-encoded per field type, bodies re-encoded with the transfer
//! encoding its cost rules pick. Shapes the gem's output was not measured
//! for are refused.

use base64::Engine;
use mail_parser::{HeaderValue, MessageParser};

use crate::Unsupported;

/// One header field: the name as written and the unfolded raw value.
#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub value: String,
}

/// A MIME entity: its fields and raw body, or its parts.
#[derive(Debug, Clone)]
pub struct Entity {
    pub fields: Vec<Field>,
    pub body: Vec<u8>,
    pub parts: Vec<Entity>,
    pub boundary: Option<String>,
}

impl Entity {
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case(name))
            .map(|f| f.value.as_str())
    }

    /// `mime_type`, lowercased; text/plain when absent.
    pub fn mime_type(&self) -> String {
        self.field("Content-Type")
            .map(|v| {
                v.split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
            })
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "text/plain".into())
    }

    pub fn has_content_type(&self) -> bool {
        self.field("Content-Type").is_some()
    }

    fn params(&self, field: &str) -> Vec<(String, String)> {
        self.field(field).map(parse_params).unwrap_or_default()
    }

    pub fn charset(&self) -> Option<String> {
        self.params("Content-Type")
            .into_iter()
            .find(|(k, _)| k == "charset")
            .map(|(_, v)| v)
    }

    pub fn multipart(&self) -> bool {
        self.mime_type().starts_with("multipart/")
    }

    /// `attachment?`: a filename or name parameter.
    pub fn attachment(&self) -> bool {
        let named = |field: &str, key: &str| self.params(field).iter().any(|(k, _)| k == key);
        named("Content-Disposition", "filename")
            || named("Content-Type", "name")
            || self
                .field("Content-Disposition")
                .is_some_and(|d| d.trim().to_ascii_lowercase().starts_with("attachment"))
    }

    fn transfer_encoding(&self) -> String {
        self.field("Content-Transfer-Encoding")
            .map(|v| v.trim().to_ascii_lowercase())
            .unwrap_or_else(|| "7bit".into())
    }

    /// `body.decoded`: the transfer encoding undone, as the gem leaves it
    /// (7bit and 8bit bodies with LF line endings, base64 as decoded).
    pub fn decoded(&self) -> Result<Vec<u8>, Unsupported> {
        Ok(match self.transfer_encoding().as_str() {
            // SevenBit.decode is binary_unsafe_to_lf; 8bit and binary are
            // the identity, and quoted-printable keeps its line breaks.
            "7bit" | "" => to_lf(&self.body),
            "8bit" | "binary" => self.body.clone(),
            // (an ASCII result has its line breaks made LF, as the gem's
            // to_lf only converts it then)
            "quoted-printable" => {
                let decoded = qp_decode(&self.body);
                if decoded.is_ascii() {
                    to_lf(&decoded)
                } else {
                    decoded
                }
            }
            "base64" => {
                let compact: Vec<u8> = self
                    .body
                    .iter()
                    .copied()
                    .filter(|b| !b.is_ascii_whitespace())
                    .collect();
                base64::engine::general_purpose::STANDARD
                    .decode(compact)
                    .map_err(|_| Unsupported("an undecodable base64 email body"))?
            }
            _ => return Err(Unsupported("an email body in an unknown transfer encoding")),
        })
    }

    /// `find_first_mime_type`: this part's children first, then theirs.
    fn find_first(&self, mime_type: &str) -> Option<&Entity> {
        self.parts
            .iter()
            .find(|p| p.mime_type() == mime_type && !p.attachment())
            .or_else(|| self.parts.iter().find_map(|p| p.find_first(mime_type)))
    }

    fn all_attachments<'a>(&'a self, out: &mut Vec<&'a Entity>) {
        for part in &self.parts {
            if part.multipart() {
                part.all_attachments(out);
            } else if part.attachment() {
                out.push(part);
            }
        }
    }
}

/// `binary_unsafe_to_lf`: CRLF and lone CR to LF.
fn to_lf(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        if body[i] == b'\r' {
            out.push(b'\n');
            i += if body.get(i + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
            continue;
        }
        out.push(body[i]);
        i += 1;
    }
    out
}

/// Quoted-printable decoding (`unpack1("M*")`).
fn qp_decode(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    let hex = |b: u8| (b as char).to_digit(16);
    while i < body.len() {
        if body[i] == b'=' {
            // soft line break
            if body.get(i + 1) == Some(&b'\r') && body.get(i + 2) == Some(&b'\n') {
                i += 3;
                continue;
            }
            if body.get(i + 1) == Some(&b'\n') {
                i += 2;
                continue;
            }
            if let (Some(h), Some(l)) = (
                body.get(i + 1).and_then(|b| hex(*b)),
                body.get(i + 2).and_then(|b| hex(*b)),
            ) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(body[i]);
        i += 1;
    }
    out
}

/// `key=value; key="value"` parameters, keys lowercased.
fn parse_params(value: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = value.split_once(';').map_or("", |(_, r)| r);
    while !rest.trim().is_empty() {
        let trimmed = rest.trim_start_matches([' ', '\t', ';']);
        let Some((key, after)) = trimmed.split_once('=') else {
            break;
        };
        let after = after.trim_start();
        let (val, remaining) = if let Some(q) = after.strip_prefix('"') {
            let mut val = String::new();
            let mut chars = q.char_indices();
            let mut end = q.len();
            while let Some((i, c)) = chars.next() {
                match c {
                    '\\' => {
                        if let Some((_, n)) = chars.next() {
                            val.push(n);
                        }
                    }
                    '"' => {
                        end = i + 1;
                        break;
                    }
                    c => val.push(c),
                }
            }
            (val, &q[end..])
        } else {
            let end = after.find(';').unwrap_or(after.len());
            (after[..end].trim().to_string(), &after[end..])
        };
        out.push((key.trim().to_ascii_lowercase(), val));
        rest = remaining;
    }
    out
}

/// Splits a raw entity into its unfolded fields and its body.
fn parse_entity(raw: &[u8]) -> Result<Entity, Unsupported> {
    let (head, body) = split_head(raw);
    let head = std::str::from_utf8(head).map_err(|_| Unsupported("non-UTF-8 email headers"))?;
    let mut fields: Vec<Field> = Vec::new();
    for line in head.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            let last = fields
                .last_mut()
                .ok_or(Unsupported("an email starting with a folded line"))?;
            last.value.push_str(line);
            continue;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or(Unsupported("an email header line without a colon"))?;
        fields.push(Field {
            name: name.to_string(),
            value: value.trim_start().to_string(),
        });
    }
    for f in &mut fields {
        f.value = f.value.trim_end().to_string();
    }
    let mut entity = Entity {
        fields,
        body: body.to_vec(),
        parts: Vec::new(),
        boundary: None,
    };
    if entity.multipart() {
        let boundary = entity
            .params("Content-Type")
            .into_iter()
            .find(|(k, _)| k == "boundary")
            .map(|(_, v)| v)
            .ok_or(Unsupported("a multipart email without a boundary"))?;
        entity.parts = split_parts(&entity.body, &boundary)?
            .iter()
            .map(|p| parse_entity(p))
            .collect::<Result<_, _>>()?;
        entity.boundary = Some(boundary);
    }
    Ok(entity)
}

/// The header block and the body (after the first empty line).
fn split_head(raw: &[u8]) -> (&[u8], &[u8]) {
    let mut i = 0;
    let mut line_start = 0;
    while i < raw.len() {
        if raw[i] == b'\n' {
            let line = &raw[line_start..i];
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.is_empty() {
                return (&raw[..line_start], &raw[i + 1..]);
            }
            line_start = i + 1;
        }
        i += 1;
    }
    (raw, &[])
}

/// The parts between `--boundary` lines, each without the line break that
/// belongs to the delimiter after it. The preamble and epilogue must be
/// empty.
fn split_parts(body: &[u8], boundary: &str) -> Result<Vec<Vec<u8>>, Unsupported> {
    let open = format!("--{boundary}");
    let close = format!("--{boundary}--");
    let mut parts: Vec<Vec<u8>> = Vec::new();
    let mut current: Option<Vec<u8>> = None;
    let mut preamble = Vec::new();
    let mut closed = false;
    for line in body.split_inclusive(|b| *b == b'\n') {
        let bare = line
            .strip_suffix(b"\n")
            .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
            .unwrap_or(line);
        let text = String::from_utf8_lossy(bare);
        let text = text.trim_end();
        if closed {
            if !text.is_empty() {
                return Err(Unsupported("a multipart email with an epilogue"));
            }
            continue;
        }
        if text == close || text == open {
            if let Some(mut part) = current.take() {
                // the line break before the delimiter is the delimiter's
                if part.ends_with(b"\r\n") {
                    part.truncate(part.len() - 2);
                } else if part.ends_with(b"\n") {
                    part.truncate(part.len() - 1);
                }
                parts.push(part);
            }
            if text == close {
                closed = true;
            } else {
                current = Some(Vec::new());
            }
            continue;
        }
        match &mut current {
            Some(part) => part.extend_from_slice(line),
            None => preamble.extend_from_slice(line),
        }
    }
    if !closed {
        return Err(Unsupported(
            "a multipart email without its closing boundary",
        ));
    }
    if preamble.iter().any(|b| !b.is_ascii_whitespace()) {
        return Err(Unsupported("a multipart email with a preamble"));
    }
    Ok(parts)
}

/// What Email::Receiver reads from a message.
#[derive(Debug)]
pub struct Incoming {
    pub raw: String,
    pub root: Entity,
    /// `@message_id`: the Message-ID without brackets, or the raw's MD5.
    pub message_id: String,
    /// `parse_from_field`: the address (lowercased) and display name.
    pub from: Option<(String, String)>,
    /// `mail.from`: the From addresses as written.
    pub from_addresses: Vec<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    /// `mail.subject`, decoded; None when absent.
    pub subject: Option<String>,
    /// `mail.date` in UTC.
    pub date: Option<chrono::NaiveDateTime>,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
    pub delivered_to: Vec<String>,
    pub x_forwarded_to: Vec<String>,
}

fn addresses(value: Option<&mail_parser::Address>) -> Vec<String> {
    match value {
        Some(mail_parser::Address::List(list)) => list
            .iter()
            .filter_map(|a| a.address.as_ref().map(|s| s.to_string()))
            .collect(),
        Some(mail_parser::Address::Group(groups)) => groups
            .iter()
            .flat_map(|g| g.addresses.iter())
            .filter_map(|a| a.address.as_ref().map(|s| s.to_string()))
            .collect(),
        None => Vec::new(),
    }
}

fn ids(value: &HeaderValue) -> Vec<String> {
    match value {
        HeaderValue::Text(t) => vec![t.to_string()],
        HeaderValue::TextList(list) => list.iter().map(|t| t.to_string()).collect(),
        _ => Vec::new(),
    }
}

/// `Mail.new(raw)` and what the receiver reads from it.
pub fn parse(raw: &str) -> Result<Incoming, Unsupported> {
    let root = parse_entity(raw.as_bytes())?;
    let parsed = MessageParser::default()
        .parse(raw.as_bytes())
        .ok_or(Unsupported("an email mail-parser cannot read"))?;
    for name in [
        "X-Mailman-Version",
        "X-Original-From",
        "Resent-From",
        "Resent-To",
    ] {
        if root.field(name).is_some() {
            return Err(Unsupported("mailing list and resent emails"));
        }
    }
    let message_id = match parsed.message_id() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => format!("{:x}", md5::compute(raw.as_bytes())),
    };
    let from = match parsed.from() {
        Some(mail_parser::Address::List(list)) if list.len() == 1 => {
            let a = &list[0];
            a.address
                .as_ref()
                .filter(|a| a.contains('@'))
                .map(|address| {
                    let mut name = a.name.as_deref().unwrap_or("").trim().to_string();
                    let quoted =
                        |q: char| name.len() > 2 && name.starts_with(q) && name.ends_with(q);
                    if quoted('"') || quoted('\'') {
                        name = name[1..name.len() - 1].to_string();
                    }
                    (address.trim().to_lowercase(), name)
                })
        }
        Some(_) => return Err(Unsupported("an email from several senders")),
        None => None,
    };
    let date = parsed.date().and_then(|d| {
        let naive = chrono::NaiveDate::from_ymd_opt(d.year as i32, d.month as u32, d.day as u32)?
            .and_hms_opt(d.hour as u32, d.minute as u32, d.second as u32)?;
        let offset = (d.tz_hour as i64 * 60 + d.tz_minute as i64) * 60;
        let offset = if d.tz_before_gmt { -offset } else { offset };
        Some(naive - chrono::Duration::seconds(offset))
    });
    let decoded_values = |name: &str| -> Vec<String> {
        root.fields
            .iter()
            .filter(|f| f.name.eq_ignore_ascii_case(name))
            .map(|f| f.value.trim().to_string())
            .collect()
    };
    Ok(Incoming {
        raw: raw.to_string(),
        message_id,
        from,
        from_addresses: addresses(parsed.from()),
        to: addresses(parsed.to()),
        cc: addresses(parsed.cc()),
        bcc: addresses(parsed.bcc()),
        subject: parsed.subject().map(str::to_string),
        date,
        in_reply_to: ids(parsed.in_reply_to()),
        references: ids(parsed.references()),
        delivered_to: decoded_values("Delivered-To"),
        x_forwarded_to: decoded_values("X-Forwarded-To"),
        root,
    })
}

impl Incoming {
    /// `mail.text_part` (or the message itself when it is plain text).
    pub fn text_part(&self) -> Option<&Entity> {
        if self.root.multipart() {
            self.root.find_first("text/plain")
        } else if self.root.mime_type().contains("text/html") {
            None
        } else if !self.root.has_content_type() || self.root.mime_type().contains("text/plain") {
            Some(&self.root)
        } else {
            None
        }
    }

    /// `mail.html_part` (or the message itself when it is HTML).
    pub fn html_part(&self) -> Option<&Entity> {
        if self.root.multipart() {
            self.root.find_first("text/html")
        } else if self.root.mime_type().contains("text/html") {
            Some(&self.root)
        } else {
            None
        }
    }

    /// `fix_charset(part)`: the decoded body as UTF-8.
    pub fn fix_charset(part: Option<&Entity>) -> Result<Option<String>, Unsupported> {
        let Some(part) = part else {
            return Ok(None);
        };
        let decoded = part.decoded()?;
        if decoded.is_empty() {
            return Ok(None);
        }
        let charset = part.charset().map(|c| c.to_ascii_lowercase());
        if !matches!(
            charset.as_deref(),
            None | Some("utf-8") | Some("utf8") | Some("us-ascii")
        ) {
            return Err(Unsupported("email bodies in a charset other than UTF-8"));
        }
        let text = String::from_utf8(decoded)
            .map_err(|_| Unsupported("an email body that is not valid UTF-8"))?;
        Ok((!text.trim().is_empty() || !text.is_empty()).then_some(text))
    }

    /// `mail.destinations` (to, cc, bcc) then X-Forwarded-To and
    /// Delivered-To, the blank and repeated ones out.
    pub fn all_destinations(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for d in self
            .to
            .iter()
            .chain(&self.cc)
            .chain(&self.bcc)
            .chain(&self.x_forwarded_to)
            .chain(&self.delivered_to)
        {
            if !d.trim().is_empty() && !out.contains(d) {
                out.push(d.clone());
            }
        }
        out
    }

    /// `mail.attachments` and the message's own attachment parts.
    pub fn attachment_count(&self) -> usize {
        let mut found = Vec::new();
        self.root.all_attachments(&mut found);
        found.len() + usize::from(!self.root.multipart() && self.root.attachment())
    }

    /// The header block, as `@mail.header.to_s` reads for the
    /// auto-generated check (one `Name: value` line per field).
    pub fn header_text(&self, except: &str) -> String {
        self.root
            .fields
            .iter()
            .filter(|f| !f.name.eq_ignore_ascii_case(except))
            .map(|f| format!("{}: {}\r\n", f.name, f.value))
            .collect()
    }
}

// --- Email::Cleaner -------------------------------------------------------

/// `Email::Cleaner.new(raw).execute` with attachments removed and bodies
/// within `limit` characters.
pub fn clean(incoming: &Incoming, limit: usize) -> Result<String, Unsupported> {
    let mut out = String::new();
    render_entity(&incoming.root, limit, true, &mut out)?;
    Ok(out.replace('\0', ""))
}

fn render_entity(e: &Entity, limit: usize, top: bool, out: &mut String) -> Result<(), Unsupported> {
    let multipart = e.multipart();
    let (body, encoding) = if multipart {
        (None, "7bit".to_string())
    } else {
        let decoded = e.decoded()?;
        let text = String::from_utf8(decoded)
            .map_err(|_| Unsupported("cleaning a body that is not UTF-8"))?;
        if text.chars().count() > limit {
            return Err(Unsupported("truncating a long incoming email"));
        }
        // 7bit and 8bit bodies decode to a string still in the message's
        // UTF-8; quoted-printable and base64 ones to binary.
        let binary = matches!(
            e.transfer_encoding().as_str(),
            "quoted-printable" | "base64"
        );
        let (encoded, encoding) = encode_body(text.as_bytes(), binary);
        (Some(encoded), encoding)
    };
    // The fields, with the charset set to UTF-8 and the transfer encoding
    // the body takes, in the gem's order.
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut saw_cte = false;
    for f in &e.fields {
        let lower = f.name.to_ascii_lowercase();
        let rendered = match lower.as_str() {
            "content-type" => content_type(
                &f.value,
                multipart || top || e.mime_type().starts_with("text/"),
            )?,
            "content-transfer-encoding" => {
                saw_cte = true;
                format!("Content-Transfer-Encoding: {encoding}\r\n")
            }
            _ => render_field(f)?,
        };
        fields.push((lower, rendered));
    }
    if !e.has_content_type() {
        return Err(Unsupported("cleaning an email without a Content-Type"));
    }
    if !saw_cte {
        fields.push((
            "content-transfer-encoding".into(),
            format!("Content-Transfer-Encoding: {encoding}\r\n"),
        ));
    }
    if top {
        for required in ["date", "message-id", "mime-version"] {
            if !fields.iter().any(|(n, _)| n == required) {
                return Err(Unsupported("cleaning an email the gem would add fields to"));
            }
        }
    }
    let order = |name: &str| {
        super::sender::FIELD_ORDER
            .iter()
            .position(|f| *f == name)
            .unwrap_or(100)
    };
    let mut indexed: Vec<(usize, usize, String)> = fields
        .into_iter()
        .enumerate()
        .map(|(i, (name, text))| (order(&name), i, text))
        .collect();
    indexed.sort_by_key(|(o, i, _)| (*o, *i));
    for (_, _, text) in indexed {
        out.push_str(&text);
    }
    out.push_str("\r\n");
    match body {
        Some(body) => out.push_str(&body),
        None => {
            let boundary = e.boundary.as_deref().expect("a multipart boundary");
            for part in &e.parts {
                if part.attachment() && !part.multipart() {
                    continue;
                }
                out.push_str(&format!("\r\n--{boundary}\r\n"));
                render_entity(part, limit, false, out)?;
            }
            out.push_str(&format!("\r\n--{boundary}--\r\n"));
        }
    }
    Ok(())
}

/// The body in the transfer encoding the gem negotiates for a 7bit
/// message: 7bit when it can, else the cheaper of quoted-printable and
/// base64 (quoted-printable on a tie). `binary`: the decoded body is a
/// binary string to Ruby.
fn encode_body(decoded: &[u8], binary: bool) -> (String, String) {
    let ascii = decoded.is_ascii();
    let long_line = decoded.split(|b| *b == b'\n').any(|line| line.len() > 998);
    if ascii && !long_line {
        // SevenBit.encode: binary_unsafe_to_crlf
        return (
            String::from_utf8(binary_unsafe_to_crlf(decoded)).expect("ASCII"),
            "7bit".into(),
        );
    }
    let cheap = decoded
        .iter()
        .filter(|&&b| matches!(b, b'\t' | b'\n' | b'\r' | 0x20..=0x3C | 0x3E..=0x7E))
        .count();
    let qp_cost = ((decoded.len() - cheap) * 3 + cheap) as f64 / decoded.len().max(1) as f64;
    if qp_cost <= 4.0 / 3.0 {
        // QuotedPrintable.encode: to_crlf([to_lf(str)].pack("M")). The
        // gem's to_lf leaves a binary, non-ASCII string alone, whose CRs
        // become "=0D"; a text one has its line breaks made LF.
        let lf = if binary && !ascii {
            decoded.to_vec()
        } else {
            to_lf(decoded)
        };
        (
            String::from_utf8(binary_unsafe_to_crlf(qp_encode(&lf).as_bytes())).expect("ASCII"),
            "quoted-printable".into(),
        )
    } else {
        let encoded = base64::engine::general_purpose::STANDARD.encode(decoded);
        // Ruby's pack("m"): 60-character lines, each ending in a newline.
        let mut out = String::new();
        for chunk in encoded.as_bytes().chunks(60) {
            out.push_str(std::str::from_utf8(chunk).expect("base64"));
            out.push_str("\r\n");
        }
        (out, "base64".into())
    }
}

/// `binary_unsafe_to_crlf`: every line break as CRLF.
fn binary_unsafe_to_crlf(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() + 16);
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'\r' if s.get(i + 1) == Some(&b'\n') => {
                out.extend_from_slice(b"\r\n");
                i += 2;
                continue;
            }
            b'\r' | b'\n' => out.extend_from_slice(b"\r\n"),
            b => out.push(b),
        }
        i += 1;
    }
    out
}

/// Ruby's `[str].pack("M")` (pack.c qpencode, line length 72): `=`,
/// controls other than tab and newline, and 8-bit bytes as `=XX`; a soft
/// break once a line passes 72 characters, before a newline that follows
/// a space or tab, and at the end of a text not ending in a newline.
fn qp_encode(text: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::new();
    let mut n = 0usize;
    let mut prev: Option<u8> = None;
    for &b in text {
        if b > 126 || (b < 32 && b != b'\n' && b != b'\t') || b == b'=' {
            out.push('=');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0f) as usize] as char);
            n += 3;
            prev = None;
        } else if b == b'\n' {
            if matches!(prev, Some(b' ') | Some(b'\t')) {
                out.push_str("=\n");
            }
            out.push('\n');
            n = 0;
            prev = Some(b);
        } else {
            out.push(b as char);
            n += 1;
            prev = Some(b);
        }
        if n > 72 {
            out.push_str("=\n");
            n = 0;
        }
    }
    if n > 0 {
        out.push_str("=\n");
    }
    out
}

const PHRASE_UNSAFE: &str = "()<>[]:;@\\,.\"";

/// A field as the gem's field class for it encodes it.
fn render_field(f: &Field) -> Result<String, Unsupported> {
    let name = canonical_name(&f.name);
    match name.to_ascii_lowercase().as_str() {
        "date" => {
            let parsed = mail_parser::MessageParser::default()
                .parse(format!("Date: {}\r\n\r\n", f.value).as_bytes())
                .and_then(|m| m.date().cloned())
                .ok_or(Unsupported("an unparseable Date"))?;
            Ok(format!("Date: {}\r\n", rfc2822(&parsed)))
        }
        "from" | "to" | "cc" | "bcc" | "reply-to" | "sender" => {
            let source = format!("To: {}\r\n\r\n", f.value);
            let parsed = mail_parser::MessageParser::default()
                .parse(source.as_bytes())
                .ok_or(Unsupported("an unparseable address field"))?;
            let list = match parsed.to() {
                Some(mail_parser::Address::List(list)) => list.clone(),
                _ => return Err(Unsupported("address groups in an email")),
            };
            let rendered: Vec<String> = list
                .iter()
                .map(|a| {
                    let address = a.address.as_deref().unwrap_or("");
                    match a.name.as_deref().filter(|n| !n.is_empty()) {
                        Some(n) if !n.is_ascii() => format!(
                            "=?UTF-8?B?{}?= <{address}>",
                            base64::engine::general_purpose::STANDARD.encode(n)
                        ),
                        Some(n) if n.chars().any(|c| PHRASE_UNSAFE.contains(c)) => {
                            format!(
                                "\"{}\" <{address}>",
                                n.replace('\\', "\\\\").replace('"', "\\\"")
                            )
                        }
                        Some(n) => format!("{n} <{address}>"),
                        None => address.to_string(),
                    }
                })
                .collect();
            Ok(format!("{name}: {}\r\n", rendered.join(", \r\n ")))
        }
        "message-id" | "in-reply-to" | "references" => {
            let ids: Vec<String> = f
                .value
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|s| !s.is_empty())
                .map(|id| format!("<{}>", id.trim_matches(['<', '>'])))
                .collect();
            Ok(format!("{name}: {}\r\n", ids.join("\r\n ")))
        }
        "mime-version" => Ok(format!("MIME-Version: {}\r\n", f.value.trim())),
        "received"
        | "return-path"
        | "content-id"
        | "content-location"
        | "content-disposition"
        | "content-description"
        | "comments"
        | "keywords" => Err(Unsupported("cleaning this email field")),
        _ => {
            let decoded = if f.value.contains("=?") {
                decode_words(&f.value)?
            } else {
                f.value.clone()
            };
            Ok(unstructured(&name, &decoded))
        }
    }
}

/// The gem's capitalization of the fields it knows.
fn canonical_name(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "message-id" => "Message-ID".into(),
        "mime-version" => "MIME-Version".into(),
        "in-reply-to" => "In-Reply-To".into(),
        "reply-to" => "Reply-To".into(),
        "date" => "Date".into(),
        "from" => "From".into(),
        "to" => "To".into(),
        "cc" => "Cc".into(),
        "bcc" => "Bcc".into(),
        "subject" => "Subject".into(),
        "references" => "References".into(),
        "sender" => "Sender".into(),
        _ => name.to_string(),
    }
}

fn decode_words(value: &str) -> Result<String, Unsupported> {
    let source = format!("Subject: {value}\r\n\r\n");
    let parsed = mail_parser::MessageParser::default()
        .parse(source.as_bytes())
        .ok_or(Unsupported("an undecodable header"))?;
    Ok(parsed.subject().unwrap_or("").to_string())
}

/// `%a, %d %b %Y %H:%M:%S %z` in the date's own offset.
fn rfc2822(d: &mail_parser::DateTime) -> String {
    let days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let weekday = chrono::NaiveDate::from_ymd_opt(d.year as i32, d.month as u32, d.day as u32)
        .map(|n| chrono::Datelike::weekday(&n).num_days_from_monday() as usize)
        .unwrap_or(0);
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} {}{:02}{:02}",
        days[weekday],
        d.day,
        months[(d.month.max(1) - 1) as usize],
        d.year,
        d.hour,
        d.minute,
        d.second,
        if d.tz_before_gmt { '-' } else { '+' },
        d.tz_hour,
        d.tz_minute
    )
}

/// `ContentTypeField#encoded` after `charset = "UTF-8"`: the type, then
/// each parameter (sorted) on a folded line.
fn content_type(value: &str, set_charset: bool) -> Result<String, Unsupported> {
    let mime = value
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let mut params = parse_params(value);
    if set_charset {
        match params.iter_mut().find(|(k, _)| k == "charset") {
            Some(p) => p.1 = "UTF-8".into(),
            None => params.push(("charset".into(), "UTF-8".into())),
        }
    }
    params.sort_by(|a, b| a.0.cmp(&b.0));
    let token = |v: &str| {
        !v.is_empty()
            && v.chars()
                .all(|c| c.is_ascii_graphic() && !"()<>@,;:\\\"/[]?=".contains(c))
    };
    let mut out = format!("Content-Type: {mime}");
    for (k, v) in params {
        if token(&v) {
            out.push_str(&format!(";\r\n {k}={v}"));
        } else {
            out.push_str(&format!(";\r\n {k}=\"{}\"", v.replace('"', "\\\"")));
        }
    }
    out.push_str("\r\n");
    Ok(out)
}

/// `UnstructuredField#fold`, `wrap_lines` and the encoded words it makes
/// of non-ASCII values (UTF-8, Q encoding).
fn unstructured(name: &str, value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    let should_encode = !value.is_ascii();
    let mut words: Vec<String> = Vec::new();
    if should_encode {
        for (i, word) in value.split([' ', '\t']).enumerate() {
            let word = if i == 0 {
                word.to_string()
            } else {
                format!(" {word}")
            };
            if word.is_ascii() {
                // word.scan(/.{7}|.+$/)
                let chars: Vec<char> = word.chars().collect();
                for chunk in chars.chunks(7) {
                    words.push(chunk.iter().collect());
                }
            } else {
                words.push(word);
            }
        }
    } else {
        words = value.split([' ', '\t']).map(str::to_string).collect();
        // Ruby's split drops trailing empty words
        while words.last().is_some_and(|w| w.is_empty()) {
            words.pop();
        }
    }
    let mut prepend = name.len() + 2;
    let mut lines: Vec<String> = Vec::new();
    let mut queue: std::collections::VecDeque<String> = words.into();
    while !queue.is_empty() {
        let mut limit = 78usize.saturating_sub(prepend);
        if should_encode {
            limit = limit.saturating_sub(7 + "UTF-8".len());
        }
        let mut line = String::new();
        let mut first = true;
        while let Some(word) = queue.front() {
            let word = if should_encode {
                q_encode(word)
            } else {
                word.clone()
            };
            let word = word.replace('\r', "=0D").replace('\n', "=0A");
            if !line.is_empty() && line.len() + word.len() + 1 > limit {
                break;
            }
            queue.pop_front();
            if first {
                first = false;
            } else if !should_encode {
                line.push(' ');
            }
            line.push_str(&word);
        }
        if should_encode {
            line = format!("=?UTF-8?Q?{line}?=");
        }
        lines.push(line);
        prepend = 0;
    }
    format!("{name}: {}\r\n", lines.join("\r\n "))
}

/// `UnstructuredField#encode`: `[value].pack("M")` without soft breaks,
/// then the characters encoded words may not hold.
fn q_encode(word: &str) -> String {
    let mut out = String::new();
    for &b in word.as_bytes() {
        let printable = (b == b'\t' || (0x20..=0x7E).contains(&b)) && b != b'=';
        if printable {
            out.push(b as char);
        } else {
            out.push_str(&format!("={b:02X}"));
        }
    }
    out.replace('"', "=22")
        .replace('(', "=28")
        .replace(')', "=29")
        .replace('?', "=3F")
        .replace('_', "=5F")
        .replace(' ', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_parse_quoted_and_bare() {
        assert_eq!(
            parse_params("multipart/alternative; boundary=\"a b\"; charset=utf-8"),
            vec![
                ("boundary".to_string(), "a b".to_string()),
                ("charset".to_string(), "utf-8".to_string())
            ]
        );
    }

    #[test]
    fn subjects_fold_at_78() {
        let folded = unstructured("Subject", "word ".repeat(20).trim_end());
        assert!(folded.lines().all(|l| l.trim_end_matches('\r').len() <= 78));
    }
}
