use std::future::Future;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use http_body_util::BodyExt;
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_product_adapters::auth::mark_shared_secret_header_verified;
use ironclaw_product_adapters::identity::AdapterInstallationId;
use ironclaw_product_adapters::{ProtocolAuthEvidence, ProtocolAuthFailure};
use ironclaw_wasm_product_adapters::{RunnerError, WebhookProcessOutcome};
use tower::ServiceExt;

use super::*;
use crate::slack_actor_identity::{RebornUserIdentityLookup, RebornUserIdentityLookupError};
use crate::slack_personal_binding::{
    RebornUserIdentityBinding, RebornUserIdentityBindingError, RebornUserIdentityBindingStore,
    SlackPersonalUserBindingService,
};
use crate::slack_personal_binding_pairing::{
    IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingChallenge,
    SlackPersonalBindingPairingChallengeStore, SlackPersonalBindingPairingCode,
    SlackPersonalBindingPairingError, SlackPersonalBindingPairingNotification,
    SlackPersonalBindingPairingNotifier, SlackPersonalBindingPairingService,
};

struct HeaderSecretDispatcher {
    expected_secret: &'static str,
    subject: &'static str,
    dispatch_calls: Arc<AtomicUsize>,
}

impl HeaderSecretDispatcher {
    fn new(expected_secret: &'static str, subject: &'static str) -> Self {
        Self {
            expected_secret,
            subject,
            dispatch_calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl SlackEventsWebhookDispatcher for HeaderSecretDispatcher {
    fn verify_webhook_auth(
        &self,
        headers: &HeaderMap,
        _body: &[u8],
    ) -> Result<ProtocolAuthEvidence, RunnerError> {
        if headers
            .get("X-Test-Secret")
            .and_then(|value| value.to_str().ok())
            == Some(self.expected_secret)
        {
            return Ok(mark_shared_secret_header_verified(
                "X-Test-Secret",
                self.subject,
            ));
        }
        Err(RunnerError::AuthenticationFailed {
            failure: ProtocolAuthFailure::SignatureMismatch,
        })
    }

    fn process_verified_webhook_immediate_ack<'a>(
        &'a self,
        _body: &'a [u8],
        _evidence: &'a ProtocolAuthEvidence,
        _observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
    ) -> Pin<Box<dyn Future<Output = Result<WebhookProcessOutcome, RunnerError>> + Send + 'a>> {
        self.dispatch_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(WebhookProcessOutcome::AcceptedForAsyncDispatch) })
    }

    fn drain_immediate_ack_tasks<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }
}

fn tenant_id(value: &str) -> TenantId {
    TenantId::new(value).expect("valid tenant") // safety: test helper only passes hard-coded valid tenant identifiers.
}

fn installation_id(value: &str) -> AdapterInstallationId {
    AdapterInstallationId::new(value).expect("valid installation") // safety: test helper only passes hard-coded valid installation identifiers.
}

async fn post_to_mount(
    mount: &PublicRouteMount,
    body: &'static str,
    secret: &'static str,
) -> Response {
    mount
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(SLACK_EVENTS_PATH)
                .header("X-Test-Secret", secret)
                .body(Body::from(body))
                .expect("request should build"), // safety: test builds a valid fixed POST request.
        )
        .await
        .expect("router should respond") // safety: axum test router should produce a response for fixed route input.
}

async fn assert_error_body(response: Response, expected: &str) {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body should collect") // safety: test response bodies are small and fully buffered.
        .to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json error body"); // safety: error responses are generated as JSON by this route.
    assert_eq!(body["error"], expected); // safety: assertion is in a test-only helper.
}

const TEAM_A_BODY: &str = r#"{
    "type": "event_callback",
    "team_id": "T-A",
    "api_app_id": "A-slack",
    "event_id": "Ev-A",
    "event": {
        "type": "message",
        "channel_type": "im",
        "user": "U123",
        "channel": "D-A",
        "text": "hello from A",
        "ts": "1710000000.000001"
    }
}"#;

