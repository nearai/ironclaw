//! trace_commons domain tools profiles (populated by the profile migration).

use ironclaw_host_api::{CapabilityId, EffectKind, MountView, UserId};
use ironclaw_host_runtime::{
    TRACE_COMMONS_CREDITS_CAPABILITY_ID, TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
    TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID, TRACE_COMMONS_PROFILE_TOKEN_CAPABILITY_ID,
    TRACE_COMMONS_STATUS_CAPABILITY_ID,
};

use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{HarnessResult, HostRuntimeCapabilityHarness, http_test_policy};

pub(crate) fn trace_commons_tools_profile() -> HarnessResult<ToolsProfile> {
    Ok(ToolsProfile {
        capability_ids: vec![
            CapabilityId::new(TRACE_COMMONS_ONBOARD_CAPABILITY_ID)?,
            CapabilityId::new(TRACE_COMMONS_STATUS_CAPABILITY_ID)?,
            CapabilityId::new(TRACE_COMMONS_CREDITS_CAPABILITY_ID)?,
            CapabilityId::new(TRACE_COMMONS_PROFILE_TOKEN_CAPABILITY_ID)?,
            CapabilityId::new(TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID)?,
        ],
        effect_kinds: vec![
            EffectKind::DispatchCapability,
            EffectKind::ReadFilesystem,
            // onboard persists device-key material (Ed25519 keypair +
            // policy.json) and profile_token writes profile_token.jwt, so
            // the harness allow-set must grant WriteFilesystem or those
            // capabilities are filtered out of the model-visible surface.
            EffectKind::WriteFilesystem,
            EffectKind::Network,
            EffectKind::ExternalWrite,
        ],
        user_id: UserId::new("reborn-e2e-trace-commons-user")?,
        // The Trace Commons write/network capabilities are
        // PermissionMode::Ask (onboard, profile_token, profile_set) — like
        // the skill/trigger harnesses, the scripted run enables global
        // auto-approve so it is not gated.
        options: HostRuntimeHarnessOptions::new(
            MountView::default(),
            Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                true,
            )?),
        ),
        // onboard declares EffectKind::Network, so the lease must carry a
        // non-empty network policy or the obligation check rejects dispatch
        // before the consent gate runs.
        network_policy_override: Some(http_test_policy()),
        auto_approve_default: Some(true),
        ..ToolsProfile::new("reborn-e2e-trace-commons-tools")?
    })
}

pub(crate) async fn trace_commons_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    trace_commons_tools_profile()?.build().await
}
