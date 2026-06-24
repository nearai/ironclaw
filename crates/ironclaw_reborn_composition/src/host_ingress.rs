//! Dormant generic host-ingress route composition.
//!
//! This module owns the host-side executable loop for descriptor-declared
//! ingress without taking over listener lifecycle. It returns a
//! [`PublicRouteMount`] fragment for the host gateway to merge, matching the
//! existing Slack Events API seam while keeping protocol-specific parsing and
//! signature checks behind traits.
//!
//! The public surface here is intentionally unused until the serve-wiring
//! migration step routes the Slack events webhook through
//! [`public_ingress_route_mount`] behind the `host_ingress_mode = "generic"`
//! gate. It is exposed as `pub mod` (like `slack_serve`) so the host serve
//! crate can consume it once wired; that also keeps the dormant public surface
//! reachable so it neither trips `unreachable_pub` nor `dead_code`.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, head, patch, post, put};
use futures::future::join_all;
use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::ingress::{
    BodyLimitPolicy, HostIngressRouteDeclaration, IngressCredentialHandle, IngressRouteDescriptor,
    RateLimitPolicy, RateLimitScope,
};
use ironclaw_product_adapters::{
    AuthRequirement, ProtocolAuthEvidence, mark_bearer_token_verified,
    mark_request_signature_verified, mark_session_verified, mark_shared_secret_header_verified,
};
use ironclaw_secrets::SecretMaterial;
use secrecy::ExposeSecret;
use serde::Serialize;

use crate::webui_serve::{PublicRouteDrain, PublicRouteMount};

const MAX_HOST_INGRESS_VERIFICATION_CANDIDATES: usize = 8;

type HostIngressSecretVerifier = dyn for<'a> Fn(
        &UnverifiedHostIngressRequest<'a>,
        &ResolvedIngressSecret,
    ) -> Result<bool, HostIngressError>
    + Send
    + Sync;

pub struct UnverifiedHostIngressRequest<'a> {
    headers: &'a HeaderMap,
    body: &'a [u8],
}

impl<'a> UnverifiedHostIngressRequest<'a> {
    pub fn new(headers: &'a HeaderMap, body: &'a [u8]) -> Self {
        Self { headers, body }
    }

    pub fn headers(&self) -> &HeaderMap {
        self.headers
    }

    pub fn body(&self) -> &[u8] {
        self.body
    }
}

pub struct VerifiedHostIngressRequest {
    headers: HeaderMap,
    body: Bytes,
    auth_evidence: ProtocolAuthEvidence,
}

impl VerifiedHostIngressRequest {
    pub fn new(headers: HeaderMap, body: Bytes, auth_evidence: ProtocolAuthEvidence) -> Self {
        Self {
            headers,
            body,
            auth_evidence,
        }
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn body(&self) -> &[u8] {
        self.body.as_ref()
    }

    pub fn auth_evidence(&self) -> &ProtocolAuthEvidence {
        &self.auth_evidence
    }
}

#[derive(Debug, Clone)]
pub struct HostIngressImmediateResponse {
    status: StatusCode,
    body: Bytes,
}

impl HostIngressImmediateResponse {
    pub fn accepted() -> Self {
        Self {
            status: StatusCode::OK,
            body: Bytes::from_static(b"ok"),
        }
    }

    pub fn ok_body(body: impl Into<Bytes>) -> Self {
        Self {
            status: StatusCode::OK,
            body: body.into(),
        }
    }

    fn into_response(self) -> Response {
        (self.status, self.body).into_response()
    }
}

#[derive(Clone)]
pub struct HostIngressAuthCandidate {
    candidate_id: String,
    auth_requirement: AuthRequirement,
    credential_handles: Vec<IngressCredentialHandle>,
    verifier: Arc<HostIngressSecretVerifier>,
}

impl HostIngressAuthCandidate {
    pub fn new<F>(
        candidate_id: impl Into<String>,
        auth_requirement: AuthRequirement,
        credential_handles: Vec<IngressCredentialHandle>,
        verifier: F,
    ) -> Result<Self, HostIngressError>
    where
        F: for<'a> Fn(
                &UnverifiedHostIngressRequest<'a>,
                &ResolvedIngressSecret,
            ) -> Result<bool, HostIngressError>
            + Send
            + Sync
            + 'static,
    {
        let candidate_id = candidate_id.into();
        if candidate_id.trim().is_empty() {
            return Err(HostIngressError::Internal {
                reason: "host ingress auth candidate id must not be empty".to_string(),
            });
        }
        if credential_handles.is_empty() {
            return Err(HostIngressError::Internal {
                reason: "host ingress auth candidate must declare at least one credential handle"
                    .to_string(),
            });
        }
        Ok(Self {
            candidate_id,
            auth_requirement,
            credential_handles,
            verifier: Arc::new(verifier),
        })
    }

