//! Reborn integration-test framework local-dev boot accessors.
//!
//! `build_local_dev_approval_gate_evidence_for_test`,
//! `build_default_local_dev_database_roots_for_test`,
//! `mount_local_dev_database_roots_for_test`,
//! `build_local_dev_secret_store_for_test` — mirror the production local-dev
//! boot sequence so the integration-test harness (`tests/support/reborn/`)
//! drives the real local-dev composition paths without duplicating the wiring
//! logic.

/// Filename of the local-dev libSQL database within the per-user root
/// directory. Value is derived from the production factory constant so
/// there is one owner of the string; tests access it through this
/// test-support surface.
pub const LOCAL_DEV_DB_FILENAME: &str = crate::factory::LOCAL_DEV_DB_FILENAME;

/// Test-only accessor mirroring the full local-dev database-roots boot path
/// (`build_local_dev_root_filesystem` → `build_default_local_dev_database_roots`).
///
/// Constructs the durable database backend and mounts it across the
/// control-plane roots (`/tenants`, `/memory`, `/events`) of `composite`,
/// selecting the backend by compile-time feature:
/// - With `libsql`: opens `root/reborn-local-dev.db`, runs migrations, mounts.
/// - Without a durable backend feature: mounts an in-memory backend.
///
/// Called by the Reborn integration-test framework's `StorageMode::LibSql`
/// builder arm (`tests/support/reborn/builder.rs:build_storage_composite`) so
/// the 4-step libSQL setup sequence lives once (production call site:
/// `build_local_dev_root_filesystem` → `build_default_local_dev_database_roots`).
/// For tests only — gated behind `test-support`, ships zero bytes in production.
#[cfg(feature = "test-support")]
pub async fn build_default_local_dev_database_roots_for_test(
    root: &std::path::Path,
    composite: &mut ironclaw_filesystem::CompositeRootFilesystem,
) -> Result<(), crate::RebornBuildError> {
    crate::factory::mount_default_local_dev_database_roots(root, composite).await
}

/// Test-only accessor mirroring the production local-dev boot path
/// (`build_local_dev_root_filesystem` → `mount_local_dev_database_roots`).
///
/// Mounts `database` across the control-plane roots (`/tenants`, `/memory`,
/// `/events`) of `root` exactly as the libSQL local-dev boot path does, so
/// downstream integration tests (the Reborn integration-test framework in
/// `tests/support/reborn/`) construct one real `LibSqlRootFilesystem` over a
/// composite without a second copy of the mount wiring (design spec §3.2).
/// For tests only — gated behind `test-support`, so it ships zero bytes in
/// production binaries.
#[cfg(feature = "test-support")]
pub fn mount_local_dev_database_roots_for_test<F>(
    root: &mut ironclaw_filesystem::CompositeRootFilesystem,
    database: std::sync::Arc<F>,
) -> Result<(), crate::RebornBuildError>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
{
    crate::factory::mount_local_dev_database_roots(root, database)
}

/// Test-only entry point for building a local-dev
/// [`ironclaw_secrets::FilesystemSecretStore`] without going through the full
/// Reborn runtime assembly.
///
/// Mirrors the production wiring in `build_local_runtime` where
/// `build_local_dev_secret_store` is called with the scoped filesystem and a
/// master key resolved from the environment or the root directory's cached key
/// file. Tests that need a real `FilesystemSecretStore` — for example, to
/// verify `put` + `lease_once` + `consume` round-trips against an in-process
/// backend — can call this instead of wiring a full runtime.
///
/// The master key is resolved exactly as production does: from the
/// `SECRETS_MASTER_KEY` env var when set, otherwise from (or generating to)
/// the `.reborn-local-dev-secrets-master-key` file under `root`. Using the
/// same `root` across two calls therefore yields the same key, so a second
/// `FilesystemSecretStore` over the same scoped filesystem can consume a
/// secret written by the first. For tests only — zero bytes shipped in
/// production builds.
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub async fn build_local_dev_secret_store_for_test<F>(
    root: &std::path::Path,
    scoped: std::sync::Arc<ironclaw_filesystem::ScopedFilesystem<F>>,
) -> Result<std::sync::Arc<ironclaw_secrets::FilesystemSecretStore<F>>, crate::RebornBuildError>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
{
    // `build_local_dev_secret_store` also returns the crypto (for the admin
    // secret provisioner); this test helper only needs the store.
    let (store, _crypto) = crate::factory::build_local_dev_secret_store(root, scoped, None).await?;
    Ok(store)
}

/// Mirrors the production approval-gate evidence wiring done by
/// `build_local_runtime` (runtime.rs ~line 2799) — returns the REAL
/// `LocalDevApprovalGateEvidence` so the gate-evidence lookup logic
/// (the `gate:approval-` prefix parse + `ApprovalStatus::Pending` check)
/// never drifts from production. Tests only.
///
/// Wired by the Reborn integration-test harness's one-runtime group assembly
/// (`tests/support/reborn/group.rs`'s `into_group`, which builds the group's
/// single planned runtime via `build_default_planned_runtime`) so a
/// `BlockedApproval` run is verified against the persisted `Pending` approval
/// request at loop exit and genuinely pauses — mirrors the production
/// `runtime.rs` path with the real type, never a hand-mirrored copy.
#[cfg(feature = "test-support")]
pub fn build_local_dev_approval_gate_evidence_for_test(
    approval_requests: std::sync::Arc<dyn ironclaw_run_state::ApprovalRequestStore>,
) -> std::sync::Arc<dyn ironclaw_runner::loop_exit_applier::ApprovalGateEvidenceStore> {
    crate::runtime::build_local_dev_approval_gate_evidence_for_test(approval_requests)
}
