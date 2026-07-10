//! Registered hosted-MCP lifecycle cleanup and inventory regression tests.

use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use ironclaw_approvals::{
    CapabilityPermissionOverride, CapabilityPermissionOverrideInput,
    CapabilityPermissionOverrideKey, CapabilityPermissionOverrideStore,
    InMemoryCapabilityPermissionOverrideStore, InMemoryPersistentApprovalPolicyStore,
    PersistentApprovalAction, PersistentApprovalPolicy, PersistentApprovalPolicyError,
    PersistentApprovalPolicyInput, PersistentApprovalPolicyKey, PersistentApprovalPolicyStore,
};
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionHealthSnapshot, ExtensionInstallation,
    ExtensionInstallationError, ExtensionInstallationId, ExtensionInstallationStore,
    ExtensionLifecycleService, ExtensionManifestRecord, ExtensionRegistry,
    InMemoryExtensionInstallationStore, SharedExtensionRegistry,
};
use ironclaw_filesystem::{
    CasExpectation, DirEntry, Entry, FileStat, FilesystemError, FilesystemOperation,
    LocalFilesystem, RecordVersion, RootFilesystem, VersionedEntry,
};
use ironclaw_host_api::{
    ApprovalRequestId, CapabilityId, EffectKind, ExtensionId, GrantConstraints, HostPath,
    InvocationId, MountView, NetworkMethod, NetworkPolicy, Principal, ResourceScope,
    RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse,
    TenantId, UserId, VirtualPath,
};
use ironclaw_trust::{AdminConfig, HostTrustPolicy, InvalidationBus};
use tokio::sync::Mutex;

use super::hosted_mcp_test_support::HostedMcpDiscoveryEgress;
use super::tests::extension_port_with_file_delete_failing_filesystem;
use super::*;

struct RegisteredLifecycleFixture {
    _dir: tempfile::TempDir,
    storage_root: std::path::PathBuf,
    port: Arc<RebornLocalExtensionManagementPort>,
    active_registry: Arc<SharedExtensionRegistry>,
    installation_store: Arc<dyn ExtensionInstallationStore>,
}

impl RegisteredLifecycleFixture {
    fn new() -> Self {
        Self::with_catalog_and_stores(
            AvailableExtensionCatalog::from_packages(Vec::new()),
            Arc::new(InMemoryExtensionInstallationStore::default()),
            Arc::new(InMemoryPersistentApprovalPolicyStore::new()),
            Arc::new(InMemoryCapabilityPermissionOverrideStore::new()),
            false,
        )
    }

    fn with_stores(
        installation_store: Arc<dyn ExtensionInstallationStore>,
        persistent_approval_policies: Arc<dyn PersistentApprovalPolicyStore>,
        tool_permission_overrides: Arc<dyn CapabilityPermissionOverrideStore>,
    ) -> Self {
        Self::with_catalog_and_stores(
            AvailableExtensionCatalog::from_packages(Vec::new()),
            installation_store,
            persistent_approval_policies,
            tool_permission_overrides,
            false,
        )
    }

    fn with_catalog(catalog: AvailableExtensionCatalog) -> Self {
        Self::with_catalog_and_stores(
            catalog,
            Arc::new(InMemoryExtensionInstallationStore::default()),
            Arc::new(InMemoryPersistentApprovalPolicyStore::new()),
            Arc::new(InMemoryCapabilityPermissionOverrideStore::new()),
            false,
        )
    }

    fn with_cleanup_inventory_delete_failure() -> Self {
        Self::with_catalog_and_stores(
            AvailableExtensionCatalog::from_packages(Vec::new()),
            Arc::new(InMemoryExtensionInstallationStore::default()),
            Arc::new(InMemoryPersistentApprovalPolicyStore::new()),
            Arc::new(InMemoryCapabilityPermissionOverrideStore::new()),
            true,
        )
    }

    fn with_inventory_restore_failure(
        installation_store: Arc<dyn ExtensionInstallationStore>,
        persistent_approval_policies: Arc<dyn PersistentApprovalPolicyStore>,
        tool_permission_overrides: Arc<dyn CapabilityPermissionOverrideStore>,
    ) -> Self {
        let mut fixture = Self::with_stores(
            installation_store,
            persistent_approval_policies,
            tool_permission_overrides,
        );
        let inner = Arc::clone(&fixture.port.filesystem);
        Arc::get_mut(&mut fixture.port)
            .expect("fixture port has a unique owner")
            .filesystem = Arc::new(InventoryRestoreFailingFilesystem {
            inner,
            cleanup_inventory_puts: AtomicUsize::new(0),
        });
        fixture
    }

    fn with_catalog_and_stores(
        catalog: AvailableExtensionCatalog,
        installation_store: Arc<dyn ExtensionInstallationStore>,
        persistent_approval_policies: Arc<dyn PersistentApprovalPolicyStore>,
        tool_permission_overrides: Arc<dyn CapabilityPermissionOverrideStore>,
        fail_cleanup_inventory_delete: bool,
    ) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");
        let mut filesystem = LocalFilesystem::new();
        filesystem
            .mount_local(
                VirtualPath::new("/system/extensions").expect("valid virtual path"),
                HostPath::from_path_buf(storage_root.join("system/extensions")),
            )
            .expect("mount extensions");
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(filesystem);
        let filesystem: Arc<dyn RootFilesystem> = if fail_cleanup_inventory_delete {
            Arc::new(CleanupInventoryDeleteFailingFilesystem { inner: filesystem })
        } else {
            filesystem
        };
        let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
        let trust_policy = Arc::new(
            HostTrustPolicy::new(vec![Box::new(AdminConfig::new())]).expect("trust policy"),
        );
        let active_extensions = ActiveExtensionPublisher::new(
            Arc::clone(&active_registry),
            trust_policy,
            Arc::new(InvalidationBus::new()),
        );
        let port = Arc::new(RebornLocalExtensionManagementPort::new(
            filesystem,
            catalog,
            Arc::clone(&installation_store),
            Arc::new(Mutex::new(ExtensionLifecycleService::new(
                ExtensionRegistry::new(),
            ))),
            active_extensions,
            ExtensionCapabilityAuthorityCleanup::new(
                persistent_approval_policies,
                tool_permission_overrides,
            ),
            None,
        ));
        Self {
            _dir: dir,
            storage_root,
            port,
            active_registry,
            installation_store,
        }
    }

    async fn register(
        &self,
        owner: &UserId,
        path: &str,
    ) -> (ResourceScope, LifecyclePackageRef, ExtensionId) {
        let scope = hosted_mcp_scope(owner.as_str());
        let registered = self
            .port
            .register_hosted_mcp(
                format!("{path} MCP"),
                format!("https://93.184.216.34/{path}"),
                &scope,
            )
            .await
            .expect("register hosted MCP");
        let package_ref = registered.package_ref.expect("package ref");
        let extension_id =
            ExtensionId::new(package_ref.id.as_str().to_string()).expect("extension id");
        (scope, package_ref, extension_id)
    }
}

