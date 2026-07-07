//! Private-install ownership behavior tests (#5459 P1, #5525 review),
//! driven through the lifecycle facade/port like every production caller.
//! Split out of the parent test module, whose shared fixtures it reuses
//! via `super::*`.

use super::*;
use ironclaw_product_workflow::LifecycleInstallScope;

/// #5459 P1 slot rules, driven through the facade (the caller surface the
/// WebUI and agent-tool paths both enter):
/// - a member's install is PRIVATE: invisible in others' lists, and
///   activate/remove/install by others fail without leaking that (or
///   whose) a private install exists
/// - a member cannot remove a TENANT-shared tool
/// - a tenant (operator) install EVICTS the private install (admin-wins),
///   after which everyone sees the shared tool
#[tokio::test]
async fn private_install_slot_rules_and_admin_eviction() {
    let (_dir, _root, facade, _registry, installation_store) = extension_lifecycle_fixture();
    let fixture_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").expect("fixture ref");
    let installation_id = ExtensionInstallationId::new("fixture").expect("fixture installation id");

    // alice (member) installs → owner is User(alice) in the store.
    let response = facade
        .execute(
            lifecycle_surface_context_for_user("alice"),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_ref.clone(),
            },
        )
        .await
        .expect("alice installs privately");
    assert_eq!(response.phase, LifecyclePhase::Installed);
    let installation = installation_store
        .get_installation(&installation_id)
        .await
        .expect("store read")
        .expect("installation row");
    assert_eq!(
        installation.owner().as_user().map(UserId::as_str),
        Some("alice"),
        "member install must be user-owned"
    );

    // bob's list is empty; alice's list shows a PRIVATE entry.
    let bob_list = facade
        .execute(
            lifecycle_surface_context_for_user("bob"),
            LifecycleProductAction::ExtensionList,
        )
        .await
        .expect("bob lists");
    let Some(LifecycleProductPayload::ExtensionList { count: 0, .. }) = bob_list.payload.as_ref()
    else {
        panic!("alice's private install must be invisible to bob: {bob_list:?}");
    };
    let alice_list = facade
        .execute(
            lifecycle_surface_context_for_user("alice"),
            LifecycleProductAction::ExtensionList,
        )
        .await
        .expect("alice lists");
    let Some(LifecycleProductPayload::ExtensionList {
        extensions,
        count: 1,
    }) = alice_list.payload.as_ref()
    else {
        panic!("alice must see her own install: {alice_list:?}");
    };
    assert_eq!(
        extensions[0].install_scope,
        Some(LifecycleInstallScope::Private)
    );

    // bob cannot claim the slot — and the error must not leak the owner.
    let error = facade
        .execute(
            lifecycle_surface_context_for_user("bob"),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_ref.clone(),
            },
        )
        .await
        .expect_err("bob cannot claim an id held by another user's private install");
    let rendered = error.to_string();
    assert!(rendered.contains("unavailable"), "unexpected: {rendered}");
    assert!(
        !rendered.contains("alice"),
        "slot error must not leak the private owner: {rendered}"
    );

    // bob cannot activate or remove it — reads as not installed. The
    // tenant operator gets the same masking: a private tool is invisible
    // and non-dispatchable to every non-owner, admin included — the
    // admin's only special power is the shared-install eviction below.
    for context in [
        lifecycle_surface_context_for_user("bob"),
        lifecycle_surface_context(),
    ] {
        for action in [
            LifecycleProductAction::ExtensionActivate {
                package_ref: fixture_ref.clone(),
            },
            LifecycleProductAction::ExtensionRemove {
                package_ref: fixture_ref.clone(),
            },
        ] {
            let error = facade
                .execute(context.clone(), action)
                .await
                .expect_err("foreign private install must be inoperable");
            assert!(
                error.to_string().contains("is not installed"),
                "unexpected: {error}"
            );
        }
    }

    // alice activates her private install.
    let response = facade
        .execute(
            lifecycle_surface_context_for_user("alice"),
            LifecycleProductAction::ExtensionActivate {
                package_ref: fixture_ref.clone(),
            },
        )
        .await
        .expect("alice activates her private install");
    assert_eq!(response.phase, LifecyclePhase::Active);

    // The operator installs the same id → evicts alice's private install.
    let response = facade
        .execute(
            lifecycle_surface_context(),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_ref.clone(),
            },
        )
        .await
        .expect("tenant install evicts the private install");
    assert_eq!(response.phase, LifecyclePhase::Installed);
    let installation = installation_store
        .get_installation(&installation_id)
        .await
        .expect("store read")
        .expect("installation row");
    assert!(
        installation.owner().is_tenant(),
        "tenant install must own the slot after eviction"
    );

    // Everyone now sees the SHARED entry — bob included.
    let bob_list = facade
        .execute(
            lifecycle_surface_context_for_user("bob"),
            LifecycleProductAction::ExtensionList,
        )
        .await
        .expect("bob lists after eviction");
    let Some(LifecycleProductPayload::ExtensionList {
        extensions,
        count: 1,
    }) = bob_list.payload.as_ref()
    else {
        panic!("shared install must be visible to bob: {bob_list:?}");
    };
    assert_eq!(
        extensions[0].install_scope,
        Some(LifecycleInstallScope::Shared)
    );

    // Members (alice included) cannot remove the shared tool; the operator can.
    for member in ["alice", "bob"] {
        let error = facade
            .execute(
                lifecycle_surface_context_for_user(member),
                LifecycleProductAction::ExtensionRemove {
                    package_ref: fixture_ref.clone(),
                },
            )
            .await
            .expect_err("members cannot remove a shared tool");
        assert!(
            error.to_string().contains("only the tenant admin"),
            "unexpected: {error}"
        );
    }
    facade
        .execute(
            lifecycle_surface_context(),
            LifecycleProductAction::ExtensionRemove {
                package_ref: fixture_ref,
            },
        )
        .await
        .expect("operator removes the shared tool");
}

