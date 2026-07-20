//! trace_commons domain capability profile.

use ironclaw_host_api::{CapabilityId, EffectKind, MountView};
use ironclaw_host_runtime::{
    TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID, TRACE_COMMONS_CREDITS_CAPABILITY_ID,
    TRACE_COMMONS_ONBOARD_CAPABILITY_ID, TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID,
    TRACE_COMMONS_PROFILE_TOKEN_CAPABILITY_ID, TRACE_COMMONS_STATUS_CAPABILITY_ID,
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
            CapabilityId::new(TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID)?,
        ],
        effect_kinds: vec![
            EffectKind::DispatchCapability,
            EffectKind::ReadFilesystem,
            // onboard/profile_token write device-key material and profile_token.jwt to disk;
            // WriteFilesystem must stay in the allow-set or these capabilities get filtered out.
            EffectKind::WriteFilesystem,
            EffectKind::Network,
            EffectKind::ExternalWrite,
        ],
        // onboard/profile_token/profile_set are PermissionMode::Ask; auto-approve is
        // enabled here so the scripted run isn't gated.
        options: HostRuntimeHarnessOptions::new(
            MountView::default(),
            Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                true,
            )?),
        ),
        // onboard declares EffectKind::Network, so the lease needs a non-empty network
        // policy or the obligation check rejects dispatch before the consent gate runs.
        network_policy_override: Some(http_test_policy()),
        auto_approve_default: Some(true),
        ..ToolsProfile::new(
            "reborn-e2e-trace-commons-tools",
            "reborn-e2e-trace-commons-user",
        )?
    })
}

pub(crate) async fn trace_commons_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    trace_commons_tools_profile()?.build().await
}