#[tokio::test]
async fn registered_mcp_transient_discovery_fails_closed_without_inventory_or_publication() {
    let fixture = RegisteredLifecycleFixture::new();
    let owner = UserId::new("registered-empty-owner").expect("valid owner");
    let (scope, package_ref, extension_id) = fixture.register(&owner, "empty-mcp").await;
    let installation_id =
        ExtensionInstallationId::new(extension_id.as_str().to_string()).expect("installation id");

    let error = fixture
        .port
        .activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::HostedMcpDiscovery {
                scope,
                runtime_http_egress: Arc::new(EmptyToolsHostedMcpEgress),
                network_policy_authority: extension_activate_capability_id(),
            },
        )
        .await
        .expect_err("registered empty discovery must fail closed");

    assert!(matches!(error, ProductWorkflowError::Transient { .. }));
    assert_eq!(
        fixture
            .installation_store
            .get_installation(&installation_id)
            .await
            .expect("read installation")
            .expect("installation remains")
            .activation_state(),
        ExtensionActivationState::Installed
    );
    assert!(
        fixture
            .active_registry
            .snapshot()
            .get_extension(&extension_id)
            .is_none()
    );
    assert!(
        RegisteredExtensionStore::load_discovered_capability_ids(
            fixture.port.filesystem.as_ref(),
            &owner,
            &extension_id,
        )
        .await
        .expect("load absent inventory")
        .is_empty()
    );
}

#[tokio::test]
async fn unregister_fails_closed_when_active_publication_lacks_installation_state() {
    let fixture = RegisteredLifecycleFixture::new();
    let owner = UserId::new("unregister-drift-owner").expect("valid owner");
    let (scope, package_ref, extension_id) = fixture.register(&owner, "unregister-drift").await;

    fixture
        .port
        .activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: scope.clone(),
                runtime_http_egress: Arc::new(HostedMcpDiscoveryEgress::default()),
                network_policy_authority: extension_activate_capability_id(),
            },
        )
        .await
        .expect("activate registered MCP");
    assert!(
        fixture
            .active_registry
            .snapshot()
            .get_extension(&extension_id)
            .is_some(),
        "fixture must start with an active publication"
    );

    let installation_id =
        ExtensionInstallationId::new(extension_id.as_str().to_string()).expect("installation id");
    fixture
        .installation_store
        .delete_installation(&installation_id)
        .await
        .expect("delete installation to model state drift");
    fixture
        .installation_store
        .delete_manifest(&extension_id)
        .await
        .expect("delete manifest to model state drift");

    let error = fixture
        .port
        .unregister_hosted_mcp(package_ref, &scope)
        .await
        .expect_err("active publication without installation state must fail closed");
    assert!(
        matches!(error, ProductWorkflowError::Transient { .. }),
        "unexpected state-drift error: {error}"
    );
    assert!(
        fixture
            .active_registry
            .snapshot()
            .get_extension(&extension_id)
            .is_some(),
        "failed unregister must not leave an untracked stale publication silently claimed as removed"
    );
}

#[tokio::test]
async fn registered_mcp_activation_rejects_other_owner_before_egress_or_state_mutation() {
    let installation_store = Arc::new(InMemoryExtensionInstallationStore::default());
    let policies = Arc::new(InMemoryPersistentApprovalPolicyStore::new());
    let overrides = Arc::new(InMemoryCapabilityPermissionOverrideStore::new());
    let fixture = RegisteredLifecycleFixture::with_stores(
        installation_store.clone(),
        policies.clone(),
        overrides.clone(),
    );
    let owner = UserId::new("activation-owner").expect("valid owner");
    let attacker = UserId::new("activation-attacker").expect("valid attacker");
    let (owner_scope, package_ref, extension_id) = fixture.register(&owner, "owner-only-mcp").await;
    let installation_id =
        ExtensionInstallationId::new(extension_id.as_str().to_string()).expect("installation id");
    let prior_id =
        CapabilityId::new(format!("{}.prior", extension_id.as_str())).expect("prior capability id");
    RegisteredExtensionStore::replace_discovered_capability_ids_for_scope(
        fixture.port.filesystem.as_ref(),
        &owner_scope,
        &extension_id,
        std::slice::from_ref(&prior_id),
    )
    .await
    .expect("seed victim inventory");
    let settings_scope = owner_scope.tenant_user_settings_scope();
    let grantee = Principal::Extension(extension_id.clone());
    seed_authority(
        fixture.port.as_ref(),
        &settings_scope,
        &owner,
        &grantee,
        &prior_id,
    )
    .await;
    let manifest_before = installation_store
        .get_manifest(&extension_id)
        .await
        .expect("read victim manifest")
        .expect("victim manifest exists");
    let egress = Arc::new(HostedMcpDiscoveryEgress::default());

    let error = fixture
        .port
        .activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: hosted_mcp_scope(attacker.as_str()),
                runtime_http_egress: egress.clone(),
                network_policy_authority: extension_activate_capability_id(),
            },
        )
        .await
        .expect_err("another owner must not activate a registered provider");

    assert!(
        error
            .to_string()
            .contains("available extension was not found")
    );
    assert!(
        egress.methods().is_empty(),
        "owner check must precede egress"
    );
    assert_eq!(
        installation_store
            .get_manifest(&extension_id)
            .await
            .expect("read victim manifest after rejection")
            .expect("victim manifest remains")
            .manifest_hash(),
        manifest_before.manifest_hash(),
        "cross-owner activation must not replace the victim manifest"
    );
    assert_eq!(
        installation_store
            .get_installation(&installation_id)
            .await
            .expect("read victim installation")
            .expect("victim installation remains")
            .activation_state(),
        ExtensionActivationState::Installed
    );
    assert_eq!(
        RegisteredExtensionStore::load_discovered_capability_ids_for_scope(
            fixture.port.filesystem.as_ref(),
            &owner_scope,
            &extension_id,
        )
        .await
        .expect("load victim inventory after rejection"),
        vec![prior_id.clone()]
    );
    assert!(
        fixture
            .active_registry
            .snapshot()
            .get_extension(&extension_id)
            .is_none(),
        "cross-owner activation must not publish the victim provider"
    );
    assert!(
        policies
            .lookup(&PersistentApprovalPolicyKey::new(
                &settings_scope,
                PersistentApprovalAction::Dispatch,
                prior_id.clone(),
                grantee,
            ))
            .await
            .expect("lookup victim policy")
            .is_some_and(|policy| policy.revoked_at.is_none()),
        "cross-owner activation must not revoke victim authority"
    );
    assert!(
        overrides
            .get(&CapabilityPermissionOverrideKey::new(
                &settings_scope,
                prior_id,
            ))
            .await
            .expect("lookup victim override")
            .is_some(),
        "cross-owner activation must not clear victim permission settings"
    );

    fixture
        .port
        .activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: owner_scope,
                runtime_http_egress: egress.clone(),
                network_policy_authority: extension_activate_capability_id(),
            },
        )
        .await
        .expect("the registered owner can still activate");
    assert!(!egress.methods().is_empty());
    assert!(
        fixture
            .active_registry
            .snapshot()
            .get_extension(&extension_id)
            .is_some()
    );
}

