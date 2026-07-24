// arch-exempt: large_file, pre-existing >1500-line factory test module; this PR only adds the mandatory `owner` field to an outbound-target entry fixture for the registry caller-scoping hardening, plan #6389
use super::*;
use ironclaw_approvals::{AutoApproveSettingInput, AutoApproveSettingStore};
use ironclaw_auth::{
    AuthProductScope, AuthSurface, CredentialAccountLabel, CredentialAccountStatus,
    CredentialOwnership, GOOGLE_CALENDAR_EVENTS_SCOPE, GOOGLE_GMAIL_SEND_SCOPE,
    NewCredentialAccount, ProviderScope,
};
use ironclaw_authorization::{CapabilityLeaseStatus, CapabilityLeaseStore, GrantAuthorizer};
use ironclaw_filesystem::FilesystemError;
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind, ExecutionContext,
    ExtensionId, GrantConstraints, InvocationId, MountAlias, MountGrant, MountPermissions,
    NetworkPolicy, NetworkScheme, NetworkTargetPattern, Principal, ResourceEstimate, ResourceScope,
    ResourceUsage, RunId, RuntimeKind, ScopedPath, SecretHandle, TenantId, TrustClass, UserId,
    VirtualPath,
};
use ironclaw_host_api::{
    RuntimeCredentialAccountSetup, RuntimeCredentialRequirementSource, VendorId,
};
use ironclaw_host_runtime::{
    MEMORY_SEARCH_CAPABILITY_ID, MEMORY_TREE_CAPABILITY_ID, MEMORY_WRITE_CAPABILITY_ID,
    RuntimeCapabilityOutcome, RuntimeFailureKind, SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID,
    SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY_ID,
    SKILL_UPDATE_CAPABILITY_ID, TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID,
    TRIGGER_REMOVE_CAPABILITY_ID,
};
use ironclaw_host_runtime::{RuntimeCredentialAccountRequest, RuntimeCredentialAccountResolver};
use ironclaw_product::{LifecyclePackageKind, LifecyclePackageRef, LifecyclePublicState};

use rust_decimal_macros::dec;
use secrecy::ExposeSecret;

use crate::builtin_capability_policy::{BuiltinApprovalPolicyAction, BuiltinCapabilityPolicyError};
use crate::extension_host::extension_lifecycle::ExtensionActivationMode;
use crate::extension_host::extension_lifecycle::hosted_mcp_test_support::HostedMcpDiscoveryEgress;
use crate::{
    RebornReadinessDiagnostic, RebornReadinessState, runtime::SKILL_ACTIVATE_CAPABILITY_ID,
};

#[test]
fn libsql_build_resource_governor_guard_requires_singleton_authority() {
    assert!(ensure_libsql_resource_governor_authority_for_build(true).is_ok());
    assert!(matches!(
        ensure_libsql_resource_governor_authority_for_build(false),
        Err(RebornBuildError::InvalidConfig { reason })
            if reason.contains("libSQL FilesystemResourceGovernor uses process-local tallies")
    ));
}

#[tokio::test]
async fn production_store_bundle_new_validates_runtime_storage_before_store_assembly() {
    let filesystem = empty_composite_filesystem();
    let error = match ProductionStoreBundle::new(
        Arc::clone(&filesystem),
        filesystem_resource_governor(&filesystem),
        test_secret_master_key(),
        ironclaw_reborn_event_store::RebornEventStoreConfig::InMemory,
    )
    .await
    {
        Ok(_) => panic!("missing runtime storage plane must fail bundle construction"),
        Err(error) => error,
    };

    assert_runtime_storage_validation_error(&error);
}

#[tokio::test]
async fn production_store_bundle_with_secret_credentials_validates_runtime_storage_first() {
    let credential_filesystem = empty_composite_filesystem();
    let secret_credentials = FilesystemSecretCredentialStores::from_master_key(
        crate::wrap_scoped(Arc::clone(&credential_filesystem)),
        test_secret_master_key(),
    )
    .expect("test secret stores should construct");
    let filesystem = empty_composite_filesystem();

    let error = match ProductionStoreBundle::with_secret_credentials(
        Arc::clone(&filesystem),
        filesystem_resource_governor(&filesystem),
        secret_credentials,
        ironclaw_reborn_event_store::RebornEventStoreConfig::InMemory,
    )
    .await
    {
        Ok(_) => panic!("missing runtime storage plane must fail bundle construction"),
        Err(error) => error,
    };

    assert_runtime_storage_validation_error(&error);
}

#[tokio::test]
async fn production_turn_state_store_uses_row_layout() {
    let view = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").expect("turns mount alias"),
        VirtualPath::new("/turns").expect("turns virtual path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(ironclaw_filesystem::InMemoryBackend::new()),
        view,
    ));

    // `production_turn_state_store` returns the concrete
    // `FilesystemTurnStateRowStore` by type, so "production uses the row
    // layout" is now a compile-time guarantee. This exercises the factory
    // end-to-end and confirms the constructed store answers reads.
    let store =
        production_turn_state_store(filesystem, ironclaw_turns::TurnStateStoreLimits::default());

    let snapshot = store.persistence_snapshot().await.expect("read snapshot");
    assert!(snapshot.runs.is_empty());
}

fn empty_composite_filesystem() -> Arc<CompositeRootFilesystem> {
    Arc::new(CompositeRootFilesystem::new())
}

fn filesystem_resource_governor(
    filesystem: &Arc<CompositeRootFilesystem>,
) -> ComposedResourceGovernor {
    FilesystemResourceGovernor::new(crate::wrap_scoped(Arc::clone(filesystem)))
}

fn test_secret_master_key() -> ironclaw_secrets::SecretMaterial {
    ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901")
}

fn assert_runtime_storage_validation_error(error: &RebornBuildError) {
    assert!(
        matches!(
            error,
            RebornBuildError::InvalidConfig { reason }
                if reason.contains("runtime storage plane `tenant scoped state` requires `/tenants`")
        ),
        "{error}"
    );
}

#[test]
fn build_runtime_substrate_uses_filesystem_resource_governor() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let services = runtime
        .block_on(build_runtime_substrate(
            crate::deployment::local_dev_build_input(
                "resource-governor-enabled-env-owner",
                dir.path().join("local-dev"),
            ),
        ))
        .expect("local-dev services build");
    let runtime_surfaces = services.local_runtime_for_test().expect("local runtime");
    let scope = ResourceScope {
        tenant_id: TenantId::new("resource-governor-tenant").expect("tenant"),
        user_id: UserId::new("resource-governor-user").expect("user"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let account = ironclaw_resources::ResourceAccount::tenant(scope.tenant_id.clone());

    let reservation = runtime_surfaces
        .resource_governor
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(0.10)))
        .expect("reservation");
    runtime_surfaces
        .resource_governor
        .reconcile(reservation.id, ResourceUsage::default().set_usd(dec!(0.10)))
        .expect("reconcile");

    assert_eq!(
        runtime_surfaces
            .resource_governor
            .usage_for(&account)
            .expect("usage")
            .usd,
        dec!(0.10)
    );
}

#[test]
fn extension_installation_state_path_is_single_runtime_default() {
    let path = FilesystemExtensionInstallationStore::default_state_path().expect("state path");

    assert_eq!(path.as_str(), "/system/extensions/.installations");
}

struct FailingConversationActorPairingService;

#[async_trait::async_trait]
impl ConversationActorPairingService for FailingConversationActorPairingService {
    async fn pair_external_actor(
        &self,
        _tenant_id: TenantId,
        _adapter_kind: AdapterKind,
        _adapter_installation_id: AdapterInstallationId,
        _external_actor_ref: ExternalActorRef,
        _user_id: UserId,
    ) -> Result<(), ironclaw_conversations::InboundTurnError> {
        Err(ironclaw_conversations::InboundTurnError::DurableState {
            reason: "raw durable store error".to_string(),
        })
    }

    async fn pair_external_actor_with_epoch(
        &self,
        _tenant_id: TenantId,
        _adapter_kind: AdapterKind,
        _adapter_installation_id: AdapterInstallationId,
        _external_actor_ref: ExternalActorRef,
        _user_id: UserId,
        _binding_epoch: ironclaw_conversations::ExternalActorBindingEpoch,
    ) -> Result<(), ironclaw_conversations::InboundTurnError> {
        Err(ironclaw_conversations::InboundTurnError::DurableState {
            reason: "raw durable store error".to_string(),
        })
    }

    async fn unpair_external_actor(
        &self,
        _tenant_id: TenantId,
        _adapter_kind: AdapterKind,
        _adapter_installation_id: AdapterInstallationId,
        _external_actor_ref: ExternalActorRef,
    ) -> Result<(), ironclaw_conversations::InboundTurnError> {
        Err(ironclaw_conversations::InboundTurnError::DurableState {
            reason: "raw durable store error".to_string(),
        })
    }

    async fn unpair_external_actor_if_owned_by(
        &self,
        _tenant_id: &TenantId,
        _adapter_kind: &AdapterKind,
        _adapter_installation_id: &AdapterInstallationId,
        _external_actor_ref: &ExternalActorRef,
        _expected: &ironclaw_conversations::ExpectedExternalActorOwner,
    ) -> Result<
        ironclaw_conversations::ConditionalUnpairOutcome,
        ironclaw_conversations::InboundTurnError,
    > {
        Err(ironclaw_conversations::InboundTurnError::DurableState {
            reason: "raw durable store error".to_string(),
        })
    }
}

/// Per-trigger delivery targets validate against the SAME registry the
/// outbound target surface publishes from: an id a provider resolves for
/// the caller is accepted; an unknown id (or an empty registry) fails
/// closed as `DeliveryTargetInvalid`.
#[tokio::test]
async fn trigger_delivery_target_validation_resolves_through_the_outbound_registry() {
    use crate::outbound::{
        DeliveryTargetCapabilities, MutableOutboundDeliveryTargetRegistry,
        OutboundDeliveryTargetEntry, OutboundDeliveryTargetId, OutboundDeliveryTargetOwner,
        OutboundDeliveryTargetProvider, OutboundDeliveryTargetScope, OutboundDeliveryTargetSummary,
    };
    use ironclaw_outbound::OutboundError;

    struct OneTargetProvider {
        entry: OutboundDeliveryTargetEntry,
    }

    #[async_trait::async_trait]
    impl OutboundDeliveryTargetProvider for OneTargetProvider {
        async fn list_outbound_delivery_targets(
            &self,
            caller: &OutboundDeliveryTargetScope,
        ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
            // Fixture available to whichever caller asks: claim the querying
            // caller as owner so it survives the registry caller-scoping filter.
            Ok(vec![OutboundDeliveryTargetEntry {
                summary: self.entry.summary.clone(),
                capabilities: self.entry.capabilities.clone(),
                destination: self.entry.destination.clone(),
                owner: OutboundDeliveryTargetOwner::for_scope(caller),
            }])
        }
    }

    let scope = ironclaw_host_api::ResourceScope {
        tenant_id: TenantId::new("registry-validation-tenant").expect("tenant"),
        user_id: UserId::new("registry-validation-user").expect("user"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: ironclaw_host_api::InvocationId::new(),
    };
    let target = ironclaw_triggers::TriggerDeliveryTargetId::new("slack:personal-dm:T1:me")
        .expect("target id");

    let registry = MutableOutboundDeliveryTargetRegistry::default();
    // Empty registry → fail closed.
    let rejected = validate_trigger_delivery_target_against_registry(&registry, &scope, &target)
        .await
        .expect_err("empty registry must reject");
    assert!(matches!(
        rejected,
        TriggerError::InvalidRecord {
            kind: ironclaw_triggers::TriggerRecordValidationKind::DeliveryTargetInvalid,
            ..
        }
    ));

    // Registered provider that resolves the id for the caller → accept.
    let entry = OutboundDeliveryTargetEntry {
        summary: OutboundDeliveryTargetSummary::new(
            OutboundDeliveryTargetId::new("slack:personal-dm:T1:me").expect("id"),
            "slack",
            "Slack DM".to_string(),
            None,
        )
        .expect("summary"),
        capabilities: DeliveryTargetCapabilities {
            final_replies: true,
            progress: false,
            gate_prompts: true,
            auth_prompts: true,
            modalities: Vec::new(),
        },
        destination: ironclaw_outbound::RunFinalReplyDestination::External {
            reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new(
                "reply:registry-validation",
            )
            .expect("binding ref"),
        },
        // Overwritten with the querying caller by `OneTargetProvider::list`;
        // set to the scope identity here for clarity.
        owner: OutboundDeliveryTargetOwner::new(
            TenantId::new("registry-validation-tenant").expect("tenant"),
            UserId::new("registry-validation-user").expect("user"),
        ),
    };
    registry
        .register_provider("test", Arc::new(OneTargetProvider { entry }))
        .expect("register");
    validate_trigger_delivery_target_against_registry(&registry, &scope, &target)
        .await
        .expect("registered target must validate");

    // A different id still fails closed.
    let other = ironclaw_triggers::TriggerDeliveryTargetId::new("slack:personal-dm:T1:other")
        .expect("target id");
    validate_trigger_delivery_target_against_registry(&registry, &scope, &other)
        .await
        .expect_err("unknown target must reject");
}

fn trigger_record_for_pairing_test() -> TriggerRecord {
    TriggerRecord {
        trigger_id: ironclaw_triggers::TriggerId::new(),
        tenant_id: TenantId::new("pairing-test-tenant").expect("tenant id"),
        creator_user_id: UserId::new("pairing-test-user").expect("user id"),
        agent_id: None,
        project_id: None,
        name: "pairing test".to_string(),
        source: ironclaw_triggers::TriggerSourceKind::Schedule,
        schedule: ironclaw_triggers::TriggerSchedule::cron("* * * * *")
            .expect("valid cron expression"),
        prompt: "pairing test prompt".to_string(),
        delivery_target: None,
        state: ironclaw_triggers::TriggerState::Scheduled,
        next_run_at: chrono::Utc::now(),
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: chrono::Utc::now(),
    }
}

#[tokio::test]
async fn pair_trigger_creator_maps_pairing_failure_to_sanitized_backend_error() {
    let record = trigger_record_for_pairing_test();

    let error = pair_trigger_creator(&FailingConversationActorPairingService, &record)
        .await
        .expect_err("pairing failure should surface");

    let TriggerError::Backend { reason } = error else {
        panic!("expected backend trigger error");
    };
    assert_eq!(reason, "trigger creator actor pairing failed");
}

fn failing_trigger_conversation_filesystem() -> Arc<ScopedFilesystem<CompositeRootFilesystem>> {
    let mut failing_root = CompositeRootFilesystem::new();
    failing_root
        .mount(
            local_dev_mount_descriptor(
                "/conversations",
                "failing-conversation-state",
                BackendKind::Custom("test".to_string()),
                StorageClass::StructuredRecords,
                ContentKind::StructuredRecord,
                IndexPolicy::NotIndexed,
                BackendCapabilities::default(),
            )
            .expect("mount descriptor"),
            Arc::new(
                ironclaw_filesystem::FaultInjecting::new(
                    ironclaw_filesystem::InMemoryBackend::new(),
                )
                .with_fault(
                    ironclaw_filesystem::Fault::on(
                        ironclaw_filesystem::FilesystemOperation::ReadFile,
                    )
                    .backend("conversation state load failed"),
                ),
            ),
        )
        .expect("mount failing backend");
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(failing_root),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/conversations").expect("mount alias"),
            VirtualPath::new("/conversations").expect("virtual path"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view"),
    ))
}

