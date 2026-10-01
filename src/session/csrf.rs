//! Rails' `RequestForgeryProtection` tokens: a 32-byte secret in the
//! session, handed out masked (one-time pad, URL-safe base64) and accepted
//! in its masked, raw, global or per-form forms.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// `generate_csrf_token`: `SecureRandom.urlsafe_base64(32)`.
pub fn generate() -> String {
    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    URL_SAFE_NO_PAD.encode(raw)
}

/// Ruby's `Base64.urlsafe_decode64`: URL-safe or standard alphabet, with or
/// without padding.
fn decode(s: &str) -> Option<Vec<u8>> {
    let standard: String = s.replace('-', "+").replace('_', "/");
    let unpadded = standard.trim_end_matches('=');
    let padded = match unpadded.len() % 4 {
        2 => format!("{unpadded}=="),
        3 => format!("{unpadded}="),
        _ => unpadded.to_string(),
    };
    STANDARD.decode(padded).ok()
}

/// The 32 real bytes behind a stored session token.
fn real(stored: &str) -> Option<[u8; 32]> {
    let bytes = decode(stored)?;
    bytes.try_into().ok()
}

fn hmac(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("any key length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

/// `masked_authenticity_token` with no form options: the global token
/// under a fresh one-time pad.
pub fn mask(stored: &str) -> Option<String> {
    let real = real(stored)?;
    let global = hmac(&real, b"!real_csrf_token");
    let mut pad = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut pad);
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(&pad);
    for (p, g) in pad.iter().zip(global.iter()) {
        out.push(p ^ g);
    }
    Some(URL_SAFE_NO_PAD.encode(out))
}

/// `valid_authenticity_token?`: the token presented by the client against
/// the session's; `path` and `method` for the per-form variant.
pub fn valid(stored: &str, presented: &str, path: &str, method: &str) -> bool {
    let Some(real) = real(stored) else {
        return false;
    };
    let Some(decoded) = decode(presented) else {
        return false;
    };
    let csrf: [u8; 32] = match decoded.len() {
        32 => decoded.try_into().unwrap(),
        64 => {
            let (pad, masked) = decoded.split_at(32);
            let mut out = [0u8; 32];
            for (i, (p, m)) in pad.iter().zip(masked.iter()).enumerate() {
                out[i] = p ^ m;
            }
            out
        }
        _ => return false,
    };
    let global = hmac(&real, b"!real_csrf_token");
    let form = hmac(
        &real,
        format!("{}#{}", path.trim_end_matches('/'), method.to_lowercase()).as_bytes(),
    );
    bool::from(csrf.ct_eq(&global))
        || bool::from(csrf.ct_eq(&real))
        || bool::from(csrf.ct_eq(&form))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masked_tokens_validate_and_differ_each_time() {
        let stored = generate();
        assert_eq!(stored.len(), 43);
        let a = mask(&stored).unwrap();
        let b = mask(&stored).unwrap();
        assert_eq!(a.len(), 86);
        assert_ne!(a, b);
        assert!(valid(&stored, &a, "/session", "POST"));
        assert!(valid(&stored, &b, "/session", "POST"));
        assert!(
            valid(&stored, &stored, "/session", "POST"),
            "the raw token is accepted"
        );
        assert!(!valid(
            &stored,
            &mask(&generate()).unwrap(),
            "/session",
            "POST"
        ));
        assert!(!valid(&stored, "", "/session", "POST"));
        assert!(!valid(&stored, "not base64 at all!", "/session", "POST"));
    }

    #[test]
    fn per_form_tokens_validate_for_their_action_only() {
        let stored = generate();
        let real = real(&stored).unwrap();
        let form = hmac(&real, b"/session#post");
        let presented = URL_SAFE_NO_PAD.encode(form);
        assert!(valid(&stored, &presented, "/session/", "POST"));
        assert!(!valid(&stored, &presented, "/login", "POST"));
    }
}
