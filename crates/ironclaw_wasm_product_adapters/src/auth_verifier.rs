//! Webhook/protocol authentication verifiers used by the host.
//!
//! Adapters never call these directly. The host glue:
//!
//! 1. Receives the webhook request.
//! 2. Selects a verifier based on the adapter's [`AuthRequirement`].
//! 3. Calls `verify`. On success the host calls one of the
//!    `ironclaw_product_adapters::auth::mark_*_verified` helpers to mint a
//!    sealed `Verified` evidence and only then hands the payload to the
//!    adapter.
//!
//! The verifier outcome is structured:
//! * `Verified { subject }` — proceed to adapter parse.
//! * `Failed(failure)` — return 401/403 to the protocol; do not touch the
//!   workflow.
//!
//! Verifiers in this module compute digests with constant-time comparison
//! (`subtle::ConstantTimeEq`) to avoid timing oracles.

use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use ironclaw_product_adapters::ProtocolAuthFailure;
use ironclaw_product_adapters::redaction::RedactedString;
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// Default replay-attack window for HMAC verifiers (5 minutes). Matches
/// Slack's documented recommendation. Configurable per-installation via
/// [`HmacWebhookAuth::max_age`].
pub const DEFAULT_HMAC_MAX_AGE_SECS: u64 = 300;

