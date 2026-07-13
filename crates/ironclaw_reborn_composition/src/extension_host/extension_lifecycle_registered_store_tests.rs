//! MCP-registration spec test #6 (boot-order restore): an owner-registered
//! extension's installation survives a reboot-like rebuild
//! (`restore_extension_lifecycle_state` falls back to the registered-store
//! overlay on a catalog miss). Sibling test file (module-split mandate:
//! overlay/composition logic — and its tests — may not land in the
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

use ironclaw_extensions::{ExtensionInstallationStore, InMemoryExtensionInstallationStore};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{InvocationId, ResourceScope, TenantId, UserId, VirtualPath};
use ironclaw_product_workflow::{LifecyclePackageKind, LifecyclePackageRef};

use crate::extension_host::available_extensions::{AssetLoading, AvailableExtensionCatalog};
use crate::extension_host::extension_lifecycle::restore_extension_lifecycle_state;
use crate::extension_host::registered_extension_store::{
    RegisteredExtensionStore, migrate_legacy_owner_layout,
};
use crate::extension_host::registered_test_support::{
    fresh_boot_fixture, mounted_local_filesystem, seed_registered_installation,
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
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(mounted_local_filesystem(&storage_root));

    // Seed: write the registered manifest at T1's owner-scoped path convention
    // directly onto disk (`RegisteredExtensionStore::put()` doesn't exist yet)
    // and durably persist the installation the same way an owner-aware
    // `install()` would (that doesn't exist yet either).
    let owner_dir = storage_root
        .join("system/extensions/registered")
        .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
        .join(OWNER_USER_ID)
        .join(REGISTERED_EXTENSION_ID);
    std::fs::create_dir_all(&owner_dir).expect("registered manifest dir");
    std::fs::write(owner_dir.join("manifest.toml"), REGISTERED_MANIFEST_TOML)
        .expect("write registered manifest");

    // A real owner-aware install persists the manifest hash on BOTH the stored
    // manifest record and the installation record (`prepare_install`), and the
    // installation store cross-validates the two. Seeding it here keeps them
    // consistent and lets restore's `validate_restored_manifest_hash` match on
    // the clean path. Seeding `None` would instead force restore through
    // `migrate_host_bundled_manifest_hash`, which is HostBundled-only and
    // aborts for a `UserRegistered` source — unrelated to what this test pins
    // (registered-store fallback + publish).
    let installation_store: Arc<dyn ExtensionInstallationStore> =
        Arc::new(InMemoryExtensionInstallationStore::default());
    let (extension_id, _) = seed_registered_installation(
        &installation_store,
        REGISTERED_MANIFEST_TOML,
        &TenantId::from_trusted(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string()),
        &UserId::new(OWNER_USER_ID).expect("valid owner id"),
        REGISTERED_EXTENSION_ID,
        None,
    )
    .await;

    // "Reboot": fresh, empty in-memory lifecycle service + active registry.
    // The static catalog never contains `UserRegistered` packages (T1's fix
    // for the boot-leak blocker), so restore's ONLY path to this owner's
    // installation is a registered-store fallback on `catalog.resolve()` miss.
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
        "restore must fall back to the registered store when the static catalog has no \
         entry for an owner-registered extension",
    );

    assert!(
        boot.active_registry
            .snapshot()
            .get_extension(&extension_id)
            .is_some(),
        "owner-registered extension's capability must be published after restore"
    );
}

const OTHER_OWNER_USER_ID: &str = "b2222222-7fe5-474c-965a-67cb69df3d05";
const OWNER_A_GOOD_EXTENSION_ID: &str = "acme-mcp-good";
const OWNER_A_CORRUPT_EXTENSION_ID: &str = "acme-mcp-corrupt";
const OWNER_B_EXTENSION_ID: &str = "widgets-mcp";

const OWNER_A_GOOD_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "acme-mcp-good"
name = "Acme Good MCP"
version = "0.1.0"
description = "User-registered hosted MCP server (T1 blast-radius fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://127.0.0.1:9/mcp"
"#;

