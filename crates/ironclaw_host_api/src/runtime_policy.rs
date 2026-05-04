//! Runtime policy vocabulary for IronClaw Reborn.
//!
//! This module is the contract vocabulary for runtime presets. It defines the
//! enums and the resolved-policy aggregate type, but it does not implement the
//! resolver — the deployment × profile × tenant/org policy → effective policy
//! resolver lives in `ironclaw_runtime_policy` (or
//! `ironclaw_host_runtime::policy`) and is the only sanctioned producer of
//! [`EffectiveRuntimePolicy`].
//!
//! ## What lives here
//!
//! - [`DeploymentMode`] — where IronClaw is running and who owns the machine
//!   boundary.
//! - [`RuntimeProfile`] — which preset the operator/user requested.
//! - [`EffectiveRuntimePolicy`] — the resolved policy the runtime actually
//!   enforces.
//! - Backend/policy enums consumed by [`EffectiveRuntimePolicy`]:
//!   [`FilesystemBackendKind`], [`ProcessBackendKind`], [`NetworkMode`],
//!   [`SecretMode`], [`ApprovalPolicy`], [`AuditMode`].
//!
//! ## What does *not* live here
//!
//! - The resolver itself.
//! - Authority decisions (capability grants, leases, approvals) — those stay in
//!   `ironclaw_authorization` / future `ironclaw_capabilities`.
//! - Execution mechanics (script/wasm/mcp runners) — those stay in their
//!   respective runtime crates.
//!
//! ## Distinction from [`crate::RuntimeKind`]
//!
//! [`crate::RuntimeKind`] answers *what kind of work is being performed*
//! (Wasm / Mcp / Script / FirstParty / System). [`RuntimeProfile`] answers
//! *where and how that work is executed and with what authority*. Do not add a
//! `Local` variant to [`crate::RuntimeKind`] — locality and authority belong
//! to runtime policy, not to the execution lane.
//!
//! ## Distinction from [`crate::TrustClass`]
//!
//! [`crate::TrustClass`] is a per-invocation authority ceiling produced by the
//! trust policy engine for a specific package/extension. [`RuntimeProfile`] is
//! a deployment-scoped policy preset chosen by the operator; the resolver
//! turns it into [`EffectiveRuntimePolicy`] which then constrains the
//! filesystem/process/network/secret backends offered to every invocation.
//! Trust and runtime policy compose: a `LocalDev` + `Sandbox` invocation is
//! still bounded by both.
//!
//! ## Wire format
//!
//! All enums serialize as `snake_case` strings to keep audit logs and
//! settings/blueprint TOML readable. [`EffectiveRuntimePolicy`] is a simple
//! aggregate of those enums plus the requested/resolved profile pair so audit
//! can render "you asked for X, you got Y" when deployment or tenant/org
//! policy reduces authority.

use serde::{Deserialize, Serialize};

/// Where IronClaw is running and who owns the machine boundary.
///
/// The deployment mode is the upper bound on which [`RuntimeProfile`] values
/// the resolver can accept. `LocalSingleUser` may select any `Local*` profile;
/// `HostedMultiTenant` may not. The resolver enforces this, not the type
/// itself — this enum only carries the vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentMode {
    /// Single user on a personal machine. Can host `Local*` profiles.
    LocalSingleUser,
    /// Multi-tenant hosting. Cannot expose provider-host filesystem or shell;
    /// `Local*` profiles are rejected by the resolver.
    HostedMultiTenant,
    /// Single-organization dedicated infrastructure. Can host
    /// `Enterprise*` profiles when org admin policy permits.
    EnterpriseDedicated,
}

impl DeploymentMode {
    /// Wire/audit-stable string representation.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::LocalSingleUser => "local_single_user",
            Self::HostedMultiTenant => "hosted_multi_tenant",
            Self::EnterpriseDedicated => "enterprise_dedicated",
        }
    }
}

