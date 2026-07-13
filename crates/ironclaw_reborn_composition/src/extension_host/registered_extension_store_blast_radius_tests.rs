//! Review-comment fix on PR #5916: one owner's storage-layer error must not
//! abort `list_all`/boot restore for every other owner. Distinct from
//! `extension_lifecycle_registered_store_tests`'s corrupt-manifest coverage
//! (a per-entry TOML parse failure inside `load_filesystem_packages`) — this
//! pins a directory-level `fs.list_dir` error on one owner's registered root.

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
    ExtensionId, HostPath, InvocationId, ResourceScope, TenantId, UserId, VirtualPath,
    sha256_digest_token,
};
use ironclaw_product_workflow::{
    LifecyclePackageKind, LifecyclePackageRef, LifecycleProductPayload,
};
use ironclaw_trust::{AdminConfig, HostTrustPolicy, InvalidationBus};
use tokio::sync::Mutex;

use crate::extension_host::available_extensions::{
    AvailableExtensionCatalog, AvailableExtensionPackage,
};
use crate::extension_host::extension_lifecycle::{
    ActiveExtensionPublisher, RebornLocalExtensionManagementPort, restore_extension_lifecycle_state,
};
use crate::extension_host::registered_extension_store::{
    RegisteredExtensionStore, resolve_registered_for_scope,
};

const HEALTHY_OWNER_USER_ID: &str = "c3333333-7fe5-474c-965a-67cb69df3d06";
const BROKEN_OWNER_USER_ID: &str = "d4444444-7fe5-474c-965a-67cb69df3d07";
const HEALTHY_EXTENSION_ID: &str = "healthy-mcp";

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

/// Pins the `list_all` fix: `RegisteredExtensionStore::list_for_owner`'s
/// `fs.list_dir` erroring for one owner (broken owner's directory) must be
/// skipped-and-logged, not `?`-propagated — the healthy owner's packages
/// must still come back. RED before the fix (whole call returned `Err`).
#[tokio::test]
async fn list_all_skips_owner_whose_directory_listing_errors() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");
    seed_registered_manifest(&storage_root, HEALTHY_OWNER_USER_ID, HEALTHY_EXTENSION_ID);
    // The broken owner still needs a directory entry under the registered
    // root (so `list_all`'s top-level scan reports it as a directory and
    // descends into it) — its own contents are irrelevant since `list_dir`
    // on it is intercepted.
    std::fs::create_dir_all(
        storage_root
            .join("system/extensions/registered")
            .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
            .join(BROKEN_OWNER_USER_ID),
    )
    .expect("broken owner dir");

    let mut local_filesystem = LocalFilesystem::new();
    local_filesystem
        .mount_local(
            VirtualPath::new("/system/extensions").expect("valid virtual path"),
            HostPath::from_path_buf(storage_root.join("system/extensions")),
        )
        .expect("mount system extensions");

    let broken_owner_root = VirtualPath::new(format!(
        "/system/extensions/registered/{}/{BROKEN_OWNER_USER_ID}",
        ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID
    ))
    .expect("valid virtual path");
    let filesystem = FailListDirFilesystem {
        inner: local_filesystem,
        fail_path: broken_owner_root,
    };

    let packages = RegisteredExtensionStore::list_all(&filesystem)
        .await
        .expect(
            "list_all must skip the owner whose directory listing errors, not propagate the \
             error for every owner (RED until skip-and-log lands)",
        );
    assert_eq!(
        packages.len(),
        1,
        "the healthy owner's package must still be returned despite the broken owner's \
         directory-listing error"
    );
    assert_eq!(
        packages[0].package_ref,
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, HEALTHY_EXTENSION_ID)
            .expect("valid package ref")
    );
}