const OWNER_B_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "widgets-mcp"
name = "Widgets MCP"
version = "0.1.0"
description = "User-registered hosted MCP server (T1 blast-radius fixture, owner B)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://127.0.0.1:9/mcp"
"#;

/// Not valid TOML at all (unterminated table header) — this is what a
/// corrupted or half-written `manifest.toml` looks like on disk. The bug this
/// pins is not "invalid manifest content" (that's rejected everywhere, by
/// design) but "one owner's corrupt descriptor takes down every OTHER
/// owner's listing and boot restore".
const CORRUPT_MANIFEST_TOML: &str = "[runtime\nkind = \"mcp\"\n";

/// T1 amend (docs/plans/2026-07-08-mcp-reg-t3-plan.md, "Folds into
/// already-shipped slices" / AC3): `load_filesystem_packages`
/// (`available_extensions.rs`) used to `?`-propagate on the first
/// unparseable `manifest.toml` in a directory scan, hard-failing the whole
/// listing. Once T3 makes `/system/extensions/registered/<owner>/` end-user
/// writable, one corrupt descriptor under owner A is a cross-tenant DoS:
/// `RegisteredExtensionStore::list_for_scope` goes through that loop, and it
/// backs both live scoped reads and `resolve_registered_for_owner`, the
/// row-owner-keyed boot-restore fallback. This pins the real blast radius,
/// not just "owner A's own listing survives its own corrupt entry":
///   (i)  owner A's OTHER registered entries still list despite the corrupt
///        sibling, and
///   (ii) owner B — a wholly unrelated owner — still gets a successful boot
///        restore (`restore_extension_lifecycle_state` /
///        `resolve_registered_for_owner`) with their extension published,
///        even though owner A's directory contains a corrupt manifest.
/// Pins the skip-and-log fix: the corrupt entry is skipped rather than
/// propagating a `ProductWorkflowError` for the whole directory.
#[tokio::test]
async fn corrupt_manifest_under_one_owner_does_not_break_other_owners_listing_or_restore() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(mounted_local_filesystem(&storage_root));

    let owner_a = UserId::new(OWNER_USER_ID).expect("valid owner id");
    let owner_b = UserId::new(OTHER_OWNER_USER_ID).expect("valid owner id");

    // Seed owner A: one healthy descriptor + one corrupt descriptor sitting
    // right next to it in the same owner-scoped directory scan.
    let owner_a_good_dir = storage_root
        .join("system/extensions/registered")
        .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
        .join(OWNER_USER_ID)
        .join(OWNER_A_GOOD_EXTENSION_ID);
    std::fs::create_dir_all(&owner_a_good_dir).expect("owner A good manifest dir");
    std::fs::write(
        owner_a_good_dir.join("manifest.toml"),
        OWNER_A_GOOD_MANIFEST_TOML,
    )
    .expect("write owner A good manifest");

    let owner_a_corrupt_dir = storage_root
        .join("system/extensions/registered")
        .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
        .join(OWNER_USER_ID)
        .join(OWNER_A_CORRUPT_EXTENSION_ID);
    std::fs::create_dir_all(&owner_a_corrupt_dir).expect("owner A corrupt manifest dir");
    std::fs::write(
        owner_a_corrupt_dir.join("manifest.toml"),
        CORRUPT_MANIFEST_TOML,
    )
    .expect("write owner A corrupt manifest");

    // Seed owner B: one healthy, wholly unrelated descriptor.
    let owner_b_dir = storage_root
        .join("system/extensions/registered")
        .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
        .join(OTHER_OWNER_USER_ID)
        .join(OWNER_B_EXTENSION_ID);
    std::fs::create_dir_all(&owner_b_dir).expect("owner B manifest dir");
    std::fs::write(owner_b_dir.join("manifest.toml"), OWNER_B_MANIFEST_TOML)
        .expect("write owner B manifest");

    // ── (i) owner A's OTHER registered entries still list ───────────────────
    let owner_a_scope = ResourceScope::local_default(owner_a.clone(), InvocationId::new())
        .expect("default tenant owner scope");
    let owner_a_packages = RegisteredExtensionStore::list_for_scope(
        filesystem.as_ref(),
        &owner_a_scope,
        AssetLoading::Inline,
    )
    .await
    .expect(
        "owner A's listing must skip the corrupt sibling manifest and return the healthy \
             entry, not propagate an error for the whole directory",
    );
    assert_eq!(
        owner_a_packages.len(),
        1,
        "owner A's listing must contain exactly the healthy entry, corrupt sibling skipped"
    );
    assert_eq!(
        owner_a_packages[0].package_ref,
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, OWNER_A_GOOD_EXTENSION_ID)
            .expect("valid package ref"),
        "owner A's surviving entry must be the healthy one"
    );

    // ── (ii) owner B's boot restore is unaffected by owner A's corruption ──
    let installation_store: Arc<dyn ExtensionInstallationStore> =
        Arc::new(InMemoryExtensionInstallationStore::default());
    let (owner_b_extension_id, _) = seed_registered_installation(
        &installation_store,
        OWNER_B_MANIFEST_TOML,
        &TenantId::from_trusted(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string()),
        &owner_b,
        OWNER_B_EXTENSION_ID,
        None,
    )
    .await;

    // "Reboot": fresh, empty in-memory lifecycle service + active registry,
    // static catalog empty (T1's boot-leak fix), so restore's only path to
    // owner B's installation is the any-owner registered-store fallback —
    // the exact fallback whose directory scan walks over owner A's corrupt
    // manifest too.
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
        "owner B's boot restore must succeed even though owner A's registered directory (also \
         walked by the any-owner restore fallback) contains a corrupt manifest.toml",
    );

    assert!(
        boot.active_registry
            .snapshot()
            .get_extension(&owner_b_extension_id)
            .is_some(),
        "owner B's registered extension must be published after restore, unaffected by owner \
         A's corrupt sibling manifest"
    );
}

