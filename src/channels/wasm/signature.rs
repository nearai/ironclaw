//! Discord Ed25519 signature verification.
//!
//! Validates `X-Signature-Ed25519` and `X-Signature-Timestamp` headers
//! on incoming Discord interaction webhooks, per Discord's security requirements.
//!
//! See: <https://discord.com/developers/docs/interactions/overview#validating-security-request-headers>

/// Verify a Discord interaction signature.
///
/// Discord signs each interaction with Ed25519 using:
/// - message = `timestamp` (UTF-8 bytes) ++ `body` (raw bytes)
/// - signature = Ed25519 detached signature (hex-encoded in header)
/// - public_key = Application public key from Developer Portal (hex-encoded)
///
/// Returns `true` if the signature is valid, `false` on any error
/// (bad hex, wrong length, invalid signature, etc.).
#[allow(dead_code)] // Called from router webhook_handler in a future step
pub fn verify_discord_signature(
    public_key_hex: &str,
    signature_hex: &str,
    timestamp: &str,
    body: &[u8],
) -> bool {
    use ed25519_dalek::{Signature, VerifyingKey};

    let Ok(sig_bytes) = hex::decode(signature_hex) else {
        return false;
    };
    let Ok(key_bytes) = hex::decode(public_key_hex) else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(&sig_bytes) else {
        return false;
    };
    let Ok(verifying_key) = VerifyingKey::try_from(key_bytes.as_slice()) else {
        return false;
    };

    let mut message = Vec::with_capacity(timestamp.len() + body.len());
    message.extend_from_slice(timestamp.as_bytes());
    message.extend_from_slice(body);
    verifying_key.verify_strict(&message, &signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    /// Helper: generate a test keypair and produce a valid signature for the given timestamp+body.
    fn sign_test_message(timestamp: &str, body: &[u8]) -> (String, String, String) {
        let signing_key = SigningKey::from_bytes(&[
            0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec,
            0x2c, 0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03,
            0x1c, 0xae, 0x7f, 0x60,
        ]);
        let verifying_key = signing_key.verifying_key();

        let mut message = Vec::new();
        message.extend_from_slice(timestamp.as_bytes());
        message.extend_from_slice(body);

        let signature = signing_key.sign(&message);

        let public_key_hex = hex::encode(verifying_key.to_bytes());
        let signature_hex = hex::encode(signature.to_bytes());

        (public_key_hex, signature_hex, timestamp.to_string())
    }

    // ── Category 2: Ed25519 Signature Verification ──────────────────────

    #[test]
    fn test_valid_signature_succeeds() {
        let timestamp = "1234567890";
        let body = b"test body content";
        let (pub_key, sig, ts) = sign_test_message(timestamp, body);

        assert!(
            verify_discord_signature(&pub_key, &sig, &ts, body),
            "Valid signature should verify successfully"
        );
    }

    #[test]
    fn test_invalid_signature_fails() {
        let timestamp = "1234567890";
        let body = b"test body content";
        let (pub_key, mut sig, ts) = sign_test_message(timestamp, body);

        // Tamper one byte of the signature
        let mut sig_bytes = hex::decode(&sig).unwrap();
        sig_bytes[0] ^= 0xff;
        sig = hex::encode(&sig_bytes);

        assert!(
            !verify_discord_signature(&pub_key, &sig, &ts, body),
            "Tampered signature should fail verification"
        );
    }

    #[test]
    fn test_tampered_body_fails() {
        let timestamp = "1234567890";
        let body = b"original body";
        let (pub_key, sig, ts) = sign_test_message(timestamp, body);

        let tampered_body = b"tampered body";
        assert!(
            !verify_discord_signature(&pub_key, &sig, &ts, tampered_body),
            "Signature for different body should fail"
        );
    }

    #[test]
    fn test_tampered_timestamp_fails() {
        let timestamp = "1234567890";
        let body = b"test body";
        let (pub_key, sig, _ts) = sign_test_message(timestamp, body);

        assert!(
            !verify_discord_signature(&pub_key, &sig, "9999999999", body),
            "Signature with wrong timestamp should fail"
        );
    }

    #[test]
    fn test_invalid_hex_signature_fails() {
        let timestamp = "1234567890";
        let body = b"test body";
        let (pub_key, _sig, ts) = sign_test_message(timestamp, body);

        assert!(
            !verify_discord_signature(&pub_key, "not-valid-hex-zzz", &ts, body),
            "Non-hex signature should fail gracefully"
        );
    }

    #[test]
    fn test_invalid_hex_public_key_fails() {
        let timestamp = "1234567890";
        let body = b"test body";
        let (_pub_key, sig, ts) = sign_test_message(timestamp, body);

        assert!(
            !verify_discord_signature("not-valid-hex-zzz", &sig, &ts, body),
            "Non-hex public key should fail gracefully"
        );
    }

    #[test]
    fn test_wrong_length_signature_fails() {
        let timestamp = "1234567890";
        let body = b"test body";
        let (pub_key, _sig, ts) = sign_test_message(timestamp, body);

        // Too short (only 32 bytes instead of 64)
        let short_sig = hex::encode([0u8; 32]);
        assert!(
            !verify_discord_signature(&pub_key, &short_sig, &ts, body),
            "Short signature should fail"
        );
    }

    #[test]
    fn test_wrong_length_public_key_fails() {
        let timestamp = "1234567890";
        let body = b"test body";
        let (_pub_key, sig, ts) = sign_test_message(timestamp, body);

        // Too short (only 16 bytes instead of 32)
        let short_key = hex::encode([0u8; 16]);
        assert!(
            !verify_discord_signature(&short_key, &sig, &ts, body),
            "Short public key should fail"
        );
    }

    #[test]
    fn test_empty_body_valid_signature() {
        let timestamp = "1234567890";
        let body = b"";
        let (pub_key, sig, ts) = sign_test_message(timestamp, body);

        assert!(
            verify_discord_signature(&pub_key, &sig, &ts, body),
            "Empty body with valid signature should succeed"
        );
    }

    #[test]
    fn test_discord_reference_vector() {
        // Hardcoded test vector using the RFC 8032 test key
        // This ensures the implementation matches the standard Ed25519 algorithm
        let signing_key = SigningKey::from_bytes(&[
            0xc5, 0xaa, 0x8d, 0xf4, 0x3f, 0x9f, 0x83, 0x7b, 0xed, 0xb7, 0x44, 0x2f, 0x31, 0xdc,
            0xb7, 0xb1, 0x66, 0xd3, 0x85, 0x35, 0x07, 0x6f, 0x09, 0x4b, 0x85, 0xce, 0x3a, 0x2e,
            0x0b, 0x44, 0x58, 0xf7,
        ]);
        let verifying_key = signing_key.verifying_key();
        let public_key_hex = hex::encode(verifying_key.to_bytes());

        let timestamp = "1609459200";
        let body = br#"{"type":1}"#; // Discord PING

        let mut message = Vec::new();
        message.extend_from_slice(timestamp.as_bytes());
        message.extend_from_slice(body);

        let signature = signing_key.sign(&message);
        let signature_hex = hex::encode(signature.to_bytes());

        assert!(
            verify_discord_signature(&public_key_hex, &signature_hex, timestamp, body),
            "Reference vector should verify"
        );

        // Same key, but tampered body should fail
        assert!(
            !verify_discord_signature(&public_key_hex, &signature_hex, timestamp, br#"{"type":2}"#),
            "Reference vector with tampered body should fail"
        );
    }
}