/// Pins the `restore_extension_lifecycle_state` fix: an installation whose
/// registered manifest is gone (deleted/corrupted on disk, but still on
/// record in the installation store) must be skipped via the registered-store
/// fallback's (`resolve_registered_for_owner`) miss, not abort the whole boot
/// restore — a second, healthy installation must still restore and publish.
/// RED before the fix (the whole restore returned `Err` on the first broken
/// installation).
#[tokio::test]
async fn restore_continues_past_installation_whose_registered_fallback_errors() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");
    seed_registered_manifest(&storage_root, HEALTHY_OWNER_USER_ID, HEALTHY_EXTENSION_ID);

    let mut local_filesystem = LocalFilesystem::new();
    local_filesystem
        .mount_local(
            VirtualPath::new("/system/extensions").expect("valid virtual path"),
            HostPath::from_path_buf(storage_root.join("system/extensions")),
        )
        .expect("mount system extensions");
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(local_filesystem);

    let host_ports = ironclaw_host_runtime::default_host_port_catalog().expect("host port catalog");
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().expect("host API contracts");

    // Installation #1: on record in the installation store (manifest record
    // seeded, matching a real "registered then reinstalled/deleted" history),
    // but its `manifest.toml` no longer exists anywhere on disk. `catalog.resolve()`
    // misses (static catalog never holds `UserRegistered` packages) and
    // `resolve_any_owner_for_restore` also misses, since no owner directory
    // has this extension id — the missing-manifest scenario this fix targets.
    let missing_extension_id = ExtensionId::new("missing-mcp").expect("valid extension id");
    let missing_manifest_hash = ManifestHash::new(sha256_digest_token(b"missing-mcp-placeholder"))
        .expect("valid manifest hash");
    let missing_manifest_record = ExtensionManifestRecord::from_toml_with_contracts(
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
        ManifestSource::UserRegistered {
            tenant_id: TenantId::from_trusted(
                ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string(),
            ),
            owner: UserId::new(BROKEN_OWNER_USER_ID).expect("valid owner id"),
        },
        &host_ports,
        Some(missing_manifest_hash.clone()),
        &contracts,
    )
    .expect("missing manifest record");
    let missing_installation = ExtensionInstallation::new(
        ExtensionInstallationId::new("missing-mcp").expect("valid installation id"),
        missing_extension_id.clone(),
        ExtensionActivationState::Enabled,
        ExtensionManifestRef::new(missing_extension_id.clone(), Some(missing_manifest_hash)),
        Vec::new(),
        chrono::Utc::now(),
        InstallationOwner::user(UserId::new(BROKEN_OWNER_USER_ID).expect("valid owner id")),
    )
    .expect("missing installation");

    // Installation #2: the healthy, owner-registered extension that must
    // still restore and publish despite installation #1's failure.
    let healthy_manifest_hash =
        ManifestHash::new(sha256_digest_token(HEALTHY_MANIFEST_TOML.as_bytes()))
            .expect("valid manifest hash");
    let healthy_manifest_record = ExtensionManifestRecord::from_toml_with_contracts(
        HEALTHY_MANIFEST_TOML,
        ManifestSource::UserRegistered {
            tenant_id: TenantId::from_trusted(
                ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string(),
            ),
            owner: UserId::new(HEALTHY_OWNER_USER_ID).expect("valid owner id"),
        },
        &host_ports,
        Some(healthy_manifest_hash.clone()),
        &contracts,
    )
    .expect("healthy manifest record");
    let healthy_extension_id = ExtensionId::new(HEALTHY_EXTENSION_ID).expect("valid extension id");
    let healthy_installation = ExtensionInstallation::new(
        ExtensionInstallationId::new(HEALTHY_EXTENSION_ID).expect("valid installation id"),
        healthy_extension_id.clone(),
        ExtensionActivationState::Enabled,
        ExtensionManifestRef::new(healthy_extension_id.clone(), Some(healthy_manifest_hash)),
        Vec::new(),
        chrono::Utc::now(),
        InstallationOwner::user(UserId::new(HEALTHY_OWNER_USER_ID).expect("valid owner id")),
    )
    .expect("healthy owner-registered installation");

    let installation_store: Arc<dyn ExtensionInstallationStore> =
        Arc::new(InMemoryExtensionInstallationStore::default());
    installation_store
        .upsert_manifest(missing_manifest_record)
        .await
        .expect("seed missing manifest record");
    installation_store
        .upsert_installation(missing_installation)
        .await
        .expect("seed missing installation");
    installation_store
        .upsert_manifest(healthy_manifest_record)
        .await
        .expect("seed healthy manifest record");
    installation_store
        .upsert_installation(healthy_installation)
        .await
        .expect("seed healthy installation");

    let empty_catalog = AvailableExtensionCatalog::from_packages(Vec::new());
    let lifecycle_service = Arc::new(Mutex::new(ExtensionLifecycleService::new(
        ExtensionRegistry::new(),
    )));
    let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
    let trust_policy =
        Arc::new(HostTrustPolicy::new(vec![Box::new(AdminConfig::new())]).expect("trust policy"));
    let active_extensions = ActiveExtensionPublisher::new(
        Arc::clone(&active_registry),
        Arc::clone(&trust_policy),
        Arc::new(InvalidationBus::new()),
    );

    restore_extension_lifecycle_state(
        &empty_catalog,
        &filesystem,
        &installation_store,
        &lifecycle_service,
        &active_extensions,
    )
    .await
    .expect(
        "restore must skip an installation whose registered-store fallback errors, not abort \
         the whole boot restore (RED until skip-and-log lands)",
    );

    assert!(
        active_registry
            .snapshot()
            .get_extension(&healthy_extension_id)
            .is_some(),
        "the healthy installation must still restore and publish despite the broken \
         installation's registered-store fallback error"
    );
    assert!(
        active_registry
            .snapshot()
            .get_extension(&missing_extension_id)
            .is_none(),
        "the broken installation must not be published"
    );
}

