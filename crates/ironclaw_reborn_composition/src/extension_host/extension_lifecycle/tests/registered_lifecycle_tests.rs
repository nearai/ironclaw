//! Owner-registered live-path tests, split out of the parent test module
//! (`extension_lifecycle.rs`'s `mod tests`), whose shared fixtures it reuses
//! via `super::*` — same pattern as the sibling `private_install_tests.rs`.
//! Covers the `registered_lifecycle` slice: row-authoritative owner-scope
//! resolution, row-provenance-wins resolution over the shared catalog,
//! owner/tenant isolation across list/project/activate/remove, and the
//! per-owner-batched boot-restore fallback's registered arm.

use super::*;

/// The effective owner scope is ROW-authoritative on the user axis: a
/// stale manifest whose `UserRegistered.owner` disagrees with the
/// installation row's singleton member must not re-point the install at
/// the manifest's user. Tenant still comes from the manifest provenance.
#[test]
fn effective_owner_scope_prefers_row_owner_over_stale_manifest_owner() {
    let row_owner = UserId::new("row-owner").expect("valid user");
    let stale_manifest_owner = UserId::new("stale-manifest-owner").expect("valid user");
    let tenant = TenantId::new("tenant-a").expect("valid tenant");
    let source = ManifestSource::UserRegistered {
        tenant_id: tenant.clone(),
        owner: stale_manifest_owner,
    };
    let extension_id = ExtensionId::new("fixture").expect("valid extension id");
    let installation = ExtensionInstallation::new(
        ExtensionInstallationId::new("fixture").expect("valid installation"),
        extension_id.clone(),
        ExtensionActivationState::Installed,
        ExtensionManifestRef::new(extension_id, None),
        Vec::new(),
        chrono::Utc::now(),
        InstallationOwner::user(row_owner.clone()),
    )
    .expect("registered installation");

    assert_eq!(
        effective_owner_scope(&installation, &source),
        Some((tenant, row_owner)),
        "row owner must win over the stale manifest owner"
    );
    assert_eq!(
        effective_owner_scope(&installation, &ManifestSource::HostBundled),
        None,
        "non-registered sources have no effective owner scope"
    );
}

pub(super) const REGISTERED_ISOLATION_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "acme-mcp-registered"
name = "Acme Registered MCP"
version = "0.1.0"
description = "User-registered hosted MCP server (owner isolation fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "http://127.0.0.1:9/mcp"
"#;

/// Owner-isolation harness for registered packages: writes the descriptor
/// at the tenant-scoped registered-store path (the overlay's filesystem
/// source) over an EMPTY shared catalog, with `owner_scope.user_id` also
/// wired as the tenant operator — the worst case for the row-stamping
/// rule, since `derive_owner` would map the operator to `Tenant`.
/// `pre_install: true` also seeds the installed row + lifecycle/active
/// registries the way an owner install would.
async fn user_registered_isolation_fixture(
    owner_scope: &ResourceScope,
    pre_install: bool,
) -> (
    tempfile::TempDir,
    Arc<RebornLocalExtensionManagementPort>,
    LifecyclePackageRef,
    Arc<SharedExtensionRegistry>,
    Arc<InMemoryExtensionInstallationStore>,
) {
    let extension_id = ExtensionId::new("acme-mcp-registered").expect("valid extension id");
    let source = ManifestSource::UserRegistered {
        tenant_id: owner_scope.tenant_id.clone(),
        owner: owner_scope.user_id.clone(),
    };

    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    let descriptor_dir = storage_root
        .join("system/extensions/registered")
        .join(owner_scope.tenant_id.as_str())
        .join(owner_scope.user_id.as_str())
        .join("acme-mcp-registered");
    std::fs::create_dir_all(&descriptor_dir).expect("registered descriptor dir");
    std::fs::write(
        descriptor_dir.join("manifest.toml"),
        REGISTERED_ISOLATION_MANIFEST_TOML,
    )
    .expect("write registered descriptor");
    let mut local_filesystem = LocalFilesystem::new();
    local_filesystem
        .mount_local(
            VirtualPath::new("/system/extensions").expect("valid virtual path"),
            HostPath::from_path_buf(storage_root.join("system/extensions")),
        )
        .expect("mount system extensions");

    let mut lifecycle_registry = ExtensionRegistry::new();
    let mut active_registry_initial = ExtensionRegistry::new();
    let installation_store = Arc::new(InMemoryExtensionInstallationStore::default());
    if pre_install {
        let manifest = ExtensionManifest::parse(
            REGISTERED_ISOLATION_MANIFEST_TOML,
            source.clone(),
            &HostPortCatalog::empty(),
        )
        .expect("registered manifest");
        let root =
            VirtualPath::new("/system/extensions/acme-mcp-registered").expect("extension root");
        let package = ExtensionPackage::from_manifest_toml(
            manifest,
            root,
            REGISTERED_ISOLATION_MANIFEST_TOML,
        )
        .expect("registered package");
        lifecycle_registry
            .insert(package.clone())
            .expect("lifecycle package");
        active_registry_initial
            .insert(package)
            .expect("active package");
        let manifest_record = fixture_manifest_record_with_source(
            REGISTERED_ISOLATION_MANIFEST_TOML,
            source.clone(),
            None,
        );
        installation_store
            .upsert_manifest(manifest_record)
            .await
            .expect("seed registered manifest record");
        let installation = ExtensionInstallation::new(
            ExtensionInstallationId::new("acme-mcp-registered").expect("valid installation id"),
            extension_id.clone(),
            ExtensionActivationState::Enabled,
            ExtensionManifestRef::new(extension_id, None),
            Vec::new(),
            chrono::Utc::now(),
            InstallationOwner::user(owner_scope.user_id.clone()),
        )
        .expect("registered installation");
        installation_store
            .upsert_installation(installation)
            .await
            .expect("seed registered installation");
    }
    let active_registry = Arc::new(SharedExtensionRegistry::new(active_registry_initial));

    let port = Arc::new(RebornLocalExtensionManagementPort::new(
        Arc::new(local_filesystem),
        AvailableExtensionCatalog::from_packages(Vec::new()),
        installation_store.clone(),
        Arc::new(Mutex::new(ExtensionLifecycleService::new(
            lifecycle_registry,
        ))),
        test_active_extension_publisher(
            Arc::clone(&active_registry),
            test_extension_trust_policy(),
        ),
        None,
        // The registering owner IS the tenant operator: the row must
        // still be the singleton owner, never Tenant.
        owner_scope.user_id.clone(),
    ));
    let package_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "acme-mcp-registered")
            .expect("valid ref");
    (dir, port, package_ref, active_registry, installation_store)
}

