//! Webhook signature verification (Discord Ed25519 and Slack HMAC-SHA256).
//!
//! Validates request signatures for incoming webhooks:
//! - Discord: `X-Signature-Ed25519` and `X-Signature-Timestamp` headers
//! - Slack: `X-Slack-Signature` and `X-Slack-Request-Timestamp` headers
//!
//! See: <https://discord.com/developers/docs/interactions/overview#validating-security-request-headers>
//! See: <https://api.slack.com/authentication/verifying-requests-from-slack>

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

/// Verify a Slack webhook signature using HMAC-SHA256.
///
/// Slack signs each webhook request with HMAC-SHA256 using:
/// - basestring = `"v0:" + timestamp + ":" + body`
/// - signature = hex-encoded HMAC-SHA256(signing_secret, basestring)
/// - header = `"v0=" + signature` (in `X-Slack-Signature` header)
///
/// Includes staleness check: rejects requests with timestamps older than 5 minutes.
/// Returns `true` if the signature is valid, `false` on any error
/// (bad timing, mismatched signature, invalid format, etc.).
pub fn verify_slack_signature(
    signing_secret: &str,
    timestamp: &str,
    body: &[u8],
    signature_header: &str,
    now_secs: i64,
) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    // 1. Parse and check staleness (5-minute window)
    let ts: i64 = match timestamp.parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    if (now_secs - ts).abs() > 300 {
        return false;
    }

    // 2. Build the basestring: "v0:{timestamp}:{body}"
    let mut basestring = Vec::with_capacity(3 + timestamp.len() + 1 + body.len());
    basestring.extend_from_slice(b"v0:");
    basestring.extend_from_slice(timestamp.as_bytes());
    basestring.push(b':');
    basestring.extend_from_slice(body);

    // 3. Compute HMAC-SHA256
    let mut mac = match Hmac::<Sha256>::new_from_slice(signing_secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(&basestring);
    let computed = mac.finalize().into_bytes();
    let computed_hex = hex::encode(computed);
    let expected = format!("v0={}", computed_hex);

    // 4. Constant-time compare (avoids timing side-channels)
    use subtle::ConstantTimeEq;
    expected
        .as_bytes()
        .ct_eq(signature_header.as_bytes())
        .into()
}

