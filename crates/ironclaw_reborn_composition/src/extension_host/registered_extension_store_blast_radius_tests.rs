//! Blast-radius coverage for the registered extension store: one entry's or
//! one owner's storage-layer error must not abort boot restore or the
//! installed listing for everyone else. Distinct from
//! `extension_lifecycle_registered_store_tests`'s corrupt-manifest coverage
//! (a per-entry TOML parse failure inside `load_filesystem_packages`) — this
//! pins directory-level `fs.list_dir` errors.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionInstallation, ExtensionInstallationId,
    ExtensionInstallationStore, ExtensionLifecycleService, ExtensionManifest,
    ExtensionManifestRecord, ExtensionManifestRef, ExtensionPackage, ExtensionRegistry,
    InMemoryExtensionInstallationStore, InstallationOwner, ManifestHash, ManifestSource,
    SharedExtensionRegistry,
};
use ironclaw_filesystem::{
    DirEntry, FileStat, FilesystemError, LocalFilesystem, RootFilesystem, VersionedEntry,
};
use ironclaw_host_api::{
    ExtensionId, InvocationId, ResourceScope, TenantId, UserId, VirtualPath, sha256_digest_token,
};
use ironclaw_product_workflow::{LifecyclePackageKind, LifecyclePackageRef, ProductWorkflowError};
use ironclaw_trust::{AdminConfig, HostTrustPolicy, InvalidationBus};
use tokio::sync::Mutex;

use crate::extension_host::available_extensions::{
    AvailableExtensionCatalog, AvailableExtensionPackage,
};
use crate::extension_host::extension_lifecycle::{
    ActiveExtensionPublisher, RebornLocalExtensionManagementPort, restore_extension_lifecycle_state,
};
use crate::extension_host::registered_extension_store::{
    migrate_legacy_owner_layout, migrate_unminted_registered_ids, resolve_registered_for_scope,
};
use crate::extension_host::registered_test_support::{
    fresh_boot_fixture, minted_extension_id, mounted_local_filesystem, seed_registered_installation,
};

const HEALTHY_OWNER_USER_ID: &str = "c3333333-7fe5-474c-965a-67cb69df3d06";
const BROKEN_OWNER_USER_ID: &str = "d4444444-7fe5-474c-965a-67cb69df3d07";
const HEALTHY_EXTENSION_ID: &str = "healthy-mcp";
const HEALTHY_MANIFEST_URL: &str = "http://127.0.0.1:9/mcp";

const HEALTHY_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "healthy-mcp"
name = "Healthy MCP"
version = "0.1.0"
description = "User-registered hosted MCP server (blast-radius fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://127.0.0.1:9/mcp"
"#;

/// Wraps a real `LocalFilesystem`, injecting a non-`NotFound` `list_dir`
/// error for exactly one virtual path. Everything else delegates to the
/// inner filesystem, so this simulates a transient storage-layer failure on
/// one owner's registered-extension directory (distinct from an unparseable
/// manifest, which `load_filesystem_packages` already skips-and-logs).
struct FailListDirFilesystem {
    inner: LocalFilesystem,
    fail_path: VirtualPath,
}

#[async_trait]
impl RootFilesystem for FailListDirFilesystem {
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        if path == &self.fail_path {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: ironclaw_filesystem::FilesystemOperation::ListDir,
                reason: "injected transient backend failure".to_string(),
            });
        }
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.inner.read_file(path).await
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.inner.write_file(path, bytes).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

fn seed_registered_manifest(storage_root: &std::path::Path, owner: &str, extension_id: &str) {
    let owner_dir = storage_root
        .join("system/extensions/registered")
        .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
        .join(owner)
        .join(extension_id);
    std::fs::create_dir_all(&owner_dir).expect("registered manifest dir"); // safety: test-only fixture setup.
    std::fs::write(owner_dir.join("manifest.toml"), HEALTHY_MANIFEST_TOML)
        .expect("write registered manifest"); // safety: test-only fixture setup.
}

/// Pins the `restore_extension_lifecycle_state` fix: an installation whose
/// registered manifest is gone (deleted/corrupted on disk, but still on
/// record in the installation store) must be skipped via the registered-store
/// fallback's (`list_for_owner`) miss, not abort the whole boot
/// restore — a second, healthy installation must still restore and publish.
/// RED before the fix (the whole restore returned `Err` on the first broken
/// installation).
#[tokio::test]
async fn restore_continues_past_installation_whose_registered_fallback_errors() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    seed_registered_manifest(&storage_root, HEALTHY_OWNER_USER_ID, HEALTHY_EXTENSION_ID);

    let filesystem: Arc<dyn RootFilesystem> = Arc::new(mounted_local_filesystem(&storage_root));

    let default_tenant =
        TenantId::from_trusted(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string());
    let installation_store: Arc<dyn ExtensionInstallationStore> =
        Arc::new(InMemoryExtensionInstallationStore::default());

    // Installation #1: on record in the installation store (manifest record
    // seeded, matching a real "registered then reinstalled/deleted" history),
    // but its `manifest.toml` no longer exists anywhere on disk. `catalog.resolve()`
    // misses (static catalog never holds `UserRegistered` packages) and
    // the row-owner-keyed `list_for_owner` also misses, since no owner directory
    // has this extension id — the missing-manifest scenario this fix targets.
    let (missing_extension_id, _) = seed_registered_installation(
        &installation_store,
        r#"
schema_version = "reborn.extension_manifest.v2"
id = "missing-mcp"
name = "Missing MCP"
version = "0.1.0"
description = "Deleted/corrupted user-registered manifest fixture"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://127.0.0.1:9/mcp"
"#,
        &default_tenant,
        &UserId::new(BROKEN_OWNER_USER_ID).expect("valid owner id"),
        "missing-mcp",
        Some(
            ManifestHash::new(sha256_digest_token(b"missing-mcp-placeholder"))
                .expect("valid manifest hash"),
        ),
    )
    .await;

    // Installation #2: the healthy, owner-registered extension that must
    // still restore and publish despite installation #1's failure.
    let (_healthy_extension_id, _) = seed_registered_installation(
        &installation_store,
        HEALTHY_MANIFEST_TOML,
        &default_tenant,
        &UserId::new(HEALTHY_OWNER_USER_ID).expect("valid owner id"),
        HEALTHY_EXTENSION_ID,
        None,
    )
    .await;
    let healthy_minted_id = minted_extension_id(
        &default_tenant,
        &UserId::new(HEALTHY_OWNER_USER_ID).expect("valid owner id"),
        HEALTHY_MANIFEST_URL,
    );

    let empty_catalog = AvailableExtensionCatalog::from_packages(Vec::new());
    let boot = fresh_boot_fixture();

    restore_extension_lifecycle_state(
        &empty_catalog,
        &filesystem,
        &installation_store,
        &boot.lifecycle_service,
        &boot.active_extensions,
    )
    .await
    .expect(
        "restore must skip an installation whose registered-store fallback errors, not abort \
         the whole boot restore (RED until skip-and-log lands)",
    );

    assert!(
        boot.active_registry
            .snapshot()
            .get_extension(&healthy_minted_id)
            .is_some(),
        "the healthy installation must still restore and publish despite the broken \
         installation's registered-store fallback error"
    );
    assert!(
        boot.active_registry
            .snapshot()
            .get_extension(&missing_extension_id)
            .is_none(),
        "the broken installation must not be published"
    );
}

