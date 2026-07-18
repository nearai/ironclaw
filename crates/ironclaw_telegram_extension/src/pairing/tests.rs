pub(crate) mod pairing_test_support {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_conversations::{
        AdapterInstallationId, AdapterKind, ConditionalUnpairOutcome,
        ConversationActorPairingService, ExpectedExternalActorOwner, ExternalActorBindingEpoch,
        ExternalActorRef, InboundTurnError,
    };
    use ironclaw_host_api::{TenantId, UserId};

    #[derive(Default)]
    pub(crate) struct RecordingActorPairings {
        pub(crate) conditional_unpairs: Mutex<Vec<(String, String, String)>>,
        pub(crate) fail_unpairs_remaining: std::sync::atomic::AtomicUsize,
    }

    impl RecordingActorPairings {
        pub(crate) fn shared() -> Arc<Self> {
            Arc::new(Self::default())
        }
    }

    #[async_trait]
    impl ConversationActorPairingService for RecordingActorPairings {
        async fn pair_external_actor(
            &self,
            _tenant_id: TenantId,
            _adapter_kind: AdapterKind,
            _adapter_installation_id: AdapterInstallationId,
            _external_actor_ref: ExternalActorRef,
            _user_id: UserId,
        ) -> Result<(), InboundTurnError> {
            Ok(())
        }

        async fn pair_external_actor_with_epoch(
            &self,
            _tenant_id: TenantId,
            _adapter_kind: AdapterKind,
            _adapter_installation_id: AdapterInstallationId,
            _external_actor_ref: ExternalActorRef,
            _user_id: UserId,
            _binding_epoch: ExternalActorBindingEpoch,
        ) -> Result<(), InboundTurnError> {
            Ok(())
        }

        async fn unpair_external_actor(
            &self,
            _tenant_id: TenantId,
            _adapter_kind: AdapterKind,
            _adapter_installation_id: AdapterInstallationId,
            _external_actor_ref: ExternalActorRef,
        ) -> Result<(), InboundTurnError> {
            Ok(())
        }

        async fn unpair_external_actor_if_owned_by(
            &self,
            _tenant_id: &TenantId,
            _adapter_kind: &AdapterKind,
            adapter_installation_id: &AdapterInstallationId,
            external_actor_ref: &ExternalActorRef,
            expected: &ExpectedExternalActorOwner,
        ) -> Result<ConditionalUnpairOutcome, InboundTurnError> {
            if self
                .fail_unpairs_remaining
                .fetch_update(
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                    |remaining| remaining.checked_sub(1),
                )
                .is_ok()
            {
                return Err(InboundTurnError::DurableState {
                    reason: "injected actor cleanup failure".to_string(),
                });
            }
            self.conditional_unpairs
                .lock()
                .expect("recording lock")
                .push((
                    adapter_installation_id.as_str().to_string(),
                    external_actor_ref.id().to_string(),
                    expected.user_id.as_str().to_string(),
                ));
            Ok(ConditionalUnpairOutcome::Unpaired)
        }
    }
}

use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_auth::{AuthContinuationEvent, AuthContinuationRef, AuthProductError};
use ironclaw_channel_host::auth_continuation::RebornAuthContinuationDispatcher;
use ironclaw_host_api::{AgentId, TenantId, UserId};
use ironclaw_product_adapters::AdapterInstallationId;
use ironclaw_secrets::InMemorySecretStore;
use secrecy::SecretString;

use super::*;
use crate::setup::{TelegramInstallationSetupUpdate, TelegramSetupService};
use crate::state::FilesystemTelegramHostState;
use crate::telegram_actor_identity::telegram_user_identity_provider_user_id;
use crate::test_support::{RecordingBotApi, fault_injected_telegram_state, telegram_state};

#[derive(Debug, Default)]
struct RecordingDispatcher {
    events: StdMutex<Vec<AuthContinuationEvent>>,
    fail_remaining: std::sync::atomic::AtomicUsize,
}

impl RecordingDispatcher {
    fn failing_once() -> Self {
        Self {
            events: StdMutex::new(Vec::new()),
            fail_remaining: std::sync::atomic::AtomicUsize::new(1),
        }
    }
}

