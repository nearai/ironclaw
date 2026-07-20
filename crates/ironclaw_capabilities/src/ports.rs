//! Host-mediated policy *facts* the `authorize()` fold reads.
//!
//! Dependency-inversion port (`type-placement.md` trait-justification #2):
//! defined in the kernel, implemented up-layer by
//! `ironclaw_host_runtime`'s `DefaultHostRuntime`. It returns **facts only** —
//! never a verdict. `authorize()` ([`crate::CapabilityHost`]) maps these facts
//! into the sealed [`ironclaw_host_api::AuthorizeResult`]:
//!
//! - a missing credential → `Blocked::Auth`, surfaced *before* the approval
//!   decision so a human approval is never consumed for an action that cannot
//!   yet execute; and
//! - an active persistent grant → a re-authorize with that grant injected.
//!
//! Keeping *decisions* in the kernel and *mechanism* behind this port is the
//! §5.3.2 security milestone (`docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`):
//! it makes the `Authorized` seal's guarantee real instead of vacuous. A port
//! that returned decisions would recreate the four-layer authority smear behind
//! a single trait — the exact failure this milestone exists to remove.
//!
//! **Boundary:** the sole consumer is `CapabilityHost::authorize`; the sole
//! production implementor is `ironclaw_host_runtime::DefaultHostRuntime` (the
//! `reborn_dependency_boundaries` architecture test keeps the impl up-layer).
//! The port references only `ironclaw_host_api` vocabulary plus the kernel-local
//! [`PolicyAction`], so it adds no dependency edge to the kernel crate.

use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityGrant, CapabilityId, ExecutionContext, ResourceScope,
    RuntimeCredentialAuthRequirement, SecretHandle,
};

/// Presence of a capability's required credentials, read from the host secret
/// store. A *fact*, not a decision.
#[derive(Debug, Clone)]
pub enum CredentialPresence {
    /// Every required credential is present, or the capability requires none.
    Satisfied,
    /// At least one required credential is absent. `authorize()` maps this to
    /// `Blocked::Auth` **before** the approval decision. The fields are the
    /// exact inputs host-runtime rebuilds its `AuthRequired` outcome from, so
    /// the gate identity is preserved across the relocation.
    Missing {
        required_secrets: Vec<SecretHandle>,
        requirements: Vec<RuntimeCredentialAuthRequirement>,
    },
    /// A transient store fault. `authorize()` **skips** the pre-flight (the
    /// dispatch-time obligation check remains the enforcing backstop); a fault
    /// is never treated as `Missing`, which would burn a user auth interaction.
    Indeterminate,
}

/// Which authority action a persistent-approval lookup is scoped to.
///
/// Mirrors host-runtime's `PersistentApprovalAction` without depending on
/// `ironclaw_approvals` — the port implementor maps between the two, keeping the
/// kernel's dependency surface small (the inversion is the point).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyAction {
    /// A normal dispatch invocation.
    Dispatch,
    /// A capability spawn (long-running process).
    SpawnCapability,
}

/// Host-mediated policy facts consumed by `authorize()`. **Facts only** — this
/// port never decides; the kernel maps each fact into the sealed verdict.
#[async_trait]
pub trait HostPolicyFacts: Send + Sync {
    /// Whether the capability's required credentials are present for `scope`.
    ///
    /// Presence only — the pre-flight exists solely to order the auth gate ahead
    /// of the approval gate; the dispatch-time obligation check is the enforcing
    /// authority on injection. A transient store fault returns
    /// [`CredentialPresence::Indeterminate`], never a false `Missing`.
    async fn credential_presence(
        &self,
        capability_id: &CapabilityId,
        scope: &ResourceScope,
    ) -> CredentialPresence;

    /// The active persistent-approval grants matching this invocation's identity
    /// (`capability`/`action`, and every grantee/scope derived from `context`).
    ///
    /// `context` is passed whole — not just its `ResourceScope` — because the
    /// grantee fan-out includes the `Principal::Extension` grantee read from
    /// `context.extension_id`, which the scope alone does not carry. Surfacing
    /// only scope-derived grantees would silently drop extension-grantee
    /// persistent approvals.
    ///
    /// `authorize()` owns the re-authorize loop — it re-runs the trust-aware
    /// authorizer with each grant injected into a candidate context, because it
    /// holds the authorizer. This port only surfaces the grants to fold in; it
    /// makes no authorization decision. A lookup fault yields an empty vector
    /// (fall back to normal authorization), never a synthesized grant.
    async fn persistent_grants(
        &self,
        capability_id: &CapabilityId,
        context: &ExecutionContext,
        action: PolicyAction,
    ) -> Vec<CapabilityGrant>;
}
