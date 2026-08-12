//! Behavior tests for the channel/webhook mint family (PROPOSAL §6.1.2,
//! §11.2.5).
//!
//! Under `tests/` rather than an inline `#[cfg(test)]` module because minting
//! requires a `ChannelIngressVerifier` impl and
//! `reborn_sealed_evidence_mint_ratchet` bans that impl outside
//! `ironclaw_extension_host`. Same convention as
//! `ironclaw_host_api/tests/authorized_seal.rs`.

use ironclaw_extension_contracts::verified_inbound::{
    mark_request_signature_verified, mark_shared_secret_header_verified,
};
use ironclaw_host_api::product_adapter::auth::{
    AuthRequirement, ChannelIngressVerifier, ProtocolAuthEvidence,
};

/// Stands in for `ironclaw_extension_host`'s generic ingress verifier — in
/// production, the `VerifiedEvidenceMint` value that mirrors the recipe the
/// router just executed.
struct TestIngressVerifier;
impl ChannelIngressVerifier for TestIngressVerifier {}

#[test]
fn request_signature_evidence_carries_the_recipe_the_router_executed() {
    let verifier = TestIngressVerifier;
    let evidence = mark_request_signature_verified(
        verifier.verified_inbound_grant(),
        "X-Slack-Signature",
        Some("X-Slack-Request-Timestamp".to_string()),
        "T01ABCDEF",
    );

    assert!(evidence.is_verified());
    let claim = evidence.claim().expect("claim");
    assert_eq!(
        claim.requirement(),
        &AuthRequirement::RequestSignature {
            header_name: "X-Slack-Signature".to_string(),
            timestamp_header_name: Some("X-Slack-Request-Timestamp".to_string()),
        }
    );
    assert_eq!(claim.subject(), "T01ABCDEF");
    assert!(claim.tenant_id().is_none());
}

#[test]
fn shared_secret_evidence_carries_the_recipe_the_router_executed() {
    let verifier = TestIngressVerifier;
    let evidence = mark_shared_secret_header_verified(
        verifier.verified_inbound_grant(),
        "X-Telegram-Bot-Api-Secret-Token",
        "bot-1",
    );

    let claim = evidence.claim().expect("claim");
    assert_eq!(
        claim.requirement(),
        &AuthRequirement::SharedSecretHeader {
            header_name: "X-Telegram-Bot-Api-Secret-Token".to_string(),
        }
    );
    assert_eq!(claim.subject(), "bot-1");
}

/// Minted evidence must never round-trip back in from the wire: an attacker who
/// can post a webhook body must not be able to post a *verified* claim. Pinned
/// here because this family is the one reached from the public webhook path.
#[test]
fn minted_evidence_does_not_round_trip_back_from_the_wire() {
    let verifier = TestIngressVerifier;
    let evidence = mark_shared_secret_header_verified(
        verifier.verified_inbound_grant(),
        "X-Telegram-Bot-Api-Secret-Token",
        "bot-1",
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