impl std::fmt::Display for DeploymentMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Operator/user-selected runtime preset.
///
/// Profiles are vocabulary; the resolver in `ironclaw_runtime_policy` turns
/// `(DeploymentMode, RuntimeProfile, tenant/org policy)` into an
/// [`EffectiveRuntimePolicy`] and rejects deployment/profile combinations
/// that would expand authority beyond what the deployment allows.
///
/// Naming hints for variants:
///
/// - `Safe` variants are cautious defaults that prefer ask-on-write/process.
/// - `Dev` variants allow common developer effects without prompting.
/// - `Yolo` variants intentionally reduce approvals **inside their authority
///   boundary**; they are never a path to broader authority. `LocalYolo`
///   stays on the local machine, `HostedYoloTenantScoped` stays inside the
///   tenant sandbox, `EnterpriseYoloDedicated` stays inside org-dedicated
///   infrastructure.
/// - `Sandboxed` is the general safe helper-process mode.
/// - `Experiment` is for disposable package-install/test/benchmark flows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProfile {
    /// Safe assistant default: scoped virtual filesystem, sandbox or
    /// disabled process, brokered network/secrets, policy-driven approvals.
    SecureDefault,

    /// Cautious local coding mode: selected workspace read, ask-on-write,
    /// ask-on-shell.
    LocalSafe,
    /// Default local coding-agent mode: selected workspace read/write, local
    /// shell, ask only on dangerous actions (rm -rf, push, sudo, etc.).
    LocalDev,
    /// Trusted-laptop mode: minimal approvals on the local machine. Requires
    /// explicit selection + visible disclosure. Audit/timeouts/output caps
    /// stay on.
    LocalYolo,

    /// Hosted multi-tenant default: tenant workspace read, ask-on-write,
    /// tenant-scoped sandbox process, brokered network/secrets.
    HostedSafe,
    /// Hosted developer mode inside the tenant boundary. Tenant workspace
    /// read/write, tenant-scoped sandbox, brokered network/secrets.
    HostedDev,
    /// Reduced approvals **inside** the tenant sandbox. Never a path to
    /// provider-host authority.
    HostedYoloTenantScoped,

    /// Enterprise dedicated default. Org-dedicated workspace + runner under
    /// org admin policy.
    EnterpriseSafe,
    /// Enterprise developer mode. Org-dedicated workspace read/write,
    /// org-dedicated runner.
    EnterpriseDev,
    /// Reduced approvals on org-dedicated infrastructure. Requires
    /// `EnterpriseDedicated` deployment + org admin policy.
    EnterpriseYoloDedicated,

    /// General safe helper-process mode: scoped/read-only mount + scratch,
    /// SRT/Docker/SmolVM process, brokered network/secrets.
    Sandboxed,
    /// Disposable package-install/test/benchmark mode. Copy-in or
    /// read-only repo + sandbox overlay, SmolVM/Docker process,
    /// allowlisted/brokered network.
    Experiment,
}

impl RuntimeProfile {
    /// Wire/audit-stable string representation.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::SecureDefault => "secure_default",
            Self::LocalSafe => "local_safe",
            Self::LocalDev => "local_dev",
            Self::LocalYolo => "local_yolo",
            Self::HostedSafe => "hosted_safe",
            Self::HostedDev => "hosted_dev",
            Self::HostedYoloTenantScoped => "hosted_yolo_tenant_scoped",
            Self::EnterpriseSafe => "enterprise_safe",
            Self::EnterpriseDev => "enterprise_dev",
            Self::EnterpriseYoloDedicated => "enterprise_yolo_dedicated",
            Self::Sandboxed => "sandboxed",
            Self::Experiment => "experiment",
        }
    }

    /// `true` for `Local*` variants. The resolver uses this to refuse local
    /// profiles on hosted/enterprise deployments.
    pub const fn is_local(&self) -> bool {
        matches!(self, Self::LocalSafe | Self::LocalDev | Self::LocalYolo)
    }

    /// `true` for `Hosted*` variants.
    pub const fn is_hosted(&self) -> bool {
        matches!(
            self,
            Self::HostedSafe | Self::HostedDev | Self::HostedYoloTenantScoped
        )
    }

    /// `true` for `Enterprise*` variants.
    pub const fn is_enterprise(&self) -> bool {
        matches!(
            self,
            Self::EnterpriseSafe | Self::EnterpriseDev | Self::EnterpriseYoloDedicated
        )
    }

    /// `true` for any `*Yolo*` variant.
    ///
    /// Yolo variants always require explicit selection and visible disclosure
    /// — the resolver and CLI/settings surfaces enforce that, this method is
    /// just a predicate.
    pub const fn is_yolo(&self) -> bool {
        matches!(
            self,
            Self::LocalYolo | Self::HostedYoloTenantScoped | Self::EnterpriseYoloDedicated
        )
    }
}

impl std::fmt::Display for RuntimeProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Filesystem backend the host runtime should expose for an invocation.
///
/// `ScopedVirtual` is the safe default — declared mounts only, no host paths
/// reachable. The other variants progressively widen authority and are
/// only valid in matching deployment modes (`HostWorkspace` for local,
/// `TenantWorkspace` for hosted, `OrgDedicatedWorkspace` for enterprise).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemBackendKind {
    /// Scoped virtual mounts only. No host filesystem reachable.
    ScopedVirtual,
    /// Selected host workspace root. Local single-user only.
    HostWorkspace,
    /// Tenant-scoped workspace under hosted multi-tenant boundary.
    TenantWorkspace,
    /// Single-organization dedicated workspace.
    OrgDedicatedWorkspace,
}

