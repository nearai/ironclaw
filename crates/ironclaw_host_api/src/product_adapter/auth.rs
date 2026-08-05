//! Protocol-authentication evidence and the witnesses that gate minting it.
//!
//! Webhook/protocol authentication happens in trusted host glue before an
//! inbound event enters the host product surface. Verified evidence is an
//! in-memory capability, not a wire format: the variant that says "verified" is
//! private to this module, so no downstream crate can build one with struct
//! literal syntax or replay a seal from one evidence value into another.
//! Host-minted claims may also carry a resolved tenant scope when the consuming
//! product surface is tenant-scoped. Test-support builds expose
//! [`ProtocolAuthEvidence::test_verified`] and
//! [`ProtocolAuthEvidence::test_verified_for_tenant`] for fakes only.
//!
//! ## Who may mint (PROPOSAL §11.2.5, CHECKLIST WS1's evidence-mint row)
//!
//! Minting is gated by **witness tokens**, the same idiom [`crate::authorized`]
//! uses for the `Authorized` capability witness. Two grants, because the two
//! minters hold different trust roles:
//!
//! | Grant | Source trait | Sole implementor | Trust stage |
//! |---|---|---|---|
//! | [`HostAuthenticationGrant`] | [`HostProtocolAuthenticator`] | `ironclaw_webui` | T1 — bearer/session |
//! | [`VerifiedInboundGrant`] | [`ChannelIngressVerifier`] | `ironclaw_extension_host` | T2 — channel/webhook |
//!
//! Each grant's field is private to this crate, so its *only* source is the
//! provided body of its trait method. The bearer/session mint family lives here
//! (§6.1.1); the channel/webhook family lives in
//! `ironclaw_extension_contracts::verified_inbound` (§6.1.2) and reaches the
//! sealed variant through [`ProtocolAuthEvidence::seal_verified_inbound`], which
//! consumes a [`VerifiedInboundGrant`].
//!
//! Pure cross-crate type-sealing is not expressible in Rust — this crate defines
//! the grants but the legitimate minters are other crates — so the seal has two
//! halves, exactly as `authorized.rs` records: the compiler enforces "you cannot
//! mint without a grant", and `reborn_sealed_evidence_mint_ratchet` enforces
//! "only one crate may implement each grant trait".
//!
//! **This replaced a `host-auth-mint` cargo feature that sealed nothing.** Cargo
//! unifies features across the packages selected in one build, so a single
//! consumer opting in compiled the mint family open for the entire workspace;
//! measured on this row's base, a crate naming `ironclaw_host_api` with no
//! features could mint a verified bearer claim whenever `ironclaw_webui` was in
//! the same `cargo test`. A witness cannot be switched on from another crate's
//! manifest.