#[tokio::test]
async fn registered_mcp_activation_commit_failure_restores_prior_cleanup_inventory() {
    let installation_store = Arc::new(SetActivationFailingStore::default());
    let fixture = RegisteredLifecycleFixture::with_stores(
        installation_store,
        Arc::new(InMemoryPersistentApprovalPolicyStore::new()),
        Arc::new(InMemoryCapabilityPermissionOverrideStore::new()),
    );
    let owner = UserId::new("registered-rollback-owner").expect("valid owner");
    let (scope, package_ref, extension_id) = fixture.register(&owner, "rollback-mcp").await;
    let prior_id =
        CapabilityId::new(format!("{}.prior", extension_id.as_str())).expect("prior capability id");
    RegisteredExtensionStore::replace_discovered_capability_ids_for_scope(
        fixture.port.filesystem.as_ref(),
        &scope,
        &extension_id,
        std::slice::from_ref(&prior_id),
    )
    .await
    .expect("seed prior inventory");

    let error = fixture
        .port
        .activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: scope.clone(),
                runtime_http_egress: Arc::new(HostedMcpDiscoveryEgress::default()),
                network_policy_authority: extension_activate_capability_id(),
            },
        )
        .await
        .expect_err("activation-state failure rejects activation");

    assert!(error.to_string().contains("set activation state failed"));
    assert_eq!(
        RegisteredExtensionStore::load_discovered_capability_ids_for_scope(
            fixture.port.filesystem.as_ref(),
            &scope,
            &extension_id,
        )
        .await
        .expect("load restored inventory"),
        vec![prior_id]
    );
    assert!(
        fixture
            .active_registry
            .snapshot()
            .get_extension(&extension_id)
            .is_none()
    );
}

#[tokio::test]
async fn registered_mcp_activation_inventory_restore_failure_still_restores_authority() {
    let installation_store = Arc::new(SetActivationFailingStore::default());
    let policies = Arc::new(InMemoryPersistentApprovalPolicyStore::new());
    let overrides = Arc::new(InMemoryCapabilityPermissionOverrideStore::new());
    let fixture = RegisteredLifecycleFixture::with_inventory_restore_failure(
        installation_store,
        policies.clone(),
        overrides.clone(),
    );
    let owner = UserId::new("registered-inventory-restore-owner").expect("valid owner");
    let (scope, package_ref, extension_id) = fixture.register(&owner, "inventory-restore").await;
    let prior_id =
        CapabilityId::new(format!("{}.prior", extension_id.as_str())).expect("prior capability id");
    RegisteredExtensionStore::replace_discovered_capability_ids_for_scope(
        fixture.port.filesystem.as_ref(),
        &scope,
        &extension_id,
        std::slice::from_ref(&prior_id),
    )
    .await
    .expect("seed prior inventory");
    let settings_scope = scope.tenant_user_settings_scope();
    let grantee = Principal::Extension(extension_id.clone());
    seed_authority(
        fixture.port.as_ref(),
        &settings_scope,
        &owner,
        &grantee,
        &prior_id,
    )
    .await;

    let error = fixture
        .port
        .activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: scope.clone(),
                runtime_http_egress: Arc::new(HostedMcpDiscoveryEgress::default()),
                network_policy_authority: extension_activate_capability_id(),
            },
        )
        .await
        .expect_err("activation-state failure rejects activation");
    assert!(
        error
            .to_string()
            .contains("capability inventory restore failed")
    );

    let policy = policies
        .lookup(&PersistentApprovalPolicyKey::new(
            &settings_scope,
            PersistentApprovalAction::Dispatch,
            prior_id.clone(),
            grantee,
        ))
        .await
        .expect("lookup restored policy")
        .expect("prior policy remains");
    assert!(
        policy.revoked_at.is_none(),
        "authority restore must run even when inventory restore fails"
    );
    assert!(
        overrides
            .get(&CapabilityPermissionOverrideKey::new(
                &settings_scope,
                prior_id
            ))
            .await
            .expect("lookup restored override")
            .is_some(),
        "permission override must be restored even when inventory restore fails"
    );
}

