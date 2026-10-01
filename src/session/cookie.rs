//! Rails 8.1 encrypted cookies as Discourse configures them: the value is
//! MessagePack (`cookies_serializer = :message_pack`), wrapped in the
//! `{"_rails":{"message","exp","pur"}}` envelope, encrypted with
//! AES-256-GCM under a key derived from `secret_key_base` by PBKDF2-HMAC-SHA1
//! (`key_generator_hash_digest_class = SHA1`), and sent as
//! `b64(ciphertext)--b64(iv)--b64(tag)`, form-encoded.
//!
//! Reading and writing with the same `secret_key_base` as the Rails
//! instance means a Rails session is valid here and the other way round.

use std::collections::BTreeMap;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chrono::{DateTime, Utc};
use rand::RngCore;

/// A cookie's decoded content: a flat map of strings and integers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scalar {
    Str(String),
    Int(i64),
    Bool(bool),
    Nil,
}

impl Scalar {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Scalar::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Scalar::Int(i) => Some(*i),
            _ => None,
        }
    }
}

pub type Map = BTreeMap<String, Scalar>;

/// The AES key `ActiveSupport::KeyGenerator` derives for encrypted cookies.
pub fn derive_key(secret_key_base: &str) -> [u8; 32] {
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(
        secret_key_base.as_bytes(),
        b"authenticated encrypted cookie",
        1000,
        &mut key,
    );
    key
}

pub struct Codec {
    key: [u8; 32],
}

#[derive(Debug)]
pub enum CookieError {
    Malformed,
    Decrypt,
    Expired,
    Purpose,
    Payload,
}

impl std::fmt::Display for CookieError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CookieError::Malformed => write!(f, "malformed cookie"),
            CookieError::Decrypt => write!(f, "cookie does not decrypt"),
            CookieError::Expired => write!(f, "cookie expired"),
            CookieError::Purpose => write!(f, "cookie purpose mismatch"),
            CookieError::Payload => write!(f, "cookie payload not understood"),
        }
    }
}

impl std::error::Error for CookieError {}

