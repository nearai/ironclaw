//! HTTP route handlers for the WebChat v2 OAuth login flow.
//!
//! Mounted by composition as an UNAUTHENTICATED route group — the
//! browser hits `/auth/providers`, `/auth/login/{provider}`, and
//! `/auth/callback/{provider}` before it has a session, so the
//! bearer-auth middleware must not run in front of them.
//!
//! `/auth/session/exchange` consumes the one-time login ticket the
//! callback placed in the redirect URL and returns the real session
//! bearer over a same-origin JSON response. `/auth/logout` accepts an
//! `Authorization: Bearer <token>` header (the session token the SPA
//! stored after exchange) and revokes the underlying session record.
//! Composition mounts these in the SAME public group as the login
//! routes for symmetry — logout and exchange have their own per-route
//! checks inside the handlers so a bare request is harmless.

use std::num::{NonZeroU32, NonZeroU64};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use chrono::Duration as ChronoDuration;
use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::TenantId;
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressJustification, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor, ListenerClass,
    RateLimitPolicy, RateLimitScope, StreamingMode, WebSocketOriginPolicy,
};
use ironclaw_reborn_composition::PublicRouteMount;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use super::error::OAuthError;
use super::pending::{PendingFlowStore, SessionTicketStore, sanitize_redirect};
use super::provider::OAuthProvider;
use super::provider_name::OAuthProviderName;
use super::user_directory::{UserDirectory, UserDirectoryError};
use crate::session::SessionStore;

/// Default landing page after a successful OAuth callback. The SPA
/// reads `?login_ticket=` and exchanges it for a bearer.
const DEFAULT_REDIRECT_AFTER: &str = "/v2";

/// Default session lifetime (30 days). Matches the v1 gateway's
/// `SESSION_LIFETIME_SECS`; production deployments can override via
/// [`OAuthRouterConfig::session_lifetime`].
const DEFAULT_SESSION_LIFETIME: ChronoDuration = ChronoDuration::seconds(30 * 24 * 60 * 60);

/// Owner-supplied config for the OAuth router.
///
/// `base_url` is the externally-visible origin the v2 listener is
/// reachable at (e.g. `https://app.example.com`). It is used to
/// build the OAuth `redirect_uri` Google calls back to and so it
/// must match what was registered in the Google Cloud Console.
#[derive(Clone)]
pub struct OAuthRouterConfig {
    pub tenant_id: TenantId,
    pub session_store: Arc<dyn SessionStore>,
    pub user_directory: Arc<dyn UserDirectory>,
    pub providers: Vec<Arc<dyn OAuthProvider>>,
    pub base_url: String,
    pub session_lifetime: ChronoDuration,
}

impl OAuthRouterConfig {
    /// Build a config with the default 30-day session lifetime.
    pub fn new(
        tenant_id: TenantId,
        session_store: Arc<dyn SessionStore>,
        user_directory: Arc<dyn UserDirectory>,
        providers: Vec<Arc<dyn OAuthProvider>>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            tenant_id,
            session_store,
            user_directory,
            providers,
            base_url: base_url.into(),
            session_lifetime: DEFAULT_SESSION_LIFETIME,
        }
    }

    pub fn with_session_lifetime(mut self, lifetime: ChronoDuration) -> Self {
        self.session_lifetime = lifetime;
        self
    }
}

/// Internal state shared across all `/auth/*` handlers.
struct RouterState {
    tenant_id: TenantId,
    session_store: Arc<dyn SessionStore>,
    user_directory: Arc<dyn UserDirectory>,
    providers: Vec<Arc<dyn OAuthProvider>>,
    base_url: String,
    session_lifetime: ChronoDuration,
    pending: PendingFlowStore,
    session_tickets: SessionTicketStore,
}

impl RouterState {
    fn provider(&self, name: &OAuthProviderName) -> Option<Arc<dyn OAuthProvider>> {
        self.providers
            .iter()
            .find(|p| p.name() == name)
            .map(Arc::clone)
    }

    fn callback_url(&self, provider_name: &OAuthProviderName) -> String {
        format!("{}/auth/callback/{provider_name}", self.base_url)
    }
}

type RouterStateHandle = Arc<RouterState>;

// ── route paths + descriptor IDs ──────────────────────────────────────

const PATH_PROVIDERS: &str = "/auth/providers";
const PATH_LOGIN: &str = "/auth/login/{provider}";
const PATH_CALLBACK: &str = "/auth/callback/{provider}";
const PATH_SESSION_EXCHANGE: &str = "/auth/session/exchange";
const PATH_LOGOUT: &str = "/auth/logout";

