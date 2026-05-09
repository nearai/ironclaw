//! Protocol-authentication evidence.
//!
//! Webhook/protocol authentication happens in trusted host glue before an
//! inbound event enters [`crate::ProductWorkflow`]. Verified evidence is an
//! in-memory capability, not a wire format: production constructors are kept
//! crate-private so downstream adapter crates cannot mint host-authenticated
//! claims. Test-support builds expose [`ProtocolAuthEvidence::test_verified`]
//! for fakes only.

use serde::de::{self, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use thiserror::Error;

use crate::redaction::RedactedString;

/// Host-only seal. Cannot be named or constructed outside this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HostAuthSeal(());

impl HostAuthSeal {
    fn host_only() -> Self {
        Self(())
    }
}

/// What an adapter declares it needs in order to consider a payload
/// authenticated. Hosts read this from [`crate::ProductAdapter::auth_requirement`]
/// before calling [`crate::ProductAdapter::parse_inbound`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthRequirement {
    RequestSignature {
        header_name: String,
        timestamp_header_name: Option<String>,
    },
    SharedSecretHeader {
        header_name: String,
    },
    SessionCookie {
        name: String,
    },
    BearerToken,
}

/// Verified-claim contents the workflow may consult. Fields are private so the
/// claim cannot be fabricated with struct literal syntax, and the type is not
/// deserializable from untrusted input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerifiedAuthClaim {
    requirement: AuthRequirement,
    subject: String,
}

impl VerifiedAuthClaim {
    pub(crate) fn new(requirement: AuthRequirement, subject: impl Into<String>) -> Self {
        Self {
            requirement,
            subject: subject.into(),
        }
    }

    pub fn requirement(&self) -> &AuthRequirement {
        &self.requirement
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProtocolAuthEvidenceKind {
    Verified {
        claim: VerifiedAuthClaim,
        _seal: HostAuthSeal,
    },
    Failed {
        failure: ProtocolAuthFailure,
    },
}

/// Outcome of host-side protocol authentication. The verified variant is not
/// public API, so downstream crates cannot replay a seal from one evidence
/// value into another forged evidence value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolAuthEvidence {
    kind: ProtocolAuthEvidenceKind,
}

impl Serialize for ProtocolAuthEvidence {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match &self.kind {
            ProtocolAuthEvidenceKind::Verified { claim, .. } => {
                let mut state = serializer.serialize_struct("ProtocolAuthEvidence", 2)?;
                state.serialize_field("kind", "verified")?;
                state.serialize_field("claim", claim)?;
                state.end()
            }
            ProtocolAuthEvidenceKind::Failed { failure } => {
                let mut state = serializer.serialize_struct("ProtocolAuthEvidence", 2)?;
                state.serialize_field("kind", "failed")?;
                state.serialize_field("failure", failure)?;
                state.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for ProtocolAuthEvidence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct EvidenceVisitor;

        impl<'de> Visitor<'de> for EvidenceVisitor {
            type Value = ProtocolAuthEvidence;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "a ProtocolAuthEvidence::Failed wire envelope; Verified outcomes are host-minted",
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
                        other => return Err(de::Error::unknown_field(other, &["kind", "failure"])),
                    }
                }
                let kind = kind.ok_or_else(|| de::Error::missing_field("kind"))?;
                if kind != "failed" {
                    return Err(de::Error::custom(format!(
                        "ProtocolAuthEvidence wire payload kind={kind:?} is not accepted; only `failed` may cross trust boundaries"
                    )));
                }
                let failure = failure.ok_or_else(|| de::Error::missing_field("failure"))?;
                Ok(ProtocolAuthEvidence::failed(failure))
            }
        }

        deserializer.deserialize_map(EvidenceVisitor)
    }
}

impl ProtocolAuthEvidence {
    pub(crate) fn host_verified(requirement: AuthRequirement, subject: impl Into<String>) -> Self {
        Self {
            kind: ProtocolAuthEvidenceKind::Verified {
                claim: VerifiedAuthClaim::new(requirement, subject),
                _seal: HostAuthSeal::host_only(),
            },
        }
    }