/// Legacy-fs migration (pre-tenant layout): descriptors written by pre-tenant
/// builds live at `/system/extensions/registered/<owner>/<id>/manifest.toml`
/// (no tenant segment). The tenant-scoped walker (`list_for_scope`)
/// cannot see that layout, so without migration a pre-tenant
/// registration silently vanishes from listing AND from boot restore. Boot
/// restore must migrate the legacy tree into the local default tenant —
/// mirroring the wire-format serde default in
/// `extension_installation_store.rs` — then restore through the migrated
/// path.
#[tokio::test]
async fn restore_migrates_legacy_owner_only_layout_into_default_tenant() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(mounted_local_filesystem(&storage_root));

    // Seed the PRE-TENANT layout: no tenant segment between `registered` and
    // the owner directory.
    let legacy_dir = storage_root
        .join("system/extensions/registered")
        .join(OWNER_USER_ID)
        .join(REGISTERED_EXTENSION_ID);
    std::fs::create_dir_all(&legacy_dir).expect("legacy registered manifest dir");
    std::fs::write(legacy_dir.join("manifest.toml"), REGISTERED_MANIFEST_TOML)
        .expect("write legacy registered manifest");

    let installation_store: Arc<dyn ExtensionInstallationStore> =
        Arc::new(InMemoryExtensionInstallationStore::default());
    let (extension_id, _) = seed_registered_installation(
        &installation_store,
        REGISTERED_MANIFEST_TOML,
        &TenantId::from_trusted(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string()),
        &UserId::new(OWNER_USER_ID).expect("valid owner id"),
        REGISTERED_EXTENSION_ID,
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
    .expect("restore must migrate the legacy owner-only layout, then restore through it");

    assert!(
        boot.active_registry
            .snapshot()
            .get_extension(&extension_id)
            .is_some(),
        "pre-tenant registered extension must be published after a migrating restore"
    );

    // On-disk layout must be the tenant-scoped one afterwards…
    let migrated_manifest = storage_root
        .join("system/extensions/registered")
        .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
        .join(OWNER_USER_ID)
        .join(REGISTERED_EXTENSION_ID)
        .join("manifest.toml");
    assert!(
        migrated_manifest.is_file(),
        "legacy descriptor must be moved under the local default tenant"
    );
    // …and the legacy owner-only tree must be gone, so nothing re-probes or
    // re-migrates it on the next boot.
    assert!(
        !storage_root
            .join("system/extensions/registered")
            .join(OWNER_USER_ID)
            .exists(),
        "legacy owner-only directory must be removed after migration"
    );

    // The scoped reader (live search/install path) must see the migrated
    // registration too.
    let owner_scope = ResourceScope::local_default(
        UserId::new(OWNER_USER_ID).expect("valid owner id"),
        InvocationId::new(),
    )
    .expect("default tenant owner scope");
    let packages = RegisteredExtensionStore::list_for_scope(
        filesystem.as_ref(),
        &owner_scope,
        AssetLoading::Inline,
    )
    .await
    .expect("scoped listing after migration");
    assert_eq!(
        packages.len(),
        1,
        "migrated registration must be visible to its owner's scoped listing"
    );
}

