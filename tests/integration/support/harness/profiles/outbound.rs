//! Outbound domain tools profile (`outbound_target_tools`).

use ironclaw_host_api::{CapabilityId, EffectKind, MountView};

use super::super::super::outbound_preferences::FakeOutboundPreferencesFacade;
use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{HarnessResult, HostRuntimeCapabilityHarness};

/// C-SYNTH outbound: harness surfacing the two local-dev synthetic
/// `outbound_delivery_*` capabilities over an injected
/// [`FakeOutboundPreferencesFacade`] double.
/// `create_capability_port` injects them via
/// `apply_synthetic_capability_wrappers` because
/// `outbound_target_tools` is `Some`. `target_set` runs with
/// `requires_approval = true`, so its settings decision is exercised for
/// real: global auto-approve (default ON) → `Allow`; a `Disabled` tool
/// override (`disable_outbound_target_set_tool`) → `Deny`; auto-approve
/// disabled → `Ask` (approval gate). The RETURNED harness leaves global
/// auto-approve at its default-ON state so the happy/`NotFound` arms
/// dispatch through `Allow`; the gate arm disables it per-test.
pub(crate) fn outbound_target_tools_profile() -> HarnessResult<ToolsProfile> {
    let facade = FakeOutboundPreferencesFacade::with_default_targets();
    Ok(ToolsProfile {
        capability_ids: vec![
            CapabilityId::new(
                ironclaw_composition::test_support::OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID,
            )?,
            CapabilityId::new(
                ironclaw_composition::test_support::OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID,
            )?,
        ],
        effect_kinds: vec![
            EffectKind::DispatchCapability,
            EffectKind::ExternalWrite,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
        ],
        options: HostRuntimeHarnessOptions::new(
            MountView::default(),
            Some(ironclaw_composition::local_dev_yolo_runtime_policy(true)?),
        )
        .with_outbound_target_tools(facade, true),
        ..ToolsProfile::new(
            "ironclaw-e2e-outbound-target-tools",
            "ironclaw-e2e-outbound-target-user",
        )?
    })
}

/// See [`outbound_target_tools_profile`].
pub(crate) async fn outbound_target_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    outbound_target_tools_profile()?.build().await
}
