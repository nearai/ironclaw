// Unit tests for the generic pairing service; child module so
// `use super::*` reaches crate-private items.
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use ironclaw_auth::AuthProductError;
use ironclaw_conversations::{
    ConditionalUnpairOutcome, ExternalActorRef as ConversationActorRef, InboundTurnError,
};
use ironclaw_extension_host::ingress::{InboundAdmission, InboundAdmissionAck, InboundSink};
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_product::{
    ChannelConnectionNoticePolicy, ChannelPairingCode, ChannelPairingConsumeOutcome,
    ChannelPairingError, ChannelPairingInstallationSource, ChannelPairingInterception,
    ChannelPairingInterceptor, ChannelPairingService, ChannelPairingServiceDependencies,
    ChannelPairingTemplateValues, ExtensionAccountSetupDescriptor, FilesystemChannelPairingStore,
};
use ironclaw_product::{
    ExternalActorRef, ExternalConversationRef, ExternalEventId, NormalizedInboundMessage,
    ProductAdapterId, ProductTriggerReason,
};
use tokio::sync::Notify;

use super::*;
use crate::extension_host::channel_dm_targets::FilesystemChannelDmTargetStore;
use crate::extension_host::extension_ingress::{
    ChannelInboundSinkConfig, ChannelIngressDrain, ChannelPairingOutcomeObserver,
    GenericChannelInboundSink, VerifiedEvidenceMint,
};

const EXT: &str = "vendorx";
const INSTALL: &str = "install-1";

struct StaticInstallation(Option<AdapterInstallationId>);

#[async_trait]
impl ChannelPairingInstallationSource for StaticInstallation {
    async fn current_installation(
        &self,
        _caller: &UserId,
    ) -> Result<Option<AdapterInstallationId>, String> {
        Ok(self.0.clone())
    }
}

struct StaticTemplateValues(BTreeMap<String, String>);

#[async_trait]
impl ChannelPairingTemplateValues for StaticTemplateValues {
    async fn template_value(&self, handle: &str) -> Result<Option<String>, String> {
        Ok(self.0.get(handle).cloned())
    }
}

/// Real in-memory identity semantics: first bind wins, rebind to a
/// different user refuses, deletes return per-user removal.
#[derive(Default)]
struct InMemoryIdentity {
    bindings: Mutex<BTreeMap<(String, String), UserId>>,
}

#[async_trait]
impl RebornUserIdentityBindingStore for InMemoryIdentity {
    async fn bind_user_identity(
        &self,
        binding: RebornUserIdentityBinding,
    ) -> Result<(), RebornUserIdentityBindingError> {
        let mut bindings = self.bindings.lock().expect("bindings lock");
        let key = (
            binding.provider.as_str().to_string(),
            binding.provider_user_id.as_str().to_string(),
        );
        match bindings.get(&key) {
            Some(existing) if existing != &binding.user_id => {
                Err(RebornUserIdentityBindingError::ProviderIdentityAlreadyBound)
            }
            _ => {
                bindings.insert(key, binding.user_id);
                Ok(())
            }
        }
    }
}

