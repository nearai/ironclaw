//! Slack Events API route composition for the Reborn ProductAdapter path.
//!
//! This module exposes an axum route fragment plus ingress descriptors. It does
//! not bind listeners and does not reuse the legacy v1 Slack channel. The host
//! decides whether to mount this fragment (for example behind
//! `REBORN_SLACK_ENABLED`) and supplies a preconfigured native adapter runner.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock};

use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::ingress::IngressRouteDescriptor;
use ironclaw_product_adapters::ProtocolAuthEvidence;
use ironclaw_wasm_product_adapters::{
    ImmediateAckWorkflowObserver, NativeProductAdapterRunner, RunnerError, WebhookProcessOutcome,
};
use serde::Serialize;

use crate::webui::route_mounts::{PublicRouteDrain, PublicRouteMount};

mod installation;
pub use installation::{
    ResolvedSlackIngress, ResolvedSlackInstallation, SlackApiAppId, SlackChannelId,
    SlackEnterpriseId, SlackEnvelopeMetadata, SlackIngressError, SlackInstallationRateLimitConfig,
    SlackInstallationRateLimiter, SlackInstallationRecord, SlackInstallationResolver,
    SlackInstallationSelector, SlackTeamId, SlackUserId, StaticSlackInstallationResolver,
};

#[cfg(test)]
mod e2e_tests;
#[cfg(test)]
mod handler_tests;

pub const SLACK_EVENTS_PATH: &str = "/webhooks/slack/events";
const SLACK_EVENTS_ROUTE_ID: &str = "slack.events";

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
        observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
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
        observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
    ) -> Pin<Box<dyn Future<Output = Result<WebhookProcessOutcome, RunnerError>> + Send + 'a>> {
        Box::pin(
            NativeProductAdapterRunner::process_verified_webhook_immediate_ack_with_observer(
                self, body, evidence, observer,
            ),
        )
    }

    fn drain_immediate_ack_tasks<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(NativeProductAdapterRunner::drain_immediate_ack_tasks(self))
    }
}

#[derive(Clone)]
pub struct SlackIngressService {
    resolver: Arc<dyn SlackInstallationResolver>,
    installation_rate_limiter: SlackInstallationRateLimiter,
}

impl SlackIngressService {
    pub fn new(resolver: Arc<dyn SlackInstallationResolver>) -> Self {
        Self::with_rate_limit_config(resolver, SlackInstallationRateLimitConfig::default())
    }

    pub fn with_rate_limit_config(
        resolver: Arc<dyn SlackInstallationResolver>,
        rate_limit: SlackInstallationRateLimitConfig,
    ) -> Self {
        Self {
            resolver,
            installation_rate_limiter: SlackInstallationRateLimiter::new(rate_limit),
        }
    }

    async fn handle_events(
        &self,
        headers: HeaderMap,
        body: Bytes,
        workflow_observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
    ) -> Response {
        let ingress = match self.resolver.resolve_ingress(&headers, body.as_ref()).await {
            Ok(ingress) => ingress,
            Err(error) => return ingress_error_response(error),
        };
        if let Err(error) = self.installation_rate_limiter.check(ingress.installation()) {
            return ingress_error_response(error);
        }

        match ingress {
            ResolvedSlackIngress::UrlVerification { challenge, .. } => {
                (StatusCode::OK, challenge).into_response()
            }
            ResolvedSlackIngress::Event { installation, .. } => match installation
                .dispatcher()
                .process_verified_webhook_immediate_ack(
                    body.as_ref(),
                    installation.evidence(),
                    installation.workflow_observer().or(workflow_observer),
                )
                .await
            {
                Ok(_) => (StatusCode::OK, "ok").into_response(),
                Err(error) => runner_error_response(error),
            },
        }
    }

    pub async fn drain_installations(&self) {
        self.resolver.drain_installations().await;
    }
}

impl std::fmt::Debug for SlackIngressService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlackIngressService")
            .field("resolver", &"Arc<dyn SlackInstallationResolver>")
            .field("installation_rate_limiter", &self.installation_rate_limiter)
            .finish()
    }
}

#[derive(Clone)]
pub struct SlackEventsRouteState {
    ingress: SlackIngressService,
    workflow_observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
}

