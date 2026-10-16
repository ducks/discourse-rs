//! The cose gem's part of DiscourseWebauthn: `COSE::Key.deserialize` of a
//! credential's public key and `COSE::Algorithm#verify` of an assertion's
//! signature, for the algorithms DiscourseWebauthn::SUPPORTED_ALGORITHMS
//! registers keys with (ES256, ES384, ES512, EdDSA, PS256, PS384, PS512 and
//! the RS256 freedom patch).

use ciborium::Value;

/// `COSE::Algorithm` ids.
pub const ES256: i64 = -7;
pub const EDDSA: i64 = -8;
pub const ES384: i64 = -35;
pub const ES512: i64 = -36;
pub const PS256: i64 = -37;
pub const PS384: i64 = -38;
pub const PS512: i64 = -39;
pub const RS256: i64 = -257;

/// `COSE::Algorithm.find(alg)`'s registry: the ones a key can name, the
/// HMACs and ES256K among them.
const REGISTERED: [i64; 13] = [
    ES256, ES384, ES512, -47, EDDSA, PS256, PS384, PS512, 4, 5, 6, 7, RS256,
];

/// A COSE_Key (RFC 8152 section 7) as the cose gem reads one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// kty 2: an elliptic curve point.
    Ec2 {
        alg: Option<i64>,
        crv: Option<i64>,
        x: Vec<u8>,
        y: Vec<u8>,
    },
    /// kty 1: an octet key pair (Ed25519, Ed448).
    Okp {
        alg: Option<i64>,
        crv: Option<i64>,
        x: Vec<u8>,
    },
    /// kty 3
    Rsa {
        alg: Option<i64>,
        n: Vec<u8>,
        e: Vec<u8>,
    },
    /// Any other kty (symmetric keys): readable, never a signer.
    Other { alg: Option<i64> },
}

/// What can go wrong reading or using a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// `COSE::MalformedKeyError` (and CBOR's own errors).
    Malformed,
    /// `COSE::Algorithm.find` found nothing.
    UnknownAlgorithm,
    /// `COSE::Error`: an incompatible key or a failed signature.
    Verification,
    /// A registered algorithm the port can't verify (ES256K, Ed448, the
    /// HMACs); DiscourseWebauthn never registers a key with one.
    Unported(&'static str),
}

fn int(v: &Value) -> Option<i64> {
    v.as_integer().and_then(|i| i64::try_from(i).ok())
}

fn label(map: &[(Value, Value)], key: i64) -> Option<&Value> {
    map.iter()
        .find(|(k, _)| int(k) == Some(key))
        .map(|(_, v)| v)
}

fn bytes(map: &[(Value, Value)], key: i64) -> Option<Vec<u8>> {
    label(map, key).and_then(|v| v.as_bytes()).cloned()
}

impl Key {
    /// `COSE::Key.deserialize(bytes)`
    pub fn deserialize(data: &[u8]) -> Result<Self, Error> {
        let value: Value = ciborium::de::from_reader(data).map_err(|_| Error::Malformed)?;
        Self::from_cbor(&value)
    }

    /// `COSE::Key.from_map`
    pub fn from_cbor(value: &Value) -> Result<Self, Error> {
        let map = value.as_map().ok_or(Error::Malformed)?;
        let alg = label(map, 3)
            .map(|v| int(v).ok_or(Error::Malformed))
            .transpose()?;
        let crv = label(map, -1).and_then(int);
        match label(map, 1).and_then(int) {
            Some(2) => Ok(Key::Ec2 {
                alg,
                crv,
                x: bytes(map, -2).ok_or(Error::Malformed)?,
                y: bytes(map, -3).ok_or(Error::Malformed)?,
            }),
            Some(1) => Ok(Key::Okp {
                alg,
                crv,
                x: bytes(map, -2).ok_or(Error::Malformed)?,
            }),
            Some(3) => Ok(Key::Rsa {
                alg,
                n: bytes(map, -1).ok_or(Error::Malformed)?,
                e: bytes(map, -2).ok_or(Error::Malformed)?,
            }),
            Some(_) => Ok(Key::Other { alg }),
            None => Err(Error::Malformed),
        }
    }

    pub fn alg(&self) -> Option<i64> {
        match self {
            Key::Ec2 { alg, .. } | Key::Okp { alg, .. } | Key::Rsa { alg, .. } => *alg,
            Key::Other { alg } => *alg,
        }
    }