#[async_trait]
impl crate::provider_identity::RebornUserIdentityLookup for InMemoryIdentity {
    async fn resolve_user_identity(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserId>, crate::provider_identity::RebornUserIdentityLookupError> {
        Ok(self
            .bindings
            .lock()
            .expect("bindings lock")
            .get(&(provider.to_string(), provider_user_id.to_string()))
            .cloned())
    }

    async fn user_has_provider_binding(
        &self,
        provider: &str,
        user_id: &UserId,
    ) -> Result<bool, crate::provider_identity::RebornUserIdentityLookupError> {
        Ok(self
            .bindings
            .lock()
            .expect("bindings lock")
            .iter()
            .any(|((stored, _), bound)| stored == provider && bound == user_id))
    }
}

#[async_trait]
impl RebornUserIdentityBindingDeleteStore for InMemoryIdentity {
    async fn delete_user_identity_bindings_for_user(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
    ) -> Result<usize, RebornUserIdentityBindingError> {
        let mut bindings = self.bindings.lock().expect("bindings lock");
        let before = bindings.len();
        bindings.retain(|(stored, provider_user_id_key), bound| {
            !(stored == provider
                && bound == user_id
                && provider_user_id_prefix
                    .is_none_or(|prefix| provider_user_id_key.starts_with(prefix)))
        });
        Ok(before - bindings.len())
    }
}

#[derive(Default)]
struct RecordingDispatcher {
    events: Mutex<Vec<AuthContinuationEvent>>,
}

#[async_trait]
impl RebornAuthContinuationDispatcher for RecordingDispatcher {
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
struct BlockingFanoutAcceptance {
    started: Notify,
    release: Notify,
    accepted: AtomicUsize,
}

#[async_trait]
impl RebornAuthContinuationDispatcher for BlockingFanoutAcceptance {
    async fn dispatch_auth_continuation(
        &self,
        _event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        self.started.notify_one();
        self.release.notified().await;
        self.accepted.fetch_add(1, Ordering::SeqCst);
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
struct FailOnceIdempotentFanout {
    attempts: Mutex<Vec<AuthFlowId>>,
    accepted: Mutex<HashSet<AuthFlowId>>,
    resumed: AtomicUsize,
}

#[async_trait]
impl RebornAuthContinuationDispatcher for FailOnceIdempotentFanout {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        let attempt = {
            let mut attempts = self.attempts.lock().expect("attempts lock");
            attempts.push(event.flow_id);
            attempts.len()
        };
        if attempt == 1 {
            return Err(AuthProductError::BackendUnavailable);
        }
        if self
            .accepted
            .lock()
            .expect("accepted lock")
            .insert(event.flow_id)
        {
            self.resumed.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }

    async fn dispatch_canceled_auth_continuation(
        &self,
        _event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }
}

struct UnexpectedWorkflow;

#[async_trait::async_trait]
impl ironclaw_host_api::ChannelInboundProductSurface for UnexpectedWorkflow {
    async fn admit_channel_inbound(
        &self,
        _request: ironclaw_host_api::ChannelInboundSurfaceRequest,
    ) -> ironclaw_host_api::ChannelInboundSurfaceOutcome {
        panic!("pairing tests must not reach channel admission");
    }
}

#[derive(Default)]
struct RecordingActorPairings {
    unpairs: Mutex<Vec<(String, String, Option<String>)>>,
}

#[async_trait]
impl ConversationActorPairingService for RecordingActorPairings {
    async fn pair_external_actor(
        &self,
        _tenant_id: TenantId,
        _adapter_kind: AdapterKind,
        _adapter_installation_id: ironclaw_conversations::AdapterInstallationId,
        _external_actor_ref: ConversationActorRef,
        _user_id: UserId,
    ) -> Result<(), InboundTurnError> {
        Ok(())
    }

    async fn pair_external_actor_with_epoch(
        &self,
        _tenant_id: TenantId,
        _adapter_kind: AdapterKind,
        _adapter_installation_id: ironclaw_conversations::AdapterInstallationId,
        _external_actor_ref: ConversationActorRef,
        _user_id: UserId,
        _epoch: ironclaw_conversations::ExternalActorBindingEpoch,
    ) -> Result<(), InboundTurnError> {
        Ok(())
    }

    async fn unpair_external_actor(
        &self,
        _tenant_id: TenantId,
        _adapter_kind: AdapterKind,
        _adapter_installation_id: ironclaw_conversations::AdapterInstallationId,
        _external_actor_ref: ConversationActorRef,
    ) -> Result<(), InboundTurnError> {
        Ok(())
    }

    async fn unpair_external_actor_if_owned_by(
        &self,
        _tenant_id: &TenantId,
        _adapter_kind: &AdapterKind,
        adapter_installation_id: &ironclaw_conversations::AdapterInstallationId,
        external_actor_ref: &ConversationActorRef,
        expected: &ExpectedExternalActorOwner,
    ) -> Result<ConditionalUnpairOutcome, InboundTurnError> {
        self.unpairs.lock().expect("unpairs lock").push((
            adapter_installation_id.as_str().to_string(),
            external_actor_ref.id().to_string(),
            expected
                .binding_epoch
                .as_ref()
                .map(|epoch| epoch.as_str().to_string()),
        ));
        Ok(ConditionalUnpairOutcome::Unpaired)
    }
}

struct Fixture {
    service: ChannelPairingService,
    identity: Arc<InMemoryIdentity>,
    dispatcher: Arc<RecordingDispatcher>,
    actor_pairings: Arc<RecordingActorPairings>,
    dm_targets: Arc<FilesystemChannelDmTargetStore>,
}

fn fixture_with_prefixes(
    installation: Option<&str>,
    deep_link_template: Option<&str>,
    template_values: BTreeMap<String, String>,
    inbound_code_prefixes: &[&str],
) -> Fixture {
    let backend: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let tenant = TenantId::new("tenant-alpha").expect("tenant");
    let operator = UserId::new("operator").expect("operator");
    let extension_id = ExtensionId::new(EXT).expect("extension id");
    let identity = Arc::new(InMemoryIdentity::default());
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let actor_pairings = Arc::new(RecordingActorPairings::default());
    let dm_targets = Arc::new(FilesystemChannelDmTargetStore::new(
        Arc::clone(&backend),
        tenant.clone(),
        operator.clone(),
    ));
    let store = Arc::new(FilesystemChannelPairingStore::new(
        Arc::clone(&backend),
        tenant.clone(),
        operator,
        extension_id.clone(),
    ));
    let descriptor = ExtensionAccountSetupDescriptor {
        extension_id: extension_id.clone(),
        auth_requirement: ironclaw_host_api::RuntimeCredentialAuthRequirement {
            provider: ironclaw_host_api::VendorId::new(EXT).expect("vendor"),
            setup: ironclaw_host_api::RuntimeCredentialAccountSetup::Pairing,
            requester_extension: extension_id,
            provider_scopes: Vec::new(),
        },
        connection_notices: ChannelConnectionNoticePolicy::generic("Vendor X"),
        connection_requirement: ironclaw_product::ChannelConnectionRequirement {
            channel: EXT.to_string(),
            display_name: "Vendor X".to_string(),
            strategy: ironclaw_product::RebornChannelConnectStrategy::WebGeneratedCode,
            instructions: "Send /start <code> to the bot or open the link.".to_string(),
            input_placeholder: String::new(),
            submit_label: "Connect".to_string(),
            error_message: "Invalid or expired pairing code.".to_string(),
        },
        connection_success_message: "Connected.".to_string(),
        pairing_deep_link_template: deep_link_template.map(str::to_string),
        pairing_inbound_code_prefixes: inbound_code_prefixes
            .iter()
            .map(|prefix| (*prefix).to_string())
            .collect(),
    };
    let service = ChannelPairingService::new(
        tenant,
        ironclaw_host_api::AgentId::new("agent-a").expect("agent"),
        None,
        descriptor,
        ChannelPairingServiceDependencies {
            store,
            installation: Arc::new(StaticInstallation(
                installation.map(|id| AdapterInstallationId::new(id).expect("installation id")),
            )),
            template_values: Arc::new(StaticTemplateValues(template_values)),
            identity: Arc::new(ComposedChannelPairingIdentityStore::new(
                Arc::clone(&identity) as Arc<dyn RebornUserIdentityBindingStore>,
                Arc::clone(&identity)
                    as Arc<dyn crate::provider_identity::RebornUserIdentityLookup>,
                Arc::clone(&identity) as Arc<dyn RebornUserIdentityBindingDeleteStore>,
            )),
            continuation: Arc::clone(&dispatcher) as Arc<dyn RebornAuthContinuationDispatcher>,
            conversation_actor_pairings: Arc::clone(&actor_pairings)
                as Arc<dyn ConversationActorPairingService>,
            direct_targets: Arc::new(ComposedChannelPairingDirectTargetStore::new(Arc::clone(
                &dm_targets,
            ))),
        },
    );
    Fixture {
        service,
        identity,
        dispatcher,
        actor_pairings,
        dm_targets,
    }
}

fn fixture_with(
    installation: Option<&str>,
    deep_link_template: Option<&str>,
    template_values: BTreeMap<String, String>,
) -> Fixture {
    fixture_with_prefixes(
        installation,
        deep_link_template,
        template_values,
        &["/start"],
    )
}

fn fixture() -> Fixture {
    fixture_with(
        Some(INSTALL),
        Some("https://vendor.example/{bot_username}?start={code}"),
        BTreeMap::from([("bot_username".to_string(), "acme_bot".to_string())]),
    )
}

fn pairing_ingress(service: Arc<ChannelPairingService>) -> Arc<GenericChannelInboundSink> {
    Arc::new(
        GenericChannelInboundSink::new(ChannelInboundSinkConfig {
            adapter_id: ProductAdapterId::new(EXT).expect("adapter id"),
            evidence: VerifiedEvidenceMint::SharedSecretHeader {
                header: "X-Vendor-Secret".to_string(),
            },
            surface: Arc::new(UnexpectedWorkflow),
            observer: None,
        })
        .with_pairing(service as Arc<dyn ChannelPairingInterceptor>, None),
    )
}

fn pairing_ingress_with_outcomes(
    service: Arc<ChannelPairingService>,
) -> (
    Arc<GenericChannelInboundSink>,
    Arc<Mutex<Vec<ChannelPairingConsumeOutcome>>>,
) {
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::new(
        GenericChannelInboundSink::new(ChannelInboundSinkConfig {
            adapter_id: ProductAdapterId::new(EXT).expect("adapter id"),
            evidence: VerifiedEvidenceMint::SharedSecretHeader {
                header: "X-Vendor-Secret".to_string(),
            },
            surface: Arc::new(UnexpectedWorkflow),
            observer: None,
        })
        .with_pairing(
            service as Arc<dyn ChannelPairingInterceptor>,
            Some(ChannelPairingOutcomeObserver::Recording(Arc::clone(
                &outcomes,
            ))),
        ),
    );
    (sink, outcomes)
}

fn pairing_admission(code: &ChannelPairingCode) -> InboundAdmission {
    pairing_admission_for(code, INSTALL, "u-1")
}

fn pairing_admission_for(
    code: &ChannelPairingCode,
    installation_id: &str,
    actor_id: &str,
) -> InboundAdmission {
    InboundAdmission {
        extension_id: EXT.to_string(),
        installation_id: installation_id.to_string(),
        message: direct_message(&format!("/start {}", code.as_str()), actor_id),
    }
}

fn install() -> AdapterInstallationId {
    AdapterInstallationId::new(INSTALL).expect("installation id")
}

fn user(id: &str) -> UserId {
    UserId::new(id).expect("user")
}

#[tokio::test]
async fn mint_fails_closed_without_installed_extension() {
    let fixture = fixture_with(None, None, BTreeMap::new());
    let error = fixture
        .service
        .issue_or_rotate(&user("alice"))
        .await
        .expect_err("no installation must fail closed");
    assert_eq!(error, ChannelPairingError::NotConfigured);
}

#[tokio::test]
async fn mint_rotates_to_a_single_live_code_and_resolves_the_deep_link() {
    let fixture = fixture();
    let first = fixture
        .service
        .issue_or_rotate(&user("alice"))
        .await
        .expect("mint");
    assert_eq!(
        first.deep_link.as_deref(),
        Some(&*format!(
            "https://vendor.example/acme_bot?start={}",
            first.code.as_str()
        ))
    );
    let second = fixture
        .service
        .issue_or_rotate(&user("alice"))
        .await
        .expect("rotate");
    assert_ne!(first.code, second.code);

    // The rotated-away code no longer consumes.
    let outcome = fixture
        .service
        .consume(
            &install(),
            first.code.as_str(),
            "vendor_user",
            "u-1",
            None,
            "chat-1",
        )
        .await
        .expect("consume");
    assert_eq!(outcome, ChannelPairingConsumeOutcome::ExpiredOrUnknown);

    // Status shows the live pending code.
    let status = fixture
        .service
        .status_for(&user("alice"))
        .await
        .expect("status");
    assert!(!status.connected);
    assert_eq!(status.pending.expect("pending issue").code, second.code);
}

#[tokio::test]
async fn missing_template_values_fall_back_to_code_only_presentation() {
    let fixture = fixture_with(
        Some(INSTALL),
        Some("https://vendor.example/{bot_username}?start={code}"),
        BTreeMap::new(),
    );
    let issue = fixture
        .service
        .issue_or_rotate(&user("alice"))
        .await
        .expect("mint");
    assert!(issue.deep_link.is_none());
}

#[tokio::test]
async fn consume_binds_identity_records_dm_target_then_dispatches_continuation() {
    let fixture = fixture();
    let issue = fixture
        .service
        .issue_or_rotate(&user("alice"))
        .await
        .expect("mint");

    let outcome = fixture
        .service
        .consume(
            &install(),
            &format!("  {}  ", issue.code.as_str().to_ascii_lowercase()),
            "vendor_user",
            "u-1",
            None,
            "chat-9",
        )
        .await
        .expect("consume");
    assert_eq!(
        outcome,
        ChannelPairingConsumeOutcome::Paired {
            user_id: user("alice")
        }
    );

    // Identity bound under the extension-id provider, installation-scoped.
    let bound = fixture
        .identity
        .resolve_user_identity(EXT, &format!("{INSTALL}:u-1"))
        .await
        .expect("lookup");
    assert_eq!(bound, Some(user("alice")));

    // DM target durably recorded for delivery.
    let target = fixture
        .dm_targets
        .load(EXT, &user("alice"))
        .await
        .expect("dm load")
        .expect("dm target present");
    assert_eq!(target.external_actor_id, "u-1");

    // Consume has committed one durable completion intent, but it does not
    // dispatch lifecycle policy itself. The generic ingress caller owns the
    // synchronous dispatch-before-ack boundary.
    assert_eq!(
        fixture
            .service
            .pending_completion_dispatch_ids_for_test()
            .await
            .expect("pairing completion ids")
            .len(),
        1
    );
    assert!(
        fixture
            .dispatcher
            .events
            .lock()
            .expect("events lock")
            .is_empty()
    );
    let connected = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        fixture.service.status_for(&user("alice")),
    )
    .await
    .expect("connection status must not wait for lifecycle continuation")
    .expect("connection status");
    assert!(connected.connected);
    assert!(
        fixture
            .dispatcher
            .events
            .lock()
            .expect("events lock")
            .is_empty(),
        "status reads must not recursively dispatch lifecycle continuation"
    );
    assert_eq!(
        fixture
            .service
            .pending_completion_dispatch_ids_for_test()
            .await
            .expect("pairing completion ids after status")
            .len(),
        1,
        "status reads must not settle the durable completion intent"
    );
    fixture
        .service
        .finish_pending_for_user_for_test(&user("alice"))
        .await
        .expect("finish pairing completion");

    // Pairing is the final manifest-declared setup step, so its durable
    // completion requests lifecycle reconciliation itself. The browser never
    // issues a second, best-effort activate request.
    {
        let events = fixture.dispatcher.events.lock().expect("events lock");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].provider.as_str(), EXT);
        assert_eq!(
            events[0].continuation,
            AuthContinuationRef::LifecycleActivation {
                package_ref: ironclaw_auth::LifecyclePackageRef::new(EXT)
                    .expect("lifecycle package ref"),
            }
        );
    }
    assert!(
        fixture
            .service
            .pending_completion_dispatch_ids_for_test()
            .await
            .expect("settled pairing completion ids")
            .is_empty(),
        "accepted continuation must CAS-settle the exact durable intent"
    );
    // Connected now; a second consumer of the burned code learns nothing.
    let status = fixture
        .service
        .status_for(&user("alice"))
        .await
        .expect("status");
    assert!(status.connected);
    let replay = fixture
        .service
        .consume(
            &install(),
            issue.code.as_str(),
            "vendor_user",
            "u-2",
            None,
            "chat-2",
        )
        .await
        .expect("replay consume");
    assert_eq!(replay, ChannelPairingConsumeOutcome::ExpiredOrUnknown);
}

#[tokio::test]
async fn provider_ack_waits_for_generic_fanout_acceptance() {
    let Fixture { mut service, .. } = fixture();
    let issue = service
        .issue_or_rotate(&user("alice"))
        .await
        .expect("mint pairing code");
    let fanout = Arc::new(BlockingFanoutAcceptance::default());
    service.replace_continuation_for_test(
        Arc::clone(&fanout) as Arc<dyn RebornAuthContinuationDispatcher>
    );
    let service = Arc::new(service);
    let sink = pairing_ingress(Arc::clone(&service));

    let mut admission = tokio::spawn({
        let sink = Arc::clone(&sink);
        let request = pairing_admission(&issue.code);
        async move { sink.admit(request).await }
    });
    tokio::time::timeout(
        std::time::Duration::from_millis(250),
        fanout.started.notified(),
    )
    .await
    .expect("generic continuation fan-out must start");
    assert_eq!(
        service
            .pending_completion_dispatch_ids_for_test()
            .await
            .expect("pending completion ids")
            .len(),
        1,
        "completion remains durable until fan-out accepts it"
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(25), &mut admission)
            .await
            .is_err(),
        "provider acknowledgement must not precede fan-out acceptance"
    );

