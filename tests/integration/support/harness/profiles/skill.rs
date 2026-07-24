//! Skill domain tools profiles.

use ironclaw_host_api::{CapabilityId, EffectKind, TenantId};
use ironclaw_host_runtime::{
    SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY_ID,
};

use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{HarnessResult, HostRuntimeCapabilityHarness, http_test_policy, skill_mounts};

/// `pub(crate)`: also used by `RebornIntegrationGroupBuilder::skill_management_tools`
/// (`group_constructors.rs`, C-SKILL) to wire the SAME preset onto the
/// int-tier group, so the QA/trace-tier smoke test and the int-tier group
/// never drift on capability ids / mounts / policy.
pub(crate) fn skill_management_tools_profile() -> HarnessResult<ToolsProfile> {
    Ok(ToolsProfile {
        capability_ids: vec![
            CapabilityId::new(SKILL_LIST_CAPABILITY_ID)?,
            CapabilityId::new(SKILL_INSTALL_CAPABILITY_ID)?,
            CapabilityId::new(SKILL_REMOVE_CAPABILITY_ID)?,
        ],
        effect_kinds: vec![
            EffectKind::DispatchCapability,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::DeleteFilesystem,
            EffectKind::Network,
        ],
        options: HostRuntimeHarnessOptions::new(
            skill_mounts()?,
            Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                true,
            )?),
        ),
        network_policy_override: Some(http_test_policy()),
        auto_approve_default: Some(true),
        ..ToolsProfile::new(
            "reborn-e2e-skill-management-tools",
            "reborn-e2e-skill-management-user",
        )?
    })
}

/// See [`skill_management_tools_profile`].
pub(crate) async fn skill_management_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    skill_management_tools_profile()?.build().await
}

/// Harness surfacing the local-dev synthetic `skill_activate` capability
/// (E-SKILL seam). `new_with_options` builds the `skill_activation_source`
/// (because `SKILL_ACTIVATE_CAPABILITY_ID` is in the allowlist) under
/// `tenant` — the caller's ACTUAL group run-scope tenant, passed through
/// rather than re-hardcoded here — which `create_capability_port` wraps
/// onto the port and `into_group` wires as the runtime's
/// `skill_context_source`. The skill file the model activates is seeded as
/// a system-scoped skill by `RebornIntegrationGroup::skill_activation_tools`.
/// Mirrors `skill_management_tools`/`project_tools`.
pub(crate) fn skill_activation_tools_profile(tenant: &TenantId) -> HarnessResult<ToolsProfile> {
    Ok(ToolsProfile {
        capability_ids: vec![CapabilityId::new(
            ironclaw_reborn_composition::test_support::SKILL_ACTIVATE_CAPABILITY_ID,
        )?],
        effect_kinds: vec![
            EffectKind::DispatchCapability,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::Network,
        ],
        options: HostRuntimeHarnessOptions::new(
            skill_mounts()?,
            Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                true,
            )?),
        )
        .with_skill_activation_tenant(tenant.clone())
        .with_system_skill_fixture(
            "greet",
            "greets the user warmly",
            "GREET_SKILL_PROMPT_SENTINEL",
        )
        .with_system_skill_fixture(
            "bloat",
            "an oversized skill",
            "BLOAT_SKILL_FILLER ".repeat(2200),
        )
        .with_system_skill_fixture(
            "duplicate",
            "a system-scoped skill",
            "SYSTEM_DUPLICATE_SKILL_SENTINEL",
        ),
        network_policy_override: Some(http_test_policy()),
        auto_approve_default: Some(true),
        ..ToolsProfile::new(
            "reborn-e2e-skill-activation-tools",
            "reborn-e2e-skill-activation-user",
        )?
    })
}

/// See [`skill_activation_tools_profile`].
pub(crate) async fn skill_activation_tools(
    tenant: &TenantId,
) -> HarnessResult<HostRuntimeCapabilityHarness> {
    skill_activation_tools_profile(tenant)?.build().await
}
