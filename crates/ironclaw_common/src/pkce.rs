//! RFC 7636 PKCE primitives.
//!
//! Pure protocol math: no product types, no secret wrappers, no error type.
//! Callers expose secret material at their own boundary and pass the bytes in,
//! keeping `expose()` scopes narrow. Outputs are a challenge or a hash — never
//! the verifier itself — so nothing here logs or retains secret material.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::Rng;
use sha2::{Digest, Sha256};

/// RFC 7636 §4.1 unreserved characters: ALPHA / DIGIT / "-" / "." / "_" / "~".
const VERIFIER_CHARSET: &[u8] =
    b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~";

/// Length of a generated code verifier. RFC 7636 permits 43-128 characters;
/// 64 gives ample entropy while staying well within bounds.
const VERIFIER_LEN: usize = 64;

/// Generate a 64-character URL-safe PKCE code verifier (RFC 7636 §4.1).
pub fn generate_code_verifier() -> String {
    let mut rng = rand::thread_rng();
    (0..VERIFIER_LEN)
        .map(|_| {
            let idx = rng.gen_range(0..VERIFIER_CHARSET.len());
            VERIFIER_CHARSET[idx] as char
        })
        .collect()
}

/// Compute the S256 code challenge for a verifier: `base64url-nopad(SHA-256(verifier))`
/// (RFC 7636 §4.2). The caller passes the verifier bytes.
pub fn s256_challenge(verifier: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier))
}

/// Lowercase hex of `SHA-256(bytes)`. Used to derive a stored verifier hash
/// (and other opaque-value hashes) without retaining the raw input.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex_encode(&Sha256::digest(bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // Writing to a String is infallible.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s256_challenge_matches_rfc7636_test_vector() {
        // RFC 7636 Appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            s256_challenge(verifier.as_bytes()),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn sha256_hex_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn generated_verifier_has_expected_length_and_charset() {
        let verifier = generate_code_verifier();
        assert_eq!(verifier.len(), VERIFIER_LEN);
        assert!(
            verifier
                .bytes()
                .all(|byte| VERIFIER_CHARSET.contains(&byte)),
            "verifier must only contain RFC 7636 unreserved characters"
        );
    }

    #[test]
    fn generated_verifiers_differ() {
        assert_ne!(generate_code_verifier(), generate_code_verifier());
    }
}
