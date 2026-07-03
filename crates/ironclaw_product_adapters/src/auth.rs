//! Protocol-authentication evidence.
//!
//! Webhook/protocol authentication happens in trusted host glue before an
//! inbound event enters [`crate::ProductWorkflow`]. Verified evidence is an
//! in-memory capability, not a wire format: production constructors are kept
//! crate-private so downstream adapter crates cannot mint host-authenticated
//! claims. Host-minted claims may also carry a resolved tenant scope when the
//! consuming product surface is tenant-scoped. Test-support builds expose
//! [`ProtocolAuthEvidence::test_verified`] and
//! [`ProtocolAuthEvidence::test_verified_for_tenant`] for fakes only.

use serde::de::{self, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use thiserror::Error;

use crate::error::ProductAdapterError;
use crate::redaction::RedactedString;
use ironclaw_host_api::TenantId;

/// Host-only seal. Cannot be named or constructed outside this module.
#[cfg_attr(
    not(any(test, feature = "test-support", feature = "host-auth-mint")),
    allow(
        dead_code,
        reason = "constructed only by host-auth/test-support feature gates"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HostAuthSeal(());

impl HostAuthSeal {
    #[cfg_attr(
        not(any(test, feature = "test-support", feature = "host-auth-mint")),
        allow(
            dead_code,
            reason = "constructed only by host-auth/test-support feature gates"
        )
    )]
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

impl AuthRequirement {
    pub fn validate_metadata(&self) -> Result<(), ProductAdapterError> {
        match self {
            Self::RequestSignature {
                header_name,
                timestamp_header_name,
            } => {
                validate_http_token("auth.header_name", header_name)?;
                if let Some(timestamp_header) = timestamp_header_name.as_deref() {
                    validate_http_token("auth.timestamp_header_name", timestamp_header)?;
                }
            }
            Self::SharedSecretHeader { header_name } => {
                validate_http_token("auth.header_name", header_name)?;
            }
            Self::SessionCookie { name } => {
                validate_cookie_name("auth.name", name)?;
            }
            Self::BearerToken => {}
        }
        Ok(())
    }
}

/// RFC 7230 §3.2.6 `token` = 1*tchar. Used to syntactically guard HTTP
/// header and cookie names against CRLF/whitespace/separator injection when
/// adapter metadata declares where auth evidence comes from.
fn validate_http_token(field: &'static str, value: &str) -> Result<(), ProductAdapterError> {
    if value.is_empty() {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind: field,
            reason: "must not be empty".to_string(),
        });
    }
    for c in value.chars() {
        if !is_http_tchar(c) {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: field,
                reason: format!(
                    "must be an RFC 7230 token (no CTL, whitespace, or separators); got {value:?}"
                ),
            });
        }
    }
    Ok(())
}

fn is_http_tchar(c: char) -> bool {
    matches!(
        c,
        '!' | '#' | '$' | '%' | '&' | '\'' | '*' | '+' | '-' | '.' | '^' | '_' | '`' | '|' | '~'
    ) || c.is_ascii_alphanumeric()
}

/// RFC 6265 `cookie-name` is an HTTP `token`. Reuse the same predicate so a
/// declared cookie name cannot smuggle CRLF, `=`, or `;` into downstream
/// `Set-Cookie`/`Cookie` interpolation.
fn validate_cookie_name(field: &'static str, value: &str) -> Result<(), ProductAdapterError> {
    validate_http_token(field, value)
}

/// Verified-claim contents the workflow may consult. Fields are private so the
/// claim cannot be fabricated with struct literal syntax, and the type is not
/// deserializable from untrusted input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerifiedAuthClaim {
    requirement: AuthRequirement,
    subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tenant_id: Option<TenantId>,
}

impl VerifiedAuthClaim {
    #[cfg_attr(
        not(any(test, feature = "test-support", feature = "host-auth-mint")),
        allow(
            dead_code,
            reason = "constructed only by host-auth/test-support feature gates"
        )
    )]
    pub(crate) fn new(requirement: AuthRequirement, subject: impl Into<String>) -> Self {
        Self::new_with_tenant(requirement, subject, None)
    }

    #[cfg_attr(
        not(any(test, feature = "test-support", feature = "host-auth-mint")),
        allow(
            dead_code,
            reason = "constructed only by host-auth/test-support feature gates"
        )
    )]
    pub(crate) fn new_for_tenant(
        requirement: AuthRequirement,
        subject: impl Into<String>,
        tenant_id: TenantId,
    ) -> Self {
        Self::new_with_tenant(requirement, subject, Some(tenant_id))
    }

    fn new_with_tenant(
        requirement: AuthRequirement,
        subject: impl Into<String>,
        tenant_id: Option<TenantId>,
    ) -> Self {
        Self {
            requirement,
            subject: subject.into(),
            tenant_id,
        }
    }

    pub fn requirement(&self) -> &AuthRequirement {
        &self.requirement
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    pub fn tenant_id(&self) -> Option<&TenantId> {
        self.tenant_id.as_ref()
    }
}