#[tokio::test]
async fn registered_mcp_refresh_commit_failure_restores_publication_and_authority() {
    let installation_store = Arc::new(SetActivationFailingStore::fail_on_second_enable());
    let policies = Arc::new(InMemoryPersistentApprovalPolicyStore::new());
    let overrides = Arc::new(InMemoryCapabilityPermissionOverrideStore::new());
    let fixture = RegisteredLifecycleFixture::with_stores(
        installation_store,
        policies.clone(),
        overrides.clone(),
    );
    let owner = UserId::new("registered-refresh-rollback-owner").expect("valid owner");
    let (scope, package_ref, extension_id) = fixture.register(&owner, "refresh-rollback").await;
    fixture
        .port
        .activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: scope.clone(),
                runtime_http_egress: Arc::new(HostedMcpDiscoveryEgress::default()),
                network_policy_authority: extension_activate_capability_id(),
            },
        )
        .await
        .expect("initial discovery succeeds");
    let active_before = fixture
        .active_registry
        .snapshot()
        .get_extension(&extension_id)
        .cloned()
        .expect("initial publication");
    let prior_ids = RegisteredExtensionStore::load_discovered_capability_ids_for_scope(
        fixture.port.filesystem.as_ref(),
        &scope,
        &extension_id,
    )
    .await
    .expect("load initial inventory");
    let prior_capability_id = prior_ids.first().cloned().expect("initial capability");
    let settings_scope = scope.tenant_user_settings_scope();
    let grantee = Principal::Extension(extension_id.clone());
    seed_authority(
        fixture.port.as_ref(),
        &settings_scope,
        &owner,
        &grantee,
        &prior_capability_id,
    )
    .await;

    let error = fixture
        .port
        .activate_with_prechecked_credentials_for_test(
            package_ref,
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: scope.clone(),
                runtime_http_egress: Arc::new(HostedMcpDiscoveryEgress::with_tool_name(
                    "refreshed-search",
                )),
                network_policy_authority: extension_activate_capability_id(),
            },
        )
        .await
        .expect_err("refresh commit failure must be surfaced");
    assert!(error.to_string().contains("set activation state failed"));

    assert_eq!(
        fixture
            .active_registry
            .snapshot()
            .get_extension(&extension_id)
            .expect("prior publication remains"),
        &active_before,
        "refresh failure must not replace the active package"
    );
    assert_eq!(
        RegisteredExtensionStore::load_discovered_capability_ids_for_scope(
            fixture.port.filesystem.as_ref(),
            &scope,
            &extension_id,
        )
        .await
        .expect("load restored inventory"),
        prior_ids
    );
    let policy = policies
        .lookup(&PersistentApprovalPolicyKey::new(
            &settings_scope,
            PersistentApprovalAction::Dispatch,
            prior_capability_id.clone(),
            grantee,
        ))
        .await
        .expect("lookup restored policy")
        .expect("prior policy remains");
    assert!(policy.revoked_at.is_none());
    assert!(
        overrides
            .get(&CapabilityPermissionOverrideKey::new(
                &settings_scope,
                prior_capability_id,
            ))
            .await
            .expect("lookup restored override")
            .is_some()
    );
}

#[tokio::test]
async fn registered_mcp_remove_failure_restores_exact_discovered_publication() {
    let installation_store = Arc::new(DeleteInstallationFailingStore::default());
    let fixture = RegisteredLifecycleFixture::with_stores(
        installation_store.clone(),
        Arc::new(InMemoryPersistentApprovalPolicyStore::new()),
        Arc::new(InMemoryCapabilityPermissionOverrideStore::new()),
    );
    let owner = UserId::new("registered-remove-rollback-owner").expect("valid owner");
    let (scope, package_ref, extension_id) = fixture.register(&owner, "rollback-tools").await;
    fixture
        .port
        .activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: scope.clone(),
                runtime_http_egress: Arc::new(HostedMcpDiscoveryEgress::default()),
                network_policy_authority: extension_activate_capability_id(),
            },
        )
        .await
        .expect("activate registered MCP");
    let active_before = fixture
        .active_registry
        .snapshot()
        .get_extension(&extension_id)
        .cloned()
        .expect("discovered package is active");
    assert!(!active_before.capabilities.is_empty());
    let active_capability_id = active_before.capabilities[0].id.clone();
    let settings_scope = scope.tenant_user_settings_scope();
    let grantee = Principal::Extension(extension_id.clone());
    seed_authority(
        fixture.port.as_ref(),
        &settings_scope,
        &owner,
        &grantee,
        &active_capability_id,
    )
    .await;

    let error = fixture
        .port
        .unregister_hosted_mcp(package_ref, &scope)
        .await
        .expect_err("injected installation delete failure rejects removal");

    assert!(error.to_string().contains("delete installation failed"));
    let installation_id =
        ExtensionInstallationId::new(extension_id.as_str().to_string()).expect("installation id");
    assert_eq!(
        installation_store
            .get_installation(&installation_id)
            .await
            .expect("read restored installation")
            .expect("installation remains")
            .activation_state(),
        ExtensionActivationState::Enabled,
        "failed removal must restore the enabled lifecycle state"
    );
    let active_after = fixture
        .active_registry
        .snapshot()
        .get_extension(&extension_id)
        .cloned()
        .expect("active publication is restored");
    assert_eq!(
        active_after.capabilities, active_before.capabilities,
        "rollback must restore the exact runtime-discovered tools"
    );
    let restored_policy = fixture
        .port
        .capability_authority_cleanup
        .persistent_approval_policies
        .lookup(&PersistentApprovalPolicyKey::new(
            &settings_scope,
            PersistentApprovalAction::Dispatch,
            active_capability_id.clone(),
            grantee,
        ))
        .await
        .expect("lookup restored policy");
    assert!(
        restored_policy.is_some_and(|policy| policy.revoked_at.is_none()),
        "rollback must restore the active dispatch policy"
    );
    assert!(
        fixture
            .port
            .capability_authority_cleanup
            .tool_permission_overrides
            .get(&CapabilityPermissionOverrideKey::new(
                &settings_scope,
                active_capability_id,
            ))
            .await
            .expect("lookup restored override")
            .is_some(),
        "rollback must restore the explicit permission override"
    );
}

