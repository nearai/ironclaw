use std::future::Future;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use http_body_util::BodyExt;
use ironclaw_host_api::TenantId;
use ironclaw_product_adapters::auth::mark_shared_secret_header_verified;
use ironclaw_product_adapters::identity::AdapterInstallationId;
use ironclaw_product_adapters::{ProtocolAuthEvidence, ProtocolAuthFailure};
use ironclaw_wasm_product_adapters::{RunnerError, WebhookProcessOutcome};
use tower::ServiceExt;

use super::*;

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