const OWNER_A_USER_ID: &str = "e5555555-7fe5-474c-965a-67cb69df3d08";
const OWNER_B_USER_ID: &str = "f6666666-7fe5-474c-965a-67cb69df3d09";
const OWNER_A_MANIFEST_URL: &str = "http://owner-a.example/mcp";
const OWNER_B_MANIFEST_URL: &str = "http://owner-b.example/mcp";

/// The old restore collision cannot be constructed after id minting: owner
/// identity and endpoint are both encoded into the id.
#[test]
fn restore_collision_ids_are_distinct_by_construction() {
    let default_tenant =
        TenantId::from_trusted(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string());
    let owner_a = minted_extension_id(
        &default_tenant,
        &UserId::new(OWNER_A_USER_ID).expect("valid owner id"),
        OWNER_A_MANIFEST_URL,
    );
    let owner_b = minted_extension_id(
        &default_tenant,
        &UserId::new(OWNER_B_USER_ID).expect("valid owner id"),
        OWNER_B_MANIFEST_URL,
    );
    assert_ne!(owner_a, owner_b);
}

const CROSS_OWNER_RESTORE_OWNER_A_EXTENSION_ID: &str = "owner-a-registered-mcp";
const CROSS_OWNER_RESTORE_OWNER_B_EXTENSION_ID: &str = "owner-b-registered-mcp";

const CROSS_OWNER_RESTORE_OWNER_A_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "owner-a-registered-mcp"
name = "Owner A's Registered MCP"
version = "0.1.0"
description = "Owner A's own registration (cross-owner restore-correctness fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://owner-a-restore.example/mcp"
"#;

const CROSS_OWNER_RESTORE_OWNER_B_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "owner-b-registered-mcp"
name = "Owner B's Registered MCP"
version = "0.1.0"
description = "Owner B's own registration (cross-owner restore-correctness fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://owner-b-restore.example/mcp"
"#;

/// Restore-tier counterpart of `restore_collision_ids_are_distinct_by_construction`,
/// reconstructed against the CURRENT row-owner-keyed resolution mechanism
/// (`resolve_registered_installation_for_restore` -> `list_for_owner`, which
/// reads directly from `/system/extensions/registered/<tenant>/<owner>` —
/// never a cross-owner directory scan, per `list_for_owner`'s doc comment).
/// The commit that minted owner-scoped ids (7792e9b10) deleted the only test
/// that drove `restore_extension_lifecycle_state` with 2+ DISTINCT owners and
/// asserted per-owner manifest-content correctness
/// (`restore_uses_row_owners_registered_descriptor_not_a_differently_ordered_owner`,
/// which forced a specific `list_dir` ordering on a shared tenant-root scan —
/// a mechanism that no longer exists) and replaced it with a synchronous,
/// non-restore test that only asserts two minted ids differ. That left the
/// row-keyed resolution mechanism itself with no live coverage proving owner
/// A's row restores owner A's own descriptor and owner B's row restores
/// owner B's own descriptor, never the other's. This test closes that gap
/// directly against today's architecture: two distinct owners each register
/// their own descriptor (distinct manifest name/URL), and restore must
/// publish each row's OWN owner's content.
#[tokio::test]
async fn restore_resolves_each_distinct_owners_row_to_its_own_registered_descriptor() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");

    let owner_a_dir = storage_root
        .join("system/extensions/registered")
        .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
        .join(OWNER_A_USER_ID)
        .join(CROSS_OWNER_RESTORE_OWNER_A_EXTENSION_ID);
    std::fs::create_dir_all(&owner_a_dir).expect("owner A registered manifest dir");
    std::fs::write(
        owner_a_dir.join("manifest.toml"),
        CROSS_OWNER_RESTORE_OWNER_A_MANIFEST_TOML,
    )
    .expect("write owner A's descriptor");

    let owner_b_dir = storage_root
        .join("system/extensions/registered")
        .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
        .join(OWNER_B_USER_ID)
        .join(CROSS_OWNER_RESTORE_OWNER_B_EXTENSION_ID);
    std::fs::create_dir_all(&owner_b_dir).expect("owner B registered manifest dir");
    std::fs::write(
        owner_b_dir.join("manifest.toml"),
        CROSS_OWNER_RESTORE_OWNER_B_MANIFEST_TOML,
    )
    .expect("write owner B's descriptor");

    let filesystem: Arc<dyn RootFilesystem> = Arc::new(mounted_local_filesystem(&storage_root));

    let default_tenant =
        TenantId::from_trusted(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string());
    let installation_store: Arc<dyn ExtensionInstallationStore> =
        Arc::new(InMemoryExtensionInstallationStore::default());
    let (owner_a_extension_id, _) = seed_registered_installation(
        &installation_store,
        CROSS_OWNER_RESTORE_OWNER_A_MANIFEST_TOML,
        &default_tenant,
        &UserId::new(OWNER_A_USER_ID).expect("valid owner id"),
        CROSS_OWNER_RESTORE_OWNER_A_EXTENSION_ID,
        None,
    )
    .await;
    let (owner_b_extension_id, _) = seed_registered_installation(
        &installation_store,
        CROSS_OWNER_RESTORE_OWNER_B_MANIFEST_TOML,
        &default_tenant,
        &UserId::new(OWNER_B_USER_ID).expect("valid owner id"),
        CROSS_OWNER_RESTORE_OWNER_B_EXTENSION_ID,
        None,
    )
    .await;
    assert_ne!(
        owner_a_extension_id, owner_b_extension_id,
        "sanity: distinct owners registering distinct URLs must mint distinct ids"
    );

    let empty_catalog = AvailableExtensionCatalog::from_packages(Vec::new());
    let boot = fresh_boot_fixture();

    restore_extension_lifecycle_state(
        &empty_catalog,
        &filesystem,
        &installation_store,
        &boot.lifecycle_service,
        &boot.active_extensions,
    )
    .await
    .expect("restore of two distinct owners' registered installations must succeed");

    let snapshot = boot.active_registry.snapshot();

    let published_a = snapshot
        .get_extension(&owner_a_extension_id)
        .expect("owner A's row must restore and publish");
    let ironclaw_extensions::ExtensionRuntime::Mcp { url: url_a, .. } =
        &published_a.manifest.runtime
    else {
        panic!("expected an MCP runtime declaration for owner A");
    };
    assert_eq!(
        url_a.as_deref(),
        Some("http://owner-a-restore.example/mcp"),
        "owner A's row must restore and publish owner A's OWN registered descriptor, never \
         owner B's"
    );
    assert_eq!(
        published_a.manifest.name, "Owner A's Registered MCP",
        "owner A's restored manifest name must be owner A's own, never owner B's"
    );

    let published_b = snapshot
        .get_extension(&owner_b_extension_id)
        .expect("owner B's row must restore and publish");
    let ironclaw_extensions::ExtensionRuntime::Mcp { url: url_b, .. } =
        &published_b.manifest.runtime
    else {
        panic!("expected an MCP runtime declaration for owner B");
    };
    assert_eq!(
        url_b.as_deref(),
        Some("http://owner-b-restore.example/mcp"),
        "owner B's row must restore and publish owner B's OWN registered descriptor, never \
         owner A's"
    );
    assert_eq!(
        published_b.manifest.name, "Owner B's Registered MCP",
        "owner B's restored manifest name must be owner B's own, never owner A's"
    );
}

