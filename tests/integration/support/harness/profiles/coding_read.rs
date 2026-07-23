//! Coding-read domain tools profile (`coding_read_tools`) — reference example
//! of the `ToolsProfile` pattern (see `harness/options.rs`).

use ironclaw_host_api::{CapabilityId, EffectKind, MountPermissions};
use ironclaw_host_runtime::{GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID, LIST_DIR_CAPABILITY_ID};

use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{HarnessResult, HostRuntimeCapabilityHarness, workspace_mounts};

/// Read-only coding tools (`list_dir`/`glob`/`grep`). Auto-approve is enabled
/// for the product and harness users so the model-visible surface dispatches
/// without a gate.
pub(crate) fn coding_read_tools_profile() -> HarnessResult<ToolsProfile> {
    Ok(ToolsProfile {
        capability_ids: vec![
            CapabilityId::new(LIST_DIR_CAPABILITY_ID)?,
            CapabilityId::new(GLOB_CAPABILITY_ID)?,
            CapabilityId::new(GREP_CAPABILITY_ID)?,
        ],
        effect_kinds: vec![EffectKind::ReadFilesystem],
        options: HostRuntimeHarnessOptions::new(
            workspace_mounts(MountPermissions::read_write_list_delete())?,
            None,
        ),
        auto_approve_default: Some(true),
        ..ToolsProfile::new(
            "ironclaw-e2e-coding-read-tools",
            "ironclaw-e2e-coding-read-user",
        )?
    })
}

/// See [`coding_read_tools_profile`].
pub(crate) async fn coding_read_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    coding_read_tools_profile()?.build().await
}
