//! Extension-installation durable-store test support (E-DURABLE seam).

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
