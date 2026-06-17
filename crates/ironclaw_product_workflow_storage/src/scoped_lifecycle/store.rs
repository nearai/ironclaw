use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_filesystem::{
    CasExpectation, FilesystemError, Filter, Page, RecordVersion, RootFilesystem,
};
use ironclaw_host_api::{TenantId, VirtualPath};
use ironclaw_product_workflow::{
    DeleteScopedLifecycleInstallationRequest, ProductWorkflowError, ScopedLifecycleActor,
    ScopedLifecycleInstallation, ScopedLifecycleInstallationId, ScopedLifecycleInstallationStore,
    UpsertScopedLifecycleInstallationRequest,
};

use super::{
    InstallationIdState, PackagePathState, ScopedLifecycleInstallationIdReservation,
    VersionedScopedLifecycleInstallation,
    entries::{
        entry_for_installation_id_reservation, entry_for_scoped_lifecycle_installation,
        is_installation_id_tombstone, is_scoped_lifecycle_tombstone,
        parse_installation_id_reservation, parse_versioned_scoped_lifecycle_installation,
        tombstone_entry_for_installation_id_reservation,
        tombstone_entry_for_scoped_lifecycle_installation,
    },
    paths::{
        default_scoped_lifecycle_root, scoped_lifecycle_installation_id_path,
        scoped_lifecycle_installation_path, scoped_lifecycle_installation_path_for_reservation,
        scoped_lifecycle_tenant_installations_path,
    },
    scoped_lifecycle_filesystem_error, scoped_lifecycle_invalid_request,
    scoped_lifecycle_transient,
};

pub struct FilesystemScopedLifecycleInstallationStore {
    filesystem: Arc<dyn RootFilesystem>,
    root: VirtualPath,
}

impl FilesystemScopedLifecycleInstallationStore {
    pub fn new(filesystem: Arc<dyn RootFilesystem>) -> Self {
        Self {
            filesystem,
            root: default_scoped_lifecycle_root(),
        }
    }

    pub fn with_root(filesystem: Arc<dyn RootFilesystem>, root: VirtualPath) -> Self {
        Self { filesystem, root }
    }
}

#[async_trait]
impl ScopedLifecycleInstallationStore for FilesystemScopedLifecycleInstallationStore {
    async fn upsert_installation(
        &self,
        request: UpsertScopedLifecycleInstallationRequest,
    ) -> Result<(), ProductWorkflowError> {
        let installation = request.installation;
        installation.validate()?;
        if !installation.can_be_mutated_by(&request.actor) {
            return Err(ProductWorkflowError::BindingAccessDenied);
        }
        if installation.updated_by != request.actor {
            return Err(scoped_lifecycle_invalid_request(
                "updated_by must match scoped lifecycle upsert actor",
            ));
        }
        let existing = self
            .load_installation(installation.tenant_id(), &installation.installation_id)
            .await?;
        if existing
            .as_ref()
            .is_some_and(|existing| !existing.installation.can_be_mutated_by(&request.actor))
        {
            return Err(ProductWorkflowError::BindingAccessDenied);
        }
        match existing.as_ref() {
            Some(existing) => {
                validate_scoped_lifecycle_update_identity(&existing.installation, &installation)?
            }
            None => {
                if installation.created_by != request.actor {
                    return Err(scoped_lifecycle_invalid_request(
                        "created_by must match scoped lifecycle create actor",
                    ));
                }
            }
        }
        let path = existing.as_ref().map_or_else(
            || scoped_lifecycle_installation_path(&self.root, &installation),
            |existing| Ok(existing.path.clone()),
        )?;
        let (cas, reserved_id_version) = match existing.as_ref() {
            Some(existing) => (CasExpectation::Version(existing.version), None),
            None => {
                let cas = match self.package_path_state(&path).await? {
                    PackagePathState::Absent => CasExpectation::Absent,
                    PackagePathState::Tombstone(version) => CasExpectation::Version(version),
                    PackagePathState::Occupied => {
                        return Err(scoped_lifecycle_invalid_request(
                            "scoped lifecycle installation package already exists for ownership",
                        ));
                    }
                };
                let reservation_path = scoped_lifecycle_installation_id_path(
                    &self.root,
                    installation.tenant_id(),
                    &installation.installation_id,
                )?;
                let reserved_id_version = self
                    .reserve_installation_id(&reservation_path, &installation)
                    .await?;
                (cas, reserved_id_version)
            }
        };
        let reservation_path = scoped_lifecycle_installation_id_path(
            &self.root,
            installation.tenant_id(),
            &installation.installation_id,
        )?;
        let write_result = self
            .filesystem
            .put(
                &path,
                entry_for_scoped_lifecycle_installation(&installation)?,
                cas,
            )
            .await;
        if let Err(error) = write_result {
            if let Some(version) = reserved_id_version {
                self.tombstone_installation_id_reservation_best_effort(
                    &reservation_path,
                    &installation,
                    version,
                )
                .await;
            }
            if matches!(error, FilesystemError::VersionMismatch { .. }) && existing.is_none() {
                return Err(match self.package_path_state(&path).await? {
                    PackagePathState::Occupied => scoped_lifecycle_invalid_request(
                        "scoped lifecycle installation package already exists for ownership",
                    ),
                    PackagePathState::Absent | PackagePathState::Tombstone(_) => {
                        scoped_lifecycle_transient("scoped lifecycle installation write conflict")
                    }
                });
            }
            return Err(match error {
                FilesystemError::VersionMismatch { .. } => {
                    scoped_lifecycle_transient("scoped lifecycle installation write conflict")
                }
                error => scoped_lifecycle_filesystem_error("upsert installation", error),
            });
        }
        Ok(())
    }

