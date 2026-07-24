// Unit tests for the generic pairing service; child module so
// `use super::*` reaches crate-private items.
use std::sync::Mutex;

use ironclaw_auth::AuthProductError;
use ironclaw_conversations::{
    ConditionalUnpairOutcome, ExternalActorRef as ConversationActorRef, InboundTurnError,
};
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_product::{
    ExternalActorRef, ExternalConversationRef, ExternalEventId, NormalizedInboundMessage,
    ProductTriggerReason,
};

use super::*;
use crate::extension_host::extension_ingress::{
    ChannelPairingInterception, ChannelPairingInterceptor,
};
use ironclaw_extension_host::FilesystemChannelDmTargetStore;

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
    async fn template_values(&self) -> Result<BTreeMap<String, String>, String> {
        Ok(self.0.clone())
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

fn fixture_with(
    installation: Option<&str>,
    deep_link_template: Option<&str>,
    template_values: BTreeMap<String, String>,
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
    let service = ChannelPairingService::new(ChannelPairingServiceParts {
        tenant_id: tenant,
        agent_id: ironclaw_host_api::AgentId::new("agent-a").expect("agent"),
        project_id: None,
        extension_id,
        connection_notices: ChannelConnectionNoticePolicy::generic("Vendor X"),
        deep_link_template: deep_link_template.map(str::to_string),
        store,
        installation: Arc::new(StaticInstallation(
            installation.map(|id| AdapterInstallationId::new(id).expect("installation id")),
        )),
        template_values: Arc::new(StaticTemplateValues(template_values)),
        identity_bind: Arc::clone(&identity) as Arc<dyn RebornUserIdentityBindingStore>,
        identity_lookup: Arc::clone(&identity)
            as Arc<dyn crate::provider_identity::RebornUserIdentityLookup>,
        identity_delete: Arc::clone(&identity) as Arc<dyn RebornUserIdentityBindingDeleteStore>,
        continuation: Arc::clone(&dispatcher) as Arc<dyn RebornAuthContinuationDispatcher>,
        conversation_actor_pairings: Arc::clone(&actor_pairings)
            as Arc<dyn ConversationActorPairingService>,
        dm_targets: Arc::clone(&dm_targets),
    });
    Fixture {
        service,
        identity,
        dispatcher,
        actor_pairings,
        dm_targets,
    }
}

fn fixture() -> Fixture {
    fixture_with(
        Some(INSTALL),
        Some("https://vendor.example/{bot_username}?start={code}"),
        BTreeMap::from([("bot_username".to_string(), "acme_bot".to_string())]),
    )
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
async fn consume_binds_identity_records_dm_target_and_dispatches_continuation() {
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

    // The standard fan-out continuation fired, provider-keyed SetupOnly.
    {
        let events = fixture.dispatcher.events.lock().expect("events lock");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].provider.as_str(), EXT);
        assert_eq!(events[0].continuation, AuthContinuationRef::SetupOnly);
    }
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
async fn consume_refuses_codes_for_foreign_installations_and_bound_senders() {
    let fixture = fixture();
    let issue = fixture
        .service
        .issue_or_rotate(&user("alice"))
        .await
        .expect("mint");

    // Wrong installation: indistinguishable from unknown.
    let foreign = AdapterInstallationId::new("install-2").expect("installation id");
    assert_eq!(
        fixture
            .service
            .consume(
                &foreign,
                issue.code.as_str(),
                "vendor_user",
                "u-1",
                None,
                "c"
            )
            .await
            .expect("consume"),
        ChannelPairingConsumeOutcome::ExpiredOrUnknown
    );

    // A sender already bound to bob consuming alice's live code is
    // refused, and the code stays live for alice.
    let alice_code = fixture
        .service
        .issue_or_rotate(&user("alice"))
        .await
        .expect("mint alice");
    fixture
        .identity
        .bind_user_identity(RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new(EXT).expect("provider"),
            provider_user_id: RebornIdentityProviderUserId::new(format!("{INSTALL}:u-7"))
                .expect("provider user"),
            user_id: user("bob"),
        })
        .await
        .expect("pre-bind");
    assert_eq!(
        fixture
            .service
            .consume(
                &install(),
                alice_code.code.as_str(),
                "vendor_user",
                "u-7",
                None,
                "c"
            )
            .await
            .expect("consume"),
        ChannelPairingConsumeOutcome::AlreadyBoundToOtherUser
    );
    let status = fixture
        .service
        .status_for(&user("alice"))
        .await
        .expect("status");
    assert_eq!(status.pending.expect("still live").code, alice_code.code);
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
    assert_eq!(
        unpairs[0].2, None,
        "generic identity store carries no epoch"
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
async fn interceptor_services_code_shaped_direct_messages_only() {
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

    // The deep-link `/start CODE` shape is serviced and swallowed.
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