/// Verify raw-body HMAC-SHA256 signature with a configurable prefix.
///
/// Computes `HMAC-SHA256(secret, body)` and compares against
/// `prefix + hex_digest` in constant time.
pub fn verify_hmac_sha256_prefixed(
    secret: &str,
    body: &[u8],
    signature_header: &str,
    prefix: &str,
) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use subtle::ConstantTimeEq;

    let mut mac = match Hmac::<Sha256>::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    let computed = mac.finalize().into_bytes();
    let computed_hex = hex::encode(computed);
    let expected = format!("{prefix}{computed_hex}");
    expected
        .as_bytes()
        .ct_eq(signature_header.as_bytes())
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

    // ── Category: HMAC-SHA256 Signature Verification (Slack) ────────────

    /// Helper: compute expected Slack signature for a given secret, timestamp, and body.
    fn sign_slack_message(signing_secret: &str, timestamp: &str, body: &[u8]) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let mut basestring = Vec::new();
        basestring.extend_from_slice(b"v0:");
        basestring.extend_from_slice(timestamp.as_bytes());
        basestring.push(b':');
        basestring.extend_from_slice(body);

        let mut mac = Hmac::<Sha256>::new_from_slice(signing_secret.as_bytes()).unwrap();
        mac.update(&basestring);
        let computed = mac.finalize().into_bytes();
        format!("v0={}", hex::encode(computed))
    }

    const SLACK_TEST_TS: i64 = 1234567890;

    #[test]
    fn test_slack_valid_signature_succeeds() {
        let signing_secret = "my-signing-secret";
        let timestamp = "1234567890";
        let body = b"token=xyzz0WbapA4vBCDEFasx0q6G&team_id=T1DC2JH3J";

        let signature = sign_slack_message(signing_secret, timestamp, body);
        assert!(verify_slack_signature(
            signing_secret,
            timestamp,
            body,
            &signature,
            SLACK_TEST_TS
        ));
    }

    #[test]
    fn test_slack_tampered_body_fails() {
        let signing_secret = "my-signing-secret";
        let timestamp = "1234567890";
        let original_body = b"token=xyzz0WbapA4vBCDEFasx0q6G&team_id=T1DC2JH3J";
        let tampered_body = b"token=MODIFIED&team_id=T1DC2JH3J";

        let signature = sign_slack_message(signing_secret, timestamp, original_body);
        assert!(
            !verify_slack_signature(
                signing_secret,
                timestamp,
                tampered_body,
                &signature,
                SLACK_TEST_TS
            ),
            "Signature for different body should fail"
        );
    }

    #[test]
    fn test_slack_tampered_timestamp_fails() {
        let signing_secret = "my-signing-secret";
        let timestamp = "1234567890";
        let body = b"token=xyzz0WbapA4vBCDEFasx0q6G&team_id=T1DC2JH3J";

        let signature = sign_slack_message(signing_secret, timestamp, body);
        assert!(
            !verify_slack_signature(
                signing_secret,
                "9999999999", // Different timestamp in signature
                body,
                &signature,
                SLACK_TEST_TS
            ),
            "Signature with wrong timestamp should fail"
        );
    }

    #[test]
    fn test_slack_tampered_signature_fails() {
        let signing_secret = "my-signing-secret";
        let timestamp = "1234567890";
        let body = b"token=xyzz0WbapA4vBCDEFasx0q6G&team_id=T1DC2JH3J";

        let signature = sign_slack_message(signing_secret, timestamp, body);
        // Flip a byte in the signature hex (change first char after "v0=")
        let chars: Vec<char> = signature.chars().collect();
        let mut new_chars = chars.clone();
        if chars.len() > 3 {
            new_chars[3] = if chars[3] == 'a' { 'b' } else { 'a' };
        }
        let modified_sig: String = new_chars.iter().collect();

        assert!(
            !verify_slack_signature(
                signing_secret,
                timestamp,
                body,
                &modified_sig,
                SLACK_TEST_TS
            ),
            "Tampered signature should fail"
        );
    }

    #[test]
    fn test_hmac_sha256_prefixed_valid() {
        let secret = "github-secret";
        let body = br#"{"action":"opened"}"#;
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac key");
        mac.update(body);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert!(verify_hmac_sha256_prefixed(secret, body, &sig, "sha256="));
        assert!(!verify_hmac_sha256_prefixed(
            secret,
            body,
            "sha256=deadbeef",
            "sha256="
        ));
    }

    #[test]
    fn test_slack_stale_timestamp_rejected() {
        let signing_secret = "my-signing-secret";
        let timestamp = "1234567890";
        let body = b"token=xyzz0WbapA4vBCDEFasx0q6G";

        let signature = sign_slack_message(signing_secret, timestamp, body);
        // now_secs is 400 seconds after timestamp — too stale
        assert!(
            !verify_slack_signature(
                signing_secret,
                timestamp,
                body,
                &signature,
                SLACK_TEST_TS + 400
            ),
            "Stale timestamp (400s old) should be rejected"
        );
    }

    #[test]
    fn test_slack_future_timestamp_rejected() {
        let signing_secret = "my-signing-secret";
        let timestamp = "1234567890";
        let body = b"token=xyzz0WbapA4vBCDEFasx0q6G";

        let signature = sign_slack_message(signing_secret, timestamp, body);
        // now_secs is 400 seconds before timestamp — future
        assert!(
            !verify_slack_signature(
                signing_secret,
                timestamp,
                body,
                &signature,
                SLACK_TEST_TS - 400
            ),
            "Future timestamp (400s ahead) should be rejected"
        );
    }

    #[test]
    fn test_slack_boundary_300s_accepted() {
        let signing_secret = "my-signing-secret";
        let timestamp = "1234567890";
        let body = b"token=xyzz0WbapA4vBCDEFasx0q6G";

        let signature = sign_slack_message(signing_secret, timestamp, body);
        // Exactly 300 seconds difference — should be accepted
        assert!(
            verify_slack_signature(
                signing_secret,
                timestamp,
                body,
                &signature,
                SLACK_TEST_TS + 300
            ),
            "Timestamp exactly 300s old should be accepted"
        );
    }

    #[test]
    fn test_slack_boundary_301s_rejected() {
        let signing_secret = "my-signing-secret";
        let timestamp = "1234567890";
        let body = b"token=xyzz0WbapA4vBCDEFasx0q6G";

        let signature = sign_slack_message(signing_secret, timestamp, body);
        // 301 seconds difference — should be rejected
        assert!(
            !verify_slack_signature(
                signing_secret,
                timestamp,
                body,
                &signature,
                SLACK_TEST_TS + 301
            ),
            "Timestamp 301s old should be rejected"
        );
    }

    #[test]
    fn test_slack_non_numeric_timestamp_rejected() {
        let signing_secret = "my-signing-secret";
        let body = b"token=xyzz0WbapA4vBCDEFasx0q6G";

        assert!(
            !verify_slack_signature(signing_secret, "not-a-number", body, "v0=abc123", 0),
            "Non-numeric timestamp should be rejected"
        );
    }

    #[test]
    fn test_slack_missing_v0_prefix_fails() {
        let signing_secret = "my-signing-secret";
        let timestamp = "1234567890";
        let body = b"token=xyzz0WbapA4vBCDEFasx0q6G";

        let signature = sign_slack_message(signing_secret, timestamp, body);
        // Remove the "v0=" prefix
        let bad_sig = signature.strip_prefix("v0=").unwrap_or(&signature);

        assert!(
            !verify_slack_signature(signing_secret, timestamp, body, bad_sig, SLACK_TEST_TS),
            "Missing v0= prefix should fail"
        );
    }

    #[test]
    fn test_slack_wrong_signing_secret_fails() {
        let secret_a = "secret-a";
        let secret_b = "secret-b";
        let timestamp = "1234567890";
        let body = b"token=xyzz0WbapA4vBCDEFasx0q6G";

        let signature = sign_slack_message(secret_a, timestamp, body);
        // Try to verify with a different secret
        assert!(
            !verify_slack_signature(secret_b, timestamp, body, &signature, SLACK_TEST_TS),
            "Signature from different secret should fail"
        );
    }

    #[test]
    fn test_slack_empty_body_valid() {
        let signing_secret = "my-signing-secret";
        let timestamp = "1234567890";
        let body = b"";

        let signature = sign_slack_message(signing_secret, timestamp, body);
        assert!(
            verify_slack_signature(signing_secret, timestamp, body, &signature, SLACK_TEST_TS),
            "Empty body with valid signature should succeed"
        );
    }

    #[test]
    fn test_slack_negative_timestamp_rejected() {
        let signing_secret = "my-signing-secret";
        let body = b"token=xyzz0WbapA4vBCDEFasx0q6G";

        assert!(
            !verify_slack_signature(signing_secret, "-1", body, "v0=abc123", 0),
            "Negative timestamp should be rejected"
        );
    }

    #[test]
    fn test_slack_empty_timestamp_rejected() {
        let signing_secret = "my-signing-secret";
        let body = b"token=xyzz0WbapA4vBCDEFasx0q6G";

        assert!(
            !verify_slack_signature(signing_secret, "", body, "v0=abc123", 0),
            "Empty timestamp should be rejected"
        );
    }
}