    fanout.release.notify_one();
    assert_eq!(
        admission
            .await
            .expect("admission task")
            .expect("pairing admission"),
        InboundAdmissionAck::Accepted
    );
    assert_eq!(fanout.accepted.load(Ordering::SeqCst), 1);
    assert!(
        service
            .pending_completion_dispatch_ids_for_test()
            .await
            .expect("settled completion ids")
            .is_empty()
    );
}

#[tokio::test]
async fn transient_fanout_failure_requests_redelivery_and_reuses_durable_event_identity() {
    let Fixture { mut service, .. } = fixture();
    let issue = service
        .issue_or_rotate(&user("alice"))
        .await
        .expect("mint pairing code");
    let fanout = Arc::new(FailOnceIdempotentFanout::default());
    service.replace_continuation_for_test(
        Arc::clone(&fanout) as Arc<dyn RebornAuthContinuationDispatcher>
    );
    let service = Arc::new(service);
    let sink = pairing_ingress(Arc::clone(&service));

    let first = sink
        .admit(pairing_admission(&issue.code))
        .await
        .expect_err("transient fan-out failure must not acknowledge provider ingress");
    assert!(first.retryable);
    let pending = service
        .pending_completion_dispatch_ids_for_test()
        .await
        .expect("completion ids after transient failure");
    assert_eq!(pending.len(), 1);
    let durable_dispatch_id = pending[0];
    assert_eq!(
        fanout.attempts.lock().expect("attempts lock").as_slice(),
        &[durable_dispatch_id]
    );

    let status = service
        .status_for(&user("alice"))
        .await
        .expect("side-effect-free status read");
    assert!(status.connected, "DM target was durable before fan-out");
    assert_eq!(
        fanout.attempts.lock().expect("attempts lock").len(),
        1,
        "status polling must not retry the continuation"
    );
    assert_eq!(
        service
            .pending_completion_dispatch_ids_for_test()
            .await
            .expect("completion ids after status read")
            .len(),
        1
    );

    let redelivery = sink
        .admit(pairing_admission(&issue.code))
        .await
        .expect("provider redelivery must re-drive the durable intent");
    assert_eq!(redelivery, InboundAdmissionAck::Accepted);
    assert_eq!(fanout.resumed.load(Ordering::SeqCst), 1);
    assert_eq!(
        fanout.attempts.lock().expect("attempts lock").as_slice(),
        &[durable_dispatch_id, durable_dispatch_id],
        "redelivery must reuse the exact durable continuation identity"
    );
    assert!(
        service
            .pending_completion_dispatch_ids_for_test()
            .await
            .expect("settled completion ids after redelivery")
            .is_empty(),
        "successful redelivery must CAS-settle the durable intent"
    );
}

