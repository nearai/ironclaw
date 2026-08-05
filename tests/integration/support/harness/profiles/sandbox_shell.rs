//! Minimal integration-harness profile for a real sandboxed shell turn.

use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{HarnessResult, HostRuntimeCapabilityHarness, workspace_mounts};
use ironclaw_host_api::{
    capability::EffectKind,
    ids::{AgentId, CapabilityId, TenantId, UserId},
    mount::MountPermissions,
};
use ironclaw_host_runtime::SHELL_CAPABILITY_ID;

pub(crate) async fn sandbox_shell_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    let runtime_policy =
        ironclaw_reborn_composition::hosted_single_tenant_volume_sandboxed_runtime_policy()?;
    let tenant_id = TenantId::new("tenant-itest")?;
    let user_id = UserId::new("host-user")?;
    let options = HostRuntimeHarnessOptions::new(
        workspace_mounts(MountPermissions::read_write_list_delete())?,
        Some(runtime_policy),
    )
    .with_local_runtime_identity(tenant_id, AgentId::new("sandbox-shell-agent")?)
    .with_sandboxed_shell()
    .with_durable_capability_io();

    ToolsProfile {
        capability_ids: vec![CapabilityId::new(SHELL_CAPABILITY_ID)?],
        effect_kinds: vec![
            EffectKind::DispatchCapability,
            EffectKind::ExecuteCode,
            EffectKind::SpawnProcess,
        ],
        options,
        auto_approve_default: Some(true),
        ..ToolsProfile::new("reborn-e2e-sandbox-shell-tools", user_id.as_str())?
    }
    .build()
    .await
}