    pub fn candidate_id(&self) -> &str {
        &self.candidate_id
    }

    pub fn credential_handles(&self) -> &[IngressCredentialHandle] {
        &self.credential_handles
    }

    fn verify(
        &self,
        request: &UnverifiedHostIngressRequest<'_>,
        secret: &ResolvedIngressSecret,
    ) -> Result<bool, HostIngressError> {
        (self.verifier)(request, secret)
    }

    fn mint_evidence(&self) -> ProtocolAuthEvidence {
        match &self.auth_requirement {
            AuthRequirement::RequestSignature {
                header_name,
                timestamp_header_name,
            } => mark_request_signature_verified(
                header_name.clone(),
                timestamp_header_name.clone(),
                self.candidate_id.clone(),
            ),
            AuthRequirement::SharedSecretHeader { header_name } => {
                mark_shared_secret_header_verified(header_name.clone(), self.candidate_id.clone())
            }
            AuthRequirement::SessionCookie { name } => {
                mark_session_verified(name.clone(), self.candidate_id.clone())
            }
            AuthRequirement::BearerToken => mark_bearer_token_verified(self.candidate_id.clone()),
        }
    }
}

impl std::fmt::Debug for HostIngressAuthCandidate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostIngressAuthCandidate")
            .field("candidate_id", &self.candidate_id)
            .field("auth_requirement", &self.auth_requirement)
            .field("credential_handles", &self.credential_handles)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct ResolvedIngressSecret {
    material: SecretMaterial,
}

impl ResolvedIngressSecret {
    pub fn new(material: SecretMaterial) -> Self {
        Self { material }
    }

    pub fn from_plaintext(value: impl Into<String>) -> Self {
        Self::new(SecretMaterial::from(value.into()))
    }

    pub fn expose_secret(&self) -> &str {
        self.material.expose_secret()
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.expose_secret().as_bytes()
    }
}

impl std::fmt::Debug for ResolvedIngressSecret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedIngressSecret")
            .field("material", &"<redacted>")
            .finish()
    }
}

#[async_trait]
pub trait HostIngressCredentialResolver: Send + Sync {
    async fn resolve_ingress_secret(
        &self,
        candidate: &HostIngressAuthCandidate,
        handle: &IngressCredentialHandle,
    ) -> Result<ResolvedIngressSecret, HostIngressError>;
}

#[async_trait]
pub trait HostIngressCapabilityHandler: Send + Sync {
    async fn auth_candidates(
        &self,
        request: &UnverifiedHostIngressRequest<'_>,
    ) -> Result<Vec<HostIngressAuthCandidate>, HostIngressError>;

    async fn handle_verified(
        &self,
        request: VerifiedHostIngressRequest,
    ) -> Result<HostIngressImmediateResponse, HostIngressError>;

    fn drain<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HostIngressError {
    #[error("malformed host ingress payload: {reason}")]
    MalformedPayload { reason: String },
    #[error("host ingress authentication failed: {reason}")]
    AuthenticationFailed { reason: String },
    #[error("multiple verified host ingress candidates matched")]
    AmbiguousCandidates,
    #[error("host ingress proposed {count} authentication candidates, exceeding cap {max}")]
    TooManyCandidates { count: usize, max: usize },
    #[error("host ingress payload exceeds body limit {limit} bytes")]
    PayloadTooLarge { limit: u64 },
    #[error("host ingress route rate limit exceeded")]
    RateLimited,
    #[error("host ingress route is at capacity: {reason}")]
    Capacity { reason: String },
    #[error("host ingress route is temporarily unavailable: {reason}")]
    TemporarilyUnavailable { reason: String },
    #[error("host ingress internal error: {reason}")]
    Internal { reason: String },
    #[error("duplicate host ingress route id `{route_id}`")]
    DuplicateRouteId { route_id: String },
    #[error("host ingress route collision for {method} {route_pattern}")]
    RouteCollision {
        method: NetworkMethod,
        route_pattern: String,
    },
    #[error("unsupported host ingress rate limit scope {scope:?} on route `{route_id}`")]
    UnsupportedRateLimitScope {
        route_id: String,
        scope: RateLimitScope,
    },
}

impl HostIngressError {
    pub fn into_response(self) -> Response {
        let (status, category) = self.response_status();
        tracing::debug!(
            target = "ironclaw::reborn::host_ingress",
            status = status.as_u16(),
            error = %self,
            "host ingress request rejected"
        );
        error_response(status, category)
    }

