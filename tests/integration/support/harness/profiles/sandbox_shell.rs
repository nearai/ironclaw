//! Minimal integration-harness profile for a real sandboxed shell turn.

use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{HarnessResult, HostRuntimeCapabilityHarness};
use ironclaw_host_api::{
    capability::EffectKind,
    ids::{AgentId, CapabilityId, InvocationId, TenantId, UserId},
    mount::{MountPermissions, MountView},
    resource::ResourceScope,
};
use ironclaw_host_runtime::SHELL_CAPABILITY_ID;

pub(crate) async fn sandbox_shell_tools(
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: AgentId,
) -> HarnessResult<HostRuntimeCapabilityHarness> {
    let runtime_policy =
        ironclaw_composition::hosted_single_tenant_volume_sandboxed_runtime_policy()?;
    let mounts = caller_workspace_mounts(&tenant_id, &user_id)?;
    let options = HostRuntimeHarnessOptions::new(mounts.clone(), Some(runtime_policy))
        .with_local_runtime_identity(tenant_id, agent_id)
        .with_sandboxed_shell()
        .with_workspace_scoped_per_caller()
        .with_durable_capability_io();

    let mut harness = ToolsProfile {
        capability_ids: vec![CapabilityId::new(SHELL_CAPABILITY_ID)?],
        effect_kinds: vec![
            EffectKind::DispatchCapability,
            EffectKind::ExecuteCode,
            EffectKind::SpawnProcess,
            EffectKind::Network,
        ],
        options,
        auto_approve_default: Some(true),
        ..ToolsProfile::new("reborn-e2e-sandbox-shell-tools", user_id.as_str())?
    }
    .build()
    .await?;
    // The generic capability-port test seam starts from the local-host builtin
    // policy. Mirror the user-sandbox policy's shell projection explicitly so
    // both the grant obligation and the execution context carry the caller's
    // mandatory workspace leaf.
    harness
        .capability_mount_overrides
        .push((CapabilityId::new(SHELL_CAPABILITY_ID)?, mounts));
    Ok(harness)
}

fn caller_workspace_mounts(tenant_id: &TenantId, user_id: &UserId) -> HarnessResult<MountView> {
    let scope = ResourceScope {
        tenant_id: tenant_id.clone(),
        user_id: user_id.clone(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    Ok(
        ironclaw_composition::test_support::scoped_workspace_mount_view_for_test(
            &scope,
            MountPermissions {
                execute: true,
                ..MountPermissions::read_write_list_delete()
            },
        )?,
    )
}
