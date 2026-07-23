use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_auth::{AuthContinuationEvent, AuthProductError};
use ironclaw_conversations::{
    AdapterKind, ConditionalUnpairOutcome, ConversationActorPairingService,
    ExpectedExternalActorOwner, ExternalActorBindingEpoch, ExternalActorRef, InboundTurnError,
};
use ironclaw_filesystem::{InMemoryBackend, RootFilesystem};
use ironclaw_host_api::{
    AgentId, ExtensionId, RuntimeCredentialAccountSetup, RuntimeCredentialAuthRequirement,
    TenantId, UserId, VendorId,
};
use ironclaw_product::{
    AdapterInstallationId, ExternalActorRef as ProductExternalActorRef, ProductAdapterId,
};
use ironclaw_product::{
    ChannelConnectionNoticePolicy, ChannelConnectionRequirement, ChannelPairingDirectTargetStore,
    ChannelPairingIdentityBindOutcome, ChannelPairingIdentityStore,
    ChannelPairingInstallationSource, ChannelPairingService, ChannelPairingServiceDependencies,
    ChannelPairingTemplateValues, ExtensionAccountSetupDescriptor, FilesystemChannelPairingStore,
    ProductActorUserResolutionRequest, ProductActorUserResolver, ProductAuthContinuationDispatcher,
    RebornChannelConnectStrategy,
};
use tokio::sync::Notify;

const EXTENSION_ID: &str = "vendorx";
const INSTALLATION_ID: &str = "install-1";
const EXTERNAL_ACTOR_ID: &str = "u-1";

struct StaticInstallation;

#[async_trait]
impl ChannelPairingInstallationSource for StaticInstallation {
    async fn current_installation(
        &self,
        _caller: &UserId,
    ) -> Result<Option<AdapterInstallationId>, String> {
        Ok(Some(installation_id()))
    }
}

struct EmptyTemplateValues;

#[async_trait]
impl ChannelPairingTemplateValues for EmptyTemplateValues {
    async fn template_value(&self, _handle: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
}

#[derive(Default)]
struct InMemoryIdentity {
    bindings: Mutex<BTreeMap<(String, String), UserId>>,
}

#[async_trait]
impl ChannelPairingIdentityStore for InMemoryIdentity {
    fn binding_key(
        &self,
        installation_id: &AdapterInstallationId,
        external_actor_id: &str,
    ) -> String {
        format!("{}:{external_actor_id}", installation_id.as_str())
    }

    async fn resolve_user(
        &self,
        _extension_id: &ExtensionId,
        installation_id: &AdapterInstallationId,
        external_actor_id: &str,
    ) -> Result<Option<UserId>, String> {
        Ok(self
            .bindings
            .lock()
            .expect("bindings lock")
            .get(&(
                installation_id.as_str().to_string(),
                external_actor_id.to_string(),
            ))
            .cloned())
    }

    async fn bind_user(
        &self,
        _extension_id: &ExtensionId,
        installation_id: &AdapterInstallationId,
        external_actor_id: &str,
        user_id: UserId,
    ) -> Result<ChannelPairingIdentityBindOutcome, String> {
        let key = (
            installation_id.as_str().to_string(),
            external_actor_id.to_string(),
        );
        let mut bindings = self.bindings.lock().expect("bindings lock");
        match bindings.get(&key) {
            Some(existing) if existing != &user_id => {
                Ok(ChannelPairingIdentityBindOutcome::AlreadyBoundToOtherUser)
            }
            _ => {
                bindings.insert(key, user_id);
                Ok(ChannelPairingIdentityBindOutcome::Bound)
            }
        }
    }

    async fn delete_user_bindings(
        &self,
        _extension_id: &ExtensionId,
        user_id: &UserId,
    ) -> Result<(), String> {
        self.bindings
            .lock()
            .expect("bindings lock")
            .retain(|_, bound| bound != user_id);
        Ok(())
    }
}

#[derive(Default)]
struct InMemoryDirectTargets {
    targets: Mutex<BTreeMap<UserId, String>>,
    fail_next_upsert: AtomicBool,
}

#[async_trait]
impl ChannelPairingDirectTargetStore for InMemoryDirectTargets {
    async fn is_connected(
        &self,
        _extension_id: &ExtensionId,
        user_id: &UserId,
    ) -> Result<bool, String> {
        Ok(self
            .targets
            .lock()
            .expect("targets lock")
            .contains_key(user_id))
    }