const NESTED_ASSETS_EXTENSION_ID: &str = "acme-mcp-nested";
const NESTED_ASSETS_OWNER_USER_ID: &str = "5eee560a-7fe5-474c-965a-67cb69df3d06";
const NESTED_ASSETS_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "acme-mcp-nested"
name = "Acme Nested Assets MCP"
version = "0.1.0"
description = "User-registered hosted MCP server with nested docs/schemas assets"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://127.0.0.1:9/mcp"
"#;

/// `migrate_legacy_owner_dir`'s `copy_tree` claims a full recursive copy
/// (docs/schemas alongside `manifest.toml`), but
/// `restore_migrates_legacy_owner_only_layout_into_default_tenant` above only
/// ever seeds a bare `manifest.toml` — it cannot catch a `copy_tree` that
/// silently drops nested files. Seed a legacy layout with files under nested
/// `docs/` and `schemas/` directories and assert all of them land at the
/// tenant-scoped destination, and the legacy dir is fully removed.
#[tokio::test]
async fn restore_migrates_nested_legacy_registered_assets() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(mounted_local_filesystem(&storage_root));

    // Seed the PRE-TENANT layout with nested asset directories alongside the
    // manifest.
    let legacy_dir = storage_root
        .join("system/extensions/registered")
        .join(NESTED_ASSETS_OWNER_USER_ID)
        .join(NESTED_ASSETS_EXTENSION_ID);
    std::fs::create_dir_all(legacy_dir.join("docs")).expect("legacy docs dir");
    std::fs::create_dir_all(legacy_dir.join("schemas")).expect("legacy schemas dir");
    std::fs::write(
        legacy_dir.join("manifest.toml"),
        NESTED_ASSETS_MANIFEST_TOML,
    )
    .expect("write legacy registered manifest");
    std::fs::write(legacy_dir.join("docs").join("setup.md"), "# setup")
        .expect("write legacy nested doc");
    std::fs::write(
        legacy_dir.join("schemas").join("tool.input.json"),
        "{\"type\":\"object\"}",
    )
    .expect("write legacy nested schema");

    let installation_store: Arc<dyn ExtensionInstallationStore> =
        Arc::new(InMemoryExtensionInstallationStore::default());
    seed_registered_installation(
        &installation_store,
        NESTED_ASSETS_MANIFEST_TOML,
        &TenantId::from_trusted(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID.to_string()),
        &UserId::new(NESTED_ASSETS_OWNER_USER_ID).expect("valid owner id"),
        NESTED_ASSETS_EXTENSION_ID,
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
    .expect("restore must migrate the nested legacy assets, then restore through them");

    let migrated_dir = storage_root
        .join("system/extensions/registered")
        .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
        .join(NESTED_ASSETS_OWNER_USER_ID)
        .join(NESTED_ASSETS_EXTENSION_ID);
    assert!(
        migrated_dir.join("manifest.toml").is_file(),
        "legacy manifest must be moved under the local default tenant"
    );
    assert_eq!(
        std::fs::read_to_string(migrated_dir.join("docs").join("setup.md"))
            .expect("read migrated nested doc"),
        "# setup",
        "nested docs/ file must be copied to the tenant-scoped destination"
    );
    assert_eq!(
        std::fs::read_to_string(migrated_dir.join("schemas").join("tool.input.json"))
            .expect("read migrated nested schema"),
        "{\"type\":\"object\"}",
        "nested schemas/ file must be copied to the tenant-scoped destination"
    );
    // The legacy owner-only tree (including its nested asset dirs) must be
    // fully gone, so nothing re-probes or re-migrates it on the next boot.
    assert!(
        !storage_root
            .join("system/extensions/registered")
            .join(NESTED_ASSETS_OWNER_USER_ID)
            .exists(),
        "legacy owner-only directory (with its nested assets) must be removed after migration"
    );
}

