//! Slack Events API route composition for the Reborn ProductAdapter path.
//!
//! This module exposes an axum route fragment plus ingress descriptors. It does
//! not bind listeners and does not reuse the legacy v1 Slack channel. The host
//! decides whether to mount this fragment (for example behind
//! `REBORN_SLACK_ENABLED`) and supplies a preconfigured native adapter runner.

use std::future::Future;
use std::num::{NonZeroU32, NonZeroU64};
use std::pin::Pin;
use std::sync::Arc;

use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor,
    IngressScopeSource, ListenerClass, RateLimitPolicy, RateLimitScope, StreamingMode,
    WebSocketOriginPolicy,
};
use ironclaw_product_adapters::ProtocolAuthEvidence;
use ironclaw_slack_v2_adapter::parse_slack_url_verification_challenge;
use ironclaw_wasm_product_adapters::{
    NativeProductAdapterRunner, RunnerError, WebhookProcessOutcome,
};
use serde::Serialize;

use crate::webui_serve::{PublicRouteDrain, PublicRouteMount};

pub const SLACK_EVENTS_PATH: &str = "/webhooks/slack/events";
const SLACK_EVENTS_ROUTE_ID: &str = "slack.events";
const SLACK_EVENTS_BODY_LIMIT_BYTES: NonZeroU64 = {
    // SAFETY: 1 MiB is a non-zero literal.
    unsafe { NonZeroU64::new_unchecked(1024 * 1024) }
};
const SLACK_EVENTS_MAX_REQUESTS: NonZeroU32 = {
    // SAFETY: 120 requests is a non-zero literal.
    unsafe { NonZeroU32::new_unchecked(120) }
};
const SLACK_EVENTS_RATE_WINDOW_SECONDS: NonZeroU32 = {
    // SAFETY: 60 seconds is a non-zero literal.
    unsafe { NonZeroU32::new_unchecked(60) }
};

pub trait SlackEventsWebhookDispatcher: Send + Sync {
    fn verify_webhook_auth(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<ProtocolAuthEvidence, RunnerError>;

    fn process_verified_webhook_immediate_ack<'a>(
        &'a self,
        body: &'a [u8],
        evidence: &'a ProtocolAuthEvidence,
    ) -> Pin<Box<dyn Future<Output = Result<WebhookProcessOutcome, RunnerError>> + Send + 'a>>;

    fn drain_immediate_ack_tasks<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

impl SlackEventsWebhookDispatcher for NativeProductAdapterRunner {
    fn verify_webhook_auth(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<ProtocolAuthEvidence, RunnerError> {
        NativeProductAdapterRunner::verify_webhook_auth(self, headers, body)
    }

    fn process_verified_webhook_immediate_ack<'a>(
        &'a self,
        body: &'a [u8],
        evidence: &'a ProtocolAuthEvidence,
    ) -> Pin<Box<dyn Future<Output = Result<WebhookProcessOutcome, RunnerError>> + Send + 'a>> {
        Box::pin(
            NativeProductAdapterRunner::process_verified_webhook_immediate_ack(
                self, body, evidence,
            ),
        )
    }

    fn drain_immediate_ack_tasks<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(NativeProductAdapterRunner::drain_immediate_ack_tasks(self))
    }
}

#[derive(Clone)]
pub struct SlackEventsRouteState {
    dispatcher: Arc<dyn SlackEventsWebhookDispatcher>,
}

impl SlackEventsRouteState {
    pub fn new(dispatcher: Arc<dyn SlackEventsWebhookDispatcher>) -> Self {
        Self { dispatcher }
    }

    pub async fn drain_immediate_ack_tasks(&self) {
        self.dispatcher.drain_immediate_ack_tasks().await;
    }
}

impl PublicRouteDrain for SlackEventsRouteState {
    fn drain<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(self.drain_immediate_ack_tasks())
    }
}

impl std::fmt::Debug for SlackEventsRouteState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlackEventsRouteState")
            .field("dispatcher", &"Arc<dyn SlackEventsWebhookDispatcher>")
            .finish()
    }
}