    async fn get_installation(
        &self,
        tenant_id: &TenantId,
        installation_id: &ScopedLifecycleInstallationId,
    ) -> Result<Option<ScopedLifecycleInstallation>, ProductWorkflowError> {
        Ok(self
            .load_installation(tenant_id, installation_id)
            .await?
            .map(|loaded| loaded.installation))
    }

    async fn delete_installation(
        &self,
        request: DeleteScopedLifecycleInstallationRequest,
    ) -> Result<(), ProductWorkflowError> {
        let reservation_path = scoped_lifecycle_installation_id_path(
            &self.root,
            &request.tenant_id,
            &request.installation_id,
        )?;
        let Some(existing) = self
            .load_installation(&request.tenant_id, &request.installation_id)
            .await?
        else {
            self.tombstone_installation_id_reservation_for_actor(&reservation_path, &request.actor)
                .await?;
            return Ok(());
        };
        if !existing.installation.can_be_mutated_by(&request.actor) {
            return Err(ProductWorkflowError::BindingAccessDenied);
        }
        let mut tombstone = existing.installation.clone();
        tombstone.enabled = false;
        tombstone.updated_by = request.actor;
        tombstone.updated_at = Utc::now();
        self.filesystem
            .put(
                &existing.path,
                tombstone_entry_for_scoped_lifecycle_installation(&tombstone)?,
                CasExpectation::Version(existing.version),
            )
            .await
            .map_err(|error| match error {
                FilesystemError::VersionMismatch { .. } => {
                    scoped_lifecycle_transient("scoped lifecycle installation delete conflict")
                }
                error => scoped_lifecycle_filesystem_error("mark installation deleted", error),
            })?;
        self.tombstone_installation_id_reservation_if_current(
            &reservation_path,
            &existing.installation,
        )
        .await?;
        Ok(())
    }

