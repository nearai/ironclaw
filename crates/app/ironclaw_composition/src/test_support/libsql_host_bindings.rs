use std::sync::Arc;

use crate::{RebornBuildError, RebornCompositionProfile, RebornHostBindings};

/// Build libSQL composition around a caller-supplied database handle.
///
/// Test-only: production must open the sole runtime through
/// [`ironclaw_libsql_runtime::LibSqlRuntime::open`] so its target provenance
/// can be validated by the production substrate builder.
pub fn libsql_host_bindings_for_test(
    profile: RebornCompositionProfile,
    owner_id: impl Into<String>,
    db: Arc<libsql::Database>,
    database_path_or_url: impl Into<String>,
    auth_token: Option<ironclaw_secrets::SecretMaterial>,
    secret_master_key: ironclaw_secrets::SecretMaterial,
) -> Result<RebornHostBindings, RebornBuildError> {
    crate::input::libsql_host_bindings_for_test(
        profile,
        owner_id,
        db,
        database_path_or_url,
        auth_token,
        secret_master_key,
    )
}

/// Build libSQL composition around a caller-supplied shared runtime.
///
/// Test-only: this seam lets writer-lane tests hold the exact runtime that
/// production adapters receive without exposing runtime injection in
/// production builds.
pub fn libsql_host_bindings_from_runtime_for_test(
    profile: RebornCompositionProfile,
    owner_id: impl Into<String>,
    runtime: Arc<ironclaw_libsql_runtime::LibSqlRuntime>,
    database_path_or_url: impl Into<String>,
    secret_master_key: ironclaw_secrets::SecretMaterial,
) -> RebornHostBindings {
    crate::input::libsql_host_bindings_from_runtime_for_test(
        profile,
        owner_id,
        runtime,
        database_path_or_url,
        secret_master_key,
    )
}

/// Build libSQL composition that resolves its secret master key at build time.
///
/// Test-only: production resolves the key through its declarative bootstrap
/// path.
pub fn libsql_host_bindings_with_resolved_secret_master_key_for_test(
    profile: RebornCompositionProfile,
    owner_id: impl Into<String>,
    db: Arc<libsql::Database>,
    database_path_or_url: impl Into<String>,
    auth_token: Option<ironclaw_secrets::SecretMaterial>,
) -> Result<RebornHostBindings, RebornBuildError> {
    crate::input::libsql_host_bindings_with_resolved_secret_master_key_for_test(
        profile,
        owner_id,
        db,
        database_path_or_url,
        auth_token,
    )
}
