//! Memory-context-injection domain tools profile (`memory_context_tools`).

use ironclaw_host_api::{CapabilityId, EffectKind, MountPermissions};
use ironclaw_host_runtime::MEMORY_WRITE_CAPABILITY_ID;

use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{HarnessResult, HostRuntimeCapabilityHarness, memory_mounts};

/// Group whose ONLY capability is `builtin.memory_write` (W4-MEMCTX-ENVELOPE
/// seam). Uses `new_with_options` (mirrors `profile_tools_profile`, not
/// `core_builtin_tools_from_runtime`), so `profile_filesystem` is populated
/// from `services.local_dev_profile_filesystem_for_test()` — the SAME raw
/// filesystem `ThreadBackedLoopContextPort`'s wired `memory_context_source`
/// reads through in `group.rs`'s `into_group`, so a seeded write is
/// discoverable via prompt-context injection, not just the tool round trip.
pub(crate) fn memory_context_tools_profile() -> HarnessResult<ToolsProfile> {
    Ok(ToolsProfile {
        capability_ids: vec![CapabilityId::new(MEMORY_WRITE_CAPABILITY_ID)?],
        effect_kinds: vec![
            EffectKind::DispatchCapability,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
        ],
        options: HostRuntimeHarnessOptions::new(
            memory_mounts(MountPermissions::read_write_list_delete())?,
            Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                true,
            )?),
        ),
        auto_approve_default: Some(true),
        ..ToolsProfile::new(
            "reborn-e2e-memory-context-tools",
            "reborn-e2e-memory-context-tools-user",
        )?
    })
}

/// See [`memory_context_tools_profile`].
pub(crate) async fn memory_context_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    memory_context_tools_profile()?.build().await
}