    async fn list_installations(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<ScopedLifecycleInstallation>, ProductWorkflowError> {
        let mut installations: Vec<_> = self
            .list_versioned_installations(tenant_id)
            .await?
            .into_iter()
            .map(|versioned| versioned.installation)
            .collect();
        installations.sort_by(|left, right| {
            left.installation_id
                .cmp(&right.installation_id)
                .then_with(|| {
                    left.package_ref
                        .id
                        .as_str()
                        .cmp(right.package_ref.id.as_str())
                })
        });
        Ok(installations)
    }
}

impl FilesystemScopedLifecycleInstallationStore {
    async fn reserve_installation_id(
        &self,
        path: &VirtualPath,
        installation: &ScopedLifecycleInstallation,
    ) -> Result<Option<RecordVersion>, ProductWorkflowError> {
        let reservation = ScopedLifecycleInstallationIdReservation::new(installation);
        let cas = match self.installation_id_state(path).await? {
            InstallationIdState::Absent => CasExpectation::Absent,
            InstallationIdState::Tombstone(version) => CasExpectation::Version(version),
            InstallationIdState::Reserved(existing, _) => {
                if existing.matches_installation(installation) {
                    return Ok(None);
                }
                return Err(scoped_lifecycle_invalid_request(
                    "scoped lifecycle installation id already exists",
                ));
            }
        };
        self.filesystem
            .put(
                path,
                entry_for_installation_id_reservation(&reservation)?,
                cas,
            )
            .await
            .map(Some)
            .map_err(|error| match error {
                FilesystemError::VersionMismatch { .. } => scoped_lifecycle_invalid_request(
                    "scoped lifecycle installation id already exists",
                ),
                error => scoped_lifecycle_filesystem_error("reserve installation id", error),
            })
    }

    async fn tombstone_installation_id_reservation_best_effort(
        &self,
        path: &VirtualPath,
        installation: &ScopedLifecycleInstallation,
        version: RecordVersion,
    ) {
        let reservation = ScopedLifecycleInstallationIdReservation::new(installation);
        let Ok(entry) = tombstone_entry_for_installation_id_reservation(&reservation) else {
            return;
        };
        let _ = self
            .filesystem
            .put(path, entry, CasExpectation::Version(version))
            .await;
    }

    async fn tombstone_installation_id_reservation_if_current(
        &self,
        path: &VirtualPath,
        installation: &ScopedLifecycleInstallation,
    ) -> Result<(), ProductWorkflowError> {
        let InstallationIdState::Reserved(reservation, version) =
            self.installation_id_state(path).await?
        else {
            return Ok(());
        };
        if !reservation.matches_installation(installation) {
            return Ok(());
        }
        self.tombstone_installation_id_reservation(path, &reservation, version)
            .await?;
        Ok(())
    }

    async fn tombstone_installation_id_reservation_for_actor(
        &self,
        path: &VirtualPath,
        actor: &ScopedLifecycleActor,
    ) -> Result<(), ProductWorkflowError> {
        let InstallationIdState::Reserved(reservation, version) =
            self.installation_id_state(path).await?
        else {
            return Ok(());
        };
        if !reservation.ownership.can_be_mutated_by(actor) {
            return Err(ProductWorkflowError::BindingAccessDenied);
        }
        self.tombstone_installation_id_reservation(path, &reservation, version)
            .await?;
        Ok(())
    }

    async fn tombstone_installation_id_reservation(
        &self,
        path: &VirtualPath,
        reservation: &ScopedLifecycleInstallationIdReservation,
        version: RecordVersion,
    ) -> Result<(), ProductWorkflowError> {
        self.filesystem
            .put(
                path,
                tombstone_entry_for_installation_id_reservation(reservation)?,
                CasExpectation::Version(version),
            )
            .await
            .map_err(|error| match error {
                FilesystemError::VersionMismatch { .. } => {
                    scoped_lifecycle_transient("scoped lifecycle installation id delete conflict")
                }
                error => scoped_lifecycle_filesystem_error("delete installation id", error),
            })?;
        Ok(())
    }

    async fn installation_id_state(
        &self,
        path: &VirtualPath,
    ) -> Result<InstallationIdState, ProductWorkflowError> {
        let Some(entry) =
            self.filesystem.get(path).await.map_err(|error| {
                scoped_lifecycle_filesystem_error("load installation id", error)
            })?
        else {
            return Ok(InstallationIdState::Absent);
        };
        if is_installation_id_tombstone(&entry.entry) {
            return Ok(InstallationIdState::Tombstone(entry.version));
        }
        let reservation = parse_installation_id_reservation(entry.entry)?;
        Ok(InstallationIdState::Reserved(reservation, entry.version))
    }