    async fn upsert(
        &self,
        _extension_id: &ExtensionId,
        user_id: &UserId,
        _external_actor_id: &str,
        _conversation_space_id: Option<&str>,
        conversation_id: &str,
    ) -> Result<(), String> {
        if self.fail_next_upsert.swap(false, Ordering::SeqCst) {
            return Err("injected target persistence failure".to_string());
        }
        self.targets
            .lock()
            .expect("targets lock")
            .insert(user_id.clone(), conversation_id.to_string());
        Ok(())
    }

    async fn delete(&self, _extension_id: &ExtensionId, user_id: &UserId) -> Result<(), String> {
        self.targets.lock().expect("targets lock").remove(user_id);
        Ok(())
    }
}

struct AcceptingContinuation;

#[async_trait]
impl ProductAuthContinuationDispatcher for AcceptingContinuation {
    async fn dispatch_auth_continuation(
        &self,
        _event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }

    async fn dispatch_canceled_auth_continuation(
        &self,
        _event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }
}

#[derive(Default)]
struct RecordingContinuation {
    events: Mutex<Vec<AuthContinuationEvent>>,
}

#[async_trait]
impl ProductAuthContinuationDispatcher for RecordingContinuation {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        self.events.lock().expect("events lock").push(event);
        Ok(())
    }

    async fn dispatch_canceled_auth_continuation(
        &self,
        _event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }
}

#[derive(Default)]
struct BlockingContinuation {
    attempts: AtomicUsize,
    started: Notify,
    release: Notify,
}

impl BlockingContinuation {
    async fn wait_until_started(&self) {
        while self.attempts.load(Ordering::SeqCst) == 0 {
            self.started.notified().await;
        }
    }
}

#[async_trait]
impl ProductAuthContinuationDispatcher for BlockingContinuation {
    async fn dispatch_auth_continuation(
        &self,
        _event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        self.started.notify_waiters();
        self.release.notified().await;
        Ok(())
    }

    async fn dispatch_canceled_auth_continuation(
        &self,
        _event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }
}

#[derive(Default)]
struct FaultInjectedActorPairings {
    #[allow(clippy::type_complexity)]
    owners: Mutex<BTreeMap<(String, String), (UserId, Option<ExternalActorBindingEpoch>)>>,
    unpair_calls: Mutex<Vec<(String, String)>>,
    fail_next_unpair: AtomicBool,
    block_next_unpair: AtomicBool,
    unpair_is_blocked: AtomicBool,
    unpair_started: Notify,
    unpair_release: Notify,
}

impl FaultInjectedActorPairings {
    fn owner(&self) -> Option<UserId> {
        self.owners
            .lock()
            .expect("owners lock")
            .get(&(INSTALLATION_ID.to_string(), EXTERNAL_ACTOR_ID.to_string()))
            .map(|(owner, _)| owner.clone())
    }

    fn owner_epoch(&self) -> Option<ExternalActorBindingEpoch> {
        self.owners
            .lock()
            .expect("owners lock")
            .get(&(INSTALLATION_ID.to_string(), EXTERNAL_ACTOR_ID.to_string()))
            .and_then(|(_, epoch)| epoch.clone())
    }

    fn unpair_call_count(&self) -> usize {
        self.unpair_calls.lock().expect("unpair calls lock").len()
    }

    async fn wait_until_unpair_started(&self) {
        while !self.unpair_is_blocked.load(Ordering::SeqCst) {
            self.unpair_started.notified().await;
        }
    }
}

#[async_trait]
impl ConversationActorPairingService for FaultInjectedActorPairings {
    async fn pair_external_actor(
        &self,
        _tenant_id: TenantId,
        _adapter_kind: AdapterKind,
        adapter_installation_id: ironclaw_conversations::AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
        user_id: UserId,
    ) -> Result<(), InboundTurnError> {
        self.owners.lock().expect("owners lock").insert(
            (
                adapter_installation_id.as_str().to_string(),
                external_actor_ref.id().to_string(),
            ),
            (user_id, None),
        );
        Ok(())
    }