#[tokio::test]
async fn registered_mcp_descriptor_delete_failure_restores_authority_and_inventory() {
    let (dir, port, _active_registry, _installation_store, _trust_policy) =
        extension_port_with_file_delete_failing_filesystem();
    let owner = UserId::new("registered-descriptor-rollback-owner").expect("valid owner");
    let scope = hosted_mcp_scope(owner.as_str());
    let registered = port
        .register_hosted_mcp(
            "Descriptor rollback MCP".to_string(),
            "https://93.184.216.34/descriptor-rollback".to_string(),
            &scope,
        )
        .await
        .expect("register MCP");
    let package_ref = registered.package_ref.expect("package ref");
    let extension_id = ExtensionId::new(package_ref.id.as_str()).expect("extension id");
    let capability_id =
        CapabilityId::new(format!("{}.search", extension_id.as_str())).expect("capability id");
    let inventory_dir = dir
        .path()
        .join("local-dev/system/extensions/registered-cleanup")
        .join(scope.tenant_id.as_str())
        .join(owner.as_str());
    std::fs::create_dir_all(inventory_dir.join(extension_id.as_str()))
        .expect("inventory directory");
    std::fs::write(
        inventory_dir.join(format!(
            "{}/discovered-capability-ids.json",
            extension_id.as_str()
        )),
        serde_json::json!({
            "schema_version": "registered_mcp.discovered_capability_ids.v1",
            "owner": owner.as_str(),
            "extension_id": extension_id.as_str(),
            "capability_ids": [capability_id.as_str()],
        })
        .to_string(),
    )
    .expect("seed inventory");
    let settings_scope = scope.tenant_user_settings_scope();
    let grantee = Principal::Extension(extension_id.clone());
    seed_authority(&port, &settings_scope, &owner, &grantee, &capability_id).await;

    let error = port
        .unregister_hosted_mcp(package_ref, &scope)
        .await
        .expect_err("descriptor delete failure must reject unregister");
    assert!(error.to_string().contains("delete failed"));
    let policy = port
        .capability_authority_cleanup
        .persistent_approval_policies
        .lookup(&PersistentApprovalPolicyKey::new(
            &settings_scope,
            PersistentApprovalAction::Dispatch,
            capability_id.clone(),
            grantee,
        ))
        .await
        .expect("lookup restored policy")
        .expect("policy remains");
    assert!(policy.revoked_at.is_none());
    assert!(
        port.capability_authority_cleanup
            .tool_permission_overrides
            .get(&CapabilityPermissionOverrideKey::new(
                &settings_scope,
                capability_id.clone(),
            ))
            .await
            .expect("lookup restored override")
            .is_some()
    );
    assert_eq!(
        RegisteredExtensionStore::load_discovered_capability_ids_for_scope(
            port.filesystem.as_ref(),
            &scope,
            &extension_id,
        )
        .await
        .expect("load preserved inventory"),
        vec![capability_id]
    );
    assert!(
        dir.path()
            .join("local-dev/system/extensions/registered")
            .join(scope.tenant_id.as_str())
            .join(owner.as_str())
            .join(extension_id.as_str())
            .join("manifest.toml")
            .exists()
    );
}

#[tokio::test]
async fn extension_unregister_cleans_inventory_and_exact_capability_authority() {
    let fixture = RegisteredLifecycleFixture::new();
    let owner = UserId::new("lifecycle-owner").expect("valid owner");
    let (scope, package_ref, extension_id) = fixture.register(&owner, "cleanup-mcp").await;
    fixture
        .port
        .activate_with_prechecked_credentials_for_test(
            package_ref.clone(),
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: scope.clone(),
                runtime_http_egress: Arc::new(HostedMcpDiscoveryEgress::default()),
                network_policy_authority: extension_activate_capability_id(),
            },
        )
        .await
        .expect("activate registered MCP");
    let active_package = fixture
        .active_registry
        .snapshot()
        .get_extension(&extension_id)
        .cloned()
        .expect("registered active package");
    let active_id = active_package
        .capabilities
        .first()
        .expect("discovered capability")
        .id
        .clone();
    let mut published_ids = active_package
        .capabilities
        .iter()
        .map(|capability| capability.id.clone())
        .collect::<Vec<_>>();
    published_ids.sort();
    assert_eq!(
        RegisteredExtensionStore::load_discovered_capability_ids_for_scope(
            fixture.port.filesystem.as_ref(),
            &scope,
            &extension_id,
        )
        .await
        .expect("load activation inventory"),
        published_ids,
        "activation inventory must include every discovered capability"
    );
    let stale_id =
        CapabilityId::new(format!("{}.stale", extension_id.as_str())).expect("stale capability id");
    let unrelated_id = CapabilityId::new("unrelated.keep").expect("unrelated capability id");
    RegisteredExtensionStore::replace_discovered_capability_ids_for_scope(
        fixture.port.filesystem.as_ref(),
        &scope,
        &extension_id,
        std::slice::from_ref(&stale_id),
    )
    .await
    .expect("replace inventory with stale id");

    let settings_scope = scope.tenant_user_settings_scope();
    let grantee = Principal::Extension(extension_id.clone());
    for capability_id in [&active_id, &stale_id, &unrelated_id] {
        seed_authority(
            &fixture.port,
            &settings_scope,
            &owner,
            &grantee,
            capability_id,
        )
        .await;
    }

    fixture
        .port
        .unregister_hosted_mcp(package_ref, &scope)
        .await
        .expect("unregister registered MCP");

    for removed_id in [&active_id, &stale_id] {
        let policy = fixture
            .port
            .capability_authority_cleanup
            .persistent_approval_policies
            .lookup(&PersistentApprovalPolicyKey::new(
                &settings_scope,
                PersistentApprovalAction::Dispatch,
                removed_id.clone(),
                grantee.clone(),
            ))
            .await
            .expect("lookup removed policy")
            .expect("removed policy remains as revoked audit record");
        assert!(policy.revoked_at.is_some());
        assert!(
            fixture
                .port
                .capability_authority_cleanup
                .tool_permission_overrides
                .get(&CapabilityPermissionOverrideKey::new(
                    &settings_scope,
                    removed_id.clone(),
                ))
                .await
                .expect("lookup removed override")
                .is_none()
        );
    }
    let unrelated_policy = fixture
        .port
        .capability_authority_cleanup
        .persistent_approval_policies
        .lookup(&PersistentApprovalPolicyKey::new(
            &settings_scope,
            PersistentApprovalAction::Dispatch,
            unrelated_id.clone(),
            grantee,
        ))
        .await
        .expect("lookup unrelated policy")
        .expect("unrelated policy remains");
    assert!(unrelated_policy.revoked_at.is_none());
    assert!(
        fixture
            .port
            .capability_authority_cleanup
            .tool_permission_overrides
            .get(&CapabilityPermissionOverrideKey::new(
                &settings_scope,
                unrelated_id,
            ))
            .await
            .expect("lookup unrelated override")
            .is_some()
    );
    assert!(
        RegisteredExtensionStore::load_discovered_capability_ids_for_scope(
            fixture.port.filesystem.as_ref(),
            &scope,
            &extension_id,
        )
        .await
        .expect("load deleted inventory")
        .is_empty()
    );
}

