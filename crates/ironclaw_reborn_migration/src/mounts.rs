//! Canonical per-domain mount views.
//!
//! Each Reborn domain service resolves records through a `ScopedFilesystem`
//! whose resolver maps a [`ResourceScope`] to a [`MountView`] — alias → a
//! tenant/user-scoped virtual path. These builders mirror the production
//! layout (`ironclaw_reborn_composition::local_dev_mounts` for memory; the
//! tenant/user `/threads` + `/secrets` shape the runtime resolves through).
//!
//! FOLLOW-UP: the production mount resolver lives (private) in
//! `ironclaw_reborn_composition`. Until a shared `pub` accessor exists, this
//! module reproduces that layout; it MUST be reconciled with composition when
//! the migration step is wired into `ironclaw-reborn` startup so the runtime
//! reads back exactly what was migrated. The acceptance test verifies
//! round-trip through these same services, which pins conversion correctness
//! independently of that reconciliation.

use ironclaw_host_api::{
    HostApiError, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, VirtualPath,
};

fn grant(alias: &str, target: String) -> Result<MountGrant, HostApiError> {
    Ok(MountGrant::new(
        MountAlias::new(alias)?,
        VirtualPath::new(target)?,
        MountPermissions::read_write_list_delete(),
    ))
}

/// `/threads` → `/tenants/<t>/users/<u>/threads`. Sub-scope (agent, project,
/// mission) is path-encoded by `FilesystemSessionThreadService` inside the alias.
pub(crate) fn threads_mount_view(scope: &ResourceScope) -> Result<MountView, HostApiError> {
    MountView::new(vec![grant(
        "/threads",
        format!(
            "/tenants/{}/users/{}/threads",
            scope.tenant_id.as_str(),
            scope.user_id.as_str()
        ),
    )?])
}

/// `/secrets` → `/tenants/<t>/users/<u>/secrets`. `FilesystemSecretStore`
/// path-encodes agent/project inside the alias.
pub(crate) fn secrets_mount_view(scope: &ResourceScope) -> Result<MountView, HostApiError> {
    MountView::new(vec![grant(
        "/secrets",
        format!(
            "/tenants/{}/users/{}/secrets",
            scope.tenant_id.as_str(),
            scope.user_id.as_str()
        ),
    )?])
}

/// Identity records live under the store's fixed `/tenant-shared/reborn-identity`
/// root (partitioned by tenant inside the record path), so the mount exposes the
/// `/tenant-shared` alias — matching the identity crate's own store wiring.
pub(crate) fn identity_mount_view(_scope: &ResourceScope) -> Result<MountView, HostApiError> {
    MountView::new(vec![grant("/tenant-shared", "/tenant-shared".to_string())?])
}