    async fn pair_external_actor_with_epoch(
        &self,
        tenant_id: TenantId,
        adapter_kind: AdapterKind,
        adapter_installation_id: ironclaw_conversations::AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
        user_id: UserId,
        epoch: ironclaw_conversations::ExternalActorBindingEpoch,
    ) -> Result<(), InboundTurnError> {
        let _ = (tenant_id, adapter_kind);
        self.owners.lock().expect("owners lock").insert(
            (
                adapter_installation_id.as_str().to_string(),
                external_actor_ref.id().to_string(),
            ),
            (user_id, Some(epoch)),
        );
        Ok(())
    }

    async fn unpair_external_actor(
        &self,
        _tenant_id: TenantId,
        _adapter_kind: AdapterKind,
        adapter_installation_id: ironclaw_conversations::AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
    ) -> Result<(), InboundTurnError> {
        self.owners.lock().expect("owners lock").remove(&(
            adapter_installation_id.as_str().to_string(),
            external_actor_ref.id().to_string(),
        ));
        Ok(())
    }

    async fn unpair_external_actor_if_owned_by(
        &self,
        _tenant_id: &TenantId,
        _adapter_kind: &AdapterKind,
        adapter_installation_id: &ironclaw_conversations::AdapterInstallationId,
        external_actor_ref: &ExternalActorRef,
        expected: &ExpectedExternalActorOwner,
    ) -> Result<ConditionalUnpairOutcome, InboundTurnError> {
        let key = (
            adapter_installation_id.as_str().to_string(),
            external_actor_ref.id().to_string(),
        );
        self.unpair_calls
            .lock()
            .expect("unpair calls lock")
            .push(key.clone());
        if self.fail_next_unpair.swap(false, Ordering::SeqCst) {
            return Err(InboundTurnError::DurableState {
                reason: "injected actor cleanup failure".to_string(),
            });
        }
        if self.block_next_unpair.swap(false, Ordering::SeqCst) {
            self.unpair_is_blocked.store(true, Ordering::SeqCst);
            self.unpair_started.notify_waiters();
            self.unpair_release.notified().await;
            self.unpair_is_blocked.store(false, Ordering::SeqCst);
        }
        let mut owners = self.owners.lock().expect("owners lock");
        match owners.get(&key) {
            Some((owner, epoch))
                if owner == &expected.user_id
                    && epoch.as_ref() == expected.binding_epoch.as_ref() =>
            {
                owners.remove(&key);
                Ok(ConditionalUnpairOutcome::Unpaired)
            }
            Some(_) => Ok(ConditionalUnpairOutcome::OwnerChanged),
            None => Ok(ConditionalUnpairOutcome::AlreadyAbsent),
        }
    }
}

fn installation_id() -> AdapterInstallationId {
    AdapterInstallationId::new(INSTALLATION_ID).expect("installation id")
}

fn user_id() -> UserId {
    UserId::new("alice").expect("user id")
}

fn build_service(
    filesystem: Arc<dyn RootFilesystem>,
    identity: Arc<InMemoryIdentity>,
    actor_pairings: Arc<FaultInjectedActorPairings>,
    direct_targets: Arc<InMemoryDirectTargets>,
) -> ChannelPairingService {
    build_service_with_continuation(
        filesystem,
        identity,
        actor_pairings,
        direct_targets,
        Arc::new(AcceptingContinuation),
    )
}

