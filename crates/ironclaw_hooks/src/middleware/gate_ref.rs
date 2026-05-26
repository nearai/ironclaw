//! `HookGateRefFactory` — middleware-facing seam that mints `LoopGateRef`
//! values for hook-emitted pause decisions.
//!
//! When a `before_capability` hook returns `PauseApproval` or `PauseAuth`,
//! the `HookedLoopCapabilityPort` middleware needs to produce a real
//! `LoopGateRef` so the resulting `CapabilityOutcome::ApprovalRequired` /
//! `AuthRequired` can be routed through the host's gate-resolution
//! machinery. The middleware does not know how to mint refs that scope
//! correctly to the current run / approval-router — that knowledge lives
//! in the Reborn host composition. This trait is the seam the middleware
//! depends on; production code wires a concrete factory that talks to the
//! host's gate-router.
//!
//! The foundation slice ships [`UuidHookGateRefFactory`] — a deterministic
//! local-only implementation that mints opaque, run-scope-agnostic refs
//! using `uuid::Uuid::new_v4()`. It is suitable for tests and for the
//! foundation-slice end-to-end wiring, but production deployments should
//! provide a factory that takes the `LoopRunContext` at construction time
//! and emits refs that the host's approval-router will recognize.
//!
//! Failures bubble up as `AgentLoopHostError` so the middleware can fail
//! closed (mapping the suspension back to `Denied`) rather than silently
//! producing an unresolvable gate ref.
//!
//! # Security properties
//!
//! Minted gate refs must be **unguessable** so that an Installed-tier hook
//! that requests a `PauseApproval` cannot also forge a gate ref that
//! short-circuits the approval gateway. `UuidHookGateRefFactory` derives
//! its randomness from `uuid::Uuid::new_v4()`, which gives 122 bits of
//! entropy per ref (RFC 4122 §4.4). The `gate_refs_are_v4_uuids` and
//! `gate_refs_have_no_collisions_across_many_calls` tests document and
//! pin this property.
//!
//! **One-shot consumption** of a gate ref — the property that an attacker
//! who observes a legitimately-issued ref cannot replay it to bypass a
//! second approval — is *not* the factory's responsibility. The factory's
//! contract ends at minting an unguessable identifier. One-shot
//! consumption is enforced by the host's approval gateway when the
//! gate-resolution event arrives. See the threat-model (`S1`) for the
//! split.

use async_trait::async_trait;
use ironclaw_turns::LoopGateRef;
use ironclaw_turns::run_profile::{AgentLoopHostError, AgentLoopHostErrorKind};

/// Mints gate refs for hook-emitted suspension decisions.
///
/// The trait is split into approval and auth variants so a future
/// production impl can route them through different gate-router channels
/// without having to inspect the decision kind here. Both methods return
/// a fully validated [`LoopGateRef`] or an [`AgentLoopHostError`] if the
/// gate-router refused to mint one (the middleware fails closed in that
/// case).
#[async_trait]
pub trait HookGateRefFactory: Send + Sync {
    async fn mint_approval_ref(&self, reason: &str) -> Result<LoopGateRef, AgentLoopHostError>;
    async fn mint_auth_ref(&self, reason: &str) -> Result<LoopGateRef, AgentLoopHostError>;
}

/// Foundation-slice default. Mints opaque `gate:hook-approval-<uuid>` /
/// `gate:hook-auth-<uuid>` refs using `uuid::Uuid::new_v4()`. Refs are
/// locally unique but carry no scope information — production factories
/// should embed the run context so the host's approval-router can route
/// gate-resolution events back to the right run.
#[derive(Debug, Default, Clone, Copy)]
pub struct UuidHookGateRefFactory;

impl UuidHookGateRefFactory {
    pub fn new() -> Self {
        Self
    }

    fn mint(prefix: &str) -> Result<LoopGateRef, AgentLoopHostError> {
        // Uuid hyphenated form is exclusively ASCII alphanumeric + `-`,
        // which matches LoopGateRef's opaque-id charset.
        let id = uuid::Uuid::new_v4();
        let value = format!("gate:{prefix}-{id}");
        LoopGateRef::new(value).map_err(|err| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("hook gate-ref factory failed: {err}"),
            )
        })
    }
}

#[async_trait]
impl HookGateRefFactory for UuidHookGateRefFactory {
    async fn mint_approval_ref(&self, _reason: &str) -> Result<LoopGateRef, AgentLoopHostError> {
        Self::mint("hook-approval")
    }

    async fn mint_auth_ref(&self, _reason: &str) -> Result<LoopGateRef, AgentLoopHostError> {
        Self::mint("hook-auth")
    }
}

/// Production-safe default factory: every mint call fails closed.
///
/// **Why this is the middleware default**: `UuidHookGateRefFactory` mints
/// syntactically valid but router-unregistered refs. A hook that emits
/// `PauseApproval` would surface as `CapabilityOutcome::ApprovalRequired`
/// with a ref the approval gateway has never heard of — the loop would
/// suspend on a ref that can never resolve, and there's no one-shot /
/// lease semantics behind it. Shipping that as a default is worse than
/// failing the call (henrypark133 review Critical #3).
///
/// Callers that *want* the local-only UUID behavior (tests, dev fixtures)
/// must explicitly install [`UuidHookGateRefFactory`] via
/// `with_gate_ref_factory`. Production deployments must install a factory
/// that talks to the host's real approval/auth router.
#[derive(Debug, Default, Clone, Copy)]
pub struct FailClosedHookGateRefFactory;