#[async_trait]
impl RebornAuthContinuationDispatcher for RecordingDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        if self
            .fail_remaining
            .fetch_update(
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
                |remaining| remaining.checked_sub(1),
            )
            .is_ok()
        {
            return Err(AuthProductError::BackendUnavailable);
        }
        self.events.lock().expect("lock").push(event);
        Ok(())
    }
}

struct Fixture {
    service: TelegramPairingService,
    installation_id: AdapterInstallationId,
    dispatcher: Arc<RecordingDispatcher>,
    state: Arc<FilesystemTelegramHostState>,
    setup: Arc<TelegramSetupService>,
    actor_pairings: Arc<super::pairing_test_support::RecordingActorPairings>,
}

async fn fixture(configured: bool) -> Fixture {
    fixture_with_state(
        configured,
        telegram_state(),
        Arc::new(RecordingDispatcher::default()),
    )
    .await
}

async fn fixture_with_state(
    configured: bool,
    state: Arc<FilesystemTelegramHostState>,
    dispatcher: Arc<RecordingDispatcher>,
) -> Fixture {
    let tenant_id = TenantId::new("tenant-a").expect("tenant");
    let agent_id = AgentId::new("agent-a").expect("agent");
    let bot_api = RecordingBotApi::default();
    bot_api.set_bot_identity(777, "ironclaw_qa_bot");
    let setup = Arc::new(TelegramSetupService::new(
        tenant_id.clone(),
        agent_id.clone(),
        None,
        UserId::new("operator").expect("user"),
        Arc::clone(&state),
        Arc::new(InMemorySecretStore::new()),
        bot_api.client(),
        Some("https://ironclaw.example".to_string()),
    ));
    if configured {
        setup
            .save_with_previous(TelegramInstallationSetupUpdate {
                bot_token: Some(SecretString::from("123:abc".to_string())),
                webhook_url_override: None,
            })
            .await
            .expect("setup saves");
    }
    let actor_pairings = super::pairing_test_support::RecordingActorPairings::shared();
    let service = TelegramPairingService::new(
        tenant_id,
        agent_id,
        None,
        Arc::clone(&setup),
        Arc::clone(&state),
        Arc::clone(&dispatcher) as Arc<dyn RebornAuthContinuationDispatcher>,
        Arc::clone(&actor_pairings)
            as Arc<dyn ironclaw_conversations::ConversationActorPairingService>,
    );
    Fixture {
        service,
        installation_id: AdapterInstallationId::new("tg-bot-777").expect("installation"),
        dispatcher,
        state,
        setup,
        actor_pairings,
    }
}

fn user(name: &str) -> UserId {
    UserId::new(name).expect("user")
}

#[tokio::test]
async fn issue_mints_code_with_deep_link_and_ttl() {
    let fixture = fixture(true).await;
    let issue = fixture
        .service
        .issue_or_rotate(&user("ben"))
        .await
        .expect("issue");
    assert_eq!(issue.code.len(), PAIRING_CODE_LEN);
    assert!(
        issue
            .code
            .bytes()
            .all(|byte| PAIRING_CODE_ALPHABET.contains(&byte))
    );
    assert_eq!(
        issue.deep_link,
        format!("https://t.me/ironclaw_qa_bot?start={}", issue.code)
    );
    assert!(issue.expires_at > Utc::now());
}

#[tokio::test]
async fn issue_fails_closed_when_unconfigured() {
    let fixture = fixture(false).await;
    let error = fixture
        .service
        .issue_or_rotate(&user("ben"))
        .await
        .expect_err("no code without admin setup");
    assert_eq!(error, TelegramPairingError::NotConfigured);
}

#[tokio::test]
async fn reissue_rotates_and_kills_the_old_code() {
    let fixture = fixture(true).await;
    let first = fixture
        .service
        .issue_or_rotate(&user("ben"))
        .await
        .expect("first");
    let second = fixture
        .service
        .issue_or_rotate(&user("ben"))
        .await
        .expect("second");
    assert_ne!(first.code, second.code);
    let outcome = fixture
        .service
        .consume(&fixture.installation_id, &first.code, "tg-1", 100)
        .await
        .expect("consume old");
    assert_eq!(outcome, PairingConsumeOutcome::ExpiredOrUnknown);
    let outcome = fixture
        .service
        .consume(&fixture.installation_id, &second.code, "tg-1", 100)
        .await
        .expect("consume new");
    assert!(matches!(outcome, PairingConsumeOutcome::Paired { .. }));
}