impl FilesystemBackendKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::ScopedVirtual => "scoped_virtual",
            Self::HostWorkspace => "host_workspace",
            Self::TenantWorkspace => "tenant_workspace",
            Self::OrgDedicatedWorkspace => "org_dedicated_workspace",
        }
    }
}

impl std::fmt::Display for FilesystemBackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Process backend the host runtime should plan against.
///
/// `None` disables process effects entirely. `LocalHost` runs on the
/// provider host — only valid for local single-user deployments.
/// `TenantSandbox` and `OrgDedicatedRunner` keep process effects inside
/// the matching authority boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessBackendKind {
    /// No process effects.
    None,
    /// Docker container.
    Docker,
    /// Sandboxed runtime (SRT).
    Srt,
    /// Lightweight VM (SmolVM).
    SmolVm,
    /// Provider-host shell. Local single-user only.
    LocalHost,
    /// Tenant-scoped sandbox process.
    TenantSandbox,
    /// Single-organization dedicated runner.
    OrgDedicatedRunner,
}

impl ProcessBackendKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Docker => "docker",
            Self::Srt => "srt",
            Self::SmolVm => "smol_vm",
            Self::LocalHost => "local_host",
            Self::TenantSandbox => "tenant_sandbox",
            Self::OrgDedicatedRunner => "org_dedicated_runner",
        }
    }
}

impl std::fmt::Display for ProcessBackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Network egress posture for an invocation.
///
/// Ordered roughly by widening authority: `Deny` < `Brokered` <
/// `Allowlist` < `DirectLogged` < `Direct`. `Direct` is only acceptable
/// in trusted local profiles; hosted multi-tenant must not select it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    /// No outbound network.
    Deny,
    /// Outbound through the network broker only.
    Brokered,
    /// Outbound to a configured allowlist only.
    Allowlist,
    /// Direct outbound with full request/response logging.
    DirectLogged,
    /// Direct outbound without per-request logging. Local trusted only.
    Direct,
}

impl NetworkMode {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::Brokered => "brokered",
            Self::Allowlist => "allowlist",
            Self::DirectLogged => "direct_logged",
            Self::Direct => "direct",
        }
    }
}

impl std::fmt::Display for NetworkMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Secret-access posture for an invocation.
///
/// `BrokeredHandles` is the default safe shape — invocations get opaque
/// secret handles, never the underlying value. The wider variants exist
/// for explicit local-trusted use cases and never apply to hosted
/// multi-tenant deployments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretMode {
    /// No secret access.
    Deny,
    /// Brokered handles only — opaque tokens, no plaintext exposure.
    BrokeredHandles,
    /// Tenant-scoped broker — the same shape as `BrokeredHandles` but
    /// scoped to a tenant boundary.
    TenantBroker,
    /// Org KMS / org broker — enterprise-dedicated equivalent of
    /// `BrokeredHandles`.
    OrgBroker,
    /// Scrubbed environment with explicit per-secret leases. Local
    /// trusted contexts only.
    ScrubbedEnv,
    /// Direct env-var inheritance. Trusted-laptop contexts only.
    InheritedEnv,
}

impl SecretMode {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::BrokeredHandles => "brokered_handles",
            Self::TenantBroker => "tenant_broker",
            Self::OrgBroker => "org_broker",
            Self::ScrubbedEnv => "scrubbed_env",
            Self::InheritedEnv => "inherited_env",
        }
    }
}

impl std::fmt::Display for SecretMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Approval posture the host runtime should apply.
///
/// This is the runtime-policy-side default; per-invocation approvals (from
/// `ironclaw_authorization`) still run on top.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    /// Ask before every effectful action.
    AskAlways,
    /// Ask only on writes and shell/process actions.
    AskWrites,
    /// Ask only on destructive or off-workspace actions
    /// (`rm -rf`, `git push`, `sudo`, package publish, secret inspection,
    /// out-of-workspace writes, etc.).
    AskDestructive,
    /// Honor org admin policy. The resolver fills in concrete defaults
    /// from tenant/org configuration.
    OrgPolicy,
    /// Minimal approvals. Only valid in `*Yolo*` profiles and requires
    /// explicit selection.
    Minimal,
}