const CATALOG_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "catalog-mcp"
name = "Catalog MCP"
version = "0.1.0"
description = "Shared-catalog extension (list blast-radius fixture)"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/catalog.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "catalog-mcp.search"
description = "Search catalog fixture data"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"
"#;

fn catalog_fixture_package() -> AvailableExtensionPackage {
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().expect("host port catalog"); // safety: test-only fixture setup.
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().expect("host API contracts"); // safety: test-only fixture setup.
    let manifest = ExtensionManifest::parse_with_host_api_contracts(
        CATALOG_MANIFEST_TOML,
        ManifestSource::InstalledLocal,
        &host_ports,
        &contracts,
    )
    .expect("catalog fixture manifest"); // safety: test-only fixture setup.
    let root = VirtualPath::new("/system/extensions/catalog-mcp").expect("extension root"); // safety: test-only fixture setup.
    let package = ExtensionPackage::from_manifest_toml(manifest, root, CATALOG_MANIFEST_TOML)
        .expect("catalog fixture package"); // safety: test-only fixture setup.
    AvailableExtensionPackage {
        package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, "catalog-mcp")
            .expect("catalog fixture ref"), // safety: test-only fixture setup.
        manifest_toml: CATALOG_MANIFEST_TOML.to_string(),
        source: ManifestSource::InstalledLocal,
        package,
        cleanup_requirements: Vec::new(),
        surface_kinds: Vec::new(),
        assets: Vec::new(),
    }
}

fn owner_scope(user: &str) -> ResourceScope {
    let user = UserId::new(user).expect("valid user"); // safety: test-only fixture setup.
    ResourceScope::local_default(user, InvocationId::new()).expect("valid local scope") // safety: test-only fixture setup.
}

/// Pins the resolver's miss-vs-failure split (T2 review item): a package that
/// simply is not in the caller's registered set is `Ok(None)`, while a real
/// storage failure on the owner's registered root is `Err` — callers like the
/// installed-summaries list need the distinction to skip silently on the
/// former and log on the latter instead of conflating both.
#[tokio::test]
async fn resolve_registered_for_scope_distinguishes_missing_from_read_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    seed_registered_manifest(&storage_root, HEALTHY_OWNER_USER_ID, HEALTHY_EXTENSION_ID);

    let local_filesystem = mounted_local_filesystem(&storage_root);
    let healthy_owner_root = VirtualPath::new(format!(
        "/system/extensions/registered/{}/{HEALTHY_OWNER_USER_ID}",
        ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID
    ))
    .expect("valid virtual path");
    let filesystem = FailListDirFilesystem {
        inner: local_filesystem,
        fail_path: healthy_owner_root,
    };

    let package_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "absent-mcp").expect("valid ref");
    let miss = resolve_registered_for_scope(
        &filesystem,
        &owner_scope(BROKEN_OWNER_USER_ID),
        &package_ref,
    )
    .await
    .expect("an absent package under a readable (empty) owner root is a plain miss");
    assert!(miss.is_none(), "a genuine miss must be Ok(None)");

    let healthy_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, HEALTHY_EXTENSION_ID)
            .expect("valid ref");
    resolve_registered_for_scope(
        &filesystem,
        &owner_scope(HEALTHY_OWNER_USER_ID),
        &healthy_ref,
    )
    .await
    .expect_err("a storage failure on the owner's registered root must surface as Err");
}