/// Item B fixture: a registered manifest that declares a required runtime
/// credential, so `activation_credential_requirements` returns a non-empty,
/// real requirement list instead of an empty one — the shape the masked
/// "is not installed" denial must never be distinguishable from.
const REGISTERED_CREDENTIALED_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "acme-mcp-credentialed"
name = "Acme Credentialed MCP"
version = "0.1.0"
description = "User-registered hosted MCP server requiring credentials (Item B fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "https://mcp.acme.example/mcp"

[[capabilities]]
id = "acme-mcp-credentialed.acme-mcp-credentialed-tool"
description = "Acme credentialed MCP tool."
effects = ["dispatch_capability", "network", "use_secret"]
runtime_credentials = [
  { handle = "mcp_acme_access_token", source = { type = "product_auth_account", provider = "acme", setup = { kind = "oauth", scopes = [] } }, audience = { scheme = "https", host_pattern = "mcp.acme.example" }, target = { type = "header", name = "authorization", prefix = "Bearer " } },
]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/acme/acme-mcp-credentialed-tool.input.v1.json"
output_schema_ref = "schemas/acme/acme-mcp-credentialed-tool.output.v1.json"
prompt_doc_ref = "prompts/acme/acme-mcp-credentialed-tool.md"
"#;

/// The store-side manifest record for the Item B fixture. Statically
/// declared `[[capabilities]]` are rejected by the contract-validated parse
/// (`from_toml_with_contracts`) for any non-first-party source — real
/// registered/hosted MCP installs get their capabilities from runtime
/// discovery, not the stored manifest — so the persisted record stays
/// capability-free while the in-memory lifecycle package below (built via
/// the lenient parse, mirroring what discovery would have produced) is the
/// one that actually carries the credentialed capability.
const REGISTERED_CREDENTIALED_STORE_MANIFEST_TOML: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "acme-mcp-credentialed"
name = "Acme Credentialed MCP"
version = "0.1.0"
description = "User-registered hosted MCP server requiring credentials (Item B fixture)"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "https://mcp.acme.example/mcp"
"#;

/// Same shape as `user_registered_isolation_fixture`, but seeded with
/// `REGISTERED_CREDENTIALED_MANIFEST_TOML` so the install actually declares
/// a runtime credential requirement (Item B).
async fn credentialed_registered_isolation_fixture(
    owner_scope: &ResourceScope,
) -> (
    tempfile::TempDir,
    Arc<RebornLocalExtensionManagementPort>,
    LifecyclePackageRef,
) {
    let extension_id = ExtensionId::new("acme-mcp-credentialed").expect("valid extension id");
    let source = ManifestSource::UserRegistered {
        tenant_id: owner_scope.tenant_id.clone(),
        owner: owner_scope.user_id.clone(),
    };

    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    let descriptor_dir = storage_root
        .join("system/extensions/registered")
        .join(owner_scope.tenant_id.as_str())
        .join(owner_scope.user_id.as_str())
        .join("acme-mcp-credentialed");
    std::fs::create_dir_all(&descriptor_dir).expect("registered descriptor dir");
    std::fs::write(
        descriptor_dir.join("manifest.toml"),
        REGISTERED_CREDENTIALED_MANIFEST_TOML,
    )
    .expect("write registered descriptor");
    let mut local_filesystem = LocalFilesystem::new();
    local_filesystem
        .mount_local(
            VirtualPath::new("/system/extensions").expect("valid virtual path"),
            HostPath::from_path_buf(storage_root.join("system/extensions")),
        )
        .expect("mount system extensions");

    let mut lifecycle_registry = ExtensionRegistry::new();
    let mut active_registry_initial = ExtensionRegistry::new();
    let installation_store = Arc::new(InMemoryExtensionInstallationStore::default());
    let manifest = ExtensionManifest::parse(
        REGISTERED_CREDENTIALED_MANIFEST_TOML,
        source.clone(),
        &HostPortCatalog::empty(),
    )
    .expect("credentialed registered manifest");
    let root =
        VirtualPath::new("/system/extensions/acme-mcp-credentialed").expect("extension root");
    let package =
        ExtensionPackage::from_manifest_toml(manifest, root, REGISTERED_CREDENTIALED_MANIFEST_TOML)
            .expect("credentialed registered package");
    lifecycle_registry
        .insert(package.clone())
        .expect("lifecycle package");
    active_registry_initial
        .insert(package)
        .expect("active package");
    let manifest_record = fixture_manifest_record_with_source(
        REGISTERED_CREDENTIALED_STORE_MANIFEST_TOML,
        source.clone(),
        None,
    );
    installation_store
        .upsert_manifest(manifest_record)
        .await
        .expect("seed registered manifest record");
    let installation = ExtensionInstallation::new(
        ExtensionInstallationId::new("acme-mcp-credentialed").expect("valid installation id"),
        extension_id.clone(),
        ExtensionActivationState::Enabled,
        ExtensionManifestRef::new(extension_id, None),
        Vec::new(),
        chrono::Utc::now(),
        InstallationOwner::user(owner_scope.user_id.clone()),
    )
    .expect("registered installation");
    installation_store
        .upsert_installation(installation)
        .await
        .expect("seed registered installation");
    let active_registry = Arc::new(SharedExtensionRegistry::new(active_registry_initial));

    let port = Arc::new(RebornLocalExtensionManagementPort::new(
        Arc::new(local_filesystem),
        AvailableExtensionCatalog::from_packages(Vec::new()),
        installation_store,
        Arc::new(Mutex::new(ExtensionLifecycleService::new(
            lifecycle_registry,
        ))),
        test_active_extension_publisher(
            Arc::clone(&active_registry),
            test_extension_trust_policy(),
        ),
        None,
        owner_scope.user_id.clone(),
    ));
    let package_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "acme-mcp-credentialed")
            .expect("valid ref");
    (dir, port, package_ref)
}

