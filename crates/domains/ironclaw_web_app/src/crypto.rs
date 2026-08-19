//! RFC 8291 (`aes128gcm`) Web Push payload encryption.
//!
//! One record per message: ECDH(P-256) against the subscription's `p256dh`
//! key, HKDF-SHA256 through the RFC 8291 info strings, AES-128-GCM over
//! `plaintext || 0x02`, framed per RFC 8188 as
//! `salt(16) || rs(4) || idlen(1) || as_public(65) || ciphertext`.
//!
//! All primitives come from `aws-lc-rs` (already linked workspace-wide).
//! Encryption uses only the *recipient's* public material plus a fresh
//! ephemeral keypair and salt — no stored secret is involved, which is why
//! this lives in the domain crate while VAPID signing stays at the host
//! egress boundary.

use aws_lc_rs::{aead, agreement, hkdf, rand as aws_rand};

use crate::error::WebAppError;

/// RFC 8188 record size we declare in the header. Push services cap the
/// whole body at 4096 bytes, so a single 4096-byte record always suffices.
pub const RECORD_SIZE: u32 = 4096;

/// Total encrypted-body budget push services commonly enforce (RFC 8030 §7.2
/// suggests supporting at least 4096 bytes; FCM/Mozilla/Apple all cap there).
pub const MAX_ENCRYPTED_BODY_BYTES: usize = 4096;

/// Header: salt(16) + rs(4) + idlen(1) + uncompressed P-256 point (65).
const HEADER_BYTES: usize = 16 + 4 + 1 + 65;
const TAG_BYTES: usize = 16;
/// The single-record padding delimiter (`0x02` marks the final record).
const PAD_DELIMITER_BYTES: usize = 1;

/// Largest plaintext that still fits the 4096-byte body budget.
pub const MAX_PLAINTEXT_BYTES: usize =
    MAX_ENCRYPTED_BODY_BYTES - HEADER_BYTES - TAG_BYTES - PAD_DELIMITER_BYTES;

// PROTOCOL-FIXED (RFC 8291 §3.3/§3.4): the CEK/nonce key-derivation info
// string is the literal `WebPush: info` regardless of what the channel is
// named — changing it breaks decryption at every push service. Pinned by the
// Appendix A test-vector test below and allowlisted in the
// vocabulary-retirement gate.
const KEY_INFO_PREFIX: &[u8] = b"WebPush: info\0";
const CEK_INFO: &[u8] = b"Content-Encoding: aes128gcm\0";
const NONCE_INFO: &[u8] = b"Content-Encoding: nonce\0";

struct HkdfOutputLen(usize);

impl hkdf::KeyType for HkdfOutputLen {
    fn len(&self) -> usize {
        self.0
    }
}