#[tokio::test]
async fn extension_unregister_authority_cleanup_failure_preserves_descriptor_and_inventory() {
    let fixture = RegisteredLifecycleFixture::with_stores(
        Arc::new(InMemoryExtensionInstallationStore::default()),
        Arc::new(RevokeFailingPolicyStore::default()),
        Arc::new(InMemoryCapabilityPermissionOverrideStore::new()),
    );
    let owner = UserId::new("cleanup-failure-owner").expect("valid owner");
    let (scope, package_ref, extension_id) = fixture.register(&owner, "cleanup-failure").await;
    let capability_id =
        CapabilityId::new(format!("{}.search", extension_id.as_str())).expect("capability id");
    RegisteredExtensionStore::replace_discovered_capability_ids_for_scope(
        fixture.port.filesystem.as_ref(),
        &scope,
        &extension_id,
        std::slice::from_ref(&capability_id),
    )
    .await
    .expect("seed cleanup inventory");

    let error = fixture
        .port
        .unregister_hosted_mcp(package_ref, &scope)
        .await
        .expect_err("approval cleanup failure must fail closed");

    assert!(
        error
            .to_string()
            .contains("injected policy cleanup failure")
    );
    assert!(
        fixture
            .storage_root
            .join("system/extensions/registered")
            .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
            .join(owner.as_str())
            .join(extension_id.as_str())
            .join("manifest.toml")
            .exists()
    );
    assert!(
        fixture
            .installation_store
            .get_installation(
                &ExtensionInstallationId::new(extension_id.as_str().to_string())
                    .expect("installation id")
            )
            .await
            .expect("read installation")
            .is_some()
    );
    assert_eq!(
        RegisteredExtensionStore::load_discovered_capability_ids_for_scope(
            fixture.port.filesystem.as_ref(),
            &scope,
            &extension_id,
        )
        .await
        .expect("load preserved inventory"),
        vec![capability_id]
    );
}

#[tokio::test]
async fn same_name_replacement_survives_prior_cleanup_inventory_delete_failure() {
    let fixture = RegisteredLifecycleFixture::with_cleanup_inventory_delete_failure();
    let owner = UserId::new("replacement-owner").expect("valid owner");
    let scope = hosted_mcp_scope(owner.as_str());
    let prior = fixture
        .port
        .register_hosted_mcp(
            "Stable MCP".to_string(),
            "https://93.184.216.34/prior".to_string(),
            &scope,
        )
        .await
        .expect("register prior descriptor");
    let prior_ref = prior.package_ref.expect("prior package ref");

    let replacement = fixture
        .port
        .register_hosted_mcp(
            "Stable MCP".to_string(),
            "https://93.184.216.34/replacement".to_string(),
            &scope,
        )
        .await
        .expect("post-commit cleanup failure must not reject replacement");
    let replacement_ref = replacement.package_ref.expect("replacement package ref");

    assert_ne!(prior_ref, replacement_ref);
    assert!(
        fixture
            .installation_store
            .get_installation(
                &ExtensionInstallationId::new(replacement_ref.id.as_str().to_string())
                    .expect("replacement installation id")
            )
            .await
            .expect("read replacement installation")
            .is_some(),
        "the committed replacement must remain installed"
    );
    assert!(
        fixture
            .installation_store
            .get_installation(
                &ExtensionInstallationId::new(prior_ref.id.as_str().to_string())
                    .expect("prior installation id")
            )
            .await
            .expect("read prior installation")
            .is_none(),
        "the prior registration was already committed removed"
    );
    assert!(
        fixture
            .storage_root
            .join("system/extensions/registered")
            .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
            .join(owner.as_str())
            .join(replacement_ref.id.as_str())
            .join("manifest.toml")
            .exists(),
        "replacement descriptor remains durable"
    );
    assert!(
        !fixture
            .storage_root
            .join("system/extensions/registered")
            .join(ironclaw_host_api::LOCAL_DEFAULT_TENANT_ID)
            .join(owner.as_str())
            .join(prior_ref.id.as_str())
            .exists(),
        "prior descriptor remains removed"
    );
}

#[tokio::test]
async fn extension_list_includes_own_registered_and_static_but_hides_other_owner() {
    let fixture = RegisteredLifecycleFixture::with_catalog(
        AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog"),
    );
    let owner_a = UserId::new("list-owner-a").expect("valid owner A");
    let owner_b = UserId::new("list-owner-b").expect("valid owner B");
    let (scope_a, owner_a_ref, _) = fixture.register(&owner_a, "owner-a-tools").await;
    let (scope_b, owner_b_ref, _) = fixture.register(&owner_b, "owner-b-tools").await;
    let static_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("static ref");
    fixture
        .port
        .install(static_ref.clone(), Some(&scope_a))
        .await
        .expect("install static extension");

    let owner_a_ids = listed_extension_ids(
        fixture
            .port
            .list_installed(&scope_a)
            .await
            .expect("list owner A extensions"),
    );
    assert!(owner_a_ids.contains(owner_a_ref.id.as_str()));
    assert!(!owner_a_ids.contains(owner_b_ref.id.as_str()));
    assert!(owner_a_ids.contains(static_ref.id.as_str()));

    let owner_b_ids = listed_extension_ids(
        fixture
            .port
            .list_installed(&scope_b)
            .await
            .expect("list owner B extensions"),
    );
    assert!(owner_b_ids.contains(owner_b_ref.id.as_str()));
    assert!(!owner_b_ids.contains(owner_a_ref.id.as_str()));
    assert!(owner_b_ids.contains(static_ref.id.as_str()));
}