#[tokio::test]
async fn durable_trigger_conversation_services_propagates_init_error() {
    let filesystem = failing_trigger_conversation_filesystem();

    let error = match RebornFilesystemConversationServices::new(filesystem).await {
        Ok(_) => panic!("conversation service init should fail"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        ironclaw_conversations::InboundTurnError::DurableState { .. }
    ));
}

#[tokio::test]
async fn local_runtime_trigger_create_hook_maps_conversation_init_error_to_backend() {
    let local_dev_root = tempfile::tempdir().expect("tempdir");
    let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
        "pairing-owner",
        local_dev_root.path().join("local-dev"),
    ))
    .await
    .expect("local-dev services build");
    let runtime = services.local_runtime_for_test().expect("local runtime");
    let hook = LocalRuntimeTriggerCreatorPairingHook {
        outbound_delivery_targets: Arc::clone(runtime.outbound_delivery_targets_for_test()),
        scoped_filesystem: failing_trigger_conversation_filesystem(),
        conversations: tokio::sync::OnceCell::new(),
        delivery_target_service: Arc::new(ironclaw_product::TriggerFinalReplyTargetService::new(
            Arc::new(LateBoundTriggerSourceTurnStateStore {
                source_turn_state: Arc::clone(&services.trigger_source_turn_state_store),
            }),
            Arc::clone(&services.outbound_state),
            Arc::clone(&services.current_delivery_targets)
                as Arc<dyn CurrentDeliveryTargetResolver>,
        )),
    };
    let record = trigger_record_for_pairing_test();

    let error = hook
        .after_trigger_persisted(&record)
        .await
        .expect_err("conversation init failure should surface as trigger backend error");

    let TriggerError::Backend { reason } = error else {
        panic!("expected backend trigger error");
    };
    assert_eq!(reason, "trigger creator actor pairing failed");
}

#[tokio::test]
async fn local_dev_services_include_repl_runtime_substrate() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
        "local-dev-substrate-owner",
        dir.path().join("local-dev"),
    ))
    .await
    .expect("local-dev services build");

    let _ = &services.host_runtime;
    let _ = &services.turn_coordinator;
    let _ = &services.product_auth;
    assert!(services.local_runtime_for_test().is_some());
    let _ = &services.scoped_filesystem;
    let _ = &services.turn_state;
    let _ = &services
        .local_runtime_for_test()
        .expect("local runtime")
        .extension_management;
    assert_eq!(services.readiness.state, RebornReadinessState::DevOnly);
}

#[tokio::test]
async fn hosted_single_tenant_rejects_local_dev_storage_input() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = crate::deployment::local_dev_build_input(
        "hosted-single-tenant-local-storage-owner",
        dir.path().join("local-dev"),
    );
    // Deliberate mismatch: swap the local-dev deployment for a hosted
    // single-tenant one while keeping the local-dev storage input. In
    // production this pairing is unreachable — storage is derived from the
    // deployment — so the dedicated storage-shape guard string
    // ("hosted single-tenant Postgres storage input") was removed in commit
    // 975bcd2ce ("Unify reborn runtime assembly"). What must survive is that the
    // build still FAILS CLOSED on the mismatch rather than silently composing a
    // hosted deployment over local storage. Swapping the deployment drops its
    // resolved runtime policy (policy lives on the deployment since Phase A), so
    // the surviving fail-closed guard is `MissingRuntimePolicy`.
    let input = input.with_deployment(crate::deployment::DeploymentConfig::for_profile(
        RebornCompositionProfile::HostedSingleTenant,
        false,
    ));

    let error = match build_runtime_substrate(input).await {
        Ok(_) => {
            panic!(
                "mismatched hosted-single-tenant deployment over local-dev storage must fail closed"
            )
        }
        Err(error) => error,
    };
    assert!(
        matches!(error, RebornBuildError::MissingRuntimePolicy),
        "expected the mismatched pairing to fail closed on the runtime-policy guard, got {error:?}"
    );
}

#[tokio::test]
async fn local_dev_memory_first_party_tools_use_mounted_memory_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
        "local-dev-memory-owner",
        dir.path().join("local-dev"),
    ))
    .await
    .expect("local-dev services build");
    invoke_json(
        &services,
        MEMORY_WRITE_CAPABILITY_ID,
        memory_context(MEMORY_WRITE_CAPABILITY_ID),
        serde_json::json!({
            "target": "projects/alpha/notes.md",
            "content": "local dev mounted memory root search marker",
            "append": false
        }),
    )
    .await
    .expect("memory_write should use the mounted /memory root");

    let tree = invoke_json(
        &services,
        MEMORY_TREE_CAPABILITY_ID,
        memory_context(MEMORY_TREE_CAPABILITY_ID),
        serde_json::json!({"path": "", "depth": 3}),
    )
    .await
    .expect("memory_tree should list the mounted /memory root");
    assert!(
        tree.to_string().contains("alpha/"),
        "memory_tree should include the written memory document: {tree}"
    );

    let search = invoke_json(
        &services,
        MEMORY_SEARCH_CAPABILITY_ID,
        memory_context(MEMORY_SEARCH_CAPABILITY_ID),
        serde_json::json!({"query": "mounted memory root search marker", "limit": 5}),
    )
    .await
    .expect("memory_search should query the mounted /memory root");
    assert_eq!(search["result_count"], serde_json::json!(1));
    assert_eq!(
        search["results"][0]["path"],
        serde_json::json!("projects/alpha/notes.md")
    );
}

#[tokio::test]
async fn local_dev_memory_documents_persist_across_rebuilds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let local_dev_root = dir.path().join("local-dev");
    let owner = "local-dev-durable-memory-owner";

    let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
        owner,
        local_dev_root.clone(),
    ))
    .await
    .expect("first local-dev services build");
    invoke_json(
        &services,
        MEMORY_WRITE_CAPABILITY_ID,
        memory_context(MEMORY_WRITE_CAPABILITY_ID),
        serde_json::json!({
            "target": "projects/durable/notes.md",
            "content": "local dev durable mounted memory root search marker",
            "append": false
        }),
    )
    .await
    .expect("memory_write should persist through the libsql /memory root");
    drop(services);

    let rebuilt = build_runtime_substrate(crate::deployment::local_dev_build_input(
        owner,
        local_dev_root.clone(),
    ))
    .await
    .expect("rebuilt local-dev services");

    let tree = invoke_json(
        &rebuilt,
        MEMORY_TREE_CAPABILITY_ID,
        memory_context(MEMORY_TREE_CAPABILITY_ID),
        serde_json::json!({"path": "", "depth": 3}),
    )
    .await
    .expect("memory_tree should list rebuilt libsql memory documents");
    assert!(
        tree.to_string().contains("durable/"),
        "memory_tree should include the persisted memory document: {tree}"
    );

    let search = invoke_json(
        &rebuilt,
        MEMORY_SEARCH_CAPABILITY_ID,
        memory_context(MEMORY_SEARCH_CAPABILITY_ID),
        serde_json::json!({"query": "durable mounted memory root search marker", "limit": 5}),
    )
    .await
    .expect("memory_search should query rebuilt libsql memory documents");
    assert_eq!(search["result_count"], serde_json::json!(1));
    assert_eq!(
        search["results"][0]["path"],
        serde_json::json!("projects/durable/notes.md")
    );
}

#[tokio::test]
async fn local_dev_default_product_auth_preserves_manual_token_across_rebuilds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let local_dev_root = dir.path().join("local-dev");
    let owner = "local-dev-durable-auth-owner";
    let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
        owner,
        local_dev_root.clone(),
    ))
    .await
    .expect("local-dev services build");
    let product_auth = &services.product_auth;
    let scope = AuthProductScope::new(
        ResourceScope::local_default(UserId::new(owner).unwrap(), InvocationId::new()).unwrap(),
        AuthSurface::Callback,
    );
    let mut scope = scope;
    scope.resource.thread_id = Some(ironclaw_host_api::ThreadId::new("auth-thread").unwrap());

    let challenge = product_auth
        .request_manual_token_setup(crate::RebornManualTokenSetupRequest::new(
            scope.clone(),
            ironclaw_auth::AuthProviderId::new("github").unwrap(),
            CredentialAccountLabel::new("work github").unwrap(),
            ironclaw_auth::AuthContinuationRef::SetupOnly,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        ))
        .await
        .unwrap();
    let submitted = product_auth
        .submit_manual_token(crate::RebornManualTokenSubmitRequest::new(
            scope.clone(),
            challenge.interaction_id,
            secrecy::SecretString::from("ghp_local_dev_pat"),
        ))
        .await
        .unwrap();

    let account = product_auth
        .credential_account_service()
        .get_account(ironclaw_auth::CredentialAccountLookupRequest::new(
            scope.clone(),
            submitted.account_id,
        ))
        .await
        .unwrap()
        .expect("manual-token submit should create account");
    let access_secret = account.access_secret.expect("manual token access secret");
    assert!(
        access_secret.as_str().starts_with("product-auth-manual-"),
        "local-dev default product-auth must create durable SecretStore-backed handles"
    );

    let rebuilt = build_runtime_substrate(crate::deployment::local_dev_build_input(
        owner,
        local_dev_root.clone(),
    ))
    .await
    .expect("local-dev services rebuild");
    let rebuilt_product_auth = rebuilt.product_auth.as_ref();
    let rebuilt_account = rebuilt_product_auth
        .credential_account_service()
        .get_account(ironclaw_auth::CredentialAccountLookupRequest::new(
            scope.clone(),
            submitted.account_id,
        ))
        .await
        .unwrap()
        .expect("manual-token account should survive local-dev rebuild");
    assert_eq!(rebuilt_account.access_secret.as_ref(), Some(&access_secret));

    let rebuilt_filesystem = build_local_runtime_root_filesystem(
        &local_dev_root,
        &local_dev_root.join("workspace"),
        None,
        StorageBackendInput::LocalDefault,
    )
    .await
    .expect("local-dev filesystem rebuild")
    .filesystem;
    let (rebuilt_secret_store, _rebuilt_secret_crypto) = build_secret_store(
        &local_dev_root,
        local_dev_scoped_filesystem(rebuilt_filesystem),
        None,
    )
    .await
    .expect("local-dev secret store rebuild");
    let lease = rebuilt_secret_store
        .lease_once(&scope.resource, &access_secret)
        .await
        .expect("manual token secret should survive local-dev rebuild");
    let raw_secret = rebuilt_secret_store
        .consume(&scope.resource, lease.id)
        .await
        .expect("manual token secret should decrypt after local-dev rebuild");
    assert_eq!(raw_secret.expose_secret(), "ghp_local_dev_pat");

    let flows = product_auth
        .flow_record_source()
        .expect("local-dev product-auth flow source")
        .flows_for_owner(ironclaw_auth::AuthFlowOwnerScope {
            tenant_id: scope.resource.tenant_id.clone(),
            user_id: scope.resource.user_id.clone(),
            agent_id: scope.resource.agent_id.clone(),
            project_id: scope.resource.project_id.clone(),
            thread_id: scope.resource.thread_id.clone().unwrap(),
        })
        .await
        .unwrap();
    let completed_flow = flows
        .iter()
        .find(|flow| flow.credential_account_id == Some(submitted.account_id))
        .expect("manual-token completion should remain visible to auth gates");
    assert_eq!(
        completed_flow.status,
        ironclaw_auth::AuthFlowStatus::Completed
    );
}

/// Verify that `attach_hosted_mcp_runtime` is soft-disabled when the host
/// runtime has no HTTP egress (e.g. in-memory-only test services). The
/// function must not panic or return an error; it simply skips the MCP
/// runtime attachment so the rest of the composition continues.
#[test]
fn attach_hosted_mcp_runtime_skips_services_without_http_egress() {
    let services = HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    );
    // product_auth_provider_runtime_ports() is None without HTTP egress.
    assert!(services.product_auth_provider_runtime_ports().is_none());

    // attach_hosted_mcp_runtime must succeed (soft-skip) rather than error.
    let services = attach_hosted_mcp_runtime(services).expect("soft-disable must not error");

    // Runtime ports still absent — no egress was added by the attachment.
    assert!(services.product_auth_provider_runtime_ports().is_none());
}