#[cfg_attr(
    not(any(test, feature = "test-support", feature = "host-auth-mint")),
    allow(
        dead_code,
        reason = "verified evidence is host-minted behind feature gates"
    )
)]
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
    #[cfg_attr(
        not(any(test, feature = "test-support", feature = "host-auth-mint")),
        allow(
            dead_code,
            reason = "called only by host-auth/test-support feature gates"
        )
    )]
    pub(crate) fn host_verified(requirement: AuthRequirement, subject: impl Into<String>) -> Self {
        Self {
            kind: ProtocolAuthEvidenceKind::Verified {
                claim: VerifiedAuthClaim::new(requirement, subject),
                _seal: HostAuthSeal::host_only(),
            },
        }
    }

    #[cfg_attr(
        not(any(test, feature = "test-support", feature = "host-auth-mint")),
        allow(
            dead_code,
            reason = "called only by host-auth/test-support feature gates"
        )
    )]
    pub(crate) fn host_verified_for_tenant(
        requirement: AuthRequirement,
        subject: impl Into<String>,
        tenant_id: TenantId,
    ) -> Self {
        Self {
            kind: ProtocolAuthEvidenceKind::Verified {
                claim: VerifiedAuthClaim::new_for_tenant(requirement, subject, tenant_id),
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_verified_for_tenant(
        requirement: AuthRequirement,
        subject: impl Into<String>,
        tenant_id: TenantId,
    ) -> Self {
        Self::host_verified_for_tenant(requirement, subject, tenant_id)
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
pub fn mark_request_signature_verified_for_tenant(
    header_name: impl Into<String>,
    timestamp_header_name: Option<String>,
    subject: impl Into<String>,
    tenant_id: TenantId,
) -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::host_verified_for_tenant(
        AuthRequirement::RequestSignature {
            header_name: header_name.into(),
            timestamp_header_name,
        },
        subject,
        tenant_id,
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
pub fn mark_shared_secret_header_verified_for_tenant(
    header_name: impl Into<String>,
    subject: impl Into<String>,
    tenant_id: TenantId,
) -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::host_verified_for_tenant(
        AuthRequirement::SharedSecretHeader {
            header_name: header_name.into(),
        },
        subject,
        tenant_id,
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
pub fn mark_session_verified_for_tenant(
    cookie_name: impl Into<String>,
    subject: impl Into<String>,
    tenant_id: TenantId,
) -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::host_verified_for_tenant(
        AuthRequirement::SessionCookie {
            name: cookie_name.into(),
        },
        subject,
        tenant_id,
    )
}

#[cfg(feature = "host-auth-mint")]
pub fn mark_bearer_token_verified(subject: impl Into<String>) -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::host_verified(AuthRequirement::BearerToken, subject)
}

#[cfg(feature = "host-auth-mint")]
pub fn mark_bearer_token_verified_for_tenant(
    subject: impl Into<String>,
    tenant_id: TenantId,
) -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::host_verified_for_tenant(AuthRequirement::BearerToken, subject, tenant_id)
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
        assert!(evidence.claim().expect("claim").tenant_id().is_none());
    }

    #[test]
    fn verified_claim_can_carry_host_resolved_tenant_scope() {
        let tenant_id = TenantId::new("tenant-a").expect("tenant");
        let evidence = ProtocolAuthEvidence::host_verified_for_tenant(
            AuthRequirement::BearerToken,
            "alice",
            tenant_id.clone(),
        );

        let claim = evidence.claim().expect("claim");
        assert_eq!(claim.subject(), "alice");
        assert_eq!(claim.tenant_id(), Some(&tenant_id));
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
        assert!(!json.contains("seal"));
        assert!(!json.contains("HostAuthSeal"));
        let parsed: Result<ProtocolAuthEvidence, _> = serde_json::from_str(&json);
        assert!(parsed.is_err());
    }

    #[test]
    fn auth_requirement_metadata_rejects_injection_prone_names() {
        assert!(
            AuthRequirement::SharedSecretHeader {
                header_name: "X-Foo\r\nInjected".into(),
            }
            .validate_metadata()
            .is_err()
        );
        assert!(
            AuthRequirement::RequestSignature {
                header_name: "X-Signature".into(),
                timestamp_header_name: Some("X-Time;stamp".into()),
            }
            .validate_metadata()
            .is_err()
        );
        assert!(
            AuthRequirement::SessionCookie {
                name: "session=id".into(),
            }
            .validate_metadata()
            .is_err()
        );
    }

    #[test]
    fn auth_requirement_metadata_accepts_http_tokens() {
        AuthRequirement::RequestSignature {
            header_name: "X-Slack-Signature".into(),
            timestamp_header_name: Some("X-Slack-Request-Timestamp".into()),
        }
        .validate_metadata()
        .expect("valid request signature requirement");
        AuthRequirement::SessionCookie {
            name: "__Host-session".into(),
        }
        .validate_metadata()
        .expect("valid cookie name");
    }
}