const OWNER_A_USER_ID: &str = "e5555555-7fe5-474c-965a-67cb69df3d08";
const OWNER_B_USER_ID: &str = "f6666666-7fe5-474c-965a-67cb69df3d09";
const SHARED_EXTENSION_ID: &str = "shared-mcp";

const OWNER_A_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "shared-mcp"
name = "Owner A's Shared MCP"
version = "0.1.0"
description = "Owner A's registration (row-owned)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://owner-a.example/mcp"
"#;

const OWNER_B_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "shared-mcp"
name = "Owner B's Shared MCP"
version = "0.1.0"
description = "Owner B's independent (non-row-owning) registration of the same id"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://owner-b.example/mcp"
"#;

/// Wraps a real `LocalFilesystem`, forcing a FIXED owner iteration order
/// (owner B before owner A) for exactly the registered-tenant root's
/// `list_dir`. Real directory listing order is filesystem/OS-dependent and
/// not something a correct implementation may rely on; this override makes
/// the "wrong owner scanned first" case deterministic for the regression
/// test below instead of depending on it happening to occur locally.
struct OwnerBFirstFilesystem {
    inner: LocalFilesystem,
    tenant_root: VirtualPath,
}

#[async_trait]
impl RootFilesystem for OwnerBFirstFilesystem {
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        let entries = self.inner.list_dir(path).await?;
        if path != &self.tenant_root {
            return Ok(entries);
        }
        let mut ordered = entries;
        ordered.sort_by_key(|entry| entry.name != OWNER_B_USER_ID);
        Ok(ordered)
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

/// Regression for the T2 restore-fallback ownership bug: two owners
/// independently register descriptors under the SAME bare extension id with
/// different manifest content (different runtime URL). The installation row
/// belongs to owner A. An any-owner directory scan (the old
/// `resolve_any_owner_for_restore`, order-dependent) can find owner B's
/// descriptor first and serve IT for owner A's row — publishing the wrong
/// endpoint under A's installation (or, since A's row pins the manifest hash
/// it was installed with, tripping a hash-mismatch that aborts the ENTIRE
/// boot restore even though A's own correct descriptor was available the
/// whole time). The row-owner-keyed fallback must go straight to owner A's
/// shard and never consult owner B's, regardless of directory order.
#[tokio::test]
async fn restore_uses_row_owners_registered_descriptor_not_a_differently_ordered_owner() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");
    seed_registered_manifest(&storage_root, OWNER_A_USER_ID, SHARED_EXTENSION_ID);
    // Overwrite with A's real (row-owning) content; `seed_registered_manifest`
    // writes `HEALTHY_MANIFEST_TOML` by default.
    std::fs::write(
        storage_root
            .join("system/extensions/registered")
            .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
            .join(OWNER_A_USER_ID)
            .join(SHARED_EXTENSION_ID)
            .join("manifest.toml"),
        OWNER_A_MANIFEST_TOML,
    )
    .expect("write owner A's descriptor");
    let owner_b_dir = storage_root
        .join("system/extensions/registered")
        .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
        .join(OWNER_B_USER_ID)
        .join(SHARED_EXTENSION_ID);
    std::fs::create_dir_all(&owner_b_dir).expect("owner B descriptor dir");
    std::fs::write(owner_b_dir.join("manifest.toml"), OWNER_B_MANIFEST_TOML)
        .expect("write owner B's descriptor");

