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
//!   requires `OrgPolicy::admin_approves_dedicated_yolo = true`.
//! - **Hosted multi-tenant boundary**: `Local*` profiles never resolve under
//!   `HostedMultiTenant`, and the produced policy never selects
//!   `FilesystemBackendKind::HostWorkspace` or `ProcessBackendKind::LocalHost`
//!   under `HostedMultiTenant`. The compatibility matrix prevents this at the
//!   `(deployment, profile)` step before the backend mapping runs.
//!
//! ## Determinism and audit
//!
//! [`resolve`] is deterministic — equal inputs always produce equal outputs,
//! and the result serializes round-trippably so audit logs can record the
//! exact policy that gated an invocation. [`EffectiveRuntimePolicy::was_reduced`]
//! flags the case where deployment or tenant/org policy narrowed the requested
//! profile.

mod resolver;

pub use resolver::{OrgPolicy, ResolveError, ResolveRequest, resolve};

// Re-export the host-api vocabulary so downstream callers don't need a
// separate `use ironclaw_host_api::runtime_policy::*;`. Each item is a stable
// part of the public contract this crate consumes.
pub use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