/// Fix C (#5970 review): a genuine top-level `fs.list_dir` failure on the
/// CALLER's OWN registered directory is not "one corrupt entry among many" —
/// individual corrupt manifests are already skip-and-logged one level deeper
/// in `load_filesystem_package`, so the blast-radius/skip-log policy doesn't
/// apply here. `registered_packages_by_id` now propagates this failure
/// (matching its sibling `resolve_available_for_scope`) instead of silently
/// collapsing to an empty registered set, so `list_installed` must fail
/// closed rather than quietly dropping the caller's registered installs from
/// their own listing.
#[tokio::test]
async fn list_installed_fails_closed_when_callers_own_registered_directory_read_errors() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    seed_registered_manifest(&storage_root, HEALTHY_OWNER_USER_ID, HEALTHY_EXTENSION_ID);

    let local_filesystem = mounted_local_filesystem(&storage_root);
    let owner_root = VirtualPath::new(format!(
        "/system/extensions/registered/{}/{HEALTHY_OWNER_USER_ID}",
        ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID
    ))
    .expect("valid virtual path");
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(FailListDirFilesystem {
        inner: local_filesystem,
        fail_path: owner_root,
    });

    let caller = UserId::new(HEALTHY_OWNER_USER_ID).expect("valid user");
    let installation_store: Arc<dyn ExtensionInstallationStore> =
        Arc::new(InMemoryExtensionInstallationStore::default());
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().expect("host port catalog");
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().expect("host API contracts");
    installation_store
        .upsert_manifest(
            ExtensionManifestRecord::from_toml_with_contracts(
                CATALOG_MANIFEST_TOML,
                ManifestSource::InstalledLocal,
                &host_ports,
                None,
                &contracts,
            )
            .expect("catalog manifest record"),
        )
        .await
        .expect("seed catalog manifest record");
    installation_store
        .upsert_manifest(
            ExtensionManifestRecord::from_toml_with_contracts(
                HEALTHY_MANIFEST_TOML,
                ManifestSource::UserRegistered {
                    tenant_id: TenantId::from_trusted(
                        ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string(),
                    ),
                    owner: caller.clone(),
                },
                &host_ports,
                None,
                &contracts,
            )
            .expect("registered manifest record"),
        )
        .await
        .expect("seed registered manifest record");
    // Row 1: tenant-shared, resolvable through the shared catalog.
    let catalog_extension_id = ExtensionId::new("catalog-mcp").expect("valid extension id");
    installation_store
        .upsert_installation(
            ExtensionInstallation::new(
                ExtensionInstallationId::new("catalog-mcp").expect("valid installation id"),
                catalog_extension_id.clone(),
                ExtensionActivationState::Enabled,
                ExtensionManifestRef::new(catalog_extension_id, None),
                Vec::new(),
                chrono::Utc::now(),
                InstallationOwner::Tenant,
            )
            .expect("catalog installation"),
        )
        .await
        .expect("seed catalog installation");
    // Row 2: the caller's registered install, whose overlay read errors.
    let registered_extension_id = ExtensionId::new(HEALTHY_EXTENSION_ID).expect("valid id");
    installation_store
        .upsert_installation(
            ExtensionInstallation::new(
                ExtensionInstallationId::new(HEALTHY_EXTENSION_ID).expect("valid installation id"),
                registered_extension_id.clone(),
                ExtensionActivationState::Enabled,
                ExtensionManifestRef::new(registered_extension_id, None),
                Vec::new(),
                chrono::Utc::now(),
                InstallationOwner::user(caller.clone()),
            )
            .expect("registered installation"),
        )
        .await
        .expect("seed registered installation");

    let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
    let trust_policy =
        Arc::new(HostTrustPolicy::new(vec![Box::new(AdminConfig::new())]).expect("trust policy"));
    let port = RebornLocalExtensionManagementPort::new(
        filesystem,
        AvailableExtensionCatalog::from_packages(vec![catalog_fixture_package()]),
        installation_store,
        Arc::new(Mutex::new(ExtensionLifecycleService::new(
            ExtensionRegistry::new(),
        ))),
        ActiveExtensionPublisher::new(
            active_registry,
            trust_policy,
            Arc::new(InvalidationBus::new()),
        ),
        None,
        caller.clone(),
    );

    let error = port
        .list_installed(&owner_scope(HEALTHY_OWNER_USER_ID))
        .await
        .expect_err(
            "a genuine failure reading the caller's own registered directory must fail the \
             whole listing closed, not silently drop their registered installs",
        );
    assert!(matches!(error, ProductWorkflowError::Transient { .. }));
}