    let mut local_filesystem = LocalFilesystem::new();
    local_filesystem
        .mount_local(
            VirtualPath::new("/system/extensions").expect("valid virtual path"),
            HostPath::from_path_buf(storage_root.join("system/extensions")),
        )
        .expect("mount system extensions");
    let tenant_root = VirtualPath::new(format!(
        "/system/extensions/registered/{}",
        ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID
    ))
    .expect("valid virtual path");
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(OwnerBFirstFilesystem {
        inner: local_filesystem,
        tenant_root,
    });

    let host_ports = ironclaw_host_runtime::default_host_port_catalog().expect("host port catalog");
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().expect("host API contracts");

    let owner_a_manifest_hash =
        ManifestHash::new(sha256_digest_token(OWNER_A_MANIFEST_TOML.as_bytes()))
            .expect("valid manifest hash");
    let manifest_record = ExtensionManifestRecord::from_toml_with_contracts(
        OWNER_A_MANIFEST_TOML,
        ManifestSource::UserRegistered {
            tenant_id: TenantId::from_trusted(
                ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string(),
            ),
            owner: UserId::new(OWNER_A_USER_ID).expect("valid owner id"),
        },
        &host_ports,
        Some(owner_a_manifest_hash.clone()),
        &contracts,
    )
    .expect("owner A manifest record");
    let extension_id = ExtensionId::new(SHARED_EXTENSION_ID).expect("valid extension id");
    let installation = ExtensionInstallation::new(
        ExtensionInstallationId::new(SHARED_EXTENSION_ID).expect("valid installation id"),
        extension_id.clone(),
        ExtensionActivationState::Enabled,
        ExtensionManifestRef::new(extension_id.clone(), Some(owner_a_manifest_hash)),
        Vec::new(),
        chrono::Utc::now(),
        InstallationOwner::user(UserId::new(OWNER_A_USER_ID).expect("valid owner id")),
    )
    .expect("owner A installation");

    let installation_store: Arc<dyn ExtensionInstallationStore> =
        Arc::new(InMemoryExtensionInstallationStore::default());
    installation_store
        .upsert_manifest(manifest_record)
        .await
        .expect("seed owner A manifest record");
    installation_store
        .upsert_installation(installation)
        .await
        .expect("seed owner A installation");

    let empty_catalog = AvailableExtensionCatalog::from_packages(Vec::new());
    let lifecycle_service = Arc::new(Mutex::new(ExtensionLifecycleService::new(
        ExtensionRegistry::new(),
    )));
    let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
    let trust_policy =
        Arc::new(HostTrustPolicy::new(vec![Box::new(AdminConfig::new())]).expect("trust policy"));
    let active_extensions = ActiveExtensionPublisher::new(
        Arc::clone(&active_registry),
        Arc::clone(&trust_policy),
        Arc::new(InvalidationBus::new()),
    );

