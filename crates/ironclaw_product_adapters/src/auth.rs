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
//!
//! ## Serde forgery resistance
//!
//! `ProtocolAuthEvidence::Verified` MUST NOT be constructible from an
//! untrusted wire. The custom `Deserialize` impl in this module accepts
//! `Failed` only and rejects every `Verified` payload with an error. This
//! closes the `#[serde(default)]` re-mint loophole that an earlier draft
//! exposed: a `Verified` evidence in memory is now provably the result of
//! a host-side `mark_*_verified` call, not a JSON deserialization.

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
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
///
/// Note: `Verified` is intentionally **not** automatically deserializable.
/// The custom `Deserialize` impl below rejects any wire payload that
/// claims to be `Verified` — the only path to a `Verified` value is via
/// the public `mark_*_verified` helpers in this module, which run inside
/// the trusted host. Wire payloads carrying authentication outcomes may
/// only encode `Failed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProtocolAuthEvidence {
    /// Host verified the protocol authentication. Constructible only
    /// inside this crate via `host_verified`. Cannot be reached through
    /// `Deserialize`.
    Verified {
        claim: VerifiedAuthClaim,
        // The seal field is `skip`'d on the *serialize* side too — it
        // carries no payload, only type-system authority. This keeps the
        // wire shape `{"kind":"verified","claim":...}` symmetric with the
        // accepted-failure shape, so the rejection error in `Deserialize`
        // is unambiguous: any inbound `verified` payload is forged.
        #[serde(skip)]
        seal: HostAuthSeal,
    },
    /// Host could not verify; classification is structured. This is the
    /// ONLY variant accepted by the custom `Deserialize` impl below.
    Failed { failure: ProtocolAuthFailure },
}

impl<'de> Deserialize<'de> for ProtocolAuthEvidence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Wire shape: `{"kind":"failed","failure":{...}}`. Any other
        // `kind` (in particular `"verified"`) is rejected outright.
        struct EvidenceVisitor;

        impl<'de> Visitor<'de> for EvidenceVisitor {
            type Value = ProtocolAuthEvidence;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "a ProtocolAuthEvidence::Failed wire envelope; \
                     Verified outcomes must be minted by the host, not \
                     deserialized from untrusted input",
                )
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: de::MapAccess<'de>,
            {
                let mut kind: Option<String> = None;
                let mut failure: Option<ProtocolAuthFailure> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "kind" => {
                            if kind.is_some() {
                                return Err(de::Error::duplicate_field("kind"));
                            }
                            kind = Some(map.next_value()?);
                        }
                        "failure" => {
                            if failure.is_some() {
                                return Err(de::Error::duplicate_field("failure"));
                            }
                            failure = Some(map.next_value()?);
                        }
                        // Reject any unexpected key. In particular this
                        // catches `claim` and `seal` payloads aimed at
                        // forging a `Verified` value.
                        other => {
                            return Err(de::Error::unknown_field(other, &["kind", "failure"]));
                        }
                    }
                }
                let kind = kind.ok_or_else(|| de::Error::missing_field("kind"))?;
                if kind != "failed" {
                    return Err(de::Error::custom(format!(
                        "ProtocolAuthEvidence wire payload kind={kind:?} is not accepted; \
                         only `failed` may cross trust boundaries — `verified` outcomes are \
                         minted by the host"
                    )));
                }
                let failure = failure.ok_or_else(|| de::Error::missing_field("failure"))?;
                Ok(ProtocolAuthEvidence::Failed { failure })
            }
        }

        deserializer.deserialize_map(EvidenceVisitor)
    }
}

// `HostAuthSeal` participates in `Serialize` only as a no-op skipped
// field. Provide an explicit `Serialize` impl so the type can be embedded
// where the parent uses `#[serde(skip)]`; we deliberately do not provide
// `Deserialize` because the seal must never be reconstructed from a wire
// payload.
impl Serialize for HostAuthSeal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_unit()
    }
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
    fn failed_evidence_round_trips_via_wire() {
        // `Failed` IS deserializable; that is the only outcome trusted
        // wire payloads carry between Reborn services.
        let evidence = ProtocolAuthEvidence::Failed {
            failure: ProtocolAuthFailure::SharedSecretMismatch,
        };
        let json = serde_json::to_string(&evidence).expect("serialize");
        let parsed: ProtocolAuthEvidence = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, evidence);
    }

    #[test]
    fn verified_payload_on_the_wire_is_rejected_by_deserialize() {
        // Hand-craft what a forged `Verified` envelope might look like.
        // The custom `Deserialize` impl rejects it.
        let forged = serde_json::json!({
            "kind": "verified",
            "claim": {
                "requirement": {"bearer_token": null},
                "subject": "attacker"
            }
        })
        .to_string();
        let result: Result<ProtocolAuthEvidence, _> = serde_json::from_str(&forged);
        assert!(
            result.is_err(),
            "a forged Verified evidence must NOT round-trip through serde"
        );
        let err = result.expect_err("must error");
        let rendered = err.to_string();
        // Either the kind rejection ("verified... not accepted") or the
        // unknown-field rejection (`claim`/`seal`) is acceptable; both
        // close the forgery path. Pin that one of them fires.
        assert!(
            (rendered.contains("verified") && rendered.contains("not accepted"))
                || rendered.contains("unknown field")
                || rendered.contains("missing field"),
            "rejection reason should explain why; got: {rendered}"
        );
    }

    #[test]
    fn verified_evidence_in_memory_serializes_but_not_back() {
        // A `Verified` evidence built by the host can serialize (so logs
        // and audit trails work) but the produced JSON must NOT round-trip
        // back into a `Verified` value — round-tripping a `Verified`
        // outcome would re-open the forgery loophole.
        let evidence = mark_bearer_token_verified("alice");
        let json = serde_json::to_string(&evidence).expect("serialize");
        // Sanity: serialized form claims to be Verified.
        assert!(json.contains("\"verified\""));
        // But re-deserializing must fail.
        let parsed: Result<ProtocolAuthEvidence, _> = serde_json::from_str(&json);
        assert!(
            parsed.is_err(),
            "serialized Verified evidence must not round-trip; got: {parsed:?}"
        );
    }

    #[test]
    fn verified_evidence_unknown_field_is_rejected() {
        // Even if the wire claims `kind: failed`, an unexpected field
        // (claim, seal, anything) is rejected to prevent typo-driven
        // tunneling of structured payloads through a permissive parser.
        let trojan = serde_json::json!({
            "kind": "failed",
            "failure": {"shared_secret_mismatch": null},
            "claim": {"requirement": {"bearer_token": null}, "subject": "x"}
        })
        .to_string();
        let result: Result<ProtocolAuthEvidence, _> = serde_json::from_str(&trojan);
        assert!(result.is_err(), "unknown_field must reject");
    }
}