    fn response_status(&self) -> (StatusCode, HostIngressWebhookErrorCategory) {
        match self {
            Self::MalformedPayload { .. } => (
                StatusCode::BAD_REQUEST,
                HostIngressWebhookErrorCategory::MalformedPayload,
            ),
            Self::AuthenticationFailed { .. }
            | Self::AmbiguousCandidates
            | Self::TooManyCandidates { .. } => (
                StatusCode::UNAUTHORIZED,
                HostIngressWebhookErrorCategory::Authentication,
            ),
            Self::PayloadTooLarge { .. } => (
                StatusCode::PAYLOAD_TOO_LARGE,
                HostIngressWebhookErrorCategory::MalformedPayload,
            ),
            Self::RateLimited | Self::Capacity { .. } => (
                StatusCode::TOO_MANY_REQUESTS,
                HostIngressWebhookErrorCategory::Capacity,
            ),
            Self::TemporarilyUnavailable { .. } => (
                StatusCode::SERVICE_UNAVAILABLE,
                HostIngressWebhookErrorCategory::TemporarilyUnavailable,
            ),
            Self::Internal { .. }
            | Self::DuplicateRouteId { .. }
            | Self::RouteCollision { .. }
            | Self::UnsupportedRateLimitScope { .. } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                HostIngressWebhookErrorCategory::Internal,
            ),
        }
    }

    fn authentication_failed(reason: impl Into<String>) -> Self {
        Self::AuthenticationFailed {
            reason: reason.into(),
        }
    }

    fn is_authentication_failure(&self) -> bool {
        matches!(self, Self::AuthenticationFailed { .. })
    }
}

impl IntoResponse for HostIngressError {
    fn into_response(self) -> Response {
        HostIngressError::into_response(self)
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum HostIngressWebhookErrorCategory {
    Authentication,
    Capacity,
    MalformedPayload,
    TemporarilyUnavailable,
    Internal,
}

#[derive(Debug, Serialize)]
struct HostIngressWebhookErrorBody {
    error: HostIngressWebhookErrorCategory,
}

fn error_response(status: StatusCode, category: HostIngressWebhookErrorCategory) -> Response {
    (
        status,
        axum::Json(HostIngressWebhookErrorBody { error: category }),
    )
        .into_response()
}

pub struct HostIngressDrainGuard {
    handlers: Vec<Arc<dyn HostIngressCapabilityHandler>>,
}

impl HostIngressDrainGuard {
    pub fn new(handlers: Vec<Arc<dyn HostIngressCapabilityHandler>>) -> Self {
        Self { handlers }
    }
}

impl PublicRouteDrain for HostIngressDrainGuard {
    fn drain<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            join_all(self.handlers.iter().map(|handler| handler.drain())).await;
        })
    }
}

pub struct HostIngressRegistration {
    pub declaration: HostIngressRouteDeclaration,
    pub handler: Arc<dyn HostIngressCapabilityHandler>,
}

#[derive(Clone)]
struct HostIngressRouteState {
    policy: ironclaw_host_api::ingress::IngressPolicy,
    declared_handles: Arc<HashSet<IngressCredentialHandle>>,
    handler: Arc<dyn HostIngressCapabilityHandler>,
    credentials: Arc<dyn HostIngressCredentialResolver>,
    rate_limiter: Option<HostIngressRateLimiter>,
}

impl HostIngressRouteState {
    fn new(
        descriptor: &IngressRouteDescriptor,
        declaration: &HostIngressRouteDeclaration,
        handler: Arc<dyn HostIngressCapabilityHandler>,
        credentials: Arc<dyn HostIngressCredentialResolver>,
    ) -> Result<Self, HostIngressError> {
        let route_id = descriptor.route_id().as_str().to_string();
        let rate_limiter = build_rate_limiter(route_id.as_str(), descriptor.policy().rate_limit())?;
        let declared_handles = declaration
            .auth()
            .iter()
            .flat_map(|binding| binding.credential_handles().iter().cloned())
            .collect();
        Ok(Self {
            policy: descriptor.policy().clone(),
            declared_handles: Arc::new(declared_handles),
            handler,
            credentials,
            rate_limiter,
        })
    }

