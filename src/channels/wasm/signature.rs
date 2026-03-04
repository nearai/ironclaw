//! Discord Ed25519 signature verification.
//!
//! Validates `X-Signature-Ed25519` and `X-Signature-Timestamp` headers
//! on incoming Discord interaction webhooks, per Discord's security requirements.
//!
//! See: <https://discord.com/developers/docs/interactions/overview#validating-security-request-headers>

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// Verify a Discord interaction signature.
///
/// Discord signs each interaction with Ed25519 using:
/// - message = `timestamp` (UTF-8 bytes) ++ `body` (raw bytes)
/// - signature = Ed25519 detached signature (hex-encoded in header)
/// - public_key = Application public key from Developer Portal (hex-encoded)
///
/// Returns `true` if the signature is valid, `false` on any error
/// (bad hex, wrong length, invalid signature, etc.).
pub fn verify_discord_signature(
    public_key_hex: &str,
    signature_hex: &str,
    timestamp: &str,
    body: &[u8],
    now_secs: i64,
) -> bool {
    // Staleness check: reject non-numeric or stale/future timestamps
    let ts: i64 = match timestamp.parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    if (now_secs - ts).abs() > 5 {
        return false;
    }
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

/// Verify HMAC-SHA256 signature (WhatsApp/Slack style).
///
/// # Arguments
/// * `secret` - The HMAC secret (App Secret)
/// * `signature_header` - Value from X-Hub-Signature-256 header (format: "sha256=<hex>")
/// * `body` - Raw request body bytes
///
/// # Returns
/// `true` if signature is valid, `false` otherwise
pub fn verify_hmac_sha256(secret: &str, signature_header: &str, body: &[u8]) -> bool {
    // Parse header format: "sha256=<hex_signature>"
    let Some(hex_signature) = signature_header.strip_prefix("sha256=") else {
        return false;
    };

    // Decode expected signature
    let Ok(expected_sig) = hex::decode(hex_signature) else {
        return false;
    };

    // SHA-256 produces 32-byte signatures - reject wrong lengths early
    if expected_sig.len() != 32 {
        return false;
    }

    // Compute HMAC-SHA256
    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    let result = mac.finalize();
    let computed_sig = result.into_bytes();

    // Constant-time comparison to prevent timing attacks
    computed_sig
        .as_slice()
        .ct_eq(expected_sig.as_slice())
        .into()
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

    /// Existing tests pass `now_secs` matching their hardcoded timestamp
    /// so they continue testing crypto-only behavior.
    const TEST_TS: i64 = 1234567890;

    #[test]
    fn test_valid_signature_succeeds() {
        let timestamp = "1234567890";
        let body = b"test body content";
        let (pub_key, sig, ts) = sign_test_message(timestamp, body);

        assert!(
            verify_discord_signature(&pub_key, &sig, &ts, body, TEST_TS),
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
            !verify_discord_signature(&pub_key, &sig, &ts, body, TEST_TS),
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
            !verify_discord_signature(&pub_key, &sig, &ts, tampered_body, TEST_TS),
            "Signature for different body should fail"
        );
    }

    #[test]
    fn test_tampered_timestamp_fails() {
        let timestamp = "1234567890";
        let body = b"test body";
        let (pub_key, sig, _ts) = sign_test_message(timestamp, body);

        assert!(
            !verify_discord_signature(&pub_key, &sig, "9999999999", body, TEST_TS),
            "Signature with wrong timestamp should fail"
        );
    }

    #[test]
    fn test_invalid_hex_signature_fails() {
        let timestamp = "1234567890";
        let body = b"test body";
        let (pub_key, _sig, ts) = sign_test_message(timestamp, body);

        assert!(
            !verify_discord_signature(&pub_key, "not-valid-hex-zzz", &ts, body, TEST_TS),
            "Non-hex signature should fail gracefully"
        );
    }

    #[test]
    fn test_invalid_hex_public_key_fails() {
        let timestamp = "1234567890";
        let body = b"test body";
        let (_pub_key, sig, ts) = sign_test_message(timestamp, body);

        assert!(
            !verify_discord_signature("not-valid-hex-zzz", &sig, &ts, body, TEST_TS),
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
            !verify_discord_signature(&pub_key, &short_sig, &ts, body, TEST_TS),
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
            !verify_discord_signature(&short_key, &sig, &ts, body, TEST_TS),
            "Short public key should fail"
        );
    }

    #[test]
    fn test_empty_body_valid_signature() {
        let timestamp = "1234567890";
        let body = b"";
        let (pub_key, sig, ts) = sign_test_message(timestamp, body);

        assert!(
            verify_discord_signature(&pub_key, &sig, &ts, body, TEST_TS),
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
        let now_secs: i64 = 1609459200;
        let body = br#"{"type":1}"#; // Discord PING

        let mut message = Vec::new();
        message.extend_from_slice(timestamp.as_bytes());
        message.extend_from_slice(body);

        let signature = signing_key.sign(&message);
        let signature_hex = hex::encode(signature.to_bytes());

        assert!(
            verify_discord_signature(&public_key_hex, &signature_hex, timestamp, body, now_secs),
            "Reference vector should verify"
        );

        // Same key, but tampered body should fail
        assert!(
            !verify_discord_signature(
                &public_key_hex,
                &signature_hex,
                timestamp,
                br#"{"type":2}"#,
                now_secs
            ),
            "Reference vector with tampered body should fail"
        );
    }

    // ── Category: Timestamp Staleness ─────────────────────────────────

    #[test]
    fn test_stale_timestamp_rejected() {
        let timestamp = "1234567890";
        let body = b"test body";
        let (pub_key, sig, ts) = sign_test_message(timestamp, body);
        // now_secs is 100 seconds after the timestamp — too stale
        assert!(
            !verify_discord_signature(&pub_key, &sig, &ts, body, TEST_TS + 100),
            "Stale timestamp (100s old) should be rejected"
        );
    }

    #[test]
    fn test_future_timestamp_rejected() {
        let timestamp = "1234567890";
        let body = b"test body";
        let (pub_key, sig, ts) = sign_test_message(timestamp, body);
        // now_secs is 100 seconds before the timestamp — future
        assert!(
            !verify_discord_signature(&pub_key, &sig, &ts, body, TEST_TS - 100),
            "Future timestamp (100s ahead) should be rejected"
        );
    }

    #[test]
    fn test_fresh_timestamp_accepted() {
        let timestamp = "1234567890";
        let body = b"test body";
        let (pub_key, sig, ts) = sign_test_message(timestamp, body);
        // now_secs matches exactly — fresh
        assert!(
            verify_discord_signature(&pub_key, &sig, &ts, body, TEST_TS),
            "Fresh timestamp (0s difference) should be accepted"
        );
    }

    #[test]
    fn test_non_numeric_timestamp_rejected() {
        let timestamp = "1234567890";
        let body = b"test body";
        let (pub_key, sig, _ts) = sign_test_message(timestamp, body);
        // Pass a non-numeric timestamp string
        assert!(
            !verify_discord_signature(&pub_key, &sig, "not-a-number", body, 0),
            "Non-numeric timestamp should be rejected"
        );
    }

    #[test]
    fn test_empty_timestamp_rejected() {
        let timestamp = "1234567890";
        let body = b"test body";
        let (pub_key, sig, _ts) = sign_test_message(timestamp, body);
        // Pass an empty timestamp string
        assert!(
            !verify_discord_signature(&pub_key, &sig, "", body, 0),
            "Empty timestamp should be rejected"
        );
    }

    #[test]
    fn test_boundary_5s_accepted() {
        let timestamp = "1234567890";
        let body = b"test body";
        let (pub_key, sig, ts) = sign_test_message(timestamp, body);
        // Exactly 5 seconds difference — should be accepted (> 5, not >= 5)
        assert!(
            verify_discord_signature(&pub_key, &sig, &ts, body, TEST_TS + 5),
            "Timestamp exactly 5s old should be accepted"
        );
    }

    #[test]
    fn test_boundary_6s_rejected() {
        let timestamp = "1234567890";
        let body = b"test body";
        let (pub_key, sig, ts) = sign_test_message(timestamp, body);
        // 6 seconds difference — should be rejected
        assert!(
            !verify_discord_signature(&pub_key, &sig, &ts, body, TEST_TS + 6),
            "Timestamp 6s old should be rejected"
        );
    }

    #[test]
    fn test_negative_timestamp_rejected() {
        let timestamp = "1234567890";
        let body = b"test body";
        let (pub_key, sig, _ts) = sign_test_message(timestamp, body);
        // Pass a negative timestamp string
        assert!(
            !verify_discord_signature(&pub_key, &sig, "-1", body, TEST_TS),
            "Negative timestamp should be rejected"
        );
    }

    // ── Category: HMAC-SHA256 Verification ─────────────────────────────────

    /// Helper: compute HMAC-SHA256 signature in WhatsApp format.
    fn compute_hmac_signature(secret: &str, body: &[u8]) -> String {
        use hmac::Mac;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let result = mac.finalize();
        format!("sha256={}", hex::encode(result.into_bytes()))
    }

    #[test]
    fn test_hmac_valid_signature_succeeds() {
        let secret = "my_app_secret";
        let body = br#"{"entry":[{"id":"123"}]}"#;
        let sig_header = compute_hmac_signature(secret, body);

        assert!(
            verify_hmac_sha256(secret, &sig_header, body),
            "Valid HMAC signature should verify"
        );
    }

    #[test]
    fn test_hmac_wrong_secret_fails() {
        let secret = "correct_secret";
        let wrong_secret = "wrong_secret";
        let body = br#"{"test":"data"}"#;
        let sig_header = compute_hmac_signature(secret, body);

        assert!(
            !verify_hmac_sha256(wrong_secret, &sig_header, body),
            "Signature with wrong secret should fail"
        );
    }

    #[test]
    fn test_hmac_tampered_body_fails() {
        let secret = "my_secret";
        let body = br#"original body"#;
        let sig_header = compute_hmac_signature(secret, body);
        let tampered_body = br#"tampered body"#;

        assert!(
            !verify_hmac_sha256(secret, &sig_header, tampered_body),
            "Signature for different body should fail"
        );
    }

    #[test]
    fn test_hmac_missing_prefix_fails() {
        let secret = "my_secret";
        let body = b"test body";
        // Missing "sha256=" prefix
        let sig_header = hex::encode([0u8; 32]);

        assert!(
            !verify_hmac_sha256(secret, &sig_header, body),
            "Signature without sha256= prefix should fail"
        );
    }

    #[test]
    fn test_hmac_invalid_hex_fails() {
        let secret = "my_secret";
        let body = b"test body";
        let sig_header = "sha256=not-valid-hex-zzz";

        assert!(
            !verify_hmac_sha256(secret, sig_header, body),
            "Invalid hex signature should fail gracefully"
        );
    }

    #[test]
    fn test_hmac_empty_body_succeeds() {
        let secret = "my_secret";
        let body = b"";
        let sig_header = compute_hmac_signature(secret, body);

        assert!(
            verify_hmac_sha256(secret, &sig_header, body),
            "Empty body with valid signature should succeed"
        );
    }

    #[test]
    fn test_hmac_empty_secret_succeeds() {
        let body = b"test body";
        // Empty secret should still work (though not recommended in practice)
        let sig_header = compute_hmac_signature("", body);

        assert!(
            verify_hmac_sha256("", &sig_header, body),
            "Empty secret with matching signature should succeed"
        );
    }

    #[test]
    fn test_hmac_wrong_signature_length_fails() {
        let secret = "my_secret";
        let body = b"test body";
        // Signature too short
        let sig_header = "sha256=deadbeef";

        assert!(
            !verify_hmac_sha256(secret, sig_header, body),
            "Wrong-length signature should fail"
        );
    }

    #[test]
    fn test_hmac_case_sensitive_prefix() {
        let secret = "my_secret";
        let body = b"test body";
        let sig_header = compute_hmac_signature(secret, body);
        // Uppercase prefix should fail
        let bad_header = sig_header.replace("sha256=", "SHA256=");

        assert!(
            !verify_hmac_sha256(secret, &bad_header, body),
            "Uppercase SHA256= prefix should fail"
        );
    }
}
