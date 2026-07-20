//! Runtime profile resolver for IronClaw Reborn.
//!
//! This crate is a pure-logic layer that turns the operator's *request*
//! (`DeploymentMode` + `RuntimeProfile` + tenant/org policy) into the resolved
//! [`EffectiveRuntimePolicy`] the host runtime actually enforces.
//!
//! It depends only on the contract vocabulary in [`ironclaw_host_api`] —
//! no I/O, no runtime crates, no authorization/approval policy. The resolver
//! is the only sanctioned producer of [`EffectiveRuntimePolicy`]; values
//! constructed elsewhere should be treated as untrusted.
//!
//! ## Safety invariants enforced here
//!
//! - **Monotonic safety**: deployment mode and tenant/org policy may *reduce*
//!   the requested profile's authority; they must never *increase* it.
//! - **Fail-closed by default**: invalid `(deployment, profile)` pairs are an
//!   error, not a silent downgrade. The caller must observe the rejection so
//!   the CLI/settings/blueprint surface can offer an explicit safe profile.
//! - **Yolo opt-in**: any `*Yolo*` profile requires
//!   `ResolveRequest::yolo_disclosure_acknowledged = true`. The CLI/settings
//!   layer is responsible for actually obtaining the disclosure; the resolver
//!   only enforces that it was provided.
//! - **Enterprise direct-runner gate**: `EnterpriseYoloDedicated` additionally
//!   requires `OrgPolicyConstraints::admin_approves_dedicated_yolo = true`.
//! - **Hosted multi-tenant boundary**: `Local*` profiles never resolve under
//!   `HostedMultiTenant`, and the produced policy never selects
//!   `FilesystemBackendKind::HostWorkspace`,
//!   `FilesystemBackendKind::HostWorkspaceAndHome`, or
//!   `ProcessBackendKind::LocalHost` under `HostedMultiTenant`. The
//!   compatibility matrix prevents this at the `(deployment, profile)` step
//!   before the backend mapping runs.
//!
//! ## Resolved policy values
//!
//! Beyond the backend/mode fields on [`EffectiveRuntimePolicy`], this crate
//! classifies the resolved policy into the enforcement axes composition needs:
//! [`budget_enforcement`] and [`minimal_approval_bypass`]. Both are keyed on
//! the *resolved* profile, so a tenant/org ceiling narrowing authority reaches
//! them for free. They exist so no consumer past the composition edge branches
//! on a deployment mode — §4.4 of
//! `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`.
//!
//! ## Determinism and audit
//!
//! [`resolve`] is deterministic — equal inputs always produce equal outputs,
//! and the result serializes round-trippably so audit logs can record the
//! exact policy that gated an invocation. [`EffectiveRuntimePolicy::was_reduced`]
//! flags the case where deployment or tenant/org policy narrowed the requested
//! profile.
#![warn(unreachable_pub)]

mod resolver;

pub use resolver::{
    BudgetEnforcement, MinimalApprovalBypass, OrgPolicyConstraints, ResolveError, ResolveRequest,
    budget_enforcement, minimal_approval_bypass, resolve,
};

// `EffectiveRuntimePolicy` appears in `resolve`'s return type, so it must be
// reachable from the crate root. Other host-api runtime_policy types are not
// re-exported here — callers consume them directly from `ironclaw_host_api`.
pub use ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy;