pub fn slack_events_route_mount(state: SlackEventsRouteState) -> PublicRouteMount {
    PublicRouteMount::new(
        Router::new()
            .route(SLACK_EVENTS_PATH, post(slack_events_handler))
            .with_state(state.clone()),
        slack_events_route_descriptors(),
    )
    .with_drain(Arc::new(state))
}

pub fn slack_events_route_descriptors() -> Vec<IngressRouteDescriptor> {
    let descriptor = IngressRouteDescriptor::new(
        SLACK_EVENTS_ROUTE_ID,
        NetworkMethod::Post,
        SLACK_EVENTS_PATH,
        slack_events_policy(),
    )
    .expect("Slack events route descriptor must validate at startup"); // safety: route id/path are crate-local literals and policy is built by sibling helper.
    vec![descriptor]
}

fn slack_events_policy() -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::PublicWebhook,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::WebhookSignature],
        },
        scope_source: IngressScopeSource::HostResolved,
        body_limit: BodyLimitPolicy::Limited {
            max_bytes: SLACK_EVENTS_BODY_LIMIT_BYTES,
        },
        rate_limit: RateLimitPolicy::Limited {
            // Interim pre-auth guard. The descriptor-driven limiter runs before
            // Slack signature verification, so it cannot yet bucket by verified
            // installation/workspace. `PerIp` is also not a safe substitute for
            // Slack because events arrive from shared Slack egress pools. A
            // verified-installation limiter belongs in a follow-up extension to
            // the ingress rate-limit model or a post-verification Slack-specific
            // limiter.
            scope: RateLimitScope::Global,
            max_requests: SLACK_EVENTS_MAX_REQUESTS,
            window_seconds: SLACK_EVENTS_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::NotApplicable,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::PublicCallback,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .expect("Slack events ingress policy must validate") // safety: policy combines validated constants and host-resolved webhook-signature scope.
}

async fn slack_events_handler(
    State(state): State<SlackEventsRouteState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let evidence = match state
        .dispatcher
        .verify_webhook_auth(&headers, body.as_ref())
    {
        Ok(evidence) => evidence,
        Err(error) => return runner_error_response(error),
    };

    match parse_slack_url_verification_challenge(body.as_ref(), &evidence) {
        Ok(Some(challenge)) => return (StatusCode::OK, challenge.challenge).into_response(),
        Ok(None) => {}
        Err(error) => {
            tracing::debug!(
                target = "ironclaw::reborn::slack_events",
                error = %error,
                "Slack URL verification parse failed"
            );
            return error_response(
                StatusCode::BAD_REQUEST,
                SlackWebhookErrorCategory::MalformedPayload,
            );
        }
    }

    match state
        .dispatcher
        .process_verified_webhook_immediate_ack(body.as_ref(), &evidence)
        .await
    {
        Ok(_) => (StatusCode::OK, "ok").into_response(),
        Err(error) => runner_error_response(error),
    }
}