/// Item B regression: `remove`'s credential-provider preflight
/// (`removed_extension_providers` -> `activation_credential_requirements`)
/// only checks the caller's USER axis (`ensure_caller_may_operate`), not the
/// registered row's TENANT axis, so a same-user-id caller from a foreign
/// tenant reaches it and gets a REAL non-empty requirement list back. With no
/// authenticated actor supplied, `remove` then surfaces the distinguishing
/// "extension credential cleanup requires an authenticated actor" error
/// instead of the masked "is not installed" `remove_locked`'s tenant check
/// would produce — leaking that a credentialed install exists under this id
/// before the tenant guard ever runs. Red before the fix.
#[tokio::test]
async fn remove_credential_preflight_masks_foreign_tenant_caller_before_authenticated_actor_check()
{
    let owner_scope = resource_scope_for("tenant-b", "owner-a");
    // Same user id, different (default) tenant scope, no authenticated
    // actor supplied — the exact shape that exposes the preflight ordering
    // bug.
    let cross_tenant_scope = resource_scope_for("default", "owner-a");
    let (_dir, port, package_ref) = credentialed_registered_isolation_fixture(&owner_scope).await;

    let error = port
        .remove(package_ref, &cross_tenant_scope, None)
        .await
        .expect_err("a foreign-tenant caller must not remove another tenant's registered install");

    let rendered = error.to_string();
    assert!(
        rendered.contains("is not installed"),
        "the caller's first divergence must be the masked not-installed denial, got: {rendered}"
    );
    assert!(
        !rendered.contains("authenticated actor"),
        "must not leak that a credentialed install exists via the authenticated-actor message, got: {rendered}"
    );
}

pub(super) fn resource_scope_for(tenant: &str, user: &str) -> ResourceScope {
    let mut scope =
        ResourceScope::local_default(UserId::new(user).expect("valid user"), InvocationId::new())
            .expect("valid local scope");
    scope.tenant_id = TenantId::new(tenant).expect("valid tenant");
    scope
}

/// Installed owner-registered extensions must not vanish from
/// `extension_list` (the shared catalog never holds them — the list path
/// overlays the CALLER's registered set), and only the owner sees them:
/// the row's `InstallationOwner` is the filter, so another user listing
/// the same store gets nothing.
#[tokio::test]
async fn extension_list_shows_owner_registered_install_only_to_owner() {
    let owner_scope = resource_scope_for("default", "owner-a");
    let other_scope = resource_scope_for("default", "owner-b");
    let (_dir, port, _package_ref, _active_registry, installation_store) =
        user_registered_isolation_fixture(&owner_scope, true).await;

    let list = port.list_installed(&owner_scope).await.expect("owner list");
    let Some(LifecycleProductPayload::ExtensionList { extensions, count }) = list.payload else {
        panic!("expected extension list payload");
    };
    assert_eq!(count, 1, "owner must see their registered install");
    assert_eq!(
        extensions[0].summary.package_ref.id.as_str(),
        "acme-mcp-registered"
    );
    assert_eq!(
        extensions[0].summary.source,
        LifecycleExtensionSource::UserRegistered,
        "listed registered extension must report the user_registered source"
    );

    // The installed row itself carries the singleton owner — the single
    // predicate every ownership-aware reader keys on.
    let row = installation_store
        .get_installation(&ExtensionInstallationId::new("acme-mcp-registered").expect("valid id"))
        .await
        .expect("store read")
        .expect("registered row present");
    assert!(
        row.owner().visible_to(&owner_scope.user_id)
            && !row.owner().visible_to(&other_scope.user_id)
            && !row.owner().is_tenant(),
        "registered row must carry InstallationOwner::user(owner)"
    );

    let other_list = port
        .list_installed(&other_scope)
        .await
        .expect("other owner list");
    let Some(LifecycleProductPayload::ExtensionList { extensions, count }) = other_list.payload
    else {
        panic!("expected extension list payload");
    };
    assert_eq!(
        (count, extensions.len()),
        (0, 0),
        "another owner must not see the owner's registered installation"
    );

    // Second registered install for the SAME owner: the listing loop's
    // registered-store fallback must resolve both entries from the ONE
    // batched read of the owner's registered set (installed_summaries),
    // not a per-entry rescan — a second, distinctly-ided registration
    // exercises the map lookup rather than a lucky single-entry pass.
    let second_extension_id =
        ExtensionId::new("acme-mcp-registered-two").expect("valid extension id");
    let second_manifest_toml = REGISTERED_ISOLATION_MANIFEST_TOML
        .replace("acme-mcp-registered", "acme-mcp-registered-two");
    let second_descriptor_dir = _dir
        .path()
        .join("local-dev/system/extensions/registered")
        .join(owner_scope.tenant_id.as_str())
        .join(owner_scope.user_id.as_str())
        .join("acme-mcp-registered-two");
    std::fs::create_dir_all(&second_descriptor_dir).expect("second registered descriptor dir");
    std::fs::write(
        second_descriptor_dir.join("manifest.toml"),
        &second_manifest_toml,
    )
    .expect("write second registered descriptor");
    let second_source = ManifestSource::UserRegistered {
        tenant_id: owner_scope.tenant_id.clone(),
        owner: owner_scope.user_id.clone(),
    };
    installation_store
        .upsert_manifest(fixture_manifest_record_with_source(
            &second_manifest_toml,
            second_source,
            None,
        ))
        .await
        .expect("seed second registered manifest record");
    installation_store
        .upsert_installation(
            ExtensionInstallation::new(
                ExtensionInstallationId::new("acme-mcp-registered-two")
                    .expect("valid installation id"),
                second_extension_id.clone(),
                ExtensionActivationState::Enabled,
                ExtensionManifestRef::new(second_extension_id.clone(), None),
                Vec::new(),
                chrono::Utc::now(),
                InstallationOwner::user(owner_scope.user_id.clone()),
            )
            .expect("second registered installation"),
        )
        .await
        .expect("seed second registered installation");

    let two_entry_list = port
        .list_installed(&owner_scope)
        .await
        .expect("owner list with two registered installs");
    let Some(LifecycleProductPayload::ExtensionList { extensions, count }) = two_entry_list.payload
    else {
        panic!("expected extension list payload");
    };
    assert_eq!(count, 2, "both registered installs must be listed");
    let mut ids: Vec<&str> = extensions
        .iter()
        .map(|entry| entry.summary.package_ref.id.as_str())
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["acme-mcp-registered", "acme-mcp-registered-two"]);
}