fn build_service_with_continuation(
    filesystem: Arc<dyn RootFilesystem>,
    identity: Arc<InMemoryIdentity>,
    actor_pairings: Arc<FaultInjectedActorPairings>,
    direct_targets: Arc<InMemoryDirectTargets>,
    continuation: Arc<dyn ProductAuthContinuationDispatcher>,
) -> ChannelPairingService {
    let tenant_id = TenantId::new("tenant-alpha").expect("tenant id");
    let extension_id = ExtensionId::new(EXTENSION_ID).expect("extension id");
    let descriptor = ExtensionAccountSetupDescriptor {
        extension_id: extension_id.clone(),
        auth_requirement: RuntimeCredentialAuthRequirement {
            provider: VendorId::new(EXTENSION_ID).expect("vendor id"),
            setup: RuntimeCredentialAccountSetup::Pairing,
            requester_extension: extension_id.clone(),
            provider_scopes: Vec::new(),
        },
        connection_notices: ChannelConnectionNoticePolicy::generic("Vendor X"),
        connection_requirement: ChannelConnectionRequirement {
            channel: EXTENSION_ID.to_string(),
            display_name: "Vendor X".to_string(),
            strategy: RebornChannelConnectStrategy::WebGeneratedCode,
            instructions: "Send the generated code.".to_string(),
            input_placeholder: String::new(),
            submit_label: "Connect".to_string(),
            error_message: "Invalid or expired pairing code.".to_string(),
        },
        connection_success_message: "Connected.".to_string(),
        pairing_deep_link_template: None,
        pairing_inbound_code_prefixes: Vec::new(),
    };
    ChannelPairingService::new(
        tenant_id.clone(),
        AgentId::new("agent-a").expect("agent id"),
        None,
        descriptor,
        ChannelPairingServiceDependencies {
            store: Arc::new(FilesystemChannelPairingStore::new(
                filesystem,
                tenant_id,
                UserId::new("operator").expect("operator id"),
                extension_id,
            )),
            installation: Arc::new(StaticInstallation),
            template_values: Arc::new(EmptyTemplateValues),
            identity,
            continuation,
            conversation_actor_pairings: actor_pairings,
            direct_targets,
        },
    )
}

#[tokio::test]
async fn unpair_cleanup_failure_survives_reopen_and_retry_fences_repair() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let identity = Arc::new(InMemoryIdentity::default());
    let actor_pairings = Arc::new(FaultInjectedActorPairings::default());
    let direct_targets = Arc::new(InMemoryDirectTargets::default());
    let service = build_service(
        Arc::clone(&filesystem),
        Arc::clone(&identity),
        Arc::clone(&actor_pairings),
        Arc::clone(&direct_targets),
    );
    let issue = service
        .issue_or_rotate(&user_id())
        .await
        .expect("mint pairing code");
    service
        .consume(
            &installation_id(),
            issue.code.as_str(),
            "vendor_user",
            EXTERNAL_ACTOR_ID,
            None,
            "chat-1",
        )
        .await
        .expect("consume pairing code");
    actor_pairings
        .fail_next_unpair
        .store(true, Ordering::SeqCst);

    service
        .unpair(&user_id())
        .await
        .expect_err("injected conversation cleanup failure must surface");
    assert_eq!(actor_pairings.owner(), Some(user_id()));
    assert_eq!(
        identity
            .resolve_user(
                &ExtensionId::new(EXTENSION_ID).expect("extension id"),
                &installation_id(),
                EXTERNAL_ACTOR_ID,
            )
            .await
            .expect("identity remains readable"),
        Some(user_id()),
        "proof-code identity is the connected signal and must survive every earlier cleanup failure"
    );

    let reopened = build_service(
        Arc::clone(&filesystem),
        Arc::clone(&identity),
        Arc::clone(&actor_pairings),
        Arc::clone(&direct_targets),
    );
    reopened
        .unpair(&user_id())
        .await
        .expect("retry drains durable cleanup intent");
    assert_eq!(actor_pairings.owner(), None);
    assert_eq!(actor_pairings.unpair_call_count(), 2);
    assert_eq!(
        identity
            .resolve_user(
                &ExtensionId::new(EXTENSION_ID).expect("extension id"),
                &installation_id(),
                EXTERNAL_ACTOR_ID,
            )
            .await
            .expect("identity remains readable"),
        None,
        "identity deletion is the successful retry's final commit point"
    );

    let repair = reopened
        .issue_or_rotate(&user_id())
        .await
        .expect("mint replacement pairing code");
    reopened
        .consume(
            &installation_id(),
            repair.code.as_str(),
            "vendor_user",
            EXTERNAL_ACTOR_ID,
            None,
            "chat-2",
        )
        .await
        .expect("consume replacement pairing code");

    reopened
        .issue_or_rotate(&user_id())
        .await
        .expect("new issue sees no stale cleanup intent");
    assert_eq!(actor_pairings.owner(), Some(user_id()));
    assert_eq!(
        actor_pairings.unpair_call_count(),
        2,
        "settled cleanup intent is removed before a replacement pairing"
    );
}