    restore_extension_lifecycle_state(
        &empty_catalog,
        &filesystem,
        &installation_store,
        &lifecycle_service,
        &active_extensions,
    )
    .await
    .expect(
        "restore must resolve owner A's own registered descriptor for owner A's row, not \
         owner B's differently-ordered one (RED before the row-owner-keyed fallback lands: \
         the any-owner scan finds owner B first and either serves B's manifest under A's row \
         or trips a manifest-hash mismatch that aborts the whole boot restore)",
    );

    let snapshot = active_registry.snapshot();
    let published = snapshot
        .get_extension(&extension_id)
        .expect("owner A's row must restore and publish");
    let ironclaw_extensions::ExtensionRuntime::Mcp { url, .. } = &published.manifest.runtime else {
        panic!("expected an MCP runtime declaration");
    };
    assert_eq!(
        url.as_deref(),
        Some("http://owner-a.example/mcp"),
        "restore must materialize owner A's own manifest content, never owner B's"
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
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().expect("host port catalog");
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().expect("host API contracts");
    let manifest = ExtensionManifest::parse_with_host_api_contracts(
        CATALOG_MANIFEST_TOML,
        ManifestSource::InstalledLocal,
        &host_ports,
        &contracts,
    )
    .expect("catalog fixture manifest");
    let root = VirtualPath::new("/system/extensions/catalog-mcp").expect("extension root");
    let package = ExtensionPackage::from_manifest_toml(manifest, root, CATALOG_MANIFEST_TOML)
        .expect("catalog fixture package");
    AvailableExtensionPackage {
        package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, "catalog-mcp")
            .expect("catalog fixture ref"),
        manifest_toml: CATALOG_MANIFEST_TOML.to_string(),
        source: ManifestSource::InstalledLocal,
        package,
        surface_kinds: Vec::new(),
        assets: Vec::new(),
    }
}

fn owner_scope(user: &str) -> ResourceScope {
    ResourceScope::local_default(UserId::new(user).expect("valid user"), InvocationId::new())
        .expect("valid local scope")
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
    std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");
    seed_registered_manifest(&storage_root, HEALTHY_OWNER_USER_ID, HEALTHY_EXTENSION_ID);

    let mut local_filesystem = LocalFilesystem::new();
    local_filesystem
        .mount_local(
            VirtualPath::new("/system/extensions").expect("valid virtual path"),
            HostPath::from_path_buf(storage_root.join("system/extensions")),
        )
        .expect("mount system extensions");
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

/// Pins the installed-summaries blast radius (T2 review item): one entry
/// whose registered-store resolution ERRORS (not merely misses) must be
/// logged-and-skipped, leaving every other installed summary intact — the
/// old `let Ok(...) else continue` swallowed the error indistinguishably.
#[tokio::test]
async fn list_installed_survives_one_entry_whose_registered_resolution_errors() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");
    seed_registered_manifest(&storage_root, HEALTHY_OWNER_USER_ID, HEALTHY_EXTENSION_ID);

    let mut local_filesystem = LocalFilesystem::new();
    local_filesystem
        .mount_local(
            VirtualPath::new("/system/extensions").expect("valid virtual path"),
            HostPath::from_path_buf(storage_root.join("system/extensions")),
        )
        .expect("mount system extensions");
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

    let response = port
        .list_installed(&owner_scope(HEALTHY_OWNER_USER_ID))
        .await
        .expect(
            "one entry's registered-store read error must not kill the whole installed listing",
        );
    let Some(LifecycleProductPayload::ExtensionList { extensions, count }) = response.payload
    else {
        panic!("expected extension list payload");
    };
    assert_eq!(count, 1, "the catalog-backed summary must survive");
    assert_eq!(extensions[0].summary.package_ref.id.as_str(), "catalog-mcp");
}