/// Wraps a real `LocalFilesystem`, counting `list_dir` calls against exactly
/// one owner directory. Boot restore's catalog-miss fallback must load a
/// given (tenant, owner)'s registered set at most ONCE per boot, no matter
/// how many of that owner's installations miss the shared catalog — before
/// the fix, `resolve_registered_for_owner` (via `list_for_scope`, a full
/// directory walk + manifest parse) was called once PER catalog-miss
/// installation.
struct CountingListDirFilesystem {
    inner: LocalFilesystem,
    counted_path: VirtualPath,
    count: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl RootFilesystem for CountingListDirFilesystem {
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        if path == &self.counted_path {
            self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.inner.read_file(path).await
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.inner.write_file(path, bytes).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

/// Boot-scaling regression: two registered installations owned by the SAME
/// (tenant, owner), both missing the shared catalog, must load that owner's
/// registered directory ONCE during restore, not once per installation.
/// RED before the fix: `count` was 2 (one `list_dir` per catalog-miss
/// installation via `resolve_registered_for_owner`).
#[tokio::test]
async fn restore_loads_each_owners_registered_set_once_for_multiple_installations() {
    const OWNER_USER_ID: &str = "a1111111-7fe5-474c-965a-67cb69df3d10";
    const FIRST_EXTENSION_ID: &str = "owner-first-mcp";
    const SECOND_EXTENSION_ID: &str = "owner-second-mcp";
    const FIRST_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "owner-first-mcp"
name = "Owner First MCP"
version = "0.1.0"
description = "First registered MCP under a shared owner (boot-scaling fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://127.0.0.1:9/mcp"
"#;
    const SECOND_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "owner-second-mcp"
name = "Owner Second MCP"
version = "0.1.0"
description = "Second registered MCP under the same shared owner (boot-scaling fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://127.0.0.1:9/second-mcp"
"#;

    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    seed_registered_manifest(&storage_root, OWNER_USER_ID, FIRST_EXTENSION_ID);
    std::fs::write(
        storage_root
            .join("system/extensions/registered")
            .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
            .join(OWNER_USER_ID)
            .join(FIRST_EXTENSION_ID)
            .join("manifest.toml"),
        FIRST_MANIFEST_TOML,
    )
    .expect("write first registered descriptor");
    let second_dir = storage_root
        .join("system/extensions/registered")
        .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
        .join(OWNER_USER_ID)
        .join(SECOND_EXTENSION_ID);
    std::fs::create_dir_all(&second_dir).expect("second registered descriptor dir");
    std::fs::write(second_dir.join("manifest.toml"), SECOND_MANIFEST_TOML)
        .expect("write second registered descriptor");

    let local_filesystem = mounted_local_filesystem(&storage_root);
    let owner_root = VirtualPath::new(format!(
        "/system/extensions/registered/{}/{}",
        ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID,
        OWNER_USER_ID
    ))
    .expect("valid virtual path");
    let counting_filesystem = Arc::new(CountingListDirFilesystem {
        inner: local_filesystem,
        counted_path: owner_root,
        count: std::sync::atomic::AtomicUsize::new(0),
    });
    let filesystem: Arc<dyn RootFilesystem> = counting_filesystem.clone();

    let default_tenant =
        TenantId::from_trusted(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string());
    let installation_store: Arc<dyn ExtensionInstallationStore> =
        Arc::new(InMemoryExtensionInstallationStore::default());
    let owner = UserId::new(OWNER_USER_ID).expect("valid owner id");
    let (_first_extension_id, _) = seed_registered_installation(
        &installation_store,
        FIRST_MANIFEST_TOML,
        &default_tenant,
        &owner,
        FIRST_EXTENSION_ID,
        None,
    )
    .await;
    let (_second_extension_id, _) = seed_registered_installation(
        &installation_store,
        SECOND_MANIFEST_TOML,
        &default_tenant,
        &owner,
        SECOND_EXTENSION_ID,
        None,
    )
    .await;
    let first_minted_id = minted_extension_id(&default_tenant, &owner, "http://127.0.0.1:9/mcp");
    let second_minted_id =
        minted_extension_id(&default_tenant, &owner, "http://127.0.0.1:9/second-mcp");

    let empty_catalog = AvailableExtensionCatalog::from_packages(Vec::new());
    let boot = fresh_boot_fixture();

    restore_extension_lifecycle_state(
        &empty_catalog,
        &filesystem,
        &installation_store,
        &boot.lifecycle_service,
        &boot.active_extensions,
    )
    .await
    .expect("restore of two same-owner registered installations must succeed");

    let snapshot = boot.active_registry.snapshot();
    assert!(
        snapshot.get_extension(&first_minted_id).is_some(),
        "first installation must still restore and publish"
    );
    assert!(
        snapshot.get_extension(&second_minted_id).is_some(),
        "second installation must still restore and publish"
    );

    assert_eq!(
        counting_filesystem
            .count
            .load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the owner's registered directory must be loaded once per boot, not once per \
         catalog-miss installation (RED before batching: this was 2)"
    );
}

/// Wraps a real `LocalFilesystem`, counting `list_dir` calls against exactly
/// one owner directory AND injecting a non-`NotFound` failure on every one of
/// those calls — the counting counterpart of `FailListDirFilesystem` used to
/// pin that a failed owner lookup is cached (not re-walked) across multiple
/// installations sharing that owner.
struct CountingFailListDirFilesystem {
    inner: LocalFilesystem,
    fail_path: VirtualPath,
    count: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl RootFilesystem for CountingFailListDirFilesystem {
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        if path == &self.fail_path {
            self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: ironclaw_filesystem::FilesystemOperation::ListDir,
                reason: "injected transient backend failure".to_string(),
            });
        }
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.inner.read_file(path).await
    }
}

/// CodeRabbit review pin: `resolve_registered_installation_for_restore` must
/// cache a FAILED owner lookup too, not just a successful one — otherwise
/// every installation for the same (tenant, owner) after the first re-walks
/// the filesystem and re-logs the same failure. Two installations share a
/// failing owner; `list_dir` on that owner's directory must be attempted
/// exactly once. RED before the fix: `count` was 2.
#[tokio::test]
async fn restore_caches_owners_registered_lookup_failure_across_installations() {
    const OWNER_USER_ID: &str = "d9999999-7fe5-474c-965a-67cb69df3d13";
    const FIRST_EXTENSION_ID: &str = "owner-failing-first-mcp";
    const SECOND_EXTENSION_ID: &str = "owner-failing-second-mcp";
    const FIRST_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "owner-failing-first-mcp"
name = "Owner Failing First MCP"
version = "0.1.0"
description = "First registered MCP under an owner whose list_dir fails (failure-cache fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://127.0.0.1:9/mcp"
"#;
    const SECOND_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "owner-failing-second-mcp"
name = "Owner Failing Second MCP"
version = "0.1.0"
description = "Second registered MCP under the same failing owner (failure-cache fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://127.0.0.1:9/mcp"
"#;

    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    std::fs::create_dir_all(&storage_root).expect("storage root"); // safety: test-only fixture setup.

    let local_filesystem = mounted_local_filesystem(&storage_root);
    let owner_root = VirtualPath::new(format!(
        "/system/extensions/registered/{}/{}",
        ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID,
        OWNER_USER_ID
    ))
    .expect("valid virtual path");
    let failing_filesystem = Arc::new(CountingFailListDirFilesystem {
        inner: local_filesystem,
        fail_path: owner_root,
        count: std::sync::atomic::AtomicUsize::new(0),
    });
    let filesystem: Arc<dyn RootFilesystem> = failing_filesystem.clone();

    let default_tenant =
        TenantId::from_trusted(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string());
    let installation_store: Arc<dyn ExtensionInstallationStore> =
        Arc::new(InMemoryExtensionInstallationStore::default());
    let owner = UserId::new(OWNER_USER_ID).expect("valid owner id");
    seed_registered_installation(
        &installation_store,
        FIRST_MANIFEST_TOML,
        &default_tenant,
        &owner,
        FIRST_EXTENSION_ID,
        None,
    )
    .await;
    seed_registered_installation(
        &installation_store,
        SECOND_MANIFEST_TOML,
        &default_tenant,
        &owner,
        SECOND_EXTENSION_ID,
        None,
    )
    .await;

    let empty_catalog = AvailableExtensionCatalog::from_packages(Vec::new());
    let boot = fresh_boot_fixture();

    restore_extension_lifecycle_state(
        &empty_catalog,
        &filesystem,
        &installation_store,
        &boot.lifecycle_service,
        &boot.active_extensions,
    )
    .await
    .expect(
        "restore must skip-and-continue past a failing owner's registered lookup for both \
         installations, not abort",
    );

    assert_eq!(
        failing_filesystem
            .count
            .load(std::sync::atomic::Ordering::SeqCst),
        1,
        "a failed owner lookup must be cached: only the FIRST installation should attempt \
         list_dir; the second must hit the cached failure outcome, not re-walk (RED before \
         caching the failure: this was 2)"
    );
}

const MIGRATION_GOOD_OWNER_USER_ID: &str = "b7777777-7fe5-474c-965a-67cb69df3d11";
const MIGRATION_BROKEN_OWNER_USER_ID: &str = "c8888888-7fe5-474c-965a-67cb69df3d12";
const MIGRATION_GOOD_EXTENSION_ID: &str = "good-legacy-mcp";
const MIGRATION_BROKEN_EXTENSION_ID: &str = "broken-legacy-mcp";
const MIGRATION_GOOD_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "good-legacy-mcp"
name = "Good Legacy MCP"
version = "0.1.0"
description = "Healthy sibling owner's legacy descriptor (migration blast-radius fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://127.0.0.1:9/mcp"
"#;
const MIGRATION_BROKEN_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "broken-legacy-mcp"
name = "Broken Legacy MCP"
version = "0.1.0"
description = "Legacy descriptor whose owner directory fails to list (migration blast-radius fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://127.0.0.1:9/mcp"
"#;

/// `migrate_legacy_owner_layout`'s per-owner error swallow
/// (`tracing::debug!` with no propagation, no `continue`-vs-abort branch)
/// was untested: nothing pinned that one owner's `list_dir` failure during
/// legacy-layout migration is skip-and-logged rather than aborting the
/// migration pass for every other owner. Injects a `list_dir` failure on
/// exactly the broken owner's legacy directory (reusing `FailListDirFilesystem`
/// from `resolve_registered_for_scope_distinguishes_missing_from_read_failure`
/// above) and asserts the sibling (good) owner's descriptor still migrates to
/// the tenant-scoped path despite it.
#[tokio::test]
async fn migration_continues_after_one_owner_io_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");

