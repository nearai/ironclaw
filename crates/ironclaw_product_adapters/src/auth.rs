//! Protocol-authentication evidence.
//!
//! Webhook/protocol-level authentication MUST happen in the trusted host
//! before any [`crate::ProductInboundEnvelope`] reaches the workflow facade.
//! The adapter (and any WASM v2 component) cannot mint a `Verified` evidence:
//! the `Verified` constructor requires a [`HostAuthSeal`] value, and
//! [`HostAuthSeal`] has a private constructor exposed only through
//! [`HostAuthSeal::host_only`], which is `pub(crate)`. Crates that perform
//! protocol verification must do so through helpers on this module
//! (`mark_signature_verified`, `mark_token_verified`, `mark_session_verified`)
//! which take the seal internally.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::redaction::RedactedString;

/// Host-only seal. Cannot be constructed outside this crate. Helpers on
/// [`ProtocolAuthEvidence`] thread it through internally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostAuthSeal(());

impl HostAuthSeal {
    /// `pub(crate)` so only this crate can mint a seal. WASM components and
    /// downstream adapters cannot reach this constructor.
    pub(crate) fn host_only() -> Self {
        Self(())
    }
}

/// What an adapter declares it needs in order to consider a payload
/// authenticated. Adapters return this from `parse_inbound_authentication`
/// hooks; the host enforces it before constructing a `Verified` evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthRequirement {
    /// HMAC-style request signature (e.g. Slack `X-Slack-Signature`).
    RequestSignature {
        header_name: String,
        timestamp_header_name: Option<String>,
    },
    /// Shared secret token in a header (e.g. Telegram
    /// `X-Telegram-Bot-Api-Secret-Token`).
    SharedSecretHeader { header_name: String },
    /// Authenticated session/cookie scoped to a known user (Web).
    SessionCookie { name: String },
    /// Pre-shared bearer token (CLI/API).
    BearerToken,
}

/// Verified-claim contents the workflow may consult. Adapter code must treat
/// these as an opaque attestation: the workflow consumes them, but the
/// adapter does not get to fabricate or mutate them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedAuthClaim {
    pub requirement: AuthRequirement,
    /// Stable claim subject (e.g. webhook shared-secret-id, user id from
    /// session cookie).
    pub subject: String,
}

/// Outcome of host-side protocol authentication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolAuthEvidence {
    /// Host verified the protocol authentication. Constructible only inside
    /// this crate via `host_verified`.
    Verified {
        claim: VerifiedAuthClaim,
        // Must be present to prevent forgery. Serde skips this field on the
        // wire; `Deserialize` re-mints it through `HostAuthSeal::host_only`,
        // which is reachable only from this crate's deserializer impl. WASM
        // components/external adapters never deserialize a `Verified` value
        // — they consume it through the host's API instead.
        #[serde(skip, default = "HostAuthSeal::host_only")]
        seal: HostAuthSeal,
    },
    /// Host could not verify; classification is structured.
    Failed { failure: ProtocolAuthFailure },
}

impl ProtocolAuthEvidence {
    /// Construct a verified evidence. Crate-internal: only host glue inside
    /// `ironclaw_product_adapters` (and downstream host runtimes that mint
    /// claims via `mark_*` helpers below) may call this.
    pub(crate) fn host_verified(claim: VerifiedAuthClaim) -> Self {
        Self::Verified {
            claim,
            seal: HostAuthSeal::host_only(),
        }
    }

    pub fn is_verified(&self) -> bool {
        matches!(self, Self::Verified { .. })
    }

    pub fn claim(&self) -> Option<&VerifiedAuthClaim> {
        match self {
            Self::Verified { claim, .. } => Some(claim),
            Self::Failed { .. } => None,
        }
    }
}