#[tokio::test]
async fn consume_happy_path_binds_targets_and_dispatches() {
    let fixture = fixture(true).await;
    let ben = user("ben");
    let issue = fixture.service.issue_or_rotate(&ben).await.expect("issue");
    let outcome = fixture
        .service
        .consume(
            &fixture.installation_id,
            &issue.code.to_ascii_lowercase(),
            "tg-100",
            555,
        )
        .await
        .expect("consume");
    assert_eq!(
        outcome,
        PairingConsumeOutcome::Paired {
            user_id: ben.clone()
        }
    );

    let status = fixture.service.status_for(&ben).await.expect("status");
    assert!(status.connected);
    assert!(status.pending.is_none());

    let events = fixture.dispatcher.events.lock().expect("lock").clone();
    assert_eq!(events.len(), 1, "exactly one continuation dispatch");
    assert_eq!(events[0].provider.as_str(), "telegram");
    assert!(matches!(
        events[0].continuation,
        AuthContinuationRef::SetupOnly
    ));
    assert_eq!(events[0].scope.resource.user_id, ben);

    let replay = fixture
        .service
        .consume(&fixture.installation_id, &issue.code, "tg-other", 556)
        .await
        .expect("replay");
    assert_eq!(
        replay,
        PairingConsumeOutcome::ExpiredOrUnknown,
        "single-use"
    );
}

#[tokio::test]
async fn consume_unknown_or_malformed_never_dispatches() {
    let fixture = fixture(true).await;
    for code in ["NOPE1234", "short", "!!!!!!!!"] {
        let outcome = fixture
            .service
            .consume(&fixture.installation_id, code, "tg-1", 1)
            .await
            .expect("consume");
        assert_eq!(outcome, PairingConsumeOutcome::ExpiredOrUnknown);
    }
    assert!(fixture.dispatcher.events.lock().expect("lock").is_empty());
}

#[tokio::test]
async fn bot_swap_cannot_consume_or_project_a_stale_installation_code() {
    let fixture = fixture(true).await;
    let ben = user("ben");
    let issue = fixture.service.issue_or_rotate(&ben).await.expect("issue");
    let mut swapped = fixture
        .setup
        .current_setup()
        .await
        .expect("setup read")
        .expect("configured");
    swapped.bot_id = 888;
    swapped.bot_username = "ironclaw_new_bot".to_string();
    swapped.revision += 1;
    swapped.updated_at = Utc::now();
    fixture
        .state
        .put_telegram_installation_setup(&swapped)
        .await
        .expect("bot swap persists");
    let new_installation = swapped.installation_id().expect("installation id");

    assert_eq!(
        fixture
            .service
            .consume(&new_installation, &issue.code, "tg-1", 1)
            .await
            .expect("stale code is handled"),
        PairingConsumeOutcome::ExpiredOrUnknown,
    );
    assert!(
        fixture
            .service
            .status_for(&ben)
            .await
            .expect("status")
            .pending
            .is_none(),
        "the new bot must not deep-link a pending code minted for the old installation"
    );
}

#[tokio::test]
async fn telegram_account_bound_to_other_user_is_refused() {
    let fixture = fixture(true).await;
    let ben = user("ben");
    let illia = user("illia");
    let ben_issue = fixture.service.issue_or_rotate(&ben).await.expect("issue");
    fixture
        .service
        .consume(&fixture.installation_id, &ben_issue.code, "tg-shared", 1)
        .await
        .expect("ben pairs");
    let illia_issue = fixture
        .service
        .issue_or_rotate(&illia)
        .await
        .expect("issue");
    let outcome = fixture
        .service
        .consume(&fixture.installation_id, &illia_issue.code, "tg-shared", 2)
        .await
        .expect("consume");
    assert_eq!(outcome, PairingConsumeOutcome::AlreadyBoundToOtherUser);
    let ben_status = fixture.service.status_for(&ben).await.expect("status");
    assert!(ben_status.connected, "original binding intact");
}

