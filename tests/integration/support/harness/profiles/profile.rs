//! Profile domain tools profile (`profile_tools`).

use ironclaw_host_api::{CapabilityId, EffectKind, MountPermissions};
use ironclaw_host_runtime::PROFILE_SET_CAPABILITY_ID;

use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{HarnessResult, HostRuntimeCapabilityHarness, memory_mounts};

/// Group whose ONLY capability is `builtin.profile_set` (E-PROFILE seam).
/// Uses `new_with_options` (not `core_builtin_tools_from_runtime`), so
/// `profile_filesystem` is populated from `services.local_dev_profile_filesystem_for_test()`
/// — the read-back half of the round trip a `IronClawIntegrationGroup::profile_tools()`
/// scenario needs. Base mounts are `/memory` directly (this harness's only
/// capability needs it; no per-capability mount override required, unlike
/// `core_builtin_tools_from_runtime`'s multi-capability surface).
pub(crate) fn profile_tools_profile() -> HarnessResult<ToolsProfile> {
    Ok(ToolsProfile {
        capability_ids: vec![CapabilityId::new(PROFILE_SET_CAPABILITY_ID)?],
        effect_kinds: vec![
            EffectKind::DispatchCapability,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
        ],
        options: HostRuntimeHarnessOptions::new(
            memory_mounts(MountPermissions::read_write_list_delete())?,
            Some(ironclaw_composition::local_dev_yolo_runtime_policy(true)?),
        ),
        auto_approve_default: Some(true),
        ..ToolsProfile::new(
            "ironclaw-e2e-profile-tools",
            "ironclaw-e2e-profile-tools-user",
        )?
    })
}

/// See [`profile_tools_profile`].
pub(crate) async fn profile_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    profile_tools_profile()?.build().await
}