    /// `COSE::Algorithm.find(key.alg).verify(key, signature, data)`
    pub fn verify(&self, signature: &[u8], data: &[u8]) -> Result<(), Error> {
        let alg = self.alg().ok_or(Error::UnknownAlgorithm)?;
        if !REGISTERED.contains(&alg) {
            return Err(Error::UnknownAlgorithm);
        }
        let ok = match (alg, self) {
            (
                ES256,
                Key::Ec2 {
                    crv: Some(1), x, y, ..
                },
            ) => es256(x, y, signature, data),
            (
                ES384,
                Key::Ec2 {
                    crv: Some(2), x, y, ..
                },
            ) => es384(x, y, signature, data),
            (
                ES512,
                Key::Ec2 {
                    crv: Some(3), x, y, ..
                },
            ) => es512(x, y, signature, data),
            (ES256 | ES384 | ES512, _) => false,
            (
                EDDSA,
                Key::Okp {
                    crv: Some(6), x, ..
                },
            ) => ed25519(x, signature, data),
            (EDDSA, Key::Okp { crv: Some(7), .. }) => return Err(Error::Unported("Ed448 keys")),
            (EDDSA, _) => false,
            (PS256 | PS384 | PS512 | RS256, Key::Rsa { n, e, .. }) => {
                rsa_verify(alg, n, e, signature, data)
            }
            (PS256 | PS384 | PS512 | RS256, _) => false,
            (-47, _) => return Err(Error::Unported("ES256K keys")),
            _ => return Err(Error::Unported("HMAC COSE algorithms")),
        };
        if ok { Ok(()) } else { Err(Error::Verification) }
    }
}

/// OpenSSL::SignatureAlgorithm::ECDSA#formatted_signature: a raw r||s of
/// twice the field size is taken as such, anything else as DER.
macro_rules! ecdsa_verify {
    ($name:ident, $curve:ident, $len:expr) => {
        fn $name(x: &[u8], y: &[u8], signature: &[u8], data: &[u8]) -> bool {
            use $curve::ecdsa::signature::Verifier;
            use $curve::ecdsa::{Signature, VerifyingKey};
            if x.len() != $len || y.len() != $len {
                return false;
            }
            let point = $curve::EncodedPoint::from_affine_coordinates(x.into(), y.into(), false);
            let Ok(key) = VerifyingKey::from_encoded_point(&point) else {
                return false;
            };
            let sig = if signature.len() == 2 * $len {
                Signature::from_slice(signature)
            } else {
                Signature::from_der(signature)
            };
            sig.is_ok_and(|sig| key.verify(data, &sig).is_ok())
        }
    };
}

ecdsa_verify!(es256, p256, 32);
ecdsa_verify!(es384, p384, 48);
ecdsa_verify!(es512, p521, 66);

fn ed25519(x: &[u8], signature: &[u8], data: &[u8]) -> bool {
    use ed25519_dalek::Verifier;
    let Ok(x) = <[u8; 32]>::try_from(x) else {
        return false;
    };
    let Ok(key) = ed25519_dalek::VerifyingKey::from_bytes(&x) else {
        return false;
    };
    let Ok(sig) = ed25519_dalek::Signature::from_slice(signature) else {
        return false;
    };
    key.verify(data, &sig).is_ok()
}