const ROUTE_ID_PROVIDERS: &str = "webui.sso.providers";
const ROUTE_ID_LOGIN: &str = "webui.sso.login";
const ROUTE_ID_CALLBACK: &str = "webui.sso.callback";
const ROUTE_ID_SESSION_EXCHANGE: &str = "webui.sso.session_exchange";
const ROUTE_ID_LOGOUT: &str = "webui.sso.logout";

/// Maximum session-exchange/logout body size. The exchange handler
/// reads only `{ "ticket": "..." }`; logout doesn't read a body, but
/// a tight cap still bounds oversized POSTs before handlers run.
const LOGOUT_BODY_LIMIT_BYTES: NonZeroU64 = NonZeroU64::new(1024).expect("1024 != 0"); // safety: const-evaluated, literal non-zero

/// Per-IP rate-limit window shared across every public SSO route.
/// 60-second sliding window mirrors the product-auth callback's
/// shape; the per-route `max_requests` differs by intent.
const SSO_RATE_WINDOW_SECONDS: NonZeroU32 = NonZeroU32::new(60).expect("60 != 0"); // safety: const-evaluated, literal non-zero
/// Discovery is cheap on the server side but the SPA hammers it on
/// every login-page render. 120/min/IP is comfortable for legitimate
/// browsers while blocking sustained floods.
const SSO_PROVIDERS_MAX_REQUESTS: NonZeroU32 = NonZeroU32::new(120).expect("120 != 0"); // safety: const-evaluated, literal non-zero
/// Login redirects insert into the pending-flow cache. 60/min/IP
/// caps the attack surface for filling the cache.
const SSO_LOGIN_MAX_REQUESTS: NonZeroU32 = NonZeroU32::new(60).expect("60 != 0"); // safety: const-evaluated, literal non-zero
/// Callbacks consume cache entries. Same per-IP cap as login so a
/// flood of fake callbacks cannot starve real ones.
const SSO_CALLBACK_MAX_REQUESTS: NonZeroU32 = NonZeroU32::new(60).expect("60 != 0"); // safety: const-evaluated, literal non-zero
/// Session-ticket exchanges are single-use and cheap. Keep the same
/// cap as login/callback so a brute-force loop cannot run unbounded.
const SSO_EXCHANGE_MAX_REQUESTS: NonZeroU32 = NonZeroU32::new(60).expect("60 != 0"); // safety: const-evaluated, literal non-zero
/// Logout. Per-IP, generous, because a sign-out blip should not 429.
const SSO_LOGOUT_MAX_REQUESTS: NonZeroU32 = NonZeroU32::new(60).expect("60 != 0"); // safety: const-evaluated, literal non-zero

/// Build the unauthenticated axum sub-router that mounts the OAuth
/// login endpoints plus the route descriptors composition needs to
/// install the per-route policy middleware around them.
pub fn webui_v2_auth_router(config: OAuthRouterConfig) -> PublicRouteMount {
    let state: RouterStateHandle = Arc::new(RouterState {
        tenant_id: config.tenant_id,
        session_store: config.session_store,
        user_directory: config.user_directory,
        providers: config.providers,
        base_url: config.base_url,
        session_lifetime: config.session_lifetime,
        pending: PendingFlowStore::new(),
        session_tickets: SessionTicketStore::new(),
    });

    let router = axum::Router::new()
        .route(PATH_PROVIDERS, get(providers_handler))
        .route(PATH_LOGIN, get(login_handler))
        .route(PATH_CALLBACK, get(callback_handler))
        .route(PATH_SESSION_EXCHANGE, post(session_exchange_handler))
        .route(PATH_LOGOUT, post(logout_handler))
        .with_state(state);

    PublicRouteMount::new(router, sso_route_descriptors())
}

// ── descriptors ───────────────────────────────────────────────────────