use serde::de::{self, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

use crate::product_adapter::error::ProductAdapterError;
use crate::{ids::TenantId, product_adapter_error::ProtocolAuthFailure};

/// Host-only seal carried inside the verified variant. Cannot be named or
/// constructed outside this module, so a `Verified` value cannot be assembled
/// from parts even by code that can see the claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HostAuthSeal(());

impl HostAuthSeal {
    fn host_only() -> Self {
        Self(())
    }
}

/// Proof that the caller is the host transport that just performed bearer or
/// session authentication. Zero-sized; its field is private to this crate, so
/// the only source is [`HostProtocolAuthenticator::host_authentication_grant`].
#[derive(Debug)]
pub struct HostAuthenticationGrant(());

/// Implemented by the host transport's authentication middleware — the code
/// that validated the bearer token or session cookie — and by nothing else.
///
/// Like [`crate::authorized::CapabilityAuthorizer`], this trait is deliberately
/// **not** sealed to `ironclaw_host_api`: sealing it here would stop the one
/// crate that legitimately needs it from implementing it. Its teeth are (1) the
/// only way to obtain a [`HostAuthenticationGrant`] is through it, and (2) the
/// `reborn_sealed_evidence_mint_ratchet` architecture test, which keeps the
/// implementor set at exactly one production crate (`ironclaw_webui`).
pub trait HostProtocolAuthenticator {
    /// Mint a grant. Provided and effectively not overridable: the grant's field
    /// is private to this crate, so this default body is the sole source of a
    /// bearer/session grant anywhere.
    fn host_authentication_grant(&self) -> HostAuthenticationGrant {
        HostAuthenticationGrant(())
    }
}

/// Proof that the caller is the generic channel ingress verifier — the
/// vendor-blind router that executed the manifest's verification recipe (HMAC
/// signature or shared-secret header, replay window, constant-time compare)
/// before any adapter code saw the request.
///
/// This is the witness PROPOSAL §12.1a is about: a channel package may
/// misreport parsed content, but holding no grant it cannot forge verification
/// or scope.
#[derive(Debug)]
pub struct VerifiedInboundGrant(());

/// Implemented by the generic ingress verifier (`ironclaw_extension_host`) and
/// by nothing else — never by a channel package, never by product code. Same
/// two-halves seal as [`HostProtocolAuthenticator`].
pub trait ChannelIngressVerifier {
    /// Mint a grant. Provided and effectively not overridable, for the same
    /// reason as [`HostProtocolAuthenticator::host_authentication_grant`].
    fn verified_inbound_grant(&self) -> VerifiedInboundGrant {
        VerifiedInboundGrant(())
    }
}

/// What an adapter declares it needs in order to consider a payload
/// authenticated. Hosts read this control-plane metadata to build protocol-auth
/// evidence before invoking the adapter's inbound path.
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

/// Verified-claim contents the product surface may consult. Fields are private so the
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
    pub(crate) fn new(requirement: AuthRequirement, subject: impl Into<String>) -> Self {
        Self::new_with_tenant(requirement, subject, None)
    }

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

    /// The channel/webhook half of the seal, reachable across the crate
    /// boundary because the mint family PROPOSAL §6.1.2 assigns to
    /// `ironclaw_extension_contracts` has to construct the verified variant
    /// this module keeps private.
    ///
    /// Consuming a [`VerifiedInboundGrant`] is what keeps that reachability from
    /// being a hole: the grant's only source is
    /// [`ChannelIngressVerifier::verified_inbound_grant`], and only
    /// `ironclaw_extension_host` may implement that trait. Callers pass the
    /// requirement the ingress recipe actually verified, mirroring the recipe
    /// the router executed.
    ///
    /// **Recorded residual:** the `requirement` parameter is the full
    /// [`AuthRequirement`], so a grant holder could attest a bearer-shaped
    /// requirement. The grant holder is the generic ingress verifier — trusted
    /// host code by charter — and narrowing this would mean a second enum
    /// duplicating `AuthRequirement`'s channel half. The property this seam
    /// exists for (a *package* or a *product handler* cannot mint at all) is
    /// unaffected.
    pub fn seal_verified_inbound(
        _grant: VerifiedInboundGrant,
        requirement: AuthRequirement,
        subject: impl Into<String>,
        tenant_id: Option<TenantId>,
    ) -> Self {
        match tenant_id {
            Some(tenant_id) => Self::host_verified_for_tenant(requirement, subject, tenant_id),
            None => Self::host_verified(requirement, subject),
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

// ── Bearer/session mint family (PROPOSAL §6.1.1, trust stage T1) ────────────
//
// The channel/webhook half of this family lives in
// `ironclaw_extension_contracts::verified_inbound` (§6.1.2). Splitting them by
// trust role is the point: the host transport and the ingress verifier hold
// different grants, so neither can attest the other's claim shape.
//
// Every function here consumes a `HostAuthenticationGrant`, so a caller must
// hold `&impl HostProtocolAuthenticator` — in production, `ironclaw_webui`'s
// authentication middleware and nothing else.

/// Attest that a session cookie was verified by the host transport.
pub fn mark_session_verified(
    _grant: HostAuthenticationGrant,
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

/// Tenant-scoped [`mark_session_verified`], for tenant-scoped product surfaces.
pub fn mark_session_verified_for_tenant(
    _grant: HostAuthenticationGrant,
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

/// Attest that a bearer token was verified by the host transport.
pub fn mark_bearer_token_verified(
    _grant: HostAuthenticationGrant,
    subject: impl Into<String>,
) -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::host_verified(AuthRequirement::BearerToken, subject)
}

/// Tenant-scoped [`mark_bearer_token_verified`] — what the WebUI gateway's
/// authentication middleware mints for OpenAI-compatible route mounts.
pub fn mark_bearer_token_verified_for_tenant(
    _grant: HostAuthenticationGrant,
    subject: impl Into<String>,
    tenant_id: TenantId,
) -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::host_verified_for_tenant(AuthRequirement::BearerToken, subject, tenant_id)
}

#[cfg(test)]
mod tests {
    use crate::product_adapter_error::RedactedString;

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