/// Review item 5 (list/project coverage): `project()` — the single-
/// package projection `LifecycleProductFacade::project_package` calls —
/// must mask a registered install the same way `list_installed`
/// (`extension_list_shows_owner_registered_install_only_to_owner`,
/// above) and `search` already do. `builtin.extension_search`/
/// `builtin.extension_install` are the only agent-tool-dispatched
/// entrypoints for registered packages; list/project are WebUI-facade-
/// only (`RebornServicesApi::list_extensions`, `project_package`), with
/// no `submit_turn`-reachable capability id, so this pins the same
/// owner-scoped port method the integration harness cannot dispatch to.
#[tokio::test]
async fn project_of_registered_package_masks_foreign_owner() {
    let owner_scope = resource_scope_for("default", "owner-a");
    let other_scope = resource_scope_for("default", "owner-b");
    let (_dir, port, package_ref, _active_registry, _installation_store) =
        user_registered_isolation_fixture(&owner_scope, true).await;

    let owner_projection = port
        .project(package_ref.clone(), &owner_scope)
        .await
        .expect("owner A's project must see their own registered package");
    let Some(LifecycleProductPayload::ExtensionList { extensions, count }) =
        owner_projection.payload
    else {
        panic!("expected extension list payload");
    };
    assert_eq!(count, 1, "owner A must see their registered install");
    assert_eq!(
        extensions[0].summary.package_ref.id.as_str(),
        "acme-mcp-registered"
    );

    let error = port
        .project(package_ref, &other_scope)
        .await
        .expect_err("a foreign caller's project of another owner's registered package must fail");
    assert!(matches!(
        error,
        ProductWorkflowError::InvalidBindingRequest { .. }
    ));
}

#[tokio::test]
async fn extension_activate_rejects_caller_outside_owning_scope() {
    let owner_scope = resource_scope_for("default", "owner-a");
    let other_scope = resource_scope_for("default", "owner-b");
    let (_dir, port, package_ref, _active_registry, _installation_store) =
        user_registered_isolation_fixture(&owner_scope, true).await;

    let error = port
        .activate(package_ref, ExtensionActivationMode::Static, &other_scope)
        .await
        .expect_err("a foreign caller must not activate another owner's registered install");

    assert!(matches!(
        error,
        ProductWorkflowError::InvalidBindingRequest { .. }
    ));
}

#[tokio::test]
async fn extension_remove_rejects_caller_outside_owning_scope() {
    let owner_scope = resource_scope_for("default", "owner-a");
    let other_scope = resource_scope_for("default", "owner-b");
    let (_dir, port, package_ref, active_registry, _installation_store) =
        user_registered_isolation_fixture(&owner_scope, true).await;

    let error = port
        .remove(package_ref, &other_scope, Some(&other_scope.user_id))
        .await
        .expect_err("a foreign caller must not remove another owner's registered install");

    assert!(matches!(
        error,
        ProductWorkflowError::InvalidBindingRequest { .. }
    ));
    assert!(
        active_registry
            .snapshot()
            .get_extension(&ExtensionId::new("acme-mcp-registered").expect("valid id"))
            .is_some(),
        "extension must remain published after a rejected foreign remove"
    );
}

/// T2 cross-tenant follow-up: the row's `InstallationOwner` carries only
/// USER ids, so the SAME user id arriving under a DIFFERENT tenant scope
/// passes `ensure_caller_may_operate` — the guards must also compare the
/// caller's tenant against the manifest's `UserRegistered.tenant_id`.
/// Install already fails via the caller-sharded overlay (not-found);
/// activate and remove need the explicit tenant check (RED before it:
/// both succeeded cross-tenant).
#[tokio::test]
async fn registered_mutations_reject_same_user_in_foreign_tenant_scope() {
    let owner_scope = resource_scope_for("tenant-b", "owner-a");
    // Same user id, different tenant scope.
    let cross_tenant_scope = resource_scope_for("default", "owner-a");
    let (_dir, port, package_ref, active_registry, installation_store) =
        user_registered_isolation_fixture(&owner_scope, true).await;

    // Search first: the overlay is path-sharded by (tenant, owner), so the
    // same user id under another tenant must not even see the descriptor.
    let search = port
        .search("acme-mcp-registered", None, &cross_tenant_scope)
        .await
        .expect("cross-tenant search runs (and finds nothing)");
    let Some(LifecycleProductPayload::ExtensionSearch { count, .. }) = search.payload else {
        panic!("expected extension search payload");
    };
    assert_eq!(
        count, 0,
        "cross-tenant search must not surface another tenant's registered descriptor"
    );

    let install_error = port
        .install(package_ref.clone(), &cross_tenant_scope)
        .await
        .expect_err("cross-tenant caller must not install the registered package");
    let activate_error = port
        .activate(
            package_ref.clone(),
            ExtensionActivationMode::Static,
            &cross_tenant_scope,
        )
        .await
        .expect_err("cross-tenant caller must not activate the registered install");
    let remove_error = port
        .remove(
            package_ref,
            &cross_tenant_scope,
            Some(&cross_tenant_scope.user_id),
        )
        .await
        .expect_err("cross-tenant caller must not remove the registered install");
    for error in [install_error, activate_error, remove_error] {
        let rendered = error.to_string();
        assert!(
            rendered.contains("is not installed") || rendered.contains("was not found"),
            "cross-tenant denial must be masked, got: {rendered}"
        );
    }
    assert!(
        active_registry
            .snapshot()
            .get_extension(&ExtensionId::new("acme-mcp-registered").expect("valid id"))
            .is_some(),
        "extension must remain published for its real tenant-owner"
    );
    let row = installation_store
        .get_installation(&ExtensionInstallationId::new("acme-mcp-registered").expect("valid id"))
        .await
        .expect("store read")
        .expect("registered row present");
    assert!(
        row.owner().visible_to(&owner_scope.user_id) && !row.owner().is_tenant(),
        "cross-tenant attempts must not mutate the registered row"
    );
}

