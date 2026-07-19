pub(crate) mod test_fixtures {
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use async_trait::async_trait;
    use ironclaw_auth::AuthContinuationEvent;
    use ironclaw_auth::AuthProductError;
    use ironclaw_host_api::{AgentId, TenantId, UserId};
    use ironclaw_product_adapters::{
        ProductAdapterError, ProductInboundAck, ProductInboundEnvelope,
        ProjectionSubscriptionRequest,
    };
    use ironclaw_secrets::InMemorySecretStore;
    use secrecy::SecretString;

    use super::super::*;
    use crate::setup::TelegramInstallationSetupUpdate;
    use crate::state::FilesystemTelegramHostState;
    pub(crate) use crate::test_support::RecordingBotApi;
    use crate::test_support::telegram_state;
    use ironclaw_channel_host::auth_continuation::RebornAuthContinuationDispatcher;
    use ironclaw_channel_host::identity::RebornUserIdentityLookupError;

    pub(crate) const FIXTURE_BOT_ID: i64 = 4242;
    pub(crate) const FIXTURE_BOT_USERNAME: &str = "ironclaw_qa_bot";

    #[derive(Debug, Default)]
    pub(crate) struct RecordingContinuationDispatcher {
        events: StdMutex<Vec<AuthContinuationEvent>>,
    }

    #[async_trait]
    impl RebornAuthContinuationDispatcher for RecordingContinuationDispatcher {
        async fn dispatch_auth_continuation(
            &self,
            event: AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            self.events.lock().expect("lock").push(event); // safety: test-only fixture
            Ok(())
        }
    }

    /// Identity lookup fake keyed by `(provider, provider_user_id)`. `fail()`
    /// switches every read into a backend error.
    #[derive(Debug, Default)]
    pub(crate) struct FakeIdentityLookup {
        bindings: StdMutex<HashMap<(String, String), UserId>>,
        fail: AtomicBool,
    }

    impl FakeIdentityLookup {
        pub(crate) fn bind(&self, provider: &str, provider_user_id: &str, user: &str) {
            let mut bindings = self.bindings.lock().expect("lock"); // safety: test-only fixture
            bindings.insert(
                (provider.to_string(), provider_user_id.to_string()),
                UserId::new(user).expect("valid user id"), // safety: test-only fixture
            );
        }

        pub(crate) fn fail(&self) {
            self.fail.store(true, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl RebornUserIdentityLookup for FakeIdentityLookup {
        async fn resolve_user_identity(
            &self,
            provider: &str,
            provider_user_id: &str,
        ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(RebornUserIdentityLookupError::Backend(
                    "test lookup outage".to_string(),
                ));
            }
            Ok(self
                .bindings
                .lock()
                .expect("lock") // safety: test-only fixture
                .get(&(provider.to_string(), provider_user_id.to_string()))
                .cloned())
        }

        async fn user_has_provider_binding(
            &self,
            provider: &str,
            user_id: &UserId,
        ) -> Result<bool, RebornUserIdentityLookupError> {
            Ok(self
                .bindings
                .lock()
                .expect("lock") // safety: test-only fixture
                .iter()
                .any(|((bound_provider, _), bound)| bound_provider == provider && bound == user_id))
        }
    }

    /// `ProductWorkflow` fake counting durable submissions.
    pub(crate) struct CountingWorkflow {
        submitted: Arc<AtomicUsize>,
    }

    impl CountingWorkflow {
        pub(crate) fn new(submitted: Arc<AtomicUsize>) -> Self {
            Self { submitted }
        }
    }

    #[async_trait]
    impl ironclaw_product_adapters::ProductWorkflow for CountingWorkflow {
        async fn submit_inbound(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProductInboundAck, ProductAdapterError> {
            self.submitted.fetch_add(1, Ordering::SeqCst);
            Ok(ProductInboundAck::NoOp)
        }

        async fn resolve_projection_subscription(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProjectionSubscriptionRequest, ProductAdapterError> {
            Err(ProductAdapterError::Internal {
                detail: ironclaw_product_adapters::redaction::RedactedString::new(
                    "test stub: resolve_projection_subscription not supported",
                ),
            })
        }
    }

    fn tenant_id() -> TenantId {
        TenantId::new("tenant-a").expect("valid tenant") // safety: test-only fixture
    }

    fn agent_id() -> AgentId {
        AgentId::new("agent-a").expect("valid agent") // safety: test-only fixture
    }

    pub(crate) fn unconfigured_setup_service(
        bot_api: Arc<RecordingBotApi>,
    ) -> Arc<TelegramSetupService> {
        unconfigured_setup_service_with_state(bot_api, telegram_state())
    }

    pub(crate) fn unconfigured_setup_service_with_state(
        bot_api: Arc<RecordingBotApi>,
        state: Arc<FilesystemTelegramHostState>,
    ) -> Arc<TelegramSetupService> {
        Arc::new(TelegramSetupService::new(
            tenant_id(),
            agent_id(),
            None,
            UserId::new("operator").expect("valid user"), // safety: test-only fixture
            state,
            Arc::new(InMemorySecretStore::new()),
            bot_api.client(),
            Some("https://ironclaw.example".to_string()),
        ))
    }

    /// A saved deployment bot (`tg-bot-4242`) with token + webhook secret in
    /// the in-memory secret store.
    pub(crate) async fn configured_setup_service(
        bot_api: Arc<RecordingBotApi>,
    ) -> Arc<TelegramSetupService> {
        let setup = unconfigured_setup_service(bot_api);
        setup
            .save_with_previous(TelegramInstallationSetupUpdate {
                bot_token: Some(SecretString::from("123:abc".to_string())),
                webhook_url_override: None,
            })
            .await
            .expect("test setup saves"); // safety: test-only fixture
        setup
    }

    pub(crate) fn pairing_service_with(
        setup: Arc<TelegramSetupService>,
    ) -> Arc<TelegramPairingService> {
        let state = setup.state_for_test();
        Arc::new(TelegramPairingService::new(
            tenant_id(),
            agent_id(),
            None,
            setup,
            state,
            Arc::new(RecordingContinuationDispatcher::default()),
            crate::pairing::pairing_test_support::RecordingActorPairings::shared(),
        ))
    }

    pub(crate) fn fixture_installation_id() -> AdapterInstallationId {
        AdapterInstallationId::new(format!("tg-bot-{FIXTURE_BOT_ID}"))
            .expect("valid installation id") // safety: test-only fixture
    }

    /// A private-chat update body; `text: None` models a media-only message.
    pub(crate) fn private_text_update_body(
        from_id: i64,
        chat_id: i64,
        text: Option<&str>,
    ) -> Vec<u8> {
        let mut message = serde_json::json!({
            "message_id": 100,
            "date": 1,
            "chat": {"id": chat_id, "type": "private"},
            "from": {"id": from_id, "is_bot": false, "first_name": "Test"},
        });
        if let Some(text) = text {
            message["text"] = serde_json::Value::String(text.to_string());
        }
        serde_json::to_vec(&serde_json::json!({
            "update_id": 7,
            "message": message,
        }))
        .expect("test body serializes") // safety: test-only fixture
    }
}

mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ironclaw_product_adapters::auth::mark_shared_secret_header_verified;

    use super::super::*;
    use super::test_fixtures::{
        FakeIdentityLookup, RecordingBotApi, configured_setup_service, fixture_installation_id,
        pairing_service_with, private_text_update_body,
    };
    use crate::ingress::TELEGRAM_SECRET_TOKEN_HEADER;

    struct FakeForwardRunner {
        calls: Arc<AtomicUsize>,
    }

    impl TelegramUpdatesWebhookDispatcher for FakeForwardRunner {
        fn verify_webhook_auth(
            &self,
            _headers: &HeaderMap,
            _body: &[u8],
        ) -> Result<ProtocolAuthEvidence, RunnerError> {
            Ok(verified_evidence())
        }

        fn process_verified_update<'a>(
            &'a self,
            _body: &'a [u8],
            _evidence: &'a ProtocolAuthEvidence,
            _observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
        ) -> Pin<Box<dyn Future<Output = Result<WebhookProcessOutcome, RunnerError>> + Send + 'a>>
        {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(WebhookProcessOutcome::AcceptedForAsyncDispatch) })
        }

        fn drain_immediate_ack_tasks<'a>(
            &'a self,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async {})
        }
    }

    fn verified_evidence() -> ProtocolAuthEvidence {
        mark_shared_secret_header_verified(TELEGRAM_SECRET_TOKEN_HEADER, "tg-bot-4242")
    }

    struct Fixture {
        pre_router: TelegramInboundPreRouter,
        pairing: Arc<TelegramPairingService>,
        lookup: Arc<FakeIdentityLookup>,
        bot_api: Arc<RecordingBotApi>,
        runner_calls: Arc<AtomicUsize>,
    }

    impl Fixture {
        async fn process(&self, body: &[u8]) -> Result<WebhookProcessOutcome, RunnerError> {
            let evidence = verified_evidence();
            self.pre_router
                .process_verified_update(body, &evidence, None)
                .await
        }

        fn paired_provider_user_id(&self, from_id: i64) -> String {
            telegram_user_identity_provider_user_id(
                &fixture_installation_id(),
                &from_id.to_string(),
            )
        }

        fn bind_paired(&self, from_id: i64, user: &str) {
            self.lookup.bind(
                TELEGRAM_IDENTITY_PROVIDER,
                &self.paired_provider_user_id(from_id),
                user,
            );
        }
    }

    async fn fixture() -> Fixture {
        fixture_with_window(HINT_THROTTLE_WINDOW).await
    }

    async fn fixture_with_window(window: Duration) -> Fixture {
        let bot_api = Arc::new(RecordingBotApi::default());
        let setup = configured_setup_service(Arc::clone(&bot_api)).await;
        let pairing = pairing_service_with(Arc::clone(&setup));
        let lookup = Arc::new(FakeIdentityLookup::default());
        let runner_calls = Arc::new(AtomicUsize::new(0));
        let runner = Arc::new(FakeForwardRunner {
            calls: Arc::clone(&runner_calls),
        });
        let pre_router = TelegramInboundPreRouter::new(
            Arc::clone(&pairing),
            lookup.clone() as Arc<dyn RebornUserIdentityLookup>,
            bot_api.client(),
            setup,
            fixture_installation_id(),
            runner,
        )
        .with_hint_throttle_window(window);
        Fixture {
            pre_router,
            pairing,
            lookup,
            bot_api,
            runner_calls,
        }
    }

    fn user(name: &str) -> ironclaw_host_api::UserId {
        ironclaw_host_api::UserId::new(name).expect("valid user")
    }

    fn assert_silently_handled(outcome: &WebhookProcessOutcome) {
        assert!(
            matches!(
                outcome,
                WebhookProcessOutcome::Acknowledged {
                    ack: ProductInboundAck::NoOp
                }
            ),
            "expected silent ack, got: {outcome:?}"
        );
    }

    // Row (a): non-actionable updates are acked with no reply and no forward.
    #[tokio::test]
    async fn non_actionable_updates_are_acked_silently() {
        let fixture = fixture().await;
        let group_body = serde_json::to_vec(&serde_json::json!({
            "update_id": 2,
            "message": {
                "chat": {"id": -100, "type": "supergroup"},
                "from": {"id": 42, "is_bot": false},
                "text": "hello"
            }
        }))
        .expect("body");
        let bot_sender_body = serde_json::to_vec(&serde_json::json!({
            "update_id": 3,
            "message": {
                "chat": {"id": 555, "type": "private"},
                "from": {"id": 42, "is_bot": true},
                "text": "hello"
            }
        }))
        .expect("body");
        let no_from_body = serde_json::to_vec(&serde_json::json!({
            "update_id": 4,
            "message": {"chat": {"id": 555, "type": "private"}, "text": "hello"}
        }))
        .expect("body");
        let cases: Vec<Vec<u8>> = vec![
            br#"{"update_id":1}"#.to_vec(),
            group_body,
            bot_sender_body,
            no_from_body,
            b"not json at all".to_vec(),
        ];

        for body in cases {
            let outcome = fixture.process(&body).await.expect("acked");
            assert_silently_handled(&outcome);
        }
        assert_eq!(fixture.runner_calls.load(Ordering::SeqCst), 0);
        assert!(fixture.bot_api.sends().is_empty());
    }

    // Row (b): `/start <CODE>` with a live code pairs and replies.
    #[tokio::test]
    async fn start_with_live_code_pairs_and_replies() {
        let fixture = fixture().await;
        let ben = user("ben");
        let issue = fixture
            .pairing
            .issue_or_rotate(&ben)
            .await
            .expect("code issues");

        let body = private_text_update_body(42, 555, Some(&format!("/start {}", issue.code)));
        let outcome = fixture.process(&body).await.expect("acked");

        assert_silently_handled(&outcome);
        assert_eq!(fixture.runner_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            fixture.bot_api.sends(),
            vec![(555, PAIRED_REPLY.to_string())]
        );
        let status = fixture
            .pairing
            .status_for(&ben)
            .await
            .expect("status reads");
        assert!(status.connected, "consume must actually bind the account");
    }

    // Row (b): a bare code (any case) is a pairing attempt too.
    #[tokio::test]
    async fn bare_lowercase_code_pairs() {
        let fixture = fixture().await;
        let ben = user("ben");
        let issue = fixture
            .pairing
            .issue_or_rotate(&ben)
            .await
            .expect("code issues");

        let body = private_text_update_body(42, 555, Some(&issue.code.to_ascii_lowercase()));
        fixture.process(&body).await.expect("acked");

        assert_eq!(
            fixture.bot_api.sends(),
            vec![(555, PAIRED_REPLY.to_string())]
        );
        assert_eq!(fixture.runner_calls.load(Ordering::SeqCst), 0);
    }

    // Row (b): every consume outcome maps to its static reply.
    #[tokio::test]
    async fn consume_outcome_replies_are_mapped() {
        let fixture = fixture().await;
        let ben = user("ben");
        let illia = user("illia");

        // Pair ben's telegram account 42.
        let first = fixture.pairing.issue_or_rotate(&ben).await.expect("issue");
        fixture
            .process(&private_text_update_body(
                42,
                555,
                Some(&format!("/start {}", first.code)),
            ))
            .await
            .expect("acked");

        // Same user re-pairs the same account with a fresh code.
        let second = fixture.pairing.issue_or_rotate(&ben).await.expect("issue");
        fixture
            .process(&private_text_update_body(
                42,
                555,
                Some(&format!("/start {}", second.code)),
            ))
            .await
            .expect("acked");

        // Another user's code against the already-bound telegram account.
        let illia_issue = fixture
            .pairing
            .issue_or_rotate(&illia)
            .await
            .expect("issue");
        fixture
            .process(&private_text_update_body(
                42,
                555,
                Some(&format!("/start {}", illia_issue.code)),
            ))
            .await
            .expect("acked");

        // Unknown / malformed code payload.
        fixture
            .process(&private_text_update_body(42, 555, Some("/start NOPE1234")))
            .await
            .expect("acked");

        assert_eq!(
            fixture.bot_api.sends(),
            vec![
                (555, PAIRED_REPLY.to_string()),
                (555, ALREADY_PAIRED_SAME_USER_REPLY.to_string()),
                (555, ALREADY_BOUND_TO_OTHER_USER_REPLY.to_string()),
                (555, EXPIRED_OR_UNKNOWN_REPLY.to_string()),
            ]
        );
        assert_eq!(fixture.runner_calls.load(Ordering::SeqCst), 0);
    }

    /// Invalid pairing guesses are uniformly answered but the failure reply
    /// is rate-limited per chat, and the limiter never gates a valid consume
    /// — a typo followed by the real code (the deep-link retry shape) still
    /// pairs immediately. Automates the reply-throttle half of
    /// qa-telegram:P13; guess feasibility itself is bounded by the code
    /// space, TTL, and the per-installation ingress limiter.
    #[tokio::test]
    async fn invalid_code_replies_are_throttled_per_chat_without_gating_valid_consume() {
        let fixture = fixture().await;
        let ben = user("ben");

        // Two invalid code-shaped guesses in the same chat: one reply only.
        for _ in 0..2 {
            fixture
                .process(&private_text_update_body(42, 555, Some("AAAAAAAA")))
                .await
                .expect("acked");
        }
        // A different chat gets its own (single) failure reply.
        fixture
            .process(&private_text_update_body(43, 777, Some("AAAAAAAA")))
            .await
            .expect("acked");
        assert_eq!(
            fixture.bot_api.sends(),
            vec![
                (555, EXPIRED_OR_UNKNOWN_REPLY.to_string()),
                (777, EXPIRED_OR_UNKNOWN_REPLY.to_string()),
            ],
            "the second same-chat invalid guess must be answered with silence"
        );

        // The throttle must not gate valid consumption: the real code sent
        // from the throttled chat still pairs and still confirms.
        let issue = fixture
            .pairing
            .issue_or_rotate(&ben)
            .await
            .expect("code issues");
        fixture
            .process(&private_text_update_body(
                42,
                555,
                Some(&format!("/start {}", issue.code)),
            ))
            .await
            .expect("acked");
        let status = fixture.pairing.status_for(&ben).await.expect("status");
        assert!(status.connected, "valid consume must bypass the throttle");
        assert_eq!(
            fixture.bot_api.sends().last(),
            Some(&(555, PAIRED_REPLY.to_string())),
            "the success confirmation is never throttled"
        );
    }

    /// qa-telegram:C6 — a PAIRED sender's bare `/start` is a static no-op:
    /// no turn, no reply (re-opening the chat must not pitch pairing to an
    /// already-paired account). Unpaired `/start` keeps the throttled hint
    /// (pinned above).
    #[tokio::test]
    async fn paired_start_without_payload_is_a_silent_no_op() {
        let fixture = fixture().await;
        fixture.bind_paired(42, "ben");

        let outcome = fixture
            .process(&private_text_update_body(42, 555, Some("/start")))
            .await
            .expect("acked");

        assert_silently_handled(&outcome);
        assert_eq!(fixture.runner_calls.load(Ordering::SeqCst), 0, "no turn");
        assert_eq!(
            fixture.bot_api.sends(),
            Vec::<(i64, String)>::new(),
            "no reply of any kind to a paired /start"
        );
    }

    /// A pairedness-lookup outage on `/start` acks silently rather than
    /// pitching the pairing hint to a possibly-paired sender or asking
    /// Telegram to redeliver a greeting.
    #[tokio::test]
    async fn start_without_payload_acks_silently_when_lookup_is_down() {
        let fixture = fixture().await;
        fixture.lookup.fail();

        let outcome = fixture
            .process(&private_text_update_body(42, 555, Some("/start")))
            .await
            .expect("acked");

        assert_silently_handled(&outcome);
        assert_eq!(fixture.bot_api.sends(), Vec::<(i64, String)>::new());
    }

    // Row (c): bare `/start` and unpaired ordinary text hint, throttled per
    // chat within the window.
    /// A hint send that FAILS must release the chat's throttle slot: the
    /// throttle marks before sending, so without the release one Telegram
    /// hiccup silences the chat's onboarding hint for the full window —
    /// live-observed as "DMing the disconnected bot gets no reply at all"
    /// (2026-07-17).
    #[tokio::test]
    async fn failed_hint_send_releases_the_throttle_for_the_next_message() {
        let fixture = fixture().await;

        fixture.bot_api.fail_sends();
        fixture
            .process(&private_text_update_body(42, 555, Some("hello?")))
            .await
            .expect("acked despite failed hint send");

        fixture.bot_api.succeed_sends();
        fixture
            .process(&private_text_update_body(43, 555, Some("anyone there?")))
            .await
            .expect("acked");

        let sends = fixture.bot_api.sends();
        assert_eq!(
            sends.len(),
            2,
            "the failed first hint must not consume the throttle window;              the second message retries the hint: {sends:?}"
        );
        assert!(
            sends[1].1.contains("Pair your account"),
            "the retried hint is the onboarding hint: {:?}",
            sends[1].1
        );
    }

    #[tokio::test]
    async fn unpaired_hints_are_throttled_per_chat() {
        let fixture = fixture().await;

        fixture
            .process(&private_text_update_body(42, 555, Some("/start")))
            .await
            .expect("acked");
        // Second trigger inside the window (ordinary unpaired text this time)
        // must be suppressed for the same chat.
        fixture
            .process(&private_text_update_body(42, 555, Some("hello there")))
            .await
            .expect("acked");
        // A different chat gets its own hint.
        fixture
            .process(&private_text_update_body(43, 777, Some("/start")))
            .await
            .expect("acked");

        assert_eq!(
            fixture.bot_api.sends(),
            vec![
                (555, UNPAIRED_HINT_REPLY.to_string()),
                (777, UNPAIRED_HINT_REPLY.to_string()),
            ]
        );
        assert_eq!(fixture.runner_calls.load(Ordering::SeqCst), 0);
    }

    // Throttle pruning: entries older than the window are dropped, so the
    // next hint is allowed again.
    #[tokio::test]
    async fn hint_throttle_prunes_entries_older_than_window() {
        let fixture = fixture_with_window(Duration::ZERO).await;

        fixture
            .process(&private_text_update_body(42, 555, Some("/start")))
            .await
            .expect("acked");
        fixture
            .process(&private_text_update_body(42, 555, Some("/start")))
            .await
            .expect("acked");

        assert_eq!(
            fixture.bot_api.sends(),
            vec![
                (555, UNPAIRED_HINT_REPLY.to_string()),
                (555, UNPAIRED_HINT_REPLY.to_string()),
            ],
            "a zero window prunes the previous entry, so both hints send"
        );
    }

    // Row (d): a paired sender's ordinary text forwards to the runner
    // exactly once, with no static reply.
    #[tokio::test]
    async fn paired_ordinary_text_forwards_to_runner_exactly_once() {
        let fixture = fixture().await;
        fixture.bind_paired(42, "ben");

        let outcome = fixture
            .process(&private_text_update_body(42, 555, Some("hello ironclaw")))
            .await
            .expect("forwarded");

        assert!(matches!(
            outcome,
            WebhookProcessOutcome::AcceptedForAsyncDispatch
        ));
        assert_eq!(fixture.runner_calls.load(Ordering::SeqCst), 1);
        assert!(fixture.bot_api.sends().is_empty());
    }

    // Row (d)/(c): media messages without text follow the pairing split.
    #[tokio::test]
    async fn textless_message_follows_pairing_split() {
        let fixture = fixture().await;
        fixture.bind_paired(42, "ben");

        fixture
            .process(&private_text_update_body(42, 555, None))
            .await
            .expect("forwarded");
        assert_eq!(
            fixture.runner_calls.load(Ordering::SeqCst),
            1,
            "paired media-only message forwards"
        );

        fixture
            .process(&private_text_update_body(99, 888, None))
            .await
            .expect("acked");
        assert_eq!(
            fixture.bot_api.sends(),
            vec![(888, UNPAIRED_HINT_REPLY.to_string())],
            "unpaired media-only message hints"
        );
        assert_eq!(fixture.runner_calls.load(Ordering::SeqCst), 1);
    }

    // Static-reply failures are swallowed: the webhook still acks.
    #[tokio::test]
    async fn send_failures_are_swallowed() {
        let fixture = fixture().await;
        fixture.bot_api.fail_sends();

        let outcome = fixture
            .process(&private_text_update_body(42, 555, Some("/start")))
            .await
            .expect("acked despite send failure");

        assert_silently_handled(&outcome);
        assert_eq!(fixture.bot_api.sends().len(), 1, "send was attempted");
    }

    // An identity-store outage must not silently drop a possibly-paired
    // sender's message: surface a retryable error so the transport retries.
    #[tokio::test]
    async fn lookup_outage_maps_to_retryable_error() {
        let fixture = fixture().await;
        fixture.lookup.fail();

        let error = fixture
            .process(&private_text_update_body(42, 555, Some("hello there")))
            .await
            .expect_err("lookup outage is not an ack");

        match &error {
            RunnerError::Adapter(adapter_error) => {
                assert!(adapter_error.is_retryable(), "must ask for redelivery");
            }
            other => panic!("expected retryable adapter error, got: {other:?}"),
        }
        assert_eq!(fixture.runner_calls.load(Ordering::SeqCst), 0);
        assert!(fixture.bot_api.sends().is_empty());
    }

    #[test]
    fn classify_inbound_text_covers_admission_rows() {
        assert_eq!(
            classify_inbound_text(Some("/start")),
            InboundTextClass::StartWithoutPayload
        );
        assert_eq!(
            classify_inbound_text(Some("  /start  ABCDEFGH ")),
            InboundTextClass::PairingAttempt("ABCDEFGH".to_string())
        );
        assert_eq!(
            classify_inbound_text(Some("abcdefgh")),
            InboundTextClass::PairingAttempt("abcdefgh".to_string())
        );
        // O, 0, 1 and I are not in the pairing alphabet: not code-shaped.
        assert_eq!(
            classify_inbound_text(Some("NOPE1234")),
            InboundTextClass::Ordinary
        );
        assert_eq!(
            classify_inbound_text(Some("hello there")),
            InboundTextClass::Ordinary
        );
        assert_eq!(
            classify_inbound_text(Some("/start CODE EXTRA")),
            InboundTextClass::Ordinary
        );
        assert_eq!(classify_inbound_text(None), InboundTextClass::Ordinary);
        // Multi-byte text must not panic the length checks.
        assert_eq!(
            classify_inbound_text(Some("héllö wörld ✅")),
            InboundTextClass::Ordinary
        );
    }
}