impl SlackEventsRouteState {
    pub fn new(ingress: SlackIngressService) -> Self {
        Self {
            ingress,
            workflow_observer: None,
        }
    }

    pub fn from_resolver(resolver: Arc<dyn SlackInstallationResolver>) -> Self {
        Self::new(SlackIngressService::new(resolver))
    }

    pub fn with_workflow_observer(
        mut self,
        workflow_observer: Arc<dyn ImmediateAckWorkflowObserver>,
    ) -> Self {
        self.workflow_observer = Some(workflow_observer);
        self
    }

    pub async fn drain_immediate_ack_tasks(&self) {
        self.ingress.drain_installations().await;
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
            .field("ingress", &self.ingress)
            .field("workflow_observer", &self.workflow_observer.is_some())
            .finish()
    }
}

pub fn slack_events_route_mount(state: SlackEventsRouteState) -> PublicRouteMount {
    let descriptor = SLACK_INGRESS_DESCRIPTORS.events.clone();
    PublicRouteMount::new(
        Router::new()
            .route(
                descriptor.route_pattern().as_str(),
                post(slack_events_handler),
            )
            .with_state(state.clone()),
        vec![descriptor],
    )
    .with_drain(Arc::new(state))
}

pub fn slack_events_route_descriptors() -> Vec<IngressRouteDescriptor> {
    vec![SLACK_INGRESS_DESCRIPTORS.events.clone()]
}

/// The Slack host-ingress route descriptor, projected from the bundled Slack
/// **bot** channel manifest in a single parse on first use (the manifest is a
/// compile-time constant, so the projection is deterministic and cached for
/// the process lifetime).
///
/// The route's path/method/policy are declared as data in
/// `assets/slack_bot/manifest.toml` (`[[product_adapter.inbound.host_ingress]]`)
/// and validated by `ironclaw_host_api` (incl. the fail-closed floor that a
/// `public_webhook` listener must require `webhook_signature`) plus
/// `ironclaw_product_adapter_registry` (ingress credential coherence). Only the
/// declarative descriptors live in the manifest — the axum handlers and the
/// HMAC verifier stay in this module, and the mount functions build their
/// routes from these descriptors so what axum mounts cannot drift from what
/// the manifest declares. Panics if the bundled manifest does not declare a
/// route or declares it with a non-POST method: `SLACK_BOT_MANIFEST` is a
/// compile-time constant, so either is a build-time invariant violation,
/// surfaced at startup.
static SLACK_INGRESS_DESCRIPTORS: LazyLock<SlackIngressDescriptors> = LazyLock::new(|| {
    let descriptors = crate::host_ingress::bundled_host_ingress_descriptors(
        crate::extension_host::available_extensions::slack_bot_manifest_toml(),
    )
    .unwrap_or_else(|error| {
        panic!("bundled Slack manifest must project host-ingress routes: {error}")
    });
    SlackIngressDescriptors {
        events: bundled_slack_post_descriptor(&descriptors, SLACK_EVENTS_ROUTE_ID),
    }
});

struct SlackIngressDescriptors {
    events: IngressRouteDescriptor,
}

fn bundled_slack_post_descriptor(
    descriptors: &[IngressRouteDescriptor],
    route_id: &str,
) -> IngressRouteDescriptor {
    let descriptor = crate::host_ingress::descriptor_for_route(descriptors, route_id)
        .unwrap_or_else(|error| {
            panic!("bundled Slack manifest must declare host-ingress route {route_id}: {error}")
        });
    // The mount functions wire their handlers with `post(...)`; fail closed at
    // projection time if the manifest ever declares another method.
    if descriptor.method() != NetworkMethod::Post {
        panic!(
            "bundled Slack manifest declares host-ingress route {route_id} with method {}, \
             but the serve layer mounts POST handlers",
            descriptor.method()
        );
    }
    descriptor
}

async fn slack_events_handler(
    State(state): State<SlackEventsRouteState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    state
        .ingress
        .handle_events(headers, body, state.workflow_observer.clone())
        .await
}