#[tokio::test]
async fn stale_unpair_worker_cannot_delete_a_newer_same_user_repair() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let identity = Arc::new(InMemoryIdentity::default());
    let actor_pairings = Arc::new(FaultInjectedActorPairings::default());
    let direct_targets = Arc::new(InMemoryDirectTargets::default());
    let first = Arc::new(build_service(
        Arc::clone(&filesystem),
        Arc::clone(&identity),
        Arc::clone(&actor_pairings),
        Arc::clone(&direct_targets),
    ));
    let second = build_service(
        Arc::clone(&filesystem),
        Arc::clone(&identity),
        Arc::clone(&actor_pairings),
        Arc::clone(&direct_targets),
    );
    let issue = first
        .issue_or_rotate(&user_id())
        .await
        .expect("mint first pairing code");
    first
        .consume(
            &installation_id(),
            issue.code.as_str(),
            "vendor_user",
            EXTERNAL_ACTOR_ID,
            None,
            "chat-before-unpair",
        )
        .await
        .expect("consume first pairing code");
    let first_epoch = actor_pairings
        .owner_epoch()
        .expect("first pairing has a durable epoch");

    actor_pairings
        .block_next_unpair
        .store(true, Ordering::SeqCst);
    let stale_cleanup = tokio::spawn({
        let first = Arc::clone(&first);
        async move { first.unpair(&user_id()).await }
    });
    actor_pairings.wait_until_unpair_started().await;

    second
        .unpair(&user_id())
        .await
        .expect("second instance settles staged cleanup");
    assert_eq!(actor_pairings.owner(), None);
    let repair = second
        .issue_or_rotate(&user_id())
        .await
        .expect("mint repair pairing code");
    second
        .consume(
            &installation_id(),
            repair.code.as_str(),
            "vendor_user",
            EXTERNAL_ACTOR_ID,
            None,
            "chat-after-repair",
        )
        .await
        .expect("repair actor pairing");
    let repair_epoch = actor_pairings
        .owner_epoch()
        .expect("repair pairing has a durable epoch");
    assert_ne!(
        repair_epoch, first_epoch,
        "repair must create a new actor-binding generation"
    );
    let resolved = second
        .resolve_product_actor_user(ProductActorUserResolutionRequest::new(
            ProductAdapterId::new(EXTENSION_ID).expect("adapter id"),
            installation_id(),
            ProductExternalActorRef::new("vendor_user", EXTERNAL_ACTOR_ID, None::<&str>)
                .expect("product actor ref"),
        ))
        .await
        .expect("resolve paired actor")
        .expect("paired actor resolves");
    assert_eq!(
        resolved.binding_epoch,
        Some(repair_epoch.clone()),
        "ordinary inbound resolution must preserve the exact pairing generation"
    );

    actor_pairings.unpair_release.notify_waiters();
    stale_cleanup
        .await
        .expect("stale cleanup task")
        .expect("stale cleanup converges as owner-changed");
    assert_eq!(
        actor_pairings.owner(),
        Some(user_id()),
        "stale exact-owner cleanup must preserve the newer same-user repair"
    );
    assert_eq!(actor_pairings.owner_epoch(), Some(repair_epoch));
}