/// Row-provenance-wins resolution (review item 1): once an installation
/// row exists and its stored manifest is `UserRegistered`, resolution for
/// that row must go to the registered store — never the shared catalog,
/// even when a same-id shared package sits in the catalog too. Before the
/// fix, `resolve_available_for_scope` (used by `project`/`install`)
/// checks the catalog unconditionally and would serve the colliding
/// catalog descriptor instead of the row's own registered one.
#[tokio::test]
async fn project_prefers_registered_row_over_same_id_catalog_package() {
    let owner_scope = resource_scope_for("default", "owner-a");
    let (_dir, port, package_ref, _active_registry, _installation_store) =
        user_registered_isolation_fixture(&owner_scope, true).await;

    // A shared-catalog package colliding on the SAME id but a wholly
    // different descriptor (name/description diverge from the registered
    // one, so a wrong resolution is trivially observable). A HostBundled
    // package must be wasm + declare capabilities (the bare-MCP shape is
    // only valid for `UserRegistered`), so this borrows the legacy
    // `[[capabilities]]` shape from `fixture_extension_manifest` with the
    // colliding id.
    let colliding_manifest = r#"
schema_version = "reborn.extension_manifest.v2"
id = "acme-mcp-registered"
name = "Colliding Shared Package"
version = "0.1.0"
description = "Shared catalog package colliding on the same id"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/colliding.wasm"

[[capabilities]]
id = "acme-mcp-registered.search"
description = "Search colliding data"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"
"#;
    let colliding_package = fixture_extension_package_from_manifest_with_root(
        colliding_manifest,
        "acme-mcp-registered",
    );
    {
        let mut catalog = port.catalog.write().await;
        catalog.extend(AvailableExtensionCatalog::from_packages(vec![
            colliding_package,
        ]));
    }

    let projection = port
        .project(package_ref.clone(), &owner_scope)
        .await
        .expect("owner's project must still resolve despite the catalog collision");
    let Some(LifecycleProductPayload::ExtensionList { extensions, count }) = projection.payload
    else {
        panic!("expected extension list payload");
    };
    assert_eq!(count, 1);
    assert_eq!(
        extensions[0].summary.name, "Acme Registered MCP",
        "row-provenance must win: the registered descriptor must be served, not the \
         colliding shared-catalog package"
    );
    assert_eq!(
        extensions[0].summary.source,
        LifecycleExtensionSource::UserRegistered,
        "the served descriptor must still report as user_registered"
    );

    // Inverse pin: an id with NO installation row still resolves via the
    // shared catalog as normal — row-provenance-wins must not disable
    // catalog-first resolution for a fresh, never-installed id.
    let fresh_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").expect("valid ref");
    {
        let mut catalog = port.catalog.write().await;
        catalog.extend(AvailableExtensionCatalog::from_packages(vec![
            fixture_extension_package(),
        ]));
    }
    let fresh_install = port
        .install(fresh_ref, &owner_scope)
        .await
        .expect("a fresh id with no installation row must install normally via the catalog");
    assert!(matches!(
        fresh_install.payload,
        Some(LifecycleProductPayload::ExtensionInstall {
            installed: true,
            ..
        })
    ));
}

/// Item 6: search/list callers only ever read a package's manifest-derived
/// summary fields, never `.assets`, so `search_with_owner_overlay_for_scope`
/// must skip the per-entry directory-asset read
/// (`inline_extension_dir_assets`) entirely, while a resolution path that
/// feeds install/restore (`resolve_registered_for_scope`) must still inline
/// them. Pins both sides of the seam directly against the raw
/// `AvailableExtensionPackage`, since the product-facing summary types don't
/// expose `.assets` to observe the difference through `port.search`.
#[tokio::test]
async fn search_overlay_skips_assets_while_resolve_still_inlines_them() {
    let owner_scope = resource_scope_for("default", "owner-a");
    let (dir, port, package_ref, _active_registry, _installation_store) =
        user_registered_isolation_fixture(&owner_scope, true).await;

    // An extra asset file alongside manifest.toml makes a non-empty
    // `.assets` result trivially observable (manifest.toml alone would
    // already make `Inline` non-empty, but this rules out an accidental
    // "assets == just the manifest" coincidence).
    let descriptor_dir = dir
        .path()
        .join("local-dev/system/extensions/registered")
        .join(owner_scope.tenant_id.as_str())
        .join(owner_scope.user_id.as_str())
        .join("acme-mcp-registered");
    std::fs::write(descriptor_dir.join("extra.txt"), b"asset bytes")
        .expect("write extra asset file");

    let search_results =
        search_with_owner_overlay_for_scope(port.filesystem.as_ref(), &owner_scope, "acme")
            .await
            .expect("owner overlay search runs");
    assert_eq!(search_results.len(), 1);
    assert!(
        search_results[0].assets.is_empty(),
        "search must skip asset inlining entirely, got: {:?}",
        search_results[0].assets
    );

    let resolved =
        resolve_registered_for_scope(port.filesystem.as_ref(), &owner_scope, &package_ref)
            .await
            .expect("resolve runs")
            .expect("registered package resolves");
    assert!(
        !resolved.assets.is_empty(),
        "resolve (install/restore path) must still inline assets"
    );
}