#[tokio::test]
async fn concurrent_caller_admission_has_exactly_one_pairing_winner() {
    let fixture = fixture();
    let issue = fixture
        .service
        .issue_or_rotate(&user("alice"))
        .await
        .expect("mint");
    let service = Arc::new(fixture.service);
    let (sink, outcomes) = pairing_ingress_with_outcomes(Arc::clone(&service));

    let (first, second) = tokio::join!(
        sink.admit(pairing_admission_for(&issue.code, INSTALL, "u-1")),
        sink.admit(pairing_admission_for(&issue.code, INSTALL, "u-2")),
    );
    assert_eq!(
        first.expect("first caller admission"),
        InboundAdmissionAck::Accepted
    );
    assert_eq!(
        second.expect("second caller admission"),
        InboundAdmissionAck::Accepted
    );
    ChannelIngressDrain::drain(sink.as_ref()).await;

    let outcomes = outcomes.lock().expect("outcomes lock");
    assert_eq!(outcomes.len(), 2);
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, ChannelPairingConsumeOutcome::Paired { .. }))
            .count(),
        1,
        "one CAS claimant must win the live code"
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| { matches!(outcome, ChannelPairingConsumeOutcome::ExpiredOrUnknown) })
            .count(),
        1,
        "the losing caller must learn no code ownership detail"
    );
    let bindings = fixture.identity.bindings.lock().expect("bindings lock");
    assert_eq!(bindings.len(), 1, "only the winning actor may be bound");
    assert_eq!(bindings.values().next(), Some(&user("alice")));
}

