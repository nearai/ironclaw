//! Project domain tools profiles (`project_tools`, `project_tools_with_fault_injection`).

use ironclaw_host_api::{CapabilityId, EffectKind, MountView};

use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{HarnessResult, HostRuntimeCapabilityHarness};

/// E-PROJ: harness surfacing the local-dev synthetic `project_create`
/// capability. `create_capability_port` injects the synthetic capability via
/// `apply_synthetic_capability_wrappers` because `PROJECT_CREATE_CAPABILITY_ID`
/// is in the allowlist. Auto-approve is enabled so the capability dispatches
/// without a gate.
pub(crate) fn project_tools_profile() -> HarnessResult<ToolsProfile> {
    Ok(ToolsProfile {
        capability_ids: vec![CapabilityId::new(
            ironclaw_composition::test_support::PROJECT_CREATE_CAPABILITY_ID,
        )?],
        effect_kinds: vec![
            EffectKind::DispatchCapability,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
        ],
        options: HostRuntimeHarnessOptions::new(
            MountView::default(),
            Some(ironclaw_composition::local_dev_yolo_runtime_policy(true)?),
        ),
        auto_approve_default: Some(true),
        ..ToolsProfile::new(
            "ironclaw-e2e-project-tools",
            "ironclaw-e2e-project-tools-user",
        )?
    })
}

/// See [`project_tools_profile`].
pub(crate) async fn project_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    project_tools_profile()?.build().await
}

/// C-SYNTH `project_create` fault-injection arm: same surface as
/// `project_tools()`, but the real `Arc<dyn ProjectService>` is wrapped in
/// `FaultInjectingProjectService`
/// (`with_project_service_fault_injection`) so a `create_project` call
/// naming `FAULT_INJECT_DENIED_PROJECT_NAME` returns
/// `ProjectServiceError::Denied`/`PolicyDenied` and proves the real
/// capability dispatch's recoverable `Failed` behavior. This is *not*
/// the `project_service_outcome` `Unavailable` / internal-retry path.
/// Any other `create_project` name still reaches the real store.
pub(crate) fn project_tools_with_fault_injection_profile() -> HarnessResult<ToolsProfile> {
    Ok(ToolsProfile {
        capability_ids: vec![CapabilityId::new(
            ironclaw_composition::test_support::PROJECT_CREATE_CAPABILITY_ID,
        )?],
        effect_kinds: vec![
            EffectKind::DispatchCapability,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
        ],
        options: HostRuntimeHarnessOptions::new(
            MountView::default(),
            Some(ironclaw_composition::local_dev_yolo_runtime_policy(true)?),
        )
        .with_project_service_fault_injection(),
        auto_approve_default: Some(true),
        ..ToolsProfile::new(
            "ironclaw-e2e-project-tools-fault-injection",
            "ironclaw-e2e-project-tools-fault-injection-user",
        )?
    })
}

/// See [`project_tools_with_fault_injection_profile`].
pub(crate) async fn project_tools_with_fault_injection()
-> HarnessResult<HostRuntimeCapabilityHarness> {
    project_tools_with_fault_injection_profile()?.build().await
}
