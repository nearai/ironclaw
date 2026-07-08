//! trigger domain capability profile.

use ironclaw_host_api::{CapabilityId, EffectKind, MountView};
use ironclaw_host_runtime::{
    TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID, TRIGGER_PAUSE_CAPABILITY_ID,
    TRIGGER_REMOVE_CAPABILITY_ID, TRIGGER_RESUME_CAPABILITY_ID,
};

use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{HarnessResult, HostRuntimeCapabilityHarness};

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