#[tokio::test]
async fn same_user_different_tenants_cannot_cross_registered_mcp_lifecycle() {
    let fixture = RegisteredLifecycleFixture::new();
    let user = UserId::new("same-user").expect("valid user");
    let mut tenant_a = hosted_mcp_scope(user.as_str());
    tenant_a.tenant_id = TenantId::new("tenant-a").expect("valid tenant A");
    let mut tenant_b = hosted_mcp_scope(user.as_str());
    tenant_b.tenant_id = TenantId::new("tenant-b").expect("valid tenant B");

    let package_a = fixture
        .port
        .register_hosted_mcp(
            "Tenant A MCP".to_string(),
            "https://93.184.216.34/tenant-a".to_string(),
            &tenant_a,
        )
        .await
        .expect("register tenant A MCP")
        .package_ref
        .expect("tenant A package ref");
    let package_b = fixture
        .port
        .register_hosted_mcp(
            "Tenant B MCP".to_string(),
            "https://93.184.216.34/tenant-b".to_string(),
            &tenant_b,
        )
        .await
        .expect("register tenant B MCP")
        .package_ref
        .expect("tenant B package ref");
    assert_ne!(
        package_a, package_b,
        "tenant must participate in package identity"
    );

    let tenant_a_ids = listed_extension_ids(
        fixture
            .port
            .list_installed(&tenant_a)
            .await
            .expect("list tenant A extensions"),
    );
    let tenant_b_ids = listed_extension_ids(
        fixture
            .port
            .list_installed(&tenant_b)
            .await
            .expect("list tenant B extensions"),
    );
    assert!(tenant_a_ids.contains(package_a.id.as_str()));
    assert!(!tenant_a_ids.contains(package_b.id.as_str()));
    assert!(tenant_b_ids.contains(package_b.id.as_str()));
    assert!(!tenant_b_ids.contains(package_a.id.as_str()));

    let cross_tenant_egress = Arc::new(HostedMcpDiscoveryEgress::default());
    let error = fixture
        .port
        .activate_with_prechecked_credentials_for_test(
            package_a.clone(),
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: tenant_b.clone(),
                runtime_http_egress: cross_tenant_egress.clone(),
                network_policy_authority: extension_activate_capability_id(),
            },
        )
        .await
        .expect_err("tenant B must not activate tenant A's MCP");
    assert!(
        error
            .to_string()
            .contains("available extension was not found")
    );
    assert!(
        cross_tenant_egress.methods().is_empty(),
        "cross-tenant activation must not egress"
    );

    fixture
        .port
        .activate_with_prechecked_credentials_for_test(
            package_a.clone(),
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: tenant_a.clone(),
                runtime_http_egress: Arc::new(HostedMcpDiscoveryEgress::default()),
                network_policy_authority: extension_activate_capability_id(),
            },
        )
        .await
        .expect("tenant A activates its MCP");

    let error = fixture
        .port
        .unregister_hosted_mcp(package_a.clone(), &tenant_b)
        .await
        .expect_err("tenant B must not unregister tenant A's MCP");
    assert!(
        error
            .to_string()
            .contains("available extension was not found")
    );
    assert!(
        listed_extension_ids(
            fixture
                .port
                .list_installed(&tenant_a)
                .await
                .expect("list tenant A after rejected unregister")
        )
        .contains(package_a.id.as_str()),
        "cross-tenant unregister must not remove tenant A's installation"
    );
    let extension_a = ExtensionId::new(package_a.id.as_str()).expect("tenant A extension id");
    assert!(
        fixture
            .active_registry
            .snapshot()
            .get_extension(&extension_a)
            .is_some(),
        "cross-tenant unregister must not unpublish tenant A's tools"
    );
}

fn listed_extension_ids(response: LifecycleProductResponse) -> BTreeSet<String> {
    let Some(LifecycleProductPayload::ExtensionList { extensions, .. }) = response.payload else {
        panic!("expected extension list payload");
    };
    extensions
        .into_iter()
        .map(|extension| extension.summary.package_ref.id.as_str().to_string())
        .collect()
}

async fn seed_authority(
    port: &RebornLocalExtensionManagementPort,
    scope: &ResourceScope,
    owner: &UserId,
    grantee: &Principal,
    capability_id: &CapabilityId,
) {
    port.capability_authority_cleanup
        .persistent_approval_policies
        .allow(PersistentApprovalPolicyInput {
            scope: scope.clone(),
            action: PersistentApprovalAction::Dispatch,
            capability_id: capability_id.clone(),
            grantee: grantee.clone(),
            approved_by: Principal::User(owner.clone()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::ExternalWrite],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
            source_approval_request_id: None,
        })
        .await
        .expect("seed persistent approval");
    port.capability_authority_cleanup
        .tool_permission_overrides
        .set(CapabilityPermissionOverrideInput {
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            state: CapabilityPermissionOverride::Disabled,
            updated_by: Principal::User(owner.clone()),
        })
        .await
        .expect("seed permission override");
}

fn hosted_mcp_scope(user_id: &str) -> ResourceScope {
    ResourceScope::local_default(
        UserId::new(user_id).expect("valid user"),
        InvocationId::new(),
    )
    .expect("valid local scope")
}

fn extension_activate_capability_id() -> CapabilityId {
    CapabilityId::new(
        crate::extension_host::extension_lifecycle_capabilities::EXTENSION_ACTIVATE_CAPABILITY_ID,
    )
    .expect("valid extension activation capability id")
}

#[derive(Default)]
struct RevokeFailingPolicyStore {
    inner: InMemoryPersistentApprovalPolicyStore,
}

#[async_trait]
impl PersistentApprovalPolicyStore for RevokeFailingPolicyStore {
    async fn allow(
        &self,
        input: PersistentApprovalPolicyInput,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError> {
        self.inner.allow(input).await
    }

    async fn lookup(
        &self,
        key: &PersistentApprovalPolicyKey,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        self.inner.lookup(key).await
    }

    async fn revoke(
        &self,
        _key: &PersistentApprovalPolicyKey,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError> {
        Err(PersistentApprovalPolicyError::Filesystem(
            "injected policy cleanup failure".to_string(),
        ))
    }

    async fn revoke_if_source_approval_request(
        &self,
        key: &PersistentApprovalPolicyKey,
        source_approval_request_id: ApprovalRequestId,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        self.inner
            .revoke_if_source_approval_request(key, source_approval_request_id)
            .await
    }
}

struct SetActivationFailingStore {
    inner: InMemoryExtensionInstallationStore,
    fail_on_second_enable: bool,
    enable_calls: AtomicUsize,
}

impl Default for SetActivationFailingStore {
    fn default() -> Self {
        Self {
            inner: InMemoryExtensionInstallationStore::default(),
            fail_on_second_enable: false,
            enable_calls: AtomicUsize::new(0),
        }
    }
}

impl SetActivationFailingStore {
    fn fail_on_second_enable() -> Self {
        Self {
            inner: InMemoryExtensionInstallationStore::default(),
            fail_on_second_enable: true,
            enable_calls: AtomicUsize::new(0),
        }
    }
}

#[derive(Default)]
struct DeleteInstallationFailingStore {
    inner: InMemoryExtensionInstallationStore,
}

#[async_trait]
impl ExtensionInstallationStore for DeleteInstallationFailingStore {
    async fn list_manifests(
        &self,
    ) -> Result<Vec<ExtensionManifestRecord>, ExtensionInstallationError> {
        self.inner.list_manifests().await
    }

    async fn get_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionManifestRecord>, ExtensionInstallationError> {
        self.inner.get_manifest(extension_id).await
    }

    async fn upsert_manifest(
        &self,
        manifest: ExtensionManifestRecord,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner.upsert_manifest(manifest).await
    }

    async fn upsert_manifest_and_installation(
        &self,
        manifest: ExtensionManifestRecord,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner
            .upsert_manifest_and_installation(manifest, installation)
            .await
    }

    async fn list_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        self.inner.list_installations().await
    }

    async fn list_enabled_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        self.inner.list_enabled_installations().await
    }

