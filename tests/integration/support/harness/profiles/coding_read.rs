//! Template domain migration for the `ToolsProfile` infrastructure (see
//! `harness/options.rs`). `coding_read_tools()` previously lived as a
//! hand-built `Self { .. }` constructor in `harness/mod.rs`; this module is
//! the pattern later domain migrations follow.

use ironclaw_host_api::{CapabilityId, EffectKind, MountPermissions, UserId};
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
        user_id: UserId::new("reborn-e2e-coding-read-user")?,
        options: HostRuntimeHarnessOptions::new(
            workspace_mounts(MountPermissions::read_write_list_delete())?,
            None,
        ),
        auto_approve_default: Some(true),
        ..ToolsProfile::new("reborn-e2e-coding-read-tools")?
    })
}

/// Read-only coding tools (`list_dir`/`glob`/`grep`). Auto-approve is enabled
/// for the product and harness users so the model-visible surface dispatches
/// without a gate.
pub(crate) async fn coding_read_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    coding_read_tools_profile()?.build().await
}
