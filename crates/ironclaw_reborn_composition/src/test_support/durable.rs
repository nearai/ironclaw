//! Durable-store test support for capability-produced state that outlives a
//! process restart: extension installs (E-DURABLE seam), and approval
//! requests + triggers (C-DURABLE seam). All three reopen at the SAME
//! on-disk local-dev `storage_root` — the capability harness's always-real
//! filesystem, independent of the integration harness's own `StorageMode`
//! (which governs only the SEPARATE thread/turn composite; see
//! `assert_reply_persists_after_reopen` for that one).

/// Test-support entry point (E-DURABLE seam): reopen a fresh, independent
/// extension-installation store at an existing local-dev `storage_root`. Lets
/// the integration harness prove capability-produced durable state survives a
/// reopen, paralleling `assert_reply_persists_after_reopen`. Delegates to the
/// production filesystem mounts + install-store load in `factory` so the reopen
/// path never drifts from `build_reborn_services`. Tests only.
#[cfg(feature = "test-support")]
pub async fn open_local_dev_extension_installation_store_for_test(
    storage_root: &std::path::Path,
) -> Result<
    std::sync::Arc<dyn ironclaw_extensions::ExtensionInstallationStore>,
    crate::RebornBuildError,
> {
    crate::factory::open_local_dev_extension_installation_store_for_test(storage_root).await
}

/// Test-support entry point (C-DURABLE): reopen a fresh, independent
/// `ApprovalRequestStore` at an existing local-dev `storage_root`. Mirrors
/// [`open_local_dev_extension_installation_store_for_test`] for approval-gate
/// records instead of extension installs. Tests only.
#[cfg(all(feature = "test-support", feature = "libsql"))]
pub async fn open_local_dev_approval_request_store_for_test(
    storage_root: &std::path::Path,
) -> Result<std::sync::Arc<dyn ironclaw_run_state::ApprovalRequestStore>, crate::RebornBuildError> {
    crate::factory::open_local_dev_approval_request_store_for_test(storage_root).await
}

/// Test-support entry point (C-DURABLE): reopen a fresh, independent
/// `TriggerRepository` at an existing local-dev `storage_root`. Mirrors
/// [`open_local_dev_extension_installation_store_for_test`] for triggers
/// instead of extension installs. Tests only.
#[cfg(all(feature = "test-support", feature = "libsql"))]
pub async fn open_local_dev_trigger_repository_for_test(
    storage_root: &std::path::Path,
) -> Result<std::sync::Arc<dyn ironclaw_triggers::TriggerRepository>, crate::RebornBuildError> {
    crate::factory::open_local_dev_trigger_repository_for_test(storage_root).await
}

/// Test-support entry point (W5-WEBUI-API-1 seam): reopen FRESH, independent
/// `ToolPermissionOverrideStore` / `AutoApproveSettingStore` /
/// `PersistentApprovalPolicyStore` handles at an existing local-dev
/// `storage_root`. Mirrors [`open_local_dev_extension_installation_store_for_test`]
/// for the tool-settings/approval-policy stores instead of extension installs
/// — lets a cold-reopen test prove settings state survives a fresh local-dev
/// store reopen rather than re-reading the same live `Arc`s. Tests only.
#[cfg(all(feature = "test-support", feature = "libsql"))]
pub async fn open_local_dev_approval_settings_stores_for_test(
    storage_root: &std::path::Path,
) -> Result<
    (
        std::sync::Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore>,
        std::sync::Arc<dyn ironclaw_approvals::AutoApproveSettingStore>,
        std::sync::Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStore>,
    ),
    crate::RebornBuildError,
> {
    crate::factory::open_local_dev_approval_settings_stores_for_test(storage_root).await
}
