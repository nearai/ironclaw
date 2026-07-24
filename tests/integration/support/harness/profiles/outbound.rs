//! Outbound domain tools profile (`outbound_target_tools`).

use ironclaw_host_api::{CapabilityId, EffectKind, MountView};

use super::super::super::outbound_preferences::FakeOutboundPreferencesFacade;
use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{HarnessResult, HostRuntimeCapabilityHarness};

/// Outbound-target harness surfacing the local-dev synthetic list/set
/// capabilities over an injected [`FakeOutboundPreferencesFacade`] double,
/// plus the normal first-party current-run routing capability backed by the
/// production registry, product routing service, and outbound state store.
/// `create_capability_port` injects list/set via synthetic wrappers because
/// `outbound_target_tools` is `Some`; route-current stays on the mediated
/// first-party lane. `target_set` runs with
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
                ironclaw_reborn_composition::test_support::OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID,
            )?,
            CapabilityId::new(
                ironclaw_reborn_composition::test_support::OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID,
            )?,
            CapabilityId::new(
                ironclaw_host_runtime::OUTBOUND_DELIVERY_TARGET_ROUTE_CURRENT_CAPABILITY_ID,
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
            Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                true,
            )?),
        )
        .with_outbound_target_tools(facade, true),
        ..ToolsProfile::new("reborn-e2e-outbound-target-tools", "reborn-e2e-outbound-target-user")?
    })
}

/// See [`outbound_target_tools_profile`].
pub(crate) async fn outbound_target_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    outbound_target_tools_profile()?.build().await
}
