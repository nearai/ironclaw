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

use crate::slack_actor_identity::{
    RebornUserIdentityLookup, SLACK_IDENTITY_PROVIDER, slack_user_identity_provider_user_id,
};
use crate::slack_personal_binding_pairing::SlackPersonalBindingPairingService;
use crate::webui_serve::{PublicRouteDrain, PublicRouteMount};

mod installation;
pub use installation::{
    ResolvedSlackCommand, ResolvedSlackIngress, ResolvedSlackInstallation, SlackApiAppId,
    SlackChannelId, SlackEnterpriseId, SlackEnvelopeMetadata, SlackIngressError,
    SlackInstallationRateLimitConfig, SlackInstallationRateLimiter, SlackInstallationRecord,
    SlackInstallationResolver, SlackInstallationSelector, SlackTeamId, SlackUserId,
    StaticSlackInstallationResolver,
};

#[cfg(test)]
mod e2e_tests;
#[cfg(test)]
mod handler_tests;

pub const SLACK_EVENTS_PATH: &str = "/webhooks/slack/events";
const SLACK_EVENTS_ROUTE_ID: &str = "slack.events";

pub const SLACK_COMMANDS_PATH: &str = "/webhooks/slack/commands";
const SLACK_COMMANDS_ROUTE_ID: &str = "slack.commands";

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

    /// Resolve and verify a Slack slash-command request, then apply the same
    /// per-installation post-verification rate limit the events path uses.
    /// Encapsulates the private rate limiter so the commands route never
    /// reaches past this service into installation internals.
    async fn resolve_command(
        &self,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<ResolvedSlackCommand, SlackIngressError> {
        let command = self
            .resolver
            .resolve_command_ingress(&headers, body.as_ref())
            .await?;
        self.installation_rate_limiter
            .check(command.installation())?;
        Ok(command)
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

/// Both Slack host-ingress route descriptors, projected from the bundled Slack
/// extension manifest in a single parse on first use (the manifest is a
/// compile-time constant, so the projection is deterministic and cached for
/// the process lifetime).
///
/// The routes' path/method/policy are declared as data in
/// `assets/slack/manifest.toml` (`[[product_adapter.inbound.host_ingress]]`)
/// and validated by `ironclaw_host_api` (incl. the fail-closed floor that a
/// `public_webhook` listener must require `webhook_signature`) plus
/// `ironclaw_product_adapter_registry` (ingress credential coherence). Only the
/// declarative descriptors live in the manifest — the axum handlers and the
/// HMAC verifier stay in this module, and the mount functions build their
/// routes from these descriptors so what axum mounts cannot drift from what
/// the manifest declares. Panics if the bundled manifest does not declare a
/// route or declares it with a non-POST method: `SLACK_MANIFEST` is a
/// compile-time constant, so either is a build-time invariant violation,
/// surfaced at startup.
static SLACK_INGRESS_DESCRIPTORS: LazyLock<SlackIngressDescriptors> = LazyLock::new(|| {
    let descriptors = crate::host_ingress::bundled_host_ingress_descriptors(
        crate::extension_host::available_extensions::slack_manifest_toml(),
    )
    .unwrap_or_else(|error| {
        panic!("bundled Slack manifest must project host-ingress routes: {error}")
    });
    SlackIngressDescriptors {
        events: bundled_slack_post_descriptor(&descriptors, SLACK_EVENTS_ROUTE_ID),
        commands: bundled_slack_post_descriptor(&descriptors, SLACK_COMMANDS_ROUTE_ID),
    }
});

struct SlackIngressDescriptors {
    events: IngressRouteDescriptor,
    commands: IngressRouteDescriptor,
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

/// Ephemeral copy shown when the Slack user is already linked to Ironclaw.
/// Carries no code — an already-paired user has nothing to redeem.
const SLACK_PAIR_ALREADY_LINKED_MESSAGE: &str = "You're already connected.";

/// Ephemeral fallback when pairing cannot be completed right now. Carries no
/// code and no internal detail; the cause is logged at `debug` only.
const SLACK_PAIR_UNAVAILABLE_MESSAGE: &str =
    "Pairing is temporarily unavailable — please try again in a moment.";

/// The only slash command this endpoint serves. Slack delivers a command here
/// only if its Request URL points here, but guard anyway so a misconfigured
/// second command can't mint a pairing code.
const SLACK_PAIR_COMMAND: &str = "/pair";

/// Ephemeral reply when a command other than `/pair` reaches this endpoint.
const SLACK_PAIR_UNKNOWN_COMMAND_MESSAGE: &str =
    "Unknown command. Run /pair to get a fresh Ironclaw pairing code.";

/// Route state for the `/pair` slash command. Holds the shared ingress service
/// (verification + per-installation rate limit), the force-mint pairing
/// service, and the identity lookup used to short-circuit already-linked
/// users before a code is minted.
#[derive(Clone)]
pub struct SlackCommandsRouteState {
    ingress: SlackIngressService,
    pairing: SlackPersonalBindingPairingService,
    lookup: Arc<dyn RebornUserIdentityLookup>,
}

impl SlackCommandsRouteState {
    pub fn new(
        ingress: SlackIngressService,
        pairing: SlackPersonalBindingPairingService,
        lookup: Arc<dyn RebornUserIdentityLookup>,
    ) -> Self {
        Self {
            ingress,
            pairing,
            lookup,
        }
    }

    async fn handle_pair_command(&self, headers: HeaderMap, body: Bytes) -> Response {
        let command = match self.ingress.resolve_command(headers, body).await {
            Ok(command) => command,
            // Pre-auth failures never mint a code; reuse the shared webhook
            // error surface (401/400/429) rather than an ephemeral reply.
            Err(error) => return ingress_error_response(error),
        };

        // This endpoint is dedicated to `/pair`; never mint a code for some
        // other command pointed here by a misconfigured app.
        if command.command() != SLACK_PAIR_COMMAND {
            return slack_slash_ephemeral(SLACK_PAIR_UNKNOWN_COMMAND_MESSAGE);
        }

        // Already-linked users get a confirmation, never a fresh code. Bindings
        // are keyed by the installation-scoped identity (matching the events
        // path), not the bare Slack user id.
        let provider_user_id = slack_user_identity_provider_user_id(
            command.installation().adapter_installation_id(),
            command.slack_user_id().as_str(),
        );
        match self
            .lookup
            .resolve_user_identity(SLACK_IDENTITY_PROVIDER, &provider_user_id)
            .await
        {
            Ok(Some(_)) => return slack_slash_ephemeral(SLACK_PAIR_ALREADY_LINKED_MESSAGE),
            Ok(None) => {}
            Err(error) => {
                tracing::debug!(
                    target = "ironclaw::reborn::slack_commands",
                    error = %error,
                    "Slack /pair identity lookup failed"
                );
                return slack_slash_ephemeral(SLACK_PAIR_UNAVAILABLE_MESSAGE);
            }
        }

        // Force-mint a brand-new code, invalidating any prior one. The code is
        // returned in the ephemeral reply only — never logged, never DM'd.
        match self
            .pairing
            .reissue_challenge(
                command.installation().adapter_installation_id().clone(),
                command.slack_user_id().clone(),
            )
            .await
        {
            Ok(issued) => slack_slash_ephemeral(format!(
                "Here's your fresh Ironclaw pairing code: {}\n\
                 Paste it into Ironclaw to finish connecting. It expires in 10 minutes.",
                issued.code.as_str()
            )),
            Err(error) => {
                tracing::debug!(
                    target = "ironclaw::reborn::slack_commands",
                    error = %error,
                    "Slack /pair challenge reissue failed"
                );
                slack_slash_ephemeral(SLACK_PAIR_UNAVAILABLE_MESSAGE)
            }
        }
    }
}

impl std::fmt::Debug for SlackCommandsRouteState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlackCommandsRouteState")
            .field("ingress", &self.ingress)
            .field("pairing", &"SlackPersonalBindingPairingService")
            .field("lookup", &"Arc<dyn RebornUserIdentityLookup>")
            .finish()
    }
}

pub fn slack_commands_route_mount(state: SlackCommandsRouteState) -> PublicRouteMount {
    let descriptor = SLACK_INGRESS_DESCRIPTORS.commands.clone();
    PublicRouteMount::new(
        Router::new()
            .route(
                descriptor.route_pattern().as_str(),
                post(slack_commands_handler),
            )
            .with_state(state),
        vec![descriptor],
    )
}

pub fn slack_commands_route_descriptors() -> Vec<IngressRouteDescriptor> {
    vec![SLACK_INGRESS_DESCRIPTORS.commands.clone()]
}

async fn slack_commands_handler(
    State(state): State<SlackCommandsRouteState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    state.handle_pair_command(headers, body).await
}

/// Slack slash-command response delivered in-place to the invoking user only.
/// `response_type: "ephemeral"` keeps the reply private to that user in the
/// channel where they ran the command, so the pairing code never lands in
/// shared history.
#[derive(Debug, Serialize)]
struct SlackSlashResponse {
    response_type: &'static str,
    text: String,
}

fn slack_slash_ephemeral(text: impl Into<String>) -> Response {
    (
        StatusCode::OK,
        Json(SlackSlashResponse {
            response_type: "ephemeral",
            text: text.into(),
        }),
    )
        .into_response()
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

        fn resolve_command_ingress<'a>(
            &'a self,
            headers: &'a HeaderMap,
            body: &'a [u8],
        ) -> Pin<
            Box<dyn Future<Output = Result<ResolvedSlackCommand, SlackIngressError>> + Send + 'a>,
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
                let mut command = None;
                let mut user_id = None;
                for (key, value) in url::form_urlencoded::parse(body) {
                    match key.as_ref() {
                        "command" => command = Some(value.into_owned()),
                        "user_id" => user_id = Some(value.into_owned()),
                        _ => {}
                    }
                }
                Ok(ResolvedSlackCommand::new(
                    installation,
                    command.unwrap_or_else(|| "/pair".to_string()),
                    SlackUserId::new(user_id.unwrap_or_else(|| "U123".to_string())),
                ))
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

    #[test]
    fn slack_commands_route_descriptor_matches_manifest_projection() {
        // Same guard for the slash-commands route (16 KiB body, 6k req / 60s).
        assert_eq!(
            slack_commands_route_descriptors(),
            vec![expected_slack_descriptor(
                SLACK_COMMANDS_ROUTE_ID,
                SLACK_COMMANDS_PATH,
                NonZeroU64::new(16 * 1024).expect("nonzero"),
                NonZeroU32::new(6_000).expect("nonzero"),
            )]
        );
    }

    /// Caller-level coverage for the `/pair` slash command handler: signature
    /// verification runs (via the ingress), an already-linked caller gets a
    /// confirmation instead of a code, an unlinked caller gets a fresh code, and
    /// a challenge-store fault degrades to an ephemeral reply — never a 5xx that
    /// Slack renders as "the app did not respond".
    mod pair_command_tests {
        use super::*;
        use crate::slack_actor_identity::{
            RebornUserIdentityLookup, RebornUserIdentityLookupError,
        };
        use crate::slack_personal_binding::{
            RebornUserIdentityBinding, SlackPersonalBindingPrincipal, SlackPersonalUserBindingError,
        };
        use crate::slack_personal_binding_pairing::{
            IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingChallenge,
            SlackPersonalBindingPairingChallengeStore, SlackPersonalBindingPairingCode,
            SlackPersonalBindingPairingError, SlackPersonalBindingPairingNotification,
            SlackPersonalBindingPairingNotifier, SlackPersonalUserBinder,
        };
        use ironclaw_host_api::UserId;

        struct StubChallengeStore {
            reissue_ok: bool,
        }

        #[async_trait]
        impl SlackPersonalBindingPairingChallengeStore for StubChallengeStore {
            async fn issue_challenge(
                &self,
                _challenge: SlackPersonalBindingPairingChallenge,
            ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
            {
                unreachable!("/pair reissues; it never issues a first-time challenge")
            }

            async fn get_challenge(
                &self,
                _code: &SlackPersonalBindingPairingCode,
            ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
            {
                Err(SlackPersonalBindingPairingError::ChallengeNotFound)
            }

            async fn consume_challenge(
                &self,
                _code: &SlackPersonalBindingPairingCode,
            ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
            {
                Err(SlackPersonalBindingPairingError::ChallengeNotFound)
            }

            async fn reissue_challenge(
                &self,
                challenge: SlackPersonalBindingPairingChallenge,
            ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
            {
                if !self.reissue_ok {
                    return Err(SlackPersonalBindingPairingError::Backend(
                        "challenge store unavailable".to_string(),
                    ));
                }
                Ok(IssuedSlackPersonalBindingPairingChallenge {
                    code: SlackPersonalBindingPairingCode::new("PAIR1234").expect("code"),
                    challenge,
                })
            }
        }

        #[derive(Debug)]
        struct NoopBinder;

        #[async_trait]
        impl SlackPersonalUserBinder for NoopBinder {
            async fn validate_installation_actor(
                &self,
                _principal: &SlackPersonalBindingPrincipal,
                _installation_id: &AdapterInstallationId,
                _slack_user_id: &SlackUserId,
            ) -> Result<(), SlackPersonalUserBindingError> {
                Ok(())
            }

            async fn bind_installation_actor(
                &self,
                _principal: SlackPersonalBindingPrincipal,
                _installation_id: AdapterInstallationId,
                _slack_user_id: SlackUserId,
            ) -> Result<RebornUserIdentityBinding, SlackPersonalUserBindingError> {
                unreachable!("/pair reissues a code; it never binds an actor")
            }
        }

        struct NoopNotifier;

        #[async_trait]
        impl SlackPersonalBindingPairingNotifier for NoopNotifier {
            async fn send_pairing_challenge(
                &self,
                _notification: SlackPersonalBindingPairingNotification,
            ) -> Result<(), SlackPersonalBindingPairingError> {
                Ok(())
            }
        }

        struct StubLookup {
            linked: bool,
        }

        #[async_trait]
        impl RebornUserIdentityLookup for StubLookup {
            async fn resolve_user_identity(
                &self,
                _provider: &str,
                _provider_user_id: &str,
            ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
                Ok(self
                    .linked
                    .then(|| UserId::new("user:already-linked").expect("user")))
            }

            async fn user_has_provider_binding(
                &self,
                _provider: &str,
                _user_id: &UserId,
            ) -> Result<bool, RebornUserIdentityLookupError> {
                Ok(false)
            }

            async fn user_has_provider_binding_with_provider_user_id_prefix(
                &self,
                _provider: &str,
                _user_id: &UserId,
                _prefix: Option<&str>,
            ) -> Result<bool, RebornUserIdentityLookupError> {
                Ok(false)
            }
        }

        fn pair_state(linked: bool, reissue_ok: bool) -> SlackCommandsRouteState {
            let ingress = SlackIngressService::new(Arc::new(FakeSlackResolver::new(Arc::new(
                FakeSlackDispatcher::verified(),
            ))));
            let pairing = SlackPersonalBindingPairingService::new_with_binder(
                Arc::new(NoopBinder),
                Arc::new(StubChallengeStore { reissue_ok }),
                Arc::new(NoopNotifier),
            );
            SlackCommandsRouteState::new(ingress, pairing, Arc::new(StubLookup { linked }))
        }

        async fn post_pair(state: SlackCommandsRouteState) -> Response {
            let mount = slack_commands_route_mount(state);
            let request = Request::builder()
                .method("POST")
                .uri(SLACK_COMMANDS_PATH)
                .header("content-type", "application/x-www-form-urlencoded")
                .header("X-Slack-Signature", "v0=stub")
                .header("X-Slack-Request-Timestamp", "1700000000")
                .body(Body::from("command=%2Fpair&user_id=U123"))
                .expect("request should build");
            mount
                .router
                .clone()
                .oneshot(request)
                .await
                .expect("router should respond")
        }

        async fn ephemeral_text(response: Response) -> String {
            let bytes = response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes();
            let body: serde_json::Value =
                serde_json::from_slice(&bytes).expect("ephemeral json body");
            body["text"].as_str().unwrap_or_default().to_string()
        }

        #[tokio::test]
        async fn pair_command_mints_a_fresh_code_for_an_unlinked_caller() {
            let response = post_pair(pair_state(false, true)).await;
            assert_eq!(response.status(), StatusCode::OK);
            let text = ephemeral_text(response).await;
            assert!(
                text.contains("PAIR1234"),
                "the ephemeral reply must carry the fresh pairing code, got: {text}"
            );
        }

        #[tokio::test]
        async fn pair_command_tells_an_already_linked_caller_they_are_connected() {
            let response = post_pair(pair_state(true, true)).await;
            assert_eq!(response.status(), StatusCode::OK);
            let text = ephemeral_text(response).await;
            assert!(
                text.contains("already connected"),
                "an already-linked caller gets a confirmation, got: {text}"
            );
            assert!(
                !text.contains("PAIR1234"),
                "an already-linked caller must never be minted a code"
            );
        }

        #[tokio::test]
        async fn pair_command_degrades_to_ephemeral_reply_when_reissue_faults() {
            // A challenge-store fault must not become a 5xx (Slack renders that as
            // "the app did not respond"); it surfaces as an ephemeral message.
            let response = post_pair(pair_state(false, false)).await;
            assert_eq!(response.status(), StatusCode::OK);
            let text = ephemeral_text(response).await;
            assert!(
                !text.contains("PAIR1234"),
                "no code is minted on fault: {text}"
            );
        }
    }
}
