//! trigger domain capability profile.

use ironclaw_host_api::{CapabilityId, EffectKind, MountPermissions, MountView};
use ironclaw_host_runtime::{
    TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID, TRIGGER_PAUSE_CAPABILITY_ID,
    TRIGGER_REMOVE_CAPABILITY_ID, TRIGGER_RESUME_CAPABILITY_ID, WRITE_FILE_CAPABILITY_ID,
};

use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{HarnessResult, HostRuntimeCapabilityHarness, workspace_mounts};

pub(crate) fn trigger_management_tools_profile() -> HarnessResult<ToolsProfile> {
    Ok(ToolsProfile {
        capability_ids: vec![
            CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID)?,
            CapabilityId::new(TRIGGER_LIST_CAPABILITY_ID)?,
            CapabilityId::new(TRIGGER_PAUSE_CAPABILITY_ID)?,
            CapabilityId::new(TRIGGER_RESUME_CAPABILITY_ID)?,
            CapabilityId::new(TRIGGER_REMOVE_CAPABILITY_ID)?,
        ],
        effect_kinds: vec![EffectKind::DispatchCapability, EffectKind::ExternalWrite],
        options: HostRuntimeHarnessOptions::new(
            MountView::default(),
            Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                true,
            )?),
        ),
        auto_approve_default: Some(true),
        ..ToolsProfile::new(
            "reborn-e2e-trigger-management-tools",
            "reborn-e2e-trigger-management-user",
        )?
    })
}

pub(crate) async fn trigger_management_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    trigger_management_tools_profile()?.build().await
}

/// Trigger verbs PLUS `builtin.write_file` on ONE runtime (#5886 hold
/// visibility): auto-approve stays ON so the verbs dispatch gate-free, and a
/// scenario installs an `AskEachTime` override to gate only the write.
pub(crate) fn trigger_management_with_gated_write_profile() -> HarnessResult<ToolsProfile> {
    let mut profile = trigger_management_tools_profile()?;
    profile
        .capability_ids
        .push(CapabilityId::new(WRITE_FILE_CAPABILITY_ID)?);
    profile.effect_kinds.push(EffectKind::WriteFilesystem);
    profile.options = HostRuntimeHarnessOptions::new(
        workspace_mounts(MountPermissions::read_write_list_delete())?,
        Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
            true,
        )?),
    )
    // #5886: this profile's group asserts `trigger_list`'s `active_hold`
    // against a REAL gate-parked run, which lives in the group's shared
    // turn-state store, not this harness's own — see
    // `install_trigger_active_run_lookup_for_test`'s doc.
    .with_trigger_active_run_lookup_for_test();
    Ok(profile)
}

pub(crate) async fn trigger_management_with_gated_write()
-> HarnessResult<HostRuntimeCapabilityHarness> {
    trigger_management_with_gated_write_profile()?.build().await
}
