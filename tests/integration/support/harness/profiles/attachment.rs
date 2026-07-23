//! Attachment domain tools profile (`attachment_tools`).

use ironclaw_host_api::{EffectKind, MountView};

use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{HarnessResult, HostRuntimeCapabilityHarness};

/// Group with NO first-party capability dispatch — the test drives the
/// C-ATTACH seam purely through the attachment read port + inbound lander,
/// never a tool call. Uses `new_with_options` (mirrors `profile_tools()`),
/// so `attachment_test_support` is populated from
/// `services.local_dev_attachment_test_support_for_test()`. No mounts needed:
/// attachment landing/reading goes through `local_runtime.workspace_filesystem`
/// directly, not the capability-dispatch `MountView` (mirrors
/// `trigger_management_tools()`'s `MountView::default()`, which also has no
/// filesystem capability to gate).
pub(crate) fn attachment_tools_profile() -> HarnessResult<ToolsProfile> {
    Ok(ToolsProfile {
        effect_kinds: vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
        options: HostRuntimeHarnessOptions::new(
            MountView::default(),
            Some(ironclaw_composition::local_dev_yolo_runtime_policy(true)?),
        ),
        ..ToolsProfile::new(
            "ironclaw-e2e-attachment-tools",
            "ironclaw-e2e-attachment-tools-user",
        )?
    })
}

/// See [`attachment_tools_profile`].
pub(crate) async fn attachment_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    attachment_tools_profile()?.build().await
}