#[tokio::test]
async fn same_user_re_pair_is_idempotent() {
    let fixture = fixture(true).await;
    let ben = user("ben");
    let first = fixture.service.issue_or_rotate(&ben).await.expect("issue");
    fixture
        .service
        .consume(&fixture.installation_id, &first.code, "tg-100", 1)
        .await
        .expect("pair");
    let second = fixture.service.issue_or_rotate(&ben).await.expect("issue");
    let outcome = fixture
        .service
        .consume(&fixture.installation_id, &second.code, "tg-100", 1)
        .await
        .expect("re-pair");
    assert_eq!(
        outcome,
        PairingConsumeOutcome::AlreadyPairedSameUser { user_id: ben }
    );
}

/// Two concurrent consumers of the same live code, from different
/// Telegram accounts, both read the record before either claims it (the
/// barrier pins that interleaving). Exactly one may bind: the claim is
/// single-consumer and happens before any identity/target side effect.
#[tokio::test]
async fn concurrent_consume_of_one_code_binds_exactly_one_winner() {
    let (state, filesystem) = fault_injected_telegram_state();
    let fixture = fixture_with_state(true, state, Arc::new(RecordingDispatcher::default())).await;
    let ben = user("ben");
    let issue = fixture.service.issue_or_rotate(&ben).await.expect("issue");
    filesystem.hold_next_reads_at(2, Arc::new(tokio::sync::Barrier::new(2)));

    let (first, second) = tokio::join!(
        fixture
            .service
            .consume(&fixture.installation_id, &issue.code, "tg-attacker", 111,),
        fixture
            .service
            .consume(&fixture.installation_id, &issue.code, "tg-victim", 222,),
    );
    let outcomes = [first.expect("consume"), second.expect("consume")];

    let paired = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, PairingConsumeOutcome::Paired { .. }))
        .count();
    let refused = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, PairingConsumeOutcome::ExpiredOrUnknown))
        .count();
    assert_eq!(paired, 1, "exactly one concurrent consumer may pair");
    assert_eq!(refused, 1, "the claim loser is refused");
    let installation_id = fixture
        .setup
        .current_setup()
        .await
        .expect("setup read")
        .expect("configured")
        .installation_id()
        .expect("installation id");
    let mut bound_count = 0;
    for telegram_user_id in ["tg-attacker", "tg-victim"] {
        let provider_user_id =
            telegram_user_identity_provider_user_id(&installation_id, telegram_user_id);
        if fixture
            .state
            .bound_user_for(&provider_user_id)
            .await
            .expect("binding read")
            .is_some()
        {
            bound_count += 1;
        }
    }
    assert_eq!(bound_count, 1, "the loser must not leave a binding behind");
    assert_eq!(
        fixture.dispatcher.events.lock().expect("lock").len(),
        1,
        "exactly one continuation dispatch"
    );
}

/// A continuation dispatch that fails after the code was claimed must not
/// strand the blocked run: the WebUI's existing status poll drains the durable
/// completion record — no consumed-code resend is required.
#[tokio::test]
async fn status_poll_retries_durable_completion_after_dispatch_failure() {
    let fixture = fixture_with_state(
        true,
        telegram_state(),
        Arc::new(RecordingDispatcher::failing_once()),
    )
    .await;
    let ben = user("ben");
    let issue = fixture.service.issue_or_rotate(&ben).await.expect("issue");

    let error = fixture
        .service
        .consume(&fixture.installation_id, &issue.code, "tg-100", 555)
        .await
        .expect_err("first consume surfaces the dispatch failure");
    assert!(matches!(
        error,
        TelegramPairingError::ContinuationDispatch { .. }
    ));
    assert!(
        fixture.dispatcher.events.lock().expect("lock").is_empty(),
        "failed dispatch recorded no continuation"
    );

    let status = fixture
        .service
        .status_for(&ben)
        .await
        .expect("status poll repairs pending completion");
    assert!(status.connected, "completion retry publishes the DM target");
    let events = fixture.dispatcher.events.lock().expect("lock").clone();
    assert_eq!(events.len(), 1, "repair re-dispatches the continuation");
    assert_eq!(events[0].scope.resource.user_id, ben);
}