impl ApprovalPolicy {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::AskAlways => "ask_always",
            Self::AskWrites => "ask_writes",
            Self::AskDestructive => "ask_destructive",
            Self::OrgPolicy => "org_policy",
            Self::Minimal => "minimal",
        }
    }
}

impl std::fmt::Display for ApprovalPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Audit retention/redaction posture for an invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditMode {
    /// Minimal local audit — invocation outcomes only.
    LocalMinimal,
    /// Standard durable audit with redaction.
    Standard,
    /// Org-policy-driven retention/redaction.
    OrgPolicy,
}

impl AuditMode {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::LocalMinimal => "local_minimal",
            Self::Standard => "standard",
            Self::OrgPolicy => "org_policy",
        }
    }
}

impl std::fmt::Display for AuditMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Resolved runtime policy the host actually enforces.
///
/// Produced by the resolver in `ironclaw_runtime_policy` from
/// `(DeploymentMode, RuntimeProfile, tenant/org policy)`. Carries both the
/// requested and resolved profile so audit can render "you asked for X, you
/// got Y" when deployment or tenant/org policy reduces authority. The two
/// match in the unconstrained case.
///
/// Construction is `pub` because this crate only owns the vocabulary;
/// callers that must produce an effective policy go through
/// `ironclaw_runtime_policy`, which is the only sanctioned source. Treat
/// values produced outside that resolver as untrusted.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EffectiveRuntimePolicy {
    pub deployment: DeploymentMode,
    pub requested_profile: RuntimeProfile,
    pub resolved_profile: RuntimeProfile,
    pub filesystem_backend: FilesystemBackendKind,
    pub process_backend: ProcessBackendKind,
    pub network_mode: NetworkMode,
    pub secret_mode: SecretMode,
    pub approval_policy: ApprovalPolicy,
    pub audit_mode: AuditMode,
}

impl EffectiveRuntimePolicy {
    /// `true` when the resolver had to narrow the requested profile to a
    /// less-permissive one (deployment or tenant/org policy reduced
    /// authority). Audit log surfaces should highlight this case.
    pub fn was_reduced(&self) -> bool {
        self.requested_profile != self.resolved_profile
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployment_mode_as_str_matches_serde_wire_name() {
        for mode in [
            DeploymentMode::LocalSingleUser,
            DeploymentMode::HostedMultiTenant,
            DeploymentMode::EnterpriseDedicated,
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(json, format!("\"{}\"", mode.as_str()));
        }
    }

    #[test]
    fn runtime_profile_as_str_matches_serde_wire_name() {
        for profile in [
            RuntimeProfile::SecureDefault,
            RuntimeProfile::LocalSafe,
            RuntimeProfile::LocalDev,
            RuntimeProfile::LocalYolo,
            RuntimeProfile::HostedSafe,
            RuntimeProfile::HostedDev,
            RuntimeProfile::HostedYoloTenantScoped,
            RuntimeProfile::EnterpriseSafe,
            RuntimeProfile::EnterpriseDev,
            RuntimeProfile::EnterpriseYoloDedicated,
            RuntimeProfile::Sandboxed,
            RuntimeProfile::Experiment,
        ] {
            let json = serde_json::to_string(&profile).unwrap();
            assert_eq!(json, format!("\"{}\"", profile.as_str()));
        }
    }

    #[test]
    fn runtime_profile_family_predicates_partition_correctly() {
        // Each variant belongs to at most one family. Sandboxed/Experiment/
        // SecureDefault are deployment-agnostic and report false for all.
        for profile in [
            RuntimeProfile::SecureDefault,
            RuntimeProfile::LocalSafe,
            RuntimeProfile::LocalDev,
            RuntimeProfile::LocalYolo,
            RuntimeProfile::HostedSafe,
            RuntimeProfile::HostedDev,
            RuntimeProfile::HostedYoloTenantScoped,
            RuntimeProfile::EnterpriseSafe,
            RuntimeProfile::EnterpriseDev,
            RuntimeProfile::EnterpriseYoloDedicated,
            RuntimeProfile::Sandboxed,
            RuntimeProfile::Experiment,
        ] {
            let families = [
                profile.is_local(),
                profile.is_hosted(),
                profile.is_enterprise(),
            ];
            let count = families.iter().filter(|f| **f).count();
            assert!(
                count <= 1,
                "profile {profile:?} matched more than one family: {families:?}"
            );
        }
    }

    #[test]
    fn yolo_predicate_matches_only_yolo_variants() {
        let yolo = [
            RuntimeProfile::LocalYolo,
            RuntimeProfile::HostedYoloTenantScoped,
            RuntimeProfile::EnterpriseYoloDedicated,
        ];
        let non_yolo = [
            RuntimeProfile::SecureDefault,
            RuntimeProfile::LocalSafe,
            RuntimeProfile::LocalDev,
            RuntimeProfile::HostedSafe,
            RuntimeProfile::HostedDev,
            RuntimeProfile::EnterpriseSafe,
            RuntimeProfile::EnterpriseDev,
            RuntimeProfile::Sandboxed,
            RuntimeProfile::Experiment,
        ];
        for profile in yolo {
            assert!(profile.is_yolo(), "{profile:?} must be yolo");
        }
        for profile in non_yolo {
            assert!(!profile.is_yolo(), "{profile:?} must not be yolo");
        }
    }

    #[test]
    fn effective_runtime_policy_round_trips_through_serde() {
        let policy = EffectiveRuntimePolicy {
            deployment: DeploymentMode::LocalSingleUser,
            requested_profile: RuntimeProfile::LocalDev,
            resolved_profile: RuntimeProfile::LocalDev,
            filesystem_backend: FilesystemBackendKind::HostWorkspace,
            process_backend: ProcessBackendKind::LocalHost,
            network_mode: NetworkMode::DirectLogged,
            secret_mode: SecretMode::ScrubbedEnv,
            approval_policy: ApprovalPolicy::AskDestructive,
            audit_mode: AuditMode::LocalMinimal,
        };
        let json = serde_json::to_string(&policy).unwrap();
        let parsed: EffectiveRuntimePolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, policy);
        assert!(!parsed.was_reduced());
    }

