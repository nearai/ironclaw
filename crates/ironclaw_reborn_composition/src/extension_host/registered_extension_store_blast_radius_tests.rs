//! Review-comment fix on PR #5916: one owner's storage-layer error must not
//! abort `list_all`/boot restore for every other owner. Distinct from
//! `extension_lifecycle_registered_store_tests`'s corrupt-manifest coverage
//! (a per-entry TOML parse failure inside `load_filesystem_packages`) — this
//! pins a directory-level `fs.list_dir` error on one owner's registered root.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionInstallation, ExtensionInstallationId,
    ExtensionInstallationStore, ExtensionLifecycleService, ExtensionManifestRecord,
    ExtensionManifestRef, ExtensionRegistry, InMemoryExtensionInstallationStore, InstallationOwner,
    ManifestHash, ManifestSource, SharedExtensionRegistry,
};
use ironclaw_filesystem::{
    DirEntry, FileStat, FilesystemError, LocalFilesystem, RootFilesystem, VersionedEntry,
};
use ironclaw_host_api::{
    ExtensionId, HostPath, TenantId, UserId, VirtualPath, sha256_digest_token,
};
use ironclaw_product_workflow::{LifecyclePackageKind, LifecyclePackageRef};
use ironclaw_trust::{AdminConfig, HostTrustPolicy, InvalidationBus};
use tokio::sync::Mutex;

use crate::extension_host::available_extensions::AvailableExtensionCatalog;
use crate::extension_host::extension_lifecycle::{
    ActiveExtensionPublisher, restore_extension_lifecycle_state,
};
use crate::extension_host::registered_extension_store::RegisteredExtensionStore;

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
/// fallback's `resolve_any_owner_for_restore` error, not abort the whole boot
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