/// A corrupt local-dev key file must fail loud with a path-naming error,
/// not the opaque "Invalid master key" that surfaces when the unvalidated
/// material reaches `SecretsCrypto::new` several layers deep. Mirrors the
/// real all-zeros key an `[env] SECRETS_MASTER_KEY = "000...0"` cargo
/// override writes into the cached key file.
#[tokio::test]
async fn resolve_local_dev_secret_master_key_rejects_malformed_file_with_path_context() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let key_path = root.join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH);
    // 64 zero chars: passes the length floor but has a single distinct
    // byte, which `SecretsCrypto::new` rejects on the entropy check.
    std::fs::write(&key_path, "0".repeat(64)).expect("write malformed key");

    let error = resolve_local_dev_secret_master_key(root)
        .await
        .expect_err("malformed local-dev master key must be rejected");

    match error {
        RebornBuildError::InvalidConfig { reason } => {
            assert!(
                reason.contains(&key_path.display().to_string()),
                "error must name the offending key file path, got: {reason}"
            );
            assert!(
                reason.contains("master key"),
                "error must mention the master key, got: {reason}"
            );
        }
        other => panic!("expected InvalidConfig, got {other:?}"),
    }
}

/// An explicit but malformed `SECRETS_MASTER_KEY` env value (the actual
/// root cause of the original report) must fail loud and name the env var.
/// Driven through the real caller `resolve_local_dev_secret_master_key`
/// (via its env-parameterized inner) so this also guards the
/// write-before-validate invariant: a rejected env key must never be
/// persisted to the cached `.reborn-local-dev-secrets-master-key` file.
#[tokio::test]
async fn resolve_local_dev_secret_master_key_rejects_malformed_env_without_persisting() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let key_path = root.join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH);
    assert!(
        !key_path.exists(),
        "precondition: cached key file must not exist yet"
    );

    // 64 zero chars: passes the length floor but has a single distinct byte,
    // so the entropy check rejects it.
    let error = resolve_local_dev_secret_master_key_with_env(root, Some("0".repeat(64)))
        .await
        .expect_err("malformed env master key must be rejected");

    match error {
        RebornBuildError::InvalidConfig { reason } => {
            assert!(
                reason.contains(ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV),
                "error must name the env var, got: {reason}"
            );
            assert!(
                reason.contains("master key"),
                "error must mention the master key, got: {reason}"
            );
        }
        other => panic!("expected InvalidConfig, got {other:?}"),
    }

    // Write-before-validate regression guard: the rejected key must NOT have
    // been persisted to the cached file.
    assert!(
        !key_path.exists(),
        "rejected env master key must not be persisted to {}",
        key_path.display()
    );
}

#[tokio::test]
async fn resolve_local_dev_secret_master_key_rejects_set_but_empty_env_without_persisting() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let key_path = root.join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH);

    // A set-but-empty (or whitespace-only) env value is explicit-but-unusable
    // configuration: it must fail closed, NOT collapse to "absent" and
    // generate + persist a fresh key the operator never chose.
    for empty in ["", "   ", "\n\t "] {
        let error = resolve_local_dev_secret_master_key_with_env(root, Some(empty.to_string()))
            .await
            .expect_err("set-but-empty env master key must be rejected");
        match error {
            RebornBuildError::InvalidConfig { reason } => assert!(
                reason.contains(ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV),
                "error must name the env var, got: {reason}"
            ),
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
        assert!(
            !key_path.exists(),
            "a set-but-empty env master key must not generate/persist a key at {}",
            key_path.display()
        );
    }
}

#[tokio::test]
async fn resolve_local_dev_secret_master_key_rejects_empty_env_even_with_cached_file() {
    // Regression: the empty-env rejection must run BEFORE the cached-file
    // read, so an explicitly-set-but-empty SECRETS_MASTER_KEY fails closed
    // on a rebuild even when `.reborn-local-dev-secrets-master-key` already
    // exists — it must not be silently ignored in favor of the cached key.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let key_path = root.join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH);

    // Seed directly (not through the resolver): this test is about
    // empty-env/cached-file precedence, not the keychain step, and this
    // crate can't suppress the OS keychain in-process (`forbid(unsafe_code)`
    // blocks `set_var`; see the fallthrough test in `tests/facade_factory.rs`).
    std::fs::write(
        &key_path,
        ironclaw_secrets::keychain::generate_master_key_hex(),
    )
    .expect("seed a valid cached master key file");
    assert!(key_path.exists(), "precondition: cached key file exists");
    let cached_before = std::fs::read_to_string(&key_path).expect("read cached key");

    let error = resolve_local_dev_secret_master_key_with_env(root, Some("   ".to_string()))
        .await
        .expect_err("empty env must fail closed even with a cached file");
    match error {
        RebornBuildError::InvalidConfig { reason } => assert!(
            reason.contains(ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV),
            "error must name the env var, got: {reason}"
        ),
        other => panic!("expected InvalidConfig, got {other:?}"),
    }
    // The cached key is left untouched (not silently returned, not rewritten).
    assert_eq!(
        std::fs::read_to_string(&key_path).expect("read cached key"),
        cached_before,
        "the cached key must be left unchanged when the env value is rejected"
    );
}

#[tokio::test]
async fn resolve_local_dev_secret_master_key_rejects_malformed_env_even_with_cached_file() {
    // A non-empty-but-malformed env value must also fail closed BEFORE the
    // cached-file read, so `SECRETS_MASTER_KEY=0000...` is not silently
    // ignored in favor of a valid cached key on a rebuild.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let key_path = root.join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH);

    // Seed directly, not through the resolver — see the comment in
    // `resolve_local_dev_secret_master_key_rejects_empty_env_even_with_cached_file`
    // for why a `None`-env resolver call here would hit the real OS
    // keychain in-process.
    std::fs::write(
        &key_path,
        ironclaw_secrets::keychain::generate_master_key_hex(),
    )
    .expect("seed a valid cached master key file");
    let cached_before = std::fs::read_to_string(&key_path).expect("read cached key");

    // 64 zero chars: passes the length floor but fails the entropy check.
    let error = resolve_local_dev_secret_master_key_with_env(root, Some("0".repeat(64)))
        .await
        .expect_err("malformed env must fail closed even with a cached file");
    match error {
        RebornBuildError::InvalidConfig { reason } => assert!(
            reason.contains(ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV),
            "error must name the env var, got: {reason}"
        ),
        other => panic!("expected InvalidConfig, got {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(&key_path).expect("read cached key"),
        cached_before,
        "the cached key must be left unchanged when a malformed env value is rejected"
    );
}

/// A well-formed cached key file passes through unchanged.
#[tokio::test]
async fn resolve_local_dev_secret_master_key_accepts_valid_cached_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let valid = ironclaw_secrets::keychain::generate_master_key_hex();
    std::fs::write(root.join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH), &valid).expect("write valid key");

    resolve_local_dev_secret_master_key(root)
        .await
        .expect("valid cached key must be accepted");
}

/// `open_local_dev_secret_store` is the narrow pre-composition opener
/// onboard needs: no full [`CompositeRootFilesystem`], just the physical
/// libSQL file backing `/secrets`. A cached master-key dotfile is seeded
/// up front so the resolver never touches the OS keychain or env (see the
/// `forbid(unsafe_code)` note above — this crate's inline tests cannot
/// mutate process env, and a cached dotfile is the non-env-mutating way
/// to make the resolver deterministic here).
#[tokio::test]
async fn open_local_dev_secret_store_opens_a_working_store_over_the_bare_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let valid = ironclaw_secrets::keychain::generate_master_key_hex();
    std::fs::write(root.join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH), &valid)
        .expect("seed cached master key");

    let store = open_local_dev_secret_store(root)
        .await
        .expect("opener must succeed over a bare root");

    let keys = crate::LlmKeyStore::new(store);
    keys.put(
        "nearai",
        ironclaw_secrets::SecretMaterial::from("sk-test-value"),
    )
    .await
    .expect("put through the opened store");
    let read = keys
        .read("nearai")
        .await
        .expect("read through the opened store")
        .expect("value must be present");
    assert_eq!(secrecy::ExposeSecret::expose_secret(&read), "sk-test-value");
}

/// The opener is idempotent: reopening over the same root (same physical
/// db file, same cached master key) must decrypt a value written by a
/// prior open — this is the "onboard writes, serve reads" contract B2
/// exists to satisfy.
#[tokio::test]
async fn open_local_dev_secret_store_is_visible_across_reopens_of_the_same_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let valid = ironclaw_secrets::keychain::generate_master_key_hex();
    std::fs::write(root.join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH), &valid)
        .expect("seed cached master key");

    let first = open_local_dev_secret_store(root)
        .await
        .expect("first open must succeed");
    crate::LlmKeyStore::new(first)
        .put(
            "nearai",
            ironclaw_secrets::SecretMaterial::from("sk-reopen-value"),
        )
        .await
        .expect("put through the first open");

    let second = open_local_dev_secret_store(root)
        .await
        .expect("second open (simulating `serve`) must succeed");
    let read = crate::LlmKeyStore::new(second)
        .read("nearai")
        .await
        .expect("read through the second open")
        .expect("value written by the first open must be visible");
    assert_eq!(
        secrecy::ExposeSecret::expose_secret(&read),
        "sk-reopen-value"
    );
}

// The keychain-fallthrough + idempotency test for
// `resolve_local_dev_secret_master_key_with_env` lives in
// `tests/facade_factory.rs`
// (`local_dev_secret_store_falls_through_suppressed_keychain_to_dotfile`):
// proving it needs the real process env var `IRONCLAW_DISABLE_OS_KEYCHAIN`
// set, and `set_var` is `unsafe` — blocked here by this crate's
// `forbid(unsafe_code)` even in `#[cfg(test)]`. `tests/*.rs` binaries are
// separate crates the `forbid` doesn't reach.

#[tokio::test]
async fn local_dev_gsuite_installs_activates_and_dispatches_through_host_runtime() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
        "local-dev-gsuite-owner",
        dir.path().join("local-dev"),
    ))
    .await
    .expect("local-dev services build");
    let runtime_surfaces = services.local_runtime_for_test().expect("local runtime");
    let extension_management = &runtime_surfaces.extension_management;
    let gmail_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "gmail").expect("valid ref");
    let calendar_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "google-calendar")
        .expect("valid ref");

    // #6520 removed the port-side operator accessor: install as the owner the
    // runtime was constructed with.
    let caller = UserId::new("local-dev-gsuite-owner").expect("valid lifecycle caller");
    extension_management
        .install(gmail_ref.clone(), &caller)
        .await
        .expect("install Gmail");
    extension_management
        .activate_with_prechecked_credentials_for_test(
            gmail_ref,
            ExtensionActivationMode::Static,
            &caller,
        )
        .await
        .expect("activate Gmail");
    extension_management
        .install(calendar_ref.clone(), &caller)
        .await
        .expect("install Google Calendar");
    extension_management
        .activate_with_prechecked_credentials_for_test(
            calendar_ref,
            ExtensionActivationMode::Static,
            &caller,
        )
        .await
        .expect("activate Google Calendar");

    let gmail_context = gsuite_context("gmail.send_message");
    let gmail_scope = gmail_context.resource_scope.clone();
    let gmail_capability =
        CapabilityId::new("gmail.send_message").expect("valid Gmail capability id");
    assert!(matches!(
        runtime_surfaces
            .capability_policy_for_test()
            .lease_approval_for(
                BuiltinApprovalPolicyAction::Dispatch {
                    capability: &gmail_capability,
                },
                runtime_surfaces.workspace_mounts_for_test(),
                runtime_surfaces.skill_mounts_for_test(),
                runtime_surfaces.memory_mounts_for_test(),
                runtime_surfaces.system_extensions_lifecycle_mounts_for_test(),
            ),
        Err(BuiltinCapabilityPolicyError::MissingGrant { .. })
    ));
    let auth_scope = AuthProductScope::new(gmail_context.resource_scope.clone(), AuthSurface::Api);
    services
        .product_auth
        .as_ref()
        .credential_account_service()
        .create_account(NewCredentialAccount {
            scope: auth_scope,
            provider: ironclaw_first_party_extensions::google_provider_id()
                .expect("Google provider id"),
            label: CredentialAccountLabel::new("work google").expect("valid label"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("missing-google-access-token").unwrap()),
            refresh_secret: None,
            scopes: vec![
                ProviderScope::new(GOOGLE_GMAIL_SEND_SCOPE).unwrap(),
                ProviderScope::new(GOOGLE_CALENDAR_EVENTS_SCOPE).unwrap(),
            ],
        })
        .await
        .expect("create Google account");

    disable_global_auto_approve(runtime_surfaces, &gmail_context).await;
    let failure = invoke_json(
        &services,
        "gmail.send_message",
        gmail_context,
        serde_json::json!({ "message": { "raw": "base64url-rfc822" } }),
    )
    .await
    .expect_err("missing token should fail after approval resume");
    assert_ne!(failure, RuntimeFailureKind::Authorization);
    assert_ne!(failure, RuntimeFailureKind::MissingRuntime);
    let gmail_leases = runtime_surfaces
        .capability_leases_for_test()
        .leases_for_scope(&gmail_scope)
        .await;
    assert_eq!(gmail_leases.len(), 1);
    assert_eq!(gmail_leases[0].grant.issued_by, Principal::HostRuntime);
    assert_eq!(gmail_leases[0].grant.constraints.max_invocations, Some(1));
    assert_eq!(gmail_leases[0].status, CapabilityLeaseStatus::Revoked);

    let calendar_context = gsuite_context("google-calendar.create_event");
    disable_global_auto_approve(runtime_surfaces, &calendar_context).await;
    let failure = invoke_json(
        &services,
        "google-calendar.create_event",
        calendar_context,
        serde_json::json!({
            "calendar_id": "primary",
            "event": { "summary": "Review" }
        }),
    )
    .await
    .expect_err("missing token should fail after approval resume");
    assert_ne!(failure, RuntimeFailureKind::Authorization);
    assert_ne!(failure, RuntimeFailureKind::MissingRuntime);
}