    #[test]
    fn effective_runtime_policy_was_reduced_flips_when_resolver_narrows() {
        let policy = EffectiveRuntimePolicy {
            deployment: DeploymentMode::HostedMultiTenant,
            requested_profile: RuntimeProfile::LocalDev,
            resolved_profile: RuntimeProfile::HostedSafe,
            filesystem_backend: FilesystemBackendKind::TenantWorkspace,
            process_backend: ProcessBackendKind::TenantSandbox,
            network_mode: NetworkMode::Brokered,
            secret_mode: SecretMode::TenantBroker,
            approval_policy: ApprovalPolicy::AskWrites,
            audit_mode: AuditMode::Standard,
        };
        assert!(policy.was_reduced());
    }

    #[test]
    fn backend_and_mode_enums_round_trip_through_serde() {
        for fs in [
            FilesystemBackendKind::ScopedVirtual,
            FilesystemBackendKind::HostWorkspace,
            FilesystemBackendKind::TenantWorkspace,
            FilesystemBackendKind::OrgDedicatedWorkspace,
        ] {
            let json = serde_json::to_string(&fs).unwrap();
            let parsed: FilesystemBackendKind = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, fs);
            assert_eq!(json, format!("\"{}\"", fs.as_str()));
        }
        for proc in [
            ProcessBackendKind::None,
            ProcessBackendKind::Docker,
            ProcessBackendKind::Srt,
            ProcessBackendKind::SmolVm,
            ProcessBackendKind::LocalHost,
            ProcessBackendKind::TenantSandbox,
            ProcessBackendKind::OrgDedicatedRunner,
        ] {
            let json = serde_json::to_string(&proc).unwrap();
            let parsed: ProcessBackendKind = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, proc);
            assert_eq!(json, format!("\"{}\"", proc.as_str()));
        }
        for net in [
            NetworkMode::Deny,
            NetworkMode::Brokered,
            NetworkMode::Allowlist,
            NetworkMode::DirectLogged,
            NetworkMode::Direct,
        ] {
            let json = serde_json::to_string(&net).unwrap();
            let parsed: NetworkMode = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, net);
        }
        for sec in [
            SecretMode::Deny,
            SecretMode::BrokeredHandles,
            SecretMode::TenantBroker,
            SecretMode::OrgBroker,
            SecretMode::ScrubbedEnv,
            SecretMode::InheritedEnv,
        ] {
            let json = serde_json::to_string(&sec).unwrap();
            let parsed: SecretMode = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, sec);
        }
        for ap in [
            ApprovalPolicy::AskAlways,
            ApprovalPolicy::AskWrites,
            ApprovalPolicy::AskDestructive,
            ApprovalPolicy::OrgPolicy,
            ApprovalPolicy::Minimal,
        ] {
            let json = serde_json::to_string(&ap).unwrap();
            let parsed: ApprovalPolicy = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, ap);
        }
        for au in [
            AuditMode::LocalMinimal,
            AuditMode::Standard,
            AuditMode::OrgPolicy,
        ] {
            let json = serde_json::to_string(&au).unwrap();
            let parsed: AuditMode = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, au);
        }
    }
}