    async fn get_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<ExtensionInstallation>, ExtensionInstallationError> {
        self.inner.get_installation(installation_id).await
    }

    async fn upsert_installation(
        &self,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner.upsert_installation(installation).await
    }

    async fn set_activation_state(
        &self,
        installation_id: &ExtensionInstallationId,
        state: ExtensionActivationState,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner
            .set_activation_state(installation_id, state)
            .await
    }

    async fn delete_installation(
        &self,
        _installation_id: &ExtensionInstallationId,
    ) -> Result<(), ExtensionInstallationError> {
        Err(ExtensionInstallationError::InvalidInstallation {
            reason: "delete installation failed".to_string(),
        })
    }

    async fn delete_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner.delete_manifest(extension_id).await
    }

    async fn update_health(
        &self,
        installation_id: &ExtensionInstallationId,
        health: ExtensionHealthSnapshot,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner.update_health(installation_id, health).await
    }
}

#[async_trait]
impl ExtensionInstallationStore for SetActivationFailingStore {
    async fn list_manifests(
        &self,
    ) -> Result<Vec<ExtensionManifestRecord>, ExtensionInstallationError> {
        self.inner.list_manifests().await
    }

    async fn get_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionManifestRecord>, ExtensionInstallationError> {
        self.inner.get_manifest(extension_id).await
    }

    async fn upsert_manifest(
        &self,
        manifest: ExtensionManifestRecord,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner.upsert_manifest(manifest).await
    }

    async fn upsert_manifest_and_installation(
        &self,
        manifest: ExtensionManifestRecord,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner
            .upsert_manifest_and_installation(manifest, installation)
            .await
    }

    async fn list_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        self.inner.list_installations().await
    }

    async fn list_enabled_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        self.inner.list_enabled_installations().await
    }

    async fn get_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<ExtensionInstallation>, ExtensionInstallationError> {
        self.inner.get_installation(installation_id).await
    }

    async fn upsert_installation(
        &self,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner.upsert_installation(installation).await
    }

    async fn set_activation_state(
        &self,
        installation_id: &ExtensionInstallationId,
        state: ExtensionActivationState,
    ) -> Result<(), ExtensionInstallationError> {
        let should_fail = state == ExtensionActivationState::Enabled
            && (!self.fail_on_second_enable
                || self.enable_calls.fetch_add(1, Ordering::SeqCst) >= 1);
        if should_fail {
            return Err(ExtensionInstallationError::InvalidInstallation {
                reason: "set activation state failed".to_string(),
            });
        }
        self.inner
            .set_activation_state(installation_id, state)
            .await
    }

    async fn delete_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner.delete_installation(installation_id).await
    }

    async fn delete_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner.delete_manifest(extension_id).await
    }

    async fn update_health(
        &self,
        installation_id: &ExtensionInstallationId,
        health: ExtensionHealthSnapshot,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner.update_health(installation_id, health).await
    }
}

struct CleanupInventoryDeleteFailingFilesystem {
    inner: Arc<dyn RootFilesystem>,
}

#[async_trait]
impl RootFilesystem for CleanupInventoryDeleteFailingFilesystem {
    fn capabilities(&self) -> ironclaw_filesystem::BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.inner.read_file(path).await
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.inner.write_file(path, bytes).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        if path
            .as_str()
            .starts_with("/system/extensions/registered-cleanup/")
        {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::Delete,
                reason: "injected cleanup inventory delete failure".to_string(),
            });
        }
        self.inner.delete(path).await
    }
}

struct InventoryRestoreFailingFilesystem {
    inner: Arc<dyn RootFilesystem>,
    cleanup_inventory_puts: AtomicUsize,
}

#[async_trait]
impl RootFilesystem for InventoryRestoreFailingFilesystem {
    fn capabilities(&self) -> ironclaw_filesystem::BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        if path
            .as_str()
            .starts_with("/system/extensions/registered-cleanup/")
            && self.cleanup_inventory_puts.fetch_add(1, Ordering::SeqCst) == 2
        {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::WriteFile,
                reason: "injected cleanup inventory restore failure".to_string(),
            });
        }
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
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

struct EmptyToolsHostedMcpEgress;

#[async_trait]
impl RuntimeHttpEgress for EmptyToolsHostedMcpEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        if request.method != NetworkMethod::Post {
            return Err(egress_error(&request, "unexpected_method"));
        }
        let body: serde_json::Value = serde_json::from_slice(&request.body)
            .map_err(|_| egress_error(&request, "invalid_json"))?;
        let method = body
            .get("method")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| egress_error(&request, "missing_method"))?;
        let result = match method {
            "initialize" => serde_json::json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "empty-test", "version": "1.0.0"}
            }),
            "notifications/initialized" => serde_json::json!({}),
            "tools/list" => serde_json::json!({"tools": []}),
            _ => return Err(egress_error(&request, "unexpected_method")),
        };
        let response_body = serde_json::to_vec(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": body.get("id").cloned().unwrap_or(serde_json::Value::Null),
            "result": result,
        }))
        .map_err(|_| egress_error(&request, "serialize_response"))?;
        Ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            response_bytes: response_body.len() as u64,
            body: response_body,
            saved_body: None,
            request_bytes: request.body.len() as u64,
            redaction_applied: false,
        })
    }
}

fn egress_error(request: &RuntimeHttpEgressRequest, reason: &str) -> RuntimeHttpEgressError {
    RuntimeHttpEgressError::Request {
        reason: reason.to_string(),
        request_bytes: request.body.len() as u64,
        response_bytes: 0,
    }
}