#[tokio::test]
async fn unpair_removes_binding_target_and_pending_code() {
    let fixture = fixture(true).await;
    let ben = user("ben");
    let issue = fixture.service.issue_or_rotate(&ben).await.expect("issue");
    fixture
        .service
        .consume(&fixture.installation_id, &issue.code, "tg-100", 1)
        .await
        .expect("pair");
    fixture.service.unpair(&ben).await.expect("unpair");
    let status = fixture.service.status_for(&ben).await.expect("status");
    assert!(!status.connected);
    let fresh = fixture.service.issue_or_rotate(&ben).await.expect("issue");
    let outcome = fixture
        .service
        .consume(&fixture.installation_id, &fresh.code, "tg-100", 1)
        .await
        .expect("re-pair after unpair");
    assert!(matches!(outcome, PairingConsumeOutcome::Paired { .. }));
    let unpairs = fixture
        .actor_pairings
        .conditional_unpairs
        .lock()
        .expect("recording lock")
        .clone();
    assert_eq!(
        unpairs.len(),
        1,
        "unpair clears the conversation-actor pairing (Slack disconnect parity) — \
             leaving it re-attaches a re-paired chat to its old thread"
    );
    assert!(
        unpairs[0].0.starts_with("tg-bot-"),
        "cleanup targets the stored installation: {unpairs:?}"
    );
    drop(fixture.setup);
}

#[tokio::test]
async fn unpair_retries_actor_cleanup_from_durable_binding_metadata() {
    let fixture = fixture(true).await;
    let ben = user("ben");
    let issue = fixture.service.issue_or_rotate(&ben).await.expect("issue");
    fixture
        .service
        .consume(&fixture.installation_id, &issue.code, "tg-100", 1)
        .await
        .expect("pair");
    fixture
        .actor_pairings
        .fail_unpairs_remaining
        .store(1, std::sync::atomic::Ordering::SeqCst);

    assert!(
        fixture.service.unpair(&ben).await.is_err(),
        "the injected actor cleanup failure is surfaced"
    );
    assert!(
        fixture
            .state
            .bound_user_for(&telegram_user_identity_provider_user_id(
                &fixture.installation_id,
                "tg-100",
            ))
            .await
            .expect("binding lookup")
            .is_none(),
        "the inactive binding already fails closed"
    );
    assert!(
        fixture
            .state
            .dm_target_for_user(&fixture.installation_id, &ben)
            .await
            .expect("DM target lookup")
            .is_none(),
        "delivery authority is removed before retryable actor cleanup"
    );

    fixture
        .service
        .unpair(&ben)
        .await
        .expect("retry reconstructs and completes actor cleanup");
    assert_eq!(
        fixture
            .actor_pairings
            .conditional_unpairs
            .lock()
            .expect("recording lock")
            .len(),
        1,
        "the retained index and epoch make cleanup retryable"
    );
}

/// Unpair must not depend on the current bot setup: after an admin clears
/// the deployment, a user's disconnect still removes their durable
/// binding — reconfiguring the same bot must not silently resurrect the
/// connection they explicitly severed.
#[tokio::test]
async fn unpair_after_admin_cleared_setup_still_removes_the_binding() {
    let fixture = fixture(true).await;
    let ben = user("ben");
    let issue = fixture.service.issue_or_rotate(&ben).await.expect("issue");
    fixture
        .service
        .consume(&fixture.installation_id, &issue.code, "tg-100", 1)
        .await
        .expect("pair");

    fixture.setup.clear().await.expect("admin clears setup");
    fixture.service.unpair(&ben).await.expect("unpair");
    let provider_user_id = telegram_user_identity_provider_user_id(
        &AdapterInstallationId::new("tg-bot-777").expect("installation"),
        "tg-100",
    );
    assert!(
        fixture
            .state
            .bound_user_for(&provider_user_id)
            .await
            .expect("binding read")
            .is_none(),
        "unpair without a current setup must still remove the binding"
    );

    // Reconfigure the same bot: the disconnected user must NOT come back
    // paired, and their old Telegram account is unbound.
    fixture
        .setup
        .save_with_previous(TelegramInstallationSetupUpdate {
            bot_token: Some(SecretString::from("123:abc".to_string())),
            webhook_url_override: None,
        })
        .await
        .expect("same bot reconfigures");
    let status = fixture.service.status_for(&ben).await.expect("status");
    assert!(
        !status.connected,
        "clear-setup → unpair → reconfigure must not resurrect the pairing"
    );
}
