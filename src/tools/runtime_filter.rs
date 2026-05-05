//! Visible capability surface filter for #3045.
//!
//! Maps a [`Tool::runtime_affordance`] declaration against an
//! [`EffectiveRuntimePolicy`] to decide whether the tool should appear in
//! the model-facing tool list. This is **visibility only** — action-time
//! authorization (capability grants, approvals, resource checks) still
//! runs on every invocation regardless of visibility.
//!
//! The contract per #3045's "Visible capability surface" rules:
//!
//! - Profile-impossible affordances are hidden before the model call.
//! - Hosted multi-tenant surfaces never expose `LocalHost` shell or host
//!   workspace filesystem affordances — the resolver guarantees the
//!   `EffectiveRuntimePolicy` doesn't select those backends, so the
//!   filter naturally hides any tool that declares them.
//! - Capabilities that are possible but approval/auth/resource-dependent
//!   stay visible and fail structurally at action time.

use ironclaw_host_api::runtime_policy::{
    EffectiveRuntimePolicy, FilesystemBackendKind, NetworkMode, ProcessBackendKind,
};

use crate::tools::tool::ToolRuntimeAffordance;

/// `true` if a tool with the given affordance declaration should be
/// visible in the model-facing tool list under `policy`.
///
/// `ToolRuntimeAffordance::None` is always visible. The other variants
/// match the resolved policy's backend/mode choice as documented on
/// each variant.
pub fn is_visible_under(
    policy: &EffectiveRuntimePolicy,
    affordance: ToolRuntimeAffordance,
) -> bool {
    match affordance {
        ToolRuntimeAffordance::None => true,
        ToolRuntimeAffordance::AnyProcess => {
            !matches!(policy.process_backend, ProcessBackendKind::None)
        }
        ToolRuntimeAffordance::LocalShell => {
            matches!(policy.process_backend, ProcessBackendKind::LocalHost)
        }
        ToolRuntimeAffordance::HostFilesystem => {
            matches!(
                policy.filesystem_backend,
                FilesystemBackendKind::HostWorkspace
            )
        }
        ToolRuntimeAffordance::DirectNetwork => {
            matches!(
                policy.network_mode,
                NetworkMode::Direct | NetworkMode::DirectLogged
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::runtime_policy::{
        ApprovalPolicy, AuditMode, DeploymentMode, RuntimeProfile, SecretMode,
    };

    fn policy(
        process_backend: ProcessBackendKind,
        filesystem_backend: FilesystemBackendKind,
        network_mode: NetworkMode,
    ) -> EffectiveRuntimePolicy {
        EffectiveRuntimePolicy {
            deployment: DeploymentMode::LocalSingleUser,
            requested_profile: RuntimeProfile::SecureDefault,
            resolved_profile: RuntimeProfile::SecureDefault,
            filesystem_backend,
            process_backend,
            network_mode,
            secret_mode: SecretMode::BrokeredHandles,
            approval_policy: ApprovalPolicy::AskAlways,
            audit_mode: AuditMode::LocalMinimal,
        }
    }

    #[test]
    fn none_affordance_is_always_visible() {
        // Under SecureDefault-shaped policy with no process backend, even.
        let p = policy(
            ProcessBackendKind::None,
            FilesystemBackendKind::ScopedVirtual,
            NetworkMode::Brokered,
        );
        assert!(is_visible_under(&p, ToolRuntimeAffordance::None));
    }

    #[test]
    fn any_process_hidden_when_process_backend_is_none() {
        let p = policy(
            ProcessBackendKind::None,
            FilesystemBackendKind::ScopedVirtual,
            NetworkMode::Brokered,
        );
        assert!(!is_visible_under(&p, ToolRuntimeAffordance::AnyProcess));
    }

    #[test]
    fn any_process_visible_under_every_real_process_backend() {
        for backend in [
            ProcessBackendKind::Docker,
            ProcessBackendKind::Srt,
            ProcessBackendKind::SmolVm,
            ProcessBackendKind::LocalHost,
            ProcessBackendKind::TenantSandbox,
            ProcessBackendKind::OrgDedicatedRunner,
        ] {
            let p = policy(
                backend,
                FilesystemBackendKind::ScopedVirtual,
                NetworkMode::Brokered,
            );
            assert!(
                is_visible_under(&p, ToolRuntimeAffordance::AnyProcess),
                "AnyProcess should be visible under {backend:?}"
            );
        }
    }

    #[test]
    fn local_shell_visible_only_under_local_host_process() {
        for (backend, expect) in [
            (ProcessBackendKind::LocalHost, true),
            (ProcessBackendKind::Docker, false),
            (ProcessBackendKind::TenantSandbox, false),
            (ProcessBackendKind::OrgDedicatedRunner, false),
            (ProcessBackendKind::None, false),
        ] {
            let p = policy(
                backend,
                FilesystemBackendKind::ScopedVirtual,
                NetworkMode::Brokered,
            );
            assert_eq!(
                is_visible_under(&p, ToolRuntimeAffordance::LocalShell),
                expect,
                "LocalShell visibility under {backend:?}"
            );
        }
    }

    #[test]
    fn host_filesystem_visible_only_under_host_workspace_backend() {
        for (fs, expect) in [
            (FilesystemBackendKind::HostWorkspace, true),
            (FilesystemBackendKind::ScopedVirtual, false),
            (FilesystemBackendKind::TenantWorkspace, false),
            (FilesystemBackendKind::OrgDedicatedWorkspace, false),
        ] {
            let p = policy(ProcessBackendKind::None, fs, NetworkMode::Brokered);
            assert_eq!(
                is_visible_under(&p, ToolRuntimeAffordance::HostFilesystem),
                expect,
                "HostFilesystem visibility under {fs:?}"
            );
        }
    }

    #[test]
    fn direct_network_visible_only_under_direct_modes() {
        for (mode, expect) in [
            (NetworkMode::Direct, true),
            (NetworkMode::DirectLogged, true),
            (NetworkMode::Allowlist, false),
            (NetworkMode::Brokered, false),
            (NetworkMode::Deny, false),
        ] {
            let p = policy(
                ProcessBackendKind::None,
                FilesystemBackendKind::ScopedVirtual,
                mode,
            );
            assert_eq!(
                is_visible_under(&p, ToolRuntimeAffordance::DirectNetwork),
                expect,
                "DirectNetwork visibility under {mode:?}"
            );
        }
    }
}