/// Regression: the installation row is a single flat map keyed only by
/// extension id (no tenant axis), so the SAME user registering the SAME
/// extension id under two different tenants must not let an install row
/// from tenant B leak its phase onto tenant A's (never-installed)
/// descriptor. `search_summary` must require the row's own effective
/// registered tenant to match the caller's search scope tenant.
#[tokio::test]
async fn search_does_not_report_foreign_tenants_installation_phase() {
    let installed_scope = resource_scope_for("tenant-b", "owner-a");
    // Same user id, but registers (never installs) the same extension id
    // under a different tenant.
    let uninstalled_scope = resource_scope_for("default", "owner-a");
    let (dir, port, _package_ref, _active_registry, _installation_store) =
        user_registered_isolation_fixture(&installed_scope, true).await;

    // Register (not install) the same extension id under the caller's
    // OWN tenant — a legitimate, distinct registration that has never
    // been installed anywhere.
    let uninstalled_descriptor_dir = dir
        .path()
        .join("local-dev/system/extensions/registered")
        .join(uninstalled_scope.tenant_id.as_str())
        .join(uninstalled_scope.user_id.as_str())
        .join("acme-mcp-registered");
    std::fs::create_dir_all(&uninstalled_descriptor_dir)
        .expect("uninstalled registered descriptor dir");
    std::fs::write(
        uninstalled_descriptor_dir.join("manifest.toml"),
        REGISTERED_ISOLATION_MANIFEST_TOML,
    )
    .expect("write uninstalled registered descriptor");

    let search = port
        .search("acme-mcp-registered", None, &uninstalled_scope)
        .await
        .expect("search under the uninstalled tenant runs");
    let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) = search.payload else {
        panic!("expected extension search payload");
    };
    assert_eq!(
        extensions.len(),
        1,
        "the caller's own registered descriptor must be found"
    );
    assert!(
        extensions[0].installation_phase.is_none(),
        "must not report tenant-b's installation phase for a descriptor never installed in this tenant"
    );
}

/// Regression sibling of `search_does_not_report_foreign_tenants_installation_phase`
/// for the LIST surface: `installed_summaries` currently masks rows by
/// `UserId` only, so the same user's OWN registration under their own
/// tenant gets paired with a foreign-tenant install row of the same
/// extension id and reported as installed. Must apply the same
/// row-authoritative tenant check `search_summary` does. RED until fixed.
#[tokio::test]
async fn list_installed_does_not_report_foreign_tenants_installation_as_installed() {
    let installed_scope = resource_scope_for("tenant-b", "owner-a");
    let uninstalled_scope = resource_scope_for("default", "owner-a");
    let (dir, port, _package_ref, _active_registry, _installation_store) =
        user_registered_isolation_fixture(&installed_scope, true).await;

    // Register (not install) the same extension id under the caller's OWN
    // tenant — a distinct registration that has never been installed.
    let uninstalled_descriptor_dir = dir
        .path()
        .join("local-dev/system/extensions/registered")
        .join(uninstalled_scope.tenant_id.as_str())
        .join(uninstalled_scope.user_id.as_str())
        .join("acme-mcp-registered");
    std::fs::create_dir_all(&uninstalled_descriptor_dir)
        .expect("uninstalled registered descriptor dir");
    std::fs::write(
        uninstalled_descriptor_dir.join("manifest.toml"),
        REGISTERED_ISOLATION_MANIFEST_TOML,
    )
    .expect("write uninstalled registered descriptor");

    let list = port
        .list_installed(&uninstalled_scope)
        .await
        .expect("list under the uninstalled tenant runs");
    let Some(LifecycleProductPayload::ExtensionList { count, .. }) = list.payload else {
        panic!("expected extension list payload");
    };
    assert_eq!(
        count, 0,
        "must not report tenant-b's install row as installed under the caller's own tenant"
    );
}

/// Regression sibling of `search_does_not_report_foreign_tenants_installation_phase`
/// for the PROJECT surface: `project()` must apply the same
/// row-authoritative tenant check before returning phase/install_scope
/// derived from a foreign-tenant install row. RED until fixed.
#[tokio::test]
async fn project_does_not_report_foreign_tenants_installation_as_installed() {
    let installed_scope = resource_scope_for("tenant-b", "owner-a");
    let uninstalled_scope = resource_scope_for("default", "owner-a");
    let (dir, port, package_ref, _active_registry, _installation_store) =
        user_registered_isolation_fixture(&installed_scope, true).await;

    let uninstalled_descriptor_dir = dir
        .path()
        .join("local-dev/system/extensions/registered")
        .join(uninstalled_scope.tenant_id.as_str())
        .join(uninstalled_scope.user_id.as_str())
        .join("acme-mcp-registered");
    std::fs::create_dir_all(&uninstalled_descriptor_dir)
        .expect("uninstalled registered descriptor dir");
    std::fs::write(
        uninstalled_descriptor_dir.join("manifest.toml"),
        REGISTERED_ISOLATION_MANIFEST_TOML,
    )
    .expect("write uninstalled registered descriptor");

    let projection = port
        .project(package_ref, &uninstalled_scope)
        .await
        .expect("project under the uninstalled tenant runs");
    assert_eq!(
        projection.phase,
        LifecyclePhase::Discovered,
        "must project as discovered/not-installed, not tenant-b's installed phase"
    );
    let Some(LifecycleProductPayload::ExtensionList { extensions, .. }) = projection.payload else {
        panic!("expected extension list payload");
    };
    assert_eq!(extensions.len(), 1);
    assert!(
        extensions[0].install_scope.is_none(),
        "must not surface tenant-b's install_scope for this tenant's projection"
    );
}