    // Seed BOTH owners in the pre-tenant (legacy) layout: no tenant segment
    // between `registered` and the owner directory.
    let good_legacy_dir = storage_root
        .join("system/extensions/registered")
        .join(MIGRATION_GOOD_OWNER_USER_ID)
        .join(MIGRATION_GOOD_EXTENSION_ID);
    std::fs::create_dir_all(&good_legacy_dir).expect("good owner legacy dir");
    std::fs::write(
        good_legacy_dir.join("manifest.toml"),
        MIGRATION_GOOD_MANIFEST_TOML,
    )
    .expect("write good owner legacy manifest");

    let broken_legacy_dir = storage_root
        .join("system/extensions/registered")
        .join(MIGRATION_BROKEN_OWNER_USER_ID)
        .join(MIGRATION_BROKEN_EXTENSION_ID);
    std::fs::create_dir_all(&broken_legacy_dir).expect("broken owner legacy dir");
    std::fs::write(
        broken_legacy_dir.join("manifest.toml"),
        MIGRATION_BROKEN_MANIFEST_TOML,
    )
    .expect("write broken owner legacy manifest");

    let local_filesystem = mounted_local_filesystem(&storage_root);
    // `migrate_legacy_owner_dir` lists exactly this path for the broken
    // owner — inject the failure there, leaving the top-level registered
    // root listing and the good owner's own directory untouched.
    let broken_owner_legacy_root = VirtualPath::new(format!(
        "/system/extensions/registered/{MIGRATION_BROKEN_OWNER_USER_ID}"
    ))
    .expect("valid virtual path");
    let filesystem = FailListDirFilesystem {
        inner: local_filesystem,
        fail_path: broken_owner_legacy_root,
    };

    migrate_legacy_owner_layout(&filesystem).await.expect(
        "one owner's list_dir failure during legacy migration must be skip-and-logged, \
             never abort the whole migration pass",
    );

    // Legacy migration also mints a hosted-MCP id for each descriptor it
    // moves (R1), so the migrated directory is keyed by the minted id, not
    // the pre-migration `MIGRATION_GOOD_EXTENSION_ID` folder name.
    let default_tenant =
        TenantId::from_trusted(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string());
    let good_owner = UserId::new(MIGRATION_GOOD_OWNER_USER_ID).expect("valid owner id");
    let good_minted_id =
        minted_extension_id(&default_tenant, &good_owner, "http://127.0.0.1:9/mcp");

    let migrated_good_manifest = storage_root
        .join("system/extensions/registered")
        .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
        .join(MIGRATION_GOOD_OWNER_USER_ID)
        .join(good_minted_id.as_str())
        .join("manifest.toml");
    assert!(
        migrated_good_manifest.is_file(),
        "the sibling (good) owner's descriptor must still migrate to the tenant-scoped path \
         despite the broken owner's list_dir failure"
    );

    // The broken owner's descriptor must be left exactly where it was — the
    // failure occurred before any file was moved, so nothing should have
    // been touched or partially migrated for it.
    assert!(
        broken_legacy_dir.join("manifest.toml").is_file(),
        "the broken owner's legacy descriptor must remain untouched after the skipped migration"
    );
    assert!(
        !storage_root
            .join("system/extensions/registered")
            .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
            .join(MIGRATION_BROKEN_OWNER_USER_ID)
            .exists(),
        "the broken owner's descriptor must not have been migrated"
    );
}

const SIBLING_MIGRATION_OWNER_USER_ID: &str = "a2222222-7fe5-474c-965a-67cb69df3d14";
const SIBLING_HEALTHY_EXTENSION_ID: &str = "sibling-healthy-mcp";
const SIBLING_BROKEN_EXTENSION_ID: &str = "sibling-broken-mcp";
const SIBLING_HEALTHY_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "sibling-healthy-mcp"
name = "Sibling Healthy MCP"
version = "0.1.0"
description = "Healthy sibling legacy descriptor (mint-failure containment fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://127.0.0.1:9/mcp"
"#;
// No `[runtime].url` — `minted_manifest_for_legacy` returns `Err` for this
// descriptor, the exact per-sibling failure Fix 1's skip-and-log contains.
const SIBLING_BROKEN_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "sibling-broken-mcp"
name = "Sibling Broken MCP"
version = "0.1.0"
description = "Legacy descriptor with no hosted MCP URL (mint-failure containment fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
"#;

/// Pins Fix 1's `migrate_legacy_owner_dir` site: a mint failure on ONE legacy
/// descriptor must not abort processing of the REST of that owner's sibling
/// descriptors. The broken sibling's legacy manifest and the owner directory
/// itself (via `migrated_all = false`) must stay intact, while a healthy
/// sibling under the SAME owner directory still migrates successfully.
#[tokio::test]
async fn legacy_owner_migration_skips_one_minting_failure_and_migrates_healthy_sibling() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");

    let healthy_legacy_dir = storage_root
        .join("system/extensions/registered")
        .join(SIBLING_MIGRATION_OWNER_USER_ID)
        .join(SIBLING_HEALTHY_EXTENSION_ID);
    std::fs::create_dir_all(&healthy_legacy_dir).expect("healthy sibling legacy dir");
    std::fs::write(
        healthy_legacy_dir.join("manifest.toml"),
        SIBLING_HEALTHY_MANIFEST_TOML,
    )
    .expect("write healthy sibling legacy manifest");

    let broken_legacy_dir = storage_root
        .join("system/extensions/registered")
        .join(SIBLING_MIGRATION_OWNER_USER_ID)
        .join(SIBLING_BROKEN_EXTENSION_ID);
    std::fs::create_dir_all(&broken_legacy_dir).expect("broken sibling legacy dir");
    std::fs::write(
        broken_legacy_dir.join("manifest.toml"),
        SIBLING_BROKEN_MANIFEST_TOML,
    )
    .expect("write broken sibling legacy manifest");

    let filesystem = mounted_local_filesystem(&storage_root);

    migrate_legacy_owner_layout(&filesystem).await.expect(
        "one sibling's mint failure must be skip-and-logged, never abort the whole owner's \
         migration pass",
    );

    let default_tenant =
        TenantId::from_trusted(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string());
    let owner = UserId::new(SIBLING_MIGRATION_OWNER_USER_ID).expect("valid owner id");
    let healthy_minted_id = minted_extension_id(&default_tenant, &owner, "http://127.0.0.1:9/mcp");

    let migrated_healthy_manifest = storage_root
        .join("system/extensions/registered")
        .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
        .join(SIBLING_MIGRATION_OWNER_USER_ID)
        .join(healthy_minted_id.as_str())
        .join("manifest.toml");
    assert!(
        migrated_healthy_manifest.is_file(),
        "the healthy sibling must still migrate to the tenant-scoped path despite the broken \
         sibling's mint failure"
    );
    assert!(
        !healthy_legacy_dir.exists(),
        "the healthy sibling's legacy directory must be removed once migrated"
    );

    assert_eq!(
        std::fs::read_to_string(broken_legacy_dir.join("manifest.toml"))
            .expect("read broken sibling's legacy manifest"),
        SIBLING_BROKEN_MANIFEST_TOML,
        "the broken sibling's legacy manifest must be left byte-unchanged, not deleted or \
         corrupted"
    );
    assert!(
        storage_root
            .join("system/extensions/registered")
            .join(SIBLING_MIGRATION_OWNER_USER_ID)
            .exists(),
        "the legacy owner directory must not be deleted while the broken sibling's descriptor \
         remains unmigrated inside it"
    );
}