impl FailClosedHookGateRefFactory {
    pub fn new() -> Self {
        Self
    }

    fn fail(kind: &str) -> AgentLoopHostError {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            format!(
                "no production hook gate-ref factory installed; refusing to \
                 mint a {kind} ref that the approval/auth router cannot \
                 resolve (see HookedLoopCapabilityPort::with_gate_ref_factory)"
            ),
        )
    }
}

#[async_trait]
impl HookGateRefFactory for FailClosedHookGateRefFactory {
    async fn mint_approval_ref(&self, _reason: &str) -> Result<LoopGateRef, AgentLoopHostError> {
        Err(Self::fail("approval"))
    }

    async fn mint_auth_ref(&self, _reason: &str) -> Result<LoopGateRef, AgentLoopHostError> {
        Err(Self::fail("auth"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn approval_ref_has_valid_format() {
        let factory = UuidHookGateRefFactory;
        let r = factory
            .mint_approval_ref("needs approval")
            .await
            .expect("mints");
        assert!(r.as_str().starts_with("gate:hook-approval-"));
    }

    #[tokio::test]
    async fn auth_ref_has_valid_format() {
        let factory = UuidHookGateRefFactory;
        let r = factory.mint_auth_ref("needs auth").await.expect("mints");
        assert!(r.as_str().starts_with("gate:hook-auth-"));
    }

    #[tokio::test]
    async fn refs_are_unique_across_calls() {
        let factory = UuidHookGateRefFactory;
        let a = factory.mint_approval_ref("r").await.expect("mints");
        let b = factory.mint_approval_ref("r").await.expect("mints");
        assert_ne!(a.as_str(), b.as_str());
    }

    /// Pins the unguessability source: every minted ref must contain a
    /// parseable v4 UUID. If the factory ever moves to a non-random source
    /// (counter, deterministic derivation, weaker UUID version), this test
    /// fails — that's the design-time guardrail. Threat-model finding S1.
    #[tokio::test]
    async fn gate_refs_are_v4_uuids() {
        let factory = UuidHookGateRefFactory;
        for prefix in ["hook-approval-", "hook-auth-"] {
            let r = if prefix == "hook-approval-" {
                factory.mint_approval_ref("r").await.expect("mints")
            } else {
                factory.mint_auth_ref("r").await.expect("mints")
            };
            let suffix = r
                .as_str()
                .strip_prefix("gate:")
                .and_then(|s| s.strip_prefix(prefix))
                .unwrap_or_else(|| panic!("unexpected gate-ref shape: {}", r.as_str()));
            let parsed = uuid::Uuid::parse_str(suffix)
                .unwrap_or_else(|e| panic!("ref suffix `{suffix}` not a uuid: {e}"));
            assert_eq!(
                parsed.get_version(),
                Some(uuid::Version::Random),
                "gate-ref `{}` must be v4 (122 random bits); got version {:?}",
                r.as_str(),
                parsed.get_version()
            );
        }
    }

    /// Statistical unguessability proxy: 20_000 distinct refs from one
    /// factory must produce zero collisions. With 122 random bits the
    /// expected collision count over 20k draws is ~2.4e-32, so any
    /// collision here indicates the entropy source has regressed
    /// catastrophically. Threat-model finding S1.
    #[tokio::test]
    async fn gate_refs_have_no_collisions_across_many_calls() {
        const N: usize = 20_000;
        let factory = UuidHookGateRefFactory;
        let mut seen = std::collections::HashSet::with_capacity(N);
        for _ in 0..N {
            let r = factory.mint_approval_ref("r").await.expect("mints");
            assert!(
                seen.insert(r.as_str().to_string()),
                "duplicate gate-ref minted within {N} calls: {}",
                r.as_str()
            );
        }
        for _ in 0..N {
            let r = factory.mint_auth_ref("r").await.expect("mints");
            assert!(
                seen.insert(r.as_str().to_string()),
                "auth-ref collided with approval-ref space: {}",
                r.as_str()
            );
        }
        assert_eq!(seen.len(), 2 * N);
    }

    /// Cross-namespace separation: a `hook-approval-` and a `hook-auth-`
    /// ref with the same suffix would still be distinct strings, but the
    /// prefix is the routing key for the approval gateway. Confirm the
    /// two namespaces don't share format-level overlap that could let an
    /// attacker forge one from the other.
    #[tokio::test]
    async fn approval_and_auth_namespaces_do_not_overlap() {
        let factory = UuidHookGateRefFactory;
        let approval = factory.mint_approval_ref("r").await.expect("mints");
        let auth = factory.mint_auth_ref("r").await.expect("mints");
        assert!(approval.as_str().starts_with("gate:hook-approval-"));
        assert!(auth.as_str().starts_with("gate:hook-auth-"));
        assert!(!approval.as_str().starts_with("gate:hook-auth-"));
        assert!(!auth.as_str().starts_with("gate:hook-approval-"));
    }
}