/// RSASSA-PSS (salt length :auto, as openssl-signature_algorithm verifies)
/// or, for RS256, RSASSA-PKCS1-v1_5.
fn rsa_verify(alg: i64, n: &[u8], e: &[u8], signature: &[u8], data: &[u8]) -> bool {
    use rsa::signature::Verifier;
    let Ok(key) = rsa::RsaPublicKey::new(
        rsa::BigUint::from_bytes_be(n),
        rsa::BigUint::from_bytes_be(e),
    ) else {
        return false;
    };
    macro_rules! check {
        ($verifying:ty, $signature:ty) => {{
            let verifying = <$verifying>::new(key);
            <$signature>::try_from(signature).is_ok_and(|sig| verifying.verify(data, &sig).is_ok())
        }};
    }
    match alg {
        RS256 => check!(
            rsa::pkcs1v15::VerifyingKey<sha2::Sha256>,
            rsa::pkcs1v15::Signature
        ),
        PS256 => check!(rsa::pss::VerifyingKey<sha2::Sha256>, rsa::pss::Signature),
        PS384 => check!(rsa::pss::VerifyingKey<sha2::Sha384>, rsa::pss::Signature),
        _ => check!(rsa::pss::VerifyingKey<sha2::Sha512>, rsa::pss::Signature),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(map: Vec<(i64, Value)>) -> Vec<u8> {
        let value = Value::Map(map.into_iter().map(|(k, v)| (Value::from(k), v)).collect());
        let mut out = Vec::new();
        ciborium::ser::into_writer(&value, &mut out).unwrap();
        out
    }

    const DATA: &[u8] = b"authenticator data and the client data hash";

    #[test]
    fn es256_der_and_raw() {
        use p256::ecdsa::signature::Signer;
        let signing = p256::ecdsa::SigningKey::from_slice(&[7u8; 32]).unwrap();
        let point = signing.verifying_key().to_encoded_point(false);
        let key = Key::deserialize(&encode(vec![
            (1, Value::from(2)),
            (3, Value::from(ES256)),
            (-1, Value::from(1)),
            (-2, Value::Bytes(point.x().unwrap().to_vec())),
            (-3, Value::Bytes(point.y().unwrap().to_vec())),
        ]))
        .unwrap();
        let sig: p256::ecdsa::Signature = signing.sign(DATA);
        assert_eq!(key.verify(sig.to_der().as_bytes(), DATA), Ok(()));
        assert_eq!(key.verify(&sig.to_bytes(), DATA), Ok(()));
        assert_eq!(
            key.verify(sig.to_der().as_bytes(), b"other data"),
            Err(Error::Verification)
        );
    }

    #[test]
    fn es384_and_es512() {
        use p384::ecdsa::signature::Signer;
        let signing = p384::ecdsa::SigningKey::from_slice(&[9u8; 48]).unwrap();
        let point = signing.verifying_key().to_encoded_point(false);
        let key = Key::Ec2 {
            alg: Some(ES384),
            crv: Some(2),
            x: point.x().unwrap().to_vec(),
            y: point.y().unwrap().to_vec(),
        };
        let sig: p384::ecdsa::Signature = signing.sign(DATA);
        assert_eq!(key.verify(sig.to_der().as_bytes(), DATA), Ok(()));
        // The curve must be the algorithm's.
        let wrong = Key::Ec2 {
            alg: Some(ES256),
            crv: Some(2),
            x: point.x().unwrap().to_vec(),
            y: point.y().unwrap().to_vec(),
        };
        assert_eq!(
            wrong.verify(sig.to_der().as_bytes(), DATA),
            Err(Error::Verification)
        );

        let mut scalar = [0u8; 66];
        scalar[65] = 5;
        use p521::ecdsa::signature::RandomizedSigner as _;
        let signing = p521::ecdsa::SigningKey::from_slice(&scalar).unwrap();
        let point = p521::ecdsa::VerifyingKey::from(&signing).to_encoded_point(false);
        let key = Key::Ec2 {
            alg: Some(ES512),
            crv: Some(3),
            x: point.x().unwrap().to_vec(),
            y: point.y().unwrap().to_vec(),
        };
        let sig: p521::ecdsa::Signature = signing.sign_with_rng(&mut rand::thread_rng(), DATA);
        assert_eq!(key.verify(sig.to_der().as_bytes(), DATA), Ok(()));
        assert_eq!(key.verify(&sig.to_bytes(), DATA), Ok(()));
    }

    #[test]
    fn eddsa() {
        use ed25519_dalek::Signer;
        // RFC 8032 section 7.1, test 1: the empty message.
        let secret: [u8; 32] = [
            0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec,
            0x2c, 0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03,
            0x1c, 0xae, 0x7f, 0x60,
        ];
        let signing = ed25519_dalek::SigningKey::from_bytes(&secret);
        let key = Key::Okp {
            alg: Some(EDDSA),
            crv: Some(6),
            x: signing.verifying_key().to_bytes().to_vec(),
        };
        let sig = signing.sign(b"");
        assert_eq!(
            &sig.to_bytes()[..4],
            &[0xe5, 0x56, 0x43, 0x00],
            "RFC 8032's signature"
        );
        assert_eq!(key.verify(&sig.to_bytes(), b""), Ok(()));
        assert_eq!(key.verify(&sig.to_bytes(), b"x"), Err(Error::Verification));
    }

    #[test]
    fn rsa_pkcs1_and_pss() {
        use rsa::signature::{RandomizedSigner, SignatureEncoding, Signer};
        let mut rng = rand::thread_rng();
        let private = rsa::RsaPrivateKey::new(&mut rng, 1024).unwrap();
        let public = private.to_public_key();
        use rsa::traits::PublicKeyParts;
        let key = |alg| Key::Rsa {
            alg: Some(alg),
            n: public.n().to_bytes_be(),
            e: public.e().to_bytes_be(),
        };
        let pkcs1 = rsa::pkcs1v15::SigningKey::<sha2::Sha256>::new(private.clone()).sign(DATA);
        assert_eq!(key(RS256).verify(&pkcs1.to_bytes(), DATA), Ok(()));
        assert_eq!(
            key(PS256).verify(&pkcs1.to_bytes(), DATA),
            Err(Error::Verification)
        );
        let pss = rsa::pss::SigningKey::<sha2::Sha256>::new(private.clone())
            .sign_with_rng(&mut rng, DATA);
        assert_eq!(key(PS256).verify(&pss.to_bytes(), DATA), Ok(()));
        let pss = rsa::pss::SigningKey::<sha2::Sha384>::new(private).sign_with_rng(&mut rng, DATA);
        assert_eq!(key(PS384).verify(&pss.to_bytes(), DATA), Ok(()));
    }

    #[test]
    fn unknown_and_malformed_keys() {
        let key = Key::Ec2 {
            alg: Some(-999),
            crv: Some(1),
            x: vec![],
            y: vec![],
        };
        assert_eq!(key.verify(b"", b""), Err(Error::UnknownAlgorithm));
        assert_eq!(Key::deserialize(b"\xff"), Err(Error::Malformed));
        assert_eq!(
            Key::deserialize(&encode(vec![(3, Value::from(ES256))])),
            Err(Error::Malformed)
        );
    }
}
