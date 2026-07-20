use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier as StdBarrier, Mutex as StdMutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt;
use ironclaw_channel_host::identity::RebornUserIdentityLookup;
use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor,
    IngressScopeSource, ListenerClass, RateLimitPolicy, RateLimitScope, StreamingMode,
    WebSocketOriginPolicy,
};
use ironclaw_product_adapters::auth::mark_shared_secret_header_verified;
use ironclaw_product_adapters::{AdapterInstallationId, ProtocolAuthEvidence, ProtocolAuthFailure};
use ironclaw_wasm_product_adapters::{
    ImmediateAckWorkflowObserver, RunnerError, WebhookProcessOutcome,
};
use secrecy::ExposeSecret;
use std::num::{NonZeroU32, NonZeroU64};
use tower::ServiceExt;

use super::*;
use crate::ingress::dispatch::test_fixtures::{
    CountingWorkflow, FakeIdentityLookup, RecordingBotApi, configured_setup_service,
    fixture_installation_id, pairing_service_with, private_text_update_body,
    unconfigured_setup_service, unconfigured_setup_service_with_state,
};
use crate::setup::{
    TelegramInstallationSetup, TelegramInstallationSetupUpdate, TelegramSetupService,
};
use crate::telegram_actor_identity::{
    TELEGRAM_IDENTITY_PROVIDER, telegram_user_identity_provider_user_id,
};
use crate::test_support::fault_injected_telegram_state;
use secrecy::SecretString;

/// Rebuild the Telegram ingress descriptor as a Rust literal so the
/// manifest-projected descriptor can be asserted equal to it (the
/// manifest-driven ingress contract stays real and load-bearing).
fn expected_telegram_descriptor() -> IngressRouteDescriptor {
    let policy = IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::PublicWebhook,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::WebhookSignature],
        },
        scope_source: IngressScopeSource::HostResolved,
        body_limit: BodyLimitPolicy::Limited {
            max_bytes: NonZeroU64::new(1024 * 1024).expect("nonzero"),
        },
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::Global,
            max_requests: NonZeroU32::new(12_000).expect("nonzero"),
            window_seconds: NonZeroU32::new(60).expect("nonzero"),
        },
        cors: CorsPolicy::NotApplicable,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::PublicCallback,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .expect("policy validates");
    IngressRouteDescriptor::new(
        TELEGRAM_UPDATES_ROUTE_ID,
        NetworkMethod::Post,
        TELEGRAM_UPDATES_PATH,
        policy,
    )
    .expect("descriptor validates")
}

#[derive(Clone)]
struct FakeTelegramDispatcher {
    verify_result: Result<ProtocolAuthEvidence, RunnerError>,
    dispatch_result: Result<WebhookProcessOutcome, RunnerError>,
    dispatch_calls: Arc<AtomicUsize>,
}

impl FakeTelegramDispatcher {
    fn verified() -> Self {
        Self {
            verify_result: Ok(mark_shared_secret_header_verified(
                TELEGRAM_SECRET_TOKEN_HEADER,
                "tg-bot-4242",
            )),
            dispatch_result: Ok(WebhookProcessOutcome::AcceptedForAsyncDispatch),
            dispatch_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn auth_failure() -> Self {
        Self {
            verify_result: Err(RunnerError::AuthenticationFailed {
                failure: ProtocolAuthFailure::Missing,
            }),
            ..Self::verified()
        }
    }

    fn at_capacity() -> Self {
        Self {
            dispatch_result: Err(RunnerError::TooManyInFlight { max_in_flight: 1 }),
            ..Self::verified()
        }
    }

    fn workflow_timeout() -> Self {
        Self {
            dispatch_result: Err(RunnerError::WorkflowTimeout {
                timeout: Duration::from_secs(1),
            }),
            ..Self::verified()
        }
    }
}

impl TelegramUpdatesWebhookDispatcher for FakeTelegramDispatcher {
    fn verify_webhook_auth(
        &self,
        _headers: &HeaderMap,
        _body: &[u8],
    ) -> Result<ProtocolAuthEvidence, RunnerError> {
        self.verify_result.clone()
    }

    fn process_verified_update<'a>(
        &'a self,
        _body: &'a [u8],
        _evidence: &'a ProtocolAuthEvidence,
        _observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
    ) -> Pin<Box<dyn Future<Output = Result<WebhookProcessOutcome, RunnerError>> + Send + 'a>> {
        self.dispatch_calls.fetch_add(1, Ordering::SeqCst);
        let result = self.dispatch_result.clone();
        Box::pin(async move { result })
    }