/// `Rack::Utils.escape`: form encoding of the Set-Cookie value.
pub fn form_escape(s: &str) -> String {
    form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// `Rack::Utils.unescape`
pub fn form_unescape(s: &str) -> String {
    form_urlencoded::parse(format!("v={s}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

impl Codec {
    pub fn new(secret_key_base: &str) -> Codec {
        Codec {
            key: derive_key(secret_key_base),
        }
    }

    /// Decrypts a cookie named `name`; `None` when it is not one of ours
    /// (Rails treats every failure as "no cookie").
    pub fn decrypt(&self, name: &str, raw: &str) -> Result<Map, CookieError> {
        let value = form_unescape(raw);
        // The tag and iv are fixed-length base64 at the end: 24 + 2 + 16.
        let (rest, tag_b64) = value.rsplit_once("--").ok_or(CookieError::Malformed)?;
        let (ct_b64, iv_b64) = rest.rsplit_once("--").ok_or(CookieError::Malformed)?;
        let ciphertext = B64.decode(ct_b64).map_err(|_| CookieError::Malformed)?;
        let iv = B64.decode(iv_b64).map_err(|_| CookieError::Malformed)?;
        let tag = B64.decode(tag_b64).map_err(|_| CookieError::Malformed)?;
        if iv.len() != 12 || tag.len() != 16 {
            return Err(CookieError::Malformed);
        }
        let mut sealed = ciphertext;
        sealed.extend_from_slice(&tag);
        let cipher = Aes256Gcm::new(&self.key.into());
        let plain = cipher
            .decrypt(
                Nonce::from_slice(&iv),
                Payload {
                    msg: &sealed,
                    aad: b"",
                },
            )
            .map_err(|_| CookieError::Decrypt)?;
        let text = String::from_utf8(plain).map_err(|_| CookieError::Payload)?;
        let message = if text.starts_with("{\"_rails\":{\"message\":") {
            let envelope: serde_json::Value =
                serde_json::from_str(&text).map_err(|_| CookieError::Payload)?;
            let rails = &envelope["_rails"];
            if let Some(exp) = rails["exp"].as_str() {
                let exp: DateTime<Utc> = exp.parse().map_err(|_| CookieError::Payload)?;
                if Utc::now() >= exp {
                    return Err(CookieError::Expired);
                }
            }
            if rails["pur"].as_str() != Some(&format!("cookie.{name}")) {
                return Err(CookieError::Purpose);
            }
            B64.decode(rails["message"].as_str().ok_or(CookieError::Payload)?)
                .map_err(|_| CookieError::Payload)?
        } else {
            text.into_bytes()
        };
        decode_payload(&message)
    }

    /// Encrypts `map` as the cookie `name`; `symbol_keys` for the `_t`
    /// cookie (Discourse builds it with symbol keys), string keys for the
    /// session cookie. Returns the form-escaped value.
    pub fn encrypt(
        &self,
        name: &str,
        map: &[(&str, Scalar)],
        symbol_keys: bool,
        expires: Option<DateTime<Utc>>,
    ) -> String {
        let payload = encode_payload(map, symbol_keys);
        let exp = match expires {
            Some(t) => format!("\"{}\"", t.format("%Y-%m-%dT%H:%M:%S%.3fZ")),
            None => "null".to_string(),
        };
        let envelope = format!(
            "{{\"_rails\":{{\"message\":\"{}\",\"exp\":{exp},\"pur\":\"cookie.{name}\"}}}}",
            B64.encode(&payload)
        );
        let mut iv = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut iv);
        let cipher = Aes256Gcm::new(&self.key.into());
        let sealed = cipher
            .encrypt(
                Nonce::from_slice(&iv),
                Payload {
                    msg: envelope.as_bytes(),
                    aad: b"",
                },
            )
            .expect("AES-GCM encryption cannot fail");
        let (ciphertext, tag) = sealed.split_at(sealed.len() - 16);
        form_escape(&format!(
            "{}--{}--{}",
            B64.encode(ciphertext),
            B64.encode(iv),
            B64.encode(tag)
        ))
    }
}

/// `ActiveSupport::MessagePack.dump`: the `CC 80` signature then one map.
/// Only what cookies hold is encoded: string or symbol keys, string,
/// integer, boolean and nil values.
pub fn encode_payload(map: &[(&str, Scalar)], symbol_keys: bool) -> Vec<u8> {
    let mut out = vec![0xcc, 0x80];
    encode_map_header(&mut out, map.len());
    for (key, value) in map {
        if symbol_keys {
            encode_symbol(&mut out, key);
        } else {
            encode_str(&mut out, key);
        }
        match value {
            Scalar::Str(s) => encode_str(&mut out, s),
            Scalar::Int(i) => encode_int(&mut out, *i),
            Scalar::Bool(b) => out.push(if *b { 0xc3 } else { 0xc2 }),
            Scalar::Nil => out.push(0xc0),
        }
    }
    out
}

fn encode_map_header(out: &mut Vec<u8>, len: usize) {
    if len < 16 {
        out.push(0x80 | len as u8);
    } else {
        out.push(0xde);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    }
}

fn encode_str(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    match b.len() {
        n if n < 32 => out.push(0xa0 | n as u8),
        n if n < 256 => {
            out.push(0xd9);
            out.push(n as u8);
        }
        n => {
            out.push(0xda);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
    }
    out.extend_from_slice(b);
}

/// Ruby symbols: msgpack ext type 0 with the name as payload.
fn encode_symbol(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    match b.len() {
        1 => out.extend_from_slice(&[0xd4, 0x00]),
        2 => out.extend_from_slice(&[0xd5, 0x00]),
        4 => out.extend_from_slice(&[0xd6, 0x00]),
        8 => out.extend_from_slice(&[0xd7, 0x00]),
        16 => out.extend_from_slice(&[0xd8, 0x00]),
        n if n < 256 => out.extend_from_slice(&[0xc7, n as u8, 0x00]),
        n => {
            out.push(0xc8);
            out.extend_from_slice(&(n as u16).to_be_bytes());
            out.push(0x00);
        }
    }
    out.extend_from_slice(b);
}

/// The smallest encoding, as Ruby's msgpack picks it.
fn encode_int(out: &mut Vec<u8>, i: i64) {
    // Positive and negative fixints are the byte itself.
    if (-32..128).contains(&i) {
        out.push(i as u8);
    } else if (0..256).contains(&i) {
        out.extend_from_slice(&[0xcc, i as u8]);
    } else if (0..65536).contains(&i) {
        out.push(0xcd);
        out.extend_from_slice(&(i as u16).to_be_bytes());
    } else if (0..4294967296).contains(&i) {
        out.push(0xce);
        out.extend_from_slice(&(i as u32).to_be_bytes());
    } else if i >= 0 {
        out.push(0xcf);
        out.extend_from_slice(&(i as u64).to_be_bytes());
    } else if i >= i32::MIN as i64 {
        out.push(0xd2);
        out.extend_from_slice(&(i as i32).to_be_bytes());
    } else {
        out.push(0xd3);
        out.extend_from_slice(&i.to_be_bytes());
    }
}

/// The inner payload: msgpack with the signature, else JSON (Rails'
/// serializer accepts both).
fn decode_payload(bytes: &[u8]) -> Result<Map, CookieError> {
    if let Some(rest) = bytes.strip_prefix(&[0xcc, 0x80]) {
        let mut reader = Reader {
            bytes: rest,
            pos: 0,
        };
        return reader.map();
    }
    let json: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| CookieError::Payload)?;
    let object = json.as_object().ok_or(CookieError::Payload)?;
    let mut map = Map::new();
    for (k, v) in object {
        let scalar = match v {
            serde_json::Value::String(s) => Scalar::Str(s.clone()),
            serde_json::Value::Number(n) => Scalar::Int(n.as_i64().ok_or(CookieError::Payload)?),
            serde_json::Value::Bool(b) => Scalar::Bool(*b),
            serde_json::Value::Null => Scalar::Nil,
            _ => return Err(CookieError::Payload),
        };
        map.insert(k.clone(), scalar);
    }
    Ok(map)
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, CookieError> {
        let b = *self.bytes.get(self.pos).ok_or(CookieError::Payload)?;
        self.pos += 1;
        Ok(b)
    }

    fn take(&mut self, n: usize) -> Result<&[u8], CookieError> {
        let end = self.pos.checked_add(n).ok_or(CookieError::Payload)?;
        let slice = self.bytes.get(self.pos..end).ok_or(CookieError::Payload)?;
        self.pos = end;
        Ok(slice)
    }

    fn be(&mut self, n: usize) -> Result<u64, CookieError> {
        let mut v = 0u64;
        for b in self.take(n)? {
            v = (v << 8) | u64::from(*b);
        }
        Ok(v)
    }

    fn map(&mut self) -> Result<Map, CookieError> {
        let header = self.byte()?;
        let len = match header {
            0x80..=0x8f => usize::from(header & 0x0f),
            0xde => self.be(2)? as usize,
            0xdf => self.be(4)? as usize,
            _ => return Err(CookieError::Payload),
        };
        let mut map = Map::new();
        for _ in 0..len {
            let key = self.key()?;
            let value = self.scalar()?;
            map.insert(key, value);
        }
        Ok(map)
    }

    /// A string or an ext-0 symbol.
    fn key(&mut self) -> Result<String, CookieError> {
        let header = self.byte()?;
        let len = match header {
            0xa0..=0xbf => usize::from(header & 0x1f),
            0xd9 => self.be(1)? as usize,
            0xda => self.be(2)? as usize,
            0xdb => self.be(4)? as usize,
            0xd4 => self.ext_type(1)?,
            0xd5 => self.ext_type(2)?,
            0xd6 => self.ext_type(4)?,
            0xd7 => self.ext_type(8)?,
            0xd8 => self.ext_type(16)?,
            0xc7 => {
                let n = self.be(1)? as usize;
                self.ext_type(n)?
            }
            0xc8 => {
                let n = self.be(2)? as usize;
                self.ext_type(n)?
            }
            _ => return Err(CookieError::Payload),
        };
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| CookieError::Payload)
    }

    fn ext_type(&mut self, len: usize) -> Result<usize, CookieError> {
        if self.byte()? != 0 {
            return Err(CookieError::Payload);
        }
        Ok(len)
    }

    fn scalar(&mut self) -> Result<Scalar, CookieError> {
        let header = self.byte()?;
        Ok(match header {
            0x00..=0x7f => Scalar::Int(i64::from(header)),
            0xe0..=0xff => Scalar::Int(i64::from(header as i8)),
            0xc0 => Scalar::Nil,
            0xc2 => Scalar::Bool(false),
            0xc3 => Scalar::Bool(true),
            0xcc => Scalar::Int(self.be(1)? as i64),
            0xcd => Scalar::Int(self.be(2)? as i64),
            0xce => Scalar::Int(self.be(4)? as i64),
            0xcf => Scalar::Int(self.be(8)? as i64),
            0xd0 => Scalar::Int(i64::from(self.be(1)? as u8 as i8)),
            0xd1 => Scalar::Int(i64::from(self.be(2)? as u16 as i16)),
            0xd2 => Scalar::Int(i64::from(self.be(4)? as u32 as i32)),
            0xd3 => Scalar::Int(self.be(8)? as i64),
            0xa0..=0xbf | 0xd9 | 0xda | 0xdb => {
                self.pos -= 1;
                Scalar::Str(self.key()?)
            }
            _ => return Err(CookieError::Payload),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn encodes_the_auth_payload_like_ruby_msgpack() {
        let token = "a".repeat(32);
        let map = [
            ("token", Scalar::Str(token.clone())),
            ("user_id", Scalar::Int(1)),
            ("username", Scalar::Str("admin".into())),
            ("trust_level", Scalar::Int(4)),
            ("issued_at", Scalar::Int(1790000000)),
        ];
        let bytes = encode_payload(&map, true);
        let expected = format!(
            "cc8085c70500{}d920{}c70700{}01d700{}a5{}c70b00{}04c70900{}ce6ab13b80",
            hex(b"token"),
            hex(token.as_bytes()),
            hex(b"user_id"),
            hex(b"username"),
            hex(b"admin"),
            hex(b"trust_level"),
            hex(b"issued_at"),
        );
        assert_eq!(hex(&bytes), expected);
        let decoded = decode_payload(&bytes).unwrap();
        assert_eq!(decoded["token"], Scalar::Str(token));
        assert_eq!(decoded["issued_at"], Scalar::Int(1790000000));
        assert_eq!(decoded["user_id"], Scalar::Int(1));
    }

    #[test]
    fn encodes_the_session_payload_with_string_keys() {
        let map = [
            ("session_id", Scalar::Str("f".repeat(32))),
            ("_csrf_token", Scalar::Str("c".repeat(43))),
        ];
        let bytes = encode_payload(&map, false);
        assert!(hex(&bytes).starts_with(&format!("cc8082aa{}d920", hex(b"session_id"))));
        assert!(hex(&bytes).contains(&format!("ab{}d92b", hex(b"_csrf_token"))));
        assert_eq!(decode_payload(&bytes).unwrap().len(), 2);
    }

    #[test]
    fn round_trips_through_encryption_with_purpose_and_expiry() {
        let codec = Codec::new(&"0123456789abcdef".repeat(8));
        let map = [
            ("token", Scalar::Str("x".repeat(32))),
            ("user_id", Scalar::Int(300)),
        ];
        let value = codec.encrypt(
            "_t",
            &map,
            true,
            Some(Utc::now() + chrono::Duration::hours(1)),
        );
        assert!(value.contains("--"));
        assert!(
            !value.contains('+') && !value.contains('/') && !value.contains('='),
            "{value}"
        );
        let decoded = codec.decrypt("_t", &value).unwrap();
        assert_eq!(decoded["user_id"], Scalar::Int(300));
        assert!(matches!(
            codec.decrypt("_forum_session", &value),
            Err(CookieError::Purpose)
        ));
        let expired = codec.encrypt(
            "_t",
            &map,
            true,
            Some(Utc::now() - chrono::Duration::seconds(1)),
        );
        assert!(matches!(
            codec.decrypt("_t", &expired),
            Err(CookieError::Expired)
        ));
        let other = Codec::new(&"f".repeat(128));
        assert!(matches!(
            other.decrypt("_t", &value),
            Err(CookieError::Decrypt)
        ));
        // Tampering with the ciphertext fails authentication.
        let mut chars: Vec<char> = form_unescape(&value).chars().collect();
        // Replaced, not swapped with its neighbour: two equal base64
        // characters (1 in 64) would leave the value unchanged.
        chars[2] = if chars[2] == 'A' { 'B' } else { 'A' };
        let tampered: String = chars.into_iter().collect();
        assert!(codec.decrypt("_t", &form_escape(&tampered)).is_err());
    }

    #[test]
    fn accepts_json_payloads() {
        let map = decode_payload(br#"{"token":"abc","user_id":5}"#).unwrap();
        assert_eq!(map["token"], Scalar::Str("abc".into()));
        assert_eq!(map["user_id"], Scalar::Int(5));
    }
}