/// Encrypt `plaintext` for the subscription identified by `ua_public`
/// (65-byte uncompressed P-256 point) and `auth_secret` (16 bytes), using a
/// fresh ephemeral keypair and salt. Returns the complete `aes128gcm` body.
pub fn encrypt_payload(
    ua_public: &[u8],
    auth_secret: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, WebAppError> {
    let rng = aws_rand::SystemRandom::new();
    let mut salt = [0u8; 16];
    aws_rand::SecureRandom::fill(&rng, &mut salt)
        .map_err(|_| WebAppError::crypto("salt generation failed"))?;
    let as_private = agreement::PrivateKey::generate(&agreement::ECDH_P256)
        .map_err(|_| WebAppError::crypto("ephemeral key generation failed"))?;
    encrypt_with_materials(&as_private, salt, ua_public, auth_secret, plaintext)
}

/// Deterministic core, split out so the RFC 8291 appendix vector can drive
/// it with the fixed ephemeral key and salt.
pub(crate) fn encrypt_with_materials(
    as_private: &agreement::PrivateKey,
    salt: [u8; 16],
    ua_public: &[u8],
    auth_secret: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, WebAppError> {
    if plaintext.len() > MAX_PLAINTEXT_BYTES {
        return Err(WebAppError::PayloadTooLarge {
            bytes: plaintext.len(),
            limit: MAX_PLAINTEXT_BYTES,
        });
    }
    if ua_public.len() != 65 || ua_public[0] != 0x04 {
        return Err(WebAppError::InvalidSubscription {
            reason: "p256dh must be a 65-byte uncompressed P-256 point".to_string(),
        });
    }
    if auth_secret.len() != 16 {
        return Err(WebAppError::InvalidSubscription {
            reason: "auth must be 16 bytes".to_string(),
        });
    }

    let as_public = as_private
        .compute_public_key()
        .map_err(|_| WebAppError::crypto("ephemeral public key derivation failed"))?;
    let as_public_bytes = as_public.as_ref();
    if as_public_bytes.len() != 65 {
        return Err(WebAppError::crypto(
            "ephemeral public key is not an uncompressed P-256 point",
        ));
    }

    let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, ua_public);
    let ecdh_secret = agreement::agree(
        as_private,
        peer,
        WebAppError::crypto("ECDH agreement failed"),
        |secret| Ok(secret.to_vec()),
    )?;

    let (cek, nonce) =
        derive_cek_and_nonce(&ecdh_secret, auth_secret, ua_public, as_public_bytes, &salt)?;

    // record = plaintext || 0x02 (final-record padding delimiter).
    let mut record = Vec::with_capacity(plaintext.len() + PAD_DELIMITER_BYTES + TAG_BYTES);
    record.extend_from_slice(plaintext);
    record.push(0x02);

    let unbound = aead::UnboundKey::new(&aead::AES_128_GCM, &cek)
        .map_err(|_| WebAppError::crypto("content-encryption key rejected"))?;
    let key = aead::LessSafeKey::new(unbound);
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::empty(),
        &mut record,
    )
    .map_err(|_| WebAppError::crypto("payload sealing failed"))?;

    let mut body = Vec::with_capacity(HEADER_BYTES + record.len());
    body.extend_from_slice(&salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(65u8);
    body.extend_from_slice(as_public_bytes);
    body.extend_from_slice(&record);
    Ok(body)
}

/// RFC 8291 §3.3-3.4: two HKDF stages from the ECDH secret to CEK + nonce.
fn derive_cek_and_nonce(
    ecdh_secret: &[u8],
    auth_secret: &[u8],
    ua_public: &[u8],
    as_public: &[u8],
    salt: &[u8; 16],
) -> Result<([u8; 16], [u8; 12]), WebAppError> {
    // IKM = HKDF(salt=auth_secret, ikm=ecdh_secret, info="WebPush: info\0" || ua_public || as_public, 32)
    let mut ikm = [0u8; 32];
    hkdf_expand(
        auth_secret,
        ecdh_secret,
        &[KEY_INFO_PREFIX, ua_public, as_public],
        &mut ikm,
    )?;
    // CEK = HKDF(salt, IKM, "Content-Encoding: aes128gcm\0", 16)
    let mut cek = [0u8; 16];
    hkdf_expand(salt, &ikm, &[CEK_INFO], &mut cek)?;
    // NONCE = HKDF(salt, IKM, "Content-Encoding: nonce\0", 12)
    let mut nonce = [0u8; 12];
    hkdf_expand(salt, &ikm, &[NONCE_INFO], &mut nonce)?;
    Ok((cek, nonce))
}

fn hkdf_expand(salt: &[u8], ikm: &[u8], info: &[&[u8]], out: &mut [u8]) -> Result<(), WebAppError> {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(ikm);
    let okm = prk
        .expand(info, HkdfOutputLen(out.len()))
        .map_err(|_| WebAppError::crypto("HKDF expand failed"))?;
    okm.fill(out)
        .map_err(|_| WebAppError::crypto("HKDF fill failed"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    /// Wrap a raw P-256 scalar in a PKCS#8 `ECPrivateKey` DER (no embedded
    /// public key) so the RFC vector's fixed keys can drive the deterministic
    /// core. Fixed template per RFC 5208/5915.
    fn p256_pkcs8_from_scalar(scalar: &[u8]) -> Vec<u8> {
        assert_eq!(scalar.len(), 32, "P-256 scalar is 32 bytes");
        let mut der = Vec::with_capacity(67);
        der.extend_from_slice(&[0x30, 0x41, 0x02, 0x01, 0x00]);
        // AlgorithmIdentifier: ecPublicKey + prime256v1.
        der.extend_from_slice(&[
            0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
            0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07,
        ]);
        // OCTET STRING { ECPrivateKey { version 1, privateKey } }
        der.extend_from_slice(&[0x04, 0x27, 0x30, 0x25, 0x02, 0x01, 0x01, 0x04, 0x20]);
        der.extend_from_slice(scalar);
        der
    }

    fn b64(value: &str) -> Vec<u8> {
        URL_SAFE_NO_PAD.decode(value).expect("test vector base64")
    }

    /// RFC 8291 Appendix A inputs.
    struct Rfc8291Vector {
        ua_public: Vec<u8>,
        ua_private_scalar: Vec<u8>,
        as_private_scalar: Vec<u8>,
        auth: Vec<u8>,
        salt: [u8; 16],
        plaintext: &'static [u8],
    }

    fn vector() -> Rfc8291Vector {
        let salt_bytes = b64("DGv6ra1nlYgDCS1FRnbzlw");
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&salt_bytes);
        Rfc8291Vector {
            ua_public: b64(
                "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
            ),
            ua_private_scalar: b64("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94"),
            as_private_scalar: b64("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw"),
            auth: b64("BTBZMqHH6r4Tts7J_aSIgg"),
            salt,
            plaintext: b"When I grow up, I want to be a watermelon",
        }
    }

    #[test]
    fn rfc8291_appendix_a_intermediate_values_match() {
        let vector = vector();
        let as_private = agreement::PrivateKey::from_private_key_der(
            &agreement::ECDH_P256,
            &p256_pkcs8_from_scalar(&vector.as_private_scalar),
        )
        .expect("vector private key parses");
        let as_public = as_private.compute_public_key().expect("public key");
        assert_eq!(
            URL_SAFE_NO_PAD.encode(as_public.as_ref()),
            "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8",
            "as_public must match the vector"
        );

        let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, &vector.ua_public);
        let ecdh_secret = agreement::agree(
            &as_private,
            peer,
            WebAppError::crypto("agree failed"),
            |secret| Ok(secret.to_vec()),
        )
        .expect("ECDH agreement");
        assert_eq!(
            URL_SAFE_NO_PAD.encode(&ecdh_secret),
            "kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs",
            "ecdh_secret must match the vector"
        );

        let (cek, nonce) = derive_cek_and_nonce(
            &ecdh_secret,
            &vector.auth,
            &vector.ua_public,
            as_public.as_ref(),
            &vector.salt,
        )
        .expect("derivation");
        assert_eq!(URL_SAFE_NO_PAD.encode(cek), "oIhVW04MRdy2XN9CiKLxTg");
        assert_eq!(URL_SAFE_NO_PAD.encode(nonce), "4h_95klXJ5E_qnoN");
    }

    #[test]
    fn rfc8291_appendix_a_body_round_trips_and_frames_correctly() {
        let vector = vector();
        let as_private = agreement::PrivateKey::from_private_key_der(
            &agreement::ECDH_P256,
            &p256_pkcs8_from_scalar(&vector.as_private_scalar),
        )
        .expect("vector private key parses");
        let body = encrypt_with_materials(
            &as_private,
            vector.salt,
            &vector.ua_public,
            &vector.auth,
            vector.plaintext,
        )
        .expect("encryption succeeds");

        // Header framing: salt || rs=4096 || idlen=65 || as_public.
        assert_eq!(&body[..16], vector.salt.as_slice());
        assert_eq!(&body[16..20], &4096u32.to_be_bytes());
        assert_eq!(body[20], 65);
        assert_eq!(
            URL_SAFE_NO_PAD.encode(&body[21..86]),
            "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8"
        );
        assert_eq!(
            body.len(),
            HEADER_BYTES + vector.plaintext.len() + PAD_DELIMITER_BYTES + TAG_BYTES,
        );

        // Receiver side: decrypt with the vector's UA private key.
        let ua_private = agreement::PrivateKey::from_private_key_der(
            &agreement::ECDH_P256,
            &p256_pkcs8_from_scalar(&vector.ua_private_scalar),
        )
        .expect("ua private key parses");
        let as_public_bytes = body[21..86].to_vec();
        let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, &as_public_bytes);
        let ecdh_secret = agreement::agree(
            &ua_private,
            peer,
            WebAppError::crypto("agree failed"),
            |secret| Ok(secret.to_vec()),
        )
        .expect("receiver ECDH");
        let (cek, nonce) = derive_cek_and_nonce(
            &ecdh_secret,
            &vector.auth,
            &vector.ua_public,
            &as_public_bytes,
            &vector.salt,
        )
        .expect("receiver derivation");

        let unbound = aead::UnboundKey::new(&aead::AES_128_GCM, &cek).expect("cek");
        let key = aead::LessSafeKey::new(unbound);
        let mut ciphertext = body[86..].to_vec();
        let opened = key
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::empty(),
                &mut ciphertext,
            )
            .expect("decryption succeeds");
        assert_eq!(opened.last(), Some(&0x02u8), "final-record delimiter");
        assert_eq!(&opened[..opened.len() - 1], vector.plaintext);
    }

    #[test]
    fn random_path_produces_unique_bodies_that_fit_the_budget() {
        let vector = vector();
        let first = encrypt_payload(&vector.ua_public, &vector.auth, b"hello").expect("encrypts");
        let second = encrypt_payload(&vector.ua_public, &vector.auth, b"hello").expect("encrypts");
        assert_ne!(first, second, "fresh salt + ephemeral key per message");
        assert!(first.len() <= MAX_ENCRYPTED_BODY_BYTES);
    }

    #[test]
    fn oversized_plaintext_fails_closed() {
        let vector = vector();
        let oversized = vec![0u8; MAX_PLAINTEXT_BYTES + 1];
        assert!(matches!(
            encrypt_payload(&vector.ua_public, &vector.auth, &oversized),
            Err(WebAppError::PayloadTooLarge { .. })
        ));
    }

    #[test]
    fn malformed_recipient_material_fails_closed() {
        let vector = vector();
        assert!(encrypt_payload(&[0u8; 65], &vector.auth, b"x").is_err());
        assert!(encrypt_payload(&vector.ua_public[..64], &vector.auth, b"x").is_err());
        assert!(encrypt_payload(&vector.ua_public, &[0u8; 5], b"x").is_err());
    }
}
