//! Behavior tests for the witness-gated protocol-auth evidence seal
//! (PROPOSAL §11.2.5, CHECKLIST WS1's evidence-mint row).
//!
//! These live under `tests/` (not an inline `#[cfg(test)]` module) for exactly
//! the reason `tests/authorized_seal.rs` does: minting requires implementing
//! `HostProtocolAuthenticator` or `ChannelIngressVerifier`, and
//! `reborn_sealed_evidence_mint_ratchet` bans those impls outside the one
//! production crate sanctioned for each. A test double under `tests/` is not
//! inventoried by that ratchet, so this is where the seal's own stand-in
//! minters belong — and keeping the ratchet free of a `#[cfg(test)]` carve-out
//! means an inline test module cannot become a hiding place for a grant source.

use ironclaw_host_api::ids::TenantId;
use ironclaw_host_api::product_adapter::auth::{
    AuthRequirement, ChannelIngressVerifier, HostAuthenticationGrant, HostProtocolAuthenticator,
    ProtocolAuthEvidence, VerifiedInboundGrant, mark_bearer_token_verified,
    mark_bearer_token_verified_for_tenant, mark_session_verified, mark_session_verified_for_tenant,
};

/// Stands in for `ironclaw_webui`'s authentication middleware, whose real
/// (and only) implementation sits on a module-private `AuthLayerState`.
struct TestHostAuthenticator;
impl HostProtocolAuthenticator for TestHostAuthenticator {}

/// Stands in for `ironclaw_extension_host`'s generic ingress verifier.
struct TestIngressVerifier;
impl ChannelIngressVerifier for TestIngressVerifier {}

fn tenant() -> TenantId {
    TenantId::new("tenant-a").expect("tenant")
}

#[test]
fn bearer_mint_requires_a_host_authentication_grant() {
    let authenticator = TestHostAuthenticator;

    let evidence = mark_bearer_token_verified(authenticator.host_authentication_grant(), "alice");
    assert!(evidence.is_verified());
    let claim = evidence.claim().expect("claim");
    assert_eq!(claim.requirement(), &AuthRequirement::BearerToken);
    assert_eq!(claim.subject(), "alice");
    assert!(claim.tenant_id().is_none());

    let scoped = mark_bearer_token_verified_for_tenant(
        authenticator.host_authentication_grant(),
        "alice",
        tenant(),
    );
    assert_eq!(scoped.claim().expect("claim").tenant_id(), Some(&tenant()));
}

#[test]
fn session_mint_requires_a_host_authentication_grant() {
    let authenticator = TestHostAuthenticator;

    let evidence = mark_session_verified(
        authenticator.host_authentication_grant(),
        "__Host-session",
        "alice",
    );
    assert_eq!(
        evidence.claim().expect("claim").requirement(),
        &AuthRequirement::SessionCookie {
            name: "__Host-session".to_string()
        }
    );

    let scoped = mark_session_verified_for_tenant(
        authenticator.host_authentication_grant(),
        "__Host-session",
        "alice",
        tenant(),
    );
    assert_eq!(scoped.claim().expect("claim").tenant_id(), Some(&tenant()));
}

/// The cross-crate seam `ironclaw_extension_contracts::verified_inbound` uses.
/// It must carry the requirement and tenant through verbatim — the evidence a
/// channel package sees has to describe the recipe the router actually
/// executed, not a normalized approximation of it.
#[test]
fn verified_inbound_seal_requires_a_verifier_grant_and_preserves_its_inputs() {
    let verifier = TestIngressVerifier;
    let requirement = AuthRequirement::RequestSignature {
        header_name: "X-Slack-Signature".into(),
        timestamp_header_name: Some("X-Slack-Request-Timestamp".into()),
    };

    let evidence = ProtocolAuthEvidence::seal_verified_inbound(
        verifier.verified_inbound_grant(),
        requirement.clone(),
        "T01ABCDEF",
        None,
    );
    assert!(evidence.is_verified());
    let claim = evidence.claim().expect("claim");
    assert_eq!(claim.requirement(), &requirement);
    assert_eq!(claim.subject(), "T01ABCDEF");
    assert!(claim.tenant_id().is_none());

    let scoped = ProtocolAuthEvidence::seal_verified_inbound(
        verifier.verified_inbound_grant(),
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Telegram-Bot-Api-Secret-Token".into(),
        },
        "bot-1",
        Some(tenant()),
    );
    assert_eq!(scoped.claim().expect("claim").tenant_id(), Some(&tenant()));
}

/// The two grants are distinct types, so the bearer minter cannot stand in for
/// the ingress verifier or vice versa. That separation is a *compile-time*
/// property; this pins that the two are never collapsed into one alias, which
/// would silently merge two trust roles.
#[test]
fn the_two_grants_are_distinct_zero_sized_witnesses() {
    assert_eq!(std::mem::size_of::<HostAuthenticationGrant>(), 0);
    assert_eq!(std::mem::size_of::<VerifiedInboundGrant>(), 0);
    assert_ne!(
        std::any::TypeId::of::<HostAuthenticationGrant>(),
        std::any::TypeId::of::<VerifiedInboundGrant>(),
        "collapsing the two grants would let the host authenticator mint channel evidence and the \
         ingress verifier mint bearer evidence"
    );
}

/// Minted evidence must never round-trip back in from the wire: an attacker who
/// can post a body must not be able to post a *verified* claim. `auth.rs`'s
/// inline suite pins this for the crate-private constructors; this pins that
/// the grant-gated entry points inherit it, which is what a caller across the
/// crate boundary actually reaches.
#[test]
fn grant_minted_evidence_does_not_round_trip_back_from_the_wire() {
    let authenticator = TestHostAuthenticator;
    let evidence = mark_bearer_token_verified_for_tenant(
        authenticator.host_authentication_grant(),
        "alice",
        tenant(),
    );

    let json = serde_json::to_string(&evidence).expect("serialize");
    assert!(json.contains("\"verified\""));
    assert!(!json.contains("seal"));
    let parsed: Result<ProtocolAuthEvidence, _> = serde_json::from_str(&json);
    assert!(
        parsed.is_err(),
        "a verified claim must not be reconstructible from wire input"
    );
}