    fn drain_immediate_ack_tasks<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }
}

async fn post_to_state(
    state: &TelegramUpdatesRouteState,
    body: String,
    headers: Vec<(&'static str, String)>,
) -> Response {
    let (router, _descriptors) = telegram_updates_route_parts(state.clone());
    let mut builder = Request::builder().method("POST").uri(TELEGRAM_UPDATES_PATH);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    router
        .oneshot(
            builder
                .body(Body::from(body))
                .expect("request should build"),
        )
        .await
        .expect("router should respond")
}

async fn post_with_fake_dispatcher(
    dispatcher: FakeTelegramDispatcher,
    body: &'static str,
) -> Response {
    let headers = HeaderMap::new();
    let evidence = match dispatcher.verify_webhook_auth(&headers, body.as_bytes()) {
        Ok(evidence) => evidence,
        Err(error) => return ingress_error_response(TelegramIngressError::Runner(error)),
    };
    match dispatcher
        .process_verified_update(body.as_bytes(), &evidence, None)
        .await
    {
        Ok(_) => (StatusCode::OK, "ok").into_response(),
        Err(error) => runner_error_response(error),
    }
}

async fn assert_error_body(response: Response, expected: &str) {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body should collect")
        .to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json error body");
    assert_eq!(body["error"], expected);
}

/// Observer stub the fake revision builder attaches so the dispatch path
/// exercises the per-revision `Some(observer)` shape end to end (which
/// revision handled an update is asserted via the per-installation
/// counting workflows).
struct NoopObserver;

#[async_trait::async_trait]
impl ImmediateAckWorkflowObserver for NoopObserver {
    async fn observe_workflow_ack(
        &self,
        _envelope: ironclaw_product_adapters::ProductInboundEnvelope,
        _ack: ironclaw_product_adapters::ProductInboundAck,
    ) {
    }
}

/// Revision-builder fake: hands out one `CountingWorkflow` PER
/// INSTALLATION ID so tests can assert which setup revision's workflow
/// (and observer) handled a given update after a bot swap.
#[derive(Default)]
struct FakeRevisionWorkflowBuilder {
    counters: StdMutex<HashMap<String, Arc<AtomicUsize>>>,
    next_build_barriers: StdMutex<Option<(Arc<StdBarrier>, Arc<StdBarrier>)>>,
    builds: AtomicUsize,
}

impl FakeRevisionWorkflowBuilder {
    fn counter_for_installation(&self, installation_id: &str) -> Arc<AtomicUsize> {
        Arc::clone(
            self.counters
                .lock()
                .expect("lock")
                .entry(installation_id.to_string())
                .or_default(),
        )
    }

    fn builds(&self) -> usize {
        self.builds.load(Ordering::SeqCst)
    }

    fn hold_next_build(&self, entered: Arc<StdBarrier>, release: Arc<StdBarrier>) {
        *self.next_build_barriers.lock().expect("build barrier lock") = Some((entered, release));
    }
}

impl TelegramRevisionWorkflowBuilder for FakeRevisionWorkflowBuilder {
    fn build_revision_workflow(
        &self,
        setup: &TelegramInstallationSetup,
    ) -> Result<TelegramRevisionWorkflow, TelegramRevisionWorkflowBuildError> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        let barriers = self
            .next_build_barriers
            .lock()
            .expect("build barrier lock")
            .take();
        if let Some((entered, release)) = barriers {
            entered.wait();
            release.wait();
        }
        let installation_id = setup
            .installation_id()
            .map_err(|error| TelegramRevisionWorkflowBuildError::new(error.to_string()))?;
        let counter = self.counter_for_installation(installation_id.as_str());
        Ok(TelegramRevisionWorkflow {
            workflow: Arc::new(CountingWorkflow::new(counter)),
            workflow_observer: Some(Arc::new(NoopObserver)),
        })
    }
}