    async fn package_path_state(
        &self,
        path: &VirtualPath,
    ) -> Result<PackagePathState, ProductWorkflowError> {
        let Some(entry) = self
            .filesystem
            .get(path)
            .await
            .map_err(|error| scoped_lifecycle_filesystem_error("load package path", error))?
        else {
            return Ok(PackagePathState::Absent);
        };
        if is_scoped_lifecycle_tombstone(&entry.entry) {
            return Ok(PackagePathState::Tombstone(entry.version));
        }
        Ok(PackagePathState::Occupied)
    }

    async fn load_installation(
        &self,
        tenant_id: &TenantId,
        installation_id: &ScopedLifecycleInstallationId,
    ) -> Result<Option<VersionedScopedLifecycleInstallation>, ProductWorkflowError> {
        let reservation_path =
            scoped_lifecycle_installation_id_path(&self.root, tenant_id, installation_id)?;
        let reservation = match self.installation_id_state(&reservation_path).await? {
            InstallationIdState::Absent | InstallationIdState::Tombstone(_) => return Ok(None),
            InstallationIdState::Reserved(reservation, _) => reservation,
        };
        let package_path =
            scoped_lifecycle_installation_path_for_reservation(&self.root, &reservation)?;
        let Some(package_entry) = self
            .filesystem
            .get(&package_path)
            .await
            .map_err(|error| scoped_lifecycle_filesystem_error("load installation", error))?
        else {
            return Ok(None);
        };
        if is_scoped_lifecycle_tombstone(&package_entry.entry) {
            return Ok(None);
        }
        let loaded = parse_versioned_scoped_lifecycle_installation(package_entry)?;
        if loaded.installation.tenant_id() != tenant_id {
            return Err(scoped_lifecycle_transient(
                "scoped lifecycle installation tenant mismatch",
            ));
        }
        if loaded.installation.installation_id != *installation_id {
            return Err(scoped_lifecycle_transient(
                "scoped lifecycle installation id mismatch",
            ));
        }
        Ok(Some(loaded))
    }

    async fn list_versioned_installations(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<VersionedScopedLifecycleInstallation>, ProductWorkflowError> {
        let path = scoped_lifecycle_tenant_installations_path(&self.root, tenant_id)?;
        let mut installations = Vec::new();
        let mut offset = 0_u64;
        loop {
            let entries = self
                .filesystem
                .query(&path, &Filter::All, Page::new(offset, Page::MAX_LIMIT))
                .await
                .map_err(|error| scoped_lifecycle_filesystem_error("list installations", error))?;
            let entry_count = entries.len();
            for entry in entries {
                if is_scoped_lifecycle_tombstone(&entry.entry) {
                    continue;
                }
                let loaded = parse_versioned_scoped_lifecycle_installation(entry)?;
                if loaded.installation.tenant_id() == tenant_id {
                    installations.push(loaded);
                }
            }
            if entry_count < Page::MAX_LIMIT as usize {
                break;
            }
            offset = offset
                .checked_add(Page::MAX_LIMIT as u64)
                .ok_or_else(|| scoped_lifecycle_transient("scoped lifecycle list page overflow"))?;
        }
        Ok(installations)
    }
}

#[cfg(feature = "libsql")]
pub struct RebornLibSqlScopedLifecycleInstallationStore {
    inner: FilesystemScopedLifecycleInstallationStore,
}

#[cfg(feature = "libsql")]
impl RebornLibSqlScopedLifecycleInstallationStore {
    pub fn new(filesystem: Arc<LibSqlRootFilesystem>) -> Self {
        Self {
            inner: FilesystemScopedLifecycleInstallationStore::new(filesystem),
        }
    }