#[tokio::test]
async fn slack_events_handler_rejects_malformed_event_envelope_before_dispatch() {
    let dispatcher = Arc::new(HeaderSecretDispatcher::new("secret-a", "install-a"));
    let resolver = StaticSlackInstallationResolver::new(vec![SlackInstallationRecord::new(
        tenant_id("tenant-a"),
        installation_id("install-a"),
        SlackInstallationSelector::team("T-A"),
        dispatcher.clone(),
    )]);
    let mount = slack_events_route_mount(SlackEventsRouteState::new(SlackIngressService::new(
        Arc::new(resolver),
    )));

    let response = post_to_mount(&mount, "{", "secret-a").await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_error_body(response, "malformed_payload").await;
    assert_eq!(dispatcher.dispatch_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn slack_events_handler_rejects_missing_installation_without_dispatch() {
    let dispatcher = Arc::new(HeaderSecretDispatcher::new("secret-a", "install-a"));
    let resolver = StaticSlackInstallationResolver::new(vec![SlackInstallationRecord::new(
        tenant_id("tenant-a"),
        installation_id("install-a"),
        SlackInstallationSelector::team("T-A"),
        dispatcher.clone(),
    )]);
    let mount = slack_events_route_mount(SlackEventsRouteState::new(SlackIngressService::new(
        Arc::new(resolver),
    )));
    let unknown_team_body =
        r#"{"type":"event_callback","team_id":"T-unknown","event":{"type":"message"}}"#;

    let response = post_to_mount(&mount, unknown_team_body, "secret-a").await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_error_body(response, "authentication").await;
    assert_eq!(dispatcher.dispatch_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn slack_events_handler_rejects_ambiguous_installation_without_dispatch() {
    let dispatcher_a = Arc::new(HeaderSecretDispatcher::new("shared-secret", "install-a"));
    let dispatcher_b = Arc::new(HeaderSecretDispatcher::new("shared-secret", "install-b"));
    let resolver = StaticSlackInstallationResolver::new(vec![
        SlackInstallationRecord::new(
            tenant_id("tenant-a"),
            installation_id("install-a"),
            SlackInstallationSelector::team("T-A"),
            dispatcher_a.clone(),
        ),
        SlackInstallationRecord::new(
            tenant_id("tenant-b"),
            installation_id("install-b"),
            SlackInstallationSelector::team("T-A"),
            dispatcher_b.clone(),
        ),
    ]);
    let mount = slack_events_route_mount(SlackEventsRouteState::new(SlackIngressService::new(
        Arc::new(resolver),
    )));

    let response = post_to_mount(&mount, TEAM_A_BODY, "shared-secret").await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_error_body(response, "authentication").await;
    assert_eq!(dispatcher_a.dispatch_calls.load(Ordering::SeqCst), 0);
    assert_eq!(dispatcher_b.dispatch_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn slack_events_handler_rate_limit_refills_after_window() {
    let dispatcher = Arc::new(HeaderSecretDispatcher::new("secret-a", "install-a"));
    let resolver = StaticSlackInstallationResolver::new(vec![SlackInstallationRecord::new(
        tenant_id("tenant-a"),
        installation_id("install-a"),
        SlackInstallationSelector::team("T-A"),
        dispatcher.clone(),
    )]);
    let rate_limit = SlackInstallationRateLimitConfig::new(
        NonZeroU32::new(1).expect("nonzero"),
        Duration::from_millis(50),
    );
    let mount = slack_events_route_mount(SlackEventsRouteState::new(
        SlackIngressService::with_rate_limit_config(Arc::new(resolver), rate_limit),
    ));

    let first = post_to_mount(&mount, TEAM_A_BODY, "secret-a").await;
    let second = post_to_mount(&mount, TEAM_A_BODY, "secret-a").await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    let third = post_to_mount(&mount, TEAM_A_BODY, "secret-a").await;

    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_error_body(second, "capacity").await;
    assert_eq!(third.status(), StatusCode::OK);
    assert_eq!(dispatcher.dispatch_calls.load(Ordering::SeqCst), 2);
}

// ----- Task 5: `/pair` slash-command handler -----

/// In-memory force-mint challenge store. Each (re)issue mints a brand-new
/// code and makes it the only live one, mirroring the production reissue
/// contract: any prior code is invalidated.
struct FakeReissueStore {
    next: AtomicUsize,
    live: std::sync::Mutex<Option<IssuedSlackPersonalBindingPairingChallenge>>,
}

impl FakeReissueStore {
    fn new() -> Self {
        Self {
            next: AtomicUsize::new(1),
            live: std::sync::Mutex::new(None),
        }
    }

    fn mint(
        &self,
        challenge: SlackPersonalBindingPairingChallenge,
    ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        let serial = self.next.fetch_add(1, Ordering::SeqCst);
        let code = SlackPersonalBindingPairingCode::new(format!("PAIRCODE{serial:04}"))?;
        let issued = IssuedSlackPersonalBindingPairingChallenge { code, challenge };
        *self.live_guard() = Some(issued.clone());
        Ok(issued)
    }

    fn is_live(&self, code: &str) -> bool {
        self.live_guard()
            .as_ref()
            .is_some_and(|issued| issued.code.as_str() == code)
    }

    fn live_guard(
        &self,
    ) -> std::sync::MutexGuard<'_, Option<IssuedSlackPersonalBindingPairingChallenge>> {
        self.live
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[async_trait]
impl SlackPersonalBindingPairingChallengeStore for FakeReissueStore {
    async fn issue_challenge(
        &self,
        challenge: SlackPersonalBindingPairingChallenge,
    ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        self.mint(challenge)
    }

    async fn get_challenge(
        &self,
        code: &SlackPersonalBindingPairingCode,
    ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        let live = self.live_guard();
        match live.as_ref() {
            Some(issued) if &issued.code == code => Ok(issued.challenge.clone()),
            _ => Err(SlackPersonalBindingPairingError::ChallengeNotFound),
        }
    }

    async fn consume_challenge(
        &self,
        code: &SlackPersonalBindingPairingCode,
    ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        let mut live = self.live_guard();
        match live.take() {
            Some(issued) if &issued.code == code => Ok(issued.challenge),
            other => {
                *live = other;
                Err(SlackPersonalBindingPairingError::ChallengeNotFound)
            }
        }
    }

    async fn reissue_challenge(
        &self,
        challenge: SlackPersonalBindingPairingChallenge,
    ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        self.mint(challenge)
    }
}

/// A challenge store that always fails — drives the "temporarily unavailable"
/// ephemeral mapping.
struct FailingReissueStore;

#[async_trait]
impl SlackPersonalBindingPairingChallengeStore for FailingReissueStore {
    async fn issue_challenge(
        &self,
        _challenge: SlackPersonalBindingPairingChallenge,
    ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        Err(SlackPersonalBindingPairingError::Backend(
            "store offline".into(),
        ))
    }

    async fn get_challenge(
        &self,
        _code: &SlackPersonalBindingPairingCode,
    ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        Err(SlackPersonalBindingPairingError::ChallengeNotFound)
    }

    async fn consume_challenge(
        &self,
        _code: &SlackPersonalBindingPairingCode,
    ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        Err(SlackPersonalBindingPairingError::ChallengeNotFound)
    }

    async fn reissue_challenge(
        &self,
        _challenge: SlackPersonalBindingPairingChallenge,
    ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        Err(SlackPersonalBindingPairingError::Backend(
            "store offline".into(),
        ))
    }
}

struct NoopPairingNotifier;

#[async_trait]
impl SlackPersonalBindingPairingNotifier for NoopPairingNotifier {
    async fn send_pairing_challenge(
        &self,
        _notification: SlackPersonalBindingPairingNotification,
    ) -> Result<(), SlackPersonalBindingPairingError> {
        Ok(())
    }
}

struct NoopBindingStore;

#[async_trait]
impl RebornUserIdentityBindingStore for NoopBindingStore {
    async fn bind_user_identity(
        &self,
        _binding: RebornUserIdentityBinding,
    ) -> Result<(), RebornUserIdentityBindingError> {
        Ok(())
    }
}

/// Identity lookup that answers from a fixed map keyed by the exact
/// `provider_user_id` the handler queries — so a test fails if the handler
/// looks up the wrong (e.g. non-installation-scoped) key.
struct FakeIdentityLookup {
    bindings: std::collections::HashMap<String, UserId>,
}

impl FakeIdentityLookup {
    fn unlinked() -> Self {
        Self {
            bindings: std::collections::HashMap::new(),
        }
    }

    fn linked(provider_user_id: &str, user: UserId) -> Self {
        Self {
            bindings: std::collections::HashMap::from([(provider_user_id.to_string(), user)]),
        }
    }
}

#[async_trait]
impl RebornUserIdentityLookup for FakeIdentityLookup {
    async fn resolve_user_identity(
        &self,
        _provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
        Ok(self.bindings.get(provider_user_id).cloned())
    }

    async fn user_has_provider_binding(
        &self,
        _provider: &str,
        user_id: &UserId,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        Ok(self.bindings.values().any(|bound| bound == user_id))
    }
}

fn single_team_resolver() -> (
    Arc<HeaderSecretDispatcher>,
    Arc<StaticSlackInstallationResolver>,
) {
    let dispatcher = Arc::new(HeaderSecretDispatcher::new("secret-a", "install-a"));
    let resolver = Arc::new(StaticSlackInstallationResolver::new(vec![
        SlackInstallationRecord::new(
            tenant_id("tenant-a"),
            installation_id("install-a"),
            SlackInstallationSelector::team("T-A"),
            dispatcher.clone(),
        ),
    ]));
    (dispatcher, resolver)
}

fn pairing_service(
    store: Arc<dyn SlackPersonalBindingPairingChallengeStore>,
) -> SlackPersonalBindingPairingService {
    let binder = SlackPersonalUserBindingService::new([], Arc::new(NoopBindingStore));
    SlackPersonalBindingPairingService::new(binder, store, Arc::new(NoopPairingNotifier))
}

fn commands_mount(
    resolver: Arc<dyn SlackInstallationResolver>,
    pairing: SlackPersonalBindingPairingService,
    lookup: Arc<dyn RebornUserIdentityLookup>,
) -> PublicRouteMount {
    let state = SlackCommandsRouteState::new(SlackIngressService::new(resolver), pairing, lookup);
    slack_commands_route_mount(state)
}

fn slack_command_form(team: &str, user: &str, command: &str, response_url: &str) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .append_pair("token", "verification-token")
        .append_pair("team_id", team)
        .append_pair("api_app_id", "A-slack")
        .append_pair("channel_id", "D123")
        .append_pair("user_id", user)
        .append_pair("command", command)
        .append_pair("text", "")
        .append_pair("response_url", response_url)
        .append_pair("trigger_id", "123.456.abc")
        .finish()
}

async fn post_command(mount: &PublicRouteMount, body: String, secret: &str) -> Response {
    mount
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(SLACK_COMMANDS_PATH)
                .header("X-Test-Secret", secret)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .expect("request should build"), // safety: test builds a valid fixed POST request.
        )
        .await
        .expect("router should respond") // safety: axum test router should produce a response for fixed route input.
}

async fn ephemeral_text(response: Response) -> String {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body should collect") // safety: test response bodies are small and fully buffered.
        .to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json ephemeral body"); // safety: the slash handler always replies with JSON.
    assert_eq!(body["response_type"], "ephemeral"); // safety: assertion in a test-only helper.
    body["text"]
        .as_str()
        .expect("ephemeral text is a string") // safety: handler always sets a string `text`.
        .to_string()
}

#[tokio::test]
async fn slack_pair_command_returns_fresh_code_ephemerally() {
    let (_dispatcher, resolver) = single_team_resolver();
    let store = Arc::new(FakeReissueStore::new());
    let lookup = Arc::new(FakeIdentityLookup::unlinked());
    let mount = commands_mount(resolver, pairing_service(store.clone()), lookup);

    let response = post_command(
        &mount,
        slack_command_form(
            "T-A",
            "U123",
            "/pair",
            "https://hooks.slack.com/commands/T-A/1/a",
        ),
        "secret-a",
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let text = ephemeral_text(response).await;
    assert!(
        text.contains("PAIRCODE0001"),
        "ephemeral reply should carry the fresh code: {text}"
    );
    assert!(store.is_live("PAIRCODE0001"));
}

#[tokio::test]
async fn slack_pair_twice_returns_different_codes_and_invalidates_first() {
    let (_dispatcher, resolver) = single_team_resolver();
    let store = Arc::new(FakeReissueStore::new());
    let lookup = Arc::new(FakeIdentityLookup::unlinked());
    let mount = commands_mount(resolver, pairing_service(store.clone()), lookup);

    let first = post_command(
        &mount,
        slack_command_form(
            "T-A",
            "U123",
            "/pair",
            "https://hooks.slack.com/commands/T-A/1/a",
        ),
        "secret-a",
    )
    .await;
    let first_text = ephemeral_text(first).await;
    let second = post_command(
        &mount,
        slack_command_form(
            "T-A",
            "U123",
            "/pair",
            "https://hooks.slack.com/commands/T-A/2/b",
        ),
        "secret-a",
    )
    .await;
    let second_text = ephemeral_text(second).await;

    assert!(first_text.contains("PAIRCODE0001"), "{first_text}");
    assert!(second_text.contains("PAIRCODE0002"), "{second_text}");
    assert_ne!(first_text, second_text);
    // Only the most recent code remains live; the first is invalidated.
    assert!(!store.is_live("PAIRCODE0001"));
    assert!(store.is_live("PAIRCODE0002"));
}

#[tokio::test]
async fn slack_pair_already_linked_returns_connected_message() {
    let (_dispatcher, resolver) = single_team_resolver();
    let store = Arc::new(FakeReissueStore::new());
    // The binding is stored under the installation-scoped identity, so the
    // handler must look up "<installation_id>:<slack_user_id>" — not the bare
    // Slack user id. Seeding the composite key proves the handler builds it.
    let lookup = Arc::new(FakeIdentityLookup::linked(
        "install-a:U123",
        UserId::new("user-1").expect("valid user"),
    ));
    let mount = commands_mount(resolver, pairing_service(store.clone()), lookup);

    let response = post_command(
        &mount,
        slack_command_form(
            "T-A",
            "U123",
            "/pair",
            "https://hooks.slack.com/commands/T-A/1/a",
        ),
        "secret-a",
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let text = ephemeral_text(response).await;
    assert_eq!(text, "You're already connected.");
    // An already-linked user is never minted a code.
    assert!(!text.contains("PAIRCODE"));
    assert!(!store.is_live("PAIRCODE0001"));
}

#[tokio::test]
async fn slack_pair_bad_signature_returns_401() {
    let (_dispatcher, resolver) = single_team_resolver();
    let store = Arc::new(FakeReissueStore::new());
    let lookup = Arc::new(FakeIdentityLookup::unlinked());
    let mount = commands_mount(resolver, pairing_service(store.clone()), lookup);

    let response = post_command(
        &mount,
        slack_command_form(
            "T-A",
            "U123",
            "/pair",
            "https://hooks.slack.com/commands/T-A/1/a",
        ),
        "wrong-secret",
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_error_body(response, "authentication").await;
    // An unsigned request never mints a code.
    assert!(!store.is_live("PAIRCODE0001"));
}

#[tokio::test]
async fn slack_pair_service_error_returns_ephemeral_unavailable() {
    let (_dispatcher, resolver) = single_team_resolver();
    let lookup = Arc::new(FakeIdentityLookup::unlinked());
    let mount = commands_mount(
        resolver,
        pairing_service(Arc::new(FailingReissueStore)),
        lookup,
    );

    let response = post_command(
        &mount,
        slack_command_form(
            "T-A",
            "U123",
            "/pair",
            "https://hooks.slack.com/commands/T-A/1/a",
        ),
        "secret-a",
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let text = ephemeral_text(response).await;
    assert!(
        text.to_lowercase().contains("temporarily unavailable"),
        "service failure should map to a friendly ephemeral message: {text}"
    );
    assert!(!text.contains("PAIRCODE"));
}

#[tokio::test]
async fn slack_pair_unknown_command_returns_ephemeral_error() {
    let (_dispatcher, resolver) = single_team_resolver();
    let store = Arc::new(FakeReissueStore::new());
    let lookup = Arc::new(FakeIdentityLookup::unlinked());
    let mount = commands_mount(resolver, pairing_service(store.clone()), lookup);

    let response = post_command(
        &mount,
        slack_command_form(
            "T-A",
            "U123",
            "/unpair",
            "https://hooks.slack.com/commands/T-A/1/a",
        ),
        "secret-a",
    )
    .await;

    // A signed command other than `/pair` gets a friendly ephemeral, never a code.
    assert_eq!(response.status(), StatusCode::OK);
    let text = ephemeral_text(response).await;
    assert!(
        text.to_lowercase().contains("unknown command"),
        "a non-/pair command should be rejected: {text}"
    );
    assert!(!text.contains("PAIRCODE"));
    assert!(!store.is_live("PAIRCODE0001"));
}