    pub fn failed(failure: ProtocolAuthFailure) -> Self {
        Self {
            kind: ProtocolAuthEvidenceKind::Failed { failure },
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_verified(requirement: AuthRequirement, subject: impl Into<String>) -> Self {
        Self::host_verified(requirement, subject)
    }

    pub fn is_verified(&self) -> bool {
        matches!(self.kind, ProtocolAuthEvidenceKind::Verified { .. })
    }

    pub fn claim(&self) -> Option<&VerifiedAuthClaim> {
        match &self.kind {
            ProtocolAuthEvidenceKind::Verified { claim, .. } => Some(claim),
            ProtocolAuthEvidenceKind::Failed { .. } => None,
        }
    }

    pub fn failure(&self) -> Option<&ProtocolAuthFailure> {
        match &self.kind {
            ProtocolAuthEvidenceKind::Failed { failure } => Some(failure),
            ProtocolAuthEvidenceKind::Verified { .. } => None,
        }
    }
}

#[cfg(feature = "host-auth-mint")]
pub fn mark_request_signature_verified(
    header_name: impl Into<String>,
    timestamp_header_name: Option<String>,
    subject: impl Into<String>,
) -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::host_verified(
        AuthRequirement::RequestSignature {
            header_name: header_name.into(),
            timestamp_header_name,
        },
        subject,
    )
}

#[cfg(feature = "host-auth-mint")]
pub fn mark_shared_secret_header_verified(
    header_name: impl Into<String>,
    subject: impl Into<String>,
) -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::host_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: header_name.into(),
        },
        subject,
    )
}

#[cfg(feature = "host-auth-mint")]
pub fn mark_session_verified(
    cookie_name: impl Into<String>,
    subject: impl Into<String>,
) -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::host_verified(
        AuthRequirement::SessionCookie {
            name: cookie_name.into(),
        },
        subject,
    )
}

#[cfg(feature = "host-auth-mint")]
pub fn mark_bearer_token_verified(subject: impl Into<String>) -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::host_verified(AuthRequirement::BearerToken, subject)
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
    fn verified_can_only_be_constructed_via_host_helper_inside_crate() {
        let evidence = ProtocolAuthEvidence::host_verified(
            AuthRequirement::RequestSignature {
                header_name: "X-Slack-Signature".into(),
                timestamp_header_name: Some("X-Slack-Request-Timestamp".into()),
            },
            "T01ABCDEF",
        );
        assert!(evidence.is_verified());
        assert!(evidence.claim().is_some());
    }

    #[test]
    fn failed_evidence_carries_no_secret_in_display() {
        let evidence = ProtocolAuthEvidence::failed(ProtocolAuthFailure::Other {
            detail: RedactedString::new("bot12345:AAEFGH-private-token"),
        });
        let rendered = format!("{evidence:?}");
        assert!(!rendered.contains("AAEFGH-private-token"));
        let display = evidence.failure().expect("failure").to_string();
        assert!(!display.contains("AAEFGH-private-token"));
    }

    #[test]
    fn failed_evidence_round_trips_via_wire() {
        let evidence = ProtocolAuthEvidence::failed(ProtocolAuthFailure::SharedSecretMismatch);
        let json = serde_json::to_string(&evidence).expect("serialize");
        let parsed: ProtocolAuthEvidence = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, evidence);
    }

    #[test]
    fn verified_payload_on_the_wire_is_rejected_by_deserialize() {
        let forged = serde_json::json!({
            "kind": "verified",
            "claim": {
                "requirement": {"bearer_token": null},
                "subject": "attacker"
            }
        })
        .to_string();
        let result: Result<ProtocolAuthEvidence, _> = serde_json::from_str(&forged);
        assert!(result.is_err());
    }

    #[test]
    fn verified_evidence_in_memory_serializes_but_not_back() {
        let evidence = ProtocolAuthEvidence::host_verified(AuthRequirement::BearerToken, "alice");
        let json = serde_json::to_string(&evidence).expect("serialize");
        assert!(json.contains("\"verified\""));
        let parsed: Result<ProtocolAuthEvidence, _> = serde_json::from_str(&json);
        assert!(parsed.is_err());
    }
}