#[tokio::test]
async fn committed_completion_recovers_target_and_dispatch_after_service_reopen() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let identity = Arc::new(InMemoryIdentity::default());
    let actor_pairings = Arc::new(FaultInjectedActorPairings::default());
    let direct_targets = Arc::new(InMemoryDirectTargets::default());
    let service = build_service(
        Arc::clone(&filesystem),
        Arc::clone(&identity),
        Arc::clone(&actor_pairings),
        Arc::clone(&direct_targets),
    );
    let issue = service
        .issue_or_rotate(&user_id())
        .await
        .expect("mint pairing code");
    direct_targets
        .fail_next_upsert
        .store(true, Ordering::SeqCst);
    service
        .consume(
            &installation_id(),
            issue.code.as_str(),
            "vendor_user",
            EXTERNAL_ACTOR_ID,
            None,
            "chat-after-restart",
        )
        .await
        .expect_err("fault after outbox commit simulates process loss");
    assert_eq!(
        service
            .pending_completion_dispatch_ids_for_test()
            .await
            .expect("pending completion ids")
            .len(),
        1,
        "completion intent must commit before target persistence"
    );
    assert!(
        !direct_targets
            .is_connected(
                &ExtensionId::new(EXTENSION_ID).expect("extension id"),
                &user_id(),
            )
            .await
            .expect("target lookup")
    );

    let continuation = Arc::new(RecordingContinuation::default());
    let reopened = build_service_with_continuation(
        Arc::clone(&filesystem),
        Arc::clone(&identity),
        Arc::clone(&actor_pairings),
        Arc::clone(&direct_targets),
        Arc::clone(&continuation) as Arc<dyn ProductAuthContinuationDispatcher>,
    );
    reopened
        .reconcile_pending_completions()
        .await
        .expect("reopened service reconciles committed completion");

    assert!(
        direct_targets
            .is_connected(
                &ExtensionId::new(EXTENSION_ID).expect("extension id"),
                &user_id(),
            )
            .await
            .expect("target lookup"),
        "restart reconciliation must replay target persistence before settlement"
    );
    assert_eq!(continuation.events.lock().expect("events lock").len(), 1);
    assert!(
        reopened
            .pending_completion_dispatch_ids_for_test()
            .await
            .expect("pending completion ids")
            .is_empty()
    );
}

#[tokio::test]
async fn two_service_instances_contend_for_one_completion_owner() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let identity = Arc::new(InMemoryIdentity::default());
    let actor_pairings = Arc::new(FaultInjectedActorPairings::default());
    let direct_targets = Arc::new(InMemoryDirectTargets::default());
    let seed = build_service(
        Arc::clone(&filesystem),
        Arc::clone(&identity),
        Arc::clone(&actor_pairings),
        Arc::clone(&direct_targets),
    );
    let issue = seed
        .issue_or_rotate(&user_id())
        .await
        .expect("mint pairing code");
    seed.consume(
        &installation_id(),
        issue.code.as_str(),
        "vendor_user",
        EXTERNAL_ACTOR_ID,
        None,
        "chat-contended",
    )
    .await
    .expect("commit completion intent");

    let continuation = Arc::new(BlockingContinuation::default());
    let mut first_service = build_service_with_continuation(
        Arc::clone(&filesystem),
        Arc::clone(&identity),
        Arc::clone(&actor_pairings),
        Arc::clone(&direct_targets),
        Arc::clone(&continuation) as Arc<dyn ProductAuthContinuationDispatcher>,
    );
    first_service
        .set_completion_lease_timing_for_test(
            std::time::Duration::from_millis(120),
            std::time::Duration::from_millis(25),
        )
        .expect("short first-instance lease timing");
    let first = Arc::new(first_service);
    let mut second_service = build_service_with_continuation(
        Arc::clone(&filesystem),
        Arc::clone(&identity),
        Arc::clone(&actor_pairings),
        Arc::clone(&direct_targets),
        Arc::clone(&continuation) as Arc<dyn ProductAuthContinuationDispatcher>,
    );
    second_service
        .set_completion_lease_timing_for_test(
            std::time::Duration::from_millis(120),
            std::time::Duration::from_millis(25),
        )
        .expect("short second-instance lease timing");
    let second = Arc::new(second_service);
    let first_reconcile = tokio::spawn({
        let first = Arc::clone(&first);
        async move { first.reconcile_pending_completions().await }
    });
    continuation.wait_until_started().await;
    tokio::time::sleep(std::time::Duration::from_millis(220)).await;
    assert_eq!(
        second
            .live_completion_lease_count_for_test()
            .await
            .expect("inspect live completion lease"),
        1,
        "the blocked dispatch must have one durable outbox owner"
    );

    tokio::time::timeout(
        std::time::Duration::from_millis(100),
        second.reconcile_pending_completions(),
    )
    .await
    .expect("second instance must observe the live outbox lease and return")
    .expect("second reconciliation");
    assert_eq!(
        continuation.attempts.load(Ordering::SeqCst),
        1,
        "only one service instance may own and dispatch a live completion lease"
    );

    continuation.release.notify_waiters();
    first_reconcile
        .await
        .expect("first reconciliation task")
        .expect("first reconciliation");
    assert!(
        first
            .pending_completion_dispatch_ids_for_test()
            .await
            .expect("pending completion ids")
            .is_empty()
    );
}