#[tokio::test]
async fn caller_admission_isolates_foreign_installations_and_wrong_users() {
    let fixture = fixture();
    let issue = fixture
        .service
        .issue_or_rotate(&user("alice"))
        .await
        .expect("mint");
    fixture
        .identity
        .bind_user_identity(RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new(EXT).expect("provider"),
            provider_user_id: RebornIdentityProviderUserId::new(format!("{INSTALL}:u-bob"))
                .expect("provider user"),
            user_id: user("bob"),
        })
        .await
        .expect("pre-bind bob");
    let service = Arc::new(fixture.service);
    let (sink, outcomes) = pairing_ingress_with_outcomes(Arc::clone(&service));

    // Wrong installation: indistinguishable from unknown.
    assert_eq!(
        sink.admit(pairing_admission_for(&issue.code, "install-2", "u-foreign"))
            .await
            .expect("foreign-installation admission"),
        InboundAdmissionAck::Accepted
    );
    ChannelIngressDrain::drain(sink.as_ref()).await;
    assert_eq!(
        outcomes.lock().expect("outcomes lock").as_slice(),
        &[ChannelPairingConsumeOutcome::ExpiredOrUnknown]
    );

    // A sender already bound to Bob cannot consume Alice's still-live code.
    outcomes.lock().expect("outcomes lock").clear();
    assert_eq!(
        sink.admit(pairing_admission_for(&issue.code, INSTALL, "u-bob"))
            .await
            .expect("wrong-user admission"),
        InboundAdmissionAck::Accepted
    );
    ChannelIngressDrain::drain(sink.as_ref()).await;
    assert_eq!(
        outcomes.lock().expect("outcomes lock").as_slice(),
        &[ChannelPairingConsumeOutcome::AlreadyBoundToOtherUser]
    );

    let status = service.status_for(&user("alice")).await.expect("status");
    assert_eq!(status.pending.expect("still live").code, issue.code);

    // The rightful caller can still consume the code after both refusals.
    outcomes.lock().expect("outcomes lock").clear();
    assert_eq!(
        sink.admit(pairing_admission_for(&issue.code, INSTALL, "u-alice"))
            .await
            .expect("rightful caller admission"),
        InboundAdmissionAck::Accepted
    );
    ChannelIngressDrain::drain(sink.as_ref()).await;
    assert_eq!(
        outcomes.lock().expect("outcomes lock").as_slice(),
        &[ChannelPairingConsumeOutcome::Paired {
            user_id: user("alice")
        }]
    );
    assert_eq!(
        fixture
            .identity
            .resolve_user_identity(EXT, &format!("{INSTALL}:u-bob"))
            .await
            .expect("bob binding lookup"),
        Some(user("bob")),
        "wrong-user refusal must preserve Bob's existing binding"
    );
}

