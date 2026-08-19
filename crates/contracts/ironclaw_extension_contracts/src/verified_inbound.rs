//! Sealed verified-inbound evidence — the channel/webhook mint family.
//!
//! PROPOSAL §6.1.2 assigns this family here, and PROPOSAL §12.1a is the reason:
//! trust stage T2 is where a raw webhook becomes a *verified* inbound request.
//! The generic ingress router executes the manifest's verification recipe (HMAC
//! signature or shared-secret header, replay window, constant-time compare,
//! body bounds) and only then mints the evidence below. The
//! `ChannelIngress::receive` call that follows may translate the verified
//! payload through manifest-restricted egress to complete attachments/context.
//! A malicious package can lie about content, but holding no grant it cannot
//! forge verification, scope, or wider network authority.
//!
//! ## The seal
//!
//! Every function here consumes an
//! [`ironclaw_host_api::product_adapter::auth::VerifiedInboundGrant`], whose only
//! source is the provided body of
//! [`ironclaw_host_api::product_adapter::auth::ChannelIngressVerifier::verified_inbound_grant`].
//! Minting therefore requires *implementing* that trait, and
//! `reborn_sealed_evidence_mint_ratchet` keeps the implementor set at exactly
//! one production crate: `ironclaw_extension_host`, the vendor-blind verifier.
//!
//! The evidence *type* stays in `ironclaw_host_api` (§6.1.1 owns it, and the
//! bearer/session half of the family with it). This crate cannot construct its
//! verified variant directly — that variant is private to its module — so these
//! functions go through the grant-gated
//! [`ProtocolAuthEvidence::seal_verified_inbound`] seam. That split is what lets
//! the two halves of the mint family live in the two crates §6 assigns them to
//! without either crate handing out an unguarded constructor.
//!
//! Before WS1.5 this family sat in `ironclaw_host_api` behind a `host-auth-mint`
//! cargo feature and was reached through two `ironclaw_assistant` re-export paths.
//! The feature sealed nothing — cargo unifies features across a build, so one
//! consumer's opt-in compiled the family open workspace-wide. See
//! `crates/contracts/ironclaw_host_api/src/product_adapter/auth.rs` for the measurement.
//!
//! ✎ **WS8, 2026-08-05:** the family is two functions, not four. The
//! `_for_tenant` variants of both recipes had zero callers in any build and
//! were deleted as dead mint surface; the ingress router mints unscoped
//! evidence and resolves tenancy from the installation afterwards. Tenant
//! threading through a grant-gated mint stays pinned by the surviving
//! `mark_bearer_token_verified_for_tenant` and by
//! `ProtocolAuthEvidence::seal_verified_inbound`'s own `Some(tenant)` path in
//! `ironclaw_host_api/tests/protocol_auth_evidence_seal.rs`. Reviving a
//! tenant-scoped channel mint means adding it back **with** its caller.
//!
//! Tests that need verified evidence use
//! [`ProtocolAuthEvidence::test_verified`] (the `test-support` seam), not these
//! functions: a test standing in for the host is exactly the case
//! `test_verified` exists for, and it keeps adapter crates out of the grant
//! path entirely.

use ironclaw_host_api::product_adapter::auth::{
    AuthRequirement, ProtocolAuthEvidence, VerifiedInboundGrant,
};

/// Attest that the request-signature recipe verified this request.
///
/// `header_name` and `timestamp_header_name` mirror the recipe the router
/// executed, so the claim describes the verification that actually happened.
pub fn mark_request_signature_verified(
    grant: VerifiedInboundGrant,
    header_name: impl Into<String>,
    timestamp_header_name: Option<String>,
    subject: impl Into<String>,
) -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::seal_verified_inbound(
        grant,
        AuthRequirement::RequestSignature {
            header_name: header_name.into(),
            timestamp_header_name,
        },
        subject,
        None,
    )
}

/// Attest that the shared-secret-header recipe verified this request.
pub fn mark_shared_secret_header_verified(
    grant: VerifiedInboundGrant,
    header_name: impl Into<String>,
    subject: impl Into<String>,
) -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::seal_verified_inbound(
        grant,
        AuthRequirement::SharedSecretHeader {
            header_name: header_name.into(),
        },
        subject,
        None,
    )
}

// Tests live in `tests/verified_inbound_seal.rs`, not an inline `#[cfg(test)]`
// module: exercising the mint family requires a `ChannelIngressVerifier` impl,
// and `reborn_sealed_evidence_mint_ratchet` bans that impl outside
// `ironclaw_extension_host`. A test double under `tests/` is not inventoried by
// that ratchet — the same convention `ironclaw_host_api`'s `Authorized` witness
// established.
