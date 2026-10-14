//! UserApiKey::DeviceAuth::Crypto: the client's RSA public key, as
//! `OpenSSL::PKey::RSA.new` reads it (PEM or DER, public or private), and
//! the payload encrypted to it with PKCS#1 v1.5 or OAEP (SHA-1, OpenSSL's
//! default digest for `rsa_padding_mode: oaep`).

use rsa::pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey};
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey};
use rsa::traits::PublicKeyParts;
use rsa::{Oaep, Pkcs1v15Encrypt, RsaPrivateKey, RsaPublicKey};

/// `OpenSSL::PKey::RSA.new(value)`; None where OpenSSL raises RSAError.
pub fn parse_public_key(value: &str) -> Option<RsaPublicKey> {
    let pem = value.trim();
    if let Ok(k) = RsaPublicKey::from_public_key_pem(pem) {
        return Some(k);
    }
    if let Ok(k) = RsaPublicKey::from_pkcs1_pem(pem) {
        return Some(k);
    }
    if let Ok(k) = RsaPrivateKey::from_pkcs1_pem(pem) {
        return Some(k.to_public_key());
    }
    if let Ok(k) = RsaPrivateKey::from_pkcs8_pem(pem) {
        return Some(k.to_public_key());
    }
    let der = value.as_bytes();
    if let Ok(k) = RsaPublicKey::from_public_key_der(der) {
        return Some(k);
    }
    if let Ok(k) = RsaPublicKey::from_pkcs1_der(der) {
        return Some(k);
    }
    if let Ok(k) = RsaPrivateKey::from_pkcs1_der(der) {
        return Some(k.to_public_key());
    }
    RsaPrivateKey::from_pkcs8_der(der)
        .ok()
        .map(|k| k.to_public_key())
}

fn oaep(padding: Option<&str>) -> bool {
    padding == Some("oaep")
}

/// `validate_payload_size!`: the message InvalidParameters carries when
/// the payload is too long for the key and padding.
pub fn payload_too_large(
    payload: &str,
    key: &RsaPublicKey,
    padding: Option<&str>,
) -> Option<String> {
    let key_size = key.size();
    let max = if oaep(padding) {
        key_size.saturating_sub(2 * 20 + 2)
    } else {
        key_size.saturating_sub(11)
    };
    if payload.len() <= max {
        return None;
    }
    let name = if oaep(padding) { "OAEP" } else { "PKCS#1" };
    Some(format!(
        "Payload too large for {name} encryption with this key size. Maximum: {max} bytes, got: {} bytes. \
         Try using a shorter nonce or a larger RSA key (minimum 2048-bit recommended).",
        payload.len()
    ))
}

/// `encrypt!`; None where OpenSSL raises.
pub fn encrypt(key: &RsaPublicKey, data: &[u8], padding: Option<&str>) -> Option<Vec<u8>> {
    let mut rng = rand::thread_rng();
    if oaep(padding) {
        key.encrypt(&mut rng, Oaep::new::<sha1::Sha1>(), data).ok()
    } else {
        key.encrypt(&mut rng, Pkcs1v15Encrypt, data).ok()
    }
}

/// Ruby's `Base64.encode64`: standard alphabet, a newline after every 60
/// characters and at the end.
pub fn encode64(bytes: &[u8]) -> String {
    use base64::Engine;
    let flat = base64::engine::general_purpose::STANDARD.encode(bytes);
    let mut out = String::with_capacity(flat.len() + flat.len() / 60 + 1);
    for chunk in flat.as_bytes().chunks(60) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode64_wraps_like_ruby() {
        assert_eq!(encode64(b""), "");
        assert_eq!(encode64(b"hi"), "aGk=\n");
        let long = encode64(&[0u8; 50]);
        let lines: Vec<&str> = long.lines().collect();
        assert_eq!(lines[0].len(), 60);
        assert!(long.ends_with('\n'));
    }
}