#[tokio::test]
async fn bound_sender_rerunning_a_code_repairs_completion_idempotently() {
    let fixture = fixture();
    let issue = fixture
        .service
        .issue_or_rotate(&user("alice"))
        .await
        .expect("mint");
    fixture
        .service
        .consume(
            &install(),
            issue.code.as_str(),
            "vendor_user",
            "u-1",
            None,
            "chat-1",
        )
        .await
        .expect("consume");
    // Simulate a lost DM target (the completion side effect).
    fixture
        .dm_targets
        .delete(EXT, &user("alice"))
        .await
        .expect("delete target");

    let repair = fixture
        .service
        .consume(
            &install(),
            issue.code.as_str(),
            "vendor_user",
            "u-1",
            None,
            "chat-1",
        )
        .await
        .expect("repair consume");
    assert_eq!(
        repair,
        ChannelPairingConsumeOutcome::AlreadyPairedSameUser {
            user_id: user("alice")
        }
    );
    assert!(
        fixture
            .dm_targets
            .load(EXT, &user("alice"))
            .await
            .expect("dm load")
            .is_some(),
        "repair path re-records the DM target"
    );
}

#[tokio::test]
async fn unpair_drops_bindings_target_codes_and_conversation_actor_pairings() {
    let fixture = fixture();
    let issue = fixture
        .service
        .issue_or_rotate(&user("alice"))
        .await
        .expect("mint");
    fixture
        .service
        .consume(
            &install(),
            issue.code.as_str(),
            "vendor_user",
            "u-1",
            None,
            "chat-1",
        )
        .await
        .expect("consume");
    // A fresh pending code exists too; unpair must invalidate it.
    fixture
        .service
        .issue_or_rotate(&user("alice"))
        .await
        .expect("re-mint");

    fixture
        .service
        .unpair(&user("alice"))
        .await
        .expect("unpair");

    assert_eq!(
        fixture
            .identity
            .resolve_user_identity(EXT, &format!("{INSTALL}:u-1"))
            .await
            .expect("lookup"),
        None
    );
    assert!(
        fixture
            .dm_targets
            .load(EXT, &user("alice"))
            .await
            .expect("dm load")
            .is_none()
    );
    let status = fixture
        .service
        .status_for(&user("alice"))
        .await
        .expect("status");
    assert!(!status.connected);
    assert!(status.pending.is_none(), "pending code invalidated");
    let unpairs = fixture.actor_pairings.unpairs.lock().expect("unpairs lock");
    assert_eq!(unpairs.len(), 1);
    assert_eq!(unpairs[0].0, INSTALL);
    assert_eq!(unpairs[0].1, "u-1");
    assert!(
        unpairs[0].2.is_some(),
        "pairing completion carries its durable exact-owner epoch into cleanup"
    );
}