fn ingress_error_response(error: SlackIngressError) -> Response {
    match error {
        SlackIngressError::Runner(error) => runner_error_response(error),
        SlackIngressError::Envelope(error) => {
            tracing::debug!(
                target = "ironclaw::reborn::slack_events",
                error = %error,
                "Slack Events API envelope metadata parse failed"
            );
            error_response(
                StatusCode::BAD_REQUEST,
                SlackWebhookErrorCategory::MalformedPayload,
            )
        }
        SlackIngressError::InstallationNotFound => {
            tracing::debug!(
                target = "ironclaw::reborn::slack_events",
                reason = "not_found",
                "Slack Events API installation resolution failed"
            );
            error_response(
                StatusCode::UNAUTHORIZED,
                SlackWebhookErrorCategory::Authentication,
            )
        }
        SlackIngressError::AmbiguousInstallation => {
            tracing::debug!(
                target = "ironclaw::reborn::slack_events",
                reason = "ambiguous",
                "Slack Events API installation resolution failed"
            );
            error_response(
                StatusCode::UNAUTHORIZED,
                SlackWebhookErrorCategory::Authentication,
            )
        }
        SlackIngressError::InstallationRateLimited {
            tenant_id,
            adapter_installation_id,
        } => {
            tracing::debug!(
                target = "ironclaw::reborn::slack_events",
                tenant_id = %tenant_id,
                adapter_installation_id = %adapter_installation_id,
                "Slack Events API installation rate limit exceeded"
            );
            error_response(
                StatusCode::TOO_MANY_REQUESTS,
                SlackWebhookErrorCategory::Capacity,
            )
        }
        SlackIngressError::ConversationStoreUnavailable { reason } => {
            tracing::error!(
                target = "ironclaw::reborn::slack_events",
                reason = %reason,
                "Slack Events API durable conversation store unavailable"
            );
            // Storage/infra outage, not an auth failure: 503 so Slack retries
            // delivery instead of treating the installation as unauthorized.
            error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                SlackWebhookErrorCategory::TemporarilyUnavailable,
            )
        }
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
    use ironclaw_host_api::TenantId;
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
    use ironclaw_slack_v2_adapter::SlackPayloadParseError;
    use ironclaw_wasm_product_adapters::{
        NativeProductAdapterRunnerConfig, SharedSecretHeaderAuth, WebhookAuth,
    };
    use tower::ServiceExt;

    use super::*;
    use ironclaw_host_api::NetworkMethod;
    use ironclaw_host_api::ingress::{
        AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
        IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressScopeSource, ListenerClass,
        RateLimitPolicy, RateLimitScope, StreamingMode, WebSocketOriginPolicy,
    };
    use std::num::{NonZeroU32, NonZeroU64};

    /// Rebuild the pre-migration Slack ingress descriptor literal, so the
    /// manifest-projected descriptor can be asserted equal to it
    /// (behavior-preserving migration guard). `window_seconds` is 60 for both
    /// Slack routes.
    fn expected_slack_descriptor(
        route_id: &str,
        path: &str,
        body_limit: NonZeroU64,
        max_requests: NonZeroU32,
    ) -> IngressRouteDescriptor {
        let policy = IngressPolicy::new(IngressPolicyParts {
            listener_class: ListenerClass::PublicWebhook,
            auth: IngressAuthPolicy::Required {
                schemes: vec![IngressAuthScheme::WebhookSignature],
            },
            scope_source: IngressScopeSource::HostResolved,
            body_limit: BodyLimitPolicy::Limited {
                max_bytes: body_limit,
            },
            rate_limit: RateLimitPolicy::Limited {
                scope: RateLimitScope::Global,
                max_requests,
                window_seconds: NonZeroU32::new(60).expect("nonzero"),
            },
            cors: CorsPolicy::NotApplicable,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::PublicCallback,
            effect_path: AllowedEffectPath::ProductWorkflow,
        })
        .expect("policy validates");
        IngressRouteDescriptor::new(route_id, NetworkMethod::Post, path, policy)
            .expect("descriptor validates")
    }

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

    struct FakeSlackResolver {
        dispatcher: Arc<dyn SlackEventsWebhookDispatcher>,
    }

    impl FakeSlackResolver {
        fn new(dispatcher: Arc<dyn SlackEventsWebhookDispatcher>) -> Self {
            Self { dispatcher }
        }
    }

    impl SlackInstallationResolver for FakeSlackResolver {
        fn resolve_ingress<'a>(
            &'a self,
            headers: &'a HeaderMap,
            body: &'a [u8],
        ) -> Pin<
            Box<dyn Future<Output = Result<ResolvedSlackIngress, SlackIngressError>> + Send + 'a>,
        > {
            Box::pin(async move {
                let evidence = self.dispatcher.verify_webhook_auth(headers, body)?;
                let installation = ResolvedSlackInstallation::new(
                    tenant_id("tenant-alpha"),
                    installation_id("install-alpha"),
                    evidence,
                    Arc::clone(&self.dispatcher),
                    None,
                );
                let value: serde_json::Value = serde_json::from_slice(body).map_err(|err| {
                    SlackIngressError::Envelope(SlackPayloadParseError::InvalidJson {
                        reason: err.to_string(),
                    })
                })?;
                if value.get("type").and_then(|kind| kind.as_str()) == Some("url_verification") {
                    let challenge = value
                        .get("challenge")
                        .and_then(|challenge| challenge.as_str())
                        .ok_or_else(|| {
                            SlackIngressError::Envelope(SlackPayloadParseError::InvalidJson {
                                reason: "missing challenge".into(),
                            })
                        })?;
                    return Ok(ResolvedSlackIngress::UrlVerification {
                        installation,
                        challenge: challenge.to_string(),
                    });
                }
                Ok(ResolvedSlackIngress::Event {
                    installation,
                    metadata: SlackEnvelopeMetadata::new(
                        Some(SlackTeamId::new("T-alpha")),
                        None,
                        Some(SlackApiAppId::new("A-alpha")),
                        Some(SlackUserId::new("U-install-alpha")),
                        Some(SlackUserId::new("U123")),
                        Some(SlackChannelId::new("D123")),
                    ),
                })
            })
        }

        fn drain_installations<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async move {
                self.dispatcher.drain_immediate_ack_tasks().await;
            })
        }
    }

    fn tenant_id(value: &str) -> TenantId {
        TenantId::new(value).expect("valid tenant")
    }

    fn installation_id(value: &str) -> AdapterInstallationId {
        AdapterInstallationId::new(value).expect("valid installation")
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
        async fn submit_inbound(
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
            _observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
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
        post_slack_events_with_headers(Arc::new(dispatcher), body, Vec::new()).await
    }

    async fn post_slack_events_with_headers(
        dispatcher: Arc<dyn SlackEventsWebhookDispatcher>,
        body: &'static str,
        headers: Vec<(&'static str, &'static str)>,
    ) -> Response {
        let resolver = Arc::new(FakeSlackResolver::new(dispatcher));
        let mount = slack_events_route_mount(SlackEventsRouteState::from_resolver(resolver));
        post_to_mount(&mount, body, headers).await
    }

    async fn post_to_mount(
        mount: &PublicRouteMount,
        body: &'static str,
        headers: Vec<(&'static str, &'static str)>,
    ) -> Response {
        let mut builder = Request::builder().method("POST").uri(SLACK_EVENTS_PATH);
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        mount
            .router
            .clone()
            .oneshot(
                builder
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
        let state = SlackEventsRouteState::from_resolver(Arc::new(FakeSlackResolver::new(
            Arc::new(runner),
        )));
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
    fn ingress_error_response_maps_conversation_store_unavailable_to_503() {
        // Regression: a durable conversation-binding-store outage is an infra
        // fault, not an authentication failure. It must surface as 503 (so
        // Slack retries delivery), never the 401 that InstallationNotFound
        // would have produced.
        let response = ingress_error_response(SlackIngressError::ConversationStoreUnavailable {
            reason: "host state filesystem offline".to_string(),
        });

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn slack_events_route_descriptor_matches_manifest_projection() {
        // Behavior-preserving migration guard: the Slack events descriptor
        // projected from the bundled manifest's `[[host_ingress]]` declaration
        // equals the pre-migration Rust literal (1 MiB body, 12k req / 60s,
        // public_webhook + webhook_signature). This is the load-bearing example
        // that the manifest-driven ingress contract is real and used.
        assert_eq!(
            slack_events_route_descriptors(),
            vec![expected_slack_descriptor(
                SLACK_EVENTS_ROUTE_ID,
                SLACK_EVENTS_PATH,
                NonZeroU64::new(1024 * 1024).expect("nonzero"),
                NonZeroU32::new(12_000).expect("nonzero"),
            )]
        );
    }
}
