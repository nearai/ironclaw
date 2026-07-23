//! Process domain tools profile (`process_tools`) — see `harness/options.rs`
//! for the `ToolsProfile` pattern.

use ironclaw_host_api::{CapabilityId, EffectKind, MountView};
use ironclaw_host_runtime::{
    ECHO_CAPABILITY_ID, SHELL_CAPABILITY_ID, SPAWN_SUBAGENT_CAPABILITY_ID,
};

use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{HarnessResult, HostRuntimeCapabilityHarness};

pub(crate) fn process_tools_profile() -> HarnessResult<ToolsProfile> {
    Ok(ToolsProfile {
        capability_ids: vec![
            CapabilityId::new(ECHO_CAPABILITY_ID)?,
            CapabilityId::new(SHELL_CAPABILITY_ID)?,
            CapabilityId::new(SPAWN_SUBAGENT_CAPABILITY_ID)?,
        ],
        effect_kinds: vec![
            EffectKind::DispatchCapability,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::Network,
            EffectKind::SpawnProcess,
            EffectKind::ExecuteCode,
        ],
        options: HostRuntimeHarnessOptions::new(MountView::default(), None),
        auto_approve_default: Some(true),
        ..ToolsProfile::new("ironclaw-e2e-process-tools", "ironclaw-e2e-process-user")?
    })
}

pub(crate) async fn process_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    process_tools_profile()?.build().await
}