/// #5525 review: `LifecycleProductCommandService` dispatches every
/// `/extension_*` command as `LifecycleProductContext::Command`, so the
/// facade must derive the caller from the verified command auth claim
/// instead of rejecting non-surface contexts outright — and the private
/// ownership masking must hold on the command path too.
#[tokio::test]
async fn extension_lifecycle_commands_derive_caller_from_command_auth_claim() {
    let (_dir, _root, facade, _registry, installation_store) = extension_lifecycle_fixture();
    let fixture_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").expect("fixture ref");
    let installation_id = ExtensionInstallationId::new("fixture").expect("fixture installation id");

    // alice installs through the command path → owner derives from the claim.
    let response = facade
        .execute(
            lifecycle_command_context_for_user("alice"),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_ref.clone(),
            },
        )
        .await
        .expect("command install derives caller from auth claim");
    assert_eq!(response.phase, LifecyclePhase::Installed);
    let installation = installation_store
        .get_installation(&installation_id)
        .await
        .expect("store read")
        .expect("installation row");
    assert_eq!(
        installation.owner().as_user().map(UserId::as_str),
        Some("alice"),
        "command install must be owned by the claim subject"
    );

    // alice's command list sees it; bob's command list stays masked.
    let alice_list = facade
        .execute(
            lifecycle_command_context_for_user("alice"),
            LifecycleProductAction::ExtensionList,
        )
        .await
        .expect("alice lists via command");
    let Some(LifecycleProductPayload::ExtensionList { count: 1, .. }) = alice_list.payload.as_ref()
    else {
        panic!("alice must see her install via command: {alice_list:?}");
    };
    let bob_list = facade
        .execute(
            lifecycle_command_context_for_user("bob"),
            LifecycleProductAction::ExtensionList,
        )
        .await
        .expect("bob lists via command");
    let Some(LifecycleProductPayload::ExtensionList { count: 0, .. }) = bob_list.payload.as_ref()
    else {
        panic!("alice's private install must stay invisible on the command path: {bob_list:?}");
    };

    // Owner masking holds for command-path mutations by a non-owner.
    let error = facade
        .execute(
            lifecycle_command_context_for_user("bob"),
            LifecycleProductAction::ExtensionActivate {
                package_ref: fixture_ref.clone(),
            },
        )
        .await
        .expect_err("foreign private install must be inoperable via command");
    assert!(
        error.to_string().contains("is not installed"),
        "unexpected: {error}"
    );

    // alice activates and removes her install through the command path.
    let response = facade
        .execute(
            lifecycle_command_context_for_user("alice"),
            LifecycleProductAction::ExtensionActivate {
                package_ref: fixture_ref.clone(),
            },
        )
        .await
        .expect("alice activates via command");
    assert_eq!(response.phase, LifecyclePhase::Active);
    facade
        .execute(
            lifecycle_command_context_for_user("alice"),
            LifecycleProductAction::ExtensionRemove {
                package_ref: fixture_ref,
            },
        )
        .await
        .expect("alice removes via command");
}

