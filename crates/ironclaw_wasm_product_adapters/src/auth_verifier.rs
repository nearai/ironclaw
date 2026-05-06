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

use hmac::{Hmac, Mac};
use ironclaw_product_adapters::ProtocolAuthFailure;
use sha2::Sha256;
use subtle::ConstantTimeEq;

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
pub struct HmacWebhookAuth {
    pub signature_header: String,
    pub timestamp_header: String,
    pub signing_secret: Vec<u8>,
    pub subject: String,
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
        let Some(timestamp) = headers
            .get(self.timestamp_header.as_str())
            .and_then(|v| v.to_str().ok())
        else {
            return VerificationOutcome::Failed {
                failure: ProtocolAuthFailure::Missing,
            };
        };
        let signed_payload = format!("v0:{timestamp}:");
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.signing_secret).expect("hmac key");
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
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        let mut out = String::with_capacity(bytes.as_ref().len() * 2);
        for byte in bytes.as_ref() {
            out.push_str(&format!("{byte:02x}"));
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
    fn hmac_verifier_accepts_canonical_signature() {
        let secret = b"super-shared-secret".to_vec();
        let timestamp = "1234567890";
        let body = b"{\"event\":\"hello\"}";
        let mut mac = Hmac::<Sha256>::new_from_slice(&secret).expect("hmac key");
        mac.update(format!("v0:{timestamp}:").as_bytes());
        mac.update(body);
        let digest_hex = hex::encode(mac.finalize().into_bytes());
        let signature = format!("v0={digest_hex}");
        let headers = header_map(&[
            ("X-Slack-Signature", &signature),
            ("X-Slack-Request-Timestamp", timestamp),
        ]);
        let verifier = HmacWebhookAuth {
            signature_header: "X-Slack-Signature".into(),
            timestamp_header: "X-Slack-Request-Timestamp".into(),
            signing_secret: secret,
            subject: "slack_install_beta".into(),
        };
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
        let mut mac = Hmac::<Sha256>::new_from_slice(&secret).expect("hmac key");
        mac.update(format!("v0:{timestamp}:").as_bytes());
        mac.update(body);
        let digest_hex = hex::encode(mac.finalize().into_bytes());
        let signature = format!("v0={digest_hex}");
        let headers = header_map(&[
            ("X-Slack-Signature", &signature),
            ("X-Slack-Request-Timestamp", timestamp),
        ]);
        let verifier = HmacWebhookAuth {
            signature_header: "X-Slack-Signature".into(),
            timestamp_header: "X-Slack-Request-Timestamp".into(),
            signing_secret: secret,
            subject: "slack_install_beta".into(),
        };
        // Verifier is asked to authenticate a different body — must fail
        // signature mismatch.
        match verifier.verify(&headers, b"{\"event\":\"tampered\"}") {
            VerificationOutcome::Failed { failure } => {
                assert!(matches!(failure, ProtocolAuthFailure::SignatureMismatch));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }
}