#[tokio::test]
async fn local_dev_notion_mcp_installs_activates_and_reaches_auth_gate() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_runtime_substrate(
        crate::deployment::local_dev_build_input_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "local-dev-notion-mcp-owner",
            dir.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_minimal_approval_policy()),
    )
    .await
    .expect("local-dev services build");
    let runtime_surfaces = services.local_runtime_for_test().expect("local runtime");
    let extension_management = &runtime_surfaces.extension_management;
    let notion_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");
    let catalog =
        AvailableExtensionCatalog::from_first_party_assets().expect("first-party extensions load");
    let notion_package = catalog.resolve(&notion_ref).expect("Notion MCP is bundled");
    // v3 hosted-MCP manifests declare one [mcp] block instead of placeholder
    // static tools: the only bundled capability is the synthesized
    // host-internal connection template. Model-visible Notion tools exist
    // only after live tools/list discovery, so this test scripts discovery
    // below to reach the auth gate.
    let capability_ids = notion_package
        .package
        .manifest
        .capabilities
        .iter()
        .map(|capability| capability.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(capability_ids, vec!["notion.mcp_server"]);
    assert_eq!(
        notion_package.package.manifest.capabilities[0].visibility,
        ironclaw_extensions::CapabilityVisibility::HostInternal
    );

    // #6520 removed the port-side operator accessor: install as the owner the
    // runtime was constructed with.
    let caller = UserId::new("local-dev-notion-mcp-owner").expect("valid lifecycle caller");
    extension_management
        .install(notion_ref.clone(), &caller)
        .await
        .expect("install Notion MCP");
    extension_management
        .activate_with_prechecked_credentials_for_test(
            notion_ref,
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: ResourceScope::local_default(caller.clone(), InvocationId::new())
                    .expect("valid scope"),
                runtime_http_egress: Arc::new(
                    HostedMcpDiscoveryEgress::with_tool_name("notion-search").read_only(),
                ),
            },
            &caller,
        )
        .await
        .expect("activate Notion MCP with scripted discovery");

    let context = notion_mcp_context("notion.notion-search");
    enable_global_auto_approve_for_context(runtime_surfaces, &context).await;
    let outcome = services
        .host_runtime
        .as_ref()
        .invoke_capability((
            context,
            CapabilityId::new("notion.notion-search").unwrap(),
            ResourceEstimate::default(),
            serde_json::json!({ "query": "project notes" }),
        ))
        .await
        .expect("runtime invocation completes");

    let RuntimeCapabilityOutcome::AuthRequired(gate) = outcome else {
        panic!("expected missing Notion token to open auth gate, got {outcome:?}");
    };
    assert_eq!(gate.capability_id.as_str(), "notion.notion-search");
}

#[tokio::test]
async fn local_dev_web_access_installs_activates_and_dispatches_through_host_runtime() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_runtime_substrate(
        crate::deployment::local_dev_build_input_with_profile(
            RebornCompositionProfile::LocalDevYolo,
            "local-dev-web-access-owner",
            dir.path().join("local-dev"),
        )
        .with_runtime_policy(local_dev_minimal_approval_policy()),
    )
    .await
    .expect("local-dev services build");
    let runtime_surfaces = services.local_runtime_for_test().expect("local runtime");
    let extension_management = &runtime_surfaces.extension_management;
    let web_access_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "web-access").expect("valid ref");

    // #6520 removed the port-side operator accessor: install as the owner the
    // runtime was constructed with.
    let caller = UserId::new("local-dev-web-access-owner").expect("valid lifecycle caller");
    extension_management
        .install(web_access_ref.clone(), &caller)
        .await
        .expect("install Web Access");
    extension_management
        .activate_with_prechecked_credentials_for_test(
            web_access_ref,
            ExtensionActivationMode::Static,
            &caller,
        )
        .await
        .expect("activate Web Access");

    let context = web_access_context("web-access.search");
    enable_global_auto_approve_for_context(runtime_surfaces, &context).await;
    let outcome = services
        .host_runtime
        .as_ref()
        .invoke_capability((
            context,
            CapabilityId::new("web-access.search").unwrap(),
            ResourceEstimate::default(),
            serde_json::json!({
                "provider": "brave",
                "query": "ironclaw reborn"
            }),
        ))
        .await
        .expect("runtime invocation completes");

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected fail-closed handler outcome, got {outcome:?}");
    };
    assert_eq!(failure.capability_id.as_str(), "web-access.search");
    // A capability the model named with no registered first-party handler
    // is a model-fixable, model-visible failure (#5389 reclassified the
    // missing-handler dispatch failure from Backend to InvalidInput so it
    // does not burn the retry budget on a call that can never resolve). The
    // capability still fails closed — only the disposition changed.
    assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
}

fn nearai_bootstrap_input_with_base(
    owner: &str,
    root: PathBuf,
    base_url: &str,
    api_key: &str,
) -> RebornHostBindings {
    crate::deployment::local_dev_build_input(owner, root).with_nearai_mcp_bootstrap_config(
        crate::llm_admin::nearai_mcp::NearAiMcpBootstrapConfig::new(
            base_url,
            secrecy::SecretString::from(api_key.to_string()),
        )
        .expect("valid NEAR AI MCP bootstrap config"),
    )
}

fn nearai_bootstrap_input(owner: &str, root: PathBuf, api_key: &str) -> RebornHostBindings {
    nearai_bootstrap_input_with_base(owner, root, "https://private.near.ai", api_key)
}

#[test]
fn hosted_single_tenant_nearai_mcp_bootstrap_scope_uses_runtime_identity() {
    let owner = UserId::new("hosted-nearai-owner").expect("owner");
    let identity = RuntimeOwnerIdentity {
        tenant_id: ironclaw_host_api::TenantId::new("hosted-nearai-tenant").expect("tenant"),
        agent_id: ironclaw_host_api::AgentId::new("hosted-nearai-agent").expect("agent"),
    };

    let scope = configured_runtime_owner_scope(owner.clone(), &identity);

    assert_eq!(scope.tenant_id, identity.tenant_id);
    assert_eq!(scope.user_id, owner);
    assert_eq!(scope.agent_id, Some(identity.agent_id));
    assert!(scope.project_id.is_none());
}

#[test]
fn turn_state_filesystem_routes_global_store_ops_to_owner_turns_path() {
    let root = Arc::new(ironclaw_filesystem::InMemoryBackend::default());
    let owner_scope = ResourceScope {
        tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
        user_id: UserId::new("owner-alpha").expect("owner"),
        agent_id: Some(ironclaw_host_api::AgentId::new("agent-alpha").expect("agent")),
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let scoped =
        owner_turn_state_filesystem(root, &owner_scope).expect("owner turn-state filesystem");
    let path = ScopedPath::new("/turns/state.json").expect("turn state path");
    let resolved = scoped
        .resolve(&ResourceScope::system(), &path)
        .expect("fixed view should resolve global store operation");

    assert_eq!(
        resolved.as_str(),
        "/tenants/tenant-alpha/users/owner-alpha/turns/state.json"
    );
}

#[test]
fn runtime_owner_scope_uses_configured_runtime_identity_for_turn_state() {
    let owner = UserId::new("configured-owner").expect("owner");
    let identity = RuntimeOwnerIdentity {
        tenant_id: TenantId::new("configured-tenant").expect("tenant"),
        agent_id: ironclaw_host_api::AgentId::new("configured-agent").expect("agent"),
    };
    let scope = configured_runtime_owner_scope(owner.clone(), &identity);

    assert_eq!(scope.tenant_id, identity.tenant_id);
    assert_eq!(scope.user_id, owner);
    assert_eq!(scope.agent_id, Some(identity.agent_id));
}

#[tokio::test]
async fn production_database_root_filesystem_mounts_canonical_runtime_roots() {
    let filesystem =
        production_database_root_filesystem(Arc::new(InMemoryBackend::new()), "production-test")
            .expect("production composite filesystem");
    let mounted_roots: Vec<String> = filesystem
        .mounts()
        .await
        .expect("production composite mounts")
        .into_iter()
        .map(|descriptor| descriptor.virtual_root.as_str().to_owned())
        .collect();
    assert_eq!(
        mounted_roots,
        vec![
            "/events",
            "/memory",
            "/projects",
            "/system/extensions",
            "/system/settings",
            "/system/skills",
            "/tenants",
        ]
    );
}

#[tokio::test]
async fn production_libsql_turn_state_uses_configured_runtime_identity() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(
        libsql::Builder::new_local(dir.path().join("reborn.db").display().to_string())
            .build()
            .await
            .expect("build libsql database"),
    );
    let assertion_filesystem = LibSqlRootFilesystem::new(Arc::clone(&db));
    let owner = UserId::new("configured-owner").expect("owner");
    let tenant = TenantId::new("configured-tenant").expect("tenant");
    let agent = ironclaw_host_api::AgentId::new("configured-agent").expect("agent");
    let services = build_runtime_substrate(
        RebornHostBindings::libsql(
            RebornCompositionProfile::Production,
            owner.as_str(),
            db,
            dir.path().join("events.db").display().to_string(),
            None,
            ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
        )
        .with_local_runtime_identity(tenant.clone(), agent.clone())
        .with_production_trust_policy(Arc::new(
            builtin_first_party_trust_policy().expect("builtin trust policy"),
        ))
        .with_runtime_policy(EffectiveRuntimePolicy {
            deployment: ironclaw_host_api::DeploymentMode::HostedMultiTenant,
            requested_profile: ironclaw_host_api::RuntimeProfile::HostedSafe,
            resolved_profile: ironclaw_host_api::RuntimeProfile::HostedSafe,
            filesystem_backend: FilesystemBackendKind::TenantWorkspace,
            process_backend: ProcessBackendKind::None,
            network_mode: ironclaw_host_api::NetworkMode::Brokered,
            secret_mode: SecretMode::TenantBroker,
            approval_policy: ironclaw_host_api::runtime_policy::ApprovalPolicy::AskAlways,
            audit_mode: ironclaw_host_api::AuditMode::Standard,
        }),
    )
    .await
    .expect("production libsql services build");

    let turn_state = &services.turn_state;
    // Runtime-store unification (branch `unify-runtime-store-graph`): every
    // build — production libsql included — now composes the single unified
    // runtime store graph (`extension_lifecycle_surface_context` is no longer
    // optional; `local_runtime_for_test` is unconditionally `Some`). The old
    // split-runtime premise ("production has no local runtime") no longer holds,
    // so this assertion tracks the new-but-correct unified shape. The test's
    // real subject — turn_state keyed by the configured runtime identity —
    // continues below.
    assert!(services.local_runtime_for_test().is_some());
    let scope = ironclaw_turns::TurnScope::new_with_owner(
        tenant,
        Some(agent),
        None,
        ironclaw_host_api::ThreadId::new("configured-thread").expect("thread"),
        Some(owner.clone()),
    );
    let submit = ironclaw_turns::SubmitTurnRequest {
        requested_model: None,
        scope,
        actor: ironclaw_turns::TurnActor::new(owner),
        accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("configured-message-ref")
            .expect("message ref"),
        source_binding_ref: ironclaw_turns::SourceBindingRef::new("source-web")
            .expect("source binding"),
        reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new("reply-web")
            .expect("reply binding"),
        requested_run_profile: Some(
            ironclaw_turns::RunProfileRequest::new("default").expect("run profile"),
        ),
        idempotency_key: ironclaw_turns::IdempotencyKey::new("configured-turn")
            .expect("idempotency key"),
        received_at: chrono::Utc::now(),
        requested_run_id: None,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
    };
    ironclaw_turns::TurnStateStore::submit_turn(
        turn_state.as_ref(),
        submit,
        &ironclaw_turns::AllowAllTurnAdmissionPolicy,
        &InMemoryRunProfileResolver::default(),
    )
    .await
    .expect("submit through production turn-state store");

    let configured_path = VirtualPath::new(
        "/tenants/configured-tenant/users/configured-owner/turns/rows/v1/deltas/log",
    )
    .expect("configured turn-state row delta log path");
    let system_path =
        VirtualPath::new("/tenants/__system__/users/__system__/turns/rows/v1/deltas/log")
            .expect("system turn-state row delta log path");

    assert!(
        append_log_has_entries(
            &assertion_filesystem,
            &configured_path,
            "configured turn-state row delta log read"
        )
        .await
    );
    assert!(
        !append_log_has_entries(
            &assertion_filesystem,
            &system_path,
            "system turn-state row delta log read"
        )
        .await
    );
}

