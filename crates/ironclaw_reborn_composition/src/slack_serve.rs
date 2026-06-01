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
    Router,
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

use crate::webui_serve::PublicRouteMount;

pub const SLACK_EVENTS_PATH: &str = "/webhooks/slack/events";
const SLACK_EVENTS_ROUTE_ID: &str = "slack.events";
const SLACK_EVENTS_BODY_LIMIT_BYTES: NonZeroU64 = match NonZeroU64::new(1024 * 1024) {
    Some(value) => value,
    None => NonZeroU64::MIN,
};
const SLACK_EVENTS_MAX_REQUESTS: NonZeroU32 = match NonZeroU32::new(120) {
    Some(value) => value,
    None => NonZeroU32::MIN,
};
const SLACK_EVENTS_RATE_WINDOW_SECONDS: NonZeroU32 = match NonZeroU32::new(60) {
    Some(value) => value,
    None => NonZeroU32::MIN,
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
}

#[derive(Clone)]
pub struct SlackEventsRouteState {
    dispatcher: Arc<dyn SlackEventsWebhookDispatcher>,
}

impl SlackEventsRouteState {
    pub fn new(dispatcher: Arc<dyn SlackEventsWebhookDispatcher>) -> Self {
        Self { dispatcher }
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
    PublicRouteMount {
        router: Router::new()
            .route(SLACK_EVENTS_PATH, post(slack_events_handler))
            .with_state(state),
        descriptors: slack_events_route_descriptors(),
    }
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
            scope: RateLimitScope::PerIp,
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
            return StatusCode::BAD_REQUEST.into_response();
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
    let status = match &error {
        RunnerError::AuthenticationFailed { .. } => StatusCode::UNAUTHORIZED,
        RunnerError::TooManyInFlight { .. } => StatusCode::TOO_MANY_REQUESTS,
        RunnerError::Adapter(adapter_error) if adapter_error.is_retryable() => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        RunnerError::WorkflowTimeout { .. }
        | RunnerError::WorkflowJoinFailed
        | RunnerError::WorkflowPanicked
        | RunnerError::AdapterPanicked => StatusCode::SERVICE_UNAVAILABLE,
        RunnerError::Adapter(_) => StatusCode::BAD_REQUEST,
    };
    tracing::debug!(
        target = "ironclaw::reborn::slack_events",
        status = status.as_u16(),
        error = %error,
        "Slack Events API webhook rejected"
    );
    status.into_response()
}