/// Design point 4a/4b: a registered package resolves through the CALLER's
/// path-sharded overlay only, so neither another member nor the tenant
/// operator can even resolve it — install fails not-found BEFORE
/// `decide_install_on_existing` could join a foreign caller or evict the
/// row to `Tenant`, and the row is untouched.
#[tokio::test]
async fn foreign_and_operator_install_of_registered_package_is_not_found() {
    let owner_scope = resource_scope_for("default", "owner-a");
    let other_scope = resource_scope_for("default", "owner-b");
    let (dir, port, package_ref, _active_registry, installation_store) =
        user_registered_isolation_fixture(&owner_scope, true).await;
    // The fixture wires owner-a as the tenant operator, but a distinct
    // operator identity must ALSO fail to resolve a foreign registration;
    // other_scope covers the plain-member probe.
    let error = port
        .install(package_ref.clone(), &other_scope)
        .await
        .expect_err("foreign caller must not install another owner's registered package");
    assert!(matches!(
        error,
        ProductWorkflowError::InvalidBindingRequest { .. }
    ));
    let row = installation_store
        .get_installation(&ExtensionInstallationId::new("acme-mcp-registered").expect("valid id"))
        .await
        .expect("store read")
        .expect("registered row present");
    assert!(
        !row.owner().is_tenant() && row.owner().visible_to(&owner_scope.user_id),
        "failed foreign install attempts must not evict the registered row to Tenant"
    );

    // Item E: `other_scope` above is a plain member under the SAME port,
    // whose fixture-wired tenant operator IS the manifest owner (owner-a) —
    // it never actually exercises an operator distinct from the owner. Wire
    // a genuinely third identity as this port's tenant operator, over the
    // SAME installation store and on-disk descriptor, and confirm their
    // install of the same id is masked identically.
    let operator_scope = resource_scope_for("default", "tenant-operator-c");
    let mut operator_filesystem = LocalFilesystem::new();
    operator_filesystem
        .mount_local(
            VirtualPath::new("/system/extensions").expect("valid virtual path"),
            HostPath::from_path_buf(dir.path().join("local-dev/system/extensions")),
        )
        .expect("mount system extensions");
    let operator_active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
    let operator_port = RebornLocalExtensionManagementPort::new(
        Arc::new(operator_filesystem),
        AvailableExtensionCatalog::from_packages(Vec::new()),
        installation_store.clone(),
        Arc::new(Mutex::new(ExtensionLifecycleService::new(
            ExtensionRegistry::new(),
        ))),
        test_active_extension_publisher(operator_active_registry, test_extension_trust_policy()),
        None,
        operator_scope.user_id.clone(),
    );
    let operator_error = operator_port
        .install(package_ref, &operator_scope)
        .await
        .expect_err(
            "a distinct tenant operator must not install another owner's registered package",
        );
    assert!(matches!(
        operator_error,
        ProductWorkflowError::InvalidBindingRequest { .. }
    ));
    let row_after_operator_attempt = installation_store
        .get_installation(&ExtensionInstallationId::new("acme-mcp-registered").expect("valid id"))
        .await
        .expect("store read")
        .expect("registered row present");
    assert!(
        !row_after_operator_attempt.owner().is_tenant()
            && row_after_operator_attempt
                .owner()
                .visible_to(&owner_scope.user_id),
        "a distinct operator's failed install attempt must not evict the registered row to Tenant"
    );
}

/// Design points 2 and 5: a fresh owner install of a registered package
/// stamps `InstallationOwner::user(<manifest owner>)` even though the
/// owner IS the tenant operator (`derive_owner` would produce `Tenant`
/// and leak the private registration tenant-wide), and an owner
/// RE-install keeps the singleton row instead of routing through
/// `decide_install_on_existing`'s operator eviction.
#[tokio::test]
async fn owner_install_of_registered_package_stamps_manifest_owner_row() {
    let owner_scope = resource_scope_for("default", "owner-a");
    let other_scope = resource_scope_for("default", "owner-b");
    let (_dir, port, package_ref, _active_registry, installation_store) =
        user_registered_isolation_fixture(&owner_scope, false).await;

    for pass in ["fresh install", "re-install"] {
        port.install(package_ref.clone(), &owner_scope)
            .await
            .expect("owner installs their registered package");
        let row = installation_store
            .get_installation(
                &ExtensionInstallationId::new("acme-mcp-registered").expect("valid id"),
            )
            .await
            .expect("store read")
            .expect("registered row present");
        assert!(
            !row.owner().is_tenant()
                && row.owner().visible_to(&owner_scope.user_id)
                && !row.owner().visible_to(&other_scope.user_id),
            "{pass}: registered row must be the singleton manifest owner, never Tenant"
        );
    }
}

/// Review item 1 (row-takeover guard): rows are keyed by bare
/// `ExtensionId` while registered descriptors are owner-scoped, so
/// before owner-unique id minting a second owner can write their own
/// registered descriptor under the SAME id. Installing it as owner B
/// must not re-stamp owner A's existing row to B — it must be masked
/// exactly like a foreign non-member install, and A's row must survive
/// untouched.
#[tokio::test]
async fn install_of_same_id_by_different_registered_owner_does_not_take_over_row() {
    let owner_a = resource_scope_for("default", "owner-a");
    let owner_b = resource_scope_for("default", "owner-b");
    let (dir, port, package_ref, _active_registry, installation_store) =
        user_registered_isolation_fixture(&owner_a, true).await;

    // Owner B registers their own descriptor under the same bare id.
    let descriptor_dir = dir
        .path()
        .join("local-dev/system/extensions/registered")
        .join(owner_b.tenant_id.as_str())
        .join(owner_b.user_id.as_str())
        .join("acme-mcp-registered");
    std::fs::create_dir_all(&descriptor_dir).expect("registered descriptor dir for owner b");
    std::fs::write(
        descriptor_dir.join("manifest.toml"),
        REGISTERED_ISOLATION_MANIFEST_TOML,
    )
    .expect("write owner b registered descriptor");

    let error = port
        .install(package_ref, &owner_b)
        .await
        .expect_err("owner B must not take over owner A's registered row");
    assert!(
        matches!(error, ProductWorkflowError::InvalidBindingRequest { .. })
            && error.to_string().contains("is not installed"),
        "takeover attempt must be masked like a foreign install: {error}"
    );

    let row = installation_store
        .get_installation(&ExtensionInstallationId::new("acme-mcp-registered").expect("valid id"))
        .await
        .expect("store read")
        .expect("registered row present");
    assert!(
        row.owner().visible_to(&owner_a.user_id) && !row.owner().visible_to(&owner_b.user_id),
        "owner A's row must be unchanged after the rejected takeover"
    );
}

