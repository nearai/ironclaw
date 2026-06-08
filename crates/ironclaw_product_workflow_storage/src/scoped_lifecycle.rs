use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_filesystem::{
    CasExpectation, Entry, FilesystemError, Filter, IndexKey, IndexValue, Page, RecordKind,
    RecordVersion, RootFilesystem, VersionedEntry,
};
use ironclaw_host_api::{TenantId, VirtualPath};
use ironclaw_product_workflow::{
    DeleteScopedLifecycleInstallationRequest, LifecyclePackageRef, ProductWorkflowError,
    ScopedLifecycleActor, ScopedLifecycleInstallation, ScopedLifecycleInstallationId,
    ScopedLifecycleInstallationStore, ScopedLifecycleOwnership,
    UpsertScopedLifecycleInstallationRequest, lifecycle_package_kind_label,
};

const DEFAULT_SCOPED_LIFECYCLE_ROOT: &str = "/engine/product_workflow/scoped_lifecycle";
const SCOPED_LIFECYCLE_RECORD_KIND: &str = "scoped_lifecycle_installation";
const SCOPED_LIFECYCLE_TOMBSTONE_RECORD_KIND: &str = "scoped_lifecycle_tombstone";
const SCOPED_LIFECYCLE_ID_RESERVATION_RECORD_KIND: &str = "scoped_lifecycle_installation_id";
const SCOPED_LIFECYCLE_ID_TOMBSTONE_RECORD_KIND: &str =
    "scoped_lifecycle_installation_id_tombstone";

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
        if loaded.installation.installation_id != *installation_id {
            return Ok(None);
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

enum PackagePathState {
    Absent,
    Tombstone(RecordVersion),
    Occupied,
}

enum InstallationIdState {
    Absent,
    Tombstone(RecordVersion),
    Reserved(ScopedLifecycleInstallationIdReservation, RecordVersion),
}

struct VersionedScopedLifecycleInstallation {
    path: VirtualPath,
    installation: ScopedLifecycleInstallation,
    version: RecordVersion,
}

#[derive(Debug, Clone, PartialEq)]
struct ScopedLifecycleInstallationIdReservation {
    installation_id: ScopedLifecycleInstallationId,
    package_ref: LifecyclePackageRef,
    ownership: ScopedLifecycleOwnership,
}

impl ScopedLifecycleInstallationIdReservation {
    fn new(installation: &ScopedLifecycleInstallation) -> Self {
        Self {
            installation_id: installation.installation_id.clone(),
            package_ref: installation.package_ref.clone(),
            ownership: installation.ownership.clone(),
        }
    }

    fn matches_installation(&self, installation: &ScopedLifecycleInstallation) -> bool {
        self.installation_id == installation.installation_id
            && self.package_ref == installation.package_ref
            && self.ownership == installation.ownership
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

fn default_scoped_lifecycle_root() -> VirtualPath {
    VirtualPath::new(DEFAULT_SCOPED_LIFECYCLE_ROOT).expect("DEFAULT_SCOPED_LIFECYCLE_ROOT is valid") // safety: hard-coded /engine virtual path literal.
}

fn entry_for_scoped_lifecycle_installation(
    installation: &ScopedLifecycleInstallation,
) -> Result<Entry, ProductWorkflowError> {
    entry_for_scoped_lifecycle_record(installation, SCOPED_LIFECYCLE_RECORD_KIND)
}

fn tombstone_entry_for_scoped_lifecycle_installation(
    installation: &ScopedLifecycleInstallation,
) -> Result<Entry, ProductWorkflowError> {
    entry_for_scoped_lifecycle_record(installation, SCOPED_LIFECYCLE_TOMBSTONE_RECORD_KIND)
}

fn entry_for_installation_id_reservation(
    reservation: &ScopedLifecycleInstallationIdReservation,
) -> Result<Entry, ProductWorkflowError> {
    entry_for_installation_id_reservation_record(
        reservation,
        SCOPED_LIFECYCLE_ID_RESERVATION_RECORD_KIND,
    )
}

fn tombstone_entry_for_installation_id_reservation(
    reservation: &ScopedLifecycleInstallationIdReservation,
) -> Result<Entry, ProductWorkflowError> {
    entry_for_installation_id_reservation_record(
        reservation,
        SCOPED_LIFECYCLE_ID_TOMBSTONE_RECORD_KIND,
    )
}

fn entry_for_installation_id_reservation_record(
    reservation: &ScopedLifecycleInstallationIdReservation,
    record_kind: &'static str,
) -> Result<Entry, ProductWorkflowError> {
    let payload = serde_json::json!({
        "installation_id": reservation.installation_id,
        "package_ref": reservation.package_ref,
        "ownership": reservation.ownership,
    });
    let kind = RecordKind::new(record_kind).map_err(|error| {
        scoped_lifecycle_durable_error("construct scoped lifecycle id record kind", error)
    })?;
    let entry = Entry::record(kind, &payload)
        .map_err(|error| scoped_lifecycle_durable_error("serialize installation id entry", error))?
        .with_indexed(
            index_key("tenant_id")?,
            text(reservation.ownership.tenant_id().as_str()),
        )
        .with_indexed(
            index_key("installation_id")?,
            text(reservation.installation_id.as_str()),
        )
        .with_indexed(
            index_key("package_kind")?,
            text(lifecycle_package_kind_label(reservation.package_ref.kind)),
        )
        .with_indexed(
            index_key("package_id")?,
            text(reservation.package_ref.id.as_str()),
        )
        .with_indexed(index_key("ownership")?, text(reservation.ownership.label()));
    Ok(entry)
}

fn entry_for_scoped_lifecycle_record(
    installation: &ScopedLifecycleInstallation,
    record_kind: &'static str,
) -> Result<Entry, ProductWorkflowError> {
    let payload = serde_json::to_value(installation)
        .map_err(|error| scoped_lifecycle_durable_error("serialize installation", error))?;
    let kind = RecordKind::new(record_kind).map_err(|error| {
        scoped_lifecycle_durable_error("construct scoped lifecycle record kind", error)
    })?;
    let entry = Entry::record(kind, &payload)
        .map_err(|error| scoped_lifecycle_durable_error("serialize installation entry", error))?
        .with_indexed(
            index_key("tenant_id")?,
            text(installation.tenant_id().as_str()),
        )
        .with_indexed(
            index_key("installation_id")?,
            text(installation.installation_id.as_str()),
        )
        .with_indexed(
            index_key("package_kind")?,
            text(lifecycle_package_kind_label(installation.package_ref.kind)),
        )
        .with_indexed(
            index_key("package_id")?,
            text(installation.package_ref.id.as_str()),
        )
        .with_indexed(
            index_key("ownership")?,
            text(installation.ownership.label()),
        )
        .with_indexed(
            index_key("enabled")?,
            IndexValue::Bool(installation.enabled),
        )
        .with_indexed(
            index_key("updated_at_ms")?,
            IndexValue::I64(installation.updated_at.timestamp_millis()),
        );
    Ok(entry)
}

fn is_scoped_lifecycle_tombstone(entry: &Entry) -> bool {
    entry
        .kind
        .as_ref()
        .is_some_and(|kind| kind.as_str() == SCOPED_LIFECYCLE_TOMBSTONE_RECORD_KIND)
}

fn is_installation_id_tombstone(entry: &Entry) -> bool {
    entry
        .kind
        .as_ref()
        .is_some_and(|kind| kind.as_str() == SCOPED_LIFECYCLE_ID_TOMBSTONE_RECORD_KIND)
}

fn parse_installation_id_reservation(
    entry: Entry,
) -> Result<ScopedLifecycleInstallationIdReservation, ProductWorkflowError> {
    let payload = entry
        .parse_json::<serde_json::Value>()
        .map_err(|error| scoped_lifecycle_durable_error("deserialize installation id", error))?;
    let installation_id = serde_json::from_value(reservation_field(&payload, "installation_id")?)
        .map_err(|error| {
        scoped_lifecycle_durable_error("deserialize installation id", error)
    })?;
    let package_ref =
        serde_json::from_value(reservation_field(&payload, "package_ref")?).map_err(|error| {
            scoped_lifecycle_durable_error("deserialize installation id package", error)
        })?;
    let ownership =
        serde_json::from_value(reservation_field(&payload, "ownership")?).map_err(|error| {
            scoped_lifecycle_durable_error("deserialize installation id ownership", error)
        })?;
    Ok(ScopedLifecycleInstallationIdReservation {
        installation_id,
        package_ref,
        ownership,
    })
}

fn reservation_field(
    payload: &serde_json::Value,
    field: &'static str,
) -> Result<serde_json::Value, ProductWorkflowError> {
    payload
        .get(field)
        .cloned()
        .ok_or_else(|| scoped_lifecycle_transient(format!("scoped lifecycle missing {field}")))
}

fn parse_scoped_lifecycle_installation(
    entry: ironclaw_filesystem::VersionedEntry,
) -> Result<ScopedLifecycleInstallation, ProductWorkflowError> {
    let installation = entry
        .entry
        .parse_json::<ScopedLifecycleInstallation>()
        .map_err(|error| scoped_lifecycle_durable_error("deserialize installation", error))?;
    installation.validate()?;
    Ok(installation)
}

fn parse_versioned_scoped_lifecycle_installation(
    entry: VersionedEntry,
) -> Result<VersionedScopedLifecycleInstallation, ProductWorkflowError> {
    let path = entry.path.clone();
    let version = entry.version;
    let installation = parse_scoped_lifecycle_installation(entry)?;
    Ok(VersionedScopedLifecycleInstallation {
        path,
        installation,
        version,
    })
}

fn validate_scoped_lifecycle_update_identity(
    existing: &ScopedLifecycleInstallation,
    next: &ScopedLifecycleInstallation,
) -> Result<(), ProductWorkflowError> {
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
    Ok(())
}

fn scoped_lifecycle_tenant_installations_path(
    root: &VirtualPath,
    tenant_id: &TenantId,
) -> Result<VirtualPath, ProductWorkflowError> {
    let path = format!(
        "{}/tenants/{}/installations",
        root.as_str().trim_end_matches('/'),
        hex_component(tenant_id.as_str())
    );
    VirtualPath::new(path)
        .map_err(|error| scoped_lifecycle_durable_error("construct tenant lifecycle path", error))
}

fn scoped_lifecycle_installation_path(
    root: &VirtualPath,
    installation: &ScopedLifecycleInstallation,
) -> Result<VirtualPath, ProductWorkflowError> {
    scoped_lifecycle_installation_path_for_parts(
        root,
        installation.tenant_id(),
        &installation.ownership,
        &installation.package_ref,
    )
}

fn scoped_lifecycle_installation_path_for_reservation(
    root: &VirtualPath,
    reservation: &ScopedLifecycleInstallationIdReservation,
) -> Result<VirtualPath, ProductWorkflowError> {
    scoped_lifecycle_installation_path_for_parts(
        root,
        reservation.ownership.tenant_id(),
        &reservation.ownership,
        &reservation.package_ref,
    )
}

fn scoped_lifecycle_installation_path_for_parts(
    root: &VirtualPath,
    tenant_id: &TenantId,
    ownership: &ScopedLifecycleOwnership,
    package_ref: &LifecyclePackageRef,
) -> Result<VirtualPath, ProductWorkflowError> {
    let path = format!(
        "{}/{}/{}/{}.json",
        scoped_lifecycle_tenant_installations_path(root, tenant_id)?.as_str(),
        ownership_path_component(ownership),
        lifecycle_package_kind_label(package_ref.kind),
        hex_component(package_ref.id.as_str())
    );
    VirtualPath::new(path).map_err(|error| {
        scoped_lifecycle_durable_error("construct scoped lifecycle installation path", error)
    })
}

fn scoped_lifecycle_installation_id_path(
    root: &VirtualPath,
    tenant_id: &TenantId,
    installation_id: &ScopedLifecycleInstallationId,
) -> Result<VirtualPath, ProductWorkflowError> {
    let path = format!(
        "{}/tenants/{}/installation_ids/{}.json",
        root.as_str().trim_end_matches('/'),
        hex_component(tenant_id.as_str()),
        hex_component(installation_id.as_str())
    );
    VirtualPath::new(path)
        .map_err(|error| scoped_lifecycle_durable_error("construct installation id path", error))
}

fn ownership_path_component(ownership: &ScopedLifecycleOwnership) -> String {
    match ownership {
        ScopedLifecycleOwnership::AdminShared { .. } => "admin_shared".to_string(),
        ScopedLifecycleOwnership::UserPrivate { user_id, .. } => {
            format!("user_private/{}", hex_component(user_id.as_str()))
        }
    }
}

fn index_key(value: &'static str) -> Result<IndexKey, ProductWorkflowError> {
    IndexKey::new(value)
        .map_err(|error| scoped_lifecycle_durable_error("construct lifecycle index key", error))
}

fn text(value: &str) -> IndexValue {
    IndexValue::Text(value.to_string())
}

fn scoped_lifecycle_transient(reason: impl Into<String>) -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: reason.into(),
    }
}

fn scoped_lifecycle_invalid_request(reason: &'static str) -> ProductWorkflowError {
    ProductWorkflowError::InvalidBindingRequest {
        reason: reason.to_string(),
    }
}

fn scoped_lifecycle_durable_error(
    operation: &'static str,
    error: impl std::fmt::Display,
) -> ProductWorkflowError {
    let error_type = std::any::type_name_of_val(&error);
    tracing::error!(
        operation,
        error_type,
        "product workflow scoped lifecycle store failed"
    );
    scoped_lifecycle_transient(format!("scoped lifecycle store failed to {operation}"))
}

fn scoped_lifecycle_filesystem_error(
    operation: &'static str,
    error: FilesystemError,
) -> ProductWorkflowError {
    scoped_lifecycle_durable_error(operation, error)
}

fn hex_component(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{Duration, Utc};
    use ironclaw_filesystem::{DirEntry, FileStat, FilesystemOperation, Filter, Page};
    use ironclaw_host_api::UserId;
    use ironclaw_product_workflow::{
        LifecyclePackageKind, LifecyclePackageRef, ScopedLifecycleActor,
    };
    use tokio::sync::Mutex;

    use super::*;

    struct CapturingFilesystem {
        entry: Mutex<Option<VersionedEntry>>,
        entries: Mutex<HashMap<String, VersionedEntry>>,
        put_error: Mutex<Option<FilesystemError>>,
        observed_cas: Mutex<Vec<CasExpectation>>,
        delete_count: Mutex<usize>,
    }

    impl CapturingFilesystem {
        fn new(entry: Option<VersionedEntry>) -> Self {
            Self {
                entry: Mutex::new(entry),
                entries: Mutex::new(HashMap::new()),
                put_error: Mutex::new(None),
                observed_cas: Mutex::new(Vec::new()),
                delete_count: Mutex::new(0),
            }
        }

        fn with_entries(entries: Vec<VersionedEntry>) -> Self {
            Self {
                entry: Mutex::new(None),
                entries: Mutex::new(
                    entries
                        .into_iter()
                        .map(|entry| (entry.path.as_str().to_string(), entry))
                        .collect(),
                ),
                put_error: Mutex::new(None),
                observed_cas: Mutex::new(Vec::new()),
                delete_count: Mutex::new(0),
            }
        }

        fn with_put_error(entry: Option<VersionedEntry>, error: FilesystemError) -> Self {
            Self {
                entry: Mutex::new(entry),
                entries: Mutex::new(HashMap::new()),
                put_error: Mutex::new(Some(error)),
                observed_cas: Mutex::new(Vec::new()),
                delete_count: Mutex::new(0),
            }
        }

        async fn observed_cas(&self) -> Vec<CasExpectation> {
            self.observed_cas.lock().await.clone()
        }

        async fn delete_count(&self) -> usize {
            *self.delete_count.lock().await
        }

        async fn stored_entry(&self, path: &VirtualPath) -> Option<VersionedEntry> {
            self.entries.lock().await.get(path.as_str()).cloned()
        }
    }

    #[async_trait]
    impl RootFilesystem for CapturingFilesystem {
        async fn put(
            &self,
            path: &VirtualPath,
            entry: Entry,
            cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            self.observed_cas.lock().await.push(cas);
            if let Some(error) = self.put_error.lock().await.take() {
                return Err(error);
            }
            let version = match cas {
                CasExpectation::Version(version) => version.next(),
                CasExpectation::Absent | CasExpectation::Any => RecordVersion::from_backend(1),
            };
            self.entries.lock().await.insert(
                path.as_str().to_string(),
                VersionedEntry {
                    path: path.clone(),
                    entry,
                    version,
                },
            );
            Ok(version)
        }

        async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
            if let Some(entry) = self.entries.lock().await.get(path.as_str()).cloned() {
                return Ok(Some(entry));
            }
            Ok(self.entry.lock().await.clone())
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::ListDir,
            })
        }

        async fn query(
            &self,
            _path: &VirtualPath,
            _filter: &Filter,
            _page: Page,
        ) -> Result<Vec<VersionedEntry>, FilesystemError> {
            Ok(self.entry.lock().await.iter().cloned().collect())
        }

        async fn delete(&self, _path: &VirtualPath) -> Result<(), FilesystemError> {
            *self.delete_count.lock().await += 1;
            Ok(())
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::Stat,
            })
        }
    }

    #[tokio::test]
    async fn upsert_uses_absent_cas_when_creating_installation() {
        let filesystem = Arc::new(CapturingFilesystem::new(None));
        let store =
            FilesystemScopedLifecycleInstallationStore::with_root(filesystem.clone(), test_root());
        let admin = admin_actor();
        let installation = ScopedLifecycleInstallation::admin_shared(
            install_id(),
            package("github"),
            admin.clone(),
            Utc::now(),
        )
        .expect("admin shared install");

        store
            .upsert_installation(UpsertScopedLifecycleInstallationRequest {
                actor: admin,
                installation,
            })
            .await
            .expect("upsert creates installation");

        assert_eq!(
            filesystem.observed_cas().await,
            vec![CasExpectation::Absent, CasExpectation::Absent]
        );
    }

    #[tokio::test]
    async fn upsert_uses_version_cas_when_updating_installation() {
        let admin = admin_actor();
        let existing = ScopedLifecycleInstallation::admin_shared(
            install_id(),
            package("github"),
            admin.clone(),
            Utc::now(),
        )
        .expect("admin shared install");
        let version = RecordVersion::from_backend(7);
        let entry = VersionedEntry {
            path: VirtualPath::new("/engine/product_workflow/scoped_lifecycle/test/existing.json")
                .expect("valid path"),
            entry: entry_for_scoped_lifecycle_installation(&existing).expect("serialize entry"),
            version,
        };
        let filesystem = Arc::new(CapturingFilesystem::new(Some(entry)));
        let store =
            FilesystemScopedLifecycleInstallationStore::with_root(filesystem.clone(), test_root());
        let mut update = existing;
        update.enabled = false;
        update.updated_by = admin.clone();
        update.updated_at += Duration::seconds(1);

        store
            .upsert_installation(UpsertScopedLifecycleInstallationRequest {
                actor: admin,
                installation: update,
            })
            .await
            .expect("upsert updates installation");

        assert_eq!(
            filesystem.observed_cas().await,
            vec![CasExpectation::Version(version)]
        );
    }

    #[tokio::test]
    async fn delete_uses_version_cas_without_physical_delete() {
        let admin = admin_actor();
        let existing = ScopedLifecycleInstallation::admin_shared(
            install_id(),
            package("github"),
            admin.clone(),
            Utc::now(),
        )
        .expect("admin shared install");
        let version = RecordVersion::from_backend(7);
        let entry = VersionedEntry {
            path: VirtualPath::new("/engine/product_workflow/scoped_lifecycle/test/existing.json")
                .expect("valid path"),
            entry: entry_for_scoped_lifecycle_installation(&existing).expect("serialize entry"),
            version,
        };
        let filesystem = Arc::new(CapturingFilesystem::new(Some(entry)));
        let store =
            FilesystemScopedLifecycleInstallationStore::with_root(filesystem.clone(), test_root());

        store
            .delete_installation(DeleteScopedLifecycleInstallationRequest {
                actor: admin,
                tenant_id: existing.tenant_id().clone(),
                installation_id: existing.installation_id.clone(),
            })
            .await
            .expect("delete installation");

        assert_eq!(
            filesystem.observed_cas().await,
            vec![
                CasExpectation::Version(version),
                CasExpectation::Version(version)
            ]
        );
        assert_eq!(filesystem.delete_count().await, 0);
    }

    #[tokio::test]
    async fn delete_retries_cleanup_live_reservation_when_package_is_already_tombstoned() {
        let admin = admin_actor();
        let existing = ScopedLifecycleInstallation::admin_shared(
            install_id(),
            package("github"),
            admin.clone(),
            Utc::now(),
        )
        .expect("admin shared install");
        let version = RecordVersion::from_backend(7);
        let root = test_root();
        let reservation = ScopedLifecycleInstallationIdReservation::new(&existing);
        let reservation_path = scoped_lifecycle_installation_id_path(
            &root,
            existing.tenant_id(),
            &existing.installation_id,
        )
        .expect("reservation path");
        let package_path =
            scoped_lifecycle_installation_path(&root, &existing).expect("package path");
        let filesystem = Arc::new(CapturingFilesystem::with_entries(vec![
            VersionedEntry {
                path: reservation_path.clone(),
                entry: entry_for_installation_id_reservation(&reservation)
                    .expect("reservation entry"),
                version,
            },
            VersionedEntry {
                path: package_path,
                entry: tombstone_entry_for_scoped_lifecycle_installation(&existing)
                    .expect("package tombstone"),
                version,
            },
        ]));
        let store = FilesystemScopedLifecycleInstallationStore::with_root(filesystem.clone(), root);

        store
            .delete_installation(DeleteScopedLifecycleInstallationRequest {
                actor: admin,
                tenant_id: existing.tenant_id().clone(),
                installation_id: existing.installation_id.clone(),
            })
            .await
            .expect("delete cleans live reservation retry");

        assert_eq!(
            filesystem.observed_cas().await,
            vec![CasExpectation::Version(version)]
        );
        let stored = filesystem
            .stored_entry(&reservation_path)
            .await
            .expect("reservation tombstone stored");
        assert!(is_installation_id_tombstone(&stored.entry));
        assert_eq!(filesystem.delete_count().await, 0);
    }

    #[tokio::test]
    async fn delete_does_not_remove_when_loaded_version_changed() {
        let admin = admin_actor();
        let existing = ScopedLifecycleInstallation::admin_shared(
            install_id(),
            package("github"),
            admin.clone(),
            Utc::now(),
        )
        .expect("admin shared install");
        let version = RecordVersion::from_backend(7);
        let path = VirtualPath::new("/engine/product_workflow/scoped_lifecycle/test/existing.json")
            .expect("valid path");
        let entry = VersionedEntry {
            path: path.clone(),
            entry: entry_for_scoped_lifecycle_installation(&existing).expect("serialize entry"),
            version,
        };
        let filesystem = Arc::new(CapturingFilesystem::with_put_error(
            Some(entry),
            FilesystemError::VersionMismatch {
                path,
                expected: Some(version),
                found: Some(version.next()),
            },
        ));
        let store =
            FilesystemScopedLifecycleInstallationStore::with_root(filesystem.clone(), test_root());

        let error = store
            .delete_installation(DeleteScopedLifecycleInstallationRequest {
                actor: admin,
                tenant_id: existing.tenant_id().clone(),
                installation_id: existing.installation_id.clone(),
            })
            .await
            .expect_err("stale delete must fail before physical delete");

        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
        assert_eq!(
            filesystem.observed_cas().await,
            vec![CasExpectation::Version(version)]
        );
        assert_eq!(filesystem.delete_count().await, 0);
    }

    fn test_root() -> VirtualPath {
        VirtualPath::new("/engine/product_workflow/scoped_lifecycle/test").expect("valid root")
    }

    fn admin_actor() -> ScopedLifecycleActor {
        ScopedLifecycleActor::admin(
            TenantId::new("tenant-alpha").expect("valid tenant"),
            UserId::new("admin-alpha").expect("valid user"),
        )
    }

    fn install_id() -> ScopedLifecycleInstallationId {
        ScopedLifecycleInstallationId::new("shared-github").expect("valid install id")
    }

    fn package(id: &str) -> LifecyclePackageRef {
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, id).expect("valid package")
    }
}