fn direct_message(text: &str, actor_id: &str) -> NormalizedInboundMessage {
    NormalizedInboundMessage {
        actor: ExternalActorRef::new("vendor_user", actor_id, None::<&str>).expect("actor"),
        conversation: ExternalConversationRef::new(None, "chat-1", None, None)
            .expect("conversation"),
        event_id: ExternalEventId::new("evt-1").expect("event"),
        text: text.to_string(),
        trigger: ProductTriggerReason::DirectChat,
        attachments: Vec::new(),
        reply_context: None,
    }
}

#[tokio::test]
async fn interceptor_services_manifest_declared_start_messages_only() {
    let fixture = fixture();
    let issue = fixture
        .service
        .issue_or_rotate(&user("alice"))
        .await
        .expect("mint");

    // Non-code text flows to admission.
    assert_eq!(
        fixture
            .service
            .intercept(&install(), &direct_message("hello there", "u-1"))
            .await,
        ChannelPairingInterception::NotHandled
    );
    // Group-triggered code text flows to admission (pairing is DM-only).
    let mut group = direct_message(issue.code.as_str(), "u-1");
    group.trigger = ProductTriggerReason::BotMention;
    assert_eq!(
        fixture.service.intercept(&install(), &group).await,
        ChannelPairingInterception::NotHandled
    );
    // `/pair` remains ordinary text because Telegram declares only `/start`.
    let pair = direct_message(&format!("/pair {}", issue.code.as_str()), "u-1");
    assert_eq!(
        fixture.service.intercept(&install(), &pair).await,
        ChannelPairingInterception::NotHandled
    );

    // The Telegram-style `/start CODE` shape is serviced because this
    // fixture declares `/start` as an allowed proof-code prefix.
    let start = direct_message(&format!("/start {}", issue.code.as_str()), "u-1");
    assert_eq!(
        fixture.service.intercept(&install(), &start).await,
        ChannelPairingInterception::Consumed(ChannelPairingConsumeOutcome::Paired {
            user_id: user("alice"),
        })
    );
    assert_eq!(
        fixture
            .identity
            .resolve_user_identity(EXT, &format!("{INSTALL}:u-1"))
            .await
            .expect("lookup"),
        Some(user("alice"))
    );
}