#[tokio::test]
async fn production_libsql_turn_state_uses_default_runtime_identity_when_unconfigured() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(
        libsql::Builder::new_local(dir.path().join("reborn.db").display().to_string())
            .build()
            .await
            .expect("build libsql database"),
    );
    let assertion_filesystem = LibSqlRootFilesystem::new(Arc::clone(&db));
    let owner = UserId::new("default-owner").expect("owner");
    let services = build_runtime_substrate(
        RebornHostBindings::libsql(
            RebornCompositionProfile::Production,
            owner.as_str(),
            db,
            dir.path().join("events.db").display().to_string(),
            None,
            ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
        )
        .with_production_trust_policy(Arc::new(
            builtin_first_party_trust_policy().expect("builtin trust policy"),
        ))
        .with_runtime_policy(EffectiveRuntimePolicy {
            deployment: ironclaw_host_api::DeploymentMode::HostedMultiTenant,
            requested_profile: ironclaw_host_api::RuntimeProfile::HostedSafe,
            resolved_profile: ironclaw_host_api::RuntimeProfile::HostedSafe,
            filesystem_backend: FilesystemBackendKind::TenantWorkspace,
            process_backend: ProcessBackendKind::None,
            network_mode: ironclaw_host_api::NetworkMode::Brokered,
            secret_mode: SecretMode::TenantBroker,
            approval_policy: ironclaw_host_api::runtime_policy::ApprovalPolicy::AskAlways,
            audit_mode: ironclaw_host_api::AuditMode::Standard,
        }),
    )
    .await
    .expect("production libsql services build");

    let turn_state = &services.turn_state;
    let default_path =
        VirtualPath::new("/tenants/reborn-cli/users/default-owner/turns/rows/v1/deltas/log")
            .expect("default turn-state row delta log path");
    let system_path =
        VirtualPath::new("/tenants/__system__/users/__system__/turns/rows/v1/deltas/log")
            .expect("system turn-state row delta log path");
    let default_identity = RebornRuntimeIdentity::reborn_cli();
    let default_tenant = TenantId::new(default_identity.tenant_id).expect("default tenant");
    let scope = ironclaw_turns::TurnScope::new_with_owner(
        default_tenant,
        None,
        None,
        ironclaw_host_api::ThreadId::new("default-thread").expect("thread"),
        Some(owner.clone()),
    );
    let submit = ironclaw_turns::SubmitTurnRequest {
        requested_model: None,
        scope,
        actor: ironclaw_turns::TurnActor::new(owner),
        accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("default-message-ref")
            .expect("message ref"),
        source_binding_ref: ironclaw_turns::SourceBindingRef::new("source-web")
            .expect("source binding"),
        reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new("reply-web")
            .expect("reply binding"),
        requested_run_profile: Some(
            ironclaw_turns::RunProfileRequest::new("default").expect("run profile"),
        ),
        idempotency_key: ironclaw_turns::IdempotencyKey::new("default-turn")
            .expect("idempotency key"),
        received_at: chrono::Utc::now(),
        requested_run_id: None,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
    };
    ironclaw_turns::TurnStateStore::submit_turn(
        turn_state.as_ref(),
        submit,
        &ironclaw_turns::AllowAllTurnAdmissionPolicy,
        &InMemoryRunProfileResolver::default(),
    )
    .await
    .expect("submit through production turn-state store");

    assert!(
        append_log_has_entries(
            &assertion_filesystem,
            &default_path,
            "default turn-state row delta log read"
        )
        .await
    );
    assert!(
        !append_log_has_entries(
            &assertion_filesystem,
            &system_path,
            "system turn-state row delta log read"
        )
        .await
    );
}

async fn append_log_has_entries<F>(filesystem: &F, path: &VirtualPath, label: &str) -> bool
where
    F: RootFilesystem,
{
    match filesystem
        .tail(path, ironclaw_filesystem::SeqNo::ZERO)
        .await
    {
        Ok(entries) => !entries.is_empty(),
        Err(ironclaw_filesystem::FilesystemError::NotFound { .. }) => false,
        Err(error) => panic!("{label}: {error}"),
    }
}

#[tokio::test]
async fn production_libsql_builder_rejects_invalid_owner_id_at_composition_boundary() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(
        libsql::Builder::new_local(dir.path().join("reborn.db").display().to_string())
            .build()
            .await
            .expect("build libsql database"),
    );

    let result = build_runtime_substrate(
        RebornHostBindings::libsql(
            RebornCompositionProfile::Production,
            "",
            db,
            dir.path().join("events.db").display().to_string(),
            None,
            ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
        )
        .with_production_trust_policy(Arc::new(
            builtin_first_party_trust_policy().expect("builtin trust policy"),
        ))
        .with_runtime_policy(EffectiveRuntimePolicy {
            deployment: ironclaw_host_api::DeploymentMode::HostedMultiTenant,
            requested_profile: ironclaw_host_api::RuntimeProfile::HostedSafe,
            resolved_profile: ironclaw_host_api::RuntimeProfile::HostedSafe,
            filesystem_backend: FilesystemBackendKind::TenantWorkspace,
            process_backend: ProcessBackendKind::None,
            network_mode: ironclaw_host_api::NetworkMode::Brokered,
            secret_mode: SecretMode::TenantBroker,
            approval_policy: ironclaw_host_api::runtime_policy::ApprovalPolicy::AskAlways,
            audit_mode: ironclaw_host_api::AuditMode::Standard,
        }),
    )
    .await;

    assert!(
        matches!(result, Err(RebornBuildError::InvalidConfig { ref reason }) if reason.contains("must not be empty")),
        "expected invalid owner id error, got {result:?}"
    );
}

#[tokio::test]
async fn local_dev_nearai_mcp_auto_bootstraps_from_injected_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let owner = "local-dev-nearai-mcp-owner";
    let services = build_runtime_substrate(nearai_bootstrap_input_with_base(
        owner,
        dir.path().join("local-dev"),
        "https://nearai-db.example.test:9443/v1",
        "nearai-test-key",
    ))
    .await
    .expect("local-dev services build");
    let runtime_surfaces = services.local_runtime_for_test().expect("local runtime");
    let extension_management = &runtime_surfaces.extension_management;
    let nearai_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "nearai").expect("valid ref");

    // #6520 lifecycle projection is caller-scoped and takes the production
    // credential gate; the owner is the operator this runtime was built with.
    let owner_scope =
        default_runtime_owner_scope(UserId::new(owner).unwrap()).expect("NEAR AI MCP owner scope");
    let credential_gate = crate::extension_host::extension_activation_credentials::RuntimeExtensionActivationCredentialGate::new(
        owner_scope.clone(),
        services
            .product_auth
            .runtime_credential_account_selection_service(),
    );
    let projection = extension_management
        .project(
            nearai_ref.clone(),
            &owner_scope.user_id,
            Some(&credential_gate),
        )
        .await
        .expect("NEAR AI MCP projected");
    assert_eq!(projection.phase, LifecyclePublicState::Active);

    // v3 hosted-MCP surface: boot-time bootstrap activates the package
    // statically, publishing the host-internal MCP connection template
    // plus the statically pinned web_search tool (main parity: searchable
    // from first boot); live tools/list discovery replaces the static set
    // with the server's catalog.
    let capabilities = extension_management
        .active_model_visible_capabilities()
        .await
        .expect("active capabilities");
    assert_eq!(
        capabilities
            .iter()
            .filter(|capability| capability.provider.as_str() == "nearai")
            .map(|capability| capability.id.as_str())
            .collect::<Vec<_>>(),
        vec!["nearai.web_search"],
        "activated hosted-MCP package must pin exactly the static web_search tool before discovery"
    );
    let template_id = CapabilityId::new("nearai.mcp_server").unwrap();
    let registry = extension_management.active_extensions_for_test().snapshot();
    assert!(
        registry.get_capability(&template_id).is_some(),
        "host-internal MCP connection template should be published"
    );
    assert_eq!(
        registry.capability_visibility(&template_id),
        Some(ironclaw_extensions::CapabilityVisibility::HostInternal)
    );

    // Script live tools/list discovery through the hosted-MCP seam so the
    // discovered web_search tool surfaces with the connection template's
    // credential wiring (the injected endpoint override patches
    // [mcp].server only; the audience derives from that server host).
    extension_management
        .activate_with_prechecked_credentials_for_test(
            nearai_ref,
            ExtensionActivationMode::HostedMcpDiscovery {
                scope: ResourceScope::local_default(
                    UserId::new(owner).unwrap(),
                    InvocationId::new(),
                )
                .expect("valid scope"),
                runtime_http_egress: Arc::new(HostedMcpDiscoveryEgress::with_tool_name(
                    "web_search",
                )),
            },
            &UserId::new(owner).expect("valid lifecycle caller"),
        )
        .await
        .expect("scripted NEAR AI discovery activation");

    let capabilities = extension_management
        .active_model_visible_capabilities()
        .await
        .expect("active capabilities");
    let search = capabilities
        .iter()
        .find(|capability| capability.id.as_str() == "nearai.web_search")
        .expect("nearai.web_search active");

    assert_eq!(search.provider.as_str(), "nearai");
    assert_eq!(search.effects, nearai_allowed_effects());
    assert_eq!(search.runtime_credentials.len(), 1);
    assert_eq!(
        search.runtime_credentials[0].handle,
        SecretHandle::new("llm_nearai_api_key").unwrap()
    );
    assert_eq!(
        search.runtime_credentials[0].source,
        RuntimeCredentialRequirementSource::ProductAuthAccount {
            provider: VendorId::new("nearai").unwrap(),
            setup: Default::default(),
        }
    );
    assert_eq!(
        search.runtime_credentials[0].audience.host_pattern,
        "nearai-db.example.test"
    );
    // v3 derives the credential audience from the [mcp].server host; the
    // audience pattern carries the host only (port unconstrained).
    assert_eq!(search.runtime_credentials[0].audience.port, None);

    let auth_scope = AuthProductScope::new(
        default_runtime_owner_scope(UserId::new(owner).unwrap()).expect("NEAR AI MCP owner scope"),
        AuthSurface::Api,
    );
    let accounts = services
        .product_auth
        .as_ref()
        .credential_account_record_source()
        .accounts_for_owner(&auth_scope)
        .await
        .expect("credential accounts load");
    let nearai_account = accounts
        .iter()
        .find(|account| account.provider.as_str() == "nearai")
        .expect("NEAR AI product-auth account");
    assert_eq!(nearai_account.status, CredentialAccountStatus::Configured);
    assert!(nearai_account.access_secret.is_some());
    let nearai_access_secret = nearai_account
        .access_secret
        .clone()
        .expect("NEAR AI product-auth access secret");
    let nearai_account_scope = nearai_account.scope.resource.clone();
    let resolver = ProductAuthRuntimeCredentialResolver::new_with_refresh(
        services
            .product_auth
            .runtime_credential_account_selection_service(),
        services
            .product_auth
            .runtime_credential_account_refresh_service(),
    );
    let sso_scope = ResourceScope {
        tenant_id: nearai_account_scope.tenant_id.clone(),
        user_id: UserId::new("local-dev-nearai-mcp-sso-user").unwrap(),
        agent_id: nearai_account_scope.agent_id.clone(),
        project_id: nearai_account_scope.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &sso_scope,
            provider: &VendorId::new("nearai").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("nearai").unwrap(),
        })
        .await
        .expect("SSO user should resolve host-managed NEAR AI credential");
    assert_eq!(resolved.handle, nearai_access_secret);
    assert_eq!(resolved.scope, nearai_account_scope);
}

#[tokio::test]
async fn local_dev_nearai_mcp_rebootstrap_reuses_existing_account() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("local-dev");
    let owner = "local-dev-nearai-mcp-idempotent-owner";
    let auth_scope = AuthProductScope::new(
        default_runtime_owner_scope(UserId::new(owner).unwrap()).expect("NEAR AI MCP owner scope"),
        AuthSurface::Api,
    );

    let first = build_runtime_substrate(nearai_bootstrap_input(owner, root, "nearai-first-key"))
        .await
        .expect("first local-dev services build");
    let first_account = first
        .product_auth
        .as_ref()
        .credential_account_record_source()
        .accounts_for_owner(&auth_scope)
        .await
        .expect("credential accounts load")
        .into_iter()
        .find(|account| account.provider.as_str() == "nearai")
        .expect("NEAR AI product-auth account");
    let extension_management = &first
        .local_runtime_for_test()
        .expect("local runtime")
        .extension_management;
    let outcome = crate::llm_admin::nearai_mcp::bootstrap_nearai_mcp(
        Some(
            crate::llm_admin::nearai_mcp::NearAiMcpBootstrapConfig::new(
                "https://private.near.ai",
                secrecy::SecretString::from("nearai-second-key"),
            )
            .expect("valid NEAR AI MCP bootstrap config"),
        ),
        &first.product_auth,
        extension_management,
        auth_scope.resource.clone(),
    )
    .await
    .expect("second NEAR AI MCP bootstrap");
    assert_eq!(
        outcome,
        crate::llm_admin::nearai_mcp::NearAiMcpBootstrapOutcome::ReusedCredential
    );
    let accounts = first
        .product_auth
        .credential_account_record_source()
        .accounts_for_owner(&auth_scope)
        .await
        .expect("credential accounts load");
    let nearai_accounts = accounts
        .iter()
        .filter(|account| account.provider.as_str() == "nearai")
        .collect::<Vec<_>>();

    assert_eq!(nearai_accounts.len(), 1);
    assert_eq!(nearai_accounts[0].id, first_account.id);
    assert_eq!(
        nearai_accounts[0].access_secret,
        first_account.access_secret
    );
    assert_eq!(nearai_accounts[0].updated_at, first_account.updated_at);
    assert_eq!(
        nearai_accounts[0].status,
        CredentialAccountStatus::Configured
    );
}

