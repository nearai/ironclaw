//! File domain tools profiles: `file_tools()` / `file_tools_requiring_approval()`
//! / `write_only()`, sharing `file_tools_with_runtime_policy` as their
//! internal tail. See `harness/options.rs` for the `ToolsProfile` pattern.

use ironclaw_host_api::{CapabilityId, EffectKind, MountPermissions};
use ironclaw_host_runtime::{READ_FILE_CAPABILITY_ID, WRITE_FILE_CAPABILITY_ID};

use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{HarnessResult, HostRuntimeCapabilityHarness, workspace_mounts};

fn file_tools_with_runtime_policy(
    runtime_policy: Option<ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy>,
) -> HarnessResult<ToolsProfile> {
    Ok(ToolsProfile {
        capability_ids: vec![
            CapabilityId::new(WRITE_FILE_CAPABILITY_ID)?,
            CapabilityId::new(READ_FILE_CAPABILITY_ID)?,
        ],
        effect_kinds: vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
        options: HostRuntimeHarnessOptions::new(
            workspace_mounts(MountPermissions::read_write_list_delete())?,
            runtime_policy,
        ),
        ..ToolsProfile::new("reborn-e2e-builtin-tools", "reborn-e2e-builtin-user")?
    })
}

pub(crate) fn file_tools_profile() -> HarnessResult<ToolsProfile> {
    Ok(file_tools_with_runtime_policy(Some(
        ironclaw_reborn_composition::local_dev_yolo_runtime_policy(true)?,
    ))?
    .with_auto_approve_default(true))
}

pub(crate) async fn file_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    file_tools_profile()?.build().await
}

pub(crate) fn file_tools_requiring_approval_profile() -> HarnessResult<ToolsProfile> {
    // Global auto-approve now defaults ON, so disable it explicitly to keep
    // this constructor's per-tool approval gate behavior.
    Ok(file_tools_with_runtime_policy(None)?.with_auto_approve_default(false))
}

pub(crate) async fn file_tools_requiring_approval() -> HarnessResult<HostRuntimeCapabilityHarness> {
    file_tools_requiring_approval_profile()?.build().await
}

pub(crate) fn write_only_profile() -> HarnessResult<ToolsProfile> {
    Ok(ToolsProfile {
        capability_ids: vec![CapabilityId::new(WRITE_FILE_CAPABILITY_ID)?],
        effect_kinds: vec![EffectKind::WriteFilesystem],
        options: HostRuntimeHarnessOptions::new(
            workspace_mounts(MountPermissions::read_write_list_delete())?,
            None,
        ),
        ..ToolsProfile::new("reborn-e2e-write-only", "reborn-e2e-write-only-user")?
    })
}

pub(crate) async fn write_only() -> HarnessResult<HostRuntimeCapabilityHarness> {
    write_only_profile()?.build().await
}