/// Wraps a real `LocalFilesystem`, injecting a non-`NotFound` `write_file`
/// error for exactly one destination path — the counting-free counterpart of
/// `FailListDirFilesystem` targeting `copy_tree`'s asset writes instead of a
/// directory listing, to simulate a partial `copy_tree` failure.
struct FailWriteFileFilesystem {
    inner: LocalFilesystem,
    fail_path: VirtualPath,
}

#[async_trait]
impl RootFilesystem for FailWriteFileFilesystem {
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.inner.read_file(path).await
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        if path == &self.fail_path {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: ironclaw_filesystem::FilesystemOperation::WriteFile,
                reason: "injected transient backend failure".to_string(),
            });
        }
        self.inner.write_file(path, bytes).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

const COPY_TREE_MIGRATION_OWNER_USER_ID: &str = "a3333333-7fe5-474c-965a-67cb69df3d15";
const COPY_TREE_MIGRATION_OLD_EXTENSION_ID: &str = "acme-mcp-unminted-copytree";
const COPY_TREE_MIGRATION_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "acme-mcp-unminted-copytree"
name = "Acme Unminted Copytree MCP"
version = "0.1.0"
description = "Unminted registered MCP with a nested asset (copy_tree partial-failure fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://127.0.0.1:9/mcp-unminted-copytree"
"#;

/// #5970 review: `copy_tree` used to copy `manifest.toml` itself as an
/// incidental side effect of its recursive walk, with no rollback on partial
/// failure. If a later sibling asset file failed to copy, the destination
/// `manifest.toml` (written first, since it sits at the top level and is
/// visited before the nested asset directory) was left behind with STALE
/// (pre-remint) content — corrupting `destination_manifest`'s role as a
/// trustworthy one-shot completion sentinel in `migrate_registered_id`. A
/// caller retrying later would see `fs.stat(&destination_manifest)` succeed,
/// skip `copy_tree` entirely, and finish "migrating" onto an incomplete,
/// wrongly-keyed destination tree while deleting the original — real data
/// loss (more severe than `migrate_legacy_owner_dir`'s orphaned-but-intact
/// consequence).
///
/// RED before the fix: assertion (b) below (`destination_manifest` absent
/// after the failed attempt) failed, because `copy_tree` wrote it before
/// reaching the nested asset file that fails.
#[tokio::test]
async fn migrate_registered_id_does_not_leave_stale_destination_manifest_on_partial_copy_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");

    let default_tenant =
        TenantId::from_trusted(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string());
    let owner = UserId::new(COPY_TREE_MIGRATION_OWNER_USER_ID).expect("valid owner id");

    // Already tenant-scoped (not pre-tenant), so `migrate_legacy_owner_layout`
    // never touches it — only the unminted-id path (`migrate_registered_id`)
    // is exercised.
    let old_dir = storage_root
        .join("system/extensions/registered")
        .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
        .join(COPY_TREE_MIGRATION_OWNER_USER_ID)
        .join(COPY_TREE_MIGRATION_OLD_EXTENSION_ID);
    std::fs::create_dir_all(old_dir.join("schemas")).expect("old descriptor schemas dir");
    std::fs::write(
        old_dir.join("manifest.toml"),
        COPY_TREE_MIGRATION_MANIFEST_TOML,
    )
    .expect("write old manifest");
    std::fs::write(
        old_dir.join("schemas").join("tool.input.json"),
        "{\"type\":\"object\"}",
    )
    .expect("write old nested asset");

    let installation_store: Arc<dyn ExtensionInstallationStore> =
        Arc::new(InMemoryExtensionInstallationStore::default());
    let (new_id, _) = seed_registered_installation(
        &installation_store,
        COPY_TREE_MIGRATION_MANIFEST_TOML,
        &default_tenant,
        &owner,
        COPY_TREE_MIGRATION_OLD_EXTENSION_ID,
        None,
    )
    .await;
    let old_installation_id = ExtensionInstallationId::new(COPY_TREE_MIGRATION_OLD_EXTENSION_ID)
        .expect("valid installation id");
    let old_extension_id =
        ExtensionId::new(COPY_TREE_MIGRATION_OLD_EXTENSION_ID).expect("valid extension id");

    let destination_manifest = storage_root
        .join("system/extensions/registered")
        .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
        .join(COPY_TREE_MIGRATION_OWNER_USER_ID)
        .join(new_id.as_str())
        .join("manifest.toml");
    let destination_asset_virtual_path = VirtualPath::new(format!(
        "/system/extensions/registered/{}/{COPY_TREE_MIGRATION_OWNER_USER_ID}/{}/schemas/tool.input.json",
        ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID,
        new_id.as_str()
    ))
    .expect("valid virtual path");

    // ── First attempt: the sibling asset write fails partway through. ──────
    let failing_filesystem = FailWriteFileFilesystem {
        inner: mounted_local_filesystem(&storage_root),
        fail_path: destination_asset_virtual_path,
    };
    migrate_unminted_registered_ids(&failing_filesystem, &installation_store)
        .await
        .expect("a per-record migration failure is skip-and-logged, never propagated");

    assert!(
        !destination_manifest.is_file(),
        "(b) destination_manifest must never be written with stale content while the copy is \
         incomplete (RED before the fix: copy_tree wrote manifest.toml before reaching the \
         failing nested asset)"
    );
    assert!(
        old_dir.join("manifest.toml").is_file(),
        "(a) the original source manifest must survive an incomplete copy"
    );
    assert_eq!(
        std::fs::read_to_string(old_dir.join("schemas").join("tool.input.json"))
            .expect("read old nested asset"),
        "{\"type\":\"object\"}",
        "(a) the original source asset must be left byte-unchanged"
    );
    assert!(
        installation_store
            .get_installation(&old_installation_id)
            .await
            .expect("store read")
            .is_some(),
        "(a) the original installation row must survive an incomplete copy"
    );
    assert!(
        installation_store
            .get_manifest(&old_extension_id)
            .await
            .expect("store read")
            .is_some(),
        "(a) the original manifest record must survive an incomplete copy"
    );

    // ── Retry: same storage root, no injected failure. ──────────────────────
    let healthy_filesystem = mounted_local_filesystem(&storage_root);
    migrate_unminted_registered_ids(&healthy_filesystem, &installation_store)
        .await
        .expect("retry must succeed once the injected failure is gone");

    assert!(
        destination_manifest.is_file(),
        "(c) retry must complete migration: destination_manifest now exists"
    );
    let migrated_toml =
        std::fs::read_to_string(&destination_manifest).expect("read migrated manifest");
    assert!(
        migrated_toml.contains(new_id.as_str()),
        "(c) the migrated manifest must carry the newly minted id, not stale old-id content"
    );
    assert_eq!(
        std::fs::read_to_string(
            storage_root
                .join("system/extensions/registered")
                .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
                .join(COPY_TREE_MIGRATION_OWNER_USER_ID)
                .join(new_id.as_str())
                .join("schemas")
                .join("tool.input.json")
        )
        .expect("read migrated nested asset"),
        "{\"type\":\"object\"}",
        "(c) the nested asset must be copied on the successful retry"
    );
    assert!(
        !old_dir.exists(),
        "(c) the original source directory must be removed once migration completes"
    );
    assert!(
        installation_store
            .get_installation(&old_installation_id)
            .await
            .expect("store read")
            .is_none(),
        "(c) the old installation row must be removed once migration completes"
    );
}