#[tokio::test]
async fn local_dev_nearai_mcp_bootstrap_reinstalls_discovered_reused_credential() {
    let dir = tempfile::tempdir().expect("tempdir");
    let owner = "local-dev-nearai-mcp-discovered-owner";
    let nearai_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "nearai").expect("valid ref");

    let services = build_runtime_substrate(nearai_bootstrap_input(
        owner,
        dir.path().join("local-dev"),
        "nearai-test-key",
    ))
    .await
    .expect("local-dev services build");
    let extension_management = &services
        .local_runtime_for_test()
        .expect("local runtime")
        .extension_management;
    let removal_scope = ironclaw_host_api::ResourceScope::local_default(
        ironclaw_host_api::UserId::new(owner).expect("valid user"),
        ironclaw_host_api::InvocationId::new(),
    )
    .expect("valid scope");
    extension_management
        .remove(
            nearai_ref.clone(),
            &removal_scope,
            Some(&removal_scope.user_id),
        )
        .await
        .expect("disable NEAR AI MCP extension");
    let outcome = crate::llm_admin::nearai_mcp::bootstrap_nearai_mcp(
        Some(
            crate::llm_admin::nearai_mcp::NearAiMcpBootstrapConfig::new(
                "https://private.near.ai",
                secrecy::SecretString::from("nearai-test-key"),
            )
            .expect("valid NEAR AI MCP bootstrap config"),
        ),
        &services.product_auth,
        extension_management,
        default_runtime_owner_scope(UserId::new(owner).unwrap()).expect("NEAR AI MCP owner scope"),
    )
    .await
    .expect("bootstrap should reinstall discovered extension");
    assert_eq!(
        outcome,
        crate::llm_admin::nearai_mcp::NearAiMcpBootstrapOutcome::Activated
    );
    // #6520 lifecycle projection is caller-scoped and takes the production
    // credential gate; the owner is the operator this runtime was built with.
    let owner_scope =
        default_runtime_owner_scope(UserId::new(owner).unwrap()).expect("NEAR AI MCP owner scope");
    let credential_gate = crate::extension_host::extension_activation_credentials::RuntimeExtensionActivationCredentialGate::new(
        owner_scope.clone(),
        services
            .product_auth
            .runtime_credential_account_selection_service(),
    );
    let projection = extension_management
        .project(nearai_ref, &owner_scope.user_id, Some(&credential_gate))
        .await
        .expect("NEAR AI MCP projected");
    assert_eq!(projection.phase, LifecyclePublicState::Active);

    // v3 hosted-MCP surface: reinstall-and-activate publishes the
    // host-internal MCP connection template plus the statically pinned
    // web_search tool (main parity: searchable from first boot); a
    // successful live tools/list discovery — which this bootstrap-focused
    // test does not run — replaces the static set with the live catalog.
    let capabilities = extension_management
        .active_model_visible_capabilities()
        .await
        .expect("active capabilities");
    assert_eq!(
        capabilities
            .iter()
            .filter(|capability| capability.provider.as_str() == "nearai")
            .map(|capability| capability.id.as_str())
            .collect::<Vec<_>>(),
        vec!["nearai.web_search"],
        "reinstalled hosted-MCP package must pin exactly the static web_search tool before discovery"
    );
    let template_id = CapabilityId::new("nearai.mcp_server").unwrap();
    let registry = extension_management.active_extensions_for_test().snapshot();
    assert!(
        registry.get_capability(&template_id).is_some(),
        "host-internal MCP connection template should be published"
    );
    assert_eq!(
        registry.capability_visibility(&template_id),
        Some(ironclaw_extensions::CapabilityVisibility::HostInternal)
    );
}

#[tokio::test]
async fn local_dev_nearai_mcp_invalid_base_url_fails_build() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = crate::llm_admin::nearai_mcp::NearAiMcpBootstrapConfig::new(
        "http://private.near.ai",
        secrecy::SecretString::from("nearai-test-key"),
    )
    .expect("config shape");
    let error = build_runtime_substrate(
        crate::deployment::local_dev_build_input(
            "local-dev-nearai-mcp-invalid-owner",
            dir.path().join("local-dev"),
        )
        .with_nearai_mcp_bootstrap_config(config),
    )
    .await
    .expect_err("invalid endpoint should fail build");

    let RebornBuildError::InvalidConfig { reason } = error else {
        panic!("expected invalid config");
    };
    assert!(reason.contains("NEARAI_BASE_URL must use https"));
}

#[test]
fn attach_hosted_mcp_runtime_skips_services_without_runtime_http_egress() {
    let services = HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    );

    let services = attach_hosted_mcp_runtime(services).expect("attach is optional");

    assert!(services.product_auth_provider_runtime_ports().is_none());
}

#[tokio::test]
async fn local_dev_services_persist_thread_records_across_rebuilds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("local-dev");
    let scope = ironclaw_threads::ThreadScope {
        tenant_id: ironclaw_host_api::TenantId::new("persist-tenant").unwrap(),
        agent_id: ironclaw_host_api::AgentId::new("persist-agent").unwrap(),
        project_id: None,
        owner_user_id: Some(ironclaw_host_api::UserId::new("persist-owner").unwrap()),
        mission_id: None,
    };
    let thread_id = ironclaw_host_api::ThreadId::new("persisted-thread").unwrap();

    let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
        "persist-owner",
        root.clone(),
    ))
    .await
    .expect("first local-dev services build");
    services
        .local_runtime_for_test()
        .expect("local runtime")
        .thread_service
        .ensure_thread(ironclaw_threads::EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "persist-owner".to_string(),
            title: Some("Persisted thread".to_string()),
            metadata_json: None,
        })
        .await
        .expect("persist thread");
    drop(services);

    let rebuilt = build_runtime_substrate(crate::deployment::local_dev_build_input(
        "persist-owner",
        root.clone(),
    ))
    .await
    .expect("rebuilt local-dev services");
    let history = rebuilt
        .local_runtime_for_test()
        .expect("rebuilt local runtime")
        .thread_service
        .list_thread_history(ironclaw_threads::ThreadHistoryRequest {
            scope,
            thread_id: thread_id.clone(),
        })
        .await
        .expect("read persisted thread");

    assert_eq!(history.thread.thread_id, thread_id);
    assert!(
        root.join("reborn-local-dev.db").exists(),
        "local-dev should use a libSQL database under the local-dev root"
    );
}

#[tokio::test]
async fn local_dev_setup_marker_workspace_filesystem_is_read_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    let marker_path = storage_root.join("workspace/markers/setup.done");
    std::fs::create_dir_all(marker_path.parent().expect("marker parent"))
        .expect("marker directory");
    std::fs::write(&marker_path, "done").expect("marker file");
    let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
        "local-dev-marker-workspace-owner",
        storage_root,
    ))
    .await
    .expect("local-dev services build");
    let runtime_surfaces = services
        .local_runtime_for_test()
        .expect("local-dev runtime substrate");
    let scope = ResourceScope::local_default(
        UserId::new("local-dev-marker-user").expect("valid user"),
        InvocationId::new(),
    )
    .expect("valid resource scope");

    let stat = runtime_surfaces
        .workspace_filesystem_for_test()
        .stat(
            &scope,
            &ScopedPath::new("/workspace/markers/setup.done").expect("valid marker path"),
        )
        .await
        .expect("marker stat succeeds");
    assert_eq!(stat.len, 4);

    let error = runtime_surfaces
        .workspace_filesystem_for_test()
        .write_file(
            &scope,
            &ScopedPath::new("/workspace/markers/new.done").expect("valid marker path"),
            b"done",
        )
        .await
        .expect_err("setup marker workspace filesystem should be read-only");
    assert!(matches!(error, FilesystemError::PermissionDenied { .. }));
}

#[tokio::test]
async fn local_dev_skill_management_invokes_through_first_party_runtime() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
        "local-dev-skill-tools-owner",
        storage_root.clone(),
    ))
    .await
    .expect("local-dev services build");

    let install_output = invoke_json(
        &services,
        SKILL_INSTALL_CAPABILITY_ID,
        skill_context(SKILL_INSTALL_CAPABILITY_ID),
        serde_json::json!({
            "content": skill_md("runtime-sentinel", "runtime skill", "RUNTIME_SENTINEL")
        }),
    )
    .await
    .expect("skill install succeeds");
    assert_eq!(install_output["installed"], true);
    assert_eq!(install_output["name"], "runtime-sentinel");
    assert!(
        storage_root
            .join("tenants/default/users/local-dev-test-user/skills/runtime-sentinel/SKILL.md")
            .exists()
    );

    let list_output = invoke_json(
        &services,
        SKILL_LIST_CAPABILITY_ID,
        skill_context(SKILL_LIST_CAPABILITY_ID),
        serde_json::json!({}),
    )
    .await
    .expect("skill list succeeds");
    assert!(
        list_output["skills"]
            .as_array()
            .unwrap()
            .iter()
            .any(|skill| { skill["name"] == "runtime-sentinel" && skill["source"] == "user" })
    );

    let update_output = invoke_json(
        &services,
        SKILL_UPDATE_CAPABILITY_ID,
        skill_context(SKILL_UPDATE_CAPABILITY_ID),
        serde_json::json!({
            "name": "runtime-sentinel",
            "content": skill_md("runtime-sentinel", "updated runtime skill", "UPDATED_SENTINEL")
        }),
    )
    .await
    .expect("skill update succeeds");
    assert_eq!(update_output["updated"], true);
    assert_eq!(update_output["name"], "runtime-sentinel");

    let auto_activate_output = invoke_json(
        &services,
        SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID,
        skill_context(SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID),
        serde_json::json!({
            "name": "runtime-sentinel",
            "enabled": false
        }),
    )
    .await
    .expect("skill auto-activate update succeeds");
    assert_eq!(auto_activate_output["updated"], true);
    assert_eq!(auto_activate_output["name"], "runtime-sentinel");
    assert_eq!(auto_activate_output["auto_activate"], false);
    let updated_skill = std::fs::read_to_string(
        storage_root
            .join("tenants/default/users/local-dev-test-user/skills/runtime-sentinel/SKILL.md"),
    )
    .expect("updated skill");
    assert!(updated_skill.contains("auto_activate: false"));

    let remove_output = invoke_json(
        &services,
        SKILL_REMOVE_CAPABILITY_ID,
        skill_context(SKILL_REMOVE_CAPABILITY_ID),
        serde_json::json!({"name": "runtime-sentinel"}),
    )
    .await
    .expect("skill remove succeeds");
    assert_eq!(remove_output["removed"], true);
    assert!(
        !storage_root
            .join("tenants/default/users/local-dev-test-user/skills/runtime-sentinel/SKILL.md")
            .exists()
    );
}

#[tokio::test]
async fn local_dev_workspace_mounts_do_not_authorize_skill_writes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
        "local-dev-workspace-skill-boundary-owner",
        storage_root.clone(),
    ))
    .await
    .expect("local-dev services build");

    let failure = invoke_json(
        &services,
        "builtin.write_file",
        workspace_context("builtin.write_file"),
        serde_json::json!({
            "path": "/skills/blocked/SKILL.md",
            "content": skill_md("blocked", "blocked skill", "BLOCKED")
        }),
    )
    .await
    .expect_err("workspace tool cannot write skill root");

    assert_eq!(failure, RuntimeFailureKind::Authorization);
    assert!(
        !storage_root
            .join("tenants/default/users/local-dev-test-user/skills/blocked/SKILL.md")
            .exists()
    );
}

#[test]
fn local_dev_workspace_root_overlapping_skill_root_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");

    for skill_root in [
        storage_root.join("skills"),
        storage_root.join("tenant-shared/skills"),
        storage_root.join("system/skills"),
    ] {
        for workspace_root in [
            skill_root.clone(),
            skill_root
                .parent()
                .expect("skill root parent")
                .to_path_buf(),
            skill_root.join("nested-workspace"),
        ] {
            let error =
                validate_local_dev_workspace_skill_isolation(&storage_root, &workspace_root)
                    .expect_err("workspace root overlapping skill root should be rejected");
            assert!(
                matches!(error, RebornBuildError::InvalidConfig { .. }),
                "unexpected error: {error:?}"
            );
        }
    }
}

#[test]
fn local_dev_legacy_skill_backfill_marker_preserves_deletions() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    let legacy_skill_dir = storage_root.join("skills/legacy-skill");
    std::fs::create_dir_all(&legacy_skill_dir).expect("legacy skill dir");
    std::fs::write(legacy_skill_dir.join("SKILL.md"), "legacy skill").expect("legacy skill");
    let owner_user_id = UserId::new("owner").expect("owner");

    backfill_local_dev_legacy_user_skills(&storage_root, &owner_user_id).expect("initial backfill");
    let scoped_skill_dir = storage_root.join("tenants/default/users/owner/skills/legacy-skill");
    let reborn_cli_skill_dir =
        storage_root.join("tenants/reborn-cli/users/owner/skills/legacy-skill");
    assert!(scoped_skill_dir.join("SKILL.md").exists());
    assert!(reborn_cli_skill_dir.join("SKILL.md").exists());

    std::fs::remove_dir_all(&scoped_skill_dir).expect("delete migrated skill");
    backfill_local_dev_legacy_user_skills(&storage_root, &owner_user_id).expect("second backfill");
    assert!(
        !scoped_skill_dir.exists(),
        "one-time legacy backfill must not resurrect user-deleted migrated skills"
    );
}

#[cfg(unix)]
#[test]
fn local_dev_legacy_skill_backfill_skips_symlinks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("local-dev");
    let legacy_root = storage_root.join("skills");
    let target_dir = storage_root.join("target-skill");
    std::fs::create_dir_all(&legacy_root).expect("legacy root");
    std::fs::create_dir_all(&target_dir).expect("target dir");
    std::os::unix::fs::symlink(&target_dir, legacy_root.join("linked-skill"))
        .expect("legacy symlink");
    let owner_user_id = UserId::new("owner").expect("owner");

    backfill_local_dev_legacy_user_skills(&storage_root, &owner_user_id)
        .expect("symlink should be skipped, not fail startup");
    assert!(
        !storage_root
            .join("tenants/default/users/owner/skills/linked-skill")
            .exists()
    );
    assert!(
        storage_root
            .join(format!(
                "tenants/default/users/owner/skills/{LOCAL_DEV_LEGACY_SKILLS_BACKFILL_MARKER}"
            ))
            .exists(),
        "migration should still be marked complete after skipping symlinks"
    );
}