const COLLISION_EXTENSION_ID: &str = "acme-mcp-collision";
const COLLISION_OWNER_USER_ID: &str = "6eee560a-7fe5-474c-965a-67cb69df3d07";
const COLLISION_LEGACY_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "acme-mcp-collision"
name = "Acme Collision MCP (legacy)"
version = "0.1.0"
description = "Legacy pre-tenant descriptor, divergent from the tenant-scoped one"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://legacy.example/mcp"
"#;
const COLLISION_TENANT_SCOPED_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "acme-mcp-collision"
name = "Acme Collision MCP (tenant-scoped)"
version = "0.1.0"
description = "Already-migrated tenant-scoped descriptor, must not be clobbered"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://tenant-scoped.example/mcp"
"#;

/// Migration collision: a tenant-scoped descriptor already exists for an id
/// AND a divergent legacy (pre-tenant) copy of the same id also exists on
/// disk. `migrate_legacy_owner_dir` must never clobber the existing
/// tenant-scoped file, and per its documented stance must leave the
/// divergent legacy copy in place (untested until now).
#[tokio::test]
async fn migration_preserves_existing_tenant_scoped_descriptor_and_leaves_divergent_legacy_copy() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    let filesystem = mounted_local_filesystem(&storage_root);

    // Seed the tenant-scoped layout FIRST, with content that differs from
    // the legacy copy below.
    let tenant_scoped_dir = storage_root
        .join("system/extensions/registered")
        .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
        .join(COLLISION_OWNER_USER_ID)
        .join(COLLISION_EXTENSION_ID);
    std::fs::create_dir_all(&tenant_scoped_dir).expect("tenant-scoped manifest dir");
    std::fs::write(
        tenant_scoped_dir.join("manifest.toml"),
        COLLISION_TENANT_SCOPED_MANIFEST_TOML,
    )
    .expect("write tenant-scoped manifest");

    // Seed a DIVERGENT legacy (pre-tenant) copy of the same extension id.
    let legacy_dir = storage_root
        .join("system/extensions/registered")
        .join(COLLISION_OWNER_USER_ID)
        .join(COLLISION_EXTENSION_ID);
    std::fs::create_dir_all(&legacy_dir).expect("legacy manifest dir");
    std::fs::write(
        legacy_dir.join("manifest.toml"),
        COLLISION_LEGACY_MANIFEST_TOML,
    )
    .expect("write legacy manifest");

    migrate_legacy_owner_layout(&filesystem)
        .await
        .expect("migration must succeed even with a colliding tenant-scoped descriptor");

    assert_eq!(
        std::fs::read_to_string(tenant_scoped_dir.join("manifest.toml"))
            .expect("read tenant-scoped manifest after migration"),
        COLLISION_TENANT_SCOPED_MANIFEST_TOML,
        "the existing tenant-scoped descriptor must be byte-unchanged after migration"
    );
    assert_eq!(
        std::fs::read_to_string(legacy_dir.join("manifest.toml"))
            .expect("read legacy manifest after migration"),
        COLLISION_LEGACY_MANIFEST_TOML,
        "the divergent legacy copy must remain on disk after migration, not be deleted or merged"
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