/// Clock seam used by [`HmacWebhookAuth`]. Production hosts use
/// [`SystemClock`]; tests inject a [`FixedClock`] to drive the timestamp
/// window deterministically.
pub trait Clock: Send + Sync {
    /// Current time as seconds since the Unix epoch.
    fn now_unix_seconds(&self) -> u64;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// Test-only clock that returns a fixed timestamp.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock(pub u64);

impl Clock for FixedClock {
    fn now_unix_seconds(&self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationOutcome {
    Verified { subject: String },
    Failed { failure: ProtocolAuthFailure },
}

pub trait WebhookAuthVerifier {
    /// Verify a webhook request given the request headers and body. The
    /// returned `subject` is an opaque attestation identifier (e.g. the bot
    /// installation id) suitable for inclusion in a `VerifiedAuthClaim`.
    fn verify(&self, headers: &http::HeaderMap, body: &[u8]) -> VerificationOutcome;
}

/// Slack-style request-signature HMAC-SHA-256 verifier.
///
/// Verification enforces three properties:
///
/// 1. The HMAC digest matches the expected signature (constant-time).
/// 2. The supplied timestamp is parseable as a Unix epoch second.
/// 3. The timestamp is within `max_age_secs` of the verifier's clock —
///    rejecting both stale captures (replay attacks) and timestamps far
///    in the future (clock-drift / forgery attempts).
pub struct HmacWebhookAuth {
    pub signature_header: String,
    pub timestamp_header: String,
    pub signing_secret: Vec<u8>,
    pub subject: String,
    /// Maximum acceptable absolute distance between the request timestamp
    /// and the verifier's current clock, in seconds. Default
    /// [`DEFAULT_HMAC_MAX_AGE_SECS`] (5 minutes).
    pub max_age_secs: u64,
    /// Clock seam — production passes [`SystemClock`], tests pass
    /// [`FixedClock`]. Boxed so installations can swap implementations
    /// without making the verifier generic.
    pub clock: Box<dyn Clock>,
}

impl HmacWebhookAuth {
    pub fn new(
        signature_header: impl Into<String>,
        timestamp_header: impl Into<String>,
        signing_secret: Vec<u8>,
        subject: impl Into<String>,
    ) -> Self {
        Self {
            signature_header: signature_header.into(),
            timestamp_header: timestamp_header.into(),
            signing_secret,
            subject: subject.into(),
            max_age_secs: DEFAULT_HMAC_MAX_AGE_SECS,
            clock: Box::new(SystemClock),
        }
    }

    pub fn with_max_age(mut self, max_age_secs: u64) -> Self {
        self.max_age_secs = max_age_secs;
        self
    }

    pub fn with_clock(mut self, clock: Box<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }
}

impl WebhookAuthVerifier for HmacWebhookAuth {
    fn verify(&self, headers: &http::HeaderMap, body: &[u8]) -> VerificationOutcome {
        let Some(signature) = headers
            .get(self.signature_header.as_str())
            .and_then(|v| v.to_str().ok())
        else {
            return VerificationOutcome::Failed {
                failure: ProtocolAuthFailure::Missing,
            };
        };
        let Some(timestamp_str) = headers
            .get(self.timestamp_header.as_str())
            .and_then(|v| v.to_str().ok())
        else {
            return VerificationOutcome::Failed {
                failure: ProtocolAuthFailure::Missing,
            };
        };
        // Replay-window check before computing HMAC. Reject stale or
        // far-future timestamps; both are forgery attempts. The window is
        // symmetric: |now - ts| > max_age_secs => fail.
        //
        // Parse the (untrusted) header value as i128, not i64: in
        // overflow-checked builds (default for `cargo test` / debug),
        // `(now_secs - timestamp_secs).abs()` with `i64` panics on
        // `timestamp_secs = i64::MIN` (subtraction overflows for any
        // nonnegative `now_secs`, and `i64::MIN.abs()` itself overflows).
        // A pathological header like `-9223372036854775808` would crash
        // the verifier before it could return `Malformed`. `i128` covers
        // the full `i64` value range with several orders of magnitude of
        // headroom, so neither the subtraction nor the abs() can overflow.
        let Ok(timestamp_secs) = timestamp_str.parse::<i128>() else {
            return VerificationOutcome::Failed {
                failure: ProtocolAuthFailure::Malformed,
            };
        };
        let now_secs = i128::from(self.clock.now_unix_seconds());
        let max_age = i128::from(self.max_age_secs);
        let drift = (now_secs - timestamp_secs).abs();
        if drift > max_age {
            return VerificationOutcome::Failed {
                failure: ProtocolAuthFailure::Other {
                    detail: RedactedString::new(format!(
                        "request timestamp drift {drift}s exceeds {max_age}s window"
                    )),
                },
            };
        }

        if self.signing_secret.is_empty() {
            return VerificationOutcome::Failed {
                failure: ProtocolAuthFailure::Malformed,
            };
        }

        let signed_payload = format!("v0:{timestamp_str}:");
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(&self.signing_secret) else {
            // HMAC-SHA-256 accepts arbitrary non-empty key lengths in the
            // algorithm spec; any error here is a malformed configuration.
            return VerificationOutcome::Failed {
                failure: ProtocolAuthFailure::Malformed,
            };
        };
        mac.update(signed_payload.as_bytes());
        mac.update(body);
        let expected_bytes = mac.finalize().into_bytes();
        let expected = hex::encode(expected_bytes);
        let expected_full = format!("v0={expected}");
        if !bool::from(expected_full.as_bytes().ct_eq(signature.as_bytes())) {
            return VerificationOutcome::Failed {
                failure: ProtocolAuthFailure::SignatureMismatch,
            };
        }
        VerificationOutcome::Verified {
            subject: self.subject.clone(),
        }
    }
}

/// Telegram-style shared-secret-header verifier.
pub struct SharedSecretHeaderAuth {
    pub header_name: String,
    pub expected_secret: String,
    pub subject: String,
}

impl WebhookAuthVerifier for SharedSecretHeaderAuth {
    fn verify(&self, headers: &http::HeaderMap, _body: &[u8]) -> VerificationOutcome {
        // Fail closed on misconfigured installation: an empty configured
        // secret would `ct_eq("", "")` to true, letting an attacker who
        // knows the header name authenticate with an empty value. Mirrors
        // the `HmacWebhookAuth` empty-signing-secret check above.
        if self.expected_secret.is_empty() {
            return VerificationOutcome::Failed {
                failure: ProtocolAuthFailure::Malformed,
            };
        }
        let Some(received) = headers
            .get(self.header_name.as_str())
            .and_then(|v| v.to_str().ok())
        else {
            return VerificationOutcome::Failed {
                failure: ProtocolAuthFailure::Missing,
            };
        };
        if !bool::from(received.as_bytes().ct_eq(self.expected_secret.as_bytes())) {
            return VerificationOutcome::Failed {
                failure: ProtocolAuthFailure::SharedSecretMismatch,
            };
        }
        VerificationOutcome::Verified {
            subject: self.subject.clone(),
        }
    }
}

mod hex {
    use std::fmt::Write as _;

    pub(super) fn encode(bytes: impl AsRef<[u8]>) -> String {
        let mut out = String::with_capacity(bytes.as_ref().len() * 2);
        for byte in bytes.as_ref() {
            let _ = write!(&mut out, "{byte:02x}");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderMap;
    use http::header::HeaderValue;

    fn header_map(entries: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in entries {
            map.insert(
                http::header::HeaderName::from_bytes(k.as_bytes()).expect("name"),
                HeaderValue::from_str(v).expect("value"),
            );
        }
        map
    }

    #[test]
    fn shared_secret_header_verifies_match() {
        let verifier = SharedSecretHeaderAuth {
            header_name: "X-Telegram-Bot-Api-Secret-Token".into(),
            expected_secret: "topsecret".into(),
            subject: "telegram_install_alpha".into(),
        };
        let headers = header_map(&[("X-Telegram-Bot-Api-Secret-Token", "topsecret")]);
        match verifier.verify(&headers, b"") {
            VerificationOutcome::Verified { subject } => {
                assert_eq!(subject, "telegram_install_alpha");
            }
            other => panic!("expected Verified, got {other:?}"),
        }
    }

    #[test]
    fn shared_secret_header_rejects_mismatch() {
        let verifier = SharedSecretHeaderAuth {
            header_name: "X-Telegram-Bot-Api-Secret-Token".into(),
            expected_secret: "topsecret".into(),
            subject: "telegram_install_alpha".into(),
        };
        let headers = header_map(&[("X-Telegram-Bot-Api-Secret-Token", "wrong")]);
        match verifier.verify(&headers, b"") {
            VerificationOutcome::Failed { failure } => {
                assert!(matches!(failure, ProtocolAuthFailure::SharedSecretMismatch));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn shared_secret_header_rejects_missing() {
        let verifier = SharedSecretHeaderAuth {
            header_name: "X-Telegram-Bot-Api-Secret-Token".into(),
            expected_secret: "topsecret".into(),
            subject: "telegram_install_alpha".into(),
        };
        let headers = header_map(&[]);
        match verifier.verify(&headers, b"") {
            VerificationOutcome::Failed { failure } => {
                assert!(matches!(failure, ProtocolAuthFailure::Missing));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn shared_secret_header_rejects_empty_expected_secret_as_malformed_config() {
        // Misconfigured installation: empty `expected_secret`. Without the
        // fail-closed guard, `ct_eq("", "")` returns true and any request
        // with an empty header value would authenticate. Verifier must
        // reject as `Malformed`, mirroring the HMAC empty-signing-secret
        // check.
        let verifier = SharedSecretHeaderAuth {
            header_name: "X-Telegram-Bot-Api-Secret-Token".into(),
            expected_secret: String::new(),
            subject: "telegram_install_alpha".into(),
        };
        let headers = header_map(&[("X-Telegram-Bot-Api-Secret-Token", "")]);
        match verifier.verify(&headers, b"") {
            VerificationOutcome::Failed { failure } => {
                assert!(matches!(failure, ProtocolAuthFailure::Malformed));
            }
            other => panic!("expected Failed(Malformed), got {other:?}"),
        }
        // Independent of what header value is sent — the misconfiguration
        // rejection must precede any per-request comparison.
        let with_value = header_map(&[("X-Telegram-Bot-Api-Secret-Token", "anything")]);
        match verifier.verify(&with_value, b"") {
            VerificationOutcome::Failed { failure } => {
                assert!(matches!(failure, ProtocolAuthFailure::Malformed));
            }
            other => panic!("expected Failed(Malformed), got {other:?}"),
        }
    }

    fn build_signed_request(
        secret: &[u8],
        timestamp: &str,
        body: &[u8],
    ) -> (String, http::HeaderMap) {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("hmac key");
        mac.update(format!("v0:{timestamp}:").as_bytes());
        mac.update(body);
        let digest_hex = hex::encode(mac.finalize().into_bytes());
        let signature = format!("v0={digest_hex}");
        let headers = header_map(&[
            ("X-Slack-Signature", &signature),
            ("X-Slack-Request-Timestamp", timestamp),
        ]);
        (signature, headers)
    }

    fn verifier_at(now_secs: u64, max_age_secs: u64, secret: Vec<u8>) -> HmacWebhookAuth {
        HmacWebhookAuth::new(
            "X-Slack-Signature",
            "X-Slack-Request-Timestamp",
            secret,
            "slack_install_beta",
        )
        .with_max_age(max_age_secs)
        .with_clock(Box::new(FixedClock(now_secs)))
    }

    #[test]
    fn hmac_verifier_rejects_missing_signature_header() {
        let secret = b"super-shared-secret".to_vec();
        let headers = header_map(&[("X-Slack-Request-Timestamp", "1234567890")]);
        let verifier = verifier_at(1_234_567_900, 60, secret);
        match verifier.verify(&headers, b"{}") {
            VerificationOutcome::Failed { failure } => {
                assert!(matches!(failure, ProtocolAuthFailure::Missing));
            }
            other => panic!("expected Failed(Missing), got {other:?}"),
        }
    }

    #[test]
    fn hmac_verifier_rejects_missing_timestamp_header() {
        let secret = b"super-shared-secret".to_vec();
        let headers = header_map(&[("X-Slack-Signature", "v0=abc")]);
        let verifier = verifier_at(1_234_567_900, 60, secret);
        match verifier.verify(&headers, b"{}") {
            VerificationOutcome::Failed { failure } => {
                assert!(matches!(failure, ProtocolAuthFailure::Missing));
            }
            other => panic!("expected Failed(Missing), got {other:?}"),
        }
    }

    #[test]
    fn hmac_verifier_accepts_canonical_signature_within_window() {
        let secret = b"super-shared-secret".to_vec();
        let timestamp = "1234567890";
        let body = b"{\"event\":\"hello\"}";
        let (_, headers) = build_signed_request(&secret, timestamp, body);
        let verifier = verifier_at(1_234_567_900, 60, secret);
        match verifier.verify(&headers, body) {
            VerificationOutcome::Verified { subject } => {
                assert_eq!(subject, "slack_install_beta");
            }
            other => panic!("expected Verified, got {other:?}"),
        }
    }

    #[test]
    fn hmac_verifier_rejects_tampered_body() {
        let secret = b"super-shared-secret".to_vec();
        let timestamp = "1234567890";
        let body = b"{\"event\":\"hello\"}";
        let (_, headers) = build_signed_request(&secret, timestamp, body);
        let verifier = verifier_at(1_234_567_900, 60, secret);
        match verifier.verify(&headers, b"{\"event\":\"tampered\"}") {
            VerificationOutcome::Failed { failure } => {
                assert!(matches!(failure, ProtocolAuthFailure::SignatureMismatch));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn hmac_verifier_rejects_stale_timestamp_replay() {
        // Captured request from 10 minutes ago against a 5-minute window
        // — must reject before computing the HMAC.
        let secret = b"super-shared-secret".to_vec();
        let timestamp = "1_700_000_000".replace('_', "");
        let body = b"{\"event\":\"hello\"}";
        let (_, headers) = build_signed_request(&secret, &timestamp, body);
        let now = 1_700_000_000 + 600; // 10 min later
        let verifier = verifier_at(now, 300, secret); // 5 min window
        match verifier.verify(&headers, body) {
            VerificationOutcome::Failed { failure } => match failure {
                ProtocolAuthFailure::Other { .. } => {}
                other => panic!("expected Other (drift), got {other:?}"),
            },
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn hmac_verifier_rejects_far_future_timestamp() {
        // Far-future timestamps are also forgery attempts — symmetric
        // window check on |now - ts|.
        let secret = b"super-shared-secret".to_vec();
        let timestamp = "1_700_000_000".replace('_', "");
        let body = b"{}";
        let (_, headers) = build_signed_request(&secret, &timestamp, body);
        let now = 1_700_000_000 - 600; // 10 min before
        let verifier = verifier_at(now, 300, secret);
        match verifier.verify(&headers, body) {
            VerificationOutcome::Failed { failure } => match failure {
                ProtocolAuthFailure::Other { .. } => {}
                other => panic!("expected Other (drift), got {other:?}"),
            },
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn hmac_verifier_rejects_malformed_timestamp() {
        let secret = b"super-shared-secret".to_vec();
        let body = b"{}";
        let headers = header_map(&[
            ("X-Slack-Signature", "v0=abc"),
            ("X-Slack-Request-Timestamp", "not-a-number"),
        ]);
        let verifier = verifier_at(1_700_000_000, 300, secret);
        match verifier.verify(&headers, body) {
            VerificationOutcome::Failed { failure } => {
                assert!(matches!(failure, ProtocolAuthFailure::Malformed));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn hmac_verifier_rejects_extreme_negative_timestamp_without_overflow() {
        // `i64::MIN` as the timestamp header would have panicked the
        // previous `(now - ts as i64).abs()` arithmetic in overflow-
        // checked builds (subtraction overflows for any nonnegative
        // `now_secs`, and `i64::MIN.abs()` itself overflows). With i128
        // arithmetic the drift is computable and the verifier must
        // return `Failed` for an out-of-window timestamp, not panic.
        let secret = b"super-shared-secret".to_vec();
        let body = b"{}";
        let headers = header_map(&[
            ("X-Slack-Signature", "v0=abc"),
            ("X-Slack-Request-Timestamp", "-9223372036854775808"),
        ]);
        let verifier = verifier_at(1_700_000_000, 300, secret);
        match verifier.verify(&headers, body) {
            VerificationOutcome::Failed { .. } => {}
            other => panic!("extreme negative timestamp must fail closed, got {other:?}"),
        }
    }

    #[test]
    fn hmac_verifier_rejects_extreme_positive_timestamp_without_overflow() {
        // Mirror of the negative case for symmetry: `i64::MAX` would
        // also have overflowed the previous arithmetic on a typical
        // `now_secs ~ 1.7e9`. Must fail closed without panicking.
        let secret = b"super-shared-secret".to_vec();
        let body = b"{}";
        let headers = header_map(&[
            ("X-Slack-Signature", "v0=abc"),
            ("X-Slack-Request-Timestamp", "9223372036854775807"),
        ]);
        let verifier = verifier_at(1_700_000_000, 300, secret);
        match verifier.verify(&headers, body) {
            VerificationOutcome::Failed { .. } => {}
            other => panic!("extreme positive timestamp must fail closed, got {other:?}"),
        }
    }

    #[test]
    fn hmac_verifier_rejects_empty_signing_secret_as_malformed_config() {
        let timestamp = "1_700_000_000".replace('_', "");
        let body = b"{}";
        let (_, headers) = build_signed_request(&[], &timestamp, body);
        let verifier = verifier_at(1_700_000_000, 300, vec![]);
        match verifier.verify(&headers, body) {
            VerificationOutcome::Failed { failure } => {
                assert!(matches!(failure, ProtocolAuthFailure::Malformed));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn hmac_verifier_accepts_timestamp_at_window_boundary() {
        let secret = b"super-shared-secret".to_vec();
        let timestamp = "1_700_000_000".replace('_', "");
        let body = b"{}";
        let (_, headers) = build_signed_request(&secret, &timestamp, body);
        // Exactly at the boundary: drift == max_age. Must pass (closed
        // window, not open).
        let now = 1_700_000_000 + 300;
        let verifier = verifier_at(now, 300, secret);
        match verifier.verify(&headers, body) {
            VerificationOutcome::Verified { .. } => {}
            other => panic!("boundary timestamp should pass, got {other:?}"),
        }
    }

    #[test]
    fn hmac_verifier_rejects_timestamp_just_outside_window() {
        let secret = b"super-shared-secret".to_vec();
        let timestamp = "1_700_000_000".replace('_', "");
        let body = b"{}";
        let (_, headers) = build_signed_request(&secret, &timestamp, body);
        // 1 second past the boundary.
        let now = 1_700_000_000 + 301;
        let verifier = verifier_at(now, 300, secret);
        match verifier.verify(&headers, body) {
            VerificationOutcome::Failed { failure } => {
                assert!(matches!(failure, ProtocolAuthFailure::Other { .. }));
            }
            other => panic!("just-outside should fail, got {other:?}"),
        }
    }

    #[test]
    fn hmac_verifier_with_zero_max_age_accepts_exact_timestamp() {
        let secret = b"super-shared-secret".to_vec();
        let timestamp = "1_700_000_000".replace('_', "");
        let body = b"{}";
        let (_, headers) = build_signed_request(&secret, &timestamp, body);
        let verifier = verifier_at(1_700_000_000, 0, secret);
        match verifier.verify(&headers, body) {
            VerificationOutcome::Verified { .. } => {}
            other => panic!("zero max_age with exact timestamp should pass, got {other:?}"),
        }
    }

    #[test]
    fn hmac_verifier_with_zero_max_age_rejects_any_drift() {
        let secret = b"super-shared-secret".to_vec();
        let timestamp = "1_700_000_000".replace('_', "");
        let body = b"{}";
        let (_, headers) = build_signed_request(&secret, &timestamp, body);
        let verifier = verifier_at(1_700_000_001, 0, secret);
        match verifier.verify(&headers, body) {
            VerificationOutcome::Failed { failure } => {
                assert!(matches!(failure, ProtocolAuthFailure::Other { .. }));
            }
            other => panic!("zero max_age with drift should fail, got {other:?}"),
        }
    }
}
