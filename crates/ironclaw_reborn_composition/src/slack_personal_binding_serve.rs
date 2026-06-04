//! WebUI route composition for Slack personal user binding.
//!
//! This module owns only HTTP/session boundary work: a WebUI-authenticated
//! start route, bounded single-use OAuth state, a public callback route, and
//! sanitized success/error redirects. Slack token exchange and persistence stay
//! behind host-supplied ports.

use std::{
    collections::HashMap,
    num::{NonZeroU32, NonZeroU64},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{Extension, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_auth::Timestamp;
use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor,
    IngressScopeSource, ListenerClass, RateLimitPolicy, RateLimitScope, StreamingMode,
    WebSocketOriginPolicy,
};
use ironclaw_product_adapters::AdapterInstallationId;
use ironclaw_product_workflow::WebUiAuthenticatedCaller;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::slack_personal_binding::{
    SlackPersonalBindingPrincipal, SlackPersonalUserBindingError, SlackPersonalUserBindingRequest,
    SlackPersonalUserBindingService,
};
use crate::slack_serve::{SlackApiAppId, SlackEnterpriseId, SlackTeamId, SlackUserId};

pub const SLACK_PERSONAL_BINDING_OAUTH_START_PATH: &str =
    "/api/reborn/slack/personal-binding/oauth/start";
pub const SLACK_PERSONAL_BINDING_OAUTH_CALLBACK_PATH: &str =
    "/api/reborn/slack/personal-binding/oauth/callback";

const SLACK_PERSONAL_BINDING_OAUTH_START_ROUTE_ID: &str = "slack.personal_binding.oauth.start";
const SLACK_PERSONAL_BINDING_OAUTH_CALLBACK_ROUTE_ID: &str =
    "slack.personal_binding.oauth.callback";
const SLACK_PERSONAL_BINDING_BODY_LIMIT_BYTES: NonZeroU64 = NonZeroU64::new(16 * 1024).unwrap(); // safety: 16 KiB is non-zero.
const SLACK_PERSONAL_BINDING_START_MAX_REQUESTS: NonZeroU32 = NonZeroU32::new(20).unwrap(); // safety: 20 is non-zero.
const SLACK_PERSONAL_BINDING_CALLBACK_MAX_REQUESTS: NonZeroU32 = NonZeroU32::new(120).unwrap(); // safety: 120 is non-zero.
const SLACK_PERSONAL_BINDING_RATE_WINDOW_SECONDS: NonZeroU32 = NonZeroU32::new(60).unwrap(); // safety: 60 is non-zero.
const SLACK_PERSONAL_BINDING_STATE_TTL: Duration = Duration::from_secs(5 * 60);
const SLACK_PERSONAL_BINDING_STATE_CAPACITY: usize = 1024;
const SLACK_PERSONAL_BINDING_STATE_CAPACITY_PER_USER: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackPersonalBindingAuthorizationUrl(String);

impl SlackPersonalBindingAuthorizationUrl {
    pub fn new(value: impl Into<String>) -> Result<Self, SlackPersonalBindingOAuthError> {
        let value = value.into();
        let parsed = url::Url::parse(&value).map_err(|error| {
            SlackPersonalBindingOAuthError::InvalidAuthorizationUrl {
                reason: error.to_string(),
            }
        })?;
        match parsed.scheme() {
            "https" | "http" => Ok(Self(value)),
            _ => Err(SlackPersonalBindingOAuthError::InvalidAuthorizationUrl {
                reason: "authorization URL must be http or https".into(),
            }),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SlackPersonalBindingAuthorizationUrl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackPersonalBindingOAuthIdentity {
    pub slack_user_id: SlackUserId,
    pub team_id: SlackTeamId,
    pub enterprise_id: Option<SlackEnterpriseId>,
    pub api_app_id: SlackApiAppId,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SlackPersonalBindingOAuthError {
    #[error("slack authorization URL is invalid: {reason}")]
    InvalidAuthorizationUrl { reason: String },
    #[error("slack OAuth backend unavailable: {0}")]
    Backend(String),
    #[error("slack OAuth response was invalid: {0}")]
    InvalidResponse(String),
}

#[async_trait::async_trait]
pub trait SlackPersonalBindingOAuthClient: Send + Sync {
    fn authorization_url(
        &self,
        callback_url: &str,
        state: &str,
    ) -> Result<SlackPersonalBindingAuthorizationUrl, SlackPersonalBindingOAuthError>;

    async fn exchange_code(
        &self,
        code: &str,
        callback_url: &str,
    ) -> Result<SlackPersonalBindingOAuthIdentity, SlackPersonalBindingOAuthError>;
}

#[derive(Clone)]
pub struct SlackPersonalBindingRouteConfig {
    binding_service: SlackPersonalUserBindingService,
    oauth_client: Arc<dyn SlackPersonalBindingOAuthClient>,
    external_base_url: String,
}

impl SlackPersonalBindingRouteConfig {
    pub fn new(
        binding_service: SlackPersonalUserBindingService,
        oauth_client: Arc<dyn SlackPersonalBindingOAuthClient>,
        external_base_url: impl Into<String>,
    ) -> Result<Self, SlackPersonalBindingRouteConfigError> {
        let external_base_url = external_base_url.into();
        let parsed = url::Url::parse(&external_base_url).map_err(|error| {
            SlackPersonalBindingRouteConfigError::InvalidExternalBaseUrl {
                reason: error.to_string(),
            }
        })?;
        match parsed.scheme() {
            "http" | "https" => Ok(Self {
                binding_service,
                oauth_client,
                external_base_url: external_base_url.trim_end_matches('/').to_string(),
            }),
            _ => Err(
                SlackPersonalBindingRouteConfigError::InvalidExternalBaseUrl {
                    reason: "external base URL must be http or https".into(),
                },
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SlackPersonalBindingRouteConfigError {
    #[error("invalid Slack personal binding external base URL: {reason}")]
    InvalidExternalBaseUrl { reason: String },
}

#[derive(Clone)]
pub(crate) struct SlackPersonalBindingRouteState {
    binding_service: SlackPersonalUserBindingService,
    oauth_client: Arc<dyn SlackPersonalBindingOAuthClient>,
    callback_url: String,
    pending: PendingSlackPersonalBindingStore,
}

impl SlackPersonalBindingRouteState {
    pub(crate) fn new(config: SlackPersonalBindingRouteConfig) -> Self {
        Self {
            binding_service: config.binding_service,
            oauth_client: config.oauth_client,
            callback_url: format!(
                "{}{}",
                config.external_base_url, SLACK_PERSONAL_BINDING_OAUTH_CALLBACK_PATH
            ),
            pending: PendingSlackPersonalBindingStore::new(),
        }
    }
}

impl std::fmt::Debug for SlackPersonalBindingRouteState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlackPersonalBindingRouteState")
            .field("binding_service", &self.binding_service)
            .field("oauth_client", &"Arc<dyn SlackPersonalBindingOAuthClient>")
            .field("callback_url", &self.callback_url)
            .field("pending", &"PendingSlackPersonalBindingStore")
            .finish()
    }
}

pub(crate) struct SlackPersonalBindingRouteMount {
    pub(crate) protected: Router,
    pub(crate) public: Router,
    pub(crate) descriptors: Vec<IngressRouteDescriptor>,
}

pub(crate) fn slack_personal_binding_route_mount(
    state: SlackPersonalBindingRouteState,
) -> SlackPersonalBindingRouteMount {
    SlackPersonalBindingRouteMount {
        protected: Router::new()
            .route(
                SLACK_PERSONAL_BINDING_OAUTH_START_PATH,
                post(slack_personal_binding_oauth_start_handler),
            )
            .with_state(state.clone()),
        public: Router::new()
            .route(
                SLACK_PERSONAL_BINDING_OAUTH_CALLBACK_PATH,
                get(slack_personal_binding_oauth_callback_handler),
            )
            .with_state(state),
        descriptors: slack_personal_binding_route_descriptors(),
    }
}

pub(crate) fn slack_personal_binding_route_descriptors() -> Vec<IngressRouteDescriptor> {
    vec![
        descriptor(
            SLACK_PERSONAL_BINDING_OAUTH_START_ROUTE_ID,
            NetworkMethod::Post,
            SLACK_PERSONAL_BINDING_OAUTH_START_PATH,
            protected_start_policy(),
        ),
        descriptor(
            SLACK_PERSONAL_BINDING_OAUTH_CALLBACK_ROUTE_ID,
            NetworkMethod::Get,
            SLACK_PERSONAL_BINDING_OAUTH_CALLBACK_PATH,
            callback_policy(),
        ),
    ]
}

fn descriptor(
    route_id: &str,
    method: NetworkMethod,
    pattern: &str,
    policy: IngressPolicy,
) -> IngressRouteDescriptor {
    IngressRouteDescriptor::new(route_id.to_string(), method, pattern.to_string(), policy)
        .expect("Slack personal binding route descriptor must validate at startup") // safety: ids/patterns are literals and policies are constructed by sibling helpers.
}

fn protected_start_policy() -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::BearerToken],
        },
        scope_source: IngressScopeSource::AuthenticatedCaller,
        body_limit: BodyLimitPolicy::Limited {
            max_bytes: SLACK_PERSONAL_BINDING_BODY_LIMIT_BYTES,
        },
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerCaller,
            max_requests: SLACK_PERSONAL_BINDING_START_MAX_REQUESTS,
            window_seconds: SLACK_PERSONAL_BINDING_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::UserAction,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .expect("Slack personal binding start policy must validate") // safety: authenticated local gateway mutation matches WebUI/product-auth shape.
}

fn callback_policy() -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::OAuthCallback,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::OAuthState],
        },
        scope_source: IngressScopeSource::HostResolved,
        body_limit: BodyLimitPolicy::NoBody,
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerIp,
            max_requests: SLACK_PERSONAL_BINDING_CALLBACK_MAX_REQUESTS,
            window_seconds: SLACK_PERSONAL_BINDING_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::NotApplicable,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::PublicCallback,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .expect("Slack personal binding callback policy must validate") // safety: OAuth callback + state + host-resolved scope; handler validates state before effects.
}

#[derive(Debug, Deserialize)]
struct SlackPersonalBindingStartRequest {
    installation_id: String,
    redirect_after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlackPersonalBindingStartResponse {
    pub authorization_url: String,
    pub expires_at: Timestamp,
}

async fn slack_personal_binding_oauth_start_handler(
    State(state): State<SlackPersonalBindingRouteState>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<SlackPersonalBindingStartRequest>,
) -> Result<Json<SlackPersonalBindingStartResponse>, SlackPersonalBindingRouteFailure> {
    let installation_id = AdapterInstallationId::new(request.installation_id)
        .map_err(|_| SlackPersonalBindingRouteFailure::invalid_request())?;
    let redirect_after = sanitize_redirect(request.redirect_after);
    let principal = SlackPersonalBindingPrincipal {
        tenant_id: caller.tenant_id,
        user_id: caller.user_id,
    };
    let pending = PendingSlackPersonalBinding {
        principal,
        installation_id,
        redirect_after,
        created_at: Instant::now(),
    };
    let expires_at = Utc::now()
        + ChronoDuration::from_std(SLACK_PERSONAL_BINDING_STATE_TTL)
            .map_err(|_| SlackPersonalBindingRouteFailure::server_error())?;
    let oauth_state = state
        .pending
        .insert(pending)
        .ok_or_else(SlackPersonalBindingRouteFailure::server_error)?;
    let authorization_url = state
        .oauth_client
        .authorization_url(&state.callback_url, oauth_state.as_str())
        .map_err(SlackPersonalBindingRouteFailure::from)?;

    Ok(Json(SlackPersonalBindingStartResponse {
        authorization_url: authorization_url.to_string(),
        expires_at,
    }))
}

#[derive(Debug, Deserialize)]
struct SlackPersonalBindingCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

async fn slack_personal_binding_oauth_callback_handler(
    State(state): State<SlackPersonalBindingRouteState>,
    Query(query): Query<SlackPersonalBindingCallbackQuery>,
) -> Response {
    let Some(oauth_state) = query
        .state
        .filter(|state| !state.is_empty())
        .map(SlackPersonalBindingOAuthState)
    else {
        return redirect_error(None, SlackPersonalBindingErrorCode::InvalidState);
    };
    let Some(pending) = state.pending.take(&oauth_state) else {
        return redirect_error(None, SlackPersonalBindingErrorCode::InvalidState);
    };
    if query.error.is_some() {
        return redirect_error(
            pending.redirect_after.as_deref(),
            SlackPersonalBindingErrorCode::Denied,
        );
    }
    let Some(code) = query.code.filter(|code| !code.is_empty()) else {
        return redirect_error(
            pending.redirect_after.as_deref(),
            SlackPersonalBindingErrorCode::InvalidRequest,
        );
    };
    let identity = match state
        .oauth_client
        .exchange_code(&code, &state.callback_url)
        .await
    {
        Ok(identity) => identity,
        Err(_) => {
            return redirect_error(
                pending.redirect_after.as_deref(),
                SlackPersonalBindingErrorCode::ExchangeFailed,
            );
        }
    };
    let request = SlackPersonalUserBindingRequest {
        installation_id: pending.installation_id,
        slack_user_id: identity.slack_user_id,
        team_id: identity.team_id,
        enterprise_id: identity.enterprise_id,
        api_app_id: identity.api_app_id,
    };
    match state
        .binding_service
        .bind_personal_user(pending.principal, request)
        .await
    {
        Ok(_) => redirect_success(pending.redirect_after.as_deref()),
        Err(SlackPersonalUserBindingError::UnknownInstallation { .. })
        | Err(SlackPersonalUserBindingError::InstallationNotTenantScoped { .. })
        | Err(SlackPersonalUserBindingError::SlackInstallationContextMismatch { .. })
        | Err(SlackPersonalUserBindingError::InvalidSlackId { .. }) => redirect_error(
            pending.redirect_after.as_deref(),
            SlackPersonalBindingErrorCode::Unauthorized,
        ),
        Err(SlackPersonalUserBindingError::BindingStore(_)) => redirect_error(
            pending.redirect_after.as_deref(),
            SlackPersonalBindingErrorCode::ServerError,
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlackPersonalBindingErrorCode {
    InvalidState,
    Denied,
    InvalidRequest,
    ExchangeFailed,
    Unauthorized,
    ServerError,
}

impl SlackPersonalBindingErrorCode {
    fn as_str(self) -> &'static str {
        match self {
            Self::InvalidState => "invalid_state",
            Self::Denied => "denied",
            Self::InvalidRequest => "invalid_request",
            Self::ExchangeFailed => "exchange_failed",
            Self::Unauthorized => "unauthorized",
            Self::ServerError => "server_error",
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum SlackPersonalBindingRouteFailure {
    #[error("invalid request")]
    InvalidRequest,
    #[error("slack personal binding backend unavailable")]
    ServerError,
}

impl SlackPersonalBindingRouteFailure {
    fn invalid_request() -> Self {
        Self::InvalidRequest
    }

    fn server_error() -> Self {
        Self::ServerError
    }
}

impl From<SlackPersonalBindingOAuthError> for SlackPersonalBindingRouteFailure {
    fn from(_error: SlackPersonalBindingOAuthError) -> Self {
        Self::ServerError
    }
}

impl IntoResponse for SlackPersonalBindingRouteFailure {
    fn into_response(self) -> Response {
        let status = match self {
            Self::InvalidRequest => StatusCode::BAD_REQUEST,
            Self::ServerError => StatusCode::SERVICE_UNAVAILABLE,
        };
        (status, self.to_string()).into_response()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SlackPersonalBindingOAuthState(String);

impl SlackPersonalBindingOAuthState {
    fn mint() -> Self {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        Self(URL_SAFE_NO_PAD.encode(bytes))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone)]
struct PendingSlackPersonalBinding {
    principal: SlackPersonalBindingPrincipal,
    installation_id: AdapterInstallationId,
    redirect_after: Option<String>,
    created_at: Instant,
}

#[derive(Clone, Default)]
struct PendingSlackPersonalBindingStore {
    inner: Arc<Mutex<HashMap<SlackPersonalBindingOAuthState, PendingSlackPersonalBinding>>>,
}

impl PendingSlackPersonalBindingStore {
    fn new() -> Self {
        Self::default()
    }

    fn insert(
        &self,
        pending: PendingSlackPersonalBinding,
    ) -> Option<SlackPersonalBindingOAuthState> {
        let state = SlackPersonalBindingOAuthState::mint();
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.len() >= SLACK_PERSONAL_BINDING_STATE_CAPACITY {
            guard.retain(|_, pending| {
                pending.created_at.elapsed() < SLACK_PERSONAL_BINDING_STATE_TTL
            });
        }
        while pending_count_for_principal(&guard, &pending.principal)
            >= SLACK_PERSONAL_BINDING_STATE_CAPACITY_PER_USER
        {
            remove_oldest_for_principal(&mut guard, &pending.principal)?;
        }
        if guard.len() >= SLACK_PERSONAL_BINDING_STATE_CAPACITY {
            remove_oldest_for_principal(&mut guard, &pending.principal)?;
        }
        guard.insert(state.clone(), pending);
        Some(state)
    }

    fn take(&self, state: &SlackPersonalBindingOAuthState) -> Option<PendingSlackPersonalBinding> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let pending = guard.remove(state)?;
        if pending.created_at.elapsed() >= SLACK_PERSONAL_BINDING_STATE_TTL {
            return None;
        }
        Some(pending)
    }
}

fn pending_count_for_principal(
    pending: &HashMap<SlackPersonalBindingOAuthState, PendingSlackPersonalBinding>,
    principal: &SlackPersonalBindingPrincipal,
) -> usize {
    pending
        .values()
        .filter(|pending| pending.principal == *principal)
        .count()
}

fn remove_oldest_for_principal(
    pending: &mut HashMap<SlackPersonalBindingOAuthState, PendingSlackPersonalBinding>,
    principal: &SlackPersonalBindingPrincipal,
) -> Option<PendingSlackPersonalBinding> {
    let oldest = pending
        .iter()
        .filter(|(_, pending)| pending.principal == *principal)
        .min_by_key(|(_, pending)| pending.created_at)
        .map(|(state, _)| state.clone())?;
    pending.remove(&oldest)
}

fn sanitize_redirect(input: Option<String>) -> Option<String> {
    input.filter(|raw| is_safe_redirect(raw))
}

fn is_safe_redirect(value: &str) -> bool {
    if !check_redirect_chars(value) {
        return false;
    }
    let decoded = percent_decode(value);
    check_redirect_chars(&decoded)
}

fn check_redirect_chars(value: &str) -> bool {
    if !value.starts_with('/') || value.starts_with("//") || value.starts_with("/\\") {
        return false;
    }
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"/_-.~:@!$&'()*+,;=?[]%".contains(&byte))
}

fn redirect_success(redirect_after: Option<&str>) -> Response {
    Redirect::temporary(&append_query_param(
        redirect_after.unwrap_or("/v2"),
        "slack_binding",
        "connected",
    ))
    .into_response()
}

fn redirect_error(redirect_after: Option<&str>, code: SlackPersonalBindingErrorCode) -> Response {
    Redirect::temporary(&append_query_param(
        redirect_after.unwrap_or("/v2"),
        "slack_binding_error",
        code.as_str(),
    ))
    .into_response()
}

fn append_query_param(target: &str, key: &str, value: &str) -> String {
    let separator = if target.contains('?') { '&' } else { '?' };
    format!(
        "{target}{separator}{}={}",
        encode_query_component(key),
        encode_query_component(value)
    )
}

fn encode_query_component(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hi = hex_value(bytes[index + 1]);
                let lo = hex_value(bytes[index + 2]);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi << 4) | lo);
                    index += 3;
                } else {
                    out.push(bytes[index]);
                    index += 1;
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        RebornUserIdentityBinding, RebornUserIdentityBindingError, RebornUserIdentityBindingStore,
        SlackPersonalBindingInstallation,
    };
    use axum::{
        body::Body,
        http::{Request, header},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn start_and_callback_bind_session_user_to_proven_slack_identity() {
        let store = Arc::new(RecordingBindingStore::default());
        let service = SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: tenant("tenant-alpha"),
                installation_id: installation("install-alpha"),
                selector: crate::slack_serve::SlackInstallationSelector::app_team(
                    "A-app", "T-team",
                ),
            }],
            store.clone(),
        );
        let oauth = Arc::new(RecordingOAuthClient::default());
        let state = SlackPersonalBindingRouteState::new(
            SlackPersonalBindingRouteConfig::new(service, oauth.clone(), "https://app.example")
                .expect("valid config"),
        );
        let mount = slack_personal_binding_route_mount(state);
        let app = Router::new()
            .merge(mount.protected)
            .merge(mount.public)
            .layer(Extension(caller("tenant-alpha", "user:alice")));

        let start_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(SLACK_PERSONAL_BINDING_OAUTH_START_PATH)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"installation_id":"install-alpha","redirect_after":"/v2/settings"}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("start response");
        assert_eq!(start_response.status(), StatusCode::OK);
        let body = start_response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let start: SlackPersonalBindingStartResponse =
            serde_json::from_slice(&body).expect("start json");
        let state_value = url::Url::parse(&start.authorization_url)
            .expect("authorization URL")
            .query_pairs()
            .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
            .expect("state query");

        let callback_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!(
                        "{SLACK_PERSONAL_BINDING_OAUTH_CALLBACK_PATH}?state={state_value}&code=ok"
                    ))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("callback response");
        assert_eq!(callback_response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            callback_response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok()),
            Some("/v2/settings?slack_binding=connected")
        );
        assert_eq!(oauth.exchanges(), vec!["ok".to_string()]);
        assert_eq!(
            store.bindings(),
            vec![RebornUserIdentityBinding {
                provider: crate::slack_personal_binding::RebornIdentityProviderId::new("slack")
                    .expect("provider"),
                provider_user_id: crate::slack_personal_binding::RebornIdentityProviderUserId::new(
                    "install-alpha:U123",
                )
                .expect("provider user id"),
                user_id: user("user:alice"),
            }]
        );
    }

    #[tokio::test]
    async fn callback_state_is_single_use() {
        let store = Arc::new(RecordingBindingStore::default());
        let service = SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: tenant("tenant-alpha"),
                installation_id: installation("install-alpha"),
                selector: crate::slack_serve::SlackInstallationSelector::app_team(
                    "A-app", "T-team",
                ),
            }],
            store.clone(),
        );
        let oauth = Arc::new(RecordingOAuthClient::default());
        let state = SlackPersonalBindingRouteState::new(
            SlackPersonalBindingRouteConfig::new(service, oauth, "https://app.example")
                .expect("valid config"),
        );
        let pending_state = state
            .pending
            .insert(PendingSlackPersonalBinding {
                principal: SlackPersonalBindingPrincipal {
                    tenant_id: tenant("tenant-alpha"),
                    user_id: user("user:alice"),
                },
                installation_id: installation("install-alpha"),
                redirect_after: None,
                created_at: Instant::now(),
            })
            .expect("pending state");
        let mount = slack_personal_binding_route_mount(state);
        let app = Router::new().merge(mount.public);
        let uri = format!(
            "{SLACK_PERSONAL_BINDING_OAUTH_CALLBACK_PATH}?state={}&code=ok",
            pending_state.as_str()
        );

        let first = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&uri)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("first callback");
        let second = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&uri)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("second callback");

        assert_eq!(first.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            first
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok()),
            Some("/v2?slack_binding=connected")
        );
        assert_eq!(second.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            second
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok()),
            Some("/v2?slack_binding_error=invalid_state")
        );
        assert_eq!(store.bindings().len(), 1);
    }

    #[tokio::test]
    async fn callback_redirects_denied_on_slack_error_query_param() {
        let response = callback_with_state(
            route_state(
                Arc::new(RecordingBindingStore::default()),
                Arc::new(RecordingOAuthClient::default()),
            ),
            PendingOptions::default(),
            "error=access_denied",
        )
        .await;

        assert_redirect_location(response, "/v2?slack_binding_error=denied");
    }

    #[tokio::test]
    async fn callback_redirects_invalid_request_on_missing_code() {
        let response = callback_with_state(
            route_state(
                Arc::new(RecordingBindingStore::default()),
                Arc::new(RecordingOAuthClient::default()),
            ),
            PendingOptions::default(),
            "",
        )
        .await;

        assert_redirect_location(response, "/v2?slack_binding_error=invalid_request");
    }

    #[tokio::test]
    async fn callback_redirects_exchange_failed_on_client_error() {
        let response = callback_with_state(
            route_state(
                Arc::new(RecordingBindingStore::default()),
                Arc::new(FailingOAuthClient),
            ),
            PendingOptions::default(),
            "code=bad",
        )
        .await;

        assert_redirect_location(response, "/v2?slack_binding_error=exchange_failed");
    }

    #[tokio::test]
    async fn callback_redirects_unauthorized_on_binding_mismatch() {
        let service = SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: tenant("tenant-alpha"),
                installation_id: installation("install-alpha"),
                selector: crate::slack_serve::SlackInstallationSelector::app_team(
                    "A-other", "T-team",
                ),
            }],
            Arc::new(RecordingBindingStore::default()),
        );
        let state = SlackPersonalBindingRouteState::new(
            SlackPersonalBindingRouteConfig::new(
                service,
                Arc::new(RecordingOAuthClient::default()),
                "https://app.example",
            )
            .expect("valid config"),
        );

        let response = callback_with_state(state, PendingOptions::default(), "code=ok").await;

        assert_redirect_location(response, "/v2?slack_binding_error=unauthorized");
    }

    #[tokio::test]
    async fn callback_redirects_server_error_on_store_failure() {
        let response = callback_with_state(
            route_state(
                Arc::new(FailingBindingStore),
                Arc::new(RecordingOAuthClient::default()),
            ),
            PendingOptions::default(),
            "code=ok",
        )
        .await;

        assert_redirect_location(response, "/v2?slack_binding_error=server_error");
    }

    #[tokio::test]
    async fn callback_redirects_invalid_state_for_expired_oauth_state() {
        let response = callback_with_state(
            route_state(
                Arc::new(RecordingBindingStore::default()),
                Arc::new(RecordingOAuthClient::default()),
            ),
            PendingOptions {
                created_at: Instant::now() - SLACK_PERSONAL_BINDING_STATE_TTL,
                ..PendingOptions::default()
            },
            "code=ok",
        )
        .await;

        assert_redirect_location(response, "/v2?slack_binding_error=invalid_state");
    }

    #[test]
    fn pending_store_evicts_requester_oldest_without_evicting_other_users() {
        let store = PendingSlackPersonalBindingStore::new();
        let other_state = store
            .insert(pending_for("tenant-alpha", "user:other", 10))
            .expect("other pending");
        let first_requester_state = store
            .insert(pending_for("tenant-alpha", "user:alice", 9))
            .expect("first requester pending");
        store
            .insert(pending_for("tenant-alpha", "user:alice", 8))
            .expect("second requester pending");
        store
            .insert(pending_for("tenant-alpha", "user:alice", 7))
            .expect("third requester pending");
        store
            .insert(pending_for("tenant-alpha", "user:alice", 6))
            .expect("fourth requester pending");

        assert!(store.take(&first_requester_state).is_none());
        assert!(store.take(&other_state).is_some());
    }

    #[derive(Default)]
    struct RecordingOAuthClient {
        exchanges: Mutex<Vec<String>>,
    }

    impl RecordingOAuthClient {
        fn exchanges(&self) -> Vec<String> {
            self.exchanges.lock().expect("lock").clone()
        }
    }

    #[async_trait::async_trait]
    impl SlackPersonalBindingOAuthClient for RecordingOAuthClient {
        fn authorization_url(
            &self,
            callback_url: &str,
            state: &str,
        ) -> Result<SlackPersonalBindingAuthorizationUrl, SlackPersonalBindingOAuthError> {
            SlackPersonalBindingAuthorizationUrl::new(format!(
                "https://slack.example/oauth?redirect_uri={}&state={}",
                encode_query_component(callback_url),
                encode_query_component(state)
            ))
        }

        async fn exchange_code(
            &self,
            code: &str,
            _callback_url: &str,
        ) -> Result<SlackPersonalBindingOAuthIdentity, SlackPersonalBindingOAuthError> {
            self.exchanges.lock().expect("lock").push(code.to_string());
            Ok(SlackPersonalBindingOAuthIdentity {
                slack_user_id: SlackUserId::new("U123"),
                team_id: SlackTeamId::new("T-team"),
                enterprise_id: None,
                api_app_id: SlackApiAppId::new("A-app"),
            })
        }
    }

    struct FailingOAuthClient;

    #[async_trait::async_trait]
    impl SlackPersonalBindingOAuthClient for FailingOAuthClient {
        fn authorization_url(
            &self,
            _callback_url: &str,
            _state: &str,
        ) -> Result<SlackPersonalBindingAuthorizationUrl, SlackPersonalBindingOAuthError> {
            SlackPersonalBindingAuthorizationUrl::new("https://slack.example/oauth")
        }

        async fn exchange_code(
            &self,
            _code: &str,
            _callback_url: &str,
        ) -> Result<SlackPersonalBindingOAuthIdentity, SlackPersonalBindingOAuthError> {
            Err(SlackPersonalBindingOAuthError::Backend("down".into()))
        }
    }

    #[derive(Default)]
    struct RecordingBindingStore {
        bindings: Mutex<Vec<RebornUserIdentityBinding>>,
    }

    impl RecordingBindingStore {
        fn bindings(&self) -> Vec<RebornUserIdentityBinding> {
            self.bindings.lock().expect("lock").clone()
        }
    }

    #[async_trait::async_trait]
    impl RebornUserIdentityBindingStore for RecordingBindingStore {
        async fn bind_user_identity(
            &self,
            binding: RebornUserIdentityBinding,
        ) -> Result<(), RebornUserIdentityBindingError> {
            self.bindings.lock().expect("lock").push(binding);
            Ok(())
        }
    }

    struct FailingBindingStore;

    #[async_trait::async_trait]
    impl RebornUserIdentityBindingStore for FailingBindingStore {
        async fn bind_user_identity(
            &self,
            _binding: RebornUserIdentityBinding,
        ) -> Result<(), RebornUserIdentityBindingError> {
            Err(RebornUserIdentityBindingError::Backend("down".into()))
        }
    }

    #[derive(Clone)]
    struct PendingOptions {
        created_at: Instant,
        redirect_after: Option<String>,
    }

    impl Default for PendingOptions {
        fn default() -> Self {
            Self {
                created_at: Instant::now(),
                redirect_after: None,
            }
        }
    }

    async fn callback_with_state(
        state: SlackPersonalBindingRouteState,
        options: PendingOptions,
        extra_query: &str,
    ) -> Response {
        let pending_state = state
            .pending
            .insert(PendingSlackPersonalBinding {
                principal: SlackPersonalBindingPrincipal {
                    tenant_id: tenant("tenant-alpha"),
                    user_id: user("user:alice"),
                },
                installation_id: installation("install-alpha"),
                redirect_after: options.redirect_after,
                created_at: options.created_at,
            })
            .expect("pending state");
        let separator = if extra_query.is_empty() { "" } else { "&" };
        let app = Router::new().merge(slack_personal_binding_route_mount(state).public);
        app.oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "{SLACK_PERSONAL_BINDING_OAUTH_CALLBACK_PATH}?state={}{}{}",
                    pending_state.as_str(),
                    separator,
                    extra_query
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("callback response")
    }

    fn route_state(
        store: Arc<dyn RebornUserIdentityBindingStore>,
        oauth: Arc<dyn SlackPersonalBindingOAuthClient>,
    ) -> SlackPersonalBindingRouteState {
        let service = SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: tenant("tenant-alpha"),
                installation_id: installation("install-alpha"),
                selector: crate::slack_serve::SlackInstallationSelector::app_team(
                    "A-app", "T-team",
                ),
            }],
            store,
        );
        SlackPersonalBindingRouteState::new(
            SlackPersonalBindingRouteConfig::new(service, oauth, "https://app.example")
                .expect("valid config"),
        )
    }

    fn assert_redirect_location(response: Response, expected: &str) {
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok()),
            Some(expected)
        );
    }

    fn pending_for(
        tenant_id: &str,
        user_id: &str,
        age_seconds: u64,
    ) -> PendingSlackPersonalBinding {
        PendingSlackPersonalBinding {
            principal: SlackPersonalBindingPrincipal {
                tenant_id: tenant(tenant_id),
                user_id: user(user_id),
            },
            installation_id: installation("install-alpha"),
            redirect_after: None,
            created_at: Instant::now() - Duration::from_secs(age_seconds),
        }
    }

    fn caller(tenant_id: &str, user_id: &str) -> WebUiAuthenticatedCaller {
        WebUiAuthenticatedCaller::new(tenant(tenant_id), user(user_id), None, None)
    }

    fn tenant(value: &str) -> ironclaw_host_api::TenantId {
        ironclaw_host_api::TenantId::new(value).expect("tenant")
    }

    fn user(value: &str) -> ironclaw_host_api::UserId {
        ironclaw_host_api::UserId::new(value).expect("user")
    }

    fn installation(value: &str) -> AdapterInstallationId {
        AdapterInstallationId::new(value).expect("installation")
    }
}