/// #5459 P1: the owner join in `active_model_visible_capabilities` — a
/// privately installed+activated extension's capabilities carry the
/// owning user, which is what the grant-minting filter keys on.
#[tokio::test]
async fn active_capabilities_carry_installation_owner() {
    let (_dir, _root, port, _registry, _store) =
        extension_management_port_fixture_with_catalog_and_service(
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            ExtensionLifecycleService::new(ExtensionRegistry::new()),
        );
    let alice = UserId::new("alice").expect("valid user");
    let package_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").expect("fixture ref");
    port.install(package_ref.clone(), &alice)
        .await
        .expect("alice installs privately");
    port.activate(package_ref, ExtensionActivationMode::Static, &alice)
        .await
        .expect("alice activates");

    let capabilities = port
        .active_model_visible_capabilities()
        .await
        .expect("active capabilities");
    assert!(!capabilities.is_empty(), "fixture capability published");
    for capability in &capabilities {
        assert_eq!(
            capability.owner.as_user().map(UserId::as_str),
            Some("alice"),
            "capability must carry the private owner for grant filtering"
        );
    }

    // The operator/settings tool catalog joins THIS owner map to hide a
    // foreign user's private tool (#5459 P1 leak fix). Pin that the map
    // reports the private owner keyed by extension id.
    let owners = port
        .installation_owners()
        .await
        .expect("installation owners");
    assert_eq!(
        owners
            .get(&ExtensionId::new("fixture").unwrap())
            .and_then(InstallationOwner::as_user)
            .map(UserId::as_str),
        Some("alice"),
        "installation_owners must report the private owner the catalog filters on"
    );
}