/// Design point 4c: the owner removing their registered install is the
/// last (only) holder, so the remove tears the installation down —
/// no row, no lifecycle residue, no published capability.
#[tokio::test]
async fn owner_remove_of_registered_install_tears_down_without_residue() {
    let owner_scope = resource_scope_for("default", "owner-a");
    let (_dir, port, package_ref, active_registry, installation_store) =
        user_registered_isolation_fixture(&owner_scope, true).await;

    port.remove(
        package_ref.clone(),
        &owner_scope,
        Some(&owner_scope.user_id),
    )
    .await
    .expect("owner removes their registered install");

    assert!(
        installation_store
            .get_installation(
                &ExtensionInstallationId::new("acme-mcp-registered").expect("valid id"),
            )
            .await
            .expect("store read")
            .is_none(),
        "registered row must be gone after the owner's remove"
    );
    assert!(
        active_registry
            .snapshot()
            .get_extension(&ExtensionId::new("acme-mcp-registered").expect("valid id"))
            .is_none(),
        "registered extension must be unpublished after the owner's remove"
    );
}

/// Item 2 regression: `remove`'s orphan-cleanup branch (row present,
/// lifecycle package absent) never runs `ensure_registered_row_tenant_match`
/// — that tenant-axis check lives only in `remove_locked`, which the orphan
/// branch does not call. A same-user-id caller from a FOREIGN tenant reaches
/// `ensure_caller_may_operate` (user-axis only, passes for a same-user-id
/// row) and deletes the orphaned registered row outright. Red before the
/// fix: this call succeeds and tears the row down.
#[tokio::test]
async fn remove_of_orphaned_registered_row_rejects_foreign_tenant_same_user_id() {
    let owner_scope = resource_scope_for("tenant-a", "owner-a");
    let foreign_tenant_scope = resource_scope_for("tenant-b", "owner-a");
    let (_dir, port, package_ref, _active_registry, installation_store) =
        user_registered_isolation_fixture(&owner_scope, true).await;

    // Orphan the row: drop it from the lifecycle registry while leaving the
    // installation row and its stored manifest in place, matching the
    // `installation.is_some() && !lifecycle_package_present` branch in
    // `remove`.
    let extension_id = ExtensionId::new("acme-mcp-registered").expect("valid extension id");
    port.lifecycle_service
        .lock()
        .await
        .remove(&extension_id)
        .await
        .expect("drop lifecycle package to orphan the row");

    let error = port
        .remove(
            package_ref,
            &foreign_tenant_scope,
            Some(&foreign_tenant_scope.user_id),
        )
        .await
        .expect_err(
            "a foreign-tenant caller with the same user id must not remove another tenant's \
             orphaned registered row",
        );
    assert!(
        error.to_string().contains("is not installed"),
        "must use the masked not-installed denial, got: {error}"
    );

    let row = installation_store
        .get_installation(&ExtensionInstallationId::new("acme-mcp-registered").expect("valid id"))
        .await
        .expect("store read")
        .expect("orphaned registered row must survive the rejected foreign-tenant remove");
    assert!(
        row.owner().visible_to(&owner_scope.user_id),
        "the orphaned row must remain the real owner's, untouched by the foreign-tenant attempt"
    );
}

/// Item A regression: a registered install is never materialized under
/// `/system/extensions/<id>/` (mirrors the remove path's `is_owner_registered`
/// guard), so a DB failure during `persist_install_plan` must not delete
/// pre-existing content there — this operation never created it. Red before
/// the fix: the compensating cleanup ran unconditionally on every
/// `persist_install_plan` failure and deleted the caller's pre-seeded file.
#[tokio::test]
async fn failed_registered_install_persist_does_not_delete_preexisting_extension_files() {
    let owner_scope = resource_scope_for("default", "owner-a");

    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    let descriptor_dir = storage_root
        .join("system/extensions/registered")
        .join(owner_scope.tenant_id.as_str())
        .join(owner_scope.user_id.as_str())
        .join("acme-mcp-registered");
    std::fs::create_dir_all(&descriptor_dir).expect("registered descriptor dir");
    std::fs::write(
        descriptor_dir.join("manifest.toml"),
        REGISTERED_ISOLATION_MANIFEST_TOML,
    )
    .expect("write registered descriptor");

    // Pre-existing content at the shared materialization path this
    // operation must never touch for a registered install.
    let preexisting_dir = storage_root
        .join("system/extensions")
        .join("acme-mcp-registered");
    std::fs::create_dir_all(&preexisting_dir).expect("preexisting extension dir");
    std::fs::write(preexisting_dir.join("leftover.txt"), b"do not delete")
        .expect("seed preexisting extension file");

    let mut local_filesystem = LocalFilesystem::new();
    local_filesystem
        .mount_local(
            VirtualPath::new("/system/extensions").expect("valid virtual path"),
            HostPath::from_path_buf(storage_root.join("system/extensions")),
        )
        .expect("mount system extensions");
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(local_filesystem);

    let failing_store = DeleteInstallationFailingStore::default();
    failing_store
        .fail_next_upsert_installation
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let installation_store: Arc<dyn ExtensionInstallationStore> = Arc::new(failing_store);

    let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
    let port = RebornLocalExtensionManagementPort::new(
        filesystem,
        AvailableExtensionCatalog::from_packages(Vec::new()),
        installation_store,
        Arc::new(Mutex::new(ExtensionLifecycleService::new(
            ExtensionRegistry::new(),
        ))),
        test_active_extension_publisher(
            Arc::clone(&active_registry),
            test_extension_trust_policy(),
        ),
        None,
        owner_scope.user_id.clone(),
    );
    let package_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "acme-mcp-registered")
            .expect("valid ref");

    port.install(package_ref, &owner_scope)
        .await
        .expect_err("injected persistence failure must fail the install");

    assert!(
        preexisting_dir.join("leftover.txt").exists(),
        "a DB failure during a registered install must not delete pre-existing shared \
         extension files this operation never created"
    );
}