struct DynamicFixture {
    state: TelegramUpdatesRouteState,
    webhook_secret: Option<String>,
    submitted: Arc<AtomicUsize>,
    bot_api: Arc<RecordingBotApi>,
    lookup: Arc<FakeIdentityLookup>,
    setup: Arc<TelegramSetupService>,
    revision_workflows: Arc<FakeRevisionWorkflowBuilder>,
}

async fn dynamic_fixture(configured: bool) -> DynamicFixture {
    let bot_api = Arc::new(RecordingBotApi::default());
    let setup = if configured {
        configured_setup_service(bot_api.clone()).await
    } else {
        unconfigured_setup_service(bot_api.clone())
    };
    let webhook_secret = if configured {
        Some(
            setup
                .webhook_secret()
                .await
                .expect("secret read")
                .expect("secret present")
                .expose_secret()
                .to_string(),
        )
    } else {
        None
    };
    let pairing = pairing_service_with(Arc::clone(&setup));
    let lookup = Arc::new(FakeIdentityLookup::default());
    let revision_workflows = Arc::new(FakeRevisionWorkflowBuilder::default());
    // The pre-swap deployment bot's workflow counter (existing tests
    // assert against the fixture bot `tg-bot-4242`).
    let submitted = revision_workflows.counter_for_installation(fixture_installation_id().as_str());
    let resolver = Arc::new(DynamicTelegramInstallationResolver::new(
        Arc::clone(&setup),
        pairing,
        lookup.clone() as Arc<dyn RebornUserIdentityLookup>,
        Arc::clone(&revision_workflows) as Arc<dyn TelegramRevisionWorkflowBuilder>,
    ));
    DynamicFixture {
        state: TelegramUpdatesRouteState::from_resolver(resolver),
        webhook_secret,
        submitted,
        bot_api,
        lookup,
        setup,
        revision_workflows,
    }
}

async fn current_webhook_secret(setup: &TelegramSetupService) -> String {
    setup
        .webhook_secret()
        .await
        .expect("secret read")
        .expect("secret present")
        .expose_secret()
        .to_string()
}

fn bind_paired_sender(
    lookup: &FakeIdentityLookup,
    installation_id: &AdapterInstallationId,
    telegram_user_id: &str,
    user: &str,
) {
    lookup.bind(
        TELEGRAM_IDENTITY_PROVIDER,
        &telegram_user_identity_provider_user_id(installation_id, telegram_user_id),
        user,
    );
}