fn sso_route_descriptors() -> Vec<IngressRouteDescriptor> {
    vec![
        descriptor(
            ROUTE_ID_PROVIDERS,
            NetworkMethod::Get,
            PATH_PROVIDERS,
            public_policy(BodyLimitPolicy::NoBody, SSO_PROVIDERS_MAX_REQUESTS),
        ),
        descriptor(
            ROUTE_ID_LOGIN,
            NetworkMethod::Get,
            PATH_LOGIN,
            public_policy(BodyLimitPolicy::NoBody, SSO_LOGIN_MAX_REQUESTS),
        ),
        descriptor(
            ROUTE_ID_CALLBACK,
            NetworkMethod::Get,
            PATH_CALLBACK,
            // OAuthCallback listener class + Public auth + NoEffect
            // is the only shape `IngressPolicy::new` accepts for an
            // unauthenticated OAuth callback. Mirrors the existing
            // product-auth callback policy.
            callback_policy(SSO_CALLBACK_MAX_REQUESTS),
        ),
        descriptor(
            ROUTE_ID_SESSION_EXCHANGE,
            NetworkMethod::Post,
            PATH_SESSION_EXCHANGE,
            public_policy(
                BodyLimitPolicy::Limited {
                    max_bytes: LOGOUT_BODY_LIMIT_BYTES,
                },
                SSO_EXCHANGE_MAX_REQUESTS,
            ),
        ),
        descriptor(
            ROUTE_ID_LOGOUT,
            NetworkMethod::Post,
            PATH_LOGOUT,
            public_policy(
                BodyLimitPolicy::Limited {
                    max_bytes: LOGOUT_BODY_LIMIT_BYTES,
                },
                SSO_LOGOUT_MAX_REQUESTS,
            ),
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
        .expect("SSO route descriptor must validate at startup") // safety: ids/patterns are crate-local literals and policies are constructed by sibling helpers.
}

fn public_policy(body_limit: BodyLimitPolicy, max_requests: NonZeroU32) -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: IngressAuthPolicy::Public {
            justification: sso_justification(),
        },
        scope_source: ironclaw_host_api::IngressScopeSource::PublicRoute,
        body_limit,
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerIp,
            max_requests,
            window_seconds: SSO_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::PublicCallback,
        effect_path: AllowedEffectPath::NoEffect,
    })
    .expect("SSO public policy must validate") // safety: LocalGateway + Public + NoEffect is permitted; rate-limit window/max are non-zero by construction.
}

fn callback_policy(max_requests: NonZeroU32) -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::OAuthCallback,
        auth: IngressAuthPolicy::Public {
            justification: sso_justification(),
        },
        scope_source: ironclaw_host_api::IngressScopeSource::PublicRoute,
        body_limit: BodyLimitPolicy::NoBody,
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerIp,
            max_requests,
            window_seconds: SSO_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::NotApplicable,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::PublicCallback,
        effect_path: AllowedEffectPath::NoEffect,
    })
    .expect("SSO callback policy must validate") // safety: OAuthCallback + Public + NoEffect is the documented exception in `validate_listener_auth`.
}

fn sso_justification() -> IngressJustification {
    IngressJustification::new(
        "webui-v2 sso",
        "OAuth login surface is unauthenticated by design — \
         the user has no session yet; the handler mints one on \
         successful callback through SessionStore",
    )
    .expect("SSO justification literal must validate") // safety: non-empty, no leading/trailing whitespace.
}

// ─── /auth/providers ──────────────────────────────────────────────────

#[derive(Serialize)]
struct ProvidersResponse {
    providers: Vec<String>,
}

/// `GET /auth/providers` — list the providers configured at startup.
/// Empty list when no provider was wired. The SPA filters this list
/// against its supported set so a future backend that adds new
/// providers without a matching SPA build still renders cleanly.
async fn providers_handler(State(state): State<RouterStateHandle>) -> Json<ProvidersResponse> {
    let mut providers: Vec<String> = state
        .providers
        .iter()
        .map(|p| p.name().as_str().to_string())
        .collect();
    providers.sort_unstable();
    Json(ProvidersResponse { providers })
}

// ─── /auth/login/{provider} ───────────────────────────────────────────

#[derive(Deserialize)]
struct LoginParams {
    /// Optional same-origin path the SPA should land on after the
    /// callback completes. Validated through `sanitize_redirect` to
    /// block open redirects.
    redirect_after: Option<String>,
}

/// `GET /auth/login/{provider}` — initiate the OAuth flow. Mints a
/// pending-flow entry and redirects the browser to the provider's
/// authorization URL.
async fn login_handler(
    State(state): State<RouterStateHandle>,
    Path(raw_provider): Path<String>,
    Query(params): Query<LoginParams>,
) -> Response {
    // Validate at the boundary: an ill-formed `{provider}` segment
    // (path traversal, uppercase, oversized) fails closed before
    // any state-store mutation.
    let Ok(provider_name) = OAuthProviderName::new(raw_provider.clone()) else {
        return (
            StatusCode::NOT_FOUND,
            format!("Unknown OAuth provider: {raw_provider}"),
        )
            .into_response();
    };
    let Some(provider) = state.provider(&provider_name) else {
        return (
            StatusCode::NOT_FOUND,
            format!("Unknown OAuth provider: {provider_name}"),
        )
            .into_response();
    };

    let redirect_after = sanitize_redirect(params.redirect_after);
    let (csrf_state, flow) = state
        .pending
        .insert(provider.name().clone(), redirect_after);
    let callback_url = state.callback_url(provider.name());
    // `flow.code_challenge` is the SHA-256 the pending store
    // pre-computed at mint time — no second hash per login redirect.
    let auth_url = provider.authorization_url(&callback_url, &csrf_state, &flow.code_challenge);

    Redirect::temporary(&auth_url).into_response()
}