    async fn verify_candidates(
        &self,
        request: &UnverifiedHostIngressRequest<'_>,
        candidates: Vec<HostIngressAuthCandidate>,
    ) -> Result<ProtocolAuthEvidence, HostIngressError> {
        let count = candidates.len();
        if count > MAX_HOST_INGRESS_VERIFICATION_CANDIDATES {
            return Err(HostIngressError::TooManyCandidates {
                count,
                max: MAX_HOST_INGRESS_VERIFICATION_CANDIDATES,
            });
        }
        if candidates.is_empty() {
            return Err(HostIngressError::authentication_failed(
                "no host ingress authentication candidates",
            ));
        }

        let mut first_auth_failure = None;
        let mut verified = Vec::new();
        for candidate in candidates {
            if !self.candidate_handles_are_declared(&candidate) {
                remember_auth_failure(
                    &mut first_auth_failure,
                    HostIngressError::authentication_failed(
                        "candidate referenced an undeclared ingress credential handle",
                    ),
                );
                continue;
            }

            match self.verify_candidate(request, &candidate).await {
                Ok(true) => verified.push(candidate.mint_evidence()),
                Ok(false) => remember_auth_failure(
                    &mut first_auth_failure,
                    HostIngressError::authentication_failed(
                        "candidate secrets did not verify request",
                    ),
                ),
                Err(error) if error.is_authentication_failure() => {
                    remember_auth_failure(&mut first_auth_failure, error);
                }
                Err(error) => return Err(error),
            }
        }

        match verified.len() {
            1 => Ok(verified.remove(0)),
            0 => Err(first_auth_failure.unwrap_or_else(|| {
                HostIngressError::authentication_failed("no verified host ingress candidate")
            })),
            _ => Err(HostIngressError::AmbiguousCandidates),
        }
    }

    async fn verify_candidate(
        &self,
        request: &UnverifiedHostIngressRequest<'_>,
        candidate: &HostIngressAuthCandidate,
    ) -> Result<bool, HostIngressError> {
        let mut first_auth_failure = None;
        for handle in candidate.credential_handles() {
            let secret = match self
                .credentials
                .resolve_ingress_secret(candidate, handle)
                .await
            {
                Ok(secret) => secret,
                Err(error) if error.is_authentication_failure() => {
                    remember_auth_failure(&mut first_auth_failure, error);
                    continue;
                }
                Err(error) => return Err(error),
            };
            match candidate.verify(request, &secret) {
                Ok(true) => return Ok(true),
                Ok(false) => remember_auth_failure(
                    &mut first_auth_failure,
                    HostIngressError::authentication_failed(
                        "resolved ingress credential did not verify request",
                    ),
                ),
                Err(error) if error.is_authentication_failure() => {
                    remember_auth_failure(&mut first_auth_failure, error);
                }
                Err(error) => return Err(error),
            }
        }
        if let Some(error) = first_auth_failure {
            return Err(error);
        }
        Ok(false)
    }

    fn candidate_handles_are_declared(&self, candidate: &HostIngressAuthCandidate) -> bool {
        candidate
            .credential_handles()
            .iter()
            .all(|handle| self.declared_handles.contains(handle))
    }
}

fn remember_auth_failure(slot: &mut Option<HostIngressError>, error: HostIngressError) {
    if slot.is_none() {
        *slot = Some(error);
    }
}

