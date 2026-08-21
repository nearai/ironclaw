//! pinned coding tools profiles (issue #7392 slice 3 registration seam).
//!
//! Grants the six pinned coding capability ids
//! (`builtin.read`/`builtin.write`/`builtin.edit`/`builtin.glob`/
//! `builtin.grep`/`builtin.bash`) over a read-write workspace mount, with the
//! composed runtime wired to the pinned coding built-in package + handlers
//! (`HostRuntimeHarnessOptions::with_coding_tools`). The approval arm mirrors
//! `file_tools_requiring_approval`: auto-approve OFF and no runtime policy,
//! so a scripted `write` raises a real `BlockedApproval` gate.

use ironclaw_host_api::{capability::EffectKind, ids::CapabilityId, mount::MountPermissions};
use ironclaw_host_runtime::{
    CODING_BASH_CAPABILITY_ID, CODING_EDIT_CAPABILITY_ID, CODING_GLOB_CAPABILITY_ID,
    CODING_GREP_CAPABILITY_ID, CODING_READ_CAPABILITY_ID, CODING_WRITE_CAPABILITY_ID,
};

use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{
    HarnessResult, HostRuntimeCapabilityHarness, http_test_policy, workspace_mounts,
};

fn coding_tools_with_runtime_policy(
    runtime_policy: Option<ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy>,
) -> HarnessResult<ToolsProfile> {
    Ok(ToolsProfile {
        // The temporary benchmark arm advertises only the pinned coding
        // capabilities so the comparison measures their contract directly.
        capability_ids: vec![
            CapabilityId::new(CODING_READ_CAPABILITY_ID)?,
            CapabilityId::new(CODING_WRITE_CAPABILITY_ID)?,
            CapabilityId::new(CODING_EDIT_CAPABILITY_ID)?,
            CapabilityId::new(CODING_GLOB_CAPABILITY_ID)?,
            CapabilityId::new(CODING_GREP_CAPABILITY_ID)?,
            CapabilityId::new(CODING_BASH_CAPABILITY_ID)?,
        ],
        effect_kinds: vec![
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::DeleteFilesystem,
            EffectKind::DispatchCapability,
            EffectKind::SpawnProcess,
            EffectKind::ExecuteCode,
            EffectKind::Network,
        ],
        options: HostRuntimeHarnessOptions::new(
            workspace_mounts(MountPermissions::read_write_list_delete())?,
            runtime_policy,
        )
        .with_coding_tools(),
        network_policy_override: Some(http_test_policy()),
        ..ToolsProfile::new("reborn-e2e-coding-tools", "reborn-e2e-coding-tools-user")?
    })
}

pub(crate) fn coding_tools_profile() -> HarnessResult<ToolsProfile> {
    Ok(coding_tools_with_runtime_policy(Some(
        ironclaw_composition::standalone_unrestricted_runtime_policy(true)?,
    ))?
    .with_auto_approve_default(true))
}

pub(crate) async fn coding_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    coding_tools_profile()?.build().await
}

/// Same pinned coding surface with auto-approve OFF and no runtime policy —
/// mirrors `file_tools_requiring_approval_profile` so a scripted coding
/// `write` raises a real approval gate through the ordinary gate path.
pub(crate) fn coding_tools_requiring_approval_profile() -> HarnessResult<ToolsProfile> {
    Ok(coding_tools_with_runtime_policy(None)?.with_auto_approve_default(false))
}