// ─── /auth/callback/{provider} ────────────────────────────────────────

#[derive(Deserialize)]
struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// `GET /auth/callback/{provider}` — handle the provider's callback,
/// exchange the code, resolve the user, issue a session, redirect
/// back to the SPA with a one-time exchange ticket in the URL query.
async fn callback_handler(
    State(state): State<RouterStateHandle>,
    Path(raw_provider): Path<String>,
    Query(params): Query<CallbackParams>,
) -> Response {
    // Validate the URL `{provider}` segment at the boundary. An
    // ill-formed segment can never match a flow in the pending
    // store, but typing the segment here keeps every downstream
    // comparison a newtype `==` so a future refactor cannot
    // re-introduce stringly-typed drift.
    let Ok(provider_name) = OAuthProviderName::new(raw_provider) else {
        return spa_error_redirect("invalid_request").into_response();
    };

    // Provider-initiated denial (user clicked "cancel" on the consent
    // screen, account suspended, etc.). Surface a generic redirect
    // back to the SPA with `?login_error=denied` so the login page
    // can render an error banner without exposing the provider's
    // description verbatim.
    if let Some(error) = params.error {
        tracing::debug!(
            target = "ironclaw::reborn::webui_ingress::auth",
            provider = %provider_name,
            error = %error,
            description = ?params.error_description,
            "OAuth provider returned an error",
        );
        return spa_error_redirect("denied").into_response();
    }

    let Some(code) = params.code.filter(|c| !c.is_empty()) else {
        return spa_error_redirect("invalid_request").into_response();
    };
    let Some(csrf_state) = params.state.filter(|s| !s.is_empty()) else {
        return spa_error_redirect("invalid_request").into_response();
    };

    let Some(flow) = state.pending.take(&csrf_state) else {
        // Unknown state token: either expired (>5 min in the pending
        // store) or a replay of an already-consumed callback. Fail
        // closed — never re-use a state token.
        return spa_error_redirect("invalid_state").into_response();
    };
    if flow.provider != provider_name {
        // Cross-provider state replay (e.g. GitHub state arriving on
        // the Google callback). Fail closed.
        return spa_error_redirect("provider_mismatch").into_response();
    }

    let Some(provider) = state.provider(&provider_name) else {
        return spa_error_redirect("invalid_request").into_response();
    };

    let callback_url = state.callback_url(provider.name());
    let profile = match provider
        .exchange_code(&code, &callback_url, flow.code_verifier.expose_secret())
        .await
    {
        Ok(profile) => profile,
        Err(err) => {
            log_oauth_error(&provider_name, &err);
            return spa_error_redirect(error_code_for(&err)).into_response();
        }
    };

    let user_id = match state
        .user_directory
        .resolve(provider.name(), &profile)
        .await
    {
        Ok(uid) => uid,
        Err(UserDirectoryError::Unknown) => {
            tracing::debug!(
                target = "ironclaw::reborn::webui_ingress::auth",
                provider = %provider_name,
                email = ?profile.email,
                "user directory rejected unknown profile",
            );
            return spa_error_redirect("unauthorized").into_response();
        }
        Err(UserDirectoryError::Backend(reason)) => {
            tracing::warn!(
                target = "ironclaw::reborn::webui_ingress::auth",
                provider = %provider_name,
                error = %reason,
                "user directory backend failure",
            );
            return spa_error_redirect("server_error").into_response();
        }
    };

    let bearer = match state
        .session_store
        .create_session(state.tenant_id.clone(), user_id, state.session_lifetime)
        .await
    {
        Ok(token) => token,
        Err(err) => {
            tracing::error!(
                target = "ironclaw::reborn::webui_ingress::auth",
                provider = %provider_name,
                error = %err,
                "session store create_session failed",
            );
            return spa_error_redirect("server_error").into_response();
        }
    };

    let redirect_after = flow
        .redirect_after
        .as_deref()
        .unwrap_or(DEFAULT_REDIRECT_AFTER);
    let ticket = state.session_tickets.insert(bearer);
    let location = build_success_redirect(redirect_after, &ticket);
    Redirect::to(&location).into_response()
}