#[test]
fn builtin_first_party_package_declares_skill_management_tools() {
    let package = builtin_first_party_package().expect("built-in package builds");
    let ids = package
        .capabilities
        .iter()
        .map(|capability| capability.id.as_str())
        .collect::<Vec<_>>();
    assert!(ids.contains(&SKILL_LIST_CAPABILITY_ID));
    assert!(!ids.contains(&SKILL_ACTIVATE_CAPABILITY_ID));
    assert!(ids.contains(&SKILL_INSTALL_CAPABILITY_ID));
    assert!(ids.contains(&SKILL_UPDATE_CAPABILITY_ID));
    assert!(ids.contains(&SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID));
    assert!(ids.contains(&SKILL_REMOVE_CAPABILITY_ID));
    assert!(ids.contains(&TRIGGER_CREATE_CAPABILITY_ID));
    assert!(ids.contains(&TRIGGER_LIST_CAPABILITY_ID));
    assert!(ids.contains(&TRIGGER_REMOVE_CAPABILITY_ID));

    let registry = ironclaw_host_runtime::builtin_first_party_handlers(Arc::new(
        ironclaw_triggers::InMemoryTriggerRepository::default(),
    ))
    .expect("built-in handlers build");
    for id in [
        SKILL_LIST_CAPABILITY_ID,
        SKILL_INSTALL_CAPABILITY_ID,
        SKILL_UPDATE_CAPABILITY_ID,
        SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID,
        SKILL_REMOVE_CAPABILITY_ID,
        TRIGGER_CREATE_CAPABILITY_ID,
        TRIGGER_LIST_CAPABILITY_ID,
        TRIGGER_REMOVE_CAPABILITY_ID,
    ] {
        assert!(registry.contains_handler(&ironclaw_host_api::CapabilityId::new(id).unwrap()));
    }
    assert!(!registry.contains_handler(
        &ironclaw_host_api::CapabilityId::new(SKILL_ACTIVATE_CAPABILITY_ID).unwrap()
    ));
}

#[test]
fn production_skill_management_mounts_use_production_namespace() {
    let scope = ResourceScope {
        tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
        user_id: UserId::new("alice").expect("user"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };

    let mounts = production_skill_management_mount_view(&scope).expect("mount view");
    let skills_mount = mounts
        .mounts
        .iter()
        .find(|mount| mount.alias.as_str() == "/skills")
        .expect("skills mount");
    assert_eq!(
        skills_mount.target.as_str(),
        "/tenants/tenant-alpha/users/alice/skills"
    );
    let system_mount = mounts
        .mounts
        .iter()
        .find(|mount| mount.alias.as_str() == "/system/skills")
        .expect("system skills mount");
    assert_eq!(system_mount.target.as_str(), "/system/skills");
}

#[test]
fn production_readiness_reflects_product_auth_presence() {
    let without_auth = readiness_for(RebornCompositionProfile::Production, true, true, false);
    assert_eq!(
        without_auth.state,
        RebornReadinessState::ProductionValidated
    );
    assert!(!without_auth.facades.product_auth);
    assert!(without_auth.diagnostics.is_empty());

    let with_auth = readiness_for(RebornCompositionProfile::Production, true, true, true);
    assert_eq!(with_auth.state, RebornReadinessState::ProductionValidated);
    assert!(with_auth.facades.product_auth);
    assert!(with_auth.diagnostics.is_empty());
}

#[test]
fn readiness_for_profile_diagnostics_cover_cutover_states() {
    let migration = readiness_for(RebornCompositionProfile::MigrationDryRun, true, true, true);
    assert_eq!(
        migration.state,
        RebornReadinessState::MigrationDryRunValidated
    );
    assert!(migration.diagnostics.is_empty());

    let yolo = readiness_for(RebornCompositionProfile::LocalDevYolo, true, true, true);
    assert_eq!(yolo.state, RebornReadinessState::DevOnly);
    assert_eq!(
        yolo.diagnostics,
        vec![RebornReadinessDiagnostic::local_dev_yolo()]
    );

    let hosted_volume = readiness_for(
        RebornCompositionProfile::HostedSingleTenantVolume,
        true,
        true,
        true,
    );
    assert_eq!(
        hosted_volume.state,
        RebornReadinessState::HostedSingleTenantVolumePreviewValidated
    );
    assert_eq!(
        hosted_volume.diagnostics,
        vec![RebornReadinessDiagnostic::hosted_single_tenant_volume()]
    );
}

async fn invoke_json(
    services: &RebornRuntimeStores,
    capability_id: &str,
    context: ExecutionContext,
    input: serde_json::Value,
) -> Result<serde_json::Value, RuntimeFailureKind> {
    crate::approval_test_support::invoke_json_with_local_dev_approval(
        services,
        capability_id,
        context,
        input,
    )
    .await
}

fn skill_context(capability_id: &str) -> ExecutionContext {
    execution_context(capability_id, skill_mounts())
}

fn workspace_context(capability_id: &str) -> ExecutionContext {
    execution_context(capability_id, workspace_mounts())
}

fn memory_context(capability_id: &str) -> ExecutionContext {
    execution_context(
        capability_id,
        memory_mount_view(MountPermissions::read_write_list_delete()).expect("valid memory mounts"),
    )
}

fn gsuite_context(capability_id: &str) -> ExecutionContext {
    let extension_id = ExtensionId::new("caller").expect("valid extension id");
    let mut context = ExecutionContext::local_default(
        UserId::new("local-dev-test-user").expect("valid user id"),
        extension_id.clone(),
        RuntimeKind::FirstParty,
        TrustClass::FirstParty,
        CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: CapabilityId::new(capability_id).expect("valid capability id"),
                grantee: Principal::Extension(extension_id),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![
                        EffectKind::DispatchCapability,
                        EffectKind::Network,
                        EffectKind::UseSecret,
                        EffectKind::ExternalWrite,
                    ],
                    mounts: MountView::new(Vec::new()).expect("valid empty mount view"),
                    network: NetworkPolicy::default(),
                    secrets: vec![SecretHandle::new("missing-google-access-token").unwrap()],
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        },
        MountView::new(Vec::new()).expect("valid empty mount view"),
    )
    .expect("valid execution context");
    context.run_id = Some(RunId::new());
    context
}

/// Turn on the global auto-approve switch for `context`'s actor scope so a
/// host-runtime dispatch exercises the tool path instead of stopping at the
/// per-tool approval gate. The Tools-settings switch is authoritative for
/// first-party tool dispatch; enabling it here mirrors the operator
/// having flipped it on before letting the agent run tools.
async fn enable_global_auto_approve_for_context(
    runtime_surfaces: &RebornRuntimeStores,
    context: &ExecutionContext,
) {
    runtime_surfaces
        .auto_approve_settings_for_test()
        .set(AutoApproveSettingInput {
            updated_by: Principal::User(context.resource_scope.user_id.clone()),
            scope: context.resource_scope.clone(),
            enabled: true,
        })
        .await
        .expect("enabling global auto-approve should succeed");
}

use crate::approval_test_support::disable_global_auto_approve;

fn notion_mcp_context(capability_id: &str) -> ExecutionContext {
    let extension_id = ExtensionId::new("caller").expect("valid extension id");
    let mut context = ExecutionContext::local_default(
        UserId::new("local-dev-test-user").expect("valid user id"),
        extension_id.clone(),
        RuntimeKind::Mcp,
        TrustClass::Sandbox,
        CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: CapabilityId::new(capability_id).expect("valid capability id"),
                grantee: Principal::Extension(extension_id),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: notion_mcp_allowed_effects(),
                    mounts: MountView::new(Vec::new()).expect("valid empty mount view"),
                    network: notion_mcp_network_policy(),
                    secrets: vec![SecretHandle::new("mcp_notion_access_token").unwrap()],
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        },
        MountView::new(Vec::new()).expect("valid empty mount view"),
    )
    .expect("valid execution context");
    context.run_id = Some(RunId::new());
    context
}

fn web_access_context(capability_id: &str) -> ExecutionContext {
    let extension_id = ExtensionId::new("caller").expect("valid extension id");
    let mut context = ExecutionContext::local_default(
        UserId::new("local-dev-test-user").expect("valid user id"),
        extension_id.clone(),
        RuntimeKind::FirstParty,
        TrustClass::FirstParty,
        CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: CapabilityId::new(capability_id).expect("valid capability id"),
                grantee: Principal::Extension(extension_id),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
                    mounts: MountView::new(Vec::new()).expect("valid empty mount view"),
                    network: web_access_network_policy(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        },
        MountView::new(Vec::new()).expect("valid empty mount view"),
    )
    .expect("valid execution context");
    context.run_id = Some(RunId::new());
    context
}

fn web_access_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(ironclaw_host_api::NetworkScheme::Https),
            host_pattern: "mcp.exa.ai".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: None,
    }
}

fn execution_context(capability_id: &str, mounts: MountView) -> ExecutionContext {
    let extension_id = ExtensionId::new("caller").expect("valid extension id");
    let mut context = ExecutionContext::local_default(
        UserId::new("local-dev-test-user").expect("valid user id"),
        extension_id.clone(),
        RuntimeKind::FirstParty,
        TrustClass::FirstParty,
        CapabilitySet {
            grants: vec![capability_grant(
                capability_id,
                extension_id,
                mounts.clone(),
            )],
        },
        mounts,
    )
    .expect("valid execution context");
    context.run_id = Some(RunId::new());
    context
}

fn capability_grant(
    capability_id: &str,
    grantee: ExtensionId,
    mounts: MountView,
) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: CapabilityId::new(capability_id).expect("valid capability id"),
        grantee: Principal::Extension(grantee),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: allowed_effects(),
            mounts,
            network: network_policy(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

fn skill_mounts() -> MountView {
    let scope = ironclaw_host_api::ResourceScope::local_default(
        UserId::new("local-dev-test-user").expect("valid user id"),
        ironclaw_host_api::InvocationId::new(),
    )
    .expect("valid resource scope");
    crate::local_dev_mounts::scoped_skill_management_mount_view(&scope).expect("valid skill mounts")
}

fn workspace_mounts() -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").expect("valid mount alias"),
        VirtualPath::new("/projects/workspace").expect("valid virtual path"),
        MountPermissions::read_write(),
    )])
    .expect("valid mount view")
}

fn allowed_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::ReadFilesystem,
        EffectKind::WriteFilesystem,
        EffectKind::DeleteFilesystem,
        EffectKind::Network,
    ]
}

fn network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: None,
            host_pattern: "*".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: None,
    }
}

fn notion_mcp_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "mcp.notion.com".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: None,
    }
}

fn notion_mcp_allowed_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::Network,
        EffectKind::UseSecret,
    ]
}

fn local_dev_minimal_approval_policy() -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy
{
    let mut policy = crate::local_dev_runtime_policy().expect("local-dev policy resolves");
    policy.requested_profile = ironclaw_host_api::runtime_policy::RuntimeProfile::LocalYolo;
    policy.resolved_profile = ironclaw_host_api::runtime_policy::RuntimeProfile::LocalYolo;
    policy.approval_policy = ironclaw_host_api::runtime_policy::ApprovalPolicy::Minimal;
    policy
}

fn skill_md(name: &str, description: &str, prompt: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n{prompt}\n")
}

/// Verify that the durable `local_dev_outbound_store` bundle (libsql or postgres)
/// shares a single `FilesystemOutboundStateStore` allocation across all four
/// trait-object roles.
///
/// The assertion reads the four trait-object pointers from the built
/// `RebornRuntimeStores` and compares their data halves via
/// `std::ptr::addr_eq` (trait objects of different traits cannot be compared
/// with `Arc::ptr_eq` directly).
#[tokio::test]
async fn local_dev_outbound_store_durable_shares_one_allocation_across_all_roles() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
        "outbound-store-alloc-owner",
        dir.path().join("local-dev"),
    ))
    .await
    .expect("local-dev services build");

    let runtime_surfaces = services.local_runtime_for_test().expect("local runtime");

    // Cast each fat-pointer's data half to *const () for cross-trait comparison.
    let pref_ptr = Arc::as_ptr(runtime_surfaces.outbound_preferences_for_test()) as *const ();
    let state_ptr = Arc::as_ptr(runtime_surfaces.outbound_state_for_test()) as *const ();
    let gate_ptr = Arc::as_ptr(runtime_surfaces.delivered_gate_routes_for_test()) as *const ();
    let delivery_ptr = Arc::as_ptr(runtime_surfaces.triggered_run_delivery_for_test()) as *const ();

    assert!(
        std::ptr::addr_eq(pref_ptr, state_ptr),
        "outbound_preferences and outbound_state must share one allocation"
    );
    assert!(
        std::ptr::addr_eq(pref_ptr, gate_ptr),
        "outbound_preferences and delivered_gate_routes must share one allocation"
    );
    assert!(
        std::ptr::addr_eq(pref_ptr, delivery_ptr),
        "outbound_preferences and triggered_run_delivery must share one allocation"
    );
}

fn slack_identity(
    manifest_path: &str,
    digest: Option<String>,
) -> ironclaw_host_api::PackageIdentity {
    ironclaw_host_api::PackageIdentity::new(
        ironclaw_host_api::PackageId::new("slack").expect("slack package id"),
        ironclaw_host_api::PackageSource::LocalManifest {
            path: manifest_path.to_string(),
        },
        digest,
        None,
    )
}