const EXISTING_DESTINATION_OWNER_USER_ID: &str = "e1010101-7fe5-474c-965a-67cb69df3d16";
const EXISTING_DESTINATION_OLD_EXTENSION_ID: &str = "acme-mcp-unminted-existing-dest";
const EXISTING_DESTINATION_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "acme-mcp-unminted-existing-dest"
name = "Acme Unminted Existing-Destination MCP"
version = "0.1.0"
description = "Unminted registered MCP whose minted destination already exists (existing-destination migration fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://127.0.0.1:9/mcp-unminted-existing-dest"
"#;
const EXISTING_DESTINATION_MARKER_MANIFEST_TOML: &str = "# pre-existing destination manifest — simulates a completed prior migration attempt or a \
     genuine id collision, must not be clobbered\n";

/// #5970 review (Fix 2): `migrate_registered_id`'s destination-already-exists
/// branch (`fs.stat(&destination_manifest)` returns `Ok`) skips `copy_tree`
/// and the destination-manifest write, but every step below it still runs
/// unconditionally: installation-store rekeying onto the new id, deletion of
/// the OLD manifest/installation rows, and `fs.delete(&source)`. Pins the
/// real current behavior: the pre-existing destination content survives
/// byte-for-byte untouched, the OLD source directory is still removed, and
/// the installation store is still rekeyed onto the new (minted) id.
#[tokio::test]
async fn migrate_unminted_id_with_existing_destination() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");

    let default_tenant =
        TenantId::from_trusted(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string());
    let owner = UserId::new(EXISTING_DESTINATION_OWNER_USER_ID).expect("valid owner id");

    let old_dir = storage_root
        .join("system/extensions/registered")
        .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
        .join(EXISTING_DESTINATION_OWNER_USER_ID)
        .join(EXISTING_DESTINATION_OLD_EXTENSION_ID);
    std::fs::create_dir_all(&old_dir).expect("old descriptor dir");
    std::fs::write(
        old_dir.join("manifest.toml"),
        EXISTING_DESTINATION_MANIFEST_TOML,
    )
    .expect("write old manifest");

    let installation_store: Arc<dyn ExtensionInstallationStore> =
        Arc::new(InMemoryExtensionInstallationStore::default());
    let (new_id, _) = seed_registered_installation(
        &installation_store,
        EXISTING_DESTINATION_MANIFEST_TOML,
        &default_tenant,
        &owner,
        EXISTING_DESTINATION_OLD_EXTENSION_ID,
        None,
    )
    .await;
    let old_installation_id = ExtensionInstallationId::new(EXISTING_DESTINATION_OLD_EXTENSION_ID)
        .expect("valid installation id");
    let old_extension_id =
        ExtensionId::new(EXISTING_DESTINATION_OLD_EXTENSION_ID).expect("valid extension id");

    // Pre-populate the DESTINATION descriptor as already present — simulating
    // either a completed prior migration attempt (partial-failure retry) or a
    // genuine id collision — before running the migration.
    let destination_dir = storage_root
        .join("system/extensions/registered")
        .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
        .join(EXISTING_DESTINATION_OWNER_USER_ID)
        .join(new_id.as_str());
    std::fs::create_dir_all(&destination_dir).expect("destination descriptor dir");
    std::fs::write(
        destination_dir.join("manifest.toml"),
        EXISTING_DESTINATION_MARKER_MANIFEST_TOML,
    )
    .expect("write pre-existing destination manifest");

    let filesystem = mounted_local_filesystem(&storage_root);
    migrate_unminted_registered_ids(&filesystem, &installation_store)
        .await
        .expect("migration over an already-present destination must not fail");

    assert_eq!(
        std::fs::read_to_string(destination_dir.join("manifest.toml"))
            .expect("read destination manifest"),
        EXISTING_DESTINATION_MARKER_MANIFEST_TOML,
        "the pre-existing destination manifest must be preserved untouched, not overwritten by \
         copy_tree or a re-derived new_toml write"
    );

    assert!(
        !old_dir.exists(),
        "the source directory must still be removed even when the destination already existed"
    );

    assert!(
        installation_store
            .get_installation(&old_installation_id)
            .await
            .expect("store read")
            .is_none(),
        "the OLD installation row must be removed even when the destination already existed"
    );
    assert!(
        installation_store
            .get_manifest(&old_extension_id)
            .await
            .expect("store read")
            .is_none(),
        "the OLD manifest record must be removed even when the destination already existed"
    );

    let new_installation_id =
        ExtensionInstallationId::new(new_id.as_str()).expect("valid installation id");
    assert!(
        installation_store
            .get_installation(&new_installation_id)
            .await
            .expect("store read")
            .is_some(),
        "the installation store must still be rekeyed onto the new (minted) id even when the \
         filesystem destination already existed"
    );
    assert!(
        installation_store
            .get_manifest(&new_id)
            .await
            .expect("store read")
            .is_some(),
        "the manifest record must still be rekeyed onto the new (minted) id even when the \
         filesystem destination already existed"
    );
}