pub fn public_ingress_route_mount(
    registrations: Vec<HostIngressRegistration>,
    credentials: Arc<dyn HostIngressCredentialResolver>,
) -> Result<PublicRouteMount, HostIngressError> {
    let mut route_ids = HashSet::new();
    let mut route_keys = HashSet::new();
    let mut router = Router::new();
    let mut descriptors = Vec::new();
    let mut handlers = Vec::new();

    for registration in registrations {
        let descriptor = registration.declaration.route().clone();
        let route_id = descriptor.route_id().as_str().to_string();
        if !route_ids.insert(route_id.clone()) {
            return Err(HostIngressError::DuplicateRouteId { route_id });
        }

        let key = RouteKey {
            method: descriptor.method(),
            route_pattern: descriptor.route_pattern().as_str().to_string(),
        };
        if !route_keys.insert(key.clone()) {
            return Err(HostIngressError::RouteCollision {
                method: key.method,
                route_pattern: key.route_pattern,
            });
        }

        let state = HostIngressRouteState::new(
            &descriptor,
            &registration.declaration,
            Arc::clone(&registration.handler),
            Arc::clone(&credentials),
        )?;
        router = mount_route(
            router,
            descriptor.method(),
            descriptor.route_pattern().as_str(),
            state,
        );
        descriptors.push(descriptor);
        handlers.push(registration.handler);
    }

    Ok(PublicRouteMount::new(router, descriptors)
        .with_drain(Arc::new(HostIngressDrainGuard::new(handlers))))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RouteKey {
    method: NetworkMethod,
    route_pattern: String,
}

fn mount_route(
    router: Router,
    method: NetworkMethod,
    route_pattern: &str,
    state: HostIngressRouteState,
) -> Router {
    let method_router = match method {
        NetworkMethod::Get => get(host_ingress_handler).with_state(state),
        NetworkMethod::Post => post(host_ingress_handler).with_state(state),
        NetworkMethod::Put => put(host_ingress_handler).with_state(state),
        NetworkMethod::Patch => patch(host_ingress_handler).with_state(state),
        NetworkMethod::Delete => delete(host_ingress_handler).with_state(state),
        NetworkMethod::Head => head(host_ingress_handler).with_state(state),
    };
    router.route(route_pattern, method_router)
}

async fn host_ingress_handler(
    State(state): State<HostIngressRouteState>,
    request: Request<Body>,
) -> Response {
    match handle_host_ingress(state, request).await {
        Ok(response) => response.into_response(),
        Err(error) => error.into_response(),
    }
}

async fn handle_host_ingress(
    state: HostIngressRouteState,
    request: Request<Body>,
) -> Result<HostIngressImmediateResponse, HostIngressError> {
    let (parts, body) = request.into_parts();
    let body = read_body_under_policy(&state, &parts.headers, body).await?;
    if let Some(rate_limiter) = &state.rate_limiter {
        rate_limiter.check()?;
    }

    let unverified = UnverifiedHostIngressRequest::new(&parts.headers, body.as_ref());
    let candidates = state.handler.auth_candidates(&unverified).await?;
    let auth_evidence = state.verify_candidates(&unverified, candidates).await?;
    state
        .handler
        .handle_verified(VerifiedHostIngressRequest::new(
            parts.headers,
            body,
            auth_evidence,
        ))
        .await
}

async fn read_body_under_policy(
    state: &HostIngressRouteState,
    headers: &HeaderMap,
    body: Body,
) -> Result<Bytes, HostIngressError> {
    let max_bytes = match state.policy.body_limit() {
        BodyLimitPolicy::NoBody => 0,
        BodyLimitPolicy::Limited { max_bytes } => max_bytes.get(),
    };

    if let Some(declared) = declared_content_length(headers)
        && declared > max_bytes
    {
        return Err(HostIngressError::PayloadTooLarge { limit: max_bytes });
    }

    let max_bytes = usize::try_from(max_bytes)
        .map_err(|_| HostIngressError::PayloadTooLarge { limit: u64::MAX })?;
    axum::body::to_bytes(body, max_bytes)
        .await
        .map_err(|_| HostIngressError::PayloadTooLarge {
            limit: state.policy.body_limit().max_bytes_for_error(),
        })
}

fn declared_content_length(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
}

trait BodyLimitForError {
    fn max_bytes_for_error(self) -> u64;
}

impl BodyLimitForError for BodyLimitPolicy {
    fn max_bytes_for_error(self) -> u64 {
        match self {
            BodyLimitPolicy::NoBody => 0,
            BodyLimitPolicy::Limited { max_bytes } => max_bytes.get(),
        }
    }
}

fn build_rate_limiter(
    route_id: &str,
    policy: &RateLimitPolicy,
) -> Result<Option<HostIngressRateLimiter>, HostIngressError> {
    match policy {
        RateLimitPolicy::Disabled { .. } => Ok(None),
        RateLimitPolicy::Limited {
            scope,
            max_requests,
            window_seconds,
        } => match scope {
            RateLimitScope::Global | RateLimitScope::PerRoute => Ok(Some(
                HostIngressRateLimiter::new(*max_requests, *window_seconds),
            )),
            RateLimitScope::PerCaller | RateLimitScope::PerTenant | RateLimitScope::PerIp => {
                Err(HostIngressError::UnsupportedRateLimitScope {
                    route_id: route_id.to_string(),
                    scope: *scope,
                })
            }
        },
    }
}

#[derive(Clone)]
struct HostIngressRateLimiter {
    config: HostIngressRateLimitConfig,
    bucket: Arc<Mutex<HostIngressRateLimitBucket>>,
}

impl HostIngressRateLimiter {
    fn new(max_requests: std::num::NonZeroU32, window_seconds: std::num::NonZeroU32) -> Self {
        let now = Instant::now();
        let config = HostIngressRateLimitConfig {
            max_requests,
            window: Duration::from_secs(u64::from(window_seconds.get())),
        };
        Self {
            bucket: Arc::new(Mutex::new(HostIngressRateLimitBucket::full(now, &config))),
            config,
        }
    }

    fn check(&self) -> Result<(), HostIngressError> {
        let now = Instant::now();
        let mut bucket = match self.bucket.lock() {
            Ok(bucket) => bucket,
            Err(poisoned) => poisoned.into_inner(),
        };
        bucket.refill(now, &self.config);
        if !bucket.try_consume() {
            return Err(HostIngressError::RateLimited);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct HostIngressRateLimitConfig {
    max_requests: std::num::NonZeroU32,
    window: Duration,
}

#[derive(Debug, Clone)]
struct HostIngressRateLimitBucket {
    last_refilled_at: Instant,
    tokens: f64,
}

impl HostIngressRateLimitBucket {
    fn full(now: Instant, config: &HostIngressRateLimitConfig) -> Self {
        Self {
            last_refilled_at: now,
            tokens: config.max_requests.get() as f64,
        }
    }

    fn refill(&mut self, now: Instant, config: &HostIngressRateLimitConfig) {
        let elapsed = now.duration_since(self.last_refilled_at);
        if elapsed.is_zero() {
            return;
        }
        let capacity = config.max_requests.get() as f64;
        let refill_ratio = elapsed.as_secs_f64() / config.window.as_secs_f64();
        self.tokens = capacity.min(self.tokens + refill_ratio * capacity);
        self.last_refilled_at = now;
    }

    fn try_consume(&mut self) -> bool {
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::num::{NonZeroU32, NonZeroU64};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use axum::http::Method;
    use ironclaw_host_api::CapabilityId;
    use ironclaw_host_api::ingress::{
        AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, HostIngressTarget,
        IngressAckMode, IngressAuthBinding, IngressAuthPolicy, IngressAuthScheme,
        IngressAuthSchemeName, IngressDrainMode, IngressPolicy, IngressPolicyParts,
        IngressScopeSource, ListenerClass, StreamingMode, WebSocketOriginPolicy,
    };
    use tower::ServiceExt;

    use super::*;

    const TEST_PATH: &str = "/webhooks/test/events";

    #[derive(Default)]
    struct FakeHandler {
        candidates: Mutex<Vec<HostIngressAuthCandidate>>,
        handle_calls: AtomicUsize,
        drain_called: AtomicBool,
    }

    impl FakeHandler {
        fn with_candidates(candidates: Vec<HostIngressAuthCandidate>) -> Arc<Self> {
            Arc::new(Self {
                candidates: Mutex::new(candidates),
                handle_calls: AtomicUsize::new(0),
                drain_called: AtomicBool::new(false),
            })
        }

        fn handle_calls(&self) -> usize {
            self.handle_calls.load(Ordering::SeqCst)
        }

        fn drain_called(&self) -> bool {
            self.drain_called.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl HostIngressCapabilityHandler for FakeHandler {
        async fn auth_candidates(
            &self,
            _request: &UnverifiedHostIngressRequest<'_>,
        ) -> Result<Vec<HostIngressAuthCandidate>, HostIngressError> {
            Ok(self.candidates.lock().expect("candidates lock").clone())
        }

        async fn handle_verified(
            &self,
            request: VerifiedHostIngressRequest,
        ) -> Result<HostIngressImmediateResponse, HostIngressError> {
            assert!(request.auth_evidence().is_verified());
            self.handle_calls.fetch_add(1, Ordering::SeqCst);
            Ok(HostIngressImmediateResponse::accepted())
        }

        fn drain<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async move {
                self.drain_called.store(true, Ordering::SeqCst);
            })
        }
    }

    struct FakeCredentialResolver {
        secrets: Mutex<HashMap<String, String>>,
        calls: AtomicUsize,
    }

    impl FakeCredentialResolver {
        fn new(secrets: impl IntoIterator<Item = (&'static str, &'static str)>) -> Arc<Self> {
            Arc::new(Self {
                secrets: Mutex::new(
                    secrets
                        .into_iter()
                        .map(|(handle, secret)| (handle.to_string(), secret.to_string()))
                        .collect(),
                ),
                calls: AtomicUsize::new(0),
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl HostIngressCredentialResolver for FakeCredentialResolver {
        async fn resolve_ingress_secret(
            &self,
            _candidate: &HostIngressAuthCandidate,
            handle: &IngressCredentialHandle,
        ) -> Result<ResolvedIngressSecret, HostIngressError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let secrets = self.secrets.lock().expect("secrets lock");
            let secret = secrets.get(handle.as_str()).ok_or_else(|| {
                HostIngressError::authentication_failed("missing test ingress secret")
            })?;
            Ok(ResolvedIngressSecret::from_plaintext(secret.clone()))
        }
    }

    fn test_candidate(
        id: impl Into<String>,
        handle: impl Into<String>,
    ) -> HostIngressAuthCandidate {
        let handle = handle.into();
        HostIngressAuthCandidate::new(
            id,
            AuthRequirement::RequestSignature {
                header_name: "x-test-signature".to_string(),
                timestamp_header_name: None,
            },
            vec![credential_handle(&handle)],
            |request, secret| {
                let verified = request
                    .headers()
                    .get("x-test-signature")
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.as_bytes() == secret.as_bytes());
                Ok(verified)
            },
        )
        .expect("valid candidate")
    }

    fn credential_handle(value: &str) -> IngressCredentialHandle {
        IngressCredentialHandle::new(value).expect("valid credential handle")
    }

    fn auth_scheme() -> IngressAuthSchemeName {
        IngressAuthSchemeName::new("test-signature").expect("valid auth scheme")
    }

    fn declaration(
        route_id: &'static str,
        path: &'static str,
        handles: Vec<IngressCredentialHandle>,
    ) -> HostIngressRouteDeclaration {
        let descriptor =
            IngressRouteDescriptor::new(route_id, NetworkMethod::Post, path, ingress_policy())
                .expect("valid route descriptor");
        let binding = IngressAuthBinding::new(auth_scheme(), handles).expect("valid auth binding");
        HostIngressRouteDeclaration::new(
            descriptor,
            HostIngressTarget::HostCapability {
                capability_id: CapabilityId::new("test.ingress").expect("valid capability id"),
            },
            vec![binding],
            IngressAckMode::Immediate,
            IngressDrainMode::DrainBeforeRuntimeShutdown,
        )
        .expect("valid declaration")
    }

    fn ingress_policy() -> IngressPolicy {
        IngressPolicy::new(IngressPolicyParts {
            listener_class: ListenerClass::PublicWebhook,
            auth: IngressAuthPolicy::Required {
                schemes: vec![IngressAuthScheme::WebhookSignature],
            },
            scope_source: IngressScopeSource::HostResolved,
            body_limit: BodyLimitPolicy::Limited {
                max_bytes: NonZeroU64::new(1024 * 1024).expect("non-zero body limit"),
            },
            rate_limit: RateLimitPolicy::Limited {
                scope: RateLimitScope::Global,
                max_requests: NonZeroU32::new(12_000).expect("non-zero request limit"),
                window_seconds: NonZeroU32::new(60).expect("non-zero window"),
            },
            cors: CorsPolicy::NotApplicable,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::PublicCallback,
            effect_path: AllowedEffectPath::ProductWorkflow,
        })
        .expect("valid ingress policy")
    }

    fn registration(
        declaration: HostIngressRouteDeclaration,
        handler: Arc<dyn HostIngressCapabilityHandler>,
    ) -> HostIngressRegistration {
        HostIngressRegistration {
            declaration,
            handler,
        }
    }

    fn mount(
        handler: Arc<dyn HostIngressCapabilityHandler>,
        resolver: Arc<dyn HostIngressCredentialResolver>,
        handles: Vec<IngressCredentialHandle>,
    ) -> PublicRouteMount {
        public_ingress_route_mount(
            vec![registration(
                declaration("test.events", TEST_PATH, handles),
                handler,
            )],
            resolver,
        )
        .expect("mount should build")
    }

    async fn post_to_mount(mount: &PublicRouteMount, signature: &'static str) -> Response {
        let request = Request::builder()
            .method(Method::POST)
            .uri(TEST_PATH)
            .header("x-test-signature", signature)
            .body(Body::from(r#"{"type":"event_callback"}"#))
            .expect("valid request");
        mount
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("response")
    }

    #[tokio::test]
    async fn happy_path_one_valid_candidate_dispatches_verified_request() {
        let candidate = test_candidate("candidate-a", "signing-a");
        let handler = FakeHandler::with_candidates(vec![candidate]);
        let resolver = FakeCredentialResolver::new([("signing-a", "valid")]);
        let mount = mount(
            handler.clone(),
            resolver,
            vec![credential_handle("signing-a")],
        );

        let response = post_to_mount(&mount, "valid").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(handler.handle_calls(), 1);
    }

    #[tokio::test]
    async fn forged_request_returns_401_without_dispatch() {
        let candidate = test_candidate("candidate-a", "signing-a");
        let handler = FakeHandler::with_candidates(vec![candidate]);
        let resolver = FakeCredentialResolver::new([("signing-a", "valid")]);
        let mount = mount(
            handler.clone(),
            resolver,
            vec![credential_handle("signing-a")],
        );

        let response = post_to_mount(&mount, "forged").await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(handler.handle_calls(), 0);
    }

    #[tokio::test]
    async fn two_verified_candidates_return_401_fail_closed_without_dispatch() {
        let handler = FakeHandler::with_candidates(vec![
            test_candidate("candidate-a", "signing-a"),
            test_candidate("candidate-b", "signing-b"),
        ]);
        let resolver =
            FakeCredentialResolver::new([("signing-a", "valid"), ("signing-b", "valid")]);
        let mount = mount(
            handler.clone(),
            resolver,
            vec![
                credential_handle("signing-a"),
                credential_handle("signing-b"),
            ],
        );

        let response = post_to_mount(&mount, "valid").await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(handler.handle_calls(), 0);
    }

    #[tokio::test]
    async fn more_than_eight_candidates_returns_401_before_secret_resolution() {
        let candidates: Vec<_> = (0..9)
            .map(|index| test_candidate(format!("candidate-{index}"), format!("signing-{index}")))
            .collect();
        let handles: Vec<_> = (0..9)
            .map(|index| credential_handle(&format!("signing-{index}")))
            .collect();
        let handler = FakeHandler::with_candidates(candidates);
        let resolver =
            FakeCredentialResolver::new(std::iter::empty::<(&'static str, &'static str)>());
        let mount = mount(handler.clone(), resolver.clone(), handles);

        let response = post_to_mount(&mount, "valid").await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(handler.handle_calls(), 0);
        assert_eq!(resolver.calls(), 0);
    }

    #[test]
    fn public_ingress_route_mount_rejects_duplicate_route_id() {
        let handler =
            FakeHandler::with_candidates(vec![test_candidate("candidate-a", "signing-a")]);
        let resolver = FakeCredentialResolver::new([("signing-a", "valid")]);
        let result = public_ingress_route_mount(
            vec![
                registration(
                    declaration(
                        "test.events",
                        TEST_PATH,
                        vec![credential_handle("signing-a")],
                    ),
                    handler.clone(),
                ),
                registration(
                    declaration(
                        "test.events",
                        "/webhooks/test/other",
                        vec![credential_handle("signing-a")],
                    ),
                    handler,
                ),
            ],
            resolver,
        );

        assert!(matches!(
            result,
            Err(HostIngressError::DuplicateRouteId { .. })
        ));
    }

    #[test]
    fn public_ingress_route_mount_rejects_method_path_collision() {
        let handler =
            FakeHandler::with_candidates(vec![test_candidate("candidate-a", "signing-a")]);
        let resolver = FakeCredentialResolver::new([("signing-a", "valid")]);
        let result = public_ingress_route_mount(
            vec![
                registration(
                    declaration(
                        "test.events",
                        TEST_PATH,
                        vec![credential_handle("signing-a")],
                    ),
                    handler.clone(),
                ),
                registration(
                    declaration(
                        "test.events.alt",
                        TEST_PATH,
                        vec![credential_handle("signing-a")],
                    ),
                    handler,
                ),
            ],
            resolver,
        );

        assert!(matches!(
            result,
            Err(HostIngressError::RouteCollision { .. })
        ));
    }

    #[tokio::test]
    async fn host_ingress_drain_guard_awaits_registered_handlers() {
        let handler =
            FakeHandler::with_candidates(vec![test_candidate("candidate-a", "signing-a")]);
        let guard = HostIngressDrainGuard::new(vec![handler.clone()]);

        guard.drain().await;

        assert!(handler.drain_called());
    }
}
