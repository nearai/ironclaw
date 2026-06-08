use std::sync::Arc;

use async_trait::async_trait;
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
    DeleteScopedLifecycleInstallationRequest, ProductWorkflowError, ScopedLifecycleInstallation,
    ScopedLifecycleInstallationId, ScopedLifecycleInstallationStore,
    UpsertScopedLifecycleInstallationRequest, lifecycle_package_kind_label,
};

const DEFAULT_SCOPED_LIFECYCLE_ROOT: &str = "/engine/product_workflow/scoped_lifecycle";
const SCOPED_LIFECYCLE_RECORD_KIND: &str = "scoped_lifecycle_installation";

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
        let path = scoped_lifecycle_installation_path(
            &self.root,
            installation.tenant_id(),
            &installation.installation_id,
        )?;
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
                self.ensure_unique_package_installation(&installation)
                    .await?;
            }
        }
        let cas = existing
            .as_ref()
            .map_or(CasExpectation::Absent, |existing| {
                CasExpectation::Version(existing.version)
            });
        self.filesystem
            .put(
                &path,
                entry_for_scoped_lifecycle_installation(&installation)?,
                cas,
            )
            .await
            .map_err(|error| match error {
                FilesystemError::VersionMismatch { .. } => {
                    scoped_lifecycle_transient("scoped lifecycle installation write conflict")
                }
                error => scoped_lifecycle_filesystem_error("upsert installation", error),
            })?;
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
        let Some(existing) = self
            .get_installation(&request.tenant_id, &request.installation_id)
            .await?
        else {
            return Ok(());
        };
        if !existing.can_be_mutated_by(&request.actor) {
            return Err(ProductWorkflowError::BindingAccessDenied);
        }
        let path = scoped_lifecycle_installation_path(
            &self.root,
            &request.tenant_id,
            &request.installation_id,
        )?;
        match self.filesystem.delete(&path).await {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(scoped_lifecycle_filesystem_error(
                "delete installation",
                error,
            )),
        }
    }

    async fn list_installations(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<ScopedLifecycleInstallation>, ProductWorkflowError> {
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
                let installation = parse_scoped_lifecycle_installation(entry)?;
                if installation.tenant_id() == tenant_id {
                    installations.push(installation);
                }
            }
            if entry_count < Page::MAX_LIMIT as usize {
                break;
            }
            offset = offset
                .checked_add(Page::MAX_LIMIT as u64)
                .ok_or_else(|| scoped_lifecycle_transient("scoped lifecycle list page overflow"))?;
        }
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
    async fn ensure_unique_package_installation(
        &self,
        installation: &ScopedLifecycleInstallation,
    ) -> Result<(), ProductWorkflowError> {
        let duplicate = self
            .list_installations(installation.tenant_id())
            .await?
            .into_iter()
            .any(|existing| {
                existing.installation_id != installation.installation_id
                    && existing.ownership == installation.ownership
                    && existing.package_ref == installation.package_ref
            });
        if duplicate {
            return Err(scoped_lifecycle_invalid_request(
                "scoped lifecycle installation package already exists for ownership",
            ));
        }
        Ok(())
    }

    async fn load_installation(
        &self,
        tenant_id: &TenantId,
        installation_id: &ScopedLifecycleInstallationId,
    ) -> Result<Option<VersionedScopedLifecycleInstallation>, ProductWorkflowError> {
        let path = scoped_lifecycle_installation_path(&self.root, tenant_id, installation_id)?;
        let Some(entry) = self
            .filesystem
            .get(&path)
            .await
            .map_err(|error| scoped_lifecycle_filesystem_error("load installation", error))?
        else {
            return Ok(None);
        };
        let loaded = parse_versioned_scoped_lifecycle_installation(entry)?;
        if loaded.installation.tenant_id() != tenant_id {
            return Err(scoped_lifecycle_transient(
                "scoped lifecycle installation tenant mismatch",
            ));
        }
        Ok(Some(loaded))
    }
}

struct VersionedScopedLifecycleInstallation {
    installation: ScopedLifecycleInstallation,
    version: RecordVersion,
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
    let payload = serde_json::to_value(installation)
        .map_err(|error| scoped_lifecycle_durable_error("serialize installation", error))?;
    let kind = RecordKind::new(SCOPED_LIFECYCLE_RECORD_KIND).map_err(|error| {
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
    let version = entry.version;
    let installation = parse_scoped_lifecycle_installation(entry)?;
    Ok(VersionedScopedLifecycleInstallation {
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
    tenant_id: &TenantId,
    installation_id: &ScopedLifecycleInstallationId,
) -> Result<VirtualPath, ProductWorkflowError> {
    let path = format!(
        "{}/{}.json",
        scoped_lifecycle_tenant_installations_path(root, tenant_id)?.as_str(),
        hex_component(installation_id.as_str())
    );
    VirtualPath::new(path).map_err(|error| {
        scoped_lifecycle_durable_error("construct scoped lifecycle installation path", error)
    })
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
        observed_cas: Mutex<Vec<CasExpectation>>,
    }

    impl CapturingFilesystem {
        fn new(entry: Option<VersionedEntry>) -> Self {
            Self {
                entry: Mutex::new(entry),
                observed_cas: Mutex::new(Vec::new()),
            }
        }

        async fn observed_cas(&self) -> Vec<CasExpectation> {
            self.observed_cas.lock().await.clone()
        }
    }

    #[async_trait]
    impl RootFilesystem for CapturingFilesystem {
        async fn put(
            &self,
            _path: &VirtualPath,
            _entry: Entry,
            cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            self.observed_cas.lock().await.push(cas);
            Ok(match cas {
                CasExpectation::Version(version) => version.next(),
                CasExpectation::Absent | CasExpectation::Any => RecordVersion::from_backend(1),
            })
        }

        async fn get(
            &self,
            _path: &VirtualPath,
        ) -> Result<Option<VersionedEntry>, FilesystemError> {
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
            Ok(Vec::new())
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
            vec![CasExpectation::Absent]
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