    pub fn with_root(filesystem: Arc<LibSqlRootFilesystem>, root: VirtualPath) -> Self {
        Self {
            inner: FilesystemScopedLifecycleInstallationStore::with_root(filesystem, root),
        }
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl ScopedLifecycleInstallationStore for RebornLibSqlScopedLifecycleInstallationStore {
    async fn upsert_installation(
        &self,
        request: UpsertScopedLifecycleInstallationRequest,
    ) -> Result<(), ProductWorkflowError> {
        self.inner.upsert_installation(request).await
    }

    async fn get_installation(
        &self,
        tenant_id: &TenantId,
        installation_id: &ScopedLifecycleInstallationId,
    ) -> Result<Option<ScopedLifecycleInstallation>, ProductWorkflowError> {
        self.inner
            .get_installation(tenant_id, installation_id)
            .await
    }

    async fn delete_installation(
        &self,
        request: DeleteScopedLifecycleInstallationRequest,
    ) -> Result<(), ProductWorkflowError> {
        self.inner.delete_installation(request).await
    }

    async fn list_installations(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<ScopedLifecycleInstallation>, ProductWorkflowError> {
        self.inner.list_installations(tenant_id).await
    }
}

#[cfg(feature = "postgres")]
pub struct RebornPostgresScopedLifecycleInstallationStore {
    inner: FilesystemScopedLifecycleInstallationStore,
}

#[cfg(feature = "postgres")]
impl RebornPostgresScopedLifecycleInstallationStore {
    pub fn new(filesystem: Arc<PostgresRootFilesystem>) -> Self {
        Self {
            inner: FilesystemScopedLifecycleInstallationStore::new(filesystem),
        }
    }

    pub fn with_root(filesystem: Arc<PostgresRootFilesystem>, root: VirtualPath) -> Self {
        Self {
            inner: FilesystemScopedLifecycleInstallationStore::with_root(filesystem, root),
        }
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl ScopedLifecycleInstallationStore for RebornPostgresScopedLifecycleInstallationStore {
    async fn upsert_installation(
        &self,
        request: UpsertScopedLifecycleInstallationRequest,
    ) -> Result<(), ProductWorkflowError> {
        self.inner.upsert_installation(request).await
    }

    async fn get_installation(
        &self,
        tenant_id: &TenantId,
        installation_id: &ScopedLifecycleInstallationId,
    ) -> Result<Option<ScopedLifecycleInstallation>, ProductWorkflowError> {
        self.inner
            .get_installation(tenant_id, installation_id)
            .await
    }

    async fn delete_installation(
        &self,
        request: DeleteScopedLifecycleInstallationRequest,
    ) -> Result<(), ProductWorkflowError> {
        self.inner.delete_installation(request).await
    }

    async fn list_installations(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<ScopedLifecycleInstallation>, ProductWorkflowError> {
        self.inner.list_installations(tenant_id).await
    }
}

fn validate_scoped_lifecycle_update_identity(
    existing: &ScopedLifecycleInstallation,
    next: &ScopedLifecycleInstallation,
) -> Result<(), ProductWorkflowError> {
    if existing.installation_id != next.installation_id {
        return Err(scoped_lifecycle_invalid_request(
            "installation_id cannot change for scoped lifecycle installation update",
        ));
    }
    if existing.package_ref != next.package_ref {
        return Err(scoped_lifecycle_invalid_request(
            "package_ref cannot change for scoped lifecycle installation update",
        ));
    }
    if existing.ownership != next.ownership {
        return Err(scoped_lifecycle_invalid_request(
            "ownership cannot change for scoped lifecycle installation update",
        ));
    }
    if existing.created_by != next.created_by {
        return Err(scoped_lifecycle_invalid_request(
            "created_by cannot change for scoped lifecycle installation update",
        ));
    }
    if existing.created_at != next.created_at {
        return Err(scoped_lifecycle_invalid_request(
            "created_at cannot change for scoped lifecycle installation update",
        ));
    }
    if next.updated_at < existing.updated_at {
        return Err(scoped_lifecycle_invalid_request(
            "updated_at cannot be decreased for scoped lifecycle installation update",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