#[tokio::test]
async fn telegram_updates_handler_returns_401_when_unconfigured() {
    let fixture = dynamic_fixture(false).await;
    let response =
        post_to_state(&fixture.state, r#"{"update_id":1}"#.to_string(), Vec::new()).await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_error_body(response, "authentication").await;
}

/// A setup-store outage is an availability fault, not an authentication
/// decision: the webhook answers a retryable 503 (Telegram redelivers
/// once the store recovers). Only `Ok(None)` — genuinely unconfigured —
/// is the 401 shape.
#[tokio::test]
async fn telegram_updates_handler_returns_503_when_setup_store_is_down() {
    let (state, filesystem) = fault_injected_telegram_state();
    filesystem.fail_reads();
    let setup = unconfigured_setup_service_with_state(Arc::new(RecordingBotApi::default()), state);
    let pairing = pairing_service_with(Arc::clone(&setup));
    let resolver = Arc::new(DynamicTelegramInstallationResolver::new(
        setup,
        pairing,
        Arc::new(FakeIdentityLookup::default()) as Arc<dyn RebornUserIdentityLookup>,
        Arc::new(FakeRevisionWorkflowBuilder::default())
            as Arc<dyn TelegramRevisionWorkflowBuilder>,
    ));
    let state = TelegramUpdatesRouteState::from_resolver(resolver);

    let response = post_to_state(&state, r#"{"update_id":1}"#.to_string(), Vec::new()).await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_error_body(response, "temporarily_unavailable").await;
}

#[tokio::test]
async fn telegram_updates_handler_returns_401_on_missing_secret_header() {
    let fixture = dynamic_fixture(true).await;
    let response =
        post_to_state(&fixture.state, r#"{"update_id":1}"#.to_string(), Vec::new()).await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_error_body(response, "authentication").await;
    assert_eq!(fixture.submitted.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn telegram_updates_handler_returns_401_on_wrong_secret_header() {
    let fixture = dynamic_fixture(true).await;
    let response = post_to_state(
        &fixture.state,
        r#"{"update_id":1}"#.to_string(),
        vec![(TELEGRAM_SECRET_TOKEN_HEADER, "wrong-secret".to_string())],
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_error_body(response, "authentication").await;
    assert_eq!(fixture.submitted.load(Ordering::SeqCst), 0);
}

/// qa-telegram:S5 — malformed webhook JSON fails safely: no turn, no
/// reply, no body echo. The shipped classification for a VERIFIED but
/// unparseable body is a deliberate silent 200 ack (a permanently
/// unparseable update would otherwise be redelivered by Telegram
/// forever); the catalog row drafts a 4xx here — divergence recorded in
/// docs/qa/telegram-coverage-map.md for owner adjudication. Unverified
/// malformed bodies never get this far (the 401 tests above).
#[tokio::test]
async fn telegram_updates_handler_acks_malformed_json_without_turn_or_reply() {
    let fixture = dynamic_fixture(true).await;
    let secret = fixture.webhook_secret.clone().expect("configured secret");
    let response = post_to_state(
        &fixture.state,
        "{not-json%%".to_string(),
        vec![(TELEGRAM_SECRET_TOKEN_HEADER, secret)],
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "verified-but-unparseable updates are acked so Telegram stops redelivering"
    );
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .expect("ack body");
    assert_eq!(
        body.as_ref(),
        b"ok",
        "the ack is the fixed acknowledgment — malformed input is never echoed"
    );
    fixture.state.drain_immediate_ack_tasks().await;
    assert_eq!(
        fixture.submitted.load(Ordering::SeqCst),
        0,
        "malformed payloads must never start a turn"
    );
    assert!(
        fixture.bot_api.sends().is_empty(),
        "malformed payloads must never trigger replies"
    );
}

/// qa-slack:E17 (telegram leg) — a Slack-shaped payload cannot dispatch
/// through Telegram ingress even with telegram's own valid secret header:
/// it parses to no Telegram update, so no turn and no reply exist for it.
#[tokio::test]
async fn telegram_updates_handler_rejects_foreign_channel_payload_without_turn() {
    let fixture = dynamic_fixture(true).await;
    let secret = fixture.webhook_secret.clone().expect("configured secret");
    let slack_shaped = r#"{"token":"deprecated","team_id":"T123","api_app_id":"A1","event":{"type":"message","channel":"D123","user":"U1","text":"hi","ts":"1.2"},"type":"event_callback"}"#;
    let response = post_to_state(
        &fixture.state,
        slack_shaped.to_string(),
        vec![(TELEGRAM_SECRET_TOKEN_HEADER, secret)],
    )
    .await;

    // A foreign payload is VALID JSON with none of Telegram's update
    // fields: it classifies as non-actionable — acked so the sender stops
    // retrying — and produces neither a turn nor a reply.
    assert_eq!(response.status(), StatusCode::OK);
    fixture.state.drain_immediate_ack_tasks().await;
    assert_eq!(
        fixture.submitted.load(Ordering::SeqCst),
        0,
        "foreign-channel payloads must never start a turn"
    );
    assert!(
        fixture.bot_api.sends().is_empty(),
        "foreign-channel payloads must never trigger replies"
    );
}

#[tokio::test]
async fn telegram_updates_handler_acks_non_actionable_update_with_valid_secret() {
    let fixture = dynamic_fixture(true).await;
    let secret = fixture.webhook_secret.clone().expect("configured secret");
    let response = post_to_state(
        &fixture.state,
        r#"{"update_id":9}"#.to_string(),
        vec![(TELEGRAM_SECRET_TOKEN_HEADER, secret)],
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    fixture.state.drain_immediate_ack_tasks().await;
    assert_eq!(fixture.submitted.load(Ordering::SeqCst), 0);
    assert!(
        fixture.bot_api.sends().is_empty(),
        "silently-handled updates must not trigger replies"
    );
}

#[tokio::test]
async fn telegram_updates_handler_forwards_paired_sender_through_native_runner() {
    let fixture = dynamic_fixture(true).await;
    let secret = fixture.webhook_secret.clone().expect("configured secret");
    let installation_id = AdapterInstallationId::new("tg-bot-4242").expect("valid installation");
    fixture.lookup.bind(
        TELEGRAM_IDENTITY_PROVIDER,
        &telegram_user_identity_provider_user_id(&installation_id, "42"),
        "ben",
    );

    let body = private_text_update_body(42, 555, Some("hello ironclaw"));
    let response = post_to_state(
        &fixture.state,
        String::from_utf8(body).expect("utf8 body"),
        vec![(TELEGRAM_SECRET_TOKEN_HEADER, secret)],
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body should collect")
        .to_bytes();
    assert_eq!(&bytes[..], b"ok");
    fixture.state.drain_immediate_ack_tasks().await;
    assert_eq!(
        fixture.submitted.load(Ordering::SeqCst),
        1,
        "paired ordinary text must reach the workflow through the native runner"
    );
    assert!(
        fixture.bot_api.sends().is_empty(),
        "paired traffic must not trigger static replies"
    );
}

/// FIX-A regression, first-configure half: the workflow/observer used to
/// be assembled once at mount-build time from the boot-time setup, so
/// configuring the bot after boot required a process restart. The
/// resolver now builds workflow + observer per setup revision: boot
/// unconfigured (401), save a setup through the setup service, and —
/// WITHOUT rebuilding the route state — a verified webhook from a paired
/// sender dispatches into that revision's workflow.
#[tokio::test]
async fn telegram_updates_dispatch_after_first_configure_without_rebuild() {
    let fixture = dynamic_fixture(false).await;

    let unconfigured =
        post_to_state(&fixture.state, r#"{"update_id":1}"#.to_string(), Vec::new()).await;
    assert_eq!(unconfigured.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        fixture.revision_workflows.builds(),
        0,
        "no setup record must mean no workflow assembly"
    );

    fixture
        .setup
        .save_with_previous(TelegramInstallationSetupUpdate {
            bot_token: Some(SecretString::from("123:abc".to_string())),
            webhook_url_override: None,
        })
        .await
        .expect("first configure saves");
    let secret = current_webhook_secret(&fixture.setup).await;
    bind_paired_sender(&fixture.lookup, &fixture_installation_id(), "42", "ben");

    let body = private_text_update_body(42, 555, Some("hello after configure"));
    let response = post_to_state(
        &fixture.state,
        String::from_utf8(body).expect("utf8 body"),
        vec![(TELEGRAM_SECRET_TOKEN_HEADER, secret)],
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    fixture.state.drain_immediate_ack_tasks().await;
    assert_eq!(
        fixture.submitted.load(Ordering::SeqCst),
        1,
        "first-configure-after-boot must dispatch to the workflow without a rebuild"
    );
    assert_eq!(fixture.revision_workflows.builds(), 1);
}

/// FIX-A regression, bot-swap half: rotating the deployment to a
/// DIFFERENT bot re-keys the webhook verifier AND the workflow/observer
/// pair. The old installation's secret is rejected, the new secret
/// parses, and the update dispatches to the NEW revision's workflow
/// (asserted via per-installation counting workflows).
#[tokio::test]
async fn telegram_updates_bot_swap_rekeys_workflow_and_rejects_old_secret() {
    let fixture = dynamic_fixture(true).await;
    let old_secret = fixture.webhook_secret.clone().expect("configured secret");
    bind_paired_sender(&fixture.lookup, &fixture_installation_id(), "42", "ben");

    let before = post_to_state(
        &fixture.state,
        String::from_utf8(private_text_update_body(42, 555, Some("before swap")))
            .expect("utf8 body"),
        vec![(TELEGRAM_SECRET_TOKEN_HEADER, old_secret.clone())],
    )
    .await;
    assert_eq!(before.status(), StatusCode::OK);
    fixture.state.drain_immediate_ack_tasks().await;
    assert_eq!(fixture.submitted.load(Ordering::SeqCst), 1);

    // Swap the deployment to a different bot: new installation id
    // `tg-bot-7777`, fresh webhook secret, revision 2.
    fixture.bot_api.set_bot_identity(7777, "other_qa_bot");
    fixture
        .setup
        .save_with_previous(TelegramInstallationSetupUpdate {
            bot_token: Some(SecretString::from("777:xyz".to_string())),
            webhook_url_override: None,
        })
        .await
        .expect("bot swap saves");
    let new_secret = current_webhook_secret(&fixture.setup).await;
    assert_ne!(
        old_secret, new_secret,
        "rotation must mint a fresh webhook secret"
    );

    let stale = post_to_state(
        &fixture.state,
        String::from_utf8(private_text_update_body(42, 555, Some("stale secret")))
            .expect("utf8 body"),
        vec![(TELEGRAM_SECRET_TOKEN_HEADER, old_secret)],
    )
    .await;
    assert_eq!(
        stale.status(),
        StatusCode::UNAUTHORIZED,
        "the old installation's webhook secret must be rejected after the swap"
    );

    let new_installation = AdapterInstallationId::new("tg-bot-7777").expect("valid id");
    bind_paired_sender(&fixture.lookup, &new_installation, "42", "ben");
    let swapped = post_to_state(
        &fixture.state,
        String::from_utf8(private_text_update_body(42, 555, Some("after swap")))
            .expect("utf8 body"),
        vec![(TELEGRAM_SECRET_TOKEN_HEADER, new_secret)],
    )
    .await;
    assert_eq!(swapped.status(), StatusCode::OK);
    fixture.state.drain_immediate_ack_tasks().await;

    let new_counter = fixture
        .revision_workflows
        .counter_for_installation(new_installation.as_str());
    assert_eq!(
        new_counter.load(Ordering::SeqCst),
        1,
        "post-swap update must dispatch to the NEW revision's workflow"
    );
    assert_eq!(
        fixture.submitted.load(Ordering::SeqCst),
        1,
        "the old revision's workflow must not receive post-swap updates"
    );
    assert_eq!(
        fixture.revision_workflows.builds(),
        2,
        "one workflow assembly per setup revision"
    );
}

/// A revision-N workflow build can be slower than an admin bot swap. The
/// completed stale build must be discarded: it may neither replace revision
/// N+1 nor combine N's installation identity with N+1's webhook secret.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_bot_swap_discards_stale_revision_build_atomically() {
    let fixture = dynamic_fixture(true).await;
    let old_secret = fixture.webhook_secret.clone().expect("configured secret");
    let entered = Arc::new(StdBarrier::new(2));
    let release = Arc::new(StdBarrier::new(2));
    fixture
        .revision_workflows
        .hold_next_build(Arc::clone(&entered), Arc::clone(&release));

    let stale_state = fixture.state.clone();
    let stale_request = tokio::spawn(async move {
        post_to_state(
            &stale_state,
            String::from_utf8(private_text_update_body(42, 555, Some("stale build")))
                .expect("utf8 body"),
            vec![(TELEGRAM_SECRET_TOKEN_HEADER, old_secret)],
        )
        .await
    });
    tokio::task::spawn_blocking(move || entered.wait())
        .await
        .expect("stale build reaches barrier");

    fixture.bot_api.set_bot_identity(7777, "other_qa_bot");
    fixture
        .setup
        .save_with_previous(TelegramInstallationSetupUpdate {
            bot_token: Some(SecretString::from("777:xyz".to_string())),
            webhook_url_override: None,
        })
        .await
        .expect("bot swap saves while old build is paused");
    let new_secret = current_webhook_secret(&fixture.setup).await;
    tokio::task::spawn_blocking(move || release.wait())
        .await
        .expect("stale build releases");

    let stale_response = stale_request.await.expect("stale request joins");
    assert_eq!(
        stale_response.status(),
        StatusCode::UNAUTHORIZED,
        "the old secret must be checked against the new atomic revision after stale build discard"
    );

    let new_installation = AdapterInstallationId::new("tg-bot-7777").expect("valid id");
    bind_paired_sender(&fixture.lookup, &new_installation, "42", "ben");
    let current_response = post_to_state(
        &fixture.state,
        String::from_utf8(private_text_update_body(42, 555, Some("current build")))
            .expect("utf8 body"),
        vec![(TELEGRAM_SECRET_TOKEN_HEADER, new_secret)],
    )
    .await;
    assert_eq!(current_response.status(), StatusCode::OK);
    fixture.state.drain_immediate_ack_tasks().await;
    assert_eq!(
        fixture
            .revision_workflows
            .counter_for_installation(new_installation.as_str())
            .load(Ordering::SeqCst),
        1,
        "only the current installation workflow may receive the post-swap update"
    );
    assert_eq!(
        fixture.submitted.load(Ordering::SeqCst),
        0,
        "the stale workflow must never receive a verified update"
    );
}

#[tokio::test]
async fn telegram_updates_handler_returns_401_on_auth_failure() {
    let dispatcher = FakeTelegramDispatcher::auth_failure();
    let calls = Arc::clone(&dispatcher.dispatch_calls);
    let response = post_with_fake_dispatcher(dispatcher, r#"{"update_id":1}"#).await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_error_body(response, "authentication").await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn telegram_updates_handler_returns_ok_on_successful_dispatch() {
    let dispatcher = FakeTelegramDispatcher::verified();
    let calls = Arc::clone(&dispatcher.dispatch_calls);
    let response = post_with_fake_dispatcher(dispatcher, r#"{"update_id":1}"#).await;

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body should collect")
        .to_bytes();
    assert_eq!(&bytes[..], b"ok");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn telegram_updates_handler_returns_429_when_at_capacity() {
    let dispatcher = FakeTelegramDispatcher::at_capacity();
    let calls = Arc::clone(&dispatcher.dispatch_calls);
    let response = post_with_fake_dispatcher(dispatcher, r#"{"update_id":1}"#).await;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_error_body(response, "capacity").await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn telegram_updates_handler_returns_503_on_workflow_timeout() {
    let dispatcher = FakeTelegramDispatcher::workflow_timeout();
    let calls = Arc::clone(&dispatcher.dispatch_calls);
    let response = post_with_fake_dispatcher(dispatcher, r#"{"update_id":1}"#).await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_error_body(response, "temporarily_unavailable").await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn telegram_updates_handler_rate_limits_per_installation() {
    let bot_api = Arc::new(RecordingBotApi::default());
    let setup = configured_setup_service(bot_api).await;
    let webhook_secret = current_webhook_secret(&setup).await;
    let pairing = pairing_service_with(Arc::clone(&setup));
    let resolver = Arc::new(DynamicTelegramInstallationResolver::new(
        setup,
        pairing,
        Arc::new(FakeIdentityLookup::default()) as Arc<dyn RebornUserIdentityLookup>,
        Arc::new(FakeRevisionWorkflowBuilder::default())
            as Arc<dyn TelegramRevisionWorkflowBuilder>,
    ));
    let state = TelegramUpdatesRouteState::new(TelegramIngressService::with_rate_limit_config(
        resolver,
        ironclaw_channel_host::host_ingress::InstallationRateLimitConfig::new(
            NonZeroU32::new(1).expect("nonzero"),
            Duration::from_secs(60),
        ),
    ));

    let first = post_to_state(
        &state,
        r#"{"update_id":1}"#.to_string(),
        vec![(TELEGRAM_SECRET_TOKEN_HEADER, webhook_secret.clone())],
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);

    let second = post_to_state(
        &state,
        r#"{"update_id":2}"#.to_string(),
        vec![(TELEGRAM_SECRET_TOKEN_HEADER, webhook_secret)],
    )
    .await;
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_error_body(second, "capacity").await;
}

#[test]
fn telegram_updates_route_descriptor_matches_manifest_projection() {
    assert_eq!(
        telegram_updates_route_descriptors(),
        vec![expected_telegram_descriptor()]
    );
    // The serve-layer path (aliasing the setup pipeline's `setWebhook`
    // path) and the manifest-projected route pattern must be one value.
    assert_eq!(
        TELEGRAM_UPDATES_PATH,
        "/webhooks/extensions/telegram/updates"
    );
    assert_eq!(
        telegram_updates_route_descriptors()[0]
            .route_pattern()
            .as_str(),
        TELEGRAM_UPDATES_PATH
    );
}
