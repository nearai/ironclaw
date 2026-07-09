//! MCP-registration spec test #6 (boot-order restore), RED for T1
//! (docs/plans/2026-07-08-mcp-reg-t1-plan.md). Sibling test file (module-split
//! mandate: overlay/composition logic — and its tests — may not land in the
//! 5505-line `extension_lifecycle.rs`), wired the same way
//! `extension_lifecycle_capabilities_auth_tests.rs` is
//! (`extension_host/mod.rs`'s `#[cfg(test)] pub(crate) mod ...;`).
//!
//! SCOPE LIMIT (plan risk 4): capability publication is tenant-global today
//! (`active_model_visible_capabilities` filters by the global installation
//! store, not by owner), pre-existing and out of scope for T1. This file
//! asserts only that restore publishes the owner's registered extension after
//! a reboot-like rebuild — it does NOT assert cross-actor publication
//! isolation, which T1 does not provide.

use std::sync::Arc;

use ironclaw_extensions::{
    ExtensionActivationState, ExtensionInstallation, ExtensionInstallationId,
    ExtensionInstallationStore, ExtensionLifecycleService, ExtensionManifestRecord,
    ExtensionManifestRef, ExtensionRegistry, InMemoryExtensionInstallationStore, ManifestHash,
    ManifestSource, SharedExtensionRegistry,
};
use ironclaw_filesystem::{LocalFilesystem, RootFilesystem};
use ironclaw_host_api::{ExtensionId, HostPath, UserId, VirtualPath, sha256_digest_token};
use ironclaw_trust::{AdminConfig, HostTrustPolicy, InvalidationBus};
use tokio::sync::Mutex;

use crate::extension_host::available_extensions::AvailableExtensionCatalog;
use crate::extension_host::extension_lifecycle::{
    ActiveExtensionPublisher, restore_extension_lifecycle_state,
};

const OWNER_USER_ID: &str = "3eee560a-7fe5-474c-965a-67cb69df3d04";
const REGISTERED_EXTENSION_ID: &str = "acme-mcp-boot";
// A user-registered hosted-MCP server discovers its tools at runtime, so its
// descriptor declares zero static `[[capabilities]]` — the shape only
// `ManifestSource::UserRegistered` + an MCP runtime is allowed to parse
// (`ironclaw_extensions::v2::ExtensionManifestV2::from_raw`).
const REGISTERED_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "acme-mcp-boot"
name = "Acme Boot MCP"
version = "0.1.0"
description = "User-registered hosted MCP server (T1 boot-order fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://127.0.0.1:9/mcp"
"#;