// ─── /auth/session/exchange ───────────────────────────────────────────

#[derive(Deserialize)]
struct SessionExchangeRequest {
    ticket: String,
}

#[derive(Serialize)]
struct SessionExchangeResponse {
    token: String,
}

/// `POST /auth/session/exchange` — consume a short-lived one-time
/// ticket produced by the OAuth callback and return the real session
/// bearer. This keeps the bearer out of redirect `Location` headers
/// while preserving the existing SPA bearer/sessionStorage model.
async fn session_exchange_handler(
    State(state): State<RouterStateHandle>,
    Json(request): Json<SessionExchangeRequest>,
) -> Response {
    let ticket = request.ticket.trim();
    if ticket.is_empty() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(bearer) = state.session_tickets.take(ticket) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    Json(SessionExchangeResponse {
        token: bearer.expose_secret().to_string(),
    })
    .into_response()
}

// ─── /auth/logout ─────────────────────────────────────────────────────

/// `POST /auth/logout` — revoke the bearer session and clear it from
/// the durable session store. Honors `Authorization: Bearer <token>`
/// only — query-token shim is reserved for the SSE route per the
/// composition's `extract_bearer_token` policy.
///
/// **Status contract:**
/// - `204 No Content` when there is no bearer header (idempotent
///   sign-out — the SPA clears local state unconditionally), OR
///   when the session store confirms the revoke.
/// - `500 Internal Server Error` when the session store backend
///   fails to revoke. A success status in this case would lie to
///   the caller: the bearer might still authenticate in another
///   tab or another client until natural expiry, violating the
///   PR's revoke contract. The SPA still clears its local copy
///   regardless of the response status — losing the local token
///   is strictly weaker than a stale bearer roaming the network.
async fn logout_handler(
    State(state): State<RouterStateHandle>,
    headers: axum::http::HeaderMap,
) -> Response {
    let Some(token) = extract_bearer(&headers) else {
        return StatusCode::NO_CONTENT.into_response();
    };
    match state.session_store.revoke(&token).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => {
            tracing::warn!(
                target = "ironclaw::reborn::webui_ingress::auth",
                error = %err,
                "session store revoke failed during logout — returning 500 so the \
                 client knows the server-side revocation did not complete",
            );
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ─── helpers ──────────────────────────────────────────────────────────

fn extract_bearer(headers: &axum::http::HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?;
    let text = value.to_str().ok()?;
    let prefix = text.get(..7)?;
    if !prefix.eq_ignore_ascii_case("Bearer ") {
        return None;
    }
    let candidate = text[7..].trim();
    if candidate.is_empty() {
        None
    } else {
        Some(candidate.to_string())
    }
}

/// Build the success redirect URL:
/// `<redirect_after>?login_ticket=<ticket>` (or `&login_ticket=...`
/// if `redirect_after` already carries a query string). The ticket is
/// short-lived and single-use; the bearer never appears in a redirect
/// `Location` header.
fn build_success_redirect(redirect_after: &str, ticket: &str) -> String {
    // `redirect_after` was already validated by `sanitize_redirect`
    // to start with `/`, contain only RFC-3986 path/query chars, and
    // exclude fragments.
    let separator = if redirect_after.contains('?') {
        '&'
    } else {
        '?'
    };
    format!(
        "{redirect_after}{separator}login_ticket={}",
        urlencoding::encode(ticket)
    )
}

/// Build a redirect back to the SPA login route with an opaque error
/// code in the query string. The SPA maps the code to a localized
/// error banner.
fn spa_error_redirect(code: &str) -> Redirect {
    let target = format!("/v2?login_error={}", urlencoding::encode(code));
    Redirect::to(&target)
}

fn error_code_for(err: &OAuthError) -> &'static str {
    match err {
        OAuthError::CodeExchange(_) | OAuthError::ProfileFetch(_) => "exchange_failed",
        OAuthError::Denied(_) => "unauthorized",
    }
}

fn log_oauth_error(provider_name: &OAuthProviderName, err: &OAuthError) {
    // Provider error bodies and JWT decode details are operator-only
    // — never echoed back to the client. Logged at `warn!` so they
    // appear in production logs without spamming `info!` on every
    // user-cancelled login.
    tracing::warn!(
        target = "ironclaw::reborn::webui_ingress::auth",
        provider = %provider_name,
        error = %err,
        "OAuth flow failed",
    );
}