fn runner_error_response(error: RunnerError) -> Response {
    let (status, category) = match &error {
        RunnerError::AuthenticationFailed { .. } => (
            StatusCode::UNAUTHORIZED,
            SlackWebhookErrorCategory::Authentication,
        ),
        RunnerError::TooManyInFlight { .. } => (
            StatusCode::TOO_MANY_REQUESTS,
            SlackWebhookErrorCategory::Capacity,
        ),
        RunnerError::Adapter(adapter_error) if adapter_error.is_retryable() => (
            StatusCode::SERVICE_UNAVAILABLE,
            SlackWebhookErrorCategory::TemporarilyUnavailable,
        ),
        RunnerError::WorkflowTimeout { .. }
        | RunnerError::WorkflowJoinFailed
        | RunnerError::WorkflowPanicked
        | RunnerError::AdapterPanicked => (
            StatusCode::SERVICE_UNAVAILABLE,
            SlackWebhookErrorCategory::TemporarilyUnavailable,
        ),
        RunnerError::Adapter(_) => (StatusCode::BAD_REQUEST, SlackWebhookErrorCategory::Adapter),
    };
    tracing::debug!(
        target = "ironclaw::reborn::slack_events",
        status = status.as_u16(),
        error = %error,
        "Slack Events API webhook rejected"
    );
    error_response(status, category)
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum SlackWebhookErrorCategory {
    Authentication,
    Capacity,
    MalformedPayload,
    Adapter,
    TemporarilyUnavailable,
}

#[derive(Debug, Serialize)]
struct SlackWebhookErrorBody {
    error: SlackWebhookErrorCategory,
}

fn error_response(status: StatusCode, category: SlackWebhookErrorCategory) -> Response {
    (status, Json(SlackWebhookErrorBody { error: category })).into_response()
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use ironclaw_product_adapters::auth::mark_request_signature_verified;
    use ironclaw_product_adapters::capabilities::ProductAdapterCapabilities;
    use ironclaw_product_adapters::external::{
        ExternalActorRef, ExternalConversationRef, ExternalEventId,
    };
    use ironclaw_product_adapters::identity::{
        AdapterInstallationId, ProductAdapterId, ProductSurfaceKind,
    };
    use ironclaw_product_adapters::{
        AuthRequirement, OutboundDeliverySink, ParsedProductInbound, ProductAdapter,
        ProductAdapterError, ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload,
        ProductOutboundEnvelope, ProductRenderOutcome, ProductTriggerReason,
        ProjectionSubscriptionRequest, ProtocolAuthEvidence, ProtocolAuthFailure,
        ProtocolHttpEgress, UserMessagePayload,
    };
    use ironclaw_wasm_product_adapters::{
        NativeProductAdapterRunnerConfig, SharedSecretHeaderAuth, WebhookAuth,
    };
    use tower::ServiceExt;

    use super::*;

    #[derive(Clone)]
    struct FakeSlackDispatcher {
        verify_result: Result<ProtocolAuthEvidence, RunnerError>,
        dispatch_result: Result<WebhookProcessOutcome, RunnerError>,
        dispatch_calls: Arc<AtomicUsize>,
    }

    impl FakeSlackDispatcher {
        fn verified() -> Self {
            Self {
                verify_result: Ok(mark_request_signature_verified(
                    "X-Slack-Signature",
                    Some("X-Slack-Request-Timestamp".to_string()),
                    "slack_install_alpha",
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
                dispatch_result: Ok(WebhookProcessOutcome::AcceptedForAsyncDispatch),
                dispatch_calls: Arc::new(AtomicUsize::new(0)),
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

        fn adapter_panicked() -> Self {
            Self {
                dispatch_result: Err(RunnerError::AdapterPanicked),
                ..Self::verified()
            }
        }
    }

    struct StaticAdapter {
        adapter_id: ProductAdapterId,
        installation_id: AdapterInstallationId,
        capabilities: ProductAdapterCapabilities,
        parse_count: Arc<AtomicUsize>,
    }

    impl StaticAdapter {
        fn new(parse_count: Arc<AtomicUsize>) -> Self {
            Self {
                adapter_id: ProductAdapterId::new("slack_v2").expect("valid adapter id"),
                installation_id: AdapterInstallationId::new("install_alpha")
                    .expect("valid installation id"),
                capabilities: ProductAdapterCapabilities::empty(),
                parse_count,
            }
        }
    }

    #[async_trait]
    impl ProductAdapter for StaticAdapter {
        fn adapter_id(&self) -> &ProductAdapterId {
            &self.adapter_id
        }

        fn installation_id(&self) -> &AdapterInstallationId {
            &self.installation_id
        }

        fn surface_kind(&self) -> ProductSurfaceKind {
            ProductSurfaceKind::ExternalChannel
        }

        fn capabilities(&self) -> &ProductAdapterCapabilities {
            &self.capabilities
        }

        fn auth_requirement(&self) -> &AuthRequirement {
            static AUTH: std::sync::LazyLock<AuthRequirement> =
                std::sync::LazyLock::new(|| AuthRequirement::SharedSecretHeader {
                    header_name: "X-Test-Secret".into(),
                });
            &AUTH
        }

        fn parse_inbound(
            &self,
            _raw_payload: &[u8],
            _auth_evidence: &ProtocolAuthEvidence,
        ) -> Result<ParsedProductInbound, ProductAdapterError> {
            self.parse_count.fetch_add(1, Ordering::SeqCst);
            ParsedProductInbound::new(
                ExternalEventId::new("slack-event-1").expect("valid event id"),
                ExternalActorRef::new("slack_user", "U123", None::<String>)
                    .expect("valid actor ref"),
                ExternalConversationRef::new(None, "C123", None::<&str>, None::<&str>)
                    .expect("valid conversation ref"),
                ProductInboundPayload::UserMessage(
                    UserMessagePayload::new("hello", Vec::new(), ProductTriggerReason::DirectChat)
                        .expect("valid user message"),
                ),
            )
        }

        async fn render_outbound(
            &self,
            _envelope: ProductOutboundEnvelope,
            _egress: &dyn ProtocolHttpEgress,
            _delivery_sink: &dyn OutboundDeliverySink,
        ) -> Result<ProductRenderOutcome, ProductAdapterError> {
            Ok(ProductRenderOutcome::DeliveryRecorded)
        }
    }

    struct AckWorkflow {
        accepted_count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ironclaw_product_adapters::ProductWorkflow for AckWorkflow {
        async fn accept_inbound(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProductInboundAck, ProductAdapterError> {
            self.accepted_count.fetch_add(1, Ordering::SeqCst);
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

    impl SlackEventsWebhookDispatcher for FakeSlackDispatcher {
        fn verify_webhook_auth(
            &self,
            _headers: &HeaderMap,
            _body: &[u8],
        ) -> Result<ProtocolAuthEvidence, RunnerError> {
            self.verify_result.clone()
        }

        fn process_verified_webhook_immediate_ack<'a>(
            &'a self,
            _body: &'a [u8],
            _evidence: &'a ProtocolAuthEvidence,
        ) -> Pin<Box<dyn Future<Output = Result<WebhookProcessOutcome, RunnerError>> + Send + 'a>>
        {
            self.dispatch_calls.fetch_add(1, Ordering::SeqCst);
            let result = self.dispatch_result.clone();
            Box::pin(async move { result })
        }

        fn drain_immediate_ack_tasks<'a>(
            &'a self,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async {})
        }
    }

    async fn post_slack_events(dispatcher: FakeSlackDispatcher, body: &'static str) -> Response {
        let mount = slack_events_route_mount(SlackEventsRouteState::new(Arc::new(dispatcher)));
        mount
            .router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(SLACK_EVENTS_PATH)
                    .body(Body::from(body))
                    .expect("request should build"),
            )
            .await
            .expect("router should respond")
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

    #[tokio::test]
    async fn slack_events_handler_returns_401_on_auth_failure() {
        let dispatcher = FakeSlackDispatcher::auth_failure();
        let calls = Arc::clone(&dispatcher.dispatch_calls);
        let response = post_slack_events(dispatcher, r#"{"type":"event_callback"}"#).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn slack_events_handler_returns_challenge_on_url_verification() {
        let dispatcher = FakeSlackDispatcher::verified();
        let calls = Arc::clone(&dispatcher.dispatch_calls);
        let response = post_slack_events(
            dispatcher,
            r#"{"type":"url_verification","challenge":"challenge-token"}"#,
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        assert_eq!(&bytes[..], b"challenge-token");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn slack_events_handler_returns_400_on_url_verification_parse_error() {
        let dispatcher = FakeSlackDispatcher::verified();
        let calls = Arc::clone(&dispatcher.dispatch_calls);
        let response = post_slack_events(dispatcher, r#"{"type":"url_verification"}"#).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_error_body(response, "malformed_payload").await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn slack_events_handler_returns_429_when_at_capacity() {
        let dispatcher = FakeSlackDispatcher::at_capacity();
        let calls = Arc::clone(&dispatcher.dispatch_calls);
        let response = post_slack_events(dispatcher, r#"{"type":"event_callback"}"#).await;

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_error_body(response, "capacity").await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn slack_events_handler_returns_503_on_workflow_timeout() {
        let dispatcher = FakeSlackDispatcher::workflow_timeout();
        let calls = Arc::clone(&dispatcher.dispatch_calls);
        let response = post_slack_events(dispatcher, r#"{"type":"event_callback"}"#).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_error_body(response, "temporarily_unavailable").await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn slack_events_handler_returns_503_on_adapter_panic() {
        let dispatcher = FakeSlackDispatcher::adapter_panicked();
        let calls = Arc::clone(&dispatcher.dispatch_calls);
        let response = post_slack_events(dispatcher, r#"{"type":"event_callback"}"#).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_error_body(response, "temporarily_unavailable").await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn slack_events_handler_returns_ok_on_successful_dispatch() {
        let dispatcher = FakeSlackDispatcher::verified();
        let calls = Arc::clone(&dispatcher.dispatch_calls);
        let response = post_slack_events(dispatcher, r#"{"type":"event_callback"}"#).await;

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
    async fn slack_events_handler_dispatches_through_native_runner() {
        let parse_count = Arc::new(AtomicUsize::new(0));
        let accepted_count = Arc::new(AtomicUsize::new(0));
        let runner = NativeProductAdapterRunner::with_config(
            Arc::new(StaticAdapter::new(Arc::clone(&parse_count))),
            Arc::new(AckWorkflow {
                accepted_count: Arc::clone(&accepted_count),
            }),
            WebhookAuth::SharedSecretHeader(SharedSecretHeaderAuth {
                header_name: "X-Test-Secret".into(),
                expected_secret: "topsecret".into(),
                subject: "slack_install_alpha".into(),
            }),
            NativeProductAdapterRunnerConfig::new(
                Duration::from_secs(1),
                std::num::NonZeroUsize::new(1).expect("nonzero"),
            ),
        );
        let state = SlackEventsRouteState::new(Arc::new(runner));
        let mount = slack_events_route_mount(state.clone());
        let response = mount
            .router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(SLACK_EVENTS_PATH)
                    .header("X-Test-Secret", "topsecret")
                    .body(Body::from(r#"{"type":"event_callback"}"#))
                    .expect("request should build"),
            )
            .await
            .expect("router should respond");

        assert_eq!(response.status(), StatusCode::OK);
        state.drain_immediate_ack_tasks().await;
        assert_eq!(parse_count.load(Ordering::SeqCst), 1);
        assert_eq!(accepted_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn runner_error_response_maps_adapter_panicked_to_503() {
        let response = runner_error_response(RunnerError::AdapterPanicked);

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn slack_events_route_uses_limited_body_policy() {
        let descriptors = slack_events_route_descriptors();
        let [descriptor] = descriptors.as_slice() else {
            panic!("expected exactly one Slack Events route descriptor")
        };
        let BodyLimitPolicy::Limited { max_bytes } = descriptor.policy().body_limit() else {
            panic!("Slack Events route should have a body limit")
        };

        assert_eq!(*max_bytes, SLACK_EVENTS_BODY_LIMIT_BYTES);
    }

    #[test]
    fn slack_events_route_uses_global_rate_limit_scope() {
        let descriptors = slack_events_route_descriptors();
        let [descriptor] = descriptors.as_slice() else {
            panic!("expected exactly one Slack Events route descriptor")
        };
        let RateLimitPolicy::Limited { scope, .. } = descriptor.policy().rate_limit() else {
            panic!("Slack Events route should be rate limited")
        };

        assert_eq!(*scope, RateLimitScope::Global);
    }
}