#[test]
fn builtin_first_party_trust_policy_includes_slack_local_manifest_entry() {
    // slack migrated to the self-contained inventory; its first-party trust
    // entry is now produced by the generic `bundled_packages()` loop. This
    // pin locks that the migration preserved slack's first-party grant and
    // its manifest-digest binding (wrong digest / wrong path → Sandbox).
    let policy = builtin_first_party_trust_policy().expect("trust policy");
    let slack_bundle = ironclaw_first_party_extensions::packages::bundled_packages()
        .into_iter()
        .find(|bundle| bundle.id == "slack")
        .expect("slack is in the bundled inventory");
    let expected_digest =
        ironclaw_host_api::sha256_digest_token(slack_bundle.manifest_toml.as_bytes());

    let matching = ironclaw_trust::TrustPolicy::evaluate(
        &policy,
        &ironclaw_trust::TrustPolicyInput {
            identity: slack_identity(
                "/system/extensions/slack/manifest.toml",
                Some(expected_digest.clone()),
            ),
            requested_trust: ironclaw_host_api::RequestedTrustClass::FirstPartyRequested,
            requested_authority: Default::default(),
        },
    )
    .expect("matching slack identity should evaluate");

    assert_eq!(matching.effective_trust.class(), TrustClass::FirstParty);
    assert_eq!(
        matching.provenance,
        ironclaw_trust::TrustProvenance::AdminConfig
    );

    let wrong_digest = ironclaw_trust::TrustPolicy::evaluate(
        &policy,
        &ironclaw_trust::TrustPolicyInput {
            identity: slack_identity(
                "/system/extensions/slack/manifest.toml",
                Some(
                    "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                        .to_string(),
                ),
            ),
            requested_trust: ironclaw_host_api::RequestedTrustClass::FirstPartyRequested,
            requested_authority: Default::default(),
        },
    )
    .expect("wrong digest slack identity should evaluate");

    assert_eq!(wrong_digest.effective_trust.class(), TrustClass::Sandbox);
    assert_eq!(
        wrong_digest.provenance,
        ironclaw_trust::TrustProvenance::Default
    );

    let wrong_path = ironclaw_trust::TrustPolicy::evaluate(
        &policy,
        &ironclaw_trust::TrustPolicyInput {
            identity: slack_identity(
                "/system/extensions/slack/other-manifest.toml",
                Some(expected_digest),
            ),
            requested_trust: ironclaw_host_api::RequestedTrustClass::FirstPartyRequested,
            requested_authority: Default::default(),
        },
    )
    .expect("wrong path slack identity should evaluate");

    assert_eq!(wrong_path.effective_trust.class(), TrustClass::Sandbox);
    assert_eq!(
        wrong_path.provenance,
        ironclaw_trust::TrustProvenance::Default
    );
}

#[test]
fn builtin_first_party_trust_policy_grants_migrated_gmail_via_inventory() {
    // gmail migrated to the self-contained inventory; its first-party trust
    // entry is now produced by the generic `bundled_packages()` loop, not a
    // hardcoded `AdminEntry`. Lock that the migration preserved gmail's
    // first-party grant AND its manifest-digest binding (a wrong digest must
    // still fall back to Sandbox — the loop didn't drop the digest).
    let policy = builtin_first_party_trust_policy().expect("trust policy");
    let gmail_bundle = ironclaw_first_party_extensions::packages::bundled_packages()
        .into_iter()
        .find(|bundle| bundle.id == "gmail")
        .expect("gmail is in the bundled inventory");
    let expected_digest =
        ironclaw_host_api::sha256_digest_token(gmail_bundle.manifest_toml.as_bytes());

    let gmail_identity = |digest: Option<String>| {
        ironclaw_host_api::PackageIdentity::new(
            ironclaw_host_api::PackageId::new("gmail").expect("gmail package id"),
            ironclaw_host_api::PackageSource::LocalManifest {
                path: "/system/extensions/gmail/manifest.toml".to_string(),
            },
            digest,
            None,
        )
    };

    let matching = ironclaw_trust::TrustPolicy::evaluate(
        &policy,
        &ironclaw_trust::TrustPolicyInput {
            identity: gmail_identity(Some(expected_digest.clone())),
            requested_trust: ironclaw_host_api::RequestedTrustClass::FirstPartyRequested,
            requested_authority: Default::default(),
        },
    )
    .expect("matching gmail identity should evaluate");
    assert_eq!(matching.effective_trust.class(), TrustClass::FirstParty);
    assert_eq!(
        matching.provenance,
        ironclaw_trust::TrustProvenance::AdminConfig
    );

    let wrong_digest = ironclaw_trust::TrustPolicy::evaluate(
        &policy,
        &ironclaw_trust::TrustPolicyInput {
            identity: gmail_identity(Some(
                "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                    .to_string(),
            )),
            requested_trust: ironclaw_host_api::RequestedTrustClass::FirstPartyRequested,
            requested_authority: Default::default(),
        },
    )
    .expect("wrong digest gmail identity should evaluate");
    assert_eq!(wrong_digest.effective_trust.class(), TrustClass::Sandbox);
}

/// Regression (#6520 merge reconciliation): the production factory composes
/// `lifecycle_auth_continuation_dispatcher` over the base product-auth
/// dispatcher, so a completed extension-card OAuth (a `LifecycleActivation`
/// continuation) re-enters the canonical lifecycle install/readiness command
/// instead of being durably fenced un-activated. Pre-fix the base dispatcher
/// answered `Ok` ("deferred to follow-up handler"), the fence stamped, and the
/// extension could never activate.
#[tokio::test]
async fn completed_lifecycle_activation_continuation_installs_the_extension() {
    use ironclaw_auth::{
        AuthChallenge, AuthContinuationRef, AuthFlowKind, AuthProductScope, AuthProviderId,
        AuthSurface, AuthorizationCodeHash, CredentialAccountLabel, NewAuthFlow,
        OAuthAuthorizationUrl, OAuthCallbackClaimRequest, OAuthCallbackInput,
        OAuthProviderExchange, OpaqueStateHash, PkceVerifierHash, ProviderCallbackOutcome,
        ProviderScope,
    };
    use ironclaw_host_api::SecretHandle;

    fn fake_digest(value: &str) -> String {
        format!(
            "{:064x}",
            value.bytes().fold(0_u64, |hash, byte| {
                hash.wrapping_mul(31).wrapping_add(u64::from(byte))
            })
        )
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let owner = "lifecycle-continuation-owner";
    let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
        owner,
        dir.path().join("local-dev"),
    ))
    .await
    .expect("local-dev services build");
    let runtime_surfaces = services.local_runtime_for_test().expect("local runtime");
    let product_auth = Arc::clone(&services.product_auth);
    let user = UserId::new(owner).expect("owner user id");
    let scope = AuthProductScope::new(
        ironclaw_host_api::ResourceScope::local_default(
            user.clone(),
            ironclaw_host_api::InvocationId::new(),
        )
        .expect("owner scope"),
        AuthSurface::Api,
    );
    let provider = AuthProviderId::new("github").expect("provider id");
    // The auth-flow continuation carries the string-shaped auth package ref;
    // the lifecycle wrapper converts it to the workflow ref internally.
    let package_ref =
        ironclaw_auth::LifecyclePackageRef::new("github").expect("github package ref");
    let expires_at = chrono::Utc::now() + chrono::Duration::minutes(5);
    let state_hash = OpaqueStateHash::new(fake_digest("lifecycle-state")).unwrap();
    let pkce_hash = PkceVerifierHash::new(fake_digest("lifecycle-pkce")).unwrap();

    let flow = product_auth
        .flow_manager()
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider.clone(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new("https://provider.example/oauth")
                    .unwrap(),
                expires_at,
            },
            continuation: AuthContinuationRef::LifecycleActivation {
                package_ref: package_ref.clone(),
            },
            update_binding: None,
            opaque_state_hash: Some(state_hash.clone()),
            pkce_verifier_hash: Some(pkce_hash.clone()),
            expires_at,
        })
        .await
        .expect("create lifecycle-activation flow");
    product_auth
        .flow_manager()
        .claim_oauth_callback(
            &scope,
            OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash.clone(),
                provider: provider.clone(),
                pkce_verifier_hash: pkce_hash.clone(),
            },
        )
        .await
        .expect("claim callback");
    product_auth
        .flow_manager()
        .complete_oauth_callback(
            &scope,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash,
                outcome: ProviderCallbackOutcome::Authorized {
                    exchange: Box::new(OAuthProviderExchange {
                        provider: provider.clone(),
                        account_label: CredentialAccountLabel::new("GitHub Account").unwrap(),
                        authorization_code_hash: AuthorizationCodeHash::new(fake_digest(
                            "lifecycle-code",
                        ))
                        .unwrap(),
                        pkce_verifier_hash: pkce_hash,
                        access_secret: SecretHandle::new("lifecycle-github-access").unwrap(),
                        refresh_secret: None,
                        scopes: vec![ProviderScope::new("repo.readonly").unwrap()],
                        account_id: None,
                        provider_identity: None,
                    }),
                },
            },
        )
        .await
        .expect("complete callback");

    // Reconciling the completed-but-unfenced flow drives the composed
    // dispatcher: the lifecycle wrapper re-enters the canonical install
    // command, the just-minted github credential account satisfies the
    // credential gate, and install auto-advances the extension to Active
    // before the fan-out settles the flow. Pre-fix the base dispatcher
    // answered `Ok` without installing anything.
    let status = product_auth
        .reconcile_oauth_flow(&scope, flow.id)
        .await
        .expect("lifecycle continuation reconciles");
    assert_eq!(status, ironclaw_auth::AuthFlowStatus::Completed);

    let installation = runtime_surfaces
        .extension_management
        .installation_store_for_test()
        .list_installations()
        .await
        .expect("list installations")
        .into_iter()
        .find(|installation| installation.extension_id().as_str() == "github")
        .expect("lifecycle continuation must install the github extension");
    assert!(
        installation.owner().visible_to(&user),
        "the continuation's caller must hold the installation membership"
    );
    // Install drove readiness all the way to runtime publication: the github
    // tool surface is model-visible without any separate Activate action.
    let capabilities = runtime_surfaces
        .extension_management
        .active_model_visible_capabilities()
        .await
        .expect("active capabilities");
    assert!(
        capabilities
            .iter()
            .any(|capability| capability.provider.as_str() == "github"),
        "github capabilities must be published after the continuation"
    );

    // A fanned-out continuation stamps the durable fence exactly once.
    let record = product_auth
        .flow_manager()
        .get_flow(&scope, flow.id)
        .await
        .expect("get flow")
        .expect("flow record exists");
    assert!(
        record.continuation_emitted_at.is_some(),
        "a fanned-out continuation must stamp the durable fence"
    );
}

/// #6520 live-repro regression: a completed channel pairing must run the SAME
/// lifecycle-wrapped continuation dispatcher product-auth uses — readiness
/// reconciliation (runtime publication) before the blocked-run fan-out. When
/// composition handed pairing a bare turn-resume dispatcher instead, a
/// freshly paired channel extension (telegram: remove → install → pair) sat
/// at setup_needed forever because nothing re-published it. Pinned by pointer
/// identity at the composition seam: every pairing service's dispatcher IS
/// product-auth's composed dispatcher.
#[tokio::test]
async fn channel_pairing_completions_run_the_lifecycle_wrapped_continuation_dispatcher() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
        "local-dev-pairing-continuation-owner",
        dir.path().join("local-dev"),
    ))
    .await
    .expect("local-dev services build");

    let product_auth_dispatcher = services.product_auth.continuation_dispatcher_for_test();
    let channel_pairing = services
        .channel_pairing
        .as_ref()
        .expect("local-dev build composes the channel pairing registry");
    let mut pairing_services_checked = 0usize;
    for extension_id in ["telegram", "slack"] {
        let Some(pairing) = channel_pairing.get(extension_id) else {
            continue;
        };
        pairing_services_checked += 1;
        assert!(
            Arc::ptr_eq(
                &pairing.continuation_dispatcher_for_test(),
                &product_auth_dispatcher,
            ),
            "{extension_id} pairing completions must dispatch through product-auth's \
             lifecycle-wrapped continuation dispatcher, not a bare turn-resume one",
        );
    }
    assert!(
        pairing_services_checked > 0,
        "expected at least one bundled channel extension with a pairing service",
    );
}

/// Live-repro regression (demo-stack defect): removing an installed channel
/// extension through the lifecycle port with an authenticated actor must
/// actually delete the caller's durable membership — and must be POSSIBLE in
/// every composition that can install one (the channel-connection disconnect
/// slot is filled at factory tier, not only in `build_reborn_runtime`).
#[tokio::test]
async fn telegram_remove_with_authenticated_actor_deletes_the_membership() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
        "local-dev-telegram-remove-owner",
        dir.path().join("local-dev"),
    ))
    .await
    .expect("local-dev services build");
    let runtime_surfaces = services.local_runtime_for_test().expect("local runtime");
    let extension_management = &runtime_surfaces.extension_management;
    let caller = UserId::new("telegram-remove-user").expect("user id");
    let telegram_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "telegram").expect("valid ref");

    extension_management
        .install(telegram_ref.clone(), &caller)
        .await
        .expect("install telegram");

    let removal_scope =
        default_runtime_owner_scope(caller.clone()).expect("telegram removal scope");
    let removed = extension_management
        .remove(telegram_ref.clone(), &removal_scope, Some(&caller))
        .await
        .expect("remove telegram");
    assert!(
        matches!(
            removed.payload.as_ref(),
            Some(ironclaw_product::LifecycleProductPayload::ExtensionRemove { removed: true })
        ),
        "remove must report the membership it deleted, got {:?}",
        removed.payload
    );

    let projection = extension_management
        .project(telegram_ref, &caller, None)
        .await
        .expect("project telegram after remove");
    assert_eq!(
        projection.phase,
        ironclaw_product::LifecyclePublicState::Uninstalled,
        "removed telegram must project uninstalled for its former member",
    );
}