/// #5459 P1 (should-fix): a tenant install that fails AFTER eviction has
/// deregistered the private package must not brick the id tenant-wide.
/// Eviction is retry-safe, so the admin's retry heals the slot to Tenant.
#[tokio::test]
async fn admin_install_retry_heals_after_eviction_then_persist_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    std::fs::create_dir_all(storage_root.join("system/extensions")).expect("storage root");
    let mut filesystem = LocalFilesystem::new();
    filesystem
        .mount_local(
            VirtualPath::new("/projects").expect("valid virtual path"),
            HostPath::from_path_buf(storage_root.clone()),
        )
        .expect("mount storage root");
    filesystem
        .mount_local(
            VirtualPath::new("/system/extensions").expect("valid virtual path"),
            HostPath::from_path_buf(storage_root.join("system/extensions")),
        )
        .expect("mount system extensions");
    let root_filesystem: Arc<dyn RootFilesystem> = Arc::new(filesystem);
    let active_registry = Arc::new(SharedExtensionRegistry::new(ExtensionRegistry::new()));
    // Concrete Arc so the test can arm the one-shot persist failure AFTER
    // alice's install; the port sees it as `dyn ExtensionInstallationStore`.
    let store = Arc::new(DeleteInstallationFailingStore::default());
    let store_dyn: Arc<dyn ExtensionInstallationStore> = store.clone();
    let port = RebornLocalExtensionManagementPort::new(
        root_filesystem,
        AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
        store_dyn,
        Arc::new(Mutex::new(ExtensionLifecycleService::new(
            ExtensionRegistry::new(),
        ))),
        test_active_extension_publisher(
            Arc::clone(&active_registry),
            test_extension_trust_policy(),
        ),
        None,
        lifecycle_owner(),
    );

    let alice = UserId::new("alice").expect("user");
    let package_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").expect("fixture ref");

    // alice privately installs + activates (Enabled, package published).
    port.install(package_ref.clone(), &alice)
        .await
        .expect("alice installs privately");
    port.activate(package_ref.clone(), ExtensionActivationMode::Static, &alice)
        .await
        .expect("alice activates");

    // Admin install fails at persist (upsert_installation), AFTER eviction
    // has already deregistered alice's package.
    store
        .fail_next_upsert_installation
        .store(true, std::sync::atomic::Ordering::SeqCst);
    port.install(package_ref.clone(), &lifecycle_owner())
        .await
        .expect_err("persist failure aborts the tenant install");

    // #5525 review (non-interference): the FAILED shared install must
    // leave alice's private install exactly as it was — owner alice, row
    // still Enabled, and her capability still published — not disabled/
    // unpublished until an admin retry shows up.
    let fixture_id = ExtensionId::new("fixture").expect("extension id");
    let owners = port.installation_owners().await.expect("owners");
    assert_eq!(
        owners
            .get(&fixture_id)
            .and_then(InstallationOwner::as_user)
            .map(UserId::as_str),
        Some("alice"),
        "failed tenant install must restore alice's private ownership"
    );
    let installation = store
        .get_installation(&ExtensionInstallationId::new("fixture").expect("installation id"))
        .await
        .expect("store read")
        .expect("installation row survives the failed tenant install");
    assert_eq!(
        installation.activation_state(),
        ExtensionActivationState::Enabled,
        "failed tenant install must re-enable the evicted private install"
    );
    assert!(
        active_registry
            .snapshot()
            .get_extension(&fixture_id)
            .is_some(),
        "failed tenant install must re-publish the evicted private capability"
    );

    // Retry: eviction re-runs against the restored private install and the
    // slot heals to a tenant-owned install rather than dead-ending on
    // 'not installed'.
    port.install(package_ref, &lifecycle_owner())
        .await
        .expect("admin retry heals the slot after a failed eviction install");

    let owners = port.installation_owners().await.expect("owners");
    assert!(
        owners
            .get(&ExtensionId::new("fixture").unwrap())
            .expect("fixture installed")
            .is_tenant(),
        "the slot must heal to a tenant install, not stay bricked"
    );
}

/// Command-path context whose verified auth claim subject is `user` —
/// mirrors `LifecycleProductCommandService` dispatching `/extension_*`
/// commands as `LifecycleProductContext::Command`.
fn lifecycle_command_context_for_user(user: &str) -> LifecycleProductContext {
    use ironclaw_product_adapters::{
        AdapterInstallationId, AuthRequirement, ExternalActorRef, ExternalConversationRef,
        ExternalEventId, ProductAdapterId, ProductTriggerReason, ProtocolAuthEvidence,
    };
    use ironclaw_product_workflow::{
        ActionFingerprintKey, ProductActionId, ProductCommandContext, SourceBindingKey,
    };

    let adapter_id = ProductAdapterId::new("test_adapter").expect("valid adapter");
    let installation_id = AdapterInstallationId::new("install_alpha").expect("valid installation");
    let actor = ExternalActorRef::new("test", user, Option::<String>::None).expect("valid actor");
    let conversation =
        ExternalConversationRef::new(None, "conv1", None, None).expect("valid conversation");
    let auth_claim = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Secret".into(),
        },
        user,
    )
    .claim()
    .cloned()
    .expect("verified claim");
    LifecycleProductContext::Command(Box::new(ProductCommandContext {
        action_id: ProductActionId::new(),
        fingerprint: ActionFingerprintKey::new(
            adapter_id.clone(),
            installation_id.clone(),
            actor.clone(),
            SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;").expect("binding key"),
            ExternalEventId::new("evt:lifecycle-command").expect("valid event"),
        ),
        adapter_id,
        installation_id,
        external_actor_ref: actor,
        external_conversation_ref: conversation,
        auth_claim,
        trigger: ProductTriggerReason::BotCommand,
        received_at: chrono::Utc::now(),
    }))
}