#[tokio::test]
async fn interceptor_treats_undeclared_commands_as_ordinary_text_but_accepts_bare_codes() {
    let fixture = fixture_with_prefixes(Some(INSTALL), None, BTreeMap::new(), &[]);
    let issue = fixture
        .service
        .issue_or_rotate(&user("alice"))
        .await
        .expect("mint");

    for prefix in ["/start", "/pair"] {
        let command = direct_message(&format!("{prefix} {}", issue.code.as_str()), "u-1");
        assert_eq!(
            fixture.service.intercept(&install(), &command).await,
            ChannelPairingInterception::NotHandled,
            "undeclared {prefix} must remain ordinary inbound text"
        );
    }

    assert_eq!(
        fixture
            .service
            .intercept(&install(), &direct_message(issue.code.as_str(), "u-1"))
            .await,
        ChannelPairingInterception::Consumed(ChannelPairingConsumeOutcome::Paired {
            user_id: user("alice"),
        })
    );
}

#[tokio::test]
async fn interceptor_accepts_another_manifest_declared_prefix() {
    let fixture = fixture_with_prefixes(Some(INSTALL), None, BTreeMap::new(), &["/connect"]);
    let issue = fixture
        .service
        .issue_or_rotate(&user("alice"))
        .await
        .expect("mint");
    let command = direct_message(&format!("/connect {}", issue.code.as_str()), "u-1");

    assert_eq!(
        fixture.service.intercept(&install(), &command).await,
        ChannelPairingInterception::Consumed(ChannelPairingConsumeOutcome::Paired {
            user_id: user("alice"),
        })
    );
}
