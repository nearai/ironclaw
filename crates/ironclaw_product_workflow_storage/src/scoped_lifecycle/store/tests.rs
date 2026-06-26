use std::collections::HashMap;

use chrono::{Duration, Utc};
use ironclaw_filesystem::{
    DirEntry, Entry, FileStat, FilesystemOperation, Filter, Page, VersionedEntry,
};
use ironclaw_host_api::UserId;
use ironclaw_product_workflow::{LifecyclePackageKind, LifecyclePackageRef, ScopedLifecycleActor};
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
async fn upsert_rejects_decreased_updated_at() {
    let admin = admin_actor();
    let now = Utc::now();
    let mut existing = ScopedLifecycleInstallation::admin_shared(
        install_id(),
        package("github"),
        admin.clone(),
        now,
    )
    .expect("admin shared install");
    existing.updated_at = now + Duration::seconds(5);
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
    update.updated_by = admin.clone();
    update.updated_at = now + Duration::seconds(1);

    let error = store
        .upsert_installation(UpsertScopedLifecycleInstallationRequest {
            actor: admin,
            installation: update,
        })
        .await
        .expect_err("stale update must fail");

    assert!(matches!(
        error,
        ProductWorkflowError::InvalidBindingRequest { .. }
    ));
    assert!(filesystem.observed_cas().await.is_empty());
}

#[test]
fn update_identity_rejects_changed_installation_id() {
    let admin = admin_actor();
    let existing = ScopedLifecycleInstallation::admin_shared(
        install_id(),
        package("github"),
        admin,
        Utc::now(),
    )
    .expect("admin shared install");
    let mut next = existing.clone();
    next.installation_id =
        ScopedLifecycleInstallationId::new("shared-github-renamed").expect("valid install id");

    let error = validate_scoped_lifecycle_update_identity(&existing, &next)
        .expect_err("installation id is immutable");

    assert!(matches!(
        error,
        ProductWorkflowError::InvalidBindingRequest { .. }
    ));
}

#[tokio::test]
async fn get_installation_rejects_loaded_installation_id_mismatch() {
    let admin = admin_actor();
    let existing = ScopedLifecycleInstallation::admin_shared(
        install_id(),
        package("github"),
        admin,
        Utc::now(),
    )
    .expect("admin shared install");
    let mut corrupted = existing.clone();
    corrupted.installation_id =
        ScopedLifecycleInstallationId::new("different-installation").expect("valid install id");
    let (filesystem, root) = filesystem_with_reserved_package(&existing, &corrupted);
    let store = FilesystemScopedLifecycleInstallationStore::with_root(filesystem, root);

    let error = store
        .get_installation(existing.tenant_id(), &existing.installation_id)
        .await
        .expect_err("loaded id mismatch must fail");

    assert!(matches!(error, ProductWorkflowError::Transient { .. }));
}

#[tokio::test]
async fn get_installation_rejects_loaded_tenant_mismatch() {
    let admin = admin_actor();
    let existing = ScopedLifecycleInstallation::admin_shared(
        install_id(),
        package("github"),
        admin,
        Utc::now(),
    )
    .expect("admin shared install");
    let tenant_beta = TenantId::new("tenant-beta").expect("valid tenant");
    let admin_beta =
        ScopedLifecycleActor::admin(tenant_beta, UserId::new("admin-beta").expect("valid user"));
    let corrupted = ScopedLifecycleInstallation::admin_shared(
        existing.installation_id.clone(),
        existing.package_ref.clone(),
        admin_beta,
        existing.created_at,
    )
    .expect("admin shared install");
    let (filesystem, root) = filesystem_with_reserved_package(&existing, &corrupted);
    let store = FilesystemScopedLifecycleInstallationStore::with_root(filesystem, root);

    let error = store
        .get_installation(existing.tenant_id(), &existing.installation_id)
        .await
        .expect_err("loaded tenant mismatch must fail");

    assert!(matches!(error, ProductWorkflowError::Transient { .. }));
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
    let package_path = scoped_lifecycle_installation_path(&root, &existing).expect("package path");
    let filesystem = Arc::new(CapturingFilesystem::with_entries(vec![
        VersionedEntry {
            path: reservation_path.clone(),
            entry: entry_for_installation_id_reservation(&reservation).expect("reservation entry"),
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

fn filesystem_with_reserved_package(
    reservation_installation: &ScopedLifecycleInstallation,
    stored_installation: &ScopedLifecycleInstallation,
) -> (Arc<CapturingFilesystem>, VirtualPath) {
    let version = RecordVersion::from_backend(7);
    let root = test_root();
    let reservation = ScopedLifecycleInstallationIdReservation::new(reservation_installation);
    let reservation_path = scoped_lifecycle_installation_id_path(
        &root,
        reservation_installation.tenant_id(),
        &reservation_installation.installation_id,
    )
    .expect("reservation path");
    let package_path =
        scoped_lifecycle_installation_path(&root, reservation_installation).expect("package path");
    let filesystem = Arc::new(CapturingFilesystem::with_entries(vec![
        VersionedEntry {
            path: reservation_path,
            entry: entry_for_installation_id_reservation(&reservation).expect("reservation entry"),
            version,
        },
        VersionedEntry {
            path: package_path,
            entry: entry_for_scoped_lifecycle_installation(stored_installation)
                .expect("package entry"),
            version,
        },
    ]));
    (filesystem, root)
}

fn package(id: &str) -> LifecyclePackageRef {
    LifecyclePackageRef::new(LifecyclePackageKind::Extension, id).expect("valid package")
}
