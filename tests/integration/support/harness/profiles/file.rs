//! File domain tools profiles: `file_tools()` / `file_tools_requiring_approval()`
//! / `write_only()`, sharing `file_tools_with_runtime_policy` as their
//! internal tail. See `harness/options.rs` for the `ToolsProfile` pattern.

use ironclaw_host_api::{CapabilityId, EffectKind, MountPermissions, UserId};
use ironclaw_host_runtime::{
    JSON_CAPABILITY_ID, READ_FILE_CAPABILITY_ID, WRITE_FILE_CAPABILITY_ID,
};

use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{HarnessResult, HostRuntimeCapabilityHarness, workspace_mounts};

fn file_tools_with_runtime_policy(
    runtime_policy: Option<ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy>,
) -> HarnessResult<ToolsProfile> {
    Ok(ToolsProfile {
        capability_ids: vec![
            CapabilityId::new(WRITE_FILE_CAPABILITY_ID)?,
            CapabilityId::new(READ_FILE_CAPABILITY_ID)?,
        ],
        effect_kinds: vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
        options: HostRuntimeHarnessOptions::new(
            workspace_mounts(MountPermissions::read_write_list_delete())?,
            runtime_policy,
        ),
        ..ToolsProfile::new("reborn-e2e-builtin-tools", "reborn-e2e-builtin-user")?
    })
}

pub(crate) fn file_tools_profile() -> HarnessResult<ToolsProfile> {
    Ok(file_tools_with_runtime_policy(Some(
        ironclaw_reborn_composition::local_dev_yolo_runtime_policy(true)?,
    ))?
    .with_auto_approve_default(true))
}

pub(crate) async fn file_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    file_tools_profile()?.build().await
}

pub(crate) fn file_tools_requiring_approval_profile() -> HarnessResult<ToolsProfile> {
    file_tools_requiring_approval_profile_for_user("reborn-e2e-builtin-user")
}

/// Same profile as [`file_tools_requiring_approval_profile`], but disables
/// global auto-approve under a caller-supplied `user_id` instead of the fixed
/// test constant. The production capability port resolves the dispatch scope
/// owner-first from the turn's real binding subject, not from this harness's
/// `user_id` field alone -- a caller whose turn runs as a different actor
/// (e.g. `RebornBinaryE2EHarness::with_host_runtime_file_capabilities_requiring_approval`,
/// which submits as a fixed `"alice"` actor) must build under that SAME
/// resolved subject, mirroring `extension_lifecycle_tools_profile_for_user`,
/// or `disable_global_auto_approve_for_product_and_harness_users` disables
/// the wrong `(tenant, user)` scope and global auto-approve's default-ON
/// value silently lets the gate through.
pub(crate) fn file_tools_requiring_approval_profile_for_user(
    user_id: &str,
) -> HarnessResult<ToolsProfile> {
    // Global auto-approve now defaults ON, so disable it explicitly to keep
    // this constructor's per-tool approval gate behavior.
    Ok(file_tools_with_runtime_policy(None)?
        .with_user_id(UserId::new(user_id)?)
        .with_auto_approve_default(false))
}

pub(crate) async fn file_tools_requiring_approval() -> HarnessResult<HostRuntimeCapabilityHarness> {
    file_tools_requiring_approval_profile()?.build().await
}

/// Same capability set as [`file_tools`], but opts the harness into the real
/// `StagedCapabilityIo` (durable tool-result projection seam, issue #5838)
/// instead of the ephemeral `ProductLiveCapabilityIo` test double, so
/// `read_file`'s large output is persisted durably and `result_read` can page
/// through it. Auto-approve on, like `file_tools`.
pub(crate) fn file_tools_with_durable_capability_io_profile() -> HarnessResult<ToolsProfile> {
    let mut profile = file_tools_with_runtime_policy(Some(
        ironclaw_reborn_composition::local_dev_yolo_runtime_policy(true)?,
    ))?
    .with_auto_approve_default(true);
    profile.options = std::mem::take(&mut profile.options).with_durable_capability_io();
    // Grants the synthetic `result_read` id so
    // `apply_synthetic_capability_wrappers` wraps it onto this harness's
    // port (mirrors `project_create`'s `PROJECT_CREATE_CAPABILITY_ID`
    // opt-in pattern -- see `profiles/project.rs`).
    profile.capability_ids.push(CapabilityId::new(
        ironclaw_reborn_composition::test_support::RESULT_READ_CAPABILITY_ID,
    )?);
    // `builtin.json` (`parse`) is the minimal granted capability whose output
    // is a top-level JSON array, needed to drive the truncated-array
    // `item_count` observation through this durable-io seam.
    profile
        .capability_ids
        .push(CapabilityId::new(JSON_CAPABILITY_ID)?);
    profile.effect_kinds.push(EffectKind::DispatchCapability);
    Ok(profile)
}

pub(crate) async fn file_tools_with_durable_capability_io()
-> HarnessResult<HostRuntimeCapabilityHarness> {
    file_tools_with_durable_capability_io_profile()?
        .build()
        .await
}

pub(crate) fn write_only_profile() -> HarnessResult<ToolsProfile> {
    Ok(ToolsProfile {
        capability_ids: vec![CapabilityId::new(WRITE_FILE_CAPABILITY_ID)?],
        effect_kinds: vec![EffectKind::WriteFilesystem],
        options: HostRuntimeHarnessOptions::new(
            workspace_mounts(MountPermissions::read_write_list_delete())?,
            None,
        ),
        ..ToolsProfile::new("reborn-e2e-write-only", "reborn-e2e-write-only-user")?
    })
}

pub(crate) async fn write_only() -> HarnessResult<HostRuntimeCapabilityHarness> {
    write_only_profile()?.build().await
}