/// Boot-order restore: an owner-registered extension's installation must
/// survive a reboot (fresh in-memory `ExtensionLifecycleService` +
/// `ActiveExtensionPublisher` over the SAME durable filesystem +
/// installation store) even though the static `AvailableExtensionCatalog`
/// never contains `UserRegistered` packages (T1's fix for the boot-leak
/// blocker in the plan). Today `restore_extension_lifecycle_state` has no
/// registered-store fallback on `catalog.resolve()` miss, so this fails.
#[tokio::test]
async fn restore_publishes_owner_registered_extension_without_static_catalog_entry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");

    let mut local_filesystem = LocalFilesystem::new();
    local_filesystem
        .mount_local(
            VirtualPath::new("/system/extensions").expect("valid virtual path"),
            HostPath::from_path_buf(storage_root.join("system/extensions")),
        )
        .expect("mount system extensions");
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(local_filesystem);

    // Seed: write the registered manifest at T1's owner-scoped path convention
    // directly onto disk (`RegisteredExtensionStore::put()` doesn't exist yet)
    // and durably persist the installation the same way an owner-aware
    // `install()` would (that doesn't exist yet either).
    let owner_dir = storage_root
        .join("system/extensions/registered")
        .join(OWNER_USER_ID)
        .join(REGISTERED_EXTENSION_ID);
    std::fs::create_dir_all(&owner_dir).expect("registered manifest dir");
    std::fs::write(owner_dir.join("manifest.toml"), REGISTERED_MANIFEST_TOML)
        .expect("write registered manifest");

    let host_ports = ironclaw_host_runtime::default_host_port_catalog().expect("host port catalog");
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().expect("host API contracts");
    // A real owner-aware install persists the manifest hash on BOTH the stored
    // manifest record and the installation record (`prepare_install`), and the
    // installation store cross-validates the two. Seeding it here keeps them
    // consistent and lets restore's `validate_restored_manifest_hash` match on
    // the clean path. Seeding `None` would instead force restore through
    // `migrate_host_bundled_manifest_hash`, which is HostBundled-only and
    // aborts for a `UserRegistered` source — unrelated to what this test pins
    // (registered-store fallback + publish).
    let manifest_hash = ManifestHash::new(sha256_digest_token(REGISTERED_MANIFEST_TOML.as_bytes()))
        .expect("valid manifest hash");
    let manifest_record = ExtensionManifestRecord::from_toml_with_contracts(
        REGISTERED_MANIFEST_TOML,
        // Real provenance: a user-registered descriptor. This is the same
        // source the boot-time restore fallback re-parses the on-disk manifest
        // under, so the seeded record matches what restore reconstructs.
        ManifestSource::UserRegistered {
            owner: UserId::new(OWNER_USER_ID).expect("valid owner id"),
        },
        &host_ports,
        Some(manifest_hash.clone()),
        &contracts,
    )
    .expect("registered manifest record");
    let extension_id = ExtensionId::new(REGISTERED_EXTENSION_ID).expect("valid extension id");
    let installation = ExtensionInstallation::new(
        ExtensionInstallationId::new(REGISTERED_EXTENSION_ID).expect("valid installation id"),
        extension_id.clone(),
        ExtensionActivationState::Enabled,
        ExtensionManifestRef::new(extension_id.clone(), Some(manifest_hash)),
        Vec::new(),
        chrono::Utc::now(),
    )
    .expect("owner-registered installation");
    let installation_store: Arc<dyn ExtensionInstallationStore> =
        Arc::new(InMemoryExtensionInstallationStore::default());
    installation_store
        .upsert_manifest(manifest_record)
        .await
        .expect("seed registered manifest record");
    installation_store
        .upsert_installation(installation)
        .await
        .expect("seed owner-registered installation");

    // "Reboot": fresh, empty in-memory lifecycle service + active registry.
    // The static catalog never contains `UserRegistered` packages (T1's fix
    // for the boot-leak blocker), so restore's ONLY path to this owner's
    // installation is a registered-store fallback on `catalog.resolve()` miss.
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
        "T1: restore must fall back to the registered store when the static catalog has no \
         entry for an owner-registered extension (RED until RegisteredExtensionStore lands)",
    );

    assert!(
        active_registry
            .snapshot()
            .get_extension(&extension_id)
            .is_some(),
        "owner-registered extension's capability must be published after restore"
    );
}

/// Safety invariant the registered-store's owner-scoped path convention
/// (`/system/extensions/registered/<owner>/<id>/manifest.toml`, T1) relies
/// on: `UserId`'s own validation rejects any value that could escape that
/// path prefix, so composing a `VirtualPath` from a valid `UserId` can never
/// traverse out of the owner's directory. Passes today (pins an existing
/// invariant, not new T1 behavior).
#[test]
fn owner_user_id_rejects_path_traversal_segments_for_registered_store_paths() {
    for unsafe_owner in ["..", ".", "../../etc", "a/b", "a\\b", ""] {
        assert!(
            UserId::new(unsafe_owner).is_err(),
            "UserId::new({unsafe_owner:?}) must reject path-unsafe segments"
        );
    }

    let owner = UserId::new(OWNER_USER_ID).expect("valid owner id");
    let path = VirtualPath::new(format!(
        "/system/extensions/registered/{}/{REGISTERED_EXTENSION_ID}/manifest.toml",
        owner.as_str()
    ))
    .expect("owner-scoped registered manifest path is valid");
    assert_eq!(
        path.as_str(),
        format!(
            "/system/extensions/registered/{OWNER_USER_ID}/{REGISTERED_EXTENSION_ID}/manifest.toml"
        )
    );
}