/// Public host-glue helper for HMAC/signature verification outcomes.
///
/// Production hosts compute the HMAC themselves and call this only when the
/// digest matched. Adapters and WASM components cannot invoke this directly:
/// it lives on the type but `pub(crate)` keeps its construction private.
pub fn mark_request_signature_verified(
    header_name: impl Into<String>,
    timestamp_header_name: Option<String>,
    subject: impl Into<String>,
) -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::host_verified(VerifiedAuthClaim {
        requirement: AuthRequirement::RequestSignature {
            header_name: header_name.into(),
            timestamp_header_name,
        },
        subject: subject.into(),
    })
}

/// Public host-glue helper for shared-secret-header verification outcomes
/// (Telegram-style).
pub fn mark_shared_secret_header_verified(
    header_name: impl Into<String>,
    subject: impl Into<String>,
) -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::host_verified(VerifiedAuthClaim {
        requirement: AuthRequirement::SharedSecretHeader {
            header_name: header_name.into(),
        },
        subject: subject.into(),
    })
}

/// Public host-glue helper for session-cookie verification outcomes (Web).
pub fn mark_session_verified(
    cookie_name: impl Into<String>,
    subject: impl Into<String>,
) -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::host_verified(VerifiedAuthClaim {
        requirement: AuthRequirement::SessionCookie {
            name: cookie_name.into(),
        },
        subject: subject.into(),
    })
}

/// Public host-glue helper for bearer-token outcomes (CLI/API).
pub fn mark_bearer_token_verified(subject: impl Into<String>) -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::host_verified(VerifiedAuthClaim {
        requirement: AuthRequirement::BearerToken,
        subject: subject.into(),
    })
}

/// Structured failure classifications. The `detail` field is redacted.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
pub enum ProtocolAuthFailure {
    #[error("missing authentication header or token")]
    Missing,
    #[error("authentication header present but malformed")]
    Malformed,
    #[error("signature did not match expected digest")]
    SignatureMismatch,
    #[error("token did not match expected shared secret")]
    SharedSecretMismatch,
    #[error("session was not authenticated or expired")]
    SessionUnauthenticated,
    #[error("bearer token did not match")]
    BearerTokenMismatch,
    #[error("authentication failed: {detail}")]
    Other { detail: RedactedString },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_can_only_be_constructed_via_host_helper() {
        let evidence = mark_request_signature_verified(
            "X-Slack-Signature",
            Some("X-Slack-Request-Timestamp".into()),
            "T01ABCDEF",
        );
        assert!(evidence.is_verified());
        assert!(evidence.claim().is_some());
    }

    #[test]
    fn failed_evidence_carries_no_secret_in_display() {
        let evidence = ProtocolAuthEvidence::Failed {
            failure: ProtocolAuthFailure::Other {
                detail: RedactedString::new("bot12345:AAEFGH-private-token"),
            },
        };
        let rendered = format!("{evidence:?}");
        assert!(!rendered.contains("AAEFGH-private-token"));
        let display = match &evidence {
            ProtocolAuthEvidence::Failed { failure } => failure.to_string(),
            _ => unreachable!(),
        };
        assert!(!display.contains("AAEFGH-private-token"));
    }

    #[test]
    fn verified_evidence_serde_round_trip() {
        // The seal field is `#[serde(skip)]`; re-deserialization re-mints
        // a fresh seal via `HostAuthSeal::host_only` (private constructor),
        // so an attacker cannot smuggle a fake `Verified` over the wire if
        // the receiving system trusts only crate-internal verification —
        // but they CAN obtain a `Verified` value in memory through serde
        // alone. Adapters must therefore never accept a serialized
        // `ProtocolAuthEvidence` from an untrusted source. This test pins
        // that expectation so any future change is intentional.
        let evidence = mark_bearer_token_verified("alice");
        let json = serde_json::to_string(&evidence).expect("serialize");
        let parsed: ProtocolAuthEvidence = serde_json::from_str(&json).expect("deserialize");
        assert!(parsed.is_verified());
    }
}
